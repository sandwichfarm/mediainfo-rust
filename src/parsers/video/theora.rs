//! Theora identification header (Theora Specification §6.2) and the Xiph-laced Matroska private
//! data that carries the three header packets.

use crate::io::{be16, be24, be32};
use crate::model::Stream;

#[derive(Debug, Clone, Default)]
pub struct Ident {
    pub version: (u8, u8, u8),
    pub frame_width: u32,  // in macroblocks × 16
    pub frame_height: u32,
    pub pic_width: u32,
    pub pic_height: u32,
    pub pic_x: u8,
    pub pic_y: u8,
    pub frame_rate: (u32, u32),
    pub aspect: (u32, u32),
    pub colour_space: u8,
    pub nominal_bitrate: u32,
    pub quality: u8,
    pub keyframe_shift: u8,
    pub pixel_format: u8,
}

pub fn parse_ident(d: &[u8]) -> Option<Ident> {
    if d.len() < 42 || d[0] != 0x80 || &d[1..7] != b"theora" {
        return None;
    }
    let i = Ident {
        version: (d[7], d[8], d[9]),
        frame_width: be16(d, 10)? as u32 * 16,
        frame_height: be16(d, 12)? as u32 * 16,
        pic_width: be24(d, 14)?,
        pic_height: be24(d, 17)?,
        pic_x: d[20],
        pic_y: d[21],
        frame_rate: (be32(d, 22)?, be32(d, 26)?),
        aspect: (be24(d, 30)?, be24(d, 33)?),
        colour_space: d[36],
        nominal_bitrate: be24(d, 37)?,
        quality: d[40] >> 2,
        keyframe_shift: ((d[40] & 3) << 3) | (d[41] >> 5),
        pixel_format: (d[41] >> 3) & 3,
    };
    if i.version.0 != 3 || i.frame_width == 0 || i.frame_height == 0 {
        return None;
    }
    Some(i)
}

/// Fill a video stream from the identification header.
pub fn apply_ident(s: &mut Stream, d: &[u8]) -> bool {
    let Some(i) = parse_ident(d) else { return false };
    s.set_if_empty("Format", "Theora");
    let (w, h) = if i.pic_width > 0 && i.pic_height > 0 { (i.pic_width, i.pic_height) } else { (i.frame_width, i.frame_height) };
    s.set_if_empty("Width", w.to_string());
    s.set_if_empty("Height", h.to_string());
    if i.frame_rate.0 > 0 && i.frame_rate.1 > 0 {
        s.set_if_empty("FrameRate", format!("{:.3}", i.frame_rate.0 as f64 / i.frame_rate.1 as f64));
    }
    if i.aspect.0 > 0 && i.aspect.1 > 0 && !s.has("PixelAspectRatio") && !s.has("DisplayAspectRatio") {
        s.set("PixelAspectRatio", format!("{:.3}", i.aspect.0 as f64 / i.aspect.1 as f64));
    }
    if i.nominal_bitrate > 0 {
        s.set_if_empty("BitRate_Nominal", i.nominal_bitrate.to_string());
    }
    s.set_if_empty("Compression_Mode", "Lossy");
    true
}

/// Vendor string of a Vorbis-style comment header (`\x81theora`).
pub fn comment_vendor(d: &[u8]) -> Option<String> {
    if d.len() < 11 || d[0] != 0x81 || &d[1..7] != b"theora" {
        return None;
    }
    let len = crate::io::le32(d, 7)? as usize;
    let v = d.get(11..11 + len.min(256))?;
    let text = crate::io::clean_text(&String::from_utf8_lossy(v));
    if text.is_empty() { None } else { Some(text) }
}

/// Split Xiph-laced private data (count byte, 255-run sizes, packets).
pub fn xiph_packets(d: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let Some(&n) = d.first() else { return out };
    let mut sizes = Vec::new();
    let mut i = 1;
    for _ in 0..n {
        let mut size = 0usize;
        loop {
            let Some(&b) = d.get(i) else { return out };
            i += 1;
            size += b as usize;
            if b != 255 {
                break;
            }
        }
        sizes.push(size);
    }
    for size in sizes {
        let Some(p) = d.get(i..i + size) else { return out };
        out.push(p);
        i += size;
    }
    if i <= d.len() {
        out.push(&d[i..]);
    }
    out
}

/// Matroska CodecPrivate: the identification, comment and setup headers.
pub fn apply_xiph_private(s: &mut Stream, d: &[u8]) -> bool {
    let packets = xiph_packets(d);
    let mut ok = false;
    for p in &packets {
        if apply_ident(s, p) {
            ok = true;
        } else if let Some(vendor) = comment_vendor(p) {
            s.set_if_empty("Encoded_Library", vendor);
        }
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;

    fn ident() -> Vec<u8> {
        let mut d = vec![0x80];
        d.extend(b"theora");
        d.extend([3, 2, 1]);
        d.extend(4u16.to_be_bytes()); // 64
        d.extend(3u16.to_be_bytes()); // 48
        d.extend([0, 0, 64, 0, 0, 48, 0, 0]);
        d.extend(25u32.to_be_bytes());
        d.extend(1u32.to_be_bytes());
        d.extend([0, 0, 1, 0, 0, 1]); // 1:1
        d.push(1); // colour space
        d.extend([0x01, 0x86, 0xA0]); // 100000 bps
        d.extend([40 << 2, 0]); // quality 40, kfgshift 0, pf 0
        d
    }

    #[test]
    fn ident_header() {
        let d = ident();
        let i = parse_ident(&d).unwrap();
        assert_eq!((i.pic_width, i.pic_height), (64, 48));
        assert_eq!(i.frame_rate, (25, 1));
        assert_eq!(i.nominal_bitrate, 100000);
        assert_eq!(i.quality, 40);
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_ident(&mut s, &d));
        assert_eq!(s.get("Format"), "Theora");
        assert_eq!(s.get("FrameRate"), "25.000");
        assert_eq!(s.get("BitRate_Nominal"), "100000");
        assert!(!apply_ident(&mut s, &d[..30]));
    }

    #[test]
    fn xiph_lacing() {
        let id = ident();
        let mut comment = vec![0x81];
        comment.extend(b"theora");
        comment.extend(12u32.to_le_bytes());
        comment.extend(b"Lavf63.1.101");
        comment.extend(0u32.to_le_bytes());
        let setup = vec![0x82, 1, 2, 3];
        let mut d = vec![2, id.len() as u8, comment.len() as u8];
        d.extend(&id);
        d.extend(&comment);
        d.extend(&setup);
        let p = xiph_packets(&d);
        assert_eq!(p.len(), 3);
        assert_eq!(p[2], &setup[..]);
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_xiph_private(&mut s, &d));
        assert_eq!(s.get("Width"), "64");
        assert_eq!(s.get("Encoded_Library"), "Lavf63.1.101");
        assert!(xiph_packets(&[2, 255]).is_empty());
        assert!(!apply_xiph_private(&mut Stream::new(StreamKind::Video), &[]));
    }
}

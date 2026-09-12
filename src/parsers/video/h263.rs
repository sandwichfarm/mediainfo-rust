//! H.263 picture header (ITU-T H.263 §5.1): picture start code, PTYPE source format, PLUSPTYPE
//! custom picture format and pixel aspect ratio.

use crate::io::bits::BitReader;
use crate::model::Stream;

#[derive(Debug, Clone, Default)]
pub struct PictureHeader {
    pub temporal_reference: u8,
    pub source_format: u8,
    pub width: u32,
    pub height: u32,
    pub intra: bool,
    pub plus_ptype: bool,
    pub optional_modes: bool,
    pub par: Option<(u32, u32)>,
}

fn standard_size(source_format: u8) -> Option<(u32, u32)> {
    Some(match source_format {
        1 => (128, 96),
        2 => (176, 144),
        3 => (352, 288),
        4 => (704, 576),
        5 => (1408, 1152),
        _ => return None,
    })
}

pub fn parse_picture(d: &[u8]) -> Option<PictureHeader> {
    let mut r = BitReader::new(d);
    if r.u32(22)? != 0x20 {
        return None;
    }
    let mut h = PictureHeader { temporal_reference: r.u8(8)?, ..Default::default() };
    if !r.bit()? || r.bit()? {
        return None; // PTYPE bits 1 and 2 must be 1 and 0
    }
    r.skip(3); // split screen, document camera, freeze picture release
    h.source_format = r.u8(3)?;
    if h.source_format == 7 {
        h.plus_ptype = true;
        let ufep = r.u8(3)?;
        if ufep == 1 {
            h.source_format = r.u8(3)?;
            r.skip(15); // custom PCF, optional modes and reserved bits of OPPTYPE
        }
        let picture_type_code = r.u8(3)?;
        h.intra = picture_type_code == 0;
        r.skip(6); // remaining MPPTYPE bits
        r.bit()?; // CPM
        if ufep == 1 && h.source_format == 6 {
            let par_code = r.u8(4)?;
            let pwi = r.u32(9)?;
            r.bit()?;
            let phi = r.u32(9)?;
            h.width = (pwi + 1) * 4;
            h.height = phi * 4;
            h.par = match par_code {
                1 => Some((1, 1)),
                2 => Some((12, 11)),
                3 => Some((10, 11)),
                4 => Some((16, 11)),
                5 => Some((40, 33)),
                15 => Some((r.u32(8)?, r.u32(8)?)),
                _ => None,
            };
        }
    } else {
        h.intra = !r.bit()?;
        h.optional_modes = r.u8(4)? != 0; // UMV, SAC, AP, PB
    }
    if h.width == 0 {
        let (w, hh) = standard_size(h.source_format)?;
        h.width = w;
        h.height = hh;
        h.par = Some((12, 11));
    }
    if h.width == 0 || h.height == 0 {
        return None;
    }
    Some(h)
}

/// Fill a video stream from a picture. `Format_Profile` (profile and level) is container metadata.
pub fn apply_frame(s: &mut Stream, d: &[u8]) -> bool {
    let Some(h) = parse_picture(d) else { return false };
    s.set_if_empty("Format", "H.263");
    s.set_if_empty("Width", h.width.to_string());
    s.set_if_empty("Height", h.height.to_string());
    if let Some((n, dn)) = h.par {
        if n > 0 && dn > 0 && !s.has("PixelAspectRatio") && !s.has("DisplayAspectRatio") {
            s.set("PixelAspectRatio", format!("{:.3}", n as f64 / dn as f64));
        }
    }
    s.set_if_empty("ColorSpace", "YUV");
    s.set_if_empty("ChromaSubsampling", "4:2:0");
    s.set_if_empty("BitDepth", "8");
    s.set_if_empty("ScanType", "Progressive");
    s.set_if_empty("Compression_Mode", "Lossy");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;
    use crate::parsers::video::testutil::BitWriter;

    #[test]
    fn baseline_qcif() {
        let mut b = BitWriter::new();
        b.b(0x20, 22);
        b.b(5, 8); // TR
        b.b(0b10, 2);
        b.b(0, 3);
        b.b(2, 3); // QCIF
        b.b(0, 1); // intra
        b.b(0, 4);
        b.b(10, 5); // PQUANT
        b.b(0, 1);
        let d = b.done();
        let h = parse_picture(&d).unwrap();
        assert_eq!((h.width, h.height), (176, 144));
        assert!(h.intra && !h.plus_ptype);
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_frame(&mut s, &d));
        assert_eq!(s.get("Format"), "H.263");
        assert_eq!(s.get("PixelAspectRatio"), "1.091");
        assert_eq!(s.get("Width"), "176");
        assert!(parse_picture(&[0, 0, 0x80]).is_none());
        assert!(parse_picture(&[]).is_none());
    }

    #[test]
    fn plusptype_custom() {
        let mut b = BitWriter::new();
        b.b(0x20, 22);
        b.b(0, 8);
        b.b(0b10, 2);
        b.b(0, 3);
        b.b(7, 3); // PLUSPTYPE
        b.b(1, 3); // UFEP
        b.b(6, 3); // custom format
        b.b(0, 15);
        b.b(1, 3); // inter
        b.b(0, 6);
        b.b(0, 1); // CPM
        b.b(1, 4); // PAR 1:1
        b.b(159, 9); // (159+1)*4 = 640
        b.b(1, 1);
        b.b(120, 9); // 480
        let d = b.done();
        let h = parse_picture(&d).unwrap();
        assert_eq!((h.width, h.height), (640, 480));
        assert_eq!(h.par, Some((1, 1)));
        assert!(!h.intra && h.plus_ptype);
    }
}

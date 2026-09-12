//! VP9 uncompressed frame header (VP9 Bitstream Specification §6.2) and the ISOBMFF `vpcC` box
//! (VP Codec ISO Media File Format Binding §2.3).

use crate::io::bits::BitReader;
use crate::model::Stream;

#[derive(Debug, Clone, Default)]
pub struct FrameHeader {
    pub profile: u8,
    pub show_existing_frame: bool,
    pub key_frame: bool,
    pub show_frame: bool,
    pub error_resilient: bool,
    pub bit_depth: u8,
    pub color_space: u8,
    pub color_range: Option<bool>, // full range
    pub subsampling_x: bool,
    pub subsampling_y: bool,
    pub width: u32,
    pub height: u32,
    pub render_width: u32,
    pub render_height: u32,
}

fn color_config(r: &mut BitReader, h: &mut FrameHeader) -> Option<()> {
    h.bit_depth = if h.profile >= 2 { if r.bit()? { 12 } else { 10 } } else { 8 };
    h.color_space = r.u8(3)?;
    if h.color_space != 7 {
        h.color_range = Some(r.bit()?);
        if h.profile == 1 || h.profile == 3 {
            h.subsampling_x = r.bit()?;
            h.subsampling_y = r.bit()?;
            r.bit()?; // reserved
        } else {
            h.subsampling_x = true;
            h.subsampling_y = true;
        }
    } else {
        h.color_range = Some(true);
        if h.profile == 1 || h.profile == 3 {
            r.bit()?; // reserved
        }
    }
    Some(())
}

fn frame_size(r: &mut BitReader, h: &mut FrameHeader) -> Option<()> {
    h.width = r.u32(16)? + 1;
    h.height = r.u32(16)? + 1;
    h.render_width = h.width;
    h.render_height = h.height;
    if r.bit()? {
        h.render_width = r.u32(16)? + 1;
        h.render_height = r.u32(16)? + 1;
    }
    Some(())
}

/// Parse the uncompressed header; colour config and size are only present on key / intra-only frames.
pub fn parse_frame(d: &[u8]) -> Option<FrameHeader> {
    let mut r = BitReader::new(d);
    if r.u8(2)? != 2 {
        return None;
    }
    let low = r.u8(1)?;
    let high = r.u8(1)?;
    let mut h = FrameHeader { profile: (high << 1) | low, ..Default::default() };
    if h.profile == 3 && r.bit()? {
        return None;
    }
    h.show_existing_frame = r.bit()?;
    if h.show_existing_frame {
        r.skip(3);
        return Some(h);
    }
    h.key_frame = !r.bit()?;
    h.show_frame = r.bit()?;
    h.error_resilient = r.bit()?;
    if h.key_frame {
        if r.u32(24)? != 0x498342 {
            return None;
        }
        color_config(&mut r, &mut h)?;
        frame_size(&mut r, &mut h)?;
    } else {
        let intra_only = if h.show_frame { false } else { r.bit()? };
        if !h.error_resilient {
            r.skip(2); // reset_frame_context
        }
        if intra_only {
            if r.u32(24)? != 0x498342 {
                return None;
            }
            if h.profile > 0 {
                color_config(&mut r, &mut h)?;
            } else {
                h.bit_depth = 8;
                h.subsampling_x = true;
                h.subsampling_y = true;
            }
            r.skip(8); // refresh_frame_flags
            frame_size(&mut r, &mut h)?;
        }
    }
    Some(h)
}

pub fn chroma_name(sub_x: bool, sub_y: bool) -> &'static str {
    match (sub_x, sub_y) {
        (true, true) => "4:2:0",
        (true, false) => "4:2:2",
        (false, false) => "4:4:4",
        (false, true) => "4:4:0",
    }
}

/// Fill a video stream from a frame. The reference only takes the format and, from a key frame,
/// the picture size from the VP9 bitstream; the detailed colour configuration is reported through
/// `apply_vpcc` when a container carries it.
pub fn apply_frame(s: &mut Stream, d: &[u8]) -> bool {
    let Some(h) = parse_frame(d) else { return false };
    s.set_if_empty("Format", "VP9");
    if h.key_frame && h.width > 0 && h.height > 0 {
        s.set_if_empty("Width", h.width.to_string());
        s.set_if_empty("Height", h.height.to_string());
    }
    true
}

/// `vpcC` box payload (after the full-box header): profile, level, bit depth, chroma, colour.
pub fn apply_vpcc(s: &mut Stream, d: &[u8]) -> bool {
    // FullBox: version(1) flags(3), then profile(1) level(1) bitDepth(4)|chromaSubsampling(3)|videoFullRangeFlag(1),
    // colourPrimaries(1) transferCharacteristics(1) matrixCoefficients(1) codecInitializationDataSize(2)
    if d.len() < 12 || d[0] != 1 {
        return false;
    }
    let profile = d[4];
    let level = d[5];
    let bit_depth = d[6] >> 4;
    let chroma = (d[6] >> 1) & 7;
    let full_range = d[6] & 1 == 1;
    s.set_if_empty("Format", "VP9");
    let mut fp = profile.to_string();
    if level > 0 {
        fp.push_str(&format!("@L{}.{}", level / 10, level % 10));
    }
    s.set_if_empty("Format_Profile", fp);
    if bit_depth > 0 {
        s.set_if_empty("BitDepth", bit_depth.to_string());
    }
    s.set_if_empty("ColorSpace", "YUV");
    s.set_if_empty(
        "ChromaSubsampling",
        match chroma {
            0 | 1 => "4:2:0",
            2 => "4:2:2",
            3 => "4:4:4",
            4 => "4:4:0",
            _ => "",
        },
    );
    super::colour::set_range(s, full_range, "Container");
    super::colour::set_description(s, d[7], d[8], d[9], "Container");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;
    use crate::parsers::video::testutil::BitWriter;

    fn key_frame(profile: u8, w: u32, h: u32) -> Vec<u8> {
        let mut b = BitWriter::new();
        b.b(2, 2);
        b.b((profile & 1) as u64, 1);
        b.b((profile >> 1) as u64, 1);
        if profile == 3 {
            b.b(0, 1);
        }
        b.b(0, 1); // show_existing_frame
        b.b(0, 1); // key frame
        b.b(1, 1); // show_frame
        b.b(0, 1); // error resilient
        b.b(0x498342, 24);
        if profile >= 2 {
            b.b(0, 1); // 10 bit
        }
        b.b(1, 3); // BT.601
        b.b(0, 1); // limited range
        if profile == 1 || profile == 3 {
            b.b(0, 1);
            b.b(0, 1);
            b.b(0, 1);
        }
        b.b((w - 1) as u64, 16);
        b.b((h - 1) as u64, 16);
        b.b(0, 1);
        b.done()
    }

    #[test]
    fn frames() {
        let d = key_frame(0, 64, 48);
        let h = parse_frame(&d).unwrap();
        assert!(h.key_frame);
        assert_eq!((h.width, h.height, h.bit_depth), (64, 48, 8));
        assert_eq!(chroma_name(h.subsampling_x, h.subsampling_y), "4:2:0");
        assert_eq!(h.color_range, Some(false));
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_frame(&mut s, &d));
        assert_eq!(s.get("Format"), "VP9");
        assert_eq!(s.get("Width"), "64");
        let h = parse_frame(&key_frame(3, 1920, 1080)).unwrap();
        assert_eq!((h.profile, h.bit_depth), (3, 10));
        assert_eq!(chroma_name(h.subsampling_x, h.subsampling_y), "4:4:4");
        // inter frame
        let h = parse_frame(&[0b1000_0110, 0]).unwrap();
        assert!(!h.key_frame && h.show_frame);
        assert!(parse_frame(&[0xC0]).is_none());
        assert!(parse_frame(&[]).is_none());
        for n in 0..d.len() {
            let _ = parse_frame(&d[..n]);
        }
    }

    #[test]
    fn vpcc() {
        let d = [1, 0, 0, 0, 2, 31, (10 << 4) | (1 << 1), 9, 16, 9, 0, 0];
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_vpcc(&mut s, &d));
        assert_eq!(s.get("Format_Profile"), "2@L3.1");
        assert_eq!(s.get("BitDepth"), "10");
        assert_eq!(s.get("ChromaSubsampling"), "4:2:0");
        assert_eq!(s.get("colour_range"), "Limited");
        assert_eq!(s.get("colour_primaries"), "BT.2020");
        assert_eq!(s.get("transfer_characteristics"), "PQ");
        assert!(!apply_vpcc(&mut s, &d[..5]));
    }
}

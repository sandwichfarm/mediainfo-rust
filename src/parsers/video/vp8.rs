//! VP8 frame header (RFC 6386 §9.1): frame tag and key frame start code / dimensions.

use crate::model::Stream;

#[derive(Debug, Clone, Default)]
pub struct FrameHeader {
    pub key_frame: bool,
    pub version: u8,
    pub show_frame: bool,
    pub first_part_size: u32,
    pub width: u32,
    pub height: u32,
    pub horizontal_scale: u8,
    pub vertical_scale: u8,
}

/// Parse the uncompressed data chunk; dimensions are only present on key frames.
pub fn parse_frame(d: &[u8]) -> Option<FrameHeader> {
    let tag = crate::io::le24(d, 0)?;
    let mut h = FrameHeader { key_frame: tag & 1 == 0, version: ((tag >> 1) & 7) as u8, show_frame: (tag >> 4) & 1 == 1, first_part_size: tag >> 5, ..Default::default() };
    if h.version > 3 {
        return None;
    }
    if h.key_frame {
        if d.get(3..6)? != [0x9D, 0x01, 0x2A] {
            return None;
        }
        let w = crate::io::le16(d, 6)?;
        let hgt = crate::io::le16(d, 8)?;
        h.width = (w & 0x3FFF) as u32;
        h.height = (hgt & 0x3FFF) as u32;
        h.horizontal_scale = (w >> 14) as u8;
        h.vertical_scale = (hgt >> 14) as u8;
        if h.width == 0 || h.height == 0 {
            return None;
        }
    }
    Some(h)
}

/// Fill a video stream from a (key) frame.
pub fn apply_frame(s: &mut Stream, d: &[u8]) -> bool {
    let Some(h) = parse_frame(d) else { return false };
    s.set_if_empty("Format", "VP8");
    s.set_if_empty("Compression_Mode", "Lossy");
    if h.key_frame {
        s.set_if_empty("Width", h.width.to_string());
        s.set_if_empty("Height", h.height.to_string());
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;

    #[test]
    fn key_frame() {
        // key frame, version 0, shown, first partition 100 bytes; 64x48 unscaled
        let tag: u32 = (100 << 5) | (1 << 4);
        let mut d = tag.to_le_bytes()[..3].to_vec();
        d.extend([0x9D, 0x01, 0x2A, 64, 0, 48, 0, 0xAA]);
        let h = parse_frame(&d).unwrap();
        assert!(h.key_frame && h.show_frame);
        assert_eq!((h.width, h.height), (64, 48));
        assert_eq!(h.first_part_size, 100);
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_frame(&mut s, &d));
        assert_eq!(s.get("Format"), "VP8");
        assert_eq!(s.get("Width"), "64");
        assert_eq!(s.get("Compression_Mode"), "Lossy");
        // inter frame: no dimensions
        let h = parse_frame(&[0x11, 0, 0]).unwrap();
        assert!(!h.key_frame);
        assert!(parse_frame(&[0, 0, 0, 1, 2, 3, 0, 0, 0, 0]).is_none());
        assert!(parse_frame(&[0, 0]).is_none());
    }
}

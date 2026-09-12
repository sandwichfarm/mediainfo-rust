//! Apple ProRes frame header (SMPTE RDD 36 §5.1): `icpf` frame identifier, picture size, chroma
//! format, interlace mode, aspect ratio and frame rate codes, colour description.

use crate::io::{be16, be32};
use crate::model::Stream;

#[derive(Debug, Clone, Default)]
pub struct FrameHeader {
    pub frame_size: u32,
    pub header_size: u16,
    pub version: u8,
    pub encoder_id: [u8; 4],
    pub width: u16,
    pub height: u16,
    pub chroma_format: u8,  // 2 = 4:2:2, 3 = 4:4:4
    pub interlace_mode: u8, // 0 progressive, 1 interlaced TFF, 2 interlaced BFF
    pub aspect_ratio: u8,   // 0 unknown, 1 square, 2 4:3, 3 16:9
    pub frame_rate_code: u8,
    pub colour: (u8, u8, u8),
    pub alpha_channel_type: u8,
}

pub fn parse_frame(d: &[u8]) -> Option<FrameHeader> {
    if d.get(4..8)? != b"icpf" {
        return None;
    }
    let h = FrameHeader {
        frame_size: be32(d, 0)?,
        header_size: be16(d, 8)?,
        version: *d.get(11)?,
        encoder_id: [*d.get(12)?, *d.get(13)?, *d.get(14)?, *d.get(15)?],
        width: be16(d, 16)?,
        height: be16(d, 18)?,
        chroma_format: d.get(20)? >> 6,
        interlace_mode: (d.get(20)? >> 2) & 3,
        aspect_ratio: d.get(21)? >> 4,
        frame_rate_code: d.get(21)? & 0xF,
        colour: (*d.get(22)?, *d.get(23)?, *d.get(24)?),
        alpha_channel_type: d.get(25)? & 0xF,
    };
    if h.header_size < 20 || h.width == 0 || h.height == 0 {
        return None;
    }
    Some(h)
}

pub fn frame_rate(code: u8) -> Option<f64> {
    Some(match code {
        1 => 24000.0 / 1001.0,
        2 => 24.0,
        3 => 25.0,
        4 => 30000.0 / 1001.0,
        5 => 30.0,
        6 => 50.0,
        7 => 60000.0 / 1001.0,
        8 => 60.0,
        9 => 100.0,
        10 => 120000.0 / 1001.0,
        11 => 120.0,
        _ => return None,
    })
}

/// Fill a video stream from a frame. `Format_Profile` comes from the container's codec ID.
pub fn apply_frame(s: &mut Stream, d: &[u8]) -> bool {
    let Some(h) = parse_frame(d) else { return false };
    s.set_if_empty("Format", "ProRes");
    s.set_if_empty("Format_Version", format!("Version {}", h.version));
    s.set_if_empty("Width", h.width.to_string());
    s.set_if_empty("Height", h.height.to_string());
    match h.aspect_ratio {
        1 => s.set_if_empty("PixelAspectRatio", "1.000"),
        2 => s.set_if_empty("DisplayAspectRatio", "1.333"),
        3 => s.set_if_empty("DisplayAspectRatio", "1.778"),
        _ => {}
    }
    if let Some(f) = frame_rate(h.frame_rate_code) {
        s.set_if_empty("FrameRate", format!("{f:.3}"));
    }
    s.set_if_empty("ColorSpace", if h.alpha_channel_type > 0 { "YUVA" } else { "YUV" });
    s.set_if_empty("ChromaSubsampling", if h.chroma_format == 3 { "4:4:4" } else { "4:2:2" });
    match h.interlace_mode {
        0 => s.set_if_empty("ScanType", "Progressive"),
        1 => {
            s.set_if_empty("ScanType", "Interlaced");
            s.set_if_empty("ScanOrder", "TFF");
        }
        2 => {
            s.set_if_empty("ScanType", "Interlaced");
            s.set_if_empty("ScanOrder", "BFF");
        }
        _ => {}
    }
    s.set_if_empty("Compression_Mode", "Lossy");
    let enc = String::from_utf8_lossy(&h.encoder_id).into_owned();
    if h.encoder_id.iter().all(|c| c.is_ascii_graphic()) {
        s.set_if_empty("Encoded_Library", enc);
    }
    super::colour::set_description(s, h.colour.0, h.colour.1, h.colour.2, super::colour::STREAM);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;

    fn frame() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend(1000u32.to_be_bytes());
        d.extend(b"icpf");
        d.extend(148u16.to_be_bytes());
        d.push(0);
        d.push(1); // version
        d.extend(b"Lavc");
        d.extend(1920u16.to_be_bytes());
        d.extend(1080u16.to_be_bytes());
        d.push((2 << 6) | (1 << 2)); // 4:2:2, interlaced TFF
        d.push((3 << 4) | 3); // 16:9, 25 fps
        d.extend([1, 1, 1]);
        d.push(0);
        d.extend([0u8; 10]);
        d
    }

    #[test]
    fn header() {
        let d = frame();
        let h = parse_frame(&d).unwrap();
        assert_eq!((h.width, h.height), (1920, 1080));
        assert_eq!(h.chroma_format, 2);
        assert_eq!(h.interlace_mode, 1);
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_frame(&mut s, &d));
        assert_eq!(s.get("Format"), "ProRes");
        assert_eq!(s.get("ChromaSubsampling"), "4:2:2");
        assert_eq!(s.get("ScanType"), "Interlaced");
        assert_eq!(s.get("ScanOrder"), "TFF");
        assert_eq!(s.get("DisplayAspectRatio"), "1.778");
        assert_eq!(s.get("FrameRate"), "25.000");
        assert_eq!(s.get("Encoded_Library"), "Lavc");
        assert_eq!(s.get("colour_primaries"), "BT.709");
        assert!(parse_frame(&d[..20]).is_none());
        assert!(parse_frame(b"xxxxicpf").is_none());
    }
}

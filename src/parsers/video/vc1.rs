//! VC-1 sequence header (SMPTE 421M §6.1 and Annex J): Advanced profile (`00 00 01 0F` start code
//! or bare) and the Simple/Main profile `STRUCT_C` found in WMV3 codec private data.

use crate::io::bits::BitReader;
use crate::model::Stream;

#[derive(Debug, Clone, Default)]
pub struct SequenceHeader {
    pub profile: u8, // 0 simple, 1 main, 3 advanced
    pub level: u8,
    pub width: u32,
    pub height: u32,
    pub interlace: bool,
    pub pulldown: bool,
    pub max_b_frames: u8,
    pub frame_rate: Option<f64>,
    pub par: Option<(u32, u32)>,
    pub colour: Option<(u8, u8, u8)>,
}

fn aspect_ratio(code: u8) -> Option<(u32, u32)> {
    Some(match code {
        1 => (1, 1),
        2 => (12, 11),
        3 => (10, 11),
        4 => (16, 11),
        5 => (40, 33),
        6 => (24, 11),
        7 => (20, 11),
        8 => (32, 11),
        9 => (80, 33),
        10 => (18, 11),
        11 => (15, 11),
        12 => (64, 33),
        13 => (160, 99),
        _ => return None,
    })
}

/// Advanced profile sequence header payload (after the start code).
pub fn parse_advanced(d: &[u8]) -> Option<SequenceHeader> {
    let mut r = BitReader::new(d);
    let mut h = SequenceHeader { profile: r.u8(2)?, ..Default::default() };
    if h.profile != 3 {
        return None;
    }
    h.level = r.u8(3)?;
    r.skip(2); // COLORDIFF_FORMAT
    r.skip(3 + 5 + 1); // FRMRTQ_POSTPROC, BITRTQ_POSTPROC, POSTPROCFLAG
    h.width = (r.u32(12)? + 1) * 2;
    h.height = (r.u32(12)? + 1) * 2;
    h.pulldown = r.bit()?;
    h.interlace = r.bit()?;
    r.skip(1 + 1 + 1 + 1); // TFCNTRFLAG, FINTERPFLAG, RESERVED, PSF
    if r.bit()? {
        // DISPLAY_EXT
        r.skip(14 + 14);
        if r.bit()? {
            let code = r.u8(4)?;
            h.par = if code == 15 { Some((r.u32(8)?, r.u32(8)?)) } else { aspect_ratio(code) };
        }
        if r.bit()? {
            if !r.bit()? {
                let nr = r.u8(8)?;
                let dr = r.u8(4)?;
                let num = match nr {
                    1 => 24.0,
                    2 => 25.0,
                    3 => 30.0,
                    4 => 50.0,
                    5 => 60.0,
                    6 => 48.0,
                    7 => 72.0,
                    _ => 0.0,
                };
                let den = match dr {
                    1 => 1.0,
                    2 => 1.001,
                    _ => 0.0,
                };
                if num > 0.0 && den > 0.0 {
                    h.frame_rate = Some(num / den);
                }
            } else {
                let exp = r.u32(16)?;
                h.frame_rate = Some((exp + 1) as f64 / 32.0);
            }
        }
        if r.bit()? {
            h.colour = Some((r.u8(8)?, r.u8(8)?, r.u8(8)?));
        }
    }
    Some(h)
}

/// Simple/Main profile `STRUCT_C` (4 bytes).
pub fn parse_struct_c(d: &[u8]) -> Option<SequenceHeader> {
    let mut r = BitReader::new(d.get(..4)?);
    let mut h = SequenceHeader { profile: r.u8(2)?, ..Default::default() };
    if h.profile > 1 {
        return None;
    }
    r.skip(3 + 5); // FRMRTQ_POSTPROC, BITRTQ_POSTPROC
    r.skip(1 + 1 + 1 + 1 + 1 + 1); // LOOPFILTER, RESERVED, MULTIRES, RESERVED, FASTUVMC, EXTENDED_MV
    r.skip(2 + 1 + 1 + 1 + 1 + 1); // DQUANT, VSTRANSFORM, RESERVED, OVERLAP, SYNCMARKER, RANGERED
    h.max_b_frames = r.u8(3)?;
    Some(h)
}

/// Find and parse a sequence header in codec private data or a frame.
pub fn parse_sequence(d: &[u8]) -> Option<SequenceHeader> {
    let limit = d.len().min(1 << 16);
    if let Some(p) = d[..limit].windows(4).position(|w| w == [0, 0, 1, 0x0F]) {
        return parse_advanced(&d[p + 4..]);
    }
    match d.first().map(|b| b >> 6) {
        Some(3) => parse_advanced(d),
        Some(0) | Some(1) => parse_struct_c(d),
        _ => None,
    }
}

/// Fill a video stream from a sequence header.
pub fn apply_sequence(s: &mut Stream, d: &[u8]) -> bool {
    let Some(h) = parse_sequence(d) else { return false };
    s.set_if_empty("Format", "VC-1");
    let profile = match h.profile {
        0 => "Simple",
        1 => "Main",
        _ => "Advanced",
    };
    if h.profile == 3 {
        s.set_if_empty("Format_Profile", format!("{profile}@L{}", h.level));
    } else {
        s.set_if_empty("Format_Profile", profile);
        s.set_bool("Format_Settings_BVOP", h.max_b_frames > 0);
    }
    if h.width > 0 && h.height > 0 {
        s.set_if_empty("Width", h.width.to_string());
        s.set_if_empty("Height", h.height.to_string());
    }
    if let Some((n, dn)) = h.par {
        if n > 0 && dn > 0 && !s.has("PixelAspectRatio") && !s.has("DisplayAspectRatio") {
            s.set("PixelAspectRatio", format!("{:.3}", n as f64 / dn as f64));
        }
    }
    if let Some(f) = h.frame_rate {
        s.set_if_empty("FrameRate", format!("{f:.3}"));
    }
    s.set_if_empty("ColorSpace", "YUV");
    s.set_if_empty("ChromaSubsampling", "4:2:0");
    s.set_if_empty("BitDepth", "8");
    if h.profile == 3 {
        s.set_if_empty("ScanType", if h.interlace { "Interlaced" } else { "Progressive" });
    }
    s.set_if_empty("Compression_Mode", "Lossy");
    if let Some((p, t, m)) = h.colour {
        super::colour::set_description(s, p, t, m, super::colour::STREAM);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;
    use crate::parsers::video::testutil::BitWriter;

    fn advanced() -> Vec<u8> {
        let mut b = BitWriter::new();
        b.b(3, 2); // advanced
        b.b(3, 3); // level 3
        b.b(1, 2);
        b.b(0, 9);
        b.b(1920 / 2 - 1, 12);
        b.b(1080 / 2 - 1, 12);
        b.b(0, 1); // pulldown
        b.b(1, 1); // interlace
        b.b(0, 4);
        b.b(1, 1); // display ext
        b.b(1920, 14);
        b.b(1080, 14);
        b.b(1, 1);
        b.b(1, 4); // 1:1
        b.b(1, 1); // frame rate flag
        b.b(0, 1); // FRAMERATEIND = 0
        b.b(2, 8); // 25
        b.b(1, 4); // /1
        b.b(1, 1); // colour
        b.b(1, 8);
        b.b(1, 8);
        b.b(1, 8);
        b.b(0, 1); // hrd
        let mut d = vec![0, 0, 1, 0x0F];
        d.extend(b.done());
        d
    }

    #[test]
    fn advanced_profile() {
        let d = advanced();
        let h = parse_sequence(&d).unwrap();
        assert_eq!((h.profile, h.level), (3, 3));
        assert_eq!((h.width, h.height), (1920, 1080));
        assert!(h.interlace);
        assert_eq!(h.frame_rate, Some(25.0));
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_sequence(&mut s, &d));
        assert_eq!(s.get("Format_Profile"), "Advanced@L3");
        assert_eq!(s.get("ScanType"), "Interlaced");
        assert_eq!(s.get("FrameRate"), "25.000");
        assert_eq!(s.get("colour_primaries"), "BT.709");
        // bare header (no start code) also works
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_sequence(&mut s, &d[4..]));
        assert_eq!(s.get("Width"), "1920");
        for n in 0..d.len() {
            let _ = parse_sequence(&d[..n]);
        }
    }

    #[test]
    fn struct_c() {
        // Main profile, MAXBFRAMES = 2
        let mut b = BitWriter::new();
        b.b(1, 2);
        b.b(0, 8);
        b.b(0, 6);
        b.b(0, 7);
        b.b(2, 3);
        b.b(0, 2);
        b.b(0, 2);
        let d = b.done();
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_sequence(&mut s, &d));
        assert_eq!(s.get("Format_Profile"), "Main");
        assert_eq!(s.get("Format_Settings_BVOP"), "Yes");
        assert!(!apply_sequence(&mut s, &[0x80, 0, 0, 0]));
    }
}

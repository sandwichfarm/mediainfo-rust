//! MPEG-1/2 Video (ISO/IEC 11172-2, ISO/IEC 13818-2): sequence header and extensions, GOP header,
//! picture headers; shared by containers and used by the `.m1v/.m2v/.mpv` elementary parser.

use crate::io::bits::BitReader;
use crate::io::Reader;
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

pub const SC_PICTURE: u8 = 0x00;
pub const SC_USER_DATA: u8 = 0xB2;
pub const SC_SEQUENCE: u8 = 0xB3;
pub const SC_EXTENSION: u8 = 0xB5;
pub const SC_SEQUENCE_END: u8 = 0xB7;
pub const SC_GOP: u8 = 0xB8;

#[derive(Debug, Clone, Default)]
pub struct SequenceHeader {
    pub width: u32,
    pub height: u32,
    pub aspect_ratio_information: u8,
    pub frame_rate_code: u8,
    pub bit_rate_value: u32, // in 400 bps units; 0x3FFFF = variable
    pub vbv_buffer_size_value: u32, // in 16 KiB units
    pub constrained_parameters: bool,
    pub custom_intra_matrix: bool,
    pub custom_non_intra_matrix: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SequenceExtension {
    pub profile_and_level: u8,
    pub progressive_sequence: bool,
    pub chroma_format: u8,
    pub horizontal_size_extension: u32,
    pub vertical_size_extension: u32,
    pub bit_rate_extension: u32,
    pub vbv_buffer_size_extension: u32,
    pub low_delay: bool,
    pub frame_rate_extension_n: u32,
    pub frame_rate_extension_d: u32,
}

#[derive(Debug, Clone, Default)]
pub struct DisplayExtension {
    pub video_format: u8,
    pub colour: Option<(u8, u8, u8)>,
    pub display_width: u32,
    pub display_height: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Gop {
    pub drop_frame: bool,
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub pictures: u8,
    pub closed_gop: bool,
    pub broken_link: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PictureExtension {
    pub intra_dc_precision: u8,
    pub picture_structure: u8,
    pub top_field_first: bool,
    pub frame_pred_frame_dct: bool,
    pub progressive_frame: bool,
    pub repeat_first_field: bool,
}

/// Everything found in a start-code stream.
#[derive(Debug, Clone, Default)]
pub struct Headers {
    pub sequence: Option<SequenceHeader>,
    pub extension: Option<SequenceExtension>,
    pub display: Option<DisplayExtension>,
    pub gop: Option<Gop>,
    pub gop_count: u32,
    pub picture_ext: Option<PictureExtension>,
    /// picture_coding_type of each picture header (1 I, 2 P, 3 B) in stream order.
    pub picture_types: Vec<u8>,
}

pub fn parse_sequence_header(d: &[u8]) -> Option<SequenceHeader> {
    let mut r = BitReader::new(d);
    let mut h = SequenceHeader { width: r.u32(12)?, height: r.u32(12)?, aspect_ratio_information: r.u8(4)?, frame_rate_code: r.u8(4)?, bit_rate_value: r.u32(18)?, ..Default::default() };
    r.bit()?; // marker
    h.vbv_buffer_size_value = r.u32(10)?;
    h.constrained_parameters = r.bit()?;
    h.custom_intra_matrix = r.bit()?;
    if h.custom_intra_matrix {
        r.skip(64 * 8);
    }
    h.custom_non_intra_matrix = r.bit()?;
    if h.width == 0 || h.height == 0 || h.frame_rate_code == 0 {
        return None;
    }
    Some(h)
}

pub fn parse_sequence_extension(d: &[u8]) -> Option<SequenceExtension> {
    let mut r = BitReader::new(d);
    if r.u8(4)? != 1 {
        return None;
    }
    let mut e = SequenceExtension { profile_and_level: r.u8(8)?, progressive_sequence: r.bit()?, chroma_format: r.u8(2)?, horizontal_size_extension: r.u32(2)?, vertical_size_extension: r.u32(2)?, bit_rate_extension: r.u32(12)?, ..Default::default() };
    r.bit()?; // marker
    e.vbv_buffer_size_extension = r.u32(8)?;
    e.low_delay = r.bit()?;
    e.frame_rate_extension_n = r.u32(2)?;
    e.frame_rate_extension_d = r.u32(5)?;
    Some(e)
}

pub fn parse_display_extension(d: &[u8]) -> Option<DisplayExtension> {
    let mut r = BitReader::new(d);
    if r.u8(4)? != 2 {
        return None;
    }
    let mut e = DisplayExtension { video_format: r.u8(3)?, ..Default::default() };
    if r.bit()? {
        e.colour = Some((r.u8(8)?, r.u8(8)?, r.u8(8)?));
    }
    e.display_width = r.u32(14)?;
    r.bit()?;
    e.display_height = r.u32(14)?;
    Some(e)
}

pub fn parse_gop(d: &[u8]) -> Option<Gop> {
    let mut r = BitReader::new(d);
    let mut g = Gop { drop_frame: r.bit()?, hours: r.u8(5)?, minutes: r.u8(6)?, ..Default::default() };
    r.bit()?; // marker
    g.seconds = r.u8(6)?;
    g.pictures = r.u8(6)?;
    g.closed_gop = r.bit()?;
    g.broken_link = r.bit()?;
    Some(g)
}

pub fn parse_picture_extension(d: &[u8]) -> Option<PictureExtension> {
    let mut r = BitReader::new(d);
    if r.u8(4)? != 8 {
        return None;
    }
    r.skip(16); // f_code
    let mut e = PictureExtension { intra_dc_precision: r.u8(2)?, picture_structure: r.u8(2)?, top_field_first: r.bit()?, frame_pred_frame_dct: r.bit()?, ..Default::default() };
    r.skip(4); // concealment_motion_vectors, q_scale_type, intra_vlc_format, alternate_scan
    e.repeat_first_field = r.bit()?;
    r.bit()?; // chroma_420_type
    e.progressive_frame = r.bit()?;
    Some(e)
}

/// Positions of `00 00 01 xx` start codes: (offset of the code byte + 1, code).
pub fn start_codes(d: &[u8]) -> Vec<(usize, u8)> {
    let mut v = Vec::new();
    let mut i = 0;
    while i + 4 <= d.len() && v.len() < 1 << 20 {
        if d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 1 {
            v.push((i + 4, d[i + 3]));
            i += 4;
        } else {
            i += 1;
        }
    }
    v
}

/// Scan a start-code stream for all headers. `max_pictures` caps the picture type list.
pub fn scan(d: &[u8], max_pictures: usize) -> Headers {
    let mut h = Headers::default();
    let codes = start_codes(d);
    for (k, &(pos, code)) in codes.iter().enumerate() {
        let end = codes.get(k + 1).map(|(p, _)| p - 4).unwrap_or(d.len());
        let body = &d[pos..end.max(pos)];
        match code {
            SC_SEQUENCE => {
                if h.sequence.is_none() {
                    h.sequence = parse_sequence_header(body);
                }
            }
            SC_EXTENSION => match body.first().map(|b| b >> 4) {
                Some(1) if h.extension.is_none() => h.extension = parse_sequence_extension(body),
                Some(2) if h.display.is_none() => h.display = parse_display_extension(body),
                Some(8) if h.picture_ext.is_none() => h.picture_ext = parse_picture_extension(body),
                _ => {}
            },
            SC_GOP => {
                h.gop_count += 1;
                if h.gop.is_none() {
                    h.gop = parse_gop(body);
                }
            }
            SC_PICTURE => {
                if h.picture_types.len() < max_pictures {
                    if let Some(t) = body.get(1).map(|b| (b >> 3) & 7) {
                        h.picture_types.push(t);
                    }
                }
            }
            _ => {}
        }
    }
    h
}

pub fn frame_rate(code: u8) -> Option<(u32, u32)> {
    Some(match code {
        1 => (24000, 1001),
        2 => (24, 1),
        3 => (25, 1),
        4 => (30000, 1001),
        5 => (30, 1),
        6 => (50, 1),
        7 => (60000, 1001),
        8 => (60, 1),
        _ => return None,
    })
}

/// MPEG-1 pel_aspect_ratio table (height/width of a pel).
fn mpeg1_pel_aspect(code: u8) -> Option<f64> {
    Some(match code {
        1 => 1.0,
        2 => 0.6735,
        3 => 0.7031,
        4 => 0.7615,
        5 => 0.8055,
        6 => 0.8437,
        7 => 0.8935,
        8 => 0.9157,
        9 => 0.9815,
        10 => 1.0255,
        11 => 1.0695,
        12 => 1.0950,
        13 => 1.1575,
        14 => 1.2015,
        _ => return None,
    })
}

pub fn profile_level_name(v: u8) -> String {
    if v & 0x80 != 0 {
        return match v {
            0x82 => "4:2:2@High",
            0x85 => "4:2:2@Main",
            0x8A => "Multi-view@High",
            0x8B => "Multi-view@High 1440",
            0x8D => "Multi-view@Main",
            0x8E => "Multi-view@Low",
            _ => "",
        }
        .to_string();
    }
    let profile = match (v >> 4) & 7 {
        1 => "High",
        2 => "Spatial",
        3 => "SNR",
        4 => "Main",
        5 => "Simple",
        _ => "",
    };
    let level = match v & 0xF {
        4 => "High",
        6 => "High 1440",
        8 => "Main",
        10 => "Low",
        _ => "",
    };
    if profile.is_empty() || level.is_empty() {
        String::new()
    } else {
        format!("{profile}@{level}")
    }
}

/// Fill a video stream from scanned headers. `in_container`: the stream's own delay/timecode go
/// to the `_Original` fields because the container provides `Delay`.
pub fn apply(s: &mut Stream, h: &Headers, in_container: bool) -> bool {
    let Some(seq) = &h.sequence else { return false };
    let ext = h.extension.as_ref();
    s.set_if_empty("Format", "MPEG Video");
    s.set("Format_Version", if ext.is_some() { "Version 2" } else { "Version 1" });
    if let Some(e) = ext {
        let p = profile_level_name(e.profile_and_level);
        if !p.is_empty() {
            s.set("Format_Profile", p);
        }
    }
    s.set_bool("Format_Settings_BVOP", h.picture_types.contains(&3));
    s.set("Format_Settings_Matrix", if seq.custom_intra_matrix || seq.custom_non_intra_matrix { "Custom" } else { "Default" });
    // The reference reports the GOP structure only for streams with B pictures (raw.m2v, an
    // I/P-only stream with two full GOPs, shows none).
    if h.gop_count >= 2 {
        if let Some((m, n)) = gop_structure(&h.picture_types).filter(|(m, _)| *m > 1) {
            s.set("Format_Settings_GOP", format!("M={m}, N={n}"));
        }
    }
    let width = seq.width | ext.map_or(0, |e| e.horizontal_size_extension << 12);
    let height = seq.height | ext.map_or(0, |e| e.vertical_size_extension << 12);
    s.set_if_empty("Width", width.to_string());
    s.set_if_empty("Height", height.to_string());
    if !s.has("PixelAspectRatio") && !s.has("DisplayAspectRatio") {
        if ext.is_some() {
            match seq.aspect_ratio_information {
                1 => s.set("PixelAspectRatio", "1.000"),
                2 => s.set("DisplayAspectRatio", "1.333"),
                3 => s.set("DisplayAspectRatio", "1.778"),
                4 => s.set("DisplayAspectRatio", "2.210"),
                _ => {}
            }
        } else if let Some(par) = mpeg1_pel_aspect(seq.aspect_ratio_information) {
            s.set("PixelAspectRatio", format!("{:.3}", 1.0 / par));
        }
    }
    let mut fps = None;
    if let Some((n, d)) = frame_rate(seq.frame_rate_code) {
        let (en, ed) = ext.map_or((0, 0), |e| (e.frame_rate_extension_n, e.frame_rate_extension_d));
        let f = n as f64 * (en + 1) as f64 / (d as f64 * (ed + 1) as f64);
        s.set_if_empty("FrameRate", format!("{f:.3}"));
        fps = Some(f);
    }
    if let (Some(f), true) = (fps, [576, 288].contains(&height) && width <= 768) {
        if (f - 25.0).abs() < 0.01 {
            s.set_if_empty("Standard", "PAL");
        }
    }
    if let (Some(f), true) = (fps, [480, 240].contains(&height) && width <= 768) {
        if (f - 29.97).abs() < 0.01 {
            s.set_if_empty("Standard", "NTSC");
        }
    }
    let bit_rate = seq.bit_rate_value | ext.map_or(0, |e| e.bit_rate_extension << 18);
    if seq.bit_rate_value == 0x3FFFF {
        s.set_if_empty("BitRate_Mode", "VBR");
    } else if bit_rate > 0 {
        s.set_if_empty("BitRate_Mode", "CBR");
        s.set_if_empty("BitRate", (bit_rate as u64 * 400).to_string());
    }
    let vbv = seq.vbv_buffer_size_value | ext.map_or(0, |e| e.vbv_buffer_size_extension << 10);
    if vbv > 0 {
        s.set_if_empty("BufferSize", (vbv as u64 * 16 * 1024 / 8).to_string());
    }
    s.set_if_empty("ColorSpace", "YUV");
    s.set_if_empty(
        "ChromaSubsampling",
        match ext.map_or(1, |e| e.chroma_format) {
            2 => "4:2:2",
            3 => "4:4:4",
            _ => "4:2:0",
        },
    );
    s.set_if_empty("BitDepth", "8");
    match ext {
        Some(e) if !e.progressive_sequence => {
            s.set_if_empty("ScanType", "Interlaced");
            if let Some(p) = &h.picture_ext {
                if p.picture_structure == 3 {
                    s.set_if_empty("ScanOrder", if p.top_field_first { "TFF" } else { "BFF" });
                }
            }
        }
        _ => s.set_if_empty("ScanType", "Progressive"),
    }
    s.set_if_empty("Compression_Mode", "Lossy");
    if let Some(d) = &h.display {
        if let Some((p, t, m)) = d.colour {
            super::colour::set_description(s, p, t, m, super::colour::STREAM);
        }
    }
    if let Some(g) = &h.gop {
        let prefix = if in_container { "Delay_Original" } else { "Delay" };
        let mut ms = (g.hours as f64 * 3600.0 + g.minutes as f64 * 60.0 + g.seconds as f64) * 1000.0;
        if let Some(f) = fps {
            ms += g.pictures as f64 * 1000.0 / f;
        }
        s.set(prefix, format!("{}", ms.round() as i64));
        s.set(&format!("{prefix}_Settings"), format!("drop_frame_flag={} / closed_gop={} / broken_link={}", g.drop_frame as u8, g.closed_gop as u8, g.broken_link as u8));
        s.set(&format!("{prefix}_DropFrame"), if g.drop_frame { "Yes" } else { "No" });
        s.set(&format!("{prefix}_Source"), "Stream");
        s.set("TimeCode_FirstFrame", format!("{:02}:{:02}:{:02}{}{:02}", g.hours, g.minutes, g.seconds, if g.drop_frame { ";" } else { ":" }, g.pictures));
        if in_container {
            s.set("TimeCode_Source", "Group of pictures header");
        }
    }
    if let Some(p) = &h.picture_ext {
        s.set_extra("intra_dc_precision", (8 + p.intra_dc_precision).to_string(), "", "N NT");
    }
    true
}

/// (M, N) from the picture types of the first GOP: distance between reference pictures and GOP
/// length in pictures.
pub fn gop_structure(types: &[u8]) -> Option<(usize, usize)> {
    let first_i = types.iter().position(|t| *t == 1)?;
    let rest = &types[first_i + 1..];
    let n = rest.iter().position(|t| *t == 1).map(|p| p + 1)?;
    let m = rest.iter().position(|t| *t != 3).map(|p| p + 1).unwrap_or(1).min(n);
    Some((m, n))
}

/// Fill a stream from a start-code stream (CodecPrivate or first frame) inside a container.
pub fn apply_headers(s: &mut Stream, d: &[u8]) -> bool {
    let h = scan(d, 1024);
    apply(s, &h, true)
}

// ---- elementary stream

pub fn probe(p: &Probe) -> u8 {
    let head = &p.head[..p.head.len().min(8192)];
    let starts_seq = p.starts_with(&[0, 0, 1, SC_SEQUENCE]);
    let ext = p.ext_in(&["m2v", "m1v", "mpv", "mpgv", "mpeg", "mpg", "mpv2", "m2v1"]);
    if !starts_seq {
        return 0;
    }
    let codes = start_codes(head);
    let seq_ok = parse_sequence_header(head.get(4..).unwrap_or(&[])).is_some();
    let has_picture = codes.iter().any(|(_, c)| *c == SC_PICTURE);
    let has_gop_or_ext = codes.iter().any(|(_, c)| *c == SC_GOP || *c == SC_EXTENSION);
    if seq_ok && has_picture && (has_gop_or_ext || ext) {
        if ext { 95 } else { 60 }
    } else {
        0
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let size = r.len();
    let cap = size.min(8 * 1024 * 1024) as usize;
    let data = r.read_vec_at(0, cap);
    let h = scan(&data, 1 << 20);
    let mut s = Stream::new(StreamKind::Video);
    if !apply(&mut s, &h, false) {
        return false;
    }
    let mut frames = h.picture_types.len() as u64;
    if cap < size as usize && frames > 0 {
        frames = (frames as f64 * size as f64 / cap as f64).round() as u64;
    }
    if frames > 0 {
        s.set("FrameCount", frames.to_string());
        if let Some(fps) = s.get_f64("FrameRate").filter(|f| *f > 0.0) {
            let dur = frames as f64 / fps * 1000.0;
            s.set("Duration", format!("{}", dur.round() as u64));
            if !s.has("BitRate") && dur > 0.0 {
                s.set("BitRate", format!("{}", (size as f64 * 8.0 * 1000.0 / dur).round() as u64));
            }
        }
    }
    s.set("StreamSize", size.to_string());
    let g = doc.general();
    g.set("Format", "MPEG Video");
    g.set("Format_Version", s.get("Format_Version").to_string());
    g.set("StreamSize", "0");
    if s.has("Duration") {
        g.set("Duration", s.get("Duration").to_string());
    }
    if s.has("BitRate_Mode") {
        g.set("OverallBitRate_Mode", s.get("BitRate_Mode").to_string());
    }
    doc.streams[StreamKind::Video as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::video::testutil::BitWriter;

    fn seq_header(w: u32, h: u32, aspect: u8, rate: u8, bit_rate: u32, vbv: u32) -> Vec<u8> {
        let mut b = BitWriter::new();
        b.b(w as u64, 12);
        b.b(h as u64, 12);
        b.b(aspect as u64, 4);
        b.b(rate as u64, 4);
        b.b(bit_rate as u64, 18);
        b.b(1, 1);
        b.b(vbv as u64, 10);
        b.b(0, 1);
        b.b(0, 1);
        b.b(0, 1);
        b.done()
    }

    fn stream(mpeg2: bool, pictures: &[u8], gops: usize) -> Vec<u8> {
        let mut d = vec![0, 0, 1, SC_SEQUENCE];
        d.extend(seq_header(64, 48, if mpeg2 { 2 } else { 1 }, 3, 0x3FFFF, 3));
        if mpeg2 {
            let mut b = BitWriter::new();
            b.b(1, 4); // sequence extension
            b.b(0x48, 8); // Main@Main
            b.b(1, 1); // progressive
            b.b(1, 2); // 4:2:0
            b.b(0, 2);
            b.b(0, 2);
            b.b(0, 12);
            b.b(1, 1);
            b.b(0, 8);
            b.b(0, 1);
            b.b(0, 2);
            b.b(0, 5);
            d.extend([0, 0, 1, SC_EXTENSION]);
            d.extend(b.done());
            let mut b = BitWriter::new();
            b.b(2, 4); // display extension
            b.b(5, 3);
            b.b(1, 1);
            b.b(1, 8);
            b.b(1, 8);
            b.b(1, 8);
            b.b(64, 14);
            b.b(1, 1);
            b.b(48, 14);
            d.extend([0, 0, 1, SC_EXTENSION]);
            d.extend(b.done());
        }
        for g in 0..gops {
            let mut b = BitWriter::new();
            b.b(0, 1); // drop frame
            b.b(0, 5);
            b.b(0, 6);
            b.b(1, 1);
            b.b(g as u64, 6); // seconds
            b.b(0, 6);
            b.b(1, 1); // closed
            b.b(0, 1);
            d.extend([0, 0, 1, SC_GOP]);
            d.extend(b.done());
            for &t in pictures {
                let mut b = BitWriter::new();
                b.b(0, 10);
                b.b(t as u64, 3);
                b.b(0xFFFF, 16);
                d.extend([0, 0, 1, SC_PICTURE]);
                d.extend(b.done());
                if mpeg2 {
                    let mut b = BitWriter::new();
                    b.b(8, 4);
                    b.b(0xFFFF, 16);
                    b.b(0, 2); // intra_dc_precision
                    b.b(3, 2); // frame picture
                    b.b(1, 1); // tff
                    b.b(1, 1);
                    b.b(0, 4);
                    b.b(0, 1);
                    b.b(0, 1);
                    b.b(1, 1);
                    b.b(0, 1);
                    d.extend([0, 0, 1, SC_EXTENSION]);
                    d.extend(b.done());
                }
                d.extend([0, 0, 1, 1, 0xAB, 0xCD]); // a slice
            }
        }
        d
    }

    #[test]
    fn mpeg2_headers() {
        let d = stream(true, &[1, 3, 2, 3, 2, 3], 2);
        let h = scan(&d, 100);
        let seq = h.sequence.as_ref().unwrap();
        assert_eq!((seq.width, seq.height), (64, 48));
        assert_eq!(seq.bit_rate_value, 0x3FFFF);
        assert_eq!(h.extension.as_ref().unwrap().profile_and_level, 0x48);
        assert_eq!(h.gop_count, 2);
        assert_eq!(h.picture_types.len(), 12);
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply(&mut s, &h, false));
        assert_eq!(s.get("Format_Version"), "Version 2");
        assert_eq!(s.get("Format_Profile"), "Main@Main");
        assert_eq!(s.get("Format_Settings_BVOP"), "Yes");
        assert_eq!(s.get("Format_Settings_GOP"), "M=2, N=6");
        assert_eq!(gop_structure(&[1, 2, 2, 1]), Some((1, 3)));
        assert_eq!(gop_structure(&[2, 2]), None);
        assert_eq!(s.get("Format_Settings_Matrix"), "Default");
        assert_eq!(s.get("DisplayAspectRatio"), "1.333");
        assert_eq!(s.get("FrameRate"), "25.000");
        assert_eq!(s.get("BitRate_Mode"), "VBR");
        assert_eq!(s.get("BufferSize"), "6144");
        assert_eq!(s.get("ScanType"), "Progressive");
        assert_eq!(s.get("Delay"), "0");
        assert_eq!(s.get("Delay_Settings"), "drop_frame_flag=0 / closed_gop=1 / broken_link=0");
        assert_eq!(s.get("TimeCode_FirstFrame"), "00:00:00:00");
        assert_eq!(s.get("colour_primaries"), "BT.709");
        assert_eq!(s.get("intra_dc_precision"), "8");
        // In a container the stream delay is the "original" one.
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_headers(&mut s, &d));
        assert_eq!(s.get("Delay_Original"), "0");
        assert_eq!(s.get("Delay"), "");
        assert_eq!(s.get("TimeCode_Source"), "Group of pictures header");
    }

    #[test]
    fn mpeg1_headers_and_probe() {
        let d = stream(false, &[1, 2, 2], 1);
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_headers(&mut s, &d));
        assert_eq!(s.get("Format_Version"), "Version 1");
        assert_eq!(s.get("Format_Profile"), "");
        assert_eq!(s.get("PixelAspectRatio"), "1.000");
        assert_eq!(s.get("Format_Settings_BVOP"), "No");
        assert_eq!(s.get("Format_Settings_GOP"), "");
        let p = Probe { head: &d, ext: "m1v", size: d.len() as u64 };
        assert_eq!(probe(&p), 95);
        let p = Probe { head: &d, ext: "bin", size: d.len() as u64 };
        assert_eq!(probe(&p), 60);
        assert_eq!(probe(&Probe { head: &[0, 0, 1, 0xB3, 0, 0], ext: "m2v", size: 6 }), 0);
        let mut r = Reader::from_bytes(d);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("FrameCount"), "3");
        assert_eq!(v.get("Duration"), "120");
        assert_eq!(doc.general_ref().get("Format"), "MPEG Video");
    }

    #[test]
    fn names() {
        assert_eq!(profile_level_name(0x48), "Main@Main");
        assert_eq!(profile_level_name(0x14), "High@High");
        assert_eq!(profile_level_name(0x85), "4:2:2@Main");
        assert_eq!(profile_level_name(0x5A), "Simple@Low");
        assert_eq!(profile_level_name(0x00), "");
        assert!(parse_sequence_header(&[0; 3]).is_none());
        assert!(parse_gop(&[]).is_none());
    }
}

//! MPEG-4 Visual (ISO/IEC 14496-2): visual object sequence / visual object / video object layer
//! headers, user data and VOP scanning; shared by containers and used by the `.m4v` elementary parser.

use crate::io::bits::BitReader;
use crate::io::Reader;
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

pub const SC_VOS: u8 = 0xB0;
pub const SC_VOS_END: u8 = 0xB1;
pub const SC_USER_DATA: u8 = 0xB2;
pub const SC_GOV: u8 = 0xB3;
pub const SC_VISUAL_OBJECT: u8 = 0xB5;
pub const SC_VOP: u8 = 0xB6;

#[derive(Debug, Clone, Default)]
pub struct VisualObject {
    pub video_range: Option<bool>,
    pub colour: Option<(u8, u8, u8)>,
}

#[derive(Debug, Clone, Default)]
pub struct Vol {
    pub video_object_type: u8,
    pub verid: u8,
    pub aspect_ratio_info: u8,
    pub par: Option<(u32, u32)>,
    pub chroma_format: u8,
    pub low_delay: bool,
    pub bit_rate: Option<u32>,
    pub vbv_buffer_size: Option<u32>,
    pub shape: u8,
    pub time_increment_resolution: u32,
    pub fixed_vop_rate: bool,
    pub fixed_vop_time_increment: u32,
    pub width: u32,
    pub height: u32,
    pub interlaced: bool,
    pub sprite_enable: u8,
    pub sprite_warping_points: u8,
    pub bits_per_pixel: u8,
    pub quant_type: bool,
    pub custom_matrix: bool,
    pub quarter_sample: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Headers {
    pub profile_level: Option<u8>,
    pub visual_object: Option<VisualObject>,
    pub vol: Option<Vol>,
    pub user_data: Vec<String>,
    /// vop_coding_type of each VOP (0 I, 1 P, 2 B, 3 S).
    pub vop_types: Vec<u8>,
}

/// Number of bits of vop_time_increment for a resolution (14496-2 6.2.3).
fn time_increment_bits(resolution: u32) -> usize {
    let mut n = 1;
    while (1u64 << n) < resolution as u64 {
        n += 1;
    }
    n
}

pub fn parse_visual_object(d: &[u8]) -> Option<VisualObject> {
    let mut r = BitReader::new(d);
    if r.bit()? {
        r.skip(7); // verid + priority
    }
    let ty = r.u8(4)?;
    let mut v = VisualObject::default();
    if (ty == 1 || ty == 2) && r.bit()? {
        r.skip(3); // video_format
        v.video_range = Some(r.bit()?);
        if r.bit()? {
            v.colour = Some((r.u8(8)?, r.u8(8)?, r.u8(8)?));
        }
    }
    Some(v)
}

pub fn parse_vol(d: &[u8]) -> Option<Vol> {
    let mut r = BitReader::new(d);
    r.bit()?; // random_accessible_vol
    let mut v = Vol { video_object_type: r.u8(8)?, verid: 1, ..Default::default() };
    if r.bit()? {
        v.verid = r.u8(4)?;
        r.skip(3);
    }
    v.aspect_ratio_info = r.u8(4)?;
    v.par = match v.aspect_ratio_info {
        1 => Some((1, 1)),
        2 => Some((12, 11)),
        3 => Some((10, 11)),
        4 => Some((16, 11)),
        5 => Some((40, 33)),
        15 => Some((r.u32(8)?, r.u32(8)?)),
        _ => None,
    };
    v.chroma_format = 1;
    if r.bit()? {
        v.chroma_format = r.u8(2)?;
        v.low_delay = r.bit()?;
        if r.bit()? {
            let br_hi = r.u32(15)?;
            r.bit()?;
            let br_lo = r.u32(15)?;
            r.bit()?;
            let vbv_hi = r.u32(15)?;
            r.bit()?;
            let vbv_lo = r.u32(3)?;
            r.skip(11); // first_half_vbv_occupancy
            r.bit()?;
            r.skip(15);
            r.bit()?;
            v.bit_rate = Some((br_hi << 15) | br_lo);
            v.vbv_buffer_size = Some((vbv_hi << 3) | vbv_lo);
        }
    }
    v.shape = r.u8(2)?;
    if v.shape == 3 && v.verid != 1 {
        r.skip(4);
    }
    r.bit()?;
    v.time_increment_resolution = r.u32(16)?;
    r.bit()?;
    v.fixed_vop_rate = r.bit()?;
    if v.fixed_vop_rate {
        v.fixed_vop_time_increment = r.u32(time_increment_bits(v.time_increment_resolution))?;
    }
    if v.shape == 2 {
        return Some(v); // binary only: nothing more of interest
    }
    if v.shape == 0 {
        r.bit()?;
        v.width = r.u32(13)?;
        r.bit()?;
        v.height = r.u32(13)?;
        r.bit()?;
    }
    v.interlaced = r.bit()?;
    r.bit()?; // obmc_disable
    v.sprite_enable = r.u8(if v.verid == 1 { 1 } else { 2 })?;
    if v.sprite_enable == 1 || v.sprite_enable == 2 {
        if v.sprite_enable == 1 {
            for _ in 0..4 {
                r.skip(13);
                r.bit()?;
            }
        }
        v.sprite_warping_points = r.u8(6)?;
        r.skip(2); // sprite_warping_accuracy
        r.bit()?; // sprite_brightness_change
        if v.sprite_enable == 1 {
            r.bit()?; // low_latency_sprite_enable
        }
    }
    if v.verid != 1 && v.shape != 0 {
        r.bit()?; // sadct_disable
    }
    v.bits_per_pixel = 8;
    if r.bit()? {
        r.skip(4); // quant_precision
        v.bits_per_pixel = r.u8(4)?;
    }
    if v.shape == 3 {
        r.skip(3);
    }
    v.quant_type = r.bit()?;
    if v.quant_type {
        if r.bit()? {
            v.custom_matrix = true;
            skip_matrix(&mut r)?;
        }
        if r.bit()? {
            v.custom_matrix = true;
            skip_matrix(&mut r)?;
        }
    }
    if v.verid != 1 {
        v.quarter_sample = r.bit()?;
    }
    Some(v)
}

/// Skip a quantisation matrix: values until a 0 terminator or 64 entries.
fn skip_matrix(r: &mut BitReader) -> Option<()> {
    for _ in 0..64 {
        if r.u8(8)? == 0 {
            break;
        }
    }
    Some(())
}

/// Positions of `00 00 01 xx` start codes: (offset after the code, code).
pub fn start_codes(d: &[u8]) -> Vec<(usize, u8)> {
    super::mpegv::start_codes(d)
}

/// Scan a start-code stream for the headers, user data and VOP types (capped at `max_vops`).
pub fn scan(d: &[u8], max_vops: usize) -> Headers {
    let mut h = Headers::default();
    let codes = start_codes(d);
    for (k, &(pos, code)) in codes.iter().enumerate() {
        let end = codes.get(k + 1).map(|(p, _)| p - 4).unwrap_or(d.len()).max(pos);
        let body = &d[pos..end];
        match code {
            SC_VOS => {
                if h.profile_level.is_none() {
                    h.profile_level = body.first().copied();
                }
            }
            SC_VISUAL_OBJECT => {
                if h.visual_object.is_none() {
                    h.visual_object = parse_visual_object(body);
                }
            }
            0x20..=0x2F => {
                if h.vol.is_none() {
                    h.vol = parse_vol(body);
                }
            }
            SC_USER_DATA => {
                let text = crate::io::clean_text(&crate::io::cstr(&body[..body.len().min(256)]));
                if !text.is_empty() && h.user_data.len() < 16 {
                    h.user_data.push(text);
                }
            }
            SC_VOP => {
                if h.vop_types.len() < max_vops {
                    if let Some(b) = body.first() {
                        h.vop_types.push(b >> 6);
                    }
                }
            }
            _ => {}
        }
    }
    h
}

/// profile_and_level_indication → "Simple@L1" (14496-2 Table G-1).
pub fn profile_level_name(v: u8) -> String {
    let (profile, level): (&str, &str) = match v {
        0x01 => ("Simple", "1"),
        0x02 => ("Simple", "2"),
        0x03 => ("Simple", "3"),
        0x04 => ("Simple", "4a"),
        0x05 => ("Simple", "5"),
        0x06 => ("Simple", "6"),
        0x08 => ("Simple", "0"),
        0x09 => ("Simple", "0b"),
        0x10 => ("Simple Scalable", "0"),
        0x11 => ("Simple Scalable", "1"),
        0x12 => ("Simple Scalable", "2"),
        0x21 => ("Core", "1"),
        0x22 => ("Core", "2"),
        0x32 => ("Main", "2"),
        0x33 => ("Main", "3"),
        0x34 => ("Main", "4"),
        0x42 => ("N-bit", "2"),
        0x51 => ("Scalable Texture", "1"),
        0x61 => ("Simple Face Animation", "1"),
        0x62 => ("Simple Face Animation", "2"),
        0x63 => ("Simple FBA", "1"),
        0x64 => ("Simple FBA", "2"),
        0x71 => ("Basic Animated Texture", "1"),
        0x72 => ("Basic Animated Texture", "2"),
        0x81 => ("Hybrid", "1"),
        0x82 => ("Hybrid", "2"),
        0x91 => ("Advanced Real Time Simple", "1"),
        0x92 => ("Advanced Real Time Simple", "2"),
        0x93 => ("Advanced Real Time Simple", "3"),
        0x94 => ("Advanced Real Time Simple", "4"),
        0xA1 => ("Core Scalable", "1"),
        0xA2 => ("Core Scalable", "2"),
        0xA3 => ("Core Scalable", "3"),
        0xB1 => ("Advanced Coding Efficiency", "1"),
        0xB2 => ("Advanced Coding Efficiency", "2"),
        0xB3 => ("Advanced Coding Efficiency", "3"),
        0xB4 => ("Advanced Coding Efficiency", "4"),
        0xC1 => ("Advanced Core", "1"),
        0xC2 => ("Advanced Core", "2"),
        0xD1 => ("Advanced Scalable Texture", "1"),
        0xD2 => ("Advanced Scalable Texture", "2"),
        0xD3 => ("Advanced Scalable Texture", "3"),
        0xE1 => ("Simple Studio", "1"),
        0xE2 => ("Simple Studio", "2"),
        0xE3 => ("Simple Studio", "3"),
        0xE4 => ("Simple Studio", "4"),
        0xE5 => ("Core Studio", "1"),
        0xE6 => ("Core Studio", "2"),
        0xE7 => ("Core Studio", "3"),
        0xE8 => ("Core Studio", "4"),
        0xF0 => ("Advanced Simple", "0"),
        0xF1 => ("Advanced Simple", "1"),
        0xF2 => ("Advanced Simple", "2"),
        0xF3 => ("Advanced Simple", "3"),
        0xF4 => ("Advanced Simple", "4"),
        0xF5 => ("Advanced Simple", "5"),
        0xF7 => ("Advanced Simple", "3b"),
        0xF8 => ("Fine Granularity Scalable", "0"),
        0xF9 => ("Fine Granularity Scalable", "1"),
        0xFA => ("Fine Granularity Scalable", "2"),
        0xFB => ("Fine Granularity Scalable", "3"),
        0xFC => ("Fine Granularity Scalable", "4"),
        0xFD => ("Fine Granularity Scalable", "5"),
        _ => return String::new(),
    };
    format!("{profile}@L{level}")
}

/// Encoder name from a user_data string ("Lavc63.1.101", "XviD0050", "DivX503b1393p").
pub fn encoder_from_user_data(text: &str) -> Option<String> {
    let t = text.trim();
    if t.starts_with("Lavc") || t.starts_with("FFmpe") || t.starts_with("XviD") || t.starts_with("DivX") || t.starts_with("x264") || t.starts_with("MEncoder") || t.starts_with("Nero") {
        Some(t.to_string())
    } else {
        None
    }
}

/// Fill a video stream from scanned headers. Returns false when no VOL was found.
pub fn apply(s: &mut Stream, h: &Headers) -> bool {
    let Some(vol) = &h.vol else { return false };
    s.set_if_empty("Format", "MPEG-4 Visual");
    if let Some(p) = h.profile_level.map(profile_level_name).filter(|p| !p.is_empty()) {
        s.set_if_empty("Format_Profile", p);
    }
    s.set_bool("Format_Settings_BVOP", h.vop_types.contains(&2));
    s.set_bool("Format_Settings_QPel", vol.quarter_sample);
    s.set("Format_Settings_GMC", if vol.sprite_enable == 2 { vol.sprite_warping_points } else { 0 }.to_string());
    s.set(
        "Format_Settings_Matrix",
        if !vol.quant_type {
            "Default (H.263)"
        } else if vol.custom_matrix {
            "Custom"
        } else {
            "Default (MPEG)"
        },
    );
    if vol.width > 0 && vol.height > 0 {
        s.set_if_empty("Width", vol.width.to_string());
        s.set_if_empty("Height", vol.height.to_string());
    }
    if let Some((n, d)) = vol.par {
        if n > 0 && d > 0 && !s.has("PixelAspectRatio") && !s.has("DisplayAspectRatio") {
            s.set("PixelAspectRatio", format!("{:.3}", n as f64 / d as f64));
        }
    }
    if vol.fixed_vop_rate && vol.fixed_vop_time_increment > 0 && vol.time_increment_resolution > 0 {
        s.set_if_empty("FrameRate", format!("{:.3}", vol.time_increment_resolution as f64 / vol.fixed_vop_time_increment as f64));
    }
    if let Some(br) = vol.bit_rate.filter(|b| *b > 0) {
        // The reference keeps both the container's and the VOL's maximum bit rate.
        let mine = (br as u64 * 400).to_string();
        let existing = s.get("BitRate_Maximum").to_string();
        if existing.is_empty() {
            s.set("BitRate_Maximum", mine);
        } else if !existing.contains(" / ") {
            s.set("BitRate_Maximum", format!("{existing} / {mine}"));
        }
    }
    s.set_if_empty("ColorSpace", "YUV");
    s.set_if_empty("ChromaSubsampling", match vol.chroma_format { 2 => "4:2:2", 3 => "4:4:4", _ => "4:2:0" });
    s.set_if_empty("BitDepth", vol.bits_per_pixel.max(8).to_string());
    s.set_if_empty("ScanType", if vol.interlaced { "Interlaced" } else { "Progressive" });
    s.set_if_empty("Compression_Mode", "Lossy");
    if let Some(vo) = &h.visual_object {
        if let Some(full) = vo.video_range {
            super::colour::set_range(s, full, super::colour::STREAM);
        }
        if let Some((p, t, m)) = vo.colour {
            super::colour::set_description(s, p, t, m, super::colour::STREAM);
        }
    }
    apply_user_data(s, &h.user_data);
    true
}

fn apply_user_data(s: &mut Stream, user_data: &[String]) {
    if let Some(enc) = user_data.iter().find_map(|u| encoder_from_user_data(u)) {
        s.set("Encoded_Library", enc);
    }
}

/// Fill a stream from a start-code stream (CodecPrivate or first frame).
pub fn apply_headers(s: &mut Stream, d: &[u8]) -> bool {
    apply(s, &scan(d, 4096))
}

/// Encoder string from the user data of a frame, and B-VOP detection from its VOPs.
pub fn apply_frame_user_data(s: &mut Stream, d: &[u8]) {
    let h = scan(d, 4096);
    apply_user_data(s, &h.user_data);
    if h.vop_types.contains(&2) {
        s.set("Format_Settings_BVOP", "Yes");
    }
}

// ---- elementary stream

pub fn probe(p: &Probe) -> u8 {
    let head = &p.head[..p.head.len().min(8192)];
    let starts = p.starts_with(&[0, 0, 1, SC_VOS]) || p.starts_with(&[0, 0, 1, SC_VISUAL_OBJECT]) || (p.starts_with(&[0, 0, 1]) && head.get(3).is_some_and(|c| (0x00..=0x2F).contains(c)));
    if !starts {
        return 0;
    }
    let h = scan(head, 4);
    if h.vol.is_none() || h.vop_types.is_empty() {
        return 0;
    }
    if p.ext_in(&["m4v", "mp4v", "mpeg4", "m4s"]) {
        95
    } else {
        60
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let size = r.len();
    let cap = size.min(8 * 1024 * 1024) as usize;
    let data = r.read_vec_at(0, cap);
    let h = scan(&data, 1 << 20);
    let mut s = Stream::new(StreamKind::Video);
    if !apply(&mut s, &h) {
        return false;
    }
    let mut frames = h.vop_types.len() as u64;
    if cap < size as usize && frames > 0 {
        frames = (frames as f64 * size as f64 / cap as f64).round() as u64;
    }
    if let Some(fps) = s.get_f64("FrameRate").filter(|f| *f > 0.0) {
        if frames > 0 {
            s.set("FrameCount", frames.to_string());
            let dur = frames as f64 / fps * 1000.0;
            s.set("Duration", format!("{}", dur.round() as u64));
            doc.general().set("Duration", format!("{}", dur.round() as u64));
        }
    }
    doc.general().set("Format", "MPEG-4 Visual");
    doc.streams[StreamKind::Video as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::video::testutil::BitWriter;

    fn vol(fixed_rate: bool, qpel: bool, gmc: bool) -> Vec<u8> {
        let mut b = BitWriter::new();
        b.b(0, 1); // random_accessible_vol
        b.b(1, 8); // video_object_type: simple
        b.b(1, 1); // is_object_layer_identifier
        b.b(2, 4); // verid 2
        b.b(1, 3);
        b.b(1, 4); // aspect 1:1
        b.b(1, 1); // vol_control_parameters
        b.b(1, 2); // chroma 4:2:0
        b.b(1, 1); // low_delay
        b.b(0, 1); // vbv
        b.b(0, 2); // rectangular
        b.b(1, 1);
        b.b(25, 16); // time increment resolution
        b.b(1, 1);
        b.b(fixed_rate as u64, 1);
        if fixed_rate {
            b.b(1, 5); // increment 1 → 25 fps
        }
        b.b(1, 1);
        b.b(64, 13);
        b.b(1, 1);
        b.b(48, 13);
        b.b(1, 1);
        b.b(0, 1); // interlaced
        b.b(1, 1); // obmc_disable
        b.b(if gmc { 2 } else { 0 }, 2); // sprite_enable
        if gmc {
            b.b(3, 6); // warping points
            b.b(0, 2);
            b.b(0, 1);
        }
        b.b(0, 1); // not_8_bit
        b.b(0, 1); // quant_type
        b.b(qpel as u64, 1);
        b.b(0, 1); // complexity_estimation_disable
        b.b(1, 1);
        let mut d = vec![0, 0, 1, 0x20];
        d.extend(b.done());
        d
    }

    fn stream(fixed_rate: bool, vops: &[u8]) -> Vec<u8> {
        let mut d = vec![0, 0, 1, SC_VOS, 0x01, 0, 0, 1, SC_VISUAL_OBJECT, 0x89, 0x13, 0, 0, 1, 0x00];
        d.extend(vol(fixed_rate, false, false));
        d.extend([0, 0, 1, SC_USER_DATA]);
        d.extend(b"Lavc63.1.101");
        for &t in vops {
            d.extend([0, 0, 1, SC_VOP, t << 6, 0xAB, 0xCD]);
        }
        d
    }

    #[test]
    fn headers() {
        let d = stream(false, &[0, 1, 1]);
        let h = scan(&d, 100);
        assert_eq!(h.profile_level, Some(1));
        let v = h.vol.as_ref().unwrap();
        assert_eq!((v.width, v.height), (64, 48));
        assert_eq!(v.time_increment_resolution, 25);
        assert!(!v.fixed_vop_rate);
        assert_eq!(h.user_data, vec!["Lavc63.1.101".to_string()]);
        assert_eq!(h.vop_types, vec![0, 1, 1]);
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_headers(&mut s, &d));
        assert_eq!(s.get("Format_Profile"), "Simple@L1");
        assert_eq!(s.get("Format_Settings_BVOP"), "No");
        assert_eq!(s.get("Format_Settings_QPel"), "No");
        assert_eq!(s.get("Format_Settings_GMC"), "0");
        assert_eq!(s.get("Format_Settings_Matrix"), "Default (H.263)");
        assert_eq!(s.get("FrameRate"), "");
        assert_eq!(s.get("PixelAspectRatio"), "1.000");
        assert_eq!(s.get("Encoded_Library"), "Lavc63.1.101");
        assert_eq!(s.get("ScanType"), "Progressive");
        apply_frame_user_data(&mut s, &[0, 0, 1, SC_VOP, 2 << 6, 1]);
        assert_eq!(s.get("Format_Settings_BVOP"), "Yes");
    }

    #[test]
    fn vol_options_and_frame_rate() {
        let d = vol(true, true, true);
        let v = parse_vol(&d[4..]).unwrap();
        assert!(v.fixed_vop_rate);
        assert_eq!(v.fixed_vop_time_increment, 1);
        assert!(v.quarter_sample);
        assert_eq!(v.sprite_warping_points, 3);
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_headers(&mut s, &d));
        assert_eq!(s.get("FrameRate"), "25.000");
        assert_eq!(s.get("Format_Settings_QPel"), "Yes");
        assert_eq!(s.get("Format_Settings_GMC"), "3");
        for n in 0..d.len() {
            let _ = parse_vol(&d[4.min(n)..n]);
        }
    }

    #[test]
    fn elementary() {
        let d = stream(true, &[0, 1, 2, 1, 1]);
        let p = Probe { head: &d, ext: "m4v", size: d.len() as u64 };
        assert_eq!(probe(&p), 95);
        assert_eq!(probe(&Probe { head: &[0, 0, 1, SC_VOS, 1], ext: "m4v", size: 5 }), 0);
        let mut r = Reader::from_bytes(d);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("FrameCount"), "5");
        assert_eq!(v.get("Duration"), "200");
        assert_eq!(v.get("Format_Settings_BVOP"), "Yes");
        assert_eq!(doc.general_ref().get("Format"), "MPEG-4 Visual");
    }

    #[test]
    fn names() {
        assert_eq!(profile_level_name(0x01), "Simple@L1");
        assert_eq!(profile_level_name(0xF5), "Advanced Simple@L5");
        assert_eq!(profile_level_name(0x09), "Simple@L0b");
        assert_eq!(profile_level_name(0x7F), "");
        assert_eq!(time_increment_bits(25), 5);
        assert_eq!(time_increment_bits(1), 1);
        assert_eq!(time_increment_bits(30000), 15);
    }
}

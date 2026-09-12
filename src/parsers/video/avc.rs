//! H.264 / AVC: SPS/PPS/SEI parsing (shared by containers) and the Annex B elementary stream parser.

use crate::io::bits::{unescape_rbsp, BitReader};
use crate::io::Reader;
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

#[derive(Debug, Clone, Default)]
pub struct Vui {
    pub sar: Option<(u32, u32)>,
    pub timing: Option<(u32, u32, bool)>, // num_units_in_tick, time_scale, fixed_frame_rate
    pub video_full_range: Option<bool>,
    pub colour: Option<(u8, u8, u8)>, // primaries, transfer, matrix
    pub bitstream_restriction_max_num_reorder: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct Sps {
    pub profile_idc: u8,
    pub constraint_flags: u8,
    pub level_idc: u8,
    pub chroma_format_idc: u32,
    pub bit_depth_luma: u32,
    pub bit_depth_chroma: u32,
    pub width: u32,
    pub height: u32,
    pub frame_mbs_only: bool,
    pub mb_adaptive: bool,
    pub num_ref_frames: u32,
    pub poc_type: u32,
    pub vui: Vui,
}

fn scaling_list(r: &mut BitReader, size: usize) -> Option<()> {
    let mut last: i32 = 8;
    let mut next: i32 = 8;
    for _ in 0..size {
        if next != 0 {
            let delta = r.se()?;
            next = (last + delta + 256) % 256;
        }
        last = if next == 0 { last } else { next };
    }
    Some(())
}

/// Parse an SPS NAL (payload after the NAL header byte, still escaped).
pub fn parse_sps(nal: &[u8]) -> Option<Sps> {
    let data = unescape_rbsp(nal);
    let mut r = BitReader::new(&data);
    let mut s = Sps { profile_idc: r.u8(8)?, constraint_flags: r.u8(8)?, level_idc: r.u8(8)?, chroma_format_idc: 1, bit_depth_luma: 8, bit_depth_chroma: 8, ..Default::default() };
    let _sps_id = r.ue()?;
    if matches!(s.profile_idc, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        s.chroma_format_idc = r.ue()?;
        if s.chroma_format_idc == 3 {
            r.bit()?; // separate_colour_plane_flag
        }
        s.bit_depth_luma = r.ue()? + 8;
        s.bit_depth_chroma = r.ue()? + 8;
        r.bit()?; // qpprime_y_zero_transform_bypass_flag
        if r.bit()? {
            let n = if s.chroma_format_idc != 3 { 8 } else { 12 };
            for i in 0..n {
                if r.bit()? {
                    scaling_list(&mut r, if i < 6 { 16 } else { 64 })?;
                }
            }
        }
    }
    let _log2_max_frame_num = r.ue()? + 4;
    s.poc_type = r.ue()?;
    if s.poc_type == 0 {
        r.ue()?;
    } else if s.poc_type == 1 {
        r.bit()?;
        r.se()?;
        r.se()?;
        let n = r.ue()?;
        if n > 255 {
            return None;
        }
        for _ in 0..n {
            r.se()?;
        }
    }
    s.num_ref_frames = r.ue()?;
    r.bit()?; // gaps_in_frame_num_value_allowed_flag
    let w_mbs = r.ue()? + 1;
    let h_map = r.ue()? + 1;
    s.frame_mbs_only = r.bit()?;
    if !s.frame_mbs_only {
        s.mb_adaptive = r.bit()?;
    }
    r.bit()?; // direct_8x8_inference_flag
    let (mut cl, mut cr, mut ct, mut cb) = (0, 0, 0, 0);
    if r.bit()? {
        cl = r.ue()?;
        cr = r.ue()?;
        ct = r.ue()?;
        cb = r.ue()?;
    }
    let (sub_w, sub_h) = match s.chroma_format_idc {
        0 => (1, 1),
        1 => (2, 2),
        2 => (2, 1),
        _ => (1, 1),
    };
    let crop_unit_x = if s.chroma_format_idc == 0 { 1 } else { sub_w };
    let crop_unit_y = (if s.chroma_format_idc == 0 { 1 } else { sub_h }) * if s.frame_mbs_only { 1 } else { 2 };
    let width = w_mbs * 16;
    let height = (2 - s.frame_mbs_only as u32) * h_map * 16;
    s.width = width.saturating_sub((cl + cr) * crop_unit_x);
    s.height = height.saturating_sub((ct + cb) * crop_unit_y);
    if r.bit()? {
        s.vui = parse_vui(&mut r).unwrap_or_default();
    }
    Some(s)
}

fn hrd(r: &mut BitReader) -> Option<()> {
    let cpb_cnt = r.ue()? + 1;
    if cpb_cnt > 32 {
        return None;
    }
    r.skip(8);
    for _ in 0..cpb_cnt {
        r.ue()?;
        r.ue()?;
        r.bit()?;
    }
    r.skip(20);
    Some(())
}

fn parse_vui(r: &mut BitReader) -> Option<Vui> {
    let mut v = Vui::default();
    if r.bit()? {
        let idc = r.u8(8)?;
        v.sar = match idc {
            0 => None,
            255 => Some((r.u32(16)?, r.u32(16)?)),
            _ => sar_from_idc(idc),
        };
    }
    if r.bit()? {
        r.bit()?; // overscan_appropriate
    }
    if r.bit()? {
        r.skip(3); // video_format
        v.video_full_range = Some(r.bit()?);
        if r.bit()? {
            v.colour = Some((r.u8(8)?, r.u8(8)?, r.u8(8)?));
        }
    }
    if r.bit()? {
        r.ue()?;
        r.ue()?;
    }
    if r.bit()? {
        let num_units = r.u32(32)?;
        let time_scale = r.u32(32)?;
        let fixed = r.bit()?;
        v.timing = Some((num_units, time_scale, fixed));
    }
    let nal_hrd = r.bit()?;
    if nal_hrd {
        hrd(r)?;
    }
    let vcl_hrd = r.bit()?;
    if vcl_hrd {
        hrd(r)?;
    }
    if nal_hrd || vcl_hrd {
        r.bit()?;
    }
    r.bit()?; // pic_struct_present
    if r.bit()? {
        r.bit()?;
        r.ue()?;
        r.ue()?;
        r.ue()?;
        r.ue()?;
        v.bitstream_restriction_max_num_reorder = Some(r.ue()?);
        r.ue()?;
    }
    Some(v)
}

pub fn sar_from_idc(idc: u8) -> Option<(u32, u32)> {
    Some(match idc {
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
        14 => (4, 3),
        15 => (3, 2),
        16 => (2, 1),
        _ => return None,
    })
}

/// PPS: returns entropy_coding_mode_flag (CABAC).
pub fn parse_pps_cabac(nal: &[u8]) -> Option<bool> {
    let data = unescape_rbsp(nal);
    let mut r = BitReader::new(&data);
    r.ue()?;
    r.ue()?;
    r.bit()
}

/// SEI: returns the user_data_unregistered text payload (x264 info string) when present.
pub fn parse_sei_user_data(nal: &[u8]) -> Option<String> {
    let data = unescape_rbsp(nal);
    let mut i = 0;
    while i + 2 <= data.len() {
        let mut ptype = 0usize;
        while i < data.len() && data[i] == 0xFF {
            ptype += 255;
            i += 1;
        }
        ptype += *data.get(i)? as usize;
        i += 1;
        let mut size = 0usize;
        while i < data.len() && data[i] == 0xFF {
            size += 255;
            i += 1;
        }
        size += *data.get(i)? as usize;
        i += 1;
        if ptype == 5 && size > 16 {
            let payload = data.get(i + 16..(i + size).min(data.len()))?;
            let end = payload.iter().position(|&b| b == 0).unwrap_or(payload.len());
            let text = String::from_utf8_lossy(&payload[..end]).into_owned();
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
        i += size;
        if i >= data.len() || data[i] == 0x80 {
            break;
        }
    }
    None
}

/// `x264 - core 165 r3222 - H.264/... - options: a=1 b=2` → (library, settings).
pub fn split_encoder_string(text: &str) -> (String, String) {
    let text = text.trim();
    if let Some((head, opts)) = text.split_once(" - options: ").or_else(|| text.split_once("options: ")) {
        let mut parts = head.split(" - ");
        let name = parts.next().unwrap_or("").trim().to_string();
        let version = parts.next().unwrap_or("").trim().to_string();
        let lib = if version.is_empty() { name } else { format!("{name} - {version}") };
        let settings = opts.split_whitespace().collect::<Vec<_>>().join(" / ");
        return (lib, settings);
    }
    (text.to_string(), String::new())
}

pub fn profile_name(profile_idc: u8, constraint_flags: u8) -> String {
    let set3 = constraint_flags & 0x10 != 0;
    let set4 = constraint_flags & 0x08 != 0;
    let set5 = constraint_flags & 0x04 != 0;
    match profile_idc {
        66 => "Baseline",
        77 => "Main",
        88 => "Extended",
        100 => {
            if set4 && set5 {
                "Constrained High"
            } else if set4 {
                "Progressive High"
            } else {
                "High"
            }
        }
        110 => if set3 { "High 10 Intra" } else { "High 10" },
        122 => if set3 { "High 4:2:2 Intra" } else { "High 4:2:2" },
        244 => if set3 { "High 4:4:4 Intra" } else { "High 4:4:4 Predictive" },
        44 => "CAVLC 4:4:4 Intra",
        83 => "Scalable Baseline",
        86 => "Scalable High",
        118 => "Multiview High",
        128 => "Stereo High",
        138 => "Multiview Depth High",
        _ => "",
    }
    .to_string()
}

pub fn level_name(level_idc: u8, constraint_flags: u8, profile_idc: u8) -> String {
    let set3 = constraint_flags & 0x10 != 0;
    if level_idc == 9 || (level_idc == 11 && set3 && matches!(profile_idc, 66 | 77 | 88)) {
        return "1b".to_string();
    }
    if level_idc % 10 == 0 {
        format!("{}", level_idc / 10)
    } else {
        format!("{}.{}", level_idc / 10, level_idc % 10)
    }
}

/// Fill a video stream from SPS (+ optional PPS CABAC flag).
pub fn apply(s: &mut Stream, sps: &Sps, cabac: Option<bool>, container_frame_rate: bool) {
    s.set_if_empty("Format", "AVC");
    let profile = profile_name(sps.profile_idc, sps.constraint_flags);
    let level = level_name(sps.level_idc, sps.constraint_flags, sps.profile_idc);
    if !profile.is_empty() {
        s.set_if_empty("Format_Profile", format!("{profile}@L{level}"));
    }
    let mut settings = Vec::new();
    if let Some(c) = cabac {
        s.set_bool("Format_Settings_CABAC", c);
        if c {
            settings.push("CABAC".to_string());
        }
    }
    settings.push(format!("{} Ref Frames", sps.num_ref_frames));
    s.set_int("Format_Settings_RefFrames", sps.num_ref_frames as i128);
    s.set_if_empty("Format_Settings", settings.join(" / "));
    if sps.width > 0 && sps.height > 0 {
        s.set_if_empty("Width", sps.width.to_string());
        s.set_if_empty("Height", sps.height.to_string());
    }
    if let Some((n, d)) = sps.vui.sar {
        if n > 0 && d > 0 && !s.has("PixelAspectRatio") && !s.has("DisplayAspectRatio") {
            s.set("PixelAspectRatio", format!("{:.3}", n as f64 / d as f64));
        }
    }
    if let Some((num, scale, fixed)) = sps.vui.timing {
        if num > 0 && scale > 0 {
            let mut fps = scale as f64 / num as f64 / 2.0;
            if sps.poc_type == 2 {
                fps *= 2.0;
            }
            if !container_frame_rate {
                s.set_if_empty("FrameRate", format!("{fps:.3}"));
            }
            s.set_if_empty("FrameRate_Mode", if fixed { "CFR" } else { "VFR" });
        }
    }
    s.set_if_empty("ColorSpace", "YUV");
    s.set_if_empty(
        "ChromaSubsampling",
        match sps.chroma_format_idc {
            0 => "4:0:0",
            1 => "4:2:0",
            2 => "4:2:2",
            _ => "4:4:4",
        },
    );
    s.set_if_empty("BitDepth", sps.bit_depth_luma.to_string());
    if sps.frame_mbs_only {
        s.set_if_empty("ScanType", "Progressive");
    } else if sps.mb_adaptive {
        s.set_if_empty("ScanType", "MBAFF");
    } else {
        s.set_if_empty("ScanType", "Interlaced");
    }
    if let Some(full) = sps.vui.video_full_range {
        s.set_extra("colour_range", if full { "Full" } else { "Limited" }, "", "Y YTY");
        s.set_extra("colour_range_Source", "Stream", "", "N YTY");
    }
    if let Some((p, t, m)) = sps.vui.colour {
        s.set_extra("colour_description_present", "Yes", "", "N YTY");
        s.set_extra("colour_description_present_Source", "Stream", "", "N YTY");
        s.set_extra("colour_primaries", super::colour::primaries(p), "", "Y YTY");
        s.set_extra("colour_primaries_Source", "Stream", "", "N YTY");
        s.set_extra("transfer_characteristics", super::colour::transfer(t), "", "Y YTY");
        s.set_extra("transfer_characteristics_Source", "Stream", "", "N YTY");
        s.set_extra("matrix_coefficients", super::colour::matrix(m), "", "Y YTY");
        s.set_extra("matrix_coefficients_Source", "Stream", "", "N YTY");
    }
}

/// Fill encoder information from a SEI user data string.
pub fn apply_sei(s: &mut Stream, text: &str) {
    let (lib, settings) = split_encoder_string(text);
    if !lib.is_empty() {
        s.set("Encoded_Library", lib);
        s.clear("Encoded_Library/String");
        s.clear("Encoded_Library_Name");
        s.clear("Encoded_Library_Version");
    }
    if !settings.is_empty() {
        s.set("Encoded_Library_Settings", settings);
    }
}

/// Parse an `avcC` (AVCDecoderConfigurationRecord): returns (sps list, pps list, nal length size).
pub fn parse_avcc(data: &[u8]) -> Option<(Vec<Vec<u8>>, Vec<Vec<u8>>, usize)> {
    if data.len() < 7 || data[0] != 1 {
        return None;
    }
    let len_size = (data[4] & 3) as usize + 1;
    let n_sps = (data[5] & 0x1F) as usize;
    let mut i = 6;
    let mut sps = Vec::new();
    for _ in 0..n_sps {
        let l = crate::io::be16(data, i)? as usize;
        i += 2;
        sps.push(data.get(i..i + l)?.to_vec());
        i += l;
    }
    let n_pps = *data.get(i)? as usize;
    i += 1;
    let mut pps = Vec::new();
    for _ in 0..n_pps {
        let l = crate::io::be16(data, i)? as usize;
        i += 2;
        pps.push(data.get(i..i + l)?.to_vec());
        i += l;
    }
    Some((sps, pps, len_size))
}

/// Fill a stream from an avcC record: SPS/PPS; frame rate is left to the container.
pub fn apply_avcc(s: &mut Stream, avcc: &[u8]) -> bool {
    let Some((sps_list, pps_list, _)) = parse_avcc(avcc) else { return false };
    let Some(sps_nal) = sps_list.first() else { return false };
    let Some(sps) = parse_sps(sps_nal.get(1..).unwrap_or(&[])) else { return false };
    let cabac = pps_list.first().and_then(|p| parse_pps_cabac(p.get(1..).unwrap_or(&[])));
    apply(s, &sps, cabac, true);
    true
}

/// Iterate NAL units in a length-prefixed sample (MP4/Matroska block): (nal_type, payload after header).
pub fn nals_length_prefixed(data: &[u8], len_size: usize) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + len_size <= data.len() {
        let mut l = 0usize;
        for k in 0..len_size {
            l = (l << 8) | data[i + k] as usize;
        }
        i += len_size;
        if l == 0 || i + l > data.len() {
            break;
        }
        out.push((data[i] & 0x1F, &data[i + 1..i + l]));
        i += l;
    }
    out
}

/// Iterate NAL units in Annex B byte stream: (nal_type, payload after header).
pub fn nals_annexb(data: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    let starts = start_codes(data);
    for w in 0..starts.len() {
        let (s, hdr) = starts[w];
        let e = starts.get(w + 1).map(|(n, _)| *n).unwrap_or(data.len());
        let body = &data[s + hdr..e];
        // trailing zero bytes belong to the next start code
        let mut end = body.len();
        while end > 0 && body[end - 1] == 0 {
            end -= 1;
        }
        if end > 0 {
            out.push((body[0] & 0x1F, &body[1..end]));
        }
    }
    out
}

/// Positions of start codes: (offset, prefix length 3 or 4).
pub fn start_codes(data: &[u8]) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            let four = i > 0 && data[i - 1] == 0;
            v.push((if four { i - 1 } else { i }, if four { 4 } else { 3 }));
            i += 3;
        } else {
            i += 1;
        }
    }
    v
}

/// Apply SEI encoder info found in a sample's NAL units (first frame usually carries it).
pub fn apply_sei_from_nals(s: &mut Stream, nals: &[(u8, &[u8])]) {
    for (t, p) in nals {
        if *t == 6 {
            if let Some(text) = parse_sei_user_data(p) {
                apply_sei(s, &text);
                return;
            }
        }
    }
}

// ---- elementary stream

pub fn probe(p: &Probe) -> u8 {
    let nals = nals_annexb(&p.head[..p.head.len().min(4096)]);
    let has_sps = nals.iter().any(|(t, _)| *t == 7);
    let has_pps = nals.iter().any(|(t, _)| *t == 8);
    if has_sps && has_pps && (p.starts_with(&[0, 0, 0, 1]) || p.starts_with(&[0, 0, 1])) {
        if p.ext_in(&["h264", "264", "avc", "x264"]) { 95 } else { 70 }
    } else if p.ext_in(&["h264", "264", "avc"]) && nals.len() > 1 {
        40
    } else {
        0
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let size = r.len();
    let cap = size.min(16 * 1024 * 1024) as usize;
    let data = r.read_vec_at(0, cap);
    let nals = nals_annexb(&data);
    let Some((_, sps_nal)) = nals.iter().find(|(t, _)| *t == 7) else { return false };
    let Some(sps) = parse_sps(sps_nal) else { return false };
    let cabac = nals.iter().find(|(t, _)| *t == 8).and_then(|(_, p)| parse_pps_cabac(p));
    let mut s = Stream::new(StreamKind::Video);
    apply(&mut s, &sps, cabac, false);
    apply_sei_from_nals(&mut s, &nals);
    // Count frames: slices with first_mb_in_slice == 0 start a new picture.
    let mut frames = 0u64;
    for (t, p) in &nals {
        if matches!(t, 1 | 5) {
            let d = unescape_rbsp(&p[..p.len().min(8)]);
            let mut br = BitReader::new(&d);
            if br.ue() == Some(0) {
                frames += 1;
            }
        }
    }
    if cap < size as usize && frames > 0 {
        frames = (frames as f64 * size as f64 / cap as f64).round() as u64;
    }
    if frames > 0 {
        s.set("FrameCount", frames.to_string());
        if let Some(fps) = s.get_f64("FrameRate") {
            if fps > 0.0 {
                s.set("Duration", format!("{}", (frames as f64 / fps * 1000.0).round() as u64));
            }
        }
    }
    doc.general().set("Format", "AVC");
    let dur = s.get("Duration").to_string();
    if !dur.is_empty() {
        doc.general().set("Duration", dur);
    }
    doc.streams[StreamKind::Video as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_string() {
        let (lib, set) = split_encoder_string("x264 - core 165 r3222 b35605a - H.264/MPEG-4 AVC codec - Copyleft 2003-2024 - http://www.videolan.org/x264.html - options: cabac=0 ref=1 deblock=0:0:0");
        assert_eq!(lib, "x264 - core 165 r3222 b35605a");
        assert_eq!(set, "cabac=0 / ref=1 / deblock=0:0:0");
    }

    #[test]
    fn names() {
        assert_eq!(profile_name(66, 0xC0), "Baseline");
        assert_eq!(profile_name(100, 0), "High");
        assert_eq!(level_name(10, 0, 66), "1");
        assert_eq!(level_name(31, 0, 100), "3.1");
        assert_eq!(level_name(40, 0, 100), "4");
        assert_eq!(level_name(11, 0x10, 66), "1b");
    }

    #[test]
    fn annexb() {
        let d = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 9];
        let n = nals_annexb(&d);
        assert_eq!(n.len(), 3);
        assert_eq!(n[0].0, 7);
        assert_eq!(n[0].1, &[1, 2]);
        assert_eq!(n[1].0, 8);
        assert_eq!(n[2].0, 5);
    }
}

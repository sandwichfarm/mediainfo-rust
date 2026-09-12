//! H.265 / HEVC: VPS/SPS/SEI parsing (shared by containers), the `hvcC` record and the Annex B
//! elementary stream parser. Syntax from ITU-T H.265 (7.3.2.2, 7.3.3, 7.3.7, E.2.1) and
//! ISO/IEC 14496-15 (8.3.3.1).

use crate::io::bits::{unescape_rbsp, BitReader};
use crate::io::Reader;
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::video::avc;
use crate::parsers::Probe;

/// NAL unit types (H.265 Table 7-1).
pub const NAL_VPS: u8 = 32;
pub const NAL_SPS: u8 = 33;
pub const NAL_PPS: u8 = 34;
pub const NAL_AUD: u8 = 35;
pub const NAL_SEI_PREFIX: u8 = 39;
pub const NAL_SEI_SUFFIX: u8 = 40;

#[derive(Debug, Clone, Default)]
pub struct Vui {
    pub sar: Option<(u32, u32)>,
    pub video_full_range: Option<bool>,
    pub colour: Option<(u8, u8, u8)>, // primaries, transfer, matrix
    pub field_seq: bool,
    pub timing: Option<(u32, u32)>, // num_units_in_tick, time_scale
}

#[derive(Debug, Clone, Default)]
pub struct ProfileTierLevel {
    pub profile_space: u8,
    pub tier: bool,
    pub profile_idc: u8,
    pub compatibility: u32, // bit 31 - j = general_profile_compatibility_flag[j]
    pub progressive_source: bool,
    pub interlaced_source: bool,
    pub constraints: u64, // the 43 constraint bits after the four source flags (MSB first)
    pub level_idc: u8,
}

#[derive(Debug, Clone, Default)]
pub struct Sps {
    pub vps_id: u8,
    pub max_sub_layers: u8,
    pub temporal_id_nesting: bool,
    pub ptl: ProfileTierLevel,
    pub chroma_format_idc: u32,
    pub width: u32,
    pub height: u32,
    pub bit_depth_luma: u32,
    pub bit_depth_chroma: u32,
    pub vui: Vui,
}

fn profile_tier_level(r: &mut BitReader, max_sub_layers_minus1: u8) -> Option<ProfileTierLevel> {
    let mut p = ProfileTierLevel { profile_space: r.u8(2)?, tier: r.bit()?, profile_idc: r.u8(5)?, compatibility: r.u32(32)?, ..Default::default() };
    p.progressive_source = r.bit()?;
    p.interlaced_source = r.bit()?;
    r.bit()?; // non_packed_constraint_flag
    r.bit()?; // frame_only_constraint_flag
    p.constraints = r.bits(43)?;
    r.bit()?; // general_inbld_flag / reserved
    p.level_idc = r.u8(8)?;
    let n = max_sub_layers_minus1.min(7) as usize;
    let mut sub_profile = [false; 8];
    let mut sub_level = [false; 8];
    for i in 0..n {
        sub_profile[i] = r.bit()?;
        sub_level[i] = r.bit()?;
    }
    if n > 0 {
        for _ in n..8 {
            r.skip(2);
        }
    }
    for i in 0..n {
        if sub_profile[i] {
            r.skip(88);
        }
        if sub_level[i] {
            r.skip(8);
        }
    }
    if r.bits_left() == 0 && n > 0 {
        return None;
    }
    Some(p)
}

fn scaling_list_data(r: &mut BitReader) -> Option<()> {
    for size_id in 0..4 {
        let step = if size_id == 3 { 3 } else { 1 };
        let mut matrix_id = 0;
        while matrix_id < 6 {
            if !r.bit()? {
                r.ue()?; // scaling_list_pred_matrix_id_delta
            } else {
                let coef_num = 64.min(1 << (4 + (size_id << 1)));
                if size_id > 1 {
                    r.se()?;
                }
                for _ in 0..coef_num {
                    r.se()?;
                }
            }
            matrix_id += step;
        }
    }
    Some(())
}

/// Short-term reference picture set (7.3.7); only the derived delta POC lists are kept so that
/// inter-predicted sets can be sized.
fn st_ref_pic_set(r: &mut BitReader, idx: usize, num_sets: usize, sets: &mut Vec<(Vec<i32>, Vec<i32>)>) -> Option<()> {
    let inter = if idx != 0 { r.bit()? } else { false };
    let (mut s0, mut s1) = (Vec::new(), Vec::new());
    if inter {
        let delta_idx = if idx == num_sets { r.ue()? as usize + 1 } else { 1 };
        let sign = r.bit()?;
        let abs = r.ue()? as i64 + 1;
        let delta_rps = (if sign { -abs } else { abs }) as i32;
        let ref_idx = idx.checked_sub(delta_idx)?;
        let (r0, r1) = sets.get(ref_idx)?.clone();
        let num_delta = r0.len() + r1.len();
        let mut use_delta = Vec::with_capacity(num_delta + 1);
        for _ in 0..=num_delta {
            let used = r.bit()?;
            use_delta.push(if used { true } else { r.bit()? });
        }
        for j in (0..r1.len()).rev() {
            let d = r1[j].saturating_add(delta_rps);
            if d < 0 && use_delta[r0.len() + j] {
                s0.push(d);
            }
        }
        if delta_rps < 0 && use_delta[num_delta] {
            s0.push(delta_rps);
        }
        for j in 0..r0.len() {
            let d = r0[j].saturating_add(delta_rps);
            if d < 0 && use_delta[j] {
                s0.push(d);
            }
        }
        for j in (0..r0.len()).rev() {
            let d = r0[j].saturating_add(delta_rps);
            if d > 0 && use_delta[j] {
                s1.push(d);
            }
        }
        if delta_rps > 0 && use_delta[num_delta] {
            s1.push(delta_rps);
        }
        for j in 0..r1.len() {
            let d = r1[j].saturating_add(delta_rps);
            if d > 0 && use_delta[r0.len() + j] {
                s1.push(d);
            }
        }
    } else {
        let neg = r.ue()? as usize;
        let pos = r.ue()? as usize;
        if neg > 16 || pos > 16 {
            return None;
        }
        let mut poc: i32 = 0;
        for _ in 0..neg {
            poc = poc.saturating_sub(r.ue()? as i32 + 1);
            r.bit()?;
            s0.push(poc);
        }
        poc = 0;
        for _ in 0..pos {
            poc = poc.saturating_add(r.ue()? as i32 + 1);
            r.bit()?;
            s1.push(poc);
        }
    }
    sets.push((s0, s1));
    Some(())
}

fn parse_vui(r: &mut BitReader) -> Option<Vui> {
    let mut v = Vui::default();
    if r.bit()? {
        let idc = r.u8(8)?;
        v.sar = match idc {
            0 => None,
            255 => Some((r.u32(16)?, r.u32(16)?)),
            _ => avc::sar_from_idc(idc),
        };
    }
    if r.bit()? {
        r.bit()?; // overscan_appropriate_flag
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
    r.bit()?; // neutral_chroma_indication_flag
    v.field_seq = r.bit()?;
    r.bit()?; // frame_field_info_present_flag
    if r.bit()? {
        for _ in 0..4 {
            r.ue()?;
        }
    }
    if r.bit()? {
        let num = r.u32(32)?;
        let scale = r.u32(32)?;
        v.timing = Some((num, scale));
        // poc_proportional / hrd parameters follow; nothing further is needed.
    }
    Some(v)
}

/// Parse an SPS NAL (payload after the 2-byte NAL header, still escaped).
pub fn parse_sps(nal: &[u8]) -> Option<Sps> {
    let data = unescape_rbsp(nal);
    let mut r = BitReader::new(&data);
    let mut s = Sps { vps_id: r.u8(4)?, max_sub_layers: r.u8(3)? + 1, temporal_id_nesting: r.bit()?, ..Default::default() };
    s.ptl = profile_tier_level(&mut r, s.max_sub_layers - 1)?;
    let _sps_id = r.ue()?;
    s.chroma_format_idc = r.ue()?;
    if s.chroma_format_idc > 3 {
        return None;
    }
    if s.chroma_format_idc == 3 {
        r.bit()?; // separate_colour_plane_flag
    }
    let width = r.ue()?;
    let height = r.ue()?;
    let (mut cl, mut cr, mut ct, mut cb) = (0, 0, 0, 0);
    if r.bit()? {
        cl = r.ue()?;
        cr = r.ue()?;
        ct = r.ue()?;
        cb = r.ue()?;
    }
    let (sub_w, sub_h) = match s.chroma_format_idc {
        1 => (2, 2),
        2 => (2, 1),
        _ => (1, 1),
    };
    s.width = width.saturating_sub((cl.saturating_add(cr)).saturating_mul(sub_w));
    s.height = height.saturating_sub((ct.saturating_add(cb)).saturating_mul(sub_h));
    s.bit_depth_luma = r.ue()? + 8;
    s.bit_depth_chroma = r.ue()? + 8;
    let log2_max_poc_lsb = r.ue()? + 4;
    if log2_max_poc_lsb > 16 {
        return None;
    }
    let ordering_info = r.bit()?;
    let start = if ordering_info { 0 } else { s.max_sub_layers - 1 };
    for _ in start..s.max_sub_layers {
        r.ue()?;
        r.ue()?;
        r.ue()?;
    }
    r.ue()?; // log2_min_luma_coding_block_size_minus3
    r.ue()?; // log2_diff_max_min_luma_coding_block_size
    r.ue()?; // log2_min_luma_transform_block_size_minus2
    r.ue()?; // log2_diff_max_min_luma_transform_block_size
    r.ue()?; // max_transform_hierarchy_depth_inter
    r.ue()?; // max_transform_hierarchy_depth_intra
    if r.bit()? && r.bit()? {
        scaling_list_data(&mut r)?;
    }
    r.bit()?; // amp_enabled_flag
    r.bit()?; // sample_adaptive_offset_enabled_flag
    if r.bit()? {
        r.skip(8); // pcm sample bit depths
        r.ue()?;
        r.ue()?;
        r.bit()?;
    }
    let num_sets = r.ue()? as usize;
    if num_sets > 64 {
        return None;
    }
    let mut sets = Vec::with_capacity(num_sets);
    for i in 0..num_sets {
        st_ref_pic_set(&mut r, i, num_sets, &mut sets)?;
    }
    if r.bit()? {
        let n = r.ue()?;
        if n > 32 {
            return None;
        }
        for _ in 0..n {
            r.skip(log2_max_poc_lsb as usize);
            r.bit()?;
        }
    }
    r.bit()?; // sps_temporal_mvp_enabled_flag
    r.bit()?; // strong_intra_smoothing_enabled_flag
    if r.bit()? {
        s.vui = parse_vui(&mut r).unwrap_or_default();
    }
    Some(s)
}

/// Profile name from profile_tier_level (H.265 Annex A).
pub fn profile_name(p: &ProfileTierLevel) -> String {
    let compat = |j: u32| p.compatibility & (1u32 << (31 - j)) != 0;
    let idc = if p.profile_idc != 0 { p.profile_idc } else { (1..=11u8).find(|&j| compat(j as u32)).unwrap_or(0) };
    match idc {
        1 => "Main".to_string(),
        2 => "Main 10".to_string(),
        3 => "Main Still".to_string(),
        4 | 5 | 9 | 10 | 11 => {
            // Format range extension constraint flags (A.3.5): max_12bit, max_10bit, max_8bit,
            // max_422chroma, max_420chroma, max_monochrome, intra, one_picture_only, lower_bit_rate.
            let f = |i: u32| p.constraints & (1u64 << (42 - i)) != 0;
            let (b12, b10, b8, c422, c420, mono, intra, one_pic) = (f(0), f(1), f(2), f(3), f(4), f(5), f(6), f(7));
            let mut name = String::new();
            if idc == 9 || idc == 10 || idc == 11 {
                name.push_str("Screen-Extended ");
            } else if idc == 5 {
                name.push_str("High Throughput ");
            }
            name.push_str(if mono {
                "Monochrome"
            } else if c420 {
                "Main"
            } else if c422 {
                "Main 4:2:2"
            } else {
                "Main 4:4:4"
            });
            let depth = if b8 && b10 && b12 {
                ""
            } else if b10 && b12 {
                " 10"
            } else if b12 {
                " 12"
            } else {
                " 16"
            };
            name.push_str(depth);
            if one_pic {
                name.push_str(" Still Picture");
            } else if intra {
                name.push_str(" Intra");
            }
            name
        }
        6 => "Multiview Main".to_string(),
        7 => "Scalable Main".to_string(),
        8 => "3D Main".to_string(),
        _ => String::new(),
    }
}

/// `general_level_idc` → "1", "3.1", "5.2".
pub fn level_name(level_idc: u8) -> String {
    let major = level_idc / 30;
    let minor = (level_idc % 30) / 3;
    if minor == 0 {
        format!("{major}")
    } else {
        format!("{major}.{minor}")
    }
}

/// Fill a video stream from an SPS. `container_frame_rate` suppresses FrameRate from VUI timing.
pub fn apply(s: &mut Stream, sps: &Sps, container_frame_rate: bool) {
    s.set_if_empty("Format", "HEVC");
    let profile = profile_name(&sps.ptl);
    if !profile.is_empty() && sps.ptl.level_idc > 0 {
        let tier = if sps.ptl.tier { "High" } else { "Main" };
        s.set_if_empty("Format_Profile", format!("{profile}@L{}@{tier}", level_name(sps.ptl.level_idc)));
    } else if !profile.is_empty() {
        s.set_if_empty("Format_Profile", profile);
    }
    if sps.width > 0 && sps.height > 0 {
        s.set_if_empty("Width", sps.width.to_string());
        s.set_if_empty("Height", sps.height.to_string());
    }
    if let Some((n, d)) = sps.vui.sar {
        if n > 0 && d > 0 && !s.has("PixelAspectRatio") && !s.has("DisplayAspectRatio") {
            s.set("PixelAspectRatio", format!("{:.3}", n as f64 / d as f64));
        }
    }
    if let Some((num, scale)) = sps.vui.timing {
        if num > 0 && scale > 0 && !container_frame_rate {
            let fps = scale as f64 / num as f64;
            s.set_if_empty("FrameRate", format!("{fps:.3}"));
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
    if let Some(full) = sps.vui.video_full_range {
        super::colour::set_range(s, full, super::colour::STREAM);
    }
    if let Some((p, t, m)) = sps.vui.colour {
        super::colour::set_description(s, p, t, m, super::colour::STREAM);
    }
}

/// `x265 (build 217) - 4.3:[Linux]... - options: ...` → Encoded_Library / Encoded_Library_Settings.
pub fn apply_sei(s: &mut Stream, text: &str) {
    let (mut lib, settings) = avc::split_encoder_string(text);
    if let Some(start) = lib.find(" (build ") {
        if let Some(end) = lib[start..].find(')') {
            lib.replace_range(start..start + end + 1, "");
        }
    }
    if !lib.is_empty() {
        s.set("Encoded_Library", lib.clone());
        s.clear("Encoded_Library/String");
        s.clear("Encoded_Library_Name");
        s.clear("Encoded_Library_Version");
    }
    if !settings.is_empty() {
        let settings = if lib.starts_with("x265") {
            // The reference omits the bit depth and frame rate entries of the x265 option list.
            settings.split(" / ").filter(|t| !t.starts_with("bitdepth=") && !t.starts_with("fps=")).collect::<Vec<_>>().join(" / ")
        } else {
            settings
        };
        s.set("Encoded_Library_Settings", settings);
    }
}

/// Parse an `hvcC` record: (nal arrays as (type, nal unit with header) list, nal length size).
pub fn parse_hvcc(data: &[u8]) -> Option<(Vec<(u8, Vec<u8>)>, usize)> {
    if data.len() < 23 || data[0] != 1 {
        return None;
    }
    let len_size = (data[21] & 3) as usize + 1;
    let num_arrays = data[22] as usize;
    let mut i = 23;
    let mut nals = Vec::new();
    for _ in 0..num_arrays {
        let t = *data.get(i)? & 0x3F;
        let n = crate::io::be16(data, i + 1)? as usize;
        i += 3;
        for _ in 0..n {
            let l = crate::io::be16(data, i)? as usize;
            i += 2;
            nals.push((t, data.get(i..i + l)?.to_vec()));
            i += l;
        }
    }
    Some((nals, len_size))
}

/// NAL length field size of an hvcC record.
pub fn hvcc_length_size(d: &[u8]) -> Option<usize> {
    if d.len() < 23 || d[0] != 1 {
        return None;
    }
    Some((d[21] & 3) as usize + 1)
}

/// Fill a stream from an hvcC record (SPS); frame rate is left to the container.
pub fn apply_hvcc(s: &mut Stream, d: &[u8]) -> bool {
    let Some((nals, _)) = parse_hvcc(d) else { return false };
    let Some(sps) = nals.iter().filter(|(t, _)| *t == NAL_SPS).find_map(|(_, p)| parse_sps(p.get(2..).unwrap_or(&[]))) else {
        // Fall back to the record's own summary fields.
        s.set_if_empty("Format", "HEVC");
        let ptl = ProfileTierLevel { profile_space: d[1] >> 6, tier: d[1] & 0x20 != 0, profile_idc: d[1] & 0x1F, compatibility: crate::io::be32(d, 2).unwrap_or(0), constraints: (crate::io::be64(d, 6).unwrap_or(0) >> 16) & ((1u64 << 43) - 1), level_idc: d[12], ..Default::default() };
        let profile = profile_name(&ptl);
        if !profile.is_empty() && ptl.level_idc > 0 {
            s.set_if_empty("Format_Profile", format!("{profile}@L{}@{}", level_name(ptl.level_idc), if ptl.tier { "High" } else { "Main" }));
        }
        s.set_if_empty("ColorSpace", "YUV");
        s.set_if_empty("ChromaSubsampling", match d[16] & 3 { 0 => "4:0:0", 1 => "4:2:0", 2 => "4:2:2", _ => "4:4:4" });
        s.set_if_empty("BitDepth", ((d[17] & 7) + 8).to_string());
        return true;
    };
    apply(s, &sps, true);
    // Some muxers store the encoder SEI as an extra array of the record.
    let sei: Vec<(u8, &[u8])> = nals.iter().filter(|(t, _)| *t == NAL_SEI_PREFIX || *t == NAL_SEI_SUFFIX).map(|(t, p)| (*t, p.get(2..).unwrap_or(&[]))).collect();
    apply_sei_from_nals(s, &sei);
    true
}

/// Iterate NAL units in a length-prefixed sample: (nal_type, payload after the 2-byte header).
pub fn nals_length_prefixed(data: &[u8], len_size: usize) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    if len_size == 0 || len_size > 4 {
        return out;
    }
    let mut i = 0;
    while i + len_size <= data.len() && out.len() < 4096 {
        let mut l = 0usize;
        for k in 0..len_size {
            l = (l << 8) | data[i + k] as usize;
        }
        i += len_size;
        if l < 2 || i + l > data.len() {
            break;
        }
        out.push(((data[i] >> 1) & 0x3F, &data[i + 2..i + l]));
        i += l;
    }
    out
}

/// Iterate NAL units in an Annex B byte stream: (nal_type, payload after the 2-byte header).
pub fn nals_annexb(data: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    let starts = avc::start_codes(data);
    for w in 0..starts.len() {
        let (s, hdr) = starts[w];
        let e = starts.get(w + 1).map(|(n, _)| *n).unwrap_or(data.len());
        let body = &data[s + hdr..e];
        let mut end = body.len();
        while end > 0 && body[end - 1] == 0 {
            end -= 1;
        }
        if end >= 2 {
            out.push(((body[0] >> 1) & 0x3F, &body[2..end]));
        }
    }
    out
}

/// Apply SEI encoder info found in a sample's NAL units.
pub fn apply_sei_from_nals(s: &mut Stream, nals: &[(u8, &[u8])]) {
    for (t, p) in nals {
        if *t == NAL_SEI_PREFIX || *t == NAL_SEI_SUFFIX {
            if let Some(text) = avc::parse_sei_user_data(p) {
                apply_sei(s, &text);
                return;
            }
        }
    }
}

/// Whether a VCL NAL payload starts a new picture (first_slice_segment_in_pic_flag).
fn first_slice_in_pic(nal_type: u8, payload: &[u8]) -> bool {
    nal_type < 32 && payload.first().is_some_and(|b| b & 0x80 != 0)
}

// ---- elementary stream

pub fn probe(p: &Probe) -> u8 {
    let nals = nals_annexb(&p.head[..p.head.len().min(4096)]);
    let has_vps = nals.iter().any(|(t, _)| *t == NAL_VPS);
    let has_sps = nals.iter().any(|(t, _)| *t == NAL_SPS);
    let has_pps = nals.iter().any(|(t, _)| *t == NAL_PPS);
    let starts = p.starts_with(&[0, 0, 0, 1]) || p.starts_with(&[0, 0, 1]);
    if starts && has_sps && has_pps && (has_vps || nals.len() > 3) {
        if p.ext_in(&["h265", "hevc", "265", "x265"]) { 95 } else { 70 }
    } else if p.ext_in(&["h265", "hevc", "265"]) && nals.len() > 1 {
        40
    } else {
        0
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let cap = r.len().min(4 * 1024 * 1024) as usize;
    let data = r.read_vec_at(0, cap);
    let nals = nals_annexb(&data);
    let Some(sps) = nals.iter().filter(|(t, _)| *t == NAL_SPS).find_map(|(_, p)| parse_sps(p)) else { return false };
    let mut s = Stream::new(StreamKind::Video);
    apply(&mut s, &sps, false);
    apply_sei_from_nals(&mut s, &nals);
    // The reference stops demuxing an elementary HEVC stream after this many pictures and reports
    // FrameCount/Duration from what it saw (raw.h265: 16 of 25 frames).
    const FRAME_COUNT_LIMIT: u64 = 16;
    let frames = (nals.iter().filter(|(t, p)| first_slice_in_pic(*t, p)).count() as u64).min(FRAME_COUNT_LIMIT);
    if frames > 0 {
        s.set("FrameCount", frames.to_string());
        if let Some(fps) = s.get_f64("FrameRate") {
            if fps > 0.0 {
                s.set("Duration", format!("{}", (frames as f64 / fps * 1000.0).round() as u64));
            }
        }
    }
    doc.general().set("Format", "HEVC");
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
    use crate::parsers::video::testutil::BitWriter;

    fn sps_bytes(profile: u8, level: u8, width: u32, height: u32, chroma: u32, depth: u32, timing: bool) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.b(0, 4); // vps id
        w.b(0, 3); // max_sub_layers_minus1
        w.b(1, 1); // temporal_id_nesting
        w.b(0, 2); // profile_space
        w.b(0, 1); // tier
        w.b(profile as u64, 5);
        w.b(1u64 << (31 - profile as u64), 32); // compatibility
        w.b(0b1100, 4); // progressive, interlaced, non_packed, frame_only
        w.b(0, 43);
        w.b(0, 1);
        w.b(level as u64, 8);
        w.ue(0); // sps id
        w.ue(chroma);
        if chroma == 3 {
            w.b(0, 1);
        }
        w.ue(width);
        w.ue(height + 8);
        w.b(1, 1); // conformance window
        w.ue(0);
        w.ue(0);
        w.ue(0);
        w.ue(if chroma == 1 { 4 } else { 8 }); // bottom offset in chroma units → 8 luma rows
        w.ue(depth - 8);
        w.ue(depth - 8);
        w.ue(4); // log2_max_poc_lsb_minus4
        w.b(1, 1); // ordering info present
        w.ue(1);
        w.ue(0);
        w.ue(0);
        w.ue(0); // log2_min_cb
        w.ue(1);
        w.ue(0);
        w.ue(1);
        w.ue(0);
        w.ue(0);
        w.b(0, 1); // scaling list
        w.b(0, 1); // amp
        w.b(0, 1); // sao
        w.b(0, 1); // pcm
        w.ue(2); // num_short_term_ref_pic_sets
        // set 0: explicit, 1 negative pic
        w.ue(1);
        w.ue(0);
        w.ue(0);
        w.b(1, 1);
        // set 1: inter predicted from set 0
        w.b(1, 1); // inter_ref_pic_set_prediction_flag
        w.b(0, 1); // delta_rps_sign
        w.ue(0); // abs_delta_rps_minus1 → deltaRps = 1
        w.b(1, 1); // used_by_curr_pic_flag[0]
        w.b(1, 1); // used_by_curr_pic_flag[1]
        w.b(0, 1); // long_term_ref_pics_present
        w.b(1, 1); // temporal mvp
        w.b(1, 1); // strong intra smoothing
        w.b(1, 1); // vui present
        w.b(1, 1); // aspect ratio present
        w.b(1, 8); // square
        w.b(0, 1); // overscan
        w.b(1, 1); // video signal type
        w.b(5, 3);
        w.b(0, 1); // full range
        w.b(1, 1); // colour description
        w.b(2, 8);
        w.b(2, 8);
        w.b(2, 8);
        w.b(0, 1); // chroma loc
        w.b(0, 1);
        w.b(0, 1);
        w.b(0, 1);
        w.b(0, 1); // default display window
        w.b(timing as u64, 1);
        if timing {
            w.b(1, 32);
            w.b(25, 32);
            w.b(0, 1);
            w.b(0, 1);
        }
        w.b(0, 1); // bitstream restriction
        w.trailing()
    }

    #[test]
    fn sps_main() {
        let d = sps_bytes(1, 30, 64, 48, 1, 8, true);
        let sps = parse_sps(&d).expect("sps");
        assert_eq!((sps.width, sps.height), (64, 48));
        assert_eq!(sps.chroma_format_idc, 1);
        assert_eq!(sps.bit_depth_luma, 8);
        assert_eq!(sps.vui.timing, Some((1, 25)));
        assert_eq!(sps.vui.video_full_range, Some(false));
        let mut s = Stream::new(StreamKind::Video);
        apply(&mut s, &sps, false);
        assert_eq!(s.get("Format_Profile"), "Main@L1@Main");
        assert_eq!(s.get("FrameRate"), "25.000");
        assert_eq!(s.get("ChromaSubsampling"), "4:2:0");
        assert_eq!(s.get("colour_range"), "Limited");
        assert_eq!(s.get("colour_description_present"), "");
    }

    #[test]
    fn sps_main10_422() {
        let d = sps_bytes(2, 93, 1920, 1080, 2, 10, false);
        let sps = parse_sps(&d).expect("sps");
        assert_eq!((sps.width, sps.height), (1920, 1080));
        let mut s = Stream::new(StreamKind::Video);
        apply(&mut s, &sps, true);
        assert_eq!(s.get("Format_Profile"), "Main 10@L3.1@Main");
        assert_eq!(s.get("BitDepth"), "10");
        assert_eq!(s.get("ChromaSubsampling"), "4:2:2");
        assert_eq!(s.get("FrameRate"), "");
        // Truncated input never panics.
        for n in 0..d.len() {
            let _ = parse_sps(&d[..n]);
        }
        assert!(parse_sps(&[]).is_none());
    }

    #[test]
    fn names() {
        assert_eq!(level_name(30), "1");
        assert_eq!(level_name(93), "3.1");
        assert_eq!(level_name(153), "5.1");
        assert_eq!(level_name(186), "6.2");
        let rext = |bits: u64| ProfileTierLevel { profile_idc: 4, constraints: bits << (43 - 9), ..Default::default() };
        assert_eq!(profile_name(&rext(0b110100001)), "Main 4:2:2 10");
        assert_eq!(profile_name(&rext(0b100000001)), "Main 4:4:4 12");
        assert_eq!(profile_name(&rext(0b111111001)), "Monochrome");
        assert_eq!(profile_name(&rext(0b111000100)), "Main 4:4:4 Intra");
        assert_eq!(profile_name(&rext(0b111000110)), "Main 4:4:4 Still Picture");
        assert_eq!(profile_name(&ProfileTierLevel { profile_idc: 0, compatibility: 1 << 29, ..Default::default() }), "Main 10");
    }

    #[test]
    fn sei_and_hvcc() {
        let mut s = Stream::new(StreamKind::Video);
        apply_sei(&mut s, "x265 (build 217) - 4.3:[Linux][GCC 16.1.1][64 bit] 8bit+10bit+12bit - H.265/HEVC codec - Copyright 2013-2018 (c) Multicoreware, Inc - http://x265.org - options: cpuid=1 bitdepth=8 input-csp=1 fps=25/1 rc=crf");
        assert_eq!(s.get("Encoded_Library"), "x265 - 4.3:[Linux][GCC 16.1.1][64 bit] 8bit+10bit+12bit");
        assert_eq!(s.get("Encoded_Library_Settings"), "cpuid=1 / input-csp=1 / rc=crf");

        let sps = sps_bytes(1, 60, 64, 48, 1, 8, false);
        let mut hvcc = vec![1, 1, 0x60, 0, 0, 0, 0x90, 0, 0, 0, 0, 0, 60, 0xF0, 0, 0xFC, 0xFD, 0xF8, 0xF8, 0, 0, 0x0F, 1];
        hvcc.push(0x80 | NAL_SPS);
        hvcc.extend_from_slice(&[0, 1]);
        hvcc.extend_from_slice(&((sps.len() + 2) as u16).to_be_bytes());
        hvcc.extend_from_slice(&[NAL_SPS << 1, 1]);
        hvcc.extend_from_slice(&sps);
        assert_eq!(hvcc_length_size(&hvcc), Some(4));
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_hvcc(&mut s, &hvcc));
        assert_eq!(s.get("Format_Profile"), "Main@L2@Main");
        assert_eq!(s.get("Width"), "64");
        assert!(!apply_hvcc(&mut Stream::new(StreamKind::Video), &[0, 1, 2]));
        // Record without SPS array: summary fields only.
        let mut s = Stream::new(StreamKind::Video);
        let mut short = hvcc[..23].to_vec();
        short[22] = 0;
        assert!(apply_hvcc(&mut s, &short));
        assert_eq!(s.get("Format_Profile"), "Main@L2@Main");
        assert_eq!(s.get("BitDepth"), "8");
    }

    #[test]
    fn nal_iteration() {
        let d = [0, 0, 0, 1, 0x40, 1, 0xAA, 0, 0, 1, 0x42, 1, 0xBB, 0xCC, 0, 0, 0, 1, 0x26, 1, 0x80];
        let n = nals_annexb(&d);
        assert_eq!(n.len(), 3);
        assert_eq!(n[0].0, NAL_VPS);
        assert_eq!(n[1], (NAL_SPS, &[0xBB, 0xCC][..]));
        assert_eq!(n[2].0, 19);
        assert!(first_slice_in_pic(n[2].0, n[2].1));
        let lp = [0, 0, 0, 3, 0x42, 1, 0xBB, 0, 0, 0, 2, 0x26, 1];
        let n = nals_length_prefixed(&lp, 4);
        assert_eq!(n.len(), 2);
        assert_eq!(n[0], (NAL_SPS, &[0xBB][..]));
        assert!(nals_length_prefixed(&[0, 0], 4).is_empty());
    }
}

//! AV1 (AV1 Bitstream & Decoding Process Specification §5): OBU syntax, sequence header and
//! metadata OBUs, the ISOBMFF `av1C` record (AV1 Codec ISO Media File Format Binding §2.3) and a
//! parser for low-overhead OBU streams (Annex B is not handled).

use crate::io::bits::BitReader;
use crate::io::Reader;
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

pub const OBU_SEQUENCE_HEADER: u8 = 1;
pub const OBU_TEMPORAL_DELIMITER: u8 = 2;
pub const OBU_FRAME_HEADER: u8 = 3;
pub const OBU_TILE_GROUP: u8 = 4;
pub const OBU_METADATA: u8 = 5;
pub const OBU_FRAME: u8 = 6;
pub const OBU_REDUNDANT_FRAME_HEADER: u8 = 7;
pub const OBU_TILE_LIST: u8 = 8;
pub const OBU_PADDING: u8 = 15;

#[derive(Debug, Clone, Default)]
pub struct SequenceHeader {
    pub profile: u8,
    pub still_picture: bool,
    pub reduced_still_picture_header: bool,
    pub level_idx: u8,
    pub tier: bool,
    pub timing: Option<(u32, u32, Option<u32>)>, // num_units_in_display_tick, time_scale, num_ticks_per_picture
    pub max_width: u32,
    pub max_height: u32,
    pub bit_depth: u8,
    pub mono_chrome: bool,
    pub colour: Option<(u8, u8, u8)>,
    pub color_range: bool, // full range
    pub subsampling_x: bool,
    pub subsampling_y: bool,
    pub chroma_sample_position: u8,
    pub film_grain: bool,
}

/// One OBU: (type, temporal id, spatial id, payload).
#[derive(Debug, Clone, Copy)]
pub struct Obu<'a> {
    pub obu_type: u8,
    pub temporal_id: u8,
    pub spatial_id: u8,
    pub payload: &'a [u8],
}

/// leb128 (§4.10.5): (value, bytes used).
pub fn leb128(d: &[u8]) -> Option<(u64, usize)> {
    let mut v: u64 = 0;
    for i in 0..8 {
        let b = *d.get(i)?;
        v |= ((b & 0x7F) as u64) << (i * 7);
        if b & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}

/// Split a low-overhead OBU stream (`obu_has_size_field` set on every OBU, as in ISOBMFF samples
/// and Matroska blocks). A final OBU without a size field takes the rest of the data.
pub fn obus(d: &[u8]) -> Vec<Obu<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < d.len() && out.len() < 1 << 16 {
        let h = d[i];
        if h & 0x80 != 0 {
            break; // forbidden bit
        }
        let obu_type = (h >> 3) & 0xF;
        let ext = h & 4 != 0;
        let has_size = h & 2 != 0;
        i += 1;
        let (mut temporal_id, mut spatial_id) = (0, 0);
        if ext {
            let Some(e) = d.get(i) else { break };
            temporal_id = e >> 5;
            spatial_id = (e >> 3) & 3;
            i += 1;
        }
        let size = if has_size {
            let Some((v, n)) = leb128(&d[i..]) else { break };
            i += n;
            v as usize
        } else {
            d.len() - i
        };
        if i + size > d.len() {
            break;
        }
        out.push(Obu { obu_type, temporal_id, spatial_id, payload: &d[i..i + size] });
        i += size;
    }
    out
}

fn color_config(r: &mut BitReader, s: &mut SequenceHeader) -> Option<()> {
    let high_bitdepth = r.bit()?;
    s.bit_depth = if s.profile == 2 && high_bitdepth {
        if r.bit()? { 12 } else { 10 }
    } else if high_bitdepth {
        10
    } else {
        8
    };
    s.mono_chrome = if s.profile == 1 { false } else { r.bit()? };
    let (p, t, m) = if r.bit()? { (r.u8(8)?, r.u8(8)?, r.u8(8)?) } else { (2, 2, 2) };
    s.colour = Some((p, t, m));
    if s.mono_chrome {
        s.color_range = r.bit()?;
        s.subsampling_x = true;
        s.subsampling_y = true;
        return Some(());
    }
    if p == 1 && t == 13 && m == 0 {
        s.color_range = true;
        s.subsampling_x = false;
        s.subsampling_y = false;
    } else {
        s.color_range = r.bit()?;
        match s.profile {
            0 => {
                s.subsampling_x = true;
                s.subsampling_y = true;
            }
            1 => {
                s.subsampling_x = false;
                s.subsampling_y = false;
            }
            _ => {
                if s.bit_depth == 12 {
                    s.subsampling_x = r.bit()?;
                    s.subsampling_y = if s.subsampling_x { r.bit()? } else { false };
                } else {
                    s.subsampling_x = true;
                    s.subsampling_y = false;
                }
            }
        }
        if s.subsampling_x && s.subsampling_y {
            s.chroma_sample_position = r.u8(2)?;
        }
    }
    r.bit()?; // separate_uv_delta_q
    Some(())
}

/// Parse a sequence header OBU payload (§5.5).
pub fn parse_sequence_header(d: &[u8]) -> Option<SequenceHeader> {
    let mut r = BitReader::new(d);
    let mut s = SequenceHeader { profile: r.u8(3)?, still_picture: r.bit()?, reduced_still_picture_header: r.bit()?, ..Default::default() };
    if s.profile > 2 {
        return None;
    }
    let mut decoder_model_info = false;
    let mut buffer_delay_length = 0;
    if s.reduced_still_picture_header {
        s.level_idx = r.u8(5)?;
    } else {
        if r.bit()? {
            let num_units = r.u32(32)?;
            let time_scale = r.u32(32)?;
            let ticks = if r.bit()? { Some(r.ue()? + 1) } else { None };
            s.timing = Some((num_units, time_scale, ticks));
            decoder_model_info = r.bit()?;
            if decoder_model_info {
                buffer_delay_length = r.u8(5)? as usize + 1;
                r.skip(32);
                r.skip(10);
            }
        }
        let initial_display_delay_present = r.bit()?;
        let op_count = r.u8(5)? as usize + 1;
        for i in 0..op_count {
            r.skip(12); // operating_point_idc
            let level = r.u8(5)?;
            let tier = if level > 7 { r.bit()? } else { false };
            if i == 0 {
                s.level_idx = level;
                s.tier = tier;
            }
            if decoder_model_info && r.bit()? {
                r.skip(buffer_delay_length * 2 + 1);
            }
            if initial_display_delay_present && r.bit()? {
                r.skip(4);
            }
        }
    }
    let wbits = r.u8(4)? as usize + 1;
    let hbits = r.u8(4)? as usize + 1;
    s.max_width = r.u32(wbits)? + 1;
    s.max_height = r.u32(hbits)? + 1;
    if !s.reduced_still_picture_header && r.bit()? {
        r.skip(7); // delta_frame_id_length_minus_2, additional_frame_id_length_minus_1
    }
    r.skip(3); // use_128x128_superblock, enable_filter_intra, enable_intra_edge_filter
    if !s.reduced_still_picture_header {
        r.skip(4); // interintra, masked compound, warped motion, dual filter
        let enable_order_hint = r.bit()?;
        if enable_order_hint {
            r.skip(2); // jnt_comp, ref_frame_mvs
        }
        let force_sct = if r.bit()? { 2 } else { r.u8(1)? };
        if force_sct > 0 && !r.bit()? {
            r.bit()?; // seq_force_integer_mv
        }
        if enable_order_hint {
            r.skip(3);
        }
    }
    r.skip(3); // enable_superres, enable_cdef, enable_restoration
    color_config(&mut r, &mut s)?;
    s.film_grain = r.bit()?;
    Some(s)
}

pub fn level_name(idx: u8) -> String {
    if idx > 23 {
        return String::new();
    }
    format!("{}.{}", 2 + (idx >> 2), idx & 3)
}

/// Fill a video stream from a sequence header. `container_frame_rate` suppresses FrameRate from
/// the timing info.
pub fn apply_sequence_header(s: &mut Stream, h: &SequenceHeader, container_frame_rate: bool) {
    s.set_if_empty("Format", "AV1");
    let profile = match h.profile {
        0 => "Main",
        1 => "High",
        _ => "Professional",
    };
    let level = level_name(h.level_idx);
    s.set_if_empty("Format_Profile", if level.is_empty() { profile.to_string() } else { format!("{profile}@L{level}") });
    if h.max_width > 0 && h.max_height > 0 {
        s.set_if_empty("Width", h.max_width.to_string());
        s.set_if_empty("Height", h.max_height.to_string());
    }
    if let Some((num, scale, Some(ticks))) = h.timing {
        if num > 0 && scale > 0 && ticks > 0 && !container_frame_rate {
            s.set_if_empty("FrameRate", format!("{:.3}", scale as f64 / (num as f64 * ticks as f64)));
        }
    }
    s.set_if_empty("ColorSpace", if h.mono_chrome { "Y" } else { "YUV" });
    if !h.mono_chrome {
        s.set_if_empty(
            "ChromaSubsampling",
            match (h.subsampling_x, h.subsampling_y) {
                (true, true) => "4:2:0",
                (true, false) => "4:2:2",
                _ => "4:4:4",
            },
        );
    }
    s.set_if_empty("BitDepth", h.bit_depth.to_string());
    super::colour::set_range(s, h.color_range, super::colour::STREAM);
    if let Some((p, t, m)) = h.colour {
        super::colour::set_description(s, p, t, m, super::colour::STREAM);
    }
}

/// `av1C` record: header fields plus the embedded configOBUs (sequence header).
pub fn apply_av1c(s: &mut Stream, d: &[u8]) -> bool {
    if d.len() < 4 || d[0] & 0x80 == 0 || d[0] & 0x7F != 1 {
        return false;
    }
    let config = &d[4..];
    if let Some(h) = obus(config).iter().find(|o| o.obu_type == OBU_SEQUENCE_HEADER).and_then(|o| parse_sequence_header(o.payload)) {
        apply_sequence_header(s, &h, true);
        return true;
    }
    // No configOBUs: use the record's summary fields.
    let profile = d[1] >> 5;
    let level_idx = d[1] & 0x1F;
    let (high_bitdepth, twelve_bit, mono, sub_x, sub_y) = (d[2] & 0x40 != 0, d[2] & 0x20 != 0, d[2] & 0x10 != 0, d[2] & 0x08 != 0, d[2] & 0x04 != 0);
    let h = SequenceHeader { profile, level_idx, tier: d[2] & 0x80 != 0, bit_depth: if high_bitdepth && twelve_bit { 12 } else if high_bitdepth { 10 } else { 8 }, mono_chrome: mono, subsampling_x: sub_x, subsampling_y: sub_y, ..Default::default() };
    s.set_if_empty("Format", "AV1");
    let profile_name = match h.profile {
        0 => "Main",
        1 => "High",
        _ => "Professional",
    };
    let level = level_name(h.level_idx);
    s.set_if_empty("Format_Profile", if level.is_empty() { profile_name.to_string() } else { format!("{profile_name}@L{level}") });
    s.set_if_empty("ColorSpace", if mono { "Y" } else { "YUV" });
    if !mono {
        s.set_if_empty("ChromaSubsampling", match (sub_x, sub_y) { (true, true) => "4:2:0", (true, false) => "4:2:2", _ => "4:4:4" });
    }
    s.set_if_empty("BitDepth", h.bit_depth.to_string());
    true
}

/// Fill a stream from the OBUs of a sample (sequence header). Frame rate is left to the container.
pub fn apply_obus(s: &mut Stream, d: &[u8]) -> bool {
    match obus(d).iter().find(|o| o.obu_type == OBU_SEQUENCE_HEADER).and_then(|o| parse_sequence_header(o.payload)) {
        Some(h) => {
            apply_sequence_header(s, &h, true);
            true
        }
        None => false,
    }
}

/// HDR metadata OBUs (§5.8): content light level and mastering display colour volume.
pub fn apply_obus_metadata(s: &mut Stream, d: &[u8]) {
    for o in obus(d).iter().filter(|o| o.obu_type == OBU_METADATA) {
        let Some((ty, n)) = leb128(o.payload) else { continue };
        let p = &o.payload[n..];
        match ty {
            1 => {
                let (Some(cll), Some(fall)) = (crate::io::be16(p, 0), crate::io::be16(p, 2)) else { continue };
                s.set("MaxCLL", format!("{cll} cd/m2"));
                s.set("MaxCLL_Source", super::colour::STREAM);
                s.set("MaxFALL", format!("{fall} cd/m2"));
                s.set("MaxFALL_Source", super::colour::STREAM);
            }
            2 => {
                if p.len() < 24 {
                    continue;
                }
                let c = |i: usize| crate::io::be16(p, i).unwrap_or(0) as f64 / 65536.0;
                let lum_max = crate::io::be32(p, 16).unwrap_or(0) as f64 / 256.0;
                let lum_min = crate::io::be32(p, 20).unwrap_or(0) as f64 / 16384.0;
                s.set("MasteringDisplay_ColorPrimaries", format!("R: x={:.6} y={:.6}, G: x={:.6} y={:.6}, B: x={:.6} y={:.6}, White point: x={:.6} y={:.6}", c(0), c(2), c(4), c(6), c(8), c(10), c(12), c(14)));
                s.set("MasteringDisplay_ColorPrimaries_Source", super::colour::STREAM);
                s.set("MasteringDisplay_Luminance", format!("min: {lum_min:.4} cd/m2, max: {lum_max:.0} cd/m2"));
                s.set("MasteringDisplay_Luminance_Source", super::colour::STREAM);
            }
            _ => {}
        }
    }
}

// ---- low-overhead OBU stream

/// Temporal units and sequence header of a low-overhead stream: (temporal unit count, header).
fn scan_stream(d: &[u8]) -> (u64, Option<SequenceHeader>) {
    let all = obus(d);
    let tus = all.iter().filter(|o| o.obu_type == OBU_TEMPORAL_DELIMITER).count() as u64;
    let header = all.iter().find(|o| o.obu_type == OBU_SEQUENCE_HEADER).and_then(|o| parse_sequence_header(o.payload));
    (tus, header)
}

/// The reference does not identify bare `.obu` streams (raw.obu is reported as an unknown file),
/// so this never claims the file; `parse` is complete for callers that want to opt in.
pub fn probe(p: &Probe) -> u8 {
    let _ = p;
    0
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let size = r.len();
    let cap = size.min(8 * 1024 * 1024) as usize;
    let data = r.read_vec_at(0, cap);
    if !data.starts_with(&[0x12, 0x00]) {
        return false;
    }
    let (mut tus, Some(h)) = scan_stream(&data) else { return false };
    let mut s = Stream::new(StreamKind::Video);
    apply_sequence_header(&mut s, &h, false);
    if cap < size as usize && tus > 0 {
        tus = (tus as f64 * size as f64 / cap as f64).round() as u64;
    }
    if tus > 0 {
        s.set("FrameCount", tus.to_string());
        if let Some(fps) = s.get_f64("FrameRate").filter(|f| *f > 0.0) {
            let dur = (tus as f64 / fps * 1000.0).round() as u64;
            s.set("Duration", dur.to_string());
            doc.general().set("Duration", dur.to_string());
        }
    }
    doc.general().set("Format", "AV1");
    doc.streams[StreamKind::Video as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::video::testutil::BitWriter;

    fn seq_header(profile: u8, level: u8, w: u32, h: u32, timing: bool, high_bitdepth: bool) -> Vec<u8> {
        let mut b = BitWriter::new();
        b.b(profile as u64, 3);
        b.b(0, 1); // still_picture
        b.b(0, 1); // reduced
        b.b(timing as u64, 1);
        if timing {
            b.b(1, 32);
            b.b(25, 32);
            b.b(1, 1); // equal_picture_interval
            b.ue(0); // num_ticks_per_picture_minus_1
            b.b(0, 1); // decoder_model_info_present
        }
        b.b(0, 1); // initial_display_delay_present
        b.b(0, 5); // one operating point
        b.b(0, 12);
        b.b(level as u64, 5);
        if level > 7 {
            b.b(0, 1);
        }
        b.b(15, 4);
        b.b(15, 4);
        b.b((w - 1) as u64, 16);
        b.b((h - 1) as u64, 16);
        b.b(0, 1); // frame_id_numbers_present
        b.b(0, 3);
        b.b(0, 4);
        b.b(1, 1); // enable_order_hint
        b.b(0, 2);
        b.b(1, 1); // seq_choose_screen_content_tools
        b.b(1, 1); // seq_choose_integer_mv
        b.b(6, 3); // order_hint_bits
        b.b(0, 3);
        // color_config
        b.b(high_bitdepth as u64, 1);
        if profile == 2 && high_bitdepth {
            b.b(0, 1);
        }
        if profile != 1 {
            b.b(0, 1); // mono_chrome
        }
        b.b(1, 1); // color_description_present
        b.b(1, 8);
        b.b(1, 8);
        b.b(1, 8);
        b.b(0, 1); // color_range
        if profile == 2 {
            if high_bitdepth {
                // 10-bit professional: 4:2:2 implied
            }
        }
        if profile == 0 {
            b.b(0, 2); // chroma_sample_position
        }
        b.b(0, 1); // separate_uv_delta_q
        b.b(0, 1); // film grain
        b.done()
    }

    fn obu(ty: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![(ty << 3) | 2];
        v.push(payload.len() as u8);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn sequence_header() {
        let d = seq_header(0, 0, 64, 48, true, false);
        let h = parse_sequence_header(&d).unwrap();
        assert_eq!((h.profile, h.level_idx), (0, 0));
        assert_eq!((h.max_width, h.max_height), (64, 48));
        assert_eq!(h.timing, Some((1, 25, Some(1))));
        assert_eq!(h.bit_depth, 8);
        assert!(h.subsampling_x && h.subsampling_y);
        assert_eq!(h.colour, Some((1, 1, 1)));
        let mut s = Stream::new(StreamKind::Video);
        apply_sequence_header(&mut s, &h, false);
        assert_eq!(s.get("Format_Profile"), "Main@L2.0");
        assert_eq!(s.get("FrameRate"), "25.000");
        assert_eq!(s.get("ChromaSubsampling"), "4:2:0");
        assert_eq!(s.get("colour_range"), "Limited");
        assert_eq!(s.get("colour_primaries"), "BT.709");
        let h = parse_sequence_header(&seq_header(2, 13, 1920, 1080, false, true)).unwrap();
        assert_eq!((h.profile, h.bit_depth), (2, 10));
        assert!(h.subsampling_x && !h.subsampling_y);
        assert_eq!(level_name(13), "5.1");
        for n in 0..d.len() {
            let _ = parse_sequence_header(&d[..n]);
        }
    }

    #[test]
    fn obus_and_av1c() {
        let sh = seq_header(1, 8, 320, 240, false, false);
        let mut d = obu(OBU_TEMPORAL_DELIMITER, &[]);
        d.extend(obu(OBU_SEQUENCE_HEADER, &sh));
        d.extend(obu(OBU_FRAME, &[1, 2, 3]));
        let list = obus(&d);
        assert_eq!(list.len(), 3);
        assert_eq!(list[1].obu_type, OBU_SEQUENCE_HEADER);
        assert_eq!(list[2].payload, &[1, 2, 3]);
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_obus(&mut s, &d));
        assert_eq!(s.get("Format_Profile"), "High@L4.0");
        assert_eq!(s.get("ChromaSubsampling"), "4:4:4");
        let mut av1c = vec![0x81, (1 << 5) | 8, 0, 0];
        av1c.extend(obu(OBU_SEQUENCE_HEADER, &sh));
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_av1c(&mut s, &av1c));
        assert_eq!(s.get("Width"), "320");
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_av1c(&mut s, &[0x81, 0x00, 0x4C, 0]));
        assert_eq!(s.get("Format_Profile"), "Main@L2.0");
        assert_eq!(s.get("BitDepth"), "10");
        assert!(!apply_av1c(&mut s, &[0, 0, 0, 0]));
        assert_eq!(leb128(&[0xE5, 0x8E, 0x26]), Some((624485, 3)));
        assert_eq!(leb128(&[0x80]), None);
    }

    #[test]
    fn metadata() {
        let mut cll = vec![1];
        cll.extend([0x03, 0xE8, 0x00, 0x64]);
        let mut d = obu(OBU_METADATA, &cll);
        let mut mdcv = vec![2];
        for v in [0.68f64, 0.32, 0.265, 0.69, 0.15, 0.06, 0.3127, 0.329] {
            mdcv.extend(((v * 65536.0).round() as u16).to_be_bytes());
        }
        mdcv.extend((1000u32 * 256).to_be_bytes());
        mdcv.extend((82u32).to_be_bytes()); // 0.005 cd/m2
        d.extend(obu(OBU_METADATA, &mdcv));
        let mut s = Stream::new(StreamKind::Video);
        apply_obus_metadata(&mut s, &d);
        assert_eq!(s.get("MaxCLL"), "1000 cd/m2");
        assert_eq!(s.get("MaxFALL"), "100 cd/m2");
        assert!(s.get("MasteringDisplay_ColorPrimaries").starts_with("R: x=0.6799"), "{}", s.get("MasteringDisplay_ColorPrimaries"));
        assert_eq!(s.get("MasteringDisplay_Luminance"), "min: 0.0050 cd/m2, max: 1000 cd/m2");
    }

    #[test]
    fn stream() {
        let sh = seq_header(0, 0, 64, 48, true, false);
        let mut d = Vec::new();
        for _ in 0..5 {
            d.extend(obu(OBU_TEMPORAL_DELIMITER, &[]));
            d.extend(obu(OBU_SEQUENCE_HEADER, &sh));
            d.extend(obu(OBU_FRAME, &[0x10, 0, 0]));
        }
        assert_eq!(probe(&Probe { head: &d, ext: "obu", size: d.len() as u64 }), 0);
        let mut r = Reader::from_bytes(d);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("FrameCount"), "5");
        assert_eq!(v.get("Duration"), "200");
    }
}

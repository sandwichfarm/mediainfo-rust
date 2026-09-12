//! AAC: AudioSpecificConfig (ISO/IEC 14496-3 §1.6), ADTS and LATM/LOAS elementary streams.

use crate::io::bits::BitReader;
use crate::io::Reader;
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

/// Sampling frequencies indexed by samplingFrequencyIndex.
pub const SAMPLING_RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];

pub fn sampling_rate(index: u8) -> Option<u32> {
    SAMPLING_RATES.get(index as usize).copied()
}

/// channelConfiguration → number of channels.
pub fn channels_for_config(cc: u8) -> Option<u32> {
    Some(match cc {
        1..=6 => cc as u32,
        7 => 8,
        11 => 7,
        12 | 14 => 8,
        _ => return None,
    })
}

/// Decoded AudioSpecificConfig.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Asc {
    /// Object type as signalled first (5/29 for hierarchical HE-AAC signalling).
    pub aot: u8,
    /// Core object type (2 for LC even when SBR is signalled).
    pub base_aot: u8,
    pub sampling_rate: u32,
    pub channel_config: u8,
    pub channels: u32,
    pub frame_length_flag: bool,
    /// `None` = no signalling, `Some(true/false)` = explicit.
    pub sbr: Option<bool>,
    pub ps: Option<bool>,
    /// Explicit hierarchical signalling (audioObjectType 5/29).
    pub sbr_nbc: bool,
    pub ext_sampling_rate: u32,
}

fn object_type(r: &mut BitReader) -> Option<u8> {
    let t = r.u8(5)?;
    if t == 31 {
        Some(32 + r.u8(6)?)
    } else {
        Some(t)
    }
}

fn sampling_frequency(r: &mut BitReader) -> Option<u32> {
    let idx = r.u8(4)?;
    if idx == 0xF {
        r.u32(24)
    } else {
        sampling_rate(idx)
    }
}

/// program_config_element(): returns the channel count.
fn program_config_element(r: &mut BitReader) -> Option<u32> {
    r.skip(4 + 2 + 4); // element_instance_tag, object_type, sampling_frequency_index
    let num_front = r.u8(4)? as usize;
    let num_side = r.u8(4)? as usize;
    let num_back = r.u8(4)? as usize;
    let num_lfe = r.u8(2)? as u32;
    let num_assoc = r.u8(3)? as usize;
    let num_cc = r.u8(4)? as usize;
    if r.bit()? {
        r.skip(4); // mono_mixdown
    }
    if r.bit()? {
        r.skip(4); // stereo_mixdown
    }
    if r.bit()? {
        r.skip(3); // matrix_mixdown_idx + pseudo_surround
    }
    let mut channels = 0u32;
    for _ in 0..num_front + num_side + num_back {
        let cpe = r.bit()?;
        r.skip(4);
        channels += if cpe { 2 } else { 1 };
    }
    r.skip(4 * num_lfe as usize + 4 * num_assoc + 5 * num_cc);
    Some(channels + num_lfe)
}

/// Parse an AudioSpecificConfig from a bit reader; `end` is the bit position where the config
/// ends (used for the backward-compatible SBR/PS sync extension check).
pub fn parse_asc_bits(r: &mut BitReader, end: usize) -> Option<Asc> {
    let mut a = Asc { aot: object_type(r)?, ..Default::default() };
    a.base_aot = a.aot;
    a.sampling_rate = sampling_frequency(r)?;
    a.channel_config = r.u8(4)?;
    a.ext_sampling_rate = a.sampling_rate;
    let mut ext_aot = 0u8;
    if a.aot == 5 || a.aot == 29 {
        ext_aot = 5;
        a.sbr = Some(true);
        a.sbr_nbc = true;
        if a.aot == 29 {
            a.ps = Some(true);
        }
        a.ext_sampling_rate = sampling_frequency(r)?;
        a.base_aot = object_type(r)?;
        if a.base_aot == 22 {
            r.skip(4); // extensionChannelConfiguration
        }
    }
    a.channels = channels_for_config(a.channel_config).unwrap_or(0);
    match a.base_aot {
        1 | 2 | 3 | 4 | 6 | 7 | 17 | 19 | 20 | 21 | 22 | 23 => {
            // GASpecificConfig
            a.frame_length_flag = r.bit()?;
            if r.bit()? {
                r.skip(14); // coreCoderDelay
            }
            let extension_flag = r.bit()?;
            if a.channel_config == 0 {
                a.channels = program_config_element(r).unwrap_or(0);
            }
            if a.base_aot == 6 || a.base_aot == 20 {
                r.skip(3); // layerNr
            }
            if extension_flag {
                if a.base_aot == 22 {
                    r.skip(5 + 11);
                }
                if matches!(a.base_aot, 17 | 19 | 20 | 23) {
                    r.skip(3);
                }
                r.skip(1); // extensionFlag3
            }
        }
        39 => {
            // ELDSpecificConfig: frameLengthFlag is the first bit; the rest is not needed.
            a.frame_length_flag = r.bit()?;
        }
        _ => {}
    }
    // Backward-compatible explicit SBR/PS signalling.
    if ext_aot != 5 && end.saturating_sub(r.bit_pos()) >= 16 {
        let sync = r.u16(11)?;
        if sync == 0x2B7 {
            let t = object_type(r)?;
            if t == 5 {
                let present = r.bit()?;
                a.sbr = Some(present);
                if present {
                    a.ext_sampling_rate = sampling_frequency(r)?;
                    if end.saturating_sub(r.bit_pos()) >= 12 {
                        let sync2 = r.u16(11)?;
                        if sync2 == 0x548 {
                            a.ps = Some(r.bit()?);
                        }
                    }
                }
            } else if t == 22 {
                let present = r.bit()?;
                a.sbr = Some(present);
                if present {
                    a.ext_sampling_rate = sampling_frequency(r)?;
                }
                r.skip(4);
            }
        }
    }
    Some(a)
}

/// Parse an AudioSpecificConfig held in a byte slice.
pub fn parse_asc(data: &[u8]) -> Option<Asc> {
    let mut r = BitReader::new(data);
    parse_asc_bits(&mut r, data.len() * 8)
}

/// Profile name used in `Format_AdditionalFeatures`.
pub fn profile_name(aot: u8) -> &'static str {
    match aot {
        1 => "Main",
        2 => "LC",
        3 => "SSR",
        4 => "LTP",
        6 => "Scalable",
        7 => "TwinVQ",
        17 => "ER-LC",
        19 => "ER-LTP",
        20 => "ER-Scalable",
        23 => "LD",
        39 => "ELD",
        42 => "USAC",
        _ => "",
    }
}

/// Fill the profile family fields (`Format`, `Format_AdditionalFeatures`, `Format/Info`).
pub fn apply_profile(s: &mut Stream, base_aot: u8, sbr: bool, ps: bool) {
    s.set_if_empty("Format", "AAC");
    let profile = profile_name(base_aot);
    if profile.is_empty() {
        return;
    }
    let mut features = profile.to_string();
    let mut info = match base_aot {
        2 => "Advanced Audio Codec Low Complexity".to_string(),
        1 => "Advanced Audio Codec Main".to_string(),
        3 => "Advanced Audio Codec Scalable Sample Rate".to_string(),
        4 => "Advanced Audio Codec Long Term Prediction".to_string(),
        23 => "Advanced Audio Codec Low Delay".to_string(),
        39 => "Advanced Audio Codec Enhanced Low Delay".to_string(),
        _ => String::new(),
    };
    if sbr {
        features.push_str(" SBR");
        if !info.is_empty() {
            info.push_str(" with Spectral Band Replication");
        }
        if ps {
            features.push_str(" PS");
            if !info.is_empty() {
                info.push_str(" and Parametric Stereo");
            }
        }
    }
    s.set("Format_AdditionalFeatures", features);
    if !info.is_empty() {
        s.set("Format/Info", info);
    }
}

/// Channel count, positions and layout for a plain count.
fn apply_channels(s: &mut Stream, channels: u32) {
    if channels == 0 {
        return;
    }
    s.set("Channel(s)", channels.to_string());
    let (pos, layout) = super::layout_for_count(channels);
    if !pos.is_empty() {
        s.set("ChannelPositions", pos);
        s.set("ChannelLayout", layout);
    }
}

/// Fill the stream from a decoded config (shared by ASC and LATM).
fn apply_config(s: &mut Stream, a: &Asc) {
    let sbr = a.sbr == Some(true);
    let ps = a.ps == Some(true);
    apply_profile(s, a.base_aot, sbr, ps);
    match a.sbr {
        Some(true) => s.set("Format_Settings_SBR", if a.sbr_nbc { "Yes (NBC)" } else { "Yes (Explicit)" }),
        Some(false) => s.set("Format_Settings_SBR", "No (Explicit)"),
        None => {}
    }
    match a.ps {
        Some(true) => s.set("Format_Settings_PS", if a.aot == 29 { "Yes (NBC)" } else { "Yes (Explicit)" }),
        Some(false) => s.set("Format_Settings_PS", "No (Explicit)"),
        None => {}
    }
    let rate = if sbr && a.ext_sampling_rate > 0 { a.ext_sampling_rate } else { a.sampling_rate };
    if rate > 0 {
        s.set("SamplingRate", rate.to_string());
    }
    let mut channels = a.channels;
    if ps && channels == 1 {
        channels = 2;
    }
    apply_channels(s, channels);
    let mut spf: u32 = match a.base_aot {
        23 | 39 => if a.frame_length_flag { 480 } else { 512 },
        _ => if a.frame_length_flag { 960 } else { 1024 },
    };
    if sbr {
        spf *= 2;
    }
    s.set("SamplesPerFrame", spf.to_string());
    s.set_if_empty("Compression_Mode", "Lossy");
}

/// Fill from an AudioSpecificConfig; returns the (first signalled) audio object type.
pub fn apply_asc(s: &mut Stream, data: &[u8]) -> Option<u8> {
    let a = parse_asc(data)?;
    if a.sampling_rate == 0 {
        return None;
    }
    apply_config(s, &a);
    Some(a.aot)
}

// ---------------------------------------------------------------------------- ADTS

/// One ADTS frame header (ISO/IEC 14496-3 §1.A.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdtsHeader {
    pub mpeg2: bool,
    pub protection_absent: bool,
    pub profile: u8,
    pub sampling_index: u8,
    pub channel_config: u8,
    pub frame_length: usize,
    pub buffer_fullness: u16,
    pub raw_blocks: u8,
}

impl AdtsHeader {
    pub fn header_len(&self) -> usize {
        if self.protection_absent { 7 } else { 9 }
    }
}

/// Parse an ADTS header at the start of `d`.
pub fn parse_adts_header(d: &[u8]) -> Option<AdtsHeader> {
    if d.len() < 7 || d[0] != 0xFF || d[1] & 0xF6 != 0xF0 {
        return None;
    }
    let h = AdtsHeader {
        mpeg2: d[1] & 0x08 != 0,
        protection_absent: d[1] & 0x01 != 0,
        profile: d[2] >> 6,
        sampling_index: (d[2] >> 2) & 0x0F,
        channel_config: ((d[2] & 1) << 2) | (d[3] >> 6),
        frame_length: (((d[3] & 0x03) as usize) << 11) | ((d[4] as usize) << 3) | ((d[5] as usize) >> 5),
        buffer_fullness: (((d[5] & 0x1F) as u16) << 6) | ((d[6] as u16) >> 2),
        raw_blocks: d[6] & 0x03,
    };
    if h.sampling_index >= 13 || h.frame_length < h.header_len() {
        return None;
    }
    Some(h)
}

/// Size of an ID3v2 tag at the start of a buffer (0 when absent).
fn id3v2_len(d: &[u8]) -> usize {
    if d.len() >= 10 && &d[..3] == b"ID3" && (d[6] | d[7] | d[8] | d[9]) & 0x80 == 0 {
        let size = ((d[6] as usize) << 21) | ((d[7] as usize) << 14) | ((d[8] as usize) << 7) | d[9] as usize;
        10 + size + if d[5] & 0x10 != 0 { 10 } else { 0 }
    } else {
        0
    }
}

/// Number of consecutive valid ADTS frames from `pos` (capped).
fn adts_chain(d: &[u8], mut pos: usize, max: usize) -> usize {
    let mut n = 0;
    while n < max {
        let Some(h) = d.get(pos..).and_then(parse_adts_header) else { break };
        n += 1;
        pos += h.frame_length;
        if pos >= d.len() {
            break;
        }
    }
    n
}

pub fn probe_adts(p: &Probe) -> u8 {
    let start = id3v2_len(p.head);
    let ext = p.ext_in(&["aac", "adts", "aacp"]);
    if start >= p.head.len() {
        return if ext { 20 } else { 0 };
    }
    let n = adts_chain(p.head, start, 3);
    match (n, ext) {
        (3.., true) => 95,
        (3.., false) => 70,
        (2, true) => 60,
        (2, false) => 30,
        (_, true) => 20,
        _ => 0,
    }
}

const SCAN: usize = 4 * 1024 * 1024;

pub fn parse_adts(r: &mut Reader, doc: &mut Doc) -> bool {
    let size = r.len();
    let data = r.read_vec_at(0, (size as usize).min(SCAN));
    let start = id3v2_len(&data);
    let Some(first) = data.get(start..).and_then(parse_adts_header) else { return false };
    let Some(rate) = sampling_rate(first.sampling_index) else { return false };
    // Walk frames.
    let mut pos = start;
    let mut frames = 0u64;
    let mut sizes_vary = false;
    let mut fullness_vbr = false;
    let mut bytes = 0u64;
    while let Some(h) = data.get(pos..).and_then(parse_adts_header) {
        if h.frame_length != first.frame_length {
            sizes_vary = true;
        }
        if h.buffer_fullness == 0x7FF {
            fullness_vbr = true;
        }
        frames += 1;
        bytes += h.frame_length as u64;
        pos += h.frame_length;
        if pos >= data.len() || frames > 4_000_000 {
            break;
        }
    }
    if frames < 1 {
        return false;
    }
    let mut s = Stream::new(StreamKind::Audio);
    let aot = first.profile + 1;
    apply_profile(&mut s, aot, false, false);
    s.set("Format_Version", if first.mpeg2 { "Version 2" } else { "Version 4" });
    s.set("CodecID", aot.to_string());
    s.set("SamplingRate", rate.to_string());
    apply_channels(&mut s, channels_for_config(first.channel_config).unwrap_or(0));
    let spf = 1024 * (first.raw_blocks as u32 + 1);
    s.set("SamplesPerFrame", spf.to_string());
    let vbr = sizes_vary || fullness_vbr;
    s.set("BitRate_Mode", if vbr { "VBR" } else { "CBR" });
    if !vbr {
        let br = (bytes as f64 * 8.0 * rate as f64 / (frames as f64 * spf as f64)).round() as u64;
        s.set("BitRate", br.to_string());
    }
    s.set("Compression_Mode", "Lossy");
    s.set("StreamSize", (size - start as u64).to_string());
    let g = doc.general();
    g.set("Format", "ADTS");
    g.set("OverallBitRate_Mode", if vbr { "VBR" } else { "CBR" });
    g.set("StreamSize", start.to_string());
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

// ---------------------------------------------------------------------------- LATM / LOAS

/// LOAS AudioSyncStream header: returns the audioMuxLengthBytes.
fn loas_len(d: &[u8]) -> Option<usize> {
    if d.len() < 3 || d[0] != 0x56 || d[1] & 0xE0 != 0xE0 {
        return None;
    }
    let len = (((d[1] & 0x1F) as usize) << 8) | d[2] as usize;
    if len == 0 {
        None
    } else {
        Some(len)
    }
}

fn loas_chain(d: &[u8], mut pos: usize, max: usize) -> usize {
    let mut n = 0;
    while n < max {
        let Some(len) = d.get(pos..).and_then(loas_len) else { break };
        n += 1;
        pos += 3 + len;
        if pos >= d.len() {
            break;
        }
    }
    n
}

pub fn probe_latm(p: &Probe) -> u8 {
    let n = loas_chain(p.head, 0, 3);
    let ext = p.ext_in(&["latm", "loas"]);
    match (n, ext) {
        (3.., true) => 95,
        (3.., false) => 65,
        (2, true) => 60,
        (2, false) => 25,
        (_, true) => 20,
        _ => 0,
    }
}

fn latm_get_value(r: &mut BitReader) -> Option<u64> {
    let n = r.u8(2)? as usize + 1;
    r.bits(n * 8)
}

/// StreamMuxConfig: returns the first layer's AudioSpecificConfig and the latmBufferFullness of
/// the first layer (255 = VBR).
pub fn parse_stream_mux_config(r: &mut BitReader) -> Option<(Asc, Option<u8>)> {
    let version = r.bit()?;
    let version_a = if version { r.bit()? } else { false };
    if version_a {
        return None;
    }
    if version {
        latm_get_value(r)?; // taraBufferFullness
    }
    let _all_same = r.bit()?;
    let _num_sub_frames = r.u8(6)?;
    let _num_program = r.u8(4)?;
    let _num_layer = r.u8(3)?;
    // First program / first layer: useSameConfig is implicit 0.
    let asc = if version {
        let len = latm_get_value(r)? as usize;
        let start = r.bit_pos();
        let a = parse_asc_bits(r, start + len)?;
        r.seek_bits(start + len);
        a
    } else {
        let start = r.bit_pos();
        parse_asc_bits(r, start)?
    };
    let flt = r.u8(3)?;
    let fullness = if flt == 0 { Some(r.u8(8)?) } else { None };
    Some((asc, fullness))
}

pub fn parse_latm(r: &mut Reader, doc: &mut Doc) -> bool {
    let size = r.len();
    let data = r.read_vec_at(0, (size as usize).min(SCAN));
    let mut pos = 0usize;
    let mut asc: Option<Asc> = None;
    let mut fullness_vbr = false;
    let mut first_len = None;
    let mut sizes_vary = false;
    let mut frames = 0u64;
    while let Some(len) = data.get(pos..).and_then(loas_len) {
        let payload = data.get(pos + 3..(pos + 3 + len).min(data.len())).unwrap_or(&[]);
        if let Some(&b) = payload.first() {
            let use_same = b & 0x80 != 0;
            if !use_same && asc.is_none() {
                let mut br = BitReader::new(payload);
                br.skip(1);
                if let Some((a, fullness)) = parse_stream_mux_config(&mut br) {
                    if fullness == Some(0xFF) {
                        fullness_vbr = true;
                    }
                    asc = Some(a);
                }
            }
        }
        match first_len {
            None => first_len = Some(len),
            Some(f) if f != len => sizes_vary = true,
            _ => {}
        }
        frames += 1;
        pos += 3 + len;
        if pos >= data.len() || frames > 4_000_000 {
            break;
        }
    }
    let Some(a) = asc else { return false };
    if a.sampling_rate == 0 {
        return false;
    }
    let mut s = Stream::new(StreamKind::Audio);
    apply_config(&mut s, &a);
    s.set("CodecID", a.aot.to_string());
    let vbr = fullness_vbr || sizes_vary;
    s.set("BitRate_Mode", if vbr { "VBR" } else { "CBR" });
    let g = doc.general();
    g.set("Format", "LATM");
    g.set("OverallBitRate_Mode", if vbr { "VBR" } else { "CBR" });
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits_to_bytes(bits: &str) -> Vec<u8> {
        let bits: Vec<u8> = bits.bytes().filter(|c| *c == b'0' || *c == b'1').collect();
        bits.chunks(8)
            .map(|chunk| chunk.iter().enumerate().fold(0u8, |v, (i, c)| if *c == b'1' { v | (0x80 >> i) } else { v }))
            .collect()
    }

    #[test]
    fn asc_lc_mono() {
        // AOT 2, sfi 3 (48000), channel config 1, GASpecificConfig 000.
        let a = parse_asc(&[0x11, 0x88]).unwrap();
        assert_eq!(a.aot, 2);
        assert_eq!(a.sampling_rate, 48000);
        assert_eq!(a.channels, 1);
        assert_eq!(a.sbr, None);
        let mut s = Stream::new(StreamKind::Audio);
        assert_eq!(apply_asc(&mut s, &[0x11, 0x88]), Some(2));
        assert_eq!(s.get("Format_AdditionalFeatures"), "LC");
        assert_eq!(s.get("SamplesPerFrame"), "1024");
        assert_eq!(s.get("ChannelLayout"), "C");
        assert!(!s.has("Format_Settings_SBR"));
        assert!(parse_asc(&[0x11]).is_none());
        assert!(parse_asc(&[]).is_none());
    }

    #[test]
    fn asc_explicit_no_sbr() {
        // LC + sync extension 0x2B7, ext AOT 5, sbrPresentFlag 0.
        let mut s = Stream::new(StreamKind::Audio);
        assert_eq!(apply_asc(&mut s, &[0x11, 0x88, 0x56, 0xE5, 0x00]), Some(2));
        assert_eq!(s.get("Format_Settings_SBR"), "No (Explicit)");
        assert!(!s.has("Format_Settings_PS"));
        assert_eq!(s.get("Format/Info"), "Advanced Audio Codec Low Complexity");
    }

    #[test]
    fn asc_explicit_sbr_ps() {
        // LC 24000 stereo + ext: 0x2B7, AOT 5, sbr 1, ext sfi 4 (44100), 0x548, ps 1.
        let bytes = bits_to_bytes("00010 0110 0010 000 01010110111 00101 1 0100 10101001000 1");
        let mut s = Stream::new(StreamKind::Audio);
        assert_eq!(apply_asc(&mut s, &bytes), Some(2));
        assert_eq!(s.get("Format_AdditionalFeatures"), "LC SBR PS");
        assert_eq!(s.get("Format_Settings_SBR"), "Yes (Explicit)");
        assert_eq!(s.get("Format_Settings_PS"), "Yes (Explicit)");
        assert_eq!(s.get("SamplingRate"), "44100");
        assert_eq!(s.get("SamplesPerFrame"), "2048");
        assert_eq!(s.get("Format/Info"), "Advanced Audio Codec Low Complexity with Spectral Band Replication and Parametric Stereo");
    }

    #[test]
    fn asc_hierarchical() {
        // AOT 5, sfi 6 (24000), cc 2, ext sfi 4 (44100), AOT 2, GA 000.
        let bytes = bits_to_bytes("00101 0110 0010 0100 00010 000");
        let a = parse_asc(&bytes).unwrap();
        assert_eq!(a.aot, 5);
        assert_eq!(a.base_aot, 2);
        assert_eq!(a.ext_sampling_rate, 44100);
        assert!(a.sbr_nbc);
        let mut s = Stream::new(StreamKind::Audio);
        assert_eq!(apply_asc(&mut s, &bytes), Some(5));
        assert_eq!(s.get("Format_Settings_SBR"), "Yes (NBC)");
        assert_eq!(s.get("Channel(s)"), "2");
    }

    #[test]
    fn asc_pce() {
        // AOT 2, sfi 3, cc 0, GA 000 + PCE: tag 0, object 0, sfi 3, front 1, side 0, back 0, lfe 1,
        // assoc 0, cc 0, no mixdowns, front element: cpe 1 tag 0, lfe tag 0.
        let bytes = bits_to_bytes("00010 0011 0000 000 0000 00 0011 0001 0000 0000 01 000 0000 0 0 0 1 0000 0000");
        let a = parse_asc(&bytes).unwrap();
        assert_eq!(a.channels, 3);
    }

    #[test]
    fn adts_header() {
        let h = parse_adts_header(&[0xFF, 0xF1, 0x4C, 0x40, 0x12, 0x7F, 0xFC]).unwrap();
        assert!(!h.mpeg2);
        assert!(h.protection_absent);
        assert_eq!(h.profile, 1);
        assert_eq!(h.sampling_index, 3);
        assert_eq!(h.channel_config, 1);
        assert_eq!(h.frame_length, 147);
        assert_eq!(h.buffer_fullness, 0x7FF);
        assert_eq!(h.raw_blocks, 0);
        assert!(parse_adts_header(&[0xFF, 0xF1, 0x4C, 0x40, 0x00, 0x1F, 0xFC]).is_none()); // too short
        assert!(parse_adts_header(&[0xFF, 0xE1, 0x4C, 0x40, 0x12, 0x7F, 0xFC]).is_none()); // layer != 0
        assert!(parse_adts_header(&[0xFF, 0xF1, 0x4C]).is_none());
    }

    #[test]
    fn adts_probe_and_parse() {
        let mut d = Vec::new();
        for _ in 0..4 {
            d.extend_from_slice(&[0xFF, 0xF1, 0x4C, 0x40, 0x02, 0x00, 0x00]);
            d.extend_from_slice(&[0u8; 9]);
        }
        let p = Probe { head: &d, ext: "aac", size: d.len() as u64 };
        assert_eq!(probe_adts(&p), 95);
        let p = Probe { head: b"RIFF....", ext: "aac", size: 8 };
        assert_eq!(probe_adts(&p), 20);
        let mut r = Reader::from_bytes(d);
        let mut doc = Doc::new();
        assert!(parse_adts(&mut r, &mut doc));
        let s = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(s.get("Format"), "AAC");
        assert_eq!(s.get("Format_Version"), "Version 4");
        assert_eq!(s.get("CodecID"), "2");
        assert_eq!(s.get("BitRate_Mode"), "CBR");
        assert_eq!(s.get("BitRate"), "6000");
        assert_eq!(doc.general_ref().get("Format"), "ADTS");
    }

    #[test]
    fn latm_mux_config() {
        // From the fixture: 56 E0 93 | 20 00 11 88 1F E4 ...
        let payload = [0x20, 0x00, 0x11, 0x88, 0x1F, 0xE4, 0x66];
        let mut br = BitReader::new(&payload);
        assert!(!br.bit().unwrap()); // useSameStreamMux
        let (a, fullness) = parse_stream_mux_config(&mut br).unwrap();
        assert_eq!(a.aot, 2);
        assert_eq!(a.sampling_rate, 48000);
        assert_eq!(a.channels, 1);
        assert_eq!(a.sbr, None);
        assert_eq!(fullness, Some(0xFF));
        assert_eq!(loas_len(&[0x56, 0xE0, 0x93]), Some(0x93));
        assert_eq!(loas_len(&[0x56, 0xC0, 0x93]), None);
        assert!(parse_stream_mux_config(&mut BitReader::new(&[0x20])).is_none());
    }
}

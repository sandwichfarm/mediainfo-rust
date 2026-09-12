//! MLP (Meridian Lossless Packing, DVD-Audio) and Dolby TrueHD (MLP FBA) major sync headers.

use crate::io::{be16, be32, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

const SYNC_TRUEHD: u32 = 0xF872_6FBA;
const SYNC_MLP: u32 = 0xF872_6FBB;
const SIGNATURE: u16 = 0xB752;

/// Decoded major sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MajorSync {
    pub truehd: bool,
    pub sampling_rate: u32,
    /// Samples per access unit (sampling rate / 1200 for the 48 kHz family).
    pub samples_per_frame: u32,
    pub channels: u32,
    /// TrueHD: 8-channel (or 6-channel) presentation channel assignment bits; MLP: channel assignment code.
    pub channel_assignment: u16,
    /// MLP quantization word length of the first substream (bits).
    pub bit_depth: u32,
    pub variable_rate: bool,
    /// Peak data rate in bits per second.
    pub peak_bitrate: u32,
    pub substreams: u8,
}

fn sampling_rate_code(code: u8) -> Option<u32> {
    Some(match code {
        0 => 48000,
        1 => 96000,
        2 => 192000,
        8 => 44100,
        9 => 88200,
        10 => 176400,
        _ => return None,
    })
}

/// MLP channel assignment (DVD-Audio) → channel count.
pub fn mlp_channels(code: u16) -> u32 {
    const COUNTS: [u32; 21] = [1, 2, 3, 4, 3, 4, 5, 3, 4, 5, 4, 5, 6, 4, 5, 4, 5, 6, 5, 5, 6];
    COUNTS.get(code as usize).copied().unwrap_or(0)
}

/// TrueHD channel assignment bits → (count, ChannelPositions, ChannelLayout).
pub fn truehd_layout(bits: u16) -> (u32, String, String) {
    // (bit, layout names, group, group names)
    const TABLE: [(u16, &str, &str, &str); 13] = [
        (0x0001, "L R", "Front", "L R"),
        (0x0002, "C", "Front", "C"),
        (0x0004, "LFE", "LFE", ""),
        (0x0008, "Ls Rs", "Side", "L R"),
        (0x0010, "Tfl Tfr", "Top Front", "L R"),
        (0x0020, "Lc Rc", "Front", "Lc Rc"),
        (0x0040, "Lb Rb", "Back", "L R"),
        (0x0080, "Cb", "Back", "C"),
        (0x0100, "Tc", "Top", "C"),
        (0x0200, "Lsd Rsd", "Side", "Ld Rd"),
        (0x0400, "Lw Rw", "Front", "Lw Rw"),
        (0x0800, "Tfc", "Top Front", "C"),
        (0x1000, "LFE2", "LFE", ""),
    ];
    let mut count = 0;
    let mut layout: Vec<&str> = Vec::new();
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut lfe = 0;
    for (bit, names, group, gnames) in TABLE {
        if bits & bit == 0 {
            continue;
        }
        let n = names.split(' ').count() as u32;
        count += n;
        layout.extend(names.split(' '));
        if group == "LFE" {
            lfe += 1;
            continue;
        }
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, v)) => v.extend(gnames.split(' ')),
            None => groups.push((group, gnames.split(' ').collect())),
        }
    }
    // Front group reads "L C R" in the reference order.
    for (g, v) in groups.iter_mut() {
        if *g == "Front" && v.contains(&"C") {
            v.retain(|n| *n != "C");
            let at = v.iter().position(|n| *n == "R").unwrap_or(v.len());
            v.insert(at, "C");
        }
    }
    let mut pos: Vec<String> = groups.iter().map(|(g, v)| format!("{g}: {}", v.join(" "))).collect();
    if lfe > 0 {
        pos.push("LFE".to_string());
    }
    // Layout order: L R C LFE then the rest.
    let order = ["L", "R", "C", "LFE"];
    layout.sort_by_key(|n| order.iter().position(|o| o == n).unwrap_or(10));
    (count, pos.join(", "), layout.join(" "))
}

/// Parse a major sync starting at the `format_sync` word.
pub fn parse_major_sync(d: &[u8]) -> Option<MajorSync> {
    let sync = be32(d, 0)?;
    let truehd = match sync {
        SYNC_TRUEHD => true,
        SYNC_MLP => false,
        _ => return None,
    };
    let info = be32(d, 4)?;
    if be16(d, 8)? != SIGNATURE {
        return None;
    }
    // signature(16) flags(16) reserved(16) variable_rate(1) peak_data_rate(15) substreams(4)
    let rate_word = be16(d, 14)?;
    let variable_rate = rate_word & 0x8000 != 0;
    let peak = (rate_word & 0x7FFF) as u32;
    let substreams = d.get(16)? >> 4;
    if truehd {
        let sr_code = (info >> 28) as u8;
        let sampling_rate = sampling_rate_code(sr_code)?;
        let assign6 = ((info >> 15) & 0x1F) as u16;
        let assign8 = (info & 0x1FFF) as u16;
        let assignment = if assign8 != 0 { assign8 } else { assign6 };
        let (channels, _, _) = truehd_layout(assignment);
        Some(MajorSync { truehd, sampling_rate, samples_per_frame: 40 << (sr_code & 7), channels, channel_assignment: assignment, bit_depth: 0, variable_rate, peak_bitrate: peak * sampling_rate / 16, substreams })
    } else {
        let wl_code = (info >> 28) as u8;
        let sr_code = ((info >> 20) & 0xF) as u8;
        let sampling_rate = sampling_rate_code(sr_code)?;
        let assignment = (info & 0x1F) as u16;
        let bit_depth = match wl_code {
            0 => 16,
            1 => 20,
            2 => 24,
            _ => 0,
        };
        Some(MajorSync { truehd, sampling_rate, samples_per_frame: 40 << (sr_code & 7), channels: mlp_channels(assignment), channel_assignment: assignment, bit_depth, variable_rate, peak_bitrate: peak * sampling_rate / 16, substreams })
    }
}

/// Find a major sync inside an access unit / frame (it follows the 4-byte access unit header).
pub fn find_major_sync(d: &[u8]) -> Option<(usize, MajorSync)> {
    let limit = d.len().min(64 * 1024);
    for i in (0..limit.saturating_sub(21)).step_by(1) {
        if d[i] == 0xF8 && d[i + 1] == 0x72 && d[i + 2] == 0x6F && (d[i + 3] == 0xBA || d[i + 3] == 0xBB) {
            if let Some(m) = parse_major_sync(&d[i..]) {
                return Some((i, m));
            }
        }
    }
    None
}

fn apply_sync(s: &mut Stream, m: &MajorSync) {
    s.set_if_empty("Format", if m.truehd { "MLP FBA" } else { "MLP" });
    s.set("BitRate_Mode", if m.variable_rate { "VBR" } else { "CBR" });
    if m.peak_bitrate > 0 {
        s.set("BitRate_Maximum", m.peak_bitrate.to_string());
        if !m.variable_rate {
            s.set("BitRate", m.peak_bitrate.to_string());
        }
    }
    if m.channels > 0 {
        s.set("Channel(s)", m.channels.to_string());
    }
    if m.truehd {
        let (_, pos, layout) = truehd_layout(m.channel_assignment);
        if !pos.is_empty() {
            s.set("ChannelPositions", pos);
            s.set("ChannelLayout", layout);
        }
    }
    s.set("SamplesPerFrame", m.samples_per_frame.to_string());
    s.set("SamplingRate", m.sampling_rate.to_string());
    if m.bit_depth > 0 {
        s.set("BitDepth", m.bit_depth.to_string());
    }
}

/// Fill a stream from an access unit carrying a major sync (returns false when it carries none).
pub fn apply_frame(s: &mut Stream, d: &[u8]) -> bool {
    let Some((_, m)) = find_major_sync(d) else { return false };
    apply_sync(s, &m);
    true
}

/// Access unit header: length in bytes (0 when implausible).
fn access_unit_len(d: &[u8]) -> Option<usize> {
    let w = be16(d, 0)?;
    let len = ((w & 0x0FFF) as usize) * 2;
    if len < 4 {
        None
    } else {
        Some(len)
    }
}

/// Number of chained access units from `pos` (capped).
fn chain(d: &[u8], mut pos: usize, max: usize) -> usize {
    let mut n = 0;
    while n < max {
        let Some(len) = d.get(pos..).and_then(access_unit_len) else { break };
        n += 1;
        pos += len;
        if pos >= d.len() {
            break;
        }
    }
    n
}

pub fn probe(p: &Probe) -> u8 {
    let ext = p.ext_in(&["thd", "mlp", "truehd"]);
    let sync_at_start = p.head.get(4..).and_then(parse_major_sync).is_some();
    if sync_at_start {
        let n = chain(p.head, 0, 4);
        return match (n >= 4 || (n as u64) * 4 >= p.size.min(64), ext) {
            (true, true) => 95,
            (true, false) => 70,
            (false, true) => 60,
            (false, false) => 30,
        };
    }
    if ext && find_major_sync(p.head).is_some() {
        return 50;
    }
    0
}

const SCAN: usize = 4 * 1024 * 1024;

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let size = r.len();
    let data = r.read_vec_at(0, (size as usize).min(SCAN));
    let Some((sync_pos, m)) = find_major_sync(&data) else { return false };
    let mut s = Stream::new(StreamKind::Audio);
    apply_sync(&mut s, &m);
    // Walk access units from the one carrying the first major sync.
    let start = sync_pos.saturating_sub(4);
    let mut pos = start;
    let mut units = 0u64;
    while let Some(len) = data.get(pos..).and_then(access_unit_len) {
        units += 1;
        pos += len;
        if pos >= data.len() || units > 50_000_000 {
            break;
        }
    }
    // The reference stops counting access units after 1024 (and reports no duration).
    if units > 0 {
        s.set("FrameCount", units.min(1024).to_string());
    }
    let g = doc.general();
    g.set("Format", if m.truehd { "MLP FBA" } else { "MLP" });
    g.set("OverallBitRate_Mode", if m.variable_rate { "VBR" } else { "CBR" });
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // Access unit header + major sync from the fixtures.
    const THD: [u8; 32] = [0x80, 0x33, 0xFF, 0xD8, 0xF8, 0x72, 0x6F, 0xBA, 0x00, 0xF1, 0x60, 0x02, 0xB7, 0x52, 0x00, 0x00, 0x00, 0x00, 0x8C, 0x7F, 0x10, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    const MLP: [u8; 32] = [0x80, 0x33, 0xFF, 0xD8, 0xF8, 0x72, 0x6F, 0xBB, 0x0F, 0x0F, 0x00, 0x00, 0xB7, 0x52, 0x40, 0x00, 0x00, 0x00, 0x8C, 0x7F, 0x10, 0x05, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    #[test]
    fn truehd_sync() {
        let m = parse_major_sync(&THD[4..]).unwrap();
        assert!(m.truehd);
        assert_eq!(m.sampling_rate, 48000);
        assert_eq!(m.samples_per_frame, 40);
        assert_eq!(m.channels, 1);
        assert_eq!(m.channel_assignment, 2);
        assert!(m.variable_rate);
        assert_eq!(m.peak_bitrate, 9_597_000);
        assert_eq!(m.substreams, 1);
        assert!(parse_major_sync(&THD[4..16]).is_none());
        let mut bad = THD;
        bad[12] = 0;
        assert!(parse_major_sync(&bad[4..]).is_none());
    }

    #[test]
    fn mlp_sync() {
        let m = parse_major_sync(&MLP[4..]).unwrap();
        assert!(!m.truehd);
        assert_eq!(m.bit_depth, 16);
        assert_eq!(m.sampling_rate, 48000);
        assert_eq!(m.channels, 1);
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_frame(&mut s, &MLP));
        assert_eq!(s.get("Format"), "MLP");
        assert_eq!(s.get("BitRate_Maximum"), "9597000");
        assert_eq!(s.get("SamplesPerFrame"), "40");
        assert_eq!(s.get("BitRate_Mode"), "VBR");
        assert!(!s.has("ChannelLayout"));
        assert_eq!(mlp_channels(12), 6);
    }

    #[test]
    fn layouts() {
        let (n, pos, layout) = truehd_layout(0x000F);
        assert_eq!(n, 6);
        assert_eq!(pos, "Front: L C R, Side: L R, LFE");
        assert_eq!(layout, "L R C LFE Ls Rs");
        let (n, pos, layout) = truehd_layout(0x004F);
        assert_eq!(n, 8);
        assert_eq!(pos, "Front: L C R, Side: L R, Back: L R, LFE");
        assert_eq!(layout, "L R C LFE Ls Rs Lb Rb");
        assert_eq!(truehd_layout(0x0001).1, "Front: L R");
    }

    #[test]
    fn probe_and_parse() {
        let mut d = Vec::new();
        for i in 0..4 {
            if i == 0 {
                d.extend_from_slice(&THD);
                d.resize(102, 0);
            } else {
                d.extend_from_slice(&[0x80, 0x10, 0, 0]);
                d.extend_from_slice(&[0u8; 28]);
            }
        }
        let p = Probe { head: &d, ext: "thd", size: d.len() as u64 };
        assert_eq!(probe(&p), 95);
        let p = Probe { head: b"\0\0\0\0\0\0\0\0", ext: "thd", size: 8 };
        assert_eq!(probe(&p), 0);
        let mut r = Reader::from_bytes(d);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "MLP FBA");
        assert_eq!(doc.general_ref().get("OverallBitRate_Mode"), "VBR");
        let s = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(s.get("SamplingRate"), "48000");
        assert_eq!(s.get("Channel(s)"), "1");
        assert_eq!(s.get("FrameCount"), "4");
    }
}

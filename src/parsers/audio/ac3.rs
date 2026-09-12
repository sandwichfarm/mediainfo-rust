//! AC-3 (ATSC A/52 syncinfo/bsi) and E-AC-3 (A/52 Annex E) sync frames.

use crate::io::bits::BitReader;
use crate::io::Reader;
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

/// Decoded AC-3 / E-AC-3 frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub eac3: bool,
    /// E-AC-3 stream type (0 independent, 1 dependent, 2 AC-3 converted); 0 for AC-3.
    pub stream_type: u8,
    pub substream_id: u8,
    pub bsid: u8,
    pub bsmod: u8,
    pub acmod: u8,
    pub lfeon: bool,
    pub dialnorm: i32,
    pub compr: Option<u8>,
    pub dsurmod: Option<u8>,
    pub sampling_rate: u32,
    /// Frame size in bytes.
    pub frame_size: usize,
    /// Audio blocks per frame (6 for AC-3).
    pub blocks: u32,
    pub fscod: u8,
}

impl Frame {
    pub fn samples_per_frame(&self) -> u32 {
        256 * self.blocks
    }
    /// Bits per second.
    pub fn bitrate(&self) -> u32 {
        (self.frame_size as u64 * 8 * self.sampling_rate as u64 / self.samples_per_frame() as u64) as u32
    }
    pub fn channels(&self) -> u32 {
        let base = match self.acmod {
            0 => 2,
            1 => 1,
            2 => 2,
            3 => 3,
            4 => 3,
            5 => 4,
            6 => 4,
            _ => 5,
        };
        base + self.lfeon as u32
    }
}

const BITRATES: [u32; 19] = [32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 576, 640];

/// Frame size in 16-bit words for a (fscod, frmsizecod) pair (A/52 table 5.18).
pub fn ac3_frame_words(fscod: u8, frmsizecod: u8) -> Option<usize> {
    let kbps = *BITRATES.get((frmsizecod >> 1) as usize)? as usize;
    Some(match fscod {
        0 => kbps * 2,
        1 => {
            let base = (kbps * 1536 * 1000 / 44100) / 16;
            base + (frmsizecod & 1) as usize
        }
        2 => kbps * 3,
        _ => return None,
    })
}

/// Parse a sync frame header at the start of `d`.
pub fn parse_frame(d: &[u8]) -> Option<Frame> {
    if d.len() < 8 || d[0] != 0x0B || d[1] != 0x77 {
        return None;
    }
    let bsid = d[5] >> 3;
    let mut r = BitReader::new(&d[2..]);
    if bsid <= 10 {
        r.skip(16); // crc1
        let fscod = r.u8(2)?;
        let frmsizecod = r.u8(6)?;
        if fscod == 3 || frmsizecod >= 38 {
            return None;
        }
        let words = ac3_frame_words(fscod, frmsizecod)?;
        let bsid = r.u8(5)?;
        let bsmod = r.u8(3)?;
        let acmod = r.u8(3)?;
        if acmod & 1 != 0 && acmod != 1 {
            r.skip(2); // cmixlev
        }
        if acmod & 4 != 0 {
            r.skip(2); // surmixlev
        }
        let dsurmod = if acmod == 2 { Some(r.u8(2)?) } else { None };
        let lfeon = r.bit()?;
        let dialnorm = r.u8(5)? as i32;
        let compr = if r.bit()? { Some(r.u8(8)?) } else { None };
        let shift = match bsid {
            9 => 1,
            10 => 2,
            _ => 0,
        };
        let sampling_rate = [48000u32, 44100, 32000][fscod as usize] >> shift;
        Some(Frame { eac3: false, stream_type: 0, substream_id: 0, bsid, bsmod, acmod, lfeon, dialnorm: -dialnorm, compr, dsurmod, sampling_rate, frame_size: words * 2, blocks: 6, fscod })
    } else if bsid <= 16 {
        let stream_type = r.u8(2)?;
        let substream_id = r.u8(3)?;
        let frmsiz = r.u16(11)? as usize;
        let fscod = r.u8(2)?;
        let (sampling_rate, blocks) = if fscod == 3 {
            let fscod2 = r.u8(2)?;
            if fscod2 == 3 {
                return None;
            }
            ([24000u32, 22050, 16000][fscod2 as usize], 6)
        } else {
            let numblkscod = r.u8(2)?;
            ([48000u32, 44100, 32000][fscod as usize], [1u32, 2, 3, 6][numblkscod as usize])
        };
        let acmod = r.u8(3)?;
        let lfeon = r.bit()?;
        let bsid = r.u8(5)?;
        let dialnorm = r.u8(5)? as i32;
        let compr = if r.bit()? { Some(r.u8(8)?) } else { None };
        Some(Frame { eac3: true, stream_type, substream_id, bsid, bsmod: 0, acmod, lfeon, dialnorm: -dialnorm, compr, dsurmod: None, sampling_rate, frame_size: (frmsiz + 1) * 2, blocks, fscod })
    } else {
        None
    }
}

/// (ChannelPositions, ChannelLayout) for an acmod / lfeon pair.
pub fn channel_layout(acmod: u8, lfe: bool) -> (String, String) {
    let (pos, layout): (&str, &str) = match acmod {
        0 => ("Dual mono", "M M"),
        1 => ("Front: C", "C"),
        2 => ("Front: L R", "L R"),
        3 => ("Front: L C R", "L R C"),
        4 => ("Front: L R, Back: C", "L R Cb"),
        5 => ("Front: L C R, Back: C", "L R C Cb"),
        6 => ("Front: L R, Side: L R", "L R Ls Rs"),
        _ => ("Front: L C R, Side: L R", "L R C Ls Rs"),
    };
    if !lfe {
        return (pos.to_string(), layout.to_string());
    }
    let mut parts: Vec<&str> = layout.split(' ').collect();
    let at = parts.iter().position(|p| *p == "C").map(|i| i + 1).unwrap_or(2.min(parts.len()));
    parts.insert(at, "LFE");
    (format!("{pos}, LFE"), parts.join(" "))
}

pub fn service_kind(bsmod: u8, acmod: u8) -> &'static str {
    match bsmod {
        0 => "CM",
        1 => "ME",
        2 => "VI",
        3 => "HI",
        4 => "D",
        5 => "C",
        6 => "E",
        _ => if acmod == 1 { "VO" } else { "K" },
    }
}

fn service_kind_name(k: &str) -> &'static str {
    match k {
        "CM" => "Complete Main",
        "ME" => "Music and Effects",
        "VI" => "Visually Impaired",
        "HI" => "Hearing Impaired",
        "D" => "Dialogue",
        "C" => "Commentary",
        "E" => "Emergency",
        "VO" => "Voice Over",
        _ => "Karaoke",
    }
}

/// Update the dialnorm statistics kept on the stream (Average/Minimum/Maximum/Count).
fn dialnorm_stats(s: &mut Stream, dialnorm: i32) {
    let count = s.get_i64("dialnorm_Count").unwrap_or(0);
    let avg = s.get_f64("dialnorm_Average").unwrap_or(dialnorm as f64);
    let min = s.get_i64("dialnorm_Minimum").map(|v| v as i32).unwrap_or(dialnorm).min(dialnorm);
    let max = s.get_i64("dialnorm_Maximum").map(|v| v as i32).unwrap_or(dialnorm).max(dialnorm);
    let new_avg = if count == 0 { dialnorm as f64 } else { (avg * count as f64 + dialnorm as f64) / (count as f64 + 1.0) };
    let avg_text = format!("{}", new_avg.round() as i64);
    s.set_extra("dialnorm_Average", avg_text.clone(), "", "N NT");
    s.set_extra("dialnorm_Average/String", format!("{avg_text} dB"), "", "N NTN");
    s.set_extra("dialnorm_Minimum", min.to_string(), "", "N NT");
    s.set_extra("dialnorm_Minimum/String", format!("{min} dB"), "", "N NTN");
    s.set_extra("dialnorm_Maximum", max.to_string(), "", "N NTN");
    s.set_extra("dialnorm_Maximum/String", format!("{max} dB"), "", "N NTN");
    s.set_extra("dialnorm_Count", (count + 1).to_string(), "", "N NTN");
}

/// Fill a stream from one sync frame. Calling it for every frame accumulates the dialnorm
/// statistics; the static fields are set from the first frame only.
pub fn apply_frame(s: &mut Stream, d: &[u8]) -> bool {
    let Some(f) = parse_frame(d) else { return false };
    if f.eac3 && f.stream_type == 1 {
        return false; // dependent substream: describes extra channels only
    }
    if !s.has("bsid") {
        s.set_if_empty("Format", if f.eac3 { "E-AC-3" } else { "AC-3" });
        s.set("Format_Settings_Endianness", "Big");
        s.set("BitRate_Mode", "CBR");
        s.set("BitRate", f.bitrate().to_string());
        s.set("Channel(s)", f.channels().to_string());
        let (pos, layout) = channel_layout(f.acmod, f.lfeon);
        s.set("ChannelPositions", pos);
        s.set("ChannelLayout", layout);
        s.set("SamplesPerFrame", f.samples_per_frame().to_string());
        s.set("SamplingRate", f.sampling_rate.to_string());
        s.set_if_empty("Compression_Mode", "Lossy");
        let kind = service_kind(f.bsmod, f.acmod);
        s.set("ServiceKind", kind);
        s.set("ServiceKind/String", service_kind_name(kind));
        s.set_extra("bsid", f.bsid.to_string(), "", "N NT");
        s.set_extra("dialnorm", f.dialnorm.to_string(), "", "N NT");
        s.set_extra("dialnorm/String", format!("{} dB", f.dialnorm), "", "N NTN");
        if let Some(c) = f.compr {
            s.set_extra("compr", c.to_string(), "", "N NT");
        }
        s.set_extra("acmod", f.acmod.to_string(), "", "N NT");
        s.set_extra("lfeon", (f.lfeon as u8).to_string(), "", "N NT");
        if let Some(m) = f.dsurmod {
            s.set_extra("dsurmod", m.to_string(), "", "N NT");
        }
    }
    dialnorm_stats(s, f.dialnorm);
    true
}

/// Count consecutive valid frames from `pos` (capped).
fn chain(d: &[u8], mut pos: usize, max: usize) -> usize {
    let mut n = 0;
    while n < max {
        let Some(f) = d.get(pos..).and_then(parse_frame) else { break };
        n += 1;
        pos += f.frame_size;
        if pos >= d.len() {
            break;
        }
    }
    n
}

pub fn probe(p: &Probe) -> u8 {
    let ext = p.ext_in(&["ac3", "eac3", "ec3", "dd+", "ddp"]);
    let n = chain(p.head, 0, 3);
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

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let size = r.len();
    let data = r.read_vec_at(0, (size as usize).min(SCAN));
    let Some(first) = parse_frame(&data) else { return false };
    let mut s = Stream::new(StreamKind::Audio);
    let mut pos = 0usize;
    let mut frames = 0u64;
    let mut bytes = 0u64;
    let mut applied = false;
    while let Some(f) = data.get(pos..).and_then(parse_frame) {
        let own = f.eac3 == first.eac3 && f.stream_type != 1 && f.substream_id == 0;
        if own {
            if apply_frame(&mut s, &data[pos..]) {
                applied = true;
            }
            frames += 1;
        }
        bytes += f.frame_size as u64;
        pos += f.frame_size;
        if pos >= data.len() || frames > 50_000_000 {
            break;
        }
    }
    if !applied || frames == 0 {
        return false;
    }
    let spf = first.samples_per_frame() as f64;
    let sr = first.sampling_rate as f64;
    let mut total_frames = frames as f64;
    if (data.len() as u64) < size && bytes > 0 {
        total_frames = frames as f64 * size as f64 / bytes as f64;
    }
    let duration_ms = (total_frames * spf / sr * 1000.0).round();
    s.set("Duration", format!("{duration_ms}"));
    s.set("FrameCount", format!("{}", total_frames.round() as u64));
    s.set("StreamSize", size.to_string());
    let bitrate = s.get("BitRate").to_string();
    let g = doc.general();
    g.set("Format", if first.eac3 { "E-AC-3" } else { "AC-3" });
    g.set("Duration", format!("{duration_ms}"));
    g.set("OverallBitRate_Mode", "CBR");
    g.set("OverallBitRate", bitrate);
    g.set("StreamSize", "0");
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const AC3: [u8; 8] = [0x0B, 0x77, 0x6B, 0xF1, 0x08, 0x40, 0x2F, 0x84];
    const EAC3: [u8; 8] = [0x0B, 0x77, 0x00, 0x7F, 0x32, 0x87, 0xC0, 0x00];

    #[test]
    fn ac3_header() {
        let f = parse_frame(&AC3).unwrap();
        assert!(!f.eac3);
        assert_eq!((f.bsid, f.bsmod, f.acmod, f.lfeon), (8, 0, 1, false));
        assert_eq!(f.dialnorm, -31);
        assert_eq!(f.sampling_rate, 48000);
        assert_eq!(f.frame_size, 256);
        assert_eq!(f.bitrate(), 64000);
        assert_eq!(f.channels(), 1);
        assert_eq!(f.compr, None);
        assert!(parse_frame(&[0x0B, 0x77, 0, 0, 0xC0, 0x40, 0, 0]).is_none()); // fscod 3
        assert!(parse_frame(&AC3[..6]).is_none());
        assert_eq!(ac3_frame_words(1, 8), Some(139));
        assert_eq!(ac3_frame_words(1, 9), Some(140));
        assert_eq!(ac3_frame_words(2, 8), Some(192));
    }

    #[test]
    fn eac3_header() {
        let f = parse_frame(&EAC3).unwrap();
        assert!(f.eac3);
        assert_eq!((f.stream_type, f.substream_id, f.bsid), (0, 0, 16));
        assert_eq!(f.frame_size, 256);
        assert_eq!(f.blocks, 6);
        assert_eq!(f.samples_per_frame(), 1536);
        assert_eq!(f.bitrate(), 64000);
        assert_eq!(f.acmod, 1);
        assert_eq!(f.dialnorm, -31);
    }

    #[test]
    fn layouts() {
        assert_eq!(channel_layout(7, true), ("Front: L C R, Side: L R, LFE".to_string(), "L R C LFE Ls Rs".to_string()));
        assert_eq!(channel_layout(2, true), ("Front: L R, LFE".to_string(), "L R LFE".to_string()));
        assert_eq!(channel_layout(1, false), ("Front: C".to_string(), "C".to_string()));
        assert_eq!(service_kind(7, 1), "VO");
        assert_eq!(service_kind(7, 2), "K");
    }

    #[test]
    fn apply_accumulates_dialnorm() {
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_frame(&mut s, &AC3));
        assert_eq!(s.get("Format"), "AC-3");
        assert_eq!(s.get("BitRate"), "64000");
        assert_eq!(s.get("ServiceKind/String"), "Complete Main");
        assert_eq!(s.get("dialnorm"), "-31");
        assert_eq!(s.get("dialnorm_Count"), "1");
        let mut other = AC3;
        other[6] = 0x28; // dialnorm 16 -> -16
        other[7] = 0x04;
        assert!(apply_frame(&mut s, &other));
        assert_eq!(s.get("dialnorm_Count"), "2");
        assert_eq!(s.get("dialnorm_Minimum"), "-31");
        assert_eq!(s.get("dialnorm_Maximum"), "-16");
        assert_eq!(s.get("dialnorm_Average"), "-24");
        assert_eq!(s.get("dialnorm"), "-31"); // first frame
    }

    #[test]
    fn probe_and_parse() {
        let mut d = Vec::new();
        for _ in 0..4 {
            d.extend_from_slice(&AC3);
            d.extend_from_slice(&[0u8; 248]);
        }
        let p = Probe { head: &d, ext: "ac3", size: d.len() as u64 };
        assert_eq!(probe(&p), 95);
        let mut r = Reader::from_bytes(d);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let s = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(s.get("Duration"), "128");
        assert_eq!(s.get("dialnorm_Count"), "4");
        assert_eq!(doc.general_ref().get("OverallBitRate"), "64000");
        assert_eq!(doc.general_ref().get("Format"), "AC-3");
    }
}

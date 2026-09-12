//! DTS core (ETSI TS 102 114 frame header) and DTS-HD extension substream headers.

use crate::io::bits::BitReader;
use crate::io::Reader;
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

const SYNC_CORE_16BE: [u8; 4] = [0x7F, 0xFE, 0x80, 0x01];
const SYNC_CORE_16LE: [u8; 4] = [0xFE, 0x7F, 0x01, 0x80];
const SYNC_CORE_14BE: [u8; 4] = [0x1F, 0xFF, 0xE8, 0x00];
const SYNC_CORE_14LE: [u8; 4] = [0xFF, 0x1F, 0x00, 0xE8];
const SYNC_HD: [u8; 4] = [0x64, 0x58, 0x20, 0x25];
const SYNC_XLL: [u8; 4] = [0x41, 0xA2, 0x95, 0x47];
const SYNC_XBR: [u8; 4] = [0x65, 0x5E, 0x31, 0x5E];
const SYNC_X96: [u8; 4] = [0x1D, 0x95, 0xF2, 0x62];
const SYNC_XXCH: [u8; 4] = [0x47, 0x00, 0x4A, 0x03];
const SYNC_XCH: [u8; 4] = [0x5A, 0x5A, 0x5A, 0x5A];
const SYNC_LBR: [u8; 4] = [0x0A, 0x80, 0x19, 0x21];

/// How the core stream is packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packing {
    Be16,
    Le16,
    Be14,
    Le14,
}

/// Decoded core frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Core {
    pub packing: Packing,
    /// Frame size in bytes as stored in the file (14-bit packings are 16/14 larger than FSIZE).
    pub frame_size: usize,
    pub samples_per_frame: u32,
    pub amode: u8,
    pub sampling_rate: u32,
    /// Bits per second; 0 = open/variable/lossless.
    pub bitrate: u32,
    pub ext_audio_id: u8,
    pub ext_audio: bool,
    pub lfe: bool,
    pub bit_depth: u32,
    pub crc_present: bool,
}

const SAMPLING_RATES: [u32; 16] = [0, 8000, 16000, 32000, 0, 0, 11025, 22050, 44100, 0, 0, 12000, 24000, 48000, 0, 0];
const BITRATES: [u32; 29] = [
    32, 56, 64, 96, 112, 128, 192, 224, 256, 320, 384, 448, 512, 576, 640, 768, 960, 1024, 1152, 1280, 1344, 1408, 1411, 1472, 1536, 1920, 2048, 3072, 3840,
];

/// Unpack 14-bit words (each 16-bit word carries 14 payload bits) into a normal byte stream.
pub fn unpack_14(d: &[u8], le: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(d.len() * 7 / 8 + 2);
    let mut acc: u32 = 0;
    let mut nbits = 0;
    for w in d.chunks_exact(2) {
        let word = if le { u16::from_le_bytes([w[0], w[1]]) } else { u16::from_be_bytes([w[0], w[1]]) };
        acc = (acc << 14) | (word & 0x3FFF) as u32;
        nbits += 14;
        while nbits >= 8 {
            out.push((acc >> (nbits - 8)) as u8);
            nbits -= 8;
            acc &= (1 << nbits) - 1;
        }
    }
    out
}

fn to_be16(d: &[u8]) -> Vec<u8> {
    d.chunks_exact(2).flat_map(|w| [w[1], w[0]]).collect()
}

/// Detect the packing of a core sync word at the start of `d`.
pub fn packing_of(d: &[u8]) -> Option<Packing> {
    let head = d.get(..4)?;
    if head == SYNC_CORE_16BE {
        Some(Packing::Be16)
    } else if head == SYNC_CORE_16LE {
        Some(Packing::Le16)
    } else if head == SYNC_CORE_14BE {
        Some(Packing::Be14)
    } else if head == SYNC_CORE_14LE {
        Some(Packing::Le14)
    } else {
        None
    }
}

/// Parse a core frame header at the start of `d` (any packing).
pub fn parse_core(d: &[u8]) -> Option<Core> {
    let packing = packing_of(d)?;
    let take = d.len().min(64);
    let norm: Vec<u8> = match packing {
        Packing::Be16 => d[..take].to_vec(),
        Packing::Le16 => to_be16(&d[..take & !1]),
        Packing::Be14 => unpack_14(&d[..take & !1], false),
        Packing::Le14 => unpack_14(&d[..take & !1], true),
    };
    if norm.get(..4) != Some(&SYNC_CORE_16BE) {
        return None;
    }
    let mut r = BitReader::new(&norm[4..]);
    let _ftype = r.bit()?;
    let _short = r.u8(5)?;
    let crc_present = r.bit()?;
    let nblks = r.u8(7)? as u32;
    let fsize = r.u16(14)? as usize;
    let amode = r.u8(6)?;
    let sfreq = r.u8(4)?;
    let rate = r.u8(5)?;
    r.skip(5); // MIX DYNF TIMEF AUXF HDCD
    let ext_audio_id = r.u8(3)?;
    let ext_audio = r.bit()?;
    let _aspf = r.bit()?;
    let lff = r.u8(2)?;
    let _hflag = r.bit()?;
    if crc_present {
        r.skip(16);
    }
    let _filts = r.bit()?;
    let _vernum = r.u8(4)?;
    let _chist = r.u8(2)?;
    let pcmr = r.u8(3)?;
    if nblks < 5 || fsize < 95 {
        return None;
    }
    let sampling_rate = SAMPLING_RATES[sfreq as usize];
    if sampling_rate == 0 {
        return None;
    }
    let bitrate = BITRATES.get(rate as usize).map(|k| if rate == 22 { 1_411_200 } else { k * 1000 }).unwrap_or(0);
    let bit_depth = match pcmr {
        0 | 1 => 16,
        2 | 3 => 20,
        _ => 24,
    };
    let stored = fsize + 1;
    let frame_size = match packing {
        Packing::Be16 | Packing::Le16 => stored,
        _ => stored * 16 / 14,
    };
    Some(Core { packing, frame_size, samples_per_frame: (nblks + 1) * 32, amode, sampling_rate, bitrate, ext_audio_id, ext_audio, lfe: lff == 1 || lff == 2, bit_depth, crc_present })
}

/// (channel count, ChannelPositions, ChannelLayout) for an AMODE value (without LFE).
pub fn amode_layout(amode: u8) -> (u32, &'static str, &'static str) {
    match amode {
        0 => (1, "Front: C", "C"),
        1 => (2, "Front: L R", "L R"),
        2 => (2, "Front: L R", "L R"),
        3 => (2, "Front: L R", "L R"),
        4 => (2, "Front: L R", "Lt Rt"),
        5 => (3, "Front: L C R", "C L R"),
        6 => (3, "Front: L R, Back: C", "L R Cs"),
        7 => (4, "Front: L C R, Back: C", "C L R Cs"),
        8 => (4, "Front: L R, Side: L R", "L R Ls Rs"),
        9 => (5, "Front: L C R, Side: L R", "C L R Ls Rs"),
        10 => (6, "Front: L C R, Side: L R", "Lc Rc L R Ls Rs"),
        11 => (6, "Front: L C R, Side: L R, Back: C", "C L R Ls Rs Cs"),
        12 => (6, "Front: L C R, Side: L R, Back: C", "C Cs L R Ls Rs"),
        13 => (7, "Front: L C R, Side: L R", "Lc C Rc L R Ls Rs"),
        14 => (8, "Front: L C R, Side: L R", "Lc Rc L R Ls1 Ls2 Rs1 Rs2"),
        15 => (8, "Front: L C R, Side: L R, Back: C", "Lc C Rc L R Ls Cs Rs"),
        _ => (0, "", ""),
    }
}

/// Extension substream header (sync 0x64582025): returns (header size, substream frame size).
pub fn parse_hd_header(d: &[u8]) -> Option<(usize, usize)> {
    if d.get(..4)? != SYNC_HD {
        return None;
    }
    let mut r = BitReader::new(&d[4..]);
    r.skip(8); // UserDefinedBits
    let _index = r.u8(2)?;
    let header_size_type = r.bit()?;
    let (hsize, fsize) = if header_size_type { (r.u16(12)? as usize + 1, r.u32(20)? as usize + 1) } else { (r.u8(8)? as usize + 1, r.u16(16)? as usize + 1) };
    if fsize < hsize {
        return None;
    }
    Some((hsize, fsize))
}

/// Which coding components are present in a substream: (xll, xbr, x96, xxch, xch, lbr).
pub fn hd_components(sub: &[u8]) -> (bool, bool, bool, bool, bool, bool) {
    let has = |sync: &[u8; 4]| sub.windows(4).any(|w| w == sync);
    (has(&SYNC_XLL), has(&SYNC_XBR), has(&SYNC_X96), has(&SYNC_XXCH), has(&SYNC_XCH), has(&SYNC_LBR))
}

fn apply_core(s: &mut Stream, c: &Core) {
    s.set_if_empty("Format", "DTS");
    s.set("Format_Settings_Mode", if matches!(c.packing, Packing::Be14 | Packing::Le14) { "14" } else { "16" });
    s.set("Format_Settings_Endianness", if matches!(c.packing, Packing::Be16 | Packing::Be14) { "Big" } else { "Little" });
    if c.bitrate > 0 {
        s.set("BitRate_Mode", "CBR");
        s.set("BitRate", c.bitrate.to_string());
    } else {
        s.set("BitRate_Mode", "VBR");
    }
    let (n, pos, layout) = amode_layout(c.amode);
    if n > 0 {
        s.set("Channel(s)", (n + c.lfe as u32).to_string());
        if c.lfe {
            s.set("ChannelPositions", format!("{pos}, LFE"));
            s.set("ChannelLayout", format!("{layout} LFE"));
        } else {
            s.set("ChannelPositions", pos);
            s.set("ChannelLayout", layout);
        }
        // The reference always writes the LFE count for DTS ("1/0/0.0").
        let base = crate::finish::channel_positions_string2(pos);
        s.set("ChannelPositions/String2", if base.contains('.') { base } else { format!("{base}.{}", c.lfe as u8) });
    }
    s.set("SamplesPerFrame", c.samples_per_frame.to_string());
    s.set("SamplingRate", c.sampling_rate.to_string());
    s.set("BitDepth", c.bit_depth.to_string());
    s.set_if_empty("Compression_Mode", "Lossy");
}

/// Profile / commercial name from the extension components found after a core frame.
fn apply_extensions(s: &mut Stream, core: Option<&Core>, sub: Option<&[u8]>) {
    let (xll, xbr, x96, xxch, _xch, lbr) = sub.map(hd_components).unwrap_or_default();
    let core_ext = core.filter(|c| c.ext_audio).map(|c| c.ext_audio_id);
    let (profile, commercial, mode) = if xll {
        if core.is_some() { ("MA / Core", "DTS-HD Master Audio", "VBR") } else { ("MA", "DTS-HD Master Audio", "VBR") }
    } else if xbr {
        ("HRA / Core", "DTS-HD High Resolution Audio", "CBR")
    } else if lbr {
        ("Express", "DTS Express", "CBR")
    } else if xxch || core_ext == Some(6) {
        ("XXCH / Core", "DTS-HD", "CBR")
    } else if x96 || core_ext == Some(2) {
        ("96/24 / Core", "DTS 96/24", "CBR")
    } else if core_ext == Some(0) {
        ("ES", "DTS-ES", "CBR")
    } else {
        return;
    };
    s.set("Format_Profile", profile);
    s.set("Format_Commercial_IfAny", commercial);
    if xll {
        s.set("Compression_Mode", "Lossless");
        s.set("BitRate_Mode", mode);
        s.clear("BitRate");
    }
}

/// Fill a stream from one frame: a core frame (optionally followed by an extension substream)
/// or a bare extension substream.
pub fn apply_frame(s: &mut Stream, d: &[u8]) -> bool {
    if let Some(c) = parse_core(d) {
        apply_core(s, &c);
        let sub = d.get(c.frame_size..).filter(|rest| rest.get(..4) == Some(&SYNC_HD));
        apply_extensions(s, Some(&c), sub);
        return true;
    }
    if let Some((_, fsize)) = parse_hd_header(d) {
        s.set_if_empty("Format", "DTS");
        apply_extensions(s, None, Some(&d[..fsize.min(d.len())]));
        return s.has("Format_Profile");
    }
    false
}

/// Size of the frame starting at `d` (core frame plus following extension substream, or a bare
/// extension substream).
fn frame_len(d: &[u8]) -> Option<usize> {
    if let Some(c) = parse_core(d) {
        let mut len = c.frame_size;
        if let Some((_, hd)) = d.get(len..).and_then(parse_hd_header) {
            len += hd;
        }
        return Some(len);
    }
    parse_hd_header(d).map(|(_, f)| f)
}

fn chain(d: &[u8], mut pos: usize, max: usize) -> usize {
    let mut n = 0;
    while n < max {
        let Some(len) = d.get(pos..).and_then(frame_len) else { break };
        n += 1;
        pos += len;
        if pos >= d.len() {
            break;
        }
    }
    n
}

pub fn probe(p: &Probe) -> u8 {
    let ext = p.ext_in(&["dts", "dtshd", "dtsma", "dtshr"]);
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
    let mut s = Stream::new(StreamKind::Audio);
    if !apply_frame(&mut s, &data) {
        return false;
    }
    let core = parse_core(&data);
    let mut pos = 0usize;
    let mut frames = 0u64;
    let mut bytes = 0u64;
    while let Some(len) = data.get(pos..).and_then(frame_len) {
        frames += 1;
        bytes += len as u64;
        pos += len;
        if pos >= data.len() || frames > 50_000_000 {
            break;
        }
    }
    let g = doc.general();
    g.set("Format", "DTS");
    if let Some(c) = core {
        let spf = c.samples_per_frame as f64;
        let sr = c.sampling_rate as f64;
        let mut total_frames = frames as f64;
        if (data.len() as u64) < size && bytes > 0 {
            total_frames = frames as f64 * size as f64 / bytes as f64;
        }
        let duration_ms = (total_frames * spf / sr * 1000.0).round();
        if c.bitrate > 0 && s.get("BitRate_Mode") == "CBR" {
            s.set("Duration", format!("{duration_ms}"));
            s.set("StreamSize", format!("{}", (duration_ms * c.bitrate as f64 / 8000.0).round()));
            g.set("Duration", format!("{duration_ms}"));
            g.set("OverallBitRate_Mode", "CBR");
            g.set("OverallBitRate", c.bitrate.to_string());
        } else {
            s.set("Duration", format!("{duration_ms}"));
            g.set("Duration", format!("{duration_ms}"));
        }
    }
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORE: [u8; 16] = [0x7F, 0xFE, 0x80, 0x01, 0xFC, 0x3C, 0x3F, 0xF0, 0x35, 0xE0, 0x01, 0x38, 0x00, 0x01, 0xEF, 0x83];

    #[test]
    fn core_header() {
        let c = parse_core(&CORE).unwrap();
        assert_eq!(c.packing, Packing::Be16);
        assert_eq!(c.frame_size, 1024);
        assert_eq!(c.samples_per_frame, 512);
        assert_eq!(c.amode, 0);
        assert_eq!(c.sampling_rate, 48000);
        assert_eq!(c.bitrate, 768000);
        assert!(!c.ext_audio);
        assert!(!c.lfe);
        assert_eq!(c.bit_depth, 16);
        assert!(parse_core(&CORE[..8]).is_none());
        assert!(parse_core(&[0u8; 16]).is_none());
    }

    #[test]
    fn packings() {
        let le: Vec<u8> = CORE.chunks_exact(2).flat_map(|w| [w[1], w[0]]).collect();
        let c = parse_core(&le).unwrap();
        assert_eq!(c.packing, Packing::Le16);
        assert_eq!(c.bitrate, 768000);
        // Repack into 14-bit words.
        let mut bits = String::new();
        for b in CORE {
            bits.push_str(&format!("{b:08b}"));
        }
        let mut be14 = Vec::new();
        for chunk in bits.as_bytes().chunks(14) {
            if chunk.len() < 14 {
                break;
            }
            let mut v = u16::from_str_radix(std::str::from_utf8(chunk).unwrap(), 2).unwrap();
            if v & 0x2000 != 0 {
                v |= 0xC000; // words are sign-extended 14-bit values
            }
            be14.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(&be14[..4], &SYNC_CORE_14BE);
        let c = parse_core(&be14).unwrap();
        assert_eq!(c.packing, Packing::Be14);
        assert_eq!(c.frame_size, 1024 * 16 / 14);
        assert_eq!(c.sampling_rate, 48000);
        let le14: Vec<u8> = be14.chunks_exact(2).flat_map(|w| [w[1], w[0]]).collect();
        assert_eq!(parse_core(&le14).unwrap().packing, Packing::Le14);
    }

    #[test]
    fn hd_header_and_components() {
        // UserDefinedBits 0, index 0, header size type 0, header size 9 (8+1), frame size 100 (99+1).
        // bits: 00000000 00 0 00001000 0000000001100011 -> pad
        let mut d = SYNC_HD.to_vec();
        d.extend_from_slice(&[0x00, 0x01, 0x00, 0x0C, 0x60, 0x00, 0x00, 0x00]);
        assert_eq!(parse_hd_header(&d), Some((9, 100)));
        let mut sub = d.clone();
        sub.extend_from_slice(&SYNC_XLL);
        assert_eq!(hd_components(&sub).0, true);
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_frame(&mut s, &sub));
        assert_eq!(s.get("Format_Profile"), "MA");
        assert_eq!(s.get("Compression_Mode"), "Lossless");
    }

    #[test]
    fn apply_core_fields() {
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_frame(&mut s, &CORE));
        assert_eq!(s.get("Format"), "DTS");
        assert_eq!(s.get("Format_Settings_Mode"), "16");
        assert_eq!(s.get("Format_Settings_Endianness"), "Big");
        assert_eq!(s.get("BitRate"), "768000");
        assert_eq!(s.get("Channel(s)"), "1");
        assert_eq!(s.get("ChannelLayout"), "C");
        assert_eq!(s.get("BitDepth"), "16");
        assert!(!s.has("Format_Profile"));
        assert_eq!(amode_layout(9), (5, "Front: L C R, Side: L R", "C L R Ls Rs"));
    }

    #[test]
    fn probe_and_parse() {
        let mut d = Vec::new();
        for _ in 0..4 {
            d.extend_from_slice(&CORE);
            d.extend_from_slice(&[0u8; 1008]);
        }
        let p = Probe { head: &d, ext: "dts", size: d.len() as u64 };
        assert_eq!(probe(&p), 95);
        let mut r = Reader::from_bytes(d);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let s = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(s.get("Duration"), "43"); // 4 * 512 / 48000 = 42.67 ms
        assert_eq!(s.get("StreamSize"), "4128");
        assert_eq!(doc.general_ref().get("OverallBitRate"), "768000");
    }
}

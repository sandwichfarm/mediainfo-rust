//! AMR-NB / AMR-WB storage format (RFC 4867): magic, then 20 ms frames of `header byte + payload`.

use crate::io::Reader;
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

/// Payload bytes (without the header byte) per frame type, AMR-NB (RFC 4867 / 3GPP TS 26.101).
const NB_SIZES: [u8; 16] = [12, 13, 15, 17, 19, 20, 26, 31, 5, 0, 0, 0, 0, 0, 0, 0];
/// Payload bytes per frame type, AMR-WB (3GPP TS 26.201).
const WB_SIZES: [u8; 16] = [17, 23, 32, 36, 40, 46, 50, 58, 60, 5, 0, 0, 0, 0, 0, 0];

/// Frames scanned at most (~20 minutes); longer files are extrapolated from the size.
const MAX_FRAMES: u64 = 60_000;
/// Bytes scanned at most.
const MAX_SCAN: u64 = 4 << 20;

/// Which flavour and where the frames start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Magic {
    pub wide: bool,
    pub header_len: usize,
}

pub fn magic(d: &[u8]) -> Option<Magic> {
    if d.starts_with(b"#!AMR-WB\n") {
        Some(Magic { wide: true, header_len: 9 })
    } else if d.starts_with(b"#!AMR\n") {
        Some(Magic { wide: false, header_len: 6 })
    } else {
        None
    }
}

/// Result of walking the frames: frame count, bytes consumed, per-type counts, whether every speech
/// frame had the same type (SID / no-data frames ignored).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    pub frames: u64,
    pub bytes: u64,
    pub counts: [u64; 16],
}

impl Scan {
    /// Frame type of the speech frames when there is only one.
    pub fn single_type(&self, wide: bool) -> Option<usize> {
        let speech = if wide { 0..9 } else { 0..8 };
        let mut found = None;
        for t in speech {
            if self.counts[t] > 0 {
                if found.is_some() {
                    return None;
                }
                found = Some(t);
            }
        }
        found
    }
}

/// Walk frames from `d` (which starts at the first frame).
pub fn scan_frames(d: &[u8], wide: bool) -> Scan {
    let sizes = if wide { &WB_SIZES } else { &NB_SIZES };
    let mut s = Scan::default();
    let mut pos = 0usize;
    while pos < d.len() && s.frames < MAX_FRAMES {
        let ft = ((d[pos] >> 3) & 0xF) as usize;
        let len = 1 + sizes[ft] as usize;
        if pos + len > d.len() {
            break;
        }
        s.counts[ft] += 1;
        s.frames += 1;
        pos += len;
    }
    s.bytes = pos as u64;
    s
}

pub fn probe(p: &Probe) -> u8 {
    match magic(p.head) {
        Some(m) => {
            let frames = scan_frames(&p.head[m.header_len..], m.wide);
            if frames.frames >= 2 || p.size as usize <= m.header_len + 64 {
                100
            } else {
                60
            }
        }
        None => {
            if p.ext_in(&["amr", "awb"]) {
                10
            } else {
                0
            }
        }
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 16);
    let Some(m) = magic(&head) else { return false };
    let size = r.len();
    let data_len = size.saturating_sub(m.header_len as u64);
    let scan_len = data_len.min(MAX_SCAN) as usize;
    let data = r.read_vec_at(m.header_len as u64, scan_len);
    let scan = scan_frames(&data, m.wide);
    let mut frames = scan.frames as f64;
    if scan.bytes < data_len && scan.bytes > 0 {
        frames *= data_len as f64 / scan.bytes as f64;
    }
    let mut s = Stream::new(StreamKind::Audio);
    s.set("Format", "AMR");
    s.set("Format_Profile", if m.wide { "Wide band" } else { "Narrow band" });
    s.set("SamplingRate", if m.wide { "16000" } else { "8000" });
    s.set("Channel(s)", "1");
    s.set("BitDepth", if m.wide { "14" } else { "13" });
    s.set("StreamSize", data_len.to_string());
    let duration_ms = frames * 20.0;
    if duration_ms > 0.0 {
        s.set("Duration", format!("{duration_ms:.3}"));
    }
    let sizes = if m.wide { &WB_SIZES } else { &NB_SIZES };
    let bit_rate = match scan.single_type(m.wide) {
        Some(t) => {
            s.set("BitRate_Mode", "CBR");
            (1 + sizes[t] as u64) * 8 * 50
        }
        None => {
            s.set("BitRate_Mode", "VBR");
            if duration_ms > 0.0 {
                (data_len as f64 * 8.0 * 1000.0 / duration_ms).round() as u64
            } else {
                0
            }
        }
    };
    if bit_rate > 0 {
        s.set("BitRate", bit_rate.to_string());
    }
    let g = doc.general();
    g.set("Format", "AMR");
    g.set("StreamSize", m.header_len.to_string());
    if duration_ms > 0.0 {
        g.set("Duration", format!("{}", duration_ms.round() as u64));
    }
    if bit_rate > 0 {
        g.set("OverallBitRate", bit_rate.to_string());
    }
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(wide: bool, types: &[usize]) -> Vec<u8> {
        let sizes = if wide { &WB_SIZES } else { &NB_SIZES };
        let mut v = if wide { b"#!AMR-WB\n".to_vec() } else { b"#!AMR\n".to_vec() };
        for &t in types {
            v.push(((t as u8) << 3) | 0x04);
            v.extend(std::iter::repeat(0x55u8).take(sizes[t] as usize));
        }
        v
    }

    #[test]
    fn scan() {
        let d = frames(false, &[7, 7, 8, 7]);
        let s = scan_frames(&d[6..], false);
        assert_eq!(s.frames, 4);
        assert_eq!(s.bytes, 3 * 32 + 6);
        assert_eq!(s.single_type(false), Some(7));
        let d = frames(true, &[8, 0]);
        assert_eq!(scan_frames(&d[9..], true).single_type(true), None);
        assert_eq!(magic(b"#!AMR\n").unwrap().header_len, 6);
        assert!(magic(b"#!AMR").is_none());
    }

    #[test]
    fn nb_file() {
        let d = frames(false, &[7; 51]);
        assert_eq!(probe(&Probe { head: &d, ext: "amr", size: d.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "amr", size: 4 }), 10);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(d.clone()), &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "AMR");
        assert_eq!(g.get("Duration"), "1020");
        assert_eq!(g.get("OverallBitRate"), "12800");
        assert_eq!(g.get("StreamSize"), "6");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("Format_Profile"), "Narrow band");
        assert_eq!(a.get("SamplingRate"), "8000");
        assert_eq!(a.get("BitDepth"), "13");
        assert_eq!(a.get("Duration"), "1020.000");
        assert_eq!(a.get("BitRate"), "12800");
        assert_eq!(a.get("BitRate_Mode"), "CBR");
        assert_eq!(a.get("StreamSize"), (d.len() - 6).to_string());
    }

    #[test]
    fn wb_vbr_file() {
        let d = frames(true, &[8, 2, 8, 2]);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(d.clone()), &mut doc));
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("Format_Profile"), "Wide band");
        assert_eq!(a.get("SamplingRate"), "16000");
        assert_eq!(a.get("BitRate_Mode"), "VBR");
        assert_eq!(a.get("Duration"), "80.000");
        let expect = ((d.len() - 9) as f64 * 8.0 * 1000.0 / 80.0).round() as u64;
        assert_eq!(a.get("BitRate"), expect.to_string());
        assert!(!parse(&mut Reader::from_bytes(b"RIFF".to_vec()), &mut Doc::new()));
    }
}

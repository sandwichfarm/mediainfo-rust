//! WebVTT: `WEBVTT` signature line, then cue blocks with `MM:SS.mmm --> MM:SS.mmm` timings.

use super::srt::{decode_text, TEXT_LIMIT};
use crate::io::Reader;
use crate::model::{Doc, StreamKind};
use crate::parsers::Probe;

const MAX_LINES: usize = 1 << 20;

/// Cue statistics from the head of a file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub cues: usize,
    pub first_ms: Option<u64>,
    pub last_ms: Option<u64>,
}

/// `HH:MM:SS.mmm` or `MM:SS.mmm` → milliseconds.
pub fn parse_time(s: &str) -> Option<u64> {
    let s = s.trim();
    let (hms, frac) = s.split_once('.')?;
    if frac.len() != 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, sec): (u64, u64, u64) = match parts.as_slice() {
        [m, s] => (0, m.parse().ok()?, s.parse().ok()?),
        [h, m, s] => (h.parse().ok()?, m.parse().ok()?, s.parse().ok()?),
        _ => return None,
    };
    if m > 59 || sec > 59 {
        return None;
    }
    Some(((h * 60 + m) * 60 + sec) * 1000 + frac.parse::<u64>().ok()?)
}

/// Whether the text starts with the `WEBVTT` signature (optionally followed by a title).
pub fn has_signature(text: &str) -> bool {
    let t = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    match t.strip_prefix("WEBVTT") {
        Some(rest) => rest.is_empty() || rest.starts_with([' ', '\t', '\n', '\r']),
        None => false,
    }
}

pub fn scan(text: &str) -> Option<Stats> {
    if !has_signature(text) {
        return None;
    }
    let mut st = Stats::default();
    for line in text.lines().take(MAX_LINES) {
        let Some((a, b)) = line.split_once("-->") else { continue };
        let end = b.trim().split_whitespace().next().unwrap_or("");
        let (Some(s), Some(e)) = (parse_time(a), parse_time(end)) else { continue };
        st.cues += 1;
        st.first_ms = Some(st.first_ms.map_or(s, |v| v.min(s)));
        st.last_ms = Some(st.last_ms.map_or(e, |v| v.max(e)));
    }
    Some(st)
}

pub fn probe(p: &Probe) -> u8 {
    match decode_text(p.head) {
        Some(t) if has_signature(&t) => 100,
        _ => 0,
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let n = r.len().min(TEXT_LIMIT as u64) as usize;
    let Some(text) = decode_text(&r.read_vec_at(0, n)) else { return false };
    if scan(&text).is_none() {
        return false;
    }
    doc.general().set("Format", "WebVTT");
    doc.general().set_int("StreamSize", r.len() as i128);
    let s = doc.add(StreamKind::Text);
    s.set("Format", "WebVTT");
    s.set("Compression_Mode", "Lossless");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "WEBVTT\n\n00:00.000 --> 00:00.500\nHello\n\nintro\n00:00.500 --> 01:00:01.000 line:0\nWorld\n";

    #[test]
    fn times_and_scan() {
        assert_eq!(parse_time("00:00.500"), Some(500));
        assert_eq!(parse_time("01:00:01.000"), Some(3_601_000));
        assert_eq!(parse_time("00:00,500"), None);
        assert_eq!(parse_time("00:00.5"), None);
        assert_eq!(scan(SAMPLE), Some(Stats { cues: 2, first_ms: Some(0), last_ms: Some(3_601_000) }));
        assert!(has_signature("WEBVTT - title\n"));
        assert!(has_signature("\u{FEFF}WEBVTT"));
        assert!(!has_signature("WEBVTTX\n"));
        assert!(scan("1\n00:00:00,000 --> 00:00:00,500\n").is_none());
    }

    #[test]
    fn probe_and_parse() {
        assert_eq!(probe(&Probe { head: SAMPLE.as_bytes(), ext: "vtt", size: 1 }), 100);
        assert_eq!(probe(&Probe { head: b"just some text\n", ext: "vtt", size: 1 }), 0);
        assert_eq!(probe(&Probe { head: b"WEBVTT\xFF\xFE", ext: "vtt", size: 1 }), 0);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(SAMPLE.as_bytes().to_vec()), &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "WebVTT");
        let t = doc.stream(StreamKind::Text, 0).unwrap();
        assert_eq!(t.get("Format"), "WebVTT");
        assert_eq!(t.get("Compression_Mode"), "Lossless");
    }
}

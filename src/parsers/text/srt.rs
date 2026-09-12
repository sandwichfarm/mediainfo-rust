//! SubRip (.srt): numbered cues with `HH:MM:SS,mmm --> HH:MM:SS,mmm` timing lines.
//!
//! Also hosts the text-decoding helper shared by the other subtitle parsers.

use crate::io::Reader;
use crate::model::{Doc, StreamKind};
use crate::parsers::Probe;

/// Bytes read from the head of a text file.
pub const TEXT_LIMIT: usize = 4 << 20;
const MAX_CUES: usize = 1 << 20;

/// Decode the start of a text file: UTF-8 (optional BOM) or UTF-16 with BOM.
/// `None` when the bytes are not text (invalid UTF-8, NUL or other control characters).
pub fn decode_text(bytes: &[u8]) -> Option<String> {
    let text = match bytes {
        [0xFF, 0xFE, ..] | [0xFE, 0xFF, ..] => crate::io::utf16(bytes, true),
        _ => {
            let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
            match std::str::from_utf8(b) {
                Ok(s) => s.to_string(),
                // A multi-byte sequence cut by the read limit is fine; anything else is binary.
                Err(e) if e.error_len().is_none() => std::str::from_utf8(&b[..e.valid_up_to()]).ok()?.to_string(),
                Err(_) => return None,
            }
        }
    };
    if text.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\x0C')) {
        return None;
    }
    Some(text)
}

/// `HH:MM:SS,mmm` (also `.` as the separator, and hours of any width) → milliseconds.
pub fn parse_time(s: &str) -> Option<u64> {
    let s = s.trim();
    let (hms, ms) = s.split_once([',', '.'])?;
    let mut parts = hms.split(':');
    let h: u64 = parts.next()?.trim().parse().ok()?;
    let m: u64 = parts.next()?.trim().parse().ok()?;
    let sec: u64 = parts.next()?.trim().parse().ok()?;
    let frac = ms.trim();
    if parts.next().is_some() || m > 59 || sec > 59 || frac.is_empty() || frac.len() > 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let ms: u64 = frac.parse().ok()?;
    let ms = ms * 10u64.pow(3 - frac.len() as u32);
    Some(((h * 60 + m) * 60 + sec) * 1000 + ms)
}

/// `start --> end` (any extra cue settings after the end time are ignored).
pub fn parse_timing(line: &str) -> Option<(u64, u64)> {
    let (a, b) = line.split_once("-->")?;
    let end = b.trim().split_whitespace().next()?;
    let (s, e) = (parse_time(a)?, parse_time(end)?);
    (e >= s).then_some((s, e))
}

/// Cue statistics from the head of a file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub cues: usize,
    pub first_ms: u64,
    pub last_ms: u64,
}

/// Walk `index / timing / text...` blocks; `None` unless the text starts with a valid cue.
pub fn scan(text: &str) -> Option<Stats> {
    let mut stats = Stats::default();
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut lines = text.lines().map(str::trim).peekable();
    let mut first = true;
    while let Some(line) = lines.next() {
        if line.is_empty() {
            continue;
        }
        let is_index = !line.is_empty() && line.chars().all(|c| c.is_ascii_digit());
        let timing = if is_index { lines.next().and_then(parse_timing) } else { None };
        match timing {
            Some((s, e)) => {
                if first {
                    stats.first_ms = s;
                    first = false;
                }
                stats.last_ms = stats.last_ms.max(e);
                stats.cues += 1;
                if stats.cues >= MAX_CUES {
                    break;
                }
                // Skip the cue text.
                while let Some(l) = lines.peek() {
                    if l.is_empty() {
                        break;
                    }
                    lines.next();
                }
            }
            None if first => return None,
            None => break,
        }
    }
    (stats.cues > 0).then_some(stats)
}

pub fn probe(p: &Probe) -> u8 {
    match decode_text(p.head).as_deref().and_then(scan) {
        Some(_) => 100,
        None => 0,
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let n = r.len().min(TEXT_LIMIT as u64) as usize;
    let Some(text) = decode_text(&r.read_vec_at(0, n)) else { return false };
    if scan(&text).is_none() {
        return false;
    }
    doc.general().set("Format", "SubRip");
    doc.general().set_int("StreamSize", r.len() as i128);
    let s = doc.add(StreamKind::Text);
    s.set("Format", "SubRip");
    s.set("Compression_Mode", "Lossless");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "1\n00:00:00,000 --> 00:00:00,500\nHello\n\n2\n00:00:00,500 --> 00:00:01,000\nWorld\nagain\n\n3\n01:02:03,450 --> 01:02:04,000\nEnd\n";

    #[test]
    fn times() {
        assert_eq!(parse_time("00:00:00,500"), Some(500));
        assert_eq!(parse_time("01:02:03.450"), Some(3_723_450));
        assert_eq!(parse_time("1:2:3,4"), Some(3_723_400));
        assert_eq!(parse_time("00:60:00,000"), None);
        assert_eq!(parse_time("00:00,000"), None);
        assert_eq!(parse_timing("00:00:00,000 --> 00:00:00,500 X1:0"), Some((0, 500)));
        assert_eq!(parse_timing("00:00:01,000 --> 00:00:00,500"), None);
    }

    #[test]
    fn scanning() {
        assert_eq!(scan(SAMPLE), Some(Stats { cues: 3, first_ms: 0, last_ms: 3_724_000 }));
        assert_eq!(scan("\u{FEFF}\n\n1\r\n00:00:00,000 --> 00:00:00,500\r\nHi\r\n"), Some(Stats { cues: 1, first_ms: 0, last_ms: 500 }));
        assert!(scan("just some text\n").is_none());
        assert!(scan("").is_none());
        assert!(scan("1\nnot a timing\n").is_none());
    }

    #[test]
    fn decoding() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFabc").as_deref(), Some("abc"));
        assert_eq!(decode_text(&[0xFF, 0xFE, b'1', 0, b'\n', 0]).as_deref(), Some("1\n"));
        assert!(decode_text(b"\x00\x01\x02").is_none());
        assert!(decode_text(b"abc\xFFdef").is_none());
        assert_eq!(decode_text(b"caf\xC3").as_deref(), Some("caf")); // truncated multi-byte
    }

    #[test]
    fn probe_and_parse() {
        let head = SAMPLE.as_bytes();
        assert_eq!(probe(&Probe { head, ext: "srt", size: head.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"just some text\n", ext: "srt", size: 15 }), 0);
        assert_eq!(probe(&Probe { head: &[0, 159, 146, 150], ext: "srt", size: 4 }), 0);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(head.to_vec()), &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "SubRip");
        let t = doc.stream(StreamKind::Text, 0).unwrap();
        assert_eq!(t.get("Format"), "SubRip");
        assert_eq!(t.get("Compression_Mode"), "Lossless");
        let mut doc = Doc::new();
        assert!(!parse(&mut Reader::from_bytes(b"WEBVTT\n".to_vec()), &mut doc));
    }
}

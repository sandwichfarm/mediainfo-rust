//! VobSub index files (.idx): `# VobSub index file, v7`, global `size:` / `langidx:` lines, then one
//! `id: <lang>, index: <n>` block per subtitle stream with its `timestamp:` lines.

use super::srt::{decode_text, TEXT_LIMIT};
use crate::io::Reader;
use crate::model::{Doc, StreamKind};
use crate::parsers::Probe;

const MAGIC: &str = "# VobSub index file";
const MAX_LINES: usize = 1 << 20;
const MAX_STREAMS: usize = 64;

/// One `id:` block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Track {
    pub language: String,
    pub index: Option<u32>,
    pub timestamps: usize,
    /// Last `timestamp:` of the block, in ms (with the block's `delay:` applied).
    pub last_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    pub version: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub langidx: Option<u32>,
    pub tracks: Vec<Track>,
}

/// `HH:MM:SS:mmm` (optionally signed) → milliseconds.
pub fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let p: Vec<&str> = s.split(':').collect();
    if p.len() != 4 {
        return None;
    }
    let v: Vec<u64> = p.iter().map(|x| x.trim().parse::<u64>().ok()).collect::<Option<_>>()?;
    let ms = ((v[0] * 60 + v[1]) * 60 + v[2]) * 1000 + v[3];
    Some(if neg { -(ms as i64) } else { ms as i64 })
}

pub fn parse_index(text: &str) -> Option<Index> {
    if !text.trim_start_matches('\u{FEFF}').starts_with(MAGIC) {
        return None;
    }
    let mut idx = Index::default();
    let mut delay: i64 = 0;
    for line in text.lines().take(MAX_LINES) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix(MAGIC) {
            idx.version = rest.trim().trim_start_matches(',').trim().strip_prefix('v').and_then(|v| v.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        match key.as_str() {
            "size" => {
                if let Some((w, h)) = value.split_once('x') {
                    idx.width = w.trim().parse().ok();
                    idx.height = h.trim().parse().ok();
                }
            }
            "langidx" => idx.langidx = value.parse().ok(),
            "id" => {
                if idx.tracks.len() >= MAX_STREAMS {
                    break;
                }
                let mut t = Track::default();
                let mut parts = value.split(',');
                t.language = parts.next().unwrap_or("").trim().to_string();
                for p in parts {
                    if let Some((k, v)) = p.split_once(':') {
                        if k.trim().eq_ignore_ascii_case("index") {
                            t.index = v.trim().parse().ok();
                        }
                    }
                }
                delay = 0;
                idx.tracks.push(t);
            }
            "delay" => delay = parse_time(value).unwrap_or(0),
            "timestamp" => {
                let Some(t) = idx.tracks.last_mut() else { continue };
                let Some(ts) = value.split(',').next().and_then(parse_time) else { continue };
                t.timestamps += 1;
                let ms = (ts + delay).max(0) as u64;
                t.last_ms = Some(t.last_ms.map_or(ms, |v| v.max(ms)));
            }
            _ => {}
        }
    }
    Some(idx)
}

pub fn probe(p: &Probe) -> u8 {
    match decode_text(p.head) {
        Some(t) if t.trim_start_matches('\u{FEFF}').starts_with(MAGIC) => 100,
        _ => 0,
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let n = r.len().min(TEXT_LIMIT as u64) as usize;
    let Some(text) = decode_text(&r.read_vec_at(0, n)) else { return false };
    let Some(idx) = parse_index(&text) else { return false };
    doc.general().set("Format", "VobSub");
    doc.general().set_int("StreamSize", r.len() as i128);
    for t in &idx.tracks {
        let s = doc.add(StreamKind::Text);
        s.set("Format", "VobSub");
        if let Some(i) = t.index {
            s.set_int("ID", i as i128);
        }
        if let Some(ms) = t.last_ms {
            s.set_int("Duration", ms as i128);
        }
        if let (Some(w), Some(h)) = (idx.width, idx.height) {
            s.set_int("Width", w as i128);
            s.set_int("Height", h as i128);
        }
        if !t.language.is_empty() {
            s.set("Language", t.language.clone());
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# VobSub index file, v7 (do not modify this line!)\n#\nsize: 720x576\norg: 0, 0\nscale: 100%, 100%\nlangidx: 0\n\n# English\nid: en, index: 0\n# alt: English\ntimestamp: 00:00:01:000, filepos: 000000000\ntimestamp: 00:00:05:400, filepos: 000001800\n\n# German\nid: de, index: 1\ndelay: 00:00:00:500\ntimestamp: 00:01:00:000, filepos: 000004000\n";

    #[test]
    fn index() {
        let idx = parse_index(SAMPLE).unwrap();
        assert_eq!((idx.version, idx.width, idx.height, idx.langidx), (Some(7), Some(720), Some(576), Some(0)));
        assert_eq!(idx.tracks.len(), 2);
        assert_eq!(idx.tracks[0], Track { language: "en".into(), index: Some(0), timestamps: 2, last_ms: Some(5400) });
        assert_eq!(idx.tracks[1], Track { language: "de".into(), index: Some(1), timestamps: 1, last_ms: Some(60_500) });
        assert_eq!(parse_time("-00:00:01:500"), Some(-1500));
        assert_eq!(parse_time("00:00:01"), None);
        assert!(parse_index("size: 720x576\n").is_none());
    }

    #[test]
    fn probe_and_parse() {
        assert_eq!(probe(&Probe { head: SAMPLE.as_bytes(), ext: "idx", size: 1 }), 100);
        assert_eq!(probe(&Probe { head: b"# just a comment\n", ext: "idx", size: 1 }), 0);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(SAMPLE.as_bytes().to_vec()), &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "VobSub");
        assert_eq!(doc.count(StreamKind::Text), 2);
        let t = doc.stream(StreamKind::Text, 0).unwrap();
        assert_eq!((t.get("Format"), t.get("Language"), t.get("Width"), t.get("Height"), t.get("Duration"), t.get("ID")), ("VobSub", "en", "720", "576", "5400", "0"));
        assert_eq!(doc.stream(StreamKind::Text, 1).unwrap().get("Duration"), "60500");
    }
}

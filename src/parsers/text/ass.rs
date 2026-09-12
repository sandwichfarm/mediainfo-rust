//! SubStation Alpha (.ssa, v4.00) and Advanced SubStation Alpha (.ass, v4.00+): INI-like sections
//! `[Script Info]`, `[V4(+) Styles]`, `[Events]`.

use super::srt::{decode_text, TEXT_LIMIT};
use crate::io::Reader;
use crate::model::{Doc, StreamKind};
use crate::parsers::Probe;

const MAX_LINES: usize = 1 << 20;

/// What the script header and the events say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script {
    /// `true` for ASS (v4.00+), `false` for SSA (v4.00).
    pub advanced: bool,
    pub title: Option<String>,
    pub play_res_x: Option<u32>,
    pub play_res_y: Option<u32>,
    pub dialogues: usize,
    pub first_ms: Option<u64>,
    pub last_ms: Option<u64>,
}

impl Script {
    pub fn format(&self) -> &'static str {
        if self.advanced { "ASS" } else { "SSA" }
    }
}

/// `H:MM:SS.cc` → milliseconds.
pub fn parse_time(s: &str) -> Option<u64> {
    let s = s.trim();
    let (hms, cs) = s.split_once('.')?;
    let mut p = hms.split(':');
    let h: u64 = p.next()?.trim().parse().ok()?;
    let m: u64 = p.next()?.trim().parse().ok()?;
    let sec: u64 = p.next()?.trim().parse().ok()?;
    if p.next().is_some() || m > 59 || sec > 59 || cs.is_empty() || cs.len() > 3 || !cs.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let frac: u64 = cs.parse().ok()?;
    let ms = frac * 10u64.pow(3 - cs.len() as u32);
    Some(((h * 60 + m) * 60 + sec) * 1000 + ms)
}

/// Whether the text starts (after blank lines and `;` comments) with `[Script Info]`.
pub fn starts_with_script_info(text: &str) -> bool {
    text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with(';')).is_some_and(|l| l.eq_ignore_ascii_case("[script info]"))
}

pub fn parse_script(text: &str) -> Option<Script> {
    if !starts_with_script_info(text) {
        return None;
    }
    let mut sc = Script { advanced: true, ..Default::default() };
    let mut section = String::new();
    let mut script_type_seen = false;
    for line in text.lines().take(MAX_LINES) {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_ascii_lowercase();
            if !script_type_seen {
                match section.as_str() {
                    "v4+ styles" => sc.advanced = true,
                    "v4 styles" => sc.advanced = false,
                    _ => {}
                }
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let (key, value) = (key.trim(), value.trim());
        match section.as_str() {
            "script info" => match key.to_ascii_lowercase().as_str() {
                "scripttype" => {
                    script_type_seen = true;
                    sc.advanced = value.to_ascii_lowercase().contains("v4.00+") || value.to_ascii_lowercase().contains("v4+");
                }
                "title" => sc.title = Some(value.to_string()).filter(|t| !t.is_empty()),
                "playresx" => sc.play_res_x = value.parse().ok(),
                "playresy" => sc.play_res_y = value.parse().ok(),
                _ => {}
            },
            "events" if key.eq_ignore_ascii_case("dialogue") => {
                // Layer/Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
                let mut f = value.splitn(4, ',');
                let _layer = f.next();
                let (Some(start), Some(end)) = (f.next().and_then(parse_time), f.next().and_then(parse_time)) else { continue };
                sc.dialogues += 1;
                sc.first_ms = Some(sc.first_ms.map_or(start, |v| v.min(start)));
                sc.last_ms = Some(sc.last_ms.map_or(end, |v| v.max(end)));
            }
            _ => {}
        }
    }
    Some(sc)
}

pub fn probe(p: &Probe) -> u8 {
    match decode_text(p.head) {
        Some(t) if starts_with_script_info(&t) => 100,
        Some(t) if p.ext_in(&["ass", "ssa"]) && t.contains("[Events]") && t.contains("Dialogue:") => 70,
        _ => 0,
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let n = r.len().min(TEXT_LIMIT as u64) as usize;
    let Some(text) = decode_text(&r.read_vec_at(0, n)) else { return false };
    let sc = match parse_script(&text) {
        Some(sc) => sc,
        None if text.contains("[Events]") && text.contains("Dialogue:") => Script { advanced: text.contains("[V4+ Styles]"), ..Default::default() },
        None => return false,
    };
    doc.general().set("Format", sc.format());
    doc.general().set_int("StreamSize", r.len() as i128);
    let s = doc.add(StreamKind::Text);
    s.set("Format", sc.format());
    s.set("Compression_Mode", "Lossless");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASS: &str = "[Script Info]\nScriptType: v4.00+\nTitle: Demo\nPlayResX: 64\nPlayResY: 48\n\n[V4+ Styles]\nFormat: Name, Fontname\nStyle: Default,Arial\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:00.50,Default,,0,0,0,,Hello, world\nDialogue: 0,0:00:00.50,0:00:01.00,Default,,0,0,0,,World\n";
    const SSA: &str = "; comment\n\n[Script Info]\nScriptType: v4.00\n\n[V4 Styles]\n\n[Events]\nDialogue: Marked=0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Hi\n";

    #[test]
    fn scripts() {
        let sc = parse_script(ASS).unwrap();
        assert_eq!(sc, Script { advanced: true, title: Some("Demo".into()), play_res_x: Some(64), play_res_y: Some(48), dialogues: 2, first_ms: Some(0), last_ms: Some(1000) });
        let sc = parse_script(SSA).unwrap();
        assert_eq!((sc.format(), sc.dialogues, sc.first_ms, sc.last_ms), ("SSA", 1, Some(1000), Some(2000)));
        assert!(parse_script("[Events]\n").is_none());
        assert!(parse_script("").is_none());
        // No ScriptType: the styles section decides.
        assert!(!parse_script("[Script Info]\n[V4 Styles]\n").unwrap().advanced);
        assert!(parse_script("[Script Info]\n[V4+ Styles]\n").unwrap().advanced);
        assert_eq!(parse_time("1:02:03.45"), Some(3_723_450));
        assert_eq!(parse_time("0:00:00,50"), None);
    }

    #[test]
    fn probe_and_parse() {
        assert_eq!(probe(&Probe { head: ASS.as_bytes(), ext: "ass", size: 1 }), 100);
        assert_eq!(probe(&Probe { head: SSA.as_bytes(), ext: "txt", size: 1 }), 100);
        assert_eq!(probe(&Probe { head: b"just some text\n", ext: "ass", size: 1 }), 0);
        assert_eq!(probe(&Probe { head: b"\xFF\xD8\xFF", ext: "ass", size: 1 }), 0);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(ASS.as_bytes().to_vec()), &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "ASS");
        let t = doc.stream(StreamKind::Text, 0).unwrap();
        assert_eq!(t.get("Format"), "ASS");
        assert_eq!(t.get("Compression_Mode"), "Lossless");
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(SSA.as_bytes().to_vec()), &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "SSA");
        let mut doc = Doc::new();
        assert!(!parse(&mut Reader::from_bytes(b"1\n00:00:00,000 --> 00:00:01,000\nx\n".to_vec()), &mut doc));
    }
}

//! Report renderers.

pub mod json;
pub mod template;
pub mod text;
pub mod xml;

use crate::model::{Stream, StreamKind};

static INFO_PARAMETERS: &str = include_str!("../model/info_parameters.txt");

/// `Info_Parameters`: every field name with its description, per kind.
pub fn info_parameters() -> String {
    INFO_PARAMETERS.to_string()
}

/// `Info_Parameters_CSV`: `Name;Description` lines under a kind header.
pub fn info_parameters_csv() -> String {
    let mut out = String::new();
    for line in INFO_PARAMETERS.lines() {
        match line.split_once(" : ") {
            Some((name, desc)) => {
                out.push_str(name.trim_end());
                out.push(';');
                out.push_str(desc);
                out.push('\n');
            }
            None => {
                out.push_str(line.trim_end());
                out.push('\n');
            }
        }
    }
    out
}

/// Whether a field is exported to XML/JSON (options flag 4), dynamic fields always are.
pub fn in_export(stream: &Stream, index: usize, options: &str) -> bool {
    if index >= stream.schema_len() {
        return true;
    }
    options.as_bytes().get(4) == Some(&b'Y')
}

/// Element/key name for XML/JSON: `/`, `(`, `)`, spaces and punctuation become `_` (or are dropped),
/// `Channel(s)` becomes `Channels`, names starting with a digit get a leading `_`.
pub fn export_name(name: &str) -> String {
    let mut s = String::with_capacity(name.len() + 1);
    for c in name.chars() {
        match c {
            '(' | ')' => {}
            '/' | ' ' | ':' | '.' | '*' | '-' | '+' | ',' => s.push('_'),
            c if c.is_alphanumeric() || c == '_' => s.push(c),
            _ => s.push('_'),
        }
    }
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    if s.is_empty() {
        s.push('_');
    }
    s
}

/// XML/JSON value: milliseconds become seconds; `Version 4` becomes `4`; profile/level/tier are split.
/// Returns the (name, value) pairs to emit for one field.
pub fn export_value(kind: StreamKind, name: &str, text: &str, measure: &str) -> Vec<(String, String)> {
    let _ = kind;
    if measure == " ms" {
        return vec![(name.to_string(), ms_to_seconds(text))];
    }
    match name {
        "Format_Version" => vec![(name.to_string(), text.strip_prefix("Version ").unwrap_or(text).to_string())],
        "Format_Profile" => {
            let mut out = Vec::new();
            let mut parts = text.split('@');
            let profile = parts.next().unwrap_or("");
            out.push(("Format_Profile".to_string(), profile.to_string()));
            for p in parts {
                if let Some(level) = p.strip_prefix('L') {
                    out.push(("Format_Level".to_string(), level.to_string()));
                } else if !p.is_empty() {
                    out.push(("Format_Tier".to_string(), p.to_string()));
                }
            }
            out
        }
        _ => vec![(name.to_string(), text.to_string())],
    }
}

fn ms_to_seconds(text: &str) -> String {
    let t = text.trim();
    let Ok(v) = t.parse::<f64>() else { return t.to_string() };
    let decimals = t.split_once('.').map(|(_, d)| d.len()).unwrap_or(0) + 3;
    format!("{:.*}", decimals, v / 1000.0)
}

/// Fields of a stream in export order, already transformed: (name, value, is_extra).
pub fn export_fields(stream: &Stream) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    let mut seen_level = false;
    let mut seen_tier = false;
    for i in 0..stream.count() {
        let Some((name, text, measure, options)) = stream.field(i) else { continue };
        if text.is_empty() || !in_export(stream, i, options) {
            continue;
        }
        if matches!(name, "Count" | "StreamCount" | "StreamKind" | "StreamKind/String" | "StreamKindID" | "StreamKindPos" | "Status" | "Inform") {
            continue;
        }
        let extra = i >= stream.schema_len();
        for (n, v) in export_value(stream.kind, name, text, measure) {
            if n == "Format_Level" {
                if seen_level {
                    continue;
                }
                seen_level = true;
            }
            if n == "Format_Tier" {
                if seen_tier {
                    continue;
                }
                seen_tier = true;
            }
            if v.is_empty() {
                continue;
            }
            out.push((export_name(&n), v, extra));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_values() {
        assert_eq!(export_name("Channel(s)"), "Channels");
        assert_eq!(export_name("BitRate_Mode/String"), "BitRate_Mode_String");
        assert_eq!(export_name("00:00:00.000"), "_00_00_00_000");
        assert_eq!(export_name("Menu For"), "Menu_For");
        assert_eq!(export_name("Bits-(Pixel*Frame)"), "Bits_Pixel_Frame");
        assert_eq!(ms_to_seconds("1021"), "1.021");
        assert_eq!(ms_to_seconds("1000.000000"), "1.000000000");
        assert_eq!(ms_to_seconds("-3"), "-0.003");
        let v = export_value(StreamKind::Video, "Format_Profile", "Main 10@L1@Main", "");
        assert_eq!(v, vec![("Format_Profile".into(), "Main 10".into()), ("Format_Level".into(), "1".into()), ("Format_Tier".into(), "Main".into())]);
    }
}

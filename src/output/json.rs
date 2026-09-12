//! MediaInfo JSON output.

use super::export_fields;
use crate::model::{Doc, StreamKind};

pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

pub fn render(doc: &Doc, complete: bool) -> String {
    let _ = complete;
    let mut out = String::new();
    out.push_str("{\n\"creatingLibrary\": {\n");
    out.push_str(&format!("\"name\": \"mediainfo-rust\",\n\"version\": {},\n\"url\": \"https://github.com/sandwichfarm/mediainfo-rust\"\n}},\n", escape(crate::VERSION)));
    out.push_str("\"media\": {\n");
    out.push_str(&format!("\"@ref\": {},\n", escape(doc.general_ref().get("CompleteName"))));
    out.push_str("\"track\": [\n");
    let mut first_track = true;
    for kind in StreamKind::ALL {
        let streams = &doc.streams[kind as usize];
        for (i, s) in streams.iter().enumerate() {
            if !first_track {
                out.push_str(",\n");
            }
            first_track = false;
            out.push_str("{\n");
            out.push_str(&format!("\"@type\": \"{}\"", kind.name()));
            if streams.len() > 1 {
                out.push_str(&format!(",\n\"@typeorder\": \"{}\"", i + 1));
            }
            let fields = export_fields(s);
            let mut in_extra = false;
            for (n, v, extra) in fields {
                if extra && !in_extra {
                    out.push_str(",\n\"extra\": {\n");
                    out.push_str(&format!("{}: {}", escape(&n), escape(&v)));
                    in_extra = true;
                    continue;
                }
                out.push_str(",\n");
                out.push_str(&format!("{}: {}", escape(&n), escape(&v)));
            }
            if in_extra {
                out.push_str("\n}");
            }
            out.push_str("\n}");
        }
    }
    out.push_str("\n]\n}\n}\n");
    out
}

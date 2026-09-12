//! MediaInfo XML (2.0) and the legacy "OLDXML" layout.

use super::export_fields;
use crate::model::{Doc, StreamKind};

pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' && c != '\r' => {}
            c => o.push(c),
        }
    }
    o
}

pub fn render(doc: &Doc, complete: bool, old: bool) -> String {
    let _ = complete;
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    if old {
        out.push_str(&format!("<Mediainfo version=\"{}\">\n<File>\n", crate::VERSION));
        for kind in StreamKind::ALL {
            let streams = &doc.streams[kind as usize];
            for (i, s) in streams.iter().enumerate() {
                if streams.len() > 1 {
                    out.push_str(&format!("<track type=\"{}\" streamid=\"{}\">\n", kind.name(), i + 1));
                } else {
                    out.push_str(&format!("<track type=\"{}\">\n", kind.name()));
                }
                for k in 0..s.count() {
                    let Some((name, text, _, options)) = s.field(k) else { continue };
                    if text.is_empty() || options.as_bytes().first() != Some(&b'Y') {
                        continue;
                    }
                    let n = super::export_name(crate::model::labels::label(name)).replace("__", "_");
                    out.push_str(&format!("<{n}>{}</{n}>\n", escape(text)));
                }
                out.push_str("</track>\n");
            }
        }
        out.push_str("</File>\n</Mediainfo>\n");
        return out;
    }
    out.push_str("<MediaInfo\n    xmlns=\"https://mediaarea.net/mediainfo\"\n    xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"\n    xsi:schemaLocation=\"https://mediaarea.net/mediainfo https://mediaarea.net/mediainfo/mediainfo_2_0.xsd\"\n    version=\"2.0\">\n");
    out.push_str(&format!("<creatingLibrary version=\"{}\" url=\"https://github.com/sandwichfarm/mediainfo-rust\">mediainfo-rust</creatingLibrary>\n", crate::VERSION));
    let name = doc.general_ref().get("CompleteName").to_string();
    out.push_str(&format!("<media ref=\"{}\">\n", escape(&name)));
    for kind in StreamKind::ALL {
        let streams = &doc.streams[kind as usize];
        for (i, s) in streams.iter().enumerate() {
            if streams.len() > 1 {
                out.push_str(&format!("<track type=\"{}\" typeorder=\"{}\">\n", kind.name(), i + 1));
            } else {
                out.push_str(&format!("<track type=\"{}\">\n", kind.name()));
            }
            let fields = export_fields(s);
            let mut in_extra = false;
            for (n, v, extra) in fields {
                if extra && !in_extra {
                    out.push_str("<extra>\n");
                    in_extra = true;
                }
                out.push_str(&format!("<{n}>{}</{n}>\n", escape(&v)));
            }
            if in_extra {
                out.push_str("</extra>\n");
            }
            out.push_str("</track>\n");
        }
    }
    out.push_str("</media>\n</MediaInfo>\n");
    out
}

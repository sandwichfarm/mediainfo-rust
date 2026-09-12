//! Default text report: `label padded to 41 : value`, one block per stream.

use crate::model::labels::label;
use crate::model::{Doc, Stream, StreamKind};

pub const LABEL_WIDTH: usize = 41;

pub fn render(doc: &Doc, complete: bool, raw_names: bool) -> String {
    let mut out = String::new();
    for kind in StreamKind::ALL {
        let streams = &doc.streams[kind as usize];
        for (i, s) in streams.iter().enumerate() {
            if !out.is_empty() {
                out.push('\n');
            }
            if streams.len() > 1 {
                out.push_str(&format!("{} #{}\n", kind.name(), i + 1));
            } else {
                out.push_str(kind.name());
                out.push('\n');
            }
            render_stream(&mut out, s, complete, raw_names);
        }
    }
    out
}

fn render_stream(out: &mut String, s: &Stream, complete: bool, raw_names: bool) {
    for i in 0..s.count() {
        let Some((name, text, _measure, options)) = s.field(i) else { continue };
        if text.is_empty() {
            continue;
        }
        let shown = if complete { true } else { options.as_bytes().first() == Some(&b'Y') };
        if !shown {
            continue;
        }
        let lab = if raw_names { name } else { label(name) };
        push_line(out, lab, text);
    }
}

pub fn push_line(out: &mut String, lab: &str, text: &str) {
    out.push_str(lab);
    let w = lab.chars().count();
    for _ in w..LABEL_WIDTH {
        out.push(' ');
    }
    out.push_str(": ");
    out.push_str(text);
    out.push('\n');
}

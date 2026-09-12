//! `--Inform=` templates: `Kind;text with %Field% placeholders`, sections separated by `\n`.
//! `$if(%A%,yes,no)` is supported as in the reference tool.

use crate::model::{Doc, Stream, StreamKind};

pub fn render(doc: &Doc, template: &str) -> String {
    let template = template.replace("\\n", "\n").replace("\\r", "\r").replace("\\t", "\t");
    // Sections: "General;..." "Video;..." — a template without a kind prefix applies to General.
    let mut sections: Vec<(StreamKind, String)> = Vec::new();
    let mut leftover = String::new();
    for line in template.split_inclusive('\n') {
        let (body, nl) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        let mut matched = false;
        for kind in StreamKind::ALL {
            let prefix = format!("{};", kind.name());
            if let Some(rest) = body.strip_prefix(&prefix) {
                sections.push((kind, format!("{rest}{nl}")));
                matched = true;
                break;
            }
        }
        if !matched {
            if let Some((kind, text)) = sections.last_mut() {
                let _ = kind;
                text.push_str(line);
            } else {
                leftover.push_str(line);
            }
        }
    }
    if sections.is_empty() {
        return doc.streams[0].iter().map(|s| expand(s, &leftover)).collect();
    }
    let mut out = leftover;
    for (kind, text) in sections {
        for s in &doc.streams[kind as usize] {
            out.push_str(&expand(s, &text));
        }
    }
    out
}

fn expand(s: &Stream, text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                out.push_str(s.get(name));
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    expand_if(&out)
}

/// `$if(cond,then,else)` where cond is the (already expanded) value.
fn expand_if(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("$if(") {
        out.push_str(&rest[..start]);
        let body = &rest[start + 4..];
        let mut depth = 1;
        let mut end = None;
        for (i, c) in body.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            out.push_str(rest);
            return out;
        };
        let args: Vec<&str> = body[..end].splitn(3, ',').collect();
        let cond = args.first().copied().unwrap_or("");
        if !cond.is_empty() {
            out.push_str(args.get(1).copied().unwrap_or(""));
        } else {
            out.push_str(args.get(2).copied().unwrap_or(""));
        }
        rest = &body[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_fields() {
        let mut doc = Doc::new();
        doc.general().set("FileName", "movie");
        let v = doc.add(StreamKind::Video);
        v.set("Width", "64");
        assert_eq!(render(&doc, "General;%FileName%\\nVideo;%Width%x%Height%\\n"), "movie\n64x\n");
        assert_eq!(render(&doc, "%FileName% $if(%Duration%,has,none)"), "movie none");
    }
}

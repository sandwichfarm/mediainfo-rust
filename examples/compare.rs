//! Compare this library's output with the reference dumps in tests/oracle.
//!
//! `cargo run --example compare [-v] [fixture-name ...]`
//! Prints, per fixture, the fields that differ (missing / extra / different) for every stream.

use mediainfo::{MediaInfo, StreamKind};
use std::collections::BTreeMap;
use std::path::Path;

type Streams = Vec<(String, BTreeMap<String, String>)>;

fn parse_raw(text: &str) -> Streams {
    let mut out: Streams = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("== ") {
            let kind = rest.split_whitespace().next().unwrap_or("").to_string();
            out.push((kind, BTreeMap::new()));
        } else {
            let p: Vec<&str> = line.split('\t').collect();
            if p.len() >= 3 {
                if let Some(last) = out.last_mut() {
                    last.1.insert(p[1].to_string(), p[2].replace("\\n", "\n"));
                }
            }
        }
    }
    out
}

// Fields whose value is expected to differ or is uninteresting for parser work.
const IGNORE: &[&str] = &["Count", "CompleteName", "FolderName", "File_Modified_Date", "File_Modified_Date_Local", "File_Created_Date", "File_Created_Date_Local", "StreamKindPos", "Inform"];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbose = args.iter().any(|a| a == "-v");
    let filter: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("tests/fixtures");
    let oracle = root.join("tests/oracle");
    let mut names: Vec<String> = std::fs::read_dir(&fixtures).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n != "generate.sh").collect();
    names.sort();
    let (mut total_ok, mut total_fields) = (0usize, 0usize);
    for name in names {
        if !filter.is_empty() && !filter.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        let raw_path = oracle.join(format!("{name}.raw.txt"));
        let Ok(raw) = std::fs::read_to_string(&raw_path) else { continue };
        let expected = parse_raw(&raw);
        let mut mi = MediaInfo::new();
        mi.open(fixtures.join(&name));
        let mut actual: Streams = Vec::new();
        for kind in StreamKind::ALL {
            for s in mi.streams(kind) {
                let mut m = BTreeMap::new();
                for (n, t, _, _) in s.fields() {
                    m.insert(n.to_string(), t.to_string());
                }
                actual.push((kind.name().to_string(), m));
            }
        }
        let exp_kinds: Vec<&str> = expected.iter().map(|(k, _)| k.as_str()).collect();
        let act_kinds: Vec<&str> = actual.iter().map(|(k, _)| k.as_str()).collect();
        let mut lines = Vec::new();
        let (mut ok, mut n) = (0usize, 0usize);
        if exp_kinds != act_kinds {
            lines.push(format!("  streams: expected {exp_kinds:?} got {act_kinds:?}"));
        }
        for (i, (kind, exp)) in expected.iter().enumerate() {
            let act = actual.get(i).map(|(_, m)| m.clone()).unwrap_or_default();
            for (k, v) in exp {
                if IGNORE.contains(&k.as_str()) {
                    continue;
                }
                n += 1;
                match act.get(k) {
                    Some(a) if a == v => ok += 1,
                    Some(a) => lines.push(format!("  {kind}#{i} {k}: expected {v:?} got {a:?}")),
                    None => lines.push(format!("  {kind}#{i} {k}: missing (expected {v:?})")),
                }
            }
            for (k, a) in &act {
                if !exp.contains_key(k) && !IGNORE.contains(&k.as_str()) {
                    lines.push(format!("  {kind}#{i} {k}: extra {a:?}"));
                }
            }
        }
        total_ok += ok;
        total_fields += n;
        println!("{name}: {ok}/{n} fields match");
        if verbose || !filter.is_empty() {
            for l in lines {
                println!("{l}");
            }
        }
    }
    println!("TOTAL: {total_ok}/{total_fields}");
}

//! Field-by-field comparison with the reference dumps for the fields that matter most.
//!
//! For every fixture with an oracle dump, the stream layout must match and every *key* field the
//! reference reports must be reported with the same value. Known, accepted deviations are listed
//! in `ALLOWED` with the reason.

use mediainfo::{MediaInfo, StreamKind};
use std::collections::BTreeMap;
use std::path::Path;

const KEY_FIELDS: &[&str] = &[
    "Format", "Format_Version", "Format_Profile", "Format_Settings", "Format_AdditionalFeatures", "CodecID", "Duration",
    "Width", "Height", "PixelAspectRatio", "DisplayAspectRatio", "FrameRate", "FrameRate_Mode", "FrameCount", "ColorSpace",
    "ChromaSubsampling", "BitDepth", "ScanType", "Channel(s)", "ChannelPositions", "ChannelLayout", "SamplingRate", "SamplesPerFrame",
    "BitRate", "BitRate_Mode", "Compression_Mode", "Language", "Default", "Forced", "Title", "Encoded_Library", "Encoded_Application",
    "UniqueID", "ID", "StreamOrder", "FileSize", "OverallBitRate", "VideoCount", "AudioCount", "TextCount", "MenuCount",
    "Chapters_Pos_Begin", "Chapters_Pos_End",
];

/// (fixture, stream kind, field) → reason.
const ALLOWED: &[(&str, &str, &str, &str)] = &[
    ("wavpack.wv", "General", "Duration", "reference truncates to 2 blocks; we report the header total"),
    ("wavpack.wv", "Audio", "Duration", "reference truncates to 2 blocks; we report the header total"),
    ("wavpack.wv", "General", "OverallBitRate", "follows Duration"),
    ("wavpack.wv", "Audio", "BitRate", "follows Duration"),
];

fn parse_raw(text: &str) -> Vec<(String, BTreeMap<String, String>)> {
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("== ") {
            out.push((rest.split_whitespace().next().unwrap_or("").to_string(), BTreeMap::new()));
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

#[test]
fn key_fields_match_reference() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("tests/fixtures");
    let oracle = root.join("tests/oracle");
    let mut names: Vec<String> = std::fs::read_dir(&fixtures).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n != "generate.sh").collect();
    names.sort();
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for name in &names {
        let Ok(raw) = std::fs::read_to_string(oracle.join(format!("{name}.raw.txt"))) else { continue };
        let expected = parse_raw(&raw);
        let mut mi = MediaInfo::new();
        mi.open(fixtures.join(name));
        let mut actual: Vec<(String, &mediainfo::Stream)> = Vec::new();
        for kind in StreamKind::ALL {
            for s in mi.streams(kind) {
                actual.push((kind.name().to_string(), s));
            }
        }
        let exp_kinds: Vec<&str> = expected.iter().map(|(k, _)| k.as_str()).collect();
        let act_kinds: Vec<&str> = actual.iter().map(|(k, _)| k.as_str()).collect();
        if exp_kinds != act_kinds {
            failures.push(format!("{name}: streams expected {exp_kinds:?} got {act_kinds:?}"));
            continue;
        }
        for (i, (kind, exp)) in expected.iter().enumerate() {
            let act = actual[i].1;
            for field in KEY_FIELDS {
                let Some(v) = exp.get(*field) else { continue };
                checked += 1;
                let a = act.get(field);
                if a != v && !ALLOWED.iter().any(|(f, k, fl, _)| f == name && k == kind && fl == field) {
                    failures.push(format!("{name}: {kind}#{i} {field}: expected {v:?} got {a:?}"));
                }
            }
        }
    }
    assert!(checked > 1000, "too few fields checked: {checked}");
    if !failures.is_empty() {
        panic!("{} key-field mismatches:\n{}", failures.len(), failures.join("\n"));
    }
}

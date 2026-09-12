//! End-to-end tests of the command line tool and the public API.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_mediainfo")).args(args).output().expect("run mediainfo");
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn version_and_help() {
    let (code, out, _) = run(&["--Version"]);
    assert_eq!(code, 0);
    assert!(out.contains("mediainfo-rust"));
    let (code, out, _) = run(&["--Help"]);
    assert_eq!(code, 0);
    assert!(out.contains("--Output"));
    let (code, _, _) = run(&[]);
    assert_eq!(code, 1);
}

#[test]
fn text_report_matches_reference_layout() {
    let (code, out, _) = run(&[fixture("h264_aac_srt.mkv").to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(out.starts_with("General\n"));
    assert!(out.contains("Format                                   : Matroska\n"));
    assert!(out.contains("\nVideo\n"));
    assert!(out.contains("Width                                    : 64 pixels\n"));
    assert!(out.ends_with("\n\n"));
}

#[test]
fn xml_and_json_outputs() {
    let f = fixture("h264_aac_srt.mkv");
    let (code, out, _) = run(&["--Output=XML", f.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(out.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MediaInfo"));
    assert!(out.contains("<track type=\"Video\">"));
    assert!(out.contains("<Width>64</Width>"));
    assert!(out.contains("<Duration>1.021</Duration>"));
    assert!(out.trim_end().ends_with("</MediaInfo>"));
    let (code, out, _) = run(&["--Output=JSON", f.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(out.contains("\"@type\": \"Video\""));
    assert!(out.contains("\"Width\": \"64\""));
    // Balanced braces is a cheap well-formedness check without a JSON parser.
    assert_eq!(out.matches('{').count(), out.matches('}').count());
}

#[test]
fn template_and_raw_language() {
    let f = fixture("h264_aac_srt.mkv");
    let (code, out, _) = run(&["--Inform=General;%Format% %Duration%\\nVideo;%Width%x%Height%", f.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(out, "Matroska 1021\n64x48\n");
    let (_, out, _) = run(&["--Language=raw", f.to_str().unwrap()]);
    assert!(out.contains("FileSize/String                          : 10.0 KiB"));
}

#[test]
fn full_report_lists_hidden_fields() {
    let (_, out, _) = run(&["--Full", fixture("h264_aac_srt.mkv").to_str().unwrap()]);
    assert!(out.contains("Count                                    : "));
    assert!(out.contains("Video_Format_List"));
}

#[test]
fn missing_file_and_unknown_option() {
    let (code, _, err) = run(&["/nonexistent/file.mkv"]);
    assert_eq!(code, 1);
    assert!(err.contains("not found"));
    let (code, _, _) = run(&["--NoSuchOption=1", fixture("empty.bin").to_str().unwrap()]);
    assert_eq!(code, 2);
}

#[test]
fn unknown_and_empty_files_do_not_panic() {
    for name in ["empty.bin", "random.bin", "text.txt"] {
        let (code, out, _) = run(&[fixture(name).to_str().unwrap()]);
        assert_eq!(code, 0, "{name}");
        assert!(out.starts_with("General\n"), "{name}");
        assert!(!out.contains("Format "), "{name} should not be recognised");
    }
}

#[test]
fn api_get_and_count() {
    use mediainfo::{InfoKind, MediaInfo, StreamKind};
    let mut mi = MediaInfo::new();
    assert!(mi.open(fixture("h264_aac_srt.mkv")));
    assert_eq!(mi.count_get(StreamKind::Video, None), 1);
    assert_eq!(mi.count_get(StreamKind::Audio, None), 1);
    assert_eq!(mi.count_get(StreamKind::Text, None), 1);
    assert_eq!(mi.count_get(StreamKind::Menu, None), 1);
    assert_eq!(mi.get(StreamKind::General, 0, "Format", InfoKind::Text), "Matroska");
    assert_eq!(mi.get(StreamKind::Video, 0, "Width", InfoKind::Text), "64");
    assert_eq!(mi.get(StreamKind::Video, 0, "Width", InfoKind::Measure), " pixel");
    assert_eq!(mi.get(StreamKind::Video, 0, "Width", InfoKind::NameText), "Width");
    assert_eq!(mi.get(StreamKind::Video, 0, "Width", InfoKind::Options), "N YIY");
    assert_eq!(mi.get_i(StreamKind::General, 0, 89, InfoKind::Name), "FileSize");
    assert_eq!(mi.get(StreamKind::Text, 0, "Language", InfoKind::Text), "ja");
    assert_eq!(mi.get(StreamKind::Text, 0, "Language/String", InfoKind::Text), "Japanese");
    // Chapters: Chapters_Pos_Begin/End bracket the dynamic entries.
    let begin: usize = mi.get(StreamKind::Menu, 0, "Chapters_Pos_Begin", InfoKind::Text).parse().unwrap();
    let end: usize = mi.get(StreamKind::Menu, 0, "Chapters_Pos_End", InfoKind::Text).parse().unwrap();
    assert_eq!(end - begin, 2);
    assert_eq!(mi.get_i(StreamKind::Menu, 0, begin, InfoKind::Name), "00:00:00.000");
    assert_eq!(mi.get_i(StreamKind::Menu, 0, begin, InfoKind::Text), ":Intro");
    assert_eq!(mi.option("Info_Version", ""), format!("MediaInfoLib - v{} (mediainfo-rust)", mediainfo::VERSION));
    mi.close();
    assert_eq!(mi.count_get(StreamKind::Video, None), 0);
}

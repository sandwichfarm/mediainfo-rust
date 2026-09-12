//! Derivation pass: fills every `*/String*`, list, count and computed field from the base values
//! that parsers set. Runs once after parsing.

pub mod format;
pub mod language;
pub mod tables;

use crate::model::{Doc, Stream, StreamKind};
use format::*;
use std::path::Path;
use std::time::SystemTime;

/// Facts about the input that come from the file system rather than the parser.
#[derive(Debug, Clone, Default)]
pub struct FileInfo {
    pub path: Option<std::path::PathBuf>,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// Derived fields only ever land in schema slots — never as new dynamic fields.
trait Derived {
    fn derive(&mut self, name: &str, value: impl Into<String>);
}

impl Derived for Stream {
    fn derive(&mut self, name: &str, value: impl Into<String>) {
        if self.kind.index_of(name).is_some() {
            self.set_if_empty(name, value);
        }
    }
}

pub fn finish(doc: &mut Doc, file: &FileInfo) {
    fill_file_info(doc.general(), file);

    // Streams first (General aggregates them).
    for kind in StreamKind::ALL.iter().skip(1) {
        let n = doc.count(*kind);
        for i in 0..n {
            let s = &mut doc.streams[*kind as usize][i];
            finish_common(s);
            match kind {
                StreamKind::Video => finish_video(s),
                StreamKind::Audio => finish_audio(s),
                StreamKind::Image => finish_image(s),
                StreamKind::Text => finish_text(s),
                _ => {}
            }
            s.set_int("StreamKindID", i as i128);
            if n > 1 {
                s.set_int("StreamKindPos", (i + 1) as i128);
            }
        }
    }

    finish_general(doc);

    // Count/StreamCount/StreamKind on every stream, last (Count includes dynamic fields).
    for kind in StreamKind::ALL {
        let n = doc.count(kind);
        for i in 0..n {
            let s = &mut doc.streams[kind as usize][i];
            s.set("StreamKind", kind.name());
            s.set("StreamKind/String", kind.name());
            s.set_int("StreamCount", n as i128);
            s.set_int("StreamKindID", i as i128);
            if n > 1 {
                s.set_int("StreamKindPos", (i + 1) as i128);
            }
            let c = s.count();
            s.set_int("Count", c as i128);
        }
    }
}

fn fill_file_info(g: &mut Stream, file: &FileInfo) {
    if let Some(p) = &file.path {
        g.set("CompleteName", p.to_string_lossy());
        if let Some(d) = p.parent() {
            g.set("FolderName", d.to_string_lossy());
        }
        if let Some(n) = p.file_name() {
            let n = n.to_string_lossy();
            g.set("FileNameExtension", n.as_ref());
            match n.rfind('.') {
                Some(i) if i > 0 => {
                    g.set("FileName", &n[..i]);
                    g.set("FileExtension", &n[i + 1..]);
                }
                _ => g.set("FileName", n.as_ref()),
            }
        }
    }
    if !g.has("FileSize") {
        g.set_int("FileSize", file.size as i128);
    }
    if let Some(m) = file.modified {
        if let Ok(d) = m.duration_since(SystemTime::UNIX_EPOCH) {
            let secs = d.as_secs() as i64;
            g.set("File_Modified_Date", format!("UTC {}", format_datetime(secs)));
            g.set("File_Modified_Date_Local", format_datetime(secs + local_utc_offset()));
        }
    }
}

/// `YYYY-MM-DD HH:MM:SS` for a Unix timestamp.
pub fn format_datetime(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, (rem / 60) % 60, rem % 60)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Offset of local time from UTC in seconds, read from `TZ`-independent libc-free heuristics:
/// we parse `/etc/localtime` is out of scope, so use the `TZ_OFFSET` env var if set, else 0.
fn local_utc_offset() -> i64 {
    std::env::var("MEDIAINFO_TZ_OFFSET").ok().and_then(|v| v.parse().ok()).unwrap_or(0)
}

// ---------------------------------------------------------------------------- common

fn finish_common(s: &mut Stream) {
    let kind = s.kind;
    // Format family
    let format = s.get("Format").to_string();
    if !format.is_empty() {
        if !s.has("Format/String") {
            let add = s.get("Format_AdditionalFeatures").to_string();
            s.set("Format/String", if add.is_empty() { format.clone() } else { format!("{format} {add}") });
        }
        if let Some(fi) = tables::format_info(kind, &format) {
            s.derive("Format/Info", fi.info);
            s.derive("Format/Url", fi.url);
            s.derive("Format_Commercial", fi.commercial);
            s.derive("InternetMediaType", fi.mime);
            if kind == StreamKind::General {
                s.set_if_empty("Format/Extensions", fi.extensions);
            }
        }
        s.set_if_empty("Format_Commercial", &format);
        let com = s.get("Format_Commercial").to_string();
        if com != format && !com.is_empty() && !s.has("Format_Commercial_IfAny") {
            s.set("Format_Commercial_IfAny", com);
        }
    }
    // Codec ID family
    let codec_id = s.get("CodecID").to_string();
    if !codec_id.is_empty() {
        if let Some(ci) = tables::codec_id_info(&codec_id) {
            s.derive("CodecID/Info", ci.info);
            s.derive("CodecID/Hint", ci.hint);
            s.derive("CodecID/Url", ci.url);
            s.derive("CodecID_Description", ci.description);
        }
    }
    // Identifiers
    if s.has("ID") && !s.has("ID/String") {
        s.set("ID/String", s.get("ID").to_string());
    }
    if kind == StreamKind::General && !s.has("UniqueID/String") {
        if let Ok(v) = s.get("UniqueID").parse::<u128>() {
            s.set("UniqueID/String", format!("{v} (0x{v:X})"));
        }
    }
    // Durations
    let fps = if matches!(kind, StreamKind::Video | StreamKind::General) { s.get_f64("FrameRate") } else { None };
    for base in ["Duration", "Duration_FirstFrame", "Duration_LastFrame", "Source_Duration", "Source_Duration_FirstFrame", "Source_Duration_LastFrame"] {
        if let Some(ms) = s.get_f64(base) {
            let d = duration_strings(ms, if base == "Duration" { fps } else { None });
            set_strings(s, base, &d, base == "Duration");
        }
    }
    for base in ["Delay", "Delay_Original", "Video_Delay", "Video0_Delay", "TimeStamp_FirstFrame"] {
        if let Some(ms) = s.get_f64(base) {
            let d = duration_strings(ms, None);
            if ms != 0.0 {
                s.derive(&format!("{base}/String"), &d[0]);
                s.derive(&format!("{base}/String1"), &d[1]);
                s.derive(&format!("{base}/String2"), &d[2]);
            }
            s.derive(&format!("{base}/String3"), &d[3]);
        }
    }
    if s.has("Delay_Source") {
        let v = s.get("Delay_Source").to_string();
        s.derive("Delay_Source/String", if v == "Stream" { "Raw stream".to_string() } else { v });
    }
    // Bit rates
    for base in ["BitRate", "BitRate_Minimum", "BitRate_Nominal", "BitRate_Maximum", "BitRate_Encoded", "OverallBitRate", "OverallBitRate_Minimum", "OverallBitRate_Nominal", "OverallBitRate_Maximum"] {
        let v = s.get(base).to_string();
        if !v.is_empty() && !s.has(&format!("{base}/String")) {
            // Multiple values separated by " / " (e.g. per-substream) are formatted individually.
            let parts: Vec<String> = v.split(" / ").filter_map(|p| p.trim().parse::<f64>().ok()).map(bitrate_string).collect();
            if !parts.is_empty() {
                s.set(&format!("{base}/String"), parts.join(" / "));
            }
        }
    }
    for base in ["BitRate_Mode", "OverallBitRate_Mode"] {
        let v = s.get(base).to_string();
        if !v.is_empty() {
            let t = match v.as_str() {
                "CBR" => "Constant",
                "VBR" => "Variable",
                other => other,
            };
            s.derive(&format!("{base}/String"), t);
        }
    }
    // Sizes
    let file_size = s.get_f64("FileSize");
    if kind == StreamKind::General {
        if let Some(b) = s.get_u64("FileSize") {
            let z = size_strings(b);
            set_strings(s, "FileSize", &[z[0].clone(), z[1].clone(), z[2].clone(), z[3].clone(), z[4].clone()], false);
        }
    }
    for base in ["StreamSize", "Source_StreamSize", "StreamSize_Encoded", "Source_StreamSize_Encoded", "StreamSize_Demuxed"] {
        if let Some(b) = s.get_u64(base) {
            let z = size_strings(b);
            let pct = s.get_f64("__FileSize").or(file_size).filter(|f| *f >= b as f64).map(|f| format!(" ({}%)", ((b as f64) / f * 100.0).round() as u64)).unwrap_or_default();
            s.derive(&format!("{base}/String"), format!("{}{pct}", z[0]));
            s.derive(&format!("{base}/String1"), &z[1]);
            s.derive(&format!("{base}/String2"), &z[2]);
            s.derive(&format!("{base}/String3"), &z[3]);
            s.derive(&format!("{base}/String4"), &z[4]);
            s.derive(&format!("{base}/String5"), format!("{}{pct}", z[0]));
            if let Some(f) = s.get_f64("__FileSize").or(file_size).filter(|f| *f >= b as f64) {
                if base != "StreamSize_Demuxed" {
                    s.derive(&format!("{base}_Proportion"), proportion(b as f64, f));
                }
            }
        }
    }
    // Frame rates
    for base in ["FrameRate", "FrameRate_Minimum", "FrameRate_Nominal", "FrameRate_Maximum", "FrameRate_Original"] {
        if let Some(f) = s.get_f64(base) {
            if kind == StreamKind::Audio && base == "FrameRate" {
                continue; // handled with SPF in finish_audio
            }
            s.derive(&format!("{base}/String"), frame_rate_string(f, None));
        }
    }
    for base in ["FrameRate_Mode"] {
        let v = s.get(base).to_string();
        if !v.is_empty() {
            let t = match v.as_str() {
                "CFR" => "Constant",
                "VFR" => "Variable",
                o => o,
            };
            s.derive(&format!("{base}/String"), t);
        }
    }
    // Language / flags
    let lang = s.get("Language").to_string();
    if !lang.is_empty() {
        let lang = language::normalize(&lang);
        s.set("Language", &lang);
        let l = language::strings(&lang);
        set_strings(s, "Language", &[l[0].clone(), l[1].clone(), l[2].clone(), l[3].clone(), l[4].clone()], false);
    }
    for base in ["Default", "Forced", "Disabled", "AlternateGroup", "ServiceKind", "Compression_Mode", "ScanType", "ScanOrder", "ScanType_Original", "ScanOrder_Original", "ScanType_StoreMethod", "Interlacement", "Compilation", "Alignment", "Format_Settings_CABAC", "Format_Settings_BVOP", "Format_Settings_QPel", "Format_Settings_GMC", "Format_Settings_Matrix", "Format_Settings_SBR", "Format_Settings_PS", "Encoded_Application", "Gop_OpenClosed", "Gop_OpenClosed_FirstFrame", "ActiveFormatDescription", "Interleave_Duration", "Interleave_Preload", "ReplayGain_Gain", "Album_ReplayGain_Gain", "ChromaSubsampling", "Resolution", "OriginalSourceMedium_ID", "MenuID"] {
        let v = s.get(base).to_string();
        if !v.is_empty() {
            let text = match base {
                "Format_Settings_GMC" => format!("{v} warppoints"),
                "Alignment" => match v.as_str() {
                    "Aligned" => "Aligned on interleaves".to_string(),
                    "Split" => "Split across interleaves".to_string(),
                    _ => v.clone(),
                },
                "Interleave_Duration" => {
                    let frames = s.get("Interleave_VideoFrames").to_string();
                    if frames.is_empty() { format!("{v}  ms") } else { format!("{v}  ms ({frames} video frame)") }
                }
                "Interleave_Preload" => format!("{v}  ms"),
                "ReplayGain_Gain" | "Album_ReplayGain_Gain" => format!("{v} dB"),
                "ScanOrder" | "ScanOrder_Original" => match v.as_str() {
                    "TFF" => "Top Field First".to_string(),
                    "BFF" => "Bottom Field First".to_string(),
                    "2:3 Pulldown" => "2:3 Pulldown".to_string(),
                    _ => v.clone(),
                },
                "Interlacement" => match v.as_str() {
                    "PPF" => "Progressive".to_string(),
                    "TFF" => "Top Field First".to_string(),
                    "BFF" => "Bottom Field First".to_string(),
                    _ => v.clone(),
                },
                _ => v.clone(),
            };
            s.derive(&format!("{base}/String"), text);
        }
    }
    if let Some(n) = s.get_u64("Format_Settings_RefFrames") {
        s.derive("Format_Settings_RefFrames/String", frames_string(n));
    }
    // Writing library "Name - Version" → String/Name/Version
    for base in ["Encoded_Library", "Encoded_Application"] {
        let v = s.get(base).to_string();
        if v.is_empty() {
            continue;
        }
        let name = s.get(&format!("{base}_Name")).to_string();
        let version = s.get(&format!("{base}_Version")).to_string();
        if !name.is_empty() {
            let mut str = name.clone();
            if !version.is_empty() {
                str.push(' ');
                str.push_str(&version);
            }
            let date = s.get(&format!("{base}_Date")).to_string();
            if !date.is_empty() {
                str.push_str(&format!(" ({date})"));
            }
            s.derive(&format!("{base}/String"), str);
        } else if let Some((n, ver)) = v.split_once(" - ") {
            s.derive(&format!("{base}_Name"), n);
            s.derive(&format!("{base}_Version"), ver);
            s.derive(&format!("{base}/String"), format!("{n} {ver}"));
        } else {
            s.derive(&format!("{base}/String"), &v);
        }
    }
    if let Some(v) = s.get_u64("Width") {
        s.derive("Width/String", pixels_string(v));
    }
    if let Some(v) = s.get_u64("Height") {
        s.derive("Height/String", pixels_string(v));
    }
    for base in ["Width_Original", "Height_Original", "Width_CleanAperture", "Height_CleanAperture", "Width_Offset", "Height_Offset"] {
        if let Some(v) = s.get_u64(base) {
            s.derive(&format!("{base}/String"), pixels_string(v));
        }
    }
    if let Some(v) = s.get_u64("BitDepth") {
        s.derive("BitDepth/String", bits_string(v));
    }
    for base in ["BitDepth_Detected", "BitDepth_Stored"] {
        if let Some(v) = s.get_u64(base) {
            s.derive(&format!("{base}/String"), bits_string(v));
        }
    }
    for base in ["Encoded_Application", "Encoded_Library"] {
        if !s.has(base) && s.has(&format!("{base}_Name")) {
            let name = s.get(&format!("{base}_Name")).to_string();
            let version = s.get(&format!("{base}_Version")).to_string();
            s.derive(&format!("{base}/String"), if version.is_empty() { name } else { format!("{name} {version}") });
        }
    }
    // Any remaining plain "X" with a schema "X/String" twin that is still empty gets a copy for the
    // obvious identity cases.
    for base in ["HDR_Format", "Codec", "Format_Settings_Matrix", "StreamSize_Demuxed"] {
        let v = s.get(base).to_string();
        if !v.is_empty() {
            s.derive(&format!("{base}/String"), v);
        }
    }
}

/// Set `base/String..String5` from a 5- or 6-element array (String, String1, ...).
fn set_strings(s: &mut Stream, base: &str, v: &[String], with_5: bool) {
    let names = ["/String", "/String1", "/String2", "/String3", "/String4", "/String5"];
    for (i, val) in v.iter().enumerate() {
        if i >= names.len() {
            break;
        }
        if i == 5 && !with_5 {
            continue;
        }
        if !val.is_empty() {
            s.derive(&format!("{base}{}", names[i]), val);
        }
    }
}

// ---------------------------------------------------------------------------- video

fn finish_video(s: &mut Stream) {
    let w = s.get_f64("Width");
    let h = s.get_f64("Height");
    if let (Some(w), Some(h)) = (w, h) {
        s.set_if_empty("Sampled_Width", format!("{}", w as u64));
        s.set_if_empty("Sampled_Height", format!("{}", h as u64));
        let par = s.get_f64("PixelAspectRatio");
        let dar = s.get_f64("DisplayAspectRatio");
        match (par, dar) {
            (Some(p), None) => {
                if h > 0.0 {
                    s.set("DisplayAspectRatio", f3(w * p / h));
                }
            }
            (None, Some(d)) => {
                if w > 0.0 {
                    s.set("PixelAspectRatio", f3(d * h / w));
                }
            }
            (None, None) => {
                s.set("PixelAspectRatio", "1.000");
                if h > 0.0 {
                    s.set("DisplayAspectRatio", f3(w / h));
                }
            }
            _ => {}
        }
    }
    if let Some(d) = s.get_f64("DisplayAspectRatio") {
        s.derive("DisplayAspectRatio/String", aspect_ratio_string(d));
    }
    // Frame count from duration × rate
    if !s.has("FrameCount") {
        if let (Some(d), Some(f)) = (s.get_f64("Duration"), s.get_f64("FrameRate")) {
            s.set_int("FrameCount", (d / 1000.0 * f).round() as i128);
        }
    }
    if !s.has("Duration") {
        if let (Some(n), Some(f)) = (s.get_f64("FrameCount"), s.get_f64("FrameRate")) {
            if f > 0.0 {
                let ms = n / f * 1000.0;
                s.set("Duration", format!("{}", ms.round() as i64));
                let fps = Some(f);
                let d = duration_strings(ms.round(), fps);
                set_strings(s, "Duration", &d, true);
            }
        }
    }
    if !s.has("BitRate") {
        if let (Some(sz), Some(d)) = (s.get_f64("StreamSize"), s.get_f64("Duration")) {
            if d > 0.0 {
                let br = (sz * 8.0 * 1000.0 / d).round();
                s.set("BitRate", format!("{}", br as u64));
                s.set("BitRate/String", bitrate_string(br));
            }
        }
    }
    if let (Some(br), Some(w), Some(h), Some(f)) = (s.get_f64("BitRate"), w, h, s.get_f64("FrameRate")) {
        if w > 0.0 && h > 0.0 && f > 0.0 {
            s.set_if_empty("Bits-(Pixel*Frame)", f3(br / (w * h * f)));
        }
    }
    if s.has("ChromaSubsampling") && !s.has("ColorSpace") {
        s.set("ColorSpace", "YUV");
    }
    if s.has("colour_range") {
        let v = s.get("colour_range").to_string();
        s.set_extra("colour_range", v, "", "Y YTY");
    }
}

fn finish_image(s: &mut Stream) {
    if !s.has("StreamSize") {
        if let Some(fs) = s.get_f64("__FileSize") {
            let _ = fs;
        }
    }
}

fn finish_text(s: &mut Stream) {
    let _ = s;
}

// ---------------------------------------------------------------------------- audio

fn finish_audio(s: &mut Stream) {
    if let Some(c) = s.get_u64("Channel(s)") {
        s.set_if_empty("Channel(s)/String", channels_string(c));
    }
    if let Some(c) = s.get_u64("Channel(s)_Original") {
        s.set_if_empty("Channel(s)_Original/String", channels_string(c));
    }
    if let Some(c) = s.get_u64("Matrix_Channel(s)") {
        s.set_if_empty("Matrix_Channel(s)/String", channels_string(c));
    }
    for base in ["ChannelPositions", "ChannelPositions_Original", "Matrix_ChannelPositions"] {
        let v = s.get(base).to_string();
        if !v.is_empty() {
            s.derive(&format!("{base}/String2"), channel_positions_string2(&v));
        }
    }
    if let Some(sr) = s.get_f64("SamplingRate") {
        s.set_if_empty("SamplingRate/String", sampling_rate_string(sr));
        let spf = s.get_u64("SamplesPerFrame");
        if let Some(spf) = spf {
            if spf > 0 && !s.has("FrameRate") {
                s.set("FrameRate", f3(sr / spf as f64));
            }
        }
        if let Some(f) = s.get_f64("FrameRate") {
            s.set_if_empty("FrameRate/String", frame_rate_string(f, spf));
        }
        if !s.has("SamplingCount") {
            if let Some(d) = s.get_f64("Duration") {
                s.set_int("SamplingCount", (d / 1000.0 * sr).round() as i128);
            }
        }
    }
    if s.has("FrameCount") {
        if let (Some(d), Some(f)) = (s.get_f64("Duration"), s.get_f64("FrameRate")) {
            let ds = duration_strings(d, Some(f));
            s.set_if_empty("Duration/String4", &ds[4]);
            s.set("Duration/String5", &ds[5]);
        }
    }
    if !s.has("Source_FrameCount") {
        if let (Some(d), Some(f)) = (s.get_f64("Source_Duration"), s.get_f64("FrameRate")) {
            s.set_int("Source_FrameCount", (d / 1000.0 * f).round() as i128);
        }
    }
    if !s.has("BitRate") {
        if let (Some(sz), Some(d)) = (s.get_f64("StreamSize"), s.get_f64("Duration")) {
            if d > 0.0 {
                let br = (sz * 8.0 * 1000.0 / d).round();
                s.set("BitRate", format!("{}", br as u64));
                s.set("BitRate/String", bitrate_string(br));
            }
        }
    }
    if !s.has("Duration") {
        if let (Some(sz), Some(br)) = (s.get_f64("StreamSize"), s.get_f64("BitRate")) {
            if br > 0.0 {
                let ms = (sz * 8.0 / br * 1000.0).round();
                s.set("Duration", format!("{}", ms as i64));
                set_strings(s, "Duration", &duration_strings(ms, None), true);
            }
        }
    }
}

/// `Front: L C R, Side: L R, LFE` → `3/2/0.1` (front/side/back[.lfe]).
pub fn channel_positions_string2(pos: &str) -> String {
    let (mut front, mut side, mut back, mut lfe) = (0, 0, 0, 0);
    for part in pos.split(',') {
        let part = part.trim();
        if part.eq_ignore_ascii_case("LFE") {
            lfe += 1;
            continue;
        }
        let (group, list) = match part.split_once(':') {
            Some(x) => x,
            None => continue,
        };
        let n = list.split_whitespace().count();
        match group.trim() {
            "Front" => front += n,
            "Side" => side += n,
            "Back" => back += n,
            _ => {}
        }
    }
    if lfe > 0 {
        format!("{front}/{side}/{back}.{lfe}")
    } else if pos.contains("LFE") {
        format!("{front}/{side}/{back}.0")
    } else {
        format!("{front}/{side}/{back}")
    }
}

// ---------------------------------------------------------------------------- general

fn finish_general(doc: &mut Doc) {
    let file_size = doc.general_ref().get_f64("FileSize");
    // Propagate the file size so stream proportions can be computed, then hide the helper field.
    if let Some(fs) = file_size {
        for kind in StreamKind::ALL.iter().skip(1) {
            for s in doc.streams[*kind as usize].iter_mut() {
                for base in ["StreamSize", "Source_StreamSize", "StreamSize_Encoded", "Source_StreamSize_Encoded"] {
                    if let Some(b) = s.get_u64(base).filter(|b| (*b as f64) <= fs) {
                        let z = size_strings(b);
                        let pct = format!(" ({}%)", ((b as f64) / fs * 100.0).round() as u64);
                        if s.get(&format!("{base}/String")) == z[0] {
                            s.set(&format!("{base}/String"), format!("{}{pct}", z[0]));
                            s.set(&format!("{base}/String5"), format!("{}{pct}", z[0]));
                        }
                        if fs > 0.0 {
                            s.derive(&format!("{base}_Proportion"), proportion(b as f64, fs));
                        }
                    }
                }
            }
        }
    }

    // Counts and lists
    let mut lists: Vec<(String, String)> = Vec::new();
    for kind in StreamKind::ALL.iter().skip(1) {
        let streams = &doc.streams[*kind as usize];
        if streams.is_empty() {
            continue;
        }
        let name = kind.name();
        lists.push((format!("{name}Count"), streams.len().to_string()));
        let formats: Vec<String> = streams.iter().map(|s| s.get("Format/String").to_string()).filter(|f| !f.is_empty()).collect();
        let with_hint: Vec<String> = streams
            .iter()
            .map(|s| {
                let f = s.get("Format/String").to_string();
                let hint = s.get("CodecID/Hint").to_string();
                if hint.is_empty() || f.is_empty() { f } else { format!("{f} ({hint})") }
            })
            .filter(|f| !f.is_empty())
            .collect();
        let langs: Vec<String> = streams.iter().map(|s| s.get("Language/String").to_string()).filter(|f| !f.is_empty()).collect();
        if !formats.is_empty() {
            lists.push((format!("{name}_Format_List"), formats.join(" / ")));
            lists.push((format!("{name}_Format_WithHint_List"), with_hint.join(" / ")));
            lists.push((format!("{name}_Codec_List"), formats.join(" / ")));
        }
        if !langs.is_empty() {
            lists.push((format!("{name}_Language_List"), langs.join(" / ")));
        }
    }
    let first_video: Option<Stream> = doc.streams[StreamKind::Video as usize].first().cloned();
    let stream_durations: Vec<f64> = doc.iter().filter(|s| s.kind != StreamKind::General).filter_map(|s| s.get_f64("Duration")).collect();
    // Overall mode: variable if any stream is variable, constant only if every stream is constant.
    let single_mode: Option<String> = {
        let modes: Vec<String> = doc.iter().filter(|s| matches!(s.kind, StreamKind::Video | StreamKind::Audio)).map(|s| s.get("BitRate_Mode").to_string()).collect();
        if modes.iter().any(|m| m == "VBR") {
            Some("VBR".to_string())
        } else if !modes.is_empty() && modes.iter().all(|m| m == "CBR") {
            Some("CBR".to_string())
        } else {
            None
        }
    };
    let sizes_known: Option<u64> = {
        let v: Vec<Option<u64>> = doc.iter().filter(|s| s.kind != StreamKind::General).map(|s| s.get_u64("StreamSize")).collect();
        if !v.is_empty() && v.iter().all(|x| x.is_some()) { Some(v.iter().map(|x| x.unwrap()).sum()) } else { None }
    };

    // Elementary stream: the single stream's encoder info also describes the file.
    let elementary: Option<Vec<(String, String)>> = {
        let streams: Vec<&Stream> = doc.iter().filter(|s| s.kind != StreamKind::General).collect();
        if streams.len() == 1 && streams[0].get("Format") == doc.general_ref().get("Format") && streams[0].get("Format") != "FLAC" {
            Some(["Encoded_Library", "Encoded_Library_Settings", "Encoded_Library_Name", "Encoded_Library_Version", "Encoded_Library/String"].iter().map(|k| (k.to_string(), streams[0].get(k).to_string())).filter(|(_, v)| !v.is_empty()).collect())
        } else {
            None
        }
    };
    let g = doc.general();
    for (k, v) in lists {
        g.set_if_empty(&k, v);
    }
    if let Some(copy) = elementary {
        for (k, v) in copy {
            g.set_if_empty(&k, v);
        }
    }
    if let Some(v) = &first_video {
        if v.has("FrameRate") {
            g.set_if_empty("FrameRate", v.get("FrameRate").to_string());
            g.set_if_empty("FrameRate/String", v.get("FrameRate/String").to_string());
        }
        if v.has("FrameCount") {
            g.set_if_empty("FrameCount", v.get("FrameCount").to_string());
        }
    }
    if !g.has("Duration") {
        if let Some(max) = stream_durations.iter().cloned().fold(None, |m: Option<f64>, d| Some(m.map_or(d, |m| m.max(d)))) {
            g.set("Duration", format!("{}", max.round() as i64));
        }
    }
    if let Some(m) = single_mode {
        g.set_if_empty("OverallBitRate_Mode", m);
    }
    if !g.has("OverallBitRate") {
        if let (Some(fs), Some(d)) = (g.get_f64("FileSize"), g.get_f64("Duration")) {
            if d > 0.0 {
                g.set("OverallBitRate", format!("{}", (fs * 8.0 * 1000.0 / d).round() as u64));
            }
        }
    }
    if !g.has("StreamSize") {
        if let (Some(fs), Some(sum)) = (g.get_u64("FileSize"), sizes_known) {
            if fs >= sum {
                g.set_int("StreamSize", (fs - sum) as i128);
            }
        }
    }
    finish_common(g);
    let ext = g.get("FileExtension").to_ascii_lowercase();
    let exts = g.get("Format/Extensions").to_string();
    if !ext.is_empty() && !exts.is_empty() && !exts.split(' ').any(|e| e == ext) {
        g.set_extra("FileExtension_Invalid", exts, "", "Y NT");
    }
}

/// For NTSC-style rates (24000/1001, 30000/1001, 60000/1001 …) set `FrameRate_Num`/`FrameRate_Den`
/// and the `29.970 (30000/1001) FPS` string form.
pub fn set_frame_rate_fraction(s: &mut Stream, fps: f64) {
    for num in [24000u32, 30000, 60000, 48000, 120000] {
        let v = num as f64 / 1001.0;
        if (fps - v).abs() < 0.0005 {
            s.set("FrameRate_Num", num.to_string());
            s.set("FrameRate_Den", "1001");
            s.set("FrameRate/String", format!("{} ({num}/1001) FPS", format!("{v:.3}")));
            return;
        }
    }
}

/// Extension of a path, lower-cased.
pub fn extension_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(format_datetime(0), "1970-01-01 00:00:00");
        assert_eq!(format_datetime(1_789_175_572), "2026-09-12 01:12:52");
    }

    #[test]
    fn positions() {
        assert_eq!(channel_positions_string2("Front: L C R, Back: L R, LFE"), "3/0/2.1");
        assert_eq!(channel_positions_string2("Front: C"), "1/0/0");
        assert_eq!(channel_positions_string2("Front: L R"), "2/0/0");
    }

    #[test]
    fn derives_strings() {
        let mut doc = Doc::new();
        doc.general().set("Format", "Matroska");
        doc.general().set("Duration", "1021");
        let v = doc.add(StreamKind::Video);
        v.set("Format", "AVC");
        v.set("Width", "64");
        v.set("Height", "48");
        v.set("FrameRate", "25.000");
        v.set("Duration", "1000");
        v.set("Language", "eng");
        finish(&mut doc, &FileInfo { path: Some("/x/y/file.mkv".into()), size: 10285, modified: None });
        let g = doc.general_ref();
        assert_eq!(g.get("FileSize/String"), "10.0 KiB");
        assert_eq!(g.get("Format/Extensions"), "mkv mk3d mka mks");
        assert_eq!(g.get("FileName"), "file");
        assert_eq!(g.get("FileExtension"), "mkv");
        assert_eq!(g.get("OverallBitRate"), "80588");
        assert_eq!(g.get("OverallBitRate/String"), "80.6 kb/s");
        assert_eq!(g.get("VideoCount"), "1");
        assert_eq!(g.get("Video_Format_List"), "AVC");
        assert_eq!(g.get("FrameCount"), "25");
        assert_eq!(g.get("Duration/String4"), "00:00:01:00");
        let v = doc.stream(StreamKind::Video, 0).unwrap();
        assert_eq!(v.get("DisplayAspectRatio/String"), "4:3");
        assert_eq!(v.get("PixelAspectRatio"), "1.000");
        assert_eq!(v.get("Format/Info"), "Advanced Video Codec");
        assert_eq!(v.get("Language/String"), "English");
        assert_eq!(v.get("Language"), "en");
        assert_eq!(v.get("Width/String"), "64 pixels");
        assert_eq!(v.get("FrameCount"), "25");
        assert_eq!(v.get("StreamCount"), "1");
        assert_eq!(v.get("Count"), "377");
    }
}

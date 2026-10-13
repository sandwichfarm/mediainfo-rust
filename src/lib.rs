//! Cleanroom, pure-Rust reimplementation of MediaInfo: reads media files and reports their
//! container and stream properties with the same field names, ordering and string formats as
//! MediaInfoLib's `MediaInfo_*` API.
//!
//! ```no_run
//! let mut mi = mediainfo::MediaInfo::new();
//! if mi.open("movie.mkv") {
//!     println!("{}", mi.get(mediainfo::StreamKind::Video, 0, "Width", mediainfo::InfoKind::Text));
//!     println!("{}", mi.inform());
//! }
//! ```

// Stylistic lints that do not improve parser code written against binary formats.
#![allow(unknown_lints, clippy::collapsible_match, clippy::type_complexity, clippy::if_same_then_else, clippy::field_reassign_with_default, clippy::too_many_arguments, clippy::collapsible_if, clippy::collapsible_else_if, clippy::manual_range_contains, clippy::needless_range_loop, clippy::items_after_test_module, clippy::new_without_default)]

pub mod cli;
pub mod finish;
pub mod io;
pub mod model;
pub mod output;
pub mod parsers;

pub use model::{Doc, Field, InfoKind, Stream, StreamKind};

use finish::FileInfo;
use io::Reader;
use std::any::Any;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe, UnwindSafe};
use std::path::Path;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// An error encountered while opening or analysing media.
#[derive(Debug)]
#[non_exhaustive]
pub enum OpenError {
    /// Opening, seeking or reading the file failed. The original I/O error is retained.
    Io(std::io::Error),
    /// Format detection, parsing or metadata processing unexpectedly panicked.
    ParserPanic { message: String },
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "media I/O error: {error}"),
            Self::ParserPanic { message } => write!(f, "media analysis panicked: {message}"),
        }
    }
}

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::ParserPanic { .. } => None,
        }
    }
}

impl From<std::io::Error> for OpenError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl OpenError {
    fn from_panic(payload: Box<dyn Any + Send>) -> Self {
        let message = match payload.downcast::<String>() {
            Ok(message) => *message,
            Err(payload) => match payload.downcast::<&'static str>() {
                Ok(message) => (*message).to_string(),
                Err(payload) => {
                    // An opaque payload's destructor can itself panic. Do not let it escape
                    // the error boundary; ordinary string panic payloads are freed normally.
                    std::mem::forget(payload);
                    "non-string panic payload".to_string()
                }
            },
        };
        Self::ParserPanic { message }
    }
}

/// Report flavour selected with `Option("Output", ...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Output {
    #[default]
    Text,
    Xml,
    OldXml,
    Json,
}

/// Runtime options (`Option(...)`), the subset that affects reports.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// `Complete=1` / `--Full`: show every field, raw names included.
    pub complete: bool,
    pub output: Output,
    /// `Language=raw`: use field names instead of translated labels.
    pub raw_names: bool,
    /// `Inform=<template>`.
    pub template: Option<String>,
    /// `LegacyStreamDisplay`, `ParseSpeed` etc. are accepted and ignored; remembered for `Option` echoes.
    pub other: Vec<(String, String)>,
}

/// One analysis handle, mirroring the C API object.
#[derive(Debug, Default)]
pub struct MediaInfo {
    doc: Doc,
    opened: bool,
    pub options: Options,
}

impl MediaInfo {
    pub fn new() -> Self {
        Self { doc: Doc::new(), opened: false, options: Options::default() }
    }

    /// Analyse a file. Returns `true` when the format was recognised, or `false` for unrecognised
    /// input, I/O errors or caught parser panics. Use [`Self::try_open`] for error details.
    /// General file information is available for unrecognised input.
    pub fn open(&mut self, path: impl AsRef<Path>) -> bool {
        self.try_open(path).unwrap_or(false)
    }

    /// Analyse a file, retaining I/O errors and catching unwinding parser panics.
    ///
    /// `Ok(true)` means a parser recognised the format, not that all input was validated.
    /// `Ok(false)` means no parser accepted the input; general file information is still available
    /// and [`Self::is_open`] returns `true`. On `Err`, previous results are cleared and the handle
    /// is closed and reusable. Runtime options are preserved.
    ///
    /// The panic guard covers format detection, parsing and metadata processing. It cannot catch
    /// process aborts (including `panic = "abort"`); the application's panic hook still runs.
    ///
    /// ```no_run
    /// let mut mi = mediainfo::MediaInfo::new();
    /// match mi.try_open("movie.mkv") {
    ///     Ok(true) => println!("{}", mi.inform()),
    ///     Ok(false) => eprintln!("Unrecognised media format"),
    ///     Err(error) => eprintln!("Unable to analyse media: {error}"),
    /// }
    /// ```
    pub fn try_open(&mut self, path: impl AsRef<Path>) -> Result<bool, OpenError> {
        let path = path.as_ref();
        self.close();
        let reader = Reader::open(path)?;
        let meta = std::fs::metadata(path).ok();
        let info = FileInfo { path: Some(path.to_path_buf()), size: reader.len(), modified: meta.and_then(|m| m.modified().ok()) };
        let ext = finish::extension_of(path);
        self.run(reader, &ext, &info)
    }

    /// Analyse an in-memory buffer. `name` supplies the extension hint and `CompleteName`.
    /// Returns `false` for unrecognised input or caught parser panics; use [`Self::try_open_bytes`]
    /// for error details.
    pub fn open_bytes(&mut self, data: Vec<u8>, name: Option<&str>) -> bool {
        self.try_open_bytes(data, name).unwrap_or(false)
    }

    /// Analyse an in-memory buffer, catching unwinding parser panics.
    /// `name` supplies the extension hint and `CompleteName`.
    ///
    /// Recognition results, handle state and panic-guard limitations match [`Self::try_open`].
    pub fn try_open_bytes(&mut self, data: Vec<u8>, name: Option<&str>) -> Result<bool, OpenError> {
        self.close();
        let reader = Reader::from_bytes(data);
        let info = FileInfo { path: name.map(Into::into), size: reader.len(), modified: None };
        let ext = name.map(|n| finish::extension_of(Path::new(n))).unwrap_or_default();
        self.run(reader, &ext, &info)
    }

    fn run(&mut self, reader: Reader, ext: &str, info: &FileInfo) -> Result<bool, OpenError> {
        // The reader (including any I/O error) and document belong to this operation and are
        // discarded on panic. No partially updated handle or reader crosses the unwind boundary.
        self.run_guarded(AssertUnwindSafe(move || {
            let mut reader = reader;
            let (recognised, mut doc) = parsers::parse(&mut reader, ext);
            if let Some(error) = reader.take_error() {
                return Err(error.into());
            }
            finish::finish(&mut doc, info);
            Ok((recognised, doc))
        }))
    }

    /// Analyse in isolated state and publish results only after the entire operation succeeds.
    fn run_guarded(&mut self, analyse: impl FnOnce() -> Result<(bool, Doc), OpenError> + UnwindSafe) -> Result<bool, OpenError> {
        let result = catch_unwind(analyse).map_err(OpenError::from_panic).and_then(|result| result);
        match result {
            Ok((recognised, doc)) => {
                self.doc = doc;
                self.opened = true;
                Ok(recognised)
            }
            Err(error) => {
                self.close();
                Err(error)
            }
        }
    }

    pub fn close(&mut self) {
        self.doc = Doc::new();
        self.opened = false;
    }

    pub fn is_open(&self) -> bool {
        self.opened
    }

    /// The parsed document (all streams).
    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    pub fn streams(&self, kind: StreamKind) -> &[Stream] {
        &self.doc.streams[kind as usize]
    }

    /// `Count_Get`: number of streams of a kind, or number of fields of one stream.
    pub fn count_get(&self, kind: StreamKind, stream: Option<usize>) -> usize {
        match stream {
            None => self.doc.count(kind),
            Some(i) => self.doc.stream(kind, i).map(|s| s.count()).unwrap_or(0),
        }
    }

    /// `Get`: field by name.
    pub fn get(&self, kind: StreamKind, stream: usize, parameter: &str, info: InfoKind) -> String {
        let s = match self.doc.stream(kind, stream) {
            Some(s) => s,
            None => return String::new(),
        };
        match s.index_of(parameter) {
            Some(i) => self.get_i(kind, stream, i, info),
            None => String::new(),
        }
    }

    /// `GetI`: field by position.
    pub fn get_i(&self, kind: StreamKind, stream: usize, parameter: usize, info: InfoKind) -> String {
        let s = match self.doc.stream(kind, stream) {
            Some(s) => s,
            None => return String::new(),
        };
        let (name, text, measure, options) = match s.field(parameter) {
            Some(f) => f,
            None => return String::new(),
        };
        match info {
            InfoKind::Name => name.to_string(),
            InfoKind::Text => text.to_string(),
            InfoKind::Measure => measure.to_string(),
            InfoKind::Options => options.to_string(),
            InfoKind::NameText => model::labels::label(name).to_string(),
            InfoKind::MeasureText => measure.trim().to_string(),
            InfoKind::Info | InfoKind::HowTo => String::new(),
        }
    }

    /// `Option`: configure the handle or query library information. Returns the reply string.
    pub fn option(&mut self, name: &str, value: &str) -> String {
        let key = name.trim().to_ascii_lowercase();
        match key.as_str() {
            "info_version" => format!("MediaInfoLib - v{VERSION} (mediainfo-rust)"),
            "info_parameters" => output::info_parameters(),
            "info_parameters_csv" => output::info_parameters_csv(),
            "info_outputformats" => "Text                : Text text/plain\nXML                 : MediaInfo XML text/xml\nOLDXML              : MediaInfo XML (old) text/xml\nJSON                : MediaInfo JSON text/json\n".to_string(),
            "info_capacities" | "info_codecs" | "info_url" => String::new(),
            "complete" | "full" => {
                self.options.complete = matches!(value.trim(), "1" | "true" | "yes" | "Yes" | "TRUE");
                String::new()
            }
            "output" => {
                self.options.output = match value.trim().to_ascii_uppercase().as_str() {
                    "XML" => Output::Xml,
                    "OLDXML" => Output::OldXml,
                    "JSON" => Output::Json,
                    "" | "TEXT" | "TXT" => Output::Text,
                    other => {
                        self.options.other.push(("Output".into(), other.to_string()));
                        Output::Text
                    }
                };
                String::new()
            }
            "inform" => {
                let v = value.trim();
                let upper = v.to_ascii_uppercase();
                match upper.as_str() {
                    "" | "TEXT" => {
                        self.options.template = None;
                        self.options.output = Output::Text;
                    }
                    "XML" => {
                        self.options.template = None;
                        self.options.output = Output::Xml;
                    }
                    "OLDXML" => {
                        self.options.template = None;
                        self.options.output = Output::OldXml;
                    }
                    "JSON" => {
                        self.options.template = None;
                        self.options.output = Output::Json;
                    }
                    _ => self.options.template = Some(v.to_string()),
                }
                String::new()
            }
            "language" => {
                self.options.raw_names = value.trim().eq_ignore_ascii_case("raw");
                String::new()
            }
            "charset" | "setlocale_lc_ctype" | "filetestcontinuousfilenames" | "parsespeed" | "legacystreamdisplay" | "readbyhuman" | "internet" | "showfiles" | "info_version_only" | "input_compressed" | "file_isseekable" | "file_keepinfo" | "file_stopafterfilled" | "file_stopsubstreamafterfilled" | "cover_data" | "file_duplicate" | "file_parsespeed" | "file_checksideCarfiles" => {
                self.options.other.push((name.to_string(), value.to_string()));
                String::new()
            }
            _ => "Option not known".to_string(),
        }
    }

    /// `Inform`: the report in the configured output format.
    pub fn inform(&self) -> String {
        if let Some(t) = &self.options.template {
            return output::template::render(&self.doc, t);
        }
        match self.options.output {
            Output::Text => output::text::render(&self.doc, self.options.complete, self.options.raw_names),
            Output::Xml => output::xml::render(&self.doc, self.options.complete, false),
            Output::OldXml => output::xml::render(&self.doc, self.options.complete, true),
            Output::Json => output::json::render(&self.doc, self.options.complete),
        }
    }
}

/// Convenience: analyse one file with default options and return the text report.
pub fn inform_file(path: impl AsRef<Path>) -> String {
    let mut mi = MediaInfo::new();
    mi.open(path);
    mi.inform()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_panics_clear_results_preserve_options_and_allow_reuse() {
        let panics: &[(&str, fn() -> !)] = &[
            ("borrowed panic message", || panic!("borrowed panic message")),
            ("owned panic message", || std::panic::panic_any("owned panic message".to_string())),
        ];
        for &(message, panic) in panics {
            let mut mi = MediaInfo::new();
            mi.option("Output", "XML");
            mi.doc.add(StreamKind::Video).set("Format", "AVC");
            mi.opened = true;
            let error = mi.run_guarded(|| {
                let mut partial = Doc::new();
                partial.add(StreamKind::Audio).set("Format", "PCM");
                panic()
            }).unwrap_err();
            assert!(matches!(&error, OpenError::ParserPanic { message: m } if m == message));
            assert!(error.to_string().contains(message));
            assert!(std::error::Error::source(&error).is_none());
            assert!(!mi.is_open());
            assert_eq!(mi.count_get(StreamKind::Video, None), 0);
            assert_eq!(mi.count_get(StreamKind::Audio, None), 0);
            assert!(matches!(mi.options.output, Output::Xml));
            assert!(!mi.try_open_bytes(Vec::new(), None).unwrap());
            assert!(mi.is_open());
        }
    }

    #[test]
    fn opaque_panic_payload_cannot_panic_again_when_dropped() {
        struct PanicOnDrop;
        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                panic!("opaque payload destructor");
            }
        }
        let mut mi = MediaInfo::new();
        let error = mi.run_guarded(|| std::panic::panic_any(PanicOnDrop)).unwrap_err();
        assert!(matches!(error, OpenError::ParserPanic { message } if message == "non-string panic payload"));
        assert!(!mi.is_open());
    }
}

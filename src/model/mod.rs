//! Stream model: per-kind field tables in the reference field order plus dynamic fields.

pub mod labels;
pub mod schema;

use std::collections::HashMap;
use std::sync::OnceLock;

/// Kind of stream, numbered as in the `MediaInfo_*` C API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(usize)]
pub enum StreamKind {
    General = 0,
    Video = 1,
    Audio = 2,
    Text = 3,
    Other = 4,
    Image = 5,
    Menu = 6,
}

impl StreamKind {
    pub const ALL: [StreamKind; 7] = [Self::General, Self::Video, Self::Audio, Self::Text, Self::Other, Self::Image, Self::Menu];

    pub fn from_usize(v: usize) -> Option<Self> {
        Self::ALL.get(v).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Text => "Text",
            Self::Other => "Other",
            Self::Image => "Image",
            Self::Menu => "Menu",
        }
    }

    pub fn schema(self) -> &'static [(&'static str, &'static str, &'static str)] {
        match self {
            Self::General => schema::GENERAL,
            Self::Video => schema::VIDEO,
            Self::Audio => schema::AUDIO,
            Self::Text => schema::TEXT,
            Self::Other => schema::OTHER,
            Self::Image => schema::IMAGE,
            Self::Menu => schema::MENU,
        }
    }

    /// Position of a field name in this kind's schema.
    pub fn index_of(self, name: &str) -> Option<usize> {
        static MAPS: OnceLock<Vec<HashMap<&'static str, usize>>> = OnceLock::new();
        let maps = MAPS.get_or_init(|| StreamKind::ALL.iter().map(|k| k.schema().iter().enumerate().map(|(i, (n, _, _))| (*n, i)).collect()).collect());
        maps[self as usize].get(name).copied()
    }
}

/// What to fetch about a field, numbered as in the C API (`MediaInfo_info_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum InfoKind {
    Name = 0,
    Text = 1,
    Measure = 2,
    Options = 3,
    NameText = 4,
    MeasureText = 5,
    Info = 6,
    HowTo = 7,
}

impl InfoKind {
    pub fn from_usize(v: usize) -> Option<Self> {
        [Self::Name, Self::Text, Self::Measure, Self::Options, Self::NameText, Self::MeasureText, Self::Info, Self::HowTo].get(v).copied()
    }
}

/// A dynamic (non-schema) field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub text: String,
    pub measure: String,
    pub options: String,
}

/// Options string for a dynamic field shown in the text report.
pub const OPT_SHOWN: &str = "Y NT";
/// Options string for a dynamic field hidden from the text report but exported to XML/JSON.
pub const OPT_HIDDEN: &str = "N YTY";

/// One stream: values indexed by schema position plus dynamic fields.
#[derive(Debug, Clone)]
pub struct Stream {
    pub kind: StreamKind,
    values: Vec<Option<String>>,
    extra: Vec<Field>,
}

impl Stream {
    pub fn new(kind: StreamKind) -> Self {
        Self { kind, values: vec![None; kind.schema().len()], extra: Vec::new() }
    }

    /// Total number of fields (schema + dynamic).
    pub fn count(&self) -> usize {
        self.values.len() + self.extra.len()
    }

    pub fn schema_len(&self) -> usize {
        self.values.len()
    }

    /// Value of a field by name, `""` when unset.
    pub fn get(&self, name: &str) -> &str {
        match self.kind.index_of(name) {
            Some(i) => self.values[i].as_deref().unwrap_or(""),
            None => self.extra.iter().find(|f| f.name == name).map(|f| f.text.as_str()).unwrap_or(""),
        }
    }

    pub fn has(&self, name: &str) -> bool {
        !self.get(name).is_empty()
    }

    /// Set a field. Schema fields are stored at their position; unknown names become dynamic fields
    /// shown in the text report. Empty values clear the field.
    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        let value: String = value.into();
        match self.kind.index_of(name) {
            Some(i) => self.values[i] = if value.is_empty() { None } else { Some(value) },
            None => self.set_extra(name, value, "", OPT_SHOWN),
        }
    }

    /// Set a field only when it is currently empty.
    pub fn set_if_empty(&mut self, name: &str, value: impl Into<String>) {
        if !self.has(name) {
            self.set(name, value);
        }
    }

    /// Set (or replace) a dynamic field with explicit measure and options.
    pub fn set_extra(&mut self, name: &str, value: impl Into<String>, measure: &str, options: &str) {
        let text = value.into();
        if let Some(f) = self.extra.iter_mut().find(|f| f.name == name) {
            f.text = text;
            f.measure = measure.to_string();
            f.options = options.to_string();
        } else {
            self.extra.push(Field { name: name.to_string(), text, measure: measure.to_string(), options: options.to_string() });
        }
    }

    /// Append a dynamic field without de-duplicating (chapter lines share a name pattern).
    pub fn push_extra(&mut self, name: &str, value: impl Into<String>, options: &str) {
        self.extra.push(Field { name: name.to_string(), text: value.into(), measure: String::new(), options: options.to_string() });
    }

    pub fn clear(&mut self, name: &str) {
        match self.kind.index_of(name) {
            Some(i) => self.values[i] = None,
            None => self.extra.retain(|f| f.name != name),
        }
    }

    /// Field by index: (name, text, measure, options).
    pub fn field(&self, index: usize) -> Option<(&str, &str, &str, &str)> {
        if index < self.values.len() {
            let (n, m, o) = self.kind.schema()[index];
            Some((n, self.values[index].as_deref().unwrap_or(""), m, o))
        } else {
            self.extra.get(index - self.values.len()).map(|f| (f.name.as_str(), f.text.as_str(), f.measure.as_str(), f.options.as_str()))
        }
    }

    /// Index of a field by name, dynamic fields included.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.kind.index_of(name).or_else(|| self.extra.iter().position(|f| f.name == name).map(|p| p + self.values.len()))
    }

    /// Iterate all non-empty fields in order: (name, text, measure, options).
    pub fn fields(&self) -> impl Iterator<Item = (&str, &str, &str, &str)> {
        (0..self.count()).filter_map(move |i| self.field(i)).filter(|f| !f.1.is_empty())
    }

    pub fn extra(&self) -> &[Field] {
        &self.extra
    }

    // ---- typed helpers used by parsers and the finish pass

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        self.get(name).trim().parse().ok()
    }
    pub fn get_i64(&self, name: &str) -> Option<i64> {
        let s = self.get(name).trim();
        s.parse().ok().or_else(|| s.parse::<f64>().ok().map(|f| f as i64))
    }
    pub fn get_u64(&self, name: &str) -> Option<u64> {
        self.get_i64(name).filter(|v| *v >= 0).map(|v| v as u64)
    }
    pub fn set_int(&mut self, name: &str, value: impl Into<i128>) {
        self.set(name, value.into().to_string());
    }
    /// Float with fixed decimals (as the reference stores `FrameRate` with 3 decimals etc.).
    pub fn set_float(&mut self, name: &str, value: f64, decimals: usize) {
        if value.is_finite() {
            self.set(name, format!("{value:.decimals$}"));
        }
    }
    pub fn set_bool(&mut self, name: &str, value: bool) {
        self.set(name, if value { "Yes" } else { "No" });
    }
}

/// All streams of a file, filled by parsers and finished by the derivation pass.
#[derive(Debug, Clone, Default)]
pub struct Doc {
    pub streams: [Vec<Stream>; 7],
}

impl Doc {
    pub fn new() -> Self {
        let mut d = Self::default();
        d.streams[0].push(Stream::new(StreamKind::General));
        d
    }

    pub fn general(&mut self) -> &mut Stream {
        &mut self.streams[0][0]
    }

    pub fn general_ref(&self) -> &Stream {
        &self.streams[0][0]
    }

    /// Append a stream of a kind and return it.
    pub fn add(&mut self, kind: StreamKind) -> &mut Stream {
        let v = &mut self.streams[kind as usize];
        v.push(Stream::new(kind));
        v.last_mut().unwrap()
    }

    pub fn count(&self, kind: StreamKind) -> usize {
        self.streams[kind as usize].len()
    }

    pub fn stream(&self, kind: StreamKind, index: usize) -> Option<&Stream> {
        self.streams[kind as usize].get(index)
    }

    pub fn stream_mut(&mut self, kind: StreamKind, index: usize) -> Option<&mut Stream> {
        self.streams[kind as usize].get_mut(index)
    }

    pub fn last_mut(&mut self, kind: StreamKind) -> Option<&mut Stream> {
        self.streams[kind as usize].last_mut()
    }

    /// Set a field on a stream, adding the stream when it does not exist yet.
    pub fn fill(&mut self, kind: StreamKind, index: usize, name: &str, value: impl Into<String>) {
        while self.streams[kind as usize].len() <= index {
            self.add(kind);
        }
        self.streams[kind as usize][index].set(name, value);
    }

    /// Fill only when empty.
    pub fn fill_if_empty(&mut self, kind: StreamKind, index: usize, name: &str, value: impl Into<String>) {
        while self.streams[kind as usize].len() <= index {
            self.add(kind);
        }
        self.streams[kind as usize][index].set_if_empty(name, value);
    }

    /// Every stream in kind order.
    pub fn iter(&self) -> impl Iterator<Item = &Stream> {
        self.streams.iter().flatten()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Stream> {
        self.streams.iter_mut().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_positions_match_reference() {
        assert_eq!(StreamKind::General.index_of("FileSize"), Some(89));
        assert_eq!(StreamKind::Video.index_of("Width"), Some(142));
        assert_eq!(StreamKind::Audio.index_of("Channel(s)"), Some(124));
        assert_eq!(StreamKind::Menu.index_of("Chapters_Pos_Begin"), Some(92));
        assert_eq!(StreamKind::General.schema().len(), 331);
        assert_eq!(StreamKind::Video.schema().len(), 377);
    }

    #[test]
    fn set_get_and_dynamic_fields() {
        let mut s = Stream::new(StreamKind::Video);
        s.set("Width", "64");
        s.set("Custom", "x");
        assert_eq!(s.get("Width"), "64");
        assert_eq!(s.get("Custom"), "x");
        assert_eq!(s.count(), 378);
        assert_eq!(s.field(377).unwrap().0, "Custom");
        assert_eq!(s.index_of("Custom"), Some(377));
        s.clear("Width");
        assert!(!s.has("Width"));
        assert_eq!(s.fields().count(), 1);
    }
}

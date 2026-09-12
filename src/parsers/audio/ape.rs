//! Monkey's Audio (`MAC ` header, public file-format description) and the APEv2 tag reader shared
//! with WavPack and TTA (both usually carry an APEv2 tag at the end of the file).

use crate::io::{le16, le32, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

// ---------------------------------------------------------------------------- APEv2 tags

/// Largest APE tag we read.
const MAX_TAG: usize = 16 << 20;
/// Most items we read from a tag.
const MAX_ITEMS: usize = 1024;

/// An APEv2 (or v1) tag found at the end of a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApeTag {
    /// Total bytes occupied at the end of the file (items + footer + header, + ID3v1 if present).
    pub size: u64,
    /// (key, value) for text items, in file order.
    pub items: Vec<(String, String)>,
}

/// Parse the items of a tag given the bytes from the header (if any) up to and including the footer.
fn parse_items(d: &[u8], items_len: usize) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 8 < items_len && out.len() < MAX_ITEMS {
        let Some(len) = le32(d, pos) else { break };
        let Some(flags) = le32(d, pos + 4) else { break };
        pos += 8;
        let Some(key_end) = d.get(pos..items_len).and_then(|k| k.iter().position(|&b| b == 0)) else { break };
        let key = String::from_utf8_lossy(&d[pos..pos + key_end]).into_owned();
        pos += key_end + 1;
        let len = len as usize;
        let Some(value) = d.get(pos..pos + len) else { break };
        pos += len;
        // Item type in flags bits 1-2: 0 = UTF-8 text, 1 = binary, 2 = locator.
        if (flags >> 1) & 3 == 0 {
            out.push((key, crate::io::clean_text(&String::from_utf8_lossy(value))));
        } else {
            out.push((key, String::new()));
        }
    }
    out
}

/// Look for an APEv2 tag (footer, optional header) and an ID3v1 tag at the end of the file.
pub fn read_ape_tag(r: &mut Reader) -> Option<ApeTag> {
    let len = r.len();
    let mut end = len;
    // ID3v1 after the APE tag
    if len >= 128 && r.read_at(len - 128, 3) == b"TAG" {
        end -= 128;
    }
    if end < 32 {
        return if end < len { Some(ApeTag { size: len - end, items: Vec::new() }) } else { None };
    }
    let footer = r.read_vec_at(end - 32, 32);
    if &footer[..8] != b"APETAGEX" {
        return if end < len { Some(ApeTag { size: len - end, items: Vec::new() }) } else { None };
    }
    let _version = le32(&footer, 8)?;
    let tag_size = le32(&footer, 12)? as u64; // items + footer
    let count = le32(&footer, 16)?;
    let flags = le32(&footer, 20)?;
    if tag_size < 32 || tag_size as usize > MAX_TAG || tag_size > end {
        return None;
    }
    let has_header = flags & 0x8000_0000 != 0;
    let items_start = end - tag_size;
    let mut total = tag_size;
    if has_header && items_start >= 32 && r.read_at(items_start - 32, 8) == b"APETAGEX" {
        total += 32;
    }
    let items_len = (tag_size - 32) as usize;
    let data = r.read_vec_at(items_start, items_len);
    let mut items = parse_items(&data, items_len);
    items.truncate((count as usize).min(MAX_ITEMS));
    Some(ApeTag { size: total + (len - end), items })
}

/// Apply APE tag items to the General stream: known keys map to schema fields, the others are
/// shown as dynamic fields under their upper-cased key (as the reference does for `ENCODER`).
pub fn apply_ape_tag(g: &mut Stream, tag: &ApeTag) {
    for (k, v) in &tag.items {
        if v.is_empty() {
            continue;
        }
        let key = k.to_ascii_uppercase();
        let field = match key.as_str() {
            "TITLE" => "Title",
            "ARTIST" => "Performer",
            "ALBUM" => "Album",
            "ALBUM ARTIST" | "ALBUMARTIST" => "Album/Performer",
            "YEAR" => "Recorded_Date",
            "TRACK" => "Track/Position",
            "DISC" => "Part/Position",
            "GENRE" => "Genre",
            "COMMENT" => "Comment",
            "COMPOSER" => "Composer",
            "COPYRIGHT" => "Copyright",
            "PUBLISHER" => "Publisher",
            "LYRICS" => "Lyrics",
            "ISRC" => "ISRC",
            _ => "",
        };
        if field.is_empty() {
            if !g.has(&key) {
                g.set_extra(&key, v, "", "Y NT");
            }
        } else {
            g.set_if_empty(field, v);
            if field == "Title" {
                g.set_if_empty("Track", v);
            }
        }
    }
}

/// `Compression_Ratio` as the reference computes it for lossless formats: uncompressed rate / bit rate.
pub fn compression_ratio(sample_rate: u64, bit_depth: u64, channels: u64, bit_rate: f64) -> Option<String> {
    if bit_rate <= 0.0 {
        return None;
    }
    Some(format!("{:.3}", (sample_rate * bit_depth * channels) as f64 / bit_rate))
}

// ---------------------------------------------------------------------------- Monkey's Audio

/// Decoded APE header (both the >= 3.98 descriptor/header layout and the older one).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Header {
    /// Version × 1000 (3990 = 3.99).
    pub version: u16,
    pub compression_level: u16,
    pub format_flags: u16,
    pub blocks_per_frame: u32,
    pub final_frame_blocks: u32,
    pub total_frames: u32,
    pub bits_per_sample: u16,
    pub channels: u16,
    pub sample_rate: u32,
    /// Bytes before the first audio frame (descriptor + header + seek table + WAV header).
    pub header_bytes: u64,
}

impl Header {
    pub fn total_samples(&self) -> u64 {
        if self.total_frames == 0 {
            return 0;
        }
        (self.total_frames as u64 - 1) * self.blocks_per_frame as u64 + self.final_frame_blocks as u64
    }
}

fn level_name(level: u16) -> &'static str {
    match level {
        1000 => "Fast",
        2000 => "Normal",
        3000 => "High",
        4000 => "Extra High",
        5000 => "Insane",
        _ => "",
    }
}

/// Parse a `MAC ` header from the start of the file.
pub fn parse_header(d: &[u8]) -> Option<Header> {
    if !d.starts_with(b"MAC ") {
        return None;
    }
    let version = le16(d, 4)?;
    let mut h = Header { version, ..Default::default() };
    if version >= 3980 {
        let descriptor_bytes = le32(d, 8)?;
        let header_bytes = le32(d, 12)?;
        let seek_table_bytes = le32(d, 16)?;
        let wav_header_bytes = le32(d, 20)?;
        let o = descriptor_bytes as usize;
        if o < 52 || o > 4096 {
            return None;
        }
        h.compression_level = le16(d, o)?;
        h.format_flags = le16(d, o + 2)?;
        h.blocks_per_frame = le32(d, o + 4)?;
        h.final_frame_blocks = le32(d, o + 8)?;
        h.total_frames = le32(d, o + 12)?;
        h.bits_per_sample = le16(d, o + 16)?;
        h.channels = le16(d, o + 18)?;
        h.sample_rate = le32(d, o + 20)?;
        h.header_bytes = descriptor_bytes as u64 + header_bytes as u64 + seek_table_bytes as u64 + wav_header_bytes as u64;
    } else {
        h.compression_level = le16(d, 6)?;
        h.format_flags = le16(d, 8)?;
        h.channels = le16(d, 10)?;
        h.sample_rate = le32(d, 12)?;
        let wav_header_bytes = le32(d, 16)?;
        let _wav_tail_bytes = le32(d, 20)?;
        h.total_frames = le32(d, 24)?;
        h.final_frame_blocks = le32(d, 28)?;
        let mut pos = 32;
        if h.format_flags & 0x10 != 0 {
            pos += 4; // peak level
        }
        let seek_elements = if h.format_flags & 0x20 != 0 {
            pos += 4;
            le32(d, pos - 4)?
        } else {
            h.total_frames
        };
        h.bits_per_sample = if h.format_flags & 0x01 != 0 {
            8
        } else if h.format_flags & 0x08 != 0 {
            24
        } else {
            16
        };
        h.blocks_per_frame = if version >= 3950 {
            73728 * 4
        } else if version >= 3900 || (version >= 3800 && h.compression_level == 4000) {
            73728
        } else {
            9216
        };
        let wav_header = if h.format_flags & 0x04 != 0 { 0 } else { wav_header_bytes as u64 };
        h.header_bytes = pos as u64 + seek_elements as u64 * 4 + wav_header;
    }
    if h.channels == 0 || h.channels > 32 || h.sample_rate == 0 || !matches!(h.bits_per_sample, 8 | 16 | 24 | 32) {
        return None;
    }
    Some(h)
}

/// Fill an audio stream from the header.
pub fn apply_header(s: &mut Stream, h: &Header) {
    s.set_if_empty("Format", "Monkey's Audio");
    s.set("Format_Version", format!("{}.{:02}", h.version / 1000, (h.version % 1000) / 10));
    let level = level_name(h.compression_level);
    if !level.is_empty() {
        s.set("Format_Profile", level);
    }
    s.set("Channel(s)", h.channels.to_string());
    s.set("SamplingRate", h.sample_rate.to_string());
    s.set("BitDepth", h.bits_per_sample.to_string());
    let samples = h.total_samples();
    if samples > 0 {
        s.set("SamplingCount", samples.to_string());
        s.set("Duration", format!("{}", (samples as f64 * 1000.0 / h.sample_rate as f64).round() as u64));
    }
    s.set("BitRate_Mode", "VBR");
    s.set("Compression_Mode", "Lossless");
}

pub fn probe(p: &Probe) -> u8 {
    if p.starts_with(b"MAC ") {
        if parse_header(p.head).is_some() {
            100
        } else {
            50
        }
    } else if p.ext_in(&["ape", "mac"]) {
        20
    } else {
        0
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 4096);
    let Some(h) = parse_header(&head) else { return false };
    let mut s = Stream::new(StreamKind::Audio);
    apply_header(&mut s, &h);
    let tag = read_ape_tag(r);
    let tag_size = tag.as_ref().map(|t| t.size).unwrap_or(0);
    let g = doc.general();
    g.set("Format", "Monkey's Audio");
    if let Some(t) = &tag {
        apply_ape_tag(g, t);
    }
    let audio_size = r.len().saturating_sub(tag_size).saturating_sub(h.header_bytes.min(r.len()));
    s.set("StreamSize", audio_size.to_string());
    if let Some(ms) = s.get_f64("Duration") {
        if ms > 0.0 {
            let br = (audio_size as f64 * 8.0 * 1000.0 / ms).round();
            s.set("BitRate", format!("{}", br as u64));
            if let Some(cr) = compression_ratio(h.sample_rate as u64, h.bits_per_sample as u64, h.channels as u64, br) {
                s.set("Compression_Ratio", cr);
            }
            g.set("Duration", format!("{}", ms.round() as u64));
        }
    }
    g.set("StreamSize", (r.len() - audio_size).to_string());
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a >= 3.98 style file header (descriptor + header), no seek table / WAV header.
    pub(crate) fn new_header(version: u16, level: u16, bps: u16, channels: u16, rate: u32, frames: u32, final_blocks: u32) -> Vec<u8> {
        let mut v = b"MAC ".to_vec();
        v.extend_from_slice(&version.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        for x in [52u32, 24, 0, 0, 1000, 0, 0] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&[0u8; 16]);
        assert_eq!(v.len(), 52);
        v.extend_from_slice(&level.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&73728u32.to_le_bytes());
        v.extend_from_slice(&final_blocks.to_le_bytes());
        v.extend_from_slice(&frames.to_le_bytes());
        v.extend_from_slice(&bps.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v
    }

    /// Build an APEv2 tag (header + items + footer).
    pub(crate) fn ape_tag(items: &[(&str, &str)], with_header: bool) -> Vec<u8> {
        let mut body = Vec::new();
        for (k, v) in items {
            body.extend_from_slice(&(v.len() as u32).to_le_bytes());
            body.extend_from_slice(&0u32.to_le_bytes());
            body.extend_from_slice(k.as_bytes());
            body.push(0);
            body.extend_from_slice(v.as_bytes());
        }
        let size = (body.len() + 32) as u32;
        let block = |flags: u32| {
            let mut b = b"APETAGEX".to_vec();
            b.extend_from_slice(&2000u32.to_le_bytes());
            b.extend_from_slice(&size.to_le_bytes());
            b.extend_from_slice(&(items.len() as u32).to_le_bytes());
            b.extend_from_slice(&flags.to_le_bytes());
            b.extend_from_slice(&[0u8; 8]);
            b
        };
        let mut v = Vec::new();
        if with_header {
            v.extend_from_slice(&block(0xA000_0000));
        }
        v.extend_from_slice(&body);
        v.extend_from_slice(&block(if with_header { 0x8000_0000 } else { 0 }));
        v
    }

    #[test]
    fn ape_tags() {
        let mut d = vec![0u8; 100];
        d.extend_from_slice(&ape_tag(&[("encoder", "Lavf63.1.101"), ("Title", "T")], true));
        let tag_len = d.len() - 100;
        let mut r = Reader::from_bytes(d.clone());
        let t = read_ape_tag(&mut r).unwrap();
        assert_eq!(t.size as usize, tag_len);
        assert_eq!(t.items, vec![("encoder".to_string(), "Lavf63.1.101".to_string()), ("Title".to_string(), "T".to_string())]);
        let mut g = Stream::new(StreamKind::General);
        apply_ape_tag(&mut g, &t);
        assert_eq!(g.get("ENCODER"), "Lavf63.1.101");
        assert_eq!(g.get("Title"), "T");
        // footer only + trailing ID3v1
        let mut d = vec![0u8; 10];
        d.extend_from_slice(&ape_tag(&[("Artist", "A")], false));
        let footer_only = d.len() - 10;
        d.extend_from_slice(b"TAG");
        d.extend_from_slice(&[0u8; 125]);
        let t = read_ape_tag(&mut Reader::from_bytes(d)).unwrap();
        assert_eq!(t.size as usize, footer_only + 128);
        assert_eq!(t.items[0].0, "Artist");
        assert!(read_ape_tag(&mut Reader::from_bytes(vec![0u8; 50])).is_none());
        assert_eq!(compression_ratio(48000, 16, 1, 126088.0).as_deref(), Some("6.091"));
    }

    #[test]
    fn header_new_and_old() {
        let h = parse_header(&new_header(3990, 2000, 16, 2, 44100, 3, 1000)).unwrap();
        assert_eq!(h.total_samples(), 2 * 73728 + 1000);
        assert_eq!(h.header_bytes, 76);
        let mut s = Stream::new(StreamKind::Audio);
        apply_header(&mut s, &h);
        assert_eq!(s.get("Format"), "Monkey's Audio");
        assert_eq!(s.get("Format_Version"), "3.99");
        assert_eq!(s.get("Format_Profile"), "Normal");
        assert_eq!(s.get("Channel(s)"), "2");
        assert_eq!(s.get("SamplingRate"), "44100");
        assert_eq!(s.get("BitDepth"), "16");
        assert_eq!(s.get("Duration"), "3366");
        // Old layout (3.97): 24-bit flag, no peak/seek elements, 2 frames of 73728 blocks.
        let mut v = b"MAC ".to_vec();
        v.extend_from_slice(&3970u16.to_le_bytes());
        v.extend_from_slice(&4000u16.to_le_bytes());
        v.extend_from_slice(&0x08u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&48000u32.to_le_bytes());
        v.extend_from_slice(&44u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(&48000u32.to_le_bytes());
        let h = parse_header(&v).unwrap();
        assert_eq!(h.bits_per_sample, 24);
        assert_eq!(h.blocks_per_frame, 73728 * 4);
        assert_eq!(h.total_samples(), 73728 * 4 + 48000);
        assert_eq!(h.header_bytes, 32 + 2 * 4 + 44);
        let mut s = Stream::new(StreamKind::Audio);
        apply_header(&mut s, &h);
        assert_eq!(s.get("Format_Profile"), "Extra High");
        assert_eq!(s.get("Format_Version"), "3.97");
        assert!(parse_header(b"MAC \x00\x00").is_none());
        assert!(parse_header(&new_header(3990, 2000, 16, 0, 44100, 3, 1000)).is_none());
    }

    #[test]
    fn file() {
        let mut d = new_header(3990, 1000, 16, 1, 48000, 1, 48000);
        d.extend_from_slice(&[0x55u8; 6000]);
        d.extend_from_slice(&ape_tag(&[("encoder", "x")], true));
        assert_eq!(probe(&Probe { head: &d, ext: "ape", size: d.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"MAC \xff\xff", ext: "ape", size: 6 }), 50);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "ape", size: 6 }), 20);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(d), &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "Monkey's Audio");
        assert_eq!(g.get("Duration"), "1000");
        assert_eq!(g.get("ENCODER"), "x");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("StreamSize"), "6000");
        assert_eq!(a.get("BitRate"), "48000");
        assert_eq!(a.get("Format_Profile"), "Fast");
        assert_eq!(a.get("Compression_Ratio"), "16.000");
        assert!(!parse(&mut Reader::from_bytes(b"RIFF".to_vec()), &mut Doc::new()));
    }
}

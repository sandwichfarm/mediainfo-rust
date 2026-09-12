//! Sun/NeXT audio (`.snd` / AU): 24-byte big-endian header (magic, data offset, data size,
//! encoding, sample rate, channels) followed by an optional annotation and the sound data.

use crate::io::{be32, clean_text, cstr, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

const UNKNOWN_SIZE: u32 = 0xFFFF_FFFF;
const MAX_ANNOTATION: usize = 64 << 10;

pub fn probe(p: &Probe) -> u8 {
    if p.head.len() >= 24 && p.starts_with(b".snd") {
        let offset = be32(p.head, 4).unwrap_or(0);
        let encoding = be32(p.head, 12).unwrap_or(0);
        if offset >= 24 && (1..=27).contains(&encoding) {
            return 100;
        }
        return 60;
    }
    0
}

/// Header fields of an AU file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Header {
    pub data_offset: u32,
    pub data_size: u32,
    pub encoding: u32,
    pub sample_rate: u32,
    pub channels: u32,
}

pub fn parse_header(d: &[u8]) -> Option<Header> {
    if d.get(0..4)? != b".snd" {
        return None;
    }
    Some(Header { data_offset: be32(d, 4)?, data_size: be32(d, 8)?, encoding: be32(d, 12)?, sample_rate: be32(d, 16)?, channels: be32(d, 20)? })
}

/// (Format, CodecID text, bits per sample (0 = not applicable), float, show BitDepth).
fn encoding_info(e: u32) -> Option<(&'static str, &'static str, u32, bool, bool)> {
    Some(match e {
        1 => ("ADPCM", "8-bit u-law", 8, false, false),
        2 => ("PCM", "8-bit linear PCM", 8, false, true),
        3 => ("PCM", "16-bit linear PCM", 16, false, true),
        4 => ("PCM", "24-bit linear PCM", 24, false, true),
        5 => ("PCM", "32-bit linear PCM", 32, false, true),
        6 => ("PCM", "32-bit IEEE floating point", 32, true, true),
        7 => ("PCM", "64-bit IEEE floating point", 64, true, true),
        8 => ("", "Fragmented sample data", 0, false, false),
        10 => ("", "DSP program", 0, false, false),
        11 => ("PCM", "8-bit fixed point", 8, false, true),
        12 => ("PCM", "16-bit fixed point", 16, false, true),
        13 => ("PCM", "24-bit fixed point", 24, false, true),
        14 => ("PCM", "32-bit fixed point", 32, false, true),
        18 => ("PCM", "16-bit linear with emphasis", 16, false, true),
        19 => ("PCM", "16-bit linear compressed", 16, false, true),
        20 => ("PCM", "16-bit linear with emphasis and compression", 16, false, true),
        21 => ("", "Music kit DSP commands", 0, false, false),
        23 => ("ADPCM", "4-bit CCITT G.721 ADPCM", 4, false, false),
        24 => ("ADPCM", "CCITT G.722 ADPCM", 0, false, false),
        25 => ("ADPCM", "CCITT G.723 3-bit ADPCM", 3, false, false),
        26 => ("ADPCM", "CCITT G.723 5-bit ADPCM", 5, false, false),
        27 => ("ADPCM", "8-bit a-law", 8, false, false),
        _ => return None,
    })
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 24);
    let Some(h) = parse_header(&head) else { return false };
    if h.data_offset < 24 {
        return false;
    }
    let data_start = (h.data_offset as u64).min(r.len());
    let available = r.len().saturating_sub(data_start);
    let data_size = if h.data_size == UNKNOWN_SIZE { available } else { (h.data_size as u64).min(available) };
    // Annotation between the header and the data.
    let ann_len = (h.data_offset as usize).saturating_sub(24).min(MAX_ANNOTATION);
    let annotation = if ann_len > 0 { clean_text(&cstr(&r.read_vec_at(24, ann_len))) } else { String::new() };

    let g = doc.general();
    g.set("Format", "AU");
    if !annotation.is_empty() {
        g.set("Comment", annotation);
    }
    let mut s = Stream::new(StreamKind::Audio);
    match encoding_info(h.encoding) {
        Some((format, codec, bits, float, show_depth)) => {
            if !format.is_empty() {
                s.set("Format", format);
            }
            s.set("CodecID", codec);
            if float {
                s.set("Format_Profile", "Float");
            }
            if show_depth && bits > 0 {
                s.set("BitDepth", bits.to_string());
            }
            if bits > 0 && h.sample_rate > 0 && h.channels > 0 {
                let bps = h.sample_rate as u64 * h.channels as u64 * bits as u64;
                s.set("BitRate", bps.to_string());
                s.set("BitRate_Mode", "CBR");
                if data_size > 0 {
                    let ms = data_size as f64 * 8.0 / bps as f64 * 1000.0;
                    s.set("Duration", format!("{}", ms.round() as i64));
                }
            }
        }
        None => s.set("CodecID", h.encoding.to_string()),
    }
    if h.channels > 0 {
        s.set("Channel(s)", h.channels.to_string());
    }
    if h.sample_rate > 0 {
        s.set("SamplingRate", h.sample_rate.to_string());
    }
    if data_size > 0 {
        s.set("StreamSize", data_size.to_string());
    }
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn au(offset: u32, size: u32, encoding: u32, rate: u32, channels: u32, annotation: &[u8], data: usize) -> Vec<u8> {
        let mut v = b".snd".to_vec();
        for x in [offset, size, encoding, rate, channels] {
            v.extend_from_slice(&x.to_be_bytes());
        }
        v.extend_from_slice(annotation);
        v.resize(offset as usize, 0);
        v.extend(std::iter::repeat(0x55u8).take(data));
        v
    }

    #[test]
    fn header() {
        let f = au(32, 48000, 27, 48000, 1, b"note", 48000);
        assert_eq!(probe(&Probe { head: &f[..64], ext: "au", size: f.len() as u64 }), 100);
        assert_eq!(parse_header(&f), Some(Header { data_offset: 32, data_size: 48000, encoding: 27, sample_rate: 48000, channels: 1 }));
        assert_eq!(parse_header(b".snd\0\0"), None);
    }

    #[test]
    fn alaw_stream() {
        let f = au(32, 48000, 27, 48000, 1, b"note", 48000);
        let mut r = Reader::from_bytes(f);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "AU");
        assert_eq!(doc.general_ref().get("Comment"), "note");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("Format"), "ADPCM");
        assert_eq!(a.get("CodecID"), "8-bit a-law");
        assert_eq!(a.get("Duration"), "1000");
        assert_eq!(a.get("BitRate"), "384000");
        assert_eq!(a.get("BitRate_Mode"), "CBR");
        assert_eq!(a.get("Channel(s)"), "1");
        assert_eq!(a.get("SamplingRate"), "48000");
        assert_eq!(a.get("StreamSize"), "48000");
        assert!(!a.has("BitDepth"));
    }

    #[test]
    fn pcm16_unknown_size_uses_file_end() {
        let f = au(24, UNKNOWN_SIZE, 3, 8000, 2, b"", 3200);
        let mut r = Reader::from_bytes(f);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("Format"), "PCM");
        assert_eq!(a.get("BitDepth"), "16");
        assert_eq!(a.get("StreamSize"), "3200");
        assert_eq!(a.get("Duration"), "100");
    }

    #[test]
    fn malformed() {
        for data in [b".snd".to_vec(), au(8, 10, 3, 8000, 1, b"", 10), b".snd\0\0\0\x18\0\0\0\0\0\0\0\x03".to_vec()] {
            let mut r = Reader::from_bytes(data);
            let mut doc = Doc::new();
            assert!(!parse(&mut r, &mut doc));
        }
    }
}

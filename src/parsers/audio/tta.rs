//! True Audio (`TTA1` header, True Audio file format description).

use crate::io::{le16, le32, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::audio::ape;
use crate::parsers::Probe;

/// Decoded 22-byte TTA1 header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Header {
    /// 1 = TTA1, 2 = encrypted.
    pub format: u16,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub sample_rate: u32,
    /// Total samples per channel.
    pub data_length: u32,
    pub crc: u32,
}

/// Bytes of the header (magic through CRC).
pub const HEADER_LEN: u64 = 22;

/// Size of an ID3v2 tag at the start of the data, if any.
fn id3v2_size(head: &[u8]) -> usize {
    if head.len() >= 10 && &head[..3] == b"ID3" && head[6] | head[7] | head[8] | head[9] < 0x80 {
        let size = ((head[6] as usize) << 21) | ((head[7] as usize) << 14) | ((head[8] as usize) << 7) | head[9] as usize;
        10 + size + if head[5] & 0x10 != 0 { 10 } else { 0 }
    } else {
        0
    }
}

pub fn parse_header(d: &[u8]) -> Option<Header> {
    if !d.starts_with(b"TTA1") {
        return None;
    }
    let h = Header { format: le16(d, 4)?, channels: le16(d, 6)?, bits_per_sample: le16(d, 8)?, sample_rate: le32(d, 10)?, data_length: le32(d, 14)?, crc: le32(d, 18)? };
    if !matches!(h.format, 1 | 2) || h.channels == 0 || h.channels > 32 || h.sample_rate == 0 || h.bits_per_sample == 0 || h.bits_per_sample > 32 {
        return None;
    }
    Some(h)
}

/// Fill an audio stream from the header.
pub fn apply_header(s: &mut Stream, h: &Header) {
    s.set_if_empty("Format", "TTA");
    s.set("Channel(s)", h.channels.to_string());
    s.set("SamplingRate", h.sample_rate.to_string());
    s.set("BitDepth", h.bits_per_sample.to_string());
    s.set("BitRate_Mode", "VBR");
    if h.data_length > 0 {
        s.set("SamplingCount", h.data_length.to_string());
        s.set("Duration", format!("{}", (h.data_length as f64 * 1000.0 / h.sample_rate as f64).round() as u64));
    }
}

pub fn probe(p: &Probe) -> u8 {
    let skip = id3v2_size(p.head);
    let h = p.head.get(skip..).unwrap_or(&[]);
    if h.starts_with(b"TTA1") {
        if parse_header(h).is_some() {
            100
        } else {
            40
        }
    } else if p.ext == "tta" {
        20
    } else {
        0
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 16);
    let start = id3v2_size(&head) as u64;
    let hdr = r.read_vec_at(start, HEADER_LEN as usize);
    let Some(h) = parse_header(&hdr) else { return false };
    let mut s = Stream::new(StreamKind::Audio);
    apply_header(&mut s, &h);
    let tag = ape::read_ape_tag(r);
    let tag_size = tag.as_ref().map(|t| t.size).unwrap_or(0);
    let g = doc.general();
    g.set("Format", "TTA");
    if let Some(t) = &tag {
        ape::apply_ape_tag(g, t);
    }
    // The reference counts the TTA header with the audio stream; only the tags are container overhead.
    let audio_size = r.len().saturating_sub(tag_size).saturating_sub(start);
    s.set("StreamSize", audio_size.to_string());
    if let Some(ms) = s.get_f64("Duration") {
        if ms > 0.0 {
            let br = (audio_size as f64 * 8.0 * 1000.0 / ms).round();
            s.set("BitRate", format!("{}", br as u64));
            if let Some(cr) = ape::compression_ratio(h.sample_rate as u64, h.bits_per_sample as u64, h.channels as u64, br) {
                s.set("Compression_Ratio", cr);
            }
            g.set("Duration", format!("{}", ms.round() as u64));
        }
    }
    g.set("StreamSize", (tag_size + start).to_string());
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(channels: u16, bps: u16, rate: u32, samples: u32) -> Vec<u8> {
        let mut v = b"TTA1".to_vec();
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&bps.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&samples.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v
    }

    #[test]
    fn tta_header() {
        let h = parse_header(&header(2, 24, 96000, 96000)).unwrap();
        assert_eq!(h.channels, 2);
        let mut s = Stream::new(StreamKind::Audio);
        apply_header(&mut s, &h);
        assert_eq!(s.get("Format"), "TTA");
        assert_eq!(s.get("BitDepth"), "24");
        assert_eq!(s.get("SamplingRate"), "96000");
        assert_eq!(s.get("Duration"), "1000");
        assert!(parse_header(&header(0, 16, 48000, 1)).is_none());
        assert!(parse_header(&header(1, 16, 48000, 1)[..20]).is_none());
        assert!(parse_header(b"TTA2").is_none());
    }

    #[test]
    fn file() {
        let mut d = header(1, 16, 48000, 48000);
        d.extend_from_slice(&[0x33u8; 15739]);
        let audio = d.len();
        d.extend_from_slice(&ape::tests::ape_tag(&[("encoder", "Lavf63.1.101")], true));
        assert_eq!(probe(&Probe { head: &d, ext: "tta", size: d.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"TTA1\x00", ext: "tta", size: 5 }), 40);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "tta", size: 4 }), 20);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "wav", size: 4 }), 0);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(d.clone()), &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "TTA");
        assert_eq!(g.get("Duration"), "1000");
        assert_eq!(g.get("StreamSize"), (d.len() - audio).to_string());
        assert_eq!(g.get("ENCODER"), "Lavf63.1.101");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("StreamSize"), "15761");
        assert_eq!(a.get("BitRate"), "126088");
        assert_eq!(a.get("Compression_Ratio"), "6.091");
        assert!(!parse(&mut Reader::from_bytes(b"RIFF".to_vec()), &mut Doc::new()));
    }
}

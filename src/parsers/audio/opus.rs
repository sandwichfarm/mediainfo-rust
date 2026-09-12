//! Opus: `OpusHead` identification header and `OpusTags` (RFC 7845).

use crate::io::{le16, le32};
use crate::model::Stream;
use crate::parsers::audio::{layout_for_count, vorbis};

/// Decoded `OpusHead`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Head {
    pub version: u8,
    pub channels: u8,
    pub pre_skip: u16,
    pub input_sample_rate: u32,
    pub output_gain: i16,
    pub mapping_family: u8,
    pub stream_count: u8,
    pub coupled_count: u8,
}

/// Parse an `OpusHead` packet (magic required).
pub fn parse_head(d: &[u8]) -> Option<Head> {
    let b = d.strip_prefix(b"OpusHead")?;
    if b.len() < 11 {
        return None;
    }
    let mut h = Head { version: b[0], channels: b[1], pre_skip: le16(b, 2)?, input_sample_rate: le32(b, 4)?, output_gain: le16(b, 8)? as i16, mapping_family: b[10], ..Default::default() };
    if h.version >> 4 != 0 || h.channels == 0 {
        return None;
    }
    if h.mapping_family == 0 {
        h.stream_count = 1;
        h.coupled_count = if h.channels == 2 { 1 } else { 0 };
    } else {
        h.stream_count = *b.get(11)?;
        h.coupled_count = *b.get(12)?;
    }
    Some(h)
}

/// Fill a stream from `OpusHead`: Opus always decodes to 48 kHz, so that is the sampling rate.
pub fn apply_head(s: &mut Stream, d: &[u8]) -> bool {
    let Some(h) = parse_head(d) else { return false };
    s.set_if_empty("Format", "Opus");
    s.set("Channel(s)", h.channels.to_string());
    s.set("SamplingRate", "48000");
    if h.mapping_family <= 1 {
        let (pos, layout) = layout_for_count(h.channels as u32);
        if !pos.is_empty() {
            s.set("ChannelPositions", pos);
            s.set("ChannelLayout", layout);
        }
    }
    s.set_if_empty("Compression_Mode", "Lossy");
    true
}

/// `OpusTags`: a VorbisComment block after the magic; tags go to `general`, the vendor to the stream.
pub fn apply_tags(s: &mut Stream, general: &mut Stream, d: &[u8]) -> bool {
    match d.strip_prefix(b"OpusTags") {
        Some(body) => vorbis::apply_comments(s, general, body),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;

    fn head(channels: u8, family: u8) -> Vec<u8> {
        let mut v = b"OpusHead".to_vec();
        v.push(1);
        v.push(channels);
        v.extend_from_slice(&312u16.to_le_bytes());
        v.extend_from_slice(&44100u32.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.push(family);
        if family != 0 {
            v.push(channels);
            v.push(0);
            v.extend(0..channels);
        }
        v
    }

    #[test]
    fn opus_head() {
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_head(&mut s, &head(1, 0)));
        assert_eq!(s.get("Format"), "Opus");
        assert_eq!(s.get("Channel(s)"), "1");
        assert_eq!(s.get("SamplingRate"), "48000");
        assert_eq!(s.get("ChannelPositions"), "Front: C");
        assert_eq!(s.get("ChannelLayout"), "C");
        let h = parse_head(&head(6, 1)).unwrap();
        assert_eq!(h.pre_skip, 312);
        assert_eq!(h.input_sample_rate, 44100);
        assert_eq!(h.stream_count, 6);
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_head(&mut s, &head(6, 1)));
        assert_eq!(s.get("ChannelLayout"), "L R C LFE Lb Rb");
        assert!(!apply_head(&mut s, b"OpusHead\x01"));
        assert!(!apply_head(&mut s, &head(0, 0)));
        assert!(parse_head(&head(2, 1)[..19]).is_none());
    }

    #[test]
    fn opus_tags() {
        let mut d = b"OpusTags".to_vec();
        d.extend_from_slice(&vorbis::tests::comment_block("Lavf63.1.101", &["encoder=Lavc63.1.101 libopus", "ARTIST=Me"]));
        let mut s = Stream::new(StreamKind::Audio);
        let mut g = Stream::new(StreamKind::General);
        assert!(apply_tags(&mut s, &mut g, &d));
        assert_eq!(s.get("Encoded_Library"), "Lavf63.1.101");
        assert_eq!(g.get("Encoded_Application"), "Lavc63.1.101 libopus");
        assert_eq!(g.get("Performer"), "Me");
        assert!(!apply_tags(&mut s, &mut g, b"OpusHead"));
    }
}

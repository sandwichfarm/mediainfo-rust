//! FLAC (xiph.org FLAC format specification): STREAMINFO, VORBIS_COMMENT and PICTURE metadata
//! blocks, shared with containers, and the native `fLaC` file.

use crate::io::{be24, be32, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::audio::{layout_for_count, vorbis};
use crate::parsers::Probe;

/// Largest metadata block we read (spec maximum is 2^24 - 1).
const MAX_BLOCK: usize = 16 << 20;
/// Most metadata blocks we walk.
const MAX_BLOCKS: usize = 1024;

/// Decoded STREAMINFO.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamInfo {
    pub min_block_size: u16,
    pub max_block_size: u16,
    pub min_frame_size: u32,
    pub max_frame_size: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub bits_per_sample: u8,
    pub total_samples: u64,
    pub md5: [u8; 16],
}

/// Parse a bare 34-byte STREAMINFO body.
pub fn parse_streaminfo(d: &[u8]) -> Option<StreamInfo> {
    if d.len() < 34 {
        return None;
    }
    let mut si = StreamInfo {
        min_block_size: u16::from_be_bytes([d[0], d[1]]),
        max_block_size: u16::from_be_bytes([d[2], d[3]]),
        min_frame_size: be24(d, 4)?,
        max_frame_size: be24(d, 7)?,
        ..Default::default()
    };
    let w = u64::from_be_bytes([d[10], d[11], d[12], d[13], d[14], d[15], d[16], d[17]]);
    si.sample_rate = (w >> 44) as u32;
    si.channels = ((w >> 41) & 0x7) as u8 + 1;
    si.bits_per_sample = ((w >> 36) & 0x1F) as u8 + 1;
    si.total_samples = w & ((1u64 << 36) - 1);
    si.md5.copy_from_slice(&d[18..34]);
    if si.sample_rate == 0 || si.sample_rate > 655_350 {
        return None;
    }
    Some(si)
}

/// One metadata block: (type, is_last, body).
pub fn metadata_blocks(d: &[u8]) -> Vec<(u8, bool, &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 4 <= d.len() && out.len() < MAX_BLOCKS {
        let h = d[pos];
        let len = match be24(d, pos + 1) {
            Some(l) => l as usize,
            None => break,
        };
        if len > MAX_BLOCK {
            break;
        }
        let end = pos + 4 + len;
        let body = match d.get(pos + 4..end) {
            Some(b) => b,
            None => &d[pos + 4..],
        };
        let last = h & 0x80 != 0;
        out.push((h & 0x7F, last, body));
        if last || end > d.len() {
            break;
        }
        pos = end;
    }
    out
}

/// Fill a stream from STREAMINFO.
pub fn apply_streaminfo(s: &mut Stream, si: &StreamInfo) {
    s.set_if_empty("Format", "FLAC");
    s.set("SamplingRate", si.sample_rate.to_string());
    s.set("Channel(s)", si.channels.to_string());
    let (pos, layout) = layout_for_count(si.channels as u32);
    if !pos.is_empty() {
        s.set("ChannelPositions", pos);
        s.set("ChannelLayout", layout);
    }
    s.set("BitDepth", si.bits_per_sample.to_string());
    if si.total_samples > 0 {
        s.set("SamplingCount", si.total_samples.to_string());
        let ms = (si.total_samples as f64 * 1000.0 / si.sample_rate as f64).round() as u64;
        s.set("Duration", ms.to_string());
    }
    s.set_if_empty("BitRate_Mode", "VBR");
    s.set_if_empty("Compression_Mode", "Lossless");
}

/// Accepts a bare 34-byte STREAMINFO, a single STREAMINFO block with its 4-byte header, or
/// `fLaC` + metadata blocks (Matroska CodecPrivate); comments in the blocks go to the stream.
pub fn apply_streaminfo_block(s: &mut Stream, d: &[u8]) -> bool {
    if let Some(rest) = d.strip_prefix(b"fLaC") {
        let mut ok = false;
        for (t, _, body) in metadata_blocks(rest) {
            match t {
                0 => {
                    if let Some(si) = parse_streaminfo(body) {
                        apply_streaminfo(s, &si);
                        ok = true;
                    }
                }
                4 => {
                    vorbis::apply_comments_to_stream(s, body);
                }
                _ => {}
            }
        }
        return ok;
    }
    let body = if d.len() >= 38 && d[0] & 0x7F == 0 && be24(d, 1) == Some(34) { &d[4..] } else { d };
    match parse_streaminfo(body) {
        Some(si) => {
            apply_streaminfo(s, &si);
            true
        }
        None => false,
    }
}

/// Name of an ID3v2/FLAC picture type.
fn picture_type_name(t: u32) -> &'static str {
    match t {
        0 => "Other",
        1 => "32x32 pixels 'file icon' (PNG only)",
        2 => "Other file icon",
        3 => "Cover (front)",
        4 => "Cover (back)",
        5 => "Leaflet page",
        6 => "Media (e.g. label side of CD)",
        7 => "Lead artist/lead performer/soloist",
        8 => "Artist/performer",
        9 => "Conductor",
        10 => "Band/Orchestra",
        11 => "Composer",
        12 => "Lyricist/text writer",
        13 => "Recording Location",
        14 => "During recording",
        15 => "During performance",
        16 => "Movie/video screen capture",
        17 => "A bright coloured fish",
        18 => "Illustration",
        19 => "Band/artist logotype",
        20 => "Publisher/Studio logotype",
        _ => "",
    }
}

/// PICTURE block → General `Cover`, `Cover_Type`, `Cover_Mime`, `Cover_Description`.
pub fn apply_picture(g: &mut Stream, d: &[u8]) -> bool {
    let t = match be32(d, 0) {
        Some(t) => t,
        None => return false,
    };
    let mime_len = match be32(d, 4) {
        Some(l) => l as usize,
        None => return false,
    };
    if mime_len > 1024 {
        return false;
    }
    let Some(mime) = d.get(8..8 + mime_len) else { return false };
    let mime = String::from_utf8_lossy(mime).trim().to_string();
    let mut pos = 8 + mime_len;
    let desc_len = be32(d, pos).unwrap_or(0) as usize;
    pos += 4;
    let desc = if desc_len <= 64 * 1024 { d.get(pos..pos + desc_len).map(|b| String::from_utf8_lossy(b).trim().to_string()).unwrap_or_default() } else { String::new() };
    let mut append = |name: &str, v: &str| {
        if v.is_empty() {
            return;
        }
        let old = g.get(name).to_string();
        g.set(name, if old.is_empty() { v.to_string() } else { format!("{old} / {v}") });
    };
    append("Cover_Description", &desc);
    append("Cover_Type", picture_type_name(t));
    append("Cover_Mime", &mime);
    g.set("Cover", "Yes");
    true
}

// ---------------------------------------------------------------------------- native file

/// Size of an ID3v2 tag at the start of the data, if any.
fn id3v2_size(head: &[u8]) -> usize {
    if head.len() >= 10 && &head[..3] == b"ID3" && head[6] | head[7] | head[8] | head[9] < 0x80 {
        let size = ((head[6] as usize) << 21) | ((head[7] as usize) << 14) | ((head[8] as usize) << 7) | head[9] as usize;
        let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
        10 + size + footer
    } else {
        0
    }
}

pub fn probe(p: &Probe) -> u8 {
    let skip = id3v2_size(p.head);
    let h = p.head.get(skip..).unwrap_or(&[]);
    if h.starts_with(b"fLaC") {
        if h.get(4).is_some_and(|b| b & 0x7F == 0) && be24(h, 5) == Some(34) {
            100
        } else {
            80
        }
    } else if p.ext_in(&["flac", "fla"]) {
        20
    } else {
        0
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 16);
    let start = id3v2_size(&head) as u64;
    if r.read_at(start, 4) != b"fLaC" {
        return false;
    }
    // Walk the metadata blocks one at a time so a large PICTURE block is not held in memory.
    let mut pos = start + 4;
    let mut s = Stream::new(StreamKind::Audio);
    let mut info: Option<StreamInfo> = None;
    let mut comments: Option<vorbis::Comments> = None;
    let mut pictures: Vec<Vec<u8>> = Vec::new();
    for _ in 0..MAX_BLOCKS {
        let h = r.read_vec_at(pos, 4);
        if h.len() < 4 {
            break;
        }
        let t = h[0] & 0x7F;
        let last = h[0] & 0x80 != 0;
        let len = be24(&h, 1).unwrap_or(0) as usize;
        let body_pos = pos + 4;
        match t {
            0 => {
                let body = r.read_vec_at(body_pos, len.min(34));
                info = parse_streaminfo(&body);
            }
            4 => {
                let body = r.read_vec_at(body_pos, len.min(MAX_BLOCK));
                comments = vorbis::parse_comments(&body);
            }
            6 => {
                if pictures.len() < 16 {
                    // Type + mime + description only (the image data itself is not needed).
                    pictures.push(r.read_vec_at(body_pos, len.min(4096)));
                }
            }
            _ => {}
        }
        pos = body_pos + len as u64;
        if last {
            break;
        }
    }
    let Some(si) = info else { return false };
    apply_streaminfo(&mut s, &si);
    let g = doc.general();
    g.set("Format", "FLAC");
    if let Some(c) = &comments {
        vorbis::apply_parsed_comments(&mut s, Some(&mut *g), c);
    }
    for p in &pictures {
        apply_picture(g, p);
    }
    let audio_size = r.len().saturating_sub(pos);
    s.set("StreamSize", audio_size.to_string());
    if let Some(ms) = s.get_f64("Duration") {
        if ms > 0.0 {
            s.set("BitRate", format!("{}", (audio_size as f64 * 8.0 * 1000.0 / ms).round() as u64));
            g.set("Duration", format!("{}", ms.round() as u64));
        }
    }
    // The reference reports the metadata as a zero-sized General stream.
    g.set("StreamSize", "0");
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn streaminfo(rate: u32, channels: u8, bps: u8, total: u64) -> Vec<u8> {
        let mut v = vec![0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0];
        let w: u64 = ((rate as u64) << 44) | (((channels - 1) as u64) << 41) | (((bps - 1) as u64) << 36) | (total & ((1 << 36) - 1));
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&[0u8; 16]);
        v
    }

    fn block(t: u8, last: bool, body: &[u8]) -> Vec<u8> {
        let mut v = vec![t | if last { 0x80 } else { 0 }];
        v.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn streaminfo_forms() {
        let si = streaminfo(48000, 1, 16, 48000);
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_streaminfo_block(&mut s, &si));
        assert_eq!(s.get("SamplingRate"), "48000");
        assert_eq!(s.get("Channel(s)"), "1");
        assert_eq!(s.get("ChannelPositions"), "Front: C");
        assert_eq!(s.get("ChannelLayout"), "C");
        assert_eq!(s.get("BitDepth"), "16");
        assert_eq!(s.get("Duration"), "1000");
        assert_eq!(s.get("SamplingCount"), "48000");
        assert_eq!(s.get("Compression_Mode"), "Lossless");
        // With the block header
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_streaminfo_block(&mut s, &block(0, true, &si)));
        assert_eq!(s.get("BitDepth"), "16");
        // fLaC + STREAMINFO + VORBIS_COMMENT
        let mut d = b"fLaC".to_vec();
        d.extend_from_slice(&block(0, false, &si));
        d.extend_from_slice(&block(4, true, &vorbis::tests::comment_block("Lavf63.1.101", &["encoder=Lavc63.1.101 flac"])));
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_streaminfo_block(&mut s, &d));
        assert_eq!(s.get("Encoded_Library"), "Lavf63.1.101");
        assert_eq!(s.get("Encoded_Application"), "Lavc63.1.101 flac");
        assert!(!apply_streaminfo_block(&mut s, &si[..20]));
        assert!(!apply_streaminfo_block(&mut s, b"fLaC"));
        let si6 = parse_streaminfo(&streaminfo(96000, 6, 24, 1 << 35)).unwrap();
        assert_eq!(si6.channels, 6);
        assert_eq!(si6.bits_per_sample, 24);
        assert_eq!(si6.total_samples, 1 << 35);
    }

    #[test]
    fn picture() {
        let mut d = 3u32.to_be_bytes().to_vec();
        d.extend_from_slice(&10u32.to_be_bytes());
        d.extend_from_slice(b"image/jpeg");
        d.extend_from_slice(&5u32.to_be_bytes());
        d.extend_from_slice(b"front");
        d.extend_from_slice(&[0u8; 20]);
        let mut g = Stream::new(StreamKind::General);
        assert!(apply_picture(&mut g, &d));
        assert_eq!(g.get("Cover"), "Yes");
        assert_eq!(g.get("Cover_Mime"), "image/jpeg");
        assert_eq!(g.get("Cover_Type"), "Cover (front)");
        assert_eq!(g.get("Cover_Description"), "front");
        assert!(!apply_picture(&mut g, &[0, 0]));
    }

    #[test]
    fn native_file() {
        let mut d = b"fLaC".to_vec();
        d.extend_from_slice(&block(0, false, &streaminfo(48000, 2, 16, 24000)));
        d.extend_from_slice(&block(4, true, &vorbis::tests::comment_block("Lavf63.1.101", &["encoder=Lavf63.1.101", "TITLE=T"])));
        let header = d.len() as u64;
        d.extend_from_slice(&[0xFFu8; 1000]);
        let p = Probe { head: &d, ext: "flac", size: d.len() as u64 };
        assert_eq!(probe(&p), 100);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "flac", size: 4 }), 20);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "wav", size: 4 }), 0);
        let mut r = Reader::from_bytes(d.clone());
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "FLAC");
        assert_eq!(g.get("Duration"), "500");
        assert_eq!(g.get("Encoded_Application"), "Lavf63.1.101");
        assert_eq!(g.get("Title"), "T");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("StreamSize"), (d.len() as u64 - header).to_string());
        assert_eq!(a.get("BitRate"), "16000");
        assert_eq!(a.get("Encoded_Library"), "Lavf63.1.101");
        // ID3v2 prefix
        let mut with_id3 = b"ID3\x04\x00\x00\x00\x00\x00\x05".to_vec();
        with_id3.extend_from_slice(&[0u8; 5]);
        with_id3.extend_from_slice(&d);
        assert_eq!(probe(&Probe { head: &with_id3, ext: "flac", size: with_id3.len() as u64 }), 100);
        let mut r = Reader::from_bytes(with_id3);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert!(!parse(&mut Reader::from_bytes(b"fLaC\x80\x00\x00\x02\x00\x00".to_vec()), &mut Doc::new()));
    }
}

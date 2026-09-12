//! Apple Core Audio Format (`caff`): `desc`, `data`, `chan`, `info`, `kuki` and `pakt` chunks.

use crate::io::{be16, be32, be64, clean_text, cstr, Reader};
use crate::model::{Doc, Stream, StreamKind, OPT_SHOWN};
use crate::parsers::audio::{self, aac, alac};
use crate::parsers::Probe;

const MAX_CHUNKS: usize = 4096;
const MAX_META_CHUNK: usize = 1 << 20;

pub fn probe(p: &Probe) -> u8 {
    if p.head.len() >= 12 && p.starts_with(b"caff") {
        let version = be16(p.head, 4).unwrap_or(0);
        if version == 1 && p.at(8, b"desc") {
            return 100;
        }
        return 70;
    }
    0
}

/// `desc` chunk: the Audio Description.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Desc {
    pub sample_rate: f64,
    pub format_id: [u8; 4],
    pub format_flags: u32,
    pub bytes_per_packet: u32,
    pub frames_per_packet: u32,
    pub channels_per_frame: u32,
    pub bits_per_channel: u32,
}

impl Desc {
    pub fn is_float(&self) -> bool {
        self.format_flags & 1 != 0
    }
    pub fn is_little_endian(&self) -> bool {
        self.format_flags & 2 != 0
    }
}

pub fn parse_desc(d: &[u8]) -> Option<Desc> {
    if d.len() < 32 {
        return None;
    }
    let mut id = [0u8; 4];
    id.copy_from_slice(&d[8..12]);
    let desc = Desc {
        sample_rate: f64::from_bits(be64(d, 0)?),
        format_id: id,
        format_flags: be32(d, 12)?,
        bytes_per_packet: be32(d, 16)?,
        frames_per_packet: be32(d, 20)?,
        channels_per_frame: be32(d, 24)?,
        bits_per_channel: be32(d, 28)?,
    };
    if !desc.sample_rate.is_finite() || desc.sample_rate < 0.0 {
        return None;
    }
    Some(desc)
}

/// Format name for a CAF format id.
fn format_name(id: &[u8; 4]) -> &'static str {
    match id {
        b"lpcm" => "PCM",
        b"ima4" => "ADPCM",
        b"aac " | b"aach" | b"aacl" | b"aace" | b"aacf" | b"aacg" | b"aacp" => "AAC",
        b"alac" => "ALAC",
        b".mp3" => "MPEG Audio",
        b".mp2" => "MPEG Audio",
        b".mp1" => "MPEG Audio",
        b"MAC3" => "MACE 3",
        b"MAC6" => "MACE 6",
        b"ulaw" => "PCM",
        b"alaw" => "PCM",
        b"samr" => "AMR",
        b"opus" => "Opus",
        b"flac" => "FLAC",
        b"ac-3" => "AC-3",
        b"ec-3" => "E-AC-3",
        b"QDMC" | b"QDM2" => "QDesign",
        b"Qclp" => "Qualcomm PureVoice",
        b"ilbc" => "iLBC",
        b"dvi " => "ADPCM",
        _ => "",
    }
}

/// Locate the AudioSpecificConfig inside an ES_Descriptor-shaped magic cookie (tags 0x03 → 0x04 →
/// 0x05 with expandable lengths); returns the cookie itself when it is not wrapped.
pub fn audio_specific_config(cookie: &[u8]) -> &[u8] {
    fn descriptor(d: &[u8], want: u8) -> Option<(usize, usize)> {
        if *d.first()? != want {
            return None;
        }
        let mut len = 0usize;
        let mut i = 1;
        for _ in 0..4 {
            let b = *d.get(i)?;
            i += 1;
            len = (len << 7) | (b & 0x7F) as usize;
            if b & 0x80 == 0 {
                break;
            }
        }
        Some((i, (i + len).min(d.len())))
    }
    let mut d = cookie;
    if let Some((start, end)) = descriptor(d, 0x03) {
        let body = &d[start..end];
        // ES_ID (2), flags (1) [+ dependsOn (2)] [+ URL] [+ OCR (2)]
        let flags = body.get(2).copied().unwrap_or(0);
        let mut skip = 3;
        if flags & 0x80 != 0 {
            skip += 2;
        }
        if flags & 0x40 != 0 {
            skip += 1 + body.get(skip).copied().unwrap_or(0) as usize;
        }
        if flags & 0x20 != 0 {
            skip += 2;
        }
        d = body.get(skip..).unwrap_or(&[]);
    }
    if let Some((start, end)) = descriptor(d, 0x04) {
        d = d[start..end].get(13..).unwrap_or(&[]);
    }
    if let Some((start, end)) = descriptor(d, 0x05) {
        return &d[start..end];
    }
    cookie
}

fn apply_info_entry(g: &mut Stream, key: &str, value: &str) {
    let field = match key.to_ascii_lowercase().as_str() {
        "title" => "Title",
        "artist" => "Performer",
        "album" => "Album",
        "track number" => "Track/Position",
        "year" | "recorded date" => "Recorded_Date",
        "composer" => "Composer",
        "lyricist" => "Lyricist",
        "genre" => "Genre",
        "comments" => "Comment",
        "copyright" => "Copyright",
        "encoding application" => "Encoded_Application",
        "source encoder" => "Encoded_Library",
        "tempo" => "BPM",
        _ => {
            g.set_extra(key, value, "", OPT_SHOWN);
            return;
        }
    };
    g.set_if_empty(field, value);
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 8);
    if head.len() < 8 || &head[0..4] != b"caff" {
        return false;
    }
    let version = be16(&head, 4).unwrap_or(0);
    let mut desc: Option<Desc> = None;
    let mut data: Option<(u64, u64)> = None; // (audio position, audio bytes)
    let mut chan: Option<(u32, u32)> = None; // (layout tag, bitmap)
    let mut cookie: Vec<u8> = Vec::new();
    let mut pakt: Option<(u64, u64)> = None; // (packets, valid frames)
    let mut info: Vec<(String, String)> = Vec::new();
    r.seek(8);
    let mut n = 0;
    while r.pos() + 12 <= r.len() && n < MAX_CHUNKS {
        n += 1;
        let Some(id) = r.read_fourcc() else { break };
        let Some(size) = r.read_u64be() else { break };
        let pos = r.pos();
        let size_i = size as i64;
        let payload = if size_i < 0 { r.len().saturating_sub(pos) } else { size.min(r.len().saturating_sub(pos)) };
        let meta_len = payload.min(MAX_META_CHUNK as u64) as usize;
        match &id {
            b"desc" => desc = parse_desc(&r.read_vec_at(pos, 32)),
            b"data" => {
                if data.is_none() {
                    data = Some((pos + 4, payload.saturating_sub(4)));
                }
                if size_i < 0 {
                    break;
                }
            }
            b"chan" => {
                let d = r.read_vec_at(pos, 8);
                if let (Some(tag), Some(bitmap)) = (be32(&d, 0), be32(&d, 4)) {
                    chan = Some((tag, bitmap));
                }
            }
            b"kuki" => cookie = r.read_vec_at(pos, meta_len),
            b"pakt" => {
                let d = r.read_vec_at(pos, 24);
                if let (Some(p), Some(f)) = (be64(&d, 0), be64(&d, 8)) {
                    pakt = Some((p, f));
                }
            }
            b"info" => {
                let d = r.read_vec_at(pos, meta_len);
                let count = be32(&d, 0).unwrap_or(0) as usize;
                let mut off = 4usize;
                for _ in 0..count.min(256) {
                    if off >= d.len() {
                        break;
                    }
                    let key = cstr(&d[off..]);
                    off += key.len() + 1;
                    if off >= d.len() {
                        break;
                    }
                    let value = cstr(&d[off..]);
                    off += value.len() + 1;
                    let (key, value) = (clean_text(&key), clean_text(&value));
                    if !key.is_empty() && !value.is_empty() {
                        info.push((key, value));
                    }
                }
            }
            _ => {}
        }
        if size_i < 0 {
            break;
        }
        r.seek(pos.saturating_add(size));
    }
    let Some(desc) = desc else { return false };

    let g = doc.general();
    g.set("Format", "CAF");
    if version > 0 {
        g.set("Format_Version", format!("Version {version}"));
    }
    for (k, v) in &info {
        apply_info_entry(g, k, v);
    }

    let mut s = Stream::new(StreamKind::Audio);
    let id = desc.format_id;
    let format = format_name(&id);
    let codec_id: String = id.iter().map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { '?' }).collect();
    s.set("CodecID", codec_id.trim_end().to_string());
    if !format.is_empty() {
        s.set("Format", format);
    }
    match &id {
        b"alac" => {
            alac::apply_cookie(&mut s, &cookie);
        }
        b"aac " | b"aach" | b"aacl" | b"aace" | b"aacf" | b"aacg" | b"aacp" => {
            aac::apply_asc(&mut s, audio_specific_config(&cookie));
        }
        _ => {}
    }
    if &id == b"lpcm" && desc.is_float() {
        s.set_if_empty("Format_Profile", "Float");
    }
    if desc.sample_rate > 0.0 {
        s.set("SamplingRate", format!("{:.3}", desc.sample_rate));
    }
    if desc.channels_per_frame > 0 {
        s.set_if_empty("Channel(s)", desc.channels_per_frame.to_string());
    }
    if desc.bits_per_channel > 0 {
        s.set_if_empty("BitDepth", desc.bits_per_channel.to_string());
    }
    if let Some((tag, bitmap)) = chan {
        if tag == 0x2_0000 && bitmap != 0 {
            let (pos, layout) = audio::layout_from_mask(bitmap);
            if !pos.is_empty() {
                s.set_if_empty("ChannelPositions", pos);
                s.set_if_empty("ChannelLayout", layout);
            }
        }
    }
    let audio_bytes = data.map(|(_, b)| b).unwrap_or(0);
    // Duration: packet table when present, else the constant packet layout.
    let frames: Option<f64> = match pakt {
        Some((_, valid)) if valid > 0 => Some(valid as f64),
        _ if desc.bytes_per_packet > 0 && desc.frames_per_packet > 0 && audio_bytes > 0 => Some((audio_bytes / desc.bytes_per_packet as u64) as f64 * desc.frames_per_packet as f64),
        _ => None,
    };
    if let (Some(frames), true) = (frames, desc.sample_rate > 0.0) {
        s.set("Duration", format!("{}", (frames / desc.sample_rate * 1000.0).round() as i64));
    }
    if &id == b"lpcm" {
        if desc.sample_rate > 0.0 && desc.channels_per_frame > 0 && desc.bits_per_channel > 0 {
            s.set("BitRate", format!("{:.3}", desc.sample_rate * desc.channels_per_frame as f64 * desc.bits_per_channel as f64));
        }
    } else if desc.bytes_per_packet > 0 && desc.frames_per_packet > 0 && desc.sample_rate > 0.0 {
        let bps = desc.bytes_per_packet as f64 * 8.0 * desc.sample_rate / desc.frames_per_packet as f64;
        s.set_if_empty("BitRate", format!("{}", bps.round() as u64));
        s.set_if_empty("BitRate_Mode", "CBR");
    }
    if audio_bytes > 0 {
        s.set("StreamSize", audio_bytes.to_string());
    }
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &[u8; 4], payload: &[u8], size: Option<i64>) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&size.unwrap_or(payload.len() as i64).to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn desc_bytes(rate: f64, id: &[u8; 4], flags: u32, bpp: u32, fpp: u32, ch: u32, bits: u32) -> Vec<u8> {
        let mut v = rate.to_bits().to_be_bytes().to_vec();
        v.extend_from_slice(id);
        for x in [flags, bpp, fpp, ch, bits] {
            v.extend_from_slice(&x.to_be_bytes());
        }
        v
    }

    fn caf(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut v = b"caff\0\x01\0\0".to_vec();
        for c in chunks {
            v.extend_from_slice(c);
        }
        v
    }

    #[test]
    fn desc_parsing() {
        let d = parse_desc(&desc_bytes(48000.0, b"lpcm", 2, 2, 1, 1, 16)).unwrap();
        assert_eq!(d, Desc { sample_rate: 48000.0, format_id: *b"lpcm", format_flags: 2, bytes_per_packet: 2, frames_per_packet: 1, channels_per_frame: 1, bits_per_channel: 16 });
        assert!(d.is_little_endian() && !d.is_float());
        assert!(parse_desc(&[0; 31]).is_none());
        assert!(parse_desc(&desc_bytes(f64::NAN, b"lpcm", 0, 0, 0, 0, 0)).is_none());
    }

    #[test]
    fn lpcm_file() {
        let mut data = vec![0u8; 4];
        data.extend_from_slice(&[0u8; 96000]);
        let mut info = 1u32.to_be_bytes().to_vec();
        info.extend_from_slice(b"encoder\0Lavf63.1.101\0");
        let mut chan = 0x2_0000u32.to_be_bytes().to_vec();
        chan.extend_from_slice(&0x4u32.to_be_bytes());
        chan.extend_from_slice(&0u32.to_be_bytes());
        let file = caf(&[chunk(b"desc", &desc_bytes(48000.0, b"lpcm", 2, 2, 1, 1, 16), None), chunk(b"chan", &chan, None), chunk(b"info", &info, None), chunk(b"data", &data, Some(-1))]);
        assert_eq!(probe(&Probe { head: &file[..64], ext: "caf", size: file.len() as u64 }), 100);
        let mut r = Reader::from_bytes(file);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "CAF");
        assert_eq!(g.get("Format_Version"), "Version 1");
        assert_eq!(g.get("encoder"), "Lavf63.1.101");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("Format"), "PCM");
        assert_eq!(a.get("CodecID"), "lpcm");
        assert_eq!(a.get("SamplingRate"), "48000.000");
        assert_eq!(a.get("BitRate"), "768000.000");
        assert_eq!(a.get("Channel(s)"), "1");
        assert_eq!(a.get("BitDepth"), "16");
        assert_eq!(a.get("Duration"), "1000");
        assert_eq!(a.get("StreamSize"), "96000");
        assert_eq!(a.get("ChannelPositions"), "Front: C");
        assert_eq!(a.get("ChannelLayout"), "C");
    }

    #[test]
    fn packet_table_duration() {
        let mut pakt = 10u64.to_be_bytes().to_vec();
        pakt.extend_from_slice(&10240u64.to_be_bytes());
        pakt.extend_from_slice(&[0u8; 8]);
        let file = caf(&[chunk(b"desc", &desc_bytes(44100.0, b"aac ", 0, 0, 1024, 2, 0), None), chunk(b"pakt", &pakt, None), chunk(b"data", &[0u8; 104], None)]);
        let mut r = Reader::from_bytes(file);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("Format"), "AAC");
        assert_eq!(a.get("CodecID"), "aac");
        assert_eq!(a.get("Duration"), "232");
        assert_eq!(a.get("StreamSize"), "100");
        assert!(!a.has("BitDepth"));
    }

    #[test]
    fn asc_extraction() {
        // ES_Descriptor → DecoderConfigDescriptor → DecoderSpecificInfo (AAC-LC 44.1k stereo)
        let esds = [0x03, 0x19, 0x00, 0x01, 0x00, 0x04, 0x11, 0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x05, 0x02, 0x12, 0x10, 0x06, 0x01, 0x02];
        assert_eq!(audio_specific_config(&esds), &[0x12, 0x10]);
        assert_eq!(audio_specific_config(&[0x12, 0x10]), &[0x12, 0x10]);
        assert_eq!(audio_specific_config(&[0x03]), &[0x03]);
    }

    #[test]
    fn malformed() {
        for data in [b"caff".to_vec(), b"caff\0\x01\0\0desc\0\0\0\0\0\0\0\x08\0\0".to_vec(), caf(&[chunk(b"data", &[0; 4], Some(i64::MAX))])] {
            let mut r = Reader::from_bytes(data);
            let mut doc = Doc::new();
            assert!(!parse(&mut r, &mut doc));
        }
    }
}

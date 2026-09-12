//! Flash Video (Adobe FLV): header, audio/video tags, `onMetaData` (AMF0) script data.

use crate::io::{be16, be24, be32, be64, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::audio::{aac, mpeg_audio};
use crate::parsers::video::{avc, h263, hevc};
use crate::parsers::Probe;

const HEAD_SCAN: u64 = 8 * 1024 * 1024;
const TAIL_SCAN: u64 = 2 * 1024 * 1024;
const CODEC_BYTES: usize = 256 * 1024;
const MAX_TAGS: u64 = 4_000_000;

pub fn probe(p: &Probe) -> u8 {
    if p.head.len() >= 13 && p.starts_with(b"FLV") && p.head[3] == 1 && p.head[4] & 0xFA == 0 && be32(p.head, 5) == Some(9) {
        100
    } else if p.starts_with(b"FLV") && p.head.len() >= 9 {
        70
    } else {
        0
    }
}

// ---------------------------------------------------------------------------- AMF0

/// AMF0 value reduced to what the metadata needs.
#[derive(Debug, Clone, PartialEq)]
pub enum Amf {
    Number(f64),
    Bool(bool),
    Str(String),
    Object(Vec<(String, Amf)>),
    Array(Vec<Amf>),
    Date(f64),
    Null,
}

fn amf_string(b: &[u8], p: &mut usize) -> Option<String> {
    let len = be16(b, *p)? as usize;
    let s = b.get(*p + 2..*p + 2 + len)?;
    *p += 2 + len;
    Some(String::from_utf8_lossy(s).to_string())
}

/// Parse one AMF0 value at `p` (depth-limited).
pub fn amf_value(b: &[u8], p: &mut usize, depth: u32) -> Option<Amf> {
    if depth > 16 {
        return None;
    }
    let t = *b.get(*p)?;
    *p += 1;
    match t {
        0 => {
            let v = f64::from_bits(be64(b, *p)?);
            *p += 8;
            Some(Amf::Number(v))
        }
        1 => {
            let v = *b.get(*p)?;
            *p += 1;
            Some(Amf::Bool(v != 0))
        }
        2 => Some(Amf::Str(amf_string(b, p)?)),
        3 | 8 => {
            if t == 8 {
                *p += 4; // array length hint
            }
            let mut out = Vec::new();
            let mut n = 0;
            loop {
                n += 1;
                if n > 10_000 {
                    return None;
                }
                let key = amf_string(b, p)?;
                if key.is_empty() && b.get(*p) == Some(&9) {
                    *p += 1;
                    break;
                }
                let v = amf_value(b, p, depth + 1)?;
                out.push((key, v));
            }
            Some(Amf::Object(out))
        }
        5 | 6 => Some(Amf::Null),
        10 => {
            let n = be32(b, *p)? as usize;
            *p += 4;
            let mut out = Vec::new();
            for _ in 0..n.min(10_000) {
                out.push(amf_value(b, p, depth + 1)?);
            }
            Some(Amf::Array(out))
        }
        11 => {
            let v = f64::from_bits(be64(b, *p)?);
            *p += 10;
            Some(Amf::Date(v))
        }
        12 => {
            let len = be32(b, *p)? as usize;
            let s = b.get(*p + 4..*p + 4 + len)?;
            *p += 4 + len;
            Some(Amf::Str(String::from_utf8_lossy(s).to_string()))
        }
        _ => None,
    }
}

/// `onMetaData` script tag → key/value list.
pub fn parse_metadata(data: &[u8]) -> Vec<(String, Amf)> {
    let mut p = 0;
    let Some(Amf::Str(name)) = amf_value(data, &mut p, 0) else { return Vec::new() };
    if name != "onMetaData" {
        return Vec::new();
    }
    match amf_value(data, &mut p, 0) {
        Some(Amf::Object(v)) => v,
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------- scan

#[derive(Debug, Default)]
struct Track {
    present: bool,
    first_ts: Option<u32>,
    last_ts: Option<u32>,
    frames: u64,
    bytes: u64,
    tags: u64,
    codec: Option<u8>,
    flags: u8,
    config: Vec<u8>,
    data: Vec<u8>,
    timestamps: Vec<u32>,
}

#[derive(Debug, Default)]
struct Ctx {
    has_audio: bool,
    has_video: bool,
    video: Track,
    audio: Track,
    meta: Vec<(String, Amf)>,
    complete: bool,
}

fn handle_tag(kind: u8, ts: u32, body: &[u8], ctx: &mut Ctx, collect: bool) {
    match kind {
        18 => {
            if ctx.meta.is_empty() && collect {
                ctx.meta = parse_metadata(body);
            }
        }
        8 | 9 => {
            let t = if kind == 8 { &mut ctx.audio } else { &mut ctx.video };
            let Some(&h) = body.first() else { return };
            t.present = true;
            t.tags += 1;
            t.bytes += body.len() as u64;
            let codec = if kind == 8 { h >> 4 } else { h & 0x0F };
            if t.codec.is_none() {
                t.codec = Some(codec);
                t.flags = h;
            }
            let mut payload = &body[1..];
            let mut is_frame = true;
            if (kind == 9 && codec == 7) || (kind == 9 && codec == 12) || (kind == 8 && codec == 10) {
                let Some(&pkt) = payload.first() else { return };
                payload = if kind == 9 { payload.get(4..).unwrap_or(&[]) } else { &payload[1..] };
                if pkt == 0 {
                    if t.config.is_empty() && collect {
                        t.config = payload.to_vec();
                    }
                    is_frame = false;
                } else if pkt == 2 {
                    is_frame = false;
                }
            }
            if is_frame {
                t.frames += 1;
                if t.first_ts.is_none() {
                    t.first_ts = Some(ts);
                }
                t.last_ts = Some(ts);
                if t.timestamps.len() < 64 {
                    t.timestamps.push(ts);
                }
                if collect && t.data.len() < CODEC_BYTES {
                    let take = payload.len().min(CODEC_BYTES - t.data.len());
                    t.data.extend_from_slice(&payload[..take]);
                }
            } else if t.first_ts.is_none() {
                // configuration tag: timestamp still counts as the first packet
                t.first_ts = Some(ts);
            }
        }
        _ => {}
    }
}

/// Walk tags from `pos` (at a tag header) to `end`. Returns the position reached.
fn scan(r: &mut Reader, start: u64, end: u64, ctx: &mut Ctx, collect: bool) -> u64 {
    let mut pos = start;
    let mut n = 0u64;
    while pos + 11 <= end && n < MAX_TAGS {
        n += 1;
        let h = r.read_vec_at(pos, 11);
        if h.len() < 11 {
            break;
        }
        let kind = h[0] & 0x1F;
        let size = be24(&h, 1).unwrap_or(0) as usize;
        let ts = be24(&h, 4).unwrap_or(0) | (h[7] as u32) << 24;
        if !matches!(kind, 8 | 9 | 18) {
            break;
        }
        let want = if collect || kind == 18 { size.min(16 << 20) } else { size.min(16) };
        let body = r.read_vec_at(pos + 11, want);
        if body.len() < want {
            break;
        }
        handle_tag(kind, ts, &body, ctx, collect);
        pos += 11 + size as u64 + 4;
    }
    pos
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let h = r.read_vec_at(0, 13);
    if h.len() < 9 || &h[..3] != b"FLV" {
        return false;
    }
    let flags = h[4];
    let data_offset = be32(&h, 5).unwrap_or(9).max(9) as u64;
    let mut ctx = Ctx { has_audio: flags & 4 != 0, has_video: flags & 1 != 0, ..Default::default() };
    let len = r.len();
    let start = data_offset + 4;
    let head_end = len.min(start + HEAD_SCAN);
    let reached = scan(r, start, head_end, &mut ctx, true);
    ctx.complete = reached >= len || head_end >= len;
    if !ctx.complete {
        // Tail: walk backwards with the PreviousTagSize fields to find a tag boundary.
        let tail_start = len.saturating_sub(TAIL_SCAN).max(reached);
        if let Some(p) = resync_tail(r, tail_start, len) {
            scan(r, p, len, &mut ctx, false);
        }
    }
    emit(doc, &ctx, len);
    true
}

/// Find a tag boundary in the tail by walking back from the end using PreviousTagSize.
fn resync_tail(r: &mut Reader, tail_start: u64, len: u64) -> Option<u64> {
    let mut pos = len;
    let mut n = 0;
    while pos > tail_start + 4 && n < 100_000 {
        n += 1;
        let prev = r.read_u32be_at(pos - 4)? as u64;
        if prev < 11 || prev + 4 > pos {
            return None;
        }
        pos = pos - 4 - prev;
        let h = r.read_vec_at(pos, 11);
        if h.len() < 11 || !matches!(h[0] & 0x1F, 8 | 9 | 18) || be24(&h, 1)? as u64 + 11 != prev {
            return None;
        }
    }
    Some(pos)
}

trait ReadAt {
    fn read_u32be_at(&mut self, pos: u64) -> Option<u32>;
}

impl ReadAt for Reader {
    fn read_u32be_at(&mut self, pos: u64) -> Option<u32> {
        let b = self.read_vec_at(pos, 4);
        be32(&b, 0)
    }
}

// ---------------------------------------------------------------------------- emit

fn meta_num(meta: &[(String, Amf)], key: &str) -> Option<f64> {
    meta.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
        Amf::Number(n) => Some(*n),
        Amf::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Amf::Str(s) => s.trim().parse().ok(),
        _ => None,
    })
}

fn meta_str(meta: &[(String, Amf)], key: &str) -> Option<String> {
    meta.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
        Amf::Str(s) => Some(crate::io::clean_text(s)).filter(|s| !s.is_empty()),
        _ => None,
    })
}

fn video_format(codec: u8) -> &'static str {
    match codec {
        1 => "JPEG",
        2 => "Sorenson Spark",
        3 => "Screen video",
        4 | 5 => "VP6",
        6 => "Screen video 2",
        7 => "AVC",
        12 => "HEVC",
        _ => "",
    }
}

fn audio_format(codec: u8) -> &'static str {
    match codec {
        0 | 3 => "PCM",
        1 => "ADPCM",
        2 | 14 => "MPEG Audio",
        4 | 5 | 6 => "Nellymoser",
        7 | 8 => "PCM",
        10 => "AAC",
        11 => "Speex",
        _ => "",
    }
}

/// Constant intervals between consecutive timestamps → CFR.
fn constant_rate(ts: &[u32]) -> Option<bool> {
    if ts.len() < 3 {
        return None;
    }
    let deltas: Vec<i64> = ts.windows(2).map(|w| w[1] as i64 - w[0] as i64).collect();
    let first = deltas[0];
    if first <= 0 {
        return None;
    }
    Some(deltas.iter().all(|d| (d - first).abs() <= 1))
}

fn median_delta(ts: &[u32]) -> Option<f64> {
    let mut d: Vec<i64> = ts.windows(2).map(|w| w[1] as i64 - w[0] as i64).filter(|d| *d > 0).collect();
    if d.is_empty() {
        return None;
    }
    d.sort_unstable();
    Some(d[d.len() / 2] as f64)
}

fn emit(doc: &mut Doc, ctx: &Ctx, file_size: u64) {
    let g = doc.general();
    g.set("Format", "Flash Video");
    let meta = &ctx.meta;
    if let Some(e) = meta_str(meta, "encoder") {
        g.set("Encoded_Application", e);
    }
    if let Some(e) = meta_str(meta, "metadatacreator") {
        g.set_if_empty("Encoded_Library", e);
    }
    if let Some(d) = meta.iter().find(|(k, _)| k == "creationdate").and_then(|(_, v)| match v {
        Amf::Str(s) => Some(crate::io::clean_text(s)),
        Amf::Date(ms) => Some(format!("UTC {}", crate::finish::format_datetime((*ms / 1000.0) as i64))),
        _ => None,
    }) {
        if !d.is_empty() {
            g.set("Encoded_Date", d);
        }
    }
    for key in ["title", "artist", "author", "copyright", "comment", "description"] {
        if let Some(v) = meta_str(meta, key) {
            let field = match key {
                "title" => "Title",
                "artist" | "author" => "Performer",
                "copyright" => "Copyright",
                "comment" => "Comment",
                _ => "Description",
            };
            g.set_if_empty(field, v);
        }
    }
    let _ = file_size;

    // ---- video
    if ctx.video.present || (ctx.has_video && meta_num(meta, "videocodecid").is_some()) {
        let t = &ctx.video;
        let mut s = Stream::new(StreamKind::Video);
        let codec = t.codec.or_else(|| meta_num(meta, "videocodecid").map(|v| v as u8)).unwrap_or(0);
        let format = video_format(codec);
        if !format.is_empty() {
            s.set("Format", format);
        }
        s.set_int("CodecID", codec as i128);
        let mut tick: Option<f64> = None;
        match codec {
            7 => {
                avc::apply_avcc(&mut s, &t.config);
                if let Some((sps_list, _, len)) = avc::parse_avcc(&t.config) {
                    let nals = avc::nals_length_prefixed(&t.data, len);
                    avc::apply_sei_from_nals(&mut s, &nals);
                    if let Some(sps) = sps_list.first().and_then(|n| avc::parse_sps(n.get(1..).unwrap_or(&[]))) {
                        if let Some((num, scale, _)) = sps.vui.timing {
                            if num > 0 && scale > 0 {
                                tick = Some(num as f64 * 1000.0 / scale as f64);
                            }
                        }
                    }
                }
            }
            12 => {
                hevc::apply_hvcc(&mut s, &t.config);
                if let Some(len) = hevc::hvcc_length_size(&t.config) {
                    let nals = hevc::nals_length_prefixed(&t.data, len);
                    hevc::apply_sei_from_nals(&mut s, &nals);
                }
            }
            2 => {
                h263::apply_frame(&mut s, &t.data);
            }
            _ => {}
        }
        if let Some(w) = meta_num(meta, "width").filter(|w| *w > 0.0) {
            s.set_if_empty("Width", format!("{}", w as u64));
        }
        if let Some(h) = meta_num(meta, "height").filter(|h| *h > 0.0) {
            s.set_if_empty("Height", format!("{}", h as u64));
        }
        // Frame rate: metadata, else from the timestamps
        let stream_mode = s.get("FrameRate_Mode").to_string();
        let fps = meta_num(meta, "framerate").filter(|f| *f > 0.0).or_else(|| median_delta(&t.timestamps).map(|d| 1000.0 / d));
        if let Some(f) = fps {
            s.set("FrameRate", format!("{f:.3}"));
            match constant_rate(&t.timestamps) {
                Some(true) => s.set("FrameRate_Mode", "CFR"),
                Some(false) => s.set("FrameRate_Mode", "VFR"),
                None => {}
            }
            if !stream_mode.is_empty() && s.has("FrameRate_Mode") && stream_mode != s.get("FrameRate_Mode") {
                s.set("FrameRate_Mode_Original", stream_mode);
            }
        }
        let tick = tick.or_else(|| fps.map(|f| 1000.0 / f)).unwrap_or(0.0);
        if let Some(first) = t.first_ts {
            s.set_int("Delay", first as i128);
            s.set("Delay_Source", "Container");
            let dur = first as f64 + t.frames as f64 * tick;
            if dur > 0.0 && ctx.complete {
                s.set("Duration", format!("{}", dur.round() as i64));
            } else if let Some(last) = t.last_ts {
                let dur = (last as f64 - first as f64) + tick;
                if dur > 0.0 {
                    s.set("Duration", format!("{}", dur.round() as i64));
                }
            }
        }
        if let Some(br) = meta_num(meta, "videodatarate").filter(|b| *b > 0.0) {
            s.set_if_empty("BitRate", format!("{}", (br * 1000.0) as u64));
        }
        finish_size(&mut s, t, ctx.complete);
        doc.streams[StreamKind::Video as usize].push(s);
    }

    // ---- audio
    if ctx.audio.present || (ctx.has_audio && meta_num(meta, "audiocodecid").is_some()) {
        let t = &ctx.audio;
        let mut s = Stream::new(StreamKind::Audio);
        let codec = t.codec.or_else(|| meta_num(meta, "audiocodecid").map(|v| v as u8)).unwrap_or(0);
        let format = audio_format(codec);
        if !format.is_empty() {
            s.set("Format", format);
        }
        s.set_int("CodecID", codec as i128);
        let flags = t.flags;
        let rate = match (flags >> 2) & 3 {
            0 => 5512,
            1 => 11025,
            2 => 22050,
            _ => 44100,
        };
        let rate = if codec == 14 { 8000 } else { rate };
        let bits = if flags & 2 != 0 { 16 } else { 8 };
        let channels = if flags & 1 != 0 { 2 } else { 1 };
        let mut tick = 0.0;
        match codec {
            2 | 14 => {
                mpeg_audio::apply_frame(&mut s, &t.data);
                s.set("CodecID/Hint", "MP3");
            }
            10 => {
                if let Some(aot) = aac::apply_asc(&mut s, &t.config) {
                    s.set("CodecID", format!("{codec}-{aot}"));
                }
                s.set_if_empty("SamplesPerFrame", "1024");
            }
            7 => {
                s.set("Format_Settings_Law", "A-law");
            }
            8 => {
                s.set("Format_Settings_Law", "u-law");
            }
            _ => {}
        }
        // Container channel count wins; the codec's own value becomes *_Original.
        let stream_channels = s.get_u64("Channel(s)");
        if t.present {
            match stream_channels {
                Some(c) if c != channels => {
                    for (base, orig) in [("Channel(s)", "Channel(s)_Original"), ("ChannelPositions", "ChannelPositions_Original"), ("ChannelLayout", "ChannelLayout_Original")] {
                        let v = s.get(base).to_string();
                        if !v.is_empty() {
                            s.set(orig, v);
                            s.clear(base);
                        }
                    }
                    s.set_int("Channel(s)", channels);
                }
                Some(_) => {}
                None => s.set_int("Channel(s)", channels),
            }
            s.set_if_empty("SamplingRate", rate.to_string());
            if matches!(codec, 0 | 1 | 3 | 7 | 8) {
                s.set_if_empty("BitDepth", bits.to_string());
            }
        } else {
            if let Some(sr) = meta_num(meta, "audiosamplerate").filter(|v| *v > 0.0) {
                s.set_if_empty("SamplingRate", format!("{}", sr as u64));
            }
            if let Some(st) = meta_num(meta, "stereo") {
                s.set_if_empty("Channel(s)", if st != 0.0 { "2" } else { "1" });
            }
        }
        if matches!(format, "MPEG Audio" | "AAC" | "Nellymoser" | "Speex") {
            s.set_if_empty("Compression_Mode", "Lossy");
        }
        if let (Some(sr), Some(spf)) = (s.get_f64("SamplingRate"), s.get_f64("SamplesPerFrame")) {
            if sr > 0.0 {
                tick = spf / sr * 1000.0;
            }
        }
        if let (Some(first), Some(last)) = (t.first_ts, t.last_ts) {
            s.set_int("Delay", first as i128);
            s.set("Delay_Source", "Container");
            let dur = (last as f64 - first as f64) + tick;
            if dur > 0.0 {
                s.set("Duration", format!("{}", dur.round() as i64));
            }
        }
        if let Some(br) = meta_num(meta, "audiodatarate").filter(|b| *b > 0.0) {
            s.set_if_empty("BitRate", format!("{}", (br * 1000.0) as u64));
        }
        finish_size(&mut s, t, ctx.complete);
        doc.streams[StreamKind::Audio as usize].push(s);
    }
    crate::parsers::mpeg_ps::apply_video_delay(doc);
}

/// StreamSize from the bit rate and duration when a rate is known, else from the counted tag bytes.
fn finish_size(s: &mut Stream, t: &Track, complete: bool) {
    if let (Some(br), Some(d)) = (s.get_f64("BitRate"), s.get_f64("Duration")) {
        if br > 0.0 && d > 0.0 {
            s.set("StreamSize", format!("{}", (br * d / 8000.0).round() as i64));
            return;
        }
    }
    if complete && t.bytes > 0 {
        s.set_int("StreamSize", t.bytes as i128);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(kind: u8, ts: u32, body: &[u8]) -> Vec<u8> {
        let mut v = vec![kind, (body.len() >> 16) as u8, (body.len() >> 8) as u8, body.len() as u8, (ts >> 16) as u8, (ts >> 8) as u8, ts as u8, (ts >> 24) as u8, 0, 0, 0];
        v.extend_from_slice(body);
        v.extend_from_slice(&((body.len() + 11) as u32).to_be_bytes());
        v
    }

    fn amf_str(s: &str) -> Vec<u8> {
        let mut v = vec![2, (s.len() >> 8) as u8, s.len() as u8];
        v.extend_from_slice(s.as_bytes());
        v
    }

    fn key(s: &str) -> Vec<u8> {
        let mut v = vec![(s.len() >> 8) as u8, s.len() as u8];
        v.extend_from_slice(s.as_bytes());
        v
    }

    fn num(v: f64) -> Vec<u8> {
        let mut o = vec![0u8];
        o.extend_from_slice(&v.to_bits().to_be_bytes());
        o
    }

    fn metadata() -> Vec<u8> {
        let mut m = amf_str("onMetaData");
        m.extend_from_slice(&[8, 0, 0, 0, 5]);
        for (k, v) in [("duration", 1.025), ("width", 64.0), ("height", 48.0), ("videodatarate", 195.3125), ("framerate", 25.0), ("audiodatarate", 31.25)] {
            m.extend(key(k));
            m.extend(num(v));
        }
        m.extend(key("encoder"));
        m.extend(amf_str("Lavf63.1.101"));
        m.extend(key("stereo"));
        m.extend_from_slice(&[1, 0]);
        m.extend_from_slice(&[0, 0, 9]);
        m
    }

    fn build() -> Vec<u8> {
        let mut f = b"FLV\x01\x05\0\0\0\x09\0\0\0\0".to_vec();
        f.extend(tag(18, 0, &metadata()));
        // audio MP3 mono 44.1k 16 bit: 0x2E
        f.extend(tag(8, 0, &[0x2E, 0xFF, 0xFB, 0x50, 0xC4, 1, 2]));
        for i in 0..25u32 {
            let ft = if i == 0 { 0x12 } else { 0x22 };
            f.extend(tag(9, 25 + i * 40, &[ft, 0, 0, 0x84, 0]));
            f.extend(tag(8, 26 + i * 40, &[0x2E, 0xFF, 0xFB, 0x50, 0xC4, 1, 2]));
        }
        f
    }

    #[test]
    fn amf_parsing() {
        let meta = parse_metadata(&metadata());
        assert_eq!(meta_num(&meta, "duration"), Some(1.025));
        assert_eq!(meta_str(&meta, "encoder").as_deref(), Some("Lavf63.1.101"));
        assert_eq!(meta.iter().find(|(k, _)| k == "stereo").map(|(_, v)| v.clone()), Some(Amf::Bool(false)));
        assert!(parse_metadata(b"\x02\x00\x03abc").is_empty());
        let mut p = 0;
        assert_eq!(amf_value(&[10, 0, 0, 0, 2, 1, 1, 5], &mut p, 0), Some(Amf::Array(vec![Amf::Bool(true), Amf::Null])));
        let mut p = 0;
        assert_eq!(amf_value(&[3, 0, 1, b'a', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9], &mut p, 0), Some(Amf::Object(vec![("a".into(), Amf::Number(0.0))])));
        let mut p = 0;
        assert_eq!(amf_value(&[3, 0, 1], &mut p, 0), None);
    }

    #[test]
    fn parses_tags() {
        let f = build();
        assert_eq!(probe(&Probe { head: &f, ext: "flv", size: f.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"FLV\x02\x05\0\0\0\x09", ext: "flv", size: 9 }), 70);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "flv", size: 4 }), 0);
        let size = f.len() as u64;
        let mut r = Reader::from_bytes(f);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "Flash Video");
        assert_eq!(g.get("Encoded_Application"), "Lavf63.1.101");
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("Format"), "Sorenson Spark");
        assert_eq!(v.get("CodecID"), "2");
        assert_eq!(v.get("Width"), "64");
        assert_eq!(v.get("FrameRate"), "25.000");
        assert_eq!(v.get("FrameRate_Mode"), "CFR");
        assert_eq!(v.get("Delay"), "25");
        assert_eq!(v.get("Duration"), "1025");
        assert_eq!(v.get("BitRate"), "195312");
        assert_eq!(v.get("StreamSize"), "25024");
        let a = &doc.streams[StreamKind::Audio as usize][0];
        assert_eq!(a.get("Format"), "MPEG Audio");
        assert_eq!(a.get("CodecID"), "2");
        assert_eq!(a.get("CodecID/Hint"), "MP3");
        assert_eq!(a.get("Channel(s)"), "1");
        assert_eq!(a.get("SamplingRate"), "44100");
        assert_eq!(a.get("BitRate"), "31250");
        assert_eq!(a.get("Delay"), "0");
        assert_eq!(a.get("Video_Delay"), "-25");
        assert_eq!(a.get("Duration"), "986");
        let _ = size;
    }

    #[test]
    fn aac_config_and_channels() {
        let mut f = b"FLV\x01\x04\0\0\0\x09\0\0\0\0".to_vec();
        f.extend(tag(8, 0, &[0xAF, 0, 0x11, 0x88]));
        f.extend(tag(8, 0, &[0xAF, 1, 1, 2, 3]));
        f.extend(tag(8, 21, &[0xAF, 1, 1, 2, 3]));
        let mut r = Reader::from_bytes(f);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert!(doc.streams[StreamKind::Video as usize].is_empty());
        let a = &doc.streams[StreamKind::Audio as usize][0];
        assert_eq!(a.get("Format"), "AAC");
        assert_eq!(a.get("Channel(s)"), "2");
        assert_eq!(a.get("SamplesPerFrame"), "1024");
        assert_eq!(a.get("Compression_Mode"), "Lossy");
        // truncated tag body: no panic, still recognised
        let mut r = Reader::from_bytes(b"FLV\x01\x04\0\0\0\x09\0\0\0\0\x08\0\0\x10\0\0\0\0\0\0\0\xAF".to_vec());
        assert!(parse(&mut r, &mut Doc::new()));
    }

    #[test]
    fn timestamp_helpers() {
        assert_eq!(constant_rate(&[0, 40, 80, 120]), Some(true));
        assert_eq!(constant_rate(&[0, 40, 100, 120]), Some(false));
        assert_eq!(constant_rate(&[0, 40]), None);
        assert_eq!(median_delta(&[0, 40, 80, 120]), Some(40.0));
    }
}

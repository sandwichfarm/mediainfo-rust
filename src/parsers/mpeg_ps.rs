//! MPEG program stream (ISO/IEC 13818-1 / 11172-1): pack and system headers, PES packets and the
//! DVD-Video private stream 1 sub-streams (AC-3, DTS, LPCM, sub-pictures).
//!
//! The PES header parser is shared with the transport stream module.

use crate::io::{be16, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::audio::{aac, ac3, dts, mpeg_audio, pcm};
use crate::parsers::video::{avc, hevc, mpeg4v, mpegv, vc1};
use crate::parsers::Probe;

/// Bytes scanned from the start of the file.
const HEAD_SCAN: u64 = 8 * 1024 * 1024;
/// Bytes scanned at the end of a large file (last timestamps).
const TAIL_SCAN: u64 = 2 * 1024 * 1024;
/// Payload kept per stream for the codec helpers.
const CODEC_BYTES: usize = 256 * 1024;
const MAX_STREAMS: usize = 256;

// ---------------------------------------------------------------------------- PES header

/// Parsed PES packet header.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PesHeader {
    pub stream_id: u8,
    /// `PES_packet_length` (0 = unbounded, video in transport streams).
    pub packet_length: usize,
    pub pts: Option<u64>,
    pub dts: Option<u64>,
    /// Offset of the payload from the start of the packet (the `00 00 01` prefix).
    pub payload_offset: usize,
}

fn timestamp(b: &[u8]) -> Option<u64> {
    if b.len() < 5 {
        return None;
    }
    Some(((b[0] as u64 >> 1) & 7) << 30 | (b[1] as u64) << 22 | (b[2] as u64 >> 1) << 15 | (b[3] as u64) << 7 | (b[4] as u64 >> 1))
}

/// Stream ids whose packets carry no PES header fields (payload follows the length directly).
fn no_header(id: u8) -> bool {
    matches!(id, 0xBC | 0xBE | 0xBF | 0xF0 | 0xF1 | 0xF2 | 0xF8 | 0xFF)
}

/// Parse a PES packet header starting at the `00 00 01 <stream id>` prefix (MPEG-1 and MPEG-2 forms).
pub fn parse_pes(data: &[u8]) -> Option<PesHeader> {
    if data.len() < 6 || data[0] != 0 || data[1] != 0 || data[2] != 1 {
        return None;
    }
    let stream_id = data[3];
    let packet_length = be16(data, 4)? as usize;
    let mut h = PesHeader { stream_id, packet_length, ..Default::default() };
    if no_header(stream_id) {
        h.payload_offset = 6;
        return Some(h);
    }
    let b = &data[6..];
    if b.is_empty() {
        h.payload_offset = 6;
        return Some(h);
    }
    if b[0] >> 6 == 2 {
        // MPEG-2
        let flags = *b.get(1)?;
        let hlen = *b.get(2)? as usize;
        let mut p = 3;
        if flags & 0x80 != 0 {
            h.pts = timestamp(b.get(p..p + 5)?);
            p += 5;
        }
        if flags & 0x40 != 0 {
            h.dts = timestamp(b.get(p..p + 5)?);
        }
        h.payload_offset = 6 + 3 + hlen;
    } else {
        // MPEG-1
        let mut p = 0;
        while p < b.len() && b[p] == 0xFF && p < 16 {
            p += 1;
        }
        if b.get(p)? >> 6 == 1 {
            p += 2;
        }
        let v = *b.get(p)?;
        if v >> 4 == 2 {
            h.pts = timestamp(b.get(p..p + 5)?);
            p += 5;
        } else if v >> 4 == 3 {
            h.pts = timestamp(b.get(p..p + 5)?);
            h.dts = timestamp(b.get(p + 5..p + 10)?);
            p += 10;
        } else {
            p += 1;
        }
        h.payload_offset = 6 + p;
    }
    Some(h)
}

/// 90 kHz ticks → milliseconds, handling one 33-bit wrap relative to `first`.
pub fn ticks_to_ms(t: u64, first: u64) -> f64 {
    let mut t = t;
    if t + (1u64 << 32) < first {
        t += 1u64 << 33;
    }
    t as f64 / 90.0
}

// ---------------------------------------------------------------------------- pack header

/// Pack header at `data`: (MPEG version 1/2, mux rate in bps, header length).
fn parse_pack(data: &[u8]) -> Option<(u8, u64, usize)> {
    if data.len() < 12 || data[..4] != [0, 0, 1, 0xBA] {
        return None;
    }
    if data[4] >> 6 == 1 {
        if data.len() < 14 {
            return None;
        }
        let rate = ((data[10] as u64) << 14 | (data[11] as u64) << 6 | (data[12] as u64) >> 2) * 400;
        let stuffing = (data[13] & 7) as usize;
        Some((2, rate, 14 + stuffing))
    } else if data[4] >> 4 == 2 {
        let rate = ((data[9] as u64 & 0x7F) << 15 | (data[10] as u64) << 7 | (data[11] as u64) >> 1) * 400;
        Some((1, rate, 12))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------- probe

/// Position of the first pack start code within the head (after a leading zero run-in only).
fn first_pack(head: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 4 <= head.len() && i < 64 * 1024 {
        if head[i] == 0 && head[i + 1] == 0 && head[i + 2] == 1 && head[i + 3] == 0xBA {
            return Some(i);
        }
        if head[i] != 0 {
            return None;
        }
        i += 1;
    }
    None
}

pub fn probe(p: &Probe) -> u8 {
    let exts = ["mpg", "mpeg", "vob", "m2p", "evo", "vro", "pss", "mpe", "dat"];
    match first_pack(p.head) {
        Some(pos) => {
            if parse_pack(&p.head[pos..]).is_none() {
                return 0;
            }
            if pos == 0 && p.ext_in(&exts) {
                100
            } else if pos == 0 {
                90
            } else {
                60
            }
        }
        None => 0,
    }
}

// ---------------------------------------------------------------------------- scan

#[derive(Debug, Default, Clone)]
struct PsStream {
    id: u8,
    sub_id: Option<u8>,
    first_pts: Option<u64>,
    last_pts: Option<u64>,
    bytes: u64,
    packets: u64,
    data: Vec<u8>,
    lpcm: Option<[u8; 3]>,
    first_order: usize,
}

#[derive(Debug, Default)]
struct Ctx {
    version: u8,
    mux_rate: u64,
    system_ids: Vec<u8>,
    streams: Vec<PsStream>,
    has_nav: bool,
    complete: bool,
}

impl Ctx {
    fn stream(&mut self, id: u8, sub: Option<u8>) -> Option<&mut PsStream> {
        if let Some(i) = self.streams.iter().position(|s| s.id == id && s.sub_id == sub) {
            return self.streams.get_mut(i);
        }
        if self.streams.len() >= MAX_STREAMS {
            return None;
        }
        let order = self.streams.len();
        self.streams.push(PsStream { id, sub_id: sub, first_order: order, ..Default::default() });
        self.streams.last_mut()
    }
}

fn sub_stream_kind(sub: u8) -> Option<u8> {
    match sub {
        0x20..=0x3F | 0x80..=0x8F | 0xA0..=0xA7 => Some(sub),
        _ => None,
    }
}

/// Handle one PES packet: `data` is the whole packet (prefix included).
fn handle_pes(data: &[u8], ctx: &mut Ctx, collect: bool) {
    let Some(h) = parse_pes(data) else { return };
    let id = h.stream_id;
    match id {
        0xBB => {
            // system header: rate_bound(3) audio_bound(1) flags(1) video_bound(1) reserved(1), then 3-byte stream entries
            let mut p = 6 + 6;
            while p + 3 <= data.len() {
                let sid = data[p];
                if sid & 0x80 == 0 {
                    break;
                }
                if !ctx.system_ids.contains(&sid) && ctx.system_ids.len() < MAX_STREAMS {
                    ctx.system_ids.push(sid);
                }
                p += 3;
            }
            return;
        }
        0xBF => {
            ctx.has_nav = true;
            return;
        }
        0xBE | 0xBC | 0xF0..=0xF2 | 0xF8 | 0xFF => return,
        _ => {}
    }
    let payload = data.get(h.payload_offset..).unwrap_or(&[]);
    let (sub, payload) = if id == 0xBD {
        let Some(&sub) = payload.first() else { return };
        let Some(sub) = sub_stream_kind(sub) else { return };
        // sub_stream_id, number of frames, first access unit pointer
        let skip = if (0x20..=0x3F).contains(&sub) { 1 } else { 4 };
        (Some(sub), payload.get(skip..).unwrap_or(&[]))
    } else if matches!(id, 0xC0..=0xDF | 0xE0..=0xEF) {
        (None, payload)
    } else {
        return;
    };
    let Some(s) = ctx.stream(id, sub) else { return };
    let mut payload = payload;
    if let Some(sub) = sub {
        if (0xA0..=0xA7).contains(&sub) && payload.len() >= 3 {
            if s.lpcm.is_none() {
                s.lpcm = Some([payload[0], payload[1], payload[2]]);
            }
            payload = &payload[3..];
        }
    }
    s.packets += 1;
    s.bytes += payload.len() as u64;
    if let Some(pts) = h.pts {
        if s.first_pts.is_none() {
            s.first_pts = Some(pts);
        }
        s.last_pts = Some(s.last_pts.map_or(pts, |l| l.max(pts)));
    }
    if collect && s.data.len() < CODEC_BYTES {
        let take = payload.len().min(CODEC_BYTES - s.data.len());
        s.data.extend_from_slice(&payload[..take]);
    }
}

/// Walk packs/PES packets from `start` to `end`; `collect` keeps payload for codec probing.
/// Returns the position reached.
fn scan(r: &mut Reader, start: u64, end: u64, ctx: &mut Ctx, collect: bool) -> u64 {
    let mut pos = start;
    let mut iterations = 0u32;
    while pos + 4 <= end {
        iterations += 1;
        if iterations > 4_000_000 {
            break;
        }
        let head = r.read_vec_at(pos, 32);
        if head.len() < 4 {
            break;
        }
        if head[0] != 0 || head[1] != 0 || head[2] != 1 || head[3] < 0xB9 {
            // resync on the next start code
            let chunk = r.read_vec_at(pos + 1, 64 * 1024);
            match chunk.windows(4).position(|w| w[0] == 0 && w[1] == 0 && w[2] == 1 && w[3] >= 0xB9) {
                Some(off) => pos += 1 + off as u64,
                None => pos += 1 + chunk.len() as u64,
            }
            continue;
        }
        let sid = head[3];
        match sid {
            0xBA => match parse_pack(&head) {
                Some((v, rate, len)) => {
                    if ctx.version == 0 {
                        ctx.version = v;
                        ctx.mux_rate = rate;
                    }
                    pos += len as u64;
                }
                None => pos += 4,
            },
            0xB9 => {
                pos += 4;
                break;
            }
            _ => {
                let Some(len) = be16(&head, 4) else { break };
                let total = 6 + len as usize;
                let data = r.read_vec_at(pos, total);
                handle_pes(&data, ctx, collect);
                pos += total as u64;
            }
        }
    }
    pos
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 64 * 1024);
    let Some(start) = first_pack(&head) else { return false };
    let mut ctx = Ctx::default();
    let len = r.len();
    let head_end = len.min(start as u64 + HEAD_SCAN);
    let reached = scan(r, start as u64, head_end, &mut ctx, true);
    ctx.complete = reached >= len || head_end >= len;
    if ctx.version == 0 {
        return false;
    }
    if !ctx.complete {
        // Tail: resync on a pack start code and read the last timestamps.
        let tail_start = len.saturating_sub(TAIL_SCAN).max(reached);
        let chunk = r.read_vec_at(tail_start, (len - tail_start).min(TAIL_SCAN) as usize);
        if let Some(off) = chunk.windows(4).position(|w| w[0] == 0 && w[1] == 0 && w[2] == 1 && w[3] == 0xBA) {
            scan(r, tail_start + off as u64, len, &mut ctx, false);
        }
    }
    emit(doc, &ctx);
    true
}

// ---------------------------------------------------------------------------- emit

fn kind_of(s: &PsStream) -> StreamKind {
    match (s.id, s.sub_id) {
        (0xE0..=0xEF, _) => StreamKind::Video,
        (0xC0..=0xDF, _) => StreamKind::Audio,
        (0xBD, Some(0x20..=0x3F)) => StreamKind::Text,
        (0xBD, Some(_)) => StreamKind::Audio,
        _ => StreamKind::General,
    }
}

/// `"224"` / `"224 (0xE0)"` style identifiers.
fn id_strings(id: u8, sub: Option<u8>) -> (String, String) {
    match sub {
        Some(s) => (format!("{id}-{s}"), format!("{id} (0x{id:02X})-{s} (0x{s:02X})")),
        None => (id.to_string(), format!("{id} (0x{id:02X})")),
    }
}

/// Video format detection from the first payload bytes (start codes / NAL types).
pub fn video_format(data: &[u8]) -> &'static str {
    let mut i = 0;
    let mut seen_mpeg = false;
    while i + 4 <= data.len() && i < 4096 {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            let c = data[i + 3];
            let four = i > 0 && data[i - 1] == 0;
            match c {
                0xB3 | 0xB8 | 0x00 => seen_mpeg = true,
                0xB5 if seen_mpeg => {}
                0xB0 | 0xB5 | 0xB6 => return "MPEG-4 Visual",
                0x0F | 0x0E | 0x0D => return "VC-1",
                0x40 | 0x42 | 0x44 | 0x26 | 0x28 if four => return "HEVC",
                0x67 | 0x68 | 0x09 | 0x65 | 0x61 | 0x41 if four => return "AVC",
                _ => {}
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    if seen_mpeg {
        return "MPEG Video";
    }
    let nals = avc::nals_annexb(&data[..data.len().min(4096)]);
    if nals.iter().any(|(t, _)| *t == 7) {
        return "AVC";
    }
    "MPEG Video"
}

/// Frame period in ms used to extend the last timestamp to the end of the last frame.
/// AVC uses the VUI tick (the reference extends by `num_units_in_tick / time_scale`).
pub fn video_tick_ms(s: &Stream, data: &[u8]) -> f64 {
    if s.get("Format") == "AVC" {
        let nals = avc::nals_annexb(&data[..data.len().min(64 * 1024)]);
        if let Some((_, sps)) = nals.iter().find(|(t, _)| *t == 7) {
            if let Some(sps) = avc::parse_sps(sps) {
                if let Some((num, scale, _)) = sps.vui.timing {
                    if num > 0 && scale > 0 {
                        return num as f64 * 1000.0 / scale as f64;
                    }
                }
            }
        }
    }
    match s.get_f64("FrameRate") {
        Some(f) if f > 0.0 => 1000.0 / f,
        _ => 0.0,
    }
}

/// Apply the codec helpers for a video elementary stream carried in a program/transport stream.
/// `Format` may be preset by the container (transport stream type); otherwise it is detected.
pub fn apply_video_codec(s: &mut Stream, data: &[u8]) {
    let format = if s.has("Format") { s.get("Format").to_string() } else { video_format(data).to_string() };
    s.set("Format", &format);
    match format.as_str() {
        "MPEG Video" => {
            mpegv::apply_headers(s, data);
        }
        "AVC" => {
            let nals = avc::nals_annexb(data);
            if let Some((_, sps)) = nals.iter().find(|(t, _)| *t == 7) {
                if let Some(sps) = avc::parse_sps(sps) {
                    let cabac = nals.iter().find(|(t, _)| *t == 8).and_then(|(_, p)| avc::parse_pps_cabac(p));
                    avc::apply(s, &sps, cabac, true);
                }
            }
            avc::apply_sei_from_nals(s, &nals);
        }
        "HEVC" => {
            hevc::apply_annexb(s, data);
            let nals = hevc::nals_annexb(data);
            hevc::apply_sei_from_nals(s, &nals);
        }
        "MPEG-4 Visual" => {
            mpeg4v::apply_headers(s, data);
            mpeg4v::apply_frame_user_data(s, data);
        }
        "VC-1" => {
            vc1::apply_sequence(s, data);
        }
        _ => {}
    }
}

fn apply_audio_codec(s: &mut Stream, ps: &PsStream) {
    let data = ps.data.as_slice();
    match (ps.id, ps.sub_id) {
        (0xC0..=0xDF, _) => {
            if data.len() >= 2 && data[0] == 0xFF && data[1] & 0xF6 == 0xF0 {
                s.set("Format", "AAC");
                s.set("MuxingMode", "ADTS");
                aac::apply_adts_frame(s, data);
            } else {
                s.set("Format", "MPEG Audio");
                mpeg_audio::apply_frame(s, data);
            }
            s.set_if_empty("Compression_Mode", "Lossy");
        }
        (_, Some(0x80..=0x87)) => {
            s.set("Format", "AC-3");
            s.set("MuxingMode", "DVD-Video");
            ac3::apply_frame(s, data);
            s.set_if_empty("Compression_Mode", "Lossy");
        }
        (_, Some(0x88..=0x8F)) => {
            s.set("Format", "DTS");
            s.set("MuxingMode", "DVD-Video");
            dts::apply_frame(s, data);
        }
        (_, Some(0xA0..=0xA7)) => {
            s.set("Format", "PCM");
            s.set("MuxingMode", "DVD-Video");
            let (bits, rate, channels) = match ps.lpcm {
                Some(h) => {
                    let bits = match h[1] >> 6 {
                        1 => 20,
                        2 => 24,
                        _ => 16,
                    };
                    let rate = if (h[1] >> 4) & 3 == 1 { 96000 } else { 48000 };
                    (bits, rate, (h[1] & 7) as u32 + 1)
                }
                None => (16, 48000, 2),
            };
            s.set_int("SamplingRate", rate as i128);
            s.set_int("Channel(s)", channels as i128);
            s.set_int("BitDepth", bits as i128);
            s.set_int("BitRate", (rate * bits * channels) as i128);
            s.set("BitRate_Mode", "CBR");
            pcm::apply_pcm(s, Some(false), Some(true), false, bits);
            s.set_if_empty("Format_Settings_Endianness", "Big");
            s.set_if_empty("Compression_Mode", "Lossless");
        }
        _ => {}
    }
}

/// Finish the timing of a stream from its first/last timestamps (90 kHz ticks).
/// Video: `last - first + tick`. Audio: size-based when CBR (left to `finish`), else `last - first`.
pub fn apply_timing(s: &mut Stream, first: Option<u64>, last: Option<u64>, tick: f64, bytes: u64, complete: bool, count_size: bool) {
    let (Some(first), Some(last)) = (first, last) else {
        if count_size && bytes > 0 && complete {
            s.set_int("StreamSize", bytes as i128);
        }
        return;
    };
    let first_ms = first as f64 / 90.0;
    let last_ms = ticks_to_ms(last, first);
    let span = (last_ms - first_ms).max(0.0);
    s.set("Delay", format!("{first_ms:.6}"));
    s.set("Delay_Source", "Container");
    match s.kind {
        StreamKind::Video => {
            let dur = span + tick;
            if dur > 0.0 {
                s.set("Duration", format!("{}", dur.round() as i64));
            }
            if count_size && bytes > 0 && complete {
                s.set_int("StreamSize", bytes as i128);
            }
        }
        StreamKind::Audio => {
            let cbr = s.has("BitRate") && s.get("BitRate_Mode") != "VBR";
            if cbr && complete && bytes > 0 {
                // Duration follows from StreamSize / BitRate (frame-exact for CBR audio).
                s.set_int("StreamSize", bytes as i128);
                if let Some(br) = s.get_f64("BitRate").filter(|b| *b > 0.0) {
                    s.set("Duration", format!("{}", (bytes as f64 * 8000.0 / br).round() as i64));
                }
            } else {
                let dur = span + if cbr { tick } else { 0.0 };
                if dur > 0.0 {
                    s.set("Duration", format!("{}", dur.round() as i64));
                }
                if count_size && complete && bytes > 0 {
                    s.set_int("StreamSize", bytes as i128);
                } else if cbr {
                    if let Some(br) = s.get_f64("BitRate") {
                        s.set_int("StreamSize", (br * dur / 8000.0).round() as i128);
                    }
                }
            }
        }
        _ => {
            if span > 0.0 {
                s.set("Duration", format!("{}", span.round() as i64));
            }
            if count_size && bytes > 0 && complete {
                s.set_int("StreamSize", bytes as i128);
            }
        }
    }
}

/// Audio `Video_Delay` relative to the first video stream, and audio frame counts from the
/// duration and the codec frame rate (PES-based containers report both).
pub fn apply_video_delay(doc: &mut Doc) {
    let video_delay = doc.streams[StreamKind::Video as usize].first().and_then(|v| v.get_f64("Delay"));
    for a in doc.streams[StreamKind::Audio as usize].iter_mut() {
        if let (Some(vd), Some(d)) = (video_delay, a.get_f64("Delay")) {
            a.set("Video_Delay", format!("{}", (d - vd).round() as i64));
        }
        if !a.has("FrameCount") {
            let fps = a.get_f64("FrameRate").or_else(|| match (a.get_f64("SamplingRate"), a.get_f64("SamplesPerFrame")) {
                (Some(sr), Some(spf)) if spf > 0.0 => Some(sr / spf),
                _ => None,
            });
            if let (Some(d), Some(f)) = (a.get_f64("Duration"), fps) {
                a.set("FrameCount", format!("{}", (d / 1000.0 * f).round() as i64));
            }
        }
    }
}

fn emit(doc: &mut Doc, ctx: &Ctx) {
    let g = doc.general();
    g.set("Format", "MPEG-PS");
    if ctx.version == 1 {
        g.set("InternetMediaType", "video/mpeg");
    }
    let mut order: Vec<usize> = (0..ctx.streams.len()).collect();
    order.sort_by_key(|&i| (kind_of(&ctx.streams[i]) as usize, ctx.streams[i].first_order));
    for i in order {
        let ps = &ctx.streams[i];
        let kind = kind_of(ps);
        if kind == StreamKind::General {
            continue;
        }
        let mut s = Stream::new(kind);
        if let Some(pos) = ctx.system_ids.iter().position(|&x| x == ps.id) {
            s.set_int("StreamOrder", pos as i128);
        }
        s.set_int("FirstPacketOrder", ps.first_order as i128);
        let (id, id_str) = id_strings(ps.id, ps.sub_id);
        s.set("ID", id);
        s.set("ID/String", id_str);
        let mut tick = 0.0;
        match kind {
            StreamKind::Video => {
                apply_video_codec(&mut s, &ps.data);
                tick = video_tick_ms(&s, &ps.data);
            }
            StreamKind::Audio => {
                apply_audio_codec(&mut s, ps);
                if let (Some(sr), Some(spf)) = (s.get_f64("SamplingRate"), s.get_f64("SamplesPerFrame")) {
                    if sr > 0.0 {
                        tick = spf / sr * 1000.0;
                    }
                }
            }
            StreamKind::Text => {
                s.set("Format", "RLE");
                s.set("MuxingMode", "DVD-Video");
            }
            _ => {}
        }
        apply_timing(&mut s, ps.first_pts, ps.last_pts, tick, ps.bytes, ctx.complete, true);
        doc.streams[kind as usize].push(s);
    }
    apply_video_delay(doc);
    if ctx.has_nav {
        let m = doc.add(StreamKind::Menu);
        m.set("Format", "DVD-Video");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn pts_bytes(pts: u64, prefix: u8) -> [u8; 5] {
        [prefix | (((pts >> 30) & 7) as u8) << 1 | 1, (pts >> 22) as u8, (((pts >> 15) & 0x7F) as u8) << 1 | 1, (pts >> 7) as u8, ((pts & 0x7F) as u8) << 1 | 1]
    }

    #[test]
    fn pes_mpeg2_with_pts_dts() {
        let mut d = vec![0, 0, 1, 0xE0, 0, 0, 0x80, 0xC0, 10];
        d.extend_from_slice(&pts_bytes(48600, 0x30));
        d.extend_from_slice(&pts_bytes(45000, 0x10));
        d.extend_from_slice(&[0xAA, 0xBB]);
        let h = parse_pes(&d).unwrap();
        assert_eq!(h.stream_id, 0xE0);
        assert_eq!(h.pts, Some(48600));
        assert_eq!(h.dts, Some(45000));
        assert_eq!(h.payload_offset, 19);
        assert_eq!(ticks_to_ms(48600, 0), 540.0);
        assert_eq!(ticks_to_ms(10, (1u64 << 33) - 1), (10 + (1u64 << 33)) as f64 / 90.0);
    }

    #[test]
    fn pes_mpeg1_forms() {
        let mut d = vec![0, 0, 1, 0xC0, 0, 0, 0xFF, 0xFF, 0x40, 0x20];
        d.extend_from_slice(&pts_bytes(47698, 0x20));
        d.push(1);
        let h = parse_pes(&d).unwrap();
        assert_eq!(h.pts, Some(47698));
        assert_eq!(h.dts, None);
        assert_eq!(h.payload_offset, 15);
        let d = [0, 0, 1, 0xC0, 0, 0, 0x0F, 9];
        let h = parse_pes(&d).unwrap();
        assert_eq!(h.pts, None);
        assert_eq!(h.payload_offset, 7);
        let d = [0, 0, 1, 0xBE, 0, 2, 0xFF, 0xFF];
        assert_eq!(parse_pes(&d).unwrap().payload_offset, 6);
        assert!(parse_pes(&[0, 0, 1]).is_none());
        assert!(parse_pes(&[1, 0, 1, 0xE0, 0, 0]).is_none());
        assert!(parse_pes(&[0, 0, 1, 0xE0, 0, 0, 0x80, 0xC0]).is_none());
    }

    #[test]
    fn pack_headers() {
        let m2 = [0, 0, 1, 0xBA, 0x44, 0, 4, 0, 4, 1, 0x43, 0x36, 0x3B, 0xF8];
        assert_eq!(parse_pack(&m2), Some((2, 1101198 * 400, 14)));
        let m1 = [0, 0, 1, 0xBA, 0x21, 0, 1, 0, 1, 0xA1, 0x9B, 0x1D];
        assert_eq!(parse_pack(&m1), Some((1, 1101198 * 400, 12)));
        assert!(parse_pack(&[0, 0, 1, 0xBB, 0, 0, 0, 0, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn probe_and_ids() {
        let mut head = vec![0, 0, 1, 0xBA, 0x44, 0, 4, 0, 4, 1, 0x43, 0x36, 0x3B, 0xF8];
        head.resize(64, 0);
        assert_eq!(probe(&Probe { head: &head, ext: "vob", size: 64 }), 100);
        assert_eq!(probe(&Probe { head: &head, ext: "bin", size: 64 }), 90);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "mpg", size: 4 }), 0);
        assert_eq!(id_strings(0xBD, Some(0x80)), ("189-128".into(), "189 (0xBD)-128 (0x80)".into()));
        assert_eq!(id_strings(0xE0, None), ("224".into(), "224 (0xE0)".into()));
    }

    #[test]
    fn scan_small_stream() {
        // pack + system header + a video PES with PTS + an audio PES + a nav packet + end code
        let mut f = vec![0, 0, 1, 0xBA, 0x44, 0, 4, 0, 4, 1, 0x43, 0x36, 0x3B, 0xF8];
        f.extend_from_slice(&[0, 0, 1, 0xBB, 0, 12, 0xA1, 0x9B, 0x1D, 0x04, 0xE1, 0x7F, 0xE0, 0xE0, 0xE6, 0xC0, 0xC0, 0x20]);
        let mut v = vec![0, 0, 1, 0xE0, 0, 0, 0x80, 0x80, 5];
        v.extend_from_slice(&pts_bytes(48600, 0x20));
        v.extend_from_slice(&[0, 0, 1, 0xB3, 0x04, 0x00, 0x30, 0x13, 0xFF, 0xFF, 0xE0, 0x18]);
        let l = (v.len() - 6) as u16;
        v[4] = (l >> 8) as u8;
        v[5] = l as u8;
        f.extend_from_slice(&v);
        let mut a = vec![0, 0, 1, 0xC0, 0, 0, 0x80, 0x80, 5];
        a.extend_from_slice(&pts_bytes(47698, 0x20));
        a.extend_from_slice(&[0xFF, 0xFD, 0x44, 0xC4, 1, 2, 3, 4]);
        let l = (a.len() - 6) as u16;
        a[4] = (l >> 8) as u8;
        a[5] = l as u8;
        f.extend_from_slice(&a);
        f.extend_from_slice(&[0, 0, 1, 0xBF, 0, 2, 0, 0, 0, 0, 1, 0xB9]);
        let mut r = Reader::from_bytes(f);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "MPEG-PS");
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("ID/String"), "224 (0xE0)");
        assert_eq!(v.get("StreamOrder"), "0");
        assert_eq!(v.get("Delay"), "540.000000");
        assert_eq!(v.get("Format"), "MPEG Video");
        assert_eq!(v.get("StreamSize"), "12");
        let a = &doc.streams[StreamKind::Audio as usize][0];
        assert_eq!(a.get("ID"), "192");
        assert_eq!(a.get("StreamOrder"), "1");
        assert_eq!(a.get("FirstPacketOrder"), "1");
        assert_eq!(a.get("Delay"), "529.977778");
        assert_eq!(a.get("Video_Delay"), "-10");
        assert_eq!(doc.streams[StreamKind::Menu as usize][0].get("Format"), "DVD-Video");
    }

    #[test]
    fn private_stream_sub_ids() {
        let mut f = vec![0, 0, 1, 0xBA, 0x44, 0, 4, 0, 4, 1, 0x43, 0x36, 0x3B, 0xF8];
        let mut a = vec![0, 0, 1, 0xBD, 0, 0, 0x80, 0x80, 5];
        a.extend_from_slice(&pts_bytes(48120, 0x20));
        a.extend_from_slice(&[0xA0, 0x01, 0x00, 0x04, 0x00, 0x21, 0x80, 1, 2, 3, 4]);
        let l = (a.len() - 6) as u16;
        a[4] = (l >> 8) as u8;
        a[5] = l as u8;
        f.extend_from_slice(&a);
        let mut t = vec![0, 0, 1, 0xBD, 0, 5, 0x80, 0x00, 0x00, 0x21, 0xAA];
        t[5] = 5;
        f.extend_from_slice(&t);
        let mut r = Reader::from_bytes(f);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let a = &doc.streams[StreamKind::Audio as usize][0];
        assert_eq!(a.get("ID"), "189-160");
        assert_eq!(a.get("Format"), "PCM");
        assert_eq!(a.get("SamplingRate"), "48000");
        assert_eq!(a.get("Channel(s)"), "2");
        assert_eq!(a.get("BitDepth"), "16");
        assert_eq!(a.get("StreamSize"), "4");
        let t = &doc.streams[StreamKind::Text as usize][0];
        assert_eq!(t.get("ID/String"), "189 (0xBD)-33 (0x21)");
        assert_eq!(t.get("Format"), "RLE");
    }

    #[test]
    fn video_format_detection() {
        assert_eq!(video_format(&[0, 0, 1, 0xB3, 1, 2]), "MPEG Video");
        assert_eq!(video_format(&[0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x68]), "AVC");
        assert_eq!(video_format(&[0, 0, 0, 1, 0x40, 1]), "HEVC");
        assert_eq!(video_format(&[0, 0, 1, 0xB0, 1]), "MPEG-4 Visual");
        assert_eq!(video_format(&[0, 0, 1, 0x0F, 1]), "VC-1");
    }
}

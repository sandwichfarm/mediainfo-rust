//! MXF (SMPTE 377M): partition packs, primer, header metadata (Preface, Identification, packages,
//! tracks, sequences, timecode components, essence descriptors), essence elements, system items.

use crate::io::{be16, be32, be64, Reader};
use crate::model::{Doc, Stream, StreamKind, OPT_SHOWN};
use crate::parsers::audio::{ac3, mpeg_audio, pcm};
use crate::parsers::video::{avc, mpegv};
use crate::parsers::Probe;

const RUN_IN_MAX: usize = 64 * 1024;
const METADATA_CAP: u64 = 16 * 1024 * 1024;
const BODY_SCAN: u64 = 8 * 1024 * 1024;
const MAX_SETS: usize = 100_000;
const MAX_KLV: usize = 1_000_000;
const CODEC_BYTES: usize = 512 * 1024;

const PARTITION_PREFIX: [u8; 13] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01];
const LOCAL_SET_PREFIX: [u8; 14] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x01, 0x01, 0x01, 0x01];
const ESSENCE_PREFIX: [u8; 12] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01];
const SYSTEM_ITEM: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x04, 0x01, 0x01, 0x00];

// set ids (byte 14 of the local set key)
const PREFACE: u8 = 0x2F;
const IDENTIFICATION: u8 = 0x30;
const MATERIAL_PACKAGE: u8 = 0x36;
const SOURCE_PACKAGE: u8 = 0x37;
const TIMELINE_TRACK: u8 = 0x3B;
const EVENT_TRACK: u8 = 0x39;
const STATIC_TRACK: u8 = 0x3A;
const SEQUENCE: u8 = 0x0F;
const SOURCE_CLIP: u8 = 0x11;
const TIMECODE_COMPONENT: u8 = 0x14;
const MULTIPLE_DESCRIPTOR: u8 = 0x44;
const PICTURE_DESCRIPTORS: [u8; 5] = [0x27, 0x28, 0x29, 0x51, 0x5C];
const SOUND_DESCRIPTORS: [u8; 3] = [0x42, 0x47, 0x48];

// local tags
const T_INSTANCE: u16 = 0x3C0A;
const T_MOD_DATE: u16 = 0x3B02;
const T_IDENTIFICATIONS: u16 = 0x3B06;
const T_COMPANY: u16 = 0x3C01;
const T_PRODUCT: u16 = 0x3C02;
const T_PRODUCT_VERSION: u16 = 0x3C03;
const T_VERSION_STRING: u16 = 0x3C04;
const T_TOOLKIT_VERSION: u16 = 0x3C07;
const T_PLATFORM: u16 = 0x3C08;
const T_TRACKS: u16 = 0x4403;
const T_DESCRIPTOR: u16 = 0x4701;
const T_TRACK_ID: u16 = 0x4801;
const T_TRACK_NUMBER: u16 = 0x4804;
const T_EDIT_RATE: u16 = 0x4B01;
const T_SEQUENCE: u16 = 0x4803;
const T_DATA_DEFINITION: u16 = 0x0201;
const T_DURATION: u16 = 0x0202;
const T_COMPONENTS: u16 = 0x1001;
const T_START_TIMECODE: u16 = 0x1501;
const T_TIMECODE_BASE: u16 = 0x1502;
const T_DROP_FRAME: u16 = 0x1503;
const T_LINKED_TRACK: u16 = 0x3006;
const T_SAMPLE_RATE: u16 = 0x3001;
const T_CONTAINER: u16 = 0x3004;
const T_SUB_DESCRIPTORS: u16 = 0x3F01;
const T_PICTURE_CODING: u16 = 0x3201;
const T_STORED_HEIGHT: u16 = 0x3202;
const T_STORED_WIDTH: u16 = 0x3203;
const T_DISPLAY_HEIGHT: u16 = 0x3208;
const T_DISPLAY_WIDTH: u16 = 0x3209;
const T_FRAME_LAYOUT: u16 = 0x320C;
const T_ASPECT_RATIO: u16 = 0x320E;
const T_FIELD_DOMINANCE: u16 = 0x3212;
const T_COMPONENT_DEPTH: u16 = 0x3301;
const T_H_SUBSAMPLING: u16 = 0x3302;
const T_BLACK_REF: u16 = 0x3304;
const T_WHITE_REF: u16 = 0x3305;
const T_COLOR_RANGE: u16 = 0x3306;
const T_V_SUBSAMPLING: u16 = 0x3308;
const T_QUANT_BITS: u16 = 0x3D01;
const T_LOCKED: u16 = 0x3D02;
const T_AUDIO_RATE: u16 = 0x3D03;
const T_SOUND_CODING: u16 = 0x3D06;
const T_CHANNELS: u16 = 0x3D07;
const T_BLOCK_ALIGN: u16 = 0x3D0A;

/// BER length at `p`: (value, bytes used).
pub fn ber(b: &[u8], p: usize) -> Option<(u64, usize)> {
    let first = *b.get(p)?;
    if first < 0x80 {
        return Some((first as u64, 1));
    }
    let n = (first & 0x7F) as usize;
    if n == 0 || n > 8 {
        return None;
    }
    let mut v = 0u64;
    for i in 0..n {
        v = (v << 8) | *b.get(p + 1 + i)? as u64;
    }
    Some((v, 1 + n))
}

fn is_partition(key: &[u8]) -> bool {
    key.len() >= 16 && key[..13] == PARTITION_PREFIX && matches!(key[13], 2..=4)
}

/// Offset of the first partition pack within the run-in.
fn find_partition(head: &[u8]) -> Option<usize> {
    let limit = head.len().min(RUN_IN_MAX);
    (0..limit.saturating_sub(16)).find(|&i| is_partition(&head[i..i + 16]))
}

pub fn probe(p: &Probe) -> u8 {
    match find_partition(p.head) {
        Some(0) => 100,
        Some(_) => 80,
        None => 0,
    }
}

// ---------------------------------------------------------------------------- model

#[derive(Debug, Clone, Default)]
struct Set {
    id: u8,
    instance: [u8; 16],
    props: Vec<(u16, Vec<u8>)>,
}

impl Set {
    fn get(&self, tag: u16) -> Option<&[u8]> {
        self.props.iter().find(|(t, _)| *t == tag).map(|(_, v)| v.as_slice())
    }
    fn u32(&self, tag: u16) -> Option<u32> {
        self.get(tag).and_then(|v| match v.len() {
            1 => Some(v[0] as u32),
            2 => be16(v, 0).map(|x| x as u32),
            4 => be32(v, 0),
            8 => be64(v, 0).map(|x| x as u32),
            _ => None,
        })
    }
    fn u64(&self, tag: u16) -> Option<u64> {
        self.get(tag).and_then(|v| match v.len() {
            1 => Some(v[0] as u64),
            2 => be16(v, 0).map(|x| x as u64),
            4 => be32(v, 0).map(|x| x as u64),
            8 => be64(v, 0),
            _ => None,
        })
    }
    fn rational(&self, tag: u16) -> Option<(u32, u32)> {
        let v = self.get(tag)?;
        Some((be32(v, 0)?, be32(v, 4)?))
    }
    fn string(&self, tag: u16) -> Option<String> {
        let v = self.get(tag)?;
        let s = crate::io::clean_text(&crate::io::utf16(v, true));
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
    fn refs(&self, tag: u16) -> Vec<[u8; 16]> {
        let Some(v) = self.get(tag) else { return Vec::new() };
        let count = be32(v, 0).unwrap_or(0) as usize;
        let size = be32(v, 4).unwrap_or(0) as usize;
        if size != 16 {
            return Vec::new();
        }
        (0..count.min(4096)).filter_map(|i| v.get(8 + i * 16..24 + i * 16).map(|s| s.try_into().unwrap())).collect()
    }
    fn ul(&self, tag: u16) -> Option<[u8; 16]> {
        self.get(tag).and_then(|v| v.get(..16)).map(|s| s.try_into().unwrap())
    }
}

#[derive(Debug, Default)]
struct Partition {
    major: u16,
    minor: u16,
    status: u8,
    footer: u64,
    header_bytes: u64,
    op: [u8; 16],
}

#[derive(Debug, Default)]
struct Essence {
    track_number: u32,
    data: Vec<u8>,
    bytes: u64,
    count: u64,
}

#[derive(Debug, Default)]
struct Ctx {
    partition: Partition,
    primer: Vec<(u16, [u8; 16])>,
    sets: Vec<Set>,
    essence: Vec<Essence>,
    sdti_timecode: Option<[u8; 8]>,
    footer_size: u64,
}

impl Ctx {
    fn set(&self, instance: &[u8; 16]) -> Option<&Set> {
        self.sets.iter().find(|s| &s.instance == instance)
    }
    fn primer_ul(&self, tag: u16) -> Option<[u8; 16]> {
        self.primer.iter().find(|(t, _)| *t == tag).map(|(_, u)| *u)
    }
}

// ---------------------------------------------------------------------------- parsing

fn parse_partition(v: &[u8]) -> Option<Partition> {
    Some(Partition { major: be16(v, 0)?, minor: be16(v, 2)?, status: 0, footer: be64(v, 24)?, header_bytes: be64(v, 32)?, op: v.get(64..80)?.try_into().ok()? })
}

fn parse_primer(v: &[u8], ctx: &mut Ctx) {
    let count = be32(v, 0).unwrap_or(0) as usize;
    let size = be32(v, 4).unwrap_or(0) as usize;
    if size != 18 {
        return;
    }
    for i in 0..count.min(65536) {
        let p = 8 + i * 18;
        let (Some(tag), Some(ul)) = (be16(v, p), v.get(p + 2..p + 18)) else { break };
        ctx.primer.push((tag, ul.try_into().unwrap()));
    }
}

fn parse_local_set(id: u8, v: &[u8]) -> Set {
    let mut set = Set { id, ..Default::default() };
    let mut p = 0;
    let mut n = 0;
    while p + 4 <= v.len() && n < 4096 {
        n += 1;
        let tag = be16(v, p).unwrap_or(0);
        let len = be16(v, p + 2).unwrap_or(0) as usize;
        let Some(val) = v.get(p + 4..p + 4 + len) else { break };
        if tag == T_INSTANCE && len == 16 {
            set.instance.copy_from_slice(val);
        }
        set.props.push((tag, val.to_vec()));
        p += 4 + len;
    }
    set
}

/// Walk KLV triplets in `[start, end)`; header metadata sets are collected, essence elements sampled.
fn walk(r: &mut Reader, start: u64, end: u64, ctx: &mut Ctx, metadata: bool) -> u64 {
    let mut pos = start;
    let mut n = 0;
    while pos + 17 <= end && n < MAX_KLV {
        n += 1;
        let h = r.read_vec_at(pos, 32);
        if h.len() < 17 {
            break;
        }
        let Some((len, ll)) = ber(&h, 16) else { break };
        let key = &h[..16];
        let vpos = pos + 16 + ll as u64;
        let next = vpos.saturating_add(len);
        if is_partition(key) {
            if pos != start {
                // next partition: stop this walk, the caller decides what to do
                return pos;
            }
        } else if key[..14] == LOCAL_SET_PREFIX && metadata {
            let id = key[14];
            let v = r.read_vec_at(vpos, len.min(1 << 20) as usize);
            if ctx.sets.len() < MAX_SETS {
                ctx.sets.push(parse_local_set(id, &v));
            }
        } else if key[..13] == PARTITION_PREFIX && key[13] == 0x05 && metadata {
            let v = r.read_vec_at(vpos, len.min(1 << 20) as usize);
            parse_primer(&v, ctx);
        } else if key[..12] == ESSENCE_PREFIX {
            let track_number = be32(key, 12).unwrap_or(0);
            let e = match ctx.essence.iter_mut().position(|e| e.track_number == track_number) {
                Some(i) => &mut ctx.essence[i],
                None => {
                    if ctx.essence.len() < 256 {
                        ctx.essence.push(Essence { track_number, ..Default::default() });
                        ctx.essence.last_mut().unwrap()
                    } else {
                        pos = next;
                        continue;
                    }
                }
            };
            e.count += 1;
            e.bytes += len;
            if e.data.len() < CODEC_BYTES {
                let take = (len as usize).min(CODEC_BYTES - e.data.len());
                let v = r.read_vec_at(vpos, take);
                e.data.extend_from_slice(&v);
            }
        } else if key == SYSTEM_ITEM && ctx.sdti_timecode.is_none() {
            // System metadata pack: bitmap(1) rate(1) type(1) channel handle(2) continuity count(2)
            // label(16) creation timestamp(17) user timestamp(17: type + SMPTE 12M timecode)
            let v = r.read_vec_at(vpos, len.min(64) as usize);
            if v.len() >= 57 && v[40] == 0x81 {
                ctx.sdti_timecode = Some(v[41..49].try_into().unwrap());
            }
        }
        if next <= pos {
            break;
        }
        pos = next;
    }
    pos
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, RUN_IN_MAX);
    let Some(start) = find_partition(&head) else { return false };
    let len = r.len();
    let mut ctx = Ctx::default();
    // Header partition pack
    let h = r.read_vec_at(start as u64, 32);
    let Some((plen, ll)) = ber(&h, 16) else { return false };
    let status = h[14];
    let body = r.read_vec_at(start as u64 + 16 + ll as u64, plen.min(4096) as usize);
    let Some(mut part) = parse_partition(&body) else { return false };
    part.status = status;
    ctx.partition = part;
    let meta_start = start as u64 + 16 + ll as u64 + plen;
    // HeaderByteCount is counted differently by writers (some exclude the leading fill), so the walk
    // simply stops at the next partition pack; the count only bounds the read.
    let meta_end = len.min(meta_start + ctx.partition.header_bytes.clamp(64 * 1024, METADATA_CAP) + 64 * 1024);
    let reached = walk(r, meta_start, meta_end, &mut ctx, true);
    // Footer partition: size, and metadata when the header carried none
    if ctx.partition.footer > 0 && ctx.partition.footer + 16 <= len {
        let fpos = start as u64 + ctx.partition.footer;
        let fh = r.read_vec_at(fpos, 32);
        if is_partition(&fh) {
            ctx.footer_size = len - fpos;
            if ctx.sets.is_empty() {
                if let Some((fl, fll)) = ber(&fh, 16) {
                    let fbody = r.read_vec_at(fpos + 16 + fll as u64, fl.min(4096) as usize);
                    if let Some(fp) = parse_partition(&fbody) {
                        let ms = fpos + 16 + fll as u64 + fl;
                        let me = (ms + fp.header_bytes.min(METADATA_CAP)).min(len);
                        walk(r, ms, me, &mut ctx, true);
                    }
                }
            }
        }
    }
    // Essence: walk the body from where the header metadata walk stopped (bounded).
    let body_start = reached;
    let body_end = len.min(body_start + BODY_SCAN);
    let mut pos = body_start;
    let mut guard = 0;
    while pos + 17 <= body_end && guard < 64 {
        guard += 1;
        let h = r.read_vec_at(pos, 32);
        if is_partition(&h) {
            // skip the partition pack itself, then continue with its content
            let Some((pl, pll)) = ber(&h, 16) else { break };
            pos += 16 + pll as u64 + pl;
            continue;
        }
        let next = walk(r, pos, body_end, &mut ctx, false);
        if next <= pos {
            break;
        }
        pos = next;
    }
    emit(doc, &ctx);
    true
}

// ---------------------------------------------------------------------------- emit helpers

fn hex_upper(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn timecode_string(frames: u64, base: u32, drop: bool) -> String {
    let base = base.max(1) as u64;
    let ff = frames % base;
    let s = frames / base;
    let sep = if drop { ';' } else { ':' };
    format!("{:02}:{:02}:{:02}{}{:02}", s / 3600, (s / 60) % 60, s % 60, sep, ff)
}

fn bcd(v: u8) -> u64 {
    ((v >> 4) * 10 + (v & 0x0F)) as u64
}

/// SMPTE 12M 8-byte timecode → string.
fn smpte12m(b: &[u8; 8]) -> (String, bool) {
    let drop = b[0] & 0x40 != 0;
    let frames = bcd(b[0] & 0x3F);
    let secs = bcd(b[1] & 0x7F);
    let mins = bcd(b[2] & 0x7F);
    let hours = bcd(b[3] & 0x3F);
    (format!("{hours:02}:{mins:02}:{secs:02}{}{frames:02}", if drop { ';' } else { ':' }), drop)
}

fn mxf_timestamp(v: &[u8]) -> String {
    if v.len() < 8 {
        return String::new();
    }
    let year = be16(v, 0).unwrap_or(0);
    let ms = v[7] as u32 * 4;
    format!("{}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}", year, v[2], v[3], v[4], v[5], v[6], ms)
}

fn version_string(v: &[u8]) -> String {
    if v.len() >= 10 {
        (0..5).map(|i| be16(v, i * 2).unwrap_or(0).to_string()).collect::<Vec<_>>().join(".")
    } else {
        String::new()
    }
}

fn op_name(op: &[u8; 16]) -> String {
    if op[..8] != [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, op[7]] || op[8..12] != [0x0D, 0x01, 0x02, 0x01] {
        return String::new();
    }
    match (op[12], op[13]) {
        (0x10, _) => "OP-Atom".to_string(),
        (a @ 1..=3, b @ 1..=3) => format!("OP-{}{}", a, (b'a' + b - 1) as char),
        _ => String::new(),
    }
}

/// Picture/sound essence coding UL → Format.
fn coding_format(ul: &[u8; 16]) -> Option<&'static str> {
    if ul[..4] != [0x06, 0x0E, 0x2B, 0x34] || ul[8] != 0x04 {
        return None;
    }
    let f = match ul[9..13] {
        [0x01, 0x02, 0x02, 0x01] => match ul[13] {
            0x01..=0x11 => "MPEG Video",
            0x20 => "MPEG-4 Visual",
            0x31 | 0x32 => "AVC",
            0x3C | 0x3D => "HEVC",
            _ => return None,
        },
        [0x01, 0x02, 0x02, 0x02] => "DV",
        [0x01, 0x02, 0x02, 0x03] => "JPEG 2000",
        [0x01, 0x02, 0x02, 0x04] => "VC-1",
        [0x01, 0x02, 0x02, 0x71] => "VC-3",
        [0x01, 0x02, 0x01, _] => "YUV",
        [0x02, 0x02, 0x01, _] => "PCM",
        [0x02, 0x02, 0x02, 0x03] => match (ul[13], ul[14]) {
            (0x02, 0x01) => "AC-3",
            (0x02, 0x1C) => "Dolby E",
            (0x03, _) => "MPEG Audio",
            (0x01, _) => "PCM",
            _ => return None,
        },
        _ => return None,
    };
    Some(f)
}

/// Essence container UL → (Format fallback, wrapping string).
fn container_info(ul: &[u8; 16]) -> (&'static str, String) {
    if ul[8..12] != [0x0D, 0x01, 0x03, 0x01] {
        return ("", String::new());
    }
    let family = ul[13];
    let format = match family {
        0x01 | 0x04 | 0x07 | 0x08 | 0x09 => "MPEG Video",
        0x02 => "DV",
        0x05 => "YUV",
        0x06 | 0x0A => "PCM",
        0x0C => "JPEG 2000",
        0x10 => "AVC",
        0x11 => "VC-3",
        _ => "",
    };
    let wrapping = if family == 0x06 {
        match ul[14] {
            0x01 => "Frame (BWF)",
            0x02 => "Clip (BWF)",
            0x03 => "Frame (AES)",
            0x04 => "Clip (AES)",
            _ => "",
        }
        .to_string()
    } else {
        match ul[15] {
            0x01 => "Frame",
            0x02 => "Clip",
            0x03 => "Line",
            0x04 => "Custom",
            _ => "",
        }
        .to_string()
    };
    (format, wrapping)
}

#[derive(Debug, Clone)]
struct TrackInfo {
    track_id: u32,
    track_number: u32,
    edit_rate: (u32, u32),
    kind: u8, // 1 picture, 2 sound, 3 data, 4 timecode, 0 other
    duration: Option<u64>,
    timecode: Option<(u64, u32, bool)>, // start, base, drop frame
}

fn data_definition_kind(ul: &[u8; 16]) -> u8 {
    // 06.0E.2B.34.04.01.01.01.01.03.02.02.xx: 01 picture, 02 sound, 03 data;
    // 06.0E.2B.34.04.01.01.01.01.03.02.01.xx: 01/02 SMPTE 12M timecode, 03 descriptive metadata
    if ul[8..12] == [0x01, 0x03, 0x02, 0x02] {
        match ul[12] {
            0x01 => 1,
            0x02 => 2,
            0x03 => 3,
            _ => 0,
        }
    } else if ul[8..12] == [0x01, 0x03, 0x02, 0x01] {
        match ul[12] {
            0x01 | 0x02 => 4,
            0x03 => 3,
            _ => 0,
        }
    } else {
        0
    }
}

fn package_tracks(ctx: &Ctx, pkg: &Set) -> Vec<TrackInfo> {
    let mut out = Vec::new();
    for tref in pkg.refs(T_TRACKS) {
        let Some(track) = ctx.set(&tref) else { continue };
        if !matches!(track.id, TIMELINE_TRACK | EVENT_TRACK | STATIC_TRACK) {
            continue;
        }
        let mut info = TrackInfo { track_id: track.u32(T_TRACK_ID).unwrap_or(0), track_number: track.u32(T_TRACK_NUMBER).unwrap_or(0), edit_rate: track.rational(T_EDIT_RATE).unwrap_or((0, 1)), kind: 0, duration: None, timecode: None };
        if let Some(seq) = track.ul(T_SEQUENCE).and_then(|u| ctx.set(&u)) {
            if let Some(dd) = seq.ul(T_DATA_DEFINITION) {
                info.kind = data_definition_kind(&dd);
            }
            info.duration = seq.u64(T_DURATION);
            let comps = if seq.id == SEQUENCE { seq.refs(T_COMPONENTS) } else { Vec::new() };
            let components: Vec<&Set> = if comps.is_empty() { vec![seq] } else { comps.iter().filter_map(|c| ctx.set(c)).collect() };
            for c in components {
                if c.id == TIMECODE_COMPONENT {
                    info.kind = 4;
                    info.timecode = Some((c.u64(T_START_TIMECODE).unwrap_or(0), c.u32(T_TIMECODE_BASE).unwrap_or(0), c.u32(T_DROP_FRAME).unwrap_or(0) != 0));
                    if info.duration.is_none() {
                        info.duration = c.u64(T_DURATION);
                    }
                } else if c.id == SOURCE_CLIP && info.duration.is_none() {
                    info.duration = c.u64(T_DURATION);
                }
            }
        }
        out.push(info);
    }
    out
}

fn duration_ms(frames: u64, rate: (u32, u32)) -> Option<f64> {
    if rate.0 == 0 || rate.1 == 0 {
        return None;
    }
    Some(frames as f64 * rate.1 as f64 * 1000.0 / rate.0 as f64)
}

fn push_timecode_stream(doc: &mut Doc, t: &TrackInfo, label: &str) {
    let Some((start, base, drop)) = t.timecode else { return };
    let mut o = Stream::new(StreamKind::Other);
    o.set("ID", format!("{}-{label}", t.track_id));
    o.set("Type", "Time code");
    o.set("Format", "MXF TC");
    if t.edit_rate.0 > 0 && t.edit_rate.1 > 0 {
        o.set("FrameRate", format!("{:.3}", t.edit_rate.0 as f64 / t.edit_rate.1 as f64));
    }
    let base = if base > 0 { base } else if t.edit_rate.1 > 0 { (t.edit_rate.0 as f64 / t.edit_rate.1 as f64).round() as u32 } else { 25 };
    o.set("TimeCode_FirstFrame", timecode_string(start, base, drop));
    o.set("TimeCode_Settings", format!("{label} Package"));
    o.set("TimeCode_Striped", "Yes");
    doc.streams[StreamKind::Other as usize].push(o);
}

fn emit(doc: &mut Doc, ctx: &Ctx) {
    let g = doc.general();
    g.set("Format", "MXF");
    let p = &ctx.partition;
    if p.major > 0 || p.minor > 0 {
        g.set("Format_Version", format!("{}.{}", p.major, p.minor));
    }
    let op = op_name(&p.op);
    if !op.is_empty() {
        g.set("Format_Profile", op);
    }
    let status = match p.status {
        1 => "Open / Incomplete",
        2 => "Closed / Incomplete",
        3 => "Open / Complete",
        4 => "Closed / Complete",
        _ => "",
    };
    if !status.is_empty() {
        g.set("Format_Settings", status);
    }
    if ctx.footer_size > 0 {
        g.set_int("FooterSize", ctx.footer_size as i128);
    }
    // Preface / identification
    if let Some(preface) = ctx.sets.iter().find(|s| s.id == PREFACE) {
        if let Some(d) = preface.get(T_MOD_DATE) {
            let t = mxf_timestamp(d);
            if !t.is_empty() {
                g.set("Encoded_Date", t);
            }
        }
        let ids = preface.refs(T_IDENTIFICATIONS);
        let ident = ids.iter().filter_map(|u| ctx.set(u)).next().or_else(|| ctx.sets.iter().find(|s| s.id == IDENTIFICATION));
        if let Some(id) = ident {
            let company = id.string(T_COMPANY).unwrap_or_default();
            let product = id.string(T_PRODUCT).unwrap_or_default();
            let mut version = id.get(T_PRODUCT_VERSION).map(version_string).unwrap_or_default();
            if version.is_empty() || version == "0.0.0.0.0" {
                version = id.string(T_VERSION_STRING).unwrap_or_default();
            }
            if !company.is_empty() {
                g.set("Encoded_Application_CompanyName", &company);
            }
            if !product.is_empty() {
                g.set("Encoded_Application_Name", &product);
            }
            if !version.is_empty() {
                g.set("Encoded_Application_Version", &version);
            }
            let app: Vec<&str> = [company.as_str(), product.as_str(), version.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
            if !app.is_empty() {
                g.set("Encoded_Application/String", app.join(" "));
            }
            let platform = id.string(T_PLATFORM).unwrap_or_default();
            let toolkit = id.get(T_TOOLKIT_VERSION).map(version_string).unwrap_or_default();
            if !platform.is_empty() {
                g.set("Encoded_Library_Name", &platform);
            }
            if !toolkit.is_empty() && toolkit != "0.0.0.0.0" {
                g.set("Encoded_Library_Version", &toolkit);
            }
            let lib: Vec<&str> = [platform.as_str(), toolkit.as_str()].into_iter().filter(|s| !s.is_empty() && *s != "0.0.0.0.0").collect();
            if !lib.is_empty() {
                g.set("Encoded_Library/String", lib.join(" "));
            }
        }
    }

    // Packages
    let material: Vec<&Set> = ctx.sets.iter().filter(|s| s.id == MATERIAL_PACKAGE).collect();
    let sources: Vec<&Set> = ctx.sets.iter().filter(|s| s.id == SOURCE_PACKAGE).collect();
    let material_tracks: Vec<TrackInfo> = material.iter().flat_map(|p| package_tracks(ctx, p)).collect();
    let mut general_duration: Option<f64> = None;
    for t in &material_tracks {
        if let Some(d) = t.duration.and_then(|f| duration_ms(f, t.edit_rate)) {
            if matches!(t.kind, 1 | 2) {
                general_duration = Some(general_duration.map_or(d, |m: f64| m.max(d)));
            }
        }
    }
    if let Some(d) = general_duration {
        doc.general().set("Duration", format!("{}", d.round() as i64));
    }

    // Essence streams from the source (file) packages
    let sdti_ms: Option<f64> = ctx.sdti_timecode.map(|tc| {
        let hours = bcd(tc[3] & 0x3F);
        let mins = bcd(tc[2] & 0x7F);
        let secs = bcd(tc[1] & 0x7F);
        ((hours * 60 + mins) * 60 + secs) as f64 * 1000.0
    });
    let mut order = 0usize;
    let mut source_tc_tracks: Vec<TrackInfo> = Vec::new();
    for pkg in &sources {
        let tracks = package_tracks(ctx, pkg);
        let descriptor = pkg.ul(T_DESCRIPTOR).and_then(|u| ctx.set(&u));
        let mut descriptors: Vec<&Set> = Vec::new();
        if let Some(d) = descriptor {
            if d.id == MULTIPLE_DESCRIPTOR {
                descriptors.extend(d.refs(T_SUB_DESCRIPTORS).iter().filter_map(|u| ctx.set(u)));
            } else {
                descriptors.push(d);
            }
        }
        let pkg_timecode = tracks.iter().find(|t| t.kind == 4).and_then(|t| t.timecode);
        for t in tracks.iter().filter(|t| t.kind == 4) {
            source_tc_tracks.push(t.clone());
        }
        let essence_tracks: Vec<&TrackInfo> = tracks.iter().filter(|t| matches!(t.kind, 1..=3)).collect();
        for t in essence_tracks.iter() {
            let desc = descriptors.iter().find(|d| d.u32(T_LINKED_TRACK) == Some(t.track_id)).or_else(|| if descriptors.len() == 1 && essence_tracks.len() == 1 { descriptors.first() } else { None }).or_else(|| descriptors.iter().find(|d| (t.kind == 1 && PICTURE_DESCRIPTORS.contains(&d.id)) || (t.kind == 2 && SOUND_DESCRIPTORS.contains(&d.id))));
            let kind = match t.kind {
                1 => StreamKind::Video,
                2 => StreamKind::Audio,
                _ => StreamKind::Text,
            };
            let mut s = Stream::new(kind);
            s.set_int("StreamOrder", order as i128);
            order += 1;
            s.set_int("ID", t.track_id as i128);
            let essence = ctx.essence.iter().find(|e| e.track_number == t.track_number && t.track_number != 0);
            let data: &[u8] = essence.map(|e| e.data.as_slice()).unwrap_or(&[]);
            let mut container_format = "";
            if let Some(d) = desc {
                let container = d.ul(T_CONTAINER);
                let coding = if kind == StreamKind::Video { d.ul(T_PICTURE_CODING) } else { d.ul(T_SOUND_CODING) };
                let mut codec_id = String::new();
                if let Some(c) = container {
                    let (f, wrapping) = container_info(&c);
                    container_format = f;
                    codec_id = hex_upper(&c[8..16]);
                    if !wrapping.is_empty() {
                        s.set("Format_Settings_Wrapping", wrapping);
                    }
                }
                if let Some(c) = coding {
                    if !codec_id.is_empty() {
                        codec_id.push('-');
                    }
                    codec_id.push_str(&hex_upper(&c[8..16]));
                    if let Some(f) = coding_format(&c) {
                        s.set("Format", f);
                    }
                }
                if !codec_id.is_empty() {
                    s.set("CodecID", codec_id);
                }
                if !s.has("Format") && !container_format.is_empty() {
                    s.set("Format", container_format);
                }
                apply_descriptor(&mut s, d, ctx, t);
            }
            if !s.has("Format") && kind == StreamKind::Video {
                s.set("Format", crate::parsers::mpeg_ps::video_format(data));
            }
            // codec helpers on the first essence bytes
            match (kind, s.get("Format")) {
                (StreamKind::Video, "MPEG Video") => {
                    mpegv::apply_headers(&mut s, data);
                }
                (StreamKind::Video, "AVC") => {
                    let nals = avc::nals_annexb(data);
                    if let Some((_, sps)) = nals.iter().find(|(t, _)| *t == 7) {
                        if let Some(sps) = avc::parse_sps(sps) {
                            let cabac = nals.iter().find(|(t, _)| *t == 8).and_then(|(_, p)| avc::parse_pps_cabac(p));
                            avc::apply(&mut s, &sps, cabac, true);
                        }
                    }
                    avc::apply_sei_from_nals(&mut s, &nals);
                }
                (StreamKind::Audio, "AC-3") => {
                    ac3::apply_frame(&mut s, data);
                }
                (StreamKind::Audio, "MPEG Audio") => {
                    mpeg_audio::apply_frame(&mut s, data);
                }
                _ => {}
            }
            // Timing
            if t.edit_rate.0 > 0 && t.edit_rate.1 > 0 {
                let fps = t.edit_rate.0 as f64 / t.edit_rate.1 as f64;
                if kind == StreamKind::Video {
                    s.set_if_empty("FrameRate", format!("{fps:.3}"));
                }
                if kind == StreamKind::Audio && s.get("Format_Settings_Wrapping").starts_with("Frame") {
                    if let Some(sr) = s.get_f64("SamplingRate") {
                        if sr > 0.0 && fps > 0.0 {
                            s.set_if_empty("SamplesPerFrame", format!("{}", (sr / fps).round() as u64));
                        }
                    }
                }
            }
            if let Some(d) = t.duration.and_then(|f| duration_ms(f, t.edit_rate)) {
                s.set("Duration", format!("{}", d.round() as i64));
            }
            if let (Some(br), Some(d)) = (s.get_f64("BitRate"), s.get_f64("Duration")) {
                if br > 0.0 && d > 0.0 && !s.has("StreamSize") {
                    s.set("StreamSize", format!("{}", (br * d / 8000.0).round() as i64));
                }
            }
            if let Some((start, base, drop)) = pkg_timecode {
                let base = base.max(1) as f64;
                s.set("Delay", format!("{}", (start as f64 / base * 1000.0).round() as i64));
                s.set("Delay_Source", "Container");
                s.set("Delay_DropFrame", if drop { "Yes" } else { "No" });
            }
            if let Some(ms) = sdti_ms {
                s.set_extra("Delay_SDTI", format!("{}", ms as i64), "", "N NT");
            }
            doc.streams[kind as usize].push(s);
        }
    }

    // Timecode tracks → Other streams (material first, then source, then the SDTI system item)
    for t in material_tracks.iter().filter(|t| t.kind == 4) {
        push_timecode_stream(doc, t, "Material");
    }
    for t in &source_tc_tracks {
        push_timecode_stream(doc, t, "Source");
    }
    if let Some(tc) = ctx.sdti_timecode {
        let mut o = Stream::new(StreamKind::Other);
        o.set("Type", "Time code");
        o.set("Format", "SMPTE TC");
        o.set("MuxingMode", "SDTI");
        let fps = material_tracks.iter().chain(source_tc_tracks.iter()).find(|t| t.edit_rate.0 > 0).map(|t| t.edit_rate.0 as f64 / t.edit_rate.1.max(1) as f64);
        if let Some(f) = fps {
            o.set("FrameRate", format!("{f:.3}"));
        }
        let (text, _) = smpte12m(&tc);
        o.set("TimeCode_FirstFrame", text);
        doc.streams[StreamKind::Other as usize].push(o);
    }
    crate::parsers::mpeg_ps::apply_video_delay(doc);
}

/// Fill a stream from its file descriptor set.
fn apply_descriptor(s: &mut Stream, d: &Set, ctx: &Ctx, t: &TrackInfo) {
    match s.kind {
        StreamKind::Video => {
            let w = d.u32(T_DISPLAY_WIDTH).filter(|v| *v > 0).or_else(|| d.u32(T_STORED_WIDTH));
            let h = d.u32(T_DISPLAY_HEIGHT).filter(|v| *v > 0).or_else(|| d.u32(T_STORED_HEIGHT));
            if let (Some(w), Some(h)) = (w, h) {
                if w > 0 && h > 0 {
                    s.set_int("Width", w as i128);
                    s.set_int("Height", h as i128);
                }
            }
            if let Some((n, dn)) = d.rational(T_ASPECT_RATIO) {
                if n > 0 && dn > 0 {
                    s.set("DisplayAspectRatio", format!("{:.3}", n as f64 / dn as f64));
                }
            }
            if let Some((n, dn)) = d.rational(T_SAMPLE_RATE) {
                if n > 0 && dn > 0 && t.edit_rate.0 == 0 {
                    s.set("FrameRate", format!("{:.3}", n as f64 / dn as f64));
                }
            }
            match d.u32(T_FRAME_LAYOUT) {
                Some(0) => s.set("ScanType", "Progressive"),
                Some(1) | Some(2) => {
                    s.set("ScanType", "Interlaced");
                    match d.u32(T_FIELD_DOMINANCE) {
                        Some(1) => s.set("ScanOrder", "TFF"),
                        Some(2) => s.set("ScanOrder", "BFF"),
                        _ => {}
                    }
                }
                Some(3) => s.set("ScanType", "Interlaced"),
                _ => {}
            }
            if d.id == 0x29 {
                s.set("ColorSpace", "RGB");
            } else if d.id == 0x28 || d.id == 0x51 {
                s.set("ColorSpace", "YUV");
                let hs = d.u32(T_H_SUBSAMPLING).unwrap_or(0);
                let vs = d.u32(T_V_SUBSAMPLING).unwrap_or(1);
                let cs = match (hs, vs) {
                    (1, 1) => "4:4:4",
                    (2, 1) => "4:2:2",
                    (2, 2) => "4:2:0",
                    (4, 1) => "4:1:1",
                    (4, 4) => "4:1:0",
                    _ => "",
                };
                if !cs.is_empty() {
                    s.set("ChromaSubsampling", cs);
                }
            }
            if let Some(depth) = d.u32(T_COMPONENT_DEPTH).filter(|v| *v > 0) {
                s.set_int("BitDepth", depth as i128);
                let black = d.u32(T_BLACK_REF);
                let white = d.u32(T_WHITE_REF);
                let range = d.u32(T_COLOR_RANGE);
                let max = (1u32 << depth.min(16)) - 1;
                if let (Some(b), Some(w)) = (black, white) {
                    let limited = b > 0 || w < max || range.is_some_and(|r| r < max);
                    s.set("colour_range", if limited { "Limited" } else { "Full" });
                    s.set("colour_range_Source", "Container");
                }
            }
            // Dynamic (primer-mapped) properties: MPEG-2 bit rate 04.01.06.02.01.0B
            for (tag, v) in &d.props {
                if *tag < 0x8000 {
                    continue;
                }
                if let Some(ul) = ctx.primer_ul(*tag) {
                    if ul[8..14] == [0x04, 0x01, 0x06, 0x02, 0x01, 0x0B] {
                        if let Some(br) = be32(v, 0).filter(|b| *b > 0) {
                            s.set_int("BitRate", br as i128);
                        }
                    }
                }
            }
        }
        StreamKind::Audio => {
            if let Some((n, dn)) = d.rational(T_AUDIO_RATE) {
                if n > 0 && dn > 0 {
                    s.set_int("SamplingRate", (n / dn) as i128);
                }
            }
            if let Some(c) = d.u32(T_CHANNELS).filter(|v| *v > 0) {
                s.set_int("Channel(s)", c as i128);
            }
            let bits = d.u32(T_QUANT_BITS).filter(|v| *v > 0);
            let format = s.get("Format").to_string();
            if format == "PCM" || format.is_empty() {
                s.set_if_empty("Format", "PCM");
                if let Some(b) = bits {
                    s.set_int("BitDepth", b as i128);
                    if let (Some(sr), Some(ch)) = (s.get_u64("SamplingRate"), s.get_u64("Channel(s)")) {
                        s.set_int("BitRate", (sr * ch * b as u64) as i128);
                        s.set("BitRate_Mode", "CBR");
                    }
                    pcm::apply_pcm(s, Some(true), None, false, b);
                    s.set_if_empty("Format_Settings_Endianness", "Little");
                    s.set_if_empty("Format_Settings", "Little");
                }
            }
            if let Some(l) = d.u32(T_LOCKED) {
                s.set_extra("Locked", if l != 0 { "Yes" } else { "No" }, "", OPT_SHOWN);
            }
            if let Some(b) = d.u32(T_BLOCK_ALIGN).filter(|v| *v > 0) {
                s.set_extra("BlockAlignment", b.to_string(), "", "N NT");
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn klv(key: &[u8; 16], value: &[u8]) -> Vec<u8> {
        let mut v = key.to_vec();
        v.push(0x83);
        v.extend_from_slice(&(value.len() as u32).to_be_bytes()[1..]);
        v.extend_from_slice(value);
        v
    }

    fn set_key(id: u8) -> [u8; 16] {
        let mut k = [0u8; 16];
        k[..14].copy_from_slice(&LOCAL_SET_PREFIX);
        k[14] = id;
        k
    }

    fn uid(n: u8) -> [u8; 16] {
        let mut u = [0xAAu8; 16];
        u[15] = n;
        u
    }

    fn local(props: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut v = Vec::new();
        for (t, p) in props {
            v.extend_from_slice(&t.to_be_bytes());
            v.extend_from_slice(&(p.len() as u16).to_be_bytes());
            v.extend_from_slice(p);
        }
        v
    }

    fn refs(items: &[[u8; 16]]) -> Vec<u8> {
        let mut v = (items.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(&16u32.to_be_bytes());
        for i in items {
            v.extend_from_slice(i);
        }
        v
    }

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
    }

    fn build() -> Vec<u8> {
        let mut meta = Vec::new();
        // primer: 0x8000 → MPEG-2 bit rate UL
        let mut primer = 1u32.to_be_bytes().to_vec();
        primer.extend_from_slice(&18u32.to_be_bytes());
        primer.extend_from_slice(&0x8000u16.to_be_bytes());
        primer.extend_from_slice(&[0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x05, 0x04, 0x01, 0x06, 0x02, 0x01, 0x0B, 0x00, 0x00]);
        let mut primer_key = [0u8; 16];
        primer_key[..13].copy_from_slice(&PARTITION_PREFIX);
        primer_key[13] = 0x05;
        primer_key[14] = 0x01;
        meta.extend(klv(&primer_key, &primer));
        let op = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x01, 0x09, 0x00];
        meta.extend(klv(&set_key(PREFACE), &local(&[(T_INSTANCE, uid(1).to_vec()), (T_MOD_DATE, vec![0; 8]), (T_IDENTIFICATIONS, refs(&[uid(2)]))])));
        meta.extend(klv(&set_key(IDENTIFICATION), &local(&[(T_INSTANCE, uid(2).to_vec()), (T_COMPANY, utf16("FFmpeg")), (T_PRODUCT, utf16("OP1a Muxer")), (T_PRODUCT_VERSION, vec![0, 63, 0, 1, 0, 101, 0, 0, 0, 0]), (T_VERSION_STRING, utf16("63.1.101")), (T_PLATFORM, utf16("Lavf (linux)")), (T_TOOLKIT_VERSION, vec![0, 63, 0, 1, 0, 101, 0, 0, 0, 0])])));
        let picture_dd = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x02, 0x01, 0x00, 0x00, 0x00];
        let sound_dd = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x02, 0x02, 0x00, 0x00, 0x00];
        let tc_dd = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00];
        let rate25 = [0, 0, 0, 25, 0, 0, 0, 1].to_vec();
        // material package: tc track 1, video track 2
        meta.extend(klv(&set_key(MATERIAL_PACKAGE), &local(&[(T_INSTANCE, uid(10).to_vec()), (T_TRACKS, refs(&[uid(11), uid(13)]))])));
        meta.extend(klv(&set_key(TIMELINE_TRACK), &local(&[(T_INSTANCE, uid(11).to_vec()), (T_TRACK_ID, 1u32.to_be_bytes().to_vec()), (T_EDIT_RATE, rate25.clone()), (T_SEQUENCE, uid(12).to_vec())])));
        meta.extend(klv(&set_key(SEQUENCE), &local(&[(T_INSTANCE, uid(12).to_vec()), (T_DATA_DEFINITION, tc_dd.to_vec()), (T_DURATION, 25u64.to_be_bytes().to_vec()), (T_COMPONENTS, refs(&[uid(15)]))])));
        meta.extend(klv(&set_key(TIMECODE_COMPONENT), &local(&[(T_INSTANCE, uid(15).to_vec()), (T_START_TIMECODE, 90000u64.to_be_bytes().to_vec()), (T_TIMECODE_BASE, vec![0, 25]), (T_DROP_FRAME, vec![0])])));
        meta.extend(klv(&set_key(TIMELINE_TRACK), &local(&[(T_INSTANCE, uid(13).to_vec()), (T_TRACK_ID, 2u32.to_be_bytes().to_vec()), (T_EDIT_RATE, rate25.clone()), (T_SEQUENCE, uid(14).to_vec())])));
        meta.extend(klv(&set_key(SEQUENCE), &local(&[(T_INSTANCE, uid(14).to_vec()), (T_DATA_DEFINITION, picture_dd.to_vec()), (T_DURATION, 25u64.to_be_bytes().to_vec())])));
        // source package: video track 2 (number 0x15010500), audio track 3, multiple descriptor
        meta.extend(klv(&set_key(SOURCE_PACKAGE), &local(&[(T_INSTANCE, uid(20).to_vec()), (T_TRACKS, refs(&[uid(21), uid(23)])), (T_DESCRIPTOR, uid(30).to_vec())])));
        meta.extend(klv(&set_key(TIMELINE_TRACK), &local(&[(T_INSTANCE, uid(21).to_vec()), (T_TRACK_ID, 2u32.to_be_bytes().to_vec()), (T_TRACK_NUMBER, 0x15010500u32.to_be_bytes().to_vec()), (T_EDIT_RATE, rate25.clone()), (T_SEQUENCE, uid(22).to_vec())])));
        meta.extend(klv(&set_key(SEQUENCE), &local(&[(T_INSTANCE, uid(22).to_vec()), (T_DATA_DEFINITION, picture_dd.to_vec()), (T_DURATION, 25u64.to_be_bytes().to_vec())])));
        meta.extend(klv(&set_key(TIMELINE_TRACK), &local(&[(T_INSTANCE, uid(23).to_vec()), (T_TRACK_ID, 3u32.to_be_bytes().to_vec()), (T_TRACK_NUMBER, 0x16010300u32.to_be_bytes().to_vec()), (T_EDIT_RATE, rate25.clone()), (T_SEQUENCE, uid(24).to_vec())])));
        meta.extend(klv(&set_key(SEQUENCE), &local(&[(T_INSTANCE, uid(24).to_vec()), (T_DATA_DEFINITION, sound_dd.to_vec()), (T_DURATION, 25u64.to_be_bytes().to_vec())])));
        meta.extend(klv(&set_key(MULTIPLE_DESCRIPTOR), &local(&[(T_INSTANCE, uid(30).to_vec()), (T_SUB_DESCRIPTORS, refs(&[uid(31), uid(32)]))])));
        let mpeg_container = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x02, 0x0D, 0x01, 0x03, 0x01, 0x02, 0x04, 0x60, 0x01];
        let mpeg_coding = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x03, 0x04, 0x01, 0x02, 0x02, 0x01, 0x01, 0x11, 0x00];
        meta.extend(klv(&set_key(0x51), &local(&[(T_INSTANCE, uid(31).to_vec()), (T_LINKED_TRACK, 2u32.to_be_bytes().to_vec()), (T_SAMPLE_RATE, rate25.clone()), (T_CONTAINER, mpeg_container.to_vec()), (T_STORED_WIDTH, 64u32.to_be_bytes().to_vec()), (T_STORED_HEIGHT, 48u32.to_be_bytes().to_vec()), (T_FRAME_LAYOUT, vec![0]), (T_ASPECT_RATIO, [0, 0, 0, 4, 0, 0, 0, 3].to_vec()), (T_COMPONENT_DEPTH, 8u32.to_be_bytes().to_vec()), (T_H_SUBSAMPLING, 2u32.to_be_bytes().to_vec()), (T_V_SUBSAMPLING, 2u32.to_be_bytes().to_vec()), (T_BLACK_REF, 16u32.to_be_bytes().to_vec()), (T_WHITE_REF, 235u32.to_be_bytes().to_vec()), (T_PICTURE_CODING, mpeg_coding.to_vec()), (0x8000, 200000u32.to_be_bytes().to_vec())])));
        let aes_container = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x02, 0x06, 0x03, 0x00];
        meta.extend(klv(&set_key(0x47), &local(&[(T_INSTANCE, uid(32).to_vec()), (T_LINKED_TRACK, 3u32.to_be_bytes().to_vec()), (T_SAMPLE_RATE, rate25.clone()), (T_CONTAINER, aes_container.to_vec()), (T_LOCKED, vec![1]), (T_AUDIO_RATE, [0, 0, 0xBB, 0x80, 0, 0, 0, 1].to_vec()), (T_CHANNELS, 1u32.to_be_bytes().to_vec()), (T_QUANT_BITS, 16u32.to_be_bytes().to_vec()), (T_BLOCK_ALIGN, vec![0, 2])])));
        // partition pack
        let mut pp = Vec::new();
        pp.extend_from_slice(&[0, 1, 0, 3]);
        pp.extend_from_slice(&512u32.to_be_bytes());
        pp.extend_from_slice(&0u64.to_be_bytes());
        pp.extend_from_slice(&0u64.to_be_bytes());
        let footer_pos_slot = pp.len();
        pp.extend_from_slice(&0u64.to_be_bytes());
        pp.extend_from_slice(&(meta.len() as u64).to_be_bytes());
        pp.extend_from_slice(&0u64.to_be_bytes());
        pp.extend_from_slice(&0u32.to_be_bytes());
        pp.extend_from_slice(&0u64.to_be_bytes());
        pp.extend_from_slice(&0u32.to_be_bytes());
        pp.extend_from_slice(&op);
        pp.extend_from_slice(&refs(&[mpeg_container, aes_container]));
        let mut key = [0u8; 16];
        key[..13].copy_from_slice(&PARTITION_PREFIX);
        key[13] = 0x02;
        key[14] = 0x04;
        let mut f = klv(&key, &pp);
        let pp_len = f.len();
        f.extend_from_slice(&meta);
        // system item + essence elements
        let mut sys = vec![0x5C, 0x04, 0, 0, 0, 0, 0];
        sys.extend_from_slice(&[0; 16]);
        sys.extend_from_slice(&[0; 17]);
        sys.push(0x81);
        sys.extend_from_slice(&[0x05, 0x30, 0x02, 0x10, 0, 0, 0, 0]);
        sys.extend_from_slice(&[0; 8]);
        f.extend(klv(&SYSTEM_ITEM, &sys));
        let mut vkey = [0u8; 16];
        vkey[..12].copy_from_slice(&ESSENCE_PREFIX);
        vkey[12..].copy_from_slice(&0x15010500u32.to_be_bytes());
        f.extend(klv(&vkey, &[0, 0, 1, 0xB3, 0x04, 0x00, 0x30, 0x23, 0xFF, 0xFF, 0xE0, 0x18]));
        let mut akey = [0u8; 16];
        akey[..12].copy_from_slice(&ESSENCE_PREFIX);
        akey[12..].copy_from_slice(&0x16010300u32.to_be_bytes());
        f.extend(klv(&akey, &[0u8; 3840]));
        // footer partition
        let footer_pos = f.len() as u64;
        f[footer_pos_slot + 20..footer_pos_slot + 28].copy_from_slice(&footer_pos.to_be_bytes());
        let mut fkey = key;
        fkey[13] = 0x04;
        let mut fpp = pp.clone();
        fpp[32..40].copy_from_slice(&0u64.to_be_bytes());
        f.extend(klv(&fkey, &fpp));
        let _ = pp_len;
        f
    }

    #[test]
    fn ber_and_probe() {
        assert_eq!(ber(&[0x05], 0), Some((5, 1)));
        assert_eq!(ber(&[0x83, 0x01, 0x00, 0x00], 0), Some((65536, 4)));
        assert_eq!(ber(&[0x89, 0, 0, 0, 0, 0, 0, 0, 0, 0], 0), None);
        assert_eq!(ber(&[0x82, 0x01], 0), None);
        let f = build();
        assert_eq!(probe(&Probe { head: &f, ext: "mxf", size: f.len() as u64 }), 100);
        let mut run_in = vec![0u8; 100];
        run_in.extend_from_slice(&f);
        assert_eq!(probe(&Probe { head: &run_in, ext: "mxf", size: run_in.len() as u64 }), 80);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "mxf", size: 4 }), 0);
    }

    #[test]
    fn header_metadata_and_streams() {
        let f = build();
        let total = f.len() as u64;
        let mut r = Reader::from_bytes(f);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "MXF");
        assert_eq!(g.get("Format_Version"), "1.3");
        assert_eq!(g.get("Format_Profile"), "OP-1a");
        assert_eq!(g.get("Format_Settings"), "Closed / Complete");
        assert_eq!(g.get("Duration"), "1000");
        assert_eq!(g.get("Encoded_Date"), "0-00-00 00:00:00.000");
        assert_eq!(g.get("Encoded_Application/String"), "FFmpeg OP1a Muxer 63.1.101.0.0");
        assert_eq!(g.get("Encoded_Application_Version"), "63.1.101.0.0");
        assert_eq!(g.get("Encoded_Library/String"), "Lavf (linux) 63.1.101.0.0");
        assert_eq!(g.get("Encoded_Library_Name"), "Lavf (linux)");
        assert!(g.get_u64("FooterSize").unwrap() > 0 && g.get_u64("FooterSize").unwrap() < total);
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("ID"), "2");
        assert_eq!(v.get("StreamOrder"), "0");
        assert_eq!(v.get("Format"), "MPEG Video");
        assert_eq!(v.get("CodecID"), "0D01030102046001-0401020201011100");
        assert_eq!(v.get("Format_Settings_Wrapping"), "Frame");
        assert_eq!(v.get("Width"), "64");
        assert_eq!(v.get("Height"), "48");
        assert_eq!(v.get("DisplayAspectRatio"), "1.333");
        assert_eq!(v.get("FrameRate"), "25.000");
        assert_eq!(v.get("ScanType"), "Progressive");
        assert_eq!(v.get("ChromaSubsampling"), "4:2:0");
        assert_eq!(v.get("BitDepth"), "8");
        assert_eq!(v.get("colour_range"), "Limited");
        assert_eq!(v.get("BitRate"), "200000");
        assert_eq!(v.get("Duration"), "1000");
        assert_eq!(v.get("StreamSize"), "25000");
        assert_eq!(v.get("Delay_SDTI"), "36150000");
        let a = &doc.streams[StreamKind::Audio as usize][0];
        assert_eq!(a.get("ID"), "3");
        assert_eq!(a.get("Format"), "PCM");
        assert_eq!(a.get("CodecID"), "0D01030102060300");
        assert_eq!(a.get("Format_Settings_Wrapping"), "Frame (AES)");
        assert_eq!(a.get("SamplingRate"), "48000");
        assert_eq!(a.get("Channel(s)"), "1");
        assert_eq!(a.get("BitDepth"), "16");
        assert_eq!(a.get("BitRate"), "768000");
        assert_eq!(a.get("SamplesPerFrame"), "1920");
        assert_eq!(a.get("StreamSize"), "96000");
        assert_eq!(a.get("Locked"), "Yes");
        assert_eq!(a.get("BlockAlignment"), "2");
        let o = &doc.streams[StreamKind::Other as usize];
        assert_eq!(o.len(), 2);
        assert_eq!(o[0].get("ID"), "1-Material");
        assert_eq!(o[0].get("Format"), "MXF TC");
        assert_eq!(o[0].get("TimeCode_FirstFrame"), "01:00:00:00");
        assert_eq!(o[0].get("TimeCode_Settings"), "Material Package");
        assert_eq!(o[0].get("TimeCode_Striped"), "Yes");
        assert_eq!(o[1].get("Format"), "SMPTE TC");
        assert_eq!(o[1].get("MuxingMode"), "SDTI");
        assert_eq!(o[1].get("TimeCode_FirstFrame"), "10:02:30:05");
    }

    #[test]
    fn helpers() {
        assert_eq!(timecode_string(90000, 25, false), "01:00:00:00");
        assert_eq!(timecode_string(1799, 30, true), "00:00:59;29");
        assert_eq!(smpte12m(&[0x45, 0x30, 0x02, 0x10, 0, 0, 0, 0]).0, "10:02:30;05");
        assert_eq!(version_string(&[0, 1, 0, 2, 0, 3, 0, 4, 0, 5]), "1.2.3.4.5");
        assert_eq!(mxf_timestamp(&[0x07, 0xE4, 3, 4, 5, 6, 7, 250]), "2020-03-04 05:06:07.1000");
        let op = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x10, 0x00, 0x00, 0x00];
        assert_eq!(op_name(&op), "OP-Atom");
        let avc = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0A, 0x04, 0x01, 0x02, 0x02, 0x01, 0x31, 0x11, 0x01];
        assert_eq!(coding_format(&avc), Some("AVC"));
        let ac3 = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x04, 0x02, 0x02, 0x02, 0x03, 0x02, 0x01, 0x00];
        assert_eq!(coding_format(&ac3), Some("AC-3"));
        let dv = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x02, 0x02, 0x41, 0x02];
        assert_eq!(container_info(&dv), ("DV", "Clip".to_string()));
    }

    #[test]
    fn truncated_and_garbage() {
        let mut f = build();
        f.truncate(300);
        let mut r = Reader::from_bytes(f);
        assert!(parse(&mut r, &mut Doc::new()));
        let mut r = Reader::from_bytes(vec![0x06, 0x0E, 0x2B, 0x34]);
        assert!(!parse(&mut r, &mut Doc::new()));
    }
}

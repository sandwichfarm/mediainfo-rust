//! ASF / Windows Media (Microsoft Advanced Systems Format): header objects (file/stream properties,
//! header extension, codec list, content descriptions, bitrate properties), data packets.

use crate::io::{le16, le32, le64, utf16, Reader};
use crate::model::{Doc, Stream, StreamKind, OPT_SHOWN};
use crate::parsers::audio::wma;
use crate::parsers::video::{fourcc, vc1};
use crate::parsers::Probe;

const MAX_OBJECTS: usize = 4096;
const MAX_STREAMS: usize = 128;
/// Bytes of the data object walked for timestamps and sizes.
const DATA_SCAN: u64 = 8 * 1024 * 1024;
const MAX_PACKETS: u64 = 200_000;

// GUIDs as stored (little-endian first three fields)
const HEADER: [u8; 16] = [0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C];
const DATA: [u8; 16] = [0x36, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C];
const FILE_PROPERTIES: [u8; 16] = [0xA1, 0xDC, 0xAB, 0x8C, 0x47, 0xA9, 0xCF, 0x11, 0x8E, 0xE4, 0x00, 0xC0, 0x0C, 0x20, 0x53, 0x65];
const STREAM_PROPERTIES: [u8; 16] = [0x91, 0x07, 0xDC, 0xB7, 0xB7, 0xA9, 0xCF, 0x11, 0x8E, 0xE6, 0x00, 0xC0, 0x0C, 0x20, 0x53, 0x65];
const HEADER_EXTENSION: [u8; 16] = [0xB5, 0x03, 0xBF, 0x5F, 0x2E, 0xA9, 0xCF, 0x11, 0x8E, 0xE3, 0x00, 0xC0, 0x0C, 0x20, 0x53, 0x65];
const CODEC_LIST: [u8; 16] = [0x40, 0x52, 0xD1, 0x86, 0x1D, 0x31, 0xD0, 0x11, 0xA3, 0xA4, 0x00, 0xA0, 0xC9, 0x03, 0x48, 0xF6];
const CONTENT_DESCRIPTION: [u8; 16] = [0x33, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C];
const EXTENDED_CONTENT_DESCRIPTION: [u8; 16] = [0x40, 0xA4, 0xD0, 0xD2, 0x07, 0xE3, 0xD2, 0x11, 0x97, 0xF0, 0x00, 0xA0, 0xC9, 0x5E, 0xA8, 0x50];
const STREAM_BITRATE_PROPERTIES: [u8; 16] = [0xCE, 0x75, 0xF8, 0x7B, 0x8D, 0x46, 0xD1, 0x11, 0x8D, 0x82, 0x00, 0x60, 0x97, 0xC9, 0xA2, 0xB2];
const EXTENDED_STREAM_PROPERTIES: [u8; 16] = [0xCB, 0xA5, 0xE6, 0x14, 0x72, 0xC6, 0x32, 0x43, 0x83, 0x99, 0xA9, 0x69, 0x52, 0x06, 0x5B, 0x5A];
const LANGUAGE_LIST: [u8; 16] = [0xA9, 0x46, 0x43, 0x7C, 0xE0, 0xEF, 0xFC, 0x4B, 0xB2, 0x29, 0x39, 0x3C, 0xBE, 0x3E, 0x89, 0xF8];
const METADATA: [u8; 16] = [0xEA, 0xCB, 0xF8, 0xC5, 0xAF, 0x5B, 0x77, 0x48, 0x84, 0x67, 0xAA, 0x8C, 0x44, 0xFA, 0x4C, 0xCA];
const METADATA_LIBRARY: [u8; 16] = [0x94, 0x1C, 0x23, 0x44, 0x98, 0x94, 0xD1, 0x49, 0xA1, 0x41, 0x1D, 0x13, 0x4E, 0x45, 0x70, 0x54];
const AUDIO_MEDIA: [u8; 16] = [0x40, 0x9E, 0x69, 0xF8, 0x4D, 0x5B, 0xCF, 0x11, 0xA8, 0xFD, 0x00, 0x80, 0x5F, 0x5C, 0x44, 0x2B];
const VIDEO_MEDIA: [u8; 16] = [0xC0, 0xEF, 0x19, 0xBC, 0x4D, 0x5B, 0xCF, 0x11, 0xA8, 0xFD, 0x00, 0x80, 0x5F, 0x5C, 0x44, 0x2B];
const COMMAND_MEDIA: [u8; 16] = [0xC0, 0xCF, 0xDA, 0x59, 0xE6, 0x59, 0xD0, 0x11, 0xA3, 0xAC, 0x00, 0xA0, 0xC9, 0x03, 0x48, 0xF6];
const JFIF_MEDIA: [u8; 16] = [0x00, 0xE1, 0x1B, 0xB6, 0x4E, 0x5B, 0xCF, 0x11, 0xA8, 0xFD, 0x00, 0x80, 0x5F, 0x5C, 0x44, 0x2B];
const BINARY_MEDIA: [u8; 16] = [0xE2, 0x65, 0xFB, 0x3A, 0x39, 0xEF, 0x47, 0x43, 0x9E, 0x31, 0x2D, 0x32, 0x40, 0x9C, 0xE8, 0x51];

pub fn probe(p: &Probe) -> u8 {
    if p.starts_with(&HEADER) && p.head.len() >= 30 {
        100
    } else {
        0
    }
}

// ---------------------------------------------------------------------------- model

#[derive(Debug, Clone)]
struct AsfStream {
    number: u8,
    kind: StreamKind,
    type_specific: Vec<u8>,
    time_offset: u64,
    encrypted: bool,
    // extended properties
    start_time: u64,
    end_time: u64,
    data_bitrate: u32,
    avg_time_per_frame: u64,
    language_index: Option<u16>,
    names: Vec<String>,
    bitrate_prop: Option<u32>,
    aspect: (Option<u32>, Option<u32>),
    // data packets
    first_pts: Option<u32>,
    last_pts: Option<u32>,
    objects: u64,
    bytes: u64,
    pts_list: Vec<u32>,
}

impl Default for AsfStream {
    fn default() -> Self {
        Self {
            number: 0,
            kind: StreamKind::General,
            type_specific: Vec::new(),
            time_offset: 0,
            encrypted: false,
            start_time: 0,
            end_time: 0,
            data_bitrate: 0,
            avg_time_per_frame: 0,
            language_index: None,
            names: Vec::new(),
            bitrate_prop: None,
            aspect: (None, None),
            first_pts: None,
            last_pts: None,
            objects: 0,
            bytes: 0,
            pts_list: Vec::new(),
        }
    }
}

#[derive(Debug, Default)]
struct Ctx {
    header_size: u64,
    data_size: u64,
    data_pos: u64,
    creation: u64,
    play_duration: u64,
    send_duration: u64,
    preroll: u64,
    flags: u32,
    min_packet: u32,
    max_packet: u32,
    max_bitrate: u32,
    packet_count: u64,
    streams: Vec<AsfStream>,
    languages: Vec<String>,
    codecs: Vec<(u16, String, Vec<u8>)>,
    content: Vec<(String, String)>,
    extended: Vec<(String, String)>,
    /// (stream number, AspectRatioX / AspectRatioY) from the metadata object
    aspects: Vec<(u8, bool, u32)>,
    /// (stream number, average bit rate) from the stream bitrate properties object
    bitrates: Vec<(u8, u32)>,
    complete: bool,
}

impl Ctx {
    fn stream_mut(&mut self, number: u8) -> Option<&mut AsfStream> {
        let i = self.streams.iter().position(|s| s.number == number)?;
        self.streams.get_mut(i)
    }
}

fn guid_name(g: &[u8]) -> Option<StreamKind> {
    if g == AUDIO_MEDIA {
        Some(StreamKind::Audio)
    } else if g == VIDEO_MEDIA || g == JFIF_MEDIA {
        Some(StreamKind::Video)
    } else if g == COMMAND_MEDIA || g == BINARY_MEDIA {
        Some(StreamKind::General)
    } else {
        None
    }
}

fn wstr(b: &[u8]) -> String {
    crate::io::clean_text(&utf16(b, false))
}

// ---------------------------------------------------------------------------- header objects

fn parse_file_properties(b: &[u8], ctx: &mut Ctx) {
    if b.len() < 80 {
        return;
    }
    ctx.creation = le64(b, 24).unwrap_or(0);
    ctx.packet_count = le64(b, 32).unwrap_or(0);
    ctx.play_duration = le64(b, 40).unwrap_or(0);
    ctx.send_duration = le64(b, 48).unwrap_or(0);
    ctx.preroll = le64(b, 56).unwrap_or(0);
    ctx.flags = le32(b, 64).unwrap_or(0);
    ctx.min_packet = le32(b, 68).unwrap_or(0);
    ctx.max_packet = le32(b, 72).unwrap_or(0);
    ctx.max_bitrate = le32(b, 76).unwrap_or(0);
}

fn parse_stream_properties(b: &[u8], ctx: &mut Ctx) {
    if b.len() < 54 {
        return;
    }
    let kind = guid_name(&b[..16]);
    let time_offset = le64(b, 32).unwrap_or(0);
    let ts_len = le32(b, 40).unwrap_or(0) as usize;
    let flags = le16(b, 48).unwrap_or(0);
    let number = (flags & 0x7F) as u8;
    let type_specific = b.get(54..54 + ts_len).unwrap_or(&[]).to_vec();
    let Some(kind) = kind else { return };
    if ctx.streams.iter().any(|s| s.number == number) {
        if let Some(s) = ctx.stream_mut(number) {
            if s.type_specific.is_empty() {
                s.type_specific = type_specific;
                s.kind = kind;
            }
        }
        return;
    }
    if ctx.streams.len() >= MAX_STREAMS {
        return;
    }
    ctx.streams.push(AsfStream { number, kind, type_specific, time_offset, encrypted: flags & 0x8000 != 0, ..Default::default() });
}

fn parse_extended_stream_properties(b: &[u8], ctx: &mut Ctx) {
    if b.len() < 64 {
        return;
    }
    let number = le16(b, 48).unwrap_or(0) as u8;
    let start_time = le64(b, 0).unwrap_or(0);
    let end_time = le64(b, 8).unwrap_or(0);
    let data_bitrate = le32(b, 16).unwrap_or(0);
    let language_index = le16(b, 50).unwrap_or(0);
    let avg_time_per_frame = le64(b, 52).unwrap_or(0);
    let name_count = le16(b, 60).unwrap_or(0) as usize;
    let ext_count = le16(b, 62).unwrap_or(0) as usize;
    let mut p = 64;
    let mut names = Vec::new();
    for _ in 0..name_count.min(64) {
        let Some(len) = le16(b, p + 2) else { break };
        let len = len as usize;
        names.push(wstr(b.get(p + 4..p + 4 + len).unwrap_or(&[])));
        p += 4 + len;
    }
    for _ in 0..ext_count.min(64) {
        let Some(len) = le32(b, p + 18) else { break };
        p += 22 + len as usize;
    }
    // Optional embedded stream properties object
    if b.len() >= p + 24 && b[p..p + 16] == STREAM_PROPERTIES {
        let size = le64(b, p + 16).unwrap_or(0) as usize;
        if let Some(inner) = b.get(p + 24..(p + size).min(b.len())) {
            parse_stream_properties(inner, ctx);
        }
    }
    let existing = ctx.streams.iter().any(|s| s.number == number);
    if !existing {
        if ctx.streams.len() >= MAX_STREAMS {
            return;
        }
        ctx.streams.push(AsfStream { number, kind: StreamKind::General, ..Default::default() });
    }
    if let Some(s) = ctx.stream_mut(number) {
        s.start_time = start_time;
        s.end_time = end_time;
        s.data_bitrate = data_bitrate;
        s.avg_time_per_frame = avg_time_per_frame;
        s.language_index = Some(language_index);
        s.names = names;
    }
}

fn parse_language_list(b: &[u8], ctx: &mut Ctx) {
    let count = le16(b, 0).unwrap_or(0) as usize;
    let mut p = 2;
    for _ in 0..count.min(256) {
        let Some(&len) = b.get(p) else { break };
        let len = len as usize;
        ctx.languages.push(wstr(b.get(p + 1..p + 1 + len).unwrap_or(&[])));
        p += 1 + len;
    }
}

fn parse_metadata(b: &[u8], ctx: &mut Ctx) {
    let count = le16(b, 0).unwrap_or(0) as usize;
    let mut p = 2;
    for _ in 0..count.min(4096) {
        let (Some(stream), Some(name_len), Some(data_type), Some(data_len)) = (le16(b, p + 2), le16(b, p + 4), le16(b, p + 6), le32(b, p + 8)) else { break };
        let name_len = name_len as usize;
        let data_len = data_len as usize;
        let name = wstr(b.get(p + 12..p + 12 + name_len).unwrap_or(&[]));
        let data = b.get(p + 12 + name_len..p + 12 + name_len + data_len).unwrap_or(&[]);
        let value_num: Option<u32> = match data_type {
            3 => le32(data, 0),
            5 => le16(data, 0).map(|v| v as u32),
            4 => le64(data, 0).map(|v| v as u32),
            2 => le16(data, 0).map(|v| v as u32).or_else(|| le32(data, 0)),
            _ => None,
        };
        let number = stream as u8;
        match (name.as_str(), value_num) {
            ("AspectRatioX", Some(v)) => ctx.aspects.push((number, true, v)),
            ("AspectRatioY", Some(v)) => ctx.aspects.push((number, false, v)),
            _ => {
                if data_type == 0 && stream == 0 {
                    let v = wstr(data);
                    if !v.is_empty() {
                        ctx.extended.push((name, v));
                    }
                }
            }
        }
        p += 12 + name_len + data_len;
    }
}

fn parse_codec_list(b: &[u8], ctx: &mut Ctx) {
    let count = le32(b, 16).unwrap_or(0) as usize;
    let mut p = 20;
    for _ in 0..count.min(256) {
        let (Some(kind), Some(name_len)) = (le16(b, p), le16(b, p + 2)) else { break };
        let name_len = name_len as usize * 2;
        let name = wstr(b.get(p + 4..p + 4 + name_len).unwrap_or(&[]));
        p += 4 + name_len;
        let Some(desc_len) = le16(b, p) else { break };
        p += 2 + desc_len as usize * 2;
        let Some(info_len) = le16(b, p) else { break };
        let info = b.get(p + 2..p + 2 + info_len as usize).unwrap_or(&[]).to_vec();
        p += 2 + info_len as usize;
        ctx.codecs.push((kind, name, info));
    }
}

fn parse_content_description(b: &[u8], ctx: &mut Ctx) {
    let mut lens = [0usize; 5];
    for (i, l) in lens.iter_mut().enumerate() {
        *l = le16(b, i * 2).unwrap_or(0) as usize;
    }
    let mut p = 10;
    let names = ["Title", "Performer", "Copyright", "Description", "Rating"];
    for (i, name) in names.iter().enumerate() {
        let v = wstr(b.get(p..p + lens[i]).unwrap_or(&[]));
        p += lens[i];
        if !v.is_empty() {
            ctx.content.push((name.to_string(), v));
        }
    }
}

fn parse_extended_content_description(b: &[u8], ctx: &mut Ctx) {
    let count = le16(b, 0).unwrap_or(0) as usize;
    let mut p = 2;
    for _ in 0..count.min(4096) {
        let Some(name_len) = le16(b, p) else { break };
        let name_len = name_len as usize;
        let name = wstr(b.get(p + 2..p + 2 + name_len).unwrap_or(&[]));
        p += 2 + name_len;
        let (Some(data_type), Some(len)) = (le16(b, p), le16(b, p + 2)) else { break };
        let len = len as usize;
        let data = b.get(p + 4..p + 4 + len).unwrap_or(&[]);
        p += 4 + len;
        let value = match data_type {
            0 => wstr(data),
            2 => match le32(data, 0).or_else(|| le16(data, 0).map(|v| v as u32)) {
                Some(0) => "No".to_string(),
                Some(_) => "Yes".to_string(),
                None => String::new(),
            },
            3 => le32(data, 0).map(|v| v.to_string()).unwrap_or_default(),
            4 => le64(data, 0).map(|v| v.to_string()).unwrap_or_default(),
            5 => le16(data, 0).map(|v| v.to_string()).unwrap_or_default(),
            _ => String::new(),
        };
        if !name.is_empty() && !value.is_empty() {
            ctx.extended.push((name, value));
        }
    }
}

fn parse_stream_bitrate_properties(b: &[u8], ctx: &mut Ctx) {
    let count = le16(b, 0).unwrap_or(0) as usize;
    for i in 0..count.min(MAX_STREAMS) {
        let p = 2 + i * 6;
        let (Some(flags), Some(rate)) = (le16(b, p), le32(b, p + 2)) else { break };
        let number = (flags & 0x7F) as u8;
        ctx.bitrates.push((number, rate));
    }
}

/// Walk the objects of the header (or header extension) region.
fn parse_objects(r: &mut Reader, start: u64, end: u64, ctx: &mut Ctx, depth: u32) {
    let mut pos = start;
    let mut n = 0;
    while pos + 24 <= end && n < MAX_OBJECTS {
        n += 1;
        let hdr = r.read_vec_at(pos, 24);
        if hdr.len() < 24 {
            break;
        }
        let guid = &hdr[..16];
        let size = le64(&hdr, 16).unwrap_or(0);
        if size < 24 {
            break;
        }
        let body_len = (size - 24).min(end - pos - 24).min(16 << 20) as usize;
        if guid == HEADER_EXTENSION {
            if depth < 2 {
                parse_objects(r, pos + 24 + 22, (pos + size).min(end), ctx, depth + 1);
            }
        } else {
            let body = r.read_vec_at(pos + 24, body_len);
            if guid == FILE_PROPERTIES {
                parse_file_properties(&body, ctx);
            } else if guid == STREAM_PROPERTIES {
                parse_stream_properties(&body, ctx);
            } else if guid == EXTENDED_STREAM_PROPERTIES {
                parse_extended_stream_properties(&body, ctx);
            } else if guid == LANGUAGE_LIST {
                parse_language_list(&body, ctx);
            } else if guid == METADATA || guid == METADATA_LIBRARY {
                parse_metadata(&body, ctx);
            } else if guid == CODEC_LIST {
                parse_codec_list(&body, ctx);
            } else if guid == CONTENT_DESCRIPTION {
                parse_content_description(&body, ctx);
            } else if guid == EXTENDED_CONTENT_DESCRIPTION {
                parse_extended_content_description(&body, ctx);
            } else if guid == STREAM_BITRATE_PROPERTIES {
                parse_stream_bitrate_properties(&body, ctx);
            }
        }
        pos += size;
    }
}

// ---------------------------------------------------------------------------- data packets

fn read_len(b: &[u8], p: &mut usize, ty: u8) -> Option<u32> {
    let v = match ty {
        0 => 0,
        1 => *b.get(*p)? as u32,
        2 => le16(b, *p)? as u32,
        _ => le32(b, *p)?,
    };
    *p += match ty {
        0 => 0,
        1 => 1,
        2 => 2,
        _ => 4,
    };
    Some(v)
}

/// Parse one data packet; returns its length.
fn parse_packet(b: &[u8], fixed_size: usize, ctx: &mut Ctx) -> Option<usize> {
    let mut p = 0;
    let first = *b.first()?;
    if first & 0x80 != 0 {
        // error correction data
        let len = (first & 0x0F) as usize;
        if first & 0x60 != 0 {
            return None;
        }
        p += 1 + len;
    }
    let lt = *b.get(p)?;
    let pf = *b.get(p + 1)?;
    p += 2;
    let multiple = lt & 1 != 0;
    let seq_t = (lt >> 1) & 3;
    let pad_t = (lt >> 3) & 3;
    let plen_t = (lt >> 5) & 3;
    let rep_t = pf & 3;
    let off_t = (pf >> 2) & 3;
    let mon_t = (pf >> 4) & 3;
    let _sn_t = (pf >> 6) & 3; // stream number is always one byte
    let packet_len = read_len(b, &mut p, plen_t)? as usize;
    let _seq = read_len(b, &mut p, seq_t)?;
    let padding = read_len(b, &mut p, pad_t)? as usize;
    let _send_time = le32(b, p)?;
    p += 6;
    let total = if packet_len > 0 { packet_len } else { fixed_size };
    if total == 0 || total > b.len() {
        return None;
    }
    let (count, pl_t) = if multiple {
        let f = *b.get(p)?;
        p += 1;
        ((f & 0x3F) as usize, (f >> 6) & 3)
    } else {
        (1, 0)
    };
    for _ in 0..count {
        let sn = *b.get(p)?;
        p += 1;
        let _mon = read_len(b, &mut p, mon_t)?;
        let offset = read_len(b, &mut p, off_t)?;
        let rep = read_len(b, &mut p, rep_t)? as usize;
        let mut pts: Option<u32> = None;
        let mut compressed = false;
        if rep == 1 {
            compressed = true;
            pts = Some(offset);
            p += 1;
        } else {
            if rep >= 8 {
                pts = le32(b, p + 4);
            }
            p += rep;
        }
        let len = if multiple { read_len(b, &mut p, pl_t)? as usize } else { total.saturating_sub(p + padding) };
        let number = sn & 0x7F;
        if let Some(s) = ctx.stream_mut(number) {
            if compressed {
                // sub-payloads: (len byte, data)*
                let mut q = p;
                let end = (p + len).min(b.len());
                let mut k = 0;
                while q < end && k < 256 {
                    let l = b[q] as usize;
                    q += 1 + l;
                    s.objects += 1;
                    s.bytes += l as u64;
                    k += 1;
                }
                if let Some(t) = pts {
                    if s.first_pts.is_none() {
                        s.first_pts = Some(t);
                    }
                    s.last_pts = Some(t);
                }
            } else {
                if offset == 0 {
                    s.objects += 1;
                    if let Some(t) = pts {
                        if s.first_pts.is_none() {
                            s.first_pts = Some(t);
                        }
                        s.last_pts = Some(t);
                        if s.pts_list.len() < 128 {
                            s.pts_list.push(t);
                        }
                    }
                }
                s.bytes += len as u64;
            }
        }
        p += len;
        if p > total {
            return None;
        }
    }
    Some(total)
}

fn scan_data(r: &mut Reader, ctx: &mut Ctx) {
    if ctx.data_pos == 0 || ctx.data_size < 50 {
        return;
    }
    let start = ctx.data_pos + 50;
    let end = (ctx.data_pos + ctx.data_size).min(r.len());
    let limit = (start + DATA_SCAN).min(end);
    let fixed = ctx.min_packet as usize;
    let mut pos = start;
    let mut n = 0u64;
    while pos < limit && n < MAX_PACKETS {
        n += 1;
        let want = if fixed > 0 { fixed.max(64) } else { 64 * 1024 }.min((end - pos) as usize);
        let buf = r.read_vec_at(pos, want);
        match parse_packet(&buf, fixed, ctx) {
            Some(len) if len > 0 => pos += len as u64,
            _ => {
                if fixed > 0 {
                    pos += fixed as u64;
                } else {
                    break;
                }
            }
        }
    }
    ctx.complete = pos >= end;
}

// ---------------------------------------------------------------------------- parse

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let hdr = r.read_vec_at(0, 30);
    if hdr.len() < 30 || hdr[..16] != HEADER {
        return false;
    }
    let mut ctx = Ctx::default();
    ctx.header_size = le64(&hdr, 16).unwrap_or(0);
    if ctx.header_size < 30 {
        return false;
    }
    let header_end = ctx.header_size.min(r.len());
    parse_objects(r, 30, header_end, &mut ctx, 0);
    // Top-level objects after the header: data, indexes
    let mut pos = ctx.header_size;
    let mut n = 0;
    while pos + 24 <= r.len() && n < MAX_OBJECTS {
        n += 1;
        let h = r.read_vec_at(pos, 24);
        if h.len() < 24 {
            break;
        }
        let size = le64(&h, 16).unwrap_or(0);
        if h[..16] == DATA {
            ctx.data_pos = pos;
            ctx.data_size = size;
        }
        if size < 24 {
            break;
        }
        pos += size;
    }
    scan_data(r, &mut ctx);
    for (number, is_x, v) in std::mem::take(&mut ctx.aspects) {
        if let Some(s) = ctx.stream_mut(number) {
            if is_x {
                s.aspect.0 = Some(v);
            } else {
                s.aspect.1 = Some(v);
            }
        }
    }
    for (number, rate) in std::mem::take(&mut ctx.bitrates) {
        if let Some(s) = ctx.stream_mut(number) {
            s.bitrate_prop = Some(rate);
        }
    }
    emit(doc, &ctx, r.len());
    true
}

// ---------------------------------------------------------------------------- emit

fn filetime_string(ft: u64) -> String {
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    let unix100 = ft.saturating_sub(EPOCH_DIFF);
    let secs = (unix100 / 10_000_000) as i64;
    let ms = (unix100 / 10_000) % 1000;
    format!("UTC {}.{ms:03}", crate::finish::format_datetime(secs))
}

fn general_tag(g: &mut Stream, name: &str, value: &str) {
    let field = match name {
        "WM/AlbumTitle" => "Album",
        "WM/AlbumArtist" => "Album/Performer",
        "WM/Genre" => "Genre",
        "WM/Year" => "Recorded_Date",
        "WM/TrackNumber" => "Track/Position",
        "WM/Composer" => "Composer",
        "WM/Publisher" => "Publisher",
        "WM/EncodedBy" => "EncodedBy",
        "WM/ToolName" => "Encoded_Application",
        "WM/Lyrics" => "Lyrics",
        "WM/Mood" => "Mood",
        "WM/Conductor" => "Conductor",
        "WM/Writer" => "WrittenBy",
        "WM/Producer" => "Producer",
        "WM/Director" => "Director",
        "WM/OriginalAlbumTitle" => "Original/Album",
        "WM/OriginalArtist" => "Original/Performer",
        "WM/OriginalLyricist" => "Original/Lyricist",
        "WM/OriginalReleaseYear" => "Original/Released_Date",
        "WM/SubTitle" => "Title_More",
        "WM/BeatsPerMinute" => "BPM",
        "WM/Language" => "Language",
        "WM/Period" => "Period",
        "WM/ISRC" => "ISRC",
        "WM/Barcode" => "BarCode",
        "WM/Text" => "Comment",
        "WM/ParentalRating" => "LawRating",
        "WM/ContentGroupDescription" => "Grouping",
        "WM/PartOfSet" => "Part/Position",
        "WM/ProviderCopyright" => "Copyright",
        "Author" => "Performer",
        "Title" => "Title",
        "Copyright" => "Copyright",
        "Description" => "Description",
        "Rating" => "Rating",
        "WM/Track" => {
            if let Ok(v) = value.parse::<u64>() {
                g.set_if_empty("Track/Position", (v + 1).to_string());
            }
            return;
        }
        "WM/EncodingTime" => {
            if let Ok(v) = value.parse::<u64>() {
                g.set("Encoded_Date", filetime_string(v));
            }
            return;
        }
        _ => {
            const SKIP: &[&str] = &["WMFSDKVersion", "WMFSDKNeeded", "IsVBR", "ASFLeakyBucketPairs", "Buffer Average", "VBR Peak", "WM/UniqueFileIdentifier", "WM/WMContentID", "WM/WMCollectionID", "WM/WMCollectionGroupID", "WM/MediaClassPrimaryID", "WM/MediaClassSecondaryID", "WM/Picture", "DeviceConformanceTemplate", "WM/MCDI", "WM/Provider", "WM/ProviderStyle", "WM/ProviderRating"];
            if !SKIP.contains(&name) {
                g.set_extra(name, value, "", OPT_SHOWN);
            }
            return;
        }
    };
    g.set_if_empty(field, value);
}

/// Median interval of media object presentation times → frames per second.
fn estimate_frame_rate(pts: &[u32]) -> Option<f64> {
    let mut deltas: Vec<u32> = pts.windows(2).filter(|w| w[1] > w[0]).map(|w| w[1] - w[0]).collect();
    if deltas.len() < 2 {
        return None;
    }
    deltas.sort_unstable();
    let median = deltas[deltas.len() / 2];
    if median == 0 {
        return None;
    }
    Some(1000.0 / median as f64)
}

fn emit(doc: &mut Doc, ctx: &Ctx, file_size: u64) {
    let g = doc.general();
    g.set("Format", "Windows Media");
    let duration_ms: Option<u64> = if ctx.play_duration > 0 { Some((ctx.play_duration / 10_000).saturating_sub(ctx.preroll)) } else { None };
    if let Some(d) = duration_ms {
        g.set_int("Duration", d as i128);
    }
    if ctx.max_bitrate > 0 {
        g.set_int("OverallBitRate_Maximum", ctx.max_bitrate as i128);
    }
    if ctx.header_size > 0 {
        g.set_int("HeaderSize", ctx.header_size as i128);
    }
    if ctx.data_size > 0 {
        g.set_int("DataSize", ctx.data_size as i128);
    }
    if ctx.flags & 1 != 0 {
        g.set("IsStreamable", "Yes");
    }
    g.set("Encoded_Date", filetime_string(ctx.creation));
    for (k, v) in &ctx.content {
        general_tag(g, k, v);
    }
    for (k, v) in &ctx.extended {
        general_tag(g, k, v);
    }
    let _ = file_size;

    let mut streams: Vec<&AsfStream> = ctx.streams.iter().filter(|s| s.kind != StreamKind::General).collect();
    streams.sort_by_key(|s| s.number);
    let mut codec_used = vec![false; ctx.codecs.len()];
    for (order, st) in streams.iter().enumerate() {
        let kind = st.kind;
        let mut s = Stream::new(kind);
        s.set_int("StreamOrder", order as i128);
        s.set_int("ID", st.number as i128);
        let ts = st.type_specific.as_slice();
        let mut codec_key: Vec<u8> = Vec::new();
        match kind {
            StreamKind::Video => {
                // encoded width(4) height(4) reserved(1) format data size(2) BITMAPINFOHEADER
                let w = le32(ts, 0).unwrap_or(0);
                let h = le32(ts, 4).unwrap_or(0);
                let bih = ts.get(11..).unwrap_or(&[]);
                if bih.len() >= 40 {
                    fourcc::apply_bitmapinfoheader(&mut s, bih);
                    // Compressed video in ASF: 8-bit components, lossy.
                    if !s.has("BitDepth") && !matches!(s.get("Format"), "RGB" | "YUV") {
                        s.set("BitDepth", "8");
                        if s.get("Format") == "MPEG-4 Visual" {
                            s.set_if_empty("Compression_Mode", "Lossy");
                        }
                    }
                    codec_key = bih[16..20].to_vec();
                    let cc = String::from_utf8_lossy(&bih[16..20]).to_string();
                    if !s.has("CodecID") && cc.chars().all(|c| c.is_ascii_graphic()) {
                        s.set("CodecID", cc.clone());
                    }
                    if !s.has("Format") {
                        if let Some((f, _, _)) = fourcc::fourcc_format(&cc) {
                            s.set("Format", f);
                        } else if cc.chars().all(|c| c.is_ascii_graphic()) {
                            s.set("Format", cc.clone());
                        }
                    }
                    if matches!(cc.as_str(), "WMV3" | "WVC1" | "WMVA" | "WVP2") {
                        let bih_size = le32(bih, 0).unwrap_or(40) as usize;
                        if let Some(extra) = bih.get(bih_size.max(40)..) {
                            if !extra.is_empty() {
                                vc1::apply_sequence(&mut s, extra);
                            }
                        }
                    }
                }
                if w > 0 && h > 0 {
                    s.set_if_empty("Width", w.to_string());
                    s.set_if_empty("Height", h.to_string());
                }
                if let (Some(x), Some(y)) = st.aspect {
                    if x > 0 && y > 0 && x != y {
                        s.set_if_empty("PixelAspectRatio", format!("{:.3}", x as f64 / y as f64));
                    }
                }
                let fps = if st.avg_time_per_frame > 0 { Some(10_000_000.0 / st.avg_time_per_frame as f64) } else { estimate_frame_rate(&st.pts_list) };
                if let Some(f) = fps {
                    s.set("FrameRate", format!("{f:.3}"));
                }
            }
            StreamKind::Audio => {
                wma::apply_waveformatex(&mut s, ts);
                if let Some(tag) = le16(ts, 0) {
                    codec_key = tag.to_le_bytes().to_vec();
                    s.set_if_empty("CodecID", format!("{tag:X}"));
                }
            }
            _ => {}
        }
        // Codec list: description by matching codec information (fourcc / format tag), else by order.
        let mut desc: Option<&str> = None;
        for (i, (ct, name, info)) in ctx.codecs.iter().enumerate() {
            let same_kind = (*ct == 1 && kind == StreamKind::Video) || (*ct == 2 && kind == StreamKind::Audio);
            if same_kind && !codec_used[i] && (!codec_key.is_empty() && info.as_slice() == codec_key.as_slice()) {
                desc = Some(name);
                codec_used[i] = true;
                break;
            }
        }
        if desc.is_none() {
            for (i, (ct, name, _)) in ctx.codecs.iter().enumerate() {
                let same_kind = (*ct == 1 && kind == StreamKind::Video) || (*ct == 2 && kind == StreamKind::Audio);
                if same_kind && !codec_used[i] {
                    desc = Some(name);
                    codec_used[i] = true;
                    break;
                }
            }
        }
        if let Some(d) = desc {
            if !d.is_empty() {
                s.set("CodecID_Description", d);
            }
        }
        // Bit rate: stream bitrate properties > extended stream properties > codec header
        if let Some(b) = st.bitrate_prop.filter(|b| *b > 0) {
            s.set_int("BitRate", b as i128);
        } else if st.data_bitrate > 0 {
            s.set_if_empty("BitRate", st.data_bitrate.to_string());
        }
        // Duration: from the file play duration
        let stream_dur = if st.end_time > st.start_time { Some((st.end_time - st.start_time) / 10_000) } else { duration_ms };
        if let Some(d) = stream_dur {
            let mut dur_ms = d as f64;
            if kind == StreamKind::Video {
                if let Some(f) = s.get_f64("FrameRate").filter(|f| *f > 0.0) {
                    let frames = (dur_ms / 1000.0 * f).round();
                    dur_ms = (frames / f * 1000.0).round();
                    s.set("Duration", format!("{}", dur_ms as i64));
                    s.set_int("FrameCount", frames as i128);
                } else {
                    s.set("Duration", format!("{}", dur_ms as i64));
                }
            } else {
                s.set("Duration", format!("{}", dur_ms as i64));
            }
            if let Some(br) = s.get_f64("BitRate").filter(|b| *b > 0.0) {
                s.set("StreamSize", format!("{}", (br * dur_ms / 8000.0).round() as i64));
            } else if ctx.complete && st.bytes > 0 {
                s.set_int("StreamSize", st.bytes as i128);
            }
            if kind == StreamKind::Video {
                s.set_extra("FrameCount_Source", "General_Duration", "", "N NTN");
            } else {
                s.set_extra("SamplingCount_Source", "General_Duration", "", "N NTN");
            }
            s.set_extra("Duration_Source", "General_Duration", "", "N NTN");
        } else if ctx.complete && st.bytes > 0 {
            s.set_int("StreamSize", st.bytes as i128);
        }
        if let Some(li) = st.language_index {
            if let Some(l) = ctx.languages.get(li as usize) {
                if !l.is_empty() {
                    s.set("Language", l.clone());
                }
            }
        }
        if let Some(n) = st.names.first() {
            if !n.is_empty() {
                s.set("Title", n.clone());
            }
        }
        if st.encrypted {
            s.set("Encryption", "Encrypted");
        }
        if st.time_offset > 0 {
            s.set("Delay", format!("{}", st.time_offset / 10_000));
            s.set("Delay_Source", "Container");
        }
        doc.streams[kind as usize].push(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(guid: &[u8; 16], body: &[u8]) -> Vec<u8> {
        let mut v = guid.to_vec();
        v.extend_from_slice(&((body.len() + 24) as u64).to_le_bytes());
        v.extend_from_slice(body);
        v
    }

    fn wide(s: &str) -> Vec<u8> {
        let mut v: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        v.extend_from_slice(&[0, 0]);
        v
    }

    fn build() -> Vec<u8> {
        let mut fp = vec![0u8; 16];
        fp.extend_from_slice(&10499u64.to_le_bytes());
        fp.extend_from_slice(&116444736000000000u64.to_le_bytes());
        fp.extend_from_slice(&3u64.to_le_bytes());
        fp.extend_from_slice(&41430000u64.to_le_bytes());
        fp.extend_from_slice(&10430000u64.to_le_bytes());
        fp.extend_from_slice(&3100u64.to_le_bytes());
        fp.extend_from_slice(&2u32.to_le_bytes());
        fp.extend_from_slice(&3200u32.to_le_bytes());
        fp.extend_from_slice(&3200u32.to_le_bytes());
        fp.extend_from_slice(&232000u32.to_le_bytes());
        let file_props = obj(&FILE_PROPERTIES, &fp);
        // video stream 1: 64x48 WMV2
        let mut ts = Vec::new();
        ts.extend_from_slice(&64u32.to_le_bytes());
        ts.extend_from_slice(&48u32.to_le_bytes());
        ts.push(2);
        ts.extend_from_slice(&44u16.to_le_bytes());
        let mut bih = Vec::new();
        bih.extend_from_slice(&44u32.to_le_bytes());
        bih.extend_from_slice(&64u32.to_le_bytes());
        bih.extend_from_slice(&48u32.to_le_bytes());
        bih.extend_from_slice(&1u16.to_le_bytes());
        bih.extend_from_slice(&24u16.to_le_bytes());
        bih.extend_from_slice(b"WMV2");
        bih.extend_from_slice(&[0u8; 20]);
        bih.extend_from_slice(&[0xC8, 0xC3, 0xB4, 0x80]);
        ts.extend_from_slice(&bih);
        let mut sp = VIDEO_MEDIA.to_vec();
        sp.extend_from_slice(&[0u8; 16]);
        sp.extend_from_slice(&0u64.to_le_bytes());
        sp.extend_from_slice(&(ts.len() as u32).to_le_bytes());
        sp.extend_from_slice(&0u32.to_le_bytes());
        sp.extend_from_slice(&1u16.to_le_bytes());
        sp.extend_from_slice(&0u32.to_le_bytes());
        sp.extend_from_slice(&ts);
        let video = obj(&STREAM_PROPERTIES, &sp);
        // audio stream 2: WAVEFORMATEX tag 0x161
        let wf: Vec<u8> = [0x61, 0x01, 0x01, 0x00, 0x80, 0xBB, 0, 0, 0xA0, 0x0F, 0, 0, 0xAA, 0x00, 0x10, 0x00, 0x0A, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].to_vec();
        let mut sp = AUDIO_MEDIA.to_vec();
        sp.extend_from_slice(&[0u8; 16]);
        sp.extend_from_slice(&0u64.to_le_bytes());
        sp.extend_from_slice(&(wf.len() as u32).to_le_bytes());
        sp.extend_from_slice(&0u32.to_le_bytes());
        sp.extend_from_slice(&2u16.to_le_bytes());
        sp.extend_from_slice(&0u32.to_le_bytes());
        sp.extend_from_slice(&wf);
        let audio = obj(&STREAM_PROPERTIES, &sp);
        // codec list
        let mut cl = vec![0u8; 16];
        cl.extend_from_slice(&2u32.to_le_bytes());
        for (t, name, info) in [(1u16, "wmv2", b"WMV2".to_vec()), (2u16, "Windows Media Audio V8", vec![0x61, 0x01])] {
            cl.extend_from_slice(&t.to_le_bytes());
            let w = wide(name);
            cl.extend_from_slice(&((w.len() / 2) as u16).to_le_bytes());
            cl.extend_from_slice(&w);
            cl.extend_from_slice(&0u16.to_le_bytes());
            cl.extend_from_slice(&(info.len() as u16).to_le_bytes());
            cl.extend_from_slice(&info);
        }
        let codecs = obj(&CODEC_LIST, &cl);
        // extended content description + metadata (aspect ratio) inside a header extension
        let mut ecd = 1u16.to_le_bytes().to_vec();
        let n = wide("WM/EncodingSettings");
        ecd.extend_from_slice(&(n.len() as u16).to_le_bytes());
        ecd.extend_from_slice(&n);
        ecd.extend_from_slice(&0u16.to_le_bytes());
        let v = wide("Lavf63.1.101");
        ecd.extend_from_slice(&(v.len() as u16).to_le_bytes());
        ecd.extend_from_slice(&v);
        let ecd = obj(&EXTENDED_CONTENT_DESCRIPTION, &ecd);
        let mut md = 2u16.to_le_bytes().to_vec();
        for (name, val) in [("AspectRatioX", 4u32), ("AspectRatioY", 3u32)] {
            let w = wide(name);
            md.extend_from_slice(&0u16.to_le_bytes());
            md.extend_from_slice(&1u16.to_le_bytes());
            md.extend_from_slice(&(w.len() as u16).to_le_bytes());
            md.extend_from_slice(&3u16.to_le_bytes());
            md.extend_from_slice(&4u32.to_le_bytes());
            md.extend_from_slice(&w);
            md.extend_from_slice(&val.to_le_bytes());
        }
        let md = obj(&METADATA, &md);
        let mut ext_body = vec![0u8; 16];
        ext_body.extend_from_slice(&6u16.to_le_bytes());
        ext_body.extend_from_slice(&(md.len() as u32).to_le_bytes());
        ext_body.extend_from_slice(&md);
        let ext = obj(&HEADER_EXTENSION, &ext_body);
        let mut cd = Vec::new();
        let title = wide("My title");
        cd.extend_from_slice(&(title.len() as u16).to_le_bytes());
        cd.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        cd.extend_from_slice(&title);
        let cd = obj(&CONTENT_DESCRIPTION, &cd);
        let mut hb = Vec::new();
        hb.extend_from_slice(&6u32.to_le_bytes());
        hb.extend_from_slice(&[1, 2]);
        for o in [&file_props, &ext, &ecd, &video, &audio, &codecs, &cd] {
            hb.extend_from_slice(o);
        }
        let mut f = obj(&HEADER, &hb);
        // data object with one packet holding two payloads (video pts 3143 / 3183)
        // error correction (2 bytes), length type flags: multiple payloads + word padding length,
        // property flags: byte-sized replicated data length / offset / media object number / stream number
        let mut pkt = vec![0x82, 0x00, 0x00, 0x11, 0x55, 0, 0, 0, 0, 0, 0, 0x04, 0x00, 0x02 | 0x40];
        for (pts, n) in [(3143u32, 1u8), (3183, 2)] {
            pkt.push(1);
            pkt.push(n);
            pkt.push(0);
            pkt.push(8);
            pkt.extend_from_slice(&10u32.to_le_bytes());
            pkt.extend_from_slice(&pts.to_le_bytes());
            pkt.push(10);
            pkt.extend_from_slice(&[0xAA; 10]);
        }
        let padding = 3200 - pkt.len();
        pkt[5] = (padding & 0xFF) as u8;
        pkt[6] = (padding >> 8) as u8;
        pkt.resize(3200, 0);
        let mut db = vec![0u8; 16];
        db.extend_from_slice(&1u64.to_le_bytes());
        db.extend_from_slice(&[1, 1]);
        db.extend_from_slice(&pkt);
        f.extend_from_slice(&obj(&DATA, &db));
        f
    }

    #[test]
    fn parses_header_objects() {
        let f = build();
        assert_eq!(probe(&Probe { head: &f, ext: "wmv", size: f.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "wmv", size: 4 }), 0);
        let mut r = Reader::from_bytes(f);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "Windows Media");
        assert_eq!(g.get("Duration"), "1043");
        assert_eq!(g.get("OverallBitRate_Maximum"), "232000");
        assert_eq!(g.get("Encoded_Date"), "UTC 1970-01-01 00:00:00.000");
        assert_eq!(g.get("WM/EncodingSettings"), "Lavf63.1.101");
        assert_eq!(g.get("Title"), "My title");
        assert!(g.get_u64("HeaderSize").unwrap() > 0);
        assert_eq!(g.get("DataSize"), "3250");
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("ID"), "1");
        assert_eq!(v.get("StreamOrder"), "0");
        assert_eq!(v.get("CodecID"), "WMV2");
        assert_eq!(v.get("CodecID_Description"), "wmv2");
        assert_eq!(v.get("Width"), "64");
        assert_eq!(v.get("Height"), "48");
        assert_eq!(v.get("PixelAspectRatio"), "1.333");
        assert_eq!(v.get("Duration_Source"), "General_Duration");
        let a = &doc.streams[StreamKind::Audio as usize][0];
        assert_eq!(a.get("ID"), "2");
        assert_eq!(a.get("CodecID"), "161");
        assert_eq!(a.get("CodecID_Description"), "Windows Media Audio V8");
        assert_eq!(a.get("Duration"), "1043");
    }

    #[test]
    fn data_packet_payloads() {
        let f = build();
        let mut r = Reader::from_bytes(f);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        // no bit rate known for the video: counted payload bytes are reported
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("StreamSize"), "20");
        assert_eq!(estimate_frame_rate(&[0, 40, 80, 120]), Some(25.0));
        assert_eq!(estimate_frame_rate(&[0, 40]), None);
    }

    #[test]
    fn filetime() {
        assert_eq!(filetime_string(116444736000000000), "UTC 1970-01-01 00:00:00.000");
        assert_eq!(filetime_string(0), "UTC 1970-01-01 00:00:00.000");
        assert_eq!(filetime_string(116444736000000000 + 10_000_000 * 86400 + 5_000_000), "UTC 1970-01-02 00:00:00.500");
    }

    #[test]
    fn truncated_input() {
        let mut f = build();
        f.truncate(200);
        let mut r = Reader::from_bytes(f);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert!(parse(&mut Reader::from_bytes(HEADER.to_vec()), &mut Doc::new()) == false);
    }
}

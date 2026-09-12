//! Nut (FFmpeg/MPlayer container): file id, main header, stream headers, info packets.
//!
//! The reference report only identifies the container (General `Format` "Nut") without stream
//! details; the headers are still parsed so the file is validated, and stream emission can be
//! switched on with [`EMIT_STREAMS`].

use crate::io::{be64, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::audio::aac;
use crate::parsers::video::{avc, fourcc};
use crate::parsers::Probe;

/// Emit Video/Audio streams from the stream headers (off: the reference only reports the container).
const EMIT_STREAMS: bool = false;

const FILE_ID: &[u8] = b"nut/multimedia container\0";
const MAIN_STARTCODE: u64 = 0x4E4D_7A56_1F5F_04AD;
const STREAM_STARTCODE: u64 = 0x4E53_1140_5BF2_F9DB;
const SYNCPOINT_STARTCODE: u64 = 0x4E4B_E4AD_EECA_4569;
const INFO_STARTCODE: u64 = 0x4E49_AB68_B596_BA78;
const MAX_PACKETS: usize = 4096;
const HEAD_SCAN: u64 = 8 * 1024 * 1024;

pub fn probe(p: &Probe) -> u8 {
    if p.starts_with(FILE_ID) && p.head.len() >= FILE_ID.len() + 8 {
        100
    } else {
        0
    }
}

/// Variable-length unsigned integer (7 bits per byte, MSB set = continue).
pub fn read_v(b: &[u8], p: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for _ in 0..10 {
        let byte = *b.get(*p)?;
        *p += 1;
        v = (v << 7) | (byte & 0x7F) as u64;
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn read_vb<'a>(b: &'a [u8], p: &mut usize) -> Option<&'a [u8]> {
    let len = read_v(b, p)? as usize;
    let s = b.get(*p..*p + len)?;
    *p += len;
    Some(s)
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct MainHeader {
    pub version: u64,
    pub stream_count: u64,
    pub max_distance: u64,
    pub time_bases: Vec<(u64, u64)>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct StreamHeader {
    pub id: u64,
    pub class: u64,
    pub fourcc: Vec<u8>,
    pub time_base: u64,
    pub msb_pts_shift: u64,
    pub max_pts_distance: u64,
    pub decode_delay: u64,
    pub flags: u64,
    pub codec_specific: Vec<u8>,
    pub width: u64,
    pub height: u64,
    pub sample_width: u64,
    pub sample_height: u64,
    pub colorspace: u64,
    pub samplerate: (u64, u64),
    pub channels: u64,
}

pub fn parse_main_header(b: &[u8]) -> Option<MainHeader> {
    let mut p = 0;
    let version = read_v(b, &mut p)?;
    if version > 3 {
        let _minor = read_v(b, &mut p)?;
    }
    let stream_count = read_v(b, &mut p)?;
    let max_distance = read_v(b, &mut p)?;
    let count = read_v(b, &mut p)?;
    let mut time_bases = Vec::new();
    for _ in 0..count.min(256) {
        let n = read_v(b, &mut p)?;
        let d = read_v(b, &mut p)?;
        time_bases.push((n, d));
    }
    Some(MainHeader { version, stream_count, max_distance, time_bases })
}

pub fn parse_stream_header(b: &[u8]) -> Option<StreamHeader> {
    let mut p = 0;
    let mut s = StreamHeader { id: read_v(b, &mut p)?, class: read_v(b, &mut p)?, ..Default::default() };
    s.fourcc = read_vb(b, &mut p)?.to_vec();
    s.time_base = read_v(b, &mut p)?;
    s.msb_pts_shift = read_v(b, &mut p)?;
    s.max_pts_distance = read_v(b, &mut p)?;
    s.decode_delay = read_v(b, &mut p)?;
    s.flags = read_v(b, &mut p)?;
    s.codec_specific = read_vb(b, &mut p)?.to_vec();
    match s.class {
        0 => {
            s.width = read_v(b, &mut p)?;
            s.height = read_v(b, &mut p)?;
            s.sample_width = read_v(b, &mut p)?;
            s.sample_height = read_v(b, &mut p)?;
            s.colorspace = read_v(b, &mut p)?;
        }
        1 => {
            let n = read_v(b, &mut p)?;
            let d = read_v(b, &mut p)?;
            s.samplerate = (n, d);
            s.channels = read_v(b, &mut p)?;
        }
        _ => {}
    }
    Some(s)
}

#[derive(Debug, Default)]
struct Ctx {
    main: Option<MainHeader>,
    streams: Vec<StreamHeader>,
    info: Vec<(String, String)>,
}

/// Info packet: stream_id_plus1, chapter_id, chapter_start, chapter_len, count, then (id, value) pairs.
fn parse_info(b: &[u8], ctx: &mut Ctx) {
    let mut p = 0;
    let Some(_stream_id) = read_v(b, &mut p) else { return };
    let Some(_chapter_id) = read_v(b, &mut p) else { return };
    let Some(_start) = read_v(b, &mut p) else { return };
    let Some(_len) = read_v(b, &mut p) else { return };
    let Some(count) = read_v(b, &mut p) else { return };
    for _ in 0..count.min(256) {
        let Some(id) = read_v(b, &mut p) else { return };
        let name = if id == 0 {
            match read_vb(b, &mut p) {
                Some(n) => String::from_utf8_lossy(n).to_string(),
                None => return,
            }
        } else {
            String::new()
        };
        // value: signed v (type) — negative encodes the kind
        let Some(raw) = read_v(b, &mut p) else { return };
        let ty = if raw & 1 == 1 { -(((raw + 1) / 2) as i64) } else { (raw / 2) as i64 };
        let value = match ty {
            -1 => read_vb(b, &mut p).map(|v| String::from_utf8_lossy(v).to_string()),
            -2 => read_v(b, &mut p).map(|v| v.to_string()),
            -3 => {
                let a = read_v(b, &mut p);
                let b2 = read_v(b, &mut p);
                a.zip(b2).map(|(a, b)| format!("{a}/{b}"))
            }
            -4 => read_v(b, &mut p).map(|v| v.to_string()),
            v if v >= 0 => Some(v.to_string()),
            _ => None,
        };
        let Some(value) = value else { return };
        if !name.is_empty() && ctx.info.len() < 256 {
            ctx.info.push((name, value));
        }
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, FILE_ID.len());
    if head.as_slice() != FILE_ID {
        return false;
    }
    let mut ctx = Ctx::default();
    let len = r.len();
    let mut pos = FILE_ID.len() as u64;
    let end = len.min(HEAD_SCAN);
    let mut n = 0;
    while pos + 9 <= end && n < MAX_PACKETS {
        n += 1;
        let h = r.read_vec_at(pos, 24);
        let Some(startcode) = be64(&h, 0) else { break };
        if startcode >> 56 != 0x4E {
            // frame data: without the frame code table we cannot walk further
            break;
        }
        let mut p = 8;
        let Some(fwd) = read_v(&h, &mut p) else { break };
        if fwd > 4096 {
            p += 4; // header checksum
        }
        let body_pos = pos + p as u64;
        let body_len = fwd.saturating_sub(4).min(1 << 20) as usize;
        let body = r.read_vec_at(body_pos, body_len);
        match startcode {
            MAIN_STARTCODE => ctx.main = parse_main_header(&body),
            STREAM_STARTCODE => {
                if let Some(s) = parse_stream_header(&body) {
                    if ctx.streams.len() < 256 {
                        ctx.streams.push(s);
                    }
                }
            }
            INFO_STARTCODE => parse_info(&body, &mut ctx),
            SYNCPOINT_STARTCODE => break, // frames follow; the frame code table is not interpreted
            _ => {}
        }
        pos = body_pos + fwd;
    }
    emit(doc, &ctx, len);
    true
}

fn emit(doc: &mut Doc, ctx: &Ctx, file_size: u64) {
    let g = doc.general();
    g.set("Format", "Nut");
    g.set_int("StreamSize", file_size as i128);
    for (k, v) in &ctx.info {
        match k.as_str() {
            "title" | "Title" => g.set_if_empty("Title", v.clone()),
            "author" | "Author" | "artist" => g.set_if_empty("Performer", v.clone()),
            "copyright" | "Copyright" => g.set_if_empty("Copyright", v.clone()),
            "comment" | "Comment" => g.set_if_empty("Comment", v.clone()),
            "encoder" | "Encoder" => g.set_if_empty("Encoded_Application", v.clone()),
            _ => {}
        }
    }
    if !EMIT_STREAMS {
        return;
    }
    let time_bases = ctx.main.as_ref().map(|m| m.time_bases.clone()).unwrap_or_default();
    for sh in &ctx.streams {
        let kind = match sh.class {
            0 => StreamKind::Video,
            1 => StreamKind::Audio,
            2 => StreamKind::Text,
            _ => continue,
        };
        let mut s = Stream::new(kind);
        s.set_int("ID", sh.id as i128);
        let cc = String::from_utf8_lossy(&sh.fourcc).to_string();
        s.set("CodecID", cc.clone());
        match kind {
            StreamKind::Video => {
                if let Some((f, _, _)) = fourcc::fourcc_format(&cc) {
                    s.set("Format", f);
                }
                if cc == "H264" || cc == "avc1" {
                    s.set("Format", "AVC");
                    if !avc::apply_avcc(&mut s, &sh.codec_specific) {
                        let nals = avc::nals_annexb(&sh.codec_specific);
                        if let Some((_, sps)) = nals.iter().find(|(t, _)| *t == 7) {
                            if let Some(sps) = avc::parse_sps(sps) {
                                avc::apply(&mut s, &sps, None, true);
                            }
                        }
                    }
                }
                if sh.width > 0 && sh.height > 0 {
                    s.set_if_empty("Width", sh.width.to_string());
                    s.set_if_empty("Height", sh.height.to_string());
                }
                if sh.sample_width > 0 && sh.sample_height > 0 {
                    s.set("PixelAspectRatio", format!("{:.3}", sh.sample_width as f64 / sh.sample_height as f64));
                }
                if let Some((n, d)) = time_bases.get(sh.time_base as usize) {
                    if *n > 0 && *d > 0 {
                        s.set("FrameRate", format!("{:.3}", *d as f64 / *n as f64));
                    }
                }
            }
            StreamKind::Audio => {
                if cc == "mp4a" || cc == "AAC " {
                    s.set("Format", "AAC");
                    aac::apply_asc(&mut s, &sh.codec_specific);
                }
                if sh.samplerate.1 > 0 {
                    s.set_if_empty("SamplingRate", (sh.samplerate.0 / sh.samplerate.1).to_string());
                }
                if sh.channels > 0 {
                    s.set_if_empty("Channel(s)", sh.channels.to_string());
                }
            }
            _ => {}
        }
        doc.streams[kind as usize].push(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(mut x: u64) -> Vec<u8> {
        let mut out = vec![(x & 0x7F) as u8];
        x >>= 7;
        while x > 0 {
            out.insert(0, 0x80 | (x & 0x7F) as u8);
            x >>= 7;
        }
        out
    }

    fn vb(b: &[u8]) -> Vec<u8> {
        let mut o = v(b.len() as u64);
        o.extend_from_slice(b);
        o
    }

    fn packet(startcode: u64, body: &[u8]) -> Vec<u8> {
        let mut p = startcode.to_be_bytes().to_vec();
        p.extend(v(body.len() as u64 + 4));
        p.extend_from_slice(body);
        p.extend_from_slice(&[0, 0, 0, 0]);
        p
    }

    fn build() -> Vec<u8> {
        let mut f = FILE_ID.to_vec();
        let mut main = v(3);
        main.extend(v(2));
        main.extend(v(65536));
        main.extend(v(1));
        main.extend(v(1));
        main.extend(v(25));
        f.extend(packet(MAIN_STARTCODE, &main));
        let mut sv = v(0);
        sv.extend(v(0));
        sv.extend(vb(b"H264"));
        sv.extend(v(0));
        sv.extend(v(2));
        sv.extend(v(25));
        sv.extend(v(0));
        sv.extend(v(0));
        sv.extend(vb(&[]));
        sv.extend(v(64));
        sv.extend(v(48));
        sv.extend(v(1));
        sv.extend(v(1));
        sv.extend(v(0));
        f.extend(packet(STREAM_STARTCODE, &sv));
        let mut sa = v(1);
        sa.extend(v(1));
        sa.extend(vb(b"mp4a"));
        sa.extend(v(0));
        sa.extend(v(2));
        sa.extend(v(25));
        sa.extend(v(0));
        sa.extend(v(0));
        sa.extend(vb(&[0x11, 0x88]));
        sa.extend(v(48000));
        sa.extend(v(1));
        sa.extend(v(1));
        f.extend(packet(STREAM_STARTCODE, &sa));
        let mut info = v(0);
        info.extend(v(0));
        info.extend(v(0));
        info.extend(v(0));
        info.extend(v(1));
        info.extend(v(0));
        info.extend(vb(b"encoder"));
        info.extend(v(1)); // type -1 = string
        info.extend(vb(b"Lavf"));
        f.extend(packet(INFO_STARTCODE, &info));
        let mut sp = v(4000);
        sp.extend(v(0));
        f.extend(packet(SYNCPOINT_STARTCODE, &sp));
        f
    }

    #[test]
    fn varints_and_headers() {
        let mut p = 0;
        assert_eq!(read_v(&[0x81, 0x00], &mut p), Some(128));
        assert_eq!(read_v(&[0x80], &mut 0), None);
        let m = parse_main_header(&[3, 2, 0x84, 0x00, 1, 1, 25]).unwrap();
        assert_eq!(m, MainHeader { version: 3, stream_count: 2, max_distance: 512, time_bases: vec![(1, 25)] });
        assert!(parse_stream_header(&[0, 0, 4, b'H']).is_none());
    }

    #[test]
    fn parses_file() {
        let f = build();
        assert_eq!(probe(&Probe { head: &f, ext: "nut", size: f.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"nut/", ext: "nut", size: 4 }), 0);
        let size = f.len() as u64;
        let mut r = Reader::from_bytes(f);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "Nut");
        assert_eq!(g.get("StreamSize"), size.to_string());
        assert_eq!(g.get("Encoded_Application"), "Lavf");
        assert!(doc.streams[StreamKind::Video as usize].is_empty() || EMIT_STREAMS);
        let mut r = Reader::from_bytes(FILE_ID.to_vec());
        assert!(parse(&mut r, &mut Doc::new()));
        let mut r = Reader::from_bytes(b"nut/multimedia".to_vec());
        assert!(!parse(&mut r, &mut Doc::new()));
    }

    #[test]
    fn stream_headers_are_parsed() {
        let f = build();
        let mut r = Reader::from_bytes(f);
        let mut ctx = Ctx::default();
        // walk packets manually to check the stream header parser on the built data
        let mut pos = FILE_ID.len() as u64;
        for _ in 0..4 {
            let h = r.read_vec_at(pos, 24);
            let sc = be64(&h, 0).unwrap();
            let mut p = 8;
            let fwd = read_v(&h, &mut p).unwrap();
            let body = r.read_vec_at(pos + p as u64, (fwd - 4) as usize);
            if sc == STREAM_STARTCODE {
                ctx.streams.push(parse_stream_header(&body).unwrap());
            }
            pos += p as u64 + fwd;
        }
        assert_eq!(ctx.streams.len(), 2);
        assert_eq!(ctx.streams[0].fourcc, b"H264");
        assert_eq!(ctx.streams[0].width, 64);
        assert_eq!(ctx.streams[1].samplerate, (48000, 1));
        assert_eq!(ctx.streams[1].channels, 1);
        assert_eq!(ctx.streams[1].codec_specific, vec![0x11, 0x88]);
    }
}

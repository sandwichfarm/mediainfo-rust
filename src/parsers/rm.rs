//! RealMedia (`.RMF`): PROP, MDPR (per-stream properties with RealVideo / RealAudio headers), CONT, DATA, INDX.

use crate::io::{be16, be32, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::video::fourcc;
use crate::parsers::Probe;

const MAX_CHUNKS: usize = 4096;
const MAX_STREAMS: usize = 64;

pub fn probe(p: &Probe) -> u8 {
    if p.starts_with(b".RMF") && p.head.len() >= 18 {
        100
    } else {
        0
    }
}

#[derive(Debug, Default, Clone)]
struct Mdpr {
    number: u16,
    max_bitrate: u32,
    avg_bitrate: u32,
    start_time: u32,
    duration: u32,
    name: String,
    mime: String,
    specific: Vec<u8>,
}

#[derive(Debug, Default)]
struct Ctx {
    max_bitrate: u32,
    avg_bitrate: u32,
    duration: u32,
    streams: Vec<Mdpr>,
    title: String,
    author: String,
    copyright: String,
    comment: String,
    has_data: bool,
}

fn pstring(b: &[u8], p: &mut usize) -> Option<String> {
    let len = *b.get(*p)? as usize;
    let s = b.get(*p + 1..*p + 1 + len)?;
    *p += 1 + len;
    Some(crate::io::clean_text(&crate::io::latin1(s)))
}

fn wstring(b: &[u8], p: &mut usize) -> Option<String> {
    let len = be16(b, *p)? as usize;
    let s = b.get(*p + 2..*p + 2 + len)?;
    *p += 2 + len;
    Some(crate::io::clean_text(&String::from_utf8_lossy(s)))
}

fn parse_prop(b: &[u8], ctx: &mut Ctx) {
    // object_version(2) max/avg bit rate(4+4) max/avg packet size(4+4) packet count(4) duration(4)
    // preroll(4) index offset(4) data offset(4) stream count(2) flags(2)
    if b.len() < 42 {
        return;
    }
    ctx.max_bitrate = be32(b, 2).unwrap_or(0);
    ctx.avg_bitrate = be32(b, 6).unwrap_or(0);
    ctx.duration = be32(b, 22).unwrap_or(0);
}

fn parse_mdpr(b: &[u8]) -> Option<Mdpr> {
    // stream number(2) max/avg bit rate(4+4) max/avg packet size(4+4) start time(4) preroll(4) duration(4)
    let mut m = Mdpr { number: be16(b, 2)?, max_bitrate: be32(b, 4)?, avg_bitrate: be32(b, 8)?, start_time: be32(b, 20)?, duration: be32(b, 28)?, ..Default::default() };
    let mut p = 32;
    m.name = pstring(b, &mut p)?;
    m.mime = pstring(b, &mut p)?;
    let len = be32(b, p)? as usize;
    m.specific = b.get(p + 4..p + 4 + len).unwrap_or(&[]).to_vec();
    Some(m)
}

fn parse_cont(b: &[u8], ctx: &mut Ctx) {
    let mut p = 2;
    ctx.title = wstring(b, &mut p).unwrap_or_default();
    ctx.author = wstring(b, &mut p).unwrap_or_default();
    ctx.copyright = wstring(b, &mut p).unwrap_or_default();
    ctx.comment = wstring(b, &mut p).unwrap_or_default();
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let h = r.read_vec_at(0, 18);
    if h.len() < 18 || &h[..4] != b".RMF" {
        return false;
    }
    let mut ctx = Ctx::default();
    let mut pos = 0u64;
    let mut n = 0;
    let len = r.len();
    while pos + 10 <= len && n < MAX_CHUNKS {
        n += 1;
        let ch = r.read_vec_at(pos, 10);
        if ch.len() < 10 {
            break;
        }
        let id = &ch[..4];
        let size = be32(&ch, 4).unwrap_or(0) as u64;
        if size < 10 {
            break;
        }
        let body_len = (size - 8).min(1 << 20) as usize;
        match id {
            b".RMF" => {}
            b"PROP" => {
                let b = r.read_vec_at(pos + 8, body_len);
                parse_prop(&b, &mut ctx);
            }
            b"MDPR" => {
                let b = r.read_vec_at(pos + 8, body_len);
                if let Some(m) = parse_mdpr(&b) {
                    if ctx.streams.len() < MAX_STREAMS {
                        ctx.streams.push(m);
                    }
                }
            }
            b"CONT" => {
                let b = r.read_vec_at(pos + 8, body_len);
                parse_cont(&b, &mut ctx);
            }
            b"DATA" => {
                ctx.has_data = true;
                // DATA size covers all packets; a size of 0 or larger than the file ends the walk
                if size == 0 || pos + size > len {
                    break;
                }
            }
            _ => {}
        }
        pos += size;
    }
    emit(doc, &ctx);
    true
}

// ---------------------------------------------------------------------------- emit

/// RealVideo type-specific data: size(4) "VIDO" fourcc(4) width(2) height(2) bpp(2) unknown(4) fps(16.16)…
fn apply_video(s: &mut Stream, b: &[u8]) {
    if b.len() < 26 || &b[4..8] != b"VIDO" {
        return;
    }
    let cc = String::from_utf8_lossy(&b[8..12]).to_string();
    s.set("CodecID", cc.clone());
    match cc.as_str() {
        "RV10" => s.set("Format", "RealVideo 1"),
        "RV20" => s.set("Format", "RealVideo 2"),
        "RV30" => s.set("Format", "RealVideo 3"),
        "RV40" => s.set("Format", "RealVideo 4"),
        "CLV1" => s.set("Format", "ClearVideo"),
        _ => match fourcc::fourcc_format(&cc) {
            Some((f, _, _)) => s.set("Format", f),
            None => s.set("Format", cc.clone()),
        },
    }
    let w = be16(b, 12).unwrap_or(0);
    let h = be16(b, 14).unwrap_or(0);
    if w > 0 && h > 0 {
        s.set_int("Width", w as i128);
        s.set_int("Height", h as i128);
    }
    let fps = be32(b, 22).unwrap_or(0);
    if fps > 0 {
        s.set("FrameRate", format!("{:.3}", fps as f64 / 65536.0));
    }
}

/// RealAudio header (`.ra` 0xFD, version 3/4/5).
fn apply_audio(s: &mut Stream, b: &[u8]) {
    if b.len() < 6 || &b[..4] != b".ra\xfd" {
        return;
    }
    let version = be16(b, 4).unwrap_or(0);
    let (rate, bits, channels, fourcc): (u32, u32, u32, String) = match version {
        3 => {
            // header size(2), then 10 bytes, then codec data; 8 kHz mono 16 bit "14_4"
            (8000, 16, 1, "14_4".to_string())
        }
        4 => {
            // … sub packet size(2) unknown(2) | sample rate(2) unknown(2) sample size(2) channels(2)
            // interleaver(pstring) fourcc(pstring)
            let rate = be16(b, 48).unwrap_or(0) as u32;
            let bits = be16(b, 52).unwrap_or(0) as u32;
            let channels = be16(b, 54).unwrap_or(0) as u32;
            let mut p = 56;
            let _interleaver = pstring(b, &mut p);
            let cc = pstring(b, &mut p).unwrap_or_default();
            (rate, bits, channels, cc)
        }
        5 => {
            // Same tail as version 4 but with fixed 4-byte interleaver and codec ids; the position of
            // the codec id varies between writers, so it is located among the known codes.
            const KNOWN: &[&[u8; 4]] = &[b"cook", b"dnet", b"sipr", b"atrc", b"raac", b"racp", b"ralf", b"28_8", b"14_4", b"lpcJ", b"whr1", b"whr2"];
            let cc_off = [66usize, 64, 68, 62].into_iter().find(|&o| b.get(o..o + 4).is_some_and(|c| KNOWN.iter().any(|k| *k == c))).unwrap_or(66);
            let rate = be16(b, cc_off - 12).unwrap_or(0) as u32;
            let bits = be16(b, cc_off - 8).unwrap_or(0) as u32;
            let channels = be16(b, cc_off - 6).unwrap_or(0) as u32;
            let cc = b.get(cc_off..cc_off + 4).map(|c| String::from_utf8_lossy(c).to_string()).unwrap_or_default();
            (rate, bits, channels, cc)
        }
        _ => return,
    };
    let cc = cc_clean(&fourcc);
    if !cc.is_empty() {
        s.set("CodecID", cc.clone());
    }
    let format = match cc.as_str() {
        "dnet" => "AC-3",
        "cook" => "Cook",
        "sipr" => "Sipro",
        "atrc" => "Atrac",
        "raac" | "racp" => "AAC",
        "ralf" => "RealAudio Lossless",
        "14_4" | "lpcJ" => "RealAudio 1",
        "28_8" => "RealAudio 2",
        "whr1" | "whr2" => "WHR",
        _ => "",
    };
    if !format.is_empty() {
        s.set("Format", format);
        if format == "AC-3" {
            // byte-swapped AC-3 without the elementary stream analysis: plain commercial name
            s.set("Format_Commercial", "AC-3");
        }
    }
    if rate > 0 {
        s.set_int("SamplingRate", rate as i128);
    }
    if channels > 0 {
        s.set_int("Channel(s)", channels as i128);
    }
    if bits > 0 {
        s.set_int("BitDepth", bits as i128);
    }
    if matches!(format, "AC-3" | "Cook" | "Sipro" | "Atrac" | "AAC" | "RealAudio 1" | "RealAudio 2") {
        s.set("Compression_Mode", "Lossy");
    } else if format == "RealAudio Lossless" {
        s.set("Compression_Mode", "Lossless");
    }
}

fn cc_clean(cc: &str) -> String {
    cc.chars().filter(|c| c.is_ascii_graphic()).collect()
}

fn emit(doc: &mut Doc, ctx: &Ctx) {
    let g = doc.general();
    g.set("Format", "RealMedia");
    if ctx.duration > 0 {
        g.set_int("Duration", ctx.duration as i128);
    }
    if ctx.avg_bitrate > 0 {
        g.set_int("OverallBitRate", ctx.avg_bitrate as i128);
    }
    if ctx.max_bitrate > 0 && ctx.max_bitrate != ctx.avg_bitrate {
        g.set_int("OverallBitRate_Maximum", ctx.max_bitrate as i128);
    }
    if !ctx.title.is_empty() {
        g.set("Title", ctx.title.clone());
    }
    if !ctx.author.is_empty() {
        g.set("Performer", ctx.author.clone());
    }
    if !ctx.copyright.is_empty() {
        g.set("Copyright", ctx.copyright.clone());
    }
    if !ctx.comment.is_empty() {
        g.set("Comment", ctx.comment.clone());
    }
    let mut streams: Vec<&Mdpr> = ctx.streams.iter().collect();
    streams.sort_by_key(|m| m.number);
    for m in streams {
        let sp = m.specific.as_slice();
        let kind = if m.mime.starts_with("video/") || (sp.len() >= 8 && &sp[4..8] == b"VIDO") {
            StreamKind::Video
        } else if m.mime.starts_with("audio/") || sp.starts_with(b".ra\xfd") {
            StreamKind::Audio
        } else if m.mime.starts_with("logical-") {
            continue;
        } else {
            continue;
        };
        let mut s = Stream::new(kind);
        s.set_int("ID", m.number as i128);
        match kind {
            StreamKind::Video => apply_video(&mut s, sp),
            StreamKind::Audio => apply_audio(&mut s, sp),
            _ => {}
        }
        if m.avg_bitrate > 0 {
            s.set_int("BitRate", m.avg_bitrate as i128);
        }
        if m.max_bitrate > 0 && m.max_bitrate != m.avg_bitrate {
            s.set_int("BitRate_Maximum", m.max_bitrate as i128);
        }
        if m.duration > 0 {
            s.set_int("Duration", m.duration as i128);
            if m.avg_bitrate > 0 {
                s.set_int("StreamSize", (m.avg_bitrate as u64 * m.duration as u64 / 8000) as i128);
            }
        }
        if m.start_time > 0 {
            s.set_int("Delay", m.start_time as i128);
            s.set("Delay_Source", "Container");
        }
        if !m.name.is_empty() && !m.name.starts_with("The ") {
            s.set("Title", m.name.clone());
        }
        doc.streams[kind as usize].push(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    fn mdpr(number: u16, avg: u32, duration: u32, name: &str, mime: &str, specific: &[u8]) -> Vec<u8> {
        let mut b = vec![0, 0];
        b.extend_from_slice(&number.to_be_bytes());
        b.extend_from_slice(&avg.to_be_bytes());
        b.extend_from_slice(&avg.to_be_bytes());
        b.extend_from_slice(&4096u32.to_be_bytes());
        b.extend_from_slice(&256u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&duration.to_be_bytes());
        b.push(name.len() as u8);
        b.extend_from_slice(name.as_bytes());
        b.push(mime.len() as u8);
        b.extend_from_slice(mime.as_bytes());
        b.extend_from_slice(&(specific.len() as u32).to_be_bytes());
        b.extend_from_slice(specific);
        chunk(b"MDPR", &b)
    }

    fn build() -> Vec<u8> {
        let mut f = chunk(b".RMF", &[0, 0, 0, 0, 0, 0, 0, 0, 0, 3]);
        let mut prop = vec![0, 0];
        for v in [114000u32, 114000, 4096, 278, 57, 1024, 0, 0, 0x165] {
            prop.extend_from_slice(&v.to_be_bytes());
        }
        prop.extend_from_slice(&[0, 2, 0, 3]);
        f.extend(chunk(b"PROP", &prop));
        let mut cont = vec![0, 0, 0, 5];
        cont.extend_from_slice(b"Title");
        cont.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        f.extend(chunk(b"CONT", &cont));
        let mut vido = 34u32.to_be_bytes().to_vec();
        vido.extend_from_slice(b"VIDORV20");
        vido.extend_from_slice(&[0, 64, 0, 48, 0, 25, 0, 0, 0, 0, 0, 25, 0, 0, 0, 0, 0, 8, 0x20, 0x10, 0x30, 0x01]);
        f.extend(mdpr(0, 50000, 1000, "The Video Stream", "video/x-pn-realvideo", &vido));
        let mut ra = b".ra\xfd\x00\x04\x00\x00.ra4".to_vec();
        ra.resize(48, 0);
        ra.extend_from_slice(&48000u16.to_be_bytes());
        ra.extend_from_slice(&[0, 0, 0, 16, 0, 1, 4]);
        ra.extend_from_slice(b"Int0");
        ra.push(4);
        ra.extend_from_slice(b"dnet");
        f.extend(mdpr(1, 64000, 1024, "The Audio Stream", "audio/x-pn-realaudio", &ra));
        f.extend(chunk(b"DATA", &[0, 0, 0, 0, 0, 57, 0, 0, 0, 0, 1, 2, 3]));
        f
    }

    #[test]
    fn parses_headers() {
        let f = build();
        assert_eq!(probe(&Probe { head: &f, ext: "rm", size: f.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "rm", size: 4 }), 0);
        let mut r = Reader::from_bytes(f);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "RealMedia");
        assert_eq!(g.get("Duration"), "1024");
        assert_eq!(g.get("OverallBitRate"), "114000");
        assert_eq!(g.get("Title"), "Title");
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("ID"), "0");
        assert_eq!(v.get("Format"), "RealVideo 2");
        assert_eq!(v.get("CodecID"), "RV20");
        assert_eq!(v.get("Width"), "64");
        assert_eq!(v.get("Height"), "48");
        assert_eq!(v.get("FrameRate"), "25.000");
        assert_eq!(v.get("BitRate"), "50000");
        assert_eq!(v.get("Duration"), "1000");
        assert_eq!(v.get("StreamSize"), "6250");
        let a = &doc.streams[StreamKind::Audio as usize][0];
        assert_eq!(a.get("ID"), "1");
        assert_eq!(a.get("Format"), "AC-3");
        assert_eq!(a.get("CodecID"), "dnet");
        assert_eq!(a.get("SamplingRate"), "48000");
        assert_eq!(a.get("Channel(s)"), "1");
        assert_eq!(a.get("BitDepth"), "16");
        assert_eq!(a.get("StreamSize"), "8192");
        assert_eq!(a.get("Compression_Mode"), "Lossy");
    }

    #[test]
    fn ra5_header_and_truncation() {
        let mut ra = b".ra\xfd\x00\x05\x00\x00.ra5".to_vec();
        ra.resize(54, 0);
        ra.extend_from_slice(&44100u16.to_be_bytes());
        ra.extend_from_slice(&[0, 0, 0, 16, 0, 2]);
        ra.extend_from_slice(b"genr");
        ra.extend_from_slice(b"cook");
        let mut s = Stream::new(StreamKind::Audio);
        apply_audio(&mut s, &ra);
        assert_eq!(s.get("Format"), "Cook");
        assert_eq!(s.get("SamplingRate"), "44100");
        assert_eq!(s.get("Channel(s)"), "2");
        let mut s = Stream::new(StreamKind::Audio);
        apply_audio(&mut s, b".ra\xfd\x00\x04");
        assert!(!s.has("Format"));
        let mut f = build();
        f.truncate(60);
        let mut r = Reader::from_bytes(f);
        assert!(parse(&mut r, &mut Doc::new()));
    }
}

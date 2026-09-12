//! ISO Base Media / MP4 / QuickTime / 3GPP container.

use crate::io::{be16, be24, be32, be64, Reader};
use crate::model::{Doc, Stream, StreamKind, OPT_SHOWN};
use crate::parsers::audio::{self, aac, ac3, alac, dts, flac, mlp, mpeg_audio, opus, pcm, wma};
use crate::parsers::video::{self, av1, avc, h263, hevc, mpeg4v, mpegv, prores, vp9};
use crate::parsers::Probe;

pub fn probe(p: &Probe) -> u8 {
    if p.head.len() < 12 {
        return 0;
    }
    let size = be32(p.head, 0).unwrap_or(0);
    let kind = &p.head[4..8];
    match kind {
        b"ftyp" | b"moov" | b"mdat" | b"wide" | b"free" | b"skip" | b"pnot" | b"moof" | b"styp" | b"sidx" | b"uuid" | b"jP  " => {
            if kind == b"jP  " {
                return 0; // JPEG 2000 — handled by image::jp2
            }
            if kind == b"ftyp" && p.head.len() >= 12 && &p.head[8..12] == b"jp2 " {
                return 0;
            }
            if size >= 8 || size == 1 || size == 0 {
                if kind == b"ftyp" { 100 } else { 80 }
            } else {
                0
            }
        }
        _ => 0,
    }
}

#[derive(Debug, Default, Clone)]
struct Track {
    id: u32,
    handler: [u8; 4],
    handler_name: String,
    enabled: bool,
    in_movie: bool,
    alternate_group: u16,
    layer: i16,
    tk_width: f64,
    tk_height: f64,
    rotation: f64,
    timescale: u32,
    media_duration: u64,
    language: String,
    // sample description
    codec: [u8; 4],
    stsd_index_count: u32,
    width: u32,
    height: u32,
    depth: u16,
    frame_count_field: u16,
    compressor: String,
    channels: u32,
    sample_size: u32,
    sample_rate: f64,
    audio_version: u16,
    samples_per_packet: u32,
    bytes_per_frame: u32,
    lpcm_flags: u32,
    esds: Option<Esds>,
    avcc: Vec<u8>,
    hvcc: Vec<u8>,
    av1c: Vec<u8>,
    vpcc: Vec<u8>,
    d263: Vec<u8>,
    dac3: Vec<u8>,
    dec3: Vec<u8>,
    dops: Vec<u8>,
    dfla: Vec<u8>,
    alac: Vec<u8>,
    wave_extra: Vec<u8>,
    glbl: Vec<u8>,
    pasp: Option<(u32, u32)>,
    clap: Option<(f64, f64)>,
    colr: Option<(u16, u16, u16, bool)>,
    chan: Option<(u32, u32)>, // layout tag, bitmap
    btrt: Option<(u32, u32, u32)>,
    fiel: Option<(u8, u8)>,
    // sample tables
    stts: Vec<(u32, u32)>,
    stsz_default: u32,
    stsz: Vec<u32>,
    stsc: Vec<(u32, u32, u32)>,
    chunk_offsets: Vec<u64>,
    stss_count: usize,
    ctts_present: bool,
    elst: Vec<(i64, i64, u32)>, // (segment duration in movie ts, media time, rate)
    chap_refs: Vec<u32>,
    tmcd_refs: Vec<u32>,
    // fragments
    frag_samples: u64,
    frag_bytes: u64,
    frag_duration: u64,
    frag_min_dur: u32,
    frag_max_dur: u32,
    default_sample_duration: u32,
    default_sample_size: u32,
    frag_first_sample: Option<(u64, u32)>,
    first_sample_data: Option<Vec<u8>>,
    name: String,
    tmcd_frame_duration: u32,
    tmcd_timescale: u32,
    tmcd_fps: u8,
    tmcd_flags: u32,
    tmcd_first: Option<u32>,
    chapter_data: Vec<(u64, Vec<u8>)>,
    creation: u64,
    modification: u64,
}

#[derive(Debug, Default, Clone)]
struct Esds {
    object_type: u8,
    stream_type: u8,
    max_bitrate: u32,
    avg_bitrate: u32,
    decoder_specific: Vec<u8>,
}

#[derive(Debug, Default)]
struct Ctx {
    major_brand: String,
    minor_version: u32,
    compatible: Vec<String>,
    timescale: u32,
    duration: u64,
    fragment_duration: u64,
    creation: u64,
    modification: u64,
    tracks: Vec<Track>,
    tags: Vec<(String, String)>,
    chpl: Vec<(u64, String)>, // (ns, title)
    first_mdat_data: Option<u64>,
    mdat_bytes: u64,
    moov_pos: Option<u64>,
    moof_positions: Vec<u64>,
    has_moof: bool,
    xmp: bool,
}

const MAX_TABLE: usize = 8 << 20;

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let mut ctx = Ctx::default();
    let len = r.len();
    let mut pos = 0u64;
    let mut found_moov = false;
    let mut iter = 0;
    while pos + 8 <= len && iter < 1_000_000 {
        iter += 1;
        let Some((size, kind, hdr)) = box_header(r, pos, len) else { break };
        let end = pos + size;
        match &kind {
            b"ftyp" | b"styp" => {
                let d = r.read_vec_at(pos + hdr, (size - hdr).min(1024) as usize);
                if d.len() >= 8 {
                    ctx.major_brand = String::from_utf8_lossy(&d[0..4]).into_owned();
                    ctx.minor_version = be32(&d, 4).unwrap_or(0);
                    let mut i = 8;
                    while i + 4 <= d.len() {
                        let b = String::from_utf8_lossy(&d[i..i + 4]).into_owned();
                        if b.trim().is_empty() {
                            i += 4;
                            continue;
                        }
                        ctx.compatible.push(b);
                        i += 4;
                    }
                }
            }
            b"moov" => {
                found_moov = true;
                ctx.moov_pos = Some(pos);
                parse_moov(r, pos + hdr, end, &mut ctx);
            }
            b"moof" => {
                ctx.has_moof = true;
                ctx.moof_positions.push(pos);
                if ctx.moof_positions.len() <= 100_000 {
                    parse_moof(r, pos, pos + hdr, end, &mut ctx);
                }
            }
            b"mdat" => {
                if ctx.first_mdat_data.is_none() {
                    ctx.first_mdat_data = Some(pos);
                }
                ctx.mdat_bytes += size;
            }
            b"uuid" => {
                let d = r.read_vec_at(pos + hdr, 16);
                // XMP uuid
                if d == [0xBE, 0x7A, 0xCF, 0xCB, 0x97, 0xA9, 0x42, 0xE8, 0x9C, 0x71, 0x99, 0x94, 0x91, 0xE3, 0xAF, 0xAC] {
                    ctx.xmp = true;
                }
            }
            _ => {}
        }
        if size == 0 {
            break;
        }
        pos = end;
    }
    if !found_moov {
        return false;
    }
    load_first_samples(r, &mut ctx);
    emit(doc, &ctx, len);
    true
}

/// (size, type, header length); size 0 = to end of file; handles 64-bit sizes.
fn box_header(r: &mut Reader, pos: u64, len: u64) -> Option<(u64, [u8; 4], u64)> {
    let h = r.read_vec_at(pos, 16);
    if h.len() < 8 {
        return None;
    }
    let mut size = be32(&h, 0)? as u64;
    let kind = [h[4], h[5], h[6], h[7]];
    let mut hdr = 8;
    if size == 1 {
        size = be64(&h, 8)?;
        hdr = 16;
    } else if size == 0 {
        size = len - pos;
    }
    if size < hdr || pos + size > len {
        // Truncated box: clamp to file end so we still use what is there.
        if size < hdr {
            return None;
        }
        size = len - pos;
    }
    Some((size, kind, hdr))
}

fn children(r: &mut Reader, start: u64, end: u64) -> Vec<(u64, [u8; 4], u64, u64)> {
    // (payload start, type, payload end, box start)
    let mut out = Vec::new();
    let mut pos = start;
    let mut n = 0;
    while pos + 8 <= end && n < 100_000 {
        n += 1;
        let Some((size, kind, hdr)) = box_header(r, pos, end) else { break };
        if size == 0 {
            break;
        }
        out.push((pos + hdr, kind, (pos + size).min(end), pos));
        pos += size;
    }
    out
}

fn parse_moov(r: &mut Reader, start: u64, end: u64, ctx: &mut Ctx) {
    for (ps, kind, pe, _) in children(r, start, end) {
        match &kind {
            b"mvhd" => {
                let d = r.read_vec_at(ps, (pe - ps).min(120) as usize);
                let v = d.first().copied().unwrap_or(0);
                if v == 1 {
                    ctx.creation = be64(&d, 4).unwrap_or(0);
                    ctx.modification = be64(&d, 12).unwrap_or(0);
                    ctx.timescale = be32(&d, 20).unwrap_or(0);
                    ctx.duration = be64(&d, 24).unwrap_or(0);
                } else {
                    ctx.creation = be32(&d, 4).unwrap_or(0) as u64;
                    ctx.modification = be32(&d, 8).unwrap_or(0) as u64;
                    ctx.timescale = be32(&d, 12).unwrap_or(0);
                    ctx.duration = be32(&d, 16).unwrap_or(0) as u64;
                }
            }
            b"trak" => {
                let mut t = Track::default();
                parse_trak(r, ps, pe, &mut t, ctx);
                if ctx.tracks.len() < 10_000 {
                    ctx.tracks.push(t);
                }
            }
            b"mvex" => {
                for (cs, ck, ce, _) in children(r, ps, pe) {
                    match &ck {
                        b"mehd" => {
                            let d = r.read_vec_at(cs, 12);
                            ctx.fragment_duration = if d.first() == Some(&1) { be64(&d, 4).unwrap_or(0) } else { be32(&d, 4).unwrap_or(0) as u64 };
                        }
                        b"trex" => {
                            let d = r.read_vec_at(cs, (ce - cs).min(24) as usize);
                            let tid = be32(&d, 4).unwrap_or(0);
                            let dur = be32(&d, 12).unwrap_or(0);
                            let sz = be32(&d, 16).unwrap_or(0);
                            if let Some(t) = ctx.tracks.iter_mut().find(|t| t.id == tid) {
                                t.default_sample_duration = dur;
                                t.default_sample_size = sz;
                            }
                        }
                        _ => {}
                    }
                }
            }
            b"udta" => parse_udta(r, ps, pe, ctx),
            b"meta" => parse_meta(r, ps, pe, ctx),
            _ => {}
        }
    }
}

fn parse_trak(r: &mut Reader, start: u64, end: u64, t: &mut Track, ctx: &mut Ctx) {
    t.enabled = true;
    for (ps, kind, pe, _) in children(r, start, end) {
        match &kind {
            b"tkhd" => {
                let d = r.read_vec_at(ps, (pe - ps).min(104) as usize);
                let v = d.first().copied().unwrap_or(0);
                let flags = be24(&d, 1).unwrap_or(0);
                t.enabled = flags & 1 != 0;
                t.in_movie = flags & 2 != 0;
                let o = if v == 1 { 20 } else { 12 };
                if v == 1 {
                    t.creation = be64(&d, 4).unwrap_or(0);
                    t.modification = be64(&d, 12).unwrap_or(0);
                } else {
                    t.creation = be32(&d, 4).unwrap_or(0) as u64;
                    t.modification = be32(&d, 8).unwrap_or(0) as u64;
                }
                t.id = be32(&d, o).unwrap_or(0);
                let o2 = if v == 1 { 44 } else { 32 };
                t.layer = be16(&d, o2).unwrap_or(0) as i16;
                t.alternate_group = be16(&d, o2 + 2).unwrap_or(0);
                // matrix at o2+8: 9 × 32-bit fixed
                let m = o2 + 8;
                let a = be32(&d, m).unwrap_or(0x10000) as i32 as f64 / 65536.0;
                let b = be32(&d, m + 4).unwrap_or(0) as i32 as f64 / 65536.0;
                let c = be32(&d, m + 12).unwrap_or(0) as i32 as f64 / 65536.0;
                let dd = be32(&d, m + 16).unwrap_or(0x10000) as i32 as f64 / 65536.0;
                let _ = (c, dd);
                let mut rot = b.atan2(a).to_degrees();
                if rot < 0.0 {
                    rot += 360.0;
                }
                if rot.abs() < 0.001 {
                    rot = 0.0;
                }
                t.rotation = rot;
                t.tk_width = be32(&d, m + 36).unwrap_or(0) as f64 / 65536.0;
                t.tk_height = be32(&d, m + 40).unwrap_or(0) as f64 / 65536.0;
            }
            b"tref" => {
                for (cs, ck, ce, _) in children(r, ps, pe) {
                    let d = r.read_vec_at(cs, (ce - cs).min(256) as usize);
                    let ids: Vec<u32> = d.chunks_exact(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
                    match &ck {
                        b"chap" => t.chap_refs = ids,
                        b"tmcd" => t.tmcd_refs = ids,
                        _ => {}
                    }
                }
            }
            b"edts" => {
                for (cs, ck, ce, _) in children(r, ps, pe) {
                    if &ck == b"elst" {
                        let d = r.read_vec_at(cs, (ce - cs).min(4096) as usize);
                        let v = d.first().copied().unwrap_or(0);
                        let n = be32(&d, 4).unwrap_or(0) as usize;
                        let mut o = 8;
                        for _ in 0..n.min(256) {
                            if v == 1 {
                                let (Some(sd), Some(mt), Some(rate)) = (be64(&d, o), be64(&d, o + 8), be32(&d, o + 16)) else { break };
                                t.elst.push((sd as i64, mt as i64, rate));
                                o += 20;
                            } else {
                                let (Some(sd), Some(mt), Some(rate)) = (be32(&d, o), be32(&d, o + 4), be32(&d, o + 8)) else { break };
                                t.elst.push((sd as i64, mt as i32 as i64, rate));
                                o += 12;
                            }
                        }
                    }
                }
            }
            b"mdia" => parse_mdia(r, ps, pe, t, ctx),
            b"udta" => {
                for (cs, ck, ce, _) in children(r, ps, pe) {
                    if &ck == b"name" {
                        let d = r.read_vec_at(cs, (ce - cs).min(1024) as usize);
                        t.name = crate::io::clean_text(&String::from_utf8_lossy(&d));
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_mdia(r: &mut Reader, start: u64, end: u64, t: &mut Track, ctx: &mut Ctx) {
    for (ps, kind, pe, _) in children(r, start, end) {
        match &kind {
            b"mdhd" => {
                let d = r.read_vec_at(ps, (pe - ps).min(44) as usize);
                let v = d.first().copied().unwrap_or(0);
                let (ts, dur, lang_off) = if v == 1 { (be32(&d, 20), be64(&d, 24), 32) } else { (be32(&d, 12), be32(&d, 16).map(|x| x as u64), 20) };
                t.timescale = ts.unwrap_or(0);
                t.media_duration = dur.unwrap_or(0);
                if let Some(l) = be16(&d, lang_off) {
                    if l & 0x8000 == 0 && l != 0 {
                        // packed ISO 639-2
                        let c = [((l >> 10) & 0x1F) as u8 + 0x60, ((l >> 5) & 0x1F) as u8 + 0x60, (l & 0x1F) as u8 + 0x60];
                        let s = String::from_utf8_lossy(&c).into_owned();
                        if s != "und" && s.chars().all(|c| c.is_ascii_lowercase()) {
                            t.language = s;
                        }
                    } else if l < 0x400 {
                        // Macintosh language codes: 0 = English
                        if l == 0 {
                            t.language = "en".into();
                        }
                    }
                }
            }
            b"hdlr" => {
                let d = r.read_vec_at(ps, (pe - ps).min(256) as usize);
                if d.len() >= 12 {
                    t.handler = [d[8], d[9], d[10], d[11]];
                    if d.len() > 24 {
                        let name = &d[24..];
                        // Either a Pascal string (QuickTime) or a C string.
                        let s = if name.first().map(|&l| l as usize + 1 == name.len()).unwrap_or(false) { String::from_utf8_lossy(&name[1..]).into_owned() } else { crate::io::cstr(name) };
                        t.handler_name = crate::io::clean_text(&s);
                    }
                }
            }
            b"minf" => parse_minf(r, ps, pe, t, ctx),
            _ => {}
        }
    }
}

fn parse_minf(r: &mut Reader, start: u64, end: u64, t: &mut Track, ctx: &mut Ctx) {
    for (ps, kind, pe, _) in children(r, start, end) {
        if &kind == b"stbl" {
            parse_stbl(r, ps, pe, t, ctx);
        }
    }
}

fn parse_stbl(r: &mut Reader, start: u64, end: u64, t: &mut Track, ctx: &mut Ctx) {
    for (ps, kind, pe, _) in children(r, start, end) {
        match &kind {
            b"stsd" => parse_stsd(r, ps, pe, t, ctx),
            b"stts" => {
                let n = be32(&r.read_vec_at(ps + 4, 4), 0).unwrap_or(0) as usize;
                let d = r.read_vec_at(ps + 8, (n * 8).min(MAX_TABLE));
                for c in d.chunks_exact(8) {
                    t.stts.push((u32::from_be_bytes([c[0], c[1], c[2], c[3]]), u32::from_be_bytes([c[4], c[5], c[6], c[7]])));
                }
            }
            b"stsz" | b"stz2" => {
                let h = r.read_vec_at(ps, 12);
                if &kind == b"stsz" {
                    t.stsz_default = be32(&h, 4).unwrap_or(0);
                    let n = be32(&h, 8).unwrap_or(0) as usize;
                    if t.stsz_default == 0 {
                        let d = r.read_vec_at(ps + 12, (n * 4).min(MAX_TABLE * 4));
                        t.stsz = d.chunks_exact(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
                    } else {
                        t.stsz = Vec::new();
                        t.stsz_default = t.stsz_default.max(1);
                        // store count via stts; keep sample count separately
                        t.frag_samples = n as u64; // reused: constant-size sample count
                    }
                } else {
                    let field = h.get(7).copied().unwrap_or(0) as usize;
                    let n = be32(&h, 8).unwrap_or(0) as usize;
                    let d = r.read_vec_at(ps + 12, (n * field / 8 + 1).min(MAX_TABLE));
                    match field {
                        16 => t.stsz = d.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]]) as u32).collect(),
                        8 => t.stsz = d.iter().map(|&b| b as u32).collect(),
                        4 => {
                            for &b in &d {
                                t.stsz.push((b >> 4) as u32);
                                t.stsz.push((b & 15) as u32);
                            }
                            t.stsz.truncate(n);
                        }
                        _ => {}
                    }
                }
            }
            b"stsc" => {
                let n = be32(&r.read_vec_at(ps + 4, 4), 0).unwrap_or(0) as usize;
                let d = r.read_vec_at(ps + 8, (n * 12).min(MAX_TABLE));
                for c in d.chunks_exact(12) {
                    t.stsc.push((u32::from_be_bytes([c[0], c[1], c[2], c[3]]), u32::from_be_bytes([c[4], c[5], c[6], c[7]]), u32::from_be_bytes([c[8], c[9], c[10], c[11]])));
                }
            }
            b"stco" => {
                let n = be32(&r.read_vec_at(ps + 4, 4), 0).unwrap_or(0) as usize;
                let d = r.read_vec_at(ps + 8, (n * 4).min(MAX_TABLE));
                t.chunk_offsets = d.chunks_exact(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]) as u64).collect();
            }
            b"co64" => {
                let n = be32(&r.read_vec_at(ps + 4, 4), 0).unwrap_or(0) as usize;
                let d = r.read_vec_at(ps + 8, (n * 8).min(MAX_TABLE));
                t.chunk_offsets = d.chunks_exact(8).map(|c| u64::from_be_bytes(c.try_into().unwrap())).collect();
            }
            b"stss" => t.stss_count = be32(&r.read_vec_at(ps + 4, 4), 0).unwrap_or(0) as usize,
            b"ctts" => t.ctts_present = true,
            _ => {}
        }
    }
}

fn parse_stsd(r: &mut Reader, start: u64, end: u64, t: &mut Track, _ctx: &mut Ctx) {
    let h = r.read_vec_at(start, 8);
    t.stsd_index_count = be32(&h, 4).unwrap_or(0);
    let entries = children(r, start + 8, end);
    let Some((ps, kind, pe, _)) = entries.first().copied() else { return };
    t.codec = kind;
    let d = r.read_vec_at(ps, (pe - ps).min(4096) as usize);
    match &t.handler {
        b"vide" => {
            // Visual sample entry: 6 reserved + 2 dref, then 16 bytes pre-defined/reserved, width, height…
            t.width = be16(&d, 24).unwrap_or(0) as u32;
            t.height = be16(&d, 26).unwrap_or(0) as u32;
            t.frame_count_field = be16(&d, 40).unwrap_or(1);
            if let Some(name) = d.get(42..74) {
                let l = name[0] as usize;
                if l > 0 && l < 32 {
                    t.compressor = crate::io::clean_text(&String::from_utf8_lossy(&name[1..1 + l]));
                }
            }
            t.depth = be16(&d, 74).unwrap_or(24);
            parse_sample_entry_children(r, ps + 78, pe, t);
        }
        b"soun" => {
            t.audio_version = be16(&d, 8).unwrap_or(0);
            t.channels = be16(&d, 16).unwrap_or(0) as u32;
            t.sample_size = be16(&d, 18).unwrap_or(0) as u32;
            t.sample_rate = be32(&d, 24).unwrap_or(0) as f64 / 65536.0;
            let mut off = 28;
            if t.audio_version == 1 {
                t.samples_per_packet = be32(&d, 28).unwrap_or(0);
                t.bytes_per_frame = be32(&d, 36).unwrap_or(0);
                off = 44;
            } else if t.audio_version == 2 {
                if let Some(bits) = be64(&d, 32) {
                    t.sample_rate = f64::from_bits(bits);
                }
                t.channels = be32(&d, 40).unwrap_or(0);
                t.sample_size = be32(&d, 48).unwrap_or(0);
                t.lpcm_flags = be32(&d, 52).unwrap_or(0);
                t.bytes_per_frame = be32(&d, 56).unwrap_or(0);
                t.samples_per_packet = be32(&d, 60).unwrap_or(0);
                off = 64;
            }
            parse_sample_entry_children(r, ps + off as u64, pe, t);
        }
        _ => {
            // text, subtitle, tmcd, meta…
            if &t.handler == b"tmcd" || &kind == b"tmcd" {
                t.tmcd_flags = be32(&d, 12).unwrap_or(0);
                t.tmcd_timescale = be32(&d, 16).unwrap_or(0);
                t.tmcd_frame_duration = be32(&d, 20).unwrap_or(0);
                t.tmcd_fps = d.get(24).copied().unwrap_or(0);
            }
            parse_sample_entry_children(r, ps + 8, pe, t);
        }
    }
}

fn parse_sample_entry_children(r: &mut Reader, start: u64, end: u64, t: &mut Track) {
    for (cs, ck, ce, _) in children(r, start, end) {
        let d = r.read_vec_at(cs, (ce - cs).min(64 * 1024) as usize);
        match &ck {
            b"esds" => t.esds = parse_esds(&d),
            b"avcC" => t.avcc = d,
            b"hvcC" => t.hvcc = d,
            b"av1C" => t.av1c = d,
            b"vpcC" => t.vpcc = d,
            b"d263" => t.d263 = d,
            b"dac3" => t.dac3 = d,
            b"dec3" => t.dec3 = d,
            b"dOps" => t.dops = d,
            b"dfLa" => t.dfla = d,
            b"alac" => t.alac = d,
            b"glbl" => t.glbl = d,
            b"pasp" => t.pasp = Some((be32(&d, 0).unwrap_or(1), be32(&d, 4).unwrap_or(1))),
            b"clap" => {
                let (Some(wn), Some(wd), Some(hn), Some(hd)) = (be32(&d, 0), be32(&d, 4), be32(&d, 8), be32(&d, 12)) else { continue };
                if wd > 0 && hd > 0 {
                    t.clap = Some((wn as f64 / wd as f64, hn as f64 / hd as f64));
                }
            }
            b"colr" => {
                if d.len() >= 10 && (&d[0..4] == b"nclx" || &d[0..4] == b"nclc") {
                    let full = d.get(10).map(|b| b & 0x80 != 0).unwrap_or(false);
                    t.colr = Some((be16(&d, 4).unwrap_or(2), be16(&d, 6).unwrap_or(2), be16(&d, 8).unwrap_or(2), full));
                }
            }
            b"chan" => t.chan = Some((be32(&d, 4).unwrap_or(0), be32(&d, 8).unwrap_or(0))),
            b"btrt" => t.btrt = Some((be32(&d, 0).unwrap_or(0), be32(&d, 4).unwrap_or(0), be32(&d, 8).unwrap_or(0))),
            b"fiel" => t.fiel = Some((d.first().copied().unwrap_or(1), d.get(1).copied().unwrap_or(0))),
            b"wave" => {
                for (ws, wk, we, _) in children(r, cs, ce) {
                    let wd = r.read_vec_at(ws, (we - ws).min(4096) as usize);
                    match &wk {
                        b"esds" => t.esds = parse_esds(&wd),
                        b"alac" => t.alac = wd,
                        b"frma" | b"enda" | b"mp4a" => {}
                        _ => {
                            if t.wave_extra.is_empty() {
                                t.wave_extra = wd;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_esds(d: &[u8]) -> Option<Esds> {
    // version/flags then ES_Descriptor
    let mut i = 4;
    let (tag, len) = read_descr(d, &mut i)?;
    if tag != 0x03 {
        return None;
    }
    let es_end = (i + len).min(d.len());
    i += 2; // ES_ID
    let flags = *d.get(i)?;
    i += 1;
    if flags & 0x80 != 0 {
        i += 2;
    }
    if flags & 0x40 != 0 {
        let l = *d.get(i)? as usize;
        i += 1 + l;
    }
    if flags & 0x20 != 0 {
        i += 2;
    }
    let mut e = Esds::default();
    while i < es_end {
        let (tag, len) = read_descr(d, &mut i)?;
        let next = (i + len).min(d.len());
        if tag == 0x04 {
            e.object_type = *d.get(i)?;
            e.stream_type = d.get(i + 1)? >> 2;
            e.max_bitrate = be32(d, i + 5)?;
            e.avg_bitrate = be32(d, i + 9)?;
            let mut j = i + 13;
            while j < next {
                let (t2, l2) = read_descr(d, &mut j)?;
                if t2 == 0x05 {
                    e.decoder_specific = d.get(j..(j + l2).min(d.len()))?.to_vec();
                }
                j += l2;
            }
        }
        i = next;
    }
    Some(e)
}

fn read_descr(d: &[u8], i: &mut usize) -> Option<(u8, usize)> {
    let tag = *d.get(*i)?;
    *i += 1;
    let mut len = 0usize;
    for _ in 0..4 {
        let b = *d.get(*i)?;
        *i += 1;
        len = (len << 7) | (b & 0x7F) as usize;
        if b & 0x80 == 0 {
            break;
        }
    }
    Some((tag, len))
}

fn parse_udta(r: &mut Reader, start: u64, end: u64, ctx: &mut Ctx) {
    for (ps, kind, pe, _) in children(r, start, end) {
        let d = r.read_vec_at(ps, (pe - ps).min(64 * 1024) as usize);
        match &kind {
            b"meta" => parse_meta(r, ps, pe, ctx),
            b"chpl" => {
                // version(1) flags(3) reserved(1)? count(1)… Nero chapters: v1: 4 bytes version/flags, 4 bytes reserved?, 1 byte count
                let mut i = 4;
                let mut count = d.get(i + 4).copied().unwrap_or(0) as usize;
                if d.first() == Some(&1) {
                    i += 4;
                } else {
                    count = d.get(i).copied().unwrap_or(0) as usize;
                }
                i += 1;
                for _ in 0..count.min(4096) {
                    let Some(ts) = be64(&d, i) else { break };
                    let Some(&l) = d.get(i + 8) else { break };
                    let title = String::from_utf8_lossy(d.get(i + 9..i + 9 + l as usize).unwrap_or(&[])).into_owned();
                    ctx.chpl.push((ts * 100, title));
                    i += 9 + l as usize;
                }
            }
            b"\xa9nam" | b"\xa9ART" | b"\xa9alb" | b"\xa9day" | b"\xa9cmt" | b"\xa9gen" | b"\xa9too" | b"\xa9wrt" | b"\xa9cpy" | b"\xa9enc" | b"\xa9swr" | b"\xa9inf" | b"\xa9des" | b"\xa9mak" | b"\xa9mod" | b"\xa9prd" | b"\xa9src" | b"\xa9xyz" => {
                // QuickTime international text: size(2) lang(2) text
                let text = if d.len() >= 4 && be16(&d, 0).map(|l| l as usize + 4 == d.len()).unwrap_or(false) { String::from_utf8_lossy(&d[4..]).into_owned() } else { String::from_utf8_lossy(&d).into_owned() };
                let text = crate::io::clean_text(&text);
                if !text.is_empty() {
                    ctx.tags.push((crate::io::latin1(&kind), text));
                }
            }
            _ => {}
        }
    }
}

fn parse_meta(r: &mut Reader, start: u64, end: u64, ctx: &mut Ctx) {
    // Full box (version/flags) then hdlr, ilst...; some writers omit the version/flags.
    let probe = r.read_vec_at(start, 12);
    let start2 = if probe.get(4..8) == Some(b"hdlr") { start } else { start + 4 };
    for (ps, kind, pe, _) in children(r, start2, end) {
        if &kind == b"ilst" {
            for (is, ik, ie, _) in children(r, ps, pe) {
                let mut value = String::new();
                let mut kind_name = crate::io::latin1(&ik);
                for (ds, dk, de, _) in children(r, is, ie) {
                    let d = r.read_vec_at(ds, (de - ds).min(64 * 1024) as usize);
                    match &dk {
                        b"data" => {
                            let dtype = be32(&d, 0).unwrap_or(0) & 0xFFFFFF;
                            let payload = d.get(8..).unwrap_or(&[]);
                            value = match dtype {
                                1 => crate::io::clean_text(&String::from_utf8_lossy(payload)),
                                21 | 22 => match payload.len() {
                                    1 => payload[0].to_string(),
                                    2 => be16(payload, 0).unwrap_or(0).to_string(),
                                    4 => be32(payload, 0).unwrap_or(0).to_string(),
                                    8 => be64(payload, 0).unwrap_or(0).to_string(),
                                    _ => String::new(),
                                },
                                13 | 14 => "cover".to_string(),
                                _ => {
                                    if matches!(&ik, b"trkn" | b"disk") {
                                        let n = be16(payload, 2).unwrap_or(0);
                                        let total = be16(payload, 4).unwrap_or(0);
                                        if total > 0 { format!("{n}/{total}") } else { n.to_string() }
                                    } else if &ik == b"gnre" {
                                        let n = be16(payload, 0).unwrap_or(0);
                                        id3_genre(n.saturating_sub(1)).to_string()
                                    } else {
                                        String::new()
                                    }
                                }
                            };
                        }
                        b"name" => {
                            if &ik == b"----" {
                                kind_name = format!("----:{}", crate::io::cstr(d.get(4..).unwrap_or(&[])));
                            }
                        }
                        _ => {}
                    }
                }
                if !value.is_empty() {
                    ctx.tags.push((kind_name, value));
                }
            }
        }
    }
}

fn id3_genre(n: u16) -> &'static str {
    const G: &[&str] = &["Blues", "Classic Rock", "Country", "Dance", "Disco", "Funk", "Grunge", "Hip-Hop", "Jazz", "Metal", "New Age", "Oldies", "Other", "Pop", "R&B", "Rap", "Reggae", "Rock", "Techno", "Industrial", "Alternative", "Ska", "Death Metal", "Pranks", "Soundtrack", "Euro-Techno", "Ambient", "Trip-Hop", "Vocal", "Jazz+Funk", "Fusion", "Trance", "Classical", "Instrumental", "Acid", "House", "Game", "Sound Clip", "Gospel", "Noise", "AlternRock", "Bass", "Soul", "Punk", "Space", "Meditative", "Instrumental Pop", "Instrumental Rock", "Ethnic", "Gothic", "Darkwave", "Techno-Industrial", "Electronic", "Pop-Folk", "Eurodance", "Dream", "Southern Rock", "Comedy", "Cult", "Gangsta", "Top 40", "Christian Rap", "Pop/Funk", "Jungle", "Native American", "Cabaret", "New Wave", "Psychadelic", "Rave", "Showtunes", "Trailer", "Lo-Fi", "Tribal", "Acid Punk", "Acid Jazz", "Polka", "Retro", "Musical", "Rock & Roll", "Hard Rock"];
    G.get(n as usize).copied().unwrap_or("")
}

fn parse_moof(r: &mut Reader, moof_pos: u64, start: u64, end: u64, ctx: &mut Ctx) {
    for (ps, kind, pe, _) in children(r, start, end) {
        if &kind != b"traf" {
            continue;
        }
        let mut tid = 0u32;
        let mut base_offset: Option<u64> = None;
        let mut default_dur: Option<u32> = None;
        let mut default_size: Option<u32> = None;
        let mut default_base_is_moof = false;
        for (cs, ck, ce, _) in children(r, ps, pe) {
            let d = r.read_vec_at(cs, (ce - cs).min(1 << 20) as usize);
            match &ck {
                b"tfhd" => {
                    let flags = be24(&d, 1).unwrap_or(0);
                    tid = be32(&d, 4).unwrap_or(0);
                    let mut o = 8;
                    if flags & 1 != 0 {
                        base_offset = be64(&d, o);
                        o += 8;
                    }
                    if flags & 2 != 0 {
                        o += 4;
                    }
                    if flags & 8 != 0 {
                        default_dur = be32(&d, o);
                        o += 4;
                    }
                    if flags & 0x10 != 0 {
                        default_size = be32(&d, o);
                    }
                    default_base_is_moof = flags & 0x20000 != 0;
                }
                b"trun" => {
                    let Some(t) = ctx.tracks.iter_mut().find(|t| t.id == tid) else { continue };
                    let flags = be24(&d, 1).unwrap_or(0);
                    let count = be32(&d, 4).unwrap_or(0) as usize;
                    let mut o = 8;
                    let mut data_offset: Option<i32> = None;
                    if flags & 1 != 0 {
                        data_offset = be32(&d, o).map(|v| v as i32);
                        o += 4;
                    }
                    if flags & 4 != 0 {
                        o += 4;
                    }
                    let ddur = default_dur.unwrap_or(t.default_sample_duration);
                    let dsize = default_size.unwrap_or(t.default_sample_size);
                    let base = if default_base_is_moof || base_offset.is_none() { moof_pos } else { base_offset.unwrap() };
                    let first_off = (base as i64 + data_offset.unwrap_or(0) as i64).max(0) as u64;
                    let mut first = true;
                    for _ in 0..count.min(1 << 20) {
                        let mut dur = ddur;
                        let mut size = dsize;
                        if flags & 0x100 != 0 {
                            dur = be32(&d, o).unwrap_or(ddur);
                            o += 4;
                        }
                        if flags & 0x200 != 0 {
                            size = be32(&d, o).unwrap_or(dsize);
                            o += 4;
                        }
                        if flags & 0x400 != 0 {
                            o += 4;
                        }
                        if flags & 0x800 != 0 {
                            o += 4;
                        }
                        if first && t.frag_first_sample.is_none() {
                            t.frag_first_sample = Some((first_off, size));
                        }
                        first = false;
                        t.frag_samples += 1;
                        t.frag_bytes += size as u64;
                        t.frag_duration += dur as u64;
                        if dur > 0 {
                            t.frag_min_dur = if t.frag_min_dur == 0 { dur } else { t.frag_min_dur.min(dur) };
                            t.frag_max_dur = t.frag_max_dur.max(dur);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Read the first sample of each track (for codec probing: SEI, frame headers), all samples of
/// chapter text tracks, and the first sample of timecode tracks.
fn load_first_samples(r: &mut Reader, ctx: &mut Ctx) {
    let chapter_ids: Vec<u32> = ctx.tracks.iter().flat_map(|t| t.chap_refs.iter().copied()).collect();
    for t in ctx.tracks.iter_mut() {
        if chapter_ids.contains(&t.id) {
            for (off, size) in sample_locations(t, 10_000) {
                let data = r.read_vec_at(off, (size as usize).min(4096));
                t.chapter_data.push((off, data));
            }
        }
        if &t.handler == b"tmcd" {
            if let Some((off, _)) = sample_locations(t, 1).first() {
                t.tmcd_first = be32(&r.read_vec_at(*off, 4), 0);
            }
        }
        let (off, size) = if let Some(f) = t.frag_first_sample {
            f
        } else if let Some(&first_chunk) = t.chunk_offsets.first() {
            let size = if t.stsz_default > 0 { t.stsz_default } else { t.stsz.first().copied().unwrap_or(0) };
            (first_chunk, size)
        } else {
            continue;
        };
        if size == 0 {
            continue;
        }
        t.first_sample_data = Some(r.read_vec_at(off, (size as usize).min(1 << 20)));
    }
}

// ---------------------------------------------------------------------------- emit

fn track_sample_count(t: &Track) -> u64 {
    if t.frag_samples > 0 && t.stsz.is_empty() && t.stsz_default > 0 && !t.stts.is_empty() {
        // constant-size samples: frag_samples holds the stsz count
        return t.frag_samples;
    }
    let from_tables: u64 = t.stts.iter().map(|(c, _)| *c as u64).sum();
    if from_tables > 0 {
        from_tables
    } else {
        t.frag_samples
    }
}

fn track_total_bytes(t: &Track) -> u64 {
    if !t.stsz.is_empty() {
        t.stsz.iter().map(|&s| s as u64).sum()
    } else if t.stsz_default > 0 {
        t.stsz_default as u64 * track_sample_count(t)
    } else {
        t.frag_bytes
    }
}

fn emit(doc: &mut Doc, ctx: &Ctx, file_size: u64) {
    let g = doc.general();
    g.set("Format", "MPEG-4");
    let brand = ctx.major_brand.clone();
    let profile = match brand.as_str() {
        "isom" => "Base Media",
        "mp41" => "Base Media / Version 1",
        "mp42" => "Base Media / Version 2",
        "qt  " => "QuickTime",
        "M4A " => "Apple audio with iTunes info",
        "M4V " | "M4VH" | "M4VP" => "",
        "3gp4" => "3GPP Media Release 4",
        "3gp5" => "3GPP Media Release 5",
        "3gp6" => "3GPP Media Release 6",
        "3gp7" => "3GPP Media Release 7",
        "3g2a" => "3GPP2 Media Release A",
        "avc1" => "Base Media / AVC",
        "iso2" => "Base Media / Version 2",
        "iso4" => "Base Media / Version 4",
        "iso5" => "Base Media / Version 5",
        "iso6" => "Base Media / Version 6",
        "dash" => "Base Media / DASH",
        "mif1" | "heic" | "heix" => "HEIF",
        "f4v " => "Adobe Flash Video",
        "ismv" | "isml" => "Smooth Streaming",
        "MSNV" => "Sony PSP",
        "NDXP" | "NDSP" | "NDSC" | "NDSH" | "NDSM" | "NDSS" => "Nero",
        _ => "",
    };
    if !profile.is_empty() {
        g.set("Format_Profile", profile);
    }
    if !brand.is_empty() {
        g.set("CodecID", &brand);
        let compat = ctx.compatible.join("/");
        let version = if brand == "qt  " { format!("{:04X}.{:02X}", ctx.minor_version >> 16, (ctx.minor_version >> 8) & 0xFF) } else { String::new() };
        if !version.is_empty() {
            g.set("CodecID_Version", &version);
            g.set("CodecID/String", format!("{brand} {version} ({compat})"));
        } else {
            g.set("CodecID/String", format!("{brand} ({compat})"));
        }
        if !compat.is_empty() {
            g.set("CodecID_Compatible", compat);
        }
    }
    let ts = ctx.timescale.max(1) as f64;
    let mut duration_ms = if ctx.duration > 0 { ctx.duration as f64 / ts * 1000.0 } else { 0.0 };
    if ctx.has_moof {
        // Fragmented: movie duration comes from the fragments.
        let max = ctx.tracks.iter().map(|t| if t.timescale > 0 { t.frag_duration as f64 / t.timescale as f64 * 1000.0 } else { 0.0 }).fold(0.0, f64::max);
        if ctx.fragment_duration > 0 {
            duration_ms = ctx.fragment_duration as f64 / ts * 1000.0;
        } else if max > duration_ms {
            duration_ms = max;
        }
    }
    if duration_ms > 0.0 {
        g.set("Duration", format!("{}", duration_ms.round() as u64));
    }
    if ctx.creation > 2_082_844_800 {
        g.set("Encoded_Date", format!("UTC {}", crate::finish::format_datetime(ctx.creation as i64 - 2_082_844_800)));
    }
    if ctx.modification > 2_082_844_800 {
        g.set("Tagged_Date", format!("UTC {}", crate::finish::format_datetime(ctx.modification as i64 - 2_082_844_800)));
    }
    if let (Some(mdat), Some(moov)) = (ctx.first_mdat_data, ctx.moov_pos) {
        g.set("IsStreamable", if moov < mdat { "Yes" } else { "No" });
        g.set_int("HeaderSize", mdat as i128);
        g.set_int("DataSize", ctx.mdat_bytes as i128);
        g.set_int("FooterSize", file_size.saturating_sub(mdat + ctx.mdat_bytes) as i128);
    }
    for (k, v) in &ctx.tags {
        apply_tag(g, k, v);
    }
    let chapter_ids: Vec<u32> = ctx.tracks.iter().flat_map(|t| t.chap_refs.iter().copied()).collect();
    let all_bytes: u64 = ctx.tracks.iter().filter(|t| !chapter_ids.contains(&t.id)).map(|t| if !t.elst.is_empty() && t.media_duration > 0 { let pres = presentation_ms(t, ctx.timescale.max(1) as f64); if (t.media_duration as f64 / t.timescale.max(1) as f64 * 1000.0).round() != pres.round() { bytes_within(t, pres, 0.0) } else { track_total_bytes(t) } } else { track_total_bytes(t) }).sum();
    if all_bytes > 0 && all_bytes <= file_size {
        g.set_int("StreamSize", (file_size - all_bytes) as i128);
    }

    let movie_ts = ctx.timescale.max(1) as f64;
    let mut order = 0usize;
    let mut chapter_track_ids: Vec<u32> = ctx.tracks.iter().flat_map(|t| t.chap_refs.iter().copied()).collect();
    chapter_track_ids.sort();
    chapter_track_ids.dedup();

    for t in &ctx.tracks {
        let is_chapter_track = chapter_track_ids.contains(&t.id);
        let kind = match &t.handler {
            b"vide" => StreamKind::Video,
            b"soun" => StreamKind::Audio,
            b"text" | b"sbtl" | b"subt" | b"clcp" | b"subp" => {
                if is_chapter_track {
                    StreamKind::Menu
                } else {
                    StreamKind::Text
                }
            }
            b"tmcd" => StreamKind::Other,
            b"meta" | b"hint" | b"data" => continue,
            _ => continue,
        };
        let mut s = Stream::new(kind);
        s.set_int("StreamOrder", order as i128);
        order += 1;
        s.set_int("ID", t.id as i128);
        let track_ts = t.timescale.max(1) as f64;
        let mdhd_ms = t.media_duration as f64 / track_ts * 1000.0;
        // The sample table is the authority on the media duration; mdhd may disagree slightly.
        let stts_sum: u64 = t.stts.iter().map(|(c, d)| *c as u64 * *d as u64).sum();
        let media_ms = if stts_sum > 0 && !ctx.has_moof { stts_sum as f64 / track_ts * 1000.0 } else { mdhd_ms };
        if stts_sum > 0 && (mdhd_ms.round() - media_ms.round()).abs() >= 1.0 {
            s.set_extra("mdhd_Duration", format!("{}", mdhd_ms.round() as i64), "", OPT_SHOWN);
        }
        // Edit list → presentation duration & delays
        let mut pres_ms = media_ms;
        let mut source_delay_ms: Option<f64> = None;
        let mut delay_ms: Option<f64> = None;
        if !t.elst.is_empty() {
            let mut sum = 0.0;
            let mut empty = 0.0;
            for (sd, mt, _) in &t.elst {
                if *mt == -1 {
                    empty += *sd as f64 / movie_ts * 1000.0;
                } else {
                    sum += *sd as f64 / movie_ts * 1000.0;
                }
            }
            if sum > 0.0 {
                pres_ms = sum;
            }
            if let Some((_, mt, _)) = t.elst.iter().find(|(_, mt, _)| *mt >= 0) {
                if *mt > 0 {
                    source_delay_ms = Some(-(*mt as f64) / track_ts * 1000.0);
                }
            }
            if empty > 0.0 {
                delay_ms = Some(empty);
            }
        }
        if ctx.has_moof && t.frag_duration > 0 {
            pres_ms = if ctx.fragment_duration > 0 { ctx.fragment_duration as f64 / movie_ts * 1000.0 } else { t.frag_duration as f64 / track_ts * 1000.0 };
        }
        let sample_count = track_sample_count(t);
        let total_bytes = track_total_bytes(t);
        let media_ms_rounded = media_ms.round();
        if pres_ms > 0.0 {
            s.set("Duration", format!("{}", pres_ms.round() as i64));
        }
        if (media_ms_rounded - pres_ms.round()).abs() >= 1.0 && media_ms > 0.0 && !ctx.has_moof {
            s.set("Source_Duration", format!("{}", media_ms.floor() as i64));
            // Last frame shorter than the nominal frame duration?
            if let Some(diff) = last_frame_diff(t) {
                s.set("Source_Duration_LastFrame", format!("{}", diff.round() as i64));
            }
            if total_bytes > 0 && media_ms > 0.0 {
                s.set("Source_StreamSize", total_bytes.to_string());
                s.set("BitRate", format!("{}", (total_bytes as f64 * 8.0 * 1000.0 / media_ms).round() as u64));
                // Bytes of the samples that fall inside the presentation window.
                let inside = bytes_within(t, pres_ms, 0.0);
                s.set("StreamSize", inside.to_string());
            }
        } else if total_bytes > 0 {
            s.set("StreamSize", total_bytes.to_string());
            if ctx.has_moof && pres_ms > 0.0 {
                s.set("BitRate", format!("{}", (total_bytes as f64 * 8.0 * 1000.0 / pres_ms).round() as u64));
            }
            if let Some(diff) = last_frame_diff(t) {
                s.set("Duration_LastFrame", format!("{}", diff.round() as i64));
            }
        }
        if let Some(d) = delay_ms {
            s.set("Delay", format!("{}", d.round() as i64));
            s.set("Delay_Source", "Container");
        }
        if let Some(d) = source_delay_ms.filter(|_| kind == StreamKind::Audio) {
            s.set_extra("Source_Delay", format!("{}", d.round() as i64), "", "N NT");
            s.set_extra("Source_Delay_Source", "Container", "", "N NT");
        }
        if !t.language.is_empty() {
            s.set("Language", &t.language);
        }
        if t.creation > 2_082_844_800 {
            s.set("Encoded_Date", format!("UTC {}", crate::finish::format_datetime(t.creation as i64 - 2_082_844_800)));
        }
        if t.modification > 2_082_844_800 {
            s.set("Tagged_Date", format!("UTC {}", crate::finish::format_datetime(t.modification as i64 - 2_082_844_800)));
        }
        if !t.name.is_empty() {
            s.set("Title", &t.name);
        }
        if t.alternate_group > 0 {
            s.set_int("AlternateGroup", t.alternate_group as i128);
            let first_in_group = ctx.tracks.iter().find(|o| o.alternate_group == t.alternate_group && o.handler == t.handler).map(|o| o.id == t.id).unwrap_or(true);
            s.set_bool("Default", first_in_group && t.enabled);
        }
        if !t.enabled {
            s.set("Disabled", "Yes");
        }
        let codec = String::from_utf8_lossy(&t.codec).into_owned();
        match kind {
            StreamKind::Video => {
                s.set("CodecID", &codec);
                apply_video_codec(&mut s, t, &codec);
                if !s.has("Width") && t.width > 0 {
                    s.set("Width", t.width.to_string());
                    s.set("Height", t.height.to_string());
                } else if t.width > 0 && t.width != s.get_u64("Width").unwrap_or(0) as u32 {
                    // Container dimensions win (crop handled by codec parsers).
                    s.set("Width", t.width.to_string());
                    s.set("Height", t.height.to_string());
                }
                s.set("Rotation", format!("{:.3}", t.rotation));
                if let Some((h, v)) = t.pasp {
                    if h > 0 && v > 0 {
                        s.set("PixelAspectRatio", format!("{:.3}", h as f64 / v as f64));
                        s.clear("DisplayAspectRatio");
                    }
                } else if t.tk_width > 0.0 && t.tk_height > 0.0 && t.width > 0 && t.height > 0 && t.rotation == 0.0 {
                    let par = (t.tk_width / t.tk_height) / (t.width as f64 / t.height as f64);
                    if (par - 1.0).abs() > 0.001 && !s.has("PixelAspectRatio") {
                        s.set("PixelAspectRatio", format!("{par:.3}"));
                    }
                }
                if let Some((w, h)) = t.clap {
                    if w > 0.0 && h > 0.0 && (w as u32 != t.width || h as u32 != t.height) {
                        s.set("Width_CleanAperture", format!("{}", w.round() as u64));
                        s.set("Height_CleanAperture", format!("{}", h.round() as u64));
                    }
                }
                // Frame rate from stts
                let stream_mode = s.get("FrameRate_Mode").to_string();
                if sample_count > 0 && pres_ms > 0.0 {
                    let (cfr, fps, min_max) = if ctx.has_moof && t.frag_duration > 0 {
                        let cfr = t.frag_min_dur == t.frag_max_dur;
                        let fps = sample_count as f64 / (t.frag_duration as f64 / track_ts);
                        (cfr, fps, Some((track_ts / t.frag_max_dur.max(1) as f64, track_ts / t.frag_min_dur.max(1) as f64)))
                    } else {
                        let cfr = t.stts.len() <= 1 || t.stts.iter().all(|(_, d)| *d == t.stts[0].1) || (t.stts.len() == 2 && t.stts[1].0 == 1);
                        let fps = if cfr && !t.stts.is_empty() && t.stts[0].1 > 0 { track_ts / t.stts[0].1 as f64 } else if media_ms > 0.0 { sample_count as f64 / (media_ms / 1000.0) } else { 0.0 };
                        let mut durs: Vec<u32> = t.stts.iter().filter(|(c, d)| *c > 0 && *d > 0).map(|(_, d)| *d).collect();
                        durs.sort();
                        let mm = if durs.len() > 1 { Some((track_ts / *durs.last().unwrap() as f64, track_ts / durs[0] as f64)) } else { None };
                        (cfr, fps, mm)
                    };
                    if fps.is_finite() && fps > 0.0 {
                        s.set("FrameRate", format!("{fps:.3}"));
                        if cfr {
                            crate::finish::set_frame_rate_fraction(&mut s, fps);
                        }
                    }
                    s.set("FrameRate_Mode", if cfr { "CFR" } else { "VFR" });
                    if let (false, Some((min, max))) = (cfr, min_max) {
                        if min.is_finite() && max.is_finite() && min < max {
                            s.set("FrameRate_Minimum", format!("{min:.3}"));
                            s.set("FrameRate_Maximum", format!("{max:.3}"));
                        }
                    }
                    if !stream_mode.is_empty() && stream_mode != s.get("FrameRate_Mode") {
                        s.set("FrameRate_Mode_Original", stream_mode);
                    }
                    let frames = if (media_ms_rounded - pres_ms.round()).abs() >= 1.0 { (pres_ms / 1000.0 * fps).round() as u64 } else { sample_count };
                    s.set("FrameCount", frames.to_string());
                    if (media_ms_rounded - pres_ms.round()).abs() >= 1.0 && !ctx.has_moof {
                        s.set("Source_FrameCount", sample_count.to_string());
                    }
                }
                if let Some((p, tr, m, full)) = t.colr {
                    apply_colr(&mut s, p, tr, m, full);
                }
                if let Some((fields, order)) = t.fiel {
                    if fields == 2 {
                        s.set_if_empty("ScanType", "Interlaced");
                        match order {
                            1 | 6 => s.set_if_empty("ScanOrder", "TFF"),
                            9 | 14 => s.set_if_empty("ScanOrder", "BFF"),
                            _ => {}
                        }
                    } else if fields == 1 {
                        s.set_if_empty("ScanType", "Progressive");
                    }
                }
                if !t.compressor.is_empty() && !s.has("Encoded_Library") && !matches!(t.compressor.as_str(), "AVC Coding" | "HEVC Coding" | "H.264" | "H.265") {
                    // ffmpeg writes the codec name here; only keep meaningful vendor strings.
                    let _ = &t.compressor;
                }
            }
            StreamKind::Audio => {
                apply_audio_codec(&mut s, t, &codec, ctx);
                if sample_count > 0 && !ctx.has_moof {
                    if (media_ms_rounded - pres_ms.round()).abs() >= 1.0 {
                        let (n, _) = samples_within(t, pres_ms, 0.0);
                        s.set("FrameCount", n.to_string());
                        s.set("Source_FrameCount", sample_count.to_string());
                    } else {
                        s.set("FrameCount", sample_count.to_string());
                    }
                } else if ctx.has_moof && t.frag_samples > 0 {
                    s.set("FrameCount", t.frag_samples.to_string());
                }
                if !s.has("Channel(s)") && t.channels > 0 {
                    s.set("Channel(s)", t.channels.to_string());
                }
                if !s.has("SamplingRate") && t.sample_rate > 0.0 {
                    s.set("SamplingRate", format!("{}", t.sample_rate.round() as u64));
                }
                if let Some((tag, bitmap)) = t.chan {
                    if bitmap != 0 && !s.has("ChannelLayout") {
                        let (pos, layout) = audio::layout_from_mask(bitmap);
                        s.set("ChannelPositions", pos);
                        s.set("ChannelLayout", layout);
                    } else if tag != 0 && !s.has("ChannelLayout") {
                        let (pos, layout) = audio::layout_for_count(s.get_u64("Channel(s)").unwrap_or(0) as u32);
                        if !pos.is_empty() {
                            s.set("ChannelPositions", pos);
                            s.set("ChannelLayout", layout);
                        }
                    }
                }
                if !s.has("ChannelLayout") && s.get("Format") != "ALAC" {
                    let (pos, layout) = audio::layout_for_count(s.get_u64("Channel(s)").unwrap_or(0) as u32);
                    if !pos.is_empty() {
                        s.set("ChannelPositions", pos);
                        s.set("ChannelLayout", layout);
                    }
                }
                if s.get("Format") == "PCM" && !s.has("BitRate") {
                    if let (Some(sr), Some(ch), Some(bd)) = (s.get_f64("SamplingRate"), s.get_f64("Channel(s)"), s.get_f64("BitDepth")) {
                        s.set("BitRate", format!("{}", (sr * ch * bd) as u64));
                        s.set("BitRate_Mode", "CBR");
                    }
                }
                if s.get("Format") == "AAC" && (media_ms_rounded - pres_ms.round()).abs() < 1.0 && !ctx.has_moof {
                    // no edit list trimming: nothing more
                }
                if let Some(e) = &t.esds {
                    if e.avg_bitrate > 0 {
                        s.set("BitRate", e.avg_bitrate.to_string());
                        s.set_if_empty("BitRate_Mode", if e.max_bitrate == e.avg_bitrate { "CBR" } else { "VBR" });
                    }
                }
            }
            StreamKind::Text => {
                s.set("CodecID", &codec);
                let (format, extra) = match codec.as_str() {
                    "tx3g" => ("Timed Text", ""),
                    "text" => ("Timed Text", ""),
                    "c608" => ("EIA-608", ""),
                    "c708" => ("EIA-708", ""),
                    "wvtt" => ("WebVTT", ""),
                    "stpp" => ("TTML", ""),
                    "mp4s" => ("VobSub", ""),
                    _ => (codec.as_str(), ""),
                };
                s.set("Format", format);
                let _ = extra;
                if sample_count > 0 {
                    s.set_extra("Events_Total", sample_count.to_string(), "", OPT_SHOWN);
                }
            }
            StreamKind::Other => {
                s.set("Type", "Time code");
                s.set("Format", "QuickTime TC");
                if t.tmcd_frame_duration > 0 && t.tmcd_timescale > 0 {
                    s.set("FrameRate", format!("{:.3}", t.tmcd_timescale as f64 / t.tmcd_frame_duration as f64));
                }
                if let Some(first) = t.tmcd_first {
                    let fps = t.tmcd_fps.max(1) as u32;
                    let drop = t.tmcd_flags & 1 != 0;
                    let frames = first % fps;
                    let secs = first / fps;
                    s.set("TimeCode_FirstFrame", format!("{:02}:{:02}:{:02}{}{:02}", secs / 3600, (secs / 60) % 60, secs % 60, if drop { ';' } else { ':' }, frames));
                }
                s.set("TimeCode_Source", "Container");
            }
            StreamKind::Menu => {
                s.clear("StreamSize");
                s.clear("Disabled");
                s.set("CodecID", &codec);
                s.set("Format", "Timed Text");
                let users: Vec<String> = ctx.tracks.iter().filter(|o| o.chap_refs.contains(&t.id)).map(|o| o.id.to_string()).collect();
                let chapters = chapter_samples(t);
                let begin = s.schema_len();
                s.set_int("Chapters_Pos_Begin", begin as i128);
                s.set_int("Chapters_Pos_End", (begin + chapters.len()) as i128);
                if !users.is_empty() {
                    s.set_extra("Menu For", users.join(","), "", OPT_SHOWN);
                }
                for (ms, title) in chapters {
                    let name = crate::finish::format::duration_strings(ms, None)[3].clone();
                    s.push_extra(&name, title, OPT_SHOWN);
                }
            }
            _ => {}
        }
        if !t.chap_refs.is_empty() {
            s.set_extra("Menus", t.chap_refs.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","), "", OPT_SHOWN);
        }
        if kind == StreamKind::Video && !t.avcc.is_empty() {
            s.set_extra("CodecConfigurationBox", "avcC", "", OPT_SHOWN);
        } else if kind == StreamKind::Video && !t.hvcc.is_empty() {
            s.set_extra("CodecConfigurationBox", "hvcC", "", OPT_SHOWN);
        }
        doc.streams[kind as usize].push(s);
    }
    // Nero/Apple chapter list (chpl) → a plain Menu
    if !ctx.chpl.is_empty() {
        let mut m = Stream::new(StreamKind::Menu);
        let begin = m.schema_len();
        m.set_int("Chapters_Pos_Begin", begin as i128);
        m.set_int("Chapters_Pos_End", (begin + ctx.chpl.len()) as i128);
        for (ns, title) in &ctx.chpl {
            let name = crate::finish::format::duration_strings(*ns as f64 / 1_000_000.0, None)[3].clone();
            m.push_extra(&name, title.clone(), OPT_SHOWN);
        }
        doc.streams[StreamKind::Menu as usize].push(m);
    }
}

/// Timed text chapter samples: (ms, title) from the chapter track's sample table and data.
fn chapter_samples(t: &Track) -> Vec<(f64, String)> {
    let ts = t.timescale.max(1) as f64;
    let mut sample_times = Vec::new();
    let mut acc = 0u64;
    for (c, d) in &t.stts {
        for _ in 0..(*c).min(100_000) {
            sample_times.push(acc as f64 / ts * 1000.0);
            acc += *d as u64;
        }
    }
    let mut out = Vec::new();
    for (i, (_, data)) in t.chapter_data.iter().enumerate() {
        let Some(&tm) = sample_times.get(i) else { break };
        let len = be16(data, 0).unwrap_or(0) as usize;
        let text = data.get(2..2 + len.min(data.len().saturating_sub(2))).map(|b| crate::io::clean_text(&crate::io::utf16(b, true).chars().take_while(|c| *c != '\u{fffd}').collect::<String>())).unwrap_or_default();
        let text = if text.is_empty() || data.get(2..4) != Some(&[0xFE, 0xFF]) && data.get(2..4) != Some(&[0xFF, 0xFE]) { crate::io::clean_text(&String::from_utf8_lossy(data.get(2..2 + len.min(data.len().saturating_sub(2))).unwrap_or(&[]))) } else { text };
        out.push((tm, text));
    }
    out
}

/// (offset, size) of every sample, from stsc/stco/stsz (capped).
fn sample_locations(t: &Track, cap: usize) -> Vec<(u64, u32)> {
    let mut out = Vec::new();
    let mut sample_idx = 0usize;
    let n_chunks = t.chunk_offsets.len();
    for (ci, off) in t.chunk_offsets.iter().enumerate() {
        let spc = samples_per_chunk(&t.stsc, (ci + 1) as u32, n_chunks);
        let mut o = *off;
        for _ in 0..spc {
            let size = if !t.stsz.is_empty() { t.stsz.get(sample_idx).copied().unwrap_or(0) } else { t.stsz_default };
            out.push((o, size));
            o += size as u64;
            sample_idx += 1;
            if out.len() >= cap {
                return out;
            }
        }
    }
    out
}

fn samples_per_chunk(stsc: &[(u32, u32, u32)], chunk: u32, _n: usize) -> u32 {
    let mut spc = 0;
    for (first, count, _) in stsc {
        if *first <= chunk {
            spc = *count;
        } else {
            break;
        }
    }
    spc
}

/// Presentation duration (ms) after the edit list.
fn presentation_ms(t: &Track, movie_ts: f64) -> f64 {
    let media = t.media_duration as f64 / t.timescale.max(1) as f64 * 1000.0;
    let sum: f64 = t.elst.iter().filter(|(_, mt, _)| *mt != -1).map(|(sd, _, _)| *sd as f64 / movie_ts * 1000.0).sum();
    if sum > 0.0 { sum } else { media }
}

/// Difference between the last sample duration and the nominal one (ms), when the last one is shorter.
fn last_frame_diff(t: &Track) -> Option<f64> {
    let ts = t.timescale.max(1) as f64;
    let (last_count, last) = *t.stts.last()?;
    let (_, first) = *t.stts.first()?;
    if t.stts.len() == 2 && last_count == 1 && last < first {
        Some((last as f64 - first as f64) / ts * 1000.0)
    } else {
        None
    }
}

/// Sum of sample sizes whose start time lies within [start_ms, start_ms + window_ms).
fn bytes_within(t: &Track, window_ms: f64, start_ms: f64) -> u64 {
    samples_within(t, window_ms, start_ms).1
}

/// (count, bytes) of the samples whose start time lies within [start_ms, start_ms + window_ms).
fn samples_within(t: &Track, window_ms: f64, start_ms: f64) -> (u64, u64) {
    let ts = t.timescale.max(1) as f64;
    let mut acc = 0u64;
    let mut idx = 0usize;
    let mut total = 0u64;
    let mut count = 0u64;
    let end = start_ms + window_ms;
    for (c, d) in &t.stts {
        for _ in 0..*c {
            let tm = acc as f64 / ts * 1000.0;
            if tm >= start_ms - 0.001 && tm < end - 0.001 {
                total += if !t.stsz.is_empty() { t.stsz.get(idx).copied().unwrap_or(0) as u64 } else { t.stsz_default as u64 };
                count += 1;
            }
            acc += *d as u64;
            idx += 1;
        }
    }
    (count, total)
}

fn apply_colr(s: &mut Stream, p: u16, t: u16, m: u16, full: bool) {
    let mut present = false;
    let pn = video::colour::primaries(p as u8);
    if !pn.is_empty() {
        s.set_extra("colour_primaries", pn, "", "Y YTY");
        s.set_extra("colour_primaries_Source", "Container", "", "N YTY");
        present = true;
    }
    let tn = video::colour::transfer(t as u8);
    if !tn.is_empty() {
        s.set_extra("transfer_characteristics", tn, "", "Y YTY");
        s.set_extra("transfer_characteristics_Source", "Container", "", "N YTY");
        present = true;
    }
    let mn = video::colour::matrix(m as u8);
    if !mn.is_empty() {
        s.set_extra("matrix_coefficients", mn, "", "Y YTY");
        s.set_extra("matrix_coefficients_Source", "Container", "", "N YTY");
        present = true;
    }
    if present {
        s.set_extra("colour_description_present", "Yes", "", "N YTY");
        s.set_extra("colour_description_present_Source", "Container", "", "N YTY");
        s.set_extra("colour_range", if full { "Full" } else { "Limited" }, "", "Y YTY");
        let src = if s.get("colour_range_Source") == "Stream" { "Container / Stream" } else { "Container" };
        s.set_extra("colour_range_Source", src, "", "N YTY");
    }
}

fn apply_video_codec(s: &mut Stream, t: &Track, codec: &str) {
    let first = t.first_sample_data.as_deref().unwrap_or(&[]);
    match codec {
        "avc1" | "avc2" | "avc3" | "avc4" | "dvav" | "dva1" => {
            s.set("Format", "AVC");
            avc::apply_avcc(s, &t.avcc);
            if let Some((_, _, len)) = avc::parse_avcc(&t.avcc) {
                let nals = avc::nals_length_prefixed(first, len);
                avc::apply_sei_from_nals(s, &nals);
            }
        }
        "hvc1" | "hev1" | "dvh1" | "dvhe" | "hvt1" | "lhv1" => {
            s.set("Format", "HEVC");
            hevc::apply_hvcc(s, &t.hvcc);
            if let Some(len) = hevc::hvcc_length_size(&t.hvcc) {
                let nals = hevc::nals_length_prefixed(first, len);
                hevc::apply_sei_from_nals(s, &nals);
            }
        }
        "av01" => {
            s.set("Format", "AV1");
            if !av1::apply_av1c(s, &t.av1c) {
                av1::apply_obus(s, first);
            }
            av1::apply_obus_metadata(s, first);
        }
        "vp08" => {
            s.set("Format", "VP8");
            video::vp8::apply_frame(s, first);
        }
        "vp09" => {
            s.set("Format", "VP9");
            vp9::apply_vpcc(s, &t.vpcc);
            vp9::apply_frame(s, first);
        }
        "mp4v" | "xvid" | "XVID" | "DIVX" | "DX50" | "FMP4" | "3IV2" => {
            let oti = t.esds.as_ref().map(|e| e.object_type).unwrap_or(0x20);
            match oti {
                0x20 => {
                    s.set("Format", "MPEG-4 Visual");
                    s.set("CodecID", format!("{codec}-{oti:02X}"));
                    let dsi = t.esds.as_ref().map(|e| e.decoder_specific.clone()).unwrap_or_default();
                    if !mpeg4v::apply_headers(s, &dsi) {
                        mpeg4v::apply_headers(s, first);
                    }
                    mpeg4v::apply_frame_user_data(s, first);
                }
                0x60..=0x65 => {
                    s.set("Format", "MPEG Video");
                    s.set("CodecID", format!("{codec}-{oti:02X}"));
                    mpegv::apply_headers(s, first);
                }
                0x6A => {
                    s.set("Format", "MPEG Video");
                    s.set("CodecID", format!("{codec}-{oti:02X}"));
                    mpegv::apply_headers(s, first);
                }
                0x6C => {
                    s.set("Format", "JPEG");
                    s.set("CodecID", format!("{codec}-{oti:02X}"));
                }
                0x21 => {
                    s.set("Format", "AVC");
                    s.set("CodecID", format!("{codec}-{oti:02X}"));
                }
                _ => {
                    s.set("Format", "MPEG-4 Visual");
                    s.set("CodecID", format!("{codec}-{oti:02X}"));
                    mpeg4v::apply_headers(s, first);
                }
            }
            if let Some(e) = &t.esds {
                if e.max_bitrate > 0 && e.max_bitrate != e.avg_bitrate {
                    s.set("BitRate_Maximum", e.max_bitrate.to_string());
                    s.set("BitRate_Mode", "VBR");
                }
            }
        }
        "s263" | "h263" => {
            s.set("Format", "H.263");
            h263::apply_frame(s, first);
            if t.d263.len() >= 7 {
                // vendor(4) decoder_version(1) level(1) profile(1)
                let level = t.d263[5];
                let profile = t.d263[6];
                let pname = match profile {
                    0 => "BaseLine",
                    1 => "H.320 Coding",
                    2 => "Version 1 Backward Compatibility",
                    3 => "Version 2 Interactive and Streaming Wireless",
                    4 => "Version 3 Interactive and Streaming Wireless",
                    5 => "Conversational High Compression",
                    6 => "Conversational Internet",
                    7 => "Conversational Interlace",
                    8 => "High Latency",
                    _ => "",
                };
                if !pname.is_empty() {
                    s.set("Format_Profile", format!("{pname}@{}.{}", level / 10, level % 10));
                }
                let vendor = String::from_utf8_lossy(&t.d263[0..4]).into_owned();
                let ver = t.d263[4];
                if vendor.trim().eq_ignore_ascii_case("FFMP") {
                    s.set("Encoded_Library", format!("FFMpeg {ver}"));
                }
            }
        }
        "mjpa" | "mjpb" | "jpeg" | "JPEG" | "mjpg" | "MJPG" | "dmb1" | "AVDJ" => {
            s.set("Format", "JPEG");
            video::mjpeg::apply_frame(s, first);
        }
        "apch" | "apcn" | "apcs" | "apco" | "ap4h" | "ap4x" | "aprn" | "aprh" => {
            s.set("Format", "ProRes");
            prores::apply_frame(s, first);
            s.set_if_empty(
                "Format_Profile",
                match codec {
                    "apch" => "422 HQ",
                    "apcn" => "422",
                    "apcs" => "422 LT",
                    "apco" => "422 Proxy",
                    "ap4h" => "4444",
                    "ap4x" => "4444 XQ",
                    "aprn" => "RAW",
                    "aprh" => "RAW HQ",
                    _ => "",
                },
            );
        }
        "mp2v" | "mpeg" | "MPEG" | "m2v1" | "xd54" | "xd55" | "xd59" | "xd5a" | "xd5b" | "xd5c" | "xd5d" | "xd5e" | "xd5f" | "xdv1" | "xdv2" | "xdv3" | "xdvb" | "xdvc" | "xdvd" | "xdve" | "xdvf" | "hdv1" | "hdv2" | "hdv3" | "hdv5" | "hdv6" | "hdv7" | "hdv8" | "hdv9" => {
            s.set("Format", "MPEG Video");
            mpegv::apply_headers(s, first);
        }
        "dvc " | "dvcp" | "dvpp" | "dv5n" | "dv5p" | "dvhq" | "dvhp" | "dvh5" | "dvh6" | "dvhd" | "dvsd" | "DVSD" => {
            s.set("Format", "DV");
            video::dv::apply_frame(s, first);
        }
        "raw " | "yuv2" | "2vuy" | "v210" | "v308" | "v408" | "v410" | "r210" | "rgb " => {
            s.set("Format", if matches!(codec, "raw " | "rgb " | "r210") { "RGB" } else { "YUV" });
            s.set("BitDepth", if t.depth > 0 { (t.depth / 3).max(8).to_string() } else { "8".into() });
        }
        "png " => s.set("Format", "PNG"),
        "cvid" => s.set("Format", "Cinepak"),
        "SVQ1" => s.set("Format", "Sorenson Video 1"),
        "SVQ3" => s.set("Format", "Sorenson Video 3"),
        "rle " => s.set("Format", "RLE"),
        "smc " => s.set("Format", "Graphics"),
        "vc-1" => {
            s.set("Format", "VC-1");
            video::vc1::apply_sequence(s, first);
        }
        "ac16" | "ac32" | "ac48" | "ac24" => s.set("Format", "Apple Intermediate Codec"),
        "icod" => s.set("Format", "Apple Intermediate Codec"),
        _ => {
            if let Some((f, _, _)) = video::fourcc::fourcc_format(codec) {
                s.set("Format", f);
            } else {
                s.set("Format", codec.trim());
            }
        }
    }
}

fn apply_audio_codec(s: &mut Stream, t: &Track, codec: &str, _ctx: &Ctx) {
    let first = t.first_sample_data.as_deref().unwrap_or(&[]);
    match codec {
        "mp4a" => {
            let oti = t.esds.as_ref().map(|e| e.object_type).unwrap_or(0x40);
            let dsi = t.esds.as_ref().map(|e| e.decoder_specific.clone()).unwrap_or_default();
            match oti {
                0x40 | 0x66 | 0x67 | 0x68 => {
                    s.set("Format", "AAC");
                    let aot = aac::apply_asc(s, &dsi);
                    if oti == 0x40 {
                        if let Some(a) = aot {
                            s.set("CodecID", format!("mp4a-40-{a}"));
                        } else {
                            s.set("CodecID", "mp4a-40");
                        }
                    } else {
                        s.set("CodecID", format!("mp4a-{oti:02X}"));
                        if aot.is_none() {
                            s.set("Format_AdditionalFeatures", "LC");
                            s.set("SamplesPerFrame", "1024");
                        }
                    }
                    s.set("Compression_Mode", "Lossy");
                }
                0x69 | 0x6B => {
                    s.set("Format", "MPEG Audio");
                    s.set("CodecID", format!("mp4a-{oti:02X}"));
                    mpeg_audio::apply_frame(s, first);
                    s.set("Compression_Mode", "Lossy");
                }
                0xA5 => {
                    s.set("Format", "AC-3");
                    s.set("CodecID", format!("mp4a-{oti:02X}"));
                    ac3::apply_frame(s, first);
                }
                0xA6 => {
                    s.set("Format", "E-AC-3");
                    s.set("CodecID", format!("mp4a-{oti:02X}"));
                    ac3::apply_frame(s, first);
                }
                0xA9 | 0xAA | 0xAB | 0xAC => {
                    s.set("Format", "DTS");
                    s.set("CodecID", format!("mp4a-{oti:02X}"));
                    dts::apply_frame(s, first);
                }
                0xDD => {
                    s.set("Format", "Vorbis");
                    s.set("CodecID", format!("mp4a-{oti:02X}"));
                }
                0xE1 => {
                    s.set("Format", "QCELP");
                    s.set("CodecID", format!("mp4a-{oti:02X}"));
                }
                _ => {
                    s.set("Format", "AAC");
                    s.set("CodecID", format!("mp4a-{oti:02X}"));
                    aac::apply_asc(s, &dsi);
                }
            }
            if let Some(e) = &t.esds {
                if e.max_bitrate > 0 && e.avg_bitrate > 0 && e.max_bitrate != e.avg_bitrate {
                    s.set("BitRate_Maximum", e.max_bitrate.to_string());
                }
            }
        }
        "ac-3" | "sac3" => {
            s.set("Format", "AC-3");
            s.set("CodecID", codec);
            ac3::apply_frame(s, first);
        }
        "ec-3" => {
            s.set("Format", "E-AC-3");
            s.set("CodecID", codec);
            ac3::apply_frame(s, first);
        }
        "dtsc" | "dtsh" | "dtsl" | "dtse" | "dts+" | "dts-" => {
            s.set("Format", "DTS");
            s.set("CodecID", codec);
            dts::apply_frame(s, first);
        }
        "mlpa" => {
            s.set("Format", "MLP FBA");
            s.set("CodecID", codec);
            mlp::apply_frame(s, first);
        }
        "alac" => {
            s.set("Format", "ALAC");
            s.set("CodecID", codec);
            alac::apply_cookie(s, &t.alac);
        }
        "Opus" => {
            s.set("Format", "Opus");
            s.set("CodecID", codec);
            opus::apply_head(s, &t.dops);
            s.set("Compression_Mode", "Lossy");
        }
        "fLaC" => {
            s.set("Format", "FLAC");
            s.set("CodecID", codec);
            flac::apply_streaminfo_block(s, t.dfla.get(4..).unwrap_or(&[]));
            s.set("Compression_Mode", "Lossless");
        }
        ".mp3" | "mp3 " => {
            s.set("Format", "MPEG Audio");
            s.set("CodecID", codec);
            mpeg_audio::apply_frame(s, first);
        }
        "samr" | "sawb" | "sawp" => {
            s.set("Format", "AMR");
            s.set("CodecID", codec);
            s.set("Format_Profile", if codec == "samr" { "Narrow band" } else { "Wide band" });
            s.set("Compression_Mode", "Lossy");
        }
        "sowt" | "twos" | "lpcm" | "in24" | "in32" | "fl32" | "fl64" | "raw " | "NONE" | "ulaw" | "alaw" | "ipcm" | "fpcm" => {
            s.set("CodecID", codec);
            let bits = if t.sample_size > 0 { t.sample_size } else { 16 };
            let (little, signed, float) = match codec {
                "sowt" => (Some(true), Some(true), false),
                "twos" => (Some(false), Some(true), false),
                "in24" | "in32" => (Some(false), Some(true), false),
                "fl32" | "fl64" => (Some(false), None, true),
                "raw " | "NONE" => (None, Some(bits > 8), false),
                "lpcm" => (Some(t.lpcm_flags & 2 == 0), Some(t.lpcm_flags & 4 != 0), t.lpcm_flags & 1 != 0),
                "ipcm" => (Some(true), Some(true), false),
                "fpcm" => (Some(true), None, true),
                _ => (None, None, false),
            };
            if matches!(codec, "ulaw" | "alaw") {
                s.set("Format", "PCM");
                s.set("Format_Settings_Law", if codec == "ulaw" { "µ-Law" } else { "A-Law" });
                s.set("BitDepth", "8");
            } else {
                s.set("Format", "PCM");
                pcm::apply_pcm(s, little, signed, float, bits);
                s.set("BitDepth", bits.to_string());
            }
            s.set("BitRate_Mode", "CBR");
        }
        "ima4" => {
            s.set("Format", "ADPCM");
            s.set("CodecID", codec);
            s.set("Format_Profile", "IMA");
        }
        "ms\0\x11" => {
            s.set("Format", "ADPCM");
            s.set("CodecID", codec);
        }
        "QDM2" | "QDMC" => {
            s.set("Format", "QDesign");
            s.set("CodecID", codec);
        }
        "WMA2" | "wma " => {
            s.set("Format", "WMA");
            s.set("CodecID", codec);
            wma::apply_waveformatex(s, &t.wave_extra);
        }
        "ms\0U" => {
            s.set("Format", "MPEG Audio");
            s.set("CodecID", codec);
            mpeg_audio::apply_frame(s, first);
        }
        _ => {
            s.set("CodecID", codec);
            s.set("Format", codec.trim());
        }
    }
    if t.sample_size > 0 && s.get("Format") == "PCM" && !s.has("BitDepth") {
        s.set("BitDepth", t.sample_size.to_string());
    }
}

fn apply_tag(g: &mut Stream, key: &str, value: &str) {
    let field = match key {
        "\u{a9}nam" | "\u{a9}NAM" => "Title",
        "\u{a9}ART" | "\u{a9}art" => "Performer",
        "aART" => "Album/Performer",
        "\u{a9}alb" | "\u{a9}ALB" => "Album",
        "\u{a9}day" | "\u{a9}DAY" => "Recorded_Date",
        "\u{a9}cmt" | "\u{a9}CMT" => "Comment",
        "\u{a9}gen" | "gnre" | "\u{a9}GEN" => "Genre",
        "\u{a9}too" | "\u{a9}TOO" | "\u{a9}swr" => "Encoded_Application",
        "\u{a9}enc" => "EncodedBy",
        "\u{a9}wrt" | "\u{a9}WRT" => "Composer",
        "\u{a9}cpy" | "cprt" | "\u{a9}CPY" => "Copyright",
        "\u{a9}lyr" => "Lyrics",
        "\u{a9}grp" => "Grouping",
        "\u{a9}des" | "desc" | "\u{a9}inf" => "Description",
        "ldes" => "Description",
        "\u{a9}prd" => "Producer",
        "\u{a9}dir" => "Director",
        "trkn" => "Track/Position",
        "disk" => "Part/Position",
        "tmpo" => "BPM",
        "cpil" => "Compilation",
        "covr" => "Cover",
        "tvsh" => "Collection",
        "tvsn" => "Season",
        "tves" => "Part",
        "tven" => "Season_Position",
        "purd" => "Added_Date",
        "sonm" => "Title/Sort",
        "soar" => "Performer/Sort",
        "soal" => "Album/Sort",
        "soco" => "Composer/Sort",
        "keyw" => "Keywords",
        "catg" => "PodcastCategory",
        "rtng" => "LawRating",
        "\u{a9}mak" => "Encoded_Application_CompanyName",
        "\u{a9}mod" => "Encoded_Hardware",
        "\u{a9}xyz" => "Recorded_Location",
        _ => {
            if let Some(rest) = key.strip_prefix("----:") {
                let name = rest.rsplit(':').next().unwrap_or(rest);
                if !name.is_empty() {
                    g.set_extra(name, value, "", OPT_SHOWN);
                }
            }
            return;
        }
    };
    match field {
        "Track/Position" | "Part/Position" => {
            if let Some((n, total)) = value.split_once('/') {
                g.set(field, n);
                g.set(&format!("{field}_Total"), total);
            } else {
                g.set(field, value);
            }
        }
        "Cover" => g.set("Cover", "Yes"),
        "Compilation" => g.set("Compilation", if value == "1" { "Yes" } else { "No" }),
        "Title" => {
            g.set("Title", value);
            g.set("Movie", value);
        }
        _ => g.set(field, value),
    }
}

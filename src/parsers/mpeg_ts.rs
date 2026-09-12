//! MPEG transport stream (ISO/IEC 13818-1) with 188/192 (BDAV, 4-byte TP_extra_header)/204-byte packets:
//! PAT → PMT → elementary streams, descriptors, SDT service names, PES reassembly, PCR timing.

use crate::io::{be16, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::audio::{aac, ac3, dts, mlp, mpeg_audio, pcm};
use crate::parsers::mpeg_ps::{apply_timing, apply_video_codec, apply_video_delay, parse_pes, video_tick_ms};
use crate::parsers::Probe;

const HEAD_SCAN: u64 = 8 * 1024 * 1024;
const TAIL_SCAN: u64 = 2 * 1024 * 1024;
/// Payload kept per PID for the codec helpers.
const CODEC_BYTES: usize = 256 * 1024;
/// Buffer cap for one PES packet.
const PES_CAP: usize = 1024 * 1024;
const MAX_PIDS: usize = 512;
const MAX_PROGRAMS: usize = 256;
const SECTION_CAP: usize = 4096;

const PID_PAT: u16 = 0;
const PID_SDT: u16 = 0x11;
const PID_NULL: u16 = 0x1FFF;

// ---------------------------------------------------------------------------- probe

/// Detect the packet size and the offset of the first sync byte: (packet size, offset).
pub fn detect_packet_size(head: &[u8]) -> Option<(usize, usize)> {
    for &ps in &[188usize, 192, 204] {
        let lead = if ps == 192 { 4 } else { 0 };
        let max_start = ps.min(head.len());
        for start in 0..max_start {
            let sync = start + lead;
            if head.get(sync) != Some(&0x47) {
                continue;
            }
            let mut n = 0;
            let mut ok = true;
            for k in 1..8 {
                let pos = sync + k * ps;
                if pos >= head.len() {
                    break;
                }
                if head[pos] != 0x47 {
                    ok = false;
                    break;
                }
                n += 1;
            }
            if ok && n >= 2 {
                return Some((ps, start));
            }
        }
    }
    None
}

pub fn probe(p: &Probe) -> u8 {
    let Some((ps, start)) = detect_packet_size(p.head) else { return 0 };
    let exts = ["ts", "m2ts", "mts", "m2t", "tp", "trp", "ssif", "tsv", "tsa", "m2s", "m4t", "m4s", "tmf", "ty"];
    match (start == 0, p.ext_in(&exts)) {
        (true, true) => 100,
        (true, false) => {
            if ps == 188 {
                90
            } else {
                85
            }
        }
        (false, true) => 80,
        (false, false) => 50,
    }
}

// ---------------------------------------------------------------------------- model

#[derive(Debug, Default, Clone)]
struct Descriptors {
    language: String,
    audio_type: u8,
    registration: String,
    ac3: bool,
    eac3: bool,
    dts: bool,
    teletext: bool,
    dvb_subtitle: Option<String>,
    component_tag: Option<u8>,
}

#[derive(Debug, Default, Clone)]
struct Pid {
    pid: u16,
    stream_type: u8,
    program: Option<usize>,
    desc: Descriptors,
    // PES reassembly
    cur: Vec<u8>,
    cur_total: usize,
    cur_active: bool,
    pes_count: u64,
    first_pts: Option<u64>,
    last_pts: Option<u64>,
    bytes: u64,
    packets: u64,
    data: Vec<u8>,
}

#[derive(Debug, Default, Clone)]
struct Program {
    number: u16,
    pmt_pid: u16,
    pcr_pid: u16,
    es: Vec<u16>,
    pointer_field: u8,
    section_length: u16,
    pmt_seen: bool,
    service_type: u8,
    provider: String,
    name: String,
}

#[derive(Debug, Default)]
struct Ctx {
    packet_size: usize,
    tsid: Option<u16>,
    programs: Vec<Program>,
    pids: Vec<Pid>,
    sections: Vec<(u16, Vec<u8>)>,
    first_pcr: Option<(u64, u64)>, // (27 MHz value, byte position)
    last_pcr: Option<(u64, u64)>,
    complete: bool,
}

impl Ctx {
    fn pid(&mut self, pid: u16) -> Option<&mut Pid> {
        if let Some(i) = self.pids.iter().position(|p| p.pid == pid) {
            return self.pids.get_mut(i);
        }
        if self.pids.len() >= MAX_PIDS {
            return None;
        }
        self.pids.push(Pid { pid, ..Default::default() });
        self.pids.last_mut()
    }

    fn is_pmt_pid(&self, pid: u16) -> bool {
        self.programs.iter().any(|p| p.pmt_pid == pid)
    }
}

// ---------------------------------------------------------------------------- sections

fn parse_descriptors(data: &[u8], d: &mut Descriptors) {
    let mut p = 0;
    while p + 2 <= data.len() {
        let tag = data[p];
        let len = data[p + 1] as usize;
        let Some(body) = data.get(p + 2..p + 2 + len) else { break };
        match tag {
            0x05 => {
                if body.len() >= 4 {
                    d.registration = body[..4].iter().map(|&c| if c.is_ascii_graphic() || c == b' ' { c as char } else { '?' }).collect();
                }
            }
            0x0A => {
                if body.len() >= 4 {
                    d.language = String::from_utf8_lossy(&body[..3]).to_string();
                    d.audio_type = body[3];
                }
            }
            0x6A | 0x81 => d.ac3 = true,
            0x7A => d.eac3 = true,
            0x7B => d.dts = true,
            0x56 => d.teletext = true,
            0x59 => {
                if body.len() >= 3 {
                    d.dvb_subtitle = Some(String::from_utf8_lossy(&body[..3]).to_string());
                }
            }
            0x52 => d.component_tag = body.first().copied(),
            _ => {}
        }
        p += 2 + len;
    }
}

fn parse_pat(sec: &[u8], ctx: &mut Ctx) {
    if sec.len() < 12 || sec[0] != 0 {
        return;
    }
    ctx.tsid = be16(sec, 3);
    let end = sec.len() - 4;
    let mut p = 8;
    while p + 4 <= end {
        let number = be16(sec, p).unwrap_or(0);
        let pid = be16(sec, p + 2).unwrap_or(0) & 0x1FFF;
        p += 4;
        if number == 0 {
            continue; // network PID
        }
        if ctx.programs.iter().any(|x| x.number == number) || ctx.programs.len() >= MAX_PROGRAMS {
            continue;
        }
        ctx.programs.push(Program { number, pmt_pid: pid, ..Default::default() });
    }
}

fn parse_pmt(sec: &[u8], pointer_field: u8, ctx: &mut Ctx) {
    if sec.len() < 16 || sec[0] != 2 {
        return;
    }
    let number = be16(sec, 3).unwrap_or(0);
    let Some(pi) = ctx.programs.iter().position(|p| p.number == number) else { return };
    if ctx.programs[pi].pmt_seen {
        return;
    }
    let section_length = be16(sec, 1).unwrap_or(0) & 0x0FFF;
    let pcr_pid = be16(sec, 8).unwrap_or(0) & 0x1FFF;
    let info_len = (be16(sec, 10).unwrap_or(0) & 0x0FFF) as usize;
    let mut p = 12 + info_len;
    let end = sec.len() - 4;
    let mut es = Vec::new();
    while p + 5 <= end {
        let stream_type = sec[p];
        let pid = be16(sec, p + 1).unwrap_or(0) & 0x1FFF;
        let es_len = (be16(sec, p + 3).unwrap_or(0) & 0x0FFF) as usize;
        let mut desc = Descriptors::default();
        if let Some(d) = sec.get(p + 5..p + 5 + es_len) {
            parse_descriptors(d, &mut desc);
        }
        p += 5 + es_len;
        if let Some(x) = ctx.pid(pid) {
            x.stream_type = stream_type;
            x.program = Some(pi);
            x.desc = desc;
        }
        es.push(pid);
    }
    let prog = &mut ctx.programs[pi];
    prog.pcr_pid = pcr_pid;
    prog.es = es;
    prog.pointer_field = pointer_field;
    prog.section_length = section_length;
    prog.pmt_seen = true;
}

fn dvb_text(b: &[u8]) -> String {
    // Strip a leading character-table selector byte; keep printable text.
    let b = match b.first() {
        Some(0x01..=0x0B) | Some(0x10) | Some(0x11) | Some(0x15) => {
            if b.first() == Some(&0x10) {
                b.get(3..).unwrap_or(&[])
            } else {
                &b[1..]
            }
        }
        _ => b,
    };
    crate::io::clean_text(&crate::io::latin1(b))
}

fn parse_sdt(sec: &[u8], ctx: &mut Ctx) {
    if sec.len() < 15 || sec[0] != 0x42 {
        return;
    }
    let end = sec.len() - 4;
    let mut p = 11;
    while p + 5 <= end {
        let service_id = be16(sec, p).unwrap_or(0);
        let loop_len = (be16(sec, p + 3).unwrap_or(0) & 0x0FFF) as usize;
        let descs = sec.get(p + 5..p + 5 + loop_len).unwrap_or(&[]);
        p += 5 + loop_len;
        let mut q = 0;
        while q + 2 <= descs.len() {
            let tag = descs[q];
            let len = descs[q + 1] as usize;
            let Some(body) = descs.get(q + 2..q + 2 + len) else { break };
            if tag == 0x48 && body.len() >= 3 {
                let service_type = body[0];
                let pl = body[1] as usize;
                let provider = dvb_text(body.get(2..2 + pl).unwrap_or(&[]));
                let nl = *body.get(2 + pl).unwrap_or(&0) as usize;
                let name = dvb_text(body.get(3 + pl..3 + pl + nl).unwrap_or(&[]));
                if let Some(prog) = ctx.programs.iter_mut().find(|x| x.number == service_id) {
                    prog.service_type = service_type;
                    prog.provider = provider;
                    prog.name = name;
                }
            }
            q += 2 + len;
        }
    }
}

/// Feed the payload of a PSI packet; sections may span packets.
fn handle_section(pid: u16, payload: &[u8], pusi: bool, ctx: &mut Ctx) {
    let idx = ctx.sections.iter().position(|(p, _)| *p == pid);
    let mut pointer_field = 0u8;
    if pusi {
        let Some(&ptr) = payload.first() else { return };
        pointer_field = ptr;
        let start = 1 + ptr as usize;
        let Some(data) = payload.get(start..) else { return };
        match idx {
            Some(i) => {
                ctx.sections[i].1.clear();
                ctx.sections[i].1.extend_from_slice(data);
            }
            None => {
                if ctx.sections.len() < MAX_PIDS {
                    ctx.sections.push((pid, data.to_vec()));
                }
            }
        }
    } else {
        let Some(i) = idx else { return };
        if ctx.sections[i].1.len() + payload.len() > SECTION_CAP {
            ctx.sections[i].1.clear();
            return;
        }
        ctx.sections[i].1.extend_from_slice(payload);
    }
    let Some(i) = ctx.sections.iter().position(|(p, _)| *p == pid) else { return };
    // Parse every complete section in the buffer.
    loop {
        let buf = &ctx.sections[i].1;
        if buf.len() < 3 || buf[0] == 0xFF {
            return;
        }
        let total = 3 + (be16(buf, 1).unwrap_or(0) & 0x0FFF) as usize;
        if buf.len() < total {
            return;
        }
        let sec = buf[..total].to_vec();
        ctx.sections[i].1.drain(..total);
        if pid == PID_PAT {
            parse_pat(&sec, ctx);
        } else if pid == PID_SDT {
            parse_sdt(&sec, ctx);
        } else {
            parse_pmt(&sec, pointer_field, ctx);
        }
    }
}

// ---------------------------------------------------------------------------- PES

fn flush_pes(p: &mut Pid, collect: bool) {
    if !p.cur_active {
        return;
    }
    p.cur_active = false;
    let Some(h) = parse_pes(&p.cur) else { return };
    if let Some(pts) = h.pts {
        if p.first_pts.is_none() {
            p.first_pts = Some(pts);
        }
        p.last_pts = Some(pts);
    }
    p.bytes += p.cur_total.saturating_sub(h.payload_offset) as u64;
    p.pes_count += 1;
    if collect && p.data.len() < CODEC_BYTES {
        if let Some(payload) = p.cur.get(h.payload_offset..) {
            let take = payload.len().min(CODEC_BYTES - p.data.len());
            p.data.extend_from_slice(&payload[..take]);
        }
    }
}

fn handle_pes_payload(p: &mut Pid, payload: &[u8], pusi: bool, collect: bool) {
    if pusi {
        flush_pes(p, collect);
        if payload.len() >= 3 && payload[0] == 0 && payload[1] == 0 && payload[2] == 1 {
            p.cur_active = true;
            p.cur.clear();
            p.cur_total = 0;
        }
    }
    if p.cur_active {
        p.cur_total += payload.len();
        let keep = collect || p.cur.len() < 64;
        if keep && p.cur.len() < PES_CAP {
            let take = payload.len().min(PES_CAP - p.cur.len());
            p.cur.extend_from_slice(&payload[..take]);
        }
    }
}

// ---------------------------------------------------------------------------- scan

fn handle_packet(pkt: &[u8], pos: u64, ctx: &mut Ctx, collect: bool) {
    if pkt.len() < 4 || pkt[0] != 0x47 {
        return;
    }
    let tei = pkt[1] & 0x80 != 0;
    let pusi = pkt[1] & 0x40 != 0;
    let pid = be16(pkt, 1).unwrap_or(0) & 0x1FFF;
    let afc = (pkt[3] >> 4) & 3;
    if tei || pid == PID_NULL {
        return;
    }
    let mut p = 4;
    if afc & 2 != 0 {
        let al = *pkt.get(4).unwrap_or(&0) as usize;
        if al > 0 {
            if let Some(flags) = pkt.get(5) {
                if flags & 0x10 != 0 && al >= 7 {
                    if let Some(b) = pkt.get(6..12) {
                        let base = (b[0] as u64) << 25 | (b[1] as u64) << 17 | (b[2] as u64) << 9 | (b[3] as u64) << 1 | (b[4] as u64) >> 7;
                        let ext = ((b[4] as u64) & 1) << 8 | b[5] as u64;
                        let pcr = base * 300 + ext;
                        if ctx.first_pcr.is_none() {
                            ctx.first_pcr = Some((pcr, pos));
                        }
                        ctx.last_pcr = Some((pcr, pos));
                    }
                }
            }
        }
        p += 1 + al;
    }
    if afc & 1 == 0 || p >= pkt.len() {
        return;
    }
    let payload = &pkt[p..];
    if pid == PID_PAT || pid == PID_SDT || ctx.is_pmt_pid(pid) {
        handle_section(pid, payload, pusi, ctx);
        return;
    }
    let Some(x) = ctx.pid(pid) else { return };
    x.packets += 1;
    handle_pes_payload(x, payload, pusi, collect);
}

/// Scan packets from `start` (a sync position) up to `end`. Returns the position reached.
fn scan(r: &mut Reader, start: u64, end: u64, ctx: &mut Ctx, collect: bool) -> u64 {
    let ps = ctx.packet_size as u64;
    let lead = if ps == 192 { 4 } else { 0 };
    let mut pos = start;
    let chunk_packets = 2048u64;
    while pos + ps <= end {
        let want = ((end - pos) / ps).min(chunk_packets) as usize * ps as usize;
        let chunk = r.read_vec_at(pos, want);
        if chunk.len() < ps as usize {
            break;
        }
        let mut off = 0usize;
        while off + ps as usize <= chunk.len() {
            let pkt = &chunk[off + lead..off + ps as usize];
            if pkt[0] != 0x47 {
                // lost sync: resync within the chunk
                match chunk[off + 1..].iter().position(|&b| b == 0x47) {
                    Some(k) => {
                        off += 1 + k;
                        if lead > 0 && off >= lead {
                            off -= lead;
                        }
                        continue;
                    }
                    None => {
                        off = chunk.len();
                        break;
                    }
                }
            }
            handle_packet(pkt, pos + off as u64, ctx, collect);
            off += ps as usize;
        }
        pos += off as u64;
        if off == 0 {
            break;
        }
    }
    pos
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 64 * 1024);
    let Some((ps, start)) = detect_packet_size(&head) else { return false };
    let mut ctx = Ctx { packet_size: ps, ..Default::default() };
    let len = r.len();
    let head_end = len.min(start as u64 + HEAD_SCAN);
    let reached = scan(r, start as u64, head_end, &mut ctx, true);
    ctx.complete = head_end >= len;
    for p in ctx.pids.iter_mut() {
        flush_pes(p, true);
    }
    if !ctx.complete {
        let tail_start = len.saturating_sub(TAIL_SCAN).max(reached);
        let chunk = r.read_vec_at(tail_start, (len - tail_start).min(64 * 1024) as usize);
        if let Some((_, off)) = detect_packet_size(&chunk) {
            for p in ctx.pids.iter_mut() {
                p.cur_active = false;
            }
            scan(r, tail_start + off as u64, len, &mut ctx, false);
            for p in ctx.pids.iter_mut() {
                flush_pes(p, false);
            }
        }
    }
    if ctx.programs.is_empty() && ctx.pids.iter().all(|p| p.pes_count == 0) {
        return false;
    }
    emit(doc, &ctx, len);
    true
}

// ---------------------------------------------------------------------------- emit

/// Stream type + descriptors → (kind, Format, MuxingMode).
fn stream_format(st: u8, d: &Descriptors) -> Option<(StreamKind, &'static str, &'static str)> {
    use StreamKind::*;
    let r = match st {
        0x01 | 0x02 => (Video, "MPEG Video", ""),
        0x03 | 0x04 => (Audio, "MPEG Audio", ""),
        0x0F => (Audio, "AAC", "ADTS"),
        0x11 => (Audio, "AAC", "LATM"),
        0x1C => (Audio, "AAC", ""),
        0x10 => (Video, "MPEG-4 Visual", ""),
        0x1B | 0x20 => (Video, "AVC", ""),
        0x24 | 0x25 => (Video, "HEVC", ""),
        0x42 => (Video, "AVS", ""),
        0xEA => (Video, "VC-1", ""),
        0x80 => {
            if d.registration == "AC-3" || d.ac3 {
                (Audio, "AC-3", "")
            } else {
                (Audio, "PCM", "")
            }
        }
        0x81 => (Audio, "AC-3", ""),
        0x82 | 0x85 | 0x86 | 0x8A | 0xA2 => (Audio, "DTS", ""),
        0x83 => (Audio, "MLP FBA", ""),
        0x84 | 0x87 | 0xA1 => (Audio, "E-AC-3", ""),
        0x90 => (Text, "PGS", ""),
        0x92 => (Text, "HDMV-TextST", ""),
        0x06 | 0x05 | 0x0B | 0x0C | 0x0D => {
            if d.eac3 {
                (Audio, "E-AC-3", "")
            } else if d.ac3 || d.registration == "AC-3" {
                (Audio, "AC-3", "")
            } else if d.dts || d.registration.starts_with("DTS") {
                (Audio, "DTS", "")
            } else if d.registration == "Opus" {
                (Audio, "Opus", "")
            } else if d.teletext {
                (Text, "Teletext", "")
            } else if d.dvb_subtitle.is_some() {
                (Text, "DVB Subtitle", "")
            } else if d.registration == "HEVC" {
                (Video, "HEVC", "")
            } else if d.registration == "VC-1" {
                (Video, "VC-1", "")
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(r)
}

fn apply_audio(s: &mut Stream, format: &str, data: &[u8], st: u8) {
    match format {
        "MPEG Audio" => {
            mpeg_audio::apply_frame(s, data);
        }
        "AAC" => {
            if s.get("MuxingMode") == "LATM" {
                aac::apply_latm_frame(s, data);
            } else {
                aac::apply_adts_frame(s, data);
            }
            let aot = match s.get("Format_AdditionalFeatures") {
                "Main" => 1,
                "LC" | "LC SBR" | "LC SBR PS" => 2,
                "SSR" => 3,
                "LTP" => 4,
                _ => 0,
            };
            if aot > 0 {
                s.set("CodecID", format!("{st}-{aot}"));
            }
        }
        "AC-3" | "E-AC-3" => {
            ac3::apply_frame(s, data);
        }
        "DTS" => {
            dts::apply_frame(s, data);
        }
        "MLP FBA" => {
            mlp::apply_frame(s, data);
        }
        "PCM" => {
            // Blu-ray LPCM audio_data header: size(2), channel layout/frequency(1), bits(2 bits)
            if data.len() >= 4 {
                let layout = data[2] >> 4;
                let freq = data[2] & 0x0F;
                let bits = match data[3] >> 6 {
                    1 => 16,
                    2 => 20,
                    3 => 24,
                    _ => 16,
                };
                let channels = match layout {
                    1 => 1,
                    3 => 2,
                    4 => 3,
                    5 => 3,
                    6 => 4,
                    7 => 4,
                    8 => 5,
                    9 => 6,
                    10 => 7,
                    11 => 8,
                    _ => 2,
                };
                let rate = match freq {
                    1 => 48000,
                    4 => 96000,
                    5 => 192000,
                    _ => 48000,
                };
                s.set_int("Channel(s)", channels);
                s.set_int("SamplingRate", rate);
                s.set_int("BitDepth", bits);
                s.set_int("BitRate", rate * bits * channels);
                s.set("BitRate_Mode", "CBR");
                pcm::apply_pcm(s, Some(false), Some(true), false, bits as u32);
            }
        }
        _ => {}
    }
    if matches!(format, "MPEG Audio" | "AAC" | "AC-3" | "E-AC-3" | "Opus") {
        s.set_if_empty("Compression_Mode", "Lossy");
    }
    if format == "MLP FBA" {
        s.set_if_empty("Compression_Mode", "Lossless");
    }
}

fn service_type_name(t: u8) -> &'static str {
    match t {
        0x01 => "digital television",
        0x02 => "digital radio sound",
        0x03 => "Teletext",
        0x04 => "NVOD reference",
        0x05 => "NVOD time-shifted",
        0x06 => "mosaic",
        0x07 => "FM radio",
        0x08 => "DVB SRM",
        0x0A => "advanced codec digital radio sound",
        0x0B => "H.264/AVC mosaic",
        0x0C => "data broadcast",
        0x0D => "reserved for Common Interface Usage",
        0x0E => "RCS Map",
        0x0F => "RCS FLS",
        0x10 => "DVB MHP",
        0x11 => "MPEG-2 HD digital television",
        0x16 => "H.264/AVC SD digital television",
        0x17 => "H.264/AVC SD NVOD time-shifted",
        0x18 => "H.264/AVC SD NVOD reference",
        0x19 => "H.264/AVC HD digital television",
        0x1A => "H.264/AVC HD NVOD time-shifted",
        0x1B => "H.264/AVC HD NVOD reference",
        0x1C => "H.264/AVC frame compatible plano-stereoscopic HD digital television",
        0x1D => "H.264/AVC frame compatible plano-stereoscopic HD NVOD time-shifted",
        0x1E => "H.264/AVC frame compatible plano-stereoscopic HD NVOD reference",
        0x1F => "HEVC digital television",
        _ => "",
    }
}

fn id_string(v: u64) -> String {
    format!("{v} (0x{v:X})")
}

fn emit(doc: &mut Doc, ctx: &Ctx, file_size: u64) {
    let g = doc.general();
    g.set("Format", if ctx.packet_size == 192 { "BDAV" } else { "MPEG-TS" });
    if let Some(tsid) = ctx.tsid {
        g.set_int("ID", tsid as i128);
        g.set("ID/String", id_string(tsid as u64));
    }
    // PCR-based duration and bit rate.
    let mut pcr_duration: Option<f64> = None;
    let mut pcr_delay: Option<f64> = None;
    if let (Some((f, fpos)), Some((l, lpos))) = (ctx.first_pcr, ctx.last_pcr) {
        let mut l = l;
        if l < f {
            l += (1u64 << 33) * 300;
        }
        let dur_ms = (l - f) as f64 / 27000.0;
        pcr_delay = Some(f as f64 / 27000.0);
        if dur_ms > 0.0 && lpos > fpos {
            pcr_duration = Some(dur_ms);
            g.set("Duration", format!("{dur_ms:.6}"));
            let bits = (lpos - fpos) as f64 * 8.0;
            g.set("OverallBitRate", format!("{}", (bits / dur_ms * 1000.0).round() as u64));
        }
    }
    let _ = file_size;

    // Programs in PAT order, elementary streams in PMT order.
    let mut programs: Vec<usize> = (0..ctx.programs.len()).filter(|&i| ctx.programs[i].pmt_seen).collect();
    let orphan: Vec<usize> = (0..ctx.pids.len()).filter(|&i| ctx.pids[i].program.is_none() && ctx.pids[i].pes_count > 0).collect();
    if programs.is_empty() && orphan.is_empty() {
        return;
    }
    let mut menu_entries: Vec<(usize, Vec<(StreamKind, usize, u16, String)>)> = Vec::new();
    let mut pi_order = 0usize;
    for &pi in &programs {
        let prog = &ctx.programs[pi];
        let mut entries = Vec::new();
        for (ei, &pid) in prog.es.iter().enumerate() {
            let Some(x) = ctx.pids.iter().find(|p| p.pid == pid) else { continue };
            let Some((kind, format, mux)) = stream_format(x.stream_type, &x.desc) else { continue };
            let mut s = Stream::new(kind);
            s.set("StreamOrder", format!("{pi_order}-{ei}"));
            s.set_int("ID", pid as i128);
            s.set("ID/String", id_string(pid as u64));
            s.set_int("MenuID", prog.number as i128);
            s.set("MenuID/String", id_string(prog.number as u64));
            s.set("Format", format);
            if !mux.is_empty() {
                s.set("MuxingMode", mux);
            }
            s.set("CodecID", x.stream_type.to_string());
            let mut tick = 0.0;
            match kind {
                StreamKind::Video => {
                    apply_video_codec(&mut s, &x.data);
                    tick = video_tick_ms(&s, &x.data);
                }
                StreamKind::Audio => {
                    apply_audio(&mut s, format, &x.data, x.stream_type);
                    if let (Some(sr), Some(spf)) = (s.get_f64("SamplingRate"), s.get_f64("SamplesPerFrame")) {
                        if sr > 0.0 && format != "AAC" {
                            tick = spf / sr * 1000.0;
                        }
                    }
                }
                StreamKind::Text => {
                    if let Some(l) = &x.desc.dvb_subtitle {
                        s.set("Language", l.clone());
                    }
                }
                _ => {}
            }
            if !x.desc.language.is_empty() && !s.has("Language") {
                s.set("Language", x.desc.language.clone());
            }
            apply_timing(&mut s, x.first_pts, x.last_pts, tick, x.bytes, ctx.complete, false);
            if !x.desc.registration.is_empty() {
                s.set_extra("format_identifier", x.desc.registration.clone(), "", "N NT");
            }
            let pos = doc.streams[kind as usize].len();
            entries.push((kind, pos, pid, format.to_string()));
            doc.streams[kind as usize].push(s);
        }
        menu_entries.push((pi, entries));
        pi_order += 1;
    }
    // Streams outside any program (no PAT/PMT): identify by PES stream id.
    for i in orphan {
        let x = &ctx.pids[i];
        let sid = x.cur.get(3).copied().unwrap_or(0);
        let Some(h) = parse_pes(&x.cur) else { continue };
        let sid = if sid == 0 { h.stream_id } else { sid };
        let kind = match sid {
            0xE0..=0xEF => StreamKind::Video,
            0xC0..=0xDF => StreamKind::Audio,
            _ => continue,
        };
        let mut s = Stream::new(kind);
        s.set_int("ID", x.pid as i128);
        s.set("ID/String", id_string(x.pid as u64));
        let mut tick = 0.0;
        if kind == StreamKind::Video {
            apply_video_codec(&mut s, &x.data);
            tick = video_tick_ms(&s, &x.data);
        } else {
            let fmt = if x.data.len() >= 2 && x.data[0] == 0xFF && x.data[1] & 0xF6 == 0xF0 { "AAC" } else { "MPEG Audio" };
            s.set("Format", fmt);
            if fmt == "AAC" {
                s.set("MuxingMode", "ADTS");
            }
            apply_audio(&mut s, fmt, &x.data, 0);
        }
        apply_timing(&mut s, x.first_pts, x.last_pts, tick, x.bytes, ctx.complete, false);
        doc.streams[kind as usize].push(s);
    }
    apply_video_delay(doc);
    let _ = pcr_duration;

    // One Menu stream per program.
    for (order, (pi, entries)) in menu_entries.iter().enumerate() {
        let prog = &ctx.programs[*pi];
        let mut m = Stream::new(StreamKind::Menu);
        m.set_int("StreamOrder", order as i128);
        m.set_int("ID", prog.pmt_pid as i128);
        m.set("ID/String", id_string(prog.pmt_pid as u64));
        m.set_int("MenuID", prog.number as i128);
        m.set("MenuID/String", id_string(prog.number as u64));
        if !entries.is_empty() {
            let formats: Vec<&str> = entries.iter().map(|e| e.3.as_str()).collect();
            m.set("Format", formats.join(" / "));
            let kinds: Vec<String> = entries.iter().map(|e| (e.0 as usize).to_string()).collect();
            let poss: Vec<String> = entries.iter().map(|e| e.1.to_string()).collect();
            let ids: Vec<String> = entries.iter().map(|e| e.2.to_string()).collect();
            m.set("List_StreamKind", kinds.join(" / "));
            m.set("List_StreamPos", poss.join(" / "));
            m.set("List", ids.join(" / "));
        }
        if let Some(d) = pcr_duration {
            m.set("Duration", format!("{d:.6}"));
        }
        if let Some(d) = pcr_delay {
            m.set("Delay", format!("{d:.6}"));
        }
        if !prog.name.is_empty() {
            m.set("ServiceName", prog.name.clone());
        }
        if !prog.provider.is_empty() {
            m.set("ServiceProvider", prog.provider.clone());
        }
        let st = service_type_name(prog.service_type);
        if !st.is_empty() {
            m.set("ServiceType", st);
        }
        m.set_extra("pointer_field", prog.pointer_field.to_string(), "", "N NT");
        m.set_extra("section_length", prog.section_length.to_string(), "", "N NT");
        doc.streams[StreamKind::Menu as usize].push(m);
    }
    let _ = &mut programs;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc_placeholder() -> [u8; 4] {
        [0xDE, 0xAD, 0xBE, 0xEF]
    }

    fn section_packet(pid: u16, cc: u8, section: &[u8]) -> Vec<u8> {
        let mut p = vec![0x47, 0x40 | (pid >> 8) as u8, pid as u8, 0x10 | cc, 0];
        p.extend_from_slice(section);
        p.resize(188, 0xFF);
        p
    }

    fn pes_packet(pid: u16, cc: u8, pusi: bool, pes: &[u8]) -> Vec<u8> {
        let mut p = vec![0x47, if pusi { 0x40 } else { 0 } | (pid >> 8) as u8, pid as u8, 0x30 | cc];
        // adaptation field padding
        let pad = 188 - 4 - pes.len();
        p.push((pad - 1) as u8);
        if pad > 1 {
            p.push(0);
            p.resize(4 + pad, 0xFF);
        }
        p.extend_from_slice(pes);
        assert_eq!(p.len(), 188);
        p
    }

    fn pts_bytes(pts: u64, prefix: u8) -> [u8; 5] {
        [prefix | (((pts >> 30) & 7) as u8) << 1 | 1, (pts >> 22) as u8, (((pts >> 15) & 0x7F) as u8) << 1 | 1, (pts >> 7) as u8, ((pts & 0x7F) as u8) << 1 | 1]
    }

    fn pcr_packet(pid: u16, pcr_base: u64) -> Vec<u8> {
        let mut p = vec![0x47, (pid >> 8) as u8, pid as u8, 0x20, 183, 0x10];
        p.extend_from_slice(&[(pcr_base >> 25) as u8, (pcr_base >> 17) as u8, (pcr_base >> 9) as u8, (pcr_base >> 1) as u8, ((pcr_base & 1) as u8) << 7 | 0x7E, 0]);
        p.resize(188, 0xFF);
        p
    }

    fn build_ts() -> Vec<u8> {
        let mut f = Vec::new();
        // PAT: program 1 → PMT 0x1000
        let mut pat = vec![0x00, 0xB0, 13, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xF0, 0x00];
        pat.extend_from_slice(&crc_placeholder());
        f.extend(section_packet(0, 0, &pat));
        // SDT with service descriptor
        let mut sdt = vec![0x42, 0xF0, 0, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xFF, 0x01, 0xFF];
        let mut sd = vec![0x48, 0, 0x01, 6];
        sd.extend_from_slice(b"FFmpeg");
        sd.push(9);
        sd.extend_from_slice(b"Service01");
        sd[1] = (sd.len() - 2) as u8;
        sdt.extend_from_slice(&[0x00, 0x01, 0xFC, 0x80, sd.len() as u8]);
        sdt.extend_from_slice(&sd);
        sdt.extend_from_slice(&crc_placeholder());
        let l = (sdt.len() - 3) as u16;
        sdt[1] = 0xF0 | (l >> 8) as u8;
        sdt[2] = l as u8;
        f.extend(section_packet(0x11, 0, &sdt));
        // PMT: PCR PID 0x100, AVC on 0x100, AC-3 on 0x101 with registration descriptor + language
        let mut pmt = vec![0x02, 0xB0, 0, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xE1, 0x00, 0xF0, 0x00];
        pmt.extend_from_slice(&[0x1B, 0xE1, 0x00, 0xF0, 0x00]);
        pmt.extend_from_slice(&[0x81, 0xE1, 0x01, 0xF0, 12, 0x05, 4, b'A', b'C', b'-', b'3', 0x0A, 4, b'e', b'n', b'g', 0]);
        pmt.extend_from_slice(&crc_placeholder());
        let l = (pmt.len() - 3) as u16;
        pmt[1] = 0xB0 | (l >> 8) as u8;
        pmt[2] = l as u8;
        f.extend(section_packet(0x1000, 0, &pmt));
        // PCR + video PES (PTS 1421.333 ms = 127920 ticks), payload with an AUD + SPS-less data
        f.extend(pcr_packet(0x100, 64920));
        let mut pes = vec![0, 0, 1, 0xE0, 0, 0, 0x80, 0x80, 5];
        pes.extend_from_slice(&pts_bytes(127920, 0x20));
        pes.extend_from_slice(&[0, 0, 0, 1, 0x09, 0xF0, 0, 0, 0, 1, 0x67, 0x42, 0xC0, 0x0A, 0xDA, 0x11, 0xEC, 0x04, 0x40, 0, 0, 3, 0, 0x40, 0, 0, 0x0C, 0x83, 0xC4, 0x89, 0xA8, 0, 0, 0, 1, 0x68, 0xCE, 0x0F, 0xC8]);
        f.extend(pes_packet(0x100, 0, true, &pes));
        // audio PES on 0x101 (PTS 1400 ms = 126000), 8 payload bytes
        let mut pes = vec![0, 0, 1, 0xC0, 0, 0, 0x80, 0x80, 5];
        pes.extend_from_slice(&pts_bytes(126000, 0x20));
        pes.extend_from_slice(&[0x0B, 0x77, 1, 2, 3, 4, 5, 6]);
        let l = (pes.len() - 6) as u16;
        pes[4] = (l >> 8) as u8;
        pes[5] = l as u8;
        f.extend(pes_packet(0x101, 0, true, &pes));
        // second video PES (PTS 2381.333 = 214320) and final PCR
        let mut pes = vec![0, 0, 1, 0xE0, 0, 0, 0x80, 0x80, 5];
        pes.extend_from_slice(&pts_bytes(214320, 0x20));
        pes.extend_from_slice(&[0, 0, 0, 1, 0x65, 1, 2, 3]);
        f.extend(pes_packet(0x100, 1, true, &pes));
        f.extend(pcr_packet(0x100, 151320));
        f.extend(pes_packet(0x100, 2, true, &[0, 0, 1, 0xE0, 0, 0, 0x80, 0x00, 0, 1]));
        f
    }

    #[test]
    fn packet_size_detection_and_probe() {
        let ts = build_ts();
        assert_eq!(detect_packet_size(&ts), Some((188, 0)));
        let mut m2ts = Vec::new();
        for pkt in ts.chunks(188) {
            m2ts.extend_from_slice(&[0, 0, 0, 0]);
            m2ts.extend_from_slice(pkt);
        }
        assert_eq!(detect_packet_size(&m2ts), Some((192, 0)));
        assert_eq!(probe(&Probe { head: &ts, ext: "ts", size: ts.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: &m2ts, ext: "m2ts", size: m2ts.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"\x47\x00\x00", ext: "ts", size: 3 }), 0);
        let mut shifted = vec![0u8; 10];
        shifted.extend_from_slice(&ts);
        assert_eq!(detect_packet_size(&shifted), Some((188, 10)));
    }

    #[test]
    fn parses_tables_and_streams() {
        let ts = build_ts();
        let mut r = Reader::from_bytes(ts);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "MPEG-TS");
        assert_eq!(g.get("ID/String"), "1 (0x1)");
        assert_eq!(g.get("Duration"), "960.000000");
        // 4 packets between the PCR packets: 752 bytes * 8 / 0.96 s
        assert_eq!(g.get("OverallBitRate"), "6267");
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("ID/String"), "256 (0x100)");
        assert_eq!(v.get("StreamOrder"), "0-0");
        assert_eq!(v.get("MenuID/String"), "1 (0x1)");
        assert_eq!(v.get("Format"), "AVC");
        assert_eq!(v.get("CodecID"), "27");
        assert_eq!(v.get("Delay"), "1421.333333");
        assert_eq!(v.get("Format_Profile"), "Baseline@L1");
        // VUI tick 20 ms extends the 960 ms span
        assert_eq!(v.get("Duration"), "980");
        let a = &doc.streams[StreamKind::Audio as usize][0];
        assert_eq!(a.get("Format"), "AC-3");
        assert_eq!(a.get("CodecID"), "129");
        assert_eq!(a.get("Language"), "eng");
        assert_eq!(a.get("format_identifier"), "AC-3");
        assert_eq!(a.get("Delay"), "1400.000000");
        assert_eq!(a.get("Video_Delay"), "-21");
        let m = &doc.streams[StreamKind::Menu as usize][0];
        assert_eq!(m.get("ID/String"), "4096 (0x1000)");
        assert_eq!(m.get("Format"), "AVC / AC-3");
        assert_eq!(m.get("List"), "256 / 257");
        assert_eq!(m.get("List_StreamKind"), "1 / 2");
        assert_eq!(m.get("List_StreamPos"), "0 / 0");
        assert_eq!(m.get("ServiceName"), "Service01");
        assert_eq!(m.get("ServiceProvider"), "FFmpeg");
        assert_eq!(m.get("ServiceType"), "digital television");
        assert_eq!(m.get("Delay"), "721.333333");
        assert_eq!(m.get("pointer_field"), "0");
        assert_eq!(m.get("section_length"), "35");
    }

    #[test]
    fn descriptors_and_formats() {
        let mut d = Descriptors::default();
        parse_descriptors(&[0x05, 4, b'D', b'T', b'S', b'2', 0x59, 8, b'f', b'r', b'e', 0x10, 0, 1, 0, 2, 0x52, 1, 7], &mut d);
        assert_eq!(d.registration, "DTS2");
        assert_eq!(d.dvb_subtitle.as_deref(), Some("fre"));
        assert_eq!(d.component_tag, Some(7));
        assert_eq!(stream_format(0x06, &d).map(|x| x.1), Some("DTS"));
        let mut d = Descriptors::default();
        parse_descriptors(&[0x56, 5, b'e', b'n', b'g', 0x10, 0], &mut d);
        assert_eq!(stream_format(0x06, &d).map(|x| (x.0, x.1)), Some((StreamKind::Text, "Teletext")));
        assert_eq!(stream_format(0x0F, &Descriptors::default()).map(|x| x.2), Some("ADTS"));
        assert_eq!(stream_format(0x24, &Descriptors::default()).map(|x| x.1), Some("HEVC"));
        assert_eq!(stream_format(0x06, &Descriptors::default()), None);
        assert_eq!(dvb_text(&[0x10, 0, 1, b'A', b'b']), "Ab");
        assert_eq!(dvb_text(b"plain"), "plain");
    }

    #[test]
    fn malformed_input_is_rejected() {
        let mut r = Reader::from_bytes(vec![0x47; 1000]);
        let mut doc = Doc::new();
        // sync bytes everywhere but no PAT/PMT/PES: nothing recognised
        assert!(!parse(&mut r, &mut doc));
        let mut r = Reader::from_bytes(Vec::new());
        assert!(!parse(&mut r, &mut Doc::new()));
    }
}

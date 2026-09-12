//! RIFF family: AVI (RIFF `AVI ` with OpenDML `AVIX`/`indx` extensions), WAVE / RF64 / BWF, and
//! WebP (delegated to `image::webp`).
//!
//! Only the container structures are interpreted here (chunk lists, `avih`/`strh`/`strf`,
//! `fmt `/`data`/`ds64`, `INFO` tags, `idx1`/`indx`, interleaving statistics). Codec-level
//! information comes from the helper modules (`audio::wma`, `audio::pcm`, `video::fourcc`,
//! `video::avc`, `video::mpeg4v`, `video::mjpeg`, `audio::mpeg_audio`, `audio::ac3`, `audio::dts`).

use crate::io::{clean_text, cstr, le16, le32, le64, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::audio::{self, ac3, dts, mpeg_audio, pcm, wma};
use crate::parsers::image::webp;
use crate::parsers::video::{avc, fourcc, mjpeg, mpeg4v};
use crate::parsers::Probe;

/// Bytes of `movi` payload scanned before extrapolating chunk statistics.
const MAX_MOVI_SCAN: u64 = 8 << 20;
/// Cap on `idx1` / `indx` entries read (16 bytes each).
const MAX_INDEX_ENTRIES: usize = 1 << 20;
/// Cap on nested RIFF (`AVIX`) chunks visited.
const MAX_RIFF_CHUNKS: usize = 4096;
/// Frames kept per stream for the codec helpers.
const MAX_FRAMES_FOR_CODEC: usize = 4;
const MAX_VIDEO_FRAME_BYTES: usize = 1 << 20;
const MAX_AUDIO_FRAME_BYTES: usize = 64 << 10;
/// Bytes of WAVE `data` handed to the audio frame helpers.
const WAVE_DATA_PROBE: usize = 64 << 10;
/// Cap on the size of a metadata chunk read into memory.
const MAX_META_CHUNK: usize = 1 << 20;

pub fn probe(p: &Probe) -> u8 {
    let h = p.head;
    if h.len() < 12 || !matches!(&h[0..4], b"RIFF" | b"RF64" | b"RIFX" | b"BW64") {
        return 0;
    }
    match &h[8..12] {
        b"AVI " | b"WAVE" | b"WEBP" => 100,
        b"AVIX" => 60,
        _ => 0,
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 12);
    if head.len() < 12 {
        return false;
    }
    let be = &head[0..4] == b"RIFX";
    let rf64 = matches!(&head[0..4], b"RF64" | b"BW64");
    match &head[8..12] {
        b"AVI " | b"AVIX" => parse_avi(r, doc, be),
        b"WAVE" => parse_wave(r, doc, be, rf64),
        b"WEBP" => {
            r.seek(0);
            if webp::parse_riff_webp(r, doc) {
                return true;
            }
            // The RIFF form type alone identifies the format; the image helper fills the rest.
            doc.general().set("Format", "WebP");
            let s = doc.add(StreamKind::Image);
            s.set("Format", "WebP");
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------- chunk walking

#[derive(Debug, Clone, Copy)]
struct Chunk {
    id: [u8; 4],
    size: u64,
    /// Absolute position of the payload.
    pos: u64,
}

impl Chunk {
    fn end(&self) -> u64 {
        self.pos.saturating_add(self.size)
    }
    /// Position of the next chunk (payloads are padded to even sizes).
    fn next(&self) -> u64 {
        self.end().saturating_add(self.size & 1)
    }
}

fn read_chunk(r: &mut Reader, be: bool) -> Option<Chunk> {
    let id = r.read_fourcc()?;
    let size = if be { r.read_u32be()? } else { r.read_u32le()? } as u64;
    Some(Chunk { id, size, pos: r.pos() })
}

fn is_printable_fourcc(id: &[u8]) -> bool {
    id.len() == 4 && id.iter().all(|&c| (0x20..0x7F).contains(&c))
}

fn fourcc_str(id: &[u8]) -> String {
    id.iter().map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { '?' }).collect()
}

/// Stream number and chunk type (`dc`, `db`, `wb`, `tx`, `pc`, ...) of a `movi` chunk id.
fn stream_of(id: &[u8; 4]) -> Option<(usize, [u8; 2])> {
    let hex = |c: u8| (c as char).to_digit(16);
    let n = hex(id[0])? * 16 + hex(id[1])?;
    let kind = [id[2], id[3]];
    if !matches!(&kind, b"dc" | b"db" | b"wb" | b"tx" | b"pc" | b"sb" | b"iv" | b"dd") {
        return None;
    }
    Some((n as usize, kind))
}

/// RIFF `INFO` list → General tags.
fn apply_info_list(r: &mut Reader, end: u64, be: bool, g: &mut Stream) {
    let mut n = 0;
    while r.pos() + 8 <= end && n < 256 {
        n += 1;
        let Some(c) = read_chunk(r, be) else { break };
        let len = c.size.min(MAX_META_CHUNK as u64) as usize;
        let text = clean_text(&cstr(&r.read_vec_at(c.pos, len)));
        if !text.is_empty() {
            apply_info_tag(g, &c.id, &text);
        }
        r.seek(c.next());
    }
}

fn apply_info_tag(g: &mut Stream, id: &[u8; 4], value: &str) {
    let field = match id {
        b"INAM" => "Title",
        b"IART" => "Performer",
        b"ICMT" => "Comment",
        b"ICRD" => "Recorded_Date",
        b"IGNR" => "Genre",
        b"ICOP" => "Copyright",
        b"IPRD" => "Album",
        b"ISFT" => "Encoded_Application",
        b"ITRK" | b"IPRT" => "Track/Position",
        b"ISBJ" => "Subject",
        b"IKEY" => "Keywords",
        b"IENG" => "Engineer",
        b"ITCH" => "EncodedBy",
        b"IWRI" => "WrittenBy",
        b"IPRO" => "Producer",
        b"IEDT" => "EditedBy",
        b"IDIT" => "Encoded_Date",
        b"IMED" => "OriginalSourceMedium",
        b"ILNG" => "Language",
        b"ICMS" => "Commissioned",
        b"ISRC" => "Source",
        b"ICNM" => "Director",
        b"IMUS" => "Composer",
        b"ILYR" => "Lyrics",
        b"IPUB" => "Publisher",
        b"IRTD" => "LawRating",
        b"ISTR" | b"ISTA" => "Actor",
        _ => return,
    };
    g.set_if_empty(field, value);
}

// ---------------------------------------------------------------------------- WAVEFORMATEX

/// The container-level part of a `WAVEFORMATEX` / `WAVEFORMATEXTENSIBLE` (needed for durations,
/// block alignment and the choice of codec helper).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WaveFormat {
    pub tag: u16,
    pub channels: u16,
    pub rate: u32,
    pub avg_bytes: u32,
    pub block_align: u16,
    pub bits: u16,
    pub valid_bits: u16,
    pub channel_mask: u32,
    pub subformat: Option<[u8; 16]>,
}

impl WaveFormat {
    /// Format tag, resolved through the extensible sub-format GUID when it is a standard one.
    pub fn effective_tag(&self) -> u16 {
        if self.tag == 0xFFFE {
            if let Some(g) = &self.subformat {
                if g[4..] == [0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71] {
                    return u16::from_le_bytes([g[0], g[1]]);
                }
            }
        }
        self.tag
    }
    pub fn is_pcm(&self) -> bool {
        matches!(self.effective_tag(), 1 | 3)
    }
    pub fn is_float(&self) -> bool {
        self.effective_tag() == 3
    }
    /// CodecID as the reference prints it: hex tag, or the GUID for extensible formats.
    pub fn codec_id(&self) -> String {
        match &self.subformat {
            Some(g) if self.tag == 0xFFFE => format!(
                "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
                le32(g, 0).unwrap_or(0),
                le16(g, 4).unwrap_or(0),
                le16(g, 6).unwrap_or(0),
                g[8],
                g[9],
                g[10],
                g[11],
                g[12],
                g[13],
                g[14],
                g[15]
            ),
            _ => format!("{:X}", self.tag),
        }
    }
}

pub fn parse_wave_format(d: &[u8]) -> Option<WaveFormat> {
    if d.len() < 14 {
        return None;
    }
    let mut f = WaveFormat { tag: le16(d, 0)?, channels: le16(d, 2)?, rate: le32(d, 4)?, avg_bytes: le32(d, 8)?, block_align: le16(d, 12)?, ..Default::default() };
    f.bits = le16(d, 14).unwrap_or(0);
    let cb = le16(d, 16).unwrap_or(0) as usize;
    if f.tag == 0xFFFE && cb >= 22 && d.len() >= 40 {
        f.valid_bits = le16(d, 18)?;
        f.channel_mask = le32(d, 20)?;
        let mut g = [0u8; 16];
        g.copy_from_slice(&d[24..40]);
        f.subformat = Some(g);
    }
    Some(f)
}

/// Format name for a wave format tag when the `wma` helper left it empty.
fn wave_format_name(tag: u16) -> Option<&'static str> {
    Some(match tag {
        0x0001 | 0x0003 => "PCM",
        0x0050 | 0x0055 => "MPEG Audio",
        0x2000 => "AC-3",
        0x2001 => "DTS",
        0x0160..=0x0163 => "WMA",
        0x00FF | 0x1610 | 0x4143 | 0xA106 => "AAC",
        0x674F..=0x6751 | 0x6770..=0x6772 => "Vorbis",
        0xF1AC => "FLAC",
        _ => return None,
    })
}

/// Apply a `strf`/`fmt ` payload to an audio stream: codec helper first, then the container-level
/// facts every WAVEFORMATEX carries.
fn apply_audio_format(s: &mut Stream, fmt: &[u8], frames: &[Vec<u8>]) -> Option<WaveFormat> {
    wma::apply_waveformatex(s, fmt);
    let f = parse_wave_format(fmt)?;
    let tag = f.effective_tag();
    s.set_if_empty("CodecID", f.codec_id());
    if !s.has("Format") {
        if let Some(name) = wave_format_name(tag) {
            s.set("Format", name);
        }
    }
    if f.channels > 0 {
        s.set_if_empty("Channel(s)", f.channels.to_string());
    }
    if f.rate > 0 {
        s.set_if_empty("SamplingRate", f.rate.to_string());
    }
    if f.avg_bytes > 0 {
        s.set_if_empty("BitRate", (f.avg_bytes as u64 * 8).to_string());
    }
    if f.bits > 0 {
        s.set_if_empty("BitDepth", f.bits.to_string());
    }
    if f.is_pcm() {
        let bits = if f.valid_bits > 0 { f.valid_bits } else { f.bits } as u32;
        pcm::apply_pcm(s, Some(true), Some(bits > 8), f.is_float(), bits);
        if f.is_float() {
            s.set_if_empty("Format_Profile", "Float");
        }
        s.set_if_empty("BitRate_Mode", "CBR");
    }
    if f.tag == 0xFFFE && f.channel_mask != 0 && !s.has("ChannelLayout") {
        let (pos, layout) = audio::layout_from_mask(f.channel_mask);
        if !pos.is_empty() {
            s.set("ChannelPositions", pos);
            s.set("ChannelLayout", layout);
        }
    }
    // Frame-level helpers for the formats that carry their own headers.
    for frame in frames {
        let done = match tag {
            0x0050 | 0x0055 => mpeg_audio::apply_frame(s, frame),
            0x2000 => ac3::apply_frame(s, frame),
            0x2001 => dts::apply_frame(s, frame),
            _ => true,
        };
        if done {
            break;
        }
    }
    Some(f)
}

// ---------------------------------------------------------------------------- AVI

#[derive(Debug, Clone, Default)]
struct Avih {
    us_per_frame: u32,
    flags: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Default)]
struct AviStream {
    fcc_type: [u8; 4],
    handler: [u8; 4],
    scale: u32,
    rate: u32,
    start: u32,
    length: u32,
    strf: Vec<u8>,
    name: String,
    /// `vprp`: fields per frame, frame aspect ratio (x, y).
    fields_per_frame: Option<u32>,
    aspect: Option<(u32, u32)>,
    indx: Vec<u8>,
    // statistics from the movi scan
    chunks: u64,
    bytes: u64,
    chunks_before_video: u64,
    chunk_sizes: Vec<u64>,
    frames: Vec<Vec<u8>>,
    // totals from an index (idx1 or OpenDML) or extrapolated, when the scan was partial
    index_chunks: Option<u64>,
    index_bytes: Option<u64>,
}

impl AviStream {
    fn is_video(&self) -> bool {
        &self.fcc_type == b"vids"
    }
    fn is_audio(&self) -> bool {
        &self.fcc_type == b"auds"
    }
    /// Seconds for `n` stream units.
    fn seconds(&self, n: u64) -> Option<f64> {
        if self.rate == 0 {
            return None;
        }
        Some(n as f64 * self.scale as f64 / self.rate as f64)
    }
    fn total_chunks(&self) -> u64 {
        self.index_chunks.unwrap_or(self.chunks)
    }
    fn total_bytes(&self) -> u64 {
        self.index_bytes.unwrap_or(self.bytes)
    }
}

#[derive(Debug, Default)]
struct Avi {
    avih: Option<Avih>,
    streams: Vec<AviStream>,
    has_avix: bool,
    dmlh_total_frames: Option<u32>,
    movi_total: u64,
    movi_scanned: u64,
    scan_complete: bool,
    first_video_seen: bool,
    idx1: Option<(u64, u64)>,
}

fn parse_avi(r: &mut Reader, doc: &mut Doc, be: bool) -> bool {
    let mut ctx = Avi { scan_complete: true, ..Default::default() };
    let mut pos = 0u64;
    let mut riffs = 0usize;
    let mut first = true;
    while pos + 12 <= r.len() && riffs < MAX_RIFF_CHUNKS {
        r.seek(pos);
        let Some(riff) = read_chunk(r, be) else { break };
        let form = r.read_fourcc().unwrap_or([0; 4]);
        if &riff.id != b"RIFF" || !(matches!(&form, b"AVI " | b"AVIX")) {
            break;
        }
        if &form == b"AVIX" {
            ctx.has_avix = true;
        }
        riffs += 1;
        let end = riff.end().min(r.len());
        parse_avi_children(r, end, be, &mut ctx, doc, first);
        first = false;
        pos = riff.next();
    }
    if ctx.streams.is_empty() && ctx.avih.is_none() {
        return false;
    }
    apply_indexes(r, be, &mut ctx);
    emit_avi(doc, &ctx);
    true
}

fn parse_avi_children(r: &mut Reader, end: u64, be: bool, ctx: &mut Avi, doc: &mut Doc, top: bool) {
    let mut n = 0;
    while r.pos() + 8 <= end && n < 4096 {
        n += 1;
        let Some(c) = read_chunk(r, be) else { break };
        let c_end = c.end().min(end);
        match &c.id {
            b"LIST" => {
                let form = r.read_fourcc().unwrap_or([0; 4]);
                match &form {
                    b"hdrl" if top => parse_hdrl(r, c_end, be, ctx),
                    b"INFO" => apply_info_list(r, c_end, be, doc.general()),
                    b"movi" => {
                        ctx.movi_total += c_end.saturating_sub(r.pos());
                        scan_movi(r, c_end, be, ctx, 0);
                    }
                    _ => {}
                }
            }
            b"idx1" if ctx.idx1.is_none() => ctx.idx1 = Some((c.pos, c.size)),
            _ => {}
        }
        r.seek(c.next());
    }
}

fn parse_hdrl(r: &mut Reader, end: u64, be: bool, ctx: &mut Avi) {
    let mut n = 0;
    while r.pos() + 8 <= end && n < 1024 {
        n += 1;
        let Some(c) = read_chunk(r, be) else { break };
        let c_end = c.end().min(end);
        match &c.id {
            b"avih" => {
                let d = r.read_vec_at(c.pos, c.size.min(64) as usize);
                ctx.avih = Some(Avih { us_per_frame: le32(&d, 0).unwrap_or(0), flags: le32(&d, 12).unwrap_or(0), width: le32(&d, 32).unwrap_or(0), height: le32(&d, 36).unwrap_or(0) });
            }
            b"LIST" => {
                let form = r.read_fourcc().unwrap_or([0; 4]);
                match &form {
                    b"strl" if ctx.streams.len() < 256 => {
                        let st = parse_strl(r, c_end, be);
                        ctx.streams.push(st);
                    }
                    b"odml" => parse_odml(r, c_end, be, ctx),
                    _ => {}
                }
            }
            _ => {}
        }
        r.seek(c.next());
    }
}

fn parse_odml(r: &mut Reader, end: u64, be: bool, ctx: &mut Avi) {
    let mut n = 0;
    while r.pos() + 8 <= end && n < 64 {
        n += 1;
        let Some(c) = read_chunk(r, be) else { break };
        if &c.id == b"dmlh" {
            let d = r.read_vec_at(c.pos, 4);
            ctx.dmlh_total_frames = le32(&d, 0);
        }
        r.seek(c.next());
    }
}

fn parse_strl(r: &mut Reader, end: u64, be: bool) -> AviStream {
    let mut st = AviStream::default();
    let mut n = 0;
    while r.pos() + 8 <= end && n < 64 {
        n += 1;
        let Some(c) = read_chunk(r, be) else { break };
        let len = c.size.min(MAX_META_CHUNK as u64) as usize;
        match &c.id {
            b"strh" => {
                let d = r.read_vec_at(c.pos, len.min(64));
                if d.len() >= 8 {
                    st.fcc_type.copy_from_slice(&d[0..4]);
                    st.handler.copy_from_slice(&d[4..8]);
                }
                st.scale = le32(&d, 20).unwrap_or(0);
                st.rate = le32(&d, 24).unwrap_or(0);
                st.start = le32(&d, 28).unwrap_or(0);
                st.length = le32(&d, 32).unwrap_or(0);
            }
            b"strf" => st.strf = r.read_vec_at(c.pos, len),
            b"strn" => st.name = clean_text(&cstr(&r.read_vec_at(c.pos, len))),
            b"indx" => st.indx = r.read_vec_at(c.pos, len),
            b"vprp" => {
                let d = r.read_vec_at(c.pos, len.min(64));
                if let Some(ar) = le32(&d, 20) {
                    if ar != 0 {
                        st.aspect = Some((ar >> 16, ar & 0xFFFF));
                    }
                }
                st.fields_per_frame = le32(&d, 32);
            }
            _ => {}
        }
        r.seek(c.next());
    }
    st
}

/// Walk the chunks of a `movi` list (and nested `rec ` lists), recording per-stream statistics
/// and keeping the first frames for the codec helpers.
fn scan_movi(r: &mut Reader, end: u64, be: bool, ctx: &mut Avi, depth: u32) {
    let mut n = 0u64;
    while r.pos() + 8 <= end {
        n += 1;
        if n > 8_000_000 || ctx.movi_scanned >= MAX_MOVI_SCAN {
            ctx.scan_complete = false;
            break;
        }
        let Some(c) = read_chunk(r, be) else { break };
        let c_end = c.end().min(end);
        if &c.id == b"LIST" {
            let form = r.read_fourcc().unwrap_or([0; 4]);
            if &form == b"rec " && depth < 4 {
                scan_movi(r, c_end, be, ctx, depth + 1);
            }
            r.seek(c.next());
            continue;
        }
        if let Some((num, kind)) = stream_of(&c.id) {
            if let Some(st) = ctx.streams.get_mut(num) {
                if st.is_video() && !ctx.first_video_seen {
                    ctx.first_video_seen = true;
                }
                if st.is_audio() && !ctx.first_video_seen {
                    st.chunks_before_video += 1;
                }
                st.chunks += 1;
                st.bytes += c.size;
                if st.chunk_sizes.len() < 256 {
                    st.chunk_sizes.push(c.size);
                }
                if st.frames.len() < MAX_FRAMES_FOR_CODEC && c.size > 0 && !matches!(&kind, b"pc" | b"iv") {
                    let cap = if st.is_video() { MAX_VIDEO_FRAME_BYTES } else { MAX_AUDIO_FRAME_BYTES };
                    let want = (c.size.min(cap as u64)) as usize;
                    st.frames.push(r.read_vec_at(c.pos, want));
                }
            }
        }
        ctx.movi_scanned += c.size.saturating_add(8);
        r.seek(c.next());
    }
}

/// Per-stream totals from the OpenDML `indx` chain or the legacy `idx1`, else extrapolate the
/// partial `movi` scan.
fn apply_indexes(r: &mut Reader, be: bool, ctx: &mut Avi) {
    let mut any_odml = false;
    for st in ctx.streams.iter_mut() {
        if st.indx.is_empty() {
            continue;
        }
        if let Some((chunks, bytes)) = odml_index_totals(r, be, &st.indx) {
            st.index_chunks = Some(chunks);
            st.index_bytes = Some(bytes);
            any_odml = true;
        }
    }
    if !any_odml {
        if let Some((pos, size)) = ctx.idx1 {
            let mut totals: Vec<(u64, u64)> = vec![(0, 0); ctx.streams.len()];
            let entries = (size / 16) as usize;
            let read = entries.min(MAX_INDEX_ENTRIES);
            let mut off = 0usize;
            while off < read {
                let batch = (read - off).min(4096);
                let d = r.read_vec_at(pos + off as u64 * 16, batch * 16);
                if d.len() < 16 {
                    break;
                }
                for e in d.chunks_exact(16) {
                    let id = [e[0], e[1], e[2], e[3]];
                    if let Some((num, _)) = stream_of(&id) {
                        if let Some(t) = totals.get_mut(num) {
                            t.0 += 1;
                            t.1 += le32(e, 12).unwrap_or(0) as u64;
                        }
                    }
                }
                off += batch;
            }
            if read > 0 {
                let scale = entries as f64 / read as f64;
                for (st, (c, b)) in ctx.streams.iter_mut().zip(totals) {
                    if c > 0 {
                        st.index_chunks = Some((c as f64 * scale).round() as u64);
                        st.index_bytes = Some((b as f64 * scale).round() as u64);
                    }
                }
            }
        }
    }
    if !ctx.scan_complete && ctx.movi_scanned > 0 {
        let scale = ctx.movi_total as f64 / ctx.movi_scanned as f64;
        for st in ctx.streams.iter_mut() {
            if st.index_bytes.is_none() && st.chunks > 0 {
                st.index_chunks = Some((st.chunks as f64 * scale).round() as u64);
                st.index_bytes = Some((st.bytes as f64 * scale).round() as u64);
            }
        }
    }
}

/// Sum chunk counts and sizes over an OpenDML index (super index → standard indexes).
fn odml_index_totals(r: &mut Reader, be: bool, indx: &[u8]) -> Option<(u64, u64)> {
    let longs_per_entry = le16(indx, 0)? as usize;
    let index_type = *indx.get(3)?;
    let entries = le32(indx, 4)? as usize;
    match index_type {
        0 => {
            // AVI_INDEX_OF_INDEXES: qwOffset, dwSize, dwDuration per entry (after 24 bytes of header).
            if longs_per_entry != 4 {
                return None;
            }
            let (mut chunks, mut bytes) = (0u64, 0u64);
            for i in 0..entries.min(4096) {
                let off = 24 + i * 16;
                let target = le64(indx, off)?;
                r.seek(target);
                let c = read_chunk(r, be)?;
                if &c.id[..2] != b"ix" {
                    return None;
                }
                let d = r.read_vec_at(c.pos, c.size.min((MAX_INDEX_ENTRIES * 8) as u64) as usize);
                let (c2, b2) = standard_index_totals(&d)?;
                chunks += c2;
                bytes += b2;
            }
            Some((chunks, bytes))
        }
        1 => standard_index_totals(indx),
        _ => None,
    }
}

/// AVI_INDEX_OF_CHUNKS: header (24 bytes with qwBaseOffset) then (dwOffset, dwSize) pairs.
fn standard_index_totals(d: &[u8]) -> Option<(u64, u64)> {
    let longs_per_entry = le16(d, 0)? as usize;
    let index_type = *d.get(3)?;
    let entries = le32(d, 4)? as usize;
    if index_type != 1 || longs_per_entry != 2 {
        return None;
    }
    let (mut chunks, mut bytes) = (0u64, 0u64);
    for i in 0..entries.min(MAX_INDEX_ENTRIES) {
        let off = 24 + i * 8;
        let size = le32(d, off + 4)? & 0x7FFF_FFFF;
        chunks += 1;
        bytes += size as u64;
    }
    Some((chunks, bytes))
}

// ---------------------------------------------------------------------------- AVI emit

fn emit_avi(doc: &mut Doc, ctx: &Avi) {
    {
        let g = doc.general();
        g.set("Format", "AVI");
        if ctx.has_avix {
            g.set("Format_Profile", "OpenDML");
        }
        if let Some(a) = &ctx.avih {
            if a.flags & 0x100 != 0 {
                g.set("Interleaved", "Yes");
            }
        }
    }
    let video_index = ctx.streams.iter().position(|s| s.is_video());
    let video_fps = video_index.and_then(|i| {
        let v = &ctx.streams[i];
        if v.scale > 0 && v.rate > 0 {
            Some(v.rate as f64 / v.scale as f64)
        } else {
            None
        }
    });
    let video_chunks = video_index.map(|i| ctx.streams[i].total_chunks()).unwrap_or(0);
    let mut video_delay: Option<f64> = None;
    for (i, st) in ctx.streams.iter().enumerate() {
        let kind = match &st.fcc_type {
            b"vids" => StreamKind::Video,
            b"auds" => StreamKind::Audio,
            b"txts" => StreamKind::Text,
            _ => continue,
        };
        let mut s = Stream::new(kind);
        s.set_int("StreamOrder", i as i128);
        s.set_int("ID", i as i128);
        if !st.name.is_empty() {
            s.set("Title", &st.name);
        }
        match kind {
            StreamKind::Video => emit_avi_video(&mut s, st, ctx),
            StreamKind::Audio => emit_avi_audio(&mut s, st, video_fps, video_chunks, video_index.is_some()),
            _ => {
                s.set_if_empty("Format", fourcc_str(&st.handler).trim().to_string());
            }
        }
        // Timing from the stream header.
        let dur = st.seconds(st.length as u64).map(|s| s * 1000.0);
        if let Some(d) = dur.filter(|d| *d > 0.0) {
            s.set("Duration", format!("{}", d.round() as i64));
        }
        let delay = st.seconds(st.start as u64).map(|s| s * 1000.0).unwrap_or(0.0);
        s.set("Delay", format!("{}", delay.round() as i64));
        if kind == StreamKind::Audio {
            s.set("Delay_Source", "Stream");
            if let Some(vd) = video_delay {
                s.set("Video_Delay", format!("{}", (delay - vd).round() as i64));
            }
        } else if kind == StreamKind::Video && video_delay.is_none() {
            video_delay = Some(delay);
        }
        let bytes = st.total_bytes();
        if bytes > 0 {
            s.set("StreamSize", bytes.to_string());
        }
        doc.streams[kind as usize].push(s);
    }
    // Audio streams declared before the video stream still get a Video_Delay.
    if let Some(vd) = video_delay {
        for a in doc.streams[StreamKind::Audio as usize].iter_mut() {
            if !a.has("Video_Delay") {
                if let Some(d) = a.get_f64("Delay") {
                    a.set("Video_Delay", format!("{}", (d - vd).round() as i64));
                }
            }
        }
    }
}

fn is_avc_fourcc(f: &str) -> bool {
    matches!(f, "H264" | "h264" | "X264" | "x264" | "AVC1" | "avc1" | "DAVC" | "VSSH")
}

fn is_mpeg4v_fourcc(f: &str) -> bool {
    matches!(f, "FMP4" | "XVID" | "DIVX" | "DX50" | "MP4V" | "mp4v" | "M4S2" | "3IV2" | "DIV5" | "DIV6")
}

fn is_mjpeg_fourcc(f: &str) -> bool {
    matches!(f, "MJPG" | "mjpg" | "dmb1" | "JPEG" | "jpeg" | "MJPA" | "MJPB" | "AVRn")
}

fn emit_avi_video(s: &mut Stream, st: &AviStream, ctx: &Avi) {
    // Container facts first so that codec helpers (set_if_empty) do not override them.
    let bih = &st.strf;
    let width = le32(bih, 4).map(|w| w as i32).unwrap_or(0).unsigned_abs();
    let height = le32(bih, 8).map(|h| h as i32).unwrap_or(0).unsigned_abs();
    let compression: Vec<u8> = bih.get(16..20).map(|c| c.to_vec()).unwrap_or_default();
    let codec = if is_printable_fourcc(&compression) {
        fourcc_str(&compression)
    } else if is_printable_fourcc(&st.handler) {
        fourcc_str(&st.handler)
    } else if let Some(c) = le32(bih, 16) {
        format!("{c:X}")
    } else {
        String::new()
    };
    if width > 0 && height > 0 {
        s.set("Width", width.to_string());
        s.set("Height", height.to_string());
    } else if let Some(a) = &ctx.avih {
        if a.width > 0 && a.height > 0 {
            s.set("Width", a.width.to_string());
            s.set("Height", a.height.to_string());
        }
    }
    if let Some((x, y)) = st.aspect {
        if x > 0 && y > 0 {
            s.set("DisplayAspectRatio", format!("{:.3}", x as f64 / y as f64));
        }
    }
    if st.scale > 0 && st.rate > 0 {
        s.set("FrameRate", format!("{:.3}", st.rate as f64 / st.scale as f64));
    } else if let Some(a) = ctx.avih.as_ref().filter(|a| a.us_per_frame > 0) {
        s.set("FrameRate", format!("{:.3}", 1_000_000.0 / a.us_per_frame as f64));
    }
    match st.fields_per_frame {
        Some(1) => s.set("ScanType", "Progressive"),
        Some(2) => s.set("ScanType", "Interlaced"),
        _ => {}
    }
    if st.length > 0 {
        s.set("FrameCount", st.length.to_string());
    } else if let Some(n) = ctx.dmlh_total_frames.filter(|n| *n > 0) {
        s.set("FrameCount", n.to_string());
    }
    // Codec helpers.
    fourcc::apply_bitmapinfoheader(s, bih);
    if !codec.is_empty() {
        s.set_if_empty("CodecID", &codec);
    }
    if !s.has("Format") {
        if let Some((f, _, _)) = fourcc::fourcc_format(&codec) {
            s.set("Format", f);
        }
    }
    let format = s.get("Format").to_string();
    let first = st.frames.first().map(|v| v.as_slice()).unwrap_or(&[]);
    if format == "AVC" || (format.is_empty() && is_avc_fourcc(&codec)) {
        s.set_if_empty("Format", "AVC");
        let nals = avc::nals_annexb(first);
        if let Some((_, sps)) = nals.iter().find(|(t, _)| *t == 7) {
            if let Some(sps) = avc::parse_sps(sps) {
                let cabac = nals.iter().find(|(t, _)| *t == 8).and_then(|(_, p)| avc::parse_pps_cabac(p));
                avc::apply(s, &sps, cabac, true);
            }
        }
        avc::apply_sei_from_nals(s, &nals);
    } else if format == "MPEG-4 Visual" || (format.is_empty() && is_mpeg4v_fourcc(&codec)) {
        s.set_if_empty("Format", "MPEG-4 Visual");
        for f in &st.frames {
            if mpeg4v::apply_headers(s, f) {
                break;
            }
        }
        mpeg4v::apply_frame_user_data(s, first);
    } else if matches!(format.as_str(), "JPEG" | "M-JPEG" | "Motion JPEG") || (format.is_empty() && is_mjpeg_fourcc(&codec)) {
        for f in &st.frames {
            if mjpeg::apply_frame(s, f) {
                break;
            }
        }
        s.set_if_empty("Format", "JPEG");
    }
    if !s.has("Format") && !codec.is_empty() {
        s.set("Format", &codec);
    }
}

fn emit_avi_audio(s: &mut Stream, st: &AviStream, video_fps: Option<f64>, video_chunks: u64, has_video: bool) {
    let fmt = apply_audio_format(s, &st.strf, &st.frames);
    if !has_video || st.total_chunks() == 0 {
        return;
    }
    let chunks = st.total_chunks();
    let audio_ms = st.seconds(st.length as u64).map(|s| s * 1000.0).unwrap_or(0.0);
    let chunk_ms = audio_ms / chunks as f64;
    // Alignment: frame-based codecs must start every chunk on a frame; others on a block.
    let tag = fmt.as_ref().map(|f| f.effective_tag()).unwrap_or(0);
    let block_align = fmt.as_ref().map(|f| f.block_align as u64).unwrap_or(0);
    let aligned = match tag {
        0x0050 | 0x0055 => st.frames.iter().all(|f| f.len() >= 2 && f[0] == 0xFF && f[1] & 0xE0 == 0xE0),
        0x2000 => st.frames.iter().all(|f| f.len() >= 2 && f[0] == 0x0B && f[1] == 0x77),
        _ => block_align <= 1 || st.chunk_sizes.iter().all(|c| c % block_align == 0),
    };
    s.set("Alignment", if aligned { "Aligned" } else { "Split" });
    if video_chunks > 0 {
        let ratio = video_chunks as f64 / chunks as f64;
        s.set("Interleave_VideoFrames", format!("{ratio:.2}"));
        if let Some(fps) = video_fps.filter(|f| *f > 0.0) {
            s.set("Interleave_Duration", format!("{}", (ratio * 1000.0 / fps).floor() as i64));
        }
    }
    if st.chunks_before_video > 0 && chunk_ms > 0.0 {
        s.set("Interleave_Preload", format!("{}", (st.chunks_before_video as f64 * chunk_ms).round() as i64));
    }
}

// ---------------------------------------------------------------------------- WAVE

#[derive(Debug, Default)]
struct Wave {
    fmt: Vec<u8>,
    data: Option<(u64, u64)>,
    fact_samples: Option<u64>,
    ds64_data: Option<u64>,
    ds64_samples: Option<u64>,
}

fn parse_wave(r: &mut Reader, doc: &mut Doc, be: bool, rf64: bool) -> bool {
    r.seek(4);
    let riff_size = if be { r.read_u32be() } else { r.read_u32le() }.unwrap_or(0) as u64;
    let mut end = if riff_size == 0xFFFF_FFFF || riff_size == 0 { r.len() } else { (riff_size + 8).min(r.len()) };
    r.seek(12);
    let mut w = Wave::default();
    let mut n = 0;
    while r.pos() + 8 <= end && n < 4096 {
        n += 1;
        let Some(mut c) = read_chunk(r, be) else { break };
        let len = c.size.min(MAX_META_CHUNK as u64) as usize;
        match &c.id {
            b"ds64" if rf64 => {
                let d = r.read_vec_at(c.pos, len.min(64));
                if let Some(riff) = le64(&d, 0).filter(|v| *v > 0) {
                    end = riff.saturating_add(8).min(r.len());
                }
                w.ds64_data = le64(&d, 8);
                w.ds64_samples = le64(&d, 16);
            }
            b"fmt " => w.fmt = r.read_vec_at(c.pos, len),
            b"data" => {
                if c.size == 0xFFFF_FFFF {
                    c.size = w.ds64_data.unwrap_or(r.len().saturating_sub(c.pos));
                }
                let size = c.size.min(r.len().saturating_sub(c.pos));
                if w.data.is_none() {
                    w.data = Some((c.pos, size));
                }
                if c.end() >= r.len() {
                    break;
                }
            }
            b"fact" => {
                let d = r.read_vec_at(c.pos, 8);
                w.fact_samples = le32(&d, 0).map(|v| v as u64);
            }
            b"LIST" => {
                let form = r.read_fourcc().unwrap_or([0; 4]);
                if &form == b"INFO" {
                    apply_info_list(r, c.end().min(end), be, doc.general());
                }
            }
            b"bext" => {
                let d = r.read_vec_at(c.pos, len);
                apply_bext(doc.general(), &d);
            }
            _ => {}
        }
        r.seek(c.next());
    }
    if w.fmt.is_empty() && w.data.is_none() {
        return false;
    }
    emit_wave(doc, r, &w, rf64);
    true
}

/// Broadcast Wave `bext` chunk.
fn apply_bext(g: &mut Stream, d: &[u8]) {
    let text = |range: std::ops::Range<usize>| d.get(range).map(|b| clean_text(&cstr(b))).unwrap_or_default();
    let description = text(0..256);
    if !description.is_empty() {
        g.set_if_empty("Description", description);
    }
    let originator = text(256..288);
    if !originator.is_empty() {
        g.set_if_empty("Producer", originator);
    }
    let date = text(320..330);
    let time = text(330..338);
    if date.len() == 10 {
        let date = date.replace([':', '_', '/', '.'], "-");
        g.set_if_empty("Encoded_Date", if time.len() == 8 { format!("{date} {time}") } else { date });
    }
    if d.len() > 602 {
        let history = clean_text(&cstr(&d[602..]));
        if !history.is_empty() {
            g.set_if_empty("Encoded_Library_Settings", history);
        }
    }
}

fn emit_wave(doc: &mut Doc, r: &mut Reader, w: &Wave, rf64: bool) {
    {
        let g = doc.general();
        g.set("Format", "Wave");
        if rf64 {
            g.set("Format_Profile", "RF64");
        }
    }
    let mut s = Stream::new(StreamKind::Audio);
    let probe = w.data.map(|(pos, size)| r.read_vec_at(pos, size.min(WAVE_DATA_PROBE as u64) as usize)).unwrap_or_default();
    let frames: Vec<Vec<u8>> = if probe.is_empty() { Vec::new() } else { vec![probe] };
    let fmt = apply_audio_format(&mut s, &w.fmt, &frames);
    if let Some((_, size)) = w.data {
        s.set("StreamSize", size.to_string());
        if let Some(f) = &fmt {
            let seconds = if f.is_pcm() && f.block_align > 0 && f.rate > 0 {
                Some(size as f64 / f.block_align as f64 / f.rate as f64)
            } else if f.avg_bytes > 0 {
                Some(size as f64 / f.avg_bytes as f64)
            } else if f.rate > 0 {
                w.ds64_samples.or(w.fact_samples).map(|n| n as f64 / f.rate as f64)
            } else {
                None
            };
            if let Some(sec) = seconds.filter(|s| *s > 0.0) {
                s.set("Duration", format!("{}", (sec * 1000.0).round() as i64));
            }
        }
    }
    doc.streams[StreamKind::Audio as usize].push(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(id);
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    fn list(form: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = form.to_vec();
        for c in children {
            payload.extend_from_slice(c);
        }
        chunk(b"LIST", &payload)
    }

    fn riff(form: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = form.to_vec();
        for c in children {
            payload.extend_from_slice(c);
        }
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(&payload);
        v
    }

    fn u32s(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn pcm_fmt(channels: u16, rate: u32, bits: u16) -> Vec<u8> {
        let block = channels * bits / 8;
        let mut v = Vec::new();
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * block as u32).to_le_bytes());
        v.extend_from_slice(&block.to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v
    }

    #[test]
    fn wave_format_parsing() {
        let f = parse_wave_format(&pcm_fmt(2, 44100, 16)).unwrap();
        assert_eq!((f.tag, f.channels, f.rate, f.avg_bytes, f.block_align, f.bits), (1, 2, 44100, 176400, 4, 16));
        assert_eq!(f.codec_id(), "1");
        assert!(f.is_pcm());
        // WAVE_FORMAT_EXTENSIBLE, IEEE float, 6 channels.
        let mut ext = vec![0xFE, 0xFF, 6, 0, 0x80, 0xBB, 0, 0, 0, 0x94, 0x11, 0, 0x18, 0, 0x20, 0, 0x16, 0, 0x20, 0, 0x3F, 0, 0, 0];
        ext.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]);
        let f = parse_wave_format(&ext).unwrap();
        assert_eq!(f.effective_tag(), 3);
        assert!(f.is_float());
        assert_eq!(f.channel_mask, 0x3F);
        assert_eq!(f.codec_id(), "00000003-0000-0010-8000-00AA00389B71");
        assert!(parse_wave_format(&[1, 0, 2]).is_none());
    }

    #[test]
    fn probe_scores() {
        let p = |head: &[u8]| probe(&Probe { head, ext: "", size: head.len() as u64 });
        assert_eq!(p(b"RIFF\0\0\0\0AVI LIST"), 100);
        assert_eq!(p(b"RF64\xff\xff\xff\xffWAVEds64"), 100);
        assert_eq!(p(b"RIFF\0\0\0\0WEBPVP8 "), 100);
        assert_eq!(p(b"RIFF\0\0\0\0ACONanih"), 0);
        assert_eq!(p(b"RIFF"), 0);
    }

    #[test]
    fn wave_pcm() {
        let data = vec![0u8; 8000];
        let file = riff(b"WAVE", &[chunk(b"fmt ", &pcm_fmt(1, 8000, 16)), list(b"INFO", &[chunk(b"ISFT", b"test\0"), chunk(b"INAM", b"name\0")]), chunk(b"data", &data)]);
        let mut r = Reader::from_bytes(file);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "Wave");
        assert_eq!(g.get("Encoded_Application"), "test");
        assert_eq!(g.get("Title"), "name");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("CodecID"), "1");
        assert_eq!(a.get("Format"), "PCM");
        assert_eq!(a.get("SamplingRate"), "8000");
        assert_eq!(a.get("Channel(s)"), "1");
        assert_eq!(a.get("BitDepth"), "16");
        assert_eq!(a.get("BitRate"), "128000");
        assert_eq!(a.get("BitRate_Mode"), "CBR");
        assert_eq!(a.get("StreamSize"), "8000");
        assert_eq!(a.get("Duration"), "500");
    }

    #[test]
    fn rf64_uses_ds64_sizes() {
        let mut ds64 = Vec::new();
        ds64.extend_from_slice(&0u64.to_le_bytes()); // riff size (unknown → whole file)
        ds64.extend_from_slice(&4000u64.to_le_bytes()); // data size
        ds64.extend_from_slice(&1000u64.to_le_bytes()); // sample count
        ds64.extend_from_slice(&0u32.to_le_bytes());
        let mut file = b"RF64\xff\xff\xff\xffWAVE".to_vec();
        file.extend_from_slice(&chunk(b"ds64", &ds64));
        file.extend_from_slice(&chunk(b"fmt ", &pcm_fmt(2, 1000, 16)));
        file.extend_from_slice(b"data\xff\xff\xff\xff");
        file.extend_from_slice(&vec![0u8; 4000]);
        let mut r = Reader::from_bytes(file);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert_eq!(doc.general_ref().get("Format_Profile"), "RF64");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("StreamSize"), "4000");
        assert_eq!(a.get("Duration"), "1000");
    }

    fn avi_file() -> Vec<u8> {
        // 2 streams: video 10 frames at 25 fps, audio 21 chunks of 100 bytes.
        let avih = u32s(&[40000, 0, 0, 0x110, 10, 0, 2, 0, 32, 24, 0, 0, 0, 0]);
        let mut strh_v = b"vidsTEST".to_vec();
        strh_v.extend_from_slice(&u32s(&[0, 0, 0, 1, 25, 0, 10, 0, 0xFFFF_FFFF, 0]));
        strh_v.extend_from_slice(&[0, 0, 0, 0, 32, 0, 24, 0]);
        let mut bih = u32s(&[40, 32, 24]);
        bih.extend_from_slice(&[1, 0, 24, 0]);
        bih.extend_from_slice(b"TEST");
        bih.extend_from_slice(&u32s(&[0, 0, 0, 0, 0]));
        let mut vprp = u32s(&[0, 0, 25, 32, 24, (4 << 16) | 3, 32, 24, 1]);
        vprp.extend_from_slice(&u32s(&[0; 8]));
        let strl_v = list(b"strl", &[chunk(b"strh", &strh_v), chunk(b"strf", &bih), chunk(b"vprp", &vprp), chunk(b"strn", b"Video track\0")]);
        let mut strh_a = b"auds".to_vec();
        strh_a.extend_from_slice(&u32s(&[1, 0, 0, 0, 1, 8000, 0, 8000, 0, 0xFFFF_FFFF, 2, 0, 0]));
        let strl_a = list(b"strl", &[chunk(b"strh", &strh_a), chunk(b"strf", &pcm_fmt(1, 8000, 16))]);
        let hdrl = list(b"hdrl", &[chunk(b"avih", &avih), strl_v, strl_a]);
        let info = list(b"INFO", &[chunk(b"ISFT", b"muxer 1.0\0")]);
        let mut movi_children = Vec::new();
        let mut idx = Vec::new();
        let mut offset = 4u32;
        // one audio chunk before the first video frame (preload)
        let mut order: Vec<(u8, usize)> = vec![(1, 100)];
        for _ in 0..10 {
            order.push((0, 200));
            order.push((1, 100));
            order.push((1, 100));
        }
        for (stream, size) in order {
            let id: [u8; 4] = if stream == 0 { *b"00dc" } else { *b"01wb" };
            movi_children.push(chunk(&id, &vec![0x11u8; size]));
            idx.extend_from_slice(&id);
            idx.extend_from_slice(&u32s(&[0x10, offset, size as u32]));
            offset += 8 + size as u32;
        }
        let movi = list(b"movi", &movi_children);
        riff(b"AVI ", &[hdrl, info, movi, chunk(b"idx1", &idx)])
    }

    #[test]
    fn avi_streams_and_interleave() {
        let mut r = Reader::from_bytes(avi_file());
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "AVI");
        assert_eq!(g.get("Interleaved"), "Yes");
        assert_eq!(g.get("Encoded_Application"), "muxer 1.0");
        let v = doc.stream(StreamKind::Video, 0).unwrap();
        assert_eq!(v.get("ID"), "0");
        assert_eq!(v.get("CodecID"), "TEST");
        assert_eq!(v.get("Format"), "TEST");
        assert_eq!(v.get("Width"), "32");
        assert_eq!(v.get("Height"), "24");
        assert_eq!(v.get("FrameRate"), "25.000");
        assert_eq!(v.get("FrameCount"), "10");
        assert_eq!(v.get("Duration"), "400");
        assert_eq!(v.get("ScanType"), "Progressive");
        assert_eq!(v.get("DisplayAspectRatio"), "1.333");
        assert_eq!(v.get("StreamSize"), "2000");
        assert_eq!(v.get("Title"), "Video track");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("ID"), "1");
        assert_eq!(a.get("Format"), "PCM");
        assert_eq!(a.get("Duration"), "1000");
        assert_eq!(a.get("StreamSize"), "2100");
        assert_eq!(a.get("Delay_Source"), "Stream");
        assert_eq!(a.get("Video_Delay"), "0");
        assert_eq!(a.get("Alignment"), "Aligned");
        assert_eq!(a.get("Interleave_VideoFrames"), "0.48"); // 10 / 21
        assert_eq!(a.get("Interleave_Duration"), "19"); // floor(0.476 * 40)
        assert_eq!(a.get("Interleave_Preload"), "48"); // 1 chunk of 1000/21 ms
    }

    #[test]
    fn avi_without_index_scans_movi() {
        let file = avi_file();
        // Drop the idx1 chunk by truncating the buffer and fixing the RIFF size.
        let idx_pos = file.windows(4).rposition(|w| w == b"idx1").unwrap();
        let mut truncated = file[..idx_pos].to_vec();
        let size = (truncated.len() - 8) as u32;
        truncated[4..8].copy_from_slice(&size.to_le_bytes());
        let mut r = Reader::from_bytes(truncated);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert_eq!(doc.stream(StreamKind::Video, 0).unwrap().get("StreamSize"), "2000");
        assert_eq!(doc.stream(StreamKind::Audio, 0).unwrap().get("StreamSize"), "2100");
    }

    #[test]
    fn standard_index_sums() {
        let mut d = vec![2, 0, 0, 1];
        d.extend_from_slice(&3u32.to_le_bytes());
        d.extend_from_slice(b"00dc");
        d.extend_from_slice(&[0u8; 12]);
        for size in [100u32, 0x8000_0200, 300] {
            d.extend_from_slice(&u32s(&[0, size]));
        }
        assert_eq!(standard_index_totals(&d), Some((3, 912)));
        assert_eq!(standard_index_totals(&d[..10]), None);
    }

    #[test]
    fn malformed_inputs_do_not_panic() {
        for data in [b"RIFF\0\0\0\0AVI ".to_vec(), b"RIFF\xff\xff\xff\xffWAVEfmt \xff\xff\xff\xff".to_vec(), b"RIFF\x10\0\0\0AVI LIST\xff\xff\xff\xffhdrl".to_vec()] {
            let mut r = Reader::from_bytes(data);
            let mut doc = Doc::new();
            let _ = parse(&mut r, &mut doc);
        }
    }
}

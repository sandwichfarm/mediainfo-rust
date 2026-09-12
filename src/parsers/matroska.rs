//! Matroska / WebM (EBML) container.

use crate::io::Reader;
use crate::model::{Doc, Stream, StreamKind, OPT_SHOWN};
use crate::parsers::audio::{self, aac, ac3, alac, dts, flac, mlp, mpeg_audio, opus, pcm, vorbis, wma};
use crate::parsers::video::{self, av1, avc, hevc, mpeg4v, mpegv, theora, vp8, vp9};
use crate::parsers::Probe;

// EBML / Matroska element IDs
const EBML: u32 = 0x1A45DFA3;
const DOCTYPE: u32 = 0x4282;
const DOCTYPE_VERSION: u32 = 0x4287;
const SEGMENT: u32 = 0x18538067;
const SEEKHEAD: u32 = 0x114D9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549A966;
const TIMECODE_SCALE: u32 = 0x2AD7B1;
const DURATION: u32 = 0x4489;
const TITLE: u32 = 0x7BA9;
const MUXING_APP: u32 = 0x4D80;
const WRITING_APP: u32 = 0x5741;
const SEGMENT_UID: u32 = 0x73A4;
const DATE_UTC: u32 = 0x4461;
const TRACKS: u32 = 0x1654AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_UID: u32 = 0x73C5;
const TRACK_TYPE: u32 = 0x83;
const FLAG_ENABLED: u32 = 0xB9;
const FLAG_DEFAULT: u32 = 0x88;
const FLAG_FORCED: u32 = 0x55AA;
const DEFAULT_DURATION: u32 = 0x23E383;
const NAME: u32 = 0x536E;
const LANGUAGE: u32 = 0x22B59C;
const LANGUAGE_IETF: u32 = 0x22B59D;
const CODEC_ID: u32 = 0x86;
const CODEC_PRIVATE: u32 = 0x63A2;
const CODEC_NAME: u32 = 0x258688;
const CODEC_DELAY: u32 = 0x56AA;
const VIDEO: u32 = 0xE0;
const FLAG_INTERLACED: u32 = 0x9A;
const FIELD_ORDER: u32 = 0x9D;
const STEREO_MODE: u32 = 0x53B8;
const PIXEL_WIDTH: u32 = 0xB0;
const PIXEL_HEIGHT: u32 = 0xBA;
const PIXEL_CROP_BOTTOM: u32 = 0x54AA;
const PIXEL_CROP_TOP: u32 = 0x54BB;
const PIXEL_CROP_LEFT: u32 = 0x54CC;
const PIXEL_CROP_RIGHT: u32 = 0x54DD;
const DISPLAY_WIDTH: u32 = 0x54B0;
const DISPLAY_HEIGHT: u32 = 0x54BA;
const DISPLAY_UNIT: u32 = 0x54B2;
const COLOUR: u32 = 0x55B0;
const MATRIX_COEFFICIENTS: u32 = 0x55B1;
const BITS_PER_CHANNEL: u32 = 0x55B2;
const RANGE: u32 = 0x55B9;
const TRANSFER_CHARACTERISTICS: u32 = 0x55BA;
const PRIMARIES: u32 = 0x55BB;
const MAX_CLL: u32 = 0x55BC;
const MAX_FALL: u32 = 0x55BD;
const MASTERING_METADATA: u32 = 0x55D0;
const AUDIO: u32 = 0xE1;
const SAMPLING_FREQUENCY: u32 = 0xB5;
const OUTPUT_SAMPLING_FREQUENCY: u32 = 0x78B5;
const CHANNELS: u32 = 0x9F;
const BIT_DEPTH: u32 = 0x6264;
const CONTENT_ENCODINGS: u32 = 0x6D80;
const CONTENT_ENCODING: u32 = 0x6240;
const CONTENT_ENCODING_SCOPE: u32 = 0x5032;
const CONTENT_ENCODING_TYPE: u32 = 0x5033;
const CONTENT_COMPRESSION: u32 = 0x5034;
const CONTENT_COMP_ALGO: u32 = 0x4254;
const CONTENT_COMP_SETTINGS: u32 = 0x4255;
const CLUSTER: u32 = 0x1F43B675;
const TIMECODE: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const BLOCK_DURATION: u32 = 0x9B;
const CUES: u32 = 0x1C53BB6B;
const CUE_POINT: u32 = 0xBB;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_CLUSTER_POSITION: u32 = 0xF1;
const ATTACHMENTS: u32 = 0x1941A469;
const ATTACHED_FILE: u32 = 0x61A7;
const FILE_NAME: u32 = 0x466E;
const FILE_MIME_TYPE: u32 = 0x4660;
const FILE_DATA: u32 = 0x465C;
const CHAPTERS: u32 = 0x1043A770;
const EDITION_ENTRY: u32 = 0x45B9;
const CHAPTER_ATOM: u32 = 0xB6;
const CHAPTER_TIME_START: u32 = 0x91;
const CHAPTER_FLAG_HIDDEN: u32 = 0x98;
const CHAPTER_DISPLAY: u32 = 0x80;
const CHAP_STRING: u32 = 0x85;
const CHAP_LANGUAGE: u32 = 0x437C;
const TAGS: u32 = 0x1254C367;
const TAG: u32 = 0x7373;
const TARGETS: u32 = 0x63C0;
const TARGET_TYPE_VALUE: u32 = 0x68CA;
const TAG_TRACK_UID: u32 = 0x63C5;
const TAG_CHAPTER_UID: u32 = 0x63C4;
const TAG_ATTACHMENT_UID: u32 = 0x63C6;
const SIMPLE_TAG: u32 = 0x67C8;
const TAG_NAME: u32 = 0x45A3;
const TAG_STRING: u32 = 0x4487;
const CRC32: u32 = 0xBF;
const VOID: u32 = 0xEC;

pub fn probe(p: &Probe) -> u8 {
    if p.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        100
    } else {
        0
    }
}

/// Element header: (id, size (None = unknown), header length).
fn read_header(r: &mut Reader) -> Option<(u32, Option<u64>, u64)> {
    let start = r.pos();
    let b0 = r.read_u8()?;
    let id_len = b0.leading_zeros() as usize + 1;
    if id_len > 4 {
        return None;
    }
    let mut id = b0 as u32;
    for _ in 1..id_len {
        id = (id << 8) | r.read_u8()? as u32;
    }
    let s0 = r.read_u8()?;
    let size_len = s0.leading_zeros() as usize + 1;
    if size_len > 8 {
        return None;
    }
    let mut size = (s0 as u64) & ((1u64 << (8 - size_len)) - 1);
    let mut all_ones = size == (1u64 << (8 - size_len)) - 1;
    for _ in 1..size_len {
        let b = r.read_u8()?;
        all_ones &= b == 0xFF;
        size = (size << 8) | b as u64;
    }
    Some((id, if all_ones { None } else { Some(size) }, r.pos() - start))
}

fn read_uint(r: &mut Reader, size: u64) -> Option<u64> {
    if size > 8 {
        return None;
    }
    let b = r.read_exact(size as usize)?;
    Some(b.iter().fold(0u64, |a, &x| (a << 8) | x as u64))
}

fn read_float(r: &mut Reader, size: u64) -> Option<f64> {
    match size {
        4 => r.read_f32be().map(|f| f as f64),
        8 => r.read_f64be(),
        0 => Some(0.0),
        _ => {
            r.skip(size);
            None
        }
    }
}

fn read_string(r: &mut Reader, size: u64) -> String {
    let b = r.read(size.min(1 << 20) as usize);
    crate::io::clean_text(&crate::io::cstr(&b))
}

fn read_bin(r: &mut Reader, size: u64) -> Vec<u8> {
    r.read(size.min(16 << 20) as usize)
}

#[derive(Debug, Default, Clone)]
struct Track {
    number: u64,
    uid: u64,
    kind: u64,
    default: Option<bool>,
    forced: bool,
    enabled: bool,
    default_duration: Option<u64>,
    name: String,
    language: String,
    language_ietf: String,
    codec_id: String,
    codec_private: Vec<u8>,
    codec_name: String,
    codec_delay: u64,
    // video
    interlaced: Option<u64>,
    field_order: Option<u64>,
    stereo_mode: Option<u64>,
    pixel_width: u64,
    pixel_height: u64,
    crop: [u64; 4],
    display_width: u64,
    display_height: u64,
    display_unit: u64,
    colour: Colour,
    // audio
    sampling: f64,
    output_sampling: f64,
    channels: u64,
    bit_depth: u64,
    // content encodings
    header_strip: Vec<u8>,
    compression: Option<u64>,
    // block statistics
    first_ts: Option<f64>,
    last_ts: Option<f64>,
    last_duration: Option<f64>,
    frames: u64,
    bytes: u64,
    first_frame: Option<Vec<u8>>,
    frames_for_codec: Vec<Vec<u8>>,
    timestamps: Vec<f64>,
}

#[derive(Debug, Default, Clone)]
struct Colour {
    matrix: Option<u64>,
    bits: Option<u64>,
    range: Option<u64>,
    transfer: Option<u64>,
    primaries: Option<u64>,
    max_cll: Option<u64>,
    max_fall: Option<u64>,
    mastering: bool,
}

#[derive(Debug, Default)]
struct Ctx {
    doctype: String,
    doctype_version: u64,
    timecode_scale: u64,
    duration: Option<f64>,
    title: String,
    muxing_app: String,
    writing_app: String,
    segment_uid: Vec<u8>,
    date_utc: Option<i64>,
    tracks: Vec<Track>,
    chapters: Vec<Vec<(u64, String, String, bool)>>, // per edition: (start ns, lang, title, hidden)
    tags: Vec<(u64, u64, Vec<(String, String)>)>,    // (target type value, track uid, tags)
    attachments: Vec<(String, String, u64)>,
    crc_level1: bool,
    segment_pos: u64,
    cues_cluster_positions: Vec<u64>,
    clusters_seen: u64,
    header_size: u64,
    first_cluster_pos: Option<u64>,
    seeks: Vec<(u32, u64)>,
    fully_scanned: bool,
    cluster_bytes: u64,
}

const MAX_CLUSTER_SCAN_BYTES: u64 = 24 * 1024 * 1024;

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let mut ctx = Ctx { timecode_scale: 1_000_000, ..Default::default() };
    // EBML header
    let Some((id, size, _)) = read_header(r) else { return false };
    if id != EBML {
        return false;
    }
    let end = size.map(|s| r.pos() + s).unwrap_or(r.len());
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        match id {
            DOCTYPE => ctx.doctype = read_string(r, size),
            DOCTYPE_VERSION => ctx.doctype_version = read_uint(r, size).unwrap_or(0),
            _ => r.skip(size),
        }
    }
    r.seek(end);
    // Segment
    let Some((id, size, _)) = read_header(r) else { return false };
    if id != SEGMENT {
        return false;
    }
    ctx.segment_pos = r.pos();
    let seg_end = size.map(|s| r.pos() + s).unwrap_or(r.len()).min(r.len());
    let mut scanned = 0u64;
    let mut scanned_all = true;
    while r.pos() < seg_end {
        let pos = r.pos();
        let Some((id, size, hlen)) = read_header(r) else { break };
        let size_known = size.is_some();
        let size = size.unwrap_or(seg_end - r.pos());
        let elem_end = (r.pos() + size).min(seg_end);
        match id {
            INFO => parse_info(r, elem_end, &mut ctx),
            TRACKS => parse_tracks(r, elem_end, &mut ctx),
            CLUSTER => {
                if ctx.first_cluster_pos.is_none() {
                    ctx.first_cluster_pos = Some(pos);
                    ctx.header_size = pos;
                }
                if scanned < MAX_CLUSTER_SCAN_BYTES {
                    parse_cluster(r, elem_end, size_known, &mut ctx);
                    scanned += size + hlen;
                    ctx.cluster_bytes += size + hlen;
                } else {
                    scanned_all = false;
                    // Skip ahead: leave the remaining clusters, we will sample the tail below.
                    break;
                }
            }
            CUES => parse_cues(r, elem_end, &mut ctx),
            CHAPTERS => parse_chapters(r, elem_end, &mut ctx),
            TAGS => parse_tags(r, elem_end, &mut ctx),
            ATTACHMENTS => parse_attachments(r, elem_end, &mut ctx),
            SEEKHEAD => parse_seekhead(r, elem_end, &mut ctx),
            VOID | CRC32 => {}
            _ => {}
        }
        if !size_known && id == CLUSTER {
            // unknown-size cluster: parse_cluster stops at the next top-level element
            continue;
        }
        r.seek(elem_end);
    }
    ctx.fully_scanned = scanned_all;
    if !scanned_all {
        // Elements after the clusters: reach them through the SeekHead instead of walking every cluster.
        let resume = r.pos();
        let mut visited = 0;
        let seeks = ctx.seeks.clone();
        for (id, pos) in seeks {
            let abs = ctx.segment_pos + pos;
            if abs < resume || abs >= seg_end || visited > 64 {
                continue;
            }
            visited += 1;
            r.seek(abs);
            let Some((eid, size, _)) = read_header(r) else { continue };
            if eid != id {
                continue;
            }
            let end = size.map(|s| r.pos() + s).unwrap_or(seg_end).min(seg_end);
            match eid {
                CUES => parse_cues(r, end, &mut ctx),
                CHAPTERS => parse_chapters(r, end, &mut ctx),
                TAGS => parse_tags(r, end, &mut ctx),
                ATTACHMENTS => parse_attachments(r, end, &mut ctx),
                SEEKHEAD => parse_seekhead(r, end, &mut ctx),
                INFO => parse_info(r, end, &mut ctx),
                TRACKS => parse_tracks(r, end, &mut ctx),
                _ => {}
            }
        }
        scan_tail(r, seg_end, &mut ctx);
    }
    emit(doc, &ctx, r.len());
    true
}

fn parse_seekhead(r: &mut Reader, end: u64, ctx: &mut Ctx) {
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        if id == SEEK {
            let (mut sid, mut spos) = (0u32, 0u64);
            while r.pos() < next {
                let Some((id, size, _)) = read_header(r) else { break };
                let size = size.unwrap_or(0);
                let n2 = r.pos() + size;
                match id {
                    SEEK_ID => sid = read_uint(r, size).unwrap_or(0) as u32,
                    SEEK_POSITION => spos = read_uint(r, size).unwrap_or(0),
                    _ => {}
                }
                r.seek(n2);
            }
            if sid != 0 && ctx.seeks.len() < 256 {
                ctx.seeks.push((sid, spos));
            }
        }
        r.seek(next);
    }
}

fn parse_info(r: &mut Reader, end: u64, ctx: &mut Ctx) {
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        match id {
            CRC32 => ctx.crc_level1 = true,
            TIMECODE_SCALE => ctx.timecode_scale = read_uint(r, size).unwrap_or(1_000_000).max(1),
            DURATION => ctx.duration = read_float(r, size),
            TITLE => ctx.title = read_string(r, size),
            MUXING_APP => ctx.muxing_app = read_string(r, size),
            WRITING_APP => ctx.writing_app = read_string(r, size),
            SEGMENT_UID => ctx.segment_uid = read_bin(r, size),
            DATE_UTC => ctx.date_utc = read_uint(r, size).map(|v| v as i64),
            _ => {}
        }
        r.seek(next);
    }
}

fn parse_tracks(r: &mut Reader, end: u64, ctx: &mut Ctx) {
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        if id == CRC32 {
            ctx.crc_level1 = true;
        }
        if id == TRACK_ENTRY {
            let mut t = Track { enabled: true, ..Default::default() };
            parse_track_entry(r, next, &mut t);
            ctx.tracks.push(t);
        }
        r.seek(next);
    }
}

fn parse_track_entry(r: &mut Reader, end: u64, t: &mut Track) {
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        match id {
            TRACK_NUMBER => t.number = read_uint(r, size).unwrap_or(0),
            TRACK_UID => t.uid = read_uint(r, size).unwrap_or(0),
            TRACK_TYPE => t.kind = read_uint(r, size).unwrap_or(0),
            FLAG_ENABLED => t.enabled = read_uint(r, size).unwrap_or(1) != 0,
            FLAG_DEFAULT => t.default = Some(read_uint(r, size).unwrap_or(1) != 0),
            FLAG_FORCED => t.forced = read_uint(r, size).unwrap_or(0) != 0,
            DEFAULT_DURATION => t.default_duration = read_uint(r, size),
            NAME => t.name = read_string(r, size),
            LANGUAGE => t.language = read_string(r, size),
            LANGUAGE_IETF => t.language_ietf = read_string(r, size),
            CODEC_ID => t.codec_id = read_string(r, size),
            CODEC_PRIVATE => t.codec_private = read_bin(r, size),
            CODEC_NAME => t.codec_name = read_string(r, size),
            CODEC_DELAY => t.codec_delay = read_uint(r, size).unwrap_or(0),
            VIDEO => parse_video(r, next, t),
            AUDIO => parse_audio(r, next, t),
            CONTENT_ENCODINGS => parse_content_encodings(r, next, t),
            _ => {}
        }
        r.seek(next);
    }
}

fn parse_video(r: &mut Reader, end: u64, t: &mut Track) {
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        match id {
            FLAG_INTERLACED => t.interlaced = read_uint(r, size),
            FIELD_ORDER => t.field_order = read_uint(r, size),
            STEREO_MODE => t.stereo_mode = read_uint(r, size),
            PIXEL_WIDTH => t.pixel_width = read_uint(r, size).unwrap_or(0),
            PIXEL_HEIGHT => t.pixel_height = read_uint(r, size).unwrap_or(0),
            PIXEL_CROP_BOTTOM => t.crop[0] = read_uint(r, size).unwrap_or(0),
            PIXEL_CROP_TOP => t.crop[1] = read_uint(r, size).unwrap_or(0),
            PIXEL_CROP_LEFT => t.crop[2] = read_uint(r, size).unwrap_or(0),
            PIXEL_CROP_RIGHT => t.crop[3] = read_uint(r, size).unwrap_or(0),
            DISPLAY_WIDTH => t.display_width = read_uint(r, size).unwrap_or(0),
            DISPLAY_HEIGHT => t.display_height = read_uint(r, size).unwrap_or(0),
            DISPLAY_UNIT => t.display_unit = read_uint(r, size).unwrap_or(0),
            COLOUR => parse_colour(r, next, &mut t.colour),
            _ => {}
        }
        r.seek(next);
    }
}

fn parse_colour(r: &mut Reader, end: u64, c: &mut Colour) {
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        match id {
            MATRIX_COEFFICIENTS => c.matrix = read_uint(r, size),
            BITS_PER_CHANNEL => c.bits = read_uint(r, size),
            RANGE => c.range = read_uint(r, size),
            TRANSFER_CHARACTERISTICS => c.transfer = read_uint(r, size),
            PRIMARIES => c.primaries = read_uint(r, size),
            MAX_CLL => c.max_cll = read_uint(r, size),
            MAX_FALL => c.max_fall = read_uint(r, size),
            MASTERING_METADATA => c.mastering = true,
            _ => {}
        }
        r.seek(next);
    }
}

fn parse_audio(r: &mut Reader, end: u64, t: &mut Track) {
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        match id {
            SAMPLING_FREQUENCY => t.sampling = read_float(r, size).unwrap_or(8000.0),
            OUTPUT_SAMPLING_FREQUENCY => t.output_sampling = read_float(r, size).unwrap_or(0.0),
            CHANNELS => t.channels = read_uint(r, size).unwrap_or(1),
            BIT_DEPTH => t.bit_depth = read_uint(r, size).unwrap_or(0),
            _ => {}
        }
        r.seek(next);
    }
}

fn parse_content_encodings(r: &mut Reader, end: u64, t: &mut Track) {
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        if id == CONTENT_ENCODING {
            let mut scope = 1;
            while r.pos() < next {
                let Some((id, size, _)) = read_header(r) else { break };
                let size = size.unwrap_or(0);
                let n2 = r.pos() + size;
                match id {
                    CONTENT_ENCODING_SCOPE => scope = read_uint(r, size).unwrap_or(1),
                    CONTENT_ENCODING_TYPE => {}
                    CONTENT_COMPRESSION => {
                        while r.pos() < n2 {
                            let Some((id, size, _)) = read_header(r) else { break };
                            let size = size.unwrap_or(0);
                            let n3 = r.pos() + size;
                            match id {
                                CONTENT_COMP_ALGO => t.compression = read_uint(r, size),
                                CONTENT_COMP_SETTINGS => t.header_strip = read_bin(r, size),
                                _ => {}
                            }
                            r.seek(n3);
                        }
                    }
                    _ => {}
                }
                r.seek(n2);
            }
            let _ = scope;
        }
        r.seek(next);
    }
}

/// Parse blocks of one cluster and record per-track timing statistics.
fn parse_cluster(r: &mut Reader, end: u64, size_known: bool, ctx: &mut Ctx) {
    ctx.clusters_seen += 1;
    let mut cluster_tc: u64 = 0;
    let scale = ctx.timecode_scale as f64 / 1_000_000.0; // ns → ms
    while r.pos() < end {
        let pos = r.pos();
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        match id {
            TIMECODE => cluster_tc = read_uint(r, size).unwrap_or(0),
            SIMPLE_BLOCK => handle_block(r, size, cluster_tc, None, scale, ctx),
            BLOCK_GROUP => {
                let mut duration: Option<u64> = None;
                let mut block: Option<(u64, u64)> = None; // (pos, size)
                while r.pos() < next {
                    let Some((id, size, _)) = read_header(r) else { break };
                    let size = size.unwrap_or(0);
                    let n2 = r.pos() + size;
                    match id {
                        BLOCK => block = Some((r.pos(), size)),
                        BLOCK_DURATION => duration = read_uint(r, size),
                        _ => {}
                    }
                    r.seek(n2);
                }
                if let Some((bpos, bsize)) = block {
                    r.seek(bpos);
                    handle_block(r, bsize, cluster_tc, duration, scale, ctx);
                }
            }
            CLUSTER | CUES | TAGS | CHAPTERS | ATTACHMENTS | TRACKS | INFO if !size_known => {
                // Unknown-size cluster ended: rewind to let the caller handle this element.
                r.seek(pos);
                return;
            }
            _ => {}
        }
        r.seek(next);
    }
}

fn handle_block(r: &mut Reader, size: u64, cluster_tc: u64, duration: Option<u64>, scale: f64, ctx: &mut Ctx) {
    let start = r.pos();
    // track number (vint), timecode (i16), flags
    let Some(b0) = r.read_u8() else { return };
    let len = b0.leading_zeros() as usize + 1;
    if len > 8 {
        return;
    }
    let mut num = (b0 as u64) & ((1u64 << (8 - len)) - 1);
    for _ in 1..len {
        let Some(b) = r.read_u8() else { return };
        num = (num << 8) | b as u64;
    }
    let Some(tc) = r.read_u16be() else { return };
    let Some(flags) = r.read_u8() else { return };
    let header_len = r.pos() - start;
    let payload_len = size.saturating_sub(header_len);
    let ts = (cluster_tc as f64 + tc as i16 as f64) * scale;
    let Some(t) = ctx.tracks.iter_mut().find(|t| t.number == num) else { return };
    let lacing = (flags >> 1) & 3;
    let mut frame_count = 1u64;
    let mut first_frame_len = payload_len as usize;
    let mut first_frame_off = 0usize;
    if lacing != 0 {
        let Some(n) = r.read_u8() else { return };
        frame_count = n as u64 + 1;
        // Determine the first frame size for codec probing.
        match lacing {
            1 => {
                // Xiph
                let mut sz = 0usize;
                let mut off = 1usize;
                loop {
                    let Some(b) = r.read_u8() else { return };
                    off += 1;
                    sz += b as usize;
                    if b != 255 {
                        break;
                    }
                }
                // remaining lace sizes
                for _ in 1..(frame_count - 1) {
                    loop {
                        let Some(b) = r.read_u8() else { return };
                        off += 1;
                        if b != 255 {
                            break;
                        }
                    }
                }
                first_frame_len = sz;
                first_frame_off = off;
            }
            3 => {
                // EBML lacing
                let p0 = r.pos();
                let Some((sz, _)) = read_vint(r) else { return };
                let mut off = (r.pos() - p0) as usize + 1;
                for _ in 1..(frame_count - 1) {
                    let Some((_, l)) = read_vint(r) else { return };
                    off += l as usize;
                }
                first_frame_len = sz as usize;
                first_frame_off = off;
            }
            _ => {
                // fixed
                first_frame_off = 1;
                first_frame_len = (payload_len as usize).saturating_sub(1) / frame_count.max(1) as usize;
            }
        }
    }
    t.frames += frame_count;
    t.bytes += payload_len;
    if t.first_ts.is_none() {
        t.first_ts = Some(ts);
    }
    if t.last_ts.map_or(true, |l| ts >= l) {
        t.last_ts = Some(ts);
        t.last_duration = duration.map(|d| d as f64 * scale);
    }
    if t.timestamps.len() < 64 {
        t.timestamps.push(ts);
    }
    if t.frames_for_codec.len() < if t.kind == 2 { 64 } else { 4 } {
        let want = first_frame_len.min(512 * 1024);
        let data_pos = start + header_len + first_frame_off as u64;
        let mut data = r.read_vec_at(data_pos, want);
        if !t.header_strip.is_empty() && t.compression == Some(3) {
            let mut full = t.header_strip.clone();
            full.extend_from_slice(&data);
            data = full;
        }
        if t.first_frame.is_none() {
            t.first_frame = Some(data.clone());
        }
        t.frames_for_codec.push(data);
    }
}

fn read_vint(r: &mut Reader) -> Option<(u64, u32)> {
    let b0 = r.read_u8()?;
    let len = b0.leading_zeros() + 1;
    if len > 8 {
        return None;
    }
    let mut v = (b0 as u64) & ((1u64 << (8 - len)) - 1);
    for _ in 1..len {
        v = (v << 8) | r.read_u8()? as u64;
    }
    Some((v, len))
}

fn parse_cues(r: &mut Reader, end: u64, ctx: &mut Ctx) {
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        if id == CUE_POINT {
            while r.pos() < next {
                let Some((id, size, _)) = read_header(r) else { break };
                let size = size.unwrap_or(0);
                let n2 = r.pos() + size;
                if id == CUE_TRACK_POSITIONS {
                    while r.pos() < n2 {
                        let Some((id, size, _)) = read_header(r) else { break };
                        let size = size.unwrap_or(0);
                        let n3 = r.pos() + size;
                        if id == CUE_CLUSTER_POSITION {
                            if let Some(p) = read_uint(r, size) {
                                ctx.cues_cluster_positions.push(p);
                            }
                        }
                        r.seek(n3);
                    }
                }
                r.seek(n2);
            }
        }
        r.seek(next);
    }
}

/// When the file is large we stop scanning clusters early; parse the last cluster for the tail
/// timestamps so durations stay accurate.
fn scan_tail(r: &mut Reader, seg_end: u64, ctx: &mut Ctx) {
    let mut candidates: Vec<u64> = Vec::new();
    // The last cluster: search backwards through the final MiBs for a cluster ID whose element ends
    // exactly at the segment end or at another known top-level element.
    let tail_len = seg_end.min(8 * 1024 * 1024);
    let start = seg_end - tail_len;
    let data = r.read_vec_at(start, tail_len as usize);
    let mut i = data.len().saturating_sub(4);
    let mut checked = 0;
    while i > 0 && checked < 4096 {
        if data[i..i + 4] == [0x1F, 0x43, 0xB6, 0x75] {
            checked += 1;
            let pos = start + i as u64;
            r.seek(pos);
            if let Some((id, size, _)) = read_header(r) {
                if id == CLUSTER {
                    let end = size.map(|sz| r.pos() + sz).unwrap_or(seg_end);
                    let valid = end == seg_end || {
                        r.seek(end);
                        matches!(read_header(r).map(|h| h.0), Some(CUES | TAGS | CHAPTERS | ATTACHMENTS | CLUSTER | SEEKHEAD | VOID | INFO | TRACKS))
                    };
                    if valid {
                        candidates.push(pos);
                        break;
                    }
                }
            }
        }
        i -= 1;
    }
    if candidates.is_empty() {
        if let Some(p) = ctx.cues_cluster_positions.iter().max().copied().map(|p| ctx.segment_pos + p) {
            candidates.push(p);
        }
    }
    for pos in candidates {
        r.seek(pos);
        let Some((id, size, _)) = read_header(r) else { continue };
        if id != CLUSTER {
            continue;
        }
        let end = size.map(|s| r.pos() + s).unwrap_or(seg_end).min(seg_end);
        // Keep the head statistics but let last_ts advance.
        parse_cluster(r, end, size.is_some(), ctx);
    }
}

fn parse_chapters(r: &mut Reader, end: u64, ctx: &mut Ctx) {
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        if id == EDITION_ENTRY {
            let mut edition = Vec::new();
            while r.pos() < next {
                let Some((id, size, _)) = read_header(r) else { break };
                let size = size.unwrap_or(0);
                let n2 = r.pos() + size;
                if id == CHAPTER_ATOM {
                    parse_chapter_atom(r, n2, &mut edition);
                }
                r.seek(n2);
            }
            ctx.chapters.push(edition);
        }
        r.seek(next);
    }
}

fn parse_chapter_atom(r: &mut Reader, end: u64, out: &mut Vec<(u64, String, String, bool)>) {
    let (mut start, mut lang, mut title, mut hidden) = (0u64, String::new(), String::new(), false);
    let mut nested: Vec<(u64, String, String, bool)> = Vec::new();
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        match id {
            CHAPTER_TIME_START => start = read_uint(r, size).unwrap_or(0),
            CHAPTER_FLAG_HIDDEN => hidden = read_uint(r, size).unwrap_or(0) != 0,
            CHAPTER_DISPLAY => {
                let (mut s, mut l) = (String::new(), String::new());
                while r.pos() < next {
                    let Some((id, size, _)) = read_header(r) else { break };
                    let size = size.unwrap_or(0);
                    let n2 = r.pos() + size;
                    match id {
                        CHAP_STRING => s = read_string(r, size),
                        CHAP_LANGUAGE => l = read_string(r, size),
                        _ => {}
                    }
                    r.seek(n2);
                }
                if title.is_empty() {
                    title = s;
                    lang = l;
                }
            }
            CHAPTER_ATOM => parse_chapter_atom(r, next, &mut nested),
            _ => {}
        }
        r.seek(next);
    }
    out.push((start, lang, title, hidden));
    out.extend(nested);
}

fn parse_tags(r: &mut Reader, end: u64, ctx: &mut Ctx) {
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        if id == TAG {
            let (mut ttv, mut track_uid, mut other_target) = (50u64, 0u64, false);
            let mut tags = Vec::new();
            while r.pos() < next {
                let Some((id, size, _)) = read_header(r) else { break };
                let size = size.unwrap_or(0);
                let n2 = r.pos() + size;
                match id {
                    TARGETS => {
                        while r.pos() < n2 {
                            let Some((id, size, _)) = read_header(r) else { break };
                            let size = size.unwrap_or(0);
                            let n3 = r.pos() + size;
                            match id {
                                TARGET_TYPE_VALUE => ttv = read_uint(r, size).unwrap_or(50),
                                TAG_TRACK_UID => track_uid = read_uint(r, size).unwrap_or(0),
                                TAG_CHAPTER_UID | TAG_ATTACHMENT_UID => other_target = true,
                                _ => {}
                            }
                            r.seek(n3);
                        }
                    }
                    SIMPLE_TAG => parse_simple_tag(r, n2, &mut tags, ""),
                    _ => {}
                }
                r.seek(n2);
            }
            if !other_target {
                ctx.tags.push((ttv, track_uid, tags));
            }
        }
        r.seek(next);
    }
}

fn parse_simple_tag(r: &mut Reader, end: u64, out: &mut Vec<(String, String)>, prefix: &str) {
    let (mut name, mut value) = (String::new(), String::new());
    let mut nested: Vec<(u64, u64)> = Vec::new();
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        match id {
            TAG_NAME => name = read_string(r, size),
            TAG_STRING => value = read_string(r, size),
            SIMPLE_TAG => nested.push((r.pos(), next)),
            _ => {}
        }
        r.seek(next);
    }
    let full = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
    if !value.is_empty() {
        out.push((full.clone(), value));
    }
    for (s, e) in nested {
        r.seek(s);
        parse_simple_tag(r, e, out, &full);
    }
    r.seek(end);
}

fn parse_attachments(r: &mut Reader, end: u64, ctx: &mut Ctx) {
    while r.pos() < end {
        let Some((id, size, _)) = read_header(r) else { break };
        let size = size.unwrap_or(0);
        let next = r.pos() + size;
        if id == ATTACHED_FILE {
            let (mut name, mut mime, mut len) = (String::new(), String::new(), 0u64);
            while r.pos() < next {
                let Some((id, size, _)) = read_header(r) else { break };
                let size = size.unwrap_or(0);
                let n2 = r.pos() + size;
                match id {
                    FILE_NAME => name = read_string(r, size),
                    FILE_MIME_TYPE => mime = read_string(r, size),
                    FILE_DATA => len = size,
                    _ => {}
                }
                r.seek(n2);
            }
            ctx.attachments.push((name, mime, len));
        }
        r.seek(next);
    }
}

// ---------------------------------------------------------------------------- emit

fn emit(doc: &mut Doc, ctx: &Ctx, file_size: u64) {
    let g = doc.general();
    let webm = ctx.doctype.eq_ignore_ascii_case("webm");
    g.set("Format", if webm { "WebM" } else { "Matroska" });
    if ctx.doctype_version > 0 {
        g.set("Format_Version", format!("Version {}", ctx.doctype_version));
    }
    if ctx.segment_uid.len() == 16 {
        let v = u128::from_be_bytes(ctx.segment_uid[..16].try_into().unwrap());
        g.set("UniqueID", v.to_string());
    }
    let scale_ms = ctx.timecode_scale as f64 / 1_000_000.0;
    if let Some(d) = ctx.duration {
        let ms = d * scale_ms;
        if ms > 0.0 {
            g.set("Duration", format!("{}", ms.round() as i64));
        }
    }
    if !ctx.title.is_empty() {
        g.set("Title", &ctx.title);
        g.set("Movie", &ctx.title);
    }
    if !ctx.writing_app.is_empty() {
        g.set("Encoded_Application", &ctx.writing_app);
    }
    if !ctx.muxing_app.is_empty() {
        g.set("Encoded_Library", &ctx.muxing_app);
    }
    if let Some(d) = ctx.date_utc {
        // DateUTC is nanoseconds since 2001-01-01
        let secs = 978_307_200 + d / 1_000_000_000;
        g.set("Encoded_Date", format!("UTC {}", crate::finish::format_datetime(secs)));
    }
    g.set("IsStreamable", "Yes");
    let _ = (file_size, ctx.header_size);
    // Global tags
    for (ttv, uid, tags) in &ctx.tags {
        if *uid == 0 && (*ttv == 50 || *ttv == 0) {
            let g = doc.general();
            for (k, v) in tags {
                apply_general_tag(g, k, v);
            }
        }
    }
    if ctx.crc_level1 {
        doc.general().set_extra("ErrorDetectionType", "Per level 1", "", OPT_SHOWN);
    }
    if !ctx.attachments.is_empty() {
        let names: Vec<&str> = ctx.attachments.iter().map(|(n, _, _)| n.as_str()).collect();
        doc.general().set_extra("Attachments", names.join(" / "), "", OPT_SHOWN);
    }

    // The reference only knows stream sizes when it had to read the whole file, i.e. when no
    // stream parser could finish early (fast codecs) and every video track is CFR.
    let fast = |id: &str| id.starts_with("V_MPEG4/ISO/AVC") || id.starts_with("V_MPEGH") || id.starts_with("V_VP") || id.starts_with("A_AAC") || id == "A_FLAC" || id == "A_OPUS" || id == "A_VORBIS";
    let vfr_video = ctx.tracks.iter().any(|t| {
        if t.kind != 1 {
            return false;
        }
        match t.default_duration.filter(|d| *d > 0) {
            None => true,
            Some(dd) => {
                let fps = 1_000_000_000.0 / dd as f64;
                measure_frame_rate(&t.timestamps).0.map(|m| (m - fps).abs() / fps >= 0.02).unwrap_or(false)
            }
        }
    });
    let report_sizes = ctx.fully_scanned && (vfr_video || !ctx.tracks.iter().any(|t| fast(&t.codec_id)));
    let mut sizes_sum = 0u64;
    let mut residual_streams: Vec<(StreamKind, usize, u64)> = Vec::new();

    let mut order = 0usize;
    for t in &ctx.tracks {
        let kind = match t.kind {
            1 => StreamKind::Video,
            2 => StreamKind::Audio,
            17 => StreamKind::Text,
            _ => continue,
        };
        let mut s = Stream::new(kind);
        s.set_int("StreamOrder", order as i128);
        order += 1;
        s.set_int("ID", t.number as i128);
        if t.uid != 0 {
            s.set_int("UniqueID", t.uid as i128);
        }
        let codec_id = t.codec_id.clone();
        s.set("CodecID", &codec_id);
        // Track tags first: codec-level information (SEI) takes precedence over them.
        for (_, uid, tags) in &ctx.tags {
            if *uid == t.uid && t.uid != 0 {
                for (k, v) in tags {
                    apply_track_tag(&mut s, k, v);
                }
            }
        }
        apply_codec(&mut s, t, ctx, kind);
        // Container-level properties
        if kind == StreamKind::Video {
            if t.pixel_width > 0 && t.pixel_height > 0 {
                let w = t.pixel_width.saturating_sub(t.crop[2] + t.crop[3]);
                let h = t.pixel_height.saturating_sub(t.crop[0] + t.crop[1]);
                s.set("Width", w.to_string());
                s.set("Height", h.to_string());
                if t.display_width > 0 && t.display_height > 0 && t.display_unit == 0 && w > 0 && h > 0 {
                    let dar = t.display_width as f64 / t.display_height as f64;
                    let par = dar * h as f64 / w as f64;
                    s.set("PixelAspectRatio", format!("{par:.3}"));
                    s.set("DisplayAspectRatio", format!("{dar:.3}"));
                } else if t.display_width > 0 && t.display_height > 0 && t.display_unit == 3 {
                    let dar = t.display_width as f64 / t.display_height as f64;
                    s.set("DisplayAspectRatio", format!("{dar:.3}"));
                }
            }
            if t.interlaced == Some(1) {
                s.set("ScanType", "Interlaced");
                match t.field_order {
                    Some(1) | Some(9) => s.set("ScanOrder", "TFF"),
                    Some(6) | Some(14) => s.set("ScanOrder", "BFF"),
                    _ => {}
                }
            }
            let stream_mode = s.get("FrameRate_Mode").to_string();
            let (measured, regular) = measure_frame_rate(&t.timestamps);
            if let Some(dd) = t.default_duration.filter(|d| *d > 0) {
                let fps = 1_000_000_000.0 / dd as f64;
                let consistent = measured.map(|m| (m - fps).abs() / fps < 0.02).unwrap_or(true);
                if consistent {
                    s.set("FrameRate", format!("{fps:.3}"));
                    crate::finish::set_frame_rate_fraction(&mut s, fps);
                    s.set("FrameRate_Mode", "CFR");
                } else {
                    s.set("FrameRate_Mode", "VFR");
                    if fps > 1000.0 {
                        // Nonsense DefaultDuration (e.g. 1 µs): the reference reports it as the original rate.
                        s.set("FrameRate_Original", format!("{fps:.3}"));
                    }
                    if regular {
                        if let Some(m) = measured {
                            s.set("FrameRate", format!("{m:.3}"));
                        }
                    }
                }
            } else if let Some(m) = measured {
                s.set("FrameRate_Mode", "VFR");
                if regular {
                    s.set("FrameRate", format!("{m:.3}"));
                }
            }
            if !stream_mode.is_empty() && stream_mode != s.get("FrameRate_Mode") {
                s.set("FrameRate_Mode_Original", stream_mode);
            }
            if let Some(m) = t.stereo_mode.filter(|m| *m != 0) {
                s.set("MultiView_Count", "2");
                s.set("MultiView_Layout", stereo_mode_name(m));
            }
            apply_colour(&mut s, &t.colour);
        } else if kind == StreamKind::Audio {
            if t.sampling > 0.0 {
                s.set_if_empty("SamplingRate", format!("{}", t.sampling as u64));
            }
            if t.output_sampling > 0.0 && t.output_sampling != t.sampling {
                s.set("SamplingRate", format!("{}", t.output_sampling as u64));
            }
            if t.channels > 0 {
                s.set_if_empty("Channel(s)", t.channels.to_string());
            }
            if t.bit_depth > 0 && !matches!(s.get("Format"), "AAC" | "MPEG Audio" | "Vorbis") {
                s.set_if_empty("BitDepth", t.bit_depth.to_string());
            }
        }
        // Audio with a known bit rate gets BitRate × Duration once the duration is known; video
        // only gets a size when the whole file was read (residual cluster bytes).
        let mut stream_bytes = if report_sizes && t.bytes > 0 && kind != StreamKind::Text { Some(t.bytes) } else { None };
        let tag_duration = ctx.tags.iter().filter(|(_, uid, _)| *uid == t.uid && t.uid != 0).flat_map(|(_, _, tags)| tags.iter()).find(|(k, _)| k.eq_ignore_ascii_case("DURATION")).and_then(|(_, v)| parse_hms(v));
        // Timing from blocks
        if let (Some(first), Some(last)) = (t.first_ts, t.last_ts) {
            let frame_dur = t.last_duration.or_else(|| t.default_duration.map(|d| d as f64 / 1_000_000.0)).or_else(|| {
                if kind == StreamKind::Audio {
                    audio_frame_duration_ms(&s)
                } else if kind == StreamKind::Video {
                    s.get_f64("FrameRate").filter(|f| *f > 0.0).map(|f| 1000.0 / f)
                } else {
                    None
                }
            });
            let dur = last - first + frame_dur.unwrap_or(0.0);
            if let Some(tag) = tag_duration.filter(|_| frame_dur.is_none() || !ctx.fully_scanned) {
                s.set("Duration", format!("{tag:.6}"));
            } else if dur > 0.0 {
                let precise = t.last_duration.is_some() || t.default_duration.is_some();
                s.set("Duration", if precise { format!("{dur:.6}") } else { format!("{}", dur.round() as i64) });
            }
            if kind != StreamKind::Text && s.get("Format") != "VP9" {
                s.set("Delay", format!("{}", first.round() as i64));
                s.set("Delay_Source", "Container");
            }
            if kind == StreamKind::Video && t.frames > 0 {
                s.set("FrameCount", t.frames.to_string());
            }
        }
        // Flags, names, languages
        match (kind, s.get_f64("BitRate"), s.get_f64("Duration")) {
            (StreamKind::Audio, Some(br), Some(d)) if br > 0.0 && d > 0.0 => {
                let computed = (br * d / 8000.0).round() as u64;
                s.set("StreamSize", computed.to_string());
                sizes_sum += computed;
                stream_bytes = None;
            }
            _ => {}
        }
        if let Some(bytes) = stream_bytes.take() {
            // Placeholder: the remaining cluster bytes are shared out below.
            s.set("StreamSize", bytes.to_string());
            residual_streams.push((kind, doc.count(kind), bytes));
        }
        if !t.name.is_empty() {
            s.set("Title", &t.name);
        }
        let lang = if !t.language_ietf.is_empty() { t.language_ietf.clone() } else { t.language.clone() };
        if !lang.is_empty() && lang != "und" {
            s.set("Language", lang);
        }
        s.set_bool("Default", t.default.unwrap_or(true));
        s.set_bool("Forced", t.forced);
        if !t.enabled {
            s.set("Disabled", "Yes");
        }
        doc.streams[kind as usize].push(s);
    }

    if report_sizes {
        // The reference charges every cluster byte to the streams: constant-rate audio gets
        // BitRate × Duration, the rest is shared out over the other streams by their payload.
        let overhead = file_size.saturating_sub(ctx.cluster_bytes);
        let residual = ctx.cluster_bytes.saturating_sub(sizes_sum);
        let payload: u64 = residual_streams.iter().map(|(_, _, b)| *b).sum();
        for (kind, idx, bytes) in &residual_streams {
            let share = if payload > 0 { (residual as f64 * *bytes as f64 / payload as f64).round() as u64 } else { 0 };
            if let Some(s) = doc.stream_mut(*kind, *idx) {
                s.set("StreamSize", share.to_string());
            }
        }
        if ctx.cluster_bytes > 0 {
            doc.general().set_int("StreamSize", if residual_streams.is_empty() { file_size.saturating_sub(sizes_sum) } else { overhead } as i128);
        }
    }

    // Audio delay relative to the first video stream
    let video_delay = doc.streams[StreamKind::Video as usize].first().and_then(|v| v.get_f64("Delay"));
    if let Some(vd) = video_delay {
        for a in doc.streams[StreamKind::Audio as usize].iter_mut() {
            if let Some(d) = a.get_f64("Delay") {
                a.set("Video_Delay", format!("{}", (d - vd).round() as i64));
            }
        }
    }

    // Chapters → Menu
    for edition in &ctx.chapters {
        let mut m = Stream::new(StreamKind::Menu);
        let begin = m.schema_len();
        let mut n = 0;
        for (start_ns, lang, title, hidden) in edition {
            if *hidden {
                continue;
            }
            let ms = *start_ns as f64 / 1_000_000.0;
            let name = crate::finish::format::duration_strings(ms, None)[3].clone();
            let lang = if lang == "und" { "" } else { lang.as_str() };
            m.push_extra(&name, format!("{lang}:{title}"), OPT_SHOWN);
            n += 1;
        }
        if n > 0 {
            m.set_int("Chapters_Pos_Begin", begin as i128);
            m.set_int("Chapters_Pos_End", (begin + n) as i128);
            doc.streams[StreamKind::Menu as usize].push(m);
        }
    }
}

fn stereo_mode_name(m: u64) -> &'static str {
    match m {
        1 => "Side by Side (left eye first)",
        2 => "Top-Bottom (right eye first)",
        3 => "Top-Bottom (left eye first)",
        4 => "Checkboard (right eye first)",
        5 => "Checkboard (left eye first)",
        6 => "Row Interleaved (right eye first)",
        7 => "Row Interleaved (left eye first)",
        8 => "Column Interleaved (right eye first)",
        9 => "Column Interleaved (left eye first)",
        10 => "Anaglyph (cyan/red)",
        11 => "Side by Side (right eye first)",
        12 => "Anaglyph (green/magenta)",
        13 => "Both Eyes laced in one block (left eye first)",
        14 => "Both Eyes laced in one block (right eye first)",
        _ => "",
    }
}

/// Frame rate from the first timestamps: (median-based rate, whether the intervals are regular).
fn measure_frame_rate(ts: &[f64]) -> (Option<f64>, bool) {
    if ts.len() < 2 {
        return (None, false);
    }
    let mut sorted: Vec<f64> = ts.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut deltas: Vec<f64> = sorted.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 0.0).collect();
    if deltas.is_empty() {
        return (None, false);
    }
    deltas.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = deltas[deltas.len() / 2];
    if median <= 0.0 {
        return (None, false);
    }
    let regular = deltas.iter().filter(|d| ((*d - median) / median).abs() < 0.05).count() * 10 >= deltas.len() * 8;
    (Some(1000.0 / median), regular)
}

fn audio_frame_duration_ms(s: &Stream) -> Option<f64> {
    let sr = s.get_f64("SamplingRate")?;
    let spf = s.get_f64("SamplesPerFrame")?;
    if sr > 0.0 && spf > 0.0 {
        Some(spf / sr * 1000.0)
    } else {
        None
    }
}

fn apply_colour(s: &mut Stream, c: &Colour) {
    let mut present = false;
    if let Some(r) = c.range {
        match r {
            1 => s.set_extra("colour_range", "Limited", "", "Y YTY"),
            2 => s.set_extra("colour_range", "Full", "", "Y YTY"),
            _ => {}
        }
        if matches!(r, 1 | 2) {
            let src = if s.get("colour_range_Source") == "Stream" { "Container / Stream" } else { "Container" };
            s.set_extra("colour_range_Source", src, "", "N YTY");
            present = true;
        }
    }
    if let Some(p) = c.primaries.filter(|p| *p != 2) {
        let name = video::colour::primaries(p as u8);
        if !name.is_empty() {
            s.set_extra("colour_primaries", name, "", "Y YTY");
            s.set_extra("colour_primaries_Source", "Container", "", "N YTY");
            present = true;
        }
    }
    if let Some(t) = c.transfer.filter(|t| *t != 2) {
        let name = video::colour::transfer(t as u8);
        if !name.is_empty() {
            s.set_extra("transfer_characteristics", name, "", "Y YTY");
            s.set_extra("transfer_characteristics_Source", "Container", "", "N YTY");
            present = true;
        }
    }
    if let Some(m) = c.matrix.filter(|m| *m != 2) {
        let name = video::colour::matrix(m as u8);
        if !name.is_empty() {
            s.set_extra("matrix_coefficients", name, "", "Y YTY");
            s.set_extra("matrix_coefficients_Source", "Container", "", "N YTY");
            present = true;
        }
    }
    if present {
        s.set_extra("colour_description_present", "Yes", "", "N YTY");
        let src = if s.get("colour_description_present_Source") == "Stream" { "Container / Stream" } else { "Container" };
        s.set_extra("colour_description_present_Source", src, "", "N YTY");
    }
    if let Some(v) = c.max_cll {
        s.set_extra("MaxCLL", format!("{v} cd/m2"), "", "Y YTY");
    }
    if let Some(v) = c.max_fall {
        s.set_extra("MaxFALL", format!("{v} cd/m2"), "", "Y YTY");
    }
}

fn apply_general_tag(g: &mut Stream, key: &str, value: &str) {
    let k = key.to_ascii_uppercase();
    let field = match k.as_str() {
        "TITLE" => "Title",
        "ARTIST" => "Performer",
        "ALBUM" => "Album",
        "DATE_RELEASED" | "DATE_RELEASE" => "Released_Date",
        "DATE_RECORDED" => "Recorded_Date",
        "DATE_ENCODED" => "Encoded_Date",
        "DATE_TAGGED" => "Tagged_Date",
        "COMMENT" => "Comment",
        "DESCRIPTION" => "Description",
        "GENRE" => "Genre",
        "COPYRIGHT" => "Copyright",
        "ENCODER" => "Encoded_Library",
        "ENCODED_BY" => "EncodedBy",
        "PUBLISHER" => "Publisher",
        "COMPOSER" => "Composer",
        "DIRECTOR" => "Director",
        "PRODUCER" => "Producer",
        "LAW_RATING" => "LawRating",
        "SYNOPSIS" => "Synopsis",
        "SUMMARY" => "Summary",
        "SUBJECT" => "Subject",
        "KEYWORDS" => "Keywords",
        "LYRICS" => "Lyrics",
        "BPM" => "BPM",
        "ISRC" => "ISRC",
        "BARCODE" => "BarCode",
        "CATALOG_NUMBER" => "CatalogNumber",
        "LABEL" => "Label",
        "LABEL_CODE" => "LabelCode",
        "PART_NUMBER" => "Part/Position",
        "TOTAL_PARTS" => "Part/Position_Total",
        "ACTOR" => "Actor",
        "CONDUCTOR" => "Conductor",
        "LYRICIST" => "Lyricist",
        "WRITTEN_BY" => "WrittenBy",
        "SCREENPLAY_BY" => "ScreenplayBy",
        "EDITED_BY" => "EditedBy",
        "DISTRIBUTED_BY" => "DistributedBy",
        "MASTERED_BY" => "MasteredBy",
        "MIXED_BY" | "REMIXED_BY" => "RemixedBy",
        "PRODUCTION_STUDIO" => "ProductionStudio",
        "THANKS_TO" => "ThanksTo",
        "CONTENT_TYPE" => "ContentType",
        "MOOD" => "Mood",
        "PERIOD" => "Period",
        "ORIGINAL_MEDIA_TYPE" => "OriginalSourceMedium",
        "RATING" => "Rating",
        "TERMS_OF_USE" => "TermsOfUse",
        "URL" => "Title/Url",
        "SUBTITLE" => "Title_More",
        _ => {
            if key.starts_with('_') || key.contains('/') {
                return;
            }
            g.set_extra(key, value, "", OPT_SHOWN);
            return;
        }
    };
    if field == "Title" && g.has("Title") {
        return;
    }
    g.set(field, value);
    if field == "Title" {
        g.set("Movie", value);
    }
}

fn apply_track_tag(s: &mut Stream, key: &str, value: &str) {
    let k = key.to_ascii_uppercase();
    match k.as_str() {
        "ENCODER" => s.set_if_empty("Encoded_Library", value),
        "BPS" => {
            if let Ok(v) = value.trim().parse::<u64>() {
                s.set_if_empty("BitRate", v.to_string());
            }
        }
        "DURATION" => {
            if let Some(ms) = parse_hms(value) {
                if !s.has("Duration") {
                    s.set("Duration", format!("{ms:.6}"));
                }
            }
        }
        "NUMBER_OF_FRAMES" => {
            if let Ok(v) = value.trim().parse::<u64>() {
                if !s.has("FrameCount") {
                    s.set("FrameCount", v.to_string());
                }
            }
        }
        "NUMBER_OF_BYTES" => {
            if let Ok(v) = value.trim().parse::<u64>() {
                if !s.has("StreamSize") {
                    s.set("StreamSize", v.to_string());
                }
            }
        }
        "TITLE" => s.set_if_empty("Title", value),
        "LANGUAGE" => s.set_if_empty("Language", value),
        "_STATISTICS_WRITING_APP" => s.set_extra("Statistics_Writing_App", value, "", "N YTY"),
        "_STATISTICS_WRITING_DATE_UTC" => s.set_extra("Statistics_Writing_Date_UTC", value, "", "N YTY"),
        "_STATISTICS_TAGS" => {}
        _ => {
            if !key.starts_with('_') {
                s.set_extra(key, value, "", OPT_SHOWN);
            }
        }
    }
}

/// `HH:MM:SS.nnnnnnnnn` → milliseconds.
pub fn parse_hms(v: &str) -> Option<f64> {
    let mut parts = v.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(((h * 60.0 + m) * 60.0 + s) * 1000.0)
}

// ---------------------------------------------------------------------------- codec dispatch

fn apply_codec(s: &mut Stream, t: &Track, _ctx: &Ctx, kind: StreamKind) {
    let id = t.codec_id.as_str();
    let private = t.codec_private.as_slice();
    let first = t.first_frame.as_deref().unwrap_or(&[]);
    let frames: Vec<&[u8]> = t.frames_for_codec.iter().map(|v| v.as_slice()).collect();
    match kind {
        StreamKind::Video => match id {
            "V_MPEG4/ISO/AVC" => {
                s.set("Format", "AVC");
                avc::apply_avcc(s, private);
                if let Some((_, _, len)) = avc::parse_avcc(private) {
                    let nals = avc::nals_length_prefixed(first, len);
                    avc::apply_sei_from_nals(s, &nals);
                }
            }
            "V_MPEGH/ISO/HEVC" => {
                s.set("Format", "HEVC");
                hevc::apply_hvcc(s, private);
                if let Some(len) = hevc::hvcc_length_size(private) {
                    let nals = hevc::nals_length_prefixed(first, len);
                    hevc::apply_sei_from_nals(s, &nals);
                }
            }
            "V_MPEG4/ISO/ASP" | "V_MPEG4/ISO/SP" | "V_MPEG4/ISO/AP" | "V_MPEG4/ISO/AVCP" => {
                s.set("Format", "MPEG-4 Visual");
                if !mpeg4v::apply_headers(s, private) {
                    mpeg4v::apply_headers(s, first);
                }
                mpeg4v::apply_frame_user_data(s, first);
            }
            "V_MPEG4/MS/V3" => {
                s.set("Format", "MPEG-4 Visual");
                s.set("CodecID/Info", "Microsoft MPEG-4 v3 (pre-standard)");
            }
            "V_MPEG1" | "V_MPEG2" => {
                s.set("Format", "MPEG Video");
                if !mpegv::apply_headers(s, private) {
                    mpegv::apply_headers(s, first);
                }
            }
            "V_VP8" => {
                s.set("Format", "VP8");
                vp8::apply_frame(s, first);
            }
            "V_VP9" => {
                s.set("Format", "VP9");
                vp9::apply_frame(s, first);
            }
            "V_AV1" => {
                s.set("Format", "AV1");
                if !av1::apply_av1c(s, private) {
                    av1::apply_obus(s, first);
                }
                av1::apply_obus_metadata(s, first);
            }
            "V_THEORA" => {
                s.set("Format", "Theora");
                theora::apply_xiph_private(s, private);
            }
            "V_MS/VFW/FOURCC" => {
                video::fourcc::apply_bitmapinfoheader(s, private);
                let fourcc = s.get("CodecID").to_string();
                if !fourcc.is_empty() && fourcc != "V_MS/VFW/FOURCC" {
                    s.set("CodecID", format!("V_MS/VFW/FOURCC / {fourcc}"));
                }
                if let Some(fmt) = s.get("Format").to_string().strip_prefix("").map(|x| x.to_string()) {
                    if fmt == "MPEG-4 Visual" {
                        mpeg4v::apply_headers(s, first);
                    } else if fmt == "AVC" {
                        let nals = avc::nals_annexb(first);
                        if let Some((_, sps)) = nals.iter().find(|(t, _)| *t == 7) {
                            if let Some(sps) = avc::parse_sps(sps) {
                                let cabac = nals.iter().find(|(t, _)| *t == 8).and_then(|(_, p)| avc::parse_pps_cabac(p));
                                avc::apply(s, &sps, cabac, true);
                            }
                        }
                        avc::apply_sei_from_nals(s, &nals);
                    }
                }
            }
            "V_QUICKTIME" => {
                s.set("Format", "QuickTime");
            }
            "V_PRORES" => {
                s.set("Format", "ProRes");
                video::prores::apply_frame(s, first);
            }
            "V_FFV1" => s.set("Format", "FFV1"),
            "V_DIRAC" => s.set("Format", "Dirac"),
            "V_MJPEG" => s.set("Format", "JPEG"),
            "V_UNCOMPRESSED" => s.set("Format", "YUV"),
            _ if id.starts_with("V_REAL/RV") => {
                let v = &id[9..10];
                s.set("Format", format!("RealVideo {v}"));
            }
            _ => s.set("Format", id.strip_prefix("V_").unwrap_or(id)),
        },
        StreamKind::Audio => match id {
            _ if id.starts_with("A_AAC") => {
                s.set("Format", "AAC");
                if let Some(aot) = aac::apply_asc(s, private) {
                    if id == "A_AAC" {
                        s.set("CodecID", format!("A_AAC-{aot}"));
                    }
                } else {
                    // Profile from the codec ID when there is no config.
                    let (profile, sbr) = match id {
                        "A_AAC/MPEG4/LC/SBR" | "A_AAC/MPEG2/LC/SBR" => ("LC", true),
                        "A_AAC/MPEG4/MAIN" | "A_AAC/MPEG2/MAIN" => ("Main", false),
                        "A_AAC/MPEG4/SSR" | "A_AAC/MPEG2/SSR" => ("SSR", false),
                        "A_AAC/MPEG4/LTP" => ("LTP", false),
                        _ => ("LC", false),
                    };
                    s.set("Format_AdditionalFeatures", profile);
                    if sbr {
                        s.set("Format_Settings_SBR", "Yes");
                    }
                    s.set_if_empty("SamplesPerFrame", "1024");
                }
                let count = s.get_u64("Channel(s)").unwrap_or(t.channels) as u32;
                if !s.has("ChannelLayout") {
                    let (pos, layout) = audio::layout_for_count(count);
                    if !pos.is_empty() {
                        s.set("ChannelPositions", pos);
                        s.set("ChannelLayout", layout);
                    }
                }
                s.set("Compression_Mode", "Lossy");
            }
            "A_AC3" | "A_EAC3" | "A_AC3/BSID9" | "A_AC3/BSID10" => {
                s.set("Format", if id == "A_EAC3" { "E-AC-3" } else { "AC-3" });
                for f in &frames {
                    ac3::apply_frame(s, f);
                }
                s.set("Compression_Mode", "Lossy");
            }
            "A_TRUEHD" | "A_MLP" => {
                s.set("Format", if id == "A_TRUEHD" { "MLP FBA" } else { "MLP" });
                for f in &frames {
                    if mlp::apply_frame(s, f) {
                        break;
                    }
                }
                s.set("Compression_Mode", "Lossless");
            }
            "A_DTS" | "A_DTS/EXPRESS" | "A_DTS/LOSSLESS" => {
                s.set("Format", "DTS");
                for f in &frames {
                    if dts::apply_frame(s, f) {
                        break;
                    }
                }
            }
            "A_FLAC" => {
                s.set("Format", "FLAC");
                flac::apply_streaminfo_block(s, private);
                s.set("BitRate_Mode", "VBR");
                s.set("Compression_Mode", "Lossless");
            }
            "A_MPEG/L3" | "A_MPEG/L2" | "A_MPEG/L1" => {
                s.set("Format", "MPEG Audio");
                for f in &frames {
                    mpeg_audio::apply_frame(s, f);
                }
                if !s.has("Format_Profile") {
                    s.set("Format_Profile", format!("Layer {}", &id[8..]));
                }
                s.set("Compression_Mode", "Lossy");
            }
            "A_OPUS" => {
                s.set("Format", "Opus");
                opus::apply_head(s, private);
                s.set("Compression_Mode", "Lossy");
            }
            "A_VORBIS" => {
                s.set("Format", "Vorbis");
                vorbis::apply_xiph_private(s, private);
                s.set("Compression_Mode", "Lossy");
            }
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE" => {
                s.set("Format", "PCM");
                pcm::apply_matroska(s, id, t.bit_depth as u32);
                s.set("Compression_Mode", "Lossless");
            }
            "A_ALAC" => {
                s.set("Format", "ALAC");
                alac::apply_cookie(s, private);
                s.set("Compression_Mode", "Lossless");
            }
            "A_WAVPACK4" => {
                s.set("Format", "WavPack");
                s.set("Compression_Mode", "Lossless");
            }
            "A_TTA1" => {
                s.set("Format", "TTA");
                s.set("Compression_Mode", "Lossless");
            }
            "A_MS/ACM" => {
                wma::apply_waveformatex(s, private);
            }
            "A_QUICKTIME" => s.set("Format", "QuickTime"),
            _ if id.starts_with("A_REAL/") => {
                let f = &id[7..];
                s.set("Format", match f {
                    "COOK" => "Cook",
                    "SIPR" => "Sipro",
                    "ATRC" => "Atrac",
                    "RALF" => "RealAudio Lossless",
                    "14_4" => "RealAudio 1",
                    "28_8" => "RealAudio 2",
                    _ => f,
                });
            }
            _ => s.set("Format", id.strip_prefix("A_").unwrap_or(id)),
        },
        StreamKind::Text => {
            let (format, info) = match id {
                "S_TEXT/UTF8" => ("UTF-8", "UTF-8 Plain Text"),
                "S_TEXT/ASCII" => ("ASCII", "ASCII Plain Text"),
                "S_TEXT/ASS" | "S_ASS" => ("ASS", "Advanced Sub Station Alpha"),
                "S_TEXT/SSA" | "S_SSA" => ("SSA", "Sub Station Alpha"),
                "S_TEXT/USF" => ("USF", "Universal Subtitle Format"),
                "S_TEXT/WEBVTT" => ("WebVTT", "Web Video Text Tracks"),
                "S_VOBSUB" => ("VobSub", "Picture based"),
                "S_HDMV/PGS" => ("PGS", "Picture based"),
                "S_HDMV/TEXTST" => ("HDMV-TextST", ""),
                "S_DVBSUB" => ("DVB Subtitle", ""),
                "S_KATE" => ("Kate", ""),
                _ => (id.strip_prefix("S_").unwrap_or(id), ""),
            };
            s.set("Format", format);
            if !info.is_empty() {
                s.set("CodecID/Info", info);
            }
            if format == "VobSub" {
                // "size: 720x576" etc. in the .idx-style private data
                let text = String::from_utf8_lossy(private);
                for line in text.lines() {
                    if let Some(rest) = line.strip_prefix("size:") {
                        if let Some((w, h)) = rest.trim().split_once('x') {
                            s.set("Width", w.trim());
                            s.set("Height", h.trim());
                        }
                    }
                }
            }
            if matches!(format, "VobSub" | "PGS" | "DVB Subtitle") {
                s.set("Compression_Mode", "Lossless");
            }
        }
        _ => {}
    }
}


//! Ogg (RFC 3533) with the Vorbis, Opus, Theora, Speex, FLAC-in-Ogg, OGM and Kate mappings.
//!
//! The container work: page walking and packet reassembly for the header packets of each logical
//! stream (identified by serial number), the granule clock of each codec (needed to turn the last
//! granule position into a duration) and the OGM stream headers. Codec headers themselves go to
//! `audio::vorbis`, `audio::opus`, `audio::speex`, `audio::flac`, `video::theora`, `audio::wma`,
//! `video::fourcc`.

use crate::io::{be16, be32, le16, le32, le64, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::audio::{flac, opus, speex, vorbis, wma};
use crate::parsers::video::{fourcc, theora};
use crate::parsers::Probe;

/// Bytes walked from the start of the file (header packets live in the first pages).
const MAX_HEAD_SCAN: u64 = 4 << 20;
/// Stop the head walk early once every stream has its headers and this much was seen.
const MIN_HEAD_SCAN: u64 = 256 << 10;
/// Bytes examined from the end of the file for the last granule positions.
const MAX_TAIL_SCAN: u64 = 1 << 20;
/// Cap on a reassembled header packet.
const MAX_PACKET: usize = 1 << 20;
const MAX_STREAMS: usize = 64;
/// Header packets kept per stream (identification, comment, setup).
const HEADER_PACKETS: usize = 3;

pub fn probe(p: &Probe) -> u8 {
    if p.head.len() >= 27 && p.starts_with(b"OggS") && p.head[4] == 0 {
        100
    } else {
        0
    }
}

/// One page header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub pos: u64,
    pub header_type: u8,
    pub granule: i64,
    pub serial: u32,
    pub sequence: u32,
    pub segments: Vec<u8>,
}

impl Page {
    pub fn header_len(&self) -> u64 {
        27 + self.segments.len() as u64
    }
    pub fn body_len(&self) -> u64 {
        self.segments.iter().map(|&s| s as u64).sum()
    }
    pub fn next(&self) -> u64 {
        self.pos + self.header_len() + self.body_len()
    }
    pub fn is_bos(&self) -> bool {
        self.header_type & 0x02 != 0
    }
    pub fn is_eos(&self) -> bool {
        self.header_type & 0x04 != 0
    }
}

/// Parse a page header from a byte slice (the slice must start at the capture pattern).
pub fn parse_page(d: &[u8], pos: u64) -> Option<Page> {
    if d.len() < 27 || &d[0..4] != b"OggS" || d[4] != 0 {
        return None;
    }
    let nseg = d[26] as usize;
    let segments = d.get(27..27 + nseg)?.to_vec();
    Some(Page { pos, header_type: d[5], granule: le64(d, 6)? as i64, serial: le32(d, 14)?, sequence: le32(d, 18)?, segments })
}

fn read_page(r: &mut Reader, pos: u64) -> Option<Page> {
    let head = r.read_vec_at(pos, 27 + 255);
    parse_page(&head, pos)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Codec {
    Vorbis,
    Opus,
    Theora,
    Speex,
    Flac,
    OgmVideo,
    OgmAudio,
    OgmText,
    Kate,
    Skeleton,
    Dirac,
    Unknown,
}

fn identify(packet: &[u8]) -> Codec {
    if packet.starts_with(b"\x01vorbis") {
        Codec::Vorbis
    } else if packet.starts_with(b"OpusHead") {
        Codec::Opus
    } else if packet.starts_with(b"\x80theora") {
        Codec::Theora
    } else if packet.starts_with(b"Speex   ") {
        Codec::Speex
    } else if packet.starts_with(b"\x7FFLAC") {
        Codec::Flac
    } else if packet.starts_with(b"\x01video\0\0\0") {
        Codec::OgmVideo
    } else if packet.starts_with(b"\x01audio\0\0\0") {
        Codec::OgmAudio
    } else if packet.starts_with(b"\x01text\0\0\0\0") {
        Codec::OgmText
    } else if packet.starts_with(b"\x80kate\0\0\0") {
        Codec::Kate
    } else if packet.starts_with(b"fishead\0") {
        Codec::Skeleton
    } else if packet.starts_with(b"BBCD\0") {
        Codec::Dirac
    } else {
        Codec::Unknown
    }
}

#[derive(Debug)]
struct LogicalStream {
    serial: u32,
    codec: Codec,
    packets: Vec<Vec<u8>>,
    partial: Vec<u8>,
    partial_overflow: bool,
    packets_seen: u64,
    last_granule: Option<i64>,
    tail_granule: Option<i64>,
}

impl LogicalStream {
    fn headers_needed(&self) -> usize {
        match self.codec {
            Codec::Vorbis | Codec::Theora => 3,
            Codec::Opus | Codec::Speex | Codec::Flac | Codec::OgmVideo | Codec::OgmAudio | Codec::OgmText | Codec::Kate => 2,
            _ => 1,
        }
    }
    fn has_headers(&self) -> bool {
        self.packets_seen as usize >= self.headers_needed()
    }
    fn packet(&self, i: usize) -> &[u8] {
        self.packets.get(i).map(|p| p.as_slice()).unwrap_or(&[])
    }
}

/// Feed one page's segments to the stream's packet assembler.
fn feed(st: &mut LogicalStream, body: &[u8], segments: &[u8]) {
    let mut off = 0usize;
    for &seg in segments {
        let len = seg as usize;
        let chunk = body.get(off..(off + len).min(body.len())).unwrap_or(&[]);
        off += len;
        if st.packets.len() < HEADER_PACKETS {
            if st.partial.len() + chunk.len() <= MAX_PACKET {
                st.partial.extend_from_slice(chunk);
            } else {
                st.partial_overflow = true;
            }
        }
        if len < 255 {
            st.packets_seen += 1;
            if st.packets.len() < HEADER_PACKETS && !st.partial_overflow {
                st.packets.push(std::mem::take(&mut st.partial));
            } else {
                st.partial.clear();
                st.partial_overflow = false;
            }
        }
    }
}

/// Granule clock of a stream: (granule → milliseconds), from the identification header.
fn granule_to_ms(st: &LogicalStream, rate_hint: Option<f64>, granule: i64) -> Option<f64> {
    if granule < 0 {
        return None;
    }
    let p0 = st.packet(0);
    let g = granule as f64;
    match st.codec {
        Codec::Vorbis => {
            let rate = rate_hint.or_else(|| le32(p0, 12).map(|v| v as f64)).filter(|r| *r > 0.0)?;
            Some(g / rate * 1000.0)
        }
        Codec::Opus => Some(g / 48000.0 * 1000.0),
        Codec::Speex => {
            let rate = rate_hint.or_else(|| le32(p0, 36).map(|v| v as f64)).filter(|r| *r > 0.0)?;
            Some(g / rate * 1000.0)
        }
        Codec::Flac => {
            let rate = rate_hint
                .or_else(|| {
                    let si = p0.get(17..)?;
                    let v = ((*si.get(10)? as u32) << 12) | ((*si.get(11)? as u32) << 4) | ((*si.get(12)? as u32) >> 4);
                    Some(v as f64)
                })
                .filter(|r| *r > 0.0)?;
            Some(g / rate * 1000.0)
        }
        Codec::Theora => {
            let (fps, shift) = theora_clock(p0)?;
            let frames = (granule >> shift) + (granule & ((1i64 << shift) - 1));
            Some(frames as f64 / fps * 1000.0)
        }
        Codec::OgmVideo | Codec::OgmAudio | Codec::OgmText => {
            let time_unit = le64(p0, 17)? as i64;
            let samples_per_unit = le64(p0, 25)? as i64;
            if time_unit <= 0 || samples_per_unit <= 0 {
                return None;
            }
            Some(g * time_unit as f64 / samples_per_unit as f64 / 10_000.0)
        }
        _ => None,
    }
}

/// Theora identification header: (frame rate, keyframe granule shift).
fn theora_clock(p: &[u8]) -> Option<(f64, u32)> {
    let num = be32(p, 22)?;
    let den = be32(p, 26)?;
    if num == 0 || den == 0 {
        return None;
    }
    let shift = ((be16(p, 40)? >> 5) & 0x1F) as u32;
    Some((num as f64 / den as f64, shift))
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let mut streams: Vec<LogicalStream> = Vec::new();
    let mut pos = 0u64;
    let mut pages = 0u64;
    let limit = r.len().min(MAX_HEAD_SCAN);
    while pos + 27 <= r.len() && pos < limit {
        let Some(page) = read_page(r, pos) else {
            // Lost sync: look for the next capture pattern within a bounded window.
            let window = r.read_vec_at(pos + 1, 64 << 10);
            match window.windows(4).position(|w| w == b"OggS") {
                Some(i) => {
                    pos += 1 + i as u64;
                    continue;
                }
                None => break,
            }
        };
        pages += 1;
        let body_len = page.body_len().min(r.len().saturating_sub(pos + page.header_len()));
        let idx = match streams.iter().position(|s| s.serial == page.serial) {
            Some(i) => i,
            None => {
                if streams.len() >= MAX_STREAMS {
                    pos = page.next();
                    continue;
                }
                streams.push(LogicalStream { serial: page.serial, codec: Codec::Unknown, packets: Vec::new(), partial: Vec::new(), partial_overflow: false, packets_seen: 0, last_granule: None, tail_granule: None });
                streams.len() - 1
            }
        };
        let st = &mut streams[idx];
        let need_body = st.packets.len() < HEADER_PACKETS;
        let body = if need_body { r.read_vec_at(pos + page.header_len(), body_len as usize) } else { Vec::new() };
        feed(st, &body, &page.segments);
        if st.codec == Codec::Unknown && !st.packets.is_empty() {
            st.codec = identify(&st.packets[0]);
        }
        if page.granule >= 0 && page.granule > st.last_granule.unwrap_or(-1) {
            st.last_granule = Some(page.granule);
        }
        pos = page.next();
        if pos >= MIN_HEAD_SCAN && streams.iter().all(|s| s.has_headers()) {
            break;
        }
    }
    if pages == 0 || streams.is_empty() {
        return false;
    }
    scan_tail(r, &mut streams);
    emit(doc, &streams);
    true
}

/// Walk the pages of the last part of the file and remember the last granule of every stream.
fn scan_tail(r: &mut Reader, streams: &mut [LogicalStream]) {
    let len = r.len();
    let tail_len = len.min(MAX_TAIL_SCAN);
    let start = len - tail_len;
    let data = r.read_vec_at(start, tail_len as usize);
    let mut i = 0usize;
    let mut steps = 0u32;
    while i + 27 <= data.len() && steps < 1_000_000 {
        steps += 1;
        let Some(page) = parse_page(&data[i..], start + i as u64) else {
            match data[i + 1..].windows(4).position(|w| w == b"OggS") {
                Some(p) => i += 1 + p,
                None => break,
            }
            continue;
        };
        let next = i + page.header_len() as usize + page.body_len() as usize;
        if page.granule >= 0 {
            if let Some(st) = streams.iter_mut().find(|s| s.serial == page.serial) {
                if page.granule > st.tail_granule.unwrap_or(-1) {
                    st.tail_granule = Some(page.granule);
                }
            }
        }
        i = next.max(i + 1);
    }
}

fn emit(doc: &mut Doc, streams: &[LogicalStream]) {
    let mut general = std::mem::replace(doc.general(), Stream::new(StreamKind::General));
    general.set("Format", "Ogg");
    let mut durations: Vec<f64> = Vec::new();
    let mut theora_streams: Vec<(usize, f64)> = Vec::new(); // (video index, fps)
    let mut modes: Vec<Option<String>> = Vec::new();
    for st in streams {
        let kind = match st.codec {
            Codec::Vorbis | Codec::Opus | Codec::Speex | Codec::Flac | Codec::OgmAudio => StreamKind::Audio,
            Codec::Theora | Codec::OgmVideo | Codec::Dirac => StreamKind::Video,
            Codec::OgmText | Codec::Kate => StreamKind::Text,
            Codec::Skeleton | Codec::Unknown => continue,
        };
        let mut s = Stream::new(kind);
        s.set("ID", st.serial.to_string());
        s.set("ID/String", format!("{} (0x{:X})", st.serial, st.serial));
        apply_codec(&mut s, &mut general, st);
        let granule = st.tail_granule.or(st.last_granule);
        let rate_hint = s.get_f64("SamplingRate");
        let ms = granule.and_then(|g| granule_to_ms(st, rate_hint, g)).filter(|ms| *ms > 0.0);
        if let Some(ms) = ms {
            durations.push(ms);
            if st.codec == Codec::Theora {
                // The reference derives the video timing from the container duration.
                if let Some((fps, _)) = theora_clock(st.packet(0)) {
                    theora_streams.push((doc.count(StreamKind::Video), fps));
                }
            } else {
                s.set("Duration", format!("{}", ms.round() as i64));
                // The reference sizes Ogg streams from the declared bit rate, not from the pages.
                if let Some(br) = s.get_f64("BitRate").filter(|b| *b > 0.0) {
                    s.set("StreamSize", format!("{}", (br * ms / 8000.0).floor() as u64));
                }
            }
        }
        if s.has("BitRate") || kind == StreamKind::Audio {
            let m = s.get("BitRate_Mode").to_string();
            modes.push(if m.is_empty() { None } else { Some(m) });
        }
        doc.streams[kind as usize].push(s);
    }
    let total = durations.iter().cloned().fold(None, |m: Option<f64>, d| Some(m.map_or(d, |m| m.max(d))));
    if let Some(total) = total {
        general.set("Duration", format!("{}", total.round() as i64));
        for (index, fps) in theora_streams {
            if let Some(v) = doc.stream_mut(StreamKind::Video, index) {
                v.set("Duration", format!("{}", total.round() as i64));
                v.set_extra("Duration_Source", "General_Duration", "", "N NTN");
                if fps > 0.0 {
                    v.set("FrameCount", format!("{}", (total / 1000.0 * fps).round() as i64));
                    v.set_extra("FrameCount_Source", "General_Duration", "", "N NTN");
                }
            }
        }
    }
    if !modes.is_empty() && modes.iter().all(|m| m.is_some() && *m == modes[0]) {
        if let Some(Some(m)) = modes.first() {
            general.set("OverallBitRate_Mode", m.clone());
        }
    }
    let has_video = doc.count(StreamKind::Video) > 0;
    general.set("InternetMediaType", if has_video { "video/ogg" } else { "audio/ogg" });
    if streams.iter().any(|s| s.codec == Codec::Flac) {
        general.set_if_empty("Format/Info", "Free Lossless Audio Codec");
    }
    *doc.general() = general;
}

fn apply_codec(s: &mut Stream, general: &mut Stream, st: &LogicalStream) {
    let p0 = st.packet(0);
    let p1 = st.packet(1);
    match st.codec {
        Codec::Vorbis => {
            s.set("Format", "Vorbis");
            vorbis::apply_ident(s, p0);
            if p1.starts_with(b"\x03vorbis") {
                vorbis::apply_comments(s, general, &p1[7..]);
            }
            s.set_if_empty("Compression_Mode", "Lossy");
        }
        Codec::Opus => {
            s.set("Format", "Opus");
            opus::apply_head(s, p0);
            if p1.starts_with(b"OpusTags") {
                vorbis::apply_comments(s, general, &p1[8..]);
            }
            s.set_if_empty("Compression_Mode", "Lossy");
        }
        Codec::Theora => {
            s.set("Format", "Theora");
            theora::apply_ident(s, p0);
            if p1.starts_with(b"\x81theora") {
                vorbis::apply_comments(s, general, &p1[7..]);
            }
            if let Some((fps, _)) = theora_clock(p0) {
                s.set_if_empty("FrameRate", format!("{fps:.3}"));
            }
            s.set_if_empty("Compression_Mode", "Lossy");
        }
        Codec::Speex => {
            s.set("Format", "Speex");
            speex::apply_header(s, p0);
            if !p1.is_empty() {
                // The Speex version string is the writing library; the comment vendor is not.
                let lib = s.get("Encoded_Library").to_string();
                vorbis::apply_comments(s, general, p1);
                if !lib.is_empty() {
                    s.set("Encoded_Library", lib);
                    s.clear("Encoded_Library/String");
                }
            }
            s.set_if_empty("Compression_Mode", "Lossy");
        }
        Codec::Flac => {
            s.set("Format", "FLAC");
            if let Some(blocks) = p0.get(9..) {
                flac::apply_streaminfo_block(s, blocks);
            }
            if p1.first().map(|b| b & 0x7F) == Some(4) {
                vorbis::apply_comments(s, general, p1.get(4..).unwrap_or(&[]));
            }
            s.set_if_empty("BitRate_Mode", "VBR");
            s.set_if_empty("Compression_Mode", "Lossless");
        }
        Codec::OgmVideo => {
            let sub: String = p0.get(9..13).map(|c| c.iter().map(|&b| b as char).collect()).unwrap_or_default();
            if !sub.is_empty() && sub.chars().all(|c| c.is_ascii_graphic()) {
                s.set("CodecID", &sub);
                if let Some((f, _, _)) = fourcc::fourcc_format(&sub) {
                    s.set("Format", f);
                } else {
                    s.set("Format", &sub);
                }
            }
            if let Some(time_unit) = le64(p0, 17).filter(|t| *t > 0) {
                s.set("FrameRate", format!("{:.3}", 10_000_000.0 / time_unit as f64));
            }
            if let (Some(w), Some(h)) = (le32(p0, 45), le32(p0, 49)) {
                if w > 0 && h > 0 {
                    s.set("Width", w.to_string());
                    s.set("Height", h.to_string());
                }
            }
        }
        Codec::OgmAudio => {
            // The sub type is the wave format tag in ASCII hex; rebuild a WAVEFORMATEX for the helper.
            let sub: String = p0.get(9..13).map(|c| c.iter().map(|&b| b as char).collect()).unwrap_or_default();
            let tag = u16::from_str_radix(sub.trim_matches(char::from(0)).trim(), 16).unwrap_or(0);
            let rate = le64(p0, 25).unwrap_or(0).min(u32::MAX as u64) as u32;
            let bits = le16(p0, 41).unwrap_or(0);
            let channels = le16(p0, 45).unwrap_or(0);
            let block_align = le16(p0, 47).unwrap_or(0);
            let avg_bytes = le32(p0, 49).unwrap_or(0);
            let mut fmt = Vec::with_capacity(18);
            fmt.extend_from_slice(&tag.to_le_bytes());
            fmt.extend_from_slice(&channels.to_le_bytes());
            fmt.extend_from_slice(&rate.to_le_bytes());
            fmt.extend_from_slice(&avg_bytes.to_le_bytes());
            fmt.extend_from_slice(&block_align.to_le_bytes());
            fmt.extend_from_slice(&bits.to_le_bytes());
            fmt.extend_from_slice(&0u16.to_le_bytes());
            wma::apply_waveformatex(s, &fmt);
            s.set_if_empty("CodecID", format!("{tag:X}"));
            if channels > 0 {
                s.set_if_empty("Channel(s)", channels.to_string());
            }
            if rate > 0 {
                s.set_if_empty("SamplingRate", rate.to_string());
            }
            if avg_bytes > 0 {
                s.set_if_empty("BitRate", (avg_bytes as u64 * 8).to_string());
            }
            if bits > 0 {
                s.set_if_empty("BitDepth", bits.to_string());
            }
        }
        Codec::OgmText => s.set("Format", "Text"),
        Codec::Kate => s.set("Format", "Kate"),
        Codec::Dirac => s.set("Format", "Dirac"),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a page (CRC left zero: the parser does not verify it).
    fn page(serial: u32, seq: u32, htype: u8, granule: i64, packets: &[&[u8]]) -> Vec<u8> {
        let mut segments = Vec::new();
        let mut body = Vec::new();
        for p in packets {
            let mut rest = p.len();
            loop {
                let n = rest.min(255);
                segments.push(n as u8);
                rest -= n;
                if n < 255 {
                    break;
                }
            }
            body.extend_from_slice(p);
        }
        let mut v = b"OggS\0".to_vec();
        v.push(htype);
        v.extend_from_slice(&granule.to_le_bytes());
        v.extend_from_slice(&serial.to_le_bytes());
        v.extend_from_slice(&seq.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.push(segments.len() as u8);
        v.extend_from_slice(&segments);
        v.extend_from_slice(&body);
        v
    }

    fn vorbis_ident(rate: u32) -> Vec<u8> {
        let mut v = b"\x01vorbis".to_vec();
        v.extend_from_slice(&0u32.to_le_bytes());
        v.push(1);
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&80000u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.push(0xB8);
        v.push(1);
        v
    }

    fn theora_ident(fps_num: u32, fps_den: u32, shift: u16) -> Vec<u8> {
        let mut v = b"\x80theora\x03\x02\x01".to_vec();
        v.extend_from_slice(&[0, 4, 0, 3, 0, 0, 0x40, 0, 0, 0x30, 0, 0]);
        v.extend_from_slice(&fps_num.to_be_bytes());
        v.extend_from_slice(&fps_den.to_be_bytes());
        v.extend_from_slice(&[0, 0, 1, 0, 0, 1, 0, 0, 0, 0]);
        v.extend_from_slice(&((18u16 << 10) | (shift << 5)).to_be_bytes());
        v
    }

    #[test]
    fn page_header() {
        let p = page(7, 3, 0x04, 48000, &[b"abc", &[0x55u8; 300]]);
        let parsed = parse_page(&p, 100).unwrap();
        assert_eq!(parsed.serial, 7);
        assert_eq!(parsed.sequence, 3);
        assert_eq!(parsed.granule, 48000);
        assert!(parsed.is_eos() && !parsed.is_bos());
        assert_eq!(parsed.segments, vec![3, 255, 45]);
        assert_eq!(parsed.body_len(), 303);
        assert_eq!(parsed.next(), 100 + 27 + 3 + 303);
        assert!(parse_page(b"OggS\x01", 0).is_none());
        assert!(parse_page(&p[..20], 0).is_none());
    }

    #[test]
    fn packet_reassembly_across_pages() {
        let mut st = LogicalStream { serial: 1, codec: Codec::Unknown, packets: Vec::new(), partial: Vec::new(), partial_overflow: false, packets_seen: 0, last_granule: None, tail_granule: None };
        let big = vec![0x42u8; 300];
        feed(&mut st, &big[..255], &[255]);
        assert_eq!(st.packets_seen, 0);
        feed(&mut st, &big[255..], &[45]);
        assert_eq!(st.packets_seen, 1);
        assert_eq!(st.packets[0].len(), 300);
        feed(&mut st, b"xy", &[1, 1]);
        assert_eq!(st.packets_seen, 3);
        assert_eq!(st.packets.len(), 3);
        feed(&mut st, b"z", &[1]);
        assert_eq!(st.packets.len(), 3); // only the header packets are kept
        assert_eq!(st.packets_seen, 4);
    }

    #[test]
    fn vorbis_file() {
        let ident = vorbis_ident(48000);
        let comment = b"\x03vorbis\x04\x00\x00\x00test\x00\x00\x00\x00\x01".to_vec();
        let setup = b"\x05vorbis\x00".to_vec();
        let mut file = page(0x1234, 0, 0x02, 0, &[&ident]);
        file.extend_from_slice(&page(0x1234, 1, 0, 0, &[&comment, &setup]));
        file.extend_from_slice(&page(0x1234, 2, 0, 24000, &[&[1u8; 100], &[2u8; 100]]));
        file.extend_from_slice(&page(0x1234, 3, 0x04, 48000, &[&[3u8; 100]]));
        assert_eq!(probe(&Probe { head: &file[..64], ext: "ogg", size: file.len() as u64 }), 100);
        let mut r = Reader::from_bytes(file);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "Ogg");
        assert_eq!(g.get("Duration"), "1000");
        assert_eq!(g.get("InternetMediaType"), "audio/ogg");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("Format"), "Vorbis");
        assert_eq!(a.get("ID"), "4660");
        assert_eq!(a.get("ID/String"), "4660 (0x1234)");
        assert_eq!(a.get("Duration"), "1000");
        assert_eq!(a.get("Compression_Mode"), "Lossy");
    }

    #[test]
    fn theora_and_vorbis_durations() {
        let tid = theora_ident(25, 1, 6);
        let vid = vorbis_ident(48000);
        let mut file = page(1, 0, 0x02, 0, &[&tid]);
        file.extend_from_slice(&page(2, 0, 0x02, 0, &[&vid]));
        file.extend_from_slice(&page(1, 1, 0, 0, &[b"\x81theora\0\0\0\0\0\0\0\0", b"\x82theora"]));
        file.extend_from_slice(&page(2, 1, 0, 0, &[b"\x03vorbis\0\0\0\0\0\0\0\0", b"\x05vorbis"]));
        // 25 frames: keyframe at frame 16 (granule 16<<6 | 9 = frame 25)
        file.extend_from_slice(&page(1, 2, 0x04, (16 << 6) | 9, &[&[0u8; 10]]));
        file.extend_from_slice(&page(2, 2, 0x04, 36000, &[&[0u8; 10]]));
        let mut r = Reader::from_bytes(file);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Duration"), "1000");
        assert_eq!(g.get("InternetMediaType"), "video/ogg");
        let v = doc.stream(StreamKind::Video, 0).unwrap();
        assert_eq!(v.get("Format"), "Theora");
        assert_eq!(v.get("FrameRate"), "25.000");
        assert_eq!(v.get("Duration"), "1000");
        assert_eq!(v.get("Duration_Source"), "General_Duration");
        assert_eq!(v.get("FrameCount"), "25");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("Duration"), "750");
    }

    #[test]
    fn opus_and_flac_clocks() {
        let head = b"OpusHead\x01\x01\x38\x01\x80\xbb\x00\x00\x00\x00\x00".to_vec();
        let mut file = page(9, 0, 0x02, 0, &[&head]);
        file.extend_from_slice(&page(9, 1, 0, 0, &[b"OpusTags\0\0\0\0\0\0\0\0"]));
        file.extend_from_slice(&page(9, 2, 0x04, 48312, &[&[0u8; 20]]));
        let mut r = Reader::from_bytes(file);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert_eq!(doc.stream(StreamKind::Audio, 0).unwrap().get("Duration"), "1007");

        // FLAC: STREAMINFO with 44100 Hz (20 bits at byte 10)
        let mut si = vec![0u8; 34];
        si[10] = 0x0A;
        si[11] = 0xC4;
        si[12] = 0x40;
        let mut p0 = b"\x7FFLAC\x01\x00\x00\x01fLaC\x00\x00\x00\x22".to_vec();
        p0.extend_from_slice(&si);
        let mut file = page(5, 0, 0x02, 0, &[&p0]);
        file.extend_from_slice(&page(5, 1, 0, 0, &[b"\x84\0\0\x08\0\0\0\0\0\0\0\0"]));
        file.extend_from_slice(&page(5, 2, 0x04, 22050, &[&[0u8; 20]]));
        let mut r = Reader::from_bytes(file);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert_eq!(doc.stream(StreamKind::Audio, 0).unwrap().get("Duration"), "500");
        assert_eq!(doc.general_ref().get("Format/Info"), "Free Lossless Audio Codec");
    }

    #[test]
    fn ogm_video_header() {
        let mut p0 = b"\x01video\0\0\0XVID".to_vec();
        p0.extend_from_slice(&0u32.to_le_bytes()); // size
        p0.extend_from_slice(&400_000i64.to_le_bytes()); // time unit: 25 fps
        p0.extend_from_slice(&1i64.to_le_bytes());
        p0.extend_from_slice(&[0u8; 12]); // default_len, buffersize, bits, padding
        p0.extend_from_slice(&640u32.to_le_bytes());
        p0.extend_from_slice(&480u32.to_le_bytes());
        let mut file = page(3, 0, 0x02, 0, &[&p0]);
        file.extend_from_slice(&page(3, 1, 0, 0, &[b"\x03\0\0\0\0"]));
        file.extend_from_slice(&page(3, 2, 0x04, 50, &[&[0u8; 8]]));
        let mut r = Reader::from_bytes(file);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let v = doc.stream(StreamKind::Video, 0).unwrap();
        assert_eq!(v.get("CodecID"), "XVID");
        assert_eq!(v.get("Width"), "640");
        assert_eq!(v.get("Height"), "480");
        assert_eq!(v.get("FrameRate"), "25.000");
        assert_eq!(v.get("Duration"), "2000");
    }

    #[test]
    fn malformed() {
        for data in [b"OggS".to_vec(), b"OggS\0\x02\0\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\xff".to_vec(), vec![0u8; 100]] {
            let mut r = Reader::from_bytes(data);
            let mut doc = Doc::new();
            let _ = parse(&mut r, &mut doc);
        }
    }
}

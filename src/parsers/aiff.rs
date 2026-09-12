//! AIFF / AIFF-C (IFF `FORM` with `AIFF` or `AIFC` form type): `COMM`, `SSND`, `FVER` and the
//! text chunks (`NAME`, `AUTH`, `(c) `, `ANNO`).

use crate::io::{be16, be32, clean_text, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

const MAX_TEXT_CHUNK: usize = 1 << 20;
const MAX_CHUNKS: usize = 4096;

pub fn probe(p: &Probe) -> u8 {
    let h = p.head;
    if h.len() >= 12 && &h[0..4] == b"FORM" && matches!(&h[8..12], b"AIFF" | b"AIFC") {
        100
    } else {
        0
    }
}

/// `COMM` chunk: the container's description of the sound data.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Comm {
    pub channels: u16,
    pub frames: u32,
    pub sample_size: u16,
    pub sample_rate: f64,
    /// AIFF-C compression type (`NONE` for plain AIFF).
    pub compression: [u8; 4],
    pub compression_name: String,
}

/// 80-bit IEEE 754 extended precision → f64 (only what AIFF sample rates need).
pub fn extended_to_f64(b: &[u8]) -> Option<f64> {
    if b.len() < 10 {
        return None;
    }
    let sign = if b[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    let exponent = (((b[0] & 0x7F) as i32) << 8) | b[1] as i32;
    let mantissa = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
    if exponent == 0 && mantissa == 0 {
        return Some(0.0);
    }
    if exponent == 0x7FFF {
        return None; // infinity / NaN
    }
    let value = mantissa as f64 * 2f64.powi(exponent - 16383 - 63);
    Some(sign * value)
}

pub fn parse_comm(d: &[u8], aifc: bool) -> Option<Comm> {
    let mut c = Comm { channels: be16(d, 0)?, frames: be32(d, 2)?, sample_size: be16(d, 6)?, sample_rate: extended_to_f64(d.get(8..18)?)?, compression: *b"NONE", ..Default::default() };
    if aifc && d.len() >= 22 {
        c.compression.copy_from_slice(&d[18..22]);
        // Pascal string: length byte then text.
        if let Some(&len) = d.get(22) {
            let name = d.get(23..23 + len as usize).unwrap_or(&[]);
            c.compression_name = clean_text(&String::from_utf8_lossy(name));
        }
    }
    Some(c)
}

/// (Format, endianness, sign, float, bit depth override) for an AIFF-C compression type.
fn compression_info(c: &[u8; 4]) -> (&'static str, Option<&'static str>, Option<&'static str>, bool) {
    match c {
        b"NONE" | b"twos" | b"in24" | b"in32" => ("PCM", Some("Big"), None, false),
        b"sowt" => ("PCM", Some("Little"), None, false),
        b"raw " => ("PCM", Some("Big"), Some("Unsigned"), false),
        b"fl32" | b"FL32" | b"fl64" | b"FL64" => ("PCM", Some("Big"), None, true),
        b"ulaw" | b"ULAW" => ("PCM", None, None, false),
        b"alaw" | b"ALAW" => ("PCM", None, None, false),
        b"ima4" => ("ADPCM", None, None, false),
        b"MAC3" => ("MACE 3", None, None, false),
        b"MAC6" => ("MACE 6", None, None, false),
        b"GSM " => ("GSM", None, None, false),
        b"QDMC" | b"QDM2" => ("QDesign", None, None, false),
        b"G722" => ("G.722", None, None, false),
        b"G726" => ("G.726", None, None, false),
        _ => ("", None, None, false),
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 12);
    if head.len() < 12 || &head[0..4] != b"FORM" || !matches!(&head[8..12], b"AIFF" | b"AIFC") {
        return false;
    }
    let aifc = &head[8..12] == b"AIFC";
    let form_size = be32(&head, 4).unwrap_or(0) as u64;
    let end = if form_size == 0 { r.len() } else { (form_size + 8).min(r.len()) };
    let mut comm: Option<Comm> = None;
    let mut ssnd: Option<(u64, u64)> = None; // (payload position, audio bytes)
    let mut annotations: Vec<String> = Vec::new();
    r.seek(12);
    let mut n = 0;
    while r.pos() + 8 <= end && n < MAX_CHUNKS {
        n += 1;
        let Some(id) = r.read_fourcc() else { break };
        let Some(size) = r.read_u32be() else { break };
        let size = size as u64;
        let pos = r.pos();
        let next = pos.saturating_add(size).saturating_add(size & 1);
        match &id {
            b"COMM" => {
                let d = r.read_vec_at(pos, size.min(1024) as usize);
                comm = parse_comm(&d, aifc);
            }
            b"SSND" => {
                if ssnd.is_none() {
                    let d = r.read_vec_at(pos, 8);
                    let offset = be32(&d, 0).unwrap_or(0) as u64;
                    let available = r.len().saturating_sub(pos);
                    let payload = size.min(available);
                    let audio = payload.saturating_sub(8).saturating_sub(offset);
                    ssnd = Some((pos + 8 + offset, audio));
                }
                if pos.saturating_add(size) >= r.len() {
                    break;
                }
            }
            b"NAME" | b"AUTH" | b"(c) " | b"ANNO" => {
                let d = r.read_vec_at(pos, size.min(MAX_TEXT_CHUNK as u64) as usize);
                let text = clean_text(&String::from_utf8_lossy(&d));
                if !text.is_empty() {
                    let g = doc.general();
                    match &id {
                        b"NAME" => g.set_if_empty("Title", &text),
                        b"AUTH" => g.set_if_empty("Performer", &text),
                        b"(c) " => g.set_if_empty("Copyright", &text),
                        _ => {
                            if annotations.len() < 16 {
                                annotations.push(text);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        r.seek(next);
    }
    let Some(comm) = comm else { return false };
    let g = doc.general();
    g.set("Format", "AIFF");
    if !annotations.is_empty() {
        g.set_if_empty("Comment", annotations.join(" / "));
    }
    let mut s = Stream::new(StreamKind::Audio);
    let (format, endianness, sign, float) = compression_info(&comm.compression);
    if !format.is_empty() {
        s.set("Format", format);
    } else if comm.compression.iter().all(|c| (0x20..0x7F).contains(c)) {
        s.set("Format", String::from_utf8_lossy(&comm.compression).trim().to_string());
    }
    if &comm.compression != b"NONE" {
        s.set("CodecID", String::from_utf8_lossy(&comm.compression).to_string());
        if !comm.compression_name.is_empty() {
            s.set("CodecID/Info", &comm.compression_name);
        }
    }
    if float {
        s.set("Format_Profile", "Float");
    }
    let mut settings = Vec::new();
    if let Some(e) = endianness {
        s.set("Format_Settings_Endianness", e);
        settings.push(e);
    }
    if let Some(sg) = sign {
        s.set("Format_Settings_Sign", sg);
        settings.push(sg);
    }
    if !settings.is_empty() {
        s.set("Format_Settings", settings.join(" / "));
    }
    if comm.channels > 0 {
        s.set("Channel(s)", comm.channels.to_string());
    }
    if comm.sample_rate > 0.0 {
        if comm.sample_rate.fract() == 0.0 {
            s.set("SamplingRate", format!("{}", comm.sample_rate as u64));
        } else {
            s.set("SamplingRate", format!("{:.3}", comm.sample_rate));
        }
        if comm.frames > 0 {
            s.set("Duration", format!("{:.3}", comm.frames as f64 / comm.sample_rate * 1000.0));
        }
    }
    if comm.sample_size > 0 {
        s.set("BitDepth", comm.sample_size.to_string());
    }
    if format == "PCM" && comm.sample_rate > 0.0 && comm.channels > 0 && comm.sample_size > 0 {
        let bits = comm.sample_rate * comm.channels as f64 * comm.sample_size as f64;
        s.set("BitRate", format!("{}", bits.round() as u64));
        s.set("BitRate_Mode", "CBR");
    }
    if let Some((_, bytes)) = ssnd {
        if bytes > 0 {
            s.set("StreamSize", bytes.to_string());
        }
    }
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        v.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    fn form(kind: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = kind.to_vec();
        for c in chunks {
            payload.extend_from_slice(c);
        }
        let mut v = b"FORM".to_vec();
        v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        v.extend_from_slice(&payload);
        v
    }

    fn comm(channels: u16, frames: u32, bits: u16, rate_ext: [u8; 10], compression: Option<(&[u8; 4], &str)>) -> Vec<u8> {
        let mut v = channels.to_be_bytes().to_vec();
        v.extend_from_slice(&frames.to_be_bytes());
        v.extend_from_slice(&bits.to_be_bytes());
        v.extend_from_slice(&rate_ext);
        if let Some((c, name)) = compression {
            v.extend_from_slice(c);
            v.push(name.len() as u8);
            v.extend_from_slice(name.as_bytes());
        }
        v
    }

    const RATE_48000: [u8; 10] = [0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0];
    const RATE_44100: [u8; 10] = [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0];

    #[test]
    fn extended_precision() {
        assert_eq!(extended_to_f64(&RATE_48000), Some(48000.0));
        assert_eq!(extended_to_f64(&RATE_44100), Some(44100.0));
        assert_eq!(extended_to_f64(&[0; 10]), Some(0.0));
        assert_eq!(extended_to_f64(&[0x7F, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(extended_to_f64(&[0x40]), None);
    }

    #[test]
    fn plain_aiff() {
        let mut ssnd = vec![0u8; 8];
        ssnd.extend_from_slice(&[0u8; 96000]);
        let file = form(b"AIFF", &[chunk(b"COMM", &comm(1, 48000, 16, RATE_48000, None)), chunk(b"NAME", b"song"), chunk(b"ANNO", b"note"), chunk(b"SSND", &ssnd)]);
        assert_eq!(probe(&Probe { head: &file[..64], ext: "aiff", size: file.len() as u64 }), 100);
        let mut r = Reader::from_bytes(file);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "AIFF");
        assert_eq!(g.get("Title"), "song");
        assert_eq!(g.get("Comment"), "note");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("Format"), "PCM");
        assert_eq!(a.get("Format_Settings"), "Big");
        assert_eq!(a.get("Format_Settings_Endianness"), "Big");
        assert!(!a.has("CodecID"));
        assert_eq!(a.get("Channel(s)"), "1");
        assert_eq!(a.get("SamplingRate"), "48000");
        assert_eq!(a.get("BitDepth"), "16");
        assert_eq!(a.get("Duration"), "1000.000");
        assert_eq!(a.get("BitRate"), "768000");
        assert_eq!(a.get("BitRate_Mode"), "CBR");
        assert_eq!(a.get("StreamSize"), "96000");
    }

    #[test]
    fn aifc_little_endian_and_float() {
        let ssnd = vec![0u8; 8 + 400];
        let file = form(b"AIFC", &[chunk(b"FVER", &[0xA2, 0x80, 0x51, 0x40]), chunk(b"COMM", &comm(2, 100, 16, RATE_44100, Some((b"sowt", "")))), chunk(b"SSND", &ssnd)]);
        let mut r = Reader::from_bytes(file);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("Format"), "PCM");
        assert_eq!(a.get("CodecID"), "sowt");
        assert_eq!(a.get("Format_Settings_Endianness"), "Little");
        assert_eq!(a.get("Channel(s)"), "2");
        assert_eq!(a.get("StreamSize"), "400");
        assert_eq!(a.get("Duration"), "2.268");

        let file = form(b"AIFC", &[chunk(b"COMM", &comm(1, 10, 32, RATE_48000, Some((b"fl32", "32-bit floating point")))), chunk(b"SSND", &vec![0u8; 48])]);
        let mut r = Reader::from_bytes(file);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("Format_Profile"), "Float");
        assert_eq!(a.get("CodecID/Info"), "32-bit floating point");
        assert_eq!(a.get("BitDepth"), "32");
    }

    #[test]
    fn truncated_and_malformed() {
        for data in [b"FORM\0\0\0\x04AIFF".to_vec(), b"FORM\xff\xff\xff\xffAIFFCOMM\xff\xff\xff\xff\0\x01".to_vec(), b"FORM\0\0\0\x20AIFFSSND\0\0\0\x10".to_vec()] {
            let mut r = Reader::from_bytes(data);
            let mut doc = Doc::new();
            assert!(!parse(&mut r, &mut doc));
        }
    }
}

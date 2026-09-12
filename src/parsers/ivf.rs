//! IVF container (`DKIF`): 32-byte file header (version, header size, FourCC, width, height,
//! time base, frame count) followed by frames with 12-byte headers (size, timestamp).

use crate::io::{le16, le32};
use crate::io::Reader;
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::video::{av1, vp8};
use crate::parsers::Probe;

#[derive(Debug, Clone, Default)]
pub struct Header {
    pub version: u16,
    pub header_size: u16,
    pub fourcc: String,
    pub width: u16,
    pub height: u16,
    pub rate: u32,  // time base denominator (ticks per second)
    pub scale: u32, // time base numerator
    pub frame_count: u32,
}

pub fn parse_header(d: &[u8]) -> Option<Header> {
    if d.get(0..4)? != b"DKIF" {
        return None;
    }
    let h = Header { version: le16(d, 4)?, header_size: le16(d, 6)?, fourcc: String::from_utf8_lossy(d.get(8..12)?).into_owned(), width: le16(d, 12)?, height: le16(d, 14)?, rate: le32(d, 16)?, scale: le32(d, 20)?, frame_count: le32(d, 24)? };
    if h.header_size < 32 {
        return None;
    }
    Some(h)
}

pub fn probe(p: &Probe) -> u8 {
    match parse_header(p.head) {
        Some(h) if h.version == 0 && h.header_size == 32 => 100,
        Some(_) => 80,
        None => 0,
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 32);
    let Some(h) = parse_header(&head) else { return false };
    let size = r.len();
    // Walk the frames: count them and sum the payloads (headers are container overhead).
    let mut pos = h.header_size as u64;
    let mut frames = 0u64;
    let mut payload = 0u64;
    let mut first: Option<Vec<u8>> = None;
    while pos + 12 <= size && frames < 1 << 24 {
        let fh = r.read_vec_at(pos, 12);
        let Some(len) = le32(&fh, 0) else { break };
        pos += 12;
        if pos + len as u64 > size {
            break;
        }
        if first.is_none() {
            first = Some(r.read_vec_at(pos, (len as usize).min(1 << 20)));
        }
        frames += 1;
        payload += len as u64;
        pos += len as u64;
    }
    let overhead = size - payload;
    let fps = if h.scale > 0 && h.rate > 0 { h.rate as f64 / h.scale as f64 } else { 0.0 };
    let count = if h.frame_count > 0 { h.frame_count as u64 } else { frames };
    let duration_ms = if fps > 0.0 { count as f64 / fps * 1000.0 } else { 0.0 };

    let mut v = Stream::new(StreamKind::Video);
    let fourcc = h.fourcc.trim().to_string();
    v.set("CodecID", &fourcc);
    let first = first.unwrap_or_default();
    match fourcc.as_str() {
        "VP80" => {
            v.set("Format", "VP8");
            vp8::apply_frame(&mut v, &first);
        }
        "AV01" => {
            v.set("Format", "AV1");
            av1::apply_obus(&mut v, &first);
            av1::apply_obus_metadata(&mut v, &first);
        }
        // The reference has no VP9 elementary parser: the FourCC stands for the format.
        _ => v.set("Format", &fourcc),
    }
    if h.width > 0 && h.height > 0 {
        v.set("Width", h.width.to_string());
        v.set("Height", h.height.to_string());
    }
    if fps > 0.0 {
        v.set("FrameRate", format!("{fps:.3}"));
    }
    if count > 0 {
        v.set("FrameCount", count.to_string());
    }
    if duration_ms > 0.0 {
        v.set("Duration", format!("{}", duration_ms.round() as u64));
        v.set("BitRate", format!("{}", (payload as f64 * 8.0 * 1000.0 / duration_ms).round() as u64));
    }
    v.set("StreamSize", payload.to_string());

    let g = doc.general();
    g.set("Format", "IVF");
    if duration_ms > 0.0 {
        g.set("Duration", format!("{}", duration_ms.round() as u64));
        g.set("OverallBitRate", format!("{}", (size as f64 * 8.0 * 1000.0 / duration_ms).round() as u64));
    }
    g.set("StreamSize", overhead.to_string());
    doc.streams[StreamKind::Video as usize].push(v);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(fourcc: &[u8; 4], frames: &[&[u8]]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend(b"DKIF");
        d.extend(0u16.to_le_bytes());
        d.extend(32u16.to_le_bytes());
        d.extend(fourcc);
        d.extend(64u16.to_le_bytes());
        d.extend(48u16.to_le_bytes());
        d.extend(25u32.to_le_bytes());
        d.extend(1u32.to_le_bytes());
        d.extend((frames.len() as u32).to_le_bytes());
        d.extend(0u32.to_le_bytes());
        for (i, f) in frames.iter().enumerate() {
            d.extend((f.len() as u32).to_le_bytes());
            d.extend((i as u64).to_le_bytes());
            d.extend(*f);
        }
        d
    }

    #[test]
    fn vp9_file() {
        let frames: Vec<Vec<u8>> = (0..25).map(|i| vec![0x82, 0x49, 0x83, 0x42, i as u8, 0, 0, 0, 0, 0]).collect();
        let refs: Vec<&[u8]> = frames.iter().map(|f| f.as_slice()).collect();
        let d = file(b"VP90", &refs);
        assert_eq!(probe(&Probe { head: &d, ext: "ivf", size: d.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "ivf", size: 4 }), 0);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(d), &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "IVF");
        assert_eq!(g.get("Duration"), "1000");
        assert_eq!(g.get("StreamSize"), "332");
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("Format"), "VP90");
        assert_eq!(v.get("CodecID"), "VP90");
        assert_eq!(v.get("Width"), "64");
        assert_eq!(v.get("FrameRate"), "25.000");
        assert_eq!(v.get("FrameCount"), "25");
        assert_eq!(v.get("StreamSize"), "250");
        assert_eq!(v.get("BitRate"), "2000");
    }

    #[test]
    fn vp8_file() {
        let tag: u32 = (100 << 5) | (1 << 4);
        let mut f = tag.to_le_bytes()[..3].to_vec();
        f.extend([0x9D, 0x01, 0x2A, 64, 0, 48, 0, 0xAA]);
        let d = file(b"VP80", &[&f]);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(d), &mut doc));
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("Format"), "VP8");
        assert_eq!(v.get("Compression_Mode"), "Lossy");
        assert!(!parse(&mut Reader::from_bytes(b"DKIF\0\0".to_vec()), &mut Doc::new()));
    }
}

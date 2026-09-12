//! WebP: RIFF container with `VP8 ` (lossy key frame), `VP8L` (lossless) or `VP8X` (extended:
//! canvas size, alpha, animation) chunks. `parse_riff_webp` is called by the RIFF parser.

use crate::io::{le16, le24, le32, Reader};
use crate::model::{Doc, StreamKind};
use crate::parsers::Probe;

/// Bytes of the file inspected for chunks.
const HEADER_LIMIT: usize = 1 << 20;
const MAX_CHUNKS: usize = 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    /// `VP8L` bitstream (or a VP8X file whose first frame is lossless).
    pub lossless: bool,
    pub alpha: bool,
    pub animated: bool,
    /// Extended file format (VP8X chunk).
    pub extended: bool,
}

/// `VP8 ` payload: frame tag (key frame), start code, 14-bit width and height.
pub fn parse_vp8(d: &[u8]) -> Option<(u32, u32)> {
    let tag = le24(d, 0)?;
    if tag & 1 != 0 {
        return None; // inter frame
    }
    if d.get(3..6)? != [0x9D, 0x01, 0x2A] {
        return None;
    }
    let w = (le16(d, 6)? & 0x3FFF) as u32;
    let h = (le16(d, 8)? & 0x3FFF) as u32;
    (w > 0 && h > 0).then_some((w, h))
}

/// `VP8L` payload: signature byte, 14-bit width-1, 14-bit height-1, alpha bit, 3-bit version.
pub fn parse_vp8l(d: &[u8]) -> Option<(u32, u32, bool)> {
    if *d.first()? != 0x2F {
        return None;
    }
    let bits = le32(d, 1)?;
    let w = (bits & 0x3FFF) + 1;
    let h = ((bits >> 14) & 0x3FFF) + 1;
    let alpha = (bits >> 28) & 1 == 1;
    if (bits >> 29) & 7 != 0 {
        return None; // version must be 0
    }
    Some((w, h, alpha))
}

/// Walk the chunks of a `RIFF....WEBP` buffer.
pub fn parse_chunks(data: &[u8]) -> Option<Info> {
    if data.get(0..4)? != b"RIFF" || data.get(8..12)? != b"WEBP" {
        return None;
    }
    let mut info = Info::default();
    let mut pos = 12usize;
    let mut found = false;
    for _ in 0..MAX_CHUNKS {
        let Some(id) = data.get(pos..pos + 4) else { break };
        let Some(size) = le32(data, pos + 4) else { break };
        let size = size as usize;
        let body = data.get(pos + 8..(pos + 8 + size).min(data.len())).unwrap_or(&[]);
        match id {
            b"VP8 " => {
                if let Some((w, h)) = parse_vp8(body) {
                    if info.width == 0 {
                        info.width = w;
                        info.height = h;
                    }
                    found = true;
                }
            }
            b"VP8L" => {
                if let Some((w, h, alpha)) = parse_vp8l(body) {
                    if info.width == 0 {
                        info.width = w;
                        info.height = h;
                    }
                    info.lossless = true;
                    info.alpha |= alpha;
                    found = true;
                }
            }
            b"VP8X" => {
                let flags = *body.first()?;
                info.extended = true;
                info.animated = flags & 0x02 != 0;
                info.alpha |= flags & 0x10 != 0;
                info.width = le24(body, 4)? + 1;
                info.height = le24(body, 7)? + 1;
                found = true;
            }
            b"ALPH" => info.alpha = true,
            b"ANIM" | b"ANMF" => info.animated = true,
            _ => {}
        }
        match pos.checked_add(8).and_then(|p| p.checked_add(size)).and_then(|p| p.checked_add(size & 1)) {
            Some(next) if next < data.len() => pos = next,
            _ => break,
        }
    }
    found.then_some(info)
}

/// Fill the document from a WebP file (the reader may be at any position).
pub fn parse_riff_webp(r: &mut Reader, doc: &mut Doc) -> bool {
    let n = r.len().min(HEADER_LIMIT as u64) as usize;
    let data = r.read_vec_at(0, n);
    let Some(_info) = parse_chunks(&data) else { return false };
    // The reference describes a WebP file by its format only; dimensions and lossless/alpha/
    // animation flags are available from `parse_chunks` for callers that want them.
    doc.general().set("Format", "WebP");
    let s = doc.add(StreamKind::Image);
    s.set("Format", "WebP");
    true
}

pub fn probe(p: &Probe) -> u8 {
    if p.starts_with(b"RIFF") && p.at(8, b"WEBP") {
        100
    } else {
        0
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    parse_riff_webp(r, doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn riff(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut body = b"WEBP".to_vec();
        for (id, payload) in chunks {
            body.extend_from_slice(*id);
            body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            body.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(&body);
        v
    }

    fn vp8(w: u16, h: u16) -> Vec<u8> {
        let mut v = vec![0x30, 0x01, 0x00, 0x9D, 0x01, 0x2A];
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(&[0; 8]);
        v
    }

    fn vp8l(w: u32, h: u32, alpha: bool) -> Vec<u8> {
        let bits = (w - 1) | ((h - 1) << 14) | ((alpha as u32) << 28);
        let mut v = vec![0x2F];
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v
    }

    #[test]
    fn lossy_lossless_extended() {
        let i = parse_chunks(&riff(&[(b"VP8 ", vp8(64, 48))])).unwrap();
        assert_eq!(i, Info { width: 64, height: 48, ..Default::default() });
        let i = parse_chunks(&riff(&[(b"VP8L", vp8l(300, 200, true))])).unwrap();
        assert_eq!((i.width, i.height, i.lossless, i.alpha), (300, 200, true, true));
        let mut vp8x = vec![0x12, 0, 0, 0];
        vp8x.extend_from_slice(&[0xFF, 0x01, 0x00, 0x2F, 0x00, 0x00]); // 512 x 48
        let i = parse_chunks(&riff(&[(b"VP8X", vp8x), (b"ANIM", vec![0; 6]), (b"VP8 ", vp8(64, 48))])).unwrap();
        assert_eq!((i.width, i.height, i.extended, i.animated, i.alpha, i.lossless), (512, 48, true, true, true, false));
    }

    #[test]
    fn malformed() {
        assert!(parse_chunks(b"RIFF\0\0\0\0WAVE").is_none());
        assert!(parse_chunks(&riff(&[])).is_none());
        assert!(parse_chunks(&riff(&[(b"VP8 ", vec![1, 2, 3])])).is_none());
        let mut inter = vp8(64, 48);
        inter[0] |= 1;
        assert!(parse_chunks(&riff(&[(b"VP8 ", inter)])).is_none());
        let mut bad = riff(&[(b"VP8 ", vp8(64, 48))]);
        bad[16..20].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        assert!(parse_chunks(&bad).is_some()); // oversized chunk is clamped
    }

    #[test]
    fn probe_and_parse() {
        let data = riff(&[(b"VP8 ", vp8(64, 48))]);
        assert_eq!(probe(&Probe { head: &data, ext: "webp", size: data.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"RIFF\0\0\0\0AVI ", ext: "webp", size: 12 }), 0);
        let mut r = Reader::from_bytes(data);
        r.seek(12);
        let mut doc = Doc::new();
        assert!(parse_riff_webp(&mut r, &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "WebP");
        assert_eq!(doc.stream(StreamKind::Image, 0).unwrap().get("Format"), "WebP");
        let mut doc = Doc::new();
        assert!(!parse(&mut Reader::from_bytes(b"RIFF\0\0\0\0WEBP".to_vec()), &mut doc));
    }
}

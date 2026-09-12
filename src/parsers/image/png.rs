//! PNG (ISO/IEC 15948): signature and the IHDR chunk.

use crate::io::{be32, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

pub const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Image header (IHDR).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ihdr {
    pub width: u32,
    pub height: u32,
    /// Bits per sample (1, 2, 4, 8 or 16).
    pub bit_depth: u8,
    /// 0 greyscale, 2 truecolour, 3 indexed, 4 greyscale+alpha, 6 truecolour+alpha.
    pub colour_type: u8,
    pub interlace: u8,
}

impl Ihdr {
    /// Samples per pixel (1 for indexed colour).
    pub fn channels(&self) -> u8 {
        match self.colour_type {
            0 | 3 => 1,
            2 => 3,
            4 => 2,
            6 => 4,
            _ => 0,
        }
    }

    /// Bits per pixel as the reference reports `BitDepth` (24 for RGB, 32 for RGBA...).
    pub fn bits_per_pixel(&self) -> u32 {
        self.bit_depth as u32 * self.channels() as u32
    }

    pub fn color_space(&self) -> Option<&'static str> {
        Some(match self.colour_type {
            0 => "Y",
            2 | 3 => "RGB",
            4 => "YA",
            6 => "RGBA",
            _ => return None,
        })
    }
}

/// Parse the IHDR chunk, which must directly follow the signature.
pub fn parse_ihdr(data: &[u8]) -> Option<Ihdr> {
    if !data.starts_with(&SIGNATURE) {
        return None;
    }
    if be32(data, 8)? != 13 || data.get(12..16)? != b"IHDR" {
        return None;
    }
    let h = Ihdr { width: be32(data, 16)?, height: be32(data, 20)?, bit_depth: *data.get(24)?, colour_type: *data.get(25)?, interlace: *data.get(28)? };
    if h.width == 0 || h.height == 0 || !matches!(h.bit_depth, 1 | 2 | 4 | 8 | 16) || h.channels() == 0 {
        return None;
    }
    Some(h)
}

pub fn probe(p: &Probe) -> u8 {
    if p.starts_with(&SIGNATURE) {
        if p.at(12, b"IHDR") { 100 } else { 80 }
    } else {
        0
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 64);
    let Some(h) = parse_ihdr(&head) else { return false };
    doc.general().set("Format", "PNG");
    let mut s = Stream::new(StreamKind::Image);
    s.set("Format", "PNG");
    s.set("Format_Compression", "LZ77");
    s.set_int("Width", h.width as i128);
    s.set_int("Height", h.height as i128);
    s.set_int("BitDepth", h.bits_per_pixel() as i128);
    s.set("Compression_Mode", "Lossless");
    s.set_int("StreamSize", r.len() as i128);
    doc.streams[StreamKind::Image as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, depth: u8, ctype: u8) -> Vec<u8> {
        let mut v = SIGNATURE.to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[depth, ctype, 0, 0, 0]);
        v.extend_from_slice(&[0; 4]); // crc (not checked)
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(b"IEND");
        v.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]);
        v
    }

    #[test]
    fn ihdr_variants() {
        let h = parse_ihdr(&png(64, 48, 8, 2)).unwrap();
        assert_eq!((h.width, h.height, h.bits_per_pixel(), h.color_space()), (64, 48, 24, Some("RGB")));
        assert_eq!(parse_ihdr(&png(1, 1, 8, 6)).unwrap().bits_per_pixel(), 32);
        assert_eq!(parse_ihdr(&png(1, 1, 8, 0)).unwrap().bits_per_pixel(), 8);
        assert_eq!(parse_ihdr(&png(1, 1, 16, 4)).unwrap().bits_per_pixel(), 32);
        assert_eq!(parse_ihdr(&png(1, 1, 4, 3)).unwrap().bits_per_pixel(), 4);
        assert!(parse_ihdr(&png(0, 1, 8, 2)).is_none());
        assert!(parse_ihdr(&png(1, 1, 7, 2)).is_none());
        assert!(parse_ihdr(&png(1, 1, 8, 5)).is_none());
        assert!(parse_ihdr(&SIGNATURE).is_none());
        assert!(parse_ihdr(b"").is_none());
    }

    #[test]
    fn probe_and_parse() {
        let data = png(64, 48, 8, 6);
        assert_eq!(probe(&Probe { head: &data, ext: "png", size: data.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"\x89PNG", ext: "png", size: 4 }), 0);
        let mut r = Reader::from_bytes(data.clone());
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "PNG");
        let s = doc.stream(StreamKind::Image, 0).unwrap();
        assert_eq!(s.get("Width"), "64");
        assert_eq!(s.get("Height"), "48");
        assert_eq!(s.get("BitDepth"), "32");
        assert_eq!(s.get("Format_Compression"), "LZ77");
        assert_eq!(s.get("Compression_Mode"), "Lossless");
        assert_eq!(s.get("StreamSize"), data.len().to_string());
    }
}

//! JPEG (ITU-T T.81 / JFIF / Exif / Adobe): marker walk up to the first frame header (SOFn).
//!
//! `apply_frame` is shared with Motion JPEG (`video::mjpeg`) and container parsers.

use crate::io::{be16, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

/// Most bytes of a file inspected for headers (APPn segments can be large: ICC, Exif thumbnails).
const HEADER_LIMIT: usize = 4 << 20;
/// Upper bound on the number of marker segments walked.
const MAX_SEGMENTS: usize = 4096;

/// One image component from SOFn: identifier and horizontal/vertical sampling factors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    pub id: u8,
    pub h: u8,
    pub v: u8,
}

/// Frame header (SOFn) plus the application segments that influence its interpretation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frame {
    /// SOF marker byte (0xC0..=0xCF except C4/C8/CC).
    pub sof: u8,
    pub precision: u8,
    pub width: u16,
    pub height: u16,
    pub components: Vec<Component>,
    /// JFIF APP0 present.
    pub jfif: bool,
    /// JFIF density: (x, y, unit) with unit 0 = aspect ratio only, 1 = dpi, 2 = dpcm.
    pub density: Option<(u16, u16, u8)>,
    /// Adobe APP14 colour transform (0 = none/RGB or CMYK, 1 = YCbCr, 2 = YCCK).
    pub adobe_transform: Option<u8>,
    /// Exif APP1 present.
    pub exif: bool,
}

impl Frame {
    /// Progressive DCT (SOF2/6/10/14).
    pub fn progressive(&self) -> bool {
        matches!(self.sof, 0xC2 | 0xC6 | 0xCA | 0xCE)
    }

    /// Lossless (predictive) process (SOF3/7/11/15).
    pub fn lossless(&self) -> bool {
        matches!(self.sof, 0xC3 | 0xC7 | 0xCB | 0xCF)
    }

    /// `4:2:0` / `4:2:2` / `4:4:4`... from the sampling factors; `None` for unusual layouts.
    pub fn chroma_subsampling(&self) -> Option<&'static str> {
        if self.components.len() < 3 {
            return None;
        }
        let y = self.components[0];
        if !self.components[1..].iter().all(|c| c.h == 1 && c.v == 1) {
            return None;
        }
        Some(match (y.h, y.v) {
            (1, 1) => "4:4:4",
            (2, 1) => "4:2:2",
            (2, 2) => "4:2:0",
            (1, 2) => "4:4:0",
            (4, 1) => "4:1:1",
            (4, 2) => "4:1:0",
            _ => return None,
        })
    }

    /// `YUV` / `RGB` / `Y` / `CMYK` / `YCCK`.
    pub fn color_space(&self) -> Option<&'static str> {
        Some(match self.components.len() {
            1 => "Y",
            3 => match self.adobe_transform {
                Some(0) if !self.jfif => "RGB",
                _ => "YUV",
            },
            4 => match self.adobe_transform {
                Some(2) => "YCCK",
                _ => "CMYK",
            },
            _ => return None,
        })
    }
}

fn is_sof(marker: u8) -> bool {
    (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

/// Walk the marker segments from SOI to the first SOFn and collect the frame header.
pub fn parse_headers(data: &[u8]) -> Option<Frame> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut f = Frame::default();
    let mut pos = 2usize;
    for _ in 0..MAX_SEGMENTS {
        // Fill bytes (0xFF) may precede a marker.
        while pos < data.len() && data[pos] == 0xFF {
            pos += 1;
        }
        if pos == 0 || pos >= data.len() || data[pos - 1] != 0xFF {
            return None;
        }
        let marker = data[pos];
        pos += 1;
        match marker {
            0x00 | 0x01 | 0xD0..=0xD7 | 0xD8 => continue, // stuffed byte, TEM, RSTn, SOI: no length
            0xD9 | 0xDA => return None,                  // EOI / SOS before any SOF
            _ => {}
        }
        let len = be16(data, pos)? as usize;
        if len < 2 {
            return None;
        }
        let seg = data.get(pos + 2..pos + len)?;
        match marker {
            0xE0 if seg.starts_with(b"JFIF\0") => {
                f.jfif = true;
                if let (Some(x), Some(y), Some(u)) = (be16(seg, 8), be16(seg, 10), seg.get(7)) {
                    f.density = Some((x, y, *u));
                }
            }
            0xE1 if seg.starts_with(b"Exif\0") => f.exif = true,
            0xEE if seg.starts_with(b"Adobe") => f.adobe_transform = seg.get(11).copied(),
            m if is_sof(m) => {
                f.sof = m;
                f.precision = *seg.first()?;
                f.height = be16(seg, 1)?;
                f.width = be16(seg, 3)?;
                let n = *seg.get(5)? as usize;
                if n == 0 || n > 4 {
                    return None;
                }
                for i in 0..n {
                    let c = seg.get(6 + i * 3..9 + i * 3)?;
                    f.components.push(Component { id: c[0], h: c[1] >> 4, v: c[1] & 15 });
                }
                return Some(f);
            }
            _ => {}
        }
        pos += len;
    }
    None
}

/// Fill an Image or Video stream from a JPEG frame's headers. `true` when a SOF was found.
pub fn apply_frame(s: &mut Stream, data: &[u8]) -> bool {
    let Some(f) = parse_headers(data) else { return false };
    s.set_if_empty("Format", "JPEG");
    if f.progressive() {
        s.set_if_empty("Format_Profile", "Progressive");
    }
    if f.width > 0 && f.height > 0 {
        s.set_if_empty("Width", f.width.to_string());
        s.set_if_empty("Height", f.height.to_string());
    }
    if let Some(cs) = f.color_space() {
        s.set_if_empty("ColorSpace", cs);
    }
    if let Some(sub) = f.chroma_subsampling() {
        s.set_if_empty("ChromaSubsampling", sub);
    }
    s.set_if_empty("BitDepth", f.precision.to_string());
    // The reference reports every JPEG process, the lossless SOF3 included, as lossy.
    s.set_if_empty("Compression_Mode", "Lossy");
    true
}

pub fn probe(p: &Probe) -> u8 {
    if p.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return 100;
    }
    if p.ext_in(&["jpg", "jpeg", "jpe", "mjpeg", "mjpg"]) && p.starts_with(&[0xFF, 0xD8]) {
        return 60;
    }
    0
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let n = r.len().min(HEADER_LIMIT as u64) as usize;
    let head = r.read_vec_at(0, n);
    let mut s = Stream::new(StreamKind::Image);
    if !apply_frame(&mut s, &head) {
        return false;
    }
    s.set_int("StreamSize", r.len() as i128);
    // The shared format table carries the M-JPEG video MIME type for "JPEG"; a still image is image/jpeg.
    s.set("InternetMediaType", "image/jpeg");
    doc.general().set("Format", "JPEG");
    doc.streams[StreamKind::Image as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sof(marker: u8, precision: u8, w: u16, h: u16, comps: &[(u8, u8)]) -> Vec<u8> {
        let mut v = vec![0xFF, marker];
        let len = 8 + 3 * comps.len() as u16;
        v.extend_from_slice(&len.to_be_bytes());
        v.push(precision);
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&w.to_be_bytes());
        v.push(comps.len() as u8);
        for (i, (hs, vs)) in comps.iter().enumerate() {
            v.extend_from_slice(&[i as u8 + 1, (hs << 4) | vs, 0]);
        }
        v
    }

    fn jfif() -> Vec<u8> {
        let mut v = vec![0xFF, 0xE0, 0, 16];
        v.extend_from_slice(b"JFIF\0\x01\x02\x01\x00\x48\x00\x48\x00\x00");
        v
    }

    fn file(segments: &[Vec<u8>]) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        for s in segments {
            v.extend_from_slice(s);
        }
        v.extend_from_slice(&[0xFF, 0xDA, 0, 2, 0xFF, 0xD9]);
        v
    }

    #[test]
    fn baseline_420() {
        let data = file(&[jfif(), vec![0xFF, 0xDB, 0, 4, 0, 0], sof(0xC0, 8, 64, 48, &[(2, 2), (1, 1), (1, 1)])]);
        let f = parse_headers(&data).unwrap();
        assert_eq!((f.width, f.height, f.precision), (64, 48, 8));
        assert!(f.jfif);
        assert_eq!(f.density, Some((72, 72, 1)));
        assert_eq!(f.chroma_subsampling(), Some("4:2:0"));
        assert_eq!(f.color_space(), Some("YUV"));
        assert!(!f.progressive());
        let mut s = Stream::new(StreamKind::Image);
        assert!(apply_frame(&mut s, &data));
        assert_eq!(s.get("Width"), "64");
        assert_eq!(s.get("ChromaSubsampling"), "4:2:0");
        assert_eq!(s.get("BitDepth"), "8");
        assert_eq!(s.get("Compression_Mode"), "Lossy");
        assert!(!s.has("Format_Profile"));
    }

    #[test]
    fn subsampling_variants() {
        let f = |c: &[(u8, u8)]| parse_headers(&file(&[sof(0xC0, 8, 8, 8, c)])).unwrap().chroma_subsampling();
        assert_eq!(f(&[(1, 1), (1, 1), (1, 1)]), Some("4:4:4"));
        assert_eq!(f(&[(2, 1), (1, 1), (1, 1)]), Some("4:2:2"));
        assert_eq!(f(&[(1, 2), (1, 2), (1, 2)]), None);
        assert_eq!(f(&[(1, 1)]), None);
    }

    #[test]
    fn progressive_gray_and_adobe_rgb() {
        let data = file(&[sof(0xC2, 12, 10, 20, &[(1, 1)])]);
        let f = parse_headers(&data).unwrap();
        assert!(f.progressive());
        assert_eq!(f.color_space(), Some("Y"));
        assert_eq!(f.precision, 12);
        let mut adobe = vec![0xFF, 0xEE, 0, 14];
        adobe.extend_from_slice(b"Adobe\0\x64\0\0\0\0\0");
        let data = file(&[adobe, sof(0xC1, 8, 10, 20, &[(1, 1), (1, 1), (1, 1)])]);
        let f = parse_headers(&data).unwrap();
        assert_eq!(f.adobe_transform, Some(0));
        assert_eq!(f.color_space(), Some("RGB"));
        let data = file(&[sof(0xC3, 9, 10, 20, &[(1, 1), (1, 1), (1, 1)])]);
        assert!(parse_headers(&data).unwrap().lossless());
    }

    #[test]
    fn malformed_inputs() {
        assert!(parse_headers(&[]).is_none());
        assert!(parse_headers(&[0xFF, 0xD8]).is_none());
        assert!(parse_headers(&[0xFF, 0xD8, 0xFF, 0xC0, 0xFF, 0xFF]).is_none());
        assert!(parse_headers(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 1]).is_none());
        assert!(parse_headers(&[0xFF, 0xD8, 0xFF, 0xD9]).is_none());
        assert!(parse_headers(&[0xFF, 0xD8, 0x12, 0x34]).is_none());
        let mut s = Stream::new(StreamKind::Image);
        assert!(!apply_frame(&mut s, b"not a jpeg"));
    }

    #[test]
    fn probe_and_parse() {
        let data = file(&[jfif(), sof(0xC0, 8, 64, 48, &[(2, 2), (1, 1), (1, 1)])]);
        assert_eq!(probe(&Probe { head: &data, ext: "jpg", size: data.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "jpg", size: 4 }), 0);
        let mut r = Reader::from_bytes(data.clone());
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "JPEG");
        let s = doc.stream(StreamKind::Image, 0).unwrap();
        assert_eq!(s.get("Format"), "JPEG");
        assert_eq!(s.get("StreamSize"), data.len().to_string());
        assert_eq!(s.get("Height"), "48");
    }
}

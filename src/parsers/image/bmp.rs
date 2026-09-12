//! Windows Bitmap: `BM` file header followed by BITMAPCOREHEADER / BITMAPINFOHEADER (V2..V5).

use crate::io::{le16, le32, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

/// DIB header fields shared by every header version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Size of the DIB header (12, 40, 52, 56, 108, 124).
    pub header_size: u32,
    pub width: u32,
    pub height: u32,
    /// Negative height in the file: rows stored top-down.
    pub top_down: bool,
    pub planes: u16,
    pub bit_count: u16,
    /// BI_RGB 0, BI_RLE8 1, BI_RLE4 2, BI_BITFIELDS 3, BI_JPEG 4, BI_PNG 5, BI_ALPHABITFIELDS 6.
    pub compression: u32,
    /// Offset of the pixel array from the start of the file.
    pub data_offset: u32,
}

impl Header {
    /// Name of the pixel encoding as the reference reports the image `Format`.
    pub fn compression_name(&self) -> Option<&'static str> {
        Some(match self.compression {
            0 => "RGB",
            1 => "RLE8",
            2 => "RLE4",
            3 | 6 => "Bitfields",
            4 => "JPEG",
            5 => "PNG",
            11 => "CMYK",
            12 => "CMYK RLE8",
            13 => "CMYK RLE4",
            _ => return None,
        })
    }
}

pub fn parse_header(data: &[u8]) -> Option<Header> {
    if data.len() < 26 || &data[0..2] != b"BM" {
        return None;
    }
    let data_offset = le32(data, 10)?;
    let header_size = le32(data, 14)?;
    let h = if header_size == 12 {
        Header { header_size, width: le16(data, 18)? as u32, height: le16(data, 20)? as u32, top_down: false, planes: le16(data, 22)?, bit_count: le16(data, 24)?, compression: 0, data_offset }
    } else if header_size >= 40 {
        let height = le32(data, 22)? as i32;
        Header {
            header_size,
            width: le32(data, 18)?,
            height: height.unsigned_abs(),
            top_down: height < 0,
            planes: le16(data, 26)?,
            bit_count: le16(data, 28)?,
            compression: le32(data, 30)?,
            data_offset,
        }
    } else {
        return None;
    };
    if h.width == 0 || h.height == 0 || h.width > 1 << 20 || h.height > 1 << 20 || h.planes != 1 {
        return None;
    }
    if !matches!(h.bit_count, 0 | 1 | 2 | 4 | 8 | 16 | 24 | 32 | 48 | 64) {
        return None;
    }
    Some(h)
}

pub fn probe(p: &Probe) -> u8 {
    if !p.starts_with(b"BM") || p.head.len() < 26 {
        return 0;
    }
    match parse_header(p.head) {
        Some(h) if h.data_offset as u64 <= p.size => 100,
        Some(_) => 60,
        None => 0,
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 128);
    let Some(h) = parse_header(&head) else { return false };
    doc.general().set("Format", "Bitmap");
    let mut s = Stream::new(StreamKind::Image);
    if let Some(name) = h.compression_name() {
        s.set("Format", name);
    }
    s.set_int("Width", h.width as i128);
    s.set_int("Height", h.height as i128);
    s.set("ColorSpace", if h.compression >= 11 { "CMYK" } else { "RGB" });
    if h.bit_count > 0 {
        s.set_int("BitDepth", h.bit_count as i128);
    }
    doc.streams[StreamKind::Image as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bmp(width: i32, height: i32, bits: u16, compression: u32) -> Vec<u8> {
        let mut v = b"BM".to_vec();
        v.extend_from_slice(&1000u32.to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&54u32.to_le_bytes());
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&width.to_le_bytes());
        v.extend_from_slice(&height.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(&compression.to_le_bytes());
        v.extend_from_slice(&[0; 20]);
        v
    }

    #[test]
    fn info_header() {
        let h = parse_header(&bmp(64, 48, 24, 0)).unwrap();
        assert_eq!((h.width, h.height, h.bit_count, h.top_down), (64, 48, 24, false));
        assert_eq!(h.compression_name(), Some("RGB"));
        let h = parse_header(&bmp(10, -20, 32, 3)).unwrap();
        assert_eq!((h.height, h.top_down, h.compression_name()), (20, true, Some("Bitfields")));
        assert!(parse_header(&bmp(0, 1, 24, 0)).is_none());
        assert!(parse_header(&bmp(1, 1, 7, 0)).is_none());
        assert!(parse_header(b"BM").is_none());
    }

    #[test]
    fn core_header() {
        let mut v = b"BM".to_vec();
        v.extend_from_slice(&[0; 8]);
        v.extend_from_slice(&26u32.to_le_bytes());
        v.extend_from_slice(&12u32.to_le_bytes());
        v.extend_from_slice(&[5, 0, 7, 0, 1, 0, 8, 0]);
        let h = parse_header(&v).unwrap();
        assert_eq!((h.width, h.height, h.bit_count, h.header_size), (5, 7, 8, 12));
    }

    #[test]
    fn probe_and_parse() {
        let data = bmp(64, 48, 24, 0);
        assert_eq!(probe(&Probe { head: &data, ext: "bmp", size: 1000 }), 100);
        assert_eq!(probe(&Probe { head: &data, ext: "bmp", size: 20 }), 60);
        assert_eq!(probe(&Probe { head: b"BMX", ext: "bmp", size: 3 }), 0);
        let mut r = Reader::from_bytes(data);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "Bitmap");
        let s = doc.stream(StreamKind::Image, 0).unwrap();
        assert_eq!(s.get("Format"), "RGB");
        assert_eq!(s.get("Width"), "64");
        assert_eq!(s.get("Height"), "48");
        assert_eq!(s.get("ColorSpace"), "RGB");
        assert_eq!(s.get("BitDepth"), "24");
    }
}

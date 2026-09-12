//! GIF (GIF87a / GIF89a): header, logical screen descriptor and a block walk for frame counting.

use crate::finish::format::aspect_ratio_string;
use crate::io::{le16, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

/// Upper bound on blocks / sub-blocks walked by `scan`.
const MAX_BLOCKS: usize = 1 << 16;

/// Header + logical screen descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// `87a` or `89a`.
    pub version: [u8; 3],
    pub width: u16,
    pub height: u16,
    pub global_table: bool,
    /// Bits per primary colour (1..=8).
    pub colour_resolution: u8,
    /// Entries in the global colour table (0 when absent).
    pub global_table_size: u16,
    /// Raw pixel aspect ratio byte (0 = no information).
    pub aspect: u8,
}

impl Header {
    pub fn version_str(&self) -> String {
        String::from_utf8_lossy(&self.version).into_owned()
    }

    /// Pixel aspect ratio `(aspect + 15) / 64` when given.
    pub fn pixel_aspect_ratio(&self) -> Option<f64> {
        (self.aspect != 0).then(|| (self.aspect as f64 + 15.0) / 64.0)
    }
}

/// What the block walk finds after the header.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Scan {
    pub frames: u32,
    /// Sum of graphic control delays in centiseconds.
    pub total_delay_cs: u32,
    /// NETSCAPE2.0 loop count (0 = forever).
    pub loop_count: Option<u16>,
    /// Reached the trailer within the scanned bytes.
    pub complete: bool,
}

pub fn parse_header(data: &[u8]) -> Option<Header> {
    if data.len() < 13 || &data[0..3] != b"GIF" || !(&data[3..6] == b"87a" || &data[3..6] == b"89a") {
        return None;
    }
    let flags = data[10];
    let global_table = flags & 0x80 != 0;
    let h = Header {
        version: [data[3], data[4], data[5]],
        width: le16(data, 6)?,
        height: le16(data, 8)?,
        global_table,
        colour_resolution: ((flags >> 4) & 7) + 1,
        global_table_size: if global_table { 2u16 << (flags & 7) } else { 0 },
        aspect: data[12],
    };
    if h.width == 0 || h.height == 0 {
        return None;
    }
    Some(h)
}

/// Skip a sequence of data sub-blocks; returns the position after the terminator.
fn skip_sub_blocks(data: &[u8], mut pos: usize) -> Option<usize> {
    for _ in 0..MAX_BLOCKS {
        let n = *data.get(pos)? as usize;
        pos += 1;
        if n == 0 {
            return Some(pos);
        }
        pos += n;
    }
    None
}

/// Walk the blocks after the header (image descriptors, extensions) up to the trailer.
pub fn scan(data: &[u8]) -> Option<Scan> {
    let h = parse_header(data)?;
    let mut pos = 13 + 3 * h.global_table_size as usize;
    let mut s = Scan::default();
    for _ in 0..MAX_BLOCKS {
        match *data.get(pos)? {
            0x3B => {
                s.complete = true;
                return Some(s);
            }
            0x2C => {
                let flags = *data.get(pos + 9)?;
                pos += 10;
                if flags & 0x80 != 0 {
                    pos += 3 * (2usize << (flags & 7));
                }
                pos += 1; // LZW minimum code size
                pos = skip_sub_blocks(data, pos)?;
                s.frames += 1;
            }
            0x21 => {
                let label = *data.get(pos + 1)?;
                pos += 2;
                match label {
                    0xF9 => {
                        if data.get(pos) == Some(&4) {
                            s.total_delay_cs = s.total_delay_cs.saturating_add(le16(data, pos + 2)? as u32);
                        }
                    }
                    0xFF => {
                        if data.get(pos) == Some(&11) && data.get(pos + 1..pos + 12)? == b"NETSCAPE2.0" && data.get(pos + 12) == Some(&3) && data.get(pos + 13) == Some(&1) {
                            s.loop_count = le16(data, pos + 14);
                        }
                    }
                    _ => {}
                }
                pos = skip_sub_blocks(data, pos)?;
            }
            _ => return Some(s),
        }
    }
    Some(s)
}

pub fn probe(p: &Probe) -> u8 {
    if p.starts_with(b"GIF87a") || p.starts_with(b"GIF89a") {
        100
    } else {
        0
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 16);
    let Some(h) = parse_header(&head) else { return false };
    doc.general().set("Format", "GIF");
    let mut s = Stream::new(StreamKind::Image);
    s.set("Format", "GIF");
    s.set("Format_Profile", h.version_str());
    s.set_int("Width", h.width as i128);
    s.set_int("Height", h.height as i128);
    let par = h.pixel_aspect_ratio().unwrap_or(1.0);
    s.set_float("PixelAspectRatio", par, 3);
    let dar = h.width as f64 * par / h.height as f64;
    s.set_float("DisplayAspectRatio", dar, 3);
    s.set("DisplayAspectRatio/String", aspect_ratio_string(dar));
    s.set("Compression_Mode", "Lossless");
    // The reference reports a GIF, animated or not, as a single image without frame information;
    // `scan` is available for callers that want the frame count.
    doc.streams[StreamKind::Image as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gif(version: &[u8], frames: usize, netscape: bool) -> Vec<u8> {
        let mut v = b"GIF".to_vec();
        v.extend_from_slice(version);
        v.extend_from_slice(&64u16.to_le_bytes());
        v.extend_from_slice(&48u16.to_le_bytes());
        v.extend_from_slice(&[0xF7, 31, 49]); // global table 256 entries, 8-bit, aspect 49
        v.extend_from_slice(&[0u8; 3 * 256]);
        if netscape {
            v.extend_from_slice(&[0x21, 0xFF, 11]);
            v.extend_from_slice(b"NETSCAPE2.0");
            v.extend_from_slice(&[3, 1, 0, 0, 0]);
        }
        for _ in 0..frames {
            v.extend_from_slice(&[0x21, 0xF9, 4, 4, 10, 0, 0, 0]); // delay 10 cs
            v.extend_from_slice(&[0x2C, 0, 0, 0, 0, 64, 0, 48, 0, 0, 8, 2, 0x44, 0x01, 0]);
        }
        v.push(0x3B);
        v
    }

    #[test]
    fn header_and_scan() {
        let data = gif(b"89a", 3, true);
        let h = parse_header(&data).unwrap();
        assert_eq!((h.width, h.height, h.global_table_size, h.colour_resolution), (64, 48, 256, 8));
        assert_eq!(h.version_str(), "89a");
        assert_eq!(h.pixel_aspect_ratio(), Some(1.0));
        let s = scan(&data).unwrap();
        assert_eq!(s, Scan { frames: 3, total_delay_cs: 30, loop_count: Some(0), complete: true });
        let s = scan(&gif(b"87a", 1, false)).unwrap();
        assert_eq!((s.frames, s.loop_count, s.complete), (1, None, true));
        // Truncated file: the walk stops without panicking.
        let cut = &data[..data.len() - 10];
        assert!(scan(cut).is_none());
        assert!(parse_header(b"GIF88a").is_none());
        assert!(parse_header(&data[..12]).is_none());
    }

    #[test]
    fn probe_and_parse() {
        let data = gif(b"89a", 1, false);
        assert_eq!(probe(&Probe { head: &data, ext: "gif", size: data.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"GIF90a", ext: "gif", size: 6 }), 0);
        let mut r = Reader::from_bytes(data);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "GIF");
        let s = doc.stream(StreamKind::Image, 0).unwrap();
        assert_eq!(s.get("Format_Profile"), "89a");
        assert_eq!(s.get("Width"), "64");
        assert_eq!(s.get("PixelAspectRatio"), "1.000");
        assert_eq!(s.get("DisplayAspectRatio"), "1.333");
        assert_eq!(s.get("DisplayAspectRatio/String"), "4:3");
        assert_eq!(s.get("Compression_Mode"), "Lossless");
    }
}

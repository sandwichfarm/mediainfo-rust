//! VC-3 / Avid DNxHD frame header (SMPTE ST 2019-1 §5): header prefix, image geometry,
//! sample bit depth and compression ID; plus the raw `.dnxhd` elementary stream parser.

use crate::io::{be16, be32};
use crate::model::{Doc, Stream, StreamKind};
use crate::io::Reader;
use crate::parsers::Probe;

/// Header prefix `00 00 02 80 01` (DNxHD); the last byte is 2 for 4:4:4 and 3 for DNxHR headers.
pub const PREFIX: [u8; 4] = [0x00, 0x00, 0x02, 0x80];

#[derive(Debug, Clone, Default)]
pub struct FrameHeader {
    pub version: u8,
    pub interlaced: bool,
    pub second_field: bool,
    pub height: u16, // active lines per frame (per field for interlaced material)
    pub width: u16,
    pub bit_depth: u8,
    pub is_444: bool,
    pub cid: u32,
}

/// Compression ID table (Avid DNxHD / DNxHR): profile name and fixed coded frame size (0 = variable).
struct Cid {
    cid: u32,
    profile: &'static str,
    frame_size: u32,
}

const CIDS: &[Cid] = &[
    Cid { cid: 1235, profile: "HQX", frame_size: 917504 },
    Cid { cid: 1237, profile: "SQ", frame_size: 606208 },
    Cid { cid: 1238, profile: "HQ", frame_size: 917504 },
    Cid { cid: 1241, profile: "HQX", frame_size: 917504 },
    Cid { cid: 1242, profile: "SQ", frame_size: 606208 },
    Cid { cid: 1243, profile: "HQ", frame_size: 917504 },
    Cid { cid: 1244, profile: "SQ", frame_size: 606208 },
    Cid { cid: 1250, profile: "HQX", frame_size: 458752 },
    Cid { cid: 1251, profile: "HQ", frame_size: 458752 },
    Cid { cid: 1252, profile: "SQ", frame_size: 303104 },
    Cid { cid: 1253, profile: "LB", frame_size: 188416 },
    Cid { cid: 1256, profile: "444", frame_size: 1835008 },
    Cid { cid: 1258, profile: "SQ", frame_size: 212992 },
    Cid { cid: 1259, profile: "SQ", frame_size: 303104 },
    Cid { cid: 1260, profile: "SQ", frame_size: 303104 },
    Cid { cid: 1270, profile: "444", frame_size: 0 },
    Cid { cid: 1271, profile: "HQX", frame_size: 0 },
    Cid { cid: 1272, profile: "HQ", frame_size: 0 },
    Cid { cid: 1273, profile: "SQ", frame_size: 0 },
    Cid { cid: 1274, profile: "LB", frame_size: 0 },
];

fn cid_info(cid: u32) -> Option<&'static Cid> {
    CIDS.iter().find(|c| c.cid == cid)
}

pub fn parse_frame(d: &[u8]) -> Option<FrameHeader> {
    if d.len() < 0x2D || d[..4] != PREFIX || !(1..=3).contains(&d[4]) {
        return None;
    }
    let h = FrameHeader {
        version: d[4],
        interlaced: d[5] & 2 != 0,
        second_field: d[5] & 1 != 0,
        height: be16(d, 0x18)?,
        width: be16(d, 0x1A)?,
        bit_depth: match d[0x21] >> 5 {
            1 => 8,
            2 => 10,
            3 => 12,
            _ => 0,
        },
        is_444: d[0x2C] & 0x40 != 0,
        cid: be32(d, 0x28)?,
    };
    if h.width == 0 || h.height == 0 || h.bit_depth == 0 {
        return None;
    }
    Some(h)
}

/// Coded frame size for a header: fixed per compression ID (DNxHD) or from the CID table only.
pub fn frame_size(h: &FrameHeader) -> Option<u32> {
    cid_info(h.cid).map(|c| c.frame_size).filter(|s| *s > 0)
}

/// Fill a video stream from a frame header.
pub fn apply_frame(s: &mut Stream, d: &[u8]) -> bool {
    let Some(h) = parse_frame(d) else { return false };
    s.set_if_empty("Format", "VC-3");
    s.set_if_empty("Format_Version", format!("Version {}", h.version));
    let family = if h.cid >= 1270 { "HR" } else { "HD" };
    if let Some(c) = cid_info(h.cid) {
        s.set_if_empty("Format_Profile", format!("{family}@{}", c.profile));
    }
    s.set_if_empty("BitRate_Mode", "CBR");
    s.set_if_empty("Width", h.width.to_string());
    s.set_if_empty("Height", (h.height as u32 * if h.interlaced { 2 } else { 1 }).to_string());
    s.set_if_empty("ColorSpace", "YUV");
    s.set_if_empty("ChromaSubsampling", if h.is_444 || h.cid == 1256 || h.cid == 1270 { "4:4:4" } else { "4:2:2" });
    s.set_if_empty("BitDepth", h.bit_depth.to_string());
    s.set_if_empty("ScanType", if h.interlaced { "Interlaced" } else { "Progressive" });
    true
}

// ---- elementary stream

pub fn probe(p: &Probe) -> u8 {
    let Some(h) = parse_frame(p.head) else { return 0 };
    if p.ext_in(&["dnxhd", "dnxhr", "vc3"]) {
        return 90;
    }
    // Frames are larger than the probe window; a file size that is a whole number of fixed-size
    // frames is the next best evidence of an elementary stream.
    match frame_size(&h) {
        Some(fs) if p.size % fs as u64 == 0 => 70,
        _ => 40,
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 0x30);
    let Some(h) = parse_frame(&head) else { return false };
    // A second frame at the fixed frame size confirms an elementary stream.
    if let Some(fs) = frame_size(&h) {
        if r.len() >= fs as u64 + 0x30 && parse_frame(r.read_at(fs as u64, 0x30)).is_none() {
            return false;
        }
    }
    let mut s = Stream::new(StreamKind::Video);
    apply_frame(&mut s, &head);
    // The frame rate is not signalled in the frame header, so no FrameCount/Duration/BitRate is
    // reported for a bare stream (the reference reports none either).
    let g = doc.general();
    g.set("Format", "VC-3");
    g.set("OverallBitRate_Mode", "CBR");
    doc.streams[StreamKind::Video as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(cid: u32, w: u16, h: u16, depth: u8, interlaced: bool) -> Vec<u8> {
        let mut d = vec![0u8; 0x30];
        d[..4].copy_from_slice(&PREFIX);
        d[4] = 1;
        d[5] = if interlaced { 2 } else { 0 };
        d[0x18..0x1A].copy_from_slice(&h.to_be_bytes());
        d[0x1A..0x1C].copy_from_slice(&w.to_be_bytes());
        d[0x21] = match depth { 8 => 1, 10 => 2, _ => 3 } << 5;
        d[0x28..0x2C].copy_from_slice(&cid.to_be_bytes());
        d[0x2C] = 0x80;
        d
    }

    #[test]
    fn header() {
        let d = frame(1253, 1920, 1080, 8, false);
        let h = parse_frame(&d).unwrap();
        assert_eq!((h.width, h.height, h.bit_depth, h.cid), (1920, 1080, 8, 1253));
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_frame(&mut s, &d));
        assert_eq!(s.get("Format"), "VC-3");
        assert_eq!(s.get("Format_Version"), "Version 1");
        assert_eq!(s.get("Format_Profile"), "HD@LB");
        assert_eq!(s.get("ChromaSubsampling"), "4:2:2");
        assert_eq!(s.get("ScanType"), "Progressive");
        let d = frame(1241, 1920, 540, 10, true);
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_frame(&mut s, &d));
        assert_eq!(s.get("Height"), "1080");
        assert_eq!(s.get("ScanType"), "Interlaced");
        assert_eq!(s.get("Format_Profile"), "HD@HQX");
        assert!(parse_frame(&d[..20]).is_none());
        assert!(parse_frame(&[0; 0x30]).is_none());
    }

    #[test]
    fn stream() {
        let f = frame(1253, 1920, 1080, 8, false);
        let mut data = vec![0u8; 188416 * 2];
        data[..f.len()].copy_from_slice(&f);
        data[188416..188416 + f.len()].copy_from_slice(&f);
        assert_eq!(probe(&Probe { head: &data[..65536], ext: "dnxhd", size: data.len() as u64 }), 90);
        assert_eq!(probe(&Probe { head: &data[..65536], ext: "bin", size: data.len() as u64 }), 70);
        assert_eq!(probe(&Probe { head: &data[..100], ext: "bin", size: 100 }), 40);
        assert_eq!(probe(&Probe { head: &data[..20], ext: "dnxhd", size: 20 }), 0);
        let mut bad = data.clone();
        bad[188416] = 1;
        assert!(!parse(&mut Reader::from_bytes(bad), &mut Doc::new()));
        let mut r = Reader::from_bytes(data);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "VC-3");
        assert_eq!(doc.general_ref().get("OverallBitRate_Mode"), "CBR");
        assert_eq!(doc.streams[StreamKind::Video as usize][0].get("Width"), "1920");
    }
}

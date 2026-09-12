//! JPEG 2000: JP2 box structure (ISO/IEC 15444-1 Annex I) and the raw codestream (SOC/SIZ/COD).

use crate::io::{be16, be32, be64, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

/// Bytes of the file inspected for boxes and codestream headers.
const HEADER_LIMIT: usize = 1 << 20;
const MAX_BOXES: usize = 256;
const MAX_MARKERS: usize = 256;

pub const SIGNATURE: [u8; 12] = [0, 0, 0, 0x0C, b'j', b'P', b' ', b' ', 0x0D, 0x0A, 0x87, 0x0A];

/// SIZ marker segment of the codestream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Siz {
    /// Capabilities (`Rsiz`).
    pub rsiz: u16,
    pub width: u32,
    pub height: u32,
    /// Per component: (bit depth, signed, XRsiz, YRsiz).
    pub components: Vec<(u8, bool, u8, u8)>,
}

/// What the file headers say about the image.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    /// `ftyp` major brand and compatible brands (JP2 files only).
    pub brand: Option<String>,
    pub compatible: Vec<String>,
    /// `ihdr`: (height, width, component count, bits per component - 1 or 255).
    pub ihdr: Option<(u32, u32, u16, u8)>,
    /// `colr` enumerated colour space (meth 1) when present.
    pub enum_cs: Option<u32>,
    /// `colr` uses an ICC profile (meth 2 or 3).
    pub icc: bool,
    pub siz: Option<Siz>,
    /// COD wavelet: `true` for the reversible 5/3 transform.
    pub reversible: Option<bool>,
}

impl Info {
    pub fn width(&self) -> Option<u32> {
        self.siz.as_ref().map(|s| s.width).or(self.ihdr.map(|h| h.1))
    }

    pub fn height(&self) -> Option<u32> {
        self.siz.as_ref().map(|s| s.height).or(self.ihdr.map(|h| h.0))
    }

    pub fn component_count(&self) -> usize {
        self.siz.as_ref().map(|s| s.components.len()).or(self.ihdr.map(|h| h.2 as usize)).unwrap_or(0)
    }

    pub fn bit_depth(&self) -> Option<u8> {
        if let Some(c) = self.siz.as_ref().and_then(|s| s.components.first()) {
            return Some(c.0);
        }
        self.ihdr.and_then(|h| (h.3 != 255).then_some((h.3 & 0x7F) + 1))
    }

    pub fn color_space(&self) -> Option<&'static str> {
        Some(match self.enum_cs {
            Some(16 | 20 | 21) => "RGB",
            Some(17) => "Y",
            Some(18 | 22) => "YUV",
            Some(12) => "CMYK",
            Some(14) => "Lab",
            _ => match self.component_count() {
                1 => "Y",
                3 if self.icc => "RGB",
                _ => return None,
            },
        })
    }

    pub fn chroma_subsampling(&self) -> Option<&'static str> {
        // sYCC / e-sYCC are full-resolution YCbCr by definition, whatever the codestream stores.
        if matches!(self.enum_cs, Some(18 | 22)) {
            return Some("4:4:4");
        }
        let c = &self.siz.as_ref()?.components;
        if c.len() < 3 {
            return None;
        }
        let (x0, y0) = (c[0].2, c[0].3);
        if x0 == 0 || y0 == 0 {
            return None;
        }
        let (x1, y1) = (c[1].2, c[1].3);
        if c[2].2 != x1 || c[2].3 != y1 {
            return None;
        }
        Some(match (x1 / x0, y1 / y0) {
            (1, 1) => "4:4:4",
            (2, 1) => "4:2:2",
            (2, 2) => "4:2:0",
            (4, 1) => "4:1:1",
            _ => return None,
        })
    }

    /// Capabilities name from `Rsiz`.
    pub fn profile(&self) -> Option<&'static str> {
        Some(match self.siz.as_ref()?.rsiz {
            0 => "No restrictions",
            1 => "Profile 0",
            2 => "Profile 1",
            3 => "2K Digital Cinema",
            4 => "4K Digital Cinema",
            5 => "Scalable 2K Digital Cinema",
            6 => "Scalable 4K Digital Cinema",
            _ => return None,
        })
    }
}

/// Parse a raw codestream from SOC up to the first tile-part (SIZ, COD).
pub fn parse_codestream(d: &[u8]) -> Option<(Siz, Option<bool>)> {
    if d.get(0..2)? != [0xFF, 0x4F] {
        return None;
    }
    let mut pos = 2usize;
    let mut siz = None;
    let mut reversible = None;
    for _ in 0..MAX_MARKERS {
        if *d.get(pos)? != 0xFF {
            return None;
        }
        let marker = *d.get(pos + 1)?;
        if marker == 0x90 || marker == 0xD9 {
            break; // SOT / EOC
        }
        let len = be16(d, pos + 2)? as usize;
        if len < 2 {
            return None;
        }
        let seg = d.get(pos + 4..pos + 2 + len)?;
        match marker {
            0x51 => {
                let xsiz = be32(seg, 2)?;
                let ysiz = be32(seg, 6)?;
                let xo = be32(seg, 10)?;
                let yo = be32(seg, 14)?;
                let n = be16(seg, 34)? as usize;
                if n == 0 || n > 16384 || xsiz <= xo || ysiz <= yo {
                    return None;
                }
                let mut components = Vec::with_capacity(n.min(256));
                for i in 0..n {
                    let c = seg.get(36 + i * 3..39 + i * 3)?;
                    components.push(((c[0] & 0x7F) + 1, c[0] & 0x80 != 0, c[1], c[2]));
                }
                siz = Some(Siz { rsiz: be16(seg, 0)?, width: xsiz - xo, height: ysiz - yo, components });
            }
            0x52 => {
                // Scod, SGcod (progression, layers u16, mct), SPcod: levels, cbw, cbh, cbstyle, transform
                reversible = seg.get(9).map(|t| *t == 1);
            }
            _ => {}
        }
        pos += 2 + len;
        if siz.is_some() && (marker == 0x52 || reversible.is_some()) {
            break;
        }
    }
    Some((siz?, reversible))
}

fn brand(b: &[u8]) -> String {
    b.iter().map(|&c| if c.is_ascii_graphic() || c == b' ' { c as char } else { '?' }).collect()
}

/// Walk the JP2 boxes (`jP  `, `ftyp`, `jp2h`, `jp2c`).
pub fn parse_boxes(data: &[u8]) -> Option<Info> {
    if !data.starts_with(&SIGNATURE) {
        return None;
    }
    let mut info = Info::default();
    let mut pos = 0usize;
    for _ in 0..MAX_BOXES {
        let Some(size32) = be32(data, pos) else { break };
        let kind = data.get(pos + 4..pos + 8)?;
        let (header, size) = match size32 {
            0 => (8usize, data.len().saturating_sub(pos)),
            1 => (16usize, be64(data, pos + 8)?.min(usize::MAX as u64) as usize),
            n => (8usize, n as usize),
        };
        if size < header {
            break;
        }
        let end = pos.saturating_add(size).min(data.len());
        let body = data.get(pos + header..end).unwrap_or(&[]);
        match kind {
            b"ftyp" => {
                info.brand = body.get(0..4).map(brand);
                info.compatible = body.get(8..).unwrap_or(&[]).chunks_exact(4).take(64).map(brand).collect();
            }
            b"jp2h" => {
                let mut p = 0usize;
                for _ in 0..MAX_BOXES {
                    let Some(sz) = be32(body, p) else { break };
                    let Some(t) = body.get(p + 4..p + 8) else { break };
                    let sz = if sz == 0 { body.len() - p } else { sz as usize };
                    if sz < 8 {
                        break;
                    }
                    let sub = body.get(p + 8..(p + sz).min(body.len())).unwrap_or(&[]);
                    match t {
                        b"ihdr" => info.ihdr = Some((be32(sub, 0)?, be32(sub, 4)?, be16(sub, 8)?, *sub.get(10)?)),
                        b"colr" => match sub.first() {
                            Some(1) => info.enum_cs = be32(sub, 3),
                            Some(2 | 3) => info.icc = true,
                            _ => {}
                        },
                        _ => {}
                    }
                    p += sz;
                }
            }
            b"jp2c" => {
                if let Some((siz, rev)) = parse_codestream(body) {
                    info.siz = Some(siz);
                    info.reversible = rev;
                }
                break;
            }
            _ => {}
        }
        if end <= pos || end >= data.len() {
            break;
        }
        pos = end;
    }
    (info.ihdr.is_some() || info.siz.is_some()).then_some(info)
}

pub fn probe(p: &Probe) -> u8 {
    if p.starts_with(&SIGNATURE) {
        return 100;
    }
    if p.starts_with(&[0xFF, 0x4F, 0xFF, 0x51]) {
        return if p.ext_in(&["j2k", "j2c", "jpc", "jpx", "jpf"]) { 100 } else { 90 };
    }
    0
}

fn apply(s: &mut Stream, info: &Info) {
    s.set("Format", "JPEG 2000");
    if let Some(p) = info.profile() {
        s.set("Format_Profile", p);
    }
    if let (Some(w), Some(h)) = (info.width(), info.height()) {
        s.set_int("Width", w as i128);
        s.set_int("Height", h as i128);
    }
    if let Some(cs) = info.color_space() {
        s.set("ColorSpace", cs);
    }
    if let Some(sub) = info.chroma_subsampling() {
        s.set("ChromaSubsampling", sub);
    }
    if let Some(b) = info.bit_depth() {
        s.set_int("BitDepth", b as i128);
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let n = r.len().min(HEADER_LIMIT as u64) as usize;
    let data = r.read_vec_at(0, n);
    let info = if data.starts_with(&SIGNATURE) {
        parse_boxes(&data)
    } else {
        parse_codestream(&data).map(|(siz, rev)| Info { siz: Some(siz), reversible: rev, ..Default::default() })
    };
    let Some(info) = info else { return false };
    let g = doc.general();
    g.set("Format", "JPEG 2000");
    if let Some(b) = &info.brand {
        // The reference labels the box-structured (ISO base media style) file as an MPEG-4 profile.
        g.set("Format_Profile", "MPEG-4");
        g.set("CodecID", b.clone());
        let compat = if info.compatible.is_empty() { b.clone() } else { info.compatible.join("/") };
        g.set("CodecID/String", format!("{b} ({compat})"));
        g.set("CodecID_Compatible", compat);
    }
    let mut s = Stream::new(StreamKind::Image);
    apply(&mut s, &info);
    doc.streams[StreamKind::Image as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    fn codestream(w: u32, h: u32, comps: &[(u8, u8, u8)], rsiz: u16, reversible: bool) -> Vec<u8> {
        let mut v = vec![0xFF, 0x4F, 0xFF, 0x51];
        let len = 38 + 3 * comps.len() as u16;
        v.extend_from_slice(&len.to_be_bytes());
        v.extend_from_slice(&rsiz.to_be_bytes());
        for x in [w, h, 0, 0, w, h, 0, 0] {
            v.extend_from_slice(&x.to_be_bytes());
        }
        v.extend_from_slice(&(comps.len() as u16).to_be_bytes());
        for (depth, xr, yr) in comps {
            v.extend_from_slice(&[depth - 1, *xr, *yr]);
        }
        // COD: Lcod=12, Scod, SGcod(prog, layers, mct), SPcod(levels, cbw, cbh, cbstyle, transform)
        v.extend_from_slice(&[0xFF, 0x52, 0, 12, 0, 0, 0, 1, 1, 5, 4, 4, 0, if reversible { 1 } else { 0 }]);
        v.extend_from_slice(&[0xFF, 0x90, 0, 10, 0, 0, 0, 0, 0, 0, 0, 1]);
        v
    }

    fn jp2(cs: &[u8], enum_cs: u32) -> Vec<u8> {
        let mut v = SIGNATURE.to_vec();
        v.extend_from_slice(&bx(b"ftyp", b"jp2 \0\0\0\0jp2 "));
        let mut ihdr = 48u32.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&64u32.to_be_bytes());
        ihdr.extend_from_slice(&[0, 3, 7, 7, 0, 0]);
        let mut colr = vec![1, 0, 0];
        colr.extend_from_slice(&enum_cs.to_be_bytes());
        let mut jp2h = bx(b"ihdr", &ihdr);
        jp2h.extend_from_slice(&bx(b"colr", &colr));
        v.extend_from_slice(&bx(b"jp2h", &jp2h));
        v.extend_from_slice(&bx(b"jp2c", cs));
        v
    }

    #[test]
    fn codestream_siz_and_cod() {
        let (siz, rev) = parse_codestream(&codestream(64, 48, &[(8, 1, 1), (8, 2, 2), (8, 2, 2)], 0, true)).unwrap();
        assert_eq!((siz.width, siz.height, siz.rsiz, siz.components.len()), (64, 48, 0, 3));
        assert_eq!(siz.components[1], (8, false, 2, 2));
        assert_eq!(rev, Some(true));
        let info = Info { siz: Some(siz), ..Default::default() };
        assert_eq!(info.chroma_subsampling(), Some("4:2:0"));
        assert_eq!(info.profile(), Some("No restrictions"));
        assert_eq!(info.bit_depth(), Some(8));
        assert!(parse_codestream(&[0xFF, 0x4F, 0xFF, 0x51, 0, 4]).is_none());
        assert!(parse_codestream(b"\xFF\xD8").is_none());
        let mut bad = codestream(64, 48, &[(8, 1, 1)], 0, true);
        bad[16..20].copy_from_slice(&100u32.to_be_bytes()); // XOsiz > Xsiz
        assert!(parse_codestream(&bad).is_none());
    }

    #[test]
    fn boxes() {
        let data = jp2(&codestream(64, 48, &[(8, 1, 1), (8, 1, 1), (8, 1, 1)], 0, false), 18);
        let info = parse_boxes(&data).unwrap();
        assert_eq!(info.brand.as_deref(), Some("jp2 "));
        assert_eq!(info.compatible, vec!["jp2 ".to_string()]);
        assert_eq!(info.ihdr, Some((48, 64, 3, 7)));
        assert_eq!(info.enum_cs, Some(18));
        assert_eq!(info.color_space(), Some("YUV"));
        assert_eq!(info.chroma_subsampling(), Some("4:4:4"));
        assert_eq!(info.reversible, Some(false));
        assert!(parse_boxes(&data[..30]).is_none());
        assert!(parse_boxes(b"\0\0\0\x0CjP  \r\n\x87\n").is_none());
    }

    #[test]
    fn probe_and_parse() {
        let cs = codestream(64, 48, &[(8, 1, 1), (8, 1, 1), (8, 1, 1)], 0, false);
        let data = jp2(&cs, 18);
        assert_eq!(probe(&Probe { head: &data, ext: "jp2", size: data.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: &cs, ext: "j2k", size: cs.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"\xFF\xD8", ext: "jp2", size: 2 }), 0);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(data), &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "JPEG 2000");
        assert_eq!(g.get("CodecID"), "jp2 ");
        assert_eq!(g.get("CodecID/String"), "jp2  (jp2 )");
        assert_eq!(g.get("CodecID_Compatible"), "jp2 ");
        assert_eq!(g.get("Format_Profile"), "MPEG-4");
        let s = doc.stream(StreamKind::Image, 0).unwrap();
        assert_eq!(s.get("Format_Profile"), "No restrictions");
        assert_eq!((s.get("Width"), s.get("Height")), ("64", "48"));
        assert_eq!(s.get("ColorSpace"), "YUV");
        assert_eq!(s.get("ChromaSubsampling"), "4:4:4");
        assert_eq!(s.get("BitDepth"), "8");
        // Raw codestream: no brand, greyscale.
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(codestream(10, 20, &[(12, 1, 1)], 2, true)), &mut doc));
        assert!(!doc.general_ref().has("CodecID"));
        let s = doc.stream(StreamKind::Image, 0).unwrap();
        assert_eq!((s.get("Width"), s.get("BitDepth"), s.get("ColorSpace"), s.get("Format_Profile")), ("10", "12", "Y", "Profile 1"));
    }
}

//! TIFF 6.0: byte-order header and the tags of the first image file directory (IFD0).

use crate::io::{be16, be32, le16, le32, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

const MAX_ENTRIES: usize = 512;
const MAX_STRING: usize = 4096;

/// Tags of IFD0 that describe the image.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ifd {
    pub little_endian: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// BitsPerSample, first value.
    pub bits_per_sample: Option<u16>,
    pub compression: Option<u16>,
    pub photometric: Option<u16>,
    pub samples_per_pixel: Option<u16>,
    pub x_resolution: Option<(u32, u32)>,
    pub y_resolution: Option<(u32, u32)>,
    /// 1 none, 2 inch, 3 centimetre.
    pub resolution_unit: Option<u16>,
    pub software: Option<String>,
    pub date_time: Option<String>,
}

impl Ifd {
    /// Name of the compression scheme as the reference reports the image `Format`.
    pub fn compression_name(&self) -> Option<&'static str> {
        Some(match self.compression.unwrap_or(1) {
            1 => "Raw",
            2 => "CCITT RLE",
            3 => "CCITT Group 3",
            4 => "CCITT Group 4",
            5 => "LZW",
            6 | 7 => "JPEG",
            8 | 32946 => "Deflate",
            32773 => "PackBits",
            34712 => "JPEG 2000",
            _ => return None,
        })
    }

    pub fn lossy(&self) -> bool {
        matches!(self.compression, Some(6 | 7 | 34712))
    }

    pub fn color_space(&self) -> Option<&'static str> {
        Some(match self.photometric? {
            0 | 1 => "Y",
            2 | 3 => "RGB",
            5 => "CMYK",
            6 => "YUV",
            8 => "Lab",
            _ => return None,
        })
    }

    /// Density unit name (`dpi` / `dpcm`) when the resolution is meaningful.
    pub fn density_unit(&self) -> Option<&'static str> {
        match self.resolution_unit.unwrap_or(2) {
            2 => Some("dpi"),
            3 => Some("dpcm"),
            _ => None,
        }
    }
}

struct Order(bool);

impl Order {
    fn u16(&self, b: &[u8], o: usize) -> Option<u16> {
        if self.0 { le16(b, o) } else { be16(b, o) }
    }
    fn u32(&self, b: &[u8], o: usize) -> Option<u32> {
        if self.0 { le32(b, o) } else { be32(b, o) }
    }
}

/// Byte order and IFD0 offset from the 8-byte header.
pub fn parse_header(data: &[u8]) -> Option<(bool, u32)> {
    let le = match data.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let o = Order(le);
    if o.u16(data, 2)? != 42 {
        return None;
    }
    let off = o.u32(data, 4)?;
    if off < 8 {
        return None;
    }
    Some((le, off))
}

fn type_size(t: u16) -> usize {
    match t {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => 0,
    }
}

/// Read IFD0 through the reader.
pub fn parse_ifd0(r: &mut Reader) -> Option<Ifd> {
    let head = r.read_vec_at(0, 8);
    let (le, off) = parse_header(&head)?;
    let o = Order(le);
    let n = o.u16(&r.read_vec_at(off as u64, 2), 0)? as usize;
    if n == 0 || n > MAX_ENTRIES {
        return None;
    }
    let entries = r.read_vec_at(off as u64 + 2, 12 * n);
    let mut ifd = Ifd { little_endian: le, ..Default::default() };
    for e in entries.chunks_exact(12) {
        let tag = o.u16(e, 0)?;
        let typ = o.u16(e, 2)?;
        let count = o.u32(e, 4)? as usize;
        let size = type_size(typ).checked_mul(count)?;
        // Values of 4 bytes or fewer are stored in the entry itself.
        let value: Vec<u8> = if size <= 4 { e[8..8 + size].to_vec() } else { r.read_vec_at(o.u32(e, 8)? as u64, size.min(MAX_STRING)) };
        let short = || match typ {
            3 => o.u16(&value, 0).map(u32::from),
            4 => o.u32(&value, 0),
            1 => value.first().map(|v| *v as u32),
            _ => None,
        };
        let rational = || if typ == 5 { Some((o.u32(&value, 0)?, o.u32(&value, 4)?)) } else { None };
        let text = || if typ == 2 { Some(crate::io::clean_text(&crate::io::cstr(&value))) } else { None };
        match tag {
            256 => ifd.width = short(),
            257 => ifd.height = short(),
            258 => ifd.bits_per_sample = short().map(|v| v as u16),
            259 => ifd.compression = short().map(|v| v as u16),
            262 => ifd.photometric = short().map(|v| v as u16),
            277 => ifd.samples_per_pixel = short().map(|v| v as u16),
            282 => ifd.x_resolution = rational(),
            283 => ifd.y_resolution = rational(),
            296 => ifd.resolution_unit = short().map(|v| v as u16),
            305 => ifd.software = text().filter(|t| !t.is_empty()),
            306 => ifd.date_time = text().filter(|t| !t.is_empty()),
            _ => {}
        }
    }
    Some(ifd)
}

fn density(num: u32, den: u32) -> Option<f64> {
    (den != 0 && num != 0).then(|| num as f64 / den as f64)
}

fn density_text(v: f64) -> String {
    if (v - v.round()).abs() < 1e-6 { format!("{}", v.round() as u64) } else { format!("{v:.3}").trim_end_matches('0').trim_end_matches('.').to_string() }
}

pub fn probe(p: &Probe) -> u8 {
    match parse_header(p.head) {
        Some((_, off)) if (off as u64) < p.size => 100,
        Some(_) => 50,
        None => 0,
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let Some(ifd) = parse_ifd0(r) else { return false };
    let (Some(w), Some(h)) = (ifd.width, ifd.height) else { return false };
    if w == 0 || h == 0 {
        return false;
    }
    doc.general().set("Format", "TIFF");
    if let Some(sw) = &ifd.software {
        // The reference stores the Software tag as the application name only.
        doc.general().set("Encoded_Application_Name", sw.clone());
        doc.general().set("Encoded_Application/String", sw.clone());
    }
    let mut s = Stream::new(StreamKind::Image);
    if let Some(name) = ifd.compression_name() {
        s.set("Format", name);
    }
    let endian = if ifd.little_endian { "Little" } else { "Big" };
    s.set("Format_Settings_Endianness", endian);
    s.set("Format_Settings", endian);
    s.set_int("Width", w as i128);
    s.set_int("Height", h as i128);
    if let Some(cs) = ifd.color_space() {
        s.set("ColorSpace", cs);
    }
    if let Some(b) = ifd.bits_per_sample {
        s.set_int("BitDepth", b as i128);
    }
    s.set("Compression_Mode", if ifd.lossy() { "Lossy" } else { "Lossless" });
    let x = ifd.x_resolution.and_then(|(n, d)| density(n, d));
    let y = ifd.y_resolution.and_then(|(n, d)| density(n, d));
    if let (Some(x), Some(y), Some(unit)) = (x, y, ifd.density_unit()) {
        let (xs, ys) = (density_text(x), density_text(y));
        s.set_extra("Density_X", xs.clone(), "", "N NT");
        s.set_extra("Density_Y", ys.clone(), "", "N NT");
        s.set_extra("Density_Unit", unit, "", "N NT");
        let text = if xs == ys { format!("{xs} {unit}") } else { format!("{xs}x{ys} {unit}") };
        s.set_extra("Density/String", text, "", "Y NT");
    }
    doc.streams[StreamKind::Image as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a little- or big-endian TIFF with IFD0 at offset 8 and out-of-line values after it.
    fn tiff(le: bool, entries: &[(u16, u16, u32, Vec<u8>)]) -> Vec<u8> {
        let w16 = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let w32 = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut v = if le { b"II".to_vec() } else { b"MM".to_vec() };
        v.extend_from_slice(&w16(42));
        v.extend_from_slice(&w32(8));
        v.extend_from_slice(&w16(entries.len() as u16));
        let mut tail: Vec<u8> = Vec::new();
        let tail_base = 8 + 2 + 12 * entries.len() + 4;
        for (tag, typ, count, val) in entries {
            v.extend_from_slice(&w16(*tag));
            v.extend_from_slice(&w16(*typ));
            v.extend_from_slice(&w32(*count));
            if val.len() <= 4 {
                let mut inline = val.clone();
                inline.resize(4, 0);
                v.extend_from_slice(&inline);
            } else {
                v.extend_from_slice(&w32((tail_base + tail.len()) as u32));
                tail.extend_from_slice(val);
            }
        }
        v.extend_from_slice(&w32(0));
        v.extend_from_slice(&tail);
        v
    }

    fn short(le: bool, v: u16) -> Vec<u8> {
        if le { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() }
    }

    fn sample(le: bool) -> Vec<u8> {
        let l32 = |v: u32| if le { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
        let mut bps = Vec::new();
        for _ in 0..3 {
            bps.extend_from_slice(&short(le, 8));
        }
        let mut rat = l32(72);
        rat.extend_from_slice(&l32(1));
        tiff(
            le,
            &[
                (256, 4, 1, l32(64)),
                (257, 4, 1, l32(48)),
                (258, 3, 3, bps),
                (259, 3, 1, short(le, 32773)),
                (262, 3, 1, short(le, 6)),
                (277, 3, 1, short(le, 3)),
                (282, 5, 1, rat.clone()),
                (283, 5, 1, rat),
                (296, 3, 1, short(le, 2)),
                (305, 2, 13, b"Lavc63.1.101\0".to_vec()),
            ],
        )
    }

    #[test]
    fn ifd_both_orders() {
        for le in [true, false] {
            let mut r = Reader::from_bytes(sample(le));
            let ifd = parse_ifd0(&mut r).unwrap();
            assert_eq!(ifd.little_endian, le);
            assert_eq!((ifd.width, ifd.height, ifd.bits_per_sample), (Some(64), Some(48), Some(8)));
            assert_eq!(ifd.compression_name(), Some("PackBits"));
            assert_eq!(ifd.color_space(), Some("YUV"));
            assert_eq!(ifd.x_resolution, Some((72, 1)));
            assert_eq!(ifd.software.as_deref(), Some("Lavc63.1.101"));
            assert!(!ifd.lossy());
        }
    }

    #[test]
    fn malformed() {
        assert!(parse_header(b"II\x2A\x00").is_none());
        assert!(parse_header(b"II\x2B\x00\x08\x00\x00\x00").is_none());
        assert!(parse_header(b"XX\x2A\x00\x08\x00\x00\x00").is_none());
        let mut r = Reader::from_bytes(b"II\x2A\x00\x08\x00\x00\x00\xFF\xFF".to_vec());
        assert!(parse_ifd0(&mut r).is_none());
        let mut r = Reader::from_bytes(b"II\x2A\x00\x00\x10\x00\x00".to_vec());
        assert!(parse_ifd0(&mut r).is_none());
        let mut doc = Doc::new();
        let mut r = Reader::from_bytes(tiff(true, &[(259, 3, 1, short(true, 1))]));
        assert!(!parse(&mut r, &mut doc));
    }

    #[test]
    fn probe_and_parse() {
        let data = sample(true);
        assert_eq!(probe(&Probe { head: &data, ext: "tif", size: data.len() as u64 }), 100);
        let mut r = Reader::from_bytes(data);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "TIFF");
        assert_eq!(g.get("Encoded_Application_Name"), "Lavc63.1.101");
        let s = doc.stream(StreamKind::Image, 0).unwrap();
        assert_eq!(s.get("Format"), "PackBits");
        assert_eq!(s.get("Format_Settings"), "Little");
        assert_eq!(s.get("Width"), "64");
        assert_eq!(s.get("ColorSpace"), "YUV");
        assert_eq!(s.get("BitDepth"), "8");
        assert_eq!(s.get("Compression_Mode"), "Lossless");
        assert_eq!(s.get("Density_X"), "72");
        assert_eq!(s.get("Density_Unit"), "dpi");
        assert_eq!(s.get("Density/String"), "72 dpi");
        assert_eq!(density_text(300.5), "300.5");
    }
}

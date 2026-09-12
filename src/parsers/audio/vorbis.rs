//! Vorbis identification / comment / setup headers (Vorbis I specification) and the VorbisComment
//! block shared with FLAC and Opus.

use crate::io::{clean_text, le32};
use crate::model::Stream;

/// Longest vendor/tag string we keep (bytes).
const MAX_TEXT: usize = 64 * 1024;
/// Most user comments we look at.
const MAX_ITEMS: usize = 4096;

/// A decoded VorbisComment block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Comments {
    pub vendor: String,
    /// (KEY upper-cased, value) in file order.
    pub items: Vec<(String, String)>,
}

/// Parse a raw VorbisComment block (vendor length/string, item count, `KEY=value` items).
/// Lenient about a truncated item list: what was read is returned.
pub fn parse_comments(d: &[u8]) -> Option<Comments> {
    let vendor_len = le32(d, 0)? as usize;
    if vendor_len > MAX_TEXT {
        return None;
    }
    let vendor = String::from_utf8_lossy(d.get(4..4 + vendor_len)?).into_owned();
    let mut pos = 4 + vendor_len;
    let mut out = Comments { vendor: clean_text(&vendor), items: Vec::new() };
    let Some(count) = le32(d, pos) else { return Some(out) };
    pos += 4;
    for _ in 0..(count as usize).min(MAX_ITEMS) {
        let Some(len) = le32(d, pos) else { break };
        pos += 4;
        let len = len as usize;
        if len > MAX_TEXT {
            break;
        }
        let Some(item) = d.get(pos..pos + len) else { break };
        pos += len;
        let text = String::from_utf8_lossy(item);
        if let Some((k, v)) = text.split_once('=') {
            out.items.push((k.trim().to_ascii_uppercase(), clean_text(v)));
        }
    }
    Some(out)
}

/// Schema field for a VorbisComment key (General-style tag names).
fn tag_field(key: &str) -> Option<&'static str> {
    Some(match key {
        "TITLE" => "Title",
        "ARTIST" => "Performer",
        "ALBUM" => "Album",
        "ALBUMARTIST" | "ALBUM ARTIST" | "ALBUM_ARTIST" => "Album/Performer",
        "DATE" | "YEAR" => "Recorded_Date",
        "TRACKNUMBER" => "Track/Position",
        "TRACKTOTAL" | "TOTALTRACKS" => "Track/Position_Total",
        "DISCNUMBER" => "Part/Position",
        "DISCTOTAL" | "TOTALDISCS" => "Part/Position_Total",
        "GENRE" => "Genre",
        "COMMENT" | "COMMENTS" | "DESCRIPTION" => "Comment",
        "COMPOSER" => "Composer",
        "CONDUCTOR" => "Conductor",
        "LYRICIST" => "Lyricist",
        "COPYRIGHT" => "Copyright",
        "ORGANIZATION" | "PUBLISHER" | "LABEL" => "Publisher",
        "ISRC" => "ISRC",
        "LYRICS" | "UNSYNCEDLYRICS" => "Lyrics",
        "LANGUAGE" => "Language",
        "REPLAYGAIN_TRACK_GAIN" => "ReplayGain_Gain",
        "REPLAYGAIN_TRACK_PEAK" => "ReplayGain_Peak",
        "REPLAYGAIN_ALBUM_GAIN" => "Album_ReplayGain_Gain",
        "REPLAYGAIN_ALBUM_PEAK" => "Album_ReplayGain_Peak",
        "ENCODED_BY" => "EncodedBy",
        "ENCODER_OPTIONS" | "ENCODER_SETTINGS" => "Encoded_Library_Settings",
        _ => return None,
    })
}

/// Apply a decoded comment block: the vendor string is the stream's `Encoded_Library`, `ENCODER` is
/// `Encoded_Application` and the other tags go to `general` when given (native FLAC/Ogg), otherwise
/// to the stream itself (Matroska CodecPrivate).
pub fn apply_parsed_comments(s: &mut Stream, general: Option<&mut Stream>, c: &Comments) {
    if !c.vendor.is_empty() {
        apply_vendor(s, &c.vendor);
    }
    let (target, on_stream) = match general {
        Some(g) => (g, false),
        None => (&mut *s, true),
    };
    for (k, v) in &c.items {
        if v.is_empty() {
            continue;
        }
        match k.as_str() {
            "ENCODER" => target.set_if_empty("Encoded_Application", v),
            "TITLE" => {
                target.set_if_empty("Title", v);
                if !on_stream {
                    target.set_if_empty("Track", v);
                }
            }
            "METADATA_BLOCK_PICTURE" => {}
            _ => match tag_field(k) {
                Some(f) => {
                    let v = if f.ends_with("_Gain") { v.trim_end_matches("dB").trim().to_string() } else { v.clone() };
                    if !target.has(f) {
                        target.set(f, v);
                    }
                }
                None => {
                    if !target.has(k) {
                        target.set_extra(k, v, "", "Y NT");
                    }
                }
            },
        }
    }
}

/// Vendor string → `Encoded_Library` (+ Name/Version/Date for the libVorbis pattern
/// `Xiph.Org libVorbis I 20200704 (Reducing Environment)`, as the reference splits it).
pub fn apply_vendor(s: &mut Stream, vendor: &str) {
    s.set("Encoded_Library", vendor);
    s.clear("Encoded_Library/String");
    s.clear("Encoded_Library_Name");
    s.clear("Encoded_Library_Version");
    s.clear("Encoded_Library_Date");
    if let Some(rest) = vendor.strip_prefix("Xiph.Org ") {
        if let Some((name, date)) = rest.split_once(" I ") {
            s.set("Encoded_Library_Name", name);
            s.set("Encoded_Library_Date", date);
            if let Some((_, ver)) = date.split_once(' ') {
                s.set("Encoded_Library_Version", ver);
            }
        }
    }
}

/// VorbisComment block (after the 7-byte packet header for Vorbis, raw for FLAC/Opus).
pub fn apply_comments(s: &mut Stream, general: &mut Stream, d: &[u8]) -> bool {
    match parse_comments(d) {
        Some(c) => {
            apply_parsed_comments(s, Some(general), &c);
            true
        }
        None => false,
    }
}

/// Comment block with the tags on the stream itself (no General stream available).
pub fn apply_comments_to_stream(s: &mut Stream, d: &[u8]) -> bool {
    match parse_comments(d) {
        Some(c) => {
            apply_parsed_comments(s, None, &c);
            true
        }
        None => false,
    }
}

/// A `\x03vorbis` comment packet: strips the packet header then applies the block.
pub fn apply_comment_packet(s: &mut Stream, general: &mut Stream, packet: &[u8]) -> bool {
    match packet.strip_prefix(b"\x03vorbis") {
        Some(body) => apply_comments(s, general, body),
        None => false,
    }
}

// ---------------------------------------------------------------------------- identification

/// Decoded identification header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ident {
    pub version: u32,
    pub channels: u8,
    pub sample_rate: u32,
    pub bitrate_max: i32,
    pub bitrate_nominal: i32,
    pub bitrate_min: i32,
    pub blocksize_0: u32,
    pub blocksize_1: u32,
}

/// Parse a `\x01vorbis` identification packet (the packet header is required).
pub fn parse_ident(d: &[u8]) -> Option<Ident> {
    let b = d.strip_prefix(b"\x01vorbis")?;
    if b.len() < 23 {
        return None;
    }
    let bs = b[22];
    let i = Ident {
        version: le32(b, 0)?,
        channels: b[4],
        sample_rate: le32(b, 5)?,
        bitrate_max: le32(b, 9)? as i32,
        bitrate_nominal: le32(b, 13)? as i32,
        bitrate_min: le32(b, 17)? as i32,
        blocksize_0: 1 << (bs & 0x0F),
        blocksize_1: 1 << (bs >> 4),
    };
    if i.version != 0 || i.channels == 0 || i.sample_rate == 0 {
        return None;
    }
    Some(i)
}

/// Fill a stream from the identification header.
pub fn apply_ident(s: &mut Stream, d: &[u8]) -> bool {
    let Some(i) = parse_ident(d) else { return false };
    s.set_if_empty("Format", "Vorbis");
    s.set("Channel(s)", i.channels.to_string());
    s.set("SamplingRate", i.sample_rate.to_string());
    let (max, nom, min) = (i.bitrate_max.max(0), i.bitrate_nominal.max(0), i.bitrate_min.max(0));
    if nom > 0 {
        s.set("BitRate", nom.to_string());
    }
    if max > 0 {
        s.set("BitRate_Maximum", max.to_string());
    }
    if min > 0 {
        s.set("BitRate_Minimum", min.to_string());
    }
    if nom > 0 || max > 0 || min > 0 {
        let cbr = max > 0 && max == min && (nom == 0 || nom == max);
        s.set("BitRate_Mode", if cbr { "CBR" } else { "VBR" });
    }
    s.set_if_empty("Compression_Mode", "Lossy");
    true
}

// ---------------------------------------------------------------------------- setup

/// LSB-first bit reader (Vorbis packs bits starting at the least significant bit of each byte).
struct LsbReader<'a> {
    d: &'a [u8],
    pos: usize,
}

impl LsbReader<'_> {
    fn bits(&mut self, n: u32) -> Option<u64> {
        if n > 64 || self.pos.checked_add(n as usize)? > self.d.len() * 8 {
            return None;
        }
        let mut v = 0u64;
        for i in 0..n {
            let p = self.pos + i as usize;
            let bit = (self.d[p / 8] >> (p % 8)) & 1;
            v |= (bit as u64) << i;
        }
        self.pos += n as usize;
        Some(v)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        let end = self.pos.checked_add(n)?;
        if end > self.d.len() * 8 {
            return None;
        }
        self.pos = end;
        Some(())
    }
}

/// Number of bits needed to represent `x` (Vorbis `ilog`).
fn ilog(x: u32) -> u32 {
    32 - x.leading_zeros()
}

/// Largest integer `r` with `r^dim <= entries`.
fn lookup1_values(entries: u32, dim: u32) -> u64 {
    if dim == 0 {
        return 0;
    }
    let pow = |r: u64| -> Option<u64> {
        let mut p = 1u64;
        for _ in 0..dim {
            p = p.checked_mul(r)?;
        }
        Some(p)
    };
    let mut r = (entries as f64).powf(1.0 / dim as f64).floor() as u64;
    while pow(r + 1).is_some_and(|p| p <= entries as u64) {
        r += 1;
    }
    while r > 0 && !pow(r).is_some_and(|p| p <= entries as u64) {
        r -= 1;
    }
    r
}

fn skip_codebook(r: &mut LsbReader) -> Option<()> {
    if r.bits(24)? != 0x564342 {
        return None;
    }
    let dim = r.bits(16)? as u32;
    let entries = r.bits(24)? as u32;
    let ordered = r.bits(1)? == 1;
    if !ordered {
        let sparse = r.bits(1)? == 1;
        for _ in 0..entries {
            if sparse {
                if r.bits(1)? == 1 {
                    r.bits(5)?;
                }
            } else {
                r.bits(5)?;
            }
        }
    } else {
        r.bits(5)?; // current length
        let mut current = 0u32;
        let mut guard = 0u32;
        while current < entries {
            let number = r.bits(ilog(entries - current))? as u32;
            current = current.checked_add(number)?;
            guard += 1;
            if guard > entries.saturating_add(1) {
                return None;
            }
        }
        if current > entries {
            return None;
        }
    }
    match r.bits(4)? {
        0 => {}
        t @ (1 | 2) => {
            r.bits(32)?; // min
            r.bits(32)?; // delta
            let value_bits = r.bits(4)? as usize + 1;
            r.bits(1)?; // sequence_p
            let values = if t == 1 { lookup1_values(entries, dim) } else { entries as u64 * dim as u64 };
            r.skip(values.checked_mul(value_bits as u64)? as usize)?;
        }
        _ => return None,
    }
    Some(())
}

/// First floor type of a `\x05vorbis` setup packet (0 or 1), if the codebooks can be walked.
pub fn setup_floor_type(packet: &[u8]) -> Option<u32> {
    let body = packet.strip_prefix(b"\x05vorbis")?;
    let mut r = LsbReader { d: body, pos: 0 };
    let codebooks = r.bits(8)? + 1;
    for _ in 0..codebooks {
        skip_codebook(&mut r)?;
    }
    let times = r.bits(6)? + 1;
    for _ in 0..times {
        if r.bits(16)? != 0 {
            return None;
        }
    }
    let _floors = r.bits(6)? + 1;
    let t = r.bits(16)? as u32;
    if t > 1 {
        return None;
    }
    Some(t)
}

/// Apply the setup header (floor type) to a stream.
pub fn apply_setup(s: &mut Stream, packet: &[u8]) -> bool {
    match setup_floor_type(packet) {
        Some(t) => {
            s.set("Format_Settings_Floor", t.to_string());
            true
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------- Matroska

/// Split Xiph-laced CodecPrivate (count byte, lacing sizes, packets; the last packet takes the rest).
pub fn split_xiph_lacing(d: &[u8]) -> Option<Vec<&[u8]>> {
    let count = *d.first()? as usize + 1;
    if count > 64 {
        return None;
    }
    let mut pos = 1;
    let mut sizes = Vec::with_capacity(count);
    for _ in 0..count - 1 {
        let mut size = 0usize;
        loop {
            let b = *d.get(pos)? as usize;
            pos += 1;
            size += b;
            if b != 255 {
                break;
            }
        }
        sizes.push(size);
    }
    let mut out = Vec::with_capacity(count);
    for size in sizes {
        out.push(d.get(pos..pos + size)?);
        pos += size;
    }
    out.push(d.get(pos..)?);
    Some(out)
}

/// Matroska `A_VORBIS` CodecPrivate: identification, comment and setup headers.
pub fn apply_xiph_private(s: &mut Stream, d: &[u8]) -> bool {
    let Some(packets) = split_xiph_lacing(d) else { return false };
    let mut ok = false;
    for p in packets {
        match p.first() {
            Some(1) => ok |= apply_ident(s, p),
            Some(3) => {
                if let Some(body) = p.strip_prefix(b"\x03vorbis") {
                    apply_comments_to_stream(s, body);
                }
            }
            Some(5) => {
                apply_setup(s, p);
            }
            _ => {}
        }
    }
    ok
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::StreamKind;

    pub(crate) fn ident_packet(ch: u8, rate: u32, max: i32, nom: i32, min: i32) -> Vec<u8> {
        let mut v = b"\x01vorbis".to_vec();
        v.extend_from_slice(&0u32.to_le_bytes());
        v.push(ch);
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&max.to_le_bytes());
        v.extend_from_slice(&nom.to_le_bytes());
        v.extend_from_slice(&min.to_le_bytes());
        v.push(0xB8);
        v.push(1);
        v
    }

    pub(crate) fn comment_block(vendor: &str, items: &[&str]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        v.extend_from_slice(vendor.as_bytes());
        v.extend_from_slice(&(items.len() as u32).to_le_bytes());
        for i in items {
            v.extend_from_slice(&(i.len() as u32).to_le_bytes());
            v.extend_from_slice(i.as_bytes());
        }
        v
    }

    #[test]
    fn ident() {
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_ident(&mut s, &ident_packet(1, 48000, 0, 80000, 0)));
        assert_eq!(s.get("Channel(s)"), "1");
        assert_eq!(s.get("SamplingRate"), "48000");
        assert_eq!(s.get("BitRate"), "80000");
        assert_eq!(s.get("BitRate_Mode"), "VBR");
        assert!(!s.has("BitRate_Maximum"));
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_ident(&mut s, &ident_packet(2, 44100, 128000, 128000, 128000)));
        assert_eq!(s.get("BitRate_Mode"), "CBR");
        assert_eq!(s.get("BitRate_Maximum"), "128000");
        assert!(!apply_ident(&mut s, b"\x01vorbis\x00\x00"));
        assert!(!apply_ident(&mut s, &ident_packet(0, 48000, 0, 0, 0)));
    }

    #[test]
    fn comments_to_general() {
        let block = comment_block("Lavf63.1.101", &["encoder=Lavc63.1.101 libvorbis", "TITLE=Song", "tracknumber=3", "FOO=bar", "REPLAYGAIN_TRACK_GAIN=-6.5 dB"]);
        let mut s = Stream::new(StreamKind::Audio);
        let mut g = Stream::new(StreamKind::General);
        assert!(apply_comments(&mut s, &mut g, &block));
        assert_eq!(s.get("Encoded_Library"), "Lavf63.1.101");
        assert_eq!(g.get("Encoded_Application"), "Lavc63.1.101 libvorbis");
        assert_eq!(g.get("Title"), "Song");
        assert_eq!(g.get("Track"), "Song");
        assert_eq!(g.get("Track/Position"), "3");
        assert_eq!(g.get("FOO"), "bar");
        assert_eq!(g.get("ReplayGain_Gain"), "-6.5");
        let mut packet = b"\x03vorbis".to_vec();
        packet.extend_from_slice(&block);
        let mut s2 = Stream::new(StreamKind::Audio);
        assert!(apply_comment_packet(&mut s2, &mut g, &packet));
        assert!(!apply_comment_packet(&mut s2, &mut g, &block));
        assert!(parse_comments(&[1, 2]).is_none());
        // truncated item list keeps what was readable
        let c = parse_comments(&block[..block.len() - 3]).unwrap();
        assert_eq!(c.items.len(), 4);
    }

    #[test]
    fn vendor_split() {
        let mut s = Stream::new(StreamKind::Audio);
        apply_vendor(&mut s, "Xiph.Org libVorbis I 20200704 (Reducing Environment)");
        assert_eq!(s.get("Encoded_Library_Name"), "libVorbis");
        assert_eq!(s.get("Encoded_Library_Version"), "(Reducing Environment)");
        assert_eq!(s.get("Encoded_Library_Date"), "20200704 (Reducing Environment)");
        apply_vendor(&mut s, "Lavf63.1.101");
        assert!(!s.has("Encoded_Library_Name"));
    }

    #[test]
    fn xiph_private() {
        let ident = ident_packet(2, 44100, 0, 0, 0);
        let mut comment = b"\x03vorbis".to_vec();
        comment.extend_from_slice(&comment_block("vendor", &["encoder=x"]));
        let setup = b"\x05vorbis".to_vec();
        let mut d = vec![2, ident.len() as u8, comment.len() as u8];
        d.extend_from_slice(&ident);
        d.extend_from_slice(&comment);
        d.extend_from_slice(&setup);
        let p = split_xiph_lacing(&d).unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p[2], &setup[..]);
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_xiph_private(&mut s, &d));
        assert_eq!(s.get("Channel(s)"), "2");
        assert_eq!(s.get("Encoded_Library"), "vendor");
        assert_eq!(s.get("Encoded_Application"), "x");
        assert!(split_xiph_lacing(&[]).is_none());
        assert!(split_xiph_lacing(&[1, 200, 0]).is_none());
        // lacing sizes >= 255 use continuation bytes
        let mut big = vec![1, 255, 1];
        big.extend_from_slice(&[0u8; 256]);
        big.push(9);
        let p = split_xiph_lacing(&big).unwrap();
        assert_eq!(p[0].len(), 256);
        assert_eq!(p[1], &[9]);
    }

    /// Pack (value, width) pairs LSB-first.
    fn pack_lsb(fields: &[(u64, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        let (mut acc, mut n) = (0u64, 0u32);
        for &(v, w) in fields {
            acc |= v << n;
            n += w;
            while n >= 8 {
                out.push((acc & 0xFF) as u8);
                acc >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            out.push(acc as u8);
        }
        out
    }

    #[test]
    fn setup_floor() {
        // One unordered, non-sparse codebook (1 entry, dim 1, no lookup), no time transforms, floor type 1.
        let body = pack_lsb(&[(0, 8), (0x564342, 24), (1, 16), (1, 24), (0, 1), (0, 1), (0, 5), (0, 4), (0, 6), (0, 16), (0, 6), (1, 16)]);
        let mut packet = b"\x05vorbis".to_vec();
        packet.extend_from_slice(&body);
        assert_eq!(setup_floor_type(&packet), Some(1));
        // Ordered codebook with a type-1 lookup: 4 entries of dim 2 → lookup1_values = 2, 2 values × 3 bits.
        let body = pack_lsb(&[(0, 8), (0x564342, 24), (2, 16), (4, 24), (1, 1), (0, 5), (4, 3), (1, 4), (0, 32), (0, 32), (2, 4), (0, 1), (0, 6), (0, 6), (0, 16), (0, 6), (0, 16)]);
        let mut packet = b"\x05vorbis".to_vec();
        packet.extend_from_slice(&body);
        assert_eq!(setup_floor_type(&packet), Some(0));
        assert_eq!(setup_floor_type(b"\x05vorbis\x00"), None);
        assert_eq!(setup_floor_type(b"\x01vorbis"), None);
        assert_eq!(lookup1_values(8, 3), 2);
        assert_eq!(lookup1_values(9, 2), 3);
        assert_eq!(lookup1_values(9, 0), 0);
        assert_eq!(ilog(0), 0);
        assert_eq!(ilog(7), 3);
    }
}

//! MPEG Audio (MP1/MP2/MP3): frame headers (ISO/IEC 11172-3, 13818-3), ID3v1/ID3v2/APEv2 tags,
//! Xing/Info/VBRI headers and the LAME tag.

use crate::io::{be16, be32, latin1, le32, utf16, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::Probe;

// ---------------------------------------------------------------------------- frame header

/// Decoded MPEG audio frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// 1 = MPEG-1, 2 = MPEG-2, 3 = MPEG-2.5.
    pub version: u8,
    /// 1, 2 or 3.
    pub layer: u8,
    pub crc: bool,
    /// Bits per second (0 = free format).
    pub bitrate: u32,
    pub sampling_rate: u32,
    pub padding: bool,
    /// 0 stereo, 1 joint stereo, 2 dual channel, 3 single channel.
    pub mode: u8,
    pub mode_extension: u8,
    pub copyright: bool,
    pub original: bool,
    pub emphasis: u8,
}

const BITRATES_V1: [[u32; 15]; 3] = [
    [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
    [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
];
const BITRATES_V2: [[u32; 15]; 3] = [
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
];
const SAMPLING_RATES: [[u32; 3]; 3] = [[44100, 48000, 32000], [22050, 24000, 16000], [11025, 12000, 8000]];

impl FrameHeader {
    pub fn channels(&self) -> u32 {
        if self.mode == 3 { 1 } else { 2 }
    }

    pub fn samples_per_frame(&self) -> u32 {
        match self.layer {
            1 => 384,
            2 => 1152,
            _ => if self.version == 1 { 1152 } else { 576 },
        }
    }

    /// Frame length in bytes including the header (0 for free format).
    pub fn frame_size(&self) -> usize {
        if self.bitrate == 0 {
            return 0;
        }
        let br = self.bitrate as usize;
        let sr = self.sampling_rate as usize;
        let pad = self.padding as usize;
        match self.layer {
            1 => (12 * br / sr + pad) * 4,
            2 => 144 * br / sr + pad,
            _ => if self.version == 1 { 144 * br / sr + pad } else { 72 * br / sr + pad },
        }
    }

    /// Layer III side information length (bytes after the header/CRC).
    pub fn side_info_len(&self) -> usize {
        match (self.version, self.mode) {
            (1, 3) => 17,
            (1, _) => 32,
            (_, 3) => 9,
            _ => 17,
        }
    }
}

/// Parse a frame header at the start of `d`.
pub fn parse_header(d: &[u8]) -> Option<FrameHeader> {
    if d.len() < 4 || d[0] != 0xFF || d[1] & 0xE0 != 0xE0 {
        return None;
    }
    let version = match (d[1] >> 3) & 3 {
        0 => 3,
        2 => 2,
        3 => 1,
        _ => return None,
    };
    let layer = match (d[1] >> 1) & 3 {
        1 => 3,
        2 => 2,
        3 => 1,
        _ => return None,
    };
    let bitrate_index = (d[2] >> 4) as usize;
    let sampling_index = ((d[2] >> 2) & 3) as usize;
    if bitrate_index == 15 || sampling_index == 3 {
        return None;
    }
    let table = if version == 1 { &BITRATES_V1 } else { &BITRATES_V2 };
    let h = FrameHeader {
        version,
        layer,
        crc: d[1] & 1 == 0,
        bitrate: table[layer as usize - 1][bitrate_index] * 1000,
        sampling_rate: SAMPLING_RATES[version as usize - 1][sampling_index],
        padding: d[2] & 2 != 0,
        mode: d[3] >> 6,
        mode_extension: (d[3] >> 4) & 3,
        copyright: d[3] & 8 != 0,
        original: d[3] & 4 != 0,
        emphasis: d[3] & 3,
    };
    if h.emphasis == 2 || h.bitrate == 0 {
        return None; // reserved emphasis / free format not supported
    }
    Some(h)
}

// ---------------------------------------------------------------------------- Xing / VBRI / LAME

/// LAME extension of a Xing/Info header.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LameTag {
    pub encoder: String,
    pub revision: u8,
    pub vbr_method: u8,
    pub lowpass: u32,
    pub encoder_delay: u32,
    pub encoder_padding: u32,
    pub bitrate: u32,
    pub stereo_mode: u8,
    pub noise_shaping: u8,
    pub source_rate: u8,
    pub preset: u16,
}

/// Xing/Info or VBRI header found in the first frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VbrHeader {
    /// "Xing", "Info" or "VBRI".
    pub kind: &'static str,
    pub frames: Option<u32>,
    pub bytes: Option<u32>,
    pub quality: Option<u32>,
    pub lame: Option<LameTag>,
}

fn is_printable(b: u8) -> bool {
    (0x20..0x7F).contains(&b)
}

/// Parse a LAME tag starting at the 9-byte encoder string.
pub fn parse_lame(d: &[u8]) -> Option<LameTag> {
    let enc = d.get(..9)?;
    let end = enc.iter().position(|b| !is_printable(*b)).unwrap_or(9);
    let encoder = String::from_utf8_lossy(&enc[..end]).trim().to_string();
    if encoder.is_empty() {
        return None;
    }
    let mut t = LameTag { encoder, ..Default::default() };
    if d.len() < 36 {
        return Some(t);
    }
    t.revision = d[9] >> 4;
    t.vbr_method = d[9] & 0x0F;
    t.lowpass = d[10] as u32 * 100;
    t.bitrate = d[20] as u32;
    t.encoder_delay = ((d[21] as u32) << 4) | (d[22] as u32 >> 4);
    t.encoder_padding = ((d[22] as u32 & 0x0F) << 8) | d[23] as u32;
    t.noise_shaping = d[24] & 3;
    t.stereo_mode = (d[24] >> 2) & 7;
    t.source_rate = d[24] >> 6;
    t.preset = be16(d, 26)? & 0x7FF;
    Some(t)
}

/// Look for a Xing/Info/VBRI header inside a frame (`d` starts at the frame header).
pub fn parse_vbr_header(h: &FrameHeader, d: &[u8]) -> Option<VbrHeader> {
    let off = 4 + if h.crc { 2 } else { 0 } + if h.layer == 3 { h.side_info_len() } else { 0 };
    if let Some(tag) = d.get(off..off + 4) {
        if tag == b"Xing" || tag == b"Info" {
            let flags = be32(d, off + 4)?;
            let mut p = off + 8;
            let mut v = VbrHeader { kind: if tag == b"Xing" { "Xing" } else { "Info" }, ..Default::default() };
            if flags & 1 != 0 {
                v.frames = be32(d, p);
                p += 4;
            }
            if flags & 2 != 0 {
                v.bytes = be32(d, p);
                p += 4;
            }
            if flags & 4 != 0 {
                p += 100;
            }
            if flags & 8 != 0 {
                v.quality = be32(d, p);
                p += 4;
            }
            v.lame = d.get(p..).and_then(parse_lame);
            return Some(v);
        }
    }
    let off = 4 + 32;
    if d.get(off..off + 4) == Some(b"VBRI") {
        return Some(VbrHeader { kind: "VBRI", frames: be32(d, off + 14), bytes: be32(d, off + 10), quality: be16(d, off + 8).map(u32::from), lame: None });
    }
    None
}

/// LAME writes its version string into the ancillary data of ordinary frames; find it.
pub fn find_ancillary_encoder(frame: &[u8]) -> Option<String> {
    let body = frame.get(4..)?;
    let pos = body.windows(4).position(|w| w == b"LAME")?;
    let rest = &body[pos..];
    let end = rest.iter().position(|b| !is_printable(*b)).unwrap_or(rest.len()).min(64);
    let s = String::from_utf8_lossy(&rest[..end]).trim_end().to_string();
    if s.len() > 4 { Some(s) } else { None }
}

/// `-m j -V 4 -q 2 -lowpass 19.5 --vbr-new` style settings string from a LAME tag.
pub fn lame_settings(t: &LameTag, quality: Option<u32>) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mode = match t.stereo_mode {
        1 => "m",
        2 => "s",
        3 => "d",
        4 => "j",
        5 => "f",
        6 => "a",
        7 => "i",
        _ => "",
    };
    if !mode.is_empty() {
        parts.push(format!("-m {mode}"));
    }
    if let Some(q) = quality {
        if q <= 100 && matches!(t.vbr_method, 3..=6) {
            let v = (100 - q) / 10;
            let qq = (100 - q) % 10;
            parts.push(format!("-V {v}"));
            parts.push(format!("-q {qq}"));
        }
    }
    if t.lowpass > 0 {
        let khz = t.lowpass as f64 / 1000.0;
        let s = format!("{khz:.1}");
        parts.push(format!("-lowpass {}", s.trim_end_matches(".0")));
    }
    match t.vbr_method {
        1 | 8 => {
            if t.bitrate > 0 {
                parts.push(format!("-b {}", t.bitrate));
            }
        }
        2 | 9 => {
            if t.bitrate > 0 {
                parts.push(format!("--abr {}", t.bitrate));
            }
        }
        3 => parts.push("--vbr-old".to_string()),
        4 | 5 => parts.push("--vbr-new".to_string()),
        _ => {}
    }
    parts.join(" ")
}

// ---------------------------------------------------------------------------- stream fields

fn apply_header(s: &mut Stream, h: &FrameHeader) {
    s.set_if_empty("Format", "MPEG Audio");
    s.set(
        "Format_Version",
        match h.version {
            1 => "Version 1",
            2 => "Version 2",
            _ => "Version 2.5",
        },
    );
    s.set("Format_Profile", format!("Layer {}", h.layer));
    match h.mode {
        0 => s.set("Format_Settings_Mode", "Stereo"),
        1 => s.set("Format_Settings_Mode", "Joint stereo"),
        2 => s.set("Format_Settings_Mode", "Dual mono"),
        _ => {}
    }
    if h.mode == 1 && h.layer == 3 {
        let ext = match h.mode_extension {
            1 => "Intensity Stereo",
            2 => "MS Stereo",
            3 => "MS Stereo / Intensity Stereo",
            _ => "",
        };
        if !ext.is_empty() {
            s.set("Format_Settings_ModeExtension", ext);
        }
    }
    match h.emphasis {
        1 => s.set("Format_Settings_Emphasis", "50/15ms"),
        3 => s.set("Format_Settings_Emphasis", "CCITT"),
        _ => {}
    }
    s.set("SamplingRate", h.sampling_rate.to_string());
    s.set("Channel(s)", h.channels().to_string());
    if h.layer != 3 {
        s.set("SamplesPerFrame", h.samples_per_frame().to_string());
    }
    s.set_if_empty("Compression_Mode", "Lossy");
}

/// Apply Xing/Info/VBRI + LAME information (mode, encoder).
fn apply_vbr_header(s: &mut Stream, v: &VbrHeader) {
    let mut mode = "VBR";
    if let Some(l) = &v.lame {
        match l.vbr_method {
            1 | 8 => mode = "CBR",
            _ => {}
        }
        if l.encoder.starts_with("LAME") || l.encoder.starts_with("L3.9") {
            s.set("Encoded_Library", l.encoder.clone());
            let settings = lame_settings(l, v.quality);
            if !settings.is_empty() {
                s.set("Encoded_Library_Settings", settings);
            }
        }
    }
    s.set("BitRate_Mode", mode);
}

/// Fill from one frame (header at offset 0). A Xing/Info frame is recognised as well.
pub fn apply_frame(s: &mut Stream, d: &[u8]) -> bool {
    let Some(h) = parse_header(d) else { return false };
    apply_header(s, &h);
    s.set("BitRate", h.bitrate.to_string());
    s.set_if_empty("BitRate_Mode", "CBR");
    if let Some(v) = parse_vbr_header(&h, d) {
        apply_vbr_header(s, &v);
    } else if let Some(enc) = find_ancillary_encoder(d) {
        s.set("Encoded_Library", enc);
    }
    true
}

// ---------------------------------------------------------------------------- ID3v2

fn syncsafe(d: &[u8], o: usize) -> Option<usize> {
    let b = d.get(o..o + 4)?;
    if (b[0] | b[1] | b[2] | b[3]) & 0x80 != 0 {
        return None;
    }
    Some(((b[0] as usize) << 21) | ((b[1] as usize) << 14) | ((b[2] as usize) << 7) | b[3] as usize)
}

/// Total size of an ID3v2 tag at the start of `d` (0 when absent).
pub fn id3v2_size(d: &[u8]) -> usize {
    if d.len() < 10 || &d[..3] != b"ID3" || d[3] > 4 || d[3] < 2 {
        return 0;
    }
    match syncsafe(d, 6) {
        Some(size) => 10 + size + if d[3] == 4 && d[5] & 0x10 != 0 { 10 } else { 0 },
        None => 0,
    }
}

fn unsync(d: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(d.len());
    let mut i = 0;
    while i < d.len() {
        out.push(d[i]);
        if d[i] == 0xFF && d.get(i + 1) == Some(&0) {
            i += 1;
        }
        i += 1;
    }
    out
}

/// Decode an ID3v2 text payload (leading encoding byte). Multiple NUL-separated strings are
/// joined with " / ".
pub fn id3_text(d: &[u8]) -> String {
    let Some((&enc, body)) = d.split_first() else { return String::new() };
    let mut parts: Vec<String> = Vec::new();
    match enc {
        1 | 2 => {
            let mut be = enc == 2;
            let mut b = body;
            while b.len() >= 2 {
                if b.starts_with(&[0xFF, 0xFE]) {
                    be = false;
                    b = &b[2..];
                } else if b.starts_with(&[0xFE, 0xFF]) {
                    be = true;
                    b = &b[2..];
                }
                let end = b.chunks_exact(2).position(|c| c == [0, 0]).map(|p| p * 2).unwrap_or(b.len());
                parts.push(utf16(&b[..end], be));
                b = b.get(end + 2..).unwrap_or(&[]);
            }
        }
        _ => {
            for piece in body.split(|&c| c == 0) {
                let text = if enc == 3 { String::from_utf8_lossy(piece).into_owned() } else { latin1(piece) };
                parts.push(text);
            }
        }
    }
    let parts: Vec<String> = parts.into_iter().map(|p| crate::io::clean_text(&p)).filter(|p| !p.is_empty()).collect();
    parts.join(" / ")
}

/// Skip a NUL-terminated string in the given encoding; returns the remainder.
fn skip_string(enc: u8, d: &[u8]) -> &[u8] {
    if enc == 1 || enc == 2 {
        let mut i = 0;
        while i + 1 < d.len() {
            if d[i] == 0 && d[i + 1] == 0 {
                return &d[i + 2..];
            }
            i += 2;
        }
        &[]
    } else {
        match d.iter().position(|&c| c == 0) {
            Some(p) => &d[p + 1..],
            None => &[],
        }
    }
}

const GENRES: &[&str] = &[
    "Blues", "Classic Rock", "Country", "Dance", "Disco", "Funk", "Grunge", "Hip-Hop", "Jazz", "Metal", "New Age", "Oldies", "Other", "Pop", "R&B", "Rap", "Reggae", "Rock", "Techno", "Industrial", "Alternative", "Ska", "Death Metal", "Pranks", "Soundtrack", "Euro-Techno", "Ambient", "Trip-Hop", "Vocal", "Jazz+Funk", "Fusion", "Trance", "Classical", "Instrumental", "Acid", "House", "Game", "Sound Clip", "Gospel", "Noise", "AlternRock", "Bass", "Soul", "Punk", "Space", "Meditative", "Instrumental Pop", "Instrumental Rock", "Ethnic", "Gothic", "Darkwave", "Techno-Industrial", "Electronic", "Pop-Folk", "Eurodance", "Dream", "Southern Rock", "Comedy", "Cult", "Gangsta", "Top 40", "Christian Rap", "Pop/Funk", "Jungle", "Native American", "Cabaret", "New Wave", "Psychadelic", "Rave", "Showtunes", "Trailer", "Lo-Fi", "Tribal", "Acid Punk", "Acid Jazz", "Polka", "Retro", "Musical", "Rock & Roll", "Hard Rock", "Folk", "Folk-Rock", "National Folk", "Swing", "Fast Fusion", "Bebob", "Latin", "Revival", "Celtic", "Bluegrass", "Avantgarde", "Gothic Rock", "Progressive Rock", "Psychedelic Rock", "Symphonic Rock", "Slow Rock", "Big Band", "Chorus", "Easy Listening", "Acoustic", "Humour", "Speech", "Chanson", "Opera", "Chamber Music", "Sonata", "Symphony", "Booty Bass", "Primus", "Porn Groove", "Satire", "Slow Jam", "Club", "Tango", "Samba", "Folklore", "Ballad", "Power Ballad", "Rhythmic Soul", "Freestyle", "Duet", "Punk Rock", "Drum Solo", "A capella", "Euro-House", "Dance Hall", "Goa", "Drum & Bass", "Club-House", "Hardcore", "Terror", "Indie", "BritPop", "Negerpunk", "Polsk Punk", "Beat", "Christian Gangsta Rap", "Heavy Metal", "Black Metal", "Crossover", "Contemporary Christian", "Christian Rock", "Merengue", "Salsa", "Thrash Metal", "Anime", "JPop", "Synthpop",
];

/// `(17)`, `17` or `(17)Rock` → `Rock`.
fn genre_name(text: &str) -> String {
    let t = text.trim();
    let num = if let Some(rest) = t.strip_prefix('(') {
        rest.split(')').next().unwrap_or("")
    } else {
        t
    };
    if let Ok(n) = num.parse::<usize>() {
        if let Some(g) = GENRES.get(n) {
            return g.to_string();
        }
    }
    if let Some(p) = t.find(')') {
        if t.starts_with('(') {
            return t[p + 1..].trim().to_string();
        }
    }
    t.to_string()
}

/// Set a `X/Position` and `X/Position_Total` pair from "3/12".
fn set_position(g: &mut Stream, base: &str, text: &str) {
    let (pos, total) = match text.split_once('/') {
        Some((p, t)) => (p.trim(), t.trim()),
        None => (text.trim(), ""),
    };
    if !pos.is_empty() {
        g.set(format!("{base}/Position").as_str(), pos);
    }
    if !total.is_empty() {
        g.set(format!("{base}/Position_Total").as_str(), total);
    }
}

fn cover_type_name(t: u8) -> &'static str {
    match t {
        0 => "Other",
        1 => "32x32 pixels 'file icon' (PNG only)",
        2 => "Other file icon",
        3 => "Cover (front)",
        4 => "Cover (back)",
        5 => "Leaflet page",
        6 => "Media (e.g. label side of CD)",
        7 => "Lead artist/lead performer/soloist",
        8 => "Artist/performer",
        9 => "Conductor",
        10 => "Band/Orchestra",
        11 => "Composer",
        12 => "Lyricist/text writer",
        13 => "Recording Location",
        14 => "During recording",
        15 => "During performance",
        16 => "Movie/video screen capture",
        17 => "A bright coloured fish",
        18 => "Illustration",
        19 => "Band/artist logotype",
        20 => "Publisher/Studio logotype",
        _ => "",
    }
}

/// Apply one ID3v2 frame (4-char id, v2.2 ids are mapped by the caller) to the General stream.
/// Returns the encoder string of TSSE so the caller can give the stream's own value priority.
fn apply_id3_frame(g: &mut Stream, id: &str, data: &[u8], tsse: &mut Option<String>) {
    if data.is_empty() {
        return;
    }
    let text = || id3_text(data);
    match id {
        "TIT2" => {
            let t = text();
            g.set("Title", t.clone());
            g.set("Track", t);
        }
        "TPE1" => g.set("Performer", text()),
        "TALB" => g.set("Album", text()),
        "TRCK" => set_position(g, "Track", &text()),
        "TPOS" => set_position(g, "Part", &text()),
        "TYER" | "TDRC" => g.set_if_empty("Recorded_Date", text()),
        "TDRL" => g.set("Released_Date", text()),
        "TDEN" => g.set("Encoded_Date", text()),
        "TDTG" => g.set("Tagged_Date", text()),
        "TORY" | "TDOR" => g.set("Original/Released_Date", text()),
        "TCON" => g.set("Genre", genre_name(&text())),
        "TCOM" => g.set("Composer", text()),
        "TPE2" => g.set("Album/Performer", text()),
        "TPE3" => g.set("Conductor", text()),
        "TPE4" => g.set("RemixedBy", text()),
        "TPUB" => g.set("Publisher", text()),
        "TCOP" => g.set("Copyright", text()),
        "TENC" => g.set("EncodedBy", text()),
        "TBPM" => g.set("BPM", text()),
        "TSRC" => g.set("ISRC", text()),
        "TLAN" => g.set("Language", text()),
        "TIT1" => g.set("Grouping", text()),
        "TIT3" => g.set("Track_More", text()),
        "TOAL" => g.set("Original/Album", text()),
        "TOPE" => g.set("Original/Performer", text()),
        "TEXT" => g.set("Lyricist", text()),
        "TOLY" => g.set("Original/Lyricist", text()),
        "TMOO" => g.set("Mood", text()),
        "TSOT" => g.set("Track/Sort", text()),
        "TSOP" => g.set("Performer/Sort", text()),
        "TSOA" => g.set("Album/Sort", text()),
        "TOWN" => g.set("Owner", text()),
        "TRSN" => g.set("ServiceName", text()),
        "TRSO" => g.set("ServiceProvider", text()),
        "TMED" => g.set("OriginalSourceMedium", text()),
        "TCMP" => {
            if text() == "1" {
                g.set("Compilation", "Yes");
            }
        }
        "TSSE" => *tsse = Some(text()),
        "COMM" | "USLT" => {
            // encoding, language(3), short description, text
            let enc = data[0];
            let rest = data.get(4..).unwrap_or(&[]);
            let body = skip_string(enc, rest);
            let mut v = vec![enc];
            v.extend_from_slice(body);
            let t = id3_text(&v);
            if !t.is_empty() {
                g.set(if id == "COMM" { "Comment" } else { "Lyrics" }, t);
            }
        }
        "TXXX" => {
            let enc = data[0];
            let rest = data.get(1..).unwrap_or(&[]);
            let mut name = vec![enc];
            let name_end = rest.len() - skip_string(enc, rest).len();
            name.extend_from_slice(&rest[..name_end]);
            let name = id3_text(&name);
            let mut value = vec![enc];
            value.extend_from_slice(skip_string(enc, rest));
            let value = id3_text(&value);
            if !name.is_empty() && !value.is_empty() {
                g.set(&name, value);
            }
        }
        "APIC" => {
            let enc = data[0];
            let rest = data.get(1..).unwrap_or(&[]);
            let mime_end = rest.iter().position(|&c| c == 0).unwrap_or(rest.len());
            let mime = latin1(&rest[..mime_end]);
            let after = rest.get(mime_end + 1..).unwrap_or(&[]);
            let ptype = after.first().copied().unwrap_or(0);
            let desc_bytes = after.get(1..).unwrap_or(&[]);
            let desc_end = desc_bytes.len() - skip_string(enc, desc_bytes).len();
            let mut desc = vec![enc];
            desc.extend_from_slice(&desc_bytes[..desc_end]);
            let desc = id3_text(&desc);
            g.set("Cover", "Yes");
            if !desc.is_empty() {
                g.set("Cover_Description", desc);
            }
            let tn = cover_type_name(ptype);
            if !tn.is_empty() {
                g.set("Cover_Type", tn);
            }
            if !mime.is_empty() {
                g.set("Cover_Mime", mime);
            }
        }
        _ => {}
    }
}

fn map_v22_id(id: &[u8]) -> Option<&'static str> {
    Some(match id {
        b"TT2" => "TIT2",
        b"TP1" => "TPE1",
        b"TAL" => "TALB",
        b"TRK" => "TRCK",
        b"TPA" => "TPOS",
        b"TYE" => "TYER",
        b"TCO" => "TCON",
        b"TCM" => "TCOM",
        b"TP2" => "TPE2",
        b"TP3" => "TPE3",
        b"TP4" => "TPE4",
        b"TPB" => "TPUB",
        b"TCR" => "TCOP",
        b"TEN" => "TENC",
        b"TBP" => "TBPM",
        b"TRC" => "TSRC",
        b"TLA" => "TLAN",
        b"TT1" => "TIT1",
        b"TT3" => "TIT3",
        b"TOT" => "TOAL",
        b"TOA" => "TOPE",
        b"TXT" => "TEXT",
        b"TOL" => "TOLY",
        b"TSS" => "TSSE",
        b"COM" => "COMM",
        b"ULT" => "USLT",
        b"TXX" => "TXXX",
        b"PIC" => "APIC",
        _ => return None,
    })
}

/// Parse an ID3v2 tag at the start of `d` into the General stream. Returns the tag size.
pub fn apply_id3v2(g: &mut Stream, d: &[u8], tsse: &mut Option<String>) -> usize {
    let total = id3v2_size(d);
    if total == 0 {
        return 0;
    }
    let version = d[3];
    let flags = d[5];
    let body_end = (10 + syncsafe(d, 6).unwrap_or(0)).min(d.len());
    let mut body: Vec<u8> = d[10..body_end].to_vec();
    if flags & 0x80 != 0 && version < 4 {
        body = unsync(&body);
    }
    let mut p = 0usize;
    if flags & 0x40 != 0 {
        // extended header
        p = match version {
            3 => be32(&body, 0).map(|s| s as usize + 4).unwrap_or(0),
            4 => syncsafe(&body, 0).unwrap_or(0),
            _ => 0,
        };
    }
    let mut count = 0;
    while count < 10_000 {
        count += 1;
        let (id, size, fflags, hdr) = if version == 2 {
            let Some(idb) = body.get(p..p + 3) else { break };
            if idb[0] == 0 {
                break;
            }
            let size = match body.get(p + 3..p + 6) {
                Some(s) => ((s[0] as usize) << 16) | ((s[1] as usize) << 8) | s[2] as usize,
                None => break,
            };
            (map_v22_id(idb).unwrap_or("").to_string(), size, 0u16, 6)
        } else {
            let Some(idb) = body.get(p..p + 4) else { break };
            if idb[0] == 0 {
                break;
            }
            let size = if version == 4 { syncsafe(&body, p + 4) } else { be32(&body, p + 4).map(|v| v as usize) };
            let Some(size) = size else { break };
            let fflags = be16(&body, p + 8).unwrap_or(0);
            (String::from_utf8_lossy(idb).into_owned(), size, fflags, 10)
        };
        let start = p + hdr;
        let Some(mut data) = body.get(start..start + size) else { break };
        p = start + size;
        if version == 3 && fflags & 0xC0 != 0 {
            continue; // compressed / encrypted
        }
        let owned: Vec<u8>;
        if version == 4 {
            if fflags & 0x0C != 0 {
                continue;
            }
            if fflags & 0x40 != 0 {
                data = data.get(1..).unwrap_or(&[]);
            }
            if fflags & 0x01 != 0 {
                data = data.get(4..).unwrap_or(&[]);
            }
            if fflags & 0x02 != 0 {
                owned = unsync(data);
                data = &owned;
            }
        }
        if !id.is_empty() {
            apply_id3_frame(g, &id, data, tsse);
        }
    }
    total
}

// ---------------------------------------------------------------------------- ID3v1 / APE

/// Parse an ID3v1 tag (128 bytes starting with "TAG"); fills only empty fields.
pub fn apply_id3v1(g: &mut Stream, d: &[u8]) -> bool {
    if d.len() < 128 || &d[..3] != b"TAG" {
        return false;
    }
    let field = |b: &[u8]| crate::io::clean_text(&latin1(b));
    let title = field(&d[3..33]);
    if !title.is_empty() {
        g.set_if_empty("Title", title.clone());
        g.set_if_empty("Track", title);
    }
    let v = field(&d[33..63]);
    if !v.is_empty() {
        g.set_if_empty("Performer", v);
    }
    let v = field(&d[63..93]);
    if !v.is_empty() {
        g.set_if_empty("Album", v);
    }
    let v = field(&d[93..97]);
    if !v.is_empty() {
        g.set_if_empty("Recorded_Date", v);
    }
    let comment = if d[125] == 0 && d[126] != 0 { &d[97..125] } else { &d[97..127] };
    let v = field(comment);
    if !v.is_empty() {
        g.set_if_empty("Comment", v);
    }
    if d[125] == 0 && d[126] != 0 {
        g.set_if_empty("Track/Position", d[126].to_string());
    }
    if let Some(genre) = GENRES.get(d[127] as usize) {
        g.set_if_empty("Genre", *genre);
    }
    true
}

/// APEv2 tag whose footer ends at the end of `d`: returns the total tag size.
pub fn apply_apev2(g: &mut Stream, d: &[u8]) -> usize {
    if d.len() < 32 {
        return 0;
    }
    let footer = &d[d.len() - 32..];
    if &footer[..8] != b"APETAGEX" {
        return 0;
    }
    let size = le32(footer, 12).unwrap_or(0) as usize;
    let items = le32(footer, 16).unwrap_or(0) as usize;
    let flags = le32(footer, 20).unwrap_or(0);
    let has_header = flags & 0x8000_0000 != 0;
    if size < 32 || size > d.len() {
        return 0;
    }
    let total = size + if has_header { 32 } else { 0 };
    let body = &d[d.len() - size..d.len() - 32];
    let mut p = 0;
    for _ in 0..items.min(1000) {
        let Some(len) = le32(body, p).map(|v| v as usize) else { break };
        let item_flags = le32(body, p + 4).unwrap_or(0);
        let key_start = p + 8;
        let Some(key_len) = body.get(key_start..).and_then(|b| b.iter().position(|&c| c == 0)) else { break };
        let key = latin1(&body[key_start..key_start + key_len]);
        let val_start = key_start + key_len + 1;
        let Some(val) = body.get(val_start..val_start + len) else { break };
        p = val_start + len;
        if item_flags & 6 != 0 {
            continue; // binary / locator
        }
        let value = crate::io::clean_text(&String::from_utf8_lossy(val).replace('\0', " / "));
        if value.is_empty() {
            continue;
        }
        let name = match key.to_ascii_lowercase().as_str() {
            "title" => "Title",
            "artist" => "Performer",
            "album" => "Album",
            "year" => "Recorded_Date",
            "track" => "Track/Position",
            "genre" => "Genre",
            "comment" => "Comment",
            "composer" => "Composer",
            "publisher" => "Publisher",
            "copyright" => "Copyright",
            "album artist" => "Album/Performer",
            "disc" | "discnumber" => "Part/Position",
            "isrc" => "ISRC",
            "lyrics" => "Lyrics",
            "encodedby" | "encoded by" => "EncodedBy",
            _ => "",
        };
        if name.is_empty() {
            g.set_if_empty(&key, value);
        } else {
            if name == "Title" {
                g.set_if_empty("Track", value.clone());
            }
            g.set_if_empty(name, value);
        }
    }
    total
}

// ---------------------------------------------------------------------------- elementary stream

/// Find the first frame header at or after `start` that is followed by a second valid header.
fn find_sync(d: &[u8], start: usize, limit: usize) -> Option<(usize, FrameHeader)> {
    let end = d.len().min(start + limit);
    let mut i = start;
    while i + 4 <= end {
        if d[i] == 0xFF && d[i + 1] & 0xE0 == 0xE0 {
            if let Some(h) = parse_header(&d[i..]) {
                let size = h.frame_size();
                let next = i + size;
                if size > 4 && (next + 4 > d.len() || parse_header(&d[next..]).map(|n| n.sampling_rate == h.sampling_rate && n.layer == h.layer).unwrap_or(false)) {
                    return Some((i, h));
                }
            }
        }
        i += 1;
    }
    None
}

pub fn probe(p: &Probe) -> u8 {
    let start = id3v2_size(p.head);
    let ext = p.ext_in(&["mp3", "mp2", "mp1", "mpa", "m2a", "m1a"]);
    if start >= p.head.len() {
        return if ext || start > 0 { 30 } else { 0 };
    }
    // Sync must be at the very start (after the tag) for a confident match.
    let at_start = parse_header(&p.head[start..]).map(|h| {
        let next = start + h.frame_size();
        p.head.get(next..).and_then(parse_header).map(|n| n.sampling_rate == h.sampling_rate).unwrap_or(next >= p.size as usize)
    });
    match (at_start, ext) {
        (Some(true), true) => 95,
        (Some(true), false) => 70,
        (_, true) => {
            if find_sync(p.head, start, 64 * 1024).is_some() { 60 } else { 20 }
        }
        _ => 0,
    }
}

const SCAN: usize = 4 * 1024 * 1024;

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let size = r.len() as usize;
    let data = r.read_vec_at(0, size.min(SCAN));
    let mut tsse = None;
    let id3v2 = apply_id3v2(doc.general(), &data, &mut tsse);
    let Some((first_pos, first)) = find_sync(&data, id3v2, 256 * 1024) else { return false };

    // Trailing tags.
    let mut tail_tags = 0usize;
    let tail_len = size.min(256 * 1024);
    let tail = r.read_vec_at((size - tail_len) as u64, tail_len);
    if tail.len() >= 128 && apply_id3v1(doc.general(), &tail[tail.len() - 128..]) {
        tail_tags += 128;
    }
    // Lyrics3v2 ("LYRICS200" just before ID3v1) is skipped as opaque.
    if tail.len() >= tail_tags + 15 && &tail[tail.len() - tail_tags - 9..tail.len() - tail_tags] == b"LYRICS200" {
        let n = latin1(&tail[tail.len() - tail_tags - 15..tail.len() - tail_tags - 9]).parse::<usize>().unwrap_or(0);
        tail_tags += 15 + n;
    }
    if tail.len() > tail_tags + 32 {
        tail_tags += apply_apev2(doc.general(), &tail[..tail.len() - tail_tags]);
    }
    if size < first_pos + tail_tags {
        return false;
    }
    let audio_end = (size - tail_tags).min(data.len());

    // Walk frames.
    let mut pos = first_pos;
    let mut frames = 0u64;
    let mut bytes = 0u64;
    let mut bitrates_vary = false;
    let mut encoder: Option<String> = None;
    let mut vbr: Option<VbrHeader> = None;
    let mut xing_size = 0usize;
    let mut header = first;
    while pos + 4 <= audio_end {
        let Some(h) = parse_header(&data[pos..]) else { break };
        let fsize = h.frame_size();
        if fsize < 4 {
            break;
        }
        let frame = &data[pos..(pos + fsize).min(audio_end)];
        if frames == 0 && vbr.is_none() {
            if let Some(v) = parse_vbr_header(&h, frame) {
                xing_size = fsize;
                vbr = Some(v);
                pos += fsize;
                continue;
            }
        }
        if frames == 0 {
            header = h;
        }
        if h.bitrate != header.bitrate {
            bitrates_vary = true;
        }
        if encoder.is_none() {
            encoder = find_ancillary_encoder(frame);
        }
        frames += 1;
        bytes += fsize as u64;
        pos += fsize;
        if frames > 50_000_000 {
            break;
        }
    }
    if frames == 0 && vbr.is_none() {
        return false;
    }
    let scanned_all = audio_end >= size - tail_tags;
    let stream_size = (size - tail_tags).saturating_sub(first_pos + xing_size) as u64;

    let mut s = Stream::new(StreamKind::Audio);
    apply_header(&mut s, &header);
    s.set("SamplesPerFrame", header.samples_per_frame().to_string());
    let spf = header.samples_per_frame() as f64;
    let sr = header.sampling_rate as f64;

    let mut mode = if bitrates_vary { "VBR" } else { "CBR" };
    if let Some(v) = &vbr {
        apply_vbr_header(&mut s, v);
        if s.get("BitRate_Mode") == "VBR" || bitrates_vary {
            mode = "VBR";
        }
    }
    s.set("BitRate_Mode", mode);

    // Frame count: Xing/VBRI when present, else counted (extrapolated for long files).
    let total_frames: f64 = match vbr.as_ref().and_then(|v| v.frames).filter(|f| *f > 0) {
        Some(f) => f as f64,
        None if scanned_all || bytes == 0 => frames as f64,
        None => frames as f64 * stream_size as f64 / bytes as f64,
    };
    let dur_s = total_frames * spf / sr;
    let duration_ms = (dur_s * 1000.0).floor();
    let bitrate: u64 = if mode == "CBR" || !bitrates_vary {
        header.bitrate as u64
    } else if dur_s > 0.0 {
        (stream_size as f64 * 8.0 / dur_s).floor() as u64
    } else {
        header.bitrate as u64
    };
    s.set("BitRate", bitrate.to_string());
    if dur_s > 0.0 {
        s.set("Duration", format!("{duration_ms}"));
        s.set("FrameCount", format!("{}", total_frames.round() as u64));
        s.set("SamplingCount", format!("{}", (total_frames * spf).round() as u64));
    }
    s.set("StreamSize", stream_size.to_string());
    match encoder {
        Some(enc) if !s.get("Encoded_Library").starts_with("LAME") => s.set("Encoded_Library", enc),
        _ => {}
    }
    if !s.has("Encoded_Library") {
        if let Some(l) = vbr.as_ref().and_then(|v| v.lame.as_ref()) {
            s.set("Encoded_Library", l.encoder.clone());
        }
    }

    let overall: u64 = if mode == "CBR" {
        header.bitrate as u64
    } else if dur_s > 0.0 {
        (stream_size as f64 * 8.0 / dur_s).floor() as u64
    } else {
        bitrate
    };
    let g = doc.general();
    g.set("Format", "MPEG Audio");
    if dur_s > 0.0 {
        g.set("Duration", format!("{duration_ms}"));
        g.set("OverallBitRate", overall.to_string());
    }
    g.set("OverallBitRate_Mode", mode);
    g.set("StreamSize", (id3v2 + tail_tags).to_string());
    if s.has("Encoded_Library") {
        g.set("Encoded_Library", s.get("Encoded_Library").to_string());
    } else if let Some(t) = tsse {
        g.set("Encoded_Library", t);
    }
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_fields() {
        // MPEG-1 Layer III, 64 kb/s, 44100 Hz, no padding, single channel.
        let h = parse_header(&[0xFF, 0xFB, 0x50, 0xC0]).unwrap();
        assert_eq!((h.version, h.layer, h.bitrate, h.sampling_rate), (1, 3, 64000, 44100));
        assert_eq!(h.mode, 3);
        assert_eq!(h.channels(), 1);
        assert_eq!(h.frame_size(), 208);
        assert_eq!(h.samples_per_frame(), 1152);
        assert_eq!(h.side_info_len(), 17);
        // MPEG-1 Layer II 64 kb/s 48000 (mp2 fixture header).
        let h = parse_header(&[0xFF, 0xFD, 0x44, 0xC4]).unwrap();
        assert_eq!((h.layer, h.bitrate, h.sampling_rate), (2, 64000, 48000));
        assert_eq!(h.frame_size(), 192);
        // MPEG-2 Layer III 32 kb/s 22050 joint stereo with padding.
        let h = parse_header(&[0xFF, 0xF3, 0x42, 0x40]).unwrap();
        assert_eq!((h.version, h.bitrate, h.sampling_rate, h.mode), (2, 32000, 22050, 1));
        assert_eq!(h.samples_per_frame(), 576);
        assert_eq!(h.frame_size(), 72 * 32000 / 22050 + 1);
        assert!(parse_header(&[0xFF, 0xFB, 0xF0, 0xC0]).is_none()); // bad bitrate index
        assert!(parse_header(&[0xFF, 0xFB, 0x5C, 0xC0]).is_none()); // reserved sampling rate
        assert!(parse_header(&[0xFF, 0xEB, 0x50, 0xC0]).is_none()); // reserved version
        assert!(parse_header(&[0xFF, 0xFB]).is_none());
    }

    #[test]
    fn xing_and_lame() {
        let h = parse_header(&[0xFF, 0xFB, 0x50, 0xC0]).unwrap();
        let mut frame = vec![0xFF, 0xFB, 0x50, 0xC0];
        frame.extend_from_slice(&[0u8; 17]);
        frame.extend_from_slice(b"Xing");
        frame.extend_from_slice(&[0, 0, 0, 0x0B]); // frames + bytes + quality
        frame.extend_from_slice(&[0, 0, 0, 40]);
        frame.extend_from_slice(&[0, 0, 0x12, 0xB2]);
        frame.extend_from_slice(&[0, 0, 0, 78]);
        frame.extend_from_slice(b"LAME3.100");
        let mut rest = vec![0x34u8; 1]; // revision 3, vbr method 4 (mtrh)
        rest.push(195); // lowpass 19500
        rest.extend_from_slice(&[0u8; 9]); // replay gain + flags
        rest.push(0); // bitrate
        rest.extend_from_slice(&[0x24, 0x05, 0x7C]); // delay 576, padding 1404
        rest.push(0b0001_0000); // stereo mode 4 (joint)
        rest.extend_from_slice(&[0u8; 11]);
        frame.extend_from_slice(&rest);
        frame.resize(208, 0);
        let v = parse_vbr_header(&h, &frame).unwrap();
        assert_eq!(v.kind, "Xing");
        assert_eq!(v.frames, Some(40));
        assert_eq!(v.bytes, Some(0x12B2));
        assert_eq!(v.quality, Some(78));
        let l = v.lame.as_ref().unwrap();
        assert_eq!(l.encoder, "LAME3.100");
        assert_eq!(l.vbr_method, 4);
        assert_eq!(l.lowpass, 19500);
        assert_eq!((l.encoder_delay, l.encoder_padding), (576, 1404));
        assert_eq!(lame_settings(l, v.quality), "-m j -V 2 -q 2 -lowpass 19.5 --vbr-new");
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_frame(&mut s, &frame));
        assert_eq!(s.get("BitRate_Mode"), "VBR");
        assert_eq!(s.get("Encoded_Library"), "LAME3.100");
        assert_eq!(s.get("Format_Version"), "Version 1");
        assert_eq!(s.get("Format_Profile"), "Layer 3");
        assert!(!s.has("SamplesPerFrame"));
    }

    #[test]
    fn ancillary_encoder() {
        let mut frame = vec![0xFF, 0xFB, 0x50, 0xC0];
        frame.extend_from_slice(&[1, 2, 3]);
        frame.extend_from_slice(b"LAME4.0UUUU");
        frame.push(0);
        frame.extend_from_slice(b"xx");
        assert_eq!(find_ancillary_encoder(&frame).as_deref(), Some("LAME4.0UUUU"));
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_frame(&mut s, &frame));
        assert_eq!(s.get("Encoded_Library"), "LAME4.0UUUU");
        assert_eq!(s.get("BitRate_Mode"), "CBR");
        assert_eq!(s.get("BitRate"), "64000");
        assert_eq!(find_ancillary_encoder(&[0xFF, 0xFB, 0x50, 0xC0, b'L', b'A']), None);
    }

    #[test]
    fn id3v2_tags() {
        // v2.3 tag with TIT2 (latin1), TPE1 (UTF-16 with BOM), TRCK, TCON "(17)".
        let mut body = Vec::new();
        let mut frame = |id: &[u8], data: &[u8]| {
            body.extend_from_slice(id);
            body.extend_from_slice(&(data.len() as u32).to_be_bytes());
            body.extend_from_slice(&[0, 0]);
            body.extend_from_slice(data);
        };
        frame(b"TIT2", b"\x00MP3 Title");
        frame(b"TPE1", &[1, 0xFF, 0xFE, b'A', 0, b'r', 0, b't', 0]);
        frame(b"TRCK", b"\x003/12");
        frame(b"TCON", b"\x00(17)");
        frame(b"COMM", b"\x00engdesc\x00Hello");
        frame(b"TXXX", b"\x03replaygain_track_gain\x00-1.00 dB");
        let mut tag = b"ID3\x03\x00\x00".to_vec();
        let size = body.len() + 20;
        tag.extend_from_slice(&[(size >> 21) as u8 & 0x7F, (size >> 14) as u8 & 0x7F, (size >> 7) as u8 & 0x7F, size as u8 & 0x7F]);
        tag.extend_from_slice(&body);
        tag.extend_from_slice(&[0u8; 20]);
        let mut g = Stream::new(StreamKind::General);
        let mut tsse = None;
        assert_eq!(apply_id3v2(&mut g, &tag, &mut tsse), 10 + size);
        assert_eq!(g.get("Title"), "MP3 Title");
        assert_eq!(g.get("Track"), "MP3 Title");
        assert_eq!(g.get("Performer"), "Art");
        assert_eq!(g.get("Track/Position"), "3");
        assert_eq!(g.get("Track/Position_Total"), "12");
        assert_eq!(g.get("Genre"), "Rock");
        assert_eq!(g.get("Comment"), "Hello");
        assert_eq!(g.get("replaygain_track_gain"), "-1.00 dB");
        assert_eq!(id3v2_size(b"ID3\x04\x00\x10\x00\x00\x00\x05"), 10 + 5 + 10);
        assert_eq!(id3v2_size(b"ID3\x04\x00\x00\x00\x00\x00\x85"), 0);
        assert_eq!(id3v2_size(b"ID"), 0);
    }

    #[test]
    fn id3v2_v22_and_v24() {
        // v2.2: TT2 with 3-byte size.
        let mut tag = b"ID3\x02\x00\x00\x00\x00\x00\x0B".to_vec();
        tag.extend_from_slice(b"TT2\x00\x00\x05\x00Song");
        let mut g = Stream::new(StreamKind::General);
        apply_id3v2(&mut g, &tag, &mut None);
        assert_eq!(g.get("Title"), "Song");
        // v2.4: syncsafe frame size, UTF-8, TDRC.
        let mut tag = b"ID3\x04\x00\x00\x00\x00\x00\x0F".to_vec();
        tag.extend_from_slice(b"TDRC\x00\x00\x00\x05\x00\x00\x032024");
        let mut g = Stream::new(StreamKind::General);
        apply_id3v2(&mut g, &tag, &mut None);
        assert_eq!(g.get("Recorded_Date"), "2024");
    }

    #[test]
    fn id3v1_and_ape() {
        let mut tag = b"TAG".to_vec();
        tag.extend_from_slice(format!("{:<30}", "Title v1").as_bytes());
        tag.extend_from_slice(format!("{:<30}", "Artist v1").as_bytes());
        tag.extend_from_slice(format!("{:<30}", "Album v1").as_bytes());
        tag.extend_from_slice(b"1999");
        tag.extend_from_slice(format!("{:<28}", "Comment").as_bytes());
        tag.extend_from_slice(&[0, 7, 8]);
        assert_eq!(tag.len(), 128);
        let mut g = Stream::new(StreamKind::General);
        assert!(apply_id3v1(&mut g, &tag));
        assert_eq!(g.get("Title"), "Title v1");
        assert_eq!(g.get("Performer"), "Artist v1");
        assert_eq!(g.get("Recorded_Date"), "1999");
        assert_eq!(g.get("Comment"), "Comment");
        assert_eq!(g.get("Track/Position"), "7");
        assert_eq!(g.get("Genre"), "Jazz");
        assert!(!apply_id3v1(&mut g, &tag[..100]));

        // APEv2: one item "Composer" = "Bach", footer only.
        let mut items = Vec::new();
        items.extend_from_slice(&4u32.to_le_bytes());
        items.extend_from_slice(&0u32.to_le_bytes());
        items.extend_from_slice(b"Composer\x00Bach");
        let mut footer = b"APETAGEX".to_vec();
        footer.extend_from_slice(&2000u32.to_le_bytes());
        footer.extend_from_slice(&((items.len() + 32) as u32).to_le_bytes());
        footer.extend_from_slice(&1u32.to_le_bytes());
        footer.extend_from_slice(&0u32.to_le_bytes());
        footer.extend_from_slice(&[0u8; 8]);
        let mut d = b"xxxx".to_vec();
        d.extend_from_slice(&items);
        d.extend_from_slice(&footer);
        let mut g = Stream::new(StreamKind::General);
        assert_eq!(apply_apev2(&mut g, &d), items.len() + 32);
        assert_eq!(g.get("Composer"), "Bach");
    }

    #[test]
    fn genre_names() {
        assert_eq!(genre_name("(17)"), "Rock");
        assert_eq!(genre_name("17"), "Rock");
        assert_eq!(genre_name("(255)Custom"), "Custom");
        assert_eq!(genre_name("Jazz"), "Jazz");
    }

    #[test]
    fn probe_and_parse_cbr() {
        // Three MPEG-1 Layer II frames (192 bytes each) without tags.
        let mut d = Vec::new();
        for _ in 0..3 {
            d.extend_from_slice(&[0xFF, 0xFD, 0x44, 0xC4]);
            d.extend_from_slice(&[0u8; 188]);
        }
        let p = Probe { head: &d, ext: "mp2", size: d.len() as u64 };
        assert_eq!(probe(&p), 95);
        let p = Probe { head: b"\0\0\0\0", ext: "txt", size: 4 };
        assert_eq!(probe(&p), 0);
        let mut r = Reader::from_bytes(d);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let s = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(s.get("Format_Profile"), "Layer 2");
        assert_eq!(s.get("BitRate_Mode"), "CBR");
        assert_eq!(s.get("BitRate"), "64000");
        assert_eq!(s.get("Duration"), "72"); // 3 * 1152 / 48000 = 72 ms
        assert_eq!(s.get("FrameCount"), "3");
        assert_eq!(s.get("StreamSize"), "576");
        assert_eq!(doc.general_ref().get("OverallBitRate"), "64000");
        assert_eq!(doc.general_ref().get("StreamSize"), "0");
    }
}

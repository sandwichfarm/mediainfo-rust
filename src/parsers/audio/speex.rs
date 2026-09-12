//! Speex: the 80-byte stream header (`speex_header` from the Speex manual).

use crate::io::{cstr, le32};
use crate::model::Stream;

/// Decoded Speex header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Header {
    pub version: String,
    pub version_id: u32,
    pub rate: u32,
    /// 0 narrowband, 1 wideband, 2 ultra-wideband.
    pub mode: u32,
    pub mode_bitstream_version: u32,
    pub channels: u32,
    pub bitrate: i32,
    pub frame_size: u32,
    pub vbr: bool,
    pub frames_per_packet: u32,
    pub extra_headers: u32,
}

/// Parse a Speex header (`Speex   ` magic, version string, then little-endian 32-bit fields).
pub fn parse_header(d: &[u8]) -> Option<Header> {
    let b = d.strip_prefix(b"Speex   ")?;
    if b.len() < 72 {
        return None;
    }
    let f = |i: usize| le32(b, 20 + 4 * i);
    let h = Header {
        version: cstr(&b[..20]).trim().to_string(),
        version_id: f(0)?,
        rate: f(2)?,
        mode: f(3)?,
        mode_bitstream_version: f(4)?,
        channels: f(5)?,
        bitrate: f(6)? as i32,
        frame_size: f(7)?,
        vbr: f(8)? != 0,
        frames_per_packet: f(9)?,
        extra_headers: f(10)?,
    };
    if h.rate == 0 || h.channels == 0 || h.channels > 2 {
        return None;
    }
    Some(h)
}

/// Fill a stream from the Speex header.
pub fn apply_header(s: &mut Stream, d: &[u8]) -> bool {
    let Some(h) = parse_header(d) else { return false };
    s.set_if_empty("Format", "Speex");
    s.set("SamplingRate", h.rate.to_string());
    s.set("Channel(s)", h.channels.to_string());
    s.set("BitRate_Mode", if h.vbr { "VBR" } else { "CBR" });
    if h.bitrate > 0 {
        s.set("BitRate", h.bitrate.to_string());
    }
    if !h.version.is_empty() {
        s.set("Encoded_Library", &h.version);
    }
    s.set_if_empty("Compression_Mode", "Lossy");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;

    fn header(rate: u32, mode: u32, channels: u32, bitrate: i32, vbr: u32) -> Vec<u8> {
        let mut v = b"Speex   ".to_vec();
        let mut ver = b"1.2.1".to_vec();
        ver.resize(20, 0);
        v.extend_from_slice(&ver);
        for x in [1u32, 80, rate, mode, 4, channels, bitrate as u32, 640, vbr, 1, 0, 0, 0] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }

    #[test]
    fn speex() {
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_header(&mut s, &header(32000, 2, 1, 29600, 0)));
        assert_eq!(s.get("Format"), "Speex");
        assert_eq!(s.get("SamplingRate"), "32000");
        assert_eq!(s.get("Channel(s)"), "1");
        assert_eq!(s.get("BitRate"), "29600");
        assert_eq!(s.get("BitRate_Mode"), "CBR");
        assert_eq!(s.get("Encoded_Library"), "1.2.1");
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_header(&mut s, &header(8000, 0, 2, -1, 1)));
        assert_eq!(s.get("BitRate_Mode"), "VBR");
        assert!(!s.has("BitRate"));
        assert!(!apply_header(&mut s, &header(8000, 0, 2, 0, 1)[..60]));
        assert!(!apply_header(&mut s, &header(0, 0, 1, 0, 0)));
        assert!(!apply_header(&mut s, b"OpusHead"));
    }
}

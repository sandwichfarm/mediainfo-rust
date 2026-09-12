//! Apple Lossless: the `ALACSpecificConfig` magic cookie (Apple ALAC file format description),
//! optionally wrapped in `frma`/`alac` atoms as found in MP4 sample entries and some Matroska muxers.

use crate::io::{be16, be32};
use crate::model::Stream;

/// Decoded ALACSpecificConfig (24 bytes, big-endian).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub frame_length: u32,
    pub compatible_version: u8,
    pub bit_depth: u8,
    pub pb: u8,
    pub mb: u8,
    pub kb: u8,
    pub channels: u8,
    pub max_run: u16,
    pub max_frame_bytes: u32,
    pub avg_bit_rate: u32,
    pub sample_rate: u32,
}

/// Locate the 24-byte config inside an optional atom wrapper.
fn config_bytes(d: &[u8]) -> Option<&[u8]> {
    let mut b = d;
    // `frma` atom (12 bytes: size, 'frma', 'alac') then `alac` atom (size, 'alac', version/flags).
    for _ in 0..2 {
        if b.len() >= 12 && (&b[4..8] == b"frma" || &b[4..8] == b"alac") {
            b = &b[12..];
        }
    }
    // Box payload form: version/flags (4 bytes) then the 24-byte cookie.
    if b.len() >= 28 && b[..4] == [0, 0, 0, 0] {
        b = &b[4..];
    }
    if b.len() < 24 {
        return None;
    }
    Some(&b[..24])
}

/// Parse the magic cookie, tolerating the atom wrapper.
pub fn parse_cookie(d: &[u8]) -> Option<Config> {
    let b = config_bytes(d)?;
    let c = Config {
        frame_length: be32(b, 0)?,
        compatible_version: b[4],
        bit_depth: b[5],
        pb: b[6],
        mb: b[7],
        kb: b[8],
        channels: b[9],
        max_run: be16(b, 10)?,
        max_frame_bytes: be32(b, 12)?,
        avg_bit_rate: be32(b, 16)?,
        sample_rate: be32(b, 20)?,
    };
    if !matches!(c.bit_depth, 16 | 20 | 24 | 32) || c.channels == 0 || c.channels > 8 || c.sample_rate == 0 || c.sample_rate > 1_000_000 {
        return None;
    }
    Some(c)
}

/// Fill a stream from the magic cookie (`avgBitRate` is the uncompressed nominal rate in practice).
pub fn apply_cookie(s: &mut Stream, d: &[u8]) -> bool {
    let Some(c) = parse_cookie(d) else { return false };
    s.set_if_empty("Format", "ALAC");
    s.set("BitDepth", c.bit_depth.to_string());
    s.set("Channel(s)", c.channels.to_string());
    s.set("SamplingRate", c.sample_rate.to_string());
    s.set("BitRate_Mode", "VBR");
    if c.avg_bit_rate > 0 {
        s.set("BitRate_Nominal", c.avg_bit_rate.to_string());
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;

    fn config(depth: u8, channels: u8, rate: u32, avg: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&4096u32.to_be_bytes());
        v.extend_from_slice(&[0, depth, 40, 10, 14, channels]);
        v.extend_from_slice(&255u16.to_be_bytes());
        v.extend_from_slice(&8196u32.to_be_bytes());
        v.extend_from_slice(&avg.to_be_bytes());
        v.extend_from_slice(&rate.to_be_bytes());
        v
    }

    #[test]
    fn cookie() {
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_cookie(&mut s, &config(16, 1, 48000, 768000)));
        assert_eq!(s.get("Format"), "ALAC");
        assert_eq!(s.get("BitDepth"), "16");
        assert_eq!(s.get("Channel(s)"), "1");
        assert_eq!(s.get("SamplingRate"), "48000");
        assert_eq!(s.get("BitRate_Nominal"), "768000");
        assert_eq!(s.get("BitRate_Mode"), "VBR");
        // frma + alac wrapper
        let mut w = Vec::new();
        w.extend_from_slice(&12u32.to_be_bytes());
        w.extend_from_slice(b"frmaalac");
        w.extend_from_slice(&36u32.to_be_bytes());
        w.extend_from_slice(b"alac\0\0\0\0");
        w.extend_from_slice(&config(24, 2, 44100, 0));
        let c = parse_cookie(&w).unwrap();
        assert_eq!(c.bit_depth, 24);
        assert_eq!(c.frame_length, 4096);
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_cookie(&mut s, &w));
        assert!(!s.has("BitRate_Nominal"));
        assert!(!apply_cookie(&mut s, &config(16, 1, 48000, 0)[..20]));
        assert!(!apply_cookie(&mut s, &config(12, 1, 48000, 0)));
        assert!(!apply_cookie(&mut s, &config(16, 0, 48000, 0)));
    }
}

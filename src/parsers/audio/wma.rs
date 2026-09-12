//! WAVEFORMATEX / WAVEFORMATEXTENSIBLE (Microsoft multimedia headers) → Format/CodecID, used by
//! AVI, WAV, Matroska `A_MS/ACM` and ASF. Also the WAVE format tag table (`format_tag_name`).

use crate::io::{le16, le32};
use crate::model::Stream;
use crate::parsers::audio::{layout_from_mask, pcm};

/// Decoded WAVEFORMATEX with the WAVEFORMATEXTENSIBLE additions when present.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WaveFormat {
    pub format_tag: u16,
    pub channels: u16,
    pub samples_per_sec: u32,
    pub avg_bytes_per_sec: u32,
    pub block_align: u16,
    pub bits_per_sample: u16,
    pub cb_size: u16,
    /// Bytes after the 18-byte WAVEFORMATEX header (codec-specific data).
    pub extra: Vec<u8>,
    // WAVEFORMATEXTENSIBLE
    pub valid_bits_per_sample: u16,
    pub channel_mask: u32,
    pub sub_format: Option<[u8; 16]>,
}

/// Suffix of the KSDATAFORMAT_SUBTYPE_* GUIDs whose first 32 bits are a WAVE format tag.
const MS_GUID_TAIL: [u8; 12] = [0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71];

/// Parse a WAVEFORMAT / PCMWAVEFORMAT (16 bytes) / WAVEFORMATEX (18+) / WAVEFORMATEXTENSIBLE.
pub fn parse_waveformatex(d: &[u8]) -> Option<WaveFormat> {
    if d.len() < 14 {
        return None;
    }
    let mut w = WaveFormat {
        format_tag: le16(d, 0)?,
        channels: le16(d, 2)?,
        samples_per_sec: le32(d, 4)?,
        avg_bytes_per_sec: le32(d, 8)?,
        block_align: le16(d, 12)?,
        bits_per_sample: le16(d, 14).unwrap_or(0),
        cb_size: le16(d, 16).unwrap_or(0),
        ..Default::default()
    };
    if d.len() > 18 {
        let n = (w.cb_size as usize).min(d.len() - 18);
        w.extra = d[18..18 + n].to_vec();
    }
    if w.format_tag == 0xFFFE && w.extra.len() >= 22 {
        let e = &w.extra;
        w.valid_bits_per_sample = le16(e, 0)?;
        w.channel_mask = le32(e, 2)?;
        let mut g = [0u8; 16];
        g.copy_from_slice(&e[6..22]);
        w.sub_format = Some(g);
    }
    Some(w)
}

/// Format the 16 GUID bytes the way the reference prints a SubFormat (`00000001-0000-0010-8000-00AA00389B71`).
pub fn guid_string(g: &[u8; 16]) -> String {
    let d1 = u32::from_le_bytes([g[0], g[1], g[2], g[3]]);
    let d2 = u16::from_le_bytes([g[4], g[5]]);
    let d3 = u16::from_le_bytes([g[6], g[7]]);
    format!("{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}", g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15])
}

/// Format name and profile/version for a WAVE format tag: `(Format, Format_Profile-or-Version)`.
/// The second string is a `Format_Version` for WMA (`Version 1`/`Version 2`), a profile otherwise.
pub fn format_tag_name(tag: u16) -> Option<(&'static str, &'static str)> {
    Some(match tag {
        0x0001 => ("PCM", ""),
        0x0002 => ("ADPCM", ""),
        0x0003 => ("PCM", "Float"),
        0x0006 => ("ADPCM", ""),
        0x0007 => ("ADPCM", ""),
        0x000A => ("WMA", "Voice"),
        0x0011 => ("ADPCM", ""),
        0x0022 => ("DSP Group TrueSpeech", ""),
        0x0031 | 0x0032 => ("GSM", ""),
        0x0040 => ("G.721", ""),
        0x0042 => ("G.723", ""),
        0x0045 => ("G.726", ""),
        0x0050 => ("MPEG Audio", ""),
        0x0055 => ("MPEG Audio", "Layer 3"),
        0x0075 => ("Voxware", ""),
        0x00FF => ("AAC", ""),
        0x0130 => ("Sipro", ""),
        0x0160 => ("WMA", "Version 1"),
        0x0161 => ("WMA", "Version 2"),
        0x0162 => ("WMA", "Pro"),
        0x0163 => ("WMA", "Lossless"),
        0x0270 => ("ATRAC3", ""),
        0x1600 | 0x1610 | 0x4143 | 0x706D | 0xA106 => ("AAC", ""),
        0x2000 => ("AC-3", ""),
        0x2001 => ("DTS", ""),
        0x2002 => ("RealAudio 1", ""),
        0x2003 => ("RealAudio 2", ""),
        0x2004 => ("Cook", ""),
        0x566F => ("Vorbis", ""),
        0x674F | 0x6750 | 0x6751 | 0x676F | 0x6770 | 0x6771 => ("Vorbis", ""),
        0xF1AC => ("FLAC", ""),
        0xFFFE => ("Extensible", ""),
        _ => return None,
    })
}

/// Fill a stream from WAVEFORMATEX(TENSIBLE): Format (+ profile/version), CodecID (hex tag or the
/// SubFormat GUID), channels, sampling rate, bit depth, bit rate (nAvgBytesPerSec × 8), channel
/// layout from the channel mask, and the PCM sign/endianness settings.
pub fn apply_waveformatex(s: &mut Stream, d: &[u8]) -> bool {
    let Some(w) = parse_waveformatex(d) else { return false };
    // Effective format tag: for WAVEFORMATEXTENSIBLE the SubFormat GUID's first 32 bits when it is a
    // Microsoft "format tag" GUID.
    let (tag, codec_id) = match w.sub_format {
        Some(g) => {
            let t = if g[4..] == MS_GUID_TAIL { u32::from_le_bytes([g[0], g[1], g[2], g[3]]) } else { 0xFFFF_FFFF };
            (t, guid_string(&g))
        }
        None => (w.format_tag as u32, format!("{:X}", w.format_tag)),
    };
    s.set("CodecID", codec_id);
    let tag16 = if tag <= 0xFFFF { tag as u16 } else { 0 };
    match tag16 {
        0x0001 => {
            pcm::apply_pcm(s, Some(true), Some(w.bits_per_sample > 8), false, w.bits_per_sample as u32);
            s.set("BitRate_Mode", "CBR");
        }
        0x0003 => {
            pcm::apply_pcm(s, Some(true), None, true, w.bits_per_sample as u32);
            s.set("BitRate_Mode", "CBR");
        }
        _ => {
            if let Some((format, extra)) = format_tag_name(tag16) {
                s.set_if_empty("Format", format);
                if !extra.is_empty() {
                    if format == "WMA" && extra.starts_with("Version") {
                        s.set_if_empty("Format_Version", extra);
                    } else {
                        s.set_if_empty("Format_Profile", extra);
                    }
                }
            }
            if w.bits_per_sample > 0 {
                s.set_if_empty("BitDepth", w.bits_per_sample.to_string());
            }
        }
    }
    if w.channels > 0 {
        s.set_if_empty("Channel(s)", w.channels.to_string());
    }
    if w.samples_per_sec > 0 {
        s.set_if_empty("SamplingRate", w.samples_per_sec.to_string());
    }
    if w.avg_bytes_per_sec > 0 {
        s.set_if_empty("BitRate", (w.avg_bytes_per_sec as u64 * 8).to_string());
    }
    if w.channel_mask != 0 {
        let (pos, layout) = layout_from_mask(w.channel_mask);
        if !pos.is_empty() {
            s.set("ChannelPositions", pos);
            s.set("ChannelLayout", layout);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;

    fn wfx(tag: u16, ch: u16, rate: u32, avg: u32, align: u16, bits: u16, extra: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&tag.to_le_bytes());
        v.extend_from_slice(&ch.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&avg.to_le_bytes());
        v.extend_from_slice(&align.to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        v.extend_from_slice(extra);
        v
    }

    fn extensible(valid: u16, mask: u32, tag: u32) -> Vec<u8> {
        let mut e = Vec::new();
        e.extend_from_slice(&valid.to_le_bytes());
        e.extend_from_slice(&mask.to_le_bytes());
        e.extend_from_slice(&tag.to_le_bytes());
        e.extend_from_slice(&MS_GUID_TAIL);
        e
    }

    #[test]
    fn pcm16() {
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_waveformatex(&mut s, &wfx(1, 1, 48000, 96000, 2, 16, &[])));
        assert_eq!(s.get("Format"), "PCM");
        assert_eq!(s.get("CodecID"), "1");
        assert_eq!(s.get("Format_Settings"), "Little / Signed");
        assert_eq!(s.get("BitRate"), "768000");
        assert_eq!(s.get("BitRate_Mode"), "CBR");
        assert_eq!(s.get("BitDepth"), "16");
        assert_eq!(s.get("Channel(s)"), "1");
        assert_eq!(s.get("SamplingRate"), "48000");
        assert!(!s.has("ChannelPositions"));
        // 16-byte PCMWAVEFORMAT without cbSize
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_waveformatex(&mut s, &wfx(1, 2, 44100, 176400, 4, 8, &[])[..16]));
        assert_eq!(s.get("Format_Settings"), "Little / Unsigned");
        assert!(!apply_waveformatex(&mut s, &[1, 0, 2]));
    }

    #[test]
    fn extensible_pcm_and_float() {
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_waveformatex(&mut s, &wfx(0xFFFE, 2, 48000, 288000, 6, 24, &extensible(24, 3, 1))));
        assert_eq!(s.get("CodecID"), "00000001-0000-0010-8000-00AA00389B71");
        assert_eq!(s.get("Format"), "PCM");
        assert_eq!(s.get("Format_Settings"), "Little / Signed");
        assert_eq!(s.get("ChannelPositions"), "Front: L R");
        assert_eq!(s.get("ChannelLayout"), "L R");
        assert_eq!(s.get("BitDepth"), "24");
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_waveformatex(&mut s, &wfx(0xFFFE, 6, 48000, 1152000, 24, 32, &extensible(32, 0x3F, 3))));
        assert_eq!(s.get("CodecID"), "00000003-0000-0010-8000-00AA00389B71");
        assert_eq!(s.get("Format_Profile"), "Float");
        assert!(!s.has("Format_Settings"));
        assert_eq!(s.get("ChannelLayout"), "L R C LFE Lb Rb");
        assert_eq!(s.get("BitRate"), "9216000");
    }

    #[test]
    fn wma_and_others() {
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_waveformatex(&mut s, &wfx(0x161, 1, 48000, 4000, 0xAA, 16, &[0, 0, 0, 0, 1, 0, 0, 0, 0, 0])));
        assert_eq!(s.get("Format"), "WMA");
        assert_eq!(s.get("Format_Version"), "Version 2");
        assert_eq!(s.get("CodecID"), "161");
        assert_eq!(s.get("BitRate"), "32000");
        assert_eq!(s.get("BitDepth"), "16");
        assert!(!s.has("BitRate_Mode"));
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_waveformatex(&mut s, &wfx(0x55, 2, 44100, 16000, 1, 0, &[])));
        assert_eq!(s.get("Format"), "MPEG Audio");
        assert_eq!(s.get("Format_Profile"), "Layer 3");
        assert_eq!(s.get("CodecID"), "55");
        assert!(!s.has("BitDepth"));
        let mut s = Stream::new(StreamKind::Audio);
        assert!(apply_waveformatex(&mut s, &wfx(0x2000, 1, 48000, 8000, 1, 16, &[])));
        assert_eq!(s.get("Format"), "AC-3");
        assert_eq!(s.get("CodecID"), "2000");
        assert_eq!(format_tag_name(0x162), Some(("WMA", "Pro")));
        assert_eq!(format_tag_name(0x1234), None);
        assert_eq!(guid_string(&[0x01, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]), "00000001-0000-0010-8000-00AA00389B71");
    }
}

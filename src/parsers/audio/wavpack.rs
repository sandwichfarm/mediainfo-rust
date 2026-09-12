//! WavPack (`wvpk` block header, WavPack file format description v4/v5).

use crate::io::{le16, le32, Reader};
use crate::model::{Doc, Stream, StreamKind};
use crate::parsers::audio::{ape, layout_for_count, layout_from_mask};
use crate::parsers::Probe;

const SAMPLE_RATES: [u32; 15] = [6000, 8000, 9600, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000, 192000];

/// Decoded block header plus the metadata sub-blocks we care about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Block {
    /// Bytes of the whole block (`ckSize` + 8).
    pub size: u32,
    pub version: u16,
    pub total_samples: Option<u64>,
    pub block_index: u64,
    pub block_samples: u32,
    pub flags: u32,
    pub bytes_per_sample: u32,
    pub mono: bool,
    pub hybrid: bool,
    pub float: bool,
    pub initial: bool,
    pub final_: bool,
    pub sample_rate: u32,
    /// Bits per sample after the left shift / magnitude fields.
    pub bits_per_sample: u32,
    /// From the ID_CHANNEL_INFO sub-block: (channel count, channel mask).
    pub channel_info: Option<(u32, u32)>,
}

/// Parse a block header (32 bytes) and, when the whole block is given, its metadata sub-blocks.
pub fn parse_block(d: &[u8]) -> Option<Block> {
    if !d.starts_with(b"wvpk") || d.len() < 32 {
        return None;
    }
    let ck_size = le32(d, 4)?;
    let version = le16(d, 8)?;
    if !(0x402..=0x410).contains(&version) || ck_size < 24 {
        return None;
    }
    let block_index_u8 = d[10] as u64;
    let total_samples_u8 = d[11] as u64;
    let total_samples_lo = le32(d, 12)?;
    let block_index_lo = le32(d, 16)?;
    let block_samples = le32(d, 20)?;
    let flags = le32(d, 24)?;
    let mut b = Block { size: ck_size + 8, version, block_samples, flags, ..Default::default() };
    b.block_index = (block_index_u8 << 32) | block_index_lo as u64;
    b.total_samples = if total_samples_lo == 0xFFFF_FFFF { None } else { Some((total_samples_u8 << 32) | total_samples_lo as u64) };
    b.bytes_per_sample = (flags & 3) + 1;
    b.mono = flags & 0x4 != 0;
    b.hybrid = flags & 0x8 != 0;
    b.float = flags & 0x80 != 0;
    b.initial = flags & 0x800 != 0;
    b.final_ = flags & 0x1000 != 0;
    let shift = (flags >> 13) & 0x1F;
    b.bits_per_sample = (b.bytes_per_sample * 8).saturating_sub(shift);
    let rate_index = ((flags >> 23) & 0xF) as usize;
    b.sample_rate = SAMPLE_RATES.get(rate_index).copied().unwrap_or(0);
    // Metadata sub-blocks: id byte (bit 5 = odd size, bit 7 = large), size in words.
    let end = (b.size as usize).min(d.len());
    let mut pos = 32;
    let mut guard = 0;
    while pos + 2 <= end && guard < 256 {
        guard += 1;
        let id = d[pos];
        let (words, hdr) = if id & 0x80 != 0 {
            let w = match d.get(pos + 1..pos + 4) {
                Some(w) => (w[0] as usize) | ((w[1] as usize) << 8) | ((w[2] as usize) << 16),
                None => break,
            };
            (w, 4)
        } else {
            (d[pos + 1] as usize, 2)
        };
        let mut len = words * 2;
        if id & 0x40 != 0 {
            len = len.saturating_sub(1);
        }
        let body_start = pos + hdr;
        let Some(body) = d.get(body_start..body_start + len) else { break };
        match id & 0x3F {
            0x27 => {
                // ID_SAMPLE_RATE: 3-byte little-endian rate
                if body.len() >= 3 {
                    b.sample_rate = body[0] as u32 | ((body[1] as u32) << 8) | ((body[2] as u32) << 16);
                }
            }
            0x0D => {
                // ID_CHANNEL_INFO: channel count byte then the mask (1..3 bytes); the 6-byte-plus form
                // carries a wider count ((b0 | (b2 & 0xF) << 8) + 1) and the mask from byte 3.
                if !body.is_empty() {
                    let (count, mask_bytes) = if body.len() >= 6 { ((body[0] as u32 | ((body[2] as u32 & 0xF) << 8)) + 1, &body[3..]) } else { (body[0] as u32, &body[1..]) };
                    let mut mask = 0u32;
                    for (i, &x) in mask_bytes.iter().take(4).enumerate() {
                        mask |= (x as u32) << (8 * i);
                    }
                    if count > 0 {
                        b.channel_info = Some((count, mask));
                    }
                }
            }
            _ => {}
        }
        pos = body_start + words * 2;
    }
    Some(b)
}

/// Fill a stream from the first block.
pub fn apply_block(s: &mut Stream, b: &Block) {
    s.set_if_empty("Format", "WavPack");
    s.set("Format_Profile", format!("{}.{}", b.version >> 8, b.version & 0xFF));
    s.set("Format_Settings", if b.hybrid { "Hybrid" } else { "Lossless" });
    let channels = b.channel_info.map(|(c, _)| c).unwrap_or(if b.mono { 1 } else { 2 });
    s.set("Channel(s)", channels.to_string());
    if let Some((_, mask)) = b.channel_info.filter(|(_, m)| *m != 0) {
        let (pos, layout) = layout_from_mask(mask);
        if !pos.is_empty() {
            s.set("ChannelPositions", pos);
            s.set("ChannelLayout", layout);
        }
    } else if channels > 2 {
        let (pos, layout) = layout_for_count(channels);
        if !pos.is_empty() {
            s.set("ChannelPositions", pos);
            s.set("ChannelLayout", layout);
        }
    }
    if b.sample_rate > 0 {
        s.set("SamplingRate", b.sample_rate.to_string());
    }
    s.set("BitDepth", if b.float { 32 } else { b.bits_per_sample }.to_string());
    s.set("BitRate_Mode", "VBR");
    if let Some(total) = b.total_samples.filter(|t| *t > 0) {
        if b.sample_rate > 0 {
            s.set("SamplingCount", total.to_string());
            s.set("Duration", format!("{}", (total as f64 * 1000.0 / b.sample_rate as f64).round() as u64));
        }
    }
}

pub fn probe(p: &Probe) -> u8 {
    if p.starts_with(b"wvpk") {
        if parse_block(p.head).is_some() {
            100
        } else {
            40
        }
    } else if p.ext_in(&["wv", "wvc"]) {
        20
    } else {
        0
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let head = r.read_vec_at(0, 64 * 1024);
    let Some(first) = parse_block(&head) else { return false };
    let mut s = Stream::new(StreamKind::Audio);
    apply_block(&mut s, &first);
    let tag = ape::read_ape_tag(r);
    let tag_size = tag.as_ref().map(|t| t.size).unwrap_or(0);
    // Unknown total samples: walk the block headers (bounded) to sum block_samples.
    if !s.has("Duration") && first.sample_rate > 0 {
        let end = r.len().saturating_sub(tag_size);
        let mut pos = 0u64;
        let mut samples = 0u64;
        let mut blocks = 0u32;
        while pos + 32 <= end && blocks < 1_000_000 {
            let h = r.read_vec_at(pos, 32);
            let Some(b) = parse_block(&h) else { break };
            if b.initial {
                samples += b.block_samples as u64;
            }
            pos += b.size as u64;
            blocks += 1;
        }
        if samples > 0 && (pos >= end || blocks >= 1_000_000) {
            let scaled = if pos < end { samples as f64 * end as f64 / pos as f64 } else { samples as f64 };
            s.set("Duration", format!("{}", (scaled * 1000.0 / first.sample_rate as f64).round() as u64));
        }
    }
    let g = doc.general();
    g.set("Format", "WavPack");
    if let Some(t) = &tag {
        ape::apply_ape_tag(g, t);
    }
    let audio_size = r.len().saturating_sub(tag_size);
    s.set("StreamSize", audio_size.to_string());
    if let Some(ms) = s.get_f64("Duration") {
        if ms > 0.0 {
            let br = (audio_size as f64 * 8.0 * 1000.0 / ms).round();
            s.set("BitRate", format!("{}", br as u64));
            let ch = s.get_u64("Channel(s)").unwrap_or(0);
            let bd = s.get_u64("BitDepth").unwrap_or(0);
            if let Some(cr) = ape::compression_ratio(first.sample_rate as u64, bd, ch, br) {
                s.set("Compression_Ratio", cr);
            }
            g.set("Duration", format!("{}", ms.round() as u64));
        }
    }
    g.set("StreamSize", tag_size.to_string());
    doc.streams[StreamKind::Audio as usize].push(s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(total: u32, index: u32, samples: u32, flags: u32, meta: &[u8]) -> Vec<u8> {
        let mut v = b"wvpk".to_vec();
        v.extend_from_slice(&((24 + meta.len()) as u32).to_le_bytes());
        v.extend_from_slice(&0x410u16.to_le_bytes());
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(&total.to_le_bytes());
        v.extend_from_slice(&index.to_le_bytes());
        v.extend_from_slice(&samples.to_le_bytes());
        v.extend_from_slice(&flags.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(meta);
        v
    }

    #[test]
    fn header() {
        // 16-bit mono 48 kHz, initial+final, max magnitude 15.
        let flags = 0x053C_1805;
        let b = parse_block(&block(48000, 0, 1024, flags, &[])).unwrap();
        assert_eq!(b.version, 0x410);
        assert_eq!(b.total_samples, Some(48000));
        assert_eq!(b.block_samples, 1024);
        assert!(b.mono && b.initial && b.final_ && !b.hybrid && !b.float);
        assert_eq!(b.sample_rate, 48000);
        assert_eq!(b.bits_per_sample, 16);
        let mut s = Stream::new(StreamKind::Audio);
        apply_block(&mut s, &b);
        assert_eq!(s.get("Format"), "WavPack");
        assert_eq!(s.get("Format_Profile"), "4.16");
        assert_eq!(s.get("Format_Settings"), "Lossless");
        assert_eq!(s.get("Channel(s)"), "1");
        assert_eq!(s.get("SamplingRate"), "48000");
        assert_eq!(s.get("BitDepth"), "16");
        assert_eq!(s.get("Duration"), "1000");
        // Non-standard sample rate sub-block (index 15) + channel info for 6 channels, hybrid.
        let meta = [0x27, 2, 0x44, 0xAC, 0x00, 0x00, 0x0D, 2, 6, 0x3F, 0, 0];
        let flags = (15 << 23) | 0x1800 | 0x8 | 2; // 3 bytes/sample
        let b = parse_block(&block(0xFFFF_FFFF, 0, 1024, flags, &meta)).unwrap();
        assert_eq!(b.sample_rate, 44100);
        assert_eq!(b.channel_info, Some((6, 0x3F)));
        assert_eq!(b.total_samples, None);
        let mut s = Stream::new(StreamKind::Audio);
        apply_block(&mut s, &b);
        assert_eq!(s.get("Channel(s)"), "6");
        assert_eq!(s.get("ChannelLayout"), "L R C LFE Lb Rb");
        assert_eq!(s.get("BitDepth"), "24");
        assert_eq!(s.get("Format_Settings"), "Hybrid");
        assert!(!s.has("Duration"));
        assert!(parse_block(b"wvpk\x00\x00").is_none());
        assert!(parse_block(&block(0, 0, 0, 0, &[])[..31]).is_none());
    }

    #[test]
    fn file_with_tag() {
        let flags = 0x053C_1805;
        let mut d = Vec::new();
        for i in 0..4 {
            d.extend_from_slice(&block(4096, i * 1024, 1024, flags, &[0u8; 100]));
        }
        let audio = d.len();
        d.extend_from_slice(&ape::tests::ape_tag(&[("encoder", "Lavf63.1.101")], true));
        assert_eq!(probe(&Probe { head: &d, ext: "wv", size: d.len() as u64 }), 100);
        assert_eq!(probe(&Probe { head: b"wvpk\x00", ext: "wv", size: 5 }), 40);
        assert_eq!(probe(&Probe { head: b"RIFF", ext: "wv", size: 4 }), 20);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(d.clone()), &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "WavPack");
        assert_eq!(g.get("ENCODER"), "Lavf63.1.101");
        assert_eq!(g.get("StreamSize"), (d.len() - audio).to_string());
        assert_eq!(g.get("Duration"), "85");
        let a = doc.stream(StreamKind::Audio, 0).unwrap();
        assert_eq!(a.get("StreamSize"), audio.to_string());
        assert_eq!(a.get("BitRate"), ((audio as f64 * 8.0 * 1000.0 / 85.0).round() as u64).to_string());
        assert!(a.has("Compression_Ratio"));
        // Unknown total samples: summed from the block headers.
        let mut d = Vec::new();
        for i in 0..4 {
            d.extend_from_slice(&block(0xFFFF_FFFF, i * 1024, 1024, flags, &[]));
        }
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(d), &mut doc));
        assert_eq!(doc.stream(StreamKind::Audio, 0).unwrap().get("Duration"), "85");
        assert!(!parse(&mut Reader::from_bytes(b"RIFF".to_vec()), &mut Doc::new()));
    }
}

//! DV frame (IEC 61834-2 DIF structure): header, subcode, VAUX and AAUX packs of one frame.

use crate::model::{Stream, StreamKind};

pub const BLOCK: usize = 80;
pub const SEQUENCE: usize = 150 * BLOCK;

/// Section types (upper 3 bits of the first ID byte).
pub const SCT_HEADER: u8 = 0;
pub const SCT_SUBCODE: u8 = 1;
pub const SCT_VAUX: u8 = 2;
pub const SCT_AUDIO: u8 = 3;
pub const SCT_VIDEO: u8 = 4;

#[derive(Debug, Clone, Default)]
pub struct TimeCode {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    pub drop_frame: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AudioSource {
    pub locked: bool,
    pub channels_per_block: u8,
    pub stype: u8,
    pub sampling_rate: u32,
    pub quantization: u8, // bits
}

#[derive(Debug, Clone, Default)]
pub struct Frame {
    pub dsf: bool, // false = 525/60 (10 sequences), true = 625/50 (12 sequences)
    pub apt: u8,
    pub stype: Option<u8>,   // VS pack signal type
    pub wide: Option<bool>,  // VSC DISP 16:9
    pub interlaced: Option<bool>,
    pub time_code: Option<TimeCode>,
    pub date: Option<(u16, u8, u8)>,
    pub time: Option<(u8, u8, u8)>,
    pub audio: Option<AudioSource>,
}

impl Frame {
    pub fn sequences(&self) -> usize {
        if self.dsf { 12 } else { 10 }
    }
    pub fn size(&self) -> usize {
        self.sequences() * SEQUENCE
    }
    pub fn frame_rate(&self) -> f64 {
        if self.dsf { 25.0 } else { 30000.0 / 1001.0 }
    }
}

fn bcd(b: u8) -> Option<u8> {
    let (hi, lo) = (b >> 4, b & 0xF);
    if hi > 9 || lo > 9 {
        return None;
    }
    Some(hi * 10 + lo)
}

fn time_code(p: &[u8]) -> Option<TimeCode> {
    Some(TimeCode { frames: bcd(p[0] & 0x3F)?, drop_frame: p[0] & 0x40 != 0, seconds: bcd(p[1] & 0x7F)?, minutes: bcd(p[2] & 0x7F)?, hours: bcd(p[3] & 0x3F)? })
}

fn rec_date(p: &[u8]) -> Option<(u16, u8, u8)> {
    let day = bcd(p[1] & 0x3F)?;
    let month = bcd(p[2] & 0x1F)?;
    let year = bcd(p[3])?;
    if day == 0 || month == 0 || month > 12 || day > 31 {
        return None;
    }
    Some((if year >= 70 { 1900 + year as u16 } else { 2000 + year as u16 }, month, day))
}

fn rec_time(p: &[u8]) -> Option<(u8, u8, u8)> {
    let s = bcd(p[1] & 0x7F)?;
    let m = bcd(p[2] & 0x7F)?;
    let h = bcd(p[3] & 0x3F)?;
    if s > 59 || m > 59 || h > 23 {
        return None;
    }
    Some((h, m, s))
}

fn audio_source(p: &[u8]) -> Option<AudioSource> {
    if p[0] == 0xFF {
        return None;
    }
    Some(AudioSource {
        locked: p[0] & 0x80 == 0,
        channels_per_block: ((p[1] >> 4) & 3) + 1,
        stype: p[2] & 0x1F,
        sampling_rate: match (p[3] >> 3) & 7 {
            0 => 48000,
            1 => 44100,
            2 => 32000,
            _ => return None,
        },
        quantization: match p[3] & 7 {
            0 => 16,
            1 => 12,
            2 => 20,
            _ => return None,
        },
    })
}

/// Parse the packs of one frame (at least the first DIF sequence must be present).
pub fn parse_frame(d: &[u8]) -> Option<Frame> {
    if d.len() < SEQUENCE || d[0] >> 5 != SCT_HEADER || d[1] & 0xF0 != 0 {
        return None;
    }
    let mut f = Frame { dsf: d[3] & 0x80 != 0, apt: d[4] & 7, ..Default::default() };
    let blocks = (d.len() / BLOCK).min(f.sequences() * 150);
    for b in 0..blocks {
        let blk = &d[b * BLOCK..(b + 1) * BLOCK];
        match blk[0] >> 5 {
            SCT_SUBCODE => {
                for k in 0..6 {
                    let p = &blk[3 + k * 8 + 3..3 + k * 8 + 8];
                    match p[0] {
                        0x13 if f.time_code.is_none() => f.time_code = time_code(&p[1..]),
                        0x62 if f.date.is_none() => f.date = rec_date(&p[1..]),
                        0x63 if f.time.is_none() => f.time = rec_time(&p[1..]),
                        _ => {}
                    }
                }
            }
            SCT_VAUX => {
                for k in 0..15 {
                    let p = &blk[3 + k * 5..3 + k * 5 + 5];
                    match p[0] {
                        0x60 if f.stype.is_none() => f.stype = Some(p[3] & 0x1F),
                        0x61 if f.wide.is_none() => {
                            f.wide = Some(matches!(p[2] & 7, 2 | 3));
                            f.interlaced = Some(p[3] & 0x10 != 0);
                        }
                        0x62 if f.date.is_none() => f.date = rec_date(&p[1..]),
                        0x63 if f.time.is_none() => f.time = rec_time(&p[1..]),
                        _ => {}
                    }
                }
            }
            SCT_AUDIO => {
                if f.audio.is_none() && blk[3] == 0x50 {
                    f.audio = audio_source(&blk[4..8]);
                }
            }
            _ => {}
        }
    }
    Some(f)
}

/// Payload bytes the reference attributes to the video essence of one frame.
pub fn video_bytes_per_frame(f: &Frame) -> u64 {
    f.sequences() as u64 * 134 * 76
}

pub fn recorded_date(f: &Frame) -> Option<String> {
    let (y, m, d) = f.date?;
    let (hh, mm, ss) = f.time.unwrap_or((0, 0, 0));
    Some(format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}.000"))
}

/// Fill a video stream from a frame.
pub fn apply_frame(s: &mut Stream, d: &[u8]) -> bool {
    let Some(f) = parse_frame(d) else { return false };
    apply(s, &f);
    true
}

pub fn apply(s: &mut Stream, f: &Frame) {
    s.set_if_empty("Format", "DV");
    s.set_if_empty("BitRate_Mode", "CBR");
    s.set_if_empty("Width", "720");
    s.set_if_empty("Height", if f.dsf { "576" } else { "480" });
    let wide = f.wide.unwrap_or(false);
    if !s.has("PixelAspectRatio") && !s.has("DisplayAspectRatio") {
        let dar = if wide { 16.0 / 9.0 } else { 4.0 / 3.0 };
        let h = if f.dsf { 576.0 } else { 480.0 };
        s.set("PixelAspectRatio", format!("{:.3}", dar * h / 720.0));
        s.set("DisplayAspectRatio", format!("{dar:.3}"));
    }
    s.set_if_empty("FrameRate_Mode", "CFR");
    s.set_if_empty("FrameRate", format!("{:.3}", f.frame_rate()));
    s.set_if_empty("Standard", if f.dsf { "PAL" } else { "NTSC" });
    s.set_if_empty("ColorSpace", "YUV");
    let chroma = match f.stype {
        Some(4) | Some(0x14) => "4:1:1",
        Some(0x18) => "4:2:2",
        _ if f.dsf => "4:2:0",
        _ => "4:1:1",
    };
    s.set_if_empty("ChromaSubsampling", chroma);
    s.set_if_empty("BitDepth", "8");
    if f.interlaced.unwrap_or(true) {
        s.set_if_empty("ScanType", "Interlaced");
        s.set_if_empty("ScanOrder", "BFF");
    } else {
        s.set_if_empty("ScanType", "Progressive");
    }
    s.set_if_empty("Compression_Mode", "Lossy");
    if let Some(tc) = &f.time_code {
        let sep = if tc.drop_frame { ";" } else { ":" };
        s.set("TimeCode_FirstFrame", format!("{:02}:{:02}:{:02}{sep}{:02}", tc.hours, tc.minutes, tc.seconds, tc.frames));
        s.set("TimeCode_Source", "Subcode time code");
        let ms = (tc.hours as f64 * 3600.0 + tc.minutes as f64 * 60.0 + tc.seconds as f64) * 1000.0 + tc.frames as f64 * 1000.0 / f.frame_rate();
        s.set("Delay", format!("{}", ms.round() as i64));
        s.set("Delay_DropFrame", if tc.drop_frame { "Yes" } else { "No" });
        s.set("Delay_Source", "Stream");
    }
}

/// Fill an audio stream from the AAUX source pack; `None` when the frame carries no audio.
pub fn audio_stream(f: &Frame) -> Option<Stream> {
    let a = f.audio.as_ref()?;
    let mut s = Stream::new(StreamKind::Audio);
    s.set("Format", "PCM");
    s.set("BitRate_Mode", "CBR");
    let channels = if a.quantization == 12 && a.channels_per_block > 1 { 4 } else { 2 };
    s.set("Channel(s)", channels.to_string());
    s.set("SamplingRate", a.sampling_rate.to_string());
    s.set("BitDepth", a.quantization.to_string());
    s.set("BitRate", (a.sampling_rate as u64 * a.quantization as u64 * channels as u64).to_string());
    s.set("Compression_Mode", "Lossless");
    Some(s)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// One PAL DIF sequence with header, subcode timecode, VAUX packs and optional AAUX.
    pub fn sequence(dsf: bool, seq: u8, audio: bool) -> Vec<u8> {
        let mut d = vec![0xFFu8; SEQUENCE];
        let id = |sct: u8, dbn: u8| [(sct << 5) | 0x1F, (seq << 4) | 0x07, dbn];
        d[..3].copy_from_slice(&id(SCT_HEADER, 0));
        d[3] = if dsf { 0xBF } else { 0x3F };
        d[4] = 0xF8;
        for k in 0..2 {
            let o = (1 + k) * BLOCK;
            d[o..o + 3].copy_from_slice(&id(SCT_SUBCODE, k as u8));
            for j in 0..6 {
                let p = o + 3 + j * 8;
                d[p..p + 3].copy_from_slice(&[0x8F, 0xF0 | j as u8, 0xFF]);
                d[p + 3..p + 8].copy_from_slice(&[0x13, 0x05, 0x80 | 0x30, 0x80 | 0x02, 0xC0 | 0x01]);
            }
        }
        for k in 0..3 {
            let o = (3 + k) * BLOCK;
            d[o..o + 3].copy_from_slice(&id(SCT_VAUX, k as u8));
            let packs: [[u8; 5]; 4] = [[0x60, 0xFF, 0xFF, if dsf { 0xE0 } else { 0xC0 }, 0xFF], [0x61, 0x3F, 0xC8 | 2, 0xFC, 0xFF], [0x62, 0xFF, 0xC1, 0x01, 0x70], [0x63, 0xFF, 0x80, 0x80, 0xC0]];
            for (j, p) in packs.iter().enumerate() {
                d[o + 3 + j * 5..o + 8 + j * 5].copy_from_slice(p);
            }
        }
        for k in 0..9 {
            let o = (6 + k * 16) * BLOCK;
            d[o..o + 3].copy_from_slice(&id(SCT_AUDIO, k as u8));
            if audio {
                d[o + 3..o + 8].copy_from_slice(&[0x50, 0x3F, 0x00, 0xC0, 0x00]); // 48 kHz, 16 bit
            }
            for v in 0..15 {
                let vo = o + (1 + v) * BLOCK;
                d[vo..vo + 3].copy_from_slice(&id(SCT_VIDEO, (k * 15 + v) as u8));
            }
        }
        d
    }

    #[test]
    fn frame_packs() {
        let d = sequence(true, 0, true);
        let f = parse_frame(&d).unwrap();
        assert!(f.dsf);
        assert_eq!(f.sequences(), 12);
        assert_eq!(f.size(), 144000);
        let tc = f.time_code.as_ref().unwrap();
        assert_eq!((tc.hours, tc.minutes, tc.seconds, tc.frames), (1, 2, 30, 5));
        assert_eq!(f.date, Some((1970, 1, 1)));
        assert_eq!(f.time, Some((0, 0, 0)));
        assert_eq!(f.wide, Some(true));
        assert_eq!(f.interlaced, Some(true));
        assert_eq!(recorded_date(&f).as_deref(), Some("1970-01-01 00:00:00.000"));
        let a = f.audio.as_ref().unwrap();
        assert_eq!((a.sampling_rate, a.quantization), (48000, 16));
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_frame(&mut s, &d));
        assert_eq!(s.get("Height"), "576");
        assert_eq!(s.get("DisplayAspectRatio"), "1.778");
        assert_eq!(s.get("Standard"), "PAL");
        assert_eq!(s.get("ChromaSubsampling"), "4:2:0");
        assert_eq!(s.get("ScanOrder"), "BFF");
        assert_eq!(s.get("TimeCode_FirstFrame"), "01:02:30:05");
        assert_eq!(s.get("Delay"), "3750200");
        let a = audio_stream(&f).unwrap();
        assert_eq!(a.get("BitRate"), "1536000");
        assert_eq!(a.get("Channel(s)"), "2");
        assert_eq!(video_bytes_per_frame(&f), 122208);
        assert!(parse_frame(&d[..1000]).is_none());
        let f = parse_frame(&sequence(false, 0, false)).unwrap();
        assert!(!f.dsf && f.audio.is_none());
        assert_eq!(f.size(), 120000);
    }
}

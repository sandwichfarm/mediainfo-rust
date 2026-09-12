//! Raw DV (`.dv/.dif`): a sequence of fixed-size DIF frames (IEC 61834-2), 120000 bytes for
//! 525/60 and 144000 bytes for 625/50 material.

use crate::io::Reader;
use crate::model::{Doc, StreamKind};
use crate::parsers::video::dv as dvv;
use crate::parsers::Probe;

fn looks_like_frame_start(d: &[u8]) -> bool {
    // header block, then the second DIF sequence's header block 12000 bytes later
    d.len() >= dvv::SEQUENCE + 3 && d[0] >> 5 == dvv::SCT_HEADER && d[1] & 0xF0 == 0 && d[dvv::BLOCK] >> 5 == dvv::SCT_SUBCODE && d[dvv::SEQUENCE] >> 5 == dvv::SCT_HEADER && d[dvv::SEQUENCE + 1] & 0xF0 == 0x10
}

pub fn probe(p: &Probe) -> u8 {
    if !looks_like_frame_start(p.head) {
        return 0;
    }
    if p.ext_in(&["dv", "dif"]) {
        95
    } else {
        70
    }
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    let first = r.read_vec_at(0, 144000);
    let Some(f) = dvv::parse_frame(&first) else { return false };
    let frame_size = f.size() as u64;
    if r.len() < frame_size {
        return false;
    }
    // The second frame must start with a header block too.
    if r.len() >= frame_size + dvv::BLOCK as u64 && !r.read_at(frame_size, 3).first().is_some_and(|b| b >> 5 == dvv::SCT_HEADER) {
        return false;
    }
    let frames = r.len() / frame_size;
    let fps = f.frame_rate();
    let duration_ms = frames as f64 / fps * 1000.0;
    let video_size = frames * dvv::video_bytes_per_frame(&f);

    let mut v = crate::model::Stream::new(StreamKind::Video);
    dvv::apply(&mut v, &f);
    v.set("Duration", format!("{}", duration_ms.round() as u64));
    v.set("FrameCount", frames.to_string());
    v.set("StreamSize", video_size.to_string());
    if duration_ms > 0.0 {
        v.set("BitRate", format!("{}", (video_size as f64 * 8.0 * 1000.0 / duration_ms).round() as u64));
    }

    let audio = dvv::audio_stream(&f).map(|mut a| {
        a.set("Duration", format!("{}", duration_ms.round() as u64));
        a
    });

    let g = doc.general();
    g.set("Format", "DV");
    g.set("Duration", format!("{}", duration_ms.round() as u64));
    g.set("OverallBitRate_Mode", "CBR");
    if duration_ms > 0.0 {
        g.set("OverallBitRate", format!("{}", (r.len() as f64 * 8.0 * 1000.0 / duration_ms).round() as u64));
    }
    g.set("StreamSize", r.len().saturating_sub(video_size).to_string());
    if let Some(date) = dvv::recorded_date(&f) {
        g.set("Recorded_Date", date);
    }
    doc.streams[StreamKind::Video as usize].push(v);
    if let Some(a) = audio {
        doc.streams[StreamKind::Audio as usize].push(a);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::video::dv::tests::sequence;

    fn file(dsf: bool, frames: usize, audio: bool) -> Vec<u8> {
        let mut d = Vec::new();
        for _ in 0..frames {
            for s in 0..if dsf { 12 } else { 10 } {
                d.extend(sequence(dsf, s as u8, audio));
            }
        }
        d
    }

    #[test]
    fn pal_file() {
        let d = file(true, 5, false);
        assert_eq!(d.len(), 720000);
        assert_eq!(probe(&Probe { head: &d[..65536], ext: "dv", size: d.len() as u64 }), 95);
        assert_eq!(probe(&Probe { head: &d[..65536], ext: "bin", size: d.len() as u64 }), 70);
        assert_eq!(probe(&Probe { head: &d[..1000], ext: "dv", size: 1000 }), 0);
        let mut r = Reader::from_bytes(d);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let g = doc.general_ref();
        assert_eq!(g.get("Format"), "DV");
        assert_eq!(g.get("Duration"), "200");
        assert_eq!(g.get("OverallBitRate"), "28800000");
        assert_eq!(g.get("StreamSize"), "108960");
        assert_eq!(g.get("Recorded_Date"), "1970-01-01 00:00:00.000");
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("FrameCount"), "5");
        assert_eq!(v.get("StreamSize"), "611040");
        assert_eq!(v.get("BitRate"), "24441600");
        assert!(doc.streams[StreamKind::Audio as usize].is_empty());
    }

    #[test]
    fn ntsc_with_audio() {
        let d = file(false, 3, true);
        let mut r = Reader::from_bytes(d);
        let mut doc = Doc::new();
        assert!(parse(&mut r, &mut doc));
        let v = &doc.streams[StreamKind::Video as usize][0];
        assert_eq!(v.get("Height"), "480");
        assert_eq!(v.get("FrameRate"), "29.970");
        assert_eq!(v.get("Standard"), "NTSC");
        let a = &doc.streams[StreamKind::Audio as usize][0];
        assert_eq!(a.get("SamplingRate"), "48000");
        assert_eq!(a.get("Duration"), "100");
        assert!(!parse(&mut Reader::from_bytes(vec![0; 100]), &mut Doc::new()));
    }
}

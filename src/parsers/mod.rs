//! Format detection and the parser registry.
//!
//! Each format has a `probe` (score from the first bytes and extension) and a `parse` that fills a
//! [`Doc`]. Codec-level helpers (`video::*`, `audio::*`) are shared by containers.

pub mod audio;
pub mod image;
pub mod text;
pub mod video;

pub mod aiff;
pub mod asf;
pub mod au;
pub mod caf;
pub mod dv;
pub mod flv;
pub mod ivf;
pub mod matroska;
pub mod mp4;
pub mod mpeg_ps;
pub mod mpeg_ts;
pub mod mxf;
pub mod nut;
pub mod ogg;
pub mod riff;
pub mod rm;

use crate::io::Reader;
use crate::model::Doc;

/// What a probe sees: the first bytes of the file, the lower-cased extension, the file size.
pub struct Probe<'a> {
    pub head: &'a [u8],
    pub ext: &'a str,
    pub size: u64,
}

impl Probe<'_> {
    pub fn starts_with(&self, magic: &[u8]) -> bool {
        self.head.starts_with(magic)
    }
    pub fn at(&self, offset: usize, magic: &[u8]) -> bool {
        self.head.get(offset..offset + magic.len()) == Some(magic)
    }
    pub fn ext_in(&self, exts: &[&str]) -> bool {
        exts.contains(&self.ext)
    }
}

pub type ProbeFn = fn(&Probe) -> u8;
pub type ParseFn = fn(&mut Reader, &mut Doc) -> bool;

pub struct Format {
    pub name: &'static str,
    pub probe: ProbeFn,
    pub parse: ParseFn,
}

/// Registry in priority order (ties in probe score go to the earlier entry).
pub static FORMATS: &[Format] = &[
    Format { name: "Matroska", probe: matroska::probe, parse: matroska::parse },
    Format { name: "MPEG-4", probe: mp4::probe, parse: mp4::parse },
    Format { name: "RIFF", probe: riff::probe, parse: riff::parse },
    Format { name: "AIFF", probe: aiff::probe, parse: aiff::parse },
    Format { name: "AU", probe: au::probe, parse: au::parse },
    Format { name: "CAF", probe: caf::probe, parse: caf::parse },
    Format { name: "Ogg", probe: ogg::probe, parse: ogg::parse },
    Format { name: "ASF", probe: asf::probe, parse: asf::parse },
    Format { name: "FLV", probe: flv::probe, parse: flv::parse },
    Format { name: "RealMedia", probe: rm::probe, parse: rm::parse },
    Format { name: "MXF", probe: mxf::probe, parse: mxf::parse },
    Format { name: "Nut", probe: nut::probe, parse: nut::parse },
    Format { name: "IVF", probe: ivf::probe, parse: ivf::parse },
    Format { name: "MPEG-TS", probe: mpeg_ts::probe, parse: mpeg_ts::parse },
    Format { name: "MPEG-PS", probe: mpeg_ps::probe, parse: mpeg_ps::parse },
    Format { name: "DV", probe: dv::probe, parse: dv::parse },
    Format { name: "FLAC", probe: audio::flac::probe, parse: audio::flac::parse },
    Format { name: "WavPack", probe: audio::wavpack::probe, parse: audio::wavpack::parse },
    Format { name: "TTA", probe: audio::tta::probe, parse: audio::tta::parse },
    Format { name: "Monkey's Audio", probe: audio::ape::probe, parse: audio::ape::parse },
    Format { name: "AMR", probe: audio::amr::probe, parse: audio::amr::parse },
    Format { name: "MLP/TrueHD", probe: audio::mlp::probe, parse: audio::mlp::parse },
    Format { name: "AC-3", probe: audio::ac3::probe, parse: audio::ac3::parse },
    Format { name: "DTS", probe: audio::dts::probe, parse: audio::dts::parse },
    Format { name: "ADTS", probe: audio::aac::probe_adts, parse: audio::aac::parse_adts },
    Format { name: "LATM", probe: audio::aac::probe_latm, parse: audio::aac::parse_latm },
    Format { name: "MPEG Audio", probe: audio::mpeg_audio::probe, parse: audio::mpeg_audio::parse },
    Format { name: "AVC", probe: video::avc::probe, parse: video::avc::parse },
    Format { name: "HEVC", probe: video::hevc::probe, parse: video::hevc::parse },
    Format { name: "MPEG Video", probe: video::mpegv::probe, parse: video::mpegv::parse },
    Format { name: "MPEG-4 Visual", probe: video::mpeg4v::probe, parse: video::mpeg4v::parse },
    Format { name: "AV1", probe: video::av1::probe, parse: video::av1::parse },
    Format { name: "VC-3", probe: video::dnxhd::probe, parse: video::dnxhd::parse },
    Format { name: "JPEG", probe: image::jpeg::probe, parse: image::jpeg::parse },
    Format { name: "PNG", probe: image::png::probe, parse: image::png::parse },
    Format { name: "GIF", probe: image::gif::probe, parse: image::gif::parse },
    Format { name: "Bitmap", probe: image::bmp::probe, parse: image::bmp::parse },
    Format { name: "TIFF", probe: image::tiff::probe, parse: image::tiff::parse },
    Format { name: "JPEG 2000", probe: image::jp2::probe, parse: image::jp2::parse },
    Format { name: "SubRip", probe: text::srt::probe, parse: text::srt::parse },
    Format { name: "ASS/SSA", probe: text::ass::probe, parse: text::ass::parse },
    Format { name: "WebVTT", probe: text::vtt::probe, parse: text::vtt::parse },
    Format { name: "VobSub", probe: text::vobsub::probe, parse: text::vobsub::parse },
    Format { name: "Raw YUV", probe: video::rawvideo::probe, parse: video::rawvideo::parse },
];

const HEAD: usize = 64 * 1024;

/// Detect and parse. Returns whether any format accepted the file.
pub fn parse(reader: &mut Reader, ext: &str) -> (bool, Doc) {
    let head = reader.read_vec_at(0, HEAD);
    let probe = Probe { head: &head, ext, size: reader.len() };
    let mut scored: Vec<(u8, &Format)> = FORMATS.iter().map(|f| ((f.probe)(&probe), f)).filter(|(s, _)| *s > 0).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, f) in scored {
        let mut doc = Doc::new();
        reader.seek(0);
        if (f.parse)(reader, &mut doc) {
            return (true, doc);
        }
    }
    let mut doc = Doc::new();
    doc.general().set_int("StreamSize", reader.len() as i128);
    (false, doc)
}

/// Extension-only score for formats whose magic is weak.
pub fn ext_score(p: &Probe, exts: &[&str], magic_ok: bool) -> u8 {
    match (magic_ok, p.ext_in(exts)) {
        (true, true) => 90,
        (true, false) => 60,
        (false, true) => 20,
        (false, false) => 0,
    }
}

//! Video FourCC → Format table and the BITMAPINFOHEADER used by AVI, Matroska `V_MS/VFW/FOURCC`
//! and ASF.

use crate::io::{le16, le32};
use crate::model::Stream;
use crate::parsers::video::avc;

/// (Format, Format_Profile, CodecID/Hint) for a video FourCC. Lookups are case-sensitive: the
/// reference keeps unknown spellings (for example `xvid`) as the format name itself.
pub fn fourcc_format(fourcc: &str) -> Option<(&'static str, &'static str, &'static str)> {
    let f = fourcc.trim_end();
    let entry: (&str, &str, &str) = match f {
        "DIVX" | "divx" | "DX50" | "dx50" | "DIV5" | "DIV6" => ("MPEG-4 Visual", "", "DivX"),
        "XVID" | "XviD" => ("MPEG-4 Visual", "", "XviD"),
        "FMP4" | "fmp4" | "MP4V" | "mp4v" | "M4S2" | "m4s2" | "3IV2" | "3iv2" | "3IVD" | "SEDG" | "RMP4" | "UMP4" | "WV1F" | "SMP4" | "HDX4" | "hdx4" | "GEOX" | "NDIG" | "DM4V" | "MP4S" | "mp4s" | "EM4A" | "EPHV" | "VIDM" | "LMP4" | "DP02" | "BLZ0" | "EM4V" | "FVFW" | "fvfw" | "QMP4" => ("MPEG-4 Visual", "", ""),
        "MP42" | "mp42" => ("MPEG-4 Visual", "", "Microsoft"),
        "MP43" | "mp43" => ("MPEG-4 Visual", "", "Microsoft"),
        "MPG4" | "mpg4" | "MP41" => ("MPEG-4 Visual", "", "Microsoft"),
        "DIV3" | "div3" | "DIV4" | "div4" | "AP41" | "COL1" | "COL0" => ("MPEG-4 Visual", "", "DivX"),
        "H264" | "h264" | "X264" | "x264" | "AVC1" | "avc1" | "avc3" | "DAVC" | "VSSH" | "vssh" | "ai5p" | "ai5q" | "ai52" | "ai53" | "ai55" | "ai56" | "ai1p" | "ai1q" | "ai12" | "ai13" | "ai15" | "ai16" | "AVin" | "AMVC" => ("AVC", "", ""),
        "HEVC" | "hevc" | "HVC1" | "hvc1" | "HEV1" | "hev1" | "H265" | "h265" | "X265" | "x265" | "dvhe" | "dvh1" => ("HEVC", "", ""),
        "MJPG" | "mjpg" | "MJPA" | "mjpa" | "MJPB" | "mjpb" | "dmb1" | "JPEG" | "jpeg" | "LJPG" | "ljpg" | "AVRn" | "AVDJ" | "ADJV" | "IJPG" | "jpg " | "SP5X" => ("JPEG", "", ""),
        "MJ2C" | "mjp2" | "MJP2" => ("JPEG 2000", "", ""),
        "I420" | "IYUV" | "YV12" | "YUY2" | "YUYV" | "UYVY" | "YVYU" | "NV12" | "NV21" | "YV16" | "Y42B" | "I422" | "Y800" | "GREY" | "Y8  " | "YVU9" | "v210" | "v410" | "v308" | "v408" | "2vuy" | "yuv2" | "yuvs" | "UYNV" | "UYNY" | "VYUY" | "I444" | "YV24" | "Y41P" | "I411" | "IYU1" | "IYU2" | "AYUV" | "Y16 " | "Y210" | "Y410" | "P010" | "P016" => ("YUV", "", ""),
        "DIB " | "RGB " | "RGBA" | "BGRA" | "RGB2" | "RGB3" | "RGB4" | "raw " | "rgb " | "r210" | "R10k" | "R10g" | "b64a" | "b48r" | "BGR " | "BGR3" | "BGR4" | "ABGR" | "ARGB" => ("RGB", "", ""),
        "WMV1" | "wmv1" => ("WMV1", "", ""),
        "WMV2" | "wmv2" => ("WMV2", "", ""),
        "WMV3" | "wmv3" | "WVC1" | "wvc1" | "WMVA" | "wmva" | "VC-1" | "vc-1" | "vc1 " => ("VC-1", "", ""),
        "MSS1" | "MSS2" => ("Windows Media Screen", "", ""),
        "dvsd" | "DVSD" | "dvhd" | "DVHD" | "dvsl" | "DVSL" | "dvpp" | "dvp " | "dv25" | "dv50" | "dvc " | "dvcp" | "dvhq" | "dvhp" | "dvh5" | "dvh6" | "dv5n" | "dv5p" | "CDVC" | "cdvc" | "CDVH" | "CDV5" | "AVdv" | "AVd1" => ("DV", "", ""),
        "VP30" | "VP31" | "vp31" | "VP3 " => ("VP3", "", ""),
        "VP40" => ("VP4", "", ""),
        "VP50" => ("VP5", "", ""),
        "VP60" | "VP61" | "VP62" | "VP6F" | "VP6A" | "FLV4" | "vp6f" => ("VP6", "", ""),
        "VP70" | "VP71" | "VP72" => ("VP7", "", ""),
        "VP80" | "vp80" | "vp08" => ("VP8", "", ""),
        "VP90" | "vp90" | "vp09" => ("VP9", "", ""),
        "AV01" | "av01" => ("AV1", "", ""),
        "FLV1" | "flv1" => ("Sorenson Spark", "", ""),
        "RV10" | "RV13" | "rv10" | "rv13" => ("RealVideo 1", "", ""),
        "RV20" | "rv20" | "RVTR" => ("RealVideo 2", "", ""),
        "RV30" | "rv30" => ("RealVideo 3", "", ""),
        "RV40" | "rv40" | "RV41" => ("RealVideo 4", "", ""),
        "cvid" | "CVID" => ("Cinepak", "", ""),
        "SVQ1" | "svq1" | "svqi" => ("Sorenson Video 1", "", ""),
        "SVQ3" | "svq3" => ("Sorenson Video 3", "", ""),
        "apch" => ("ProRes", "422 HQ", ""),
        "apcn" => ("ProRes", "422", ""),
        "apcs" => ("ProRes", "422 LT", ""),
        "apco" => ("ProRes", "422 Proxy", ""),
        "ap4h" => ("ProRes", "4444", ""),
        "ap4x" => ("ProRes", "4444 XQ", ""),
        "aprn" => ("ProRes", "RAW", ""),
        "aprh" => ("ProRes", "RAW HQ", ""),
        "mpg2" | "MPG2" | "MPEG" | "mpeg" | "mp2v" | "MP2V" | "m2v1" | "mpg1" | "MPG1" | "PIM1" | "PIM2" | "mpgv" | "MPGV" | "MMES" | "mmes" | "LMP2" | "hdv1" | "hdv2" | "hdv3" | "hdv5" | "hdv6" | "hdv7" | "hdv8" | "hdv9" | "xd54" | "xd55" | "xd59" | "xd5a" | "xd5b" | "xd5c" | "xd5d" | "xd5e" | "xd5f" | "xdv1" | "xdv2" | "xdv3" | "xdvb" | "xdvc" | "xdvd" | "xdve" | "xdvf" | "M701" | "M702" | "M703" | "M704" | "M705" | "SLIF" | "EM2V" => ("MPEG Video", "", ""),
        "theo" | "THEO" => ("Theora", "", ""),
        "AVdn" | "AVdh" => ("VC-3", "", ""),
        "MSVC" | "msvc" | "CRAM" | "cram" | "WHAM" => ("Microsoft Video 1", "", ""),
        "MRLE" | "mrle" | "rle " | "RLE " => ("RLE", "", ""),
        "IV31" | "IV32" | "iv31" | "iv32" => ("Indeo 3", "", ""),
        "IV41" | "iv41" => ("Indeo 4", "", ""),
        "IV50" | "iv50" => ("Indeo 5", "", ""),
        "HFYU" | "hfyu" | "FFVH" => ("HuffYUV", "", ""),
        "FFV1" | "ffv1" => ("FFV1", "", ""),
        "LAGS" => ("Lagarith", "", ""),
        "ULRG" | "ULRA" | "ULY0" | "ULY2" | "ULY4" | "ULH0" | "ULH2" | "ULH4" | "UQY2" | "UQRG" | "UQRA" | "UMY2" | "UMH2" | "UMY4" | "UMH4" | "UMRG" | "UMRA" => ("Ut Video", "", ""),
        "S263" | "s263" | "H263" | "h263" | "U263" | "M263" | "D263" | "L263" | "T263" | "X263" | "I263" | "i263" => ("H.263", "", ""),
        "H261" | "h261" | "M261" => ("H.261", "", ""),
        "png " | "PNG " | "MPNG" | "mpng" => ("PNG", "", ""),
        "tscc" | "TSCC" | "tsc2" | "TSC2" => ("TechSmith", "", ""),
        "ZLIB" | "zlib" => ("ZLIB", "", ""),
        "snow" | "SNOW" => ("Snow", "", ""),
        "DRAC" | "drac" | "dirc" => ("Dirac", "", ""),
        "rpza" | "azpr" => ("Apple Video", "", ""),
        "smc " | "SMC " => ("Graphics", "", ""),
        "8BPS" => ("Planar RGB", "", ""),
        "qtrl" | "QTRL" | "rle1" => ("QuickTime RLE", "", ""),
        "CUVC" | "cuvc" => ("Canopus HQ", "", ""),
        "CLLC" | "CLJR" => ("Canopus Lossless", "", ""),
        "ac16" | "ac32" | "ac48" | "ac24" | "icod" => ("Apple Intermediate Codec", "", ""),
        "MSZH" | "ZMBV" | "CSCD" | "FPS1" | "LOCO" | "QPEG" | "VCR1" | "VCR2" | "WNV1" | "XMPG" => (fourcc_static(f), "", ""),
        _ => return None,
    };
    if entry.0.is_empty() {
        None
    } else {
        Some(entry)
    }
}

/// A few codec names equal to their FourCC; keeps the table above free of `&'static` juggling.
fn fourcc_static(f: &str) -> &'static str {
    match f {
        "MSZH" => "MSZH",
        "ZMBV" => "Zip Motion Blocks Video",
        "CSCD" => "CamStudio",
        "FPS1" => "Fraps",
        "LOCO" => "LOCO",
        "QPEG" => "QPEG",
        "VCR1" => "ATI VCR1",
        "VCR2" => "ATI VCR2",
        "WNV1" => "Winnov WNV1",
        "XMPG" => "XMPG",
        _ => "",
    }
}

#[derive(Debug, Clone, Default)]
pub struct BitmapInfoHeader {
    pub size: u32,
    pub width: i32,
    pub height: i32,
    pub planes: u16,
    pub bit_count: u16,
    pub compression: u32,
    pub size_image: u32,
}

impl BitmapInfoHeader {
    /// FourCC as text; `0x%08X` when the compression value is not printable (0 = RGB).
    pub fn fourcc(&self) -> String {
        let b = self.compression.to_le_bytes();
        if b.iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
            String::from_utf8_lossy(&b).into_owned()
        } else {
            format!("0x{:08X}", self.compression)
        }
    }
}

pub fn parse_bitmapinfoheader(d: &[u8]) -> Option<BitmapInfoHeader> {
    let size = le32(d, 0)?;
    if size < 40 || d.len() < 40 {
        return None;
    }
    Some(BitmapInfoHeader { size, width: le32(d, 4)? as i32, height: le32(d, 8)? as i32, planes: le16(d, 12)?, bit_count: le16(d, 14)?, compression: le32(d, 16)?, size_image: le32(d, 20)? })
}

/// Fill a video stream from a BITMAPINFOHEADER (+ codec-specific extra data after `biSize`).
pub fn apply_bitmapinfoheader(s: &mut Stream, d: &[u8]) -> bool {
    let Some(h) = parse_bitmapinfoheader(d) else { return false };
    let fourcc = h.fourcc();
    s.set("CodecID", &fourcc);
    let looked_up = if h.compression == 0 { Some(("RGB", "", "")) } else { fourcc_format(&fourcc) };
    match looked_up {
        Some((format, profile, hint)) => {
            s.set_if_empty("Format", format);
            if !profile.is_empty() {
                s.set_if_empty("Format_Profile", profile);
            }
            if !hint.is_empty() {
                s.set_if_empty("CodecID/Hint", hint);
            }
        }
        None => s.set_if_empty("Format", fourcc.trim()),
    }
    if h.width > 0 {
        s.set_if_empty("Width", h.width.to_string());
    }
    if h.height != 0 {
        s.set_if_empty("Height", h.height.unsigned_abs().to_string());
    }
    if s.get("Format") == "RGB" {
        s.set_if_empty("ColorSpace", "RGB");
        if h.bit_count > 0 {
            s.set_if_empty("BitDepth", (h.bit_count / 3).max(8).to_string());
        }
        s.set_if_empty("Compression_Mode", "Lossless");
    } else if s.get("Format") == "YUV" {
        s.set_if_empty("ColorSpace", "YUV");
        s.set_if_empty("Compression_Mode", "Lossless");
    }
    let extra = d.get(h.size as usize..).unwrap_or(&[]);
    if s.get("Format") == "AVC" && extra.first() == Some(&1) {
        avc::apply_avcc(s, extra);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;

    fn bih(fourcc: &[u8; 4], w: i32, h: i32, bits: u16) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend(40u32.to_le_bytes());
        d.extend(w.to_le_bytes());
        d.extend(h.to_le_bytes());
        d.extend(1u16.to_le_bytes());
        d.extend(bits.to_le_bytes());
        d.extend(fourcc);
        d.extend((w as u32 * h.unsigned_abs() * 3).to_le_bytes());
        d.extend([0u8; 16]);
        d
    }

    #[test]
    fn table() {
        assert_eq!(fourcc_format("XVID"), Some(("MPEG-4 Visual", "", "XviD")));
        assert_eq!(fourcc_format("xvid"), None);
        assert_eq!(fourcc_format("MJPG").map(|e| e.0), Some("JPEG"));
        assert_eq!(fourcc_format("I420").map(|e| e.0), Some("YUV"));
        assert_eq!(fourcc_format("H264").map(|e| e.0), Some("AVC"));
        assert_eq!(fourcc_format("apch"), Some(("ProRes", "422 HQ", "")));
        assert_eq!(fourcc_format("RV40").map(|e| e.0), Some("RealVideo 4"));
        assert_eq!(fourcc_format("MP43"), Some(("MPEG-4 Visual", "", "Microsoft")));
        assert_eq!(fourcc_format("ZMBV").map(|e| e.0), Some("Zip Motion Blocks Video"));
        assert_eq!(fourcc_format("ZZZZ"), None);
    }

    #[test]
    fn bitmapinfoheader() {
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_bitmapinfoheader(&mut s, &bih(b"MJPG", 64, 48, 24)));
        assert_eq!(s.get("CodecID"), "MJPG");
        assert_eq!(s.get("Format"), "JPEG");
        assert_eq!(s.get("Width"), "64");
        assert_eq!(s.get("Height"), "48");
        assert_eq!(s.get("BitDepth"), "");
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_bitmapinfoheader(&mut s, &bih(&[0, 0, 0, 0], 64, -48, 24)));
        assert_eq!(s.get("CodecID"), "0x00000000");
        assert_eq!(s.get("Format"), "RGB");
        assert_eq!(s.get("Height"), "48");
        assert_eq!(s.get("BitDepth"), "8");
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_bitmapinfoheader(&mut s, &bih(b"xvid", 64, 48, 24)));
        assert_eq!(s.get("Format"), "xvid");
        assert!(!apply_bitmapinfoheader(&mut s, &[40, 0, 0]));
        // AVC with an avcC after the header
        let mut d = bih(b"H264", 64, 48, 24);
        d.extend([1, 66, 0xC0, 10, 0xFF, 0xE0, 0]); // avcC with no SPS
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_bitmapinfoheader(&mut s, &d));
        assert_eq!(s.get("Format"), "AVC");
    }
}

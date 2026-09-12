//! Motion JPEG: every frame is a complete JPEG image, so the stream description comes from the
//! first frame's headers (`image::jpeg`).

use crate::model::Stream;
use crate::parsers::image::jpeg;

/// Fill a video stream from one M-JPEG frame. `true` when the frame header was found.
pub fn apply_frame(s: &mut Stream, data: &[u8]) -> bool {
    if !jpeg::apply_frame(s, data) {
        return false;
    }
    s.set_if_empty("Format", "JPEG");
    s.set_if_empty("ScanType", "Progressive");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;

    #[test]
    fn frame_fills_video_stream() {
        let mut data = vec![0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 0, 48, 0, 64, 3, 1, 0x22, 0, 2, 0x11, 0, 3, 0x11, 0];
        data.extend_from_slice(&[0xFF, 0xDA, 0, 2, 0xFF, 0xD9]);
        let mut s = Stream::new(StreamKind::Video);
        assert!(apply_frame(&mut s, &data));
        assert_eq!(s.get("Format"), "JPEG");
        assert_eq!(s.get("Width"), "64");
        assert_eq!(s.get("Height"), "48");
        assert_eq!(s.get("ColorSpace"), "YUV");
        assert_eq!(s.get("ChromaSubsampling"), "4:2:0");
        assert_eq!(s.get("ScanType"), "Progressive");
        assert_eq!(s.get("Compression_Mode"), "Lossy");
        let mut s = Stream::new(StreamKind::Video);
        assert!(!apply_frame(&mut s, &[0, 1, 2]));
    }
}

//! FourCC / BITMAPINFOHEADER (AVI, Matroska VFW, ASF) → Format. TODO
use crate::model::Stream;
pub fn apply_bitmapinfoheader(_s: &mut Stream, _d: &[u8]) -> bool { false }
/// (Format, Format_Profile/extra info, CodecID/Hint) for a video FourCC.
pub fn fourcc_format(_fourcc: &str) -> Option<(&'static str, &'static str, &'static str)> { None }

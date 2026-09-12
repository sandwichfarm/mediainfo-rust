//! WAVEFORMATEX(TENSIBLE) → Format/CodecID (AVI, WAV, Matroska A_MS/ACM, ASF). TODO
use crate::model::Stream;
pub fn apply_waveformatex(_s: &mut Stream, _d: &[u8]) -> bool { false }

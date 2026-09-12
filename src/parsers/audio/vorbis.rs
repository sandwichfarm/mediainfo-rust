//! Vorbis identification/comment headers. TODO
use crate::model::Stream;
pub fn apply_xiph_private(_s: &mut Stream, _d: &[u8]) -> bool { false }
pub fn apply_ident(_s: &mut Stream, _d: &[u8]) -> bool { false }
/// VorbisComment block (after the 7-byte packet header for Vorbis, raw for FLAC/Opus).
pub fn apply_comments(_s: &mut Stream, _general: &mut Stream, _d: &[u8]) -> bool { false }

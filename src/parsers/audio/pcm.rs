//! PCM descriptions for the various containers. TODO
use crate::model::Stream;
pub fn apply_matroska(_s: &mut Stream, _codec_id: &str, _bit_depth: u32) {}
// stub, implemented elsewhere (audio-parsers): endianness / sign / float / depth → Format_Settings*, BitDepth.
pub fn apply_pcm(_s: &mut Stream, _little_endian: Option<bool>, _signed: Option<bool>, _float: bool, _bit_depth: u32) {}

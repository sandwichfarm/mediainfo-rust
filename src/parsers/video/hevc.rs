//! H.265 / HEVC: VPS/SPS/SEI parsing and the Annex B elementary stream parser. TODO
use crate::io::Reader;
use crate::model::{Doc, Stream};
use crate::parsers::Probe;
pub fn apply_hvcc(_s: &mut Stream, _d: &[u8]) -> bool { false }
pub fn hvcc_length_size(_d: &[u8]) -> Option<usize> { None }
pub fn nals_length_prefixed(_d: &[u8], _len: usize) -> Vec<(u8, &[u8])> { Vec::new() }
pub fn nals_annexb(_d: &[u8]) -> Vec<(u8, &[u8])> { Vec::new() }
pub fn apply_sei_from_nals(_s: &mut Stream, _n: &[(u8, &[u8])]) {}
pub fn probe(_p: &Probe) -> u8 { 0 }
pub fn parse(_r: &mut Reader, _d: &mut Doc) -> bool { false }
/// Fill a video stream from an Annex B byte stream (VPS/SPS/PPS). // stub, implemented elsewhere
pub fn apply_annexb(_s: &mut Stream, _d: &[u8]) -> bool { false }

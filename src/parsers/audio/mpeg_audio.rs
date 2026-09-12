//! MPEG Audio (MP1/MP2/MP3) frames, ID3v1/v2, Xing/Info/VBRI, LAME. TODO
use crate::io::Reader;
use crate::model::{Doc, Stream};
use crate::parsers::Probe;
pub fn apply_frame(_s: &mut Stream, _d: &[u8]) -> bool { false }
pub fn probe(_p: &Probe) -> u8 { 0 }
pub fn parse(_r: &mut Reader, _d: &mut Doc) -> bool { false }

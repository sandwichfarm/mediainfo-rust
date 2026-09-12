//! FLAC: STREAMINFO and the native container. TODO
use crate::io::Reader;
use crate::model::{Doc, Stream};
use crate::parsers::Probe;
/// Accepts a bare 34-byte STREAMINFO or "fLaC" + metadata blocks.
pub fn apply_streaminfo_block(_s: &mut Stream, _d: &[u8]) -> bool { false }
pub fn probe(_p: &Probe) -> u8 { 0 }
pub fn parse(_r: &mut Reader, _d: &mut Doc) -> bool { false }

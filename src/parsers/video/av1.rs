//! AV1: av1C record, sequence header OBU, metadata OBUs, low-overhead OBU stream. TODO
use crate::io::Reader;
use crate::model::{Doc, Stream};
use crate::parsers::Probe;
pub fn apply_av1c(_s: &mut Stream, _d: &[u8]) -> bool { false }
pub fn apply_obus(_s: &mut Stream, _d: &[u8]) -> bool { false }
pub fn apply_obus_metadata(_s: &mut Stream, _d: &[u8]) {}
pub fn probe(_p: &Probe) -> u8 { 0 }
pub fn parse(_r: &mut Reader, _d: &mut Doc) -> bool { false }

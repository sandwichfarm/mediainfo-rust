//! AAC: AudioSpecificConfig, ADTS and LATM/LOAS elementary streams. TODO
use crate::io::Reader;
use crate::model::{Doc, Stream};
use crate::parsers::Probe;
/// Fill from an AudioSpecificConfig; returns the audio object type.
pub fn apply_asc(_s: &mut Stream, _d: &[u8]) -> Option<u8> { None }
pub fn probe_adts(_p: &Probe) -> u8 { 0 }
pub fn parse_adts(_r: &mut Reader, _d: &mut Doc) -> bool { false }
pub fn probe_latm(_p: &Probe) -> u8 { 0 }
pub fn parse_latm(_r: &mut Reader, _d: &mut Doc) -> bool { false }

//! MPEG-4 Visual (Part 2): VOL/VOS headers, user data. TODO
use crate::io::Reader;
use crate::model::{Doc, Stream};
use crate::parsers::Probe;
pub fn apply_headers(_s: &mut Stream, _d: &[u8]) -> bool { false }
pub fn apply_frame_user_data(_s: &mut Stream, _d: &[u8]) {}
pub fn probe(_p: &Probe) -> u8 { 0 }
pub fn parse(_r: &mut Reader, _d: &mut Doc) -> bool { false }

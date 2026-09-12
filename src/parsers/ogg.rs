//! TODO

use crate::io::Reader;
use crate::model::Doc;
use crate::parsers::Probe;

pub fn probe(_p: &Probe) -> u8 { 0 }
pub fn parse(_r: &mut Reader, _d: &mut Doc) -> bool { false }

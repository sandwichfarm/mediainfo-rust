//! Theora identification header. TODO
use crate::model::Stream;
pub fn apply_xiph_private(_s: &mut Stream, _d: &[u8]) -> bool { false }
pub fn apply_ident(_s: &mut Stream, _d: &[u8]) -> bool { false }

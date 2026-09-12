//! WebP (RIFF `WEBP`): VP8 / VP8L / VP8X bitstream headers. TODO
// stub, implemented elsewhere (image-parsers). Called by `parsers::riff` for RIFF `WEBP` files.
use crate::io::Reader;
use crate::model::Doc;
pub fn parse_riff_webp(_r: &mut Reader, _d: &mut Doc) -> bool { false }

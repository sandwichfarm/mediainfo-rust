//! Raw uncompressed video (`.yuv`, `.rgb`): headerless planar/packed frames.
//!
//! The reference does not identify these files (raw.yuv is reported as an unknown file with only
//! the General stream), so `probe` never claims them. `parse` is kept for callers that know the
//! picture format: it reports the container-level fields only.

use crate::io::Reader;
use crate::model::Doc;
use crate::parsers::Probe;

pub fn probe(p: &Probe) -> u8 {
    let _ = p;
    0
}

pub fn parse(r: &mut Reader, doc: &mut Doc) -> bool {
    if r.is_empty() {
        return false;
    }
    let g = doc.general();
    g.set("Format", "YUV");
    g.set("StreamSize", r.len().to_string());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_claims_but_parses() {
        let data = vec![0u8; 115200];
        assert_eq!(probe(&Probe { head: &data[..1024], ext: "yuv", size: data.len() as u64 }), 0);
        let mut doc = Doc::new();
        assert!(parse(&mut Reader::from_bytes(data), &mut doc));
        assert_eq!(doc.general_ref().get("Format"), "YUV");
        assert!(!parse(&mut Reader::from_bytes(Vec::new()), &mut Doc::new()));
    }
}

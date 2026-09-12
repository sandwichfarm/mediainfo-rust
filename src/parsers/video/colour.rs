//! Colour description names (ITU-T H.273) shared by the video codec parsers, plus helpers that
//! store them on a stream.

use crate::model::Stream;

/// Source tag for colour fields taken from the codec bitstream.
pub const STREAM: &str = "Stream";

/// Set `colour_range` (+ `_Source`) from a full-range flag. The colour fields are schema fields,
/// so they are set through `Stream::set` (containers read them back to merge sources).
pub fn set_range(s: &mut Stream, full: bool, source: &str) {
    s.set("colour_range", if full { "Full" } else { "Limited" });
    s.set("colour_range_Source", source);
}

/// Set primaries / transfer / matrix (unspecified values are skipped); returns whether any was set.
pub fn set_description(s: &mut Stream, p: u8, t: u8, m: u8, source: &str) -> bool {
    let mut present = false;
    for (field, name) in [("colour_primaries", primaries(p)), ("transfer_characteristics", transfer(t)), ("matrix_coefficients", matrix(m))] {
        if !name.is_empty() {
            s.set(field, name);
            s.set(&format!("{field}_Source"), source);
            present = true;
        }
    }
    if present {
        s.set("colour_description_present", "Yes");
        s.set("colour_description_present_Source", source);
    }
    present
}

pub fn primaries(v: u8) -> &'static str {
    match v {
        1 => "BT.709",
        4 => "BT.470 System M",
        5 => "BT.601 PAL",
        6 => "BT.601 NTSC",
        7 => "SMPTE 240M",
        8 => "Generic film",
        9 => "BT.2020",
        10 => "XYZ",
        11 => "DCI P3",
        12 => "Display P3",
        22 => "EBU Tech 3213",
        2 => "",
        _ => "",
    }
}

pub fn transfer(v: u8) -> &'static str {
    match v {
        1 => "BT.709",
        4 => "BT.470 System M",
        5 => "BT.470 System B/G",
        6 => "BT.601",
        7 => "SMPTE 240M",
        8 => "Linear",
        9 => "Logarithmic (100:1)",
        10 => "Logarithmic (316.22777:1)",
        11 => "xvYCC",
        12 => "BT.1361",
        13 => "sRGB/sYCC",
        14 => "BT.2020 (10-bit)",
        15 => "BT.2020 (12-bit)",
        16 => "PQ",
        17 => "SMPTE 428M",
        18 => "HLG",
        _ => "",
    }
}

pub fn matrix(v: u8) -> &'static str {
    match v {
        0 => "Identity",
        1 => "BT.709",
        4 => "FCC 73.682",
        5 => "BT.470 System B/G",
        6 => "BT.601",
        7 => "SMPTE 240M",
        8 => "YCgCo",
        9 => "BT.2020 non-constant",
        10 => "BT.2020 constant",
        11 => "Y'D'zD'x",
        12 => "Chromaticity-derived non-constant",
        13 => "Chromaticity-derived constant",
        14 => "ICtCp",
        _ => "",
    }
}

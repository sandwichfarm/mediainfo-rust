//! Colour description names (ITU-T H.273) shared by the video codec parsers.

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

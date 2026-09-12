pub mod aac;
pub mod ac3;
pub mod alac;
pub mod amr;
pub mod ape;
pub mod dts;
pub mod flac;
pub mod mlp;
pub mod mpeg_audio;
pub mod opus;
pub mod pcm;
pub mod speex;
pub mod tta;
pub mod vorbis;
pub mod wavpack;
pub mod wma;

/// Standard positions/layout strings for a plain channel count (no explicit mask).
/// Returns (ChannelPositions, ChannelLayout).
pub fn layout_for_count(channels: u32) -> (&'static str, &'static str) {
    match channels {
        1 => ("Front: C", "C"),
        2 => ("Front: L R", "L R"),
        3 => ("Front: L C R", "L R C"),
        4 => ("Front: L R, Back: L R", "L R Lb Rb"),
        5 => ("Front: L C R, Back: L R", "L R C Lb Rb"),
        6 => ("Front: L C R, Back: L R, LFE", "L R C LFE Lb Rb"),
        7 => ("Front: L C R, Side: L R, Back: C, LFE", "L R C LFE Cb Ls Rs"),
        8 => ("Front: L C R, Side: L R, Back: L R, LFE", "L R C LFE Lb Rb Ls Rs"),
        _ => ("", ""),
    }
}

/// WAVEFORMATEXTENSIBLE / MP4 channel mask (SPEAKER_* bits) → (ChannelPositions, ChannelLayout).
pub fn layout_from_mask(mask: u32) -> (String, String) {
    const NAMES: &[(u32, &str, &str)] = &[
        (0x1, "Front", "L"),
        (0x2, "Front", "R"),
        (0x4, "Front", "C"),
        (0x8, "LFE", "LFE"),
        (0x10, "Back", "L"),
        (0x20, "Back", "R"),
        (0x40, "Front", "Lc"),
        (0x80, "Front", "Rc"),
        (0x100, "Back", "C"),
        (0x200, "Side", "L"),
        (0x400, "Side", "R"),
        (0x800, "Top", "C"),
        (0x1000, "Top Front", "L"),
        (0x2000, "Top Front", "C"),
        (0x4000, "Top Front", "R"),
        (0x8000, "Top Back", "L"),
        (0x10000, "Top Back", "C"),
        (0x20000, "Top Back", "R"),
    ];
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut layout: Vec<String> = Vec::new();
    for (bit, group, name) in NAMES {
        if mask & bit == 0 {
            continue;
        }
        let layout_name = match (*group, *name) {
            ("Back", "L") => "Lb",
            ("Back", "R") => "Rb",
            ("Back", "C") => "Cb",
            ("Side", "L") => "Ls",
            ("Side", "R") => "Rs",
            (_, n) => n,
        };
        layout.push(layout_name.to_string());
        if *group == "LFE" {
            continue;
        }
        match groups.iter_mut().find(|(g, _)| g == group) {
            Some((_, v)) => v.push(name),
            None => groups.push((group, vec![name])),
        }
    }
    // Within a group the reference lists left, centre, right.
    let canon = ["L", "C", "R", "Lc", "Rc"];
    let mut pos: Vec<String> = groups
        .iter()
        .map(|(g, v)| {
            let mut v = v.clone();
            v.sort_by_key(|n| canon.iter().position(|c| c == n).unwrap_or(9));
            format!("{g}: {}", v.join(" "))
        })
        .collect();
    if mask & 0x8 != 0 {
        pos.push("LFE".to_string());
    }
    // Layout order convention: L R C LFE then the rest.
    let order = ["L", "R", "C", "LFE"];
    layout.sort_by_key(|n| order.iter().position(|o| o == n).unwrap_or(10));
    (pos.join(", "), layout.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn masks() {
        assert_eq!(layout_from_mask(0x3F), ("Front: L C R, Back: L R, LFE".to_string(), "L R C LFE Lb Rb".to_string()));
        assert_eq!(layout_from_mask(0x4), ("Front: C".to_string(), "C".to_string()));
    }
}

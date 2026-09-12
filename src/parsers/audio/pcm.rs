//! PCM descriptions for the various containers (Matroska `A_PCM/*`, WAV, AIFF, AU, CAF, MOV...).

use crate::model::Stream;

/// Describe a PCM stream. `little`: byte order when known (`Some(true)` little-endian, `Some(false)`
/// big-endian); `signed`: sample sign when known; `float`: IEEE float samples (then no
/// endianness/sign settings are shown, only `Format_Profile` `Float`); `bit_depth`: bits per sample
/// (0 = unknown). Produces `Format` `PCM`, `Format_Settings_Endianness` (`Little`/`Big`),
/// `Format_Settings_Sign` (`Signed`/`Unsigned`), the combined `Format_Settings` (`Little / Signed`,
/// `Big`) and `BitDepth`. Compression_Mode is left to the container (WAV does not show it).
pub fn apply_pcm(s: &mut Stream, little: Option<bool>, signed: Option<bool>, float: bool, bit_depth: u32) {
    s.set_if_empty("Format", "PCM");
    if float {
        s.set("Format_Profile", "Float");
    } else {
        let mut settings = Vec::new();
        if let Some(l) = little {
            let v = if l { "Little" } else { "Big" };
            s.set("Format_Settings_Endianness", v);
            settings.push(v);
        }
        if let Some(sg) = signed {
            let v = if sg { "Signed" } else { "Unsigned" };
            s.set("Format_Settings_Sign", v);
            settings.push(v);
        }
        if !settings.is_empty() {
            s.set("Format_Settings", settings.join(" / "));
        }
    }
    if bit_depth > 0 {
        s.set("BitDepth", bit_depth.to_string());
    }
}

/// Matroska `A_PCM/INT/LIT`, `A_PCM/INT/BIG`, `A_PCM/FLOAT/IEEE` with the track's BitDepth.
pub fn apply_matroska(s: &mut Stream, codec_id: &str, bit_depth: u32) {
    match codec_id {
        "A_PCM/INT/LIT" => apply_pcm(s, Some(true), Some(true), false, bit_depth),
        "A_PCM/INT/BIG" => apply_pcm(s, Some(false), Some(true), false, bit_depth),
        "A_PCM/FLOAT/IEEE" => apply_pcm(s, Some(true), None, true, bit_depth),
        _ => apply_pcm(s, None, None, false, bit_depth),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;

    #[test]
    fn settings() {
        let mut s = Stream::new(StreamKind::Audio);
        apply_pcm(&mut s, Some(true), Some(true), false, 16);
        assert_eq!(s.get("Format"), "PCM");
        assert_eq!(s.get("Format_Settings"), "Little / Signed");
        assert_eq!(s.get("Format_Settings_Endianness"), "Little");
        assert_eq!(s.get("Format_Settings_Sign"), "Signed");
        assert_eq!(s.get("BitDepth"), "16");
        let mut s = Stream::new(StreamKind::Audio);
        apply_pcm(&mut s, Some(false), None, false, 0);
        assert_eq!(s.get("Format_Settings"), "Big");
        assert!(!s.has("Format_Settings_Sign"));
        assert!(!s.has("BitDepth"));
        let mut s = Stream::new(StreamKind::Audio);
        apply_pcm(&mut s, Some(true), Some(true), true, 32);
        assert_eq!(s.get("Format_Profile"), "Float");
        assert!(!s.has("Format_Settings"));
        assert!(!s.has("Format_Settings_Endianness"));
    }

    #[test]
    fn matroska_ids() {
        let mut s = Stream::new(StreamKind::Audio);
        apply_matroska(&mut s, "A_PCM/INT/BIG", 24);
        assert_eq!(s.get("Format_Settings"), "Big / Signed");
        assert_eq!(s.get("BitDepth"), "24");
        let mut s = Stream::new(StreamKind::Audio);
        apply_matroska(&mut s, "A_PCM/FLOAT/IEEE", 32);
        assert_eq!(s.get("Format_Profile"), "Float");
        let mut s = Stream::new(StreamKind::Audio);
        apply_matroska(&mut s, "A_PCM/INT/LIT", 8);
        assert_eq!(s.get("Format_Settings"), "Little / Signed");
    }
}

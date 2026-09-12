//! Human-readable value formatting matching the reference report strings.

/// Integer with thousands separated by a space (`1 920`).
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// Round to `sig` significant digits but never fewer than 0 decimals; the integer part is always kept.
pub fn sig_digits(v: f64, sig: usize) -> String {
    if v == 0.0 {
        return format!("{:.1$}", 0.0, sig.saturating_sub(1));
    }
    let int_digits = (v.abs().log10().floor() as i64 + 1).max(1) as usize;
    let decimals = sig.saturating_sub(int_digits);
    let s = format!("{v:.decimals$}");
    // Rounding may add a digit (9.99 -> 10.0): recompute decimals for the rounded value.
    let rounded: f64 = s.parse().unwrap_or(v);
    let int_digits2 = (rounded.abs().log10().floor() as i64 + 1).max(1) as usize;
    if int_digits2 != int_digits {
        let decimals = sig.saturating_sub(int_digits2);
        return format!("{rounded:.decimals$}");
    }
    s
}

/// Like `sig_digits` but the integer part uses thousands separators.
fn sig_thousands(v: f64, sig: usize) -> String {
    let s = sig_digits(v, sig);
    match s.split_once('.') {
        Some((i, d)) => format!("{}.{}", thousands(i.parse().unwrap_or(0)), d),
        None => thousands(s.parse().unwrap_or(0)),
    }
}

/// `FileSize/String1..4` and `/String` (= String3): binary units, 1–4 significant digits.
pub fn size_strings(bytes: u64) -> [String; 5] {
    let units = ["Byte", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    let unit = if u == 0 { if bytes == 1 { "Byte" } else { "Bytes" } } else { units[u] };
    let s1 = format!("{} {unit}", v.round() as u64);
    let s2 = format!("{} {unit}", sig_digits(v, 2));
    let s3 = format!("{} {unit}", sig_digits(v, 3));
    let s4 = format!("{} {unit}", sig_digits(v, 4));
    [s3.clone(), s1, s2, s3, s4]
}

/// `Duration/String` family from milliseconds. `fps` enables String4/String5 (frames).
/// Returns (String, String1, String2, String3, String4, String5).
pub fn duration_strings(ms: f64, fps: Option<f64>) -> [String; 6] {
    let neg = ms < 0.0;
    let total = ms.abs().round() as u64;
    let h = total / 3_600_000;
    let min = (total / 60_000) % 60;
    let s = (total / 1000) % 60;
    let msr = total % 1000;
    let units: Vec<(u64, &str)> = vec![(h, "h"), (min, "min"), (s, "s"), (msr, "ms")];
    let first = units.iter().position(|(v, _)| *v > 0).unwrap_or(3);
    let part = |v: u64, u: &str| format!("{v} {u}");
    let s1 = units[first..].iter().map(|(v, u)| part(*v, u)).collect::<Vec<_>>().join(" ");
    let s2 = units[first..(first + 2).min(4)].iter().map(|(v, u)| part(*v, u)).collect::<Vec<_>>().join(" ");
    let sign = if neg { "-" } else { "" };
    let s3 = format!("{sign}{h:02}:{min:02}:{s:02}.{msr:03}");
    let s4 = match fps {
        Some(f) if f > 0.0 => {
            let frames = ((msr as f64) * f / 1000.0).floor() as u64;
            format!("{sign}{h:02}:{min:02}:{s:02}:{frames:02}")
        }
        _ => String::new(),
    };
    let s5 = if s4.is_empty() { s3.clone() } else { format!("{s3} ({s4})") };
    let s1 = format!("{sign}{s1}");
    let s2 = format!("{sign}{s2}");
    [s2.clone(), s1, s2, s3, s4, s5]
}

/// `BitRate/String`: b/s below 10 kb/s, kb/s below 10 Mb/s, then Mb/s and Gb/s.
pub fn bitrate_string(bps: f64) -> String {
    let v = bps.abs();
    if v < 10_000.0 {
        format!("{} b/s", thousands(v.round() as u64))
    } else if v < 10_000_000.0 {
        format!("{} kb/s", sig_thousands(v / 1000.0, 3))
    } else if v < 10_000_000_000.0 {
        format!("{} Mb/s", sig_thousands(v / 1_000_000.0, 3))
    } else {
        format!("{} Gb/s", sig_thousands(v / 1_000_000_000.0, 3))
    }
}

/// `SamplingRate/String`: Hz below 10 kHz, else kHz with 3 significant digits.
pub fn sampling_rate_string(hz: f64) -> String {
    if hz < 10_000.0 {
        format!("{} Hz", thousands(hz.round() as u64))
    } else {
        format!("{} kHz", sig_thousands(hz / 1000.0, 3))
    }
}

pub fn pixels_string(n: u64) -> String {
    format!("{} pixels", thousands(n))
}

pub fn channels_string(n: u64) -> String {
    if n == 1 { "1 channel".to_string() } else { format!("{} channels", thousands(n)) }
}

pub fn bits_string(n: u64) -> String {
    if n == 1 { "1 bit".to_string() } else { format!("{n} bits") }
}

pub fn frames_string(n: u64) -> String {
    if n == 1 { "1 frame".to_string() } else { format!("{n} frames") }
}

/// `FrameRate/String`: 3 decimals with thousands separator, optional samples-per-frame suffix.
pub fn frame_rate_string(fps: f64, spf: Option<u64>) -> String {
    if !fps.is_finite() {
        return String::new();
    }
    let s = format!("{fps:.3}");
    let (i, d) = s.split_once('.').unwrap();
    let base = format!("{}.{} FPS", thousands(i.parse().unwrap_or(0)), d);
    match spf {
        Some(n) => format!("{base} ({n} SPF)"),
        None => base,
    }
}

/// `DisplayAspectRatio/String`: common ratios by name, otherwise `x.xxx:1`.
pub fn aspect_ratio_string(ratio: f64) -> String {
    let named: &[(f64, &str)] = &[
        (1.0, "1.000"),
        (1.2, "6:5"),
        (1.222, "11:9"),
        (1.25, "5:4"),
        (1.333, "4:3"),
        (1.5, "3:2"),
        (1.6, "16:10"),
        (1.667, "5:3"),
        (1.778, "16:9"),
        (1.85, "1.85:1"),
        (2.2, "2.2:1"),
        (2.35, "2.35:1"),
        (2.4, "2.40:1"),
    ];
    for (v, name) in named {
        if (ratio - v).abs() < 0.011 {
            return name.to_string();
        }
    }
    format!("{ratio:.3}:1")
}

/// `x.xxx` with 3 decimals (the reference storage format for ratios and frame rates).
pub fn f3(v: f64) -> String {
    format!("{v:.3}")
}

/// Proportion with 5 decimals (`StreamSize_Proportion`).
pub fn proportion(part: f64, whole: f64) -> String {
    if whole <= 0.0 {
        return String::new();
    }
    format!("{:.5}", part / whole)
}

/// `Delay/String`: `530 ms` for values under a second, else the duration format.
pub fn delay_strings(ms: f64) -> [String; 6] {
    duration_strings(ms, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(size_strings(10285), ["10.0 KiB", "10 KiB", "10 KiB", "10.0 KiB", "10.04 KiB"]);
        assert_eq!(size_strings(1087), ["1.06 KiB", "1 KiB", "1.1 KiB", "1.06 KiB", "1.062 KiB"]);
        assert_eq!(size_strings(15), ["15.0 Bytes", "15 Bytes", "15 Bytes", "15.0 Bytes", "15.00 Bytes"]);
        assert_eq!(size_strings(114), ["114 Bytes", "114 Bytes", "114 Bytes", "114 Bytes", "114.0 Bytes"]);
        assert_eq!(size_strings(1638)[0], "1.60 KiB");
        assert_eq!(size_strings(1638)[1], "2 KiB");
        assert_eq!(size_strings(1_152_114)[0], "1.10 MiB");
    }

    #[test]
    fn durations() {
        let d = duration_strings(1021.0, None);
        assert_eq!(d[0], "1 s 21 ms");
        assert_eq!(d[3], "00:00:01.021");
        assert_eq!(d[5], "00:00:01.021");
        let d = duration_strings(1021.0, Some(25.0));
        assert_eq!(d[4], "00:00:01:00");
        assert_eq!(d[5], "00:00:01.021 (00:00:01:00)");
        assert_eq!(duration_strings(42.0, None)[0], "42 ms");
        assert_eq!(duration_strings(3_723_456.0, None)[0], "1 h 2 min");
        assert_eq!(duration_strings(3_723_456.0, None)[1], "1 h 2 min 3 s 456 ms");
        assert_eq!(duration_strings(3_723_456.0, None)[3], "01:02:03.456");
        assert_eq!(duration_strings(-3.0, None)[0], "-3 ms");
        assert_eq!(duration_strings(500.0, Some(25.0))[4], "00:00:00:12");
    }

    #[test]
    fn bitrates_and_rates() {
        assert_eq!(bitrate_string(80588.0), "80.6 kb/s");
        assert_eq!(bitrate_string(105664.0), "106 kb/s");
        assert_eq!(bitrate_string(1_067_424.0), "1 067 kb/s");
        assert_eq!(bitrate_string(4720.0), "4 720 b/s");
        assert_eq!(bitrate_string(28_800_000.0), "28.8 Mb/s");
        assert_eq!(sampling_rate_string(48000.0), "48.0 kHz");
        assert_eq!(sampling_rate_string(44100.0), "44.1 kHz");
        assert_eq!(sampling_rate_string(8000.0), "8 000 Hz");
        assert_eq!(frame_rate_string(46.875, Some(1024)), "46.875 FPS (1024 SPF)");
        assert_eq!(frame_rate_string(1200.0, Some(40)), "1 200.000 FPS (40 SPF)");
        assert_eq!(pixels_string(1920), "1 920 pixels");
        assert_eq!(aspect_ratio_string(1.333), "4:3");
        assert_eq!(aspect_ratio_string(1.778), "16:9");
        assert_eq!(aspect_ratio_string(2.0), "2.000:1");
        assert_eq!(aspect_ratio_string(1.768), "16:9");
        assert_eq!(proportion(4596.0, 10690.0), "0.42993");
    }
}

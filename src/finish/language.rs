//! ISO 639 language codes → names. `Language` is normalised to the 2-letter code when one exists.

/// (639-1, 639-2/T, 639-2/B, English name)
static LANGUAGES: &[(&str, &str, &str, &str)] = &[
    ("aa", "aar", "aar", "Afar"), ("ab", "abk", "abk", "Abkhazian"), ("af", "afr", "afr", "Afrikaans"),
    ("ak", "aka", "aka", "Akan"), ("sq", "sqi", "alb", "Albanian"), ("am", "amh", "amh", "Amharic"),
    ("ar", "ara", "ara", "Arabic"), ("an", "arg", "arg", "Aragonese"), ("hy", "hye", "arm", "Armenian"),
    ("as", "asm", "asm", "Assamese"), ("av", "ava", "ava", "Avaric"), ("ae", "ave", "ave", "Avestan"),
    ("ay", "aym", "aym", "Aymara"), ("az", "aze", "aze", "Azerbaijani"), ("ba", "bak", "bak", "Bashkir"),
    ("bm", "bam", "bam", "Bambara"), ("eu", "eus", "baq", "Basque"), ("be", "bel", "bel", "Belarusian"),
    ("bn", "ben", "ben", "Bengali"), ("bh", "bih", "bih", "Bihari"), ("bi", "bis", "bis", "Bislama"),
    ("bs", "bos", "bos", "Bosnian"), ("br", "bre", "bre", "Breton"), ("bg", "bul", "bul", "Bulgarian"),
    ("my", "mya", "bur", "Burmese"), ("ca", "cat", "cat", "Catalan"), ("ch", "cha", "cha", "Chamorro"),
    ("ce", "che", "che", "Chechen"), ("zh", "zho", "chi", "Chinese"), ("cu", "chu", "chu", "Church Slavic"),
    ("cv", "chv", "chv", "Chuvash"), ("kw", "cor", "cor", "Cornish"), ("co", "cos", "cos", "Corsican"),
    ("cr", "cre", "cre", "Cree"), ("cs", "ces", "cze", "Czech"), ("da", "dan", "dan", "Danish"),
    ("dv", "div", "div", "Divehi"), ("nl", "nld", "dut", "Dutch"), ("dz", "dzo", "dzo", "Dzongkha"),
    ("en", "eng", "eng", "English"), ("eo", "epo", "epo", "Esperanto"), ("et", "est", "est", "Estonian"),
    ("ee", "ewe", "ewe", "Ewe"), ("fo", "fao", "fao", "Faroese"), ("fj", "fij", "fij", "Fijian"),
    ("fi", "fin", "fin", "Finnish"), ("fr", "fra", "fre", "French"), ("fy", "fry", "fry", "Western Frisian"),
    ("ff", "ful", "ful", "Fulah"), ("ka", "kat", "geo", "Georgian"), ("de", "deu", "ger", "German"),
    ("gd", "gla", "gla", "Gaelic"), ("ga", "gle", "gle", "Irish"), ("gl", "glg", "glg", "Galician"),
    ("gv", "glv", "glv", "Manx"), ("el", "ell", "gre", "Greek"), ("gn", "grn", "grn", "Guarani"),
    ("gu", "guj", "guj", "Gujarati"), ("ht", "hat", "hat", "Haitian"), ("ha", "hau", "hau", "Hausa"),
    ("he", "heb", "heb", "Hebrew"), ("hz", "her", "her", "Herero"), ("hi", "hin", "hin", "Hindi"),
    ("ho", "hmo", "hmo", "Hiri Motu"), ("hr", "hrv", "hrv", "Croatian"), ("hu", "hun", "hun", "Hungarian"),
    ("ig", "ibo", "ibo", "Igbo"), ("is", "isl", "ice", "Icelandic"), ("io", "ido", "ido", "Ido"),
    ("ii", "iii", "iii", "Sichuan Yi"), ("iu", "iku", "iku", "Inuktitut"), ("ie", "ile", "ile", "Interlingue"),
    ("ia", "ina", "ina", "Interlingua"), ("id", "ind", "ind", "Indonesian"), ("ik", "ipk", "ipk", "Inupiaq"),
    ("it", "ita", "ita", "Italian"), ("jv", "jav", "jav", "Javanese"), ("ja", "jpn", "jpn", "Japanese"),
    ("kl", "kal", "kal", "Kalaallisut"), ("kn", "kan", "kan", "Kannada"), ("ks", "kas", "kas", "Kashmiri"),
    ("kr", "kau", "kau", "Kanuri"), ("kk", "kaz", "kaz", "Kazakh"), ("km", "khm", "khm", "Central Khmer"),
    ("ki", "kik", "kik", "Kikuyu"), ("rw", "kin", "kin", "Kinyarwanda"), ("ky", "kir", "kir", "Kirghiz"),
    ("kv", "kom", "kom", "Komi"), ("kg", "kon", "kon", "Kongo"), ("ko", "kor", "kor", "Korean"),
    ("kj", "kua", "kua", "Kuanyama"), ("ku", "kur", "kur", "Kurdish"), ("lo", "lao", "lao", "Lao"),
    ("la", "lat", "lat", "Latin"), ("lv", "lav", "lav", "Latvian"), ("li", "lim", "lim", "Limburgan"),
    ("ln", "lin", "lin", "Lingala"), ("lt", "lit", "lit", "Lithuanian"), ("lb", "ltz", "ltz", "Luxembourgish"),
    ("lu", "lub", "lub", "Luba-Katanga"), ("lg", "lug", "lug", "Ganda"), ("mk", "mkd", "mac", "Macedonian"),
    ("mh", "mah", "mah", "Marshallese"), ("ml", "mal", "mal", "Malayalam"), ("mi", "mri", "mao", "Maori"),
    ("mr", "mar", "mar", "Marathi"), ("ms", "msa", "may", "Malay"), ("mg", "mlg", "mlg", "Malagasy"),
    ("mt", "mlt", "mlt", "Maltese"), ("mn", "mon", "mon", "Mongolian"), ("na", "nau", "nau", "Nauru"),
    ("nv", "nav", "nav", "Navajo"), ("nr", "nbl", "nbl", "South Ndebele"), ("nd", "nde", "nde", "North Ndebele"),
    ("ng", "ndo", "ndo", "Ndonga"), ("ne", "nep", "nep", "Nepali"), ("nn", "nno", "nno", "Norwegian Nynorsk"),
    ("nb", "nob", "nob", "Norwegian Bokmal"), ("no", "nor", "nor", "Norwegian"), ("ny", "nya", "nya", "Chichewa"),
    ("oc", "oci", "oci", "Occitan"), ("oj", "oji", "oji", "Ojibwa"), ("or", "ori", "ori", "Oriya"),
    ("om", "orm", "orm", "Oromo"), ("os", "oss", "oss", "Ossetian"), ("pa", "pan", "pan", "Panjabi"),
    ("fa", "fas", "per", "Persian"), ("pi", "pli", "pli", "Pali"), ("pl", "pol", "pol", "Polish"),
    ("pt", "por", "por", "Portuguese"), ("ps", "pus", "pus", "Pushto"), ("qu", "que", "que", "Quechua"),
    ("rm", "roh", "roh", "Romansh"), ("ro", "ron", "rum", "Romanian"), ("rn", "run", "run", "Rundi"),
    ("ru", "rus", "rus", "Russian"), ("sg", "sag", "sag", "Sango"), ("sa", "san", "san", "Sanskrit"),
    ("si", "sin", "sin", "Sinhala"), ("sk", "slk", "slo", "Slovak"), ("sl", "slv", "slv", "Slovenian"),
    ("se", "sme", "sme", "Northern Sami"), ("sm", "smo", "smo", "Samoan"), ("sn", "sna", "sna", "Shona"),
    ("sd", "snd", "snd", "Sindhi"), ("so", "som", "som", "Somali"), ("st", "sot", "sot", "Southern Sotho"),
    ("es", "spa", "spa", "Spanish"), ("sc", "srd", "srd", "Sardinian"), ("sr", "srp", "srp", "Serbian"),
    ("ss", "ssw", "ssw", "Swati"), ("su", "sun", "sun", "Sundanese"), ("sw", "swa", "swa", "Swahili"),
    ("sv", "swe", "swe", "Swedish"), ("ty", "tah", "tah", "Tahitian"), ("ta", "tam", "tam", "Tamil"),
    ("tt", "tat", "tat", "Tatar"), ("te", "tel", "tel", "Telugu"), ("tg", "tgk", "tgk", "Tajik"),
    ("tl", "tgl", "tgl", "Tagalog"), ("th", "tha", "tha", "Thai"), ("bo", "bod", "tib", "Tibetan"),
    ("ti", "tir", "tir", "Tigrinya"), ("to", "ton", "ton", "Tonga"), ("tn", "tsn", "tsn", "Tswana"),
    ("ts", "tso", "tso", "Tsonga"), ("tk", "tuk", "tuk", "Turkmen"), ("tr", "tur", "tur", "Turkish"),
    ("tw", "twi", "twi", "Twi"), ("ug", "uig", "uig", "Uighur"), ("uk", "ukr", "ukr", "Ukrainian"),
    ("ur", "urd", "urd", "Urdu"), ("uz", "uzb", "uzb", "Uzbek"), ("ve", "ven", "ven", "Venda"),
    ("vi", "vie", "vie", "Vietnamese"), ("vo", "vol", "vol", "Volapuk"), ("cy", "cym", "wel", "Welsh"),
    ("wa", "wln", "wln", "Walloon"), ("wo", "wol", "wol", "Wolof"), ("xh", "xho", "xho", "Xhosa"),
    ("yi", "yid", "yid", "Yiddish"), ("yo", "yor", "yor", "Yoruba"), ("za", "zha", "zha", "Zhuang"),
    ("zu", "zul", "zul", "Zulu"),
    // 639-2 only
    ("", "fil", "fil", "Filipino"), ("", "yue", "yue", "Cantonese"), ("", "hak", "hak", "Hakka"),
    ("", "nan", "nan", "Min Nan"), ("", "ast", "ast", "Asturian"), ("", "mul", "mul", "Multiple languages"),
    ("", "und", "und", "Undetermined"), ("", "zxx", "zxx", "No linguistic content"), ("", "mis", "mis", "Uncoded languages"),
    ("", "haw", "haw", "Hawaiian"), ("", "ceb", "ceb", "Cebuano"), ("", "arc", "arc", "Aramaic"),
    ("", "grc", "grc", "Ancient Greek"), ("", "enm", "enm", "Middle English"), ("", "ang", "ang", "Old English"),
    ("", "scn", "scn", "Sicilian"), ("", "nap", "nap", "Neapolitan"), ("", "gsw", "gsw", "Swiss German"),
    ("", "tlh", "tlh", "Klingon"), ("", "jbo", "jbo", "Lojban"),
];

/// Resolve any 2- or 3-letter code (optionally with a region suffix, `en-US`) to the table entry.
pub fn lookup(code: &str) -> Option<&'static (&'static str, &'static str, &'static str, &'static str)> {
    let base = code.split(['-', '_']).next().unwrap_or(code).trim().to_ascii_lowercase();
    if base.is_empty() {
        return None;
    }
    LANGUAGES.iter().find(|(a, t, b, _)| *a == base || *t == base || *b == base)
}

/// Normalised code stored in `Language`: the 2-letter code when one exists, else the input as given.
pub fn normalize(code: &str) -> String {
    let (base, suffix) = match code.split_once(['-', '_']) {
        Some((b, s)) => (b, Some(s)),
        None => (code, None),
    };
    let norm = match lookup(base) {
        Some((a, _, _, _)) if !a.is_empty() => a.to_string(),
        Some((_, t, _, _)) => t.to_string(),
        None => base.to_ascii_lowercase(),
    };
    match suffix {
        Some(s) => format!("{norm}-{}", s.to_ascii_uppercase()),
        None => norm,
    }
}

/// `Language/String`, `/String1` (full name), `/String2` (2-letter), `/String3` (3-letter), `/String4` (2- or 3-letter).
pub fn strings(code: &str) -> [String; 5] {
    let (base, suffix) = match code.split_once(['-', '_']) {
        Some((b, s)) => (b, Some(s.to_ascii_uppercase())),
        None => (code, None),
    };
    match lookup(base) {
        Some((a, t, _b, name)) => {
            let region = suffix.as_deref().map(|s| format!("-{s}")).unwrap_or_default();
            let full = match suffix.as_deref() {
                Some(s) => format!("{name} ({s})"),
                None => name.to_string(),
            };
            let two = if a.is_empty() { String::new() } else { format!("{a}{region}") };
            let three = format!("{t}{region}");
            let four = if a.is_empty() { three.clone() } else { two.clone() };
            [full.clone(), full, two, three, four]
        }
        None => {
            let c = code.to_string();
            [c.clone(), c.clone(), String::new(), String::new(), c]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        assert_eq!(normalize("eng"), "en");
        assert_eq!(normalize("jpn"), "ja");
        assert_eq!(normalize("ger"), "de");
        assert_eq!(normalize("und"), "und");
        assert_eq!(strings("en"), ["English", "English", "en", "eng", "en"]);
        assert_eq!(strings("fil")[3], "fil");
        assert_eq!(strings("fil")[4], "fil");
        assert_eq!(strings("en-US")[0], "English (US)");
        assert_eq!(strings("xx")[0], "xx");
    }
}

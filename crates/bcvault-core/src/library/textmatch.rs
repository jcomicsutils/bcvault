use std::collections::HashSet;

/// Strips combining diacritics and converts accented Latin characters to their ASCII base.
pub fn strip_diacritics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        // Skip combining diacritical marks (U+0300 through U+036F)
        if ('\u{0300}'..='\u{036F}').contains(&c) {
            continue;
        }
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä'
            | 'Å' | 'Ā' | 'Ă' | 'Ą' => out.push('a'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'È' | 'É' | 'Ê' | 'Ë' | 'Ē'
            | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ĭ' | 'į' | 'Ì' | 'Í' | 'Î' | 'Ï' | 'Ī' | 'Ĭ' | 'Į' => {
                out.push('i')
            }
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö'
            | 'Ø' | 'Ō' | 'Ŏ' | 'Ő' => out.push('o'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ū'
            | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => out.push('u'),
            'ý' | 'ÿ' | 'ŷ' | 'Ý' | 'Ŷ' | 'Ÿ' => out.push('y'),
            'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' | 'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => out.push('c'),
            'ñ' | 'ń' | 'ņ' | 'ň' | 'Ñ' | 'Ń' | 'Ņ' | 'Ň' => out.push('n'),
            'ß' => out.push_str("ss"),
            'æ' | 'Æ' => out.push_str("ae"),
            'œ' | 'Œ' => out.push_str("oe"),
            'ð' | 'đ' | 'Ð' | 'Đ' => out.push('d'),
            'þ' | 'Þ' => out.push_str("th"),
            'ł' | 'Ł' => out.push('l'),
            'š' | 'ś' | 'ŝ' | 'ş' | 'Š' | 'Ś' | 'Ŝ' | 'Ş' => out.push('s'),
            'ž' | 'ź' | 'ż' | 'Ž' | 'Ź' | 'Ż' => out.push('z'),
            'ř' | 'ŕ' | 'Ř' | 'Ŕ' => out.push('r'),
            'ť' | 'Ť' => out.push('t'),
            other => out.push(other),
        }
    }
    out
}

/// Removes common noise tokens such as `[FLAC]`, `(Free Download)`, `V0`, `320`, trailing `(YYYY)`.
/// Keeps version words like "deluxe", "remaster", "edition".
pub fn strip_noise_tokens(s: &str) -> String {
    let mut working = s.to_string();

    // Remove bracketed noise tokens
    let bracketed_noise = [
        "[flac]",
        "(flac)",
        "[web]",
        "(web)",
        "[free download]",
        "(free download)",
        "[mp3]",
        "(mp3)",
        "[320]",
        "(320)",
        "[v0]",
        "(v0)",
        "[24bit]",
        "(24bit)",
        "[lossless]",
        "(lossless)",
    ];

    let lower = working.to_lowercase();
    for token in bracketed_noise {
        if let Some(pos) = lower.find(token) {
            let end = pos + token.len();
            working.replace_range(pos..end, " ");
        }
    }

    // Remove trailing 4-digit years in brackets or at end, e.g. "(2019)", "[2021]", " (1998)"
    let trimmed = working.trim_end();
    if (trimmed.ends_with(')') && trimmed.len() >= 6)
        || (trimmed.ends_with(']') && trimmed.len() >= 6)
    {
        let inside = &trimmed[trimmed.len() - 5..trimmed.len() - 1];
        if matches!(inside.parse::<u32>(), Ok(1900..=2099)) {
            working = trimmed[..trimmed.len() - 6].to_string();
        }
    }

    // Split words and filter standalone noise words like "flac", "v0", "320"
    let mut kept_words = Vec::new();
    for word in working.split_whitespace() {
        let w_clean = word
            .trim_matches(|c: char| c.is_ascii_punctuation())
            .to_lowercase();
        if w_clean == "flac" || w_clean == "v0" || w_clean == "320" || w_clean == "web" {
            continue;
        }
        kept_words.push(word);
    }

    kept_words.join(" ")
}

/// Full Unicode text normalisation for library and mail item matching:
/// 1. Map `&` to `and`
/// 2. Strip diacritics for comparison
/// 3. Remove noise tokens
/// 4. Case fold
/// 5. Drop punctuation
/// 6. Collapse whitespace
pub fn normalize(text: &str) -> String {
    // 1. Map & to and
    let with_and = text.replace('&', " and ");

    // 2. Strip diacritics
    let decomposed = strip_diacritics(&with_and);

    // 3. Remove noise tokens
    let cleaned = strip_noise_tokens(&decomposed);

    // 4. Case fold and 5. Drop punctuation
    let mut out = String::with_capacity(cleaned.len());
    for c in cleaned.chars() {
        if c == '\'' || c == '’' || c == '‘' {
            // Keep apostrophe contracted or skip; dropping it produces e.g. "dont"
            continue;
        }
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
        } else {
            out.push(' ');
        }
    }

    // 6. Collapse whitespace
    let words: Vec<&str> = out.split_whitespace().collect();
    words.join(" ")
}

/// Generates expected filename and path variants for an album:
/// `{artist} - {title}`, `{title}`, `{artist}/{title}`
pub fn name_variants(artist: &str, title: &str) -> Vec<String> {
    vec![
        format!("{artist} - {title}"),
        title.to_string(),
        format!("{artist}/{title}"),
    ]
}

/// Returns unique normalized keys of `name_variants`.
pub fn name_variant_keys(artist: &str, title: &str) -> Vec<String> {
    let mut set = HashSet::new();
    let mut keys = Vec::new();

    for variant in name_variants(artist, title) {
        let key = normalize(&variant);
        if !key.is_empty() && set.insert(key.clone()) {
            keys.push(key);
        }
    }

    let title_key = normalize(title);
    if !title_key.is_empty() && set.insert(title_key.clone()) {
        keys.push(title_key);
    }

    keys
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_nfc_and_nfd_identically() {
        let nfc = "Caf\u{00E9}"; // é as single codepoint
        let nfd = "Cafe\u{0301}"; // e + combining acute
        assert_eq!(normalize(nfc), "cafe");
        assert_eq!(normalize(nfd), "cafe");
        assert_eq!(normalize(nfc), normalize(nfd));
    }

    #[test]
    fn maps_ampersand_to_and() {
        assert_eq!(normalize("Simon & Garfunkel"), "simon and garfunkel");
        assert_eq!(normalize("Fish&Chips"), "fish and chips");
    }

    #[test]
    fn strips_diacritics_and_special_latin() {
        assert_eq!(normalize("Böhse Onkelz"), "bohse onkelz");
        assert_eq!(normalize("Sigur Rós"), "sigur ros");
        assert_eq!(normalize("Mötley Crüe"), "motley crue");
        assert_eq!(normalize("Dvořák"), "dvorak");
        assert_eq!(normalize("Faustrecht Straße"), "faustrecht strasse");
    }

    #[test]
    fn strips_noise_tokens_and_trailing_year() {
        assert_eq!(
            normalize("Selected Ambient Works [FLAC]"),
            "selected ambient works"
        );
        assert_eq!(
            normalize("Selected Ambient Works (2019)"),
            "selected ambient works"
        );
        assert_eq!(
            normalize("Selected Ambient Works [1992]"),
            "selected ambient works"
        );
        assert_eq!(normalize("Free Single (Free Download)"), "free single");
        assert_eq!(normalize("Album [WEB] 320"), "album");
        assert_eq!(normalize("Album V0"), "album");
    }

    #[test]
    fn preserves_version_distinctions() {
        assert_eq!(
            normalize("In Rainbows (Deluxe Edition)"),
            "in rainbows deluxe edition"
        );
        assert_eq!(
            normalize("OK Computer (Remastered)"),
            "ok computer remastered"
        );
        assert_eq!(normalize("Live at Pompeii"), "live at pompeii");
    }

    #[test]
    fn collapses_whitespace_and_punctuation() {
        assert_eq!(normalize("  Hello ...   World!  "), "hello world");
        assert_eq!(
            normalize("Don't Panic: A Hitchhiker's Guide"),
            "dont panic a hitchhikers guide"
        );
    }

    #[test]
    fn name_variants_and_keys() {
        let vars = name_variants("Radiohead", "Kid A");
        assert_eq!(vars, vec!["Radiohead - Kid A", "Kid A", "Radiohead/Kid A"]);

        let keys = name_variant_keys("Radiohead", "Kid A");
        assert!(keys.contains(&"radiohead kid a".to_string()));
        assert!(keys.contains(&"kid a".to_string()));
    }
}

use std::borrow::Cow;

fn invisible_marker(c: char) -> Option<Cow<'static, str>> {
    if c == ' ' {
        return None;
    }
    // Preserve emoji/CJK variation selectors so normal emoji sequences render cleanly.
    if ('\u{FE00}'..='\u{FE0F}').contains(&c) || ('\u{E0100}'..='\u{E01EF}').contains(&c) {
        return None;
    }
    let named = match c {
        '\t' => Some("[TAB]"),
        '\n' => Some("[LF]"),
        '\r' => Some("[CR]"),
        '\u{00A0}' => Some("[NBSP]"),
        '\u{00AD}' => Some("[SHY]"),
        '\u{200B}' => Some("[ZWSP]"),
        '\u{200C}' => Some("[ZWNJ]"),
        '\u{200D}' => Some("[ZWJ]"),
        '\u{200E}' => Some("[LRM]"),
        '\u{200F}' => Some("[RLM]"),
        '\u{2060}' => Some("[WJ]"),
        '\u{FEFF}' => Some("[BOM]"),
        '\u{2800}' => Some("[BRAILLE-BLANK]"),
        '\u{3000}' => Some("[IDSP]"),
        '\u{3164}' | '\u{FFA0}' => Some("[FILLER]"),
        _ => None,
    };
    if let Some(m) = named {
        return Some(Cow::Borrowed(m));
    }

    let is_format_or_invisible = c.is_whitespace()
        || c.is_control()
        || matches!(
            c,
            '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{180E}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2061}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
        );
    if is_format_or_invisible {
        Some(Cow::Owned(format!("[U+{:04X}]", c as u32)))
    } else {
        None
    }
}

/// Replaces non-space invisible, zero-width, control, and non-ASCII whitespace
/// characters with readable ASCII markers when `mark_invisible` is enabled.
/// Returns `Cow::Borrowed(text)` with zero allocation otherwise.
pub fn format_display_text(text: &str, mark_invisible: bool) -> Cow<'_, str> {
    if !mark_invisible || !text.chars().any(|c| invisible_marker(c).is_some()) {
        return Cow::Borrowed(text);
    }

    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        if let Some(marker) = invisible_marker(c) {
            out.push_str(&marker);
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

const FONT_LAYERS: &[(&str, &[&str])] = &[
    (
        "os_unicode_fallback",
        &[
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "C:\\Windows\\Fonts\\segoeui.ttf",
            "/System/Library/Fonts/SFNS.ttf",
        ],
    ),
    (
        "os_symbols_fallback",
        &[
            "/usr/share/fonts/noto/NotoSansSymbols-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansSymbols-Regular.ttf",
            "C:\\Windows\\Fonts\\seguisym.ttf",
            "/System/Library/Fonts/Apple Symbols.ttf",
        ],
    ),
    (
        "os_symbols2_fallback",
        &[
            "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/TTF/Symbola.ttf",
            "/usr/share/fonts/truetype/ancient-scripts/Symbola_hint.ttf",
        ],
    ),
    (
        "os_math_fallback",
        &[
            "/usr/share/fonts/noto/NotoSansMath-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansMath-Regular.ttf",
            "C:\\Windows\\Fonts\\cambria.ttc",
        ],
    ),
    (
        "os_cjk_fallback",
        &[
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "C:\\Windows\\Fonts\\msyh.ttc",
            "C:\\Windows\\Fonts\\YuGothR.ttc",
            "/System/Library/Fonts/PingFang.ttc",
        ],
    ),
    (
        "os_arabic_fallback",
        &[
            "/usr/share/fonts/noto/NotoSansArabic-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
        ],
    ),
    (
        "os_hebrew_fallback",
        &[
            "/usr/share/fonts/noto/NotoSansHebrew-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf",
        ],
    ),
    (
        "os_devanagari_fallback",
        &[
            "/usr/share/fonts/noto/NotoSansDevanagari-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansDevanagari-Regular.ttf",
        ],
    ),
    (
        "os_thai_fallback",
        &[
            "/usr/share/fonts/noto/NotoSansThai-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansThai-Regular.ttf",
        ],
    ),
];

/// Discovers and reads up to one system font per Unicode coverage layer
/// (general Unicode, symbols, symbols2, math, CJK, regional scripts).
pub fn discover_fallback_fonts() -> Vec<(&'static str, Vec<u8>)> {
    let mut loaded = Vec::new();
    for &(layer_key, candidates) in FONT_LAYERS {
        for &path in candidates {
            if let Ok(bytes) = std::fs::read(path) {
                loaded.push((layer_key, bytes));
                break;
            }
        }
    }
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn disabled_returns_borrowed_unchanged() {
        let s = "Track\u{200B}Name\u{2800}!";
        let out = format_display_text(s, false);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, s);
    }

    #[test]
    fn preserves_ascii_space_and_variation_selectors() {
        let s = "Hello World \u{2764}\u{FE0F}";
        let out = format_display_text(s, true);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, s);
    }

    #[test]
    fn marks_non_space_invisible_characters() {
        let s = "A\u{200B}B\u{200D}C\u{FEFF}D\u{00A0}E\u{2800}F\u{3000}G\tH\u{202E}I";
        let out = format_display_text(s, true);
        assert_eq!(
            out,
            "A[ZWSP]B[ZWJ]C[BOM]D[NBSP]E[BRAILLE-BLANK]F[IDSP]G[TAB]H[U+202E]I"
        );
    }

    #[test]
    fn discover_fallback_fonts_uses_unique_layer_keys() {
        let loaded = discover_fallback_fonts();
        let keys: HashSet<&'static str> = loaded.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys.len(), loaded.len());
    }
}

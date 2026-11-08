use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

/// Canonical tokenization per MATCH-FIX spec 3.1:
/// 1. Lowercase (`str::to_lowercase`).
/// 2. Canonical decomposition (NFD) using unicode-normalization, then drop combining marks.
/// 3. Map letters that do not decompose: ø to o, ł to l, đ/ð to d, ħ to h, ı to i, ß to ss, æ to ae, œ to oe, þ to th.
/// 4. Delete apostrophes: U+0027, U+2019, U+2018, U+02BC (so Don't becomes dont).
/// 5. Every other non-alphanumeric character becomes a separator.
/// 6. Split on separators and discard empty tokens.
pub fn tokens(s: &str) -> Vec<String> {
    let lower = s.to_lowercase();
    let nfd: String = lower.nfd().filter(|&c| !is_combining_mark(c)).collect();

    let mut mapped = String::with_capacity(nfd.len());
    for c in nfd.chars() {
        match c {
            'ø' => mapped.push('o'),
            'ł' => mapped.push('l'),
            'đ' | 'ð' => mapped.push('d'),
            'ħ' => mapped.push('h'),
            'ı' => mapped.push('i'),
            'ß' => mapped.push_str("ss"),
            'æ' => mapped.push_str("ae"),
            'œ' => mapped.push_str("oe"),
            'þ' => mapped.push_str("th"),
            other => mapped.push(other),
        }
    }

    let mut words = Vec::new();
    let mut current = String::new();

    for c in mapped.chars() {
        if c == '\'' || c == '\u{2019}' || c == '\u{2018}' || c == '\u{02BC}' {
            continue;
        }
        if c.is_alphanumeric() {
            current.push(c);
        } else if !current.is_empty() {
            words.push(current);
            current = String::new();
        }
    }
    if !current.is_empty() {
        words.push(current);
    }

    words
}

/// Token sequence joined by a single space.
pub fn key(s: &str) -> String {
    tokens(s).join(" ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_textmatch_tokens_and_key() {
        // Case folding and whitespace collapse
        assert_eq!(
            tokens("  Radiohead  -  KID  A  "),
            vec!["radiohead", "kid", "a"]
        );
        assert_eq!(key("  Radiohead  -  KID  A  "), "radiohead kid a");

        // Canonical decomposition (NFD) + combining marks dropped
        assert_eq!(key("Caf\u{00E9}"), "cafe");
        assert_eq!(key("Cafe\u{0301}"), "cafe");
        assert_eq!(key("Sigur R\u{00F3}s"), "sigur ros");
        assert_eq!(key("Bj\u{00F6}rk"), "bjork");

        // Special letters mapping: ø, ł, đ, ð, ħ, ı, ß, æ, œ, þ
        assert_eq!(key("Ørnen"), "ornen");
        assert_eq!(key("Łódź"), "lodz");
        assert_eq!(key("Straße"), "strasse");
        assert_eq!(key("Dvořák"), "dvorak");
        assert_eq!(key("Ænima"), "aenima");
        assert_eq!(key("Cœur"), "coeur");
        assert_eq!(key("Þórr"), "thorr");
        assert_eq!(key("Đorđe"), "dorde");
        assert_eq!(key("ħabib"), "habib");
        assert_eq!(key("kapı"), "kapi");

        // Apostrophe deletion: U+0027, U+2019, U+2018, U+02BC
        assert_eq!(key("Don't"), "dont");
        assert_eq!(key("Don’t"), "dont");
        assert_eq!(key("‘Cause"), "cause");
        assert_eq!(key("Hawai\u{02BC}i"), "hawaii");

        // Separators & non-alphanumeric
        assert_eq!(key("Rock & Roll! (Vol. 1)"), "rock roll vol 1");
        assert_eq!(key("こんにちは 世界"), "こんにちは 世界"); // CJK preserved

        // Empty keys for symbols-only
        assert!(tokens("!!!").is_empty());
        assert_eq!(key("!!!"), "");
        assert_eq!(key("△"), "");
    }
}

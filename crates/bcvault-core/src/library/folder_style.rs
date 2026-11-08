use crate::library::textmatch::{key, tokens};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub title_key: String,
    pub year: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderStyle {
    #[serde(default = "default_templates")]
    pub templates: Vec<String>,
    #[serde(default)]
    pub noise: Vec<String>,
}

fn default_templates() -> Vec<String> {
    vec!["{title}".to_string()]
}

impl Default for FolderStyle {
    fn default() -> Self {
        Self {
            templates: default_templates(),
            noise: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Title,
    Artist,
    Year,
    Noise,
    Literal(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedTemplate {
    before_title: Vec<Segment>,
    after_title: Vec<Segment>,
    has_year: bool,
}

impl FolderStyle {
    /// Validates grammar, placeholders, length limits, and noise entries.
    pub fn validate(&self) -> Result<(), String> {
        if self.templates.len() > 8 {
            return Err("At most 8 templates are allowed".to_string());
        }

        for (idx, tmpl) in self.templates.iter().enumerate() {
            let trimmed = tmpl.trim();
            if trimmed.is_empty() {
                return Err(format!("Template #{} cannot be empty", idx + 1));
            }
            if tmpl.len() > 128 {
                return Err(format!(
                    "Template #{} exceeds 128 characters ({})",
                    idx + 1,
                    tmpl.len()
                ));
            }
            Self::parse_template(tmpl).map_err(|e| format!("Template #{}: {}", idx + 1, e))?;
        }

        if self.noise.len() > 32 {
            return Err("At most 32 noise entries are allowed".to_string());
        }

        for (idx, entry) in self.noise.iter().enumerate() {
            if entry.len() > 64 {
                return Err(format!(
                    "Noise entry #{} exceeds 64 characters ({})",
                    idx + 1,
                    entry.len()
                ));
            }
            if tokens(entry).is_empty() {
                return Err(format!(
                    "Noise entry #{} produces no alphanumeric tokens",
                    idx + 1
                ));
            }
        }

        Ok(())
    }

    /// Non-blocking hints for the user (e.g., `{noise}` used with empty noise list).
    pub fn hints(&self) -> Vec<String> {
        let mut hints = Vec::new();
        let noise_empty = self.cleaned_noise().is_empty();

        for tmpl in &self.templates {
            if let Ok(parsed) = Self::parse_template(tmpl) {
                let uses_noise = parsed
                    .before_title
                    .iter()
                    .chain(parsed.after_title.iter())
                    .any(|s| matches!(s, Segment::Noise));
                if uses_noise && noise_empty {
                    hints.push(format!(
                        "Template '{}' uses {{noise}}, but the noise list is empty and so will never match",
                        tmpl
                    ));
                }
            }
        }

        hints
    }

    /// Cleans and deduplicates noise entries (compared by tokens, preserving first).
    pub fn cleaned_noise(&self) -> Vec<Vec<String>> {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();

        for entry in &self.noise {
            let tok = tokens(entry);
            if !tok.is_empty() {
                let key_str = tok.join(" ");
                if seen.insert(key_str) {
                    result.push(tok);
                }
            }
        }

        // Sort longest (most tokens) first
        result.sort_by_key(|a| std::cmp::Reverse(a.len()));
        result
    }

    /// Parses a single template into segments before and after `{title}`.
    fn parse_template(tmpl: &str) -> Result<ParsedTemplate, String> {
        let mut chars = tmpl.chars().peekable();
        let mut raw_segments = Vec::new();
        let mut current_lit = String::new();

        while let Some(c) = chars.next() {
            if c == '{' {
                if !current_lit.is_empty() {
                    let tok = tokens(&current_lit);
                    if !tok.is_empty() {
                        raw_segments.push(Segment::Literal(tok));
                    }
                    current_lit.clear();
                }

                let mut ph = String::new();
                let mut closed = false;
                for inner in chars.by_ref() {
                    if inner == '}' {
                        closed = true;
                        break;
                    } else if inner == '{' {
                        return Err("Unbalanced or nested '{' inside placeholder".to_string());
                    } else {
                        ph.push(inner);
                    }
                }
                if !closed {
                    return Err("Unclosed placeholder '{'".to_string());
                }

                match ph.trim().to_lowercase().as_str() {
                    "title" => raw_segments.push(Segment::Title),
                    "artist" => raw_segments.push(Segment::Artist),
                    "year" => raw_segments.push(Segment::Year),
                    "noise" => raw_segments.push(Segment::Noise),
                    unknown => return Err(format!("Unknown placeholder '{{{unknown}}}'")),
                }
            } else if c == '}' {
                return Err("Unmatched '}' without opening '{'".to_string());
            } else {
                current_lit.push(c);
            }
        }

        if !current_lit.is_empty() {
            let tok = tokens(&current_lit);
            if !tok.is_empty() {
                raw_segments.push(Segment::Literal(tok));
            }
        }

        let title_count = raw_segments
            .iter()
            .filter(|s| matches!(s, Segment::Title))
            .count();
        if title_count != 1 {
            return Err("Template must contain {title} exactly once".to_string());
        }

        let artist_count = raw_segments
            .iter()
            .filter(|s| matches!(s, Segment::Artist))
            .count();
        if artist_count > 1 {
            return Err("Placeholder {artist} can appear at most once".to_string());
        }

        let year_count = raw_segments
            .iter()
            .filter(|s| matches!(s, Segment::Year))
            .count();
        if year_count > 1 {
            return Err("Placeholder {year} can appear at most once".to_string());
        }

        let noise_count = raw_segments
            .iter()
            .filter(|s| matches!(s, Segment::Noise))
            .count();
        if noise_count > 1 {
            return Err("Placeholder {noise} can appear at most once".to_string());
        }

        let mut before_title = Vec::new();
        let mut after_title = Vec::new();
        let mut past_title = false;

        for seg in raw_segments {
            if matches!(seg, Segment::Title) {
                past_title = true;
            } else if past_title {
                after_title.push(seg);
            } else {
                before_title.push(seg);
            }
        }

        Ok(ParsedTemplate {
            before_title,
            after_title,
            has_year: year_count == 1,
        })
    }

    /// Fits a parsed template against folder name tokens and artist name tokens.
    fn fit_parsed(
        parsed: &ParsedTemplate,
        folder_tokens: &[String],
        artist_tokens: &[String],
        noise_list: &[Vec<String>],
    ) -> Option<Extracted> {
        let mut start_idx = 0;
        let mut end_idx = folder_tokens.len();
        let mut extracted_year = None;

        // Consume segments before title from the left
        for seg in &parsed.before_title {
            match seg {
                Segment::Title => unreachable!(),
                Segment::Artist => {
                    if artist_tokens.is_empty() {
                        return None;
                    }
                    let len = artist_tokens.len();
                    if start_idx + len > end_idx {
                        return None;
                    }
                    if &folder_tokens[start_idx..start_idx + len] != artist_tokens {
                        return None;
                    }
                    start_idx += len;
                }
                Segment::Year => {
                    if start_idx >= end_idx {
                        return None;
                    }
                    let tok = &folder_tokens[start_idx];
                    if tok.len() == 4 && tok.chars().all(|c| c.is_ascii_digit()) {
                        let y: u32 = tok.parse().ok()?;
                        if (1900..=2099).contains(&y) {
                            extracted_year = Some(y);
                            start_idx += 1;
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                Segment::Literal(lit_tokens) => {
                    let len = lit_tokens.len();
                    if start_idx + len > end_idx {
                        return None;
                    }
                    if &folder_tokens[start_idx..start_idx + len] != lit_tokens.as_slice() {
                        return None;
                    }
                    start_idx += len;
                }
                Segment::Noise => {
                    let mut matched_len = None;
                    for noise_entry in noise_list {
                        let len = noise_entry.len();
                        if start_idx + len <= end_idx
                            && &folder_tokens[start_idx..start_idx + len] == noise_entry.as_slice()
                        {
                            matched_len = Some(len);
                            break;
                        }
                    }
                    let len = matched_len?;
                    start_idx += len;
                }
            }
        }

        // Consume segments after title from the right (iterated in reverse from the end)
        for seg in parsed.after_title.iter().rev() {
            match seg {
                Segment::Title => unreachable!(),
                Segment::Artist => {
                    if artist_tokens.is_empty() {
                        return None;
                    }
                    let len = artist_tokens.len();
                    if end_idx < start_idx + len {
                        return None;
                    }
                    if &folder_tokens[end_idx - len..end_idx] != artist_tokens {
                        return None;
                    }
                    end_idx -= len;
                }
                Segment::Year => {
                    if end_idx <= start_idx {
                        return None;
                    }
                    let tok = &folder_tokens[end_idx - 1];
                    if tok.len() == 4 && tok.chars().all(|c| c.is_ascii_digit()) {
                        let y: u32 = tok.parse().ok()?;
                        if (1900..=2099).contains(&y) {
                            extracted_year = Some(y);
                            end_idx -= 1;
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                Segment::Literal(lit_tokens) => {
                    let len = lit_tokens.len();
                    if end_idx < start_idx + len {
                        return None;
                    }
                    if &folder_tokens[end_idx - len..end_idx] != lit_tokens.as_slice() {
                        return None;
                    }
                    end_idx -= len;
                }
                Segment::Noise => {
                    let mut matched_len = None;
                    for noise_entry in noise_list {
                        let len = noise_entry.len();
                        if end_idx >= start_idx + len
                            && &folder_tokens[end_idx - len..end_idx] == noise_entry.as_slice()
                        {
                            matched_len = Some(len);
                            break;
                        }
                    }
                    let len = matched_len?;
                    end_idx -= len;
                }
            }
        }

        // Remainder is Title tokens; must be non-empty
        if start_idx < end_idx {
            let title_tokens = &folder_tokens[start_idx..end_idx];
            Some(Extracted {
                title_key: title_tokens.join(" "),
                year: extracted_year,
            })
        } else {
            None
        }
    }

    /// Extracts title and year using every valid template that fits.
    pub fn extract_all(&self, folder_name: &str, artist_name: &str) -> Vec<Extracted> {
        let folder_tok = tokens(folder_name);
        if folder_tok.is_empty() {
            return Vec::new();
        }
        let artist_tok = tokens(artist_name);
        let noise_list = self.cleaned_noise();

        let mut results = Vec::new();
        let mut seen_keys = std::collections::HashSet::new();

        // Valid templates, with fallback to default if none valid
        let mut valid_parsed = Vec::new();
        for tmpl in &self.templates {
            if let Ok(p) = Self::parse_template(tmpl) {
                valid_parsed.push(p);
            }
        }
        if valid_parsed.is_empty()
            && let Ok(p) = Self::parse_template("{title}")
        {
            valid_parsed.push(p);
        }

        for parsed in valid_parsed {
            if let Some(ext) = Self::fit_parsed(&parsed, &folder_tok, &artist_tok, &noise_list)
                && seen_keys.insert((ext.title_key.clone(), ext.year))
            {
                results.push(ext);
            }
        }

        results
    }

    /// Returns candidate folder names: raw folder key + all template extracted title keys (deduplicated).
    pub fn candidate_names(&self, folder_name: &str, artist_name: &str) -> Vec<String> {
        let mut names = Vec::new();
        let raw_key = key(folder_name);
        if !raw_key.is_empty() {
            names.push(raw_key.clone());
        }

        for ext in self.extract_all(folder_name, artist_name) {
            if !ext.title_key.is_empty() && !names.contains(&ext.title_key) {
                names.push(ext.title_key);
            }
        }

        names
    }

    /// Returns the extracted year from the first fitting template that extracted one.
    pub fn extracted_year(&self, folder_name: &str, artist_name: &str) -> Option<u32> {
        for ext in self.extract_all(folder_name, artist_name) {
            if let Some(y) = ext.year {
                return Some(y);
            }
        }
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_22_folder_style_parse_validate_and_fitting() {
        // Valid default
        let default_style = FolderStyle::default();
        assert!(default_style.validate().is_ok());

        // Unknown placeholder
        let bad_ph = FolderStyle {
            templates: vec!["{title} - {format}".to_string()],
            noise: vec![],
        };
        assert!(bad_ph.validate().is_err());

        // Missing {title}
        let missing_title = FolderStyle {
            templates: vec!["{artist} - {year}".to_string()],
            noise: vec![],
        };
        assert!(missing_title.validate().is_err());

        // Duplicate placeholder (e.g. two {title} or two {noise})
        let dup_title = FolderStyle {
            templates: vec!["{title} - {title}".to_string()],
            noise: vec![],
        };
        assert!(dup_title.validate().is_err());
        let dup_noise = FolderStyle {
            templates: vec!["{noise} - {title} - {noise}".to_string()],
            noise: vec!["FLAC".to_string()],
        };
        assert!(dup_noise.validate().is_err());

        // Unbalanced braces
        let unclosed = FolderStyle {
            templates: vec!["{title - {artist}".to_string()],
            noise: vec![],
        };
        assert!(unclosed.validate().is_err());
        let unmatched = FolderStyle {
            templates: vec!["{title}}".to_string()],
            noise: vec![],
        };
        assert!(unmatched.validate().is_err());

        // Length limits (>128 chars)
        let long_tmpl = FolderStyle {
            templates: vec![format!("{{title}} {}", "a".repeat(130))],
            noise: vec![],
        };
        assert!(long_tmpl.validate().is_err());

        // Count limits (>8 templates)
        let many_tmpls = FolderStyle {
            templates: (0..9).map(|_| "{title}".to_string()).collect(),
            noise: vec![],
        };
        assert!(many_tmpls.validate().is_err());

        // Template fitting with year bounds (1900..=2099)
        let year_style = FolderStyle {
            templates: vec!["{title} ({year})".to_string()],
            noise: vec![],
        };
        let valid_year = year_style.extract_all("Album [2019]", "Artist");
        assert_eq!(
            valid_year,
            vec![Extracted {
                title_key: "album".to_string(),
                year: Some(2019),
            }]
        );
        let out_of_bounds = year_style.extract_all("Album [1899]", "Artist");
        assert!(out_of_bounds.is_empty());
        let out_of_bounds_2 = year_style.extract_all("Album [2100]", "Artist");
        assert!(out_of_bounds_2.is_empty());
    }

    #[test]
    fn test_25_noise_list_exact_fitting() {
        let style = FolderStyle {
            templates: vec!["{title} ({noise})".to_string()],
            noise: vec!["FLAC".to_string(), "MP3".to_string()],
        };
        assert!(style.validate().is_ok());

        for folder in &["Album [FLAC]", "Album (mp3)", "Album - FLAC"] {
            let extracted = style.extract_all(folder, "Artist");
            assert_eq!(
                extracted,
                vec![Extracted {
                    title_key: "album".to_string(),
                    year: None,
                }],
                "Failed on folder: {folder}"
            );
        }
    }

    #[test]
    fn test_26_unlisted_noise_does_not_fit() {
        let style = FolderStyle {
            templates: vec!["{title} ({noise})".to_string()],
            noise: vec!["FLAC".to_string(), "MP3".to_string()],
        };
        let extracted = style.extract_all("Album [OGG]", "Artist");
        assert!(extracted.is_empty());
    }

    #[test]
    fn test_27_longest_noise_entry_wins() {
        let style = FolderStyle {
            templates: vec!["{title} ({noise})".to_string()],
            noise: vec!["FLAC".to_string(), "WEB FLAC".to_string()],
        };
        let extracted = style.extract_all("Album (WEB FLAC)", "Artist");
        assert_eq!(
            extracted,
            vec![Extracted {
                title_key: "album".to_string(),
                year: None,
            }]
        );
    }

    #[test]
    fn test_28_empty_noise_list_hints_and_no_fit() {
        let style = FolderStyle {
            templates: vec!["{title} ({noise})".to_string()],
            noise: vec![],
        };
        assert!(style.validate().is_ok());
        let hints = style.hints();
        assert_eq!(hints.len(), 1);
        assert!(hints[0].contains("empty"));

        let extracted = style.extract_all("Album [FLAC]", "Artist");
        assert!(extracted.is_empty());
        // But raw candidate name still present
        assert_eq!(
            style.candidate_names("Album [FLAC]", "Artist"),
            vec!["album flac".to_string()]
        );
    }

    #[test]
    fn test_29_noise_in_title_still_matches_raw() {
        let style = FolderStyle {
            templates: vec!["{title} ({noise})".to_string()],
            noise: vec!["MP3".to_string()],
        };
        let names = style.candidate_names("Best Of MP3", "Artist");
        assert!(names.contains(&"best of mp3".to_string()));
    }

    #[test]
    fn test_30_noise_entry_validation() {
        // Empty after tokenizing (e.g. only punctuation)
        let empty_entry = FolderStyle {
            templates: vec!["{title}".to_string()],
            noise: vec!["---".to_string()],
        };
        assert!(empty_entry.validate().is_err());

        // Over 64 chars
        let long_entry = FolderStyle {
            templates: vec!["{title}".to_string()],
            noise: vec!["a".repeat(65)],
        };
        assert!(long_entry.validate().is_err());

        // Over 32 entries
        let many_entries = FolderStyle {
            templates: vec!["{title}".to_string()],
            noise: (0..33).map(|i| format!("format{i}")).collect(),
        };
        assert!(many_entries.validate().is_err());

        // Cleaned noise drops duplicates by tokens
        let dup_style = FolderStyle {
            templates: vec!["{title}".to_string()],
            noise: vec![
                "FLAC".to_string(),
                "flac".to_string(),
                " [FLAC] ".to_string(),
            ],
        };
        assert_eq!(dup_style.cleaned_noise().len(), 1);
    }
}

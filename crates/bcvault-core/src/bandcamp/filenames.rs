use crate::types::{AudioFormat, ItemType, Ts};
use chrono::TimeZone;
use percent_encoding::percent_decode_str;

#[derive(Clone, Debug)]
pub struct ReleaseFormatContext<'a> {
    pub artist: &'a str,
    pub page_artist: &'a str,
    pub title: &'a str,
    pub published_at: Option<Ts>,
    pub kind: ItemType,
    pub format: AudioFormat,
    pub item_id: i64,
}

impl ReleaseFormatContext<'_> {
    pub fn year_str(&self) -> String {
        self.published_at
            .and_then(|ts| chrono::Utc.timestamp_opt(ts, 0).single())
            .map(|dt| dt.format("%Y").to_string())
            .unwrap_or_default()
    }

    pub fn date_str(&self) -> String {
        self.published_at
            .and_then(|ts| chrono::Utc.timestamp_opt(ts, 0).single())
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    }
}

/// Formats a custom filename stem from a pattern using standard tags:
/// `{artist}`, `{page_artist}`, `{album}`, `{title}`, `{year}`, `{date}`, `{format}`, `{type}`, `{id}`
/// (also supports `%artist%`, `%album%`, etc.).
/// Returns `None` if `pattern` is empty/whitespace.
pub fn format_release_stem(pattern: &str, ctx: &ReleaseFormatContext<'_>) -> Option<String> {
    let trimmed = pattern.trim();
    if trimmed.is_empty() {
        return None;
    }

    let year = ctx.year_str();
    let date = ctx.date_str();
    let id_str = ctx.item_id.to_string();
    let fmt_str = ctx.format.label();
    let type_str = ctx.kind.as_str();

    let replacements: &[(&str, &str)] = &[
        ("{artist}", ctx.artist),
        ("%artist%", ctx.artist),
        ("{album_artist}", ctx.artist),
        ("%album_artist%", ctx.artist),
        ("{page_artist}", ctx.page_artist),
        ("%page_artist%", ctx.page_artist),
        ("{album}", ctx.title),
        ("%album%", ctx.title),
        ("{title}", ctx.title),
        ("%title%", ctx.title),
        ("{year}", &year),
        ("%year%", &year),
        ("{date}", &date),
        ("%date%", &date),
        ("{format}", fmt_str),
        ("%format%", fmt_str),
        ("{type}", type_str),
        ("%type%", type_str),
        ("{id}", &id_str),
        ("%id%", &id_str),
    ];

    let mut expanded = replace_tags_case_insensitive(trimmed, replacements);
    expanded = clean_empty_separators(&expanded);
    Some(sanitize_stem(&expanded))
}

fn replace_tags_case_insensitive(input: &str, tags: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(input.len() + 32);
    let mut rest = input;
    while !rest.is_empty() {
        let mut matched = false;
        for &(tag, val) in tags {
            if rest.len() >= tag.len()
                && rest.is_char_boundary(tag.len())
                && rest[..tag.len()].eq_ignore_ascii_case(tag)
            {
                out.push_str(val);
                rest = &rest[tag.len()..];
                matched = true;
                break;
            }
        }
        if !matched && let Some(ch) = rest.chars().next() {
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

fn clean_empty_separators(s: &str) -> String {
    // Collapse empty brackets like "()", "[]" and dangling " - " separators when a tag (e.g. {year}) is empty
    let mut cur = s
        .replace("()", "")
        .replace("[]", "")
        .replace("( )", "")
        .replace("[ ]", "");
    while cur.contains(" -  - ") {
        cur = cur.replace(" -  - ", " - ");
    }
    let words: Vec<&str> = cur.split_whitespace().collect();
    let joined = words.join(" ");
    let trimmed = joined
        .trim_matches(|c: char| matches!(c, '-' | '_' | ' '))
        .trim();
    trimmed.to_string()
}

pub fn parse_content_disposition(header: &str) -> Option<String> {
    let mut star_val: Option<String> = None;
    let mut plain_val: Option<String> = None;

    for part in split_header_params(header) {
        let trimmed = part.trim();
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim();

        if key == "filename*" {
            let unquoted = unquote(val);
            if let Some(decoded) = decode_rfc5987(&unquoted)
                && !decoded.trim().is_empty()
            {
                star_val = Some(sanitize_filename(&decoded));
            }
        } else if key == "filename" {
            let unquoted = unquote(val);
            if !unquoted.trim().is_empty() {
                plain_val = Some(sanitize_filename(&unquoted));
            }
        }
    }

    star_val.or(plain_val)
}

pub fn resolve_download_filename(
    content_disposition: Option<&str>,
    custom_stem: Option<&str>,
    artist: &str,
    title: &str,
    kind: ItemType,
    ext: &str,
) -> String {
    let clean_ext = ext.trim_start_matches('.');
    let final_ext = if kind == ItemType::Album {
        "zip"
    } else {
        clean_ext
    };

    if let Some(stem_override) = custom_stem.filter(|s| !s.trim().is_empty()) {
        let stem = sanitize_stem(stem_override);
        return if final_ext.is_empty() {
            stem
        } else {
            truncate_with_ext(&stem, final_ext, 180)
        };
    }

    if let Some(cd) = content_disposition
        && let Some(parsed) = parse_content_disposition(cd)
    {
        return parsed;
    }

    let stem = sanitize_stem(&format!("{artist} - {title}"));
    if final_ext.is_empty() {
        stem
    } else {
        truncate_with_ext(&stem, final_ext, 180)
    }
}

pub fn sanitize_filename(raw: &str) -> String {
    // Keep only the last path component (split on '/' and '\')
    let last_comp = raw
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or("");

    let (raw_stem, raw_ext) = match last_comp.rfind('.') {
        Some(idx) if idx > 0 && idx < last_comp.len() - 1 => {
            (&last_comp[..idx], Some(&last_comp[idx + 1..]))
        }
        _ => (last_comp, None),
    };

    let stem = sanitize_stem(raw_stem);
    if let Some(ext) = raw_ext {
        let clean_ext = sanitize_component(ext);
        if !clean_ext.is_empty() {
            return truncate_with_ext(&stem, &clean_ext, 180);
        }
    }
    truncate_utf8(&stem, 180)
}

pub fn sanitize_stem(raw: &str) -> String {
    let cleaned = sanitize_component(raw);
    let stem = if cleaned.is_empty() {
        "download".to_string()
    } else if is_windows_reserved(&cleaned) {
        format!("_{cleaned}")
    } else {
        cleaned
    };
    truncate_utf8(&stem, 180)
}

fn sanitize_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let trimmed = out.trim().trim_end_matches(['.', ' ']).trim();
    trimmed.to_string()
}

fn is_windows_reserved(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn truncate_with_ext(stem: &str, ext: &str, max_bytes: usize) -> String {
    let suffix = format!(".{ext}");
    if suffix.len() >= max_bytes {
        return truncate_utf8(stem, max_bytes);
    }
    let max_stem = max_bytes - suffix.len();
    let t_stem = truncate_utf8(stem, max_stem);
    let t_stem = t_stem.trim_end_matches(['.', ' ']);
    let final_stem = if t_stem.is_empty() {
        "download"
    } else {
        t_stem
    };
    format!("{final_stem}{suffix}")
}

fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn split_header_params(header: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;

    for (idx, ch) in header.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && in_quotes {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_quotes = !in_quotes;
            continue;
        }
        if ch == ';' && !in_quotes {
            out.push(&header[start..idx]);
            start = idx + 1;
        }
    }
    if start <= header.len() {
        out.push(&header[start..]);
    }
    out
}

fn unquote(val: &str) -> String {
    let trimmed = val.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut escaped = false;
        for ch in inner.chars() {
            if escaped {
                out.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else {
                out.push(ch);
            }
        }
        out
    } else {
        trimmed.to_string()
    }
}

fn decode_rfc5987(val: &str) -> Option<String> {
    let mut parts = val.splitn(3, '\'');
    let charset = parts.next()?.trim().to_ascii_uppercase();
    let _lang = parts.next()?;
    let encoded = parts.next()?;

    let raw_bytes: Vec<u8> = percent_decode_str(encoded).collect();
    match charset.as_str() {
        "UTF-8" | "UTF8" | "" => String::from_utf8(raw_bytes).ok(),
        "ISO-8859-1" | "LATIN1" => Some(raw_bytes.into_iter().map(char::from).collect()),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_vectors() {
        assert_eq!(
            parse_content_disposition(
                "attachment; filename*=UTF-8''Sigur%20R%C3%B3s%20-%20%C3%81g%C3%A6tis.zip"
            )
            .as_deref(),
            Some("Sigur Rós - Ágætis.zip")
        );

        assert_eq!(
            parse_content_disposition(r#"attachment; filename="A - B.zip""#).as_deref(),
            Some("A - B.zip")
        );

        assert_eq!(
            parse_content_disposition(
                r#"filename="fallback.zip"; filename*=UTF-8''real%20name.zip"#
            )
            .as_deref(),
            Some("real name.zip")
        );

        assert_eq!(
            parse_content_disposition(r#"attachment; filename="../../evil.zip""#).as_deref(),
            Some("evil.zip")
        );

        assert_eq!(
            parse_content_disposition(r#"attachment; filename="CON.zip""#).as_deref(),
            Some("_CON.zip")
        );

        assert_eq!(
            resolve_download_filename(None, None, "Artist", "Title", ItemType::Album, "zip"),
            "Artist - Title.zip"
        );

        assert_eq!(
            resolve_download_filename(
                Some(r#"attachment; filename="ServerName.zip""#),
                Some("2023 - Custom Artist - Custom Album"),
                "Artist",
                "Title",
                ItemType::Album,
                "zip"
            ),
            "2023 - Custom Artist - Custom Album.zip"
        );
    }

    #[test]
    fn custom_pattern_formatting() {
        let ctx = ReleaseFormatContext {
            artist: "Release Artist",
            page_artist: "Page Artist",
            title: "Great Album",
            published_at: Some(1_700_000_000), // 2023-11-14
            kind: ItemType::Album,
            format: AudioFormat::Flac,
            item_id: 42,
        };

        assert_eq!(
            format_release_stem("{year} - {artist} - {album}", &ctx).as_deref(),
            Some("2023 - Release Artist - Great Album")
        );
        assert_eq!(
            format_release_stem("{page_artist} - {title} [{format}] ({date})", &ctx).as_deref(),
            Some("Page Artist - Great Album [FLAC] (2023-11-14)")
        );

        let no_date_ctx = ReleaseFormatContext {
            published_at: None,
            ..ctx
        };
        assert_eq!(
            format_release_stem("{year} - {artist} - {album}", &no_date_ctx).as_deref(),
            Some("Release Artist - Great Album")
        );
    }
}

use crate::motion::oklab::{contrast_ratio, relative_luminance};
use crate::theme::FxTheme;
use egui::Color32;
use std::fmt;

pub const MAX_THEME_FILE_BYTES: usize = 16 * 1024;
pub const THEME_FORMAT_VERSION: i64 = 1;
pub const THEME_FILE_HEADER: &str = "# bcvault theme, format 1. Share this file as is.";

pub const COLOR_KEYS: [&str; 12] = [
    "bg0",
    "bg1",
    "bg2",
    "border",
    "border_hi",
    "text",
    "text_dim",
    "accent",
    "accent2",
    "ok",
    "warn",
    "err",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeError {
    pub message: String,
}

impl ThemeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ThemeError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeColors {
    pub bg0: Color32,
    pub bg1: Color32,
    pub bg2: Color32,
    pub border: Color32,
    pub border_hi: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
}

impl ThemeColors {
    pub fn get(&self, key: &str) -> Option<Color32> {
        match key {
            "bg0" => Some(self.bg0),
            "bg1" => Some(self.bg1),
            "bg2" => Some(self.bg2),
            "border" => Some(self.border),
            "border_hi" => Some(self.border_hi),
            "text" => Some(self.text),
            "text_dim" => Some(self.text_dim),
            "accent" => Some(self.accent),
            "accent2" => Some(self.accent2),
            "ok" => Some(self.ok),
            "warn" => Some(self.warn),
            "err" => Some(self.err),
            _ => None,
        }
    }

    pub fn set(&mut self, key: &str, color: Color32) -> bool {
        match key {
            "bg0" => self.bg0 = color,
            "bg1" => self.bg1 = color,
            "bg2" => self.bg2 = color,
            "border" => self.border = color,
            "border_hi" => self.border_hi = color,
            "text" => self.text = color,
            "text_dim" => self.text_dim = color,
            "accent" => self.accent = color,
            "accent2" => self.accent2 = color,
            "ok" => self.ok = color,
            "warn" => self.warn = color,
            "err" => self.err = color,
            _ => return false,
        }
        true
    }

    pub fn from_fx_theme(theme: &FxTheme) -> Self {
        Self {
            bg0: theme.bg0,
            bg1: theme.bg1,
            bg2: theme.bg2,
            border: theme.border,
            border_hi: theme.border_hi,
            text: theme.text,
            text_dim: theme.text_dim,
            accent: theme.accent,
            accent2: theme.accent2,
            ok: theme.ok,
            warn: theme.warn,
            err: theme.err,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeFile {
    pub format: i64,
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub dark: bool,
    pub scanlines: bool,
    pub colors: ThemeColors,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeHint {
    pub rule: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedTheme {
    pub file: ThemeFile,
    pub warnings: Vec<String>,
}

impl ThemeFile {
    pub fn to_fx_theme(&self) -> FxTheme {
        FxTheme {
            bg0: self.colors.bg0,
            bg1: self.colors.bg1,
            bg2: self.colors.bg2,
            border: self.colors.border,
            border_hi: self.colors.border_hi,
            text: self.colors.text,
            text_dim: self.colors.text_dim,
            accent: self.colors.accent,
            accent2: self.colors.accent2,
            ok: self.colors.ok,
            warn: self.colors.warn,
            err: self.colors.err,
            is_dark: self.dark,
        }
    }

    pub fn validate(&self) -> Result<(), ThemeError> {
        if self.format > THEME_FORMAT_VERSION {
            return Err(ThemeError::new(format!(
                "format: theme was made by a newer bcvault (format {} > {})",
                self.format, THEME_FORMAT_VERSION
            )));
        }
        if self.format != THEME_FORMAT_VERSION {
            return Err(ThemeError::new(format!(
                "format: expected {}, got {}",
                THEME_FORMAT_VERSION, self.format
            )));
        }
        validate_id(&self.id)?;
        validate_text_field("name", &self.name, 1, 40)?;
        validate_text_field("description", &self.description, 0, 160)?;
        validate_text_field("author", &self.author, 0, 40)?;
        Ok(())
    }

    pub fn hints(&self) -> Vec<ThemeHint> {
        let mut out = Vec::new();
        let bgs = [
            ("bg0", self.colors.bg0),
            ("bg1", self.colors.bg1),
            ("bg2", self.colors.bg2),
        ];
        for (fg_name, fg) in [
            ("text", self.colors.text),
            ("text_dim", self.colors.text_dim),
        ] {
            for (bg_name, bg) in bgs {
                let ratio = contrast_ratio(fg, bg);
                if ratio < 4.5 {
                    out.push(ThemeHint {
                        rule: format!("{fg_name}_on_{bg_name}"),
                        message: format!(
                            "{fg_name} on {bg_name} contrast is {ratio:.2}:1 (recommended >= 4.5:1)"
                        ),
                    });
                }
            }
        }

        let accent_ratio = contrast_ratio(self.colors.accent, self.colors.bg1);
        if accent_ratio < 4.5 {
            out.push(ThemeHint {
                rule: "accent_on_bg1".to_string(),
                message: format!(
                    "accent on bg1 contrast is {accent_ratio:.2}:1 (recommended >= 4.5:1)"
                ),
            });
        }

        for (key, color) in [
            ("accent2", self.colors.accent2),
            ("ok", self.colors.ok),
            ("warn", self.colors.warn),
            ("err", self.colors.err),
        ] {
            let ratio = contrast_ratio(color, self.colors.bg1);
            if ratio < 3.0 {
                out.push(ThemeHint {
                    rule: format!("{key}_on_bg1"),
                    message: format!(
                        "{key} on bg1 contrast is {ratio:.2}:1 (recommended >= 3.0:1)"
                    ),
                });
            }
        }

        let bg0_lum = relative_luminance(self.colors.bg0);
        let expected_dark = bg0_lum < 0.4;
        if self.dark != expected_dark {
            out.push(ThemeHint {
                rule: "dark_matches_bg0".to_string(),
                message: if expected_dark {
                    format!("dark is false, but bg0 luminance is {bg0_lum:.2} (< 0.4, looks dark)")
                } else {
                    format!("dark is true, but bg0 luminance is {bg0_lum:.2} (>= 0.4, looks light)")
                },
            });
        }

        out
    }

    pub fn to_toml_string(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str(THEME_FILE_HEADER);
        out.push('\n');
        out.push_str(&format!("format = {}\n", self.format));
        out.push_str(&format!("id = {}\n", toml::Value::String(self.id.clone())));
        out.push_str(&format!(
            "name = {}\n",
            toml::Value::String(self.name.trim().to_string())
        ));
        out.push_str(&format!(
            "description = {}\n",
            toml::Value::String(self.description.trim().to_string())
        ));
        out.push_str(&format!(
            "author = {}\n",
            toml::Value::String(self.author.trim().to_string())
        ));
        out.push_str(&format!("dark = {}\n", self.dark));
        out.push_str(&format!("scanlines = {}\n", self.scanlines));
        out.push_str("\n[colors]\n");
        for key in COLOR_KEYS {
            if let Some(c) = self.colors.get(key) {
                out.push_str(&format!("{key} = \"{}\"\n", format_hex(c)));
            }
        }
        out
    }
}

pub fn parse_bytes(bytes: &[u8]) -> Result<ParsedTheme, ThemeError> {
    if bytes.len() > MAX_THEME_FILE_BYTES {
        return Err(ThemeError::new(format!(
            "file exceeds maximum size of {} bytes ({} bytes)",
            MAX_THEME_FILE_BYTES,
            bytes.len()
        )));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|err| ThemeError::new(format!("file is not valid UTF-8: {err}")))?;
    parse(text)
}

pub fn parse(input: &str) -> Result<ParsedTheme, ThemeError> {
    if input.len() > MAX_THEME_FILE_BYTES {
        return Err(ThemeError::new(format!(
            "file exceeds maximum size of {} bytes ({} bytes)",
            MAX_THEME_FILE_BYTES,
            input.len()
        )));
    }

    let value: toml::Value = toml::from_str(input)
        .map_err(|err| ThemeError::new(format!("invalid TOML syntax: {err}")))?;
    let table = value
        .as_table()
        .ok_or_else(|| ThemeError::new("root: expected a TOML table"))?;

    let mut warnings = Vec::new();
    const KNOWN_TOP_KEYS: [&str; 8] = [
        "format",
        "id",
        "name",
        "description",
        "author",
        "dark",
        "scanlines",
        "colors",
    ];
    for key in table.keys() {
        if !KNOWN_TOP_KEYS.contains(&key.as_str()) {
            warnings.push(format!("unknown key '{key}' ignored"));
        }
    }

    let format = match table.get("format") {
        Some(toml::Value::Integer(v)) => *v,
        Some(_) => return Err(ThemeError::new("format: expected integer 1")),
        None => return Err(ThemeError::new("format: missing required key")),
    };
    if format > THEME_FORMAT_VERSION {
        return Err(ThemeError::new(format!(
            "format: theme was made by a newer bcvault (format {format} > {THEME_FORMAT_VERSION})"
        )));
    }
    if format != THEME_FORMAT_VERSION {
        return Err(ThemeError::new(format!(
            "format: expected {THEME_FORMAT_VERSION}, got {format}"
        )));
    }

    let id = required_string(table, "id")?;
    validate_id(&id)?;

    let name_raw = required_string(table, "name")?;
    let name = name_raw.trim().to_string();
    validate_text_field("name", &name, 1, 40)?;

    let description = optional_string(table, "description")?
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    validate_text_field("description", &description, 0, 160)?;

    let author = optional_string(table, "author")?
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    validate_text_field("author", &author, 0, 40)?;

    let dark = match table.get("dark") {
        Some(toml::Value::Boolean(b)) => *b,
        Some(_) => return Err(ThemeError::new("dark: expected boolean")),
        None => return Err(ThemeError::new("dark: missing required key")),
    };

    let scanlines = match table.get("scanlines") {
        Some(toml::Value::Boolean(b)) => *b,
        Some(_) => return Err(ThemeError::new("scanlines: expected boolean")),
        None => false,
    };

    let colors_table = match table.get("colors") {
        Some(toml::Value::Table(t)) => t,
        Some(_) => return Err(ThemeError::new("colors: expected [colors] table")),
        None => return Err(ThemeError::new("colors: missing required [colors] table")),
    };

    for key in colors_table.keys() {
        if !COLOR_KEYS.contains(&key.as_str()) {
            warnings.push(format!("unknown key 'colors.{key}' ignored"));
        }
    }

    let colors = ThemeColors {
        bg0: parse_color_key(colors_table, "bg0")?,
        bg1: parse_color_key(colors_table, "bg1")?,
        bg2: parse_color_key(colors_table, "bg2")?,
        border: parse_color_key(colors_table, "border")?,
        border_hi: parse_color_key(colors_table, "border_hi")?,
        text: parse_color_key(colors_table, "text")?,
        text_dim: parse_color_key(colors_table, "text_dim")?,
        accent: parse_color_key(colors_table, "accent")?,
        accent2: parse_color_key(colors_table, "accent2")?,
        ok: parse_color_key(colors_table, "ok")?,
        warn: parse_color_key(colors_table, "warn")?,
        err: parse_color_key(colors_table, "err")?,
    };

    Ok(ParsedTheme {
        file: ThemeFile {
            format,
            id,
            name,
            description,
            author,
            dark,
            scanlines,
            colors,
        },
        warnings,
    })
}

fn required_string(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
) -> Result<String, ThemeError> {
    match table.get(key) {
        Some(toml::Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(ThemeError::new(format!("{key}: expected string"))),
        None => Err(ThemeError::new(format!("{key}: missing required key"))),
    }
}

fn optional_string(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
) -> Result<Option<String>, ThemeError> {
    match table.get(key) {
        Some(toml::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ThemeError::new(format!("{key}: expected string"))),
        None => Ok(None),
    }
}

fn parse_color_key(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
) -> Result<Color32, ThemeError> {
    let raw = match table.get(key) {
        Some(toml::Value::String(s)) => s,
        Some(_) => {
            return Err(ThemeError::new(format!(
                "colors.{key}: expected #RRGGBB string"
            )));
        }
        None => {
            return Err(ThemeError::new(format!(
                "colors.{key}: missing required color"
            )));
        }
    };
    parse_hex_color(raw).map_err(|reason| ThemeError::new(format!("colors.{key}: {reason}")))
}

pub fn parse_hex_color(raw: &str) -> Result<Color32, String> {
    let trimmed = raw.trim();
    if trimmed.len() != raw.len() {
        return Err("unexpected whitespace around hex color".to_string());
    }
    let bytes = raw.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' {
        return Err("expected exact #RRGGBB format (6 hex digits, no alpha)".to_string());
    }
    if !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err("contains non-hexadecimal character".to_string());
    }
    let r =
        u8::from_str_radix(&raw[1..3], 16).map_err(|_| "invalid red hex component".to_string())?;
    let g = u8::from_str_radix(&raw[3..5], 16)
        .map_err(|_| "invalid green hex component".to_string())?;
    let b =
        u8::from_str_radix(&raw[5..7], 16).map_err(|_| "invalid blue hex component".to_string())?;
    Ok(Color32::from_rgb(r, g, b))
}

pub fn format_hex(color: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b())
}

pub fn validate_id(id: &str) -> Result<(), ThemeError> {
    if id.is_empty() || id.len() > 32 {
        return Err(ThemeError::new(
            "id: must be 1 to 32 characters".to_string(),
        ));
    }
    if !id
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
    {
        return Err(ThemeError::new(
            "id: only lowercase a-z, 0-9, and '-' are allowed".to_string(),
        ));
    }
    if id.starts_with('-') || id.ends_with('-') || id.contains("--") {
        return Err(ThemeError::new(
            "id: cannot start/end with '-' or contain '--'".to_string(),
        ));
    }
    Ok(())
}

fn validate_text_field(
    key: &str,
    value: &str,
    min_chars: usize,
    max_chars: usize,
) -> Result<(), ThemeError> {
    let len = value.chars().count();
    if len < min_chars {
        return Err(ThemeError::new(format!(
            "{key}: must be at least {min_chars} character(s)"
        )));
    }
    if len > max_chars {
        return Err(ThemeError::new(format!(
            "{key}: exceeds maximum length of {max_chars} characters"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(ThemeError::new(format!(
            "{key}: control characters are not allowed"
        )));
    }
    Ok(())
}

pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut prev_hyphen = true;
    for ch in name.chars() {
        let folded = fold_latin_accent(ch).to_ascii_lowercase();
        if matches!(folded, 'a'..='z' | '0'..='9') {
            slug.push(folded);
            prev_hyphen = false;
        } else if !prev_hyphen {
            slug.push('-');
            prev_hyphen = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.len() > 32 {
        slug.truncate(32);
        while slug.ends_with('-') {
            slug.pop();
        }
    }
    if slug.is_empty() {
        "theme".to_string()
    } else {
        slug
    }
}

pub fn unique_slug(base: &str, mut exists: impl FnMut(&str) -> bool) -> String {
    let clean_base = if validate_id(base).is_ok() {
        base.to_string()
    } else {
        slugify(base)
    };
    if !exists(&clean_base) {
        return clean_base;
    }
    for n in 2..=9999 {
        let suffix = format!("-{n}");
        let max_base_len = 32usize.saturating_sub(suffix.len());
        let mut prefix: String = clean_base.chars().take(max_base_len).collect();
        while prefix.ends_with('-') {
            prefix.pop();
        }
        let candidate = if prefix.is_empty() {
            format!("theme{suffix}")
        } else {
            format!("{prefix}{suffix}")
        };
        if !exists(&candidate) {
            return candidate;
        }
    }
    "theme-overflow".to_string()
}

fn fold_latin_accent(ch: char) -> char {
    match ch {
        'À'..='Å' | 'à'..='å' => 'a',
        'Ç' | 'ç' => 'c',
        'È'..='Ë' | 'è'..='ë' => 'e',
        'Ì'..='Ï' | 'ì'..='ï' => 'i',
        'Ñ' | 'ñ' => 'n',
        'Ò'..='Ö' | 'Ø' | 'ò'..='ö' | 'ø' => 'o',
        'Ù'..='Ü' | 'ù'..='ü' => 'u',
        'Ý' | 'ý' | 'ÿ' => 'y',
        _ => ch,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const CARBON_TOML: &str = "\
# bcvault theme, format 1. Share this file as is.
format = 1
id = \"carbon\"
name = \"Carbon\"
description = \"Dark neutral slate theme\"
author = \"bcvault\"
dark = true
scanlines = false

[colors]
bg0 = \"#0A0A0B\"
bg1 = \"#121315\"
bg2 = \"#1B1D20\"
border = \"#3A3D42\"
border_hi = \"#8A8F98\"
text = \"#ECECEC\"
text_dim = \"#9AA0A6\"
accent = \"#00D1B2\"
accent2 = \"#3DDC84\"
ok = \"#3DDC84\"
warn = \"#FFB300\"
err = \"#FF5252\"
";

    #[test]
    fn parse_and_serialize_carbon_roundtrips_byte_for_byte() {
        let parsed = parse(CARBON_TOML).expect("carbon must parse");
        assert!(parsed.warnings.is_empty());
        assert_eq!(parsed.file.id, "carbon");
        assert_eq!(parsed.file.name, "Carbon");
        assert!(parsed.file.dark);
        assert!(!parsed.file.scanlines);
        assert!(parsed.file.hints().is_empty());

        let serialized = parsed.file.to_toml_string();
        assert_eq!(serialized, CARBON_TOML);
        let reparsed = parse(&serialized).expect("re-parse must succeed");
        assert_eq!(reparsed.file, parsed.file);
    }

    #[test]
    fn rejects_invalid_inputs_with_key_in_message() {
        // Missing key
        let missing = CARBON_TOML.replace("accent = \"#00D1B2\"\n", "");
        let err = parse(&missing).unwrap_err();
        assert!(err.message.contains("colors.accent"), "{err}");

        // Bad hex: #FFF, red, alpha
        for bad in ["#FFF", "red", "#00D1B2FF", "#GGGGGG"] {
            let s = CARBON_TOML.replace("#00D1B2", bad);
            let err = parse(&s).unwrap_err();
            assert!(err.message.contains("colors.accent"), "{bad}: {err}");
        }

        // Wrong type
        let wrong_type = CARBON_TOML.replace("dark = true", "dark = \"yes\"");
        let err = parse(&wrong_type).unwrap_err();
        assert!(err.message.contains("dark"), "{err}");

        // Bad ids
        for bad_id in ["Carbon", "a/b", "..", "-bad", "bad-", "a--b", ""] {
            let s = CARBON_TOML.replace("id = \"carbon\"", &format!("id = \"{bad_id}\""));
            let err = parse(&s).unwrap_err();
            assert!(err.message.contains("id"), "{bad_id}: {err}");
        }

        // Empty or over-long name
        let empty_name = CARBON_TOML.replace("name = \"Carbon\"", "name = \"   \"");
        let err = parse(&empty_name).unwrap_err();
        assert!(err.message.contains("name"), "{err}");

        let long_name = CARBON_TOML.replace(
            "name = \"Carbon\"",
            &format!("name = \"{}\"", "A".repeat(41)),
        );
        let err = parse(&long_name).unwrap_err();
        assert!(err.message.contains("name"), "{err}");

        // Format 2
        let fmt2 = CARBON_TOML.replace("format = 1", "format = 2");
        let err = parse(&fmt2).unwrap_err();
        assert!(err.message.contains("newer"), "{err}");

        // Over 16 KiB
        let big = vec![b'a'; MAX_THEME_FILE_BYTES + 1];
        let err = parse_bytes(&big).unwrap_err();
        assert!(err.message.contains("16384"), "{err}");

        // Invalid UTF-8
        let err = parse_bytes(&[0xFF, 0xFE, 0x00]).unwrap_err();
        assert!(err.message.contains("UTF-8"), "{err}");
    }

    #[test]
    fn unknown_keys_produce_warnings_without_failing() {
        let with_extra = format!("{CARBON_TOML}future_field = 42\n");
        let parsed = parse(&with_extra).expect("unknown keys should be ignored");
        assert_eq!(parsed.warnings.len(), 1);
        assert!(parsed.warnings[0].contains("future_field"));
    }

    #[test]
    fn slugify_and_unique_slug_handle_edge_cases() {
        assert_eq!(slugify("  Café Noir!  "), "cafe-noir");
        assert_eq!(slugify("My___Cool   Theme"), "my-cool-theme");
        assert_eq!(slugify("!!!"), "theme");
        assert_eq!(slugify(""), "theme");
        let long = slugify("A Very Long Theme Name That Exceeds Thirty Two Characters");
        assert!(long.len() <= 32);
        assert!(validate_id(&long).is_ok());

        let existing = ["carbon", "carbon-2"];
        assert_eq!(
            unique_slug("carbon", |id| existing.contains(&id)),
            "carbon-3"
        );

        let max_id = "a".repeat(32);
        let next = unique_slug(&max_id, |id| id == max_id);
        assert_eq!(next.len(), 32);
        assert!(next.ends_with("-2"));
        assert!(validate_id(&next).is_ok());
    }

    #[test]
    fn hints_detect_low_contrast_and_dark_mismatch_without_blocking() {
        let bad = CARBON_TOML
            .replace("dark = true", "dark = false")
            .replace("text = \"#ECECEC\"", "text = \"#181818\"");
        let parsed = parse(&bad).expect("hints never block parsing");
        let hints = parsed.file.hints();
        assert!(hints.iter().any(|h| h.rule.starts_with("text_on_")));
        assert!(hints.iter().any(|h| h.rule == "dark_matches_bg0"));
    }
}

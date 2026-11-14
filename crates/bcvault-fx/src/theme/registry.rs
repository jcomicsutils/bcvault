use crate::theme::FxTheme;
use crate::theme::builtin::BUILTIN_THEME_FILES;
use crate::theme::file::{ThemeFile, ThemeHint, parse};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeSource {
    Builtin,
    User(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub source: ThemeSource,
    pub theme: FxTheme,
    pub file: ThemeFile,
    pub hints: Vec<ThemeHint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueSeverity {
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadIssue {
    pub path: PathBuf,
    pub message: String,
    pub severity: IssueSeverity,
}

#[derive(Clone, Debug, Default)]
pub struct ThemeRegistry {
    entries: Vec<ThemeEntry>,
    issues: Vec<LoadIssue>,
}

impl ThemeRegistry {
    pub fn builtin_only() -> Self {
        let mut entries = Vec::with_capacity(BUILTIN_THEME_FILES.len());
        for (_stem, raw) in BUILTIN_THEME_FILES {
            if let Ok(parsed) = parse(raw) {
                let file = parsed.file;
                let hints = file.hints();
                let theme = file.to_fx_theme();
                entries.push(ThemeEntry {
                    id: file.id.clone(),
                    name: file.name.clone(),
                    description: file.description.clone(),
                    author: file.author.clone(),
                    source: ThemeSource::Builtin,
                    theme,
                    file,
                    hints,
                });
            }
        }
        Self {
            entries,
            issues: Vec::new(),
        }
    }

    pub fn entries(&self) -> &[ThemeEntry] {
        &self.entries
    }

    pub fn issues(&self) -> &[LoadIssue] {
        &self.issues
    }

    pub fn get(&self, id: &str) -> Option<&ThemeEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn resolve(&self, id: &str) -> Option<&ThemeEntry> {
        self.get(id)
            .or_else(|| self.get("carbon"))
            .or_else(|| self.entries.first())
    }

    pub fn resolve_theme(&self, id: &str) -> FxTheme {
        self.resolve(id).map_or(FxTheme::FALLBACK, |e| e.theme)
    }

    pub fn contains_id(&self, id: &str) -> bool {
        self.entries.iter().any(|e| e.id == id)
    }

    pub fn is_builtin_id(id: &str) -> bool {
        BUILTIN_THEME_FILES.iter().any(|(stem, _)| *stem == id)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::theme::{CornerRadius, apply_theme};
    use std::collections::HashSet;
    use std::path::Path;

    #[test]
    fn every_theme_toml_in_directory_is_registered() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("themes");
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .expect("themes dir must exist")
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("toml") {
                    p.file_stem()
                        .and_then(|s| s.to_str())
                        .map(ToString::to_string)
                } else {
                    None
                }
            })
            .collect();
        on_disk.sort();

        let mut registered: Vec<String> = BUILTIN_THEME_FILES
            .iter()
            .map(|(stem, _)| (*stem).to_string())
            .collect();
        registered.sort();
        assert_eq!(on_disk, registered);
    }

    #[test]
    fn builtin_ids_unique_ordered_and_ten_total() {
        let reg = ThemeRegistry::builtin_only();
        assert_eq!(reg.entries().len(), 10);
        let expected_order = [
            "carbon",
            "paper",
            "terminal",
            "vapor",
            "midnight",
            "ember",
            "catppuccin-mocha",
            "contrast",
            "daylight",
            "sage",
        ];
        let mut seen_ids = HashSet::new();
        let mut seen_names = HashSet::new();
        for (entry, (stem, _)) in reg.entries().iter().zip(BUILTIN_THEME_FILES.iter()) {
            assert_eq!(entry.id, *stem);
            assert!(seen_ids.insert(entry.id.clone()));
            assert!(seen_names.insert(entry.name.clone()));
        }
        let actual_order: Vec<&str> = reg.entries().iter().map(|e| e.id.as_str()).collect();
        assert_eq!(actual_order, expected_order);
    }

    #[test]
    fn all_builtins_pass_contrast_and_dark_rules() {
        let reg = ThemeRegistry::builtin_only();
        for entry in reg.entries() {
            assert!(
                entry.hints.is_empty(),
                "built-in theme '{}' has hints: {:?}",
                entry.id,
                entry.hints
            );
        }
    }

    #[test]
    fn scanlines_only_terminal_and_carbon_equals_fallback() {
        let reg = ThemeRegistry::builtin_only();
        for entry in reg.entries() {
            assert_eq!(
                entry.theme.scanlines,
                entry.id == "terminal",
                "unexpected scanlines for {}",
                entry.id
            );
        }
        assert_eq!(reg.resolve_theme("carbon"), FxTheme::FALLBACK);
    }

    #[test]
    fn zero_radius_for_every_builtin_theme() {
        let ctx = egui::Context::default();
        let reg = ThemeRegistry::builtin_only();
        for entry in reg.entries() {
            apply_theme(&ctx, &entry.theme);
            let s = ctx.style();
            assert_eq!(s.visuals.window_corner_radius, CornerRadius::ZERO);
            assert_eq!(s.visuals.menu_corner_radius, CornerRadius::ZERO);
            let w = &s.visuals.widgets;
            assert_eq!(w.noninteractive.corner_radius, CornerRadius::ZERO);
            assert_eq!(w.inactive.corner_radius, CornerRadius::ZERO);
            assert_eq!(w.hovered.corner_radius, CornerRadius::ZERO);
            assert_eq!(w.active.corner_radius, CornerRadius::ZERO);
            assert_eq!(w.open.corner_radius, CornerRadius::ZERO);
        }
    }

    #[test]
    fn resolve_falls_back_to_carbon_for_unknown_id() {
        let reg = ThemeRegistry::builtin_only();
        assert!(reg.get("does-not-exist").is_none());
        assert_eq!(reg.resolve("does-not-exist").unwrap().id, "carbon");
    }
}

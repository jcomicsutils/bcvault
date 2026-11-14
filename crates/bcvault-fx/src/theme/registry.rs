use crate::theme::FxTheme;
use crate::theme::builtin::BUILTIN_THEME_FILES;
use crate::theme::file::{
    MAX_THEME_FILE_BYTES, ThemeError, ThemeFile, ThemeHint, parse, parse_bytes, unique_slug,
};
use std::path::{Path, PathBuf};

pub const MAX_USER_THEMES: usize = 100;

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub imported: usize,
    pub skipped: usize,
    pub renamed: Vec<(String, String)>,
    pub last_imported_id: Option<String>,
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

    pub fn builtin_entries(&self) -> impl Iterator<Item = &ThemeEntry> {
        self.entries
            .iter()
            .filter(|e| matches!(e.source, ThemeSource::Builtin))
    }

    pub fn user_entries(&self) -> impl Iterator<Item = &ThemeEntry> {
        self.entries
            .iter()
            .filter(|e| matches!(e.source, ThemeSource::User(_)))
    }

    pub fn issues(&self) -> &[LoadIssue] {
        &self.issues
    }

    pub fn palette_commands(&self) -> Vec<(String, String, String)> {
        self.entries
            .iter()
            .map(|e| (e.id.clone(), e.name.clone(), e.description.clone()))
            .collect()
    }

    pub fn load_user_dir(&mut self, dir: &Path) -> Vec<LoadIssue> {
        *self = Self::builtin_only();
        let mut issues = Vec::new();
        if !dir.exists() {
            self.issues = issues.clone();
            return issues;
        }

        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(err) => {
                issues.push(LoadIssue {
                    path: dir.to_path_buf(),
                    message: format!("failed to read themes directory: {err}"),
                    severity: IssueSeverity::Error,
                });
                self.issues = issues.clone();
                return issues;
            }
        };

        let mut candidates: Vec<PathBuf> = Vec::new();
        for item in read_dir {
            match item {
                Ok(entry) => {
                    let path = entry.path();
                    let meta = match std::fs::symlink_metadata(&path) {
                        Ok(m) => m,
                        Err(err) => {
                            issues.push(LoadIssue {
                                path,
                                message: format!("failed to inspect entry: {err}"),
                                severity: IssueSeverity::Warning,
                            });
                            continue;
                        }
                    };
                    if meta.file_type().is_symlink() || !meta.is_file() {
                        continue;
                    }
                    if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                        continue;
                    }
                    candidates.push(path);
                }
                Err(err) => {
                    issues.push(LoadIssue {
                        path: dir.to_path_buf(),
                        message: format!("directory entry error: {err}"),
                        severity: IssueSeverity::Warning,
                    });
                }
            }
        }

        candidates.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

        let builtin_count = self.entries.len();
        let mut loaded_user_count = 0usize;

        for path in candidates {
            if loaded_user_count >= MAX_USER_THEMES {
                issues.push(LoadIssue {
                    path,
                    message: format!("skipped: user theme limit ({MAX_USER_THEMES}) reached"),
                    severity: IssueSeverity::Warning,
                });
                continue;
            }

            let meta = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(err) => {
                    issues.push(LoadIssue {
                        path,
                        message: format!("failed to stat file: {err}"),
                        severity: IssueSeverity::Error,
                    });
                    continue;
                }
            };
            if meta.len() > MAX_THEME_FILE_BYTES as u64 {
                issues.push(LoadIssue {
                    path,
                    message: format!(
                        "file exceeds maximum size of {MAX_THEME_FILE_BYTES} bytes ({} bytes)",
                        meta.len()
                    ),
                    severity: IssueSeverity::Error,
                });
                continue;
            }

            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(err) => {
                    issues.push(LoadIssue {
                        path,
                        message: format!("failed to read file: {err}"),
                        severity: IssueSeverity::Error,
                    });
                    continue;
                }
            };

            let parsed = match parse_bytes(&bytes) {
                Ok(p) => p,
                Err(err) => {
                    issues.push(LoadIssue {
                        path,
                        message: err.message,
                        severity: IssueSeverity::Error,
                    });
                    continue;
                }
            };

            if self.contains_id(&parsed.file.id) {
                issues.push(LoadIssue {
                    path,
                    message: format!("duplicate theme id '{}'", parsed.file.id),
                    severity: IssueSeverity::Error,
                });
                continue;
            }

            for warn in parsed.warnings {
                issues.push(LoadIssue {
                    path: path.clone(),
                    message: warn,
                    severity: IssueSeverity::Warning,
                });
            }

            let file = parsed.file;
            let hints = file.hints();
            let theme = file.to_fx_theme();
            self.entries.push(ThemeEntry {
                id: file.id.clone(),
                name: file.name.clone(),
                description: file.description.clone(),
                author: file.author.clone(),
                source: ThemeSource::User(path),
                theme,
                file,
                hints,
            });
            loaded_user_count += 1;
        }

        self.entries[builtin_count..].sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });

        self.issues = issues.clone();
        issues
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

    pub fn save_user_theme(
        &mut self,
        dir: &Path,
        file: &ThemeFile,
        existing_path: Option<&Path>,
    ) -> Result<PathBuf, ThemeError> {
        file.validate()?;
        if Self::is_builtin_id(&file.id) {
            return Err(ThemeError::new(format!(
                "id: '{}' is reserved for a built-in theme",
                file.id
            )));
        }
        std::fs::create_dir_all(dir)
            .map_err(|e| ThemeError::new(format!("failed to create themes directory: {e}")))?;

        let target_path = existing_path
            .map(PathBuf::from)
            .unwrap_or_else(|| dir.join(format!("{}.toml", file.id)));
        let temp_path = dir.join(format!(".{}.toml.tmp.{}", file.id, std::process::id()));
        let content = file.to_toml_string();

        if let Err(e) = std::fs::write(&temp_path, content.as_bytes()) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(ThemeError::new(format!(
                "failed to write temp theme file: {e}"
            )));
        }
        if let Err(e) = std::fs::rename(&temp_path, &target_path) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(ThemeError::new(format!("failed to save theme file: {e}")));
        }

        self.load_user_dir(dir);
        Ok(target_path)
    }

    pub fn import_theme_files(&mut self, dir: &Path, paths: &[PathBuf]) -> ImportSummary {
        let mut summary = ImportSummary::default();
        let mut new_issues = Vec::new();

        for path in paths {
            let meta = match std::fs::symlink_metadata(path) {
                Ok(m) => m,
                Err(err) => {
                    new_issues.push(LoadIssue {
                        path: path.clone(),
                        message: format!("failed to inspect file: {err}"),
                        severity: IssueSeverity::Error,
                    });
                    summary.skipped += 1;
                    continue;
                }
            };
            if meta.file_type().is_symlink() || !meta.is_file() {
                new_issues.push(LoadIssue {
                    path: path.clone(),
                    message: "not a regular file".to_string(),
                    severity: IssueSeverity::Error,
                });
                summary.skipped += 1;
                continue;
            }
            if meta.len() > MAX_THEME_FILE_BYTES as u64 {
                new_issues.push(LoadIssue {
                    path: path.clone(),
                    message: format!(
                        "file exceeds maximum size of {MAX_THEME_FILE_BYTES} bytes ({} bytes)",
                        meta.len()
                    ),
                    severity: IssueSeverity::Error,
                });
                summary.skipped += 1;
                continue;
            }

            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(err) => {
                    new_issues.push(LoadIssue {
                        path: path.clone(),
                        message: format!("failed to read file: {err}"),
                        severity: IssueSeverity::Error,
                    });
                    summary.skipped += 1;
                    continue;
                }
            };

            let mut parsed = match parse_bytes(&bytes) {
                Ok(p) => p,
                Err(err) => {
                    new_issues.push(LoadIssue {
                        path: path.clone(),
                        message: err.message,
                        severity: IssueSeverity::Error,
                    });
                    summary.skipped += 1;
                    continue;
                }
            };

            let orig_id = parsed.file.id.clone();
            if self.contains_id(&orig_id) {
                let next_id = unique_slug(&orig_id, |id| self.contains_id(id));
                summary.renamed.push((orig_id, next_id.clone()));
                parsed.file.id = next_id;
            }

            match self.save_user_theme(dir, &parsed.file, None) {
                Ok(_) => {
                    summary.imported += 1;
                    summary.last_imported_id = Some(parsed.file.id);
                }
                Err(err) => {
                    new_issues.push(LoadIssue {
                        path: path.clone(),
                        message: err.message,
                        severity: IssueSeverity::Error,
                    });
                    summary.skipped += 1;
                }
            }
        }

        if !new_issues.is_empty() {
            self.issues.extend(new_issues);
        }
        summary
    }

    pub fn export_theme_to_path(file: &ThemeFile, dest: &Path) -> Result<(), ThemeError> {
        file.validate()?;
        std::fs::write(dest, file.to_toml_string().as_bytes())
            .map_err(|e| ThemeError::new(format!("failed to export theme: {e}")))
    }

    pub fn delete_user_theme(&mut self, dir: &Path, id: &str) -> Result<(), ThemeError> {
        let Some(entry) = self.get(id) else {
            return Err(ThemeError::new(format!("theme '{id}' not found")));
        };
        let path = match &entry.source {
            ThemeSource::User(p) => p.clone(),
            ThemeSource::Builtin => {
                return Err(ThemeError::new("built-in themes cannot be deleted"));
            }
        };
        std::fs::remove_file(&path)
            .map_err(|e| ThemeError::new(format!("failed to delete theme file: {e}")))?;
        self.load_user_dir(dir);
        Ok(())
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

    #[test]
    fn load_user_dir_handles_valid_invalid_clashes_non_toml_subdir_symlink_and_limit() {
        let dir = tempfile::tempdir().unwrap();
        let valid_toml = BUILTIN_THEME_FILES[0]
            .1
            .replace("id = \"carbon\"", "id = \"z-custom\"")
            .replace("name = \"Carbon\"", "name = \"Z Custom\"");
        std::fs::write(dir.path().join("01_valid.toml"), &valid_toml).unwrap();

        let alpha_toml = BUILTIN_THEME_FILES[0]
            .1
            .replace("id = \"carbon\"", "id = \"a-custom\"")
            .replace("name = \"Carbon\"", "name = \"A Custom\"");
        std::fs::write(dir.path().join("02_alpha.toml"), &alpha_toml).unwrap();

        std::fs::write(dir.path().join("03_invalid.toml"), "not = valid = toml [").unwrap();
        std::fs::write(
            dir.path().join("04_builtin_clash.toml"),
            BUILTIN_THEME_FILES[0].1,
        )
        .unwrap();
        std::fs::write(dir.path().join("05_dup_id.toml"), &valid_toml).unwrap();
        std::fs::write(dir.path().join("notes.txt"), &valid_toml).unwrap();
        std::fs::create_dir(dir.path().join("subdir.toml")).unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(
            dir.path().join("01_valid.toml"),
            dir.path().join("06_symlink.toml"),
        )
        .unwrap();

        let mut reg = ThemeRegistry::builtin_only();
        let issues = reg.load_user_dir(dir.path());
        assert_eq!(reg.user_entries().count(), 2);
        let user_ids: Vec<&str> = reg.user_entries().map(|e| e.id.as_str()).collect();
        assert_eq!(user_ids, vec!["a-custom", "z-custom"]);
        assert_eq!(issues.len(), 3); // invalid, builtin clash, dup id

        // Test 100-theme limit
        let limit_dir = tempfile::tempdir().unwrap();
        for i in 0..101 {
            let t = BUILTIN_THEME_FILES[0]
                .1
                .replace("id = \"carbon\"", &format!("id = \"t-{i:03}\""))
                .replace("name = \"Carbon\"", &format!("name = \"Theme {i:03}\""));
            std::fs::write(limit_dir.path().join(format!("{i:03}.toml")), t).unwrap();
        }
        let mut reg2 = ThemeRegistry::builtin_only();
        let issues2 = reg2.load_user_dir(limit_dir.path());
        assert_eq!(reg2.user_entries().count(), 100);
        assert_eq!(issues2.len(), 1);
        assert!(issues2[0].message.contains("100"));
    }

    #[test]
    fn save_user_theme_is_atomic_and_preserves_custom_filename() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = ThemeRegistry::builtin_only();
        let mut file = reg.resolve("carbon").unwrap().file.clone();
        file.id = "my-custom".to_string();
        file.name = "My Custom".to_string();

        let saved = reg.save_user_theme(dir.path(), &file, None).unwrap();
        assert_eq!(saved, dir.path().join("my-custom.toml"));
        assert!(saved.exists());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty());
        assert_eq!(reg.get("my-custom").unwrap().name, "My Custom");

        // Differently named file saves in place
        let alt_path = dir.path().join("custom_filename.toml");
        file.id = "renamed-id".to_string();
        file.name = "Renamed File".to_string();
        std::fs::write(&alt_path, file.to_toml_string()).unwrap();
        reg.load_user_dir(dir.path());
        let mut edited = reg.get("renamed-id").unwrap().file.clone();
        edited.name = "Renamed Edited".to_string();
        let saved_alt = reg
            .save_user_theme(dir.path(), &edited, Some(&alt_path))
            .unwrap();
        assert_eq!(saved_alt, alt_path);
        assert_eq!(reg.get("renamed-id").unwrap().name, "Renamed Edited");
    }

    #[test]
    fn import_never_overwrites_renames_clashes_and_tracks_summary() {
        let themes_dir = tempfile::tempdir().unwrap();
        let src_dir = tempfile::tempdir().unwrap();

        let valid = BUILTIN_THEME_FILES[0]
            .1
            .replace("id = \"carbon\"", "id = \"custom-one\"")
            .replace("name = \"Carbon\"", "name = \"Custom One\"");
        let p_valid = src_dir.path().join("custom.toml");
        std::fs::write(&p_valid, &valid).unwrap();

        let p_builtin_clash = src_dir.path().join("carbon_copy.toml");
        std::fs::write(&p_builtin_clash, BUILTIN_THEME_FILES[0].1).unwrap();

        let p_user_clash = src_dir.path().join("dup.toml");
        std::fs::write(&p_user_clash, &valid).unwrap();

        let p_bad = src_dir.path().join("bad.toml");
        std::fs::write(&p_bad, "not valid toml [").unwrap();

        let mut reg = ThemeRegistry::builtin_only();
        let summary = reg.import_theme_files(
            themes_dir.path(),
            &[p_valid, p_builtin_clash, p_user_clash, p_bad],
        );
        assert_eq!(summary.imported, 3);
        assert_eq!(summary.skipped, 1);
        assert!(reg.get("custom-one").is_some());
        assert!(reg.get("carbon-2").is_some());
        assert!(reg.get("custom-one-2").is_some());
        assert_eq!(summary.renamed.len(), 2);
    }

    #[test]
    fn export_then_import_roundtrips_and_delete_removes_file() {
        let themes_dir = tempfile::tempdir().unwrap();
        let export_dir = tempfile::tempdir().unwrap();
        let mut reg = ThemeRegistry::builtin_only();

        let mut custom = reg.resolve("vapor").unwrap().file.clone();
        custom.id = "my-vapor".to_string();
        custom.name = "My Vapor".to_string();
        let saved_path = reg
            .save_user_theme(themes_dir.path(), &custom, None)
            .unwrap();
        assert!(saved_path.exists());

        let exported_path = export_dir.path().join("my-vapor.toml");
        ThemeRegistry::export_theme_to_path(&custom, &exported_path).unwrap();

        reg.delete_user_theme(themes_dir.path(), "my-vapor")
            .unwrap();
        assert!(!saved_path.exists());
        assert!(reg.get("my-vapor").is_none());

        let summary = reg.import_theme_files(themes_dir.path(), &[exported_path]);
        assert_eq!(summary.imported, 1);
        assert_eq!(reg.get("my-vapor").unwrap().file, custom);
    }
}

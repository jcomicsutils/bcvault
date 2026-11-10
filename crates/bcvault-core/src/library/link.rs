use std::path::{Path, PathBuf};

use crate::db::repo::library::probe_folder_state;
use crate::library::scan::is_audio_file;
use crate::types::{ArtistFolder, FolderState};

/// Warnings produced during folder link validation (non-blocking).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderWarning {
    /// Soft warning when the linked folder looks like a full music library holding multiple artists.
    LooksLikeLibrary,
}

/// Errors that prevent a folder from being linked to an artist.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FolderLinkError {
    #[error("Folder does not exist: {0}")]
    NotFound(String),
    #[error("Path is not a directory: {0}")]
    NotADirectory(String),
    #[error("Folder is not readable: {0}")]
    Unreadable(String),
    #[error("Folder overlaps with download directory {0}")]
    OverlapsOutDir(String),
    #[error("Folder overlaps with existing artist folder ({other_artist}): {path}")]
    OverlapsOtherFolder { other_artist: String, path: String },
    #[error("already covered by {path}")]
    AlreadyCovered { path: String },
    #[error("contains {path}; unlink that first")]
    ContainsOwnFolder { path: String },
}

/// A validated artist folder link ready to be stored in the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedLink {
    /// Canonicalized path (using `dunce::canonicalize` on Windows).
    pub canonical_path: PathBuf,
    /// Normalized path key for SQLite unique indexing and case-insensitive lookups.
    pub path_key: String,
    /// Optional non-blocking soft warning (e.g. looks like a multi-artist library).
    pub warning: Option<FolderWarning>,
}

/// Probes whether a path is present and accessible (`Ok`), missing (`Missing`),
/// or unreadable due to permissions/I/O errors (`Unreadable`).
pub fn probe_folder(path: &Path) -> FolderState {
    probe_folder_state(path)
}

/// Normalizes a filesystem path to a canonical lowercase key with forward slashes,
/// stripping any Windows extended-length prefixes (`\\?\`).
pub fn normalize_path_key(path: &Path) -> String {
    let simplified = dunce::simplified(path);
    let s = simplified.to_string_lossy();
    let s_clean = s.strip_prefix(r"\\?\").unwrap_or(&s);
    let s_fwd = s_clean.replace('\\', "/").to_lowercase();
    s_fwd.trim_end_matches('/').to_string()
}

/// Validates that a folder is suitable for linking to an artist according to §5.1:
/// - Path exists, is a directory, is readable.
/// - Canonicalized (using `dunce::canonicalize` to avoid Windows long-path UNC issues).
/// - Path does not equal or contain `out_dir`, and `out_dir` does not contain `path`.
/// - Path does not equal, contain or sit inside any existing artist's linked folder.
/// - Emits a soft warning if the directory hierarchy looks like a multi-artist library.
pub fn validate_folder_link(
    path: &Path,
    out_dir: &Path,
    other_folders: &[ArtistFolder],
) -> Result<ValidatedLink, FolderLinkError> {
    if !path.exists() {
        return Err(FolderLinkError::NotFound(path.display().to_string()));
    }
    if !path.is_dir() {
        return Err(FolderLinkError::NotADirectory(path.display().to_string()));
    }

    // Verify readability
    if std::fs::read_dir(path).is_err() {
        return Err(FolderLinkError::Unreadable(path.display().to_string()));
    }

    let canonical_path = dunce::canonicalize(path)
        .map_err(|_| FolderLinkError::NotFound(path.display().to_string()))?;

    // Check overlap with out_dir
    if let Ok(out_canon) = dunce::canonicalize(out_dir) {
        let overlaps = canonical_path == out_canon
            || canonical_path.starts_with(&out_canon)
            || out_canon.starts_with(&canonical_path);
        if overlaps {
            return Err(FolderLinkError::OverlapsOutDir(
                out_canon.display().to_string(),
            ));
        }
    }

    // Check overlap with other artist folders
    for other in other_folders {
        if let Ok(other_canon) = dunce::canonicalize(&other.path) {
            let overlaps = canonical_path == other_canon
                || canonical_path.starts_with(&other_canon)
                || other_canon.starts_with(&canonical_path);
            if overlaps {
                return Err(FolderLinkError::OverlapsOtherFolder {
                    other_artist: other.band_id.to_string(),
                    path: other_canon.display().to_string(),
                });
            }
        }
    }

    let path_key = normalize_path_key(&canonical_path);

    let warning = if check_looks_like_library(&canonical_path) {
        Some(FolderWarning::LooksLikeLibrary)
    } else {
        None
    };

    Ok(ValidatedLink {
        canonical_path,
        path_key,
        warning,
    })
}

/// Heuristic for §5.1 soft warning:
/// If most direct children contain no audio but themselves contain subdirectories
/// that do contain audio files, the folder likely holds several artists (i.e. a full library).
pub fn check_looks_like_library(path: &Path) -> bool {
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return false,
    };

    let mut direct_audio_count = 0;
    let mut nested_artist_like_dirs = 0;

    for entry in entries.flatten() {
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_symlink() {
            continue;
        }

        let p = entry.path();
        if ft.is_file() && is_audio_file(&p) {
            direct_audio_count += 1;
        } else if ft.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') || name_str.contains("to_delete") {
                continue;
            }

            // Inspect child directory
            if let Ok(sub_entries) = std::fs::read_dir(&p) {
                let mut sub_has_direct_audio = false;
                let mut sub_has_grandchild_audio = false;

                for sub in sub_entries.flatten() {
                    let sub_ft = match sub.file_type() {
                        Ok(ft) => ft,
                        Err(_) => continue,
                    };
                    if sub_ft.is_symlink() {
                        continue;
                    }
                    let sub_p = sub.path();
                    if sub_ft.is_file() && is_audio_file(&sub_p) {
                        sub_has_direct_audio = true;
                        break;
                    } else if sub_ft.is_dir() {
                        let has_grand_audio =
                            std::fs::read_dir(&sub_p).is_ok_and(|grand_entries| {
                                grand_entries.flatten().any(|grand| {
                                    grand.file_type().is_ok_and(|ft| ft.is_file())
                                        && is_audio_file(&grand.path())
                                })
                            });
                        if has_grand_audio {
                            sub_has_grandchild_audio = true;
                        }
                    }
                }

                if !sub_has_direct_audio && sub_has_grandchild_audio {
                    nested_artist_like_dirs += 1;
                }
            }
        }
    }

    direct_audio_count == 0 && nested_artist_like_dirs >= 2
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn validates_valid_folder_link() {
        let dir = tempdir().unwrap();
        let artist_folder = dir.path().join("Artist");
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&artist_folder).unwrap();
        fs::create_dir_all(&out_dir).unwrap();

        let validated = validate_folder_link(&artist_folder, &out_dir, &[]).unwrap();
        assert_eq!(
            validated.canonical_path,
            dunce::canonicalize(&artist_folder).unwrap()
        );
        assert_eq!(validated.warning, None);
    }

    #[test]
    fn rejects_missing_or_file_paths() {
        let dir = tempdir().unwrap();
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let non_existent = dir.path().join("NonExistent");
        let err = validate_folder_link(&non_existent, &out_dir, &[]).unwrap_err();
        assert!(matches!(err, FolderLinkError::NotFound(_)));

        let file_path = dir.path().join("file.txt");
        fs::write(&file_path, b"test").unwrap();
        let err = validate_folder_link(&file_path, &out_dir, &[]).unwrap_err();
        assert!(matches!(err, FolderLinkError::NotADirectory(_)));
    }

    #[test]
    fn rejects_overlap_with_out_dir() {
        let dir = tempdir().unwrap();
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let inside_out_dir = out_dir.join("Artist");
        fs::create_dir_all(&inside_out_dir).unwrap();

        let err = validate_folder_link(&inside_out_dir, &out_dir, &[]).unwrap_err();
        assert!(matches!(err, FolderLinkError::OverlapsOutDir(_)));

        // Equal to out_dir
        let err2 = validate_folder_link(&out_dir, &out_dir, &[]).unwrap_err();
        assert!(matches!(err2, FolderLinkError::OverlapsOutDir(_)));
    }

    #[test]
    fn rejects_overlap_with_other_artist_folder() {
        let dir = tempdir().unwrap();
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let artist1 = dir.path().join("Artist1");
        fs::create_dir_all(&artist1).unwrap();

        let other = ArtistFolder {
            id: 1,
            band_id: 1234,
            path: dunce::canonicalize(&artist1).unwrap(),
            state: FolderState::Ok,
        };

        // Identical folder
        let err =
            validate_folder_link(&artist1, &out_dir, std::slice::from_ref(&other)).unwrap_err();
        assert!(matches!(err, FolderLinkError::OverlapsOtherFolder { .. }));

        // Subdirectory of other artist folder
        let sub = artist1.join("Sub");
        fs::create_dir_all(&sub).unwrap();
        let err_sub =
            validate_folder_link(&sub, &out_dir, std::slice::from_ref(&other)).unwrap_err();
        assert!(matches!(
            err_sub,
            FolderLinkError::OverlapsOtherFolder { .. }
        ));
    }

    #[test]
    fn detects_looks_like_library_warning() {
        let dir = tempdir().unwrap();
        let lib_root = dir.path().join("MusicLibrary");
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        // MusicLibrary/ArtistA/Album1/track.flac
        let artist_a_album = lib_root.join("ArtistA").join("Album1");
        fs::create_dir_all(&artist_a_album).unwrap();
        fs::write(artist_a_album.join("01.flac"), b"audio").unwrap();

        // MusicLibrary/ArtistB/Album2/track.mp3
        let artist_b_album = lib_root.join("ArtistB").join("Album2");
        fs::create_dir_all(&artist_b_album).unwrap();
        fs::write(artist_b_album.join("01.mp3"), b"audio").unwrap();

        let validated = validate_folder_link(&lib_root, &out_dir, &[]).unwrap();
        assert_eq!(validated.warning, Some(FolderWarning::LooksLikeLibrary));
    }

    #[test]
    fn probes_folder_states() {
        let dir = tempdir().unwrap();
        let ok_dir = dir.path().join("ok_dir");
        fs::create_dir_all(&ok_dir).unwrap();
        assert_eq!(probe_folder(&ok_dir), FolderState::Ok);

        let missing_dir = dir.path().join("missing_dir");
        assert_eq!(probe_folder(&missing_dir), FolderState::Missing);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let unreadable = dir.path().join("unreadable_dir");
            fs::create_dir_all(&unreadable).unwrap();
            let _ = fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000));
            // In CI or container environments running as root (CAP_DAC_OVERRIDE),
            // 0o000 permissions do not prevent directory listing.
            if fs::read_dir(&unreadable).is_err() {
                assert_eq!(probe_folder(&unreadable), FolderState::Unreadable);
            }
            let _ = fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn windows_extended_paths_and_normalization() {
        let p1 = Path::new(r"\\?\C:\Music\Artist\");
        let p2 = Path::new(r"c:/music/artist");
        let p3 = Path::new(r"C:\MUSIC\ARTIST");

        assert_eq!(normalize_path_key(p1), "c:/music/artist");
        assert_eq!(normalize_path_key(p2), "c:/music/artist");
        assert_eq!(normalize_path_key(p3), "c:/music/artist");
    }

    #[test]
    fn drive_root_probe_unmounted_returns_unreadable() {
        #[cfg(windows)]
        {
            let z_path = Path::new(r"Z:\Music\Artist");
            assert_eq!(probe_folder(z_path), FolderState::Unreadable);
        }
    }
}

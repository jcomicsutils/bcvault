use std::fs::File;
use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;
use zip::ZipArchive;

use crate::library::tags::{AudioMetadata, read_audio_tags};

/// Supported audio extensions.
pub const AUDIO_EXTENSIONS: &[&str] = &["flac", "mp3", "ogg", "opus", "m4a", "wav", "aif", "aiff"];

/// The structure or container type of an album candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateKind {
    /// A single directory directly containing audio files.
    Folder,
    /// A directory containing multi-disc subdirectories (e.g. CD1, CD2).
    MultiDiscFolder { disc_subfolders: Vec<String> },
    /// The linked root itself directly contains audio tracks.
    SingleAlbumRoot,
    /// A zip archive containing audio tracks.
    Zip,
}

/// An album candidate discovered within the linked artist folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumCandidate {
    /// Display name (folder or zip name).
    pub name: String,
    /// Absolute path to the candidate directory or archive.
    pub path: PathBuf,
    /// Normalized path key (lowercase forward-slashed for deduplication/lookup).
    pub path_key: String,
    /// Structure type of the candidate.
    pub kind: CandidateKind,
    /// Total number of audio tracks detected.
    pub track_count: usize,
    /// Audio tags from the first readable track (if available).
    pub metadata: Option<AudioMetadata>,
}

/// Summary report returned by scanning a linked artist folder.
#[derive(Debug, Default)]
pub struct ScanReport {
    /// Discovered album candidates.
    pub candidates: Vec<AlbumCandidate>,
    /// Paths of symlinks encountered and skipped.
    pub skipped_symlinks: Vec<PathBuf>,
    /// Paths of directories or files that could not be read.
    pub unreadable_entries: Vec<PathBuf>,
}

/// Checks whether a filename matches supported audio extensions.
pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let lower = ext.to_lowercase();
            AUDIO_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

/// Checks whether a folder name matches common disc subfolder conventions
/// (e.g. "CD1", "CD 2", "Disc 1", "disc_2", "Disk1", "Part 1").
pub fn is_disc_folder_name(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    let prefixes = ["cd", "disc", "disk", "part"];

    for prefix in prefixes {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let rest = rest.trim_matches(|c: char| c == ' ' || c == '_' || c == '-');
            if !rest.is_empty()
                && (rest.chars().all(|c| c.is_ascii_digit()) || is_roman_numeral(rest))
            {
                return true;
            }
        }
    }
    false
}

fn is_roman_numeral(s: &str) -> bool {
    matches!(
        s,
        "i" | "ii" | "iii" | "iv" | "v" | "vi" | "vii" | "viii" | "ix" | "x"
    )
}

/// Checks if a file or directory name should be ignored (hidden or marked for deletion).
fn is_ignored_name(name: &str) -> bool {
    name.starts_with('.') || name.contains("to_delete")
}

/// Normalizes a path into a cross-platform canonical string key.
pub fn normalize_candidate_path_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

/// Scans a linked artist folder for album candidates according to §5.2 walk rules:
/// - Only inspects direct children of `root` plus one level for disc subfolders.
/// - Never follows symlinks (records them in `skipped_symlinks`).
/// - Skips hidden directories and `_to_delete`-style names.
/// - Respects cancellation token.
/// - Never accesses any path outside `root`.
pub fn scan_linked_folder(
    root: &Path,
    cancel: &CancellationToken,
) -> Result<ScanReport, std::io::Error> {
    let mut report = ScanReport::default();

    if cancel.is_cancelled() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "scan cancelled",
        ));
    }

    let root_entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) => {
            report.unreadable_entries.push(root.to_path_buf());
            return Err(err);
        }
    };

    // First check if the root itself directly contains audio (single-album artists)
    let mut root_audio_tracks = Vec::new();
    let mut child_dirs = Vec::new();
    let mut child_zips = Vec::new();

    for entry_res in root_entries {
        if cancel.is_cancelled() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "scan cancelled",
            ));
        }

        let entry = match entry_res {
            Ok(e) => e,
            Err(_) => {
                continue;
            }
        };

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => {
                report.unreadable_entries.push(entry.path());
                continue;
            }
        };

        if file_type.is_symlink() {
            report.skipped_symlinks.push(entry.path());
            continue;
        }

        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();
        if is_ignored_name(&name_str) {
            continue;
        }

        let path = entry.path();
        if file_type.is_dir() {
            child_dirs.push(path);
        } else if file_type.is_file() {
            if is_audio_file(&path) {
                root_audio_tracks.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
            {
                child_zips.push(path);
            }
        }
    }

    // If root directly contains audio files, treat root as a single album candidate
    if !root_audio_tracks.is_empty() {
        let meta = read_candidate_metadata(&root_audio_tracks);
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Root".to_string());
        report.candidates.push(AlbumCandidate {
            name,
            path: root.to_path_buf(),
            path_key: normalize_candidate_path_key(root),
            kind: CandidateKind::SingleAlbumRoot,
            track_count: root_audio_tracks.len(),
            metadata: meta,
        });
    }

    // Process zip candidates
    for zip_path in child_zips {
        if cancel.is_cancelled() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "scan cancelled",
            ));
        }

        let audio_track_count = count_zip_audio_entries(&zip_path);
        let file_stem = zip_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Archive".to_string());

        report.candidates.push(AlbumCandidate {
            name: file_stem,
            path_key: normalize_candidate_path_key(&zip_path),
            path: zip_path,
            kind: CandidateKind::Zip,
            track_count: audio_track_count,
            metadata: None,
        });
    }

    // Process child directories
    for dir_path in child_dirs {
        if cancel.is_cancelled() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "scan cancelled",
            ));
        }

        let dir_entries = match std::fs::read_dir(&dir_path) {
            Ok(entries) => entries,
            Err(_) => {
                report.unreadable_entries.push(dir_path);
                continue;
            }
        };

        let mut dir_audio_files = Vec::new();
        let mut disc_subdirs = Vec::new();

        for child_res in dir_entries {
            let child = match child_res {
                Ok(c) => c,
                Err(_) => continue,
            };

            let child_type = match child.file_type() {
                Ok(t) => t,
                Err(_) => {
                    report.unreadable_entries.push(child.path());
                    continue;
                }
            };

            if child_type.is_symlink() {
                report.skipped_symlinks.push(child.path());
                continue;
            }

            let child_name = child.file_name();
            let child_name_str = child_name.to_string_lossy();
            if is_ignored_name(&child_name_str) {
                continue;
            }

            let child_path = child.path();
            if child_type.is_file() {
                if is_audio_file(&child_path) {
                    dir_audio_files.push(child_path);
                }
            } else if child_type.is_dir() && is_disc_folder_name(&child_name_str) {
                disc_subdirs.push((child_name_str.to_string(), child_path));
            }
        }

        let folder_name = dir_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if !dir_audio_files.is_empty() {
            // Candidate: regular folder with direct audio
            let meta = read_candidate_metadata(&dir_audio_files);
            report.candidates.push(AlbumCandidate {
                name: folder_name,
                path_key: normalize_candidate_path_key(&dir_path),
                path: dir_path,
                kind: CandidateKind::Folder,
                track_count: dir_audio_files.len(),
                metadata: meta,
            });
        } else if !disc_subdirs.is_empty() {
            // Candidate: multi-disc folder
            let mut disc_names = Vec::new();
            let mut total_tracks = 0;
            let mut sample_tracks = Vec::new();

            for (disc_name, disc_path) in disc_subdirs {
                if let Ok(entries) = std::fs::read_dir(&disc_path) {
                    let mut disc_has_audio = false;
                    for disc_entry in entries.flatten() {
                        let is_audio = disc_entry.file_type().is_ok_and(|ft| ft.is_file())
                            && is_audio_file(&disc_entry.path());
                        if is_audio {
                            total_tracks += 1;
                            disc_has_audio = true;
                            if sample_tracks.len() < 2 {
                                sample_tracks.push(disc_entry.path());
                            }
                        }
                    }
                    if disc_has_audio {
                        disc_names.push(disc_name);
                    }
                }
            }

            if total_tracks > 0 {
                let meta = read_candidate_metadata(&sample_tracks);
                report.candidates.push(AlbumCandidate {
                    name: folder_name,
                    path_key: normalize_candidate_path_key(&dir_path),
                    path: dir_path,
                    kind: CandidateKind::MultiDiscFolder {
                        disc_subfolders: disc_names,
                    },
                    track_count: total_tracks,
                    metadata: meta,
                });
            }
        }
    }

    Ok(report)
}

/// Reads tags from the first track in `tracks`. If that track has no album tag,
/// attempts reading from the second track as a fallback.
fn read_candidate_metadata(tracks: &[PathBuf]) -> Option<AudioMetadata> {
    if tracks.is_empty() {
        return None;
    }

    let first_meta = read_audio_tags(&tracks[0]);
    if first_meta.as_ref().is_some_and(|m| m.album.is_some()) {
        return first_meta;
    }

    if tracks.len() > 1 {
        let second_meta = read_audio_tags(&tracks[1]);
        if second_meta.as_ref().is_some_and(|m| m.album.is_some()) {
            return second_meta;
        }
    }

    first_meta
}

/// Inspects central directory of a zip archive to count audio entries without decompression.
fn count_zip_audio_entries(zip_path: &Path) -> usize {
    let file = match File::open(zip_path) {
        Ok(f) => f,
        Err(_) => return 0,
    };

    let mut archive = match ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => return 0,
    };

    let mut count = 0;
    for i in 0..archive.len() {
        if let Ok(file) = archive.by_index_raw(i) {
            let name = file.name();
            if is_audio_file(Path::new(name)) {
                count += 1;
            }
        }
    }

    count
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn detects_disc_folder_names() {
        assert!(is_disc_folder_name("CD1"));
        assert!(is_disc_folder_name("cd 2"));
        assert!(is_disc_folder_name("Disc 1"));
        assert!(is_disc_folder_name("disc-2"));
        assert!(is_disc_folder_name("disk_3"));
        assert!(is_disc_folder_name("Part IV"));
        assert!(!is_disc_folder_name("Album"));
        assert!(!is_disc_folder_name("Bonus Tracks"));
    }

    #[test]
    fn scans_folder_with_direct_audio_and_multidisc() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // 1. Regular album folder
        let album1 = root.join("Album One");
        fs::create_dir_all(&album1).unwrap();
        fs::write(album1.join("01_track.mp3"), b"dummy audio").unwrap();
        fs::write(album1.join("02_track.flac"), b"dummy audio").unwrap();
        fs::write(album1.join("cover.jpg"), b"cover image").unwrap();

        // 2. Multi-disc album folder
        let album2 = root.join("Album Two (2CD)");
        let cd1 = album2.join("CD1");
        let cd2 = album2.join("CD2");
        fs::create_dir_all(&cd1).unwrap();
        fs::create_dir_all(&cd2).unwrap();
        fs::write(cd1.join("01.mp3"), b"dummy").unwrap();
        fs::write(cd2.join("01.mp3"), b"dummy").unwrap();

        // 3. Hidden directory and to_delete directory (should be ignored)
        let hidden = root.join(".hidden");
        fs::create_dir_all(&hidden).unwrap();
        fs::write(hidden.join("01.mp3"), b"dummy").unwrap();

        let to_delete = root.join("Old_to_delete");
        fs::create_dir_all(&to_delete).unwrap();
        fs::write(to_delete.join("01.mp3"), b"dummy").unwrap();

        let cancel = CancellationToken::new();
        let report = scan_linked_folder(root, &cancel).unwrap();

        assert_eq!(report.candidates.len(), 2);
        let names: Vec<&str> = report.candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Album One"));
        assert!(names.contains(&"Album Two (2CD)"));

        let cd_candidate = report
            .candidates
            .iter()
            .find(|c| c.name == "Album Two (2CD)")
            .unwrap();
        assert_eq!(cd_candidate.track_count, 2);
        match &cd_candidate.kind {
            CandidateKind::MultiDiscFolder { disc_subfolders } => {
                assert_eq!(disc_subfolders.len(), 2);
            }
            other => panic!("expected MultiDiscFolder, got {:?}", other),
        }
    }

    #[test]
    fn ignores_symlinks_and_does_not_escape_root() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("artist");
        fs::create_dir_all(&root).unwrap();

        // External folder outside root
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.mp3"), b"secret").unwrap();

        // Symlink pointing to outside folder
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(&outside, root.join("symlink_outside")).unwrap();
        }

        let cancel = CancellationToken::new();
        let report = scan_linked_folder(&root, &cancel).unwrap();

        // Symlink should be skipped and recorded
        #[cfg(unix)]
        assert_eq!(report.skipped_symlinks.len(), 1);
        assert_eq!(report.candidates.len(), 0);
    }

    #[test]
    fn zip_file_is_detected_as_album_candidate() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let zip_path = root.join("My Album (2020) [FLAC].zip");

        {
            let file = File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("01 Track One.flac", options).unwrap();
            zip.start_file("02 Track Two.flac", options).unwrap();
            zip.start_file("cover.jpg", options).unwrap();
            zip.finish().unwrap();
        }

        let cancel = CancellationToken::new();
        let report = scan_linked_folder(root, &cancel).unwrap();

        assert_eq!(report.candidates.len(), 1);
        let cand = &report.candidates[0];
        assert_eq!(cand.name, "My Album (2020) [FLAC]");
        assert_eq!(cand.kind, CandidateKind::Zip);
        assert_eq!(cand.track_count, 2);
    }

    #[test]
    fn sibling_folder_isolation_and_unreadable_subdirectories() {
        let parent = tempdir().unwrap();
        let artist_dir = parent.path().join("artist");
        let sibling_dir = parent.path().join("sibling");
        fs::create_dir_all(&artist_dir).unwrap();
        fs::create_dir_all(&sibling_dir).unwrap();

        let album = artist_dir.join("Album");
        fs::create_dir_all(&album).unwrap();
        fs::write(album.join("01.mp3"), b"audio").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Make sibling unreadable: reading it would fail with permission denied
            let _ = fs::set_permissions(&sibling_dir, fs::Permissions::from_mode(0o000));

            // Also create an unreadable subdirectory inside artist
            let unreadable = artist_dir.join("unreadable_dir");
            let _ = fs::create_dir_all(&unreadable);
            let _ = fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000));
        }

        let cancel = CancellationToken::new();
        let report = scan_linked_folder(&artist_dir, &cancel).unwrap();

        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].name, "Album");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Restore permissions so tempdir cleanup succeeds
            let _ = fs::set_permissions(&sibling_dir, fs::Permissions::from_mode(0o755));
            let _ = fs::set_permissions(
                artist_dir.join("unreadable_dir"),
                fs::Permissions::from_mode(0o755),
            );
        }
    }

    #[test]
    fn respects_cancellation_promptly() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let cancel = CancellationToken::new();
        cancel.cancel();

        let res = scan_linked_folder(root, &cancel);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
    }

    #[test]
    fn handles_symlink_loop_without_hanging() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            // Symlink to itself / parent creating a loop
            let _ = symlink(root, root.join("loop_link"));
        }

        let cancel = CancellationToken::new();
        let report = scan_linked_folder(root, &cancel).unwrap();

        #[cfg(unix)]
        {
            assert_eq!(report.skipped_symlinks.len(), 1);
        }
        assert_eq!(report.candidates.len(), 0);
    }
}

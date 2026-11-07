use std::path::Path;

use crate::library::scan::is_audio_file;
use crate::types::{ItemKey, LocalStatus, Origin, ReleaseRow};

/// Verification status for a downloaded release on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyResult {
    /// Local files exist and contain valid audio (or non-empty zip).
    Present,
    /// Local files are absent or contain no audio files.
    Missing,
    /// Root or artist folder is inaccessible (drive unmounted or unreadable); absence not confirmed.
    Unknown,
}

/// Aggregated report of a verification pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyReport {
    pub present: Vec<ItemKey>,
    pub missing: Vec<ItemKey>,
    pub unknown: Vec<ItemKey>,
    /// Set true when >= 20 releases and > 30% of checked releases are missing in one pass.
    pub mass_missing: bool,
}

/// Checks whether a local path on disk contains audio files or is a non-empty audio archive.
/// For directories, searches direct children and one subfolder level (depth 2).
/// Folders containing only metadata or covers (e.g. `cover.jpg`) return false.
pub fn check_disk_release_path(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }

    if path.is_file() {
        return path.metadata().is_ok_and(|m| m.len() > 0);
    }

    if !path.is_dir() {
        return false;
    }

    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };

    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && is_audio_file(&p) {
            return true;
        }
        if p.is_dir()
            && let Ok(sub_entries) = std::fs::read_dir(&p)
        {
            for sub in sub_entries.flatten() {
                let sub_p = sub.path();
                if sub_p.is_file() && is_audio_file(&sub_p) {
                    return true;
                }
            }
        }
    }

    false
}

/// Verifies a single release against the disk and folder health state.
/// Returns `None` if the release should be skipped (e.g. not downloaded or unverified without a path).
pub fn verify_single_release(rel: &ReleaseRow, is_folder_probe_ok: bool) -> Option<VerifyResult> {
    if !rel.downloaded {
        return None;
    }

    // Unavailable folder safeguard: if folder is missing or unreadable, mark Unknown, never Missing.
    if !is_folder_probe_ok {
        return Some(VerifyResult::Unknown);
    }

    let Some(path) = &rel.local_path else {
        // Rows unverified with local_path IS NULL and origin = 'imported' are skipped
        if rel.local_status == Some(LocalStatus::Unverified) && rel.origin == Some(Origin::Imported)
        {
            return None;
        }
        if rel.local_status == Some(LocalStatus::Archived) {
            return None;
        }
        return Some(VerifyResult::Missing);
    };

    // Archived rows are re-checked only for restore detection (if files reappeared)
    if rel.local_status == Some(LocalStatus::Archived) {
        if check_disk_release_path(path) {
            return Some(VerifyResult::Present);
        }
        return None;
    }

    if check_disk_release_path(path) {
        Some(VerifyResult::Present)
    } else {
        Some(VerifyResult::Missing)
    }
}

/// Computes the mass-missing guard: true if >= 20 releases and > 30% of checked releases are missing.
pub fn is_mass_missing(missing_count: usize, total_checked: usize) -> bool {
    if total_checked == 0 || missing_count < 20 {
        return false;
    }
    (missing_count as f32 / total_checked as f32) > 0.30
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    use crate::types::{AudioFormat, ItemKey, ItemType, LocalStatus, Origin};

    fn make_test_row(
        key: ItemKey,
        downloaded: bool,
        path: Option<std::path::PathBuf>,
    ) -> ReleaseRow {
        ReleaseRow {
            key,
            band_id: 1,
            title: "Test Album".to_string(),
            artist_name: "Artist".to_string(),
            url: "https://example.com".to_string(),
            cover_url: None,
            art_id: None,
            classification: crate::types::Classification::Free,
            is_preorder: false,
            track_count: Some(10),
            published_at: None,
            modified_at: None,
            downloaded,
            downloaded_at: None,
            downloaded_format: Some(AudioFormat::Flac),
            local_path: path,
            local_status: if downloaded {
                Some(LocalStatus::Present)
            } else {
                None
            },
            origin: Some(Origin::App),
            has_remote_update: false,
            remote_update_reason: None,
        }
    }

    #[test]
    fn check_disk_path_detects_files_and_directories() {
        let dir = tempdir().unwrap();

        // Nonexistent
        let bogus = dir.path().join("missing");
        assert!(!check_disk_release_path(&bogus));

        // Zip file empty vs with content
        let empty_zip = dir.path().join("empty.zip");
        fs::write(&empty_zip, b"").unwrap();
        assert!(!check_disk_release_path(&empty_zip));

        let non_empty_zip = dir.path().join("full.zip");
        fs::write(&non_empty_zip, b"zipcontent").unwrap();
        assert!(check_disk_release_path(&non_empty_zip));

        // Directory with only cover.jpg
        let cover_only = dir.path().join("CoverOnly");
        fs::create_dir(&cover_only).unwrap();
        fs::write(cover_only.join("cover.jpg"), b"image").unwrap();
        assert!(!check_disk_release_path(&cover_only));

        // Directory with audio directly
        let with_audio = dir.path().join("WithAudio");
        fs::create_dir(&with_audio).unwrap();
        fs::write(with_audio.join("track.flac"), b"audio").unwrap();
        assert!(check_disk_release_path(&with_audio));

        // Directory with audio in subfolder (multi-disc)
        let disc_dir = dir.path().join("MultiDisc");
        let disc1 = disc_dir.join("CD1");
        fs::create_dir_all(&disc1).unwrap();
        fs::write(disc1.join("01.mp3"), b"audio").unwrap();
        assert!(check_disk_release_path(&disc_dir));
    }

    #[test]
    fn verify_single_release_safeguards_and_unverified() {
        let dir = tempdir().unwrap();
        let key = ItemKey::new(ItemType::Album, 100);

        let album_dir = dir.path().join("Album");
        fs::create_dir(&album_dir).unwrap();
        fs::write(album_dir.join("01.mp3"), b"audio").unwrap();

        let row = make_test_row(key, true, Some(album_dir));

        // 1. Folder probe is Ok -> Present
        assert_eq!(
            verify_single_release(&row, true),
            Some(VerifyResult::Present)
        );

        // 2. Folder probe is NOT Ok (unavailable drive) -> Unknown, NEVER Missing
        assert_eq!(
            verify_single_release(&row, false),
            Some(VerifyResult::Unknown)
        );

        // 3. File removed, probe Ok -> Missing
        let missing_row = make_test_row(key, true, Some(dir.path().join("deleted")));
        assert_eq!(
            verify_single_release(&missing_row, true),
            Some(VerifyResult::Missing)
        );

        // 4. File removed, probe NOT Ok -> Unknown
        assert_eq!(
            verify_single_release(&missing_row, false),
            Some(VerifyResult::Unknown)
        );

        // 5. Unverified row with no local_path is skipped
        let mut unverified_row = make_test_row(key, true, None);
        unverified_row.local_status = Some(LocalStatus::Unverified);
        unverified_row.origin = Some(Origin::Imported);
        assert_eq!(verify_single_release(&unverified_row, true), None);
    }

    #[test]
    fn mass_missing_guard_thresholds() {
        // Less than 20 missing -> false even if 100%
        assert!(!is_mass_missing(19, 19));
        // 20 missing out of 100 (20%) -> false
        assert!(!is_mass_missing(20, 100));
        // 20 missing out of 50 (40%) -> true
        assert!(is_mass_missing(20, 50));
        // 31 missing out of 100 (31%) -> true
        assert!(is_mass_missing(31, 100));
    }
}

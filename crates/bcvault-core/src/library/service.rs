use tokio_util::sync::CancellationToken;

use crate::db::repo::{library as lib_repo, releases as rel_repo};
use crate::library::folder_style::FolderStyle;
use crate::library::link::probe_folder;
use crate::library::matcher::{Conflict, LocalOrphan, MatchProposal, ReviewItem, assign_matches};
use crate::library::scan::scan_linked_folder;
use crate::types::{FolderState, ItemKey, LibraryPhase, ReleaseQuery};

/// Reconcile report containing all match proposals, review items, and unlinked tracks/folders.
#[derive(Debug, Clone, PartialEq)]
pub struct ReconcileReport {
    /// Database batch identifier created for this reconcile run.
    pub batch_id: u64,
    /// Target Bandcamp artist identifier.
    pub band_id: i64,
    /// Confident automatic match proposals.
    pub matched: Vec<MatchProposal>,
    /// Ambiguous proposals or low-confidence matches needing user review.
    pub review: Vec<ReviewItem>,
    /// Releases found on Bandcamp artist page but with no local files.
    pub page_only: Vec<ItemKey>,
    /// Local audio folders with no matching Bandcamp release.
    pub local_only: Vec<LocalOrphan>,
    /// Collisions where multiple candidates or releases conflict.
    pub conflicts: Vec<Conflict>,
    /// Previously archived releases whose files reappeared.
    pub restorable: Vec<MatchProposal>,
    /// Releases already linked to a folder inside the linked artist folder.
    /// Informational only: nothing is applied for them.
    pub linked: Vec<LinkedRelease>,
    /// Folder ids skipped because they were missing or unreadable.
    pub unavailable_folders: Vec<i64>,
}

/// A release that is already downloaded and linked to a local path.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedRelease {
    pub release_key: ItemKey,
    pub release_title: String,
    pub path: std::path::PathBuf,
}

/// Releases whose active `local_path` lies inside `root`.
pub fn linked_releases_under(
    releases: &[crate::types::ReleaseRow],
    root: &std::path::Path,
) -> Vec<LinkedRelease> {
    let root_key = lib_repo::normalize_path_key(root);
    releases
        .iter()
        .filter(|r| r.downloaded && r.local_status != Some(crate::types::LocalStatus::Archived))
        .filter_map(|r| {
            let p = r.local_path.as_ref()?;
            let k = lib_repo::normalize_path_key(p);
            (k == root_key || k.starts_with(&format!("{root_key}/"))).then(|| LinkedRelease {
                release_key: r.key,
                release_title: r.title.clone(),
                path: p.clone(),
            })
        })
        .collect()
}

/// Callback invoked for library progress updates: (phase, done, total, current item/path).
pub type LibraryProgressCallback<'a> = &'a dyn Fn(LibraryPhase, u32, u32, Option<String>);

/// Runs the scan and match pipeline for a linked artist folder:
/// 1. Probes the folder path; exits early if missing or unreadable.
/// 2. Scans the linked folder for audio candidates.
/// 3. Loads releases and ignored path keys from the database.
/// 4. Executes greedy 1-to-1 matching.
/// 5. Creates a database batch record and returns the `ReconcileReport`.
pub fn run_scan_and_match(
    conn: &mut rusqlite::Connection,
    band_id: i64,
    auto_match_min: f32,
    review_match_min: f32,
    cancel: &CancellationToken,
    progress_fn: Option<LibraryProgressCallback<'_>>,
    style: &FolderStyle,
) -> Result<Option<ReconcileReport>, anyhow::Error> {
    let folders = lib_repo::get_folders(conn, band_id)?;
    if folders.is_empty() {
        return Ok(None);
    }

    let mut ok_folders = Vec::new();
    let mut unavailable_folders = Vec::new();
    for f in folders {
        let state = probe_folder(&f.path);
        if state == FolderState::Ok {
            ok_folders.push(f);
        } else {
            unavailable_folders.push(f.id);
        }
    }

    if ok_folders.is_empty() {
        return Ok(None);
    }

    let mut all_candidates = Vec::new();
    for (idx, folder) in ok_folders.iter().enumerate() {
        if let Some(f) = progress_fn {
            f(
                LibraryPhase::Scanning,
                idx as u32,
                ok_folders.len() as u32,
                Some(folder.path.display().to_string()),
            );
        }
        let scan_report = scan_linked_folder(&folder.path, cancel)?;
        all_candidates.extend(scan_report.candidates);
    }

    all_candidates.sort_by(|a, b| {
        lib_repo::normalize_path_key(&a.path).cmp(&lib_repo::normalize_path_key(&b.path))
    });

    let query = ReleaseQuery {
        band_id: Some(band_id),
        ..Default::default()
    };
    let all_releases = rel_repo::query_releases(conn, &query)?;
    let ignored = lib_repo::get_ignored_local(conn, band_id)?;

    let active_linked_keys: std::collections::HashSet<String> = all_releases
        .iter()
        .filter(|r| r.downloaded && r.local_status != Some(crate::types::LocalStatus::Archived))
        .filter_map(|r| r.local_path.as_ref())
        .map(|p| lib_repo::normalize_path_key(p))
        .collect();

    let mut linked = Vec::new();
    let mut seen_linked = std::collections::HashSet::new();
    for folder in &ok_folders {
        for lr in linked_releases_under(&all_releases, &folder.path) {
            if seen_linked.insert(lr.release_key) {
                linked.push(lr);
            }
        }
    }

    let adoptable: Vec<_> = all_releases
        .into_iter()
        .filter(|r| !r.downloaded || r.local_status == Some(crate::types::LocalStatus::Archived))
        .collect();

    let unlinked_candidates: Vec<_> = all_candidates
        .into_iter()
        .filter(|c| !active_linked_keys.contains(&lib_repo::normalize_path_key(&c.path)))
        .collect();

    if let Some(f) = progress_fn {
        f(
            LibraryPhase::Matching,
            0,
            unlinked_candidates.len() as u32,
            None,
        );
    }

    let assignment = assign_matches(
        &unlinked_candidates,
        &adoptable,
        auto_match_min,
        review_match_min,
        &ignored,
        style,
    );

    let batch_id = lib_repo::create_batch(conn, "adopt", &crate::types::BatchSummary::default())?;

    // Attribute each local_only orphan to its owning folder and update per available folder count
    let mut counts: std::collections::HashMap<i64, u32> =
        ok_folders.iter().map(|f| (f.id, 0)).collect();
    for orphan in &assignment.local_only {
        if let Some(owner) = lib_repo::owning_folder(&ok_folders, &orphan.path) {
            *counts.entry(owner.id).or_default() += 1;
        }
    }
    for folder in &ok_folders {
        let c = counts.get(&folder.id).copied().unwrap_or(0);
        lib_repo::update_folder_local_only_count(conn, folder.id, c)?;
    }

    Ok(Some(ReconcileReport {
        batch_id: batch_id.max(0) as u64,
        band_id,
        matched: assignment.matched,
        review: assignment.review,
        page_only: assignment.page_only,
        local_only: assignment.local_only,
        conflicts: assignment.conflicts,
        restorable: assignment.restorable,
        linked,
        unavailable_folders,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    use crate::db::migrations;
    use crate::db::repo::artists::{self, UpsertArtist};
    use crate::db::repo::releases::DiscoveredRelease;
    use crate::types::{ItemKey, ItemType};

    #[test]
    fn run_scan_and_match_pipeline_end_to_end() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();

        // 1. Add artist and release
        let band_id = 42;
        artists::upsert(
            &conn,
            &UpsertArtist {
                band_id,
                slug: "aphex-twin",
                name: "Aphex Twin",
                url: "https://aphextwin.bandcamp.com",
                cover_url: None,
            },
        )
        .unwrap();

        let disc = DiscoveredRelease {
            key: ItemKey::new(ItemType::Album, 101),
            band_id,
            title: "Selected Ambient Works 85-92",
            artist_name: "Aphex Twin",
            url: "https://aphextwin.bandcamp.com/album/selected-ambient-works-85-92",
            cover_url: None,
            art_id: None,
        };
        rel_repo::upsert_discovered(&conn, &disc).unwrap();

        // Also add an unlinked release
        let disc2 = DiscoveredRelease {
            key: ItemKey::new(ItemType::Album, 102),
            band_id,
            title: "Drukqs",
            artist_name: "Aphex Twin",
            url: "https://aphextwin.bandcamp.com/album/drukqs",
            cover_url: None,
            art_id: None,
        };
        rel_repo::upsert_discovered(&conn, &disc2).unwrap();

        // 2. Set up linked folder on disk
        let dir = tempdir().unwrap();
        let artist_dir = dir.path().join("Aphex Twin");
        let album_dir = artist_dir.join("Aphex Twin - Selected Ambient Works 85-92");
        fs::create_dir_all(&album_dir).unwrap();
        fs::write(album_dir.join("01.mp3"), b"audio").unwrap();

        let orphan_dir = artist_dir.join("Unreleased Live 1993");
        fs::create_dir_all(&orphan_dir).unwrap();
        fs::write(orphan_dir.join("live.mp3"), b"audio").unwrap();

        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        lib_repo::add_folder(&mut conn, band_id, &artist_dir, &out_dir).unwrap();

        // 3. Run scan and match pipeline
        let progress_calls = std::sync::Mutex::new(Vec::new());
        let cancel = CancellationToken::new();

        let report = run_scan_and_match(
            &mut conn,
            band_id,
            0.85,
            0.70,
            &cancel,
            Some(&|phase, done, total, curr| {
                progress_calls
                    .lock()
                    .unwrap()
                    .push((phase, done, total, curr));
            }),
            &FolderStyle {
                templates: vec!["{artist} - {title}".to_string(), "{title}".to_string()],
                noise: Vec::new(),
            },
        )
        .unwrap()
        .expect("report should be returned");

        assert_eq!(report.band_id, band_id);
        assert_eq!(report.matched.len(), 1);
        assert_eq!(report.matched[0].release_key.id, 101);

        assert_eq!(report.page_only.len(), 1);
        assert_eq!(report.page_only[0].id, 102);

        assert_eq!(report.local_only.len(), 1);
        assert_eq!(report.local_only[0].name, "Unreleased Live 1993");

        // Verify progress callbacks
        let calls = progress_calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, LibraryPhase::Scanning);
        assert_eq!(calls[1].0, LibraryPhase::Matching);
    }

    #[test]
    fn test_10_service_candidates_in_two_folders() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();

        let band_id = 42;
        artists::upsert(
            &conn,
            &UpsertArtist {
                band_id,
                slug: "artist-42",
                name: "Artist 42",
                url: "https://artist42.bandcamp.com",
                cover_url: None,
            },
        )
        .unwrap();

        rel_repo::upsert_discovered(
            &conn,
            &DiscoveredRelease {
                key: ItemKey::new(ItemType::Album, 1),
                band_id,
                title: "Album One",
                artist_name: "Artist 42",
                url: "https://artist42.bandcamp.com/album/1",
                cover_url: None,
                art_id: None,
            },
        )
        .unwrap();

        rel_repo::upsert_discovered(
            &conn,
            &DiscoveredRelease {
                key: ItemKey::new(ItemType::Album, 2),
                band_id,
                title: "Album Two",
                artist_name: "Artist 42",
                url: "https://artist42.bandcamp.com/album/2",
                cover_url: None,
                art_id: None,
            },
        )
        .unwrap();

        let dir = tempdir().unwrap();
        let folder1 = dir.path().join("Folder1");
        let folder2 = dir.path().join("Folder2");
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let alb1 = folder1.join("Album One");
        fs::create_dir_all(&alb1).unwrap();
        fs::write(alb1.join("01.mp3"), b"audio").unwrap();

        let alb2 = folder2.join("Album Two");
        fs::create_dir_all(&alb2).unwrap();
        fs::write(alb2.join("01.mp3"), b"audio").unwrap();

        lib_repo::add_folder(&mut conn, band_id, &folder1, &out_dir).unwrap();
        lib_repo::add_folder(&mut conn, band_id, &folder2, &out_dir).unwrap();

        let cancel = CancellationToken::new();
        let report = run_scan_and_match(
            &mut conn,
            band_id,
            0.85,
            0.70,
            &cancel,
            None,
            &FolderStyle::default(),
        )
        .unwrap()
        .expect("report should be returned");

        assert_eq!(report.matched.len(), 2);
        let matched_keys: std::collections::HashSet<_> =
            report.matched.iter().map(|m| m.release_key.id).collect();
        assert!(matched_keys.contains(&1));
        assert!(matched_keys.contains(&2));
        assert!(report.unavailable_folders.is_empty());
    }

    #[test]
    fn test_11_service_same_album_folder_name_in_both_produces_duplicate_conflict() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();

        let band_id = 42;
        artists::upsert(
            &conn,
            &UpsertArtist {
                band_id,
                slug: "artist-42",
                name: "Artist 42",
                url: "https://artist42.bandcamp.com",
                cover_url: None,
            },
        )
        .unwrap();

        rel_repo::upsert_discovered(
            &conn,
            &DiscoveredRelease {
                key: ItemKey::new(ItemType::Album, 1),
                band_id,
                title: "Album One",
                artist_name: "Artist 42",
                url: "https://artist42.bandcamp.com/album/1",
                cover_url: None,
                art_id: None,
            },
        )
        .unwrap();

        let dir = tempdir().unwrap();
        let folder1 = dir.path().join("Folder1");
        let folder2 = dir.path().join("Folder2");
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let alb1 = folder1.join("Album One");
        fs::create_dir_all(&alb1).unwrap();
        fs::write(alb1.join("01.mp3"), b"audio").unwrap();

        let alb2 = folder2.join("Album One");
        fs::create_dir_all(&alb2).unwrap();
        fs::write(alb2.join("01.mp3"), b"audio").unwrap();

        lib_repo::add_folder(&mut conn, band_id, &folder1, &out_dir).unwrap();
        lib_repo::add_folder(&mut conn, band_id, &folder2, &out_dir).unwrap();

        let cancel = CancellationToken::new();
        let report = run_scan_and_match(
            &mut conn,
            band_id,
            0.85,
            0.70,
            &cancel,
            None,
            &FolderStyle::default(),
        )
        .unwrap()
        .expect("report should be returned");

        assert_eq!(report.conflicts.len(), 1);
        assert!(report.matched.is_empty());
    }

    #[test]
    fn test_12_service_local_only_orphans_in_each_folder_per_folder_counts() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();

        let band_id = 42;
        artists::upsert(
            &conn,
            &UpsertArtist {
                band_id,
                slug: "artist-42",
                name: "Artist 42",
                url: "https://artist42.bandcamp.com",
                cover_url: None,
            },
        )
        .unwrap();

        let dir = tempdir().unwrap();
        let folder1 = dir.path().join("Folder1");
        let folder2 = dir.path().join("Folder2");
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let orphan1 = folder1.join("Orphan 1");
        fs::create_dir_all(&orphan1).unwrap();
        fs::write(orphan1.join("01.mp3"), b"audio").unwrap();

        let orphan2a = folder2.join("Orphan 2A");
        fs::create_dir_all(&orphan2a).unwrap();
        fs::write(orphan2a.join("01.mp3"), b"audio").unwrap();

        let orphan2b = folder2.join("Orphan 2B");
        fs::create_dir_all(&orphan2b).unwrap();
        fs::write(orphan2b.join("01.mp3"), b"audio").unwrap();

        let f1 = lib_repo::add_folder(&mut conn, band_id, &folder1, &out_dir).unwrap();
        let f2 = lib_repo::add_folder(&mut conn, band_id, &folder2, &out_dir).unwrap();

        let cancel = CancellationToken::new();
        let report = run_scan_and_match(
            &mut conn,
            band_id,
            0.85,
            0.70,
            &cancel,
            None,
            &FolderStyle::default(),
        )
        .unwrap()
        .expect("report should be returned");

        assert_eq!(report.local_only.len(), 3);

        let count1: u32 = conn
            .query_row(
                "SELECT local_only_count FROM artist_folders WHERE id = ?1",
                rusqlite::params![f1.id],
                |r| r.get(0),
            )
            .unwrap();
        let count2: u32 = conn
            .query_row(
                "SELECT local_only_count FROM artist_folders WHERE id = ?1",
                rusqlite::params![f2.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count1, 1);
        assert_eq!(count2, 2);

        // Ignoring one orphan from folder2 decrements only folder2's count
        let key2a = lib_repo::normalize_path_key(&orphan2a);
        lib_repo::ignore_local(&conn, band_id, &key2a).unwrap();
        let count1_after: u32 = conn
            .query_row(
                "SELECT local_only_count FROM artist_folders WHERE id = ?1",
                rusqlite::params![f1.id],
                |r| r.get(0),
            )
            .unwrap();
        let count2_after: u32 = conn
            .query_row(
                "SELECT local_only_count FROM artist_folders WHERE id = ?1",
                rusqlite::params![f2.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count1_after, 1);
        assert_eq!(count2_after, 1);
    }

    #[test]
    fn test_13_service_one_folder_missing_one_ok() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();

        let band_id = 42;
        artists::upsert(
            &conn,
            &UpsertArtist {
                band_id,
                slug: "artist-42",
                name: "Artist 42",
                url: "https://artist42.bandcamp.com",
                cover_url: None,
            },
        )
        .unwrap();

        rel_repo::upsert_discovered(
            &conn,
            &DiscoveredRelease {
                key: ItemKey::new(ItemType::Album, 1),
                band_id,
                title: "Album One",
                artist_name: "Artist 42",
                url: "https://artist42.bandcamp.com/album/1",
                cover_url: None,
                art_id: None,
            },
        )
        .unwrap();

        let dir = tempdir().unwrap();
        let folder1 = dir.path().join("Folder1");
        let folder2 = dir.path().join("Folder2_missing");
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let alb1 = folder1.join("Album One");
        fs::create_dir_all(&alb1).unwrap();
        fs::write(alb1.join("01.mp3"), b"audio").unwrap();

        // Create folder2 on disk first so add_folder validates, set count, then delete it
        fs::create_dir_all(&folder2).unwrap();
        let f1 = lib_repo::add_folder(&mut conn, band_id, &folder1, &out_dir).unwrap();
        let f2 = lib_repo::add_folder(&mut conn, band_id, &folder2, &out_dir).unwrap();
        lib_repo::update_folder_local_only_count(&conn, f2.id, 5).unwrap();
        fs::remove_dir_all(&folder2).unwrap();

        let cancel = CancellationToken::new();
        let report = run_scan_and_match(
            &mut conn,
            band_id,
            0.85,
            0.70,
            &cancel,
            None,
            &FolderStyle::default(),
        )
        .unwrap()
        .expect("report should be returned");

        assert_eq!(report.matched.len(), 1);
        assert_eq!(report.matched[0].release_key.id, 1);
        assert_eq!(report.unavailable_folders, vec![f2.id]);

        // Unavailable folder count should be unchanged (still 5)
        let count2: u32 = conn
            .query_row(
                "SELECT local_only_count FROM artist_folders WHERE id = ?1",
                rusqlite::params![f2.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count2, 5);

        let count1: u32 = conn
            .query_row(
                "SELECT local_only_count FROM artist_folders WHERE id = ?1",
                rusqlite::params![f1.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count1, 0);
    }

    #[test]
    fn test_14_service_all_folders_unavailable() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();

        let band_id = 42;
        artists::upsert(
            &conn,
            &UpsertArtist {
                band_id,
                slug: "artist-42",
                name: "Artist 42",
                url: "https://artist42.bandcamp.com",
                cover_url: None,
            },
        )
        .unwrap();

        let dir = tempdir().unwrap();
        let folder1 = dir.path().join("Folder1");
        let folder2 = dir.path().join("Folder2");
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        fs::create_dir_all(&folder1).unwrap();
        fs::create_dir_all(&folder2).unwrap();
        lib_repo::add_folder(&mut conn, band_id, &folder1, &out_dir).unwrap();
        lib_repo::add_folder(&mut conn, band_id, &folder2, &out_dir).unwrap();

        // Delete both folders
        fs::remove_dir_all(&folder1).unwrap();
        fs::remove_dir_all(&folder2).unwrap();

        let cancel = CancellationToken::new();
        let report = run_scan_and_match(
            &mut conn,
            band_id,
            0.85,
            0.70,
            &cancel,
            None,
            &FolderStyle::default(),
        )
        .unwrap();

        assert_eq!(report, None);
    }
}

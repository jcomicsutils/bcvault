use tokio_util::sync::CancellationToken;

use crate::db::repo::{library as lib_repo, releases as rel_repo};
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
) -> Result<Option<ReconcileReport>, anyhow::Error> {
    let folder_opt = lib_repo::get_folder(conn, band_id)?;
    let folder = match folder_opt {
        Some(f) => f,
        None => return Ok(None),
    };

    let state = probe_folder(&folder.path);
    if state != FolderState::Ok {
        return Ok(None);
    }

    if let Some(f) = progress_fn {
        f(
            LibraryPhase::Scanning,
            0,
            0,
            Some(folder.path.display().to_string()),
        );
    }

    let scan_report = scan_linked_folder(&folder.path, cancel)?;

    let query = ReleaseQuery {
        band_id: Some(band_id),
        ..Default::default()
    };
    let releases = rel_repo::query_releases(conn, &query)?;
    let ignored = lib_repo::get_ignored_local(conn, band_id)?;

    if let Some(f) = progress_fn {
        f(
            LibraryPhase::Matching,
            0,
            scan_report.candidates.len() as u32,
            None,
        );
    }

    let assignment = assign_matches(
        &scan_report.candidates,
        &releases,
        auto_match_min,
        review_match_min,
        &ignored,
    );

    let batch_id = lib_repo::create_batch(conn, "adopt", &crate::types::BatchSummary::default())?;

    Ok(Some(ReconcileReport {
        batch_id: batch_id.max(0) as u64,
        band_id,
        matched: assignment.matched,
        review: assignment.review,
        page_only: assignment.page_only,
        local_only: assignment.local_only,
        conflicts: assignment.conflicts,
        restorable: assignment.restorable,
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

        lib_repo::link_folder(&mut conn, band_id, &artist_dir, &out_dir).unwrap();

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
}

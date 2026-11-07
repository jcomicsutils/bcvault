use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::db::DbPool;
use crate::db::repo::{artists, library as lib_repo, releases, settings};
use crate::error::DbError;
use crate::library::link::probe_folder;
use crate::library::matcher::assign_matches;
use crate::library::scan::scan_linked_folder;
use crate::library::service::ReconcileReport;
use crate::types::{FolderState, LibraryPhase, LogLevel, ReleaseQuery};
use crate::worker::bridge::UiBridge;
use crate::worker::messages::WorkerToUiEvent;

/// Manages background folder linking, scanning, and release matching for artist folders.
/// Ensures at most one library job runs concurrently.
#[derive(Clone)]
pub struct LibraryService {
    pool: DbPool,
    bridge: UiBridge,
    job_lock: Arc<Mutex<()>>,
    current_cancel: Arc<Mutex<Option<CancellationToken>>>,
}

impl LibraryService {
    /// Creates a new `LibraryService` instance.
    pub fn new(pool: DbPool, bridge: UiBridge) -> Self {
        Self {
            pool,
            bridge,
            job_lock: Arc::new(Mutex::new(())),
            current_cancel: Arc::new(Mutex::new(None)),
        }
    }

    /// Cancels any currently executing library job.
    pub fn cancel(&self) {
        let cancel_slot = self.current_cancel.clone();
        tokio::spawn(async move {
            let mut guard = cancel_slot.lock().await;
            if let Some(token) = guard.take() {
                token.cancel();
            }
        });
    }

    /// Validates and links an artist folder, probes its health, and triggers an initial match
    /// if the artist has completed its first sync.
    pub async fn link_artist_folder(
        &self,
        band_id: i64,
        path: PathBuf,
        auto_apply_confident: bool,
        cancel: CancellationToken,
    ) {
        let _guard = self.job_lock.lock().await;
        {
            let mut c_guard = self.current_cancel.lock().await;
            *c_guard = Some(cancel.clone());
        }

        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

        let st_res = pool
            .reader
            .call(|c| settings::get_settings(c, PathBuf::from("downloads")))
            .await;
        let st = match st_res {
            Ok(s) => s,
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to read settings: {e}"),
                    None,
                );
                return;
            }
        };

        let link_path = path.clone();
        let link_res = pool
            .writer
            .call(move |c| Ok(lib_repo::link_folder(c, band_id, &link_path, &st.out_dir)?))
            .await;

        match link_res {
            Ok(_) => {
                bridge.log(
                    LogLevel::Info,
                    "library",
                    format!("Linked folder '{}' to artist {band_id}", path.display()),
                    None,
                );
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to link folder '{}': {e}", path.display()),
                    None,
                );
                return;
            }
        }

        let state = probe_folder(&path);
        bridge.emit(WorkerToUiEvent::FolderStatus { band_id, state });

        if let Ok(Some(av)) = pool
            .reader
            .call(move |c| artists::get_view(c, band_id))
            .await
        {
            bridge.emit(WorkerToUiEvent::ArtistUpserted(av));
        }

        if state != FolderState::Ok {
            return;
        }

        let has_releases = pool
            .reader
            .call(move |c| {
                let q = ReleaseQuery {
                    band_id: Some(band_id),
                    ..Default::default()
                };
                let rels = releases::query_releases(c, &q)?;
                Ok::<_, DbError>(!rels.is_empty())
            })
            .await
            .unwrap_or(false);

        if !has_releases {
            bridge.log(
                LogLevel::Info,
                "library",
                format!("Artist {band_id} has not completed its first sync; match queued until sync finishes"),
                None,
            );
            return;
        }

        self.match_artist_folder_internal(band_id, auto_apply_confident, cancel)
            .await;
    }

    /// Removes an existing artist folder link.
    pub async fn unlink_artist_folder(&self, band_id: i64) {
        let _guard = self.job_lock.lock().await;
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

        let unlink_res = pool
            .writer
            .call(move |c| lib_repo::unlink_folder(c, band_id))
            .await;
        match unlink_res {
            Ok(true) => {
                bridge.log(
                    LogLevel::Info,
                    "library",
                    format!("Unlinked folder for artist {band_id}"),
                    None,
                );
                bridge.emit(WorkerToUiEvent::FolderStatus {
                    band_id,
                    state: FolderState::Missing,
                });
                if let Ok(Some(av)) = pool
                    .reader
                    .call(move |c| artists::get_view(c, band_id))
                    .await
                {
                    bridge.emit(WorkerToUiEvent::ArtistUpserted(av));
                }
            }
            Ok(false) => {
                bridge.log(
                    LogLevel::Warn,
                    "library",
                    format!("No linked folder found for artist {band_id}"),
                    None,
                );
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to unlink folder for artist {band_id}: {e}"),
                    None,
                );
            }
        }
    }

    /// Scans a linked artist folder and computes matching proposals against releases.
    pub async fn match_artist_folder(
        &self,
        band_id: i64,
        auto_apply_confident: bool,
        cancel: CancellationToken,
    ) {
        let _guard = self.job_lock.lock().await;
        {
            let mut c_guard = self.current_cancel.lock().await;
            *c_guard = Some(cancel.clone());
        }
        self.match_artist_folder_internal(band_id, auto_apply_confident, cancel)
            .await;
    }

    async fn match_artist_folder_internal(
        &self,
        band_id: i64,
        _auto_apply_confident: bool,
        cancel: CancellationToken,
    ) {
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

        let folder_opt = pool
            .reader
            .call(move |c| lib_repo::get_folder(c, band_id))
            .await;
        let folder = match folder_opt {
            Ok(Some(f)) => f,
            Ok(None) => {
                bridge.log(
                    LogLevel::Warn,
                    "library",
                    format!("No linked folder for artist {band_id}"),
                    None,
                );
                return;
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to query folder for artist {band_id}: {e}"),
                    None,
                );
                return;
            }
        };

        let state = probe_folder(&folder.path);
        bridge.emit(WorkerToUiEvent::FolderStatus { band_id, state });
        if state != FolderState::Ok {
            bridge.log(
                LogLevel::Warn,
                "library",
                format!("Folder is {state:?}: {}", folder.path.display()),
                None,
            );
            return;
        }

        let st = match pool
            .reader
            .call(|c| settings::get_settings(c, PathBuf::from("downloads")))
            .await
        {
            Ok(s) => s,
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to read settings: {e}"),
                    None,
                );
                return;
            }
        };

        bridge.emit(WorkerToUiEvent::LibraryProgress {
            phase: LibraryPhase::Scanning,
            done: 0,
            total: 0,
            current: Some(folder.path.display().to_string()),
        });

        let scan_path = folder.path.clone();
        let scan_cancel = cancel.clone();
        let scan_res =
            tokio::task::spawn_blocking(move || scan_linked_folder(&scan_path, &scan_cancel)).await;
        let scan_report = match scan_res {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to scan folder '{}': {e}", folder.path.display()),
                    None,
                );
                return;
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Scan task panicked: {e}"),
                    None,
                );
                return;
            }
        };

        if cancel.is_cancelled() {
            return;
        }

        let query = ReleaseQuery {
            band_id: Some(band_id),
            ..Default::default()
        };
        let rel_res = pool
            .reader
            .call(move |c| {
                let rels = releases::query_releases(c, &query)?;
                let ign = lib_repo::get_ignored_local(c, band_id)?;
                Ok::<_, DbError>((rels, ign))
            })
            .await;

        let (releases, ignored) = match rel_res {
            Ok(pair) => pair,
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to load releases for matching: {e}"),
                    None,
                );
                return;
            }
        };

        bridge.emit(WorkerToUiEvent::LibraryProgress {
            phase: LibraryPhase::Matching,
            done: 0,
            total: scan_report.candidates.len() as u32,
            current: None,
        });

        let assignment = assign_matches(
            &scan_report.candidates,
            &releases,
            st.auto_match_min,
            st.review_match_min,
            &ignored,
        );

        let batch_id = match pool
            .writer
            .call(|c| lib_repo::create_batch(c, "adopt", &crate::types::BatchSummary::default()))
            .await
        {
            Ok(id) => id.max(0) as u64,
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to create batch: {e}"),
                    None,
                );
                return;
            }
        };

        let report = ReconcileReport {
            batch_id,
            band_id,
            matched: assignment.matched,
            review: assignment.review,
            page_only: assignment.page_only,
            local_only: assignment.local_only,
            conflicts: assignment.conflicts,
            restorable: assignment.restorable,
        };

        bridge.emit(WorkerToUiEvent::ReconcileReport {
            report: Box::new(report.clone()),
        });

        bridge.log(
            LogLevel::Info,
            "library",
            format!(
                "Reconcile report ready: {} matched, {} review, {} local only, {} page only",
                report.matched.len(),
                report.review.len(),
                report.local_only.len(),
                report.page_only.len()
            ),
            None,
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    use tokio::sync::mpsc::unbounded_channel;

    use crate::db::DbPool;
    use crate::db::migrations;
    use crate::db::repo::artists::{self, UpsertArtist};
    use crate::db::repo::releases::DiscoveredRelease;
    use crate::types::{ItemKey, ItemType};

    #[tokio::test]
    async fn test_library_service_link_match_and_unlink() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Run migrations on DB first
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 1234;
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "boards-of-canada",
                        name: "Boards of Canada",
                        url: "https://boc.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let disc = DiscoveredRelease {
                    key: ItemKey::new(ItemType::Album, 1),
                    band_id,
                    title: "Music Has the Right to Children",
                    artist_name: "Boards of Canada",
                    url: "https://boc.bandcamp.com/album/music-has-the-right-to-children",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        // Prepare local folder
        let artist_dir = dir.path().join("Boards of Canada");
        let album_dir = artist_dir.join("Boards of Canada - Music Has the Right to Children");
        fs::create_dir_all(&album_dir).unwrap();
        fs::write(album_dir.join("01.mp3"), b"audio").unwrap();

        let cancel = CancellationToken::new();

        // 1. Link artist folder
        svc.link_artist_folder(band_id, artist_dir.clone(), false, cancel.clone())
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Check emitted events
        let has_folder_ok = events.iter().any(|e| {
            matches!(
                e,
                WorkerToUiEvent::FolderStatus {
                    band_id: b,
                    state: FolderState::Ok
                } if *b == band_id
            )
        });
        assert!(has_folder_ok);

        let report_opt = events.iter().find_map(|e| match e {
            WorkerToUiEvent::ReconcileReport { report } => Some(report),
            _ => None,
        });
        let report = report_opt.expect("should have emitted ReconcileReport");
        assert_eq!(report.band_id, band_id);
        assert_eq!(report.matched.len(), 1);
        assert_eq!(report.matched[0].release_key.id, 1);

        // Verify folder is linked in DB
        let folder_in_db = pool
            .reader
            .call(move |c| lib_repo::get_folder(c, band_id))
            .await
            .unwrap()
            .expect("folder should be linked");
        assert_eq!(folder_in_db.path, dunce::canonicalize(&artist_dir).unwrap());

        // 2. Unlink artist folder
        svc.unlink_artist_folder(band_id).await;

        let mut unlink_events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            unlink_events.push(ev);
        }

        let has_folder_missing = unlink_events.iter().any(|e| {
            matches!(
                e,
                WorkerToUiEvent::FolderStatus {
                    band_id: b,
                    state: FolderState::Missing
                } if *b == band_id
            )
        });
        assert!(has_folder_missing);

        let folder_after_unlink = pool
            .reader
            .call(move |c| lib_repo::get_folder(c, band_id))
            .await
            .unwrap();
        assert!(folder_after_unlink.is_none());
    }

    #[tokio::test]
    async fn test_library_service_cancellation() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("cancel_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 999;
        let artist_dir = dir.path().join("Artist");
        fs::create_dir_all(&artist_dir).unwrap();

        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist",
                        name: "Artist",
                        url: "https://artist.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let out_dir = dir.path().join("Downloads");
                lib_repo::link_folder(c, band_id, &artist_dir, &out_dir)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        cancel.cancel(); // Pre-cancel

        svc.match_artist_folder(band_id, false, cancel).await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // ReconcileReport should NOT be emitted because cancelled
        let has_report = events
            .iter()
            .any(|e| matches!(e, WorkerToUiEvent::ReconcileReport { .. }));
        assert!(!has_report);
    }
}

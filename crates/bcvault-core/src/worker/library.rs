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
use crate::library::verify::{
    VerifyResult, find_relink_candidates, is_mass_missing, verify_single_release,
};
use crate::types::{
    ArtistFolder, BatchSummary, DeletedPolicy, FolderState, ItemKey, LibraryPhase, LocalStatus,
    LogLevel, MatchDecision, MissingAction, MissingCandidate, MissingDecision, MissingItem,
    OnMissing, ReleaseQuery, ReleaseRow, RememberChoice, RescanOutcome, SyncScope, VerifyOutcome,
};
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
    pending_first_matches: Arc<Mutex<std::collections::HashMap<i64, bool>>>,
}

impl LibraryService {
    /// Creates a new `LibraryService` instance.
    pub fn new(pool: DbPool, bridge: UiBridge) -> Self {
        Self {
            pool,
            bridge,
            job_lock: Arc::new(Mutex::new(())),
            current_cancel: Arc::new(Mutex::new(None)),
            pending_first_matches: Arc::new(Mutex::new(std::collections::HashMap::new())),
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
        let af = ArtistFolder {
            band_id,
            path: path.clone(),
            state,
        };
        bridge.emit(WorkerToUiEvent::FolderStatus {
            band_id,
            folder: Some(af),
        });

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
            self.pending_first_matches
                .lock()
                .await
                .insert(band_id, auto_apply_confident);
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
        self.pending_first_matches.lock().await.remove(&band_id);
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
                    folder: None,
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

    /// Applies user-accepted reconcile decisions for a batch and emits updates.
    pub async fn apply_reconcile(&self, batch_id: u64, decisions: Vec<MatchDecision>) {
        let _guard = self.job_lock.lock().await;
        self.apply_reconcile_internal(batch_id, decisions).await;
    }

    /// Resolves user decisions from the Missing files dialog.
    pub async fn resolve_missing(
        &self,
        batch_id: u64,
        decisions: Vec<MissingDecision>,
        remember: Option<RememberChoice>,
    ) {
        let _guard = self.job_lock.lock().await;
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

        if let Some(rem) = remember {
            let rem_res = pool
                .writer
                .call(move |c| {
                    let mut st = settings::get_settings(c, PathBuf::from("downloads"))?;
                    if let Some(dp) = rem.set_default_deleted_policy {
                        st.deleted_policy = dp;
                    }
                    if rem.stop_asking {
                        st.on_missing = OnMissing::Auto;
                    }
                    settings::set_settings(c, &st)?;
                    Ok::<_, DbError>(st)
                })
                .await;
            if let Ok(st) = rem_res {
                bridge.emit(WorkerToUiEvent::SettingsUpdated(st));
            }
        }

        let apply_res = pool
            .writer
            .call(move |c| {
                let mut touched_keys = Vec::new();
                for dec in decisions {
                    touched_keys.push(dec.key);
                    match dec.action {
                        MissingAction::Moved(new_path) => {
                            lib_repo::relink(c, dec.key, &new_path, batch_id as i64)?;
                        }
                        MissingAction::Deleted(policy) => match policy {
                            DeletedPolicy::KeepMarked => {
                                lib_repo::archive(c, &[dec.key], batch_id as i64)?;
                            }
                            DeletedPolicy::MarkNotDownloaded => {
                                lib_repo::unmark(c, &[dec.key], batch_id as i64)?;
                            }
                        },
                        MissingAction::Skip => {
                            lib_repo::skip_missing(c, dec.key)?;
                        }
                    }
                }

                let summary = lib_repo::get_batch_summary(c, batch_id as i64)?.unwrap_or_default();
                let mut updated = Vec::new();
                let mut bands = std::collections::HashSet::new();
                for key in touched_keys {
                    if let Some(r) = releases::get_release(c, key)? {
                        bands.insert(r.band_id);
                        updated.push(r);
                    }
                }
                let mut stats_list = Vec::new();
                for b in bands {
                    stats_list.push((b, artists::stats_with_quota(c, b)?));
                }
                Ok::<_, DbError>((summary, updated, stats_list))
            })
            .await;

        match apply_res {
            Ok((summary, updated_releases, stats_list)) => {
                for r in updated_releases {
                    bridge.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(r)));
                }
                for (band_id, stats) in stats_list {
                    bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
                }
                bridge.emit(WorkerToUiEvent::LibraryApplied { batch_id, summary });
                bridge.log(
                    LogLevel::Info,
                    "library",
                    format!("Resolved missing batch {batch_id}"),
                    None,
                );
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to resolve missing batch {batch_id}: {e}"),
                    None,
                );
            }
        }
    }

    /// Re-points an artist's linked folder to a new path and rescans/relinks missing releases.
    pub async fn repoint_artist_folder(
        &self,
        band_id: i64,
        new_path: PathBuf,
        cancel: CancellationToken,
    ) {
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

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

        let link_path = new_path.clone();
        let link_res = pool
            .writer
            .call(move |c| Ok(lib_repo::link_folder(c, band_id, &link_path, &st.out_dir)?))
            .await;

        match link_res {
            Ok(af) => {
                bridge.log(
                    LogLevel::Info,
                    "library",
                    format!("Repointed artist {band_id} to '{}'", af.path.display()),
                    None,
                );
                let state = probe_folder(&af.path);
                let updated_af = ArtistFolder {
                    band_id,
                    path: af.path.clone(),
                    state,
                };
                bridge.emit(WorkerToUiEvent::FolderStatus {
                    band_id,
                    folder: Some(updated_af),
                });
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to repoint artist {band_id}: {e}"),
                    None,
                );
                return;
            }
        }

        if let Ok(Some(av)) = pool
            .reader
            .call(move |c| artists::get_view(c, band_id))
            .await
        {
            bridge.emit(WorkerToUiEvent::ArtistUpserted(av));
        }

        self.verify_files(SyncScope::Selected(vec![band_id]), false, cancel)
            .await;
    }

    /// Manually relinks a single release to a local path.
    pub async fn relink_release(&self, key: ItemKey, path: PathBuf) {
        let _guard = self.job_lock.lock().await;
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();
        let res = pool
            .writer
            .call(move |c| {
                let b_id = lib_repo::create_batch(c, "manual", &BatchSummary::default())?;
                lib_repo::relink(c, key, &path, b_id)?;
                let summary = lib_repo::get_batch_summary(c, b_id)?.unwrap_or_default();
                let rel = releases::get_release(c, key)?;
                let stats = if let Some(r) = &rel {
                    Some((r.band_id, artists::stats_with_quota(c, r.band_id)?))
                } else {
                    None
                };
                Ok::<_, DbError>((b_id as u64, summary, rel, stats))
            })
            .await;

        if let Ok((b_id, summary, Some(rel), stats_opt)) = res {
            bridge.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(rel)));
            if let Some((band_id, stats)) = stats_opt {
                bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
            }
            bridge.emit(WorkerToUiEvent::LibraryApplied {
                batch_id: b_id,
                summary,
            });
        }
    }

    /// Manually marks releases as archived.
    pub async fn mark_archived(&self, keys: Vec<ItemKey>) {
        let _guard = self.job_lock.lock().await;
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();
        let res = pool
            .writer
            .call(move |c| {
                let b_id = lib_repo::create_batch(c, "manual", &BatchSummary::default())?;
                lib_repo::archive(c, &keys, b_id)?;
                let summary = lib_repo::get_batch_summary(c, b_id)?.unwrap_or_default();
                let mut updated = Vec::new();
                let mut bands = std::collections::HashSet::new();
                for &k in &keys {
                    if let Some(r) = releases::get_release(c, k)? {
                        bands.insert(r.band_id);
                        updated.push(r);
                    }
                }
                let mut stats_list = Vec::new();
                for b in bands {
                    stats_list.push((b, artists::stats_with_quota(c, b)?));
                }
                Ok::<_, DbError>((b_id as u64, summary, updated, stats_list))
            })
            .await;

        if let Ok((b_id, summary, updated, stats_list)) = res {
            for r in updated {
                bridge.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(r)));
            }
            for (band_id, stats) in stats_list {
                bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
            }
            bridge.emit(WorkerToUiEvent::LibraryApplied {
                batch_id: b_id,
                summary,
            });
        }
    }

    /// Manually unmarks releases as downloaded.
    pub async fn unmark_downloaded(&self, keys: Vec<ItemKey>) {
        let _guard = self.job_lock.lock().await;
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();
        let res = pool
            .writer
            .call(move |c| {
                let b_id = lib_repo::create_batch(c, "manual", &BatchSummary::default())?;
                lib_repo::unmark(c, &keys, b_id)?;
                let summary = lib_repo::get_batch_summary(c, b_id)?.unwrap_or_default();
                let mut updated = Vec::new();
                let mut bands = std::collections::HashSet::new();
                for &k in &keys {
                    if let Some(r) = releases::get_release(c, k)? {
                        bands.insert(r.band_id);
                        updated.push(r);
                    }
                }
                let mut stats_list = Vec::new();
                for b in bands {
                    stats_list.push((b, artists::stats_with_quota(c, b)?));
                }
                Ok::<_, DbError>((b_id as u64, summary, updated, stats_list))
            })
            .await;

        if let Ok((b_id, summary, updated, stats_list)) = res {
            for r in updated {
                bridge.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(r)));
            }
            for (band_id, stats) in stats_list {
                bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
            }
            bridge.emit(WorkerToUiEvent::LibraryApplied {
                batch_id: b_id,
                summary,
            });
        }
    }

    /// Ignores specified local paths for an artist.
    pub async fn ignore_local(&self, band_id: i64, paths: Vec<PathBuf>) {
        let _guard = self.job_lock.lock().await;
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();
        let res = pool
            .writer
            .call(move |c| {
                for p in &paths {
                    let key = crate::library::link::normalize_path_key(p);
                    lib_repo::ignore_local(c, band_id, &key)?;
                }
                Ok::<_, DbError>(())
            })
            .await;
        if let Err(e) = res {
            bridge.log(
                LogLevel::Error,
                "library",
                format!("Failed to ignore paths for artist {band_id}: {e}"),
                None,
            );
        }
    }

    /// Verifies all downloaded releases in the given scope against the filesystem.
    /// Emits progress events and, if any files are missing, a MissingFound event.
    pub async fn verify_files(
        &self,
        scope: SyncScope,
        is_sync: bool,
        cancel: CancellationToken,
    ) -> VerifyOutcome {
        let _guard = self.job_lock.lock().await;
        {
            let mut c_guard = self.current_cancel.lock().await;
            *c_guard = Some(cancel.clone());
        }

        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

        // 1. Read settings for out_dir
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
                return VerifyOutcome::default();
            }
        };

        // 2. Determine artist IDs in scope
        let band_ids: Vec<i64> = match scope {
            SyncScope::Selected(v) => v,
            SyncScope::All => {
                let ids_res = pool
                    .reader
                    .call(|c| {
                        let views = artists::list_views(c)?;
                        Ok::<_, DbError>(views.into_iter().map(|v| v.band_id).collect())
                    })
                    .await;
                match ids_res {
                    Ok(ids) => ids,
                    Err(e) => {
                        bridge.log(
                            LogLevel::Error,
                            "library",
                            format!("Failed to query artists: {e}"),
                            None,
                        );
                        return VerifyOutcome::default();
                    }
                }
            }
        };

        // 3. Probe folder health for each artist in scope
        let mut folder_probe_ok = std::collections::HashMap::new();
        for &band_id in &band_ids {
            let folder_opt = pool
                .reader
                .call(move |c| lib_repo::get_folder(c, band_id))
                .await
                .unwrap_or(None);

            let state = if let Some(folder) = folder_opt {
                probe_folder(&folder.path)
            } else {
                probe_folder(&st.out_dir)
            };

            if state != FolderState::Ok {
                bridge.log(
                    LogLevel::Warn,
                    "library",
                    format!("Folder unavailable for artist {band_id}: {state:?}"),
                    None,
                );
            }
            folder_probe_ok.insert(band_id, state == FolderState::Ok);
        }

        // 4. Query all downloaded = 1 releases in scope
        let releases_res = pool
            .reader
            .call(move |c| {
                let mut rows = Vec::new();
                for band_id in band_ids {
                    let q = ReleaseQuery {
                        band_id: Some(band_id),
                        ..Default::default()
                    };
                    let rels = releases::query_releases(c, &q)?;
                    rows.extend(rels.into_iter().filter(|r| r.downloaded));
                }
                Ok::<_, DbError>(rows)
            })
            .await;

        let downloaded_releases = match releases_res {
            Ok(r) => r,
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to query downloaded releases: {e}"),
                    None,
                );
                return VerifyOutcome::default();
            }
        };

        let total_releases = downloaded_releases.len() as u32;
        bridge.emit(WorkerToUiEvent::LibraryProgress {
            phase: LibraryPhase::Verifying,
            done: 0,
            total: total_releases,
            current: None,
        });

        // 5. Verify releases in spawn_blocking
        let probe_map_clone = folder_probe_ok.clone();
        let cancel_verify = cancel.clone();
        let releases_clone = downloaded_releases.clone();

        let verify_res = tokio::task::spawn_blocking(move || {
            let mut present = Vec::new();
            let mut missing = Vec::new();
            let mut unknown = Vec::new();

            for rel in releases_clone {
                if cancel_verify.is_cancelled() {
                    break;
                }
                let is_ok = probe_map_clone.get(&rel.band_id).copied().unwrap_or(false);
                let outcome = verify_single_release(&rel, is_ok, is_sync);
                match outcome {
                    Some(VerifyResult::Present) => present.push(rel),
                    Some(VerifyResult::Missing) => missing.push(rel),
                    Some(VerifyResult::Unknown) => unknown.push(rel),
                    None => {}
                }
            }

            (present, missing, unknown)
        })
        .await;

        let (present_rels, missing_rels, unknown_rels) = match verify_res {
            Ok(triplet) => triplet,
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Verification task failed: {e}"),
                    None,
                );
                return VerifyOutcome::default();
            }
        };

        if cancel.is_cancelled() {
            return VerifyOutcome::default();
        }

        // 6. Update present rows in DB
        let present_keys: Vec<ItemKey> = present_rels.iter().map(|r| r.key).collect();
        let present_keys_db = present_keys.clone();
        let update_res = pool
            .writer
            .call(move |c| {
                let mut updated = Vec::new();
                for key in present_keys_db {
                    lib_repo::set_local_status(c, key, Some(LocalStatus::Present))?;
                    if let Some(r) = releases::get_release(c, key)? {
                        updated.push(r);
                    }
                }
                Ok::<_, DbError>(updated)
            })
            .await;

        if let Ok(updated_rows) = update_res {
            for row in updated_rows {
                bridge.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(row)));
            }
        }

        // 7. Check mass-missing guard
        let total_evaluated = present_rels.len() + missing_rels.len();
        let mass_missing = is_mass_missing(missing_rels.len(), total_evaluated);

        if mass_missing {
            bridge.log(
                LogLevel::Warn,
                "library",
                format!(
                    "Many albums are missing ({} of {}): is a drive disconnected?",
                    missing_rels.len(),
                    total_evaluated
                ),
                None,
            );
        }

        // 8. Auto-relink moved albums (if enabled, artist has a link, and not mass_missing)
        let mut still_missing: Vec<ReleaseRow> = Vec::new();
        let mut best_candidates: std::collections::HashMap<ItemKey, MissingCandidate> =
            std::collections::HashMap::new();
        let mut relinked_count = 0;

        if !mass_missing && st.auto_relink && !missing_rels.is_empty() {
            let mut by_band: std::collections::HashMap<i64, Vec<ReleaseRow>> =
                std::collections::HashMap::new();
            for r in missing_rels {
                by_band.entry(r.band_id).or_default().push(r);
            }

            for (band_id, band_rels) in by_band {
                let folder_opt = pool
                    .reader
                    .call(move |c| lib_repo::get_folder(c, band_id))
                    .await
                    .unwrap_or(None);

                let Some(folder) = folder_opt else {
                    // Artists without a link skip auto-relink
                    still_missing.extend(band_rels);
                    continue;
                };

                let state = probe_folder(&folder.path);
                if state != FolderState::Ok {
                    still_missing.extend(band_rels);
                    continue;
                }

                // Rescan artist's linked folder
                let folder_path = folder.path.clone();
                let cancel_scan = cancel.clone();
                let scan_res = tokio::task::spawn_blocking(move || {
                    scan_linked_folder(&folder_path, &cancel_scan)
                })
                .await;

                let scan_report = match scan_res {
                    Ok(Ok(r)) => r,
                    _ => {
                        still_missing.extend(band_rels);
                        continue;
                    }
                };

                // Query existing present releases & ignored local keys
                let excluded_res = pool
                    .reader
                    .call(move |c| {
                        let q = ReleaseQuery {
                            band_id: Some(band_id),
                            ..Default::default()
                        };
                        let rels = releases::query_releases(c, &q)?;
                        let mut excluded = std::collections::HashSet::new();
                        for r in rels {
                            if r.downloaded
                                && r.local_status == Some(LocalStatus::Present)
                                && let Some(p) = &r.local_path
                            {
                                excluded.insert(crate::library::link::normalize_path_key(p));
                            }
                        }
                        let ign = lib_repo::get_ignored_local(c, band_id)?;
                        excluded.extend(ign);
                        Ok::<_, DbError>(excluded)
                    })
                    .await;

                let excluded_keys = excluded_res.unwrap_or_default();

                let outcome = find_relink_candidates(
                    &band_rels,
                    &scan_report.candidates,
                    &excluded_keys,
                    st.auto_match_min,
                );

                if !outcome.relinked.is_empty() {
                    let relink_props = outcome.relinked;
                    let relink_res = pool
                        .writer
                        .call(move |c| {
                            let b_id =
                                lib_repo::create_batch(c, "relink", &BatchSummary::default())?;
                            let mut updated = Vec::new();
                            for prop in relink_props {
                                lib_repo::relink(c, prop.key, &prop.path, b_id)?;
                                if let Some(r) = releases::get_release(c, prop.key)? {
                                    updated.push(r);
                                }
                            }
                            Ok::<_, DbError>(updated)
                        })
                        .await;

                    if let Ok(updated_releases) = relink_res {
                        relinked_count += updated_releases.len();
                        for r in updated_releases {
                            bridge.log(
                                LogLevel::Info,
                                "library",
                                format!(
                                    "Auto-relinked '{}' to '{}'",
                                    r.title,
                                    r.local_path
                                        .as_ref()
                                        .map(|p| p.display().to_string())
                                        .unwrap_or_default()
                                ),
                                None,
                            );
                            bridge.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(r)));
                        }
                    }
                }

                for (unresolved_rel, best_cand_opt) in outcome.unresolved {
                    if let Some(cand) = best_cand_opt {
                        best_candidates.insert(unresolved_rel.key, cand);
                    }
                    still_missing.push(unresolved_rel);
                }
            }
        } else {
            still_missing = missing_rels;
        }

        // 9. If missing releases remain:
        // When not mass_missing and on_missing == Auto: apply deleted_policy automatically!
        // Otherwise (Ask mode or mass_missing guard): create batch and emit MissingFound.
        let missing_count = still_missing.len();
        if !still_missing.is_empty() {
            let batch_id = match pool
                .writer
                .call(|c| lib_repo::create_batch(c, "verify", &BatchSummary::default()))
                .await
            {
                Ok(id) => id.max(0) as u64,
                Err(e) => {
                    bridge.log(
                        LogLevel::Error,
                        "library",
                        format!("Failed to create verification batch: {e}"),
                        None,
                    );
                    return VerifyOutcome::default();
                }
            };

            if !mass_missing && st.on_missing == OnMissing::Auto {
                let keys: Vec<ItemKey> = still_missing.iter().map(|r| r.key).collect();
                let policy = st.deleted_policy;
                let b_id = batch_id;
                let auto_res = pool
                    .writer
                    .call(move |c| {
                        match policy {
                            DeletedPolicy::KeepMarked => {
                                lib_repo::archive(c, &keys, b_id as i64)?;
                            }
                            DeletedPolicy::MarkNotDownloaded => {
                                lib_repo::unmark(c, &keys, b_id as i64)?;
                            }
                        }
                        let mut updated = Vec::new();
                        let mut bands = std::collections::HashSet::new();
                        for &k in &keys {
                            if let Some(r) = releases::get_release(c, k)? {
                                bands.insert(r.band_id);
                                updated.push(r);
                            }
                        }
                        let mut stats_list = Vec::new();
                        for b in bands {
                            stats_list.push((b, artists::stats_with_quota(c, b)?));
                        }
                        let summary =
                            lib_repo::get_batch_summary(c, b_id as i64)?.unwrap_or_default();
                        Ok::<_, DbError>((summary, updated, stats_list))
                    })
                    .await;

                if let Ok((summary, updated_releases, stats_list)) = auto_res {
                    for r in updated_releases {
                        bridge.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(r)));
                    }
                    for (band_id, stats) in stats_list {
                        bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
                    }
                    bridge.emit(WorkerToUiEvent::LibraryApplied { batch_id, summary });
                    bridge.log(
                        LogLevel::Info,
                        "library",
                        format!(
                            "Auto-resolved {} missing releases as deleted ({:?})",
                            still_missing.len(),
                            policy
                        ),
                        None,
                    );
                }
            } else {
                let missing_items: Vec<MissingItem> = still_missing
                    .into_iter()
                    .map(|r| {
                        let best = best_candidates.remove(&r.key);
                        MissingItem {
                            key: r.key,
                            band_id: r.band_id,
                            artist_name: r.artist_name,
                            title: r.title,
                            last_known_path: r.local_path,
                            best_candidate: best,
                        }
                    })
                    .collect();

                bridge.emit(WorkerToUiEvent::MissingFound {
                    batch_id,
                    items: missing_items,
                    mass_missing,
                });
            }
        }

        bridge.log(
            LogLevel::Info,
            "library",
            format!(
                "Verification finished: {} present ({} relinked), {} missing, {} unknown",
                present_keys.len() + relinked_count,
                relinked_count,
                missing_count,
                unknown_rels.len()
            ),
            None,
        );

        VerifyOutcome {
            missing_count: missing_count as u32,
            relinked_count: relinked_count as u32,
        }
    }

    /// Reverses all modifications committed in a given batch.
    pub async fn undo_batch(&self, batch_id: u64) {
        let _guard = self.job_lock.lock().await;
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

        let undo_res = pool
            .writer
            .call(move |c| {
                let summary = lib_repo::undo_batch(c, batch_id as i64)?;

                let mut stmt = c.prepare(
                    "SELECT DISTINCT item_type, item_id FROM library_events WHERE batch_id = ?1",
                )?;
                let keys: Vec<ItemKey> = stmt
                    .query_map(rusqlite::params![batch_id as i64], |r| {
                        let k_str: String = r.get(0)?;
                        let id: i64 = r.get(1)?;
                        let kind = crate::types::ItemType::parse(&k_str)
                            .unwrap_or(crate::types::ItemType::Album);
                        Ok(ItemKey::new(kind, id))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();

                let mut updated_rows = Vec::new();
                let mut touched_bands = std::collections::HashSet::new();
                for key in keys {
                    if let Some(row) = releases::get_release(c, key)? {
                        touched_bands.insert(row.band_id);
                        updated_rows.push(row);
                    }
                }

                let mut band_stats = Vec::new();
                for band_id in touched_bands {
                    let stats = artists::stats_with_quota(c, band_id)?;
                    band_stats.push((band_id, stats));
                }

                Ok::<_, DbError>((summary, updated_rows, band_stats))
            })
            .await;

        match undo_res {
            Ok((summary, rows, stats_per_band)) => {
                for row in rows {
                    bridge.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(row)));
                }
                for (band_id, stats) in stats_per_band {
                    bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
                }
                bridge.emit(WorkerToUiEvent::LibraryApplied { batch_id, summary });
                bridge.log(
                    LogLevel::Info,
                    "library",
                    format!("Undid batch {batch_id}"),
                    None,
                );
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to undo batch {batch_id}: {e}"),
                    None,
                );
            }
        }
    }

    /// Checks if a match was queued for this artist prior to its first sync, and executes it.
    /// Otherwise rescans the linked folder on sync and auto-adopts confident matches into a "sync" batch.
    pub async fn on_artist_synced(&self, band_id: i64, cancel: CancellationToken) -> RescanOutcome {
        self.rescan_and_adopt_on_sync(band_id, cancel).await
    }

    /// Rescans the linked folder for an artist during sync, auto-adopting certain matches
    /// into a "sync" batch, and updating review items if any.
    pub async fn rescan_and_adopt_on_sync(
        &self,
        band_id: i64,
        cancel: CancellationToken,
    ) -> RescanOutcome {
        let _guard = self.job_lock.lock().await;

        let queued_opt = {
            let mut guard = self.pending_first_matches.lock().await;
            guard.remove(&band_id)
        };

        if let Some(auto_apply_confident) = queued_opt {
            self.bridge.log(
                LogLevel::Info,
                "library",
                format!("Artist {band_id} finished first sync; triggering queued match"),
                None,
            );
            let adopted = self
                .match_artist_folder_internal(band_id, auto_apply_confident, cancel)
                .await;
            return RescanOutcome {
                adopted,
                review_count: 0,
            };
        }

        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

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
                return RescanOutcome::default();
            }
        };

        if !st.rescan_linked_on_sync {
            return RescanOutcome::default();
        }

        let folder_opt = pool
            .reader
            .call(move |c| lib_repo::get_folder(c, band_id))
            .await;
        let folder = match folder_opt {
            Ok(Some(f)) => f,
            _ => return RescanOutcome::default(),
        };

        let state = probe_folder(&folder.path);
        let updated_af = ArtistFolder {
            band_id,
            path: folder.path.clone(),
            state,
        };
        bridge.emit(WorkerToUiEvent::FolderStatus {
            band_id,
            folder: Some(updated_af),
        });
        if state != FolderState::Ok {
            bridge.log(
                LogLevel::Warn,
                "library",
                format!("Folder is {state:?}: {}", folder.path.display()),
                None,
            );
            return RescanOutcome::default();
        }

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
                return RescanOutcome::default();
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Scan task panicked: {e}"),
                    None,
                );
                return RescanOutcome::default();
            }
        };

        if cancel.is_cancelled() {
            return RescanOutcome::default();
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
                let adoptable: Vec<ReleaseRow> = rels
                    .into_iter()
                    .filter(|r| !r.downloaded || r.local_status == Some(LocalStatus::Archived))
                    .collect();
                Ok::<_, DbError>((adoptable, ign))
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
                return RescanOutcome::default();
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

        let mut adopted_count = 0_u32;
        if !assignment.matched.is_empty() {
            let batch_res = pool
                .writer
                .call(|c| lib_repo::create_batch(c, "sync", &BatchSummary::default()))
                .await;
            if let Ok(b_id) = batch_res {
                let batch_id = b_id.max(0) as u64;
                let decisions: Vec<MatchDecision> = assignment
                    .matched
                    .iter()
                    .map(|m| MatchDecision::Adopt {
                        key: m.release_key,
                        path: m.candidate_path.clone(),
                        format: m.format_guess,
                        folder_mtime: None,
                    })
                    .collect();
                adopted_count = decisions.len() as u32;
                self.apply_reconcile_internal(batch_id, decisions).await;
            }
        }

        let review_count = assignment.review.len() as u32;

        let report = ReconcileReport {
            batch_id: 0,
            band_id,
            matched: assignment.matched,
            review: assignment.review,
            page_only: assignment.page_only,
            local_only: assignment.local_only,
            conflicts: assignment.conflicts,
            restorable: assignment.restorable,
        };
        bridge.emit(WorkerToUiEvent::ReconcileReport {
            report: Box::new(report),
        });

        RescanOutcome {
            adopted: adopted_count,
            review_count,
        }
    }

    async fn match_artist_folder_internal(
        &self,
        band_id: i64,
        auto_apply_confident: bool,
        cancel: CancellationToken,
    ) -> u32 {
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
                return 0;
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to query folder for artist {band_id}: {e}"),
                    None,
                );
                return 0;
            }
        };

        let state = probe_folder(&folder.path);
        let updated_af = ArtistFolder {
            band_id,
            path: folder.path.clone(),
            state,
        };
        bridge.emit(WorkerToUiEvent::FolderStatus {
            band_id,
            folder: Some(updated_af),
        });
        if state != FolderState::Ok {
            bridge.log(
                LogLevel::Warn,
                "library",
                format!("Folder is {state:?}: {}", folder.path.display()),
                None,
            );
            return 0;
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
                return 0;
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
                return 0;
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Scan task panicked: {e}"),
                    None,
                );
                return 0;
            }
        };

        if cancel.is_cancelled() {
            return 0;
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
                let adoptable: Vec<ReleaseRow> = rels
                    .into_iter()
                    .filter(|r| !r.downloaded || r.local_status == Some(LocalStatus::Archived))
                    .collect();
                Ok::<_, DbError>((adoptable, ign))
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
                return 0;
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
                return 0;
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

        let mut adopted_count = 0_u32;
        if auto_apply_confident && !report.matched.is_empty() {
            let decisions: Vec<MatchDecision> = report
                .matched
                .iter()
                .map(|m| MatchDecision::Adopt {
                    key: m.release_key,
                    path: m.candidate_path.clone(),
                    format: m.format_guess,
                    folder_mtime: None,
                })
                .collect();
            adopted_count = decisions.len() as u32;
            self.apply_reconcile_internal(report.batch_id, decisions)
                .await;
        }

        adopted_count
    }

    async fn apply_reconcile_internal(&self, batch_id: u64, decisions: Vec<MatchDecision>) {
        if decisions.is_empty() {
            return;
        }

        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

        let res = pool
            .writer
            .call(move |c| {
                let mut summary = BatchSummary::default();
                let mut touched_keys = Vec::new();

                for decision in decisions {
                    match decision {
                        MatchDecision::Adopt {
                            key,
                            path,
                            format,
                            folder_mtime,
                        }
                        | MatchDecision::Attach {
                            key,
                            path,
                            format,
                            folder_mtime,
                        } => {
                            let mtime = folder_mtime.or_else(|| {
                                std::fs::metadata(&path)
                                    .and_then(|m| m.modified())
                                    .ok()
                                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                    .map(|d| d.as_secs() as i64)
                            });
                            lib_repo::adopt(
                                c,
                                key,
                                &path,
                                format,
                                crate::types::Origin::Imported,
                                mtime,
                                batch_id as i64,
                            )?;
                            summary.adopted += 1;
                            touched_keys.push(key);
                        }
                        MatchDecision::Restore { key, path } => {
                            lib_repo::restore(c, key, &path, batch_id as i64)?;
                            summary.restored += 1;
                            touched_keys.push(key);
                        }
                        MatchDecision::Ignore { band_id, path } => {
                            let path_key =
                                crate::library::scan::normalize_candidate_path_key(&path);
                            lib_repo::ignore_local(c, band_id, &path_key)?;
                            summary.ignored += 1;
                        }
                    }
                }

                lib_repo::update_batch_summary(c, batch_id as i64, &summary)?;

                let mut updated_rows = Vec::new();
                let mut touched_bands = std::collections::HashSet::new();
                for key in touched_keys {
                    if let Some(row) = releases::get_release(c, key)? {
                        touched_bands.insert(row.band_id);
                        updated_rows.push(row);
                    }
                }

                let mut band_stats = Vec::new();
                for band_id in touched_bands {
                    let stats = artists::stats_with_quota(c, band_id)?;
                    band_stats.push((band_id, stats));
                }

                Ok::<_, DbError>((summary, updated_rows, band_stats))
            })
            .await;

        match res {
            Ok((summary, rows, stats_per_band)) => {
                for row in rows {
                    bridge.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(row)));
                }
                for (band_id, stats) in stats_per_band {
                    bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
                }
                bridge.emit(WorkerToUiEvent::LibraryApplied { batch_id, summary });
                bridge.log(
                    LogLevel::Info,
                    "library",
                    format!(
                        "Applied batch {batch_id}: {} adopted, {} restored, {} ignored",
                        summary.adopted, summary.restored, summary.ignored
                    ),
                    None,
                );
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to apply batch {batch_id}: {e}"),
                    None,
                );
            }
        }
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
    use crate::types::{ItemKey, ItemType, Settings};

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
                    folder: Some(f),
                } if *b == band_id && f.state == FolderState::Ok
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

        let has_folder_unlinked = unlink_events.iter().any(|e| {
            matches!(
                e,
                WorkerToUiEvent::FolderStatus {
                    band_id: b,
                    folder: None,
                } if *b == band_id
            )
        });
        assert!(has_folder_unlinked);

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

    #[tokio::test]
    async fn test_library_service_apply_reconcile_and_undo() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("apply_undo_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 555;
        let release_key = ItemKey::new(ItemType::Album, 10);
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-555",
                        name: "Artist 555",
                        url: "https://artist555.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let disc = DiscoveredRelease {
                    key: release_key,
                    band_id,
                    title: "Album 10",
                    artist_name: "Artist 555",
                    url: "https://artist555.bandcamp.com/album/album-10",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let artist_dir = dir.path().join("Artist 555");
        let album_dir = artist_dir.join("Album 10");
        fs::create_dir_all(&album_dir).unwrap();
        fs::write(album_dir.join("01.flac"), b"audio").unwrap();

        let cancel = CancellationToken::new();
        svc.link_artist_folder(band_id, artist_dir.clone(), false, cancel)
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        let report = events
            .into_iter()
            .find_map(|e| match e {
                WorkerToUiEvent::ReconcileReport { report } => Some(report),
                _ => None,
            })
            .expect("should have emitted ReconcileReport");

        assert_eq!(report.matched.len(), 1);
        let batch_id = report.batch_id;

        // Apply reconcile
        let decisions = vec![MatchDecision::Adopt {
            key: release_key,
            path: album_dir.clone(),
            format: Some(crate::types::AudioFormat::Flac),
            folder_mtime: None,
        }];
        svc.apply_reconcile(batch_id, decisions).await;

        // Verify DB updated
        let row_after_apply = pool
            .reader
            .call(move |c| releases::get_release(c, release_key))
            .await
            .unwrap()
            .expect("row should exist");
        assert!(row_after_apply.downloaded);
        assert_eq!(
            row_after_apply.local_status,
            Some(crate::types::LocalStatus::Present)
        );
        assert_eq!(row_after_apply.origin, Some(crate::types::Origin::Imported));
        assert_eq!(row_after_apply.local_path, Some(album_dir));

        let mut apply_events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            apply_events.push(ev);
        }

        let has_applied_event = apply_events.iter().any(|e| {
            matches!(
                e,
                WorkerToUiEvent::LibraryApplied {
                    batch_id: b,
                    summary
                } if *b == batch_id && summary.adopted == 1
            )
        });
        assert!(has_applied_event);

        // Undo batch
        svc.undo_batch(batch_id).await;

        // Verify DB reverted
        let row_after_undo = pool
            .reader
            .call(move |c| releases::get_release(c, release_key))
            .await
            .unwrap()
            .expect("row should exist");
        assert!(!row_after_undo.downloaded);
        assert_eq!(row_after_undo.local_status, None);
        assert_eq!(row_after_undo.origin, None);
        assert_eq!(row_after_undo.local_path, None);

        let mut undo_events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            undo_events.push(ev);
        }
        let has_undone_event = undo_events.iter().any(|e| {
            matches!(
                e,
                WorkerToUiEvent::LibraryApplied {
                    batch_id: b,
                    ..
                } if *b == batch_id
            )
        });
        assert!(has_undone_event);
    }

    #[tokio::test]
    async fn test_library_service_auto_apply_confident() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("auto_apply_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 777;
        let release_key = ItemKey::new(ItemType::Album, 20);
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-777",
                        name: "Artist 777",
                        url: "https://artist777.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let disc = DiscoveredRelease {
                    key: release_key,
                    band_id,
                    title: "Album 20",
                    artist_name: "Artist 777",
                    url: "https://artist777.bandcamp.com/album/album-20",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let artist_dir = dir.path().join("Artist 777");
        let album_dir = artist_dir.join("Album 20");
        fs::create_dir_all(&album_dir).unwrap();
        fs::write(album_dir.join("01.mp3"), b"audio").unwrap();

        let cancel = CancellationToken::new();
        // Link with auto_apply_confident = true
        svc.link_artist_folder(band_id, artist_dir, true, cancel)
            .await;

        let row = pool
            .reader
            .call(move |c| releases::get_release(c, release_key))
            .await
            .unwrap()
            .expect("row should exist");

        // Should be automatically adopted
        assert!(row.downloaded);
        assert_eq!(row.local_status, Some(crate::types::LocalStatus::Present));
        assert_eq!(row.origin, Some(crate::types::Origin::Imported));
        assert!(rx.try_recv().is_ok());
    }

    #[tokio::test]
    async fn test_library_service_queue_first_match_until_synced() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("queue_first_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 888;
        // Insert artist with NO releases yet
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "unsynced-artist",
                        name: "Unsynced Artist",
                        url: "https://unsynced.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let artist_dir = dir.path().join("Unsynced Artist");
        let album_dir = artist_dir.join("Debut Album");
        fs::create_dir_all(&album_dir).unwrap();
        fs::write(album_dir.join("01.mp3"), b"audio").unwrap();

        let cancel = CancellationToken::new();

        // 1. Link folder before sync
        svc.link_artist_folder(band_id, artist_dir.clone(), false, cancel.clone())
            .await;

        // Verify folder is linked in DB
        let folder_opt = pool
            .reader
            .call(move |c| lib_repo::get_folder(c, band_id))
            .await
            .unwrap();
        assert!(folder_opt.is_some());

        // Check emitted events: should have FolderStatus Ok, but NO ReconcileReport
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let has_folder_ok = events.iter().any(|e| {
            matches!(
                e,
                WorkerToUiEvent::FolderStatus {
                    band_id: b,
                    folder: Some(f),
                } if *b == band_id && f.state == FolderState::Ok
            )
        });
        assert!(has_folder_ok);

        let has_report_premature = events
            .iter()
            .any(|e| matches!(e, WorkerToUiEvent::ReconcileReport { .. }));
        assert!(!has_report_premature);

        // 2. Now simulate artist sync completing (releases added)
        let release_key = ItemKey::new(ItemType::Album, 30);
        pool.writer
            .call(move |c| {
                let disc = DiscoveredRelease {
                    key: release_key,
                    band_id,
                    title: "Debut Album",
                    artist_name: "Unsynced Artist",
                    url: "https://unsynced.bandcamp.com/album/debut-album",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        // Notify that artist sync finished
        svc.on_artist_synced(band_id, cancel).await;

        let mut sync_events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            sync_events.push(ev);
        }

        // ReconcileReport should now be emitted!
        let report = sync_events
            .into_iter()
            .find_map(|e| match e {
                WorkerToUiEvent::ReconcileReport { report } => Some(report),
                _ => None,
            })
            .expect("should have emitted queued ReconcileReport after sync");

        assert_eq!(report.band_id, band_id);
        assert_eq!(report.matched.len(), 1);
        assert_eq!(report.matched[0].release_key, release_key);
    }

    #[tokio::test]
    async fn test_library_service_verify_present_and_missing() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("verify_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 111;
        let key_present = ItemKey::new(ItemType::Album, 101);
        let key_missing = ItemKey::new(ItemType::Album, 102);

        let artist_dir = dir.path().join("Artist 111");
        let album_present_dir = artist_dir.join("Album Present");
        fs::create_dir_all(&album_present_dir).unwrap();
        fs::write(album_present_dir.join("01.mp3"), b"audio").unwrap();

        let album_missing_dir = artist_dir.join("Album Missing");
        // missing dir is deleted / not created on disk

        let p1 = album_present_dir.clone();
        let p2 = album_missing_dir.clone();
        let out_dir = dir.path().join("Downloads");

        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-111",
                        name: "Artist 111",
                        url: "https://artist111.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let disc1 = DiscoveredRelease {
                    key: key_present,
                    band_id,
                    title: "Album Present",
                    artist_name: "Artist 111",
                    url: "https://artist111.bandcamp.com/album/present",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc1)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key_present,
                    &p1,
                    Some(crate::types::AudioFormat::Mp3_320),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;

                let disc2 = DiscoveredRelease {
                    key: key_missing,
                    band_id,
                    title: "Album Missing",
                    artist_name: "Artist 111",
                    url: "https://artist111.bandcamp.com/album/missing",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc2)?;
                lib_repo::adopt(
                    c,
                    key_missing,
                    &p2,
                    Some(crate::types::AudioFormat::Mp3_320),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;

                lib_repo::link_folder(c, band_id, &artist_dir, &out_dir)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(SyncScope::Selected(vec![band_id]), false, cancel)
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Verify present album was updated to present
        let row_p = pool
            .reader
            .call(move |c| releases::get_release(c, key_present))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row_p.local_status, Some(LocalStatus::Present));

        // Verify MissingFound was emitted for key_missing
        let missing_found = events
            .into_iter()
            .find_map(|e| match e {
                WorkerToUiEvent::MissingFound {
                    items,
                    mass_missing,
                    ..
                } => Some((items, mass_missing)),
                _ => None,
            })
            .expect("should emit MissingFound");

        assert_eq!(missing_found.0.len(), 1);
        assert_eq!(missing_found.0[0].key, key_missing);
        assert!(!missing_found.1); // mass_missing is false
    }

    #[tokio::test]
    async fn test_library_service_verify_unavailable_folder_guard() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("unavailable_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 222;
        let key = ItemKey::new(ItemType::Album, 201);
        let artist_dir = dir.path().join("Unmounted Drive Artist");
        // artist_dir does not exist

        let p = artist_dir.join("Album");
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-222",
                        name: "Artist 222",
                        url: "https://artist222.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Album",
                    artist_name: "Artist 222",
                    url: "https://artist222.bandcamp.com/album/album",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;

                // Insert directly into artist_folders with nonexistent path
                c.execute(
                    "INSERT INTO artist_folders (band_id, path, path_key, added_at) VALUES (?1, ?2, ?3, unixepoch())",
                    rusqlite::params![band_id, artist_dir.to_string_lossy(), "artist_222"],
                )?;

                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key,
                    &p,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(SyncScope::Selected(vec![band_id]), false, cancel)
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Because folder is unavailable, outcome is Unknown, NEVER Missing
        let has_missing_event = events
            .iter()
            .any(|e| matches!(e, WorkerToUiEvent::MissingFound { .. }));
        assert!(!has_missing_event);

        // Release status should NOT be changed to missing
        let row = pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(row.local_status, Some(LocalStatus::Missing));
    }

    #[tokio::test]
    async fn test_library_service_verify_mass_missing_guard() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("mass_missing_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 333;
        let artist_dir = dir.path().join("Artist 333");
        fs::create_dir_all(&artist_dir).unwrap();

        let a_dir = artist_dir.clone();
        let out_dir = dir.path().join("Downloads");
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-333",
                        name: "Artist 333",
                        url: "https://artist333.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::link_folder(c, band_id, &a_dir, &out_dir)?;

                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;

                // Create 25 downloaded releases whose local folders are deleted
                for i in 1..=25 {
                    let key = ItemKey::new(ItemType::Album, i);
                    let disc = DiscoveredRelease {
                        key,
                        band_id,
                        title: &format!("Album {i}"),
                        artist_name: "Artist 333",
                        url: &format!("https://artist333.bandcamp.com/album/{i}"),
                        cover_url: None,
                        art_id: None,
                    };
                    releases::upsert_discovered(c, &disc)?;
                    let path = a_dir.join(format!("Album {i}"));
                    lib_repo::adopt(
                        c,
                        key,
                        &path,
                        Some(crate::types::AudioFormat::Flac),
                        crate::types::Origin::App,
                        None,
                        b_id,
                    )?;
                }
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(SyncScope::Selected(vec![band_id]), false, cancel)
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        let missing_found = events
            .into_iter()
            .find_map(|e| match e {
                WorkerToUiEvent::MissingFound {
                    items,
                    mass_missing,
                    ..
                } => Some((items, mass_missing)),
                _ => None,
            })
            .expect("should emit MissingFound");

        assert_eq!(missing_found.0.len(), 25);
        assert!(missing_found.1); // mass_missing MUST be true!
    }

    #[tokio::test]
    async fn test_library_service_verify_relinks_moved_album() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("relink_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 444;
        let key = ItemKey::new(ItemType::Album, 401);

        let artist_dir = dir.path().join("Artist 444");
        let original_path = artist_dir.join("Old Album Folder");
        let moved_path = artist_dir.join("Moved Album Folder");
        fs::create_dir_all(&moved_path).unwrap();
        fs::write(moved_path.join("01.mp3"), b"audio").unwrap();

        let out_dir = dir.path().join("Downloads");
        let a_dir = artist_dir.clone();
        let p_old = original_path.clone();

        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-444",
                        name: "Artist 444",
                        url: "https://artist444.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Moved Album Folder",
                    artist_name: "Artist 444",
                    url: "https://artist444.bandcamp.com/album/moved",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key,
                    &p_old,
                    Some(crate::types::AudioFormat::Mp3_320),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;

                lib_repo::link_folder(c, band_id, &a_dir, &out_dir)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(SyncScope::Selected(vec![band_id]), false, cancel)
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Release should be updated to present with new path
        let row = pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.local_status, Some(LocalStatus::Present));
        assert_eq!(row.local_path, Some(moved_path));

        // MissingFound should NOT be emitted because it was successfully relinked
        let has_missing = events
            .iter()
            .any(|e| matches!(e, WorkerToUiEvent::MissingFound { .. }));
        assert!(!has_missing);
    }

    #[tokio::test]
    async fn test_library_service_verify_relink_two_identical_candidates_asks_instead() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("relink_two_identical.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 555;
        let key = ItemKey::new(ItemType::Album, 501);

        let artist_dir = dir.path().join("Artist 555");
        let original_path = artist_dir.join("Album");
        // Two identical candidate folders
        let cand1 = artist_dir.join("Album Copy 1");
        let cand2 = artist_dir.join("Album Copy 2");
        fs::create_dir_all(&cand1).unwrap();
        fs::write(cand1.join("01.mp3"), b"audio").unwrap();
        fs::create_dir_all(&cand2).unwrap();
        fs::write(cand2.join("01.mp3"), b"audio").unwrap();

        let out_dir = dir.path().join("Downloads");
        let a_dir = artist_dir.clone();
        let p_old = original_path.clone();

        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-555",
                        name: "Artist 555",
                        url: "https://artist555.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Album",
                    artist_name: "Artist 555",
                    url: "https://artist555.bandcamp.com/album/album",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key,
                    &p_old,
                    Some(crate::types::AudioFormat::Mp3_320),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;

                lib_repo::link_folder(c, band_id, &a_dir, &out_dir)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(SyncScope::Selected(vec![band_id]), false, cancel)
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Two identical candidates -> asks instead -> MissingFound emitted
        let missing_found = events
            .into_iter()
            .find_map(|e| match e {
                WorkerToUiEvent::MissingFound { items, .. } => Some(items),
                _ => None,
            })
            .expect("should emit MissingFound for ambiguous match");

        assert_eq!(missing_found.len(), 1);
        assert_eq!(missing_found[0].key, key);
        assert!(missing_found[0].best_candidate.is_some());
    }

    #[tokio::test]
    async fn test_library_service_verify_relink_unlinked_artist_skips() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("unlinked_skips.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 666;
        let key = ItemKey::new(ItemType::Album, 601);

        let out_dir = dir.path().join("downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let deleted_path = dir.path().join("Unlinked Artist").join("Deleted Album");

        let custom_out_dir = out_dir.clone();
        pool.writer
            .call(move |c| {
                let s = Settings {
                    out_dir: custom_out_dir,
                    ..Settings::default()
                };
                settings::set_settings(c, &s)?;

                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-666",
                        name: "Artist 666",
                        url: "https://artist666.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Deleted Album",
                    artist_name: "Artist 666",
                    url: "https://artist666.bandcamp.com/album/deleted",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key,
                    &deleted_path,
                    Some(crate::types::AudioFormat::Mp3_320),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                // Artist has NO linked folder!
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(SyncScope::Selected(vec![band_id]), false, cancel)
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Unlinked artist skips relink -> emits MissingFound with best_candidate: None
        let missing_found = events
            .into_iter()
            .find_map(|e| match e {
                WorkerToUiEvent::MissingFound { items, .. } => Some(items),
                _ => None,
            })
            .expect("should emit MissingFound for unlinked artist");

        assert_eq!(missing_found.len(), 1);
        assert_eq!(missing_found[0].key, key);
        assert_eq!(missing_found[0].best_candidate, None);
    }

    #[tokio::test]
    async fn test_library_service_verify_auto_policy_keep_marked() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("auto_keep_marked.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 701;
        let key = ItemKey::new(ItemType::Album, 701);

        let out_dir = dir.path().join("downloads");
        fs::create_dir_all(&out_dir).unwrap();
        let deleted_path = dir.path().join("deleted_album");
        let p_del = deleted_path.clone();

        let custom_out_dir = out_dir.clone();
        pool.writer
            .call(move |c| {
                let s = Settings {
                    out_dir: custom_out_dir,
                    on_missing: OnMissing::Auto,
                    deleted_policy: DeletedPolicy::KeepMarked,
                    ..Settings::default()
                };
                settings::set_settings(c, &s)?;

                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-701",
                        name: "Artist 701",
                        url: "https://artist701.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Deleted Album",
                    artist_name: "Artist 701",
                    url: "https://artist701.bandcamp.com/album/deleted",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key,
                    &p_del,
                    Some(crate::types::AudioFormat::Mp3_320),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(SyncScope::Selected(vec![band_id]), false, cancel)
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Release should be archived with downloaded = 1
        let row = pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .unwrap()
            .unwrap();
        assert!(row.downloaded);
        assert_eq!(row.local_status, Some(LocalStatus::Archived));
        assert_eq!(row.local_path, Some(deleted_path));

        // MissingFound should NOT be emitted
        let has_missing = events
            .iter()
            .any(|e| matches!(e, WorkerToUiEvent::MissingFound { .. }));
        assert!(!has_missing);
    }

    #[tokio::test]
    async fn test_library_service_verify_auto_policy_mark_not_downloaded() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("auto_unmark.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 702;
        let key = ItemKey::new(ItemType::Album, 702);

        let out_dir = dir.path().join("downloads");
        fs::create_dir_all(&out_dir).unwrap();
        let deleted_path = dir.path().join("deleted_album");

        let custom_out_dir = out_dir.clone();
        pool.writer
            .call(move |c| {
                let s = Settings {
                    out_dir: custom_out_dir,
                    on_missing: OnMissing::Auto,
                    deleted_policy: DeletedPolicy::MarkNotDownloaded,
                    ..Settings::default()
                };
                settings::set_settings(c, &s)?;

                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-702",
                        name: "Artist 702",
                        url: "https://artist702.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Deleted Album",
                    artist_name: "Artist 702",
                    url: "https://artist702.bandcamp.com/album/deleted",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key,
                    &deleted_path,
                    Some(crate::types::AudioFormat::Mp3_320),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(SyncScope::Selected(vec![band_id]), false, cancel)
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Release should have downloaded = 0 and local fields cleared
        let row = pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .unwrap()
            .unwrap();
        assert!(!row.downloaded);
        assert_eq!(row.local_status, None);
        assert_eq!(row.local_path, None);

        // MissingFound should NOT be emitted
        let has_missing = events
            .iter()
            .any(|e| matches!(e, WorkerToUiEvent::MissingFound { .. }));
        assert!(!has_missing);
    }

    #[tokio::test]
    async fn test_library_service_resolve_missing_decisions() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("resolve_decisions.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 703;
        let k_moved = ItemKey::new(ItemType::Album, 710);
        let k_arch = ItemKey::new(ItemType::Album, 711);
        let k_unmk = ItemKey::new(ItemType::Album, 712);
        let k_skip = ItemKey::new(ItemType::Album, 713);

        let moved_path = dir.path().join("New Folder");
        fs::create_dir_all(&moved_path).unwrap();
        fs::write(moved_path.join("01.mp3"), b"audio").unwrap();

        let dummy_path = dir.path().join("Dummy");
        let p_dum = dummy_path.clone();
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-703",
                        name: "Artist 703",
                        url: "https://artist703.bandcamp.com",
                        cover_url: None,
                    },
                )?;

                for (k, title) in [
                    (k_moved, "Moved Album"),
                    (k_arch, "Archived Album"),
                    (k_unmk, "Unmarked Album"),
                    (k_skip, "Skipped Album"),
                ] {
                    let disc = DiscoveredRelease {
                        key: k,
                        band_id,
                        title,
                        artist_name: "Artist 703",
                        url: &format!("https://artist703.bandcamp.com/album/{title}"),
                        cover_url: None,
                        art_id: None,
                    };
                    releases::upsert_discovered(c, &disc)?;
                    let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                    lib_repo::adopt(
                        c,
                        k,
                        &p_dum,
                        Some(crate::types::AudioFormat::Mp3_320),
                        crate::types::Origin::App,
                        None,
                        b_id,
                    )?;
                }
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let batch_id = pool
            .writer
            .call(|c| lib_repo::create_batch(c, "verify", &BatchSummary::default()))
            .await
            .unwrap() as u64;

        let decisions = vec![
            MissingDecision {
                key: k_moved,
                action: MissingAction::Moved(moved_path.clone()),
            },
            MissingDecision {
                key: k_arch,
                action: MissingAction::Deleted(DeletedPolicy::KeepMarked),
            },
            MissingDecision {
                key: k_unmk,
                action: MissingAction::Deleted(DeletedPolicy::MarkNotDownloaded),
            },
            MissingDecision {
                key: k_skip,
                action: MissingAction::Skip,
            },
        ];

        let remember = Some(RememberChoice {
            set_default_deleted_policy: Some(DeletedPolicy::MarkNotDownloaded),
            stop_asking: true,
        });

        svc.resolve_missing(batch_id, decisions, remember).await;

        // Check k_moved
        let row_m = pool
            .reader
            .call(move |c| releases::get_release(c, k_moved))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row_m.local_status, Some(LocalStatus::Present));
        assert_eq!(row_m.local_path, Some(moved_path));

        // Check k_arch
        let row_a = pool
            .reader
            .call(move |c| releases::get_release(c, k_arch))
            .await
            .unwrap()
            .unwrap();
        assert!(row_a.downloaded);
        assert_eq!(row_a.local_status, Some(LocalStatus::Archived));

        // Check k_unmk
        let row_u = pool
            .reader
            .call(move |c| releases::get_release(c, k_unmk))
            .await
            .unwrap()
            .unwrap();
        assert!(!row_u.downloaded);
        assert_eq!(row_u.local_status, None);

        // Check k_skip
        let row_s = pool
            .reader
            .call(move |c| releases::get_release(c, k_skip))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row_s.local_status, Some(LocalStatus::Missing));
        assert!(row_s.missing_ack_at.is_some());

        // Check settings updated
        let st = pool
            .reader
            .call(|c| settings::get_settings(c, PathBuf::from("downloads")))
            .await
            .unwrap();
        assert_eq!(st.deleted_policy, DeletedPolicy::MarkNotDownloaded);
        assert_eq!(st.on_missing, OnMissing::Auto);

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let applied = events
            .iter()
            .any(|e| matches!(e, WorkerToUiEvent::LibraryApplied { .. }));
        assert!(applied);
    }

    #[tokio::test]
    async fn test_library_service_repoint_artist_folder() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("repoint_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 704;
        let key = ItemKey::new(ItemType::Album, 720);

        let old_artist_dir = dir.path().join("Old Artist Folder");
        let new_artist_dir = dir.path().join("New Artist Folder");
        let album_in_new = new_artist_dir.join("Album");
        fs::create_dir_all(&album_in_new).unwrap();
        fs::write(album_in_new.join("01.mp3"), b"audio").unwrap();

        let out_dir = dir.path().join("downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let dummy_old_path = old_artist_dir.join("Album");
        let a_new = new_artist_dir.clone();
        let custom_out_dir = out_dir.clone();

        pool.writer
            .call(move |c| {
                let s = Settings {
                    out_dir: custom_out_dir,
                    ..Settings::default()
                };
                settings::set_settings(c, &s)?;

                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-704",
                        name: "Artist 704",
                        url: "https://artist704.bandcamp.com",
                        cover_url: None,
                    },
                )?;

                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Album",
                    artist_name: "Artist 704",
                    url: "https://artist704.bandcamp.com/album/album",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key,
                    &dummy_old_path,
                    Some(crate::types::AudioFormat::Mp3_320),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.repoint_artist_folder(band_id, a_new, cancel).await;

        // Folder link should be updated to new_artist_dir
        let folder = pool
            .reader
            .call(move |c| lib_repo::get_folder(c, band_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(folder.path, new_artist_dir.canonicalize().unwrap());

        // Album should be relinked to album_in_new
        let row = pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.local_status, Some(LocalStatus::Present));
        assert_eq!(row.local_path, Some(album_in_new));

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let updated = events
            .iter()
            .any(|e| matches!(e, WorkerToUiEvent::ReleaseUpdated(..)));
        assert!(updated);
    }

    #[tokio::test]
    async fn test_sync_rescan_auto_adopts_certain_match_under_sync_batch() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("rescan_adopt_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 801;
        let key = ItemKey::new(ItemType::Album, 810);

        let artist_dir = dir.path().join("Artist 801");
        let album_dir = artist_dir.join("Album One");
        fs::create_dir_all(&album_dir).unwrap();
        fs::write(album_dir.join("01.mp3"), b"audio").unwrap();

        let out_dir = dir.path().join("downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let a_dir = artist_dir.clone();
        let o_dir = out_dir.clone();
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-801",
                        name: "Artist 801",
                        url: "https://artist801.bandcamp.com",
                        cover_url: None,
                    },
                )?;

                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Album One",
                    artist_name: "Artist 801",
                    url: "https://artist801.bandcamp.com/album/album-one",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;

                lib_repo::link_folder(c, band_id, &a_dir, &o_dir)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        let outcome = svc.rescan_and_adopt_on_sync(band_id, cancel).await;

        assert_eq!(outcome.adopted, 1);
        assert_eq!(outcome.review_count, 0);

        // Check release updated in DB
        let rel = pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .unwrap()
            .unwrap();
        assert!(rel.downloaded);
        assert_eq!(rel.local_status, Some(LocalStatus::Present));
        assert_eq!(rel.origin, Some(crate::types::Origin::Imported));
        assert_eq!(rel.local_path, Some(album_dir));

        // Check library_batches has kind = "sync"
        let batches = pool
            .reader
            .call(|c| {
                let mut stmt = c.prepare("SELECT kind FROM library_batches")?;
                let kinds: Vec<String> = stmt
                    .query_map([], |r| r.get(0))?
                    .filter_map(Result::ok)
                    .collect();
                Ok::<_, DbError>(kinds)
            })
            .await
            .unwrap();
        assert!(batches.contains(&"sync".to_string()));

        // Check events emitted
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let applied = events.iter().any(|e| {
            matches!(
                e,
                WorkerToUiEvent::LibraryApplied { summary, .. } if summary.adopted == 1
            )
        });
        assert!(applied);
        let updated = events.iter().any(|e| {
            matches!(
                e,
                WorkerToUiEvent::ReleaseUpdated(r) if r.key == key
            )
        });
        assert!(updated);
    }

    #[tokio::test]
    async fn test_sync_rescan_uncertain_match_raises_review_count_without_adopting() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("rescan_uncertain_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 802;
        let key = ItemKey::new(ItemType::Album, 820);

        let artist_dir = dir.path().join("Artist 802");
        let album_dir = artist_dir.join("Album Two (Deluxe Edition)");
        fs::create_dir_all(&album_dir).unwrap();
        fs::write(album_dir.join("01.mp3"), b"audio").unwrap();

        let out_dir = dir.path().join("downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let a_dir = artist_dir.clone();
        let o_dir = out_dir.clone();
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-802",
                        name: "Artist 802",
                        url: "https://artist802.bandcamp.com",
                        cover_url: None,
                    },
                )?;

                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Album Two",
                    artist_name: "Artist 802",
                    url: "https://artist802.bandcamp.com/album/album-two",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;

                lib_repo::link_folder(c, band_id, &a_dir, &o_dir)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        let outcome = svc.rescan_and_adopt_on_sync(band_id, cancel).await;

        assert_eq!(outcome.adopted, 0);
        assert_eq!(outcome.review_count, 1);

        // Check release NOT downloaded
        let rel = pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .unwrap()
            .unwrap();
        assert!(!rel.downloaded);
        assert_eq!(rel.local_status, None);

        // Check no "sync" batch created
        let batches = pool
            .reader
            .call(|c| {
                let mut stmt =
                    c.prepare("SELECT count(*) FROM library_batches WHERE kind = 'sync'")?;
                let count: i64 = stmt.query_row([], |r| r.get(0))?;
                Ok::<_, DbError>(count)
            })
            .await
            .unwrap();
        assert_eq!(batches, 0);

        // Check ReconcileReport emitted with review items
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let report = events
            .into_iter()
            .find_map(|e| match e {
                WorkerToUiEvent::ReconcileReport { report } => Some(report),
                _ => None,
            })
            .expect("should emit ReconcileReport with review items");
        assert_eq!(report.review.len(), 1);
        assert_eq!(report.matched.len(), 0);
    }

    #[tokio::test]
    async fn test_sync_rescan_disabled_by_setting_adopts_nothing() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("rescan_disabled_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, _rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 803;
        let key = ItemKey::new(ItemType::Album, 830);

        let artist_dir = dir.path().join("Artist 803");
        let album_dir = artist_dir.join("Album Three");
        fs::create_dir_all(&album_dir).unwrap();
        fs::write(album_dir.join("01.mp3"), b"audio").unwrap();

        let out_dir = dir.path().join("downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let a_dir = artist_dir.clone();
        let o_dir = out_dir.clone();
        pool.writer
            .call(move |c| {
                let s = Settings {
                    rescan_linked_on_sync: false,
                    ..Settings::default()
                };
                settings::set_settings(c, &s)?;

                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-803",
                        name: "Artist 803",
                        url: "https://artist803.bandcamp.com",
                        cover_url: None,
                    },
                )?;

                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Album Three",
                    artist_name: "Artist 803",
                    url: "https://artist803.bandcamp.com/album/album-three",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;

                lib_repo::link_folder(c, band_id, &a_dir, &o_dir)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        let outcome = svc.rescan_and_adopt_on_sync(band_id, cancel).await;

        assert_eq!(outcome.adopted, 0);
        assert_eq!(outcome.review_count, 0);

        let rel = pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .unwrap()
            .unwrap();
        assert!(!rel.downloaded);
    }

    #[tokio::test]
    async fn test_sync_rescan_unlinked_artist_does_nothing() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("rescan_unlinked_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, _rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let cancel = CancellationToken::new();
        let outcome = svc.rescan_and_adopt_on_sync(999, cancel).await;
        assert_eq!(outcome.adopted, 0);
        assert_eq!(outcome.review_count, 0);
    }
}

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::db::DbPool;
use crate::db::repo::{artists, library as lib_repo, releases, settings};
use crate::error::DbError;
use crate::library::link::probe_folder;
use crate::library::matcher::assign_matches_with_rejected;
use crate::library::scan::scan_linked_folder;
use crate::library::service::{
    ReconcileReport, drop_downloaded_page_only, split_matchable_releases,
};
use crate::library::verify::{
    VerifyResult, attribute_release_root, find_relink_candidates_with_rejected, group_unavailable_releases,
    is_mass_missing, verify_single_release,
};
use crate::types::{
    ArtistFolder, BatchSummary, DeletedPolicy, FolderState, ItemKey, LibraryPhase, LocalStatus,
    LogLevel, MatchDecision, MissingAction, MissingCandidate, MissingDecision, MissingItem,
    OnMissing, ReleaseQuery, ReleaseRow, RememberChoice, RescanOutcome, Settings, SyncScope,
    UnavailableAction, UnavailableDecision, UnavailableFolder, UnavailableRoot, VerifyOutcome,
    VerifyTrigger,
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
    pub acked_unavailable: Arc<Mutex<std::collections::HashSet<UnavailableRoot>>>,
    /// Download directory used while no settings row has been saved yet. It must be the
    /// same default the download side uses, or the `out_dir` overlap checks guard the wrong path.
    default_out_dir: PathBuf,
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
            acked_unavailable: Arc::new(Mutex::new(std::collections::HashSet::new())),
            default_out_dir: PathBuf::from("downloads"),
        }
    }

    /// Sets the download directory assumed before the user has saved any settings.
    #[must_use]
    pub fn with_default_out_dir(mut self, dir: PathBuf) -> Self {
        self.default_out_dir = dir;
        self
    }

    /// Reader closure that loads the settings with the service's default download directory.
    fn settings_reader(
        &self,
    ) -> impl FnOnce(&mut rusqlite::Connection) -> Result<Settings, DbError> + Send + 'static {
        let default_out = self.default_out_dir.clone();
        move |c| settings::get_settings(c, default_out)
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

        let st_res = pool.reader.call(self.settings_reader()).await;
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
            .call(move |c| Ok(lib_repo::add_folder(c, band_id, &link_path, &st.out_dir)?))
            .await;

        let linked_af = match link_res {
            Ok(af) => {
                bridge.log(
                    LogLevel::Info,
                    "library",
                    format!("Added folder '{}' to artist {band_id}", path.display()),
                    None,
                );
                af
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
        };

        let all_folders = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap_or_else(|_| vec![linked_af]);
        let mut probed = Vec::new();
        for mut f in all_folders {
            f.state = probe_folder(&f.path);
            probed.push(f);
        }
        let any_ok = probed.iter().any(|f| f.state == FolderState::Ok);

        bridge.emit(WorkerToUiEvent::FolderStatus {
            band_id,
            folders: probed,
        });

        if let Ok(Some(av)) = pool
            .reader
            .call(move |c| artists::get_view(c, band_id))
            .await
        {
            bridge.emit(WorkerToUiEvent::ArtistUpserted(av));
        }

        if !any_ok {
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
    pub async fn unlink_artist_folder(&self, folder_id: i64) {
        let _guard = self.job_lock.lock().await;
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

        let unlink_res = pool
            .writer
            .call(move |c| {
                let bid = lib_repo::get_folder_by_id(c, folder_id)?.map(|f| f.band_id);
                let unlinked = lib_repo::unlink_folder(c, folder_id)?;
                let remaining = if let Some(b) = bid {
                    lib_repo::get_folders(c, b)?
                } else {
                    Vec::new()
                };
                Ok::<_, DbError>((bid, unlinked, remaining))
            })
            .await;
        match unlink_res {
            Ok((Some(band_id), true, remaining)) => {
                bridge.log(
                    LogLevel::Info,
                    "library",
                    format!("Unlinked folder {folder_id} for artist {band_id}"),
                    None,
                );
                if remaining.is_empty() {
                    self.pending_first_matches.lock().await.remove(&band_id);
                }
                let mut probed = Vec::new();
                for mut f in remaining {
                    f.state = probe_folder(&f.path);
                    probed.push(f);
                }
                bridge.emit(WorkerToUiEvent::FolderStatus {
                    band_id,
                    folders: probed,
                });
                if let Ok(Some(av)) = pool
                    .reader
                    .call(move |c| artists::get_view(c, band_id))
                    .await
                {
                    bridge.emit(WorkerToUiEvent::ArtistUpserted(av));
                }
                if let Ok(stats) = pool
                    .reader
                    .call(move |c| artists::stats_with_quota(c, band_id))
                    .await
                {
                    bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
                }
            }
            Ok((_, false, _)) | Ok((None, _, _)) => {
                bridge.log(
                    LogLevel::Warn,
                    "library",
                    format!("No linked folder found with id {folder_id}"),
                    None,
                );
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to unlink folder {folder_id}: {e}"),
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
            let default_out = self.default_out_dir.clone();
            let rem_res = pool
                .writer
                .call(move |c| {
                    let mut st = settings::get_settings(c, default_out)?;
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

    /// Resolves unavailable folders according to user decisions.
    pub async fn resolve_unavailable(&self, batch_id: u64, decisions: Vec<UnavailableDecision>) {
        let _guard = self.job_lock.lock().await;
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

        for dec in decisions {
            match dec.action {
                UnavailableAction::Keep => {
                    self.acked_unavailable.lock().await.insert(dec.root);
                }
                UnavailableAction::MovedToLinked => {
                    let keys = dec.keys;
                    let pool_clone = pool.clone();
                    let bands_res = pool_clone
                        .reader
                        .call(move |c| {
                            let mut bands = Vec::new();
                            for &k in &keys {
                                if let Some(r) = releases::get_release(c, k)?
                                    && !lib_repo::get_folders(c, r.band_id)?.is_empty()
                                    && !bands.contains(&r.band_id)
                                {
                                    bands.push(r.band_id);
                                }
                            }
                            Ok::<_, DbError>(bands)
                        })
                        .await;
                    if let Ok(bands) = bands_res {
                        for band_id in bands {
                            self.match_artist_folder_internal(
                                band_id,
                                true,
                                CancellationToken::new(),
                            )
                            .await;
                        }
                    }
                }
                UnavailableAction::Deleted { policy, unlink } => {
                    let keys = dec.keys;
                    let b_id = batch_id as i64;
                    let pool_clone = pool.clone();
                    let bridge_clone = bridge.clone();

                    let res = pool_clone
                        .writer
                        .call(move |c| {
                            let mut valid_keys = Vec::new();
                            for &k in &keys {
                                if let Some(r) = releases::get_release(c, k)?
                                    && r.downloaded
                                    && r.local_status != Some(LocalStatus::Archived)
                                {
                                    valid_keys.push(k);
                                }
                            }

                            if !valid_keys.is_empty() {
                                match policy {
                                    DeletedPolicy::KeepMarked => {
                                        lib_repo::archive(c, &valid_keys, b_id)?;
                                    }
                                    DeletedPolicy::MarkNotDownloaded => {
                                        lib_repo::unmark(c, &valid_keys, b_id)?;
                                    }
                                }
                            }

                            let mut touched_bands = std::collections::HashSet::new();
                            let mut updated = Vec::new();
                            for &k in &valid_keys {
                                if let Some(r) = releases::get_release(c, k)? {
                                    touched_bands.insert(r.band_id);
                                    updated.push(r);
                                }
                            }

                            let mut unlinked_band_id = None;
                            if unlink && let UnavailableRoot::Linked { folder_id } = dec.root {
                                let bid =
                                    lib_repo::get_folder_by_id(c, folder_id)?.map(|f| f.band_id);
                                lib_repo::unlink_folder(c, folder_id)?;
                                if let Some(b) = bid {
                                    touched_bands.insert(b);
                                    unlinked_band_id = Some(b);
                                }
                            }

                            let mut stats_list = Vec::new();
                            for &b in &touched_bands {
                                stats_list.push((b, artists::stats_with_quota(c, b)?));
                            }

                            let summary = lib_repo::get_batch_summary(c, b_id)?.unwrap_or_default();
                            Ok::<_, DbError>((updated, stats_list, summary, unlinked_band_id))
                        })
                        .await;

                    if let Ok((updated_releases, stats_list, summary, unlinked_band_id)) = res {
                        if unlink && let UnavailableRoot::Linked { folder_id } = dec.root {
                            self.acked_unavailable
                                .lock()
                                .await
                                .remove(&UnavailableRoot::Linked { folder_id });
                            if let Some(band_id) = unlinked_band_id {
                                let remaining = pool
                                    .reader
                                    .call(move |c| lib_repo::get_folders(c, band_id))
                                    .await
                                    .unwrap_or_default();
                                if remaining.is_empty() {
                                    self.pending_first_matches.lock().await.remove(&band_id);
                                }
                                let mut probed = Vec::new();
                                for mut f in remaining {
                                    f.state = probe_folder(&f.path);
                                    probed.push(f);
                                }
                                bridge_clone.emit(WorkerToUiEvent::FolderStatus {
                                    band_id,
                                    folders: probed,
                                });
                                if let Ok(Some(av)) = pool
                                    .reader
                                    .call(move |c| artists::get_view(c, band_id))
                                    .await
                                {
                                    bridge_clone.emit(WorkerToUiEvent::ArtistUpserted(av));
                                }
                            }
                        }

                        for r in updated_releases {
                            bridge_clone.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(r)));
                        }
                        for (band_id, stats) in stats_list {
                            bridge_clone.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
                        }
                        bridge_clone.emit(WorkerToUiEvent::LibraryApplied { batch_id, summary });
                        bridge_clone.log(
                            LogLevel::Info,
                            "library",
                            format!("Resolved unavailable folder batch {batch_id}"),
                            None,
                        );
                    }
                }
            }
        }
    }

    /// Re-points an artist's linked folder to a new path and rescans/relinks missing releases.
    pub async fn repoint_artist_folder(
        &self,
        folder_id: i64,
        new_path: PathBuf,
        cancel: CancellationToken,
    ) {
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

        let st = match pool.reader.call(self.settings_reader()).await {
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
        let repoint_res = pool
            .writer
            .call(move |c| {
                let bid = lib_repo::get_folder_by_id(c, folder_id)?.map(|f| f.band_id);
                let af = lib_repo::replace_folder(c, folder_id, &link_path, &st.out_dir)?;
                let all_folders = if let Some(b) = bid {
                    lib_repo::get_folders(c, b)?
                } else {
                    vec![af.clone()]
                };
                Ok::<_, DbError>((bid, af, all_folders))
            })
            .await;

        let band_id = match repoint_res {
            Ok((Some(band_id), af, all_folders)) => {
                bridge.log(
                    LogLevel::Info,
                    "library",
                    format!("Repointed folder {folder_id} to '{}'", af.path.display()),
                    None,
                );
                let mut probed = Vec::new();
                for mut f in all_folders {
                    f.state = probe_folder(&f.path);
                    probed.push(f);
                }
                bridge.emit(WorkerToUiEvent::FolderStatus {
                    band_id,
                    folders: probed,
                });
                band_id
            }
            Ok((None, _, _)) => return,
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to repoint folder {folder_id}: {e}"),
                    None,
                );
                return;
            }
        };

        if let Ok(Some(av)) = pool
            .reader
            .call(move |c| artists::get_view(c, band_id))
            .await
        {
            bridge.emit(WorkerToUiEvent::ArtistUpserted(av));
        }

        self.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
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
                let stats = artists::stats_with_quota(c, band_id)?;
                Ok::<_, DbError>(stats)
            })
            .await;
        match res {
            Ok(stats) => {
                bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
            }
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "library",
                    format!("Failed to ignore paths for artist {band_id}: {e}"),
                    None,
                );
            }
        }
    }

    /// Verifies all downloaded releases in the given scope against the filesystem.
    /// Emits progress events and, if any files are missing, a MissingFound event.
    pub async fn verify_files(
        &self,
        scope: SyncScope,
        trigger: VerifyTrigger,
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
        let st = match pool.reader.call(self.settings_reader()).await {
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
        let is_all_scope = matches!(scope, SyncScope::All);
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

        // 3. Probe folder health for each artist in scope and out_dir
        let out_dir = st.out_dir.clone();
        let out_dir_state = probe_folder(&out_dir);
        if out_dir_state == FolderState::Ok {
            self.acked_unavailable
                .lock()
                .await
                .remove(&UnavailableRoot::OutDir);
        } else {
            bridge.log(
                LogLevel::Warn,
                "library",
                format!("Output directory unavailable: {out_dir_state:?}"),
                None,
            );
        }

        let mut linked_folders: std::collections::HashMap<i64, Vec<ArtistFolder>> =
            std::collections::HashMap::new();
        for &band_id in &band_ids {
            let folders = pool
                .reader
                .call(move |c| lib_repo::get_folders(c, band_id))
                .await
                .unwrap_or_default();

            let mut probed_folders = Vec::new();
            for mut folder in folders {
                let state = probe_folder(&folder.path);
                folder.state = state;
                if state == FolderState::Ok {
                    self.acked_unavailable
                        .lock()
                        .await
                        .remove(&UnavailableRoot::Linked {
                            folder_id: folder.id,
                        });
                } else {
                    bridge.log(
                        LogLevel::Warn,
                        "library",
                        format!("Folder unavailable for artist {band_id}: {state:?}"),
                        None,
                    );
                }
                probed_folders.push(folder);
            }

            bridge.emit(WorkerToUiEvent::FolderStatus {
                band_id,
                folders: probed_folders.clone(),
            });
            linked_folders.insert(band_id, probed_folders);
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
        let linked_folders_clone = linked_folders.clone();
        let out_dir_clone = out_dir.clone();
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
                let folders_for_band = linked_folders_clone
                    .get(&rel.band_id)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                let (root, root_path, root_state) =
                    attribute_release_root(&rel, folders_for_band, &out_dir_clone, out_dir_state);
                let is_ok = root_state == FolderState::Ok;
                let outcome = verify_single_release(&rel, is_ok, trigger.is_sync());
                match outcome {
                    Some(VerifyResult::Present) => present.push(rel),
                    Some(VerifyResult::Missing) => missing.push(rel),
                    Some(VerifyResult::Unknown) => {
                        unknown.push((rel, root, root_path, root_state));
                    }
                    None => {}
                }
            }

            (present, missing, unknown)
        })
        .await;

        let (present_rels, missing_rels, mut unknown_rels) = match verify_res {
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

        // 8. Auto-relink moved albums (if enabled, artist has a link, and not mass_missing).
        // Also check unknown_rels (such as releases last seen in a now-deleted out_dir)
        // when the artist has at least one reachable (Ok) linked folder.
        let mut still_missing: Vec<ReleaseRow> = Vec::new();
        let mut best_candidates: std::collections::HashMap<ItemKey, MissingCandidate> =
            std::collections::HashMap::new();
        let mut relinked_count = 0;

        if !mass_missing && st.auto_relink && (!missing_rels.is_empty() || !unknown_rels.is_empty())
        {
            let missing_key_set: std::collections::HashSet<ItemKey> =
                missing_rels.iter().map(|r| r.key).collect();
            let mut by_band: std::collections::HashMap<i64, Vec<ReleaseRow>> =
                std::collections::HashMap::new();
            for r in missing_rels {
                by_band.entry(r.band_id).or_default().push(r);
            }
            for (r, _, _, _) in &unknown_rels {
                if r.local_status != Some(LocalStatus::Archived) {
                    let has_ok_linked = linked_folders
                        .get(&r.band_id)
                        .is_some_and(|fs| fs.iter().any(|f| f.state == FolderState::Ok));
                    if has_ok_linked {
                        by_band.entry(r.band_id).or_default().push(r.clone());
                    }
                }
            }

            let mut relinked_unknown_keys: std::collections::HashSet<ItemKey> =
                std::collections::HashSet::new();

            for (band_id, band_rels) in by_band {
                let folders = linked_folders.get(&band_id).cloned().unwrap_or_default();

                let ok_folders: Vec<ArtistFolder> = folders
                    .into_iter()
                    .filter(|f| f.state == FolderState::Ok)
                    .collect();

                if ok_folders.is_empty() {
                    for r in band_rels {
                        if missing_key_set.contains(&r.key) {
                            still_missing.push(r);
                        }
                    }
                    continue;
                }

                // Rescan all Ok linked folders of the artist
                let mut merged_candidates = Vec::new();
                for folder in ok_folders {
                    let folder_path = folder.path.clone();
                    let cancel_scan = cancel.clone();
                    let scan_res = tokio::task::spawn_blocking(move || {
                        scan_linked_folder(&folder_path, &cancel_scan)
                    })
                    .await;

                    if let Ok(Ok(r)) = scan_res {
                        merged_candidates.extend(r.candidates);
                    }
                }

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
                        let rej = lib_repo::get_rejected_pairs(c, band_id)?;
                        Ok::<_, DbError>((excluded, rej))
                    })
                    .await;

                let (excluded_keys, rejected_pairs) = excluded_res.unwrap_or_default();

                let outcome = find_relink_candidates_with_rejected(
                    &band_rels,
                    &merged_candidates,
                    &excluded_keys,
                    &rejected_pairs,
                    st.auto_match_min,
                    &st.folder_style,
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
                            if !missing_key_set.contains(&r.key) {
                                relinked_unknown_keys.insert(r.key);
                            }
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
                        if let Ok(stats) = pool
                            .reader
                            .call(move |c| artists::stats_with_quota(c, band_id))
                            .await
                        {
                            bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
                        }
                    }
                }

                for (unresolved_rel, best_cand_opt) in outcome.unresolved {
                    if missing_key_set.contains(&unresolved_rel.key) {
                        if let Some(cand) = best_cand_opt {
                            best_candidates.insert(unresolved_rel.key, cand);
                        }
                        still_missing.push(unresolved_rel);
                    }
                }
            }

            if !relinked_unknown_keys.is_empty() {
                unknown_rels.retain(|(r, _, _, _)| !relinked_unknown_keys.contains(&r.key));
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

            let (auto_candidates, ask_candidates): (Vec<ReleaseRow>, Vec<ReleaseRow>) =
                if !mass_missing && st.on_missing == OnMissing::Auto {
                    still_missing.into_iter().partition(|r| {
                        let has_offline_folder = linked_folders
                            .get(&r.band_id)
                            .map(|folders| folders.iter().any(|f| f.state != FolderState::Ok))
                            .unwrap_or(false);
                        !has_offline_folder
                    })
                } else {
                    (Vec::new(), still_missing)
                };

            if !auto_candidates.is_empty() {
                let keys: Vec<ItemKey> = auto_candidates.iter().map(|r| r.key).collect();
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
                            auto_candidates.len(),
                            policy
                        ),
                        None,
                    );
                }
            }

            if !ask_candidates.is_empty() {
                let missing_items: Vec<MissingItem> = ask_candidates
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

        // 10. Handle unknown releases (unavailable roots)
        let unknown_count = unknown_rels.len();
        let mut unavailable_folders = group_unavailable_releases(unknown_rels);

        // Ensure every unavailable linked folder in scope is represented in unavailable_folders,
        // even if it has 0 active unarchived releases (e.g. all releases were archived or empty).
        for folders in linked_folders.values() {
            for folder in folders {
                if folder.state != FolderState::Ok {
                    let already_present = unavailable_folders.iter().any(
                        |f| matches!(f.root, UnavailableRoot::Linked { folder_id: fid } if fid == folder.id),
                    );
                    if !already_present {
                        unavailable_folders.push(UnavailableFolder {
                            root: UnavailableRoot::Linked {
                                folder_id: folder.id,
                            },
                            path: folder.path.clone(),
                            state: folder.state,
                            releases: Vec::new(),
                        });
                    }
                }
            }
        }

        if !unavailable_folders.is_empty() {
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
                        format!("Failed to create verification batch for unavailable folders: {e}"),
                        None,
                    );
                    0
                }
            };

            let prompt = if trigger == VerifyTrigger::Manual {
                let mut acked_lock = self.acked_unavailable.lock().await;
                for f in &unavailable_folders {
                    acked_lock.remove(&f.root);
                }
                true
            } else {
                let acked_lock = self.acked_unavailable.lock().await;
                let all_acked = unavailable_folders
                    .iter()
                    .all(|f| acked_lock.contains(&f.root));
                trigger.prompts() && !all_acked
            };

            bridge.emit(WorkerToUiEvent::FolderUnavailable {
                batch_id,
                folders: unavailable_folders,
                prompt,
            });
        }

        // 11. On manual VerifyFiles, rescan reachable linked folders so unmarked (downloaded = 0),
        // archived (KeepMarked), or out_dir-resident releases whose files are in the linked folder
        // are automatically adopted or restored.
        if trigger == VerifyTrigger::Manual && st.rescan_linked_on_sync {
            for (&band_id, folders) in &linked_folders {
                if cancel.is_cancelled() {
                    break;
                }
                let ok_folders: Vec<ArtistFolder> = folders
                    .iter()
                    .filter(|f| f.state == FolderState::Ok)
                    .cloned()
                    .collect();
                if ok_folders.is_empty() {
                    continue;
                }
                let adopted_or_restored = self
                    .auto_adopt_and_restore_linked_on_verify(
                        band_id,
                        folders,
                        &ok_folders,
                        &st,
                        &cancel,
                    )
                    .await;
                relinked_count += adopted_or_restored as usize;
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
                unknown_count
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

        let st = match pool.reader.call(self.settings_reader()).await {
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

        let folders = match pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
        {
            Ok(f) if !f.is_empty() => f,
            _ => return RescanOutcome::default(),
        };

        let mut ok_folders = Vec::new();
        let mut unavailable_folder_ids = Vec::new();
        let mut probed_folders = Vec::new();
        for f in &folders {
            let state = probe_folder(&f.path);
            let mut updated = f.clone();
            updated.state = state;
            if state == FolderState::Ok {
                ok_folders.push(updated.clone());
            } else {
                unavailable_folder_ids.push(f.id);
                bridge.log(
                    LogLevel::Warn,
                    "library",
                    format!("Folder is {state:?}: {}", f.path.display()),
                    None,
                );
            }
            probed_folders.push(updated);
        }

        bridge.emit(WorkerToUiEvent::FolderStatus {
            band_id,
            folders: probed_folders,
        });

        if ok_folders.is_empty() {
            return RescanOutcome::default();
        }

        let mut all_candidates = Vec::new();
        for (idx, folder) in ok_folders.iter().enumerate() {
            bridge.emit(WorkerToUiEvent::LibraryProgress {
                phase: LibraryPhase::Scanning,
                done: idx as u32,
                total: ok_folders.len() as u32,
                current: Some(folder.path.display().to_string()),
            });

            let scan_path = folder.path.clone();
            let scan_cancel = cancel.clone();
            let scan_res =
                tokio::task::spawn_blocking(move || scan_linked_folder(&scan_path, &scan_cancel))
                    .await;
            match scan_res {
                Ok(Ok(r)) => all_candidates.extend(r.candidates),
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
            }
        }

        if cancel.is_cancelled() {
            return RescanOutcome::default();
        }

        all_candidates.sort_by(|a, b| {
            lib_repo::normalize_path_key(&a.path).cmp(&lib_repo::normalize_path_key(&b.path))
        });

        let query = ReleaseQuery {
            band_id: Some(band_id),
            ..Default::default()
        };
        let ok_roots: Vec<PathBuf> = ok_folders.iter().map(|f| f.path.clone()).collect();
        let all_roots: Vec<PathBuf> = folders.iter().map(|f| f.path.clone()).collect();
        let rel_res = pool
            .reader
            .call(move |c| {
                let rels = releases::query_releases(c, &query)?;
                let catalog = rels.clone();
                let ign = lib_repo::get_ignored_local(c, band_id)?;
                let rej = lib_repo::get_rejected_pairs(c, band_id)?;
                let split = split_matchable_releases(rels, &all_roots, &ok_roots);
                Ok::<_, DbError>((
                    split.adoptable,
                    catalog,
                    ign,
                    rej,
                    split.active_linked_keys,
                    split.linked,
                ))
            })
            .await;

        let (releases, catalog, ignored, rejected, active_linked_keys, linked) = match rel_res {
            Ok(tuple) => tuple,
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

        let unlinked_candidates: Vec<crate::library::AlbumCandidate> = all_candidates
            .into_iter()
            .filter(|c| !active_linked_keys.contains(&lib_repo::normalize_path_key(&c.path)))
            .collect();

        bridge.emit(WorkerToUiEvent::LibraryProgress {
            phase: LibraryPhase::Matching,
            done: 0,
            total: unlinked_candidates.len() as u32,
            current: None,
        });

        let mut assignment = assign_matches_with_rejected(
            &unlinked_candidates,
            &releases,
            st.auto_match_min,
            st.review_match_min,
            &ignored,
            &rejected,
            &st.folder_style,
        );
        drop_downloaded_page_only(&mut assignment, &releases);

        let mut adopted_count = 0_u32;
        let mut linked = linked;
        let mut matched = assignment.matched;
        let mut restorable = assignment.restorable;
        if !matched.is_empty() || !restorable.is_empty() {
            let batch_res = pool
                .writer
                .call(|c| lib_repo::create_batch(c, "sync", &BatchSummary::default()))
                .await;
            if let Ok(b_id) = batch_res {
                let batch_id = b_id.max(0) as u64;
                let mut decisions: Vec<MatchDecision> = matched
                    .iter()
                    .map(|m| MatchDecision::Adopt {
                        key: m.release_key,
                        path: m.candidate_path.clone(),
                        format: m.format_guess,
                        folder_mtime: None,
                    })
                    .collect();
                for m in &restorable {
                    decisions.push(MatchDecision::Restore {
                        key: m.release_key,
                        path: m.candidate_path.clone(),
                    });
                }
                adopted_count = decisions.len() as u32;
                self.apply_reconcile_internal(batch_id, decisions).await;

                for m in matched.drain(..) {
                    linked.push(crate::library::service::LinkedRelease {
                        release_key: m.release_key,
                        release_title: m.release_title,
                        path: m.candidate_path,
                    });
                }
                for m in restorable.drain(..) {
                    linked.push(crate::library::service::LinkedRelease {
                        release_key: m.release_key,
                        release_title: m.release_title,
                        path: m.candidate_path,
                    });
                }
            }
        }

        let review_count = assignment.review.len() as u32;

        let ok_folders_clone = ok_folders.clone();
        let orphans_paths: Vec<PathBuf> = assignment
            .local_only
            .iter()
            .map(|o| o.path.clone())
            .collect();
        let _ = pool
            .writer
            .call(move |c| {
                let mut counts: std::collections::HashMap<i64, u32> =
                    ok_folders_clone.iter().map(|f| (f.id, 0)).collect();
                for p in &orphans_paths {
                    if let Some(owner) = lib_repo::owning_folder(&ok_folders_clone, p) {
                        *counts.entry(owner.id).or_default() += 1;
                    }
                }
                for f in &ok_folders_clone {
                    let cnt = counts.get(&f.id).copied().unwrap_or(0);
                    lib_repo::update_folder_local_only_count(c, f.id, cnt)?;
                }
                Ok::<_, DbError>(())
            })
            .await;

        if let Ok(stats) = pool
            .reader
            .call(move |c| artists::stats_with_quota(c, band_id))
            .await
        {
            bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
        }

        if !assignment.review.is_empty()
            || !assignment.conflicts.is_empty()
            || !restorable.is_empty()
        {
            let report = ReconcileReport {
                batch_id: 0,
                band_id,
                matched,
                review: assignment.review,
                page_only: assignment.page_only,
                local_only: assignment.local_only,
                conflicts: assignment.conflicts,
                restorable,
                linked,
                unavailable_folders: unavailable_folder_ids,
                catalog,
            };
            bridge.emit(WorkerToUiEvent::ReconcileReport {
                report: Box::new(report),
            });
        }

        RescanOutcome {
            adopted: adopted_count,
            review_count,
        }
    }

    async fn auto_adopt_and_restore_linked_on_verify(
        &self,
        band_id: i64,
        all_folders: &[ArtistFolder],
        ok_folders: &[ArtistFolder],
        st: &Settings,
        cancel: &CancellationToken,
    ) -> u32 {
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

        let query = ReleaseQuery {
            band_id: Some(band_id),
            ..Default::default()
        };
        let ok_roots: Vec<PathBuf> = ok_folders.iter().map(|f| f.path.clone()).collect();
        let all_roots: Vec<PathBuf> = all_folders.iter().map(|f| f.path.clone()).collect();

        let rel_res = pool
            .reader
            .call(move |c| {
                let rels = releases::query_releases(c, &query)?;
                let ign = lib_repo::get_ignored_local(c, band_id)?;
                let rej = lib_repo::get_rejected_pairs(c, band_id)?;
                let split = split_matchable_releases(rels, &all_roots, &ok_roots);
                Ok::<_, DbError>((split.adoptable, ign, rej, split.active_linked_keys))
            })
            .await;

        let Ok((adoptable, ignored, rejected, active_linked_keys)) = rel_res else {
            return 0;
        };

        if adoptable.is_empty() {
            return 0;
        }

        let mut all_candidates = Vec::new();
        for folder in ok_folders {
            let scan_path = folder.path.clone();
            let scan_cancel = cancel.clone();
            if let Ok(Ok(r)) =
                tokio::task::spawn_blocking(move || scan_linked_folder(&scan_path, &scan_cancel))
                    .await
            {
                all_candidates.extend(r.candidates);
            }
        }

        if cancel.is_cancelled() || all_candidates.is_empty() {
            return 0;
        }

        all_candidates.sort_by(|a, b| {
            lib_repo::normalize_path_key(&a.path).cmp(&lib_repo::normalize_path_key(&b.path))
        });

        let unlinked_candidates: Vec<crate::library::AlbumCandidate> = all_candidates
            .into_iter()
            .filter(|c| !active_linked_keys.contains(&lib_repo::normalize_path_key(&c.path)))
            .collect();

        if unlinked_candidates.is_empty() {
            return 0;
        }

        let mut assignment = assign_matches_with_rejected(
            &unlinked_candidates,
            &adoptable,
            st.auto_match_min,
            st.review_match_min,
            &ignored,
            &rejected,
            &st.folder_style,
        );
        drop_downloaded_page_only(&mut assignment, &adoptable);

        let mut applied_count = 0_u32;
        if !assignment.matched.is_empty() || !assignment.restorable.is_empty() {
            if let Ok(b_id) = pool
                .writer
                .call(|c| lib_repo::create_batch(c, "verify", &BatchSummary::default()))
                .await
            {
                let batch_id = b_id.max(0) as u64;
                let mut decisions: Vec<MatchDecision> = assignment
                    .matched
                    .iter()
                    .map(|m| MatchDecision::Adopt {
                        key: m.release_key,
                        path: m.candidate_path.clone(),
                        format: m.format_guess,
                        folder_mtime: None,
                    })
                    .collect();
                for m in &assignment.restorable {
                    decisions.push(MatchDecision::Restore {
                        key: m.release_key,
                        path: m.candidate_path.clone(),
                    });
                }
                applied_count = decisions.len() as u32;
                self.apply_reconcile_internal(batch_id, decisions).await;
            }
        }

        let ok_folders_clone = ok_folders.to_vec();
        let orphans_paths: Vec<PathBuf> = assignment
            .local_only
            .iter()
            .map(|o| o.path.clone())
            .collect();
        let _ = pool
            .writer
            .call(move |c| {
                let mut counts: std::collections::HashMap<i64, u32> =
                    ok_folders_clone.iter().map(|f| (f.id, 0)).collect();
                for p in &orphans_paths {
                    if let Some(owner) = lib_repo::owning_folder(&ok_folders_clone, p) {
                        *counts.entry(owner.id).or_default() += 1;
                    }
                }
                for f in &ok_folders_clone {
                    let cnt = counts.get(&f.id).copied().unwrap_or(0);
                    lib_repo::update_folder_local_only_count(c, f.id, cnt)?;
                }
                Ok::<_, DbError>(())
            })
            .await;

        if let Ok(stats) = pool
            .reader
            .call(move |c| artists::stats_with_quota(c, band_id))
            .await
        {
            bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
        }

        applied_count
    }

    async fn match_artist_folder_internal(
        &self,
        band_id: i64,
        auto_apply_confident: bool,
        cancel: CancellationToken,
    ) -> u32 {
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();

        let folders = match pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
        {
            Ok(f) if !f.is_empty() => f,
            Ok(_) => {
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
                    format!("Failed to query folders for artist {band_id}: {e}"),
                    None,
                );
                return 0;
            }
        };

        let mut ok_folders = Vec::new();
        let mut unavailable_folder_ids = Vec::new();
        let mut probed_folders = Vec::new();
        for f in &folders {
            let state = probe_folder(&f.path);
            let mut updated = f.clone();
            updated.state = state;
            if state == FolderState::Ok {
                ok_folders.push(updated.clone());
            } else {
                unavailable_folder_ids.push(f.id);
                bridge.log(
                    LogLevel::Warn,
                    "library",
                    format!("Folder is {state:?}: {}", f.path.display()),
                    None,
                );
            }
            probed_folders.push(updated);
        }

        bridge.emit(WorkerToUiEvent::FolderStatus {
            band_id,
            folders: probed_folders,
        });

        if ok_folders.is_empty() {
            return 0;
        }

        let st = match pool.reader.call(self.settings_reader()).await {
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

        let mut all_candidates = Vec::new();
        for (idx, folder) in ok_folders.iter().enumerate() {
            bridge.emit(WorkerToUiEvent::LibraryProgress {
                phase: LibraryPhase::Scanning,
                done: idx as u32,
                total: ok_folders.len() as u32,
                current: Some(folder.path.display().to_string()),
            });

            let scan_path = folder.path.clone();
            let scan_cancel = cancel.clone();
            let scan_res =
                tokio::task::spawn_blocking(move || scan_linked_folder(&scan_path, &scan_cancel))
                    .await;
            match scan_res {
                Ok(Ok(r)) => all_candidates.extend(r.candidates),
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
            }
        }

        if cancel.is_cancelled() {
            return 0;
        }

        all_candidates.sort_by(|a, b| {
            lib_repo::normalize_path_key(&a.path).cmp(&lib_repo::normalize_path_key(&b.path))
        });

        let query = ReleaseQuery {
            band_id: Some(band_id),
            ..Default::default()
        };
        let ok_roots: Vec<PathBuf> = ok_folders.iter().map(|f| f.path.clone()).collect();
        let all_roots: Vec<PathBuf> = folders.iter().map(|f| f.path.clone()).collect();
        let rel_res = pool
            .reader
            .call(move |c| {
                let rels = releases::query_releases(c, &query)?;
                let catalog = rels.clone();
                let ign = lib_repo::get_ignored_local(c, band_id)?;
                let rej = lib_repo::get_rejected_pairs(c, band_id)?;
                let split = split_matchable_releases(rels, &all_roots, &ok_roots);
                Ok::<_, DbError>((
                    split.adoptable,
                    catalog,
                    ign,
                    rej,
                    split.active_linked_keys,
                    split.linked,
                ))
            })
            .await;

        let (releases, catalog, ignored, rejected, active_linked_keys, linked) = match rel_res {
            Ok(tuple) => tuple,
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

        let unlinked_candidates: Vec<crate::library::AlbumCandidate> = all_candidates
            .into_iter()
            .filter(|c| !active_linked_keys.contains(&lib_repo::normalize_path_key(&c.path)))
            .collect();

        bridge.emit(WorkerToUiEvent::LibraryProgress {
            phase: LibraryPhase::Matching,
            done: 0,
            total: unlinked_candidates.len() as u32,
            current: None,
        });

        let mut assignment = assign_matches_with_rejected(
            &unlinked_candidates,
            &releases,
            st.auto_match_min,
            st.review_match_min,
            &ignored,
            &rejected,
            &st.folder_style,
        );
        drop_downloaded_page_only(&mut assignment, &releases);

        let mut batch_id = match pool
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

        let mut matched = assignment.matched;
        let mut restorable = assignment.restorable;
        let mut linked = linked;
        let mut adopted_count = 0_u32;
        if auto_apply_confident && (!matched.is_empty() || !restorable.is_empty()) {
            let mut decisions: Vec<MatchDecision> = matched
                .iter()
                .map(|m| MatchDecision::Adopt {
                    key: m.release_key,
                    path: m.candidate_path.clone(),
                    format: m.format_guess,
                    folder_mtime: None,
                })
                .collect();
            for m in &restorable {
                decisions.push(MatchDecision::Restore {
                    key: m.release_key,
                    path: m.candidate_path.clone(),
                });
            }
            adopted_count = decisions.len() as u32;
            self.apply_reconcile_internal(batch_id, decisions).await;

            for m in matched.drain(..) {
                linked.push(crate::library::service::LinkedRelease {
                    release_key: m.release_key,
                    release_title: m.release_title,
                    path: m.candidate_path,
                });
            }
            for m in restorable.drain(..) {
                linked.push(crate::library::service::LinkedRelease {
                    release_key: m.release_key,
                    release_title: m.release_title,
                    path: m.candidate_path,
                });
            }

            if let Ok(new_id) = pool
                .writer
                .call(|c| {
                    lib_repo::create_batch(c, "adopt", &crate::types::BatchSummary::default())
                })
                .await
            {
                batch_id = new_id.max(0) as u64;
            }
        }

        let ok_folders_clone = ok_folders.clone();
        let orphans_paths: Vec<PathBuf> = assignment
            .local_only
            .iter()
            .map(|o| o.path.clone())
            .collect();
        let _ = pool
            .writer
            .call(move |c| {
                let mut counts: std::collections::HashMap<i64, u32> =
                    ok_folders_clone.iter().map(|f| (f.id, 0)).collect();
                for p in &orphans_paths {
                    if let Some(owner) = lib_repo::owning_folder(&ok_folders_clone, p) {
                        *counts.entry(owner.id).or_default() += 1;
                    }
                }
                for f in &ok_folders_clone {
                    let cnt = counts.get(&f.id).copied().unwrap_or(0);
                    lib_repo::update_folder_local_only_count(c, f.id, cnt)?;
                }
                Ok::<_, DbError>(())
            })
            .await;

        let report = ReconcileReport {
            batch_id,
            band_id,
            matched,
            review: assignment.review,
            page_only: assignment.page_only,
            local_only: assignment.local_only,
            conflicts: assignment.conflicts,
            restorable,
            linked,
            unavailable_folders: unavailable_folder_ids,
            catalog,
        };

        bridge.emit(WorkerToUiEvent::ReconcileReport {
            report: Box::new(report.clone()),
        });

        if let Ok(stats) = pool
            .reader
            .call(move |c| artists::stats_with_quota(c, band_id))
            .await
        {
            bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
        }

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
                let actual_batch_id = if batch_id == 0 {
                    lib_repo::create_batch(c, "adopt", &BatchSummary::default())?
                } else {
                    batch_id as i64
                };
                let mut summary = BatchSummary::default();
                let mut touched_keys = Vec::new();
                let mut touched_bands = std::collections::HashSet::new();

                for decision in decisions {
                    match decision {
                        MatchDecision::Adopt {
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
                                actual_batch_id,
                            )?;
                            summary.adopted += 1;
                            touched_keys.push(key);
                        }
                        MatchDecision::Attach {
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
                            lib_repo::attach(
                                c,
                                key,
                                &path,
                                format,
                                crate::types::Origin::Imported,
                                mtime,
                                actual_batch_id,
                            )?;
                            summary.adopted += 1;
                            touched_keys.push(key);
                        }
                        MatchDecision::Restore { key, path } => {
                            lib_repo::restore(c, key, &path, actual_batch_id)?;
                            summary.restored += 1;
                            touched_keys.push(key);
                        }
                        MatchDecision::Ignore { band_id, path } => {
                            let path_key =
                                crate::library::scan::normalize_candidate_path_key(&path);
                            lib_repo::ignore_local(c, band_id, &path_key)?;
                            summary.ignored += 1;
                            touched_bands.insert(band_id);
                        }
                        MatchDecision::Reject { band_id, key, path } => {
                            let path_key =
                                crate::library::scan::normalize_candidate_path_key(&path);
                            lib_repo::reject_match_pair(
                                c,
                                band_id,
                                key,
                                &path_key,
                                Some(actual_batch_id),
                            )?;
                            summary.ignored += 1;
                            touched_bands.insert(band_id);
                        }
                    }
                }

                lib_repo::update_batch_summary(c, actual_batch_id, &summary)?;

                let mut updated_rows = Vec::new();
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

                Ok::<_, DbError>((actual_batch_id as u64, summary, updated_rows, band_stats))
            })
            .await;

        match res {
            Ok((actual_batch_id, summary, rows, stats_per_band)) => {
                for row in rows {
                    bridge.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(row)));
                }
                for (band_id, stats) in stats_per_band {
                    bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
                }
                bridge.emit(WorkerToUiEvent::LibraryApplied {
                    batch_id: actual_batch_id,
                    summary,
                });
                bridge.log(
                    LogLevel::Info,
                    "library",
                    format!(
                        "Applied batch {actual_batch_id}: {} adopted, {} restored, {} ignored",
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
                let mut st = Settings::default();
                st.folder_style.templates =
                    vec!["{artist} - {title}".to_string(), "{title}".to_string()];
                settings::set_settings(c, &st)?;

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
                    folders,
                } if *b == band_id && folders.iter().any(|f| f.state == FolderState::Ok)
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
        let folders_in_db = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap();
        assert_eq!(folders_in_db.len(), 1);
        let folder_id = folders_in_db[0].id;
        assert_eq!(
            folders_in_db[0].path,
            dunce::canonicalize(&artist_dir).unwrap()
        );

        // 2. Unlink artist folder
        svc.unlink_artist_folder(folder_id).await;

        let mut unlink_events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            unlink_events.push(ev);
        }

        let has_folder_unlinked = unlink_events.iter().any(|e| {
            matches!(
                e,
                WorkerToUiEvent::FolderStatus {
                    band_id: b,
                    folders,
                } if *b == band_id && folders.is_empty()
            )
        });
        assert!(has_folder_unlinked);

        let folder_after_unlink = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap();
        assert!(folder_after_unlink.is_empty());
    }

    #[tokio::test]
    async fn test_library_service_local_only_updates_stats_badge() {
        let dir = tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let db_path = root.join("local_only_stats.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 9999;
        pool.writer
            .call(move |c| {
                let mut st = Settings::default();
                st.folder_style.templates =
                    vec!["{artist} - {title}".to_string(), "{title}".to_string()];
                settings::set_settings(c, &st)?;

                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "leichen-test",
                        name: "Leichen",
                        url: "https://leichen.bandcamp.com",
                        cover_url: None,
                    },
                )?;

                // 1 release on page, free & not yet downloaded
                c.execute(
                    "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, downloaded, last_seen_at)
                     VALUES ('album', 101, ?1, 'Free 1', 'Leichen', 'https://leichen.bandcamp.com/album/101', 'free', 0, 1000)",
                    rusqlite::params![band_id],
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        // Local artist folder has Free 1, plus 2 extra local-only album folders
        let artist_dir = root.join("Leichen");
        let album_free = artist_dir.join("Leichen - Free 1");
        let album_extra1 = artist_dir.join("Leichen - Extra Bootleg 1");
        let album_extra2 = artist_dir.join("Leichen - Extra Bootleg 2");
        fs::create_dir_all(&album_free).unwrap();
        fs::write(album_free.join("01.mp3"), b"audio").unwrap();
        fs::create_dir_all(&album_extra1).unwrap();
        fs::write(album_extra1.join("01.mp3"), b"audio").unwrap();
        fs::create_dir_all(&album_extra2).unwrap();
        fs::write(album_extra2.join("01.mp3"), b"audio").unwrap();

        let cancel = CancellationToken::new();

        // 1. Link artist folder with auto_apply_confident true
        svc.link_artist_folder(band_id, artist_dir.clone(), true, cancel.clone())
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Verify ReconcileReport emitted with 2 local_only
        let report_opt = events.iter().find_map(|e| match e {
            WorkerToUiEvent::ReconcileReport { report } => Some(report),
            _ => None,
        });
        let report = report_opt.expect("should have emitted ReconcileReport");
        assert_eq!(report.local_only.len(), 2);

        // Verify local_only_count in artist_folders table
        let count_in_db: u32 = pool
            .reader
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT local_only_count FROM artist_folders WHERE band_id = ?1",
                    rusqlite::params![band_id],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(count_in_db, 2);

        // Verify stats query reflects the 2 local_only in non_eligible_downloaded and downloaded
        let stats = pool
            .reader
            .call(move |c| releases::artist_stats_for_one(c, band_id))
            .await
            .unwrap();
        assert_eq!(stats.total, 1);
        assert_eq!(stats.eligible, 1);
        assert_eq!(stats.eligible_downloaded, 1);
        assert_eq!(stats.non_eligible_downloaded, 2);
        assert_eq!(stats.downloaded, 3);
        assert_eq!(stats.format_sidebar_badge(), "1 (+2)/1/1");

        // 2. Ignore 1 local-only path
        svc.ignore_local(band_id, vec![album_extra1]).await;

        let stats_after_ignore = pool
            .reader
            .call(move |c| releases::artist_stats_for_one(c, band_id))
            .await
            .unwrap();
        assert_eq!(stats_after_ignore.non_eligible_downloaded, 1);
        assert_eq!(stats_after_ignore.downloaded, 2);
        assert_eq!(stats_after_ignore.format_sidebar_badge(), "1 (+1)/1/1");

        // 3. Unlink folder: local_only_count disappears and stats revert to 0 non-eligible
        let folders = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap();
        let folder_id = folders[0].id;
        svc.unlink_artist_folder(folder_id).await;

        let stats_after_unlink = pool
            .reader
            .call(move |c| releases::artist_stats_for_one(c, band_id))
            .await
            .unwrap();
        assert_eq!(stats_after_unlink.non_eligible_downloaded, 0);
        assert_eq!(stats_after_unlink.downloaded, 1);
        assert_eq!(stats_after_unlink.format_sidebar_badge(), "1/1/1");
    }

    #[tokio::test]
    async fn test_attach_orphan_folder_decrements_local_only_and_clears_badge_plus() {
        let dir = tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let db_path = root.join("attach_test.db");
        let music_dir = root.join("music");
        fs::create_dir_all(&music_dir).unwrap();

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 8888;
        pool.writer
            .call(move |c| {
                let mut st = Settings::default();
                st.folder_style.templates =
                    vec!["{artist} - {title}".to_string(), "{title}".to_string()];
                settings::set_settings(c, &st)?;

                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "attach-artist",
                        name: "AttachArtist",
                        url: "https://attach.bandcamp.com",
                        cover_url: None,
                    },
                )?;

                // 2 releases on page, free & not yet downloaded
                c.execute(
                    "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, downloaded, last_seen_at)
                     VALUES ('album', 201, ?1, 'Free 1', 'AttachArtist', 'https://attach.bandcamp.com/album/201', 'free', 0, 1000)",
                    rusqlite::params![band_id],
                )?;
                c.execute(
                    "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, downloaded, last_seen_at)
                     VALUES ('album', 202, ?1, 'Free 2', 'AttachArtist', 'https://attach.bandcamp.com/album/202', 'free', 0, 1000)",
                    rusqlite::params![band_id],
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        // Local artist folder has Free 1 (matches 201), plus 1 orphan folder
        let artist_dir = music_dir.join("AttachArtist");
        let album_free = artist_dir.join("AttachArtist - Free 1");
        let album_orphan = artist_dir.join("AttachArtist - Unmatched Live 1999");
        fs::create_dir_all(&album_free).unwrap();
        fs::write(album_free.join("01.mp3"), b"audio").unwrap();
        fs::create_dir_all(&album_orphan).unwrap();
        fs::write(album_orphan.join("01.mp3"), b"audio").unwrap();

        let cancel = CancellationToken::new();

        // 1. Link artist folder with auto_apply_confident true
        svc.link_artist_folder(band_id, artist_dir.clone(), true, cancel.clone())
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        let report_opt = events.iter().find_map(|e| match e {
            WorkerToUiEvent::ReconcileReport { report } => Some(report),
            _ => None,
        });
        let report = report_opt.expect("should have emitted ReconcileReport");
        assert_eq!(report.linked.len(), 1);
        assert_eq!(report.local_only.len(), 1);

        // Verify initial badge is "1 (+1)/2/2"
        let stats_before = pool
            .reader
            .call(move |c| releases::artist_stats_for_one(c, band_id))
            .await
            .unwrap();
        assert_eq!(stats_before.total, 2);
        assert_eq!(stats_before.eligible, 2);
        assert_eq!(stats_before.eligible_downloaded, 1);
        assert_eq!(stats_before.non_eligible_downloaded, 1);
        assert_eq!(stats_before.downloaded, 2);
        assert_eq!(stats_before.format_sidebar_badge(), "1 (+1)/2/2");

        // 2. Attach the orphan folder to release 202
        let key_202 = ItemKey::new(ItemType::Album, 202);
        let decisions = vec![MatchDecision::Attach {
            key: key_202,
            path: album_orphan.clone(),
            format: Some(crate::types::AudioFormat::Mp3_320),
            folder_mtime: None,
        }];
        svc.apply_reconcile(report.batch_id, decisions).await;

        // Verify local_only_count decremented to 0
        let count_in_db: u32 = pool
            .reader
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT local_only_count FROM artist_folders WHERE band_id = ?1",
                    rusqlite::params![band_id],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(count_in_db, 0);

        // Verify stats: non_eligible_downloaded is 0, badge is "2/2/2" without (+1)
        let stats_after = pool
            .reader
            .call(move |c| releases::artist_stats_for_one(c, band_id))
            .await
            .unwrap();
        assert_eq!(stats_after.eligible_downloaded, 2);
        assert_eq!(stats_after.non_eligible_downloaded, 0);
        assert_eq!(stats_after.downloaded, 2);
        assert_eq!(stats_after.format_sidebar_badge(), "2/2/2");

        // 3. Undo the batch and verify stats revert back to "1 (+1)/2/2"
        svc.undo_batch(report.batch_id).await;

        let count_after_undo: u32 = pool
            .reader
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT local_only_count FROM artist_folders WHERE band_id = ?1",
                    rusqlite::params![band_id],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(count_after_undo, 1);

        let stats_after_undo = pool
            .reader
            .call(move |c| releases::artist_stats_for_one(c, band_id))
            .await
            .unwrap();
        assert_eq!(stats_after_undo.eligible_downloaded, 1);
        assert_eq!(stats_after_undo.non_eligible_downloaded, 1);
        assert_eq!(stats_after_undo.format_sidebar_badge(), "1 (+1)/2/2");
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
                lib_repo::add_folder(c, band_id, &artist_dir, &out_dir)?;
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
        let root = dunce::canonicalize(dir.path()).unwrap();
        let db_path = root.join("apply_undo_test.db");

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

        let artist_dir = root.join("Artist 555");
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
        let folders = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap();
        assert!(!folders.is_empty());

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
                    folders,
                } if *b == band_id && folders.iter().any(|f| f.state == FolderState::Ok)
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

                lib_repo::add_folder(c, band_id, &artist_dir, &out_dir)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
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
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
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

        let folder_unavail = events.iter().find_map(|e| match e {
            WorkerToUiEvent::FolderUnavailable {
                folders, prompt, ..
            } => Some((folders, *prompt)),
            _ => None,
        });
        assert!(folder_unavail.is_some());
        let (folders, prompt) = folder_unavail.unwrap();
        assert!(prompt);
        assert_eq!(folders.len(), 1);
        let folder_id = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap()[0]
            .id;
        assert_eq!(folders[0].root, UnavailableRoot::Linked { folder_id });
        assert_eq!(folders[0].state, FolderState::Missing);
        assert_eq!(folders[0].releases.len(), 1);
        assert_eq!(folders[0].releases[0].key, key);

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
                lib_repo::add_folder(c, band_id, &a_dir, &out_dir)?;

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
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
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
        let root = dunce::canonicalize(dir.path()).unwrap();
        let db_path = root.join("relink_test.db");

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

        let artist_dir = root.join("Artist 444");
        let original_path = artist_dir.join("Old Album Folder");
        let moved_path = artist_dir.join("Moved Album Folder");
        fs::create_dir_all(&moved_path).unwrap();
        fs::write(moved_path.join("01.mp3"), b"audio").unwrap();

        let out_dir = root.join("Downloads");
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

                lib_repo::add_folder(c, band_id, &a_dir, &out_dir)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
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

                lib_repo::add_folder(c, band_id, &a_dir, &out_dir)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
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
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
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
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
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
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
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
        let root = dunce::canonicalize(dir.path()).unwrap();
        let db_path = root.join("repoint_test.db");

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

        let old_artist_dir = root.join("Old Artist Folder");
        let new_artist_dir = root.join("New Artist Folder");
        let album_in_new = new_artist_dir.join("Album");
        fs::create_dir_all(&album_in_new).unwrap();
        fs::write(album_in_new.join("01.mp3"), b"audio").unwrap();

        let out_dir = root.join("downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let dummy_old_path = old_artist_dir.join("Album");
        let a_old = old_artist_dir.clone();
        let a_new = new_artist_dir.clone();
        let custom_out_dir = out_dir.clone();

        fs::create_dir_all(&old_artist_dir).unwrap();

        let folder = pool
            .writer
            .call(move |c| {
                let s = Settings {
                    out_dir: custom_out_dir.clone(),
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
                let f = lib_repo::add_folder(c, band_id, &a_old, &custom_out_dir)?;
                Ok::<_, DbError>(f)
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.repoint_artist_folder(folder.id, a_new, cancel).await;

        // Folder link should be updated to new_artist_dir
        let folders = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(
            folders[0].path,
            dunce::canonicalize(&new_artist_dir).unwrap()
        );

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
        let root = dunce::canonicalize(dir.path()).unwrap();
        let db_path = root.join("rescan_adopt_test.db");

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

        let artist_dir = root.join("Artist 801");
        let album_dir = artist_dir.join("Album One");
        fs::create_dir_all(&album_dir).unwrap();
        fs::write(album_dir.join("01.mp3"), b"audio").unwrap();

        let out_dir = root.join("downloads");
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

                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;
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

                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;
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

                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;
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

    #[tokio::test]
    async fn test_already_linked_candidates_are_not_reported_as_local_only() {
        let dir = tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let db_path = root.join("already_linked_test.db");

        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 888;
        let artist_dir = root.join("Artist 888");
        let album_dir = artist_dir.join("Album One [FLAC]");
        fs::create_dir_all(&album_dir).unwrap();
        fs::write(album_dir.join("01.flac"), b"dummy audio").unwrap();

        let out_dir = root.join("downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let key = ItemKey {
            kind: ItemType::Album,
            id: 8881,
        };

        // Insert artist and release already downloaded and linked to album_dir
        let a_dir = artist_dir.clone();
        let alb_dir = album_dir.clone();
        let o_dir = out_dir.clone();
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-888",
                        name: "Artist 888",
                        url: "https://artist888.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key,
                        band_id,
                        title: "Album One",
                        artist_name: "Artist 888",
                        url: "https://artist888.bandcamp.com/album/album-one",
                        cover_url: None,
                        art_id: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;

                // Mark release downloaded at alb_dir
                let batch_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key,
                    &alb_dir,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::Imported,
                    None,
                    batch_id,
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();

        // 1. Calling match_artist_folder should find 0 unlinked candidates, so local_only is 0
        svc.match_artist_folder(band_id, false, cancel.clone())
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
            .expect("should emit ReconcileReport");

        // The candidate at album_dir is already linked, so local_only MUST be 0!
        assert_eq!(report.local_only.len(), 0);
        assert_eq!(report.matched.len(), 0);
        assert_eq!(report.review.len(), 0);
        assert_eq!(report.page_only.len(), 0);
        assert_eq!(report.linked.len(), 1);
        assert_eq!(report.linked[0].release_key, key);
        assert_eq!(report.linked[0].release_title, "Album One");

        // 2. Calling rescan_and_adopt_on_sync should also produce 0 review and 0 adopted
        let outcome = svc.rescan_and_adopt_on_sync(band_id, cancel).await;
        assert_eq!(outcome.adopted, 0);
        assert_eq!(outcome.review_count, 0);
    }

    #[tokio::test]
    async fn test_unavailable_folder_regression_three_adopted_releases() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("reg_test.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 501;
        let artist_dir = dir.path().join("Artist 501");
        fs::create_dir_all(&artist_dir).unwrap();
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let a_dir = artist_dir.clone();
        let o_dir = out_dir.clone();
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-501",
                        name: "Artist 501",
                        url: "https://artist501.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;

                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                for i in 1..=3 {
                    let key = ItemKey::new(ItemType::Album, i);
                    let disc = DiscoveredRelease {
                        key,
                        band_id,
                        title: &format!("Album {i}"),
                        artist_name: "Artist 501",
                        url: &format!("https://artist501.bandcamp.com/album/{i}"),
                        cover_url: None,
                        art_id: None,
                    };
                    releases::upsert_discovered(c, &disc)?;

                    let alb_dir = a_dir.join(format!("Album {i}"));
                    fs::create_dir_all(&alb_dir).unwrap();
                    fs::write(alb_dir.join("01.flac"), b"audio").unwrap();

                    lib_repo::adopt(
                        c,
                        key,
                        &alb_dir,
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

        // Delete the artist's linked folder entirely
        fs::remove_dir_all(&artist_dir).unwrap();

        // 1. VerifyFiles with Manual trigger (on_missing = Ask)
        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
        .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Must emit FolderUnavailable with prompt: true, 3 releases, NO MissingFound
        let has_missing_found = events
            .iter()
            .any(|e| matches!(e, WorkerToUiEvent::MissingFound { .. }));
        assert!(!has_missing_found, "Must NOT emit MissingFound");

        let folder_ev = events.into_iter().find_map(|e| match e {
            WorkerToUiEvent::FolderUnavailable {
                folders, prompt, ..
            } => Some((folders, prompt)),
            _ => None,
        });
        assert!(folder_ev.is_some(), "Must emit FolderUnavailable");
        let (folders, prompt) = folder_ev.unwrap();
        assert!(prompt, "Manual trigger must prompt");
        assert_eq!(folders.len(), 1);
        let folder_id = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap()[0]
            .id;
        assert_eq!(folders[0].root, UnavailableRoot::Linked { folder_id });
        assert_eq!(folders[0].state, FolderState::Missing);
        assert_eq!(folders[0].releases.len(), 3);

        // Database rows must NOT be changed to missing or archived
        for i in 1..=3 {
            let key = ItemKey::new(ItemType::Album, i);
            let rel = pool
                .reader
                .call(move |c| releases::get_release(c, key))
                .await
                .unwrap()
                .unwrap();
            assert!(rel.downloaded);
            assert_eq!(rel.local_status, Some(LocalStatus::Present));
        }
    }

    #[tokio::test]
    async fn test_unavailable_folder_auto_mode_never_auto_resolves() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("auto_never_test.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 502;
        let artist_dir = dir.path().join("Artist 502");
        fs::create_dir_all(&artist_dir).unwrap();
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let a_dir = artist_dir.clone();
        let o_dir = out_dir.clone();
        pool.writer
            .call(move |c| {
                let mut st = settings::get_settings(c, o_dir.clone())?;
                st.on_missing = OnMissing::Auto;
                st.deleted_policy = DeletedPolicy::KeepMarked;
                settings::set_settings(c, &st)?;

                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-502",
                        name: "Artist 502",
                        url: "https://artist502.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;

                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                let key = ItemKey::new(ItemType::Album, 5021);
                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Album 1",
                    artist_name: "Artist 502",
                    url: "https://artist502.bandcamp.com/album/1",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;

                let alb_dir = a_dir.join("Album 1");
                fs::create_dir_all(&alb_dir).unwrap();
                fs::write(alb_dir.join("01.flac"), b"audio").unwrap();

                lib_repo::adopt(
                    c,
                    key,
                    &alb_dir,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        fs::remove_dir_all(&artist_dir).unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
        .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Even with on_missing = Auto, it never auto-archives unavailable folders!
        let has_missing_found = events
            .iter()
            .any(|e| matches!(e, WorkerToUiEvent::MissingFound { .. }));
        assert!(!has_missing_found);

        let folder_ev = events.into_iter().find_map(|e| match e {
            WorkerToUiEvent::FolderUnavailable { folders, .. } => Some(folders),
            _ => None,
        });
        assert!(folder_ev.is_some());

        let key = ItemKey::new(ItemType::Album, 5021);
        let rel = pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .unwrap()
            .unwrap();
        assert!(rel.downloaded);
        assert_eq!(rel.local_status, Some(LocalStatus::Present));
    }

    #[tokio::test]
    async fn test_unavailable_folder_triggers_and_session_ack() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("triggers_ack_test.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 503;
        let artist_dir = dir.path().join("Artist 503");
        fs::create_dir_all(&artist_dir).unwrap();
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let key = ItemKey::new(ItemType::Album, 5031);
        let a_dir = artist_dir.clone();
        let o_dir = out_dir.clone();
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-503",
                        name: "Artist 503",
                        url: "https://artist503.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;

                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Album 1",
                    artist_name: "Artist 503",
                    url: "https://artist503.bandcamp.com/album/1",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;

                let alb_dir = a_dir.join("Album 1");
                fs::create_dir_all(&alb_dir).unwrap();
                fs::write(alb_dir.join("01.flac"), b"audio").unwrap();

                lib_repo::adopt(
                    c,
                    key,
                    &alb_dir,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        fs::remove_dir_all(&artist_dir).unwrap();

        // 1. SyncInteractive -> prompt: true
        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::SyncInteractive,
            cancel.clone(),
        )
        .await;

        let mut prompt_opt = None;
        let mut batch_id_opt = None;
        while let Ok(ev) = rx.try_recv() {
            if let WorkerToUiEvent::FolderUnavailable {
                prompt, batch_id, ..
            } = ev
            {
                prompt_opt = Some(prompt);
                batch_id_opt = Some(batch_id);
            }
        }
        assert_eq!(prompt_opt, Some(true), "SyncInteractive must prompt");
        let batch_id = batch_id_opt.unwrap();

        // 2. SyncBackground -> prompt: false
        svc.verify_files(
            SyncScope::All,
            VerifyTrigger::SyncBackground,
            cancel.clone(),
        )
        .await;
        let mut prompt_bg = None;
        while let Ok(ev) = rx.try_recv() {
            if let WorkerToUiEvent::FolderUnavailable { prompt, .. } = ev {
                prompt_bg = Some(prompt);
            }
        }
        assert_eq!(prompt_bg, Some(false), "SyncBackground must NOT prompt");

        // 3. User chooses Keep -> added to session acked_unavailable
        let folder_id = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap()[0]
            .id;
        svc.resolve_unavailable(
            batch_id,
            vec![UnavailableDecision {
                root: UnavailableRoot::Linked { folder_id },
                keys: vec![key],
                action: UnavailableAction::Keep,
            }],
        )
        .await;

        // 4. Next SyncInteractive -> prompt: false because root is acked!
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::SyncInteractive,
            cancel.clone(),
        )
        .await;
        let mut prompt_acked = None;
        while let Ok(ev) = rx.try_recv() {
            if let WorkerToUiEvent::FolderUnavailable { prompt, .. } = ev {
                prompt_acked = Some(prompt);
            }
        }
        assert_eq!(
            prompt_acked,
            Some(false),
            "Acked root on SyncInteractive must NOT prompt"
        );

        // 5. Manual verify -> prompt: true AND clears the ack
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel.clone(),
        )
        .await;
        let mut prompt_manual = None;
        while let Ok(ev) = rx.try_recv() {
            if let WorkerToUiEvent::FolderUnavailable { prompt, .. } = ev {
                prompt_manual = Some(prompt);
            }
        }
        assert_eq!(
            prompt_manual,
            Some(true),
            "Manual verify must always prompt"
        );

        // 6. Recovery: recreate folder and album -> verify finds it Present, clears ack
        fs::create_dir_all(&artist_dir).unwrap();
        let alb_dir = artist_dir.join("Album 1");
        fs::create_dir_all(&alb_dir).unwrap();
        fs::write(alb_dir.join("01.flac"), b"audio").unwrap();

        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
        .await;
        let mut has_unavail = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, WorkerToUiEvent::FolderUnavailable { .. }) {
                has_unavail = true;
            }
        }
        assert!(
            !has_unavail,
            "Recovered folder must not emit FolderUnavailable"
        );
        assert!(
            !svc.acked_unavailable
                .lock()
                .await
                .contains(&UnavailableRoot::Linked { folder_id }),
            "Recovered folder must remove ack"
        );
    }

    #[tokio::test]
    async fn test_unavailable_folder_mixed_artist_linked_and_out_dir() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("mixed_artist_test.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 504;
        let artist_dir = dir.path().join("Artist 504");
        fs::create_dir_all(&artist_dir).unwrap();
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let key_linked = ItemKey::new(ItemType::Album, 5041);
        let key_out = ItemKey::new(ItemType::Album, 5042);

        let alb_linked = artist_dir.join("Album Linked");
        fs::create_dir_all(&alb_linked).unwrap();
        fs::write(alb_linked.join("01.flac"), b"audio").unwrap();

        let alb_out = out_dir.join("Album Out");
        fs::create_dir_all(&alb_out).unwrap();
        fs::write(alb_out.join("01.flac"), b"audio").unwrap();

        let a_dir = artist_dir.clone();
        let o_dir = out_dir.clone();
        let l_path = alb_linked.clone();
        let o_path = alb_out.clone();

        pool.writer
            .call(move |c| {
                let mut st = settings::get_settings(c, o_dir.clone())?;
                st.out_dir = o_dir.clone();
                st.on_missing = OnMissing::Ask;
                settings::set_settings(c, &st)?;

                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-504",
                        name: "Artist 504",
                        url: "https://artist504.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;

                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;

                let disc1 = DiscoveredRelease {
                    key: key_linked,
                    band_id,
                    title: "Album Linked",
                    artist_name: "Artist 504",
                    url: "https://artist504.bandcamp.com/album/linked",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc1)?;
                lib_repo::adopt(
                    c,
                    key_linked,
                    &l_path,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;

                let disc2 = DiscoveredRelease {
                    key: key_out,
                    band_id,
                    title: "Album Out",
                    artist_name: "Artist 504",
                    url: "https://artist504.bandcamp.com/album/out",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc2)?;
                lib_repo::adopt(
                    c,
                    key_out,
                    &o_path,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;

                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        // Delete artist_dir (linked folder)
        fs::remove_dir_all(&artist_dir).unwrap();
        // Delete only the album folder inside out_dir (out_dir itself remains valid)
        fs::remove_dir_all(&alb_out).unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
        .await;

        let mut unavail_opt = None;
        let mut missing_opt = None;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                WorkerToUiEvent::FolderUnavailable { folders, .. } => {
                    unavail_opt = Some(folders);
                }
                WorkerToUiEvent::MissingFound { items, .. } => {
                    missing_opt = Some(items);
                }
                _ => {}
            }
        }

        // Linked release goes to FolderUnavailable
        assert!(unavail_opt.is_some());
        let folders = unavail_opt.unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].releases.len(), 1);
        assert_eq!(folders[0].releases[0].key, key_linked);

        // out_dir release goes to MissingFound
        assert!(missing_opt.is_some());
        let missing_items = missing_opt.unwrap();
        assert_eq!(missing_items.len(), 1);
        assert_eq!(missing_items[0].key, key_out);
    }

    #[tokio::test]
    async fn test_unavailable_folder_archived_releases_not_listed() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("archived_not_listed.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 505;
        let artist_dir = dir.path().join("Artist 505");
        fs::create_dir_all(&artist_dir).unwrap();
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let key_active = ItemKey::new(ItemType::Album, 5051);
        let key_archived = ItemKey::new(ItemType::Album, 5052);

        let a_dir = artist_dir.clone();
        let o_dir = out_dir.clone();

        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-505",
                        name: "Artist 505",
                        url: "https://artist505.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;

                let disc1 = DiscoveredRelease {
                    key: key_active,
                    band_id,
                    title: "Active",
                    artist_name: "Artist 505",
                    url: "https://artist505.bandcamp.com/album/active",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc1)?;
                lib_repo::adopt(
                    c,
                    key_active,
                    &a_dir.join("Active"),
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;

                let disc2 = DiscoveredRelease {
                    key: key_archived,
                    band_id,
                    title: "Archived",
                    artist_name: "Artist 505",
                    url: "https://artist505.bandcamp.com/album/archived",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc2)?;
                lib_repo::adopt(
                    c,
                    key_archived,
                    &a_dir.join("Archived"),
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                lib_repo::archive(c, &[key_archived], b_id)?;

                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        fs::remove_dir_all(&artist_dir).unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
        .await;

        let mut unavail_folders = None;
        while let Ok(ev) = rx.try_recv() {
            if let WorkerToUiEvent::FolderUnavailable { folders, .. } = ev {
                unavail_folders = Some(folders);
            }
        }

        assert!(unavail_folders.is_some());
        let folders = unavail_folders.unwrap();
        assert_eq!(folders.len(), 1);
        // Only the active release should be present, NOT the archived one
        assert_eq!(folders[0].releases.len(), 1);
        assert_eq!(folders[0].releases[0].key, key_active);
    }

    #[tokio::test]
    async fn test_unavailable_folder_when_all_releases_archived() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("all_archived_unavail.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 707;
        let artist_dir = dir.path().join("Artist 707");
        fs::create_dir_all(&artist_dir).unwrap();
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let key_archived = ItemKey::new(ItemType::Album, 7071);
        let a_dir = artist_dir.clone();
        let o_dir = out_dir.clone();

        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-707",
                        name: "Artist 707",
                        url: "https://artist707.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;

                let disc = DiscoveredRelease {
                    key: key_archived,
                    band_id,
                    title: "Archived Album",
                    artist_name: "Artist 707",
                    url: "https://artist707.bandcamp.com/album/archived",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;
                lib_repo::adopt(
                    c,
                    key_archived,
                    &a_dir.join("Archived Album"),
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                lib_repo::archive(c, &[key_archived], b_id)?;

                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        fs::remove_dir_all(&artist_dir).unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
        .await;

        let mut unavail_folders = None;
        let mut b_id = 0;
        while let Ok(ev) = rx.try_recv() {
            if let WorkerToUiEvent::FolderUnavailable {
                batch_id, folders, ..
            } = ev
            {
                b_id = batch_id;
                unavail_folders = Some(folders);
            }
        }

        assert!(
            unavail_folders.is_some(),
            "FolderUnavailable must be emitted even if all releases are archived"
        );
        let folders = unavail_folders.unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].releases.len(), 0);
        let folder_id = match folders[0].root {
            UnavailableRoot::Linked { folder_id } => folder_id,
            _ => panic!("Expected Linked root"),
        };

        // Resolving by unlinking must work
        svc.resolve_unavailable(
            b_id,
            vec![UnavailableDecision {
                root: UnavailableRoot::Linked { folder_id },
                keys: vec![],
                action: UnavailableAction::Deleted {
                    policy: DeletedPolicy::KeepMarked,
                    unlink: true,
                },
            }],
        )
        .await;

        let mut unlinked_ev = false;
        while let Ok(ev) = rx.try_recv() {
            if let WorkerToUiEvent::FolderStatus {
                band_id: b,
                folders,
            } = ev
                && b == band_id
                && folders.is_empty()
            {
                unlinked_ev = true;
            }
        }
        assert!(
            unlinked_ev,
            "FolderStatus empty must be emitted after unlinking"
        );
    }

    #[tokio::test]
    async fn test_unavailable_folder_out_dir_missing() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("out_dir_missing.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 506;
        let out_dir = dir.path().join("NonexistentDownloads");
        // out_dir does not exist

        let key = ItemKey::new(ItemType::Album, 5061);
        let o_dir = out_dir.clone();

        pool.writer
            .call(move |c| {
                let st = settings::get_settings(c, o_dir.clone())?;
                settings::set_settings(c, &st)?;

                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-506",
                        name: "Artist 506",
                        url: "https://artist506.bandcamp.com",
                        cover_url: None,
                    },
                )?;

                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Album 1",
                    artist_name: "Artist 506",
                    url: "https://artist506.bandcamp.com/album/1",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;
                lib_repo::adopt(
                    c,
                    key,
                    &o_dir.join("Album 1"),
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
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
        .await;

        let mut unavail_folders = None;
        while let Ok(ev) = rx.try_recv() {
            if let WorkerToUiEvent::FolderUnavailable { folders, .. } = ev {
                unavail_folders = Some(folders);
            }
        }

        assert!(unavail_folders.is_some());
        let folders = unavail_folders.unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].root, UnavailableRoot::OutDir);
        assert_eq!(folders[0].state, FolderState::Missing);
        assert_eq!(folders[0].releases.len(), 1);
        assert_eq!(folders[0].releases[0].key, key);
    }

    #[tokio::test]
    async fn test_unavailable_folder_mass_missing_ratio_ignores_unknown() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("mass_ratio_test.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 507;
        let artist_dir = dir.path().join("Artist 507");
        fs::create_dir_all(&artist_dir).unwrap();
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let a_dir = artist_dir.clone();
        let o_dir = out_dir.clone();

        pool.writer
            .call(move |c| {
                let mut st = settings::get_settings(c, o_dir.clone())?;
                st.out_dir = o_dir.clone();
                st.on_missing = OnMissing::Ask;
                settings::set_settings(c, &st)?;

                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-507",
                        name: "Artist 507",
                        url: "https://artist507.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;

                // 25 releases under linked folder (will become Unknown when artist_dir is deleted)
                for i in 1..=25 {
                    let key = ItemKey::new(ItemType::Album, i);
                    let disc = DiscoveredRelease {
                        key,
                        band_id,
                        title: &format!("Linked Album {i}"),
                        artist_name: "Artist 507",
                        url: &format!("https://artist507.bandcamp.com/album/{i}"),
                        cover_url: None,
                        art_id: None,
                    };
                    releases::upsert_discovered(c, &disc)?;
                    lib_repo::adopt(
                        c,
                        key,
                        &a_dir.join(format!("Album {i}")),
                        Some(crate::types::AudioFormat::Flac),
                        crate::types::Origin::App,
                        None,
                        b_id,
                    )?;
                }

                // 1 missing release in out_dir (absent)
                let key_missing = ItemKey::new(ItemType::Album, 999);
                let disc_missing = DiscoveredRelease {
                    key: key_missing,
                    band_id,
                    title: "Missing Album",
                    artist_name: "Artist 507",
                    url: "https://artist507.bandcamp.com/album/missing",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc_missing)?;
                lib_repo::adopt(
                    c,
                    key_missing,
                    &o_dir.join("Missing Album"),
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;

                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        fs::remove_dir_all(&artist_dir).unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
        .await;

        let mut mass_missing_opt = None;
        while let Ok(ev) = rx.try_recv() {
            if let WorkerToUiEvent::MissingFound { mass_missing, .. } = ev {
                mass_missing_opt = Some(mass_missing);
            }
        }

        // Even though 25 releases were Unknown, mass_missing only evaluates 1 missing out of 1 checked
        // Since missing < 20, mass_missing MUST be false!
        assert_eq!(
            mass_missing_opt,
            Some(false),
            "Mass missing guard must NOT trigger for Unknown releases"
        );
    }

    #[tokio::test]
    async fn test_resolve_unavailable_actions_and_undo() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("resolve_unavail_test.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 508;
        let artist_dir = dir.path().join("Artist 508");
        fs::create_dir_all(&artist_dir).unwrap();
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let key1 = ItemKey::new(ItemType::Album, 5081);
        let key2 = ItemKey::new(ItemType::Album, 5082);

        let a_dir = artist_dir.clone();
        let o_dir = out_dir.clone();

        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-508",
                        name: "Artist 508",
                        url: "https://artist508.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;

                for &(k, name) in &[(key1, "Album 1"), (key2, "Album 2")] {
                    let disc = DiscoveredRelease {
                        key: k,
                        band_id,
                        title: name,
                        artist_name: "Artist 508",
                        url: &format!("https://artist508.bandcamp.com/album/{name}"),
                        cover_url: None,
                        art_id: None,
                    };
                    releases::upsert_discovered(c, &disc)?;
                    lib_repo::adopt(
                        c,
                        k,
                        &a_dir.join(name),
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

        fs::remove_dir_all(&artist_dir).unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
        .await;

        let mut batch_id = 0;
        while let Ok(ev) = rx.try_recv() {
            if let WorkerToUiEvent::FolderUnavailable { batch_id: b_id, .. } = ev {
                batch_id = b_id;
            }
        }
        assert!(batch_id > 0);

        // 1. Resolve key1 with Deleted { KeepMarked, unlink: false }
        let folder_id = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap()[0]
            .id;
        svc.resolve_unavailable(
            batch_id,
            vec![UnavailableDecision {
                root: UnavailableRoot::Linked { folder_id },
                keys: vec![key1],
                action: UnavailableAction::Deleted {
                    policy: DeletedPolicy::KeepMarked,
                    unlink: false,
                },
            }],
        )
        .await;

        let rel1 = pool
            .reader
            .call(move |c| releases::get_release(c, key1))
            .await
            .unwrap()
            .unwrap();
        assert!(rel1.downloaded, "KeepMarked keeps downloaded = 1");
        assert_eq!(rel1.local_status, Some(LocalStatus::Archived));

        // Folder link should still exist
        let folders = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap();
        assert!(!folders.is_empty());

        // 2. Resolve key2 with Deleted { MarkNotDownloaded, unlink: true }
        svc.resolve_unavailable(
            batch_id,
            vec![UnavailableDecision {
                root: UnavailableRoot::Linked { folder_id },
                keys: vec![key2],
                action: UnavailableAction::Deleted {
                    policy: DeletedPolicy::MarkNotDownloaded,
                    unlink: true,
                },
            }],
        )
        .await;

        let rel2 = pool
            .reader
            .call(move |c| releases::get_release(c, key2))
            .await
            .unwrap()
            .unwrap();
        assert!(!rel2.downloaded, "MarkNotDownloaded sets downloaded = 0");
        assert_eq!(rel2.local_status, None);
        assert_eq!(rel2.local_path, None);

        // Folder link should now be unlinked
        let folders_after = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap();
        assert!(folders_after.is_empty());

        // 3. Undo the batch restores the releases!
        svc.undo_batch(batch_id).await;

        let rel1_restored = pool
            .reader
            .call(move |c| releases::get_release(c, key1))
            .await
            .unwrap()
            .unwrap();
        let rel2_restored = pool
            .reader
            .call(move |c| releases::get_release(c, key2))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(rel1_restored.local_status, Some(LocalStatus::Present));
        assert!(rel2_restored.downloaded);
        assert_eq!(rel2_restored.local_status, Some(LocalStatus::Present));

        // Invariants check
        pool.reader
            .call(|c| lib_repo::assert_library_invariants(c))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_resolve_unavailable_stale_keys_skipped() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("stale_keys_test.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, _rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 509;
        let key_not_downloaded = ItemKey::new(ItemType::Album, 5091);
        let key_already_archived = ItemKey::new(ItemType::Album, 5092);

        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-509",
                        name: "Artist 509",
                        url: "https://artist509.bandcamp.com",
                        cover_url: None,
                    },
                )?;

                // 1. Release not downloaded
                let disc1 = DiscoveredRelease {
                    key: key_not_downloaded,
                    band_id,
                    title: "Not Downloaded",
                    artist_name: "Artist 509",
                    url: "https://artist509.bandcamp.com/album/1",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc1)?;

                // 2. Release already archived
                let disc2 = DiscoveredRelease {
                    key: key_already_archived,
                    band_id,
                    title: "Already Archived",
                    artist_name: "Artist 509",
                    url: "https://artist509.bandcamp.com/album/2",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc2)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key_already_archived,
                    &PathBuf::from("/some/path"),
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                lib_repo::archive(c, &[key_already_archived], b_id)?;

                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        // Resolving stale keys should not error and skip them
        svc.resolve_unavailable(
            1,
            vec![UnavailableDecision {
                root: UnavailableRoot::Linked { folder_id: 999 },
                keys: vec![key_not_downloaded, key_already_archived],
                action: UnavailableAction::Deleted {
                    policy: DeletedPolicy::KeepMarked,
                    unlink: false,
                },
            }],
        )
        .await;

        let rel1 = pool
            .reader
            .call(move |c| releases::get_release(c, key_not_downloaded))
            .await
            .unwrap()
            .unwrap();
        assert!(!rel1.downloaded);

        let rel2 = pool
            .reader
            .call(move |c| releases::get_release(c, key_already_archived))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rel2.local_status, Some(LocalStatus::Archived));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn test_unavailable_folder_unreadable_chmod_000() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("unreadable_test.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 510;
        let artist_dir = dir.path().join("Artist 510");
        fs::create_dir_all(&artist_dir).unwrap();
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&out_dir).unwrap();

        let key = ItemKey::new(ItemType::Album, 5101);
        let a_dir = artist_dir.clone();
        let o_dir = out_dir.clone();

        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-510",
                        name: "Artist 510",
                        url: "https://artist510.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &a_dir, &o_dir)?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;

                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Album 1",
                    artist_name: "Artist 510",
                    url: "https://artist510.bandcamp.com/album/1",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;

                let alb_dir = a_dir.join("Album 1");
                fs::create_dir_all(&alb_dir).unwrap();
                fs::write(alb_dir.join("01.flac"), b"audio").unwrap();

                lib_repo::adopt(
                    c,
                    key,
                    &alb_dir,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        // Make artist_dir unreadable with chmod 000
        let mut perms = fs::metadata(&artist_dir).unwrap().permissions();
        perms.set_mode(0o000);
        let _ = fs::set_permissions(&artist_dir, perms);

        let probed = crate::library::link::probe_folder(&artist_dir);
        if probed == FolderState::Unreadable {
            let cancel = CancellationToken::new();
            svc.verify_files(
                SyncScope::Selected(vec![band_id]),
                VerifyTrigger::Manual,
                cancel,
            )
            .await;

            let mut unavail_folders = None;
            while let Ok(ev) = rx.try_recv() {
                if let WorkerToUiEvent::FolderUnavailable { folders, .. } = ev {
                    unavail_folders = Some(folders);
                }
            }

            assert!(unavail_folders.is_some());
            let folders = unavail_folders.unwrap();
            assert_eq!(folders.len(), 1);
            assert_eq!(folders[0].state, FolderState::Unreadable);
            assert_eq!(folders[0].releases.len(), 1);
        }

        // Restore permissions for tempdir cleanup
        let mut restore = fs::metadata(&artist_dir).unwrap().permissions();
        restore.set_mode(0o755);
        let _ = fs::set_permissions(&artist_dir, restore);
    }

    #[tokio::test]
    async fn test_15_verify_release_path_under_folder_b_while_folder_a_deleted() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_15.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 1500;
        let folder_a = dir.path().join("FolderA");
        let folder_b = dir.path().join("FolderB");
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&folder_a).unwrap();
        fs::create_dir_all(&folder_b).unwrap();
        fs::create_dir_all(&out_dir).unwrap();

        let alb1_dir = folder_a.join("Album 1");
        fs::create_dir_all(&alb1_dir).unwrap();
        fs::write(alb1_dir.join("01.flac"), b"audio").unwrap();

        let alb2_dir = folder_b.join("Album 2");
        fs::create_dir_all(&alb2_dir).unwrap();
        fs::write(alb2_dir.join("01.flac"), b"audio").unwrap();

        let key1 = ItemKey::new(ItemType::Album, 1501);
        let key2 = ItemKey::new(ItemType::Album, 1502);

        let a_path = folder_a.clone();
        let b_path = folder_b.clone();
        let o_path = out_dir.clone();
        let alb1_path = alb1_dir.clone();
        let alb2_path = alb2_dir.clone();

        let (f_a_id, _f_b_id) = pool
            .writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-15",
                        name: "Artist 15",
                        url: "https://artist15.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let fa = lib_repo::add_folder(c, band_id, &a_path, &o_path)?;
                let fb = lib_repo::add_folder(c, band_id, &b_path, &o_path)?;

                let disc1 = DiscoveredRelease {
                    key: key1,
                    band_id,
                    title: "Album 1",
                    artist_name: "Artist 15",
                    url: "https://artist15.bandcamp.com/album/1",
                    cover_url: None,
                    art_id: None,
                };
                let disc2 = DiscoveredRelease {
                    key: key2,
                    band_id,
                    title: "Album 2",
                    artist_name: "Artist 15",
                    url: "https://artist15.bandcamp.com/album/2",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc1)?;
                releases::upsert_discovered(c, &disc2)?;

                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key1,
                    &alb1_path,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                lib_repo::adopt(
                    c,
                    key2,
                    &alb2_path,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                Ok::<_, DbError>((fa.id, fb.id))
            })
            .await
            .unwrap();

        // Delete Folder A on disk
        fs::remove_dir_all(&folder_a).unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
        .await;

        let mut folder_unavail = None;
        let mut missing_found = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                WorkerToUiEvent::FolderUnavailable { folders, .. } => {
                    folder_unavail = Some(folders);
                }
                WorkerToUiEvent::MissingFound { .. } => {
                    missing_found = true;
                }
                _ => {}
            }
        }

        // Release 2 under Folder B must remain Present and verified normally
        let rel2 = pool
            .reader
            .call(move |c| releases::get_release(c, key2))
            .await
            .unwrap()
            .unwrap();
        assert!(rel2.downloaded);
        assert_eq!(rel2.local_status, Some(LocalStatus::Present));

        // Release 1 under deleted Folder A must go to FolderUnavailable with root = Linked { folder_id: f_a_id }
        assert!(folder_unavail.is_some());
        let folders = folder_unavail.unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(
            folders[0].root,
            UnavailableRoot::Linked { folder_id: f_a_id }
        );
        assert_eq!(folders[0].releases.len(), 1);
        assert_eq!(folders[0].releases[0].key, key1);
        assert!(!missing_found);
    }

    #[tokio::test]
    async fn test_16_verify_album_moved_from_folder_a_to_folder_b_relinks_silently() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_16.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 1600;
        let folder_a = dir.path().join("FolderA");
        let folder_b = dir.path().join("FolderB");
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&folder_a).unwrap();
        fs::create_dir_all(&folder_b).unwrap();
        fs::create_dir_all(&out_dir).unwrap();

        let alb_in_a = folder_a.join("Album 1");
        fs::create_dir_all(&alb_in_a).unwrap();
        fs::write(alb_in_a.join("01.flac"), b"audio").unwrap();

        let key = ItemKey::new(ItemType::Album, 1601);

        let a_path = folder_a.clone();
        let b_path = folder_b.clone();
        let o_path = out_dir.clone();
        let alb_init_path = alb_in_a.clone();

        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-16",
                        name: "Artist 16",
                        url: "https://artist16.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &a_path, &o_path)?;
                lib_repo::add_folder(c, band_id, &b_path, &o_path)?;

                let disc = DiscoveredRelease {
                    key,
                    band_id,
                    title: "Album 1",
                    artist_name: "Artist 16",
                    url: "https://artist16.bandcamp.com/album/1",
                    cover_url: None,
                    art_id: None,
                };
                releases::upsert_discovered(c, &disc)?;

                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key,
                    &alb_init_path,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        // Move Album 1 from folder_a to folder_b
        let alb_in_b = folder_b.join("Album 1");
        fs::rename(&alb_in_a, &alb_in_b).unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(
            SyncScope::Selected(vec![band_id]),
            VerifyTrigger::Manual,
            cancel,
        )
        .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Must NOT emit MissingFound
        let has_missing_found = events
            .iter()
            .any(|e| matches!(e, WorkerToUiEvent::MissingFound { .. }));
        assert!(!has_missing_found, "Relink must be silent without prompt");

        // Release must be updated with new local_path in Folder B
        let rel = pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .unwrap()
            .unwrap();
        assert!(rel.downloaded);
        assert_eq!(rel.local_status, Some(LocalStatus::Present));
        assert_eq!(rel.local_path, Some(alb_in_b));

        pool.reader
            .call(|c| lib_repo::assert_library_invariants(c))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_17_verify_missing_album_artist_with_offline_second_folder_never_auto_resolves() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_17.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_1 = 1701;
        let band_2 = 1702;

        let dir_1a = dir.path().join("Artist1_FolderA");
        let dir_1b = dir.path().join("Artist1_FolderB");
        let dir_2 = dir.path().join("Artist2_Folder");
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&dir_1a).unwrap();
        fs::create_dir_all(&dir_1b).unwrap();
        fs::create_dir_all(&dir_2).unwrap();
        fs::create_dir_all(&out_dir).unwrap();

        let alb1_dir = dir_1a.join("Album 1");
        fs::create_dir_all(&alb1_dir).unwrap();
        fs::write(alb1_dir.join("01.flac"), b"audio").unwrap();

        let alb2_dir = dir_2.join("Album 2");
        fs::create_dir_all(&alb2_dir).unwrap();
        fs::write(alb2_dir.join("01.flac"), b"audio").unwrap();

        let key1 = ItemKey::new(ItemType::Album, 1711);
        let key2 = ItemKey::new(ItemType::Album, 1712);

        let d1a = dir_1a.clone();
        let d1b = dir_1b.clone();
        let d2 = dir_2.clone();
        let o_path = out_dir.clone();
        let a1_path = alb1_dir.clone();
        let a2_path = alb2_dir.clone();

        pool.writer
            .call(move |c| {
                let st = Settings {
                    on_missing: OnMissing::Auto,
                    deleted_policy: DeletedPolicy::KeepMarked,
                    ..Default::default()
                };
                settings::set_settings(c, &st)?;

                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id: band_1,
                        slug: "artist-1",
                        name: "Artist 1",
                        url: "https://artist1.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id: band_2,
                        slug: "artist-2",
                        name: "Artist 2",
                        url: "https://artist2.bandcamp.com",
                        cover_url: None,
                    },
                )?;

                lib_repo::add_folder(c, band_1, &d1a, &o_path)?;
                lib_repo::add_folder(c, band_1, &d1b, &o_path)?;
                lib_repo::add_folder(c, band_2, &d2, &o_path)?;

                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key: key1,
                        band_id: band_1,
                        title: "Album 1",
                        artist_name: "Artist 1",
                        url: "https://artist1.bandcamp.com/album/1",
                        cover_url: None,
                        art_id: None,
                    },
                )?;
                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key: key2,
                        band_id: band_2,
                        title: "Album 2",
                        artist_name: "Artist 2",
                        url: "https://artist2.bandcamp.com/album/2",
                        cover_url: None,
                        art_id: None,
                    },
                )?;

                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key1,
                    &a1_path,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                lib_repo::adopt(
                    c,
                    key2,
                    &a2_path,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        // 1. Delete Album 1 inside Artist 1 Folder A
        fs::remove_dir_all(&alb1_dir).unwrap();
        // 2. Delete Artist 1 Folder B completely -> so Artist 1 has an offline folder!
        fs::remove_dir_all(&dir_1b).unwrap();
        // 3. Delete Album 2 inside Artist 2 Folder -> but Artist 2 Folder itself is OK!
        fs::remove_dir_all(&alb2_dir).unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(SyncScope::All, VerifyTrigger::SyncBackground, cancel)
            .await;

        let mut missing_items = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let WorkerToUiEvent::MissingFound { items, .. } = ev {
                missing_items.extend(items);
            }
        }

        // Artist 1 release 1 must NOT be auto-resolved; it must be in MissingFound (Ask)
        assert_eq!(missing_items.len(), 1);
        assert_eq!(missing_items[0].key, key1);

        // Artist 1 release 1 status in DB is STILL Present (not auto-archived)
        let rel1 = pool
            .reader
            .call(move |c| releases::get_release(c, key1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rel1.local_status, Some(LocalStatus::Present));

        // Artist 2 release 2 was auto-resolved (auto-archived because of KeepMarked policy)
        let rel2 = pool
            .reader
            .call(move |c| releases::get_release(c, key2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rel2.local_status, Some(LocalStatus::Archived));
    }

    #[tokio::test]
    async fn test_18_worker_link_artist_folder_twice_for_one_artist() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_18.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 1800;
        let folder_a = dir.path().join("FolderA");
        let folder_b = dir.path().join("FolderB");
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&folder_a).unwrap();
        fs::create_dir_all(&folder_b).unwrap();
        fs::create_dir_all(&out_dir).unwrap();

        let alb1_dir = folder_a.join("Album One");
        fs::create_dir_all(&alb1_dir).unwrap();
        fs::write(alb1_dir.join("01.mp3"), b"audio").unwrap();

        let alb2_dir = folder_b.join("Album Two");
        fs::create_dir_all(&alb2_dir).unwrap();
        fs::write(alb2_dir.join("01.mp3"), b"audio").unwrap();

        let key1 = ItemKey::new(ItemType::Album, 1801);
        let key2 = ItemKey::new(ItemType::Album, 1802);
        let o_dir = out_dir.clone();

        pool.writer
            .call(move |c| {
                let st = Settings {
                    out_dir: o_dir,
                    ..Settings::default()
                };
                settings::set_settings(c, &st)?;
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-1800",
                        name: "Artist 1800",
                        url: "https://artist1800.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key: key1,
                        band_id,
                        title: "Album One",
                        artist_name: "Artist 1800",
                        url: "https://artist1800.bandcamp.com/album/album-one",
                        cover_url: None,
                        art_id: None,
                    },
                )?;
                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key: key2,
                        band_id,
                        title: "Album Two",
                        artist_name: "Artist 1800",
                        url: "https://artist1800.bandcamp.com/album/album-two",
                        cover_url: None,
                        art_id: None,
                    },
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();

        // 1. Link Folder A
        svc.link_artist_folder(band_id, folder_a.clone(), false, cancel.clone())
            .await;

        let mut events1 = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events1.push(ev);
        }
        let fstatus1 = events1
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::FolderStatus {
                    band_id: b,
                    folders,
                } if *b == band_id => Some(folders.clone()),
                _ => None,
            })
            .expect("FolderStatus for Folder A");
        assert_eq!(fstatus1.len(), 1);
        assert_eq!(fstatus1[0].path, dunce::canonicalize(&folder_a).unwrap());

        let report1 = events1
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::ReconcileReport { report } => Some(report.clone()),
                _ => None,
            })
            .expect("ReconcileReport for Folder A");
        assert_eq!(report1.matched.len(), 1);
        assert_eq!(report1.matched[0].release_key, key1);

        // 2. Link Folder B (second link for the same artist)
        svc.link_artist_folder(band_id, folder_b.clone(), false, cancel.clone())
            .await;

        let mut events2 = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events2.push(ev);
        }
        let fstatus2 = events2
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::FolderStatus {
                    band_id: b,
                    folders,
                } if *b == band_id => Some(folders.clone()),
                _ => None,
            })
            .expect("FolderStatus for Folder B");
        assert_eq!(fstatus2.len(), 2);

        // First-match flow runs for Folder B, matching Album Two
        let report2 = events2
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::ReconcileReport { report } => Some(report.clone()),
                _ => None,
            })
            .expect("ReconcileReport for Folder B");
        assert!(report2.matched.iter().any(|m| m.release_key == key2));

        pool.reader
            .call(|c| lib_repo::assert_library_invariants(c))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_19_worker_unlink_artist_folder_by_id() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_19.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 1900;
        let folder_a = dir.path().join("FolderA");
        let folder_b = dir.path().join("FolderB");
        let out_dir = dir.path().join("Downloads");
        fs::create_dir_all(&folder_a).unwrap();
        fs::create_dir_all(&folder_b).unwrap();
        fs::create_dir_all(&out_dir).unwrap();

        // Local-only orphans in each folder
        fs::create_dir_all(folder_a.join("Orphan A")).unwrap();
        fs::write(folder_a.join("Orphan A").join("01.mp3"), b"audio").unwrap();
        fs::create_dir_all(folder_b.join("Orphan B")).unwrap();
        fs::write(folder_b.join("Orphan B").join("01.mp3"), b"audio").unwrap();

        let a_path = folder_a.clone();
        let b_path = folder_b.clone();
        let o_dir = out_dir.clone();

        let (f_a, f_b) = pool
            .writer
            .call(move |c| {
                let st = Settings {
                    out_dir: o_dir.clone(),
                    ..Settings::default()
                };
                settings::set_settings(c, &st)?;
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-1900",
                        name: "Artist 1900",
                        url: "https://artist1900.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let fa = lib_repo::add_folder(c, band_id, &a_path, &o_dir)?;
                let fb = lib_repo::add_folder(c, band_id, &b_path, &o_dir)?;
                lib_repo::update_folder_local_only_count(c, fa.id, 1)?;
                lib_repo::update_folder_local_only_count(c, fb.id, 1)?;
                Ok::<_, DbError>((fa, fb))
            })
            .await
            .unwrap();

        // Unlink folder A
        svc.unlink_artist_folder(f_a.id).await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // FolderStatus emitted with remaining folder B
        let fstatus = events
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::FolderStatus {
                    band_id: b,
                    folders,
                } if *b == band_id => Some(folders.clone()),
                _ => None,
            })
            .expect("FolderStatus emitted");
        assert_eq!(fstatus.len(), 1);
        assert_eq!(fstatus[0].id, f_b.id);

        // Stats re-emitted: non_eligible_downloaded now 1 (only folder B's local-only count remains)
        let stats_ev = events
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::ArtistStats { band_id: b, stats } if *b == band_id => Some(*stats),
                _ => None,
            })
            .expect("ArtistStats emitted");
        assert_eq!(stats_ev.non_eligible_downloaded, 1);

        // In DB, only folder B remains
        let folders_in_db = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap();
        assert_eq!(folders_in_db.len(), 1);
        assert_eq!(folders_in_db[0].id, f_b.id);

        // Unlink folder B
        svc.unlink_artist_folder(f_b.id).await;

        let mut events2 = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events2.push(ev);
        }
        let fstatus2 = events2
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::FolderStatus {
                    band_id: b,
                    folders,
                } if *b == band_id => Some(folders.clone()),
                _ => None,
            })
            .expect("FolderStatus emitted");
        assert!(fstatus2.is_empty());

        let folders_empty = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap();
        assert!(folders_empty.is_empty());

        pool.reader
            .call(|c| lib_repo::assert_library_invariants(c))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_20_worker_repoint_artist_folder_by_id() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_20.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 2000;
        let key = ItemKey::new(ItemType::Album, 2001);

        let folder_a_old = dir.path().join("FolderAOld");
        let folder_a_new = dir.path().join("FolderANew");
        let folder_b = dir.path().join("FolderB");
        let out_dir = dir.path().join("Downloads");

        fs::create_dir_all(&folder_a_old).unwrap();
        fs::create_dir_all(&folder_a_new).unwrap();
        fs::create_dir_all(&folder_b).unwrap();
        fs::create_dir_all(&out_dir).unwrap();

        let album_in_new = folder_a_new.join("Album");
        fs::create_dir_all(&album_in_new).unwrap();
        fs::write(album_in_new.join("01.mp3"), b"audio").unwrap();

        let dummy_old_path = folder_a_old.join("Album");
        let a_old = folder_a_old.clone();
        let a_new = folder_a_new.clone();
        let b_path = folder_b.clone();
        let o_dir = out_dir.clone();

        let (fa, _fb) = pool
            .writer
            .call(move |c| {
                let st = Settings {
                    out_dir: o_dir.clone(),
                    ..Settings::default()
                };
                settings::set_settings(c, &st)?;
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-2000",
                        name: "Artist 2000",
                        url: "https://artist2000.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key,
                        band_id,
                        title: "Album",
                        artist_name: "Artist 2000",
                        url: "https://artist2000.bandcamp.com/album/album",
                        cover_url: None,
                        art_id: None,
                    },
                )?;
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
                let fa = lib_repo::add_folder(c, band_id, &a_old, &o_dir)?;
                let fb = lib_repo::add_folder(c, band_id, &b_path, &o_dir)?;
                Ok::<_, DbError>((fa, fb))
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.repoint_artist_folder(fa.id, a_new.clone(), cancel)
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // FolderStatus carries both folders with updated folder A
        let fstatus = events
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::FolderStatus {
                    band_id: b,
                    folders,
                } if *b == band_id => Some(folders.clone()),
                _ => None,
            })
            .expect("FolderStatus emitted");
        assert_eq!(fstatus.len(), 2);
        let updated_fa = fstatus.iter().find(|f| f.id == fa.id).unwrap();
        assert_eq!(updated_fa.path, dunce::canonicalize(&folder_a_new).unwrap());

        // Verify ran: release is relinked to album_in_new
        let row = pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.local_status, Some(LocalStatus::Present));
        assert_eq!(row.local_path, Some(album_in_new));

        pool.reader
            .call(|c| lib_repo::assert_library_invariants(c))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_21_worker_resolve_unavailable_unlinks_only_target_folder() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_21.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let svc = LibraryService::new(pool.clone(), bridge);

        let band_id = 2100;
        let key1 = ItemKey::new(ItemType::Album, 2101);
        let key2 = ItemKey::new(ItemType::Album, 2102);

        let folder_a = dir.path().join("FolderA");
        let folder_b = dir.path().join("FolderB");
        let out_dir = dir.path().join("Downloads");

        fs::create_dir_all(&folder_a).unwrap();
        fs::create_dir_all(&folder_b).unwrap();
        fs::create_dir_all(&out_dir).unwrap();

        let alb1_dir = folder_a.join("Album 1");
        fs::create_dir_all(&alb1_dir).unwrap();
        fs::write(alb1_dir.join("01.flac"), b"audio").unwrap();

        let alb2_dir = folder_b.join("Album 2");
        fs::create_dir_all(&alb2_dir).unwrap();
        fs::write(alb2_dir.join("01.flac"), b"audio").unwrap();

        let a_path = folder_a.clone();
        let b_path = folder_b.clone();
        let o_dir = out_dir.clone();
        let a1_path = alb1_dir.clone();
        let a2_path = alb2_dir.clone();

        let (fa, fb) = pool
            .writer
            .call(move |c| {
                let st = Settings {
                    out_dir: o_dir.clone(),
                    ..Settings::default()
                };
                settings::set_settings(c, &st)?;
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-2100",
                        name: "Artist 2100",
                        url: "https://artist2100.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key: key1,
                        band_id,
                        title: "Album 1",
                        artist_name: "Artist 2100",
                        url: "https://artist2100.bandcamp.com/album/album-1",
                        cover_url: None,
                        art_id: None,
                    },
                )?;
                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key: key2,
                        band_id,
                        title: "Album 2",
                        artist_name: "Artist 2100",
                        url: "https://artist2100.bandcamp.com/album/album-2",
                        cover_url: None,
                        art_id: None,
                    },
                )?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key1,
                    &a1_path,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                lib_repo::adopt(
                    c,
                    key2,
                    &a2_path,
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                let fa = lib_repo::add_folder(c, band_id, &a_path, &o_dir)?;
                let fb = lib_repo::add_folder(c, band_id, &b_path, &o_dir)?;
                Ok::<_, DbError>((fa, fb))
            })
            .await
            .unwrap();

        // Delete Folder A on disk
        fs::remove_dir_all(&folder_a).unwrap();

        let cancel = CancellationToken::new();
        svc.verify_files(SyncScope::All, VerifyTrigger::Manual, cancel)
            .await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        let unavail = events
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::FolderUnavailable {
                    batch_id, folders, ..
                } => Some((*batch_id, folders.clone())),
                _ => None,
            })
            .expect("FolderUnavailable emitted");

        let batch_id = unavail.0;
        assert_eq!(unavail.1.len(), 1);
        assert_eq!(
            unavail.1[0].root,
            UnavailableRoot::Linked { folder_id: fa.id }
        );

        // Resolve by unlinking Folder A
        svc.resolve_unavailable(
            batch_id,
            vec![UnavailableDecision {
                root: UnavailableRoot::Linked { folder_id: fa.id },
                keys: vec![key1],
                action: UnavailableAction::Deleted {
                    policy: DeletedPolicy::KeepMarked,
                    unlink: true,
                },
            }],
        )
        .await;

        let mut events2 = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events2.push(ev);
        }

        // FolderStatus has folder B
        let fstatus = events2
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::FolderStatus {
                    band_id: b,
                    folders,
                } if *b == band_id => Some(folders.clone()),
                _ => None,
            })
            .expect("FolderStatus emitted");
        assert_eq!(fstatus.len(), 1);
        assert_eq!(fstatus[0].id, fb.id);

        // In DB, only folder B remains
        let remaining = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, fb.id);

        pool.reader
            .call(|c| lib_repo::assert_library_invariants(c))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_link_rejects_download_dir_before_settings_are_saved() {
        let dir = tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let db_path = root.join("default_out_dir.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, _rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        // No settings row is saved: the service must guard the real default download
        // directory, not a made-up relative one.
        let out_dir = root.join("bcvault");
        let downloaded_artist = out_dir.join("Some Artist");
        fs::create_dir_all(&downloaded_artist).unwrap();
        let svc = LibraryService::new(pool.clone(), bridge).with_default_out_dir(out_dir.clone());

        let band_id = 77;
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "some-artist",
                        name: "Some Artist",
                        url: "https://some-artist.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        svc.link_artist_folder(band_id, out_dir.clone(), true, cancel.clone())
            .await;
        svc.link_artist_folder(band_id, downloaded_artist, true, cancel)
            .await;

        let folders = pool
            .reader
            .call(move |c| lib_repo::get_folders(c, band_id))
            .await
            .unwrap();
        assert!(
            folders.is_empty(),
            "the download directory must never be linked, got {folders:?}"
        );
    }

    #[tokio::test]
    async fn test_verify_relinks_downloads_moved_from_deleted_out_dir_to_linked_folder() {
        let dir = tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let db_path = root.join("move_from_deleted_out_dir.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let out_dir = root.join("Downloads");
        let linked_dir = root.join("LibraryArtist");
        fs::create_dir_all(&out_dir).unwrap();
        fs::create_dir_all(&linked_dir).unwrap();

        // 5 pre-existing local-only albums in linked_dir (not on Bandcamp)
        for i in 1..=5 {
            let orphan = linked_dir.join(format!("Offpage Local {i}"));
            fs::create_dir_all(&orphan).unwrap();
            fs::write(orphan.join("01.flac"), b"audio").unwrap();
        }

        let svc = LibraryService::new(pool.clone(), bridge).with_default_out_dir(out_dir.clone());
        let band_id = 8800;
        let o_dir = out_dir.clone();
        let l_dir = linked_dir.clone();

        pool.writer
            .call(move |c| {
                let st = Settings {
                    out_dir: o_dir.clone(),
                    ..Settings::default()
                };
                settings::set_settings(c, &st)?;
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-8800",
                        name: "Artist 8800",
                        url: "https://artist8800.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                let af = lib_repo::add_folder(c, band_id, &l_dir, &o_dir)?;
                lib_repo::update_folder_local_only_count(c, af.id, 5)?;

                // 6 releases on Bandcamp, downloaded by the app into out_dir
                for i in 1..=6 {
                    let key = ItemKey::new(ItemType::Album, 8800 + i);
                    let title = format!("Page Release {i}");
                    let url = format!("https://artist8800.bandcamp.com/album/page-release-{i}");
                    releases::upsert_discovered(
                        c,
                        &DiscoveredRelease {
                            key,
                            band_id,
                            title: &title,
                            artist_name: "Artist 8800",
                            url: &url,
                            cover_url: None,
                            art_id: None,
                        },
                    )?;
                    releases::set_classification(c, key, crate::types::Classification::Free)?;
                    let dl_path = o_dir.join(&title);
                    fs::create_dir_all(&dl_path).unwrap();
                    fs::write(dl_path.join("01.flac"), b"audio").unwrap();
                    let hist_id = crate::db::repo::history::insert_started(
                        c,
                        &crate::db::repo::history::NewHistoryEntry {
                            key,
                            band_id,
                            format: crate::types::AudioFormat::Flac,
                            source: crate::types::DownloadSource::Api,
                            counts_against_quota: false,
                        },
                    )?;
                    releases::commit_download(
                        c,
                        key,
                        crate::types::AudioFormat::Flac,
                        &dl_path,
                        1024,
                        hist_id,
                    )?;
                }
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        // User moves all 6 downloaded folders from out_dir into linked_dir and removes out_dir
        for i in 1..=6 {
            let title = format!("Page Release {i}");
            fs::rename(out_dir.join(&title), linked_dir.join(&title)).unwrap();
        }
        fs::remove_dir_all(&out_dir).unwrap();

        let cancel = CancellationToken::new();
        let outcome = svc
            .verify_files(SyncScope::All, VerifyTrigger::Manual, cancel)
            .await;
        assert_eq!(outcome.relinked_count, 6);
        assert_eq!(outcome.missing_count, 0);

        let mut has_unavail = false;
        let mut has_missing = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, WorkerToUiEvent::FolderUnavailable { .. }) {
                has_unavail = true;
            }
            if matches!(ev, WorkerToUiEvent::MissingFound { .. }) {
                has_missing = true;
            }
        }
        assert!(
            !has_unavail,
            "Must NOT prompt FolderUnavailable when moved releases are relinked into linked_dir"
        );
        assert!(!has_missing, "Must NOT prompt MissingFound");

        for i in 1..=6 {
            let key = ItemKey::new(ItemType::Album, 8800 + i);
            let title = format!("Page Release {i}");
            let row = pool
                .reader
                .call(move |c| releases::get_release(c, key))
                .await
                .unwrap()
                .unwrap();
            assert!(row.downloaded);
            assert_eq!(row.local_status, Some(LocalStatus::Present));
            assert_eq!(row.local_path, Some(linked_dir.join(title)));
        }
    }

    #[tokio::test]
    async fn test_verify_adopts_and_restores_releases_marked_deleted_after_move_to_linked_folder() {
        let dir = tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let db_path = root.join("deleted_then_verify.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, _rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let out_dir = root.join("Downloads");
        let linked_dir = root.join("LibraryArtist");
        fs::create_dir_all(&linked_dir).unwrap();
        // out_dir does not exist

        let svc = LibraryService::new(pool.clone(), bridge).with_default_out_dir(out_dir.clone());
        let band_id = 8900;
        let key_unmarked = ItemKey::new(ItemType::Album, 8901);
        let key_archived = ItemKey::new(ItemType::Album, 8902);
        let o_dir = out_dir.clone();
        let l_dir = linked_dir.clone();

        // Both albums are placed in the linked folder
        for title in ["Album Unmarked", "Album Archived"] {
            let p = linked_dir.join(title);
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("01.flac"), b"audio").unwrap();
        }

        pool.writer
            .call(move |c| {
                let st = Settings {
                    out_dir: o_dir.clone(),
                    ..Settings::default()
                };
                settings::set_settings(c, &st)?;
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-8900",
                        name: "Artist 8900",
                        url: "https://artist8900.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &l_dir, &o_dir)?;

                // Release 1: was marked as not downloaded (downloaded = 0)
                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key: key_unmarked,
                        band_id,
                        title: "Album Unmarked",
                        artist_name: "Artist 8900",
                        url: "https://artist8900.bandcamp.com/album/album-unmarked",
                        cover_url: None,
                        art_id: None,
                    },
                )?;

                // Release 2: was marked as archived (KeepMarked) pointing at deleted out_dir
                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key: key_archived,
                        band_id,
                        title: "Album Archived",
                        artist_name: "Artist 8900",
                        url: "https://artist8900.bandcamp.com/album/album-archived",
                        cover_url: None,
                        art_id: None,
                    },
                )?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key_archived,
                    &o_dir.join("Album Archived"),
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                lib_repo::archive(c, &[key_archived], b_id)?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        // Pressing Verify files (Manual) finds both albums in linked_dir and adopts/restores them!
        let cancel = CancellationToken::new();
        let outcome = svc
            .verify_files(SyncScope::All, VerifyTrigger::Manual, cancel)
            .await;
        assert_eq!(outcome.relinked_count, 2);

        let r1 = pool
            .reader
            .call(move |c| releases::get_release(c, key_unmarked))
            .await
            .unwrap()
            .unwrap();
        assert!(r1.downloaded);
        assert_eq!(r1.local_status, Some(LocalStatus::Present));
        assert_eq!(r1.local_path, Some(linked_dir.join("Album Unmarked")));

        let r2 = pool
            .reader
            .call(move |c| releases::get_release(c, key_archived))
            .await
            .unwrap()
            .unwrap();
        assert!(r2.downloaded);
        assert_eq!(r2.local_status, Some(LocalStatus::Present));
        assert_eq!(r2.local_path, Some(linked_dir.join("Album Archived")));
    }

    #[tokio::test]
    async fn test_resolve_unavailable_moved_to_linked_scans_and_adopts() {
        let dir = tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let db_path = root.join("moved_to_linked_action.db");
        {
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            migrations::run(&mut conn).unwrap();
        }

        let pool = DbPool::open(&db_path).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let bridge = UiBridge::new(tx, None);

        let out_dir = root.join("Downloads");
        let linked_dir = root.join("LibraryArtist");
        fs::create_dir_all(&linked_dir).unwrap();

        let moved_album = linked_dir.join("Moved Album");
        fs::create_dir_all(&moved_album).unwrap();
        fs::write(moved_album.join("01.flac"), b"audio").unwrap();

        let svc = LibraryService::new(pool.clone(), bridge).with_default_out_dir(out_dir.clone());
        let band_id = 9000;
        let key = ItemKey::new(ItemType::Album, 9001);
        let o_dir = out_dir.clone();
        let l_dir = linked_dir.clone();

        pool.writer
            .call(move |c| {
                let st = Settings {
                    out_dir: o_dir.clone(),
                    auto_relink: false,
                    ..Settings::default()
                };
                settings::set_settings(c, &st)?;
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: "artist-9000",
                        name: "Artist 9000",
                        url: "https://artist9000.bandcamp.com",
                        cover_url: None,
                    },
                )?;
                lib_repo::add_folder(c, band_id, &l_dir, &o_dir)?;
                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key,
                        band_id,
                        title: "Moved Album",
                        artist_name: "Artist 9000",
                        url: "https://artist9000.bandcamp.com/album/moved-album",
                        cover_url: None,
                        art_id: None,
                    },
                )?;
                let b_id = lib_repo::create_batch(c, "test", &BatchSummary::default())?;
                lib_repo::adopt(
                    c,
                    key,
                    &o_dir.join("Moved Album"),
                    Some(crate::types::AudioFormat::Flac),
                    crate::types::Origin::App,
                    None,
                    b_id,
                )?;
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();

        svc.resolve_unavailable(
            1,
            vec![UnavailableDecision {
                root: UnavailableRoot::OutDir,
                keys: vec![key],
                action: UnavailableAction::MovedToLinked,
            }],
        )
        .await;

        while rx.try_recv().is_ok() {}

        let row = pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .unwrap()
            .unwrap();
        assert!(row.downloaded);
        assert_eq!(row.local_status, Some(LocalStatus::Present));
        assert_eq!(row.local_path, Some(moved_album));
    }
}

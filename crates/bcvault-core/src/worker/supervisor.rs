use crate::{
    bandcamp::BandcampClient,
    db::{
        DbPool,
        repo::{artists, groups as groups_repo, history, library as lib_repo, releases, settings},
    },
    error::DbError,
    paths::AppPaths,
    types::{FolderState, LogLevel, SyncScope, VerifyTrigger},
    worker::{
        bridge::UiBridge,
        downloads::DownloadManager,
        library::LibraryService,
        messages::{UiToWorkerCommand, WorkerToUiEvent},
        stream::sweep_part_files,
        sync::SyncService,
        thumbs::ThumbService,
    },
};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;

pub struct Supervisor {
    pool: DbPool,
    bridge: UiBridge,
    sync_service: SyncService,
    dl_manager: DownloadManager,
    thumb_service: ThumbService,
    library_service: LibraryService,
    current_out_dir: std::path::PathBuf,
    root_cancel: CancellationToken,
    sync_cancel: CancellationToken,
    library_cancel: CancellationToken,
}

impl Supervisor {
    pub async fn run(
        paths: AppPaths,
        pool: DbPool,
        client: BandcampClient,
        bridge: UiBridge,
        mut cmd_rx: UnboundedReceiver<UiToWorkerCommand>,
    ) {
        let default_dl_dir = paths.default_download_dir.clone();
        let boot_res = pool
            .writer
            .call(move |c| {
                let _ = history::recover_interrupted(c)?;
                let st = settings::get_settings(c, default_dl_dir)?;
                let avs = artists::list_views(c)?;
                Ok::<_, DbError>((st, avs))
            })
            .await;

        let (initial_settings, initial_artists) = match boot_res {
            Ok(pair) => pair,
            Err(e) => {
                bridge.log(
                    LogLevel::Error,
                    "boot",
                    format!("Database initialization failed: {e}"),
                    None,
                );
                return;
            }
        };

        sweep_part_files(&initial_settings.out_dir);
        client.gate.update_config(
            initial_settings.page_delay_ms,
            initial_settings.page_jitter_pct,
        );

        let root_cancel = CancellationToken::new();
        let sync_cancel = root_cancel.child_token();
        let library_cancel = root_cancel.child_token();

        let library_service = LibraryService::new(pool.clone(), bridge.clone())
            .with_default_out_dir(paths.default_download_dir.clone());
        let sync_service = SyncService::new(pool.clone(), client.clone(), bridge.clone())
            .with_library(library_service.clone());
        let dl_manager = DownloadManager::new(
            pool.clone(),
            client.clone(),
            bridge.clone(),
            initial_settings.clone(),
            root_cancel.clone(),
        );
        let thumb_service = ThumbService::new(
            pool.clone(),
            client,
            bridge.clone(),
            paths.cover_cache_dir.clone(),
        );

        let initial_folders = pool
            .reader
            .call(|c| lib_repo::all_folders(c))
            .await
            .unwrap_or_default();

        let (initial_grouping, initial_sets, initial_groups, initial_pins) = pool
            .reader
            .call(load_group_snapshot)
            .await
            .unwrap_or_default();

        let current_out_dir = initial_settings.out_dir.clone();

        bridge.emit(WorkerToUiEvent::Ready {
            artists: initial_artists,
            settings: initial_settings,
            folders: initial_folders.clone(),
            active_grouping: initial_grouping,
            group_sets: initial_sets,
            groups: initial_groups,
            pins: initial_pins,
        });

        if !initial_folders.is_empty() {
            let lib = library_service.clone();
            let tok = library_cancel.child_token();
            let folders = initial_folders;
            tokio::spawn(async move {
                for af in folders {
                    if af.state == FolderState::Ok && !tok.is_cancelled() {
                        lib.rescan_and_adopt_on_sync(af.band_id, tok.clone()).await;
                    }
                }
            });
        }

        let mut sup = Self {
            pool,
            bridge,
            sync_service,
            dl_manager,
            thumb_service,
            library_service,
            current_out_dir,
            root_cancel,
            sync_cancel,
            library_cancel,
        };

        while let Some(cmd) = cmd_rx.recv().await {
            if matches!(cmd, UiToWorkerCommand::Shutdown) {
                sup.root_cancel.cancel();
                break;
            }
            sup.handle_command(cmd).await;
        }
    }

    async fn handle_command(&mut self, cmd: UiToWorkerCommand) {
        match cmd {
            UiToWorkerCommand::AddArtist { input } => {
                let svc = self.sync_service.clone();
                let bridge = self.bridge.clone();
                let tok = self.sync_cancel.child_token();
                tokio::spawn(async move {
                    if let Err(e) = svc.add_and_sync_artist(&input, tok).await {
                        bridge.log(
                            LogLevel::Error,
                            "sync",
                            format!("Add artist '{input}' failed: {e}"),
                            None,
                        );
                        bridge.emit(WorkerToUiEvent::SyncFinished {
                            band_id: 0,
                            new_items: 0,
                            updated: 0,
                            error: Some(e.to_string()),
                            adopted: 0,
                            missing_local: 0,
                            relinked: 0,
                        });
                    }
                });
            }
            UiToWorkerCommand::RemoveArtist { band_id } => {
                let pool = self.pool.clone();
                let bridge = self.bridge.clone();
                tokio::spawn(async move {
                    let res = pool
                        .writer
                        .call(move |c| {
                            artists::delete(c, band_id)?;
                            load_group_snapshot(c)
                        })
                        .await;
                    match res {
                        Ok((active_grouping, group_sets, groups, pins)) => {
                            bridge.emit(WorkerToUiEvent::ArtistRemoved { band_id });
                            bridge.emit(WorkerToUiEvent::ArtistGroupsChanged {
                                active_grouping,
                                group_sets,
                                groups,
                                pins,
                            });
                        }
                        Err(e) => {
                            bridge.log(
                                LogLevel::Error,
                                "db",
                                format!("Failed to remove artist {band_id}: {e}"),
                                None,
                            );
                        }
                    }
                });
            }
            UiToWorkerCommand::RenameArtist {
                band_id,
                custom_name,
            } => {
                let pool = self.pool.clone();
                let bridge = self.bridge.clone();
                tokio::spawn(async move {
                    let res = pool
                        .writer
                        .call(move |c| {
                            artists::set_custom_name(c, band_id, custom_name.as_deref())?;
                            artists::get_view(c, band_id)
                        })
                        .await;
                    match res {
                        Ok(Some(av)) => {
                            bridge.emit(WorkerToUiEvent::ArtistUpserted(av));
                        }
                        Ok(None) => {}
                        Err(e) => {
                            bridge.log(
                                LogLevel::Error,
                                "db",
                                format!("Failed to rename artist {band_id}: {e}"),
                                None,
                            );
                        }
                    }
                });
            }
            UiToWorkerCommand::Sync { scope, mode } => {
                let pool = self.pool.clone();
                let svc = self.sync_service.clone();
                let bridge = self.bridge.clone();
                let tok = self.sync_cancel.child_token();
                tokio::spawn(async move {
                    let (ids, trigger) = match scope {
                        SyncScope::Selected(v) => (v, VerifyTrigger::SyncInteractive),
                        SyncScope::All => {
                            let list = pool
                                .reader
                                .call(|c| {
                                    let views = artists::list_views(c)?;
                                    Ok::<_, DbError>(views.into_iter().map(|v| v.band_id).collect())
                                })
                                .await
                                .unwrap_or_default();
                            (list, VerifyTrigger::SyncBackground)
                        }
                    };

                    for band_id in ids {
                        if tok.is_cancelled() {
                            break;
                        }
                        if let Err(e) = svc
                            .sync_artist_with_trigger(band_id, mode, tok.clone(), trigger)
                            .await
                        {
                            bridge.log(
                                LogLevel::Error,
                                "sync",
                                format!("Sync artist {band_id} failed: {e}"),
                                None,
                            );
                            bridge.emit(WorkerToUiEvent::SyncFinished {
                                band_id,
                                new_items: 0,
                                updated: 0,
                                error: Some(e.to_string()),
                                adopted: 0,
                                missing_local: 0,
                                relinked: 0,
                            });
                        }
                    }
                });
            }
            UiToWorkerCommand::CancelSync => {
                self.sync_cancel.cancel();
                self.sync_cancel = self.root_cancel.child_token();
            }
            UiToWorkerCommand::QueryReleases { request_id, query } => {
                let pool = self.pool.clone();
                let bridge = self.bridge.clone();
                tokio::spawn(async move {
                    match pool
                        .reader
                        .call(move |c| releases::query_releases(c, &query))
                        .await
                    {
                        Ok(rows) => {
                            bridge.emit(WorkerToUiEvent::ReleasesLoaded { request_id, rows });
                        }
                        Err(e) => {
                            bridge.log(
                                LogLevel::Error,
                                "db",
                                format!("QueryReleases failed: {e}"),
                                None,
                            );
                        }
                    }
                });
            }
            UiToWorkerCommand::RequestThumbs { keys } => {
                self.thumb_service.request_thumbs(keys);
            }
            UiToWorkerCommand::EnqueueDownloads {
                items,
                opts,
                ack_soft_quota,
                mode,
            } => {
                let mgr = self.dl_manager.clone();
                tokio::spawn(async move {
                    mgr.enqueue_downloads(items, opts, ack_soft_quota, mode)
                        .await;
                });
            }
            UiToWorkerCommand::EnqueueRemaining {
                scope,
                opts,
                ack_soft_quota,
            } => {
                let mgr = self.dl_manager.clone();
                tokio::spawn(async move {
                    mgr.enqueue_remaining(scope, opts, ack_soft_quota).await;
                });
            }
            UiToWorkerCommand::RedeemUrl { target, url, opts } => {
                let mgr = self.dl_manager.clone();
                tokio::spawn(async move {
                    mgr.enqueue_redeem(target, url, opts).await;
                });
            }
            UiToWorkerCommand::SetReleaseDownloaded { key, downloaded } => {
                let pool = self.pool.clone();
                let bridge = self.bridge.clone();
                tokio::spawn(async move {
                    let res = pool
                        .writer
                        .call(move |c| {
                            releases::set_downloaded_manual(c, key, downloaded)?;
                            let row = releases::get_release(c, key)?;
                            let stats = match row.as_ref() {
                                Some(r) => {
                                    Some((r.band_id, artists::stats_with_quota(c, r.band_id)?))
                                }
                                None => None,
                            };
                            Ok::<_, DbError>((row, stats))
                        })
                        .await;
                    match res {
                        Ok((row_opt, stats_opt)) => {
                            if let Some(r) = row_opt {
                                bridge.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(r)));
                            }
                            if let Some((band_id, stats)) = stats_opt {
                                bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
                            }
                        }
                        Err(e) => {
                            bridge.log(
                                LogLevel::Error,
                                "db",
                                format!("Failed to update downloaded status for {key:?}: {e}"),
                                Some(key),
                            );
                        }
                    }
                });
            }
            UiToWorkerCommand::SetReleaseUnavailable { keys, unavailable } => {
                let pool = self.pool.clone();
                let bridge = self.bridge.clone();
                tokio::spawn(async move {
                    let res = pool
                        .writer
                        .call(move |c| {
                            releases::set_unavailable_manual(c, &keys, unavailable)?;
                            let mut updated = Vec::new();
                            let mut bands = std::collections::HashSet::new();
                            for &key in &keys {
                                if let Some(r) = releases::get_release(c, key)? {
                                    bands.insert(r.band_id);
                                    updated.push(r);
                                }
                            }
                            let mut stats_list = Vec::new();
                            for band_id in bands {
                                stats_list.push((band_id, artists::stats_with_quota(c, band_id)?));
                            }
                            Ok::<_, DbError>((updated, stats_list))
                        })
                        .await;
                    match res {
                        Ok((rows, stats_list)) => {
                            for r in rows {
                                bridge.emit(WorkerToUiEvent::ReleaseUpdated(Box::new(r)));
                            }
                            for (band_id, stats) in stats_list {
                                bridge.emit(WorkerToUiEvent::ArtistStats { band_id, stats });
                            }
                        }
                        Err(e) => {
                            bridge.log(
                                LogLevel::Error,
                                "db",
                                format!("Failed to update unavailable status: {e}"),
                                None,
                            );
                        }
                    }
                });
            }
            UiToWorkerCommand::CancelJob(job_id) => {
                self.dl_manager.cancel_job(job_id);
            }
            UiToWorkerCommand::RetryJob(job_id) => {
                self.dl_manager.retry_job(job_id);
            }
            UiToWorkerCommand::CancelAllJobs => {
                self.dl_manager.cancel_all();
            }
            UiToWorkerCommand::PauseQueue => {
                self.dl_manager.pause();
            }
            UiToWorkerCommand::ResumeQueue => {
                self.dl_manager.resume();
            }
            UiToWorkerCommand::ClearFinished => {}
            UiToWorkerCommand::LinkArtistFolder {
                band_id,
                path,
                auto_apply_confident,
            } => {
                let svc = self.library_service.clone();
                let tok = self.library_cancel.child_token();
                tokio::spawn(async move {
                    svc.link_artist_folder(band_id, path, auto_apply_confident, tok)
                        .await;
                });
            }
            UiToWorkerCommand::AddAndLinkArtistFolder {
                input,
                path,
                auto_apply_confident,
            } => {
                let sync_svc = self.sync_service.clone();
                let lib_svc = self.library_service.clone();
                let bridge = self.bridge.clone();
                let sync_tok = self.sync_cancel.child_token();
                let lib_tok = self.library_cancel.child_token();
                tokio::spawn(async move {
                    match sync_svc.add_and_sync_artist(&input, sync_tok).await {
                        Ok(outcome) => {
                            lib_svc
                                .link_artist_folder(
                                    outcome.band_id,
                                    path,
                                    auto_apply_confident,
                                    lib_tok.clone(),
                                )
                                .await;
                            lib_svc.on_artist_synced(outcome.band_id, lib_tok).await;
                        }
                        Err(e) => {
                            bridge.log(
                                LogLevel::Error,
                                "library",
                                format!("Failed to add artist '{input}' for folder link: {e}"),
                                None,
                            );
                            bridge.emit(WorkerToUiEvent::SyncFinished {
                                band_id: 0,
                                new_items: 0,
                                updated: 0,
                                error: Some(e.to_string()),
                                adopted: 0,
                                missing_local: 0,
                                relinked: 0,
                            });
                        }
                    }
                });
            }
            UiToWorkerCommand::UnlinkArtistFolder { folder_id } => {
                let svc = self.library_service.clone();
                tokio::spawn(async move {
                    svc.unlink_artist_folder(folder_id).await;
                });
            }
            UiToWorkerCommand::MatchArtistFolder {
                band_id,
                auto_apply_confident,
            } => {
                let svc = self.library_service.clone();
                let tok = self.library_cancel.child_token();
                tokio::spawn(async move {
                    svc.match_artist_folder(band_id, auto_apply_confident, tok)
                        .await;
                });
            }
            UiToWorkerCommand::ApplyReconcile {
                batch_id,
                decisions,
            } => {
                let svc = self.library_service.clone();
                tokio::spawn(async move {
                    svc.apply_reconcile(batch_id, decisions).await;
                });
            }
            UiToWorkerCommand::UndoBatch { batch_id } => {
                let svc = self.library_service.clone();
                tokio::spawn(async move {
                    svc.undo_batch(batch_id).await;
                });
            }
            UiToWorkerCommand::VerifyFiles { scope } => {
                let svc = self.library_service.clone();
                let tok = self.library_cancel.child_token();
                tokio::spawn(async move {
                    svc.verify_files(scope, VerifyTrigger::Manual, tok).await;
                });
            }
            UiToWorkerCommand::ResolveMissing {
                batch_id,
                decisions,
                remember,
            } => {
                let svc = self.library_service.clone();
                tokio::spawn(async move {
                    svc.resolve_missing(batch_id, decisions, remember).await;
                });
            }
            UiToWorkerCommand::ResolveUnavailable {
                batch_id,
                decisions,
            } => {
                let svc = self.library_service.clone();
                tokio::spawn(async move {
                    svc.resolve_unavailable(batch_id, decisions).await;
                });
            }
            UiToWorkerCommand::RelinkRelease { key, path } => {
                let svc = self.library_service.clone();
                tokio::spawn(async move {
                    svc.relink_release(key, path).await;
                });
            }
            UiToWorkerCommand::MarkArchived { keys } => {
                let svc = self.library_service.clone();
                tokio::spawn(async move {
                    svc.mark_archived(keys).await;
                });
            }
            UiToWorkerCommand::UnmarkDownloaded { keys } => {
                let svc = self.library_service.clone();
                tokio::spawn(async move {
                    svc.unmark_downloaded(keys).await;
                });
            }
            UiToWorkerCommand::IgnoreLocal { band_id, paths } => {
                let svc = self.library_service.clone();
                tokio::spawn(async move {
                    svc.ignore_local(band_id, paths).await;
                });
            }
            UiToWorkerCommand::RepointArtistFolder {
                folder_id,
                new_path,
            } => {
                let svc = self.library_service.clone();
                let tok = self.library_cancel.child_token();
                tokio::spawn(async move {
                    svc.repoint_artist_folder(folder_id, new_path, tok).await;
                });
            }
            UiToWorkerCommand::CancelLibrary => {
                self.library_service.cancel();
                self.library_cancel.cancel();
                self.library_cancel = self.root_cancel.child_token();
            }
            UiToWorkerCommand::SetActiveGrouping { grouping } => {
                self.run_group_write(move |c| groups_repo::set_active_grouping(c, grouping));
            }
            UiToWorkerCommand::CreateArtistGroupSet { name } => {
                self.run_group_write(move |c| {
                    let set = groups_repo::create_group_set(c, &name)?;
                    groups_repo::set_active_grouping(
                        c,
                        crate::groups::ActiveGrouping::CustomSet(set.id),
                    )?;
                    Ok(())
                });
            }
            UiToWorkerCommand::RenameArtistGroupSet { id, name } => {
                self.run_group_write(move |c| groups_repo::rename_group_set(c, id, &name));
            }
            UiToWorkerCommand::DeleteArtistGroupSet { id } => {
                self.run_group_write(move |c| groups_repo::delete_group_set(c, id));
            }
            UiToWorkerCommand::CreateArtistGroup {
                set_id,
                name,
                member_ids,
            } => {
                self.run_group_write(move |c| {
                    let target_set_id = if let Some(sid) = set_id {
                        sid
                    } else if let crate::groups::ActiveGrouping::CustomSet(sid) =
                        groups_repo::get_active_grouping(c)?
                    {
                        sid
                    } else if let Some(first) = groups_repo::list_group_sets(c)?.first() {
                        groups_repo::set_active_grouping(
                            c,
                            crate::groups::ActiveGrouping::CustomSet(first.id),
                        )?;
                        first.id
                    } else {
                        let created = groups_repo::create_group_set(c, "Custom Groups")?;
                        groups_repo::set_active_grouping(
                            c,
                            crate::groups::ActiveGrouping::CustomSet(created.id),
                        )?;
                        created.id
                    };
                    let g = groups_repo::create_group(c, target_set_id, &name)?;
                    if !member_ids.is_empty() {
                        groups_repo::set_pins(
                            c,
                            target_set_id,
                            &member_ids,
                            crate::groups::PinTarget::Group(g.id),
                        )?;
                    }
                    Ok(())
                });
            }
            UiToWorkerCommand::UpdateArtistGroup {
                id,
                name,
                member_ids,
            } => {
                self.run_group_write(move |c| {
                    let set_id = groups_repo::update_group(c, id, &name)?;
                    groups_repo::replace_group_members(c, set_id, id, &member_ids)?;
                    Ok(())
                });
            }
            UiToWorkerCommand::DeleteArtistGroup { id } => {
                self.run_group_write(move |c| groups_repo::delete_group(c, id));
            }
            UiToWorkerCommand::ReorderArtistGroups { set_id, ids } => {
                self.run_group_write(move |c| groups_repo::reorder_groups(c, set_id, &ids));
            }
            UiToWorkerCommand::SetArtistGroupCollapsed { id, collapsed } => {
                self.run_group_write(move |c| groups_repo::set_collapsed(c, id, collapsed));
            }
            UiToWorkerCommand::PinArtists {
                set_id,
                band_ids,
                target,
            } => {
                self.run_group_write(move |c| groups_repo::set_pins(c, set_id, &band_ids, target));
            }
            UiToWorkerCommand::UpdateSettings(mut new_settings) => {
                new_settings.sanitize();
                if new_settings.out_dir.to_string_lossy().trim().is_empty() {
                    new_settings.out_dir = self.current_out_dir.clone();
                } else {
                    self.current_out_dir = new_settings.out_dir.clone();
                }
                self.sync_service
                    .client
                    .gate
                    .update_config(new_settings.page_delay_ms, new_settings.page_jitter_pct);
                self.dl_manager.update_settings(new_settings.clone());

                let pool = self.pool.clone();
                let bridge = self.bridge.clone();
                tokio::spawn(async move {
                    let s_for_db = new_settings.clone();
                    match pool
                        .writer
                        .call(move |c| settings::set_settings(c, &s_for_db))
                        .await
                    {
                        Ok(()) => {
                            bridge.emit(WorkerToUiEvent::SettingsUpdated(new_settings));
                        }
                        Err(e) => {
                            bridge.log(
                                LogLevel::Error,
                                "settings",
                                format!("Failed to save settings: {e}"),
                                None,
                            );
                        }
                    }
                });
            }
            UiToWorkerCommand::Shutdown => {
                self.root_cancel.cancel();
            }
        }
    }

    fn run_group_write<F>(&self, op: F)
    where
        F: FnOnce(&mut rusqlite::Connection) -> Result<(), DbError> + Send + 'static,
    {
        let pool = self.pool.clone();
        let bridge = self.bridge.clone();
        tokio::spawn(async move {
            let res = pool
                .writer
                .call(move |c| {
                    op(c)?;
                    load_group_snapshot(c)
                })
                .await;
            match res {
                Ok((active_grouping, group_sets, groups, pins)) => {
                    bridge.emit(WorkerToUiEvent::ArtistGroupsChanged {
                        active_grouping,
                        group_sets,
                        groups,
                        pins,
                    });
                }
                Err(e) => {
                    let message = e.to_string();
                    bridge.log(LogLevel::Warn, "groups", message.clone(), None);
                    bridge.emit(WorkerToUiEvent::ArtistGroupError { message });
                }
            }
        });
    }
}

type GroupSnapshot = (
    crate::groups::ActiveGrouping,
    Vec<crate::groups::ArtistGroupSet>,
    Vec<crate::groups::ArtistGroup>,
    std::collections::HashMap<i64, i64>,
);

fn load_group_snapshot(c: &mut rusqlite::Connection) -> Result<GroupSnapshot, DbError> {
    let active_grouping = groups_repo::get_active_grouping(c)?;
    let group_sets = groups_repo::list_group_sets(c)?;
    let (groups, pins) = if let crate::groups::ActiveGrouping::CustomSet(set_id) = active_grouping {
        (
            groups_repo::list_groups(c, set_id)?,
            groups_repo::list_pins(c, set_id)?,
        )
    } else {
        (Vec::new(), std::collections::HashMap::new())
    };
    Ok((active_grouping, group_sets, groups, pins))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::bandcamp::gate::BandcampGate;
    use crate::groups::{ActiveGrouping, BuiltinGrouping, PinTarget};
    use tokio::sync::mpsc::unbounded_channel;

    async fn next_group_event(rx: &mut UnboundedReceiver<WorkerToUiEvent>) -> WorkerToUiEvent {
        loop {
            let ev = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap();
            if matches!(
                ev,
                WorkerToUiEvent::ArtistGroupsChanged { .. }
                    | WorkerToUiEvent::ArtistGroupError { .. }
            ) {
                return ev;
            }
        }
    }

    // Tests 11 and 12: worker group commands, Ready snapshot, error event, and RemoveArtist pin cleanup
    #[tokio::test]
    async fn test_11_12_worker_artist_groups_and_remove_artist_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = DbPool::open(&db_path).unwrap();

        // Seed one artist
        pool.writer
            .call(|c| {
                c.execute(
                    "INSERT INTO artists (band_id, slug, name, url) VALUES (42, 'alpha', 'Alpha', 'https://alpha.bandcamp.com')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let paths = AppPaths {
            data_dir: dir.path().to_path_buf(),
            cache_dir: dir.path().to_path_buf(),
            db_path,
            fx_db_path: dir.path().join("fx.sqlite"),
            cover_cache_dir: dir.path().join("covers"),
            log_dir: dir.path().join("logs"),
            themes_dir: dir.path().join("themes"),
            default_download_dir: dir.path().join("downloads"),
        };
        let gate = BandcampGate::new(0, 0, 1, None);
        let client = BandcampClient::new(gate, "bcvault-test/0.1").unwrap();
        let (ev_tx, mut ev_rx) = unbounded_channel();
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let bridge = UiBridge::new(ev_tx, None);

        let handle = tokio::spawn(Supervisor::run(paths, pool, client, bridge, cmd_rx));

        // Ready includes groups and pins
        let ready = tokio::time::timeout(std::time::Duration::from_secs(5), ev_rx.recv())
            .await
            .unwrap()
            .unwrap();
        match ready {
            WorkerToUiEvent::Ready {
                active_grouping,
                group_sets,
                groups,
                pins,
                ..
            } => {
                assert_eq!(active_grouping, ActiveGrouping::None);
                assert!(group_sets.is_empty());
                assert!(groups.is_empty());
                assert!(pins.is_empty());
            }
            other => panic!("expected Ready, got {other:?}"),
        }

        // Create group set + group
        cmd_tx
            .send(UiToWorkerCommand::CreateArtistGroupSet {
                name: "Workflow".into(),
            })
            .unwrap();
        let set_id = match next_group_event(&mut ev_rx).await {
            WorkerToUiEvent::ArtistGroupsChanged {
                active_grouping,
                group_sets,
                ..
            } => {
                assert_eq!(group_sets.len(), 1);
                assert_eq!(group_sets[0].name, "Workflow");
                assert_eq!(active_grouping, ActiveGrouping::CustomSet(group_sets[0].id));
                group_sets[0].id
            }
            other => panic!("unexpected event: {other:?}"),
        };

        cmd_tx
            .send(UiToWorkerCommand::CreateArtistGroup {
                set_id: Some(set_id),
                name: "Ambient".into(),
                member_ids: vec![],
            })
            .unwrap();
        let gid = match next_group_event(&mut ev_rx).await {
            WorkerToUiEvent::ArtistGroupsChanged { groups, .. } => {
                assert_eq!(groups.len(), 1);
                assert_eq!(groups[0].name, "Ambient");
                groups[0].id
            }
            other => panic!("unexpected event: {other:?}"),
        };

        // Invalid create surfaces ArtistGroupError
        cmd_tx
            .send(UiToWorkerCommand::CreateArtistGroup {
                set_id: Some(set_id),
                name: "   ".into(),
                member_ids: vec![],
            })
            .unwrap();
        match next_group_event(&mut ev_rx).await {
            WorkerToUiEvent::ArtistGroupError { message } => {
                assert!(message.contains("empty"));
            }
            other => panic!("expected ArtistGroupError, got {other:?}"),
        }

        // Collapse + Pin + Non-destructive switch + RemoveArtist
        cmd_tx
            .send(UiToWorkerCommand::SetArtistGroupCollapsed {
                id: gid,
                collapsed: true,
            })
            .unwrap();
        match next_group_event(&mut ev_rx).await {
            WorkerToUiEvent::ArtistGroupsChanged { groups, .. } => {
                assert!(groups[0].collapsed);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        cmd_tx
            .send(UiToWorkerCommand::PinArtists {
                set_id,
                band_ids: vec![42],
                target: PinTarget::Group(gid),
            })
            .unwrap();
        match next_group_event(&mut ev_rx).await {
            WorkerToUiEvent::ArtistGroupsChanged { pins, .. } => {
                assert_eq!(pins.get(&42), Some(&gid));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        // Switch to FirstLetter, then back to CustomSet(set_id) non-destructively
        cmd_tx
            .send(UiToWorkerCommand::SetActiveGrouping {
                grouping: ActiveGrouping::Builtin(BuiltinGrouping::FirstLetter),
            })
            .unwrap();
        match next_group_event(&mut ev_rx).await {
            WorkerToUiEvent::ArtistGroupsChanged {
                active_grouping,
                groups,
                pins,
                ..
            } => {
                assert_eq!(
                    active_grouping,
                    ActiveGrouping::Builtin(BuiltinGrouping::FirstLetter)
                );
                assert!(groups.is_empty());
                assert!(pins.is_empty());
            }
            other => panic!("unexpected event: {other:?}"),
        }

        cmd_tx
            .send(UiToWorkerCommand::SetActiveGrouping {
                grouping: ActiveGrouping::CustomSet(set_id),
            })
            .unwrap();
        match next_group_event(&mut ev_rx).await {
            WorkerToUiEvent::ArtistGroupsChanged {
                active_grouping,
                groups,
                pins,
                ..
            } => {
                assert_eq!(active_grouping, ActiveGrouping::CustomSet(set_id));
                assert_eq!(groups.len(), 1);
                assert_eq!(pins.get(&42), Some(&gid));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        cmd_tx
            .send(UiToWorkerCommand::RemoveArtist { band_id: 42 })
            .unwrap();
        match next_group_event(&mut ev_rx).await {
            WorkerToUiEvent::ArtistGroupsChanged { pins, .. } => {
                assert!(!pins.contains_key(&42));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        cmd_tx.send(UiToWorkerCommand::Shutdown).unwrap();
        handle.await.unwrap();
    }
}

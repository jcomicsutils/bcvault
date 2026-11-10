use crate::{
    bandcamp::BandcampClient,
    db::{
        DbPool,
        repo::{artists, history, library as lib_repo, releases, settings},
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

        bridge.emit(WorkerToUiEvent::Ready {
            artists: initial_artists,
            settings: initial_settings,
            folders: initial_folders.clone(),
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
                    match pool.writer.call(move |c| artists::delete(c, band_id)).await {
                        Ok(()) => {
                            bridge.emit(WorkerToUiEvent::ArtistRemoved { band_id });
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
            } => {
                let mgr = self.dl_manager.clone();
                tokio::spawn(async move {
                    mgr.enqueue_downloads(items, opts, ack_soft_quota).await;
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
            UiToWorkerCommand::UpdateSettings(mut new_settings) => {
                new_settings.batch_limit = new_settings.batch_limit.min(new_settings.quota_hard);
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
}

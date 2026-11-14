pub mod details;
pub mod messages;
pub mod search;
pub mod thumbs;

use crate::{
    db::{self, FxDb},
    services::{
        details::DetailService,
        messages::{FxCommand, FxEvent, ToastKind},
        search::run_palette_search,
        thumbs::{FxThumbService, compute_dir_size},
    },
};
use bcvault_core::{
    bandcamp::BandcampClient, db::DbPool, error::DbError, paths::AppPaths,
    worker::bridge::RepaintHook,
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub struct FxWorkerHandle {
    pub cmd_tx: UnboundedSender<FxCommand>,
    pub ev_rx: UnboundedReceiver<FxEvent>,
}

pub fn spawn_fx_services(
    rt: &tokio::runtime::Handle,
    paths: AppPaths,
    main_db: DbPool,
    client: BandcampClient,
    repaint_hook: Option<RepaintHook>,
) -> Result<FxWorkerHandle, DbError> {
    let fx_db = FxDb::open(&paths.fx_db_path, &paths.db_path)?;
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();

    rt.spawn(async move {
        run_fx_loop(paths, fx_db, main_db, client, ev_tx, repaint_hook, cmd_rx).await;
    });

    Ok(FxWorkerHandle { cmd_tx, ev_rx })
}

async fn run_fx_loop(
    paths: AppPaths,
    fx_db: FxDb,
    main_db: DbPool,
    client: BandcampClient,
    ev_tx: UnboundedSender<FxEvent>,
    repaint_hook: Option<RepaintHook>,
    mut cmd_rx: UnboundedReceiver<FxCommand>,
) {
    let emit = |ev: FxEvent| {
        let _ = ev_tx.send(ev);
        if let Some(h) = &repaint_hook {
            h();
        }
    };

    let cover_dir_for_boot = paths.cover_cache_dir.clone();
    let boot_res = fx_db
        .reader
        .call(move |c| {
            let schema_err = db::verify_main_schema_contract(c)
                .err()
                .map(|e| e.to_string());
            let prefs = db::get_prefs(c)?;
            let saved_views = db::list_saved_views(c)?;
            let palettes = db::load_all_palettes(c)?;
            let recents = db::list_recents(c, 15)?;
            let cover_cache_bytes = compute_dir_size(&cover_dir_for_boot);
            Ok::<_, DbError>((
                prefs,
                saved_views,
                palettes,
                recents,
                schema_err,
                cover_cache_bytes,
            ))
        })
        .await;

    if let Ok((prefs, saved_views, palettes, recents, schema_error, cover_cache_bytes)) = boot_res {
        emit(FxEvent::Ready {
            prefs,
            saved_views,
            palettes,
            recents,
            schema_error,
            cover_cache_bytes,
        });
    }

    let thumb_svc = FxThumbService::new(
        fx_db.clone(),
        client.clone(),
        paths.cover_cache_dir.clone(),
        ev_tx.clone(),
        repaint_hook.clone(),
    );
    let detail_svc = DetailService::new(
        fx_db.clone(),
        main_db,
        client,
        ev_tx.clone(),
        repaint_hook.clone(),
    );

    let mut theme_commands: Vec<(String, String, String)> =
        crate::theme::registry::ThemeRegistry::builtin_only()
            .entries()
            .iter()
            .map(|e| (e.id.clone(), e.name.clone(), e.description.clone()))
            .collect();

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            FxCommand::Shutdown => break,
            FxCommand::SetThemeCommands(cmds) => {
                theme_commands = cmds;
            }
            FxCommand::QueryWall { request_id, query } => {
                let db_r = fx_db.reader.clone();
                let tx = ev_tx.clone();
                let hook = repaint_hook.clone();
                tokio::spawn(async move {
                    if let Ok(rows) = db_r.call(move |c| db::query_wall(c, &query)).await {
                        if rows.is_empty() {
                            let _ = tx.send(FxEvent::WallLoaded {
                                request_id,
                                chunk: 0,
                                last: true,
                                rows: Vec::new(),
                            });
                            if let Some(h) = &hook {
                                h();
                            }
                        } else {
                            let total_chunks = rows.len().div_ceil(500);
                            for (chunk_idx, slice) in rows.chunks(500).enumerate() {
                                let last = chunk_idx + 1 == total_chunks;
                                let _ = tx.send(FxEvent::WallLoaded {
                                    request_id,
                                    chunk: chunk_idx as u32,
                                    last,
                                    rows: slice.to_vec(),
                                });
                                if let Some(h) = &hook {
                                    h();
                                }
                            }
                        }
                    }
                });
            }
            FxCommand::QueryFacets { request_id, query } => {
                let db_r = fx_db.reader.clone();
                let tx = ev_tx.clone();
                let hook = repaint_hook.clone();
                tokio::spawn(async move {
                    if let Ok(facets) = db_r.call(move |c| db::query_facets(c, &query)).await {
                        let _ = tx.send(FxEvent::FacetsLoaded { request_id, facets });
                        if let Some(h) = &hook {
                            h();
                        }
                    }
                });
            }
            FxCommand::QueryStats { request_id } => {
                let db_r = fx_db.reader.clone();
                let tx = ev_tx.clone();
                let hook = repaint_hook.clone();
                tokio::spawn(async move {
                    if let Ok(stats) = db_r.call(|c| db::query_stats(c)).await {
                        let _ = tx.send(FxEvent::StatsLoaded { request_id, stats });
                        if let Some(h) = &hook {
                            h();
                        }
                    }
                });
            }
            FxCommand::Search {
                request_id,
                text,
                scope,
            } => {
                let db_r = fx_db.reader.clone();
                let tx = ev_tx.clone();
                let hook = repaint_hook.clone();
                let cmds = theme_commands.clone();
                tokio::spawn(async move {
                    if let Ok(hits) = db_r
                        .call(move |c| Ok(run_palette_search(c, &text, scope, &cmds)))
                        .await
                    {
                        let _ = tx.send(FxEvent::SearchResults { request_id, hits });
                        if let Some(h) = &hook {
                            h();
                        }
                    }
                });
            }
            FxCommand::EnsureThumbs { generation, items } => {
                thumb_svc.ensure_thumbs(generation, items);
            }
            FxCommand::EnsureHero { art_id } => {
                thumb_svc.ensure_hero(art_id);
            }
            FxCommand::EnsureDetail { key } => {
                detail_svc.ensure(key);
            }
            FxCommand::SaveView { name, state } => {
                let db_w = fx_db.writer.clone();
                let tx = ev_tx.clone();
                let hook = repaint_hook.clone();
                tokio::spawn(async move {
                    let res = db_w
                        .call(move |c| {
                            db::save_view(c, &name, &state)?;
                            db::list_saved_views(c)
                        })
                        .await;
                    if let Ok(views) = res {
                        let _ = tx.send(FxEvent::SavedViewsUpdated(views));
                        if let Some(h) = &hook {
                            h();
                        }
                    }
                });
            }
            FxCommand::DeleteView(id) => {
                let db_w = fx_db.writer.clone();
                let tx = ev_tx.clone();
                let hook = repaint_hook.clone();
                tokio::spawn(async move {
                    let res = db_w
                        .call(move |c| {
                            db::delete_saved_view(c, id)?;
                            db::list_saved_views(c)
                        })
                        .await;
                    if let Ok(views) = res {
                        let _ = tx.send(FxEvent::SavedViewsUpdated(views));
                        if let Some(h) = &hook {
                            h();
                        }
                    }
                });
            }
            FxCommand::SetPref { key, value } => {
                let db_w = fx_db.writer.clone();
                tokio::spawn(async move {
                    let _ = db_w.call(move |c| db::set_pref(c, &key, &value)).await;
                });
            }
            FxCommand::TouchRecent { kind, r#ref } => {
                let db_w = fx_db.writer.clone();
                tokio::spawn(async move {
                    let _ = db_w.call(move |c| db::touch_recent(c, &kind, &r#ref)).await;
                });
            }
            FxCommand::PurgeCovers => {
                let cover_dir = paths.cover_cache_dir.clone();
                let tx = ev_tx.clone();
                let hook = repaint_hook.clone();
                tokio::spawn(async move {
                    let _ = tokio::fs::remove_dir_all(&cover_dir).await;
                    let _ = tokio::fs::create_dir_all(&cover_dir).await;
                    let _ = tx.send(FxEvent::CoverCachePurged);
                    let _ = tx.send(FxEvent::Notice {
                        kind: ToastKind::Info,
                        message: "Purged cover thumbnail cache from disk".to_string(),
                    });
                    if let Some(h) = &hook {
                        h();
                    }
                });
            }
        }
    }
}

pub fn export_settings_json(
    rt: &tokio::runtime::Handle,
    out_dir: std::path::PathBuf,
    json: String,
) {
    rt.spawn_blocking(move || {
        let export_path = out_dir.join("bcvault-settings.json");
        let _ = std::fs::create_dir_all(&out_dir);
        let _ = std::fs::write(export_path, json);
    });
}

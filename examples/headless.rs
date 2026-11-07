use bcvault::{
    bandcamp::{BandcampClient, gate::BandcampGate},
    db::{
        DbPool,
        repo::{releases, settings},
    },
    paths::AppPaths,
    types::{AudioFormat, Classification, DownloadOpts, JobState, ReleaseQuery},
    worker::{
        bridge::UiBridge, downloads::DownloadManager, messages::WorkerToUiEvent, sync::SyncService,
    },
};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        print_usage();
        return Ok(());
    };

    let paths = AppPaths::resolve()
        .ok_or_else(|| anyhow::anyhow!("Could not resolve project directories"))?;
    paths.ensure_dirs()?;

    let pool = DbPool::open(&paths.db_path)?;
    let default_dl = paths.default_download_dir.clone();
    let app_settings = pool
        .reader
        .call(move |c| settings::get_settings(c, default_dl))
        .await?;

    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
    let bridge = UiBridge::new(ev_tx, None);
    let gate = BandcampGate::new(
        app_settings.page_delay_ms,
        app_settings.page_jitter_pct,
        app_settings.max_concurrent_downloads,
        None,
    );
    let client = BandcampClient::new(gate, &app_settings.user_agent)?;

    match cmd {
        "add" => {
            let slug = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("Missing <slug_or_url>"))?;
            let svc = SyncService::new(pool.clone(), client, bridge);
            let outcome = svc
                .add_and_sync_artist(slug, CancellationToken::new())
                .await?;
            let rows = pool
                .reader
                .call(move |c| {
                    releases::query_releases(
                        c,
                        &ReleaseQuery {
                            band_id: Some(outcome.band_id),
                            ..Default::default()
                        },
                    )
                })
                .await?;

            let mut free = 0;
            let mut nyp = 0;
            let mut email = 0;
            let mut paid = 0;
            let mut unknown = 0;
            for r in &rows {
                match r.classification {
                    Classification::Free => free += 1,
                    Classification::Nyp => nyp += 1,
                    Classification::EmailRequired => email += 1,
                    Classification::Paid => paid += 1,
                    Classification::Unknown => unknown += 1,
                }
            }
            println!(
                "Synced band_id={} (total={}): Free={free} NYP={nyp} Email={email} Paid={paid} Unknown={unknown}",
                outcome.band_id,
                rows.len()
            );
        }
        "download" => {
            let band_id: i64 = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("Missing <band_id>"))?
                .parse()?;
            let fmt = args
                .get(2)
                .and_then(|s| AudioFormat::parse(s))
                .unwrap_or(app_settings.format);
            let out_dir = args
                .get(3)
                .map(PathBuf::from)
                .unwrap_or_else(|| app_settings.out_dir.clone());

            let rows = pool
                .reader
                .call(move |c| {
                    releases::query_releases(
                        c,
                        &ReleaseQuery {
                            band_id: Some(band_id),
                            ..Default::default()
                        },
                    )
                })
                .await?;

            let keys: Vec<_> = rows
                .into_iter()
                .filter(|r| !r.downloaded && !r.is_preorder && r.classification.auto_downloadable())
                .map(|r| r.key)
                .collect();

            if keys.is_empty() {
                println!("No eligible un-downloaded items for band_id={band_id}");
                return Ok(());
            }

            let root_cancel = CancellationToken::new();
            let mgr = DownloadManager::new(
                pool,
                client,
                bridge,
                app_settings.clone(),
                root_cancel.clone(),
            );
            let expected = keys.len().min(app_settings.batch_limit as usize);
            mgr.enqueue_downloads(
                keys,
                DownloadOpts {
                    format: fmt,
                    out_dir,
                    batch_limit: app_settings.batch_limit,
                },
                true,
            )
            .await;

            let mut finished = 0;
            while let Some(ev) = ev_rx.recv().await {
                if let WorkerToUiEvent::JobState { job_id, state } = ev {
                    match state {
                        JobState::Done { path, bytes } => {
                            println!("Job #{job_id} DONE: {} ({bytes} B)", path.display());
                            finished += 1;
                        }
                        JobState::Failed(err) => {
                            eprintln!("Job #{job_id} FAILED: {err}");
                            finished += 1;
                        }
                        JobState::Cancelled => {
                            println!("Job #{job_id} CANCELLED");
                            finished += 1;
                        }
                        _ => {}
                    }
                    if finished >= expected {
                        break;
                    }
                }
            }
            root_cancel.cancel();
        }
        _ => print_usage(),
    }

    Ok(())
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  cargo run --example headless -- add <slug_or_url>");
    eprintln!("  cargo run --example headless -- download <band_id> [format] [out_dir]");
}

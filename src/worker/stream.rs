use crate::{
    bandcamp::{BandcampClient, filenames::resolve_download_filename, gate::GateClass},
    error::{BcError, DownloadError},
    paths::to_os_safe_path,
    types::{ItemType, JobId},
    worker::{bridge::UiBridge, messages::WorkerToUiEvent},
};
use futures_util::StreamExt;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct StreamOutcome {
    pub final_path: PathBuf,
    pub bytes: u64,
}

pub struct StreamRequest<'a> {
    pub job_id: JobId,
    pub url: &'a str,
    pub band_id: Option<i64>,
    pub dest_dir: &'a Path,
    pub artist: &'a str,
    pub title: &'a str,
    pub kind: ItemType,
    pub fallback_ext: &'a str,
}

pub async fn stream_download(
    client: &BandcampClient,
    bridge: &UiBridge,
    req: StreamRequest<'_>,
    cancel: CancellationToken,
) -> Result<StreamOutcome, DownloadError> {
    tokio::fs::create_dir_all(req.dest_dir).await?;

    let _permit = tokio::select! {
        _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
        p = client.gate.acquire(GateClass::Download, req.band_id) => p,
    };

    let mut resp = send_dl_request(client, req.url, cancel.clone()).await?;

    // Check if Content-Type is text/html or application/json (statdownload handshake)
    let is_non_binary = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| {
            let lower = ct.to_ascii_lowercase();
            lower.starts_with("text/html")
                || lower.starts_with("application/json")
                || lower.starts_with("text/javascript")
        });

    if is_non_binary {
        let body_text = resp.text().await.map_err(BcError::Http)?;
        let resolved_url = client
            .resolve_download_url(req.url, &body_text, req.band_id)
            .await?;
        resp = send_dl_request(client, &resolved_url, cancel.clone()).await?;
    }

    let content_disp = resp
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .map(ToString::to_string);

    let expected_len = resp.content_length();
    let file_name = resolve_download_filename(
        content_disp.as_deref(),
        req.artist,
        req.title,
        req.kind,
        req.fallback_ext,
    );

    let part_path = to_os_safe_path(
        &req.dest_dir
            .join(format!("{file_name}.part.{job}", job = req.job_id)),
    );
    let file = tokio::fs::File::create(&part_path).await?;
    let mut writer = BufWriter::with_capacity(64 * 1024, file);

    let mut stream = resp.bytes_stream();
    let mut total_bytes: u64 = 0;
    let mut last_tick = Instant::now();
    let mut bytes_at_last_tick: u64 = 0;
    let mut ema_bps: f64 = 0.0;

    let mut tick_interval = tokio::time::interval(Duration::from_millis(100));
    tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Consume the immediate first tick
    tick_interval.tick().await;

    let stream_res: Result<(), DownloadError> = async {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    return Err(DownloadError::Cancelled);
                }
                _ = tick_interval.tick() => {
                    let now = Instant::now();
                    let dt = now.duration_since(last_tick).as_secs_f64();
                    if dt > 0.0 {
                        let inst_bps = ((total_bytes - bytes_at_last_tick) as f64) / dt;
                        ema_bps = if ema_bps == 0.0 {
                            inst_bps
                        } else {
                            0.2 * inst_bps + 0.8 * ema_bps
                        };
                        last_tick = now;
                        bytes_at_last_tick = total_bytes;
                        bridge.emit(WorkerToUiEvent::DownloadByteProgress {
                            job_id: req.job_id,
                            bytes: total_bytes,
                            total: expected_len,
                            bytes_per_sec: ema_bps,
                        });
                    }
                }
                chunk_opt = stream.next() => {
                    match chunk_opt {
                        Some(Ok(chunk)) => {
                            writer.write_all(&chunk).await?;
                            total_bytes += chunk.len() as u64;
                        }
                        Some(Err(e)) => {
                            if let Some(expected) = expected_len
                                && total_bytes < expected
                            {
                                return Err(DownloadError::Truncated {
                                    expected,
                                    actual: total_bytes,
                                });
                            }
                            return Err(DownloadError::Bandcamp(BcError::Http(e)));
                        }
                        None => break,
                    }
                }
            }
        }

        writer.flush().await?;
        writer.get_ref().sync_all().await?;
        Ok(())
    }
    .await;

    drop(writer);

    if let Err(e) = stream_res {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(e);
    }

    if let Some(expected) = expected_len
        && total_bytes != expected
    {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(DownloadError::Truncated {
            expected,
            actual: total_bytes,
        });
    }

    // Emit final progress event
    bridge.emit(WorkerToUiEvent::DownloadByteProgress {
        job_id: req.job_id,
        bytes: total_bytes,
        total: Some(total_bytes),
        bytes_per_sec: ema_bps,
    });

    let final_path = to_os_safe_path(&unique_dest_path(req.dest_dir, &file_name).await);
    if let Err(e) = tokio::fs::rename(&part_path, &final_path).await {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(DownloadError::Io(e));
    }

    Ok(StreamOutcome {
        final_path,
        bytes: total_bytes,
    })
}

async fn send_dl_request(
    client: &BandcampClient,
    url: &str,
    cancel: CancellationToken,
) -> Result<reqwest::Response, DownloadError> {
    let resp = tokio::select! {
        _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
        r = client.dl_client.get(url).send() => r.map_err(BcError::Http)?,
    };

    let status = resp.status();
    let retry_after = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(crate::bandcamp::gate::parse_retry_after);

    if let Some(wait_dur) = client.gate.report(status, retry_after) {
        return Err(DownloadError::Bandcamp(BcError::RateLimited {
            retry_after_s: wait_dur.as_secs().max(1),
        }));
    }

    let resp = resp.error_for_status().map_err(BcError::Http)?;
    Ok(resp)
}

pub async fn unique_dest_path(dir: &Path, file_name: &str) -> PathBuf {
    let initial = dir.join(file_name);
    if !path_exists_ci(dir, file_name, &initial).await {
        return initial;
    }

    let (stem, ext) = match file_name.rfind('.') {
        Some(idx) if idx > 0 => (&file_name[..idx], Some(&file_name[idx..])),
        _ => (file_name, None),
    };

    for i in 1..10_000 {
        let candidate_name = match ext {
            Some(e) => format!("{stem} ({i}){e}"),
            None => format!("{stem} ({i})"),
        };
        let candidate_path = dir.join(&candidate_name);
        if !path_exists_ci(dir, &candidate_name, &candidate_path).await {
            return candidate_path;
        }
    }

    dir.join(format!("{stem}_{}", chrono::Utc::now().timestamp_millis()))
}

async fn path_exists_ci(dir: &Path, target_name: &str, exact_path: &Path) -> bool {
    if tokio::fs::try_exists(exact_path).await.unwrap_or(false) {
        return true;
    }
    let target_lower = target_name.to_lowercase();
    if let Ok(mut rd) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            if entry.file_name().to_string_lossy().to_lowercase() == target_lower {
                return true;
            }
        }
    }
    false
}

/// Sweeps any leftover `*.part*` files in the output directory tree on boot.
pub fn sweep_part_files(root: &Path) {
    if !root.exists() {
        return;
    }
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sweep_part_files(&path);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && (name.ends_with(".part") || name.contains(".part."))
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

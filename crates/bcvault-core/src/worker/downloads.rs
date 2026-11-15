use crate::{
    bandcamp::{
        BandcampClient,
        download_api::choose_format,
        filenames::{ReleaseFormatContext, format_release_stem, sanitize_stem},
    },
    db::{
        DbPool,
        repo::{
            artists,
            history::{self, NewHistoryEntry},
            releases,
        },
    },
    error::{BcError, DbError, DownloadError},
    types::{
        AudioFormat, Classification, DownloadOpts, DownloadSource, EnqueueMode, HistoryStatus,
        ItemKey, ItemType, JobError, JobId, JobState, JobView, LogLevel, OutputLayout,
        ReleaseFilter, ReleaseQuery, ReleaseRow, RemainingScope, Settings, SortDir, SortKey,
    },
    worker::{
        bridge::UiBridge,
        extract::extract_zip_archive,
        messages::{DeferCause, DeferredBatch, EnqueueSummary, WorkerToUiEvent},
        quota::{QuotaDecision, decide},
        stream::{StreamRequest, stream_download},
        thumbs::fetch_cover_to_path,
    },
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
struct JobSpec {
    id: JobId,
    key: ItemKey,
    band_id: i64,
    label: String,
    opts: DownloadOpts,
    ack_soft_quota: bool,
    redeem_url: Option<String>,
}

struct ManagerState {
    settings: Settings,
    artists_with_queue: VecDeque<i64>,
    per_artist_queue: HashMap<i64, VecDeque<JobSpec>>,
    all_jobs: HashMap<JobId, JobSpec>,
    in_flight: HashSet<ItemKey>,
    active_cancels: HashMap<JobId, CancellationToken>,
    active_bands: HashMap<i64, usize>,
    artist_blocked_until: HashMap<i64, Instant>,
    active_workers: usize,
}

#[derive(Clone)]
pub struct DownloadManager {
    pool: DbPool,
    client: BandcampClient,
    bridge: UiBridge,
    state: Arc<Mutex<ManagerState>>,
    notify: Arc<Notify>,
    paused: Arc<AtomicBool>,
    next_job_id: Arc<AtomicU64>,
    root_cancel: CancellationToken,
}

impl DownloadManager {
    pub fn new(
        pool: DbPool,
        client: BandcampClient,
        bridge: UiBridge,
        settings: Settings,
        root_cancel: CancellationToken,
    ) -> Self {
        let mgr = Self {
            pool,
            client,
            bridge,
            state: Arc::new(Mutex::new(ManagerState {
                settings,
                artists_with_queue: VecDeque::new(),
                per_artist_queue: HashMap::new(),
                all_jobs: HashMap::new(),
                in_flight: HashSet::new(),
                active_cancels: HashMap::new(),
                active_bands: HashMap::new(),
                artist_blocked_until: HashMap::new(),
                active_workers: 0,
            })),
            notify: Arc::new(Notify::new()),
            paused: Arc::new(AtomicBool::new(false)),
            next_job_id: Arc::new(AtomicU64::new(1)),
            root_cancel,
        };
        mgr.spawn_dispatcher();
        mgr
    }

    pub fn update_settings(&self, settings: Settings) {
        if let Ok(mut st) = self.state.lock() {
            st.settings = settings;
        }
        self.notify.notify_one();
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
        self.notify.notify_one();
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn cancel_job(&self, job_id: JobId) {
        let mut removed_queued = false;
        if let Ok(mut st) = self.state.lock() {
            if let Some(tok) = st.active_cancels.get(&job_id) {
                tok.cancel();
            } else if let Some(spec) = st.all_jobs.get(&job_id) {
                let bid = spec.band_id;
                let key = spec.key;
                if let Some(q) = st.per_artist_queue.get_mut(&bid)
                    && let Some(pos) = q.iter().position(|j| j.id == job_id)
                {
                    q.remove(pos);
                    st.in_flight.remove(&key);
                    removed_queued = true;
                }
            }
        }
        if removed_queued {
            self.bridge.emit(WorkerToUiEvent::JobState {
                job_id,
                state: JobState::Cancelled,
            });
        }
    }

    pub fn cancel_all(&self) {
        let mut cancelled_ids = Vec::new();
        if let Ok(mut st) = self.state.lock() {
            let mut removed_keys = Vec::new();
            for q in st.per_artist_queue.values_mut() {
                while let Some(spec) = q.pop_front() {
                    cancelled_ids.push(spec.id);
                    removed_keys.push(spec.key);
                }
            }
            for key in removed_keys {
                st.in_flight.remove(&key);
            }
            st.artists_with_queue.clear();
            for tok in st.active_cancels.values() {
                tok.cancel();
            }
        }
        for job_id in cancelled_ids {
            self.bridge.emit(WorkerToUiEvent::JobState {
                job_id,
                state: JobState::Cancelled,
            });
        }
    }

    pub fn retry_job(&self, job_id: JobId) {
        let spec_opt = self
            .state
            .lock()
            .ok()
            .and_then(|st| st.all_jobs.get(&job_id).cloned());

        let Some(mut spec) = spec_opt else { return };
        spec.id = self.next_job_id.fetch_add(1, Ordering::SeqCst);
        spec.ack_soft_quota = true;
        self.push_spec(spec);
    }

    pub async fn enqueue_downloads(
        &self,
        items: Vec<ItemKey>,
        opts: DownloadOpts,
        ack_soft_quota: bool,
        mode: EnqueueMode,
    ) {
        if items.is_empty() {
            self.bridge
                .emit(WorkerToUiEvent::EnqueueSummary(EnqueueSummary::default()));
            return;
        }

        let settings = self
            .state
            .lock()
            .map(|st| st.settings.clone())
            .unwrap_or_default();

        // Deduplicate keys while preserving order.
        let mut seen = HashSet::with_capacity(items.len());
        let unique_items: Vec<ItemKey> = items.into_iter().filter(|k| seen.insert(*k)).collect();
        let requested_total = unique_items.len() as u32;

        // Fetch release rows and group by band_id in order
        let items_clone = unique_items;
        let rows_res = self
            .pool
            .reader
            .call(move |c| {
                let mut rows = Vec::with_capacity(items_clone.len());
                let mut missing = 0u32;
                for k in items_clone {
                    if let Some(r) = releases::get_release(c, k)? {
                        rows.push(r);
                    } else {
                        missing += 1;
                    }
                }
                Ok::<_, DbError>((rows, missing))
            })
            .await;

        let Ok((rows, missing_count)) = rows_res else {
            return;
        };

        let in_flight_snapshot: HashSet<ItemKey> = self
            .state
            .lock()
            .map(|st| st.in_flight.clone())
            .unwrap_or_default();

        let mut summary = EnqueueSummary {
            requested: requested_total,
            skipped_ineligible: missing_count,
            ..EnqueueSummary::default()
        };

        let mut by_band: HashMap<i64, Vec<ReleaseRow>> = HashMap::new();
        let mut band_order: Vec<i64> = Vec::new();
        for r in rows {
            if !r.is_downloadable() {
                summary.skipped_ineligible += 1;
                continue;
            }
            if mode == EnqueueMode::New && r.downloaded {
                summary.skipped_downloaded += 1;
                continue;
            }
            if in_flight_snapshot.contains(&r.key) {
                summary.skipped_in_flight += 1;
                continue;
            }
            if !by_band.contains_key(&r.band_id) {
                band_order.push(r.band_id);
            }
            by_band.entry(r.band_id).or_default().push(r);
        }

        for band_id in band_order {
            let Some(band_rows) = by_band.remove(&band_id) else {
                continue;
            };

            if self.is_artist_blocked(band_id) {
                summary.deferred.push(DeferredBatch {
                    band_id,
                    count: band_rows.len() as u32,
                    cause: DeferCause::Blocked30Min,
                });
                for r in band_rows {
                    let jid = self.next_job_id.fetch_add(1, Ordering::SeqCst);
                    let label = format!("{} - {}", r.artist_name, r.title);
                    self.bridge.emit(WorkerToUiEvent::JobQueued(JobView {
                        id: jid,
                        key: r.key,
                        band_id,
                        label,
                        format: opts.format,
                        state: JobState::Queued,
                    }));
                    self.bridge.emit(WorkerToUiEvent::JobState {
                        job_id: jid,
                        state: JobState::Failed(JobError::QuotaExceeded),
                    });
                }
                continue;
            }

            let used_30d = self
                .pool
                .reader
                .call(move |c| history::quota_used_30d(c, band_id))
                .await
                .unwrap_or(0);

            let requested = band_rows.len() as u32;
            let decision = decide(
                used_30d,
                requested,
                opts.batch_limit.min(settings.quota_hard),
                settings.quota_soft,
                settings.quota_hard,
                ack_soft_quota,
            );

            self.bridge.emit(WorkerToUiEvent::QuotaStatus {
                band_id,
                used_30d,
                soft: settings.quota_soft,
                hard: settings.quota_hard,
            });

            match decision {
                QuotaDecision::Blocked => {
                    self.bridge.log(
                        LogLevel::Warn,
                        "quota",
                        format!(
                            "Artist {band_id} reached hard 30-day download limit ({used_30d}/{})",
                            settings.quota_hard
                        ),
                        None,
                    );
                    summary.deferred.push(DeferredBatch {
                        band_id,
                        count: requested,
                        cause: DeferCause::HardCap,
                    });
                    for r in band_rows {
                        let jid = self.next_job_id.fetch_add(1, Ordering::SeqCst);
                        let label = format!("{} - {}", r.artist_name, r.title);
                        self.bridge.emit(WorkerToUiEvent::JobQueued(JobView {
                            id: jid,
                            key: r.key,
                            band_id,
                            label,
                            format: opts.format,
                            state: JobState::Queued,
                        }));
                        self.bridge.emit(WorkerToUiEvent::JobState {
                            job_id: jid,
                            state: JobState::Failed(JobError::QuotaExceeded),
                        });
                    }
                }
                QuotaDecision::Warn { count, truncated } => {
                    if truncated {
                        self.bridge.log(
                            LogLevel::Info,
                            "quota",
                            format!("Batch truncated from {requested} to {count} due to limit"),
                            None,
                        );
                        let excess = requested.saturating_sub(count);
                        if excess > 0 {
                            let cause = if used_30d + count >= settings.quota_hard {
                                DeferCause::HardCap
                            } else {
                                DeferCause::BatchLimit
                            };
                            summary.deferred.push(DeferredBatch {
                                band_id,
                                count: excess,
                                cause,
                            });
                        }
                    }
                    summary.awaiting_ack += count;
                    let subset: Vec<ItemKey> = band_rows
                        .into_iter()
                        .take(count as usize)
                        .map(|r| r.key)
                        .collect();
                    self.bridge.emit(WorkerToUiEvent::QuotaWarning {
                        band_id,
                        used_30d,
                        requested: count,
                        soft: settings.quota_soft,
                        hard: settings.quota_hard,
                        items: subset,
                        opts: opts.clone(),
                    });
                }
                QuotaDecision::Allow { count, truncated } => {
                    if truncated {
                        self.bridge.log(
                            LogLevel::Info,
                            "quota",
                            format!("Batch truncated from {requested} to {count} due to limit"),
                            None,
                        );
                        let excess = requested.saturating_sub(count);
                        if excess > 0 {
                            let cause = if used_30d + count >= settings.quota_hard {
                                DeferCause::HardCap
                            } else {
                                DeferCause::BatchLimit
                            };
                            summary.deferred.push(DeferredBatch {
                                band_id,
                                count: excess,
                                cause,
                            });
                        }
                    }
                    for r in band_rows.into_iter().take(count as usize) {
                        let jid = self.next_job_id.fetch_add(1, Ordering::SeqCst);
                        let spec = JobSpec {
                            id: jid,
                            key: r.key,
                            band_id,
                            label: format!("{} - {}", r.artist_name, r.title),
                            opts: opts.clone(),
                            ack_soft_quota,
                            redeem_url: None,
                        };
                        if self.push_spec(spec) {
                            summary.queued += 1;
                        } else {
                            summary.skipped_in_flight += 1;
                        }
                    }
                }
            }
        }

        self.bridge.emit(WorkerToUiEvent::EnqueueSummary(summary));
    }

    pub async fn enqueue_remaining(
        &self,
        scope: RemainingScope,
        opts: DownloadOpts,
        ack_soft_quota: bool,
    ) {
        let keys_res = self
            .pool
            .reader
            .call(move |c| {
                let band_ids = match scope {
                    RemainingScope::Artists(ids) => ids,
                    RemainingScope::Library => artists::list_views(c)?
                        .into_iter()
                        .map(|a| a.band_id)
                        .collect(),
                };
                let mut keys = Vec::new();
                for band_id in band_ids {
                    let q = ReleaseQuery {
                        band_id: Some(band_id),
                        filter: ReleaseFilter::NotDownloaded,
                        search: String::new(),
                        sort: SortKey::Released,
                        dir: SortDir::Desc,
                    };
                    for r in releases::query_releases(c, &q)? {
                        if r.is_download_candidate() {
                            keys.push(r.key);
                        }
                    }
                }
                Ok::<_, DbError>(keys)
            })
            .await;

        let Ok(keys) = keys_res else { return };
        self.enqueue_downloads(keys, opts, ack_soft_quota, EnqueueMode::New)
            .await;
    }

    pub async fn enqueue_redeem(&self, target: ItemKey, url: String, opts: DownloadOpts) {
        let row_opt = self
            .pool
            .reader
            .call(move |c| releases::get_release(c, target))
            .await
            .ok()
            .flatten();

        let (band_id, label) = match row_opt {
            Some(r) => (r.band_id, format!("{} - {}", r.artist_name, r.title)),
            None => (0, format!("Redeem {:?}", target)),
        };

        let jid = self.next_job_id.fetch_add(1, Ordering::SeqCst);
        let spec = JobSpec {
            id: jid,
            key: target,
            band_id,
            label,
            opts,
            ack_soft_quota: true,
            redeem_url: Some(url),
        };
        self.push_spec(spec);
    }

    fn push_spec(&self, spec: JobSpec) -> bool {
        let view = JobView {
            id: spec.id,
            key: spec.key,
            band_id: spec.band_id,
            label: spec.label.clone(),
            format: spec.opts.format,
            state: JobState::Queued,
        };
        let inserted = if let Ok(mut st) = self.state.lock() {
            if !st.in_flight.insert(spec.key) {
                false
            } else {
                let bid = spec.band_id;
                st.all_jobs.insert(spec.id, spec.clone());
                let was_empty = {
                    let q = st.per_artist_queue.entry(bid).or_default();
                    let empty = q.is_empty();
                    q.push_back(spec);
                    empty
                };
                if was_empty && !st.artists_with_queue.contains(&bid) {
                    st.artists_with_queue.push_back(bid);
                }
                true
            }
        } else {
            false
        };
        if inserted {
            self.bridge.emit(WorkerToUiEvent::JobQueued(view));
            self.notify.notify_one();
        }
        inserted
    }

    #[cfg(test)]
    pub(crate) fn in_flight_len(&self) -> usize {
        self.state.lock().map(|st| st.in_flight.len()).unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn in_flight_contains(&self, key: &ItemKey) -> bool {
        self.state
            .lock()
            .map(|st| st.in_flight.contains(key))
            .unwrap_or(false)
    }

    fn is_artist_blocked(&self, band_id: i64) -> bool {
        let Ok(mut st) = self.state.lock() else {
            return false;
        };
        if let Some(&until) = st.artist_blocked_until.get(&band_id) {
            if Instant::now() < until {
                return true;
            }
            st.artist_blocked_until.remove(&band_id);
        }
        false
    }

    fn block_artist_for_30_mins(&self, band_id: i64) {
        let mut drained = Vec::new();
        if let Ok(mut st) = self.state.lock() {
            st.artist_blocked_until
                .insert(band_id, Instant::now() + Duration::from_secs(30 * 60));
            let mut drained_keys = Vec::new();
            if let Some(q) = st.per_artist_queue.get_mut(&band_id) {
                while let Some(spec) = q.pop_front() {
                    drained.push(spec.id);
                    drained_keys.push(spec.key);
                }
            }
            for key in drained_keys {
                st.in_flight.remove(&key);
            }
        }
        for job_id in drained {
            self.bridge.emit(WorkerToUiEvent::JobState {
                job_id,
                state: JobState::Failed(JobError::QuotaExceeded),
            });
        }
    }

    fn spawn_dispatcher(&self) {
        let mgr = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = mgr.root_cancel.cancelled() => break,
                    _ = mgr.notify.notified() => {}
                }

                loop {
                    if mgr.root_cancel.is_cancelled() || mgr.is_paused() {
                        break;
                    }

                    let next_spec = {
                        let Ok(mut st) = mgr.state.lock() else {
                            break;
                        };
                        let max_conc = usize::from(st.settings.max_concurrent_downloads.max(1));
                        if st.active_workers >= max_conc {
                            None
                        } else {
                            pop_next_spec(&mut st)
                        }
                    };

                    let Some(spec) = next_spec else {
                        break;
                    };

                    let job_cancel = mgr.root_cancel.child_token();
                    if let Ok(mut st) = mgr.state.lock() {
                        st.active_workers += 1;
                        *st.active_bands.entry(spec.band_id).or_insert(0) += 1;
                        st.active_cancels.insert(spec.id, job_cancel.clone());
                    }

                    let mgr_c = mgr.clone();
                    tokio::spawn(async move {
                        mgr_c.run_job(spec.clone(), job_cancel).await;
                        if let Ok(mut st) = mgr_c.state.lock() {
                            st.active_workers = st.active_workers.saturating_sub(1);
                            if let Some(c) = st.active_bands.get_mut(&spec.band_id) {
                                *c = c.saturating_sub(1);
                            }
                            st.active_cancels.remove(&spec.id);
                            st.in_flight.remove(&spec.key);
                        }
                        mgr_c.notify.notify_one();
                    });
                }
            }
        });
    }

    async fn run_job(&self, spec: JobSpec, cancel: CancellationToken) {
        self.bridge.emit(WorkerToUiEvent::JobState {
            job_id: spec.id,
            state: JobState::Resolving,
        });

        let settings = self
            .state
            .lock()
            .map(|st| st.settings.clone())
            .unwrap_or_default();

        let res = if let Some(ref redeem_url) = spec.redeem_url {
            self.execute_redeem_job(&spec, redeem_url, &settings, cancel)
                .await
        } else {
            self.execute_api_job(&spec, &settings, cancel).await
        };

        match res {
            Ok((final_path, bytes)) => {
                self.bridge.emit(WorkerToUiEvent::JobState {
                    job_id: spec.id,
                    state: JobState::Done {
                        path: final_path,
                        bytes,
                    },
                });
            }
            Err(DownloadError::Cancelled) => {
                self.bridge.emit(WorkerToUiEvent::JobState {
                    job_id: spec.id,
                    state: JobState::Cancelled,
                });
            }
            Err(err) => {
                let job_err = map_download_error(&err);
                self.bridge.log(
                    LogLevel::Error,
                    "download",
                    format!("Job {} ({}) failed: {job_err}", spec.id, spec.label),
                    Some(spec.key),
                );
                self.bridge.emit(WorkerToUiEvent::JobState {
                    job_id: spec.id,
                    state: JobState::Failed(job_err),
                });
            }
        }
    }

    async fn execute_api_job(
        &self,
        spec: &JobSpec,
        settings: &Settings,
        cancel: CancellationToken,
    ) -> Result<(PathBuf, u64), DownloadError> {
        let band_id = spec.band_id;
        let key = spec.key;

        if self.is_artist_blocked(band_id) {
            return Err(DownloadError::Bandcamp(BcError::QuotaExceeded));
        }

        // Re-check quota right before buy_for_nothing
        let used_30d = self
            .pool
            .reader
            .call(move |c| history::quota_used_30d(c, band_id))
            .await?;

        let decision = decide(
            used_30d,
            1,
            spec.opts.batch_limit.min(settings.quota_hard),
            settings.quota_soft,
            settings.quota_hard,
            spec.ack_soft_quota,
        );

        if !matches!(decision, QuotaDecision::Allow { .. }) {
            return Err(DownloadError::Bandcamp(BcError::QuotaExceeded));
        }

        let row = self
            .pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await?
            .ok_or_else(|| {
                DownloadError::Bandcamp(BcError::UnexpectedResponse(
                    "Release row not found".to_string(),
                ))
            })?;

        let avail_res = tokio::select! {
            _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
            r = self.client.buy_for_nothing(key, band_id) => r,
        };

        let available = match avail_res {
            Ok(map) => map,
            Err(BcError::EmailRequired) => {
                // Re-classify row as email_required
                let updated = self
                    .pool
                    .writer
                    .call(move |c| {
                        releases::set_classification(c, key, Classification::EmailRequired)?;
                        let r = releases::get_release(c, key)?;
                        let st = artists::stats_with_quota(c, band_id)?;
                        Ok::<_, DbError>((r, st))
                    })
                    .await?;
                if let Some(ur) = updated.0 {
                    self.bridge
                        .emit(WorkerToUiEvent::ReleaseUpdated(Box::new(ur)));
                }
                self.bridge.emit(WorkerToUiEvent::ArtistStats {
                    band_id,
                    stats: updated.1,
                });
                return Err(DownloadError::Bandcamp(BcError::EmailRequired));
            }
            Err(BcError::QuotaExceeded) => {
                self.block_artist_for_30_mins(band_id);
                return Err(DownloadError::Bandcamp(BcError::QuotaExceeded));
            }
            Err(e) => return Err(DownloadError::Bandcamp(e)),
        };

        // Insert started history row immediately after ok:true because allowance slot was spent
        let chosen = match choose_format(&available, spec.opts.format, settings.format_fallback) {
            Ok(c) => c,
            Err(e) => {
                let req_fmt = spec.opts.format;
                let msg = e.to_string();
                let _ = self
                    .pool
                    .writer
                    .call(move |c| {
                        let hid = history::insert_started(
                            c,
                            &NewHistoryEntry {
                                key,
                                band_id,
                                format: req_fmt,
                                source: DownloadSource::Api,
                                counts_against_quota: true,
                            },
                        )?;
                        history::finish(c, hid, HistoryStatus::Failed, None, Some(&msg))
                    })
                    .await;
                return Err(DownloadError::Bandcamp(e));
            }
        };

        let chosen_fmt = chosen.format;
        let history_id = self
            .pool
            .writer
            .call(move |c| {
                history::insert_started(
                    c,
                    &NewHistoryEntry {
                        key,
                        band_id,
                        format: chosen_fmt,
                        source: DownloadSource::Api,
                        counts_against_quota: true,
                    },
                )
            })
            .await?;

        self.stream_and_commit(
            spec,
            &row,
            chosen.format,
            &chosen.url,
            history_id,
            settings,
            cancel,
        )
        .await
    }

    async fn execute_redeem_job(
        &self,
        spec: &JobSpec,
        redeem_url: &str,
        settings: &Settings,
        cancel: CancellationToken,
    ) -> Result<(PathBuf, u64), DownloadError> {
        let key = spec.key;
        let band_id = spec.band_id;

        let items = tokio::select! {
            _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
            r = self.client.fetch_redeem_page(redeem_url) => r?,
        };

        let item = items
            .iter()
            .find(|it| it.key.id == key.id)
            .or_else(|| {
                if items.len() == 1 {
                    items.first()
                } else {
                    None
                }
            })
            .ok_or(BcError::RedeemExpiredOrEmpty)?;

        let mut avail = HashMap::new();
        for (&fmt, info) in &item.downloads {
            avail.insert(fmt, info.url.clone());
        }

        let chosen = choose_format(&avail, spec.opts.format, settings.format_fallback)?;
        let chosen_fmt = chosen.format;

        let row_opt = self
            .pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await?;

        let row = row_opt.unwrap_or_else(|| ReleaseRow {
            key,
            band_id,
            title: item.title.clone(),
            artist_name: item.artist.clone(),
            url: String::new(),
            cover_url: None,
            art_id: item.art_id,
            classification: Classification::EmailRequired,
            is_preorder: false,
            track_count: None,
            published_at: None,
            modified_at: None,
            downloaded: false,
            downloaded_at: None,
            downloaded_format: None,
            local_path: None,
            local_status: None,
            origin: None,
            missing_ack_at: None,
            has_remote_update: false,
            remote_update_reason: None,
        });

        let history_id = self
            .pool
            .writer
            .call(move |c| {
                history::insert_started(
                    c,
                    &NewHistoryEntry {
                        key,
                        band_id,
                        format: chosen_fmt,
                        source: DownloadSource::Redeem,
                        counts_against_quota: false,
                    },
                )
            })
            .await?;

        self.stream_and_commit(
            spec,
            &row,
            chosen.format,
            &chosen.url,
            history_id,
            settings,
            cancel,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn stream_and_commit(
        &self,
        spec: &JobSpec,
        row: &ReleaseRow,
        chosen_fmt: AudioFormat,
        download_url: &str,
        history_id: i64,
        settings: &Settings,
        cancel: CancellationToken,
    ) -> Result<(PathBuf, u64), DownloadError> {
        self.bridge.emit(WorkerToUiEvent::JobState {
            job_id: spec.id,
            state: JobState::Downloading,
        });

        let band_id = row.band_id;
        let page_artist = self
            .pool
            .reader
            .call(move |c| artists::get_view(c, band_id))
            .await
            .ok()
            .flatten()
            .map(|v| v.display_name().to_string())
            .unwrap_or_else(|| row.artist_name.clone());

        let base_out = match settings.layout {
            OutputLayout::Flat => spec.opts.out_dir.clone(),
            OutputLayout::ByArtist => spec.opts.out_dir.join(sanitize_stem(&row.artist_name)),
            OutputLayout::ByPageArtist => spec.opts.out_dir.join(sanitize_stem(&page_artist)),
        };

        let fmt_ctx = ReleaseFormatContext {
            artist: &row.artist_name,
            page_artist: &page_artist,
            title: &row.title,
            published_at: row.published_at,
            kind: row.key.kind,
            format: chosen_fmt,
            item_id: row.key.id,
        };
        let custom_stem = format_release_stem(&settings.filename_pattern, &fmt_ctx);

        let dest_dir = if row.key.kind == ItemType::Track {
            let folder_name = custom_stem
                .clone()
                .unwrap_or_else(|| sanitize_stem(&format!("{} - {}", row.artist_name, row.title)));
            base_out.join(folder_name)
        } else {
            base_out
        };

        let fallback_ext = if row.key.kind == ItemType::Album {
            "zip"
        } else {
            chosen_fmt.track_ext()
        };

        let stream_res = stream_download(
            &self.client,
            &self.bridge,
            StreamRequest {
                job_id: spec.id,
                url: download_url,
                band_id: Some(row.band_id),
                dest_dir: &dest_dir,
                custom_stem: custom_stem.as_deref(),
                artist: &row.artist_name,
                title: &row.title,
                kind: row.key.kind,
                fallback_ext,
            },
            cancel,
        )
        .await;

        let outcome = match stream_res {
            Ok(o) => o,
            Err(e) => {
                let status = if matches!(e, DownloadError::Cancelled) {
                    HistoryStatus::Cancelled
                } else {
                    HistoryStatus::Failed
                };
                let msg = e.to_string();
                let _ = self
                    .pool
                    .writer
                    .call(move |c| history::finish(c, history_id, status, None, Some(&msg)))
                    .await;
                return Err(e);
            }
        };

        let mut final_local_path = outcome.final_path.clone();

        if row.key.kind == ItemType::Track {
            // Fetch cover.jpg for track; cover failure is a warning, not a job failure
            let cover_url = row
                .art_id
                .map(|id| self.client.cover_url_for_art_id(id))
                .or_else(|| row.cover_url.clone());

            if let Some(c_url) = cover_url {
                let cover_dest = dest_dir.join("cover.jpg");
                if let Err(e) = fetch_cover_to_path(&self.client, &c_url, &cover_dest).await {
                    self.bridge.log(
                        LogLevel::Warn,
                        "download",
                        format!("Could not download track cover.jpg for {}: {e}", row.title),
                        Some(row.key),
                    );
                }
            }
            final_local_path = dest_dir;
        } else if settings.auto_extract {
            self.bridge.emit(WorkerToUiEvent::JobState {
                job_id: spec.id,
                state: JobState::Extracting,
            });
            match extract_zip_archive(outcome.final_path, settings.delete_zip_after_extract).await {
                Ok(extracted_dir) => {
                    final_local_path = extracted_dir;
                }
                Err(e) => {
                    let msg = e.to_string();
                    let _ = self
                        .pool
                        .writer
                        .call(move |c| {
                            history::finish(
                                c,
                                history_id,
                                HistoryStatus::Failed,
                                Some(outcome.bytes),
                                Some(&msg),
                            )
                        })
                        .await;
                    return Err(e);
                }
            }
        }

        let key = row.key;
        let band_id = row.band_id;
        let path_for_db = final_local_path.clone();
        let bytes = outcome.bytes;
        let soft = settings.quota_soft;
        let hard = settings.quota_hard;

        let (updated_row, stats, used_30d) = self
            .pool
            .writer
            .call(move |c| {
                releases::commit_download(c, key, chosen_fmt, &path_for_db, bytes, history_id)?;
                let ur = releases::get_release(c, key)?;
                let st = artists::stats_with_quota(c, band_id)?;
                let used = st.quota_used_30d;
                Ok::<_, DbError>((ur, st, used))
            })
            .await?;

        if let Some(ur) = updated_row {
            self.bridge
                .emit(WorkerToUiEvent::ReleaseUpdated(Box::new(ur)));
        }
        self.bridge
            .emit(WorkerToUiEvent::ArtistStats { band_id, stats });
        self.bridge.emit(WorkerToUiEvent::QuotaStatus {
            band_id,
            used_30d,
            soft,
            hard,
        });

        Ok((final_local_path, bytes))
    }
}

fn pop_next_spec(st: &mut ManagerState) -> Option<JobSpec> {
    let n = st.artists_with_queue.len();
    for _ in 0..n {
        let band_id = st.artists_with_queue.pop_front()?;
        if st.active_bands.get(&band_id).copied().unwrap_or(0) > 0 {
            st.artists_with_queue.push_back(band_id);
            continue;
        }
        let spec_opt = st
            .per_artist_queue
            .get_mut(&band_id)
            .and_then(VecDeque::pop_front);

        let has_more = st
            .per_artist_queue
            .get(&band_id)
            .is_some_and(|q| !q.is_empty());
        if has_more {
            st.artists_with_queue.push_back(band_id);
        }

        if let Some(spec) = spec_opt {
            return Some(spec);
        }
    }
    None
}

fn map_download_error(err: &DownloadError) -> JobError {
    match err {
        DownloadError::Bandcamp(bc) => match bc {
            BcError::EmailRequired => JobError::EmailRequired,
            BcError::QuotaExceeded => JobError::QuotaExceeded,
            BcError::FormatUnavailable => JobError::FormatUnavailable,
            BcError::RateLimited { retry_after_s } => JobError::RateLimited {
                retry_after_s: *retry_after_s,
            },
            BcError::ApiRejected(msg) | BcError::InvalidRedeemUrl(msg) => {
                JobError::ApiRejected(msg.clone())
            }
            BcError::RedeemExpiredOrEmpty => {
                JobError::ApiRejected("Redeem link expired or already used".to_string())
            }
            other => JobError::Network(other.to_string()),
        },
        DownloadError::Db(e) => JobError::Io(e.to_string()),
        DownloadError::Io(e) => JobError::Io(e.to_string()),
        DownloadError::Zip(e) => JobError::Io(format!("Zip error: {e}")),
        DownloadError::UnsafeZipEntry(e) => JobError::Io(format!("Unsafe zip entry: {e}")),
        DownloadError::ZipLimitExceeded(e) => JobError::Io(e.clone()),
        DownloadError::Truncated { expected, actual } => {
            JobError::Io(format!("truncated: expected {expected}B, got {actual}B"))
        }
        DownloadError::Cancelled => JobError::Io("cancelled".to_string()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{
        bandcamp::gate::BandcampGate,
        db::repo::{
            artists::{self, UpsertArtist},
            releases::{DiscoveredRelease, ReleaseMetadata},
        },
        types::Settings,
    };
    use tokio::sync::mpsc;

    async fn setup_mgr() -> (
        tempfile::TempDir,
        DbPool,
        DownloadManager,
        mpsc::UnboundedReceiver<WorkerToUiEvent>,
        CancellationToken,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.sqlite");
        let pool = DbPool::open(&db_path).unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        let bridge = UiBridge::new(tx, None);
        let root_cancel = CancellationToken::new();
        let gate = BandcampGate::new(0, 0, 1, None);
        let client = BandcampClient::new(gate, "bcvault-test")
            .unwrap()
            .with_base_urls("http://127.0.0.1:9", "http://127.0.0.1:9");
        let settings = Settings {
            out_dir: dir.path().join("out"),
            ..Settings::default()
        };
        let mgr = DownloadManager::new(pool.clone(), client, bridge, settings, root_cancel.clone());
        (dir, pool, mgr, rx, root_cancel)
    }

    async fn seed_release(pool: &DbPool, band_id: i64, id: i64, downloaded: bool) -> ItemKey {
        let key = ItemKey {
            kind: ItemType::Album,
            id,
        };
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: &format!("a-{band_id}"),
                        name: &format!("Artist {band_id}"),
                        url: &format!("https://a-{band_id}.bandcamp.com"),
                        cover_url: None,
                    },
                )?;
                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key,
                        band_id,
                        title: &format!("Album {id}"),
                        artist_name: &format!("Artist {band_id}"),
                        url: &format!("https://a-{band_id}.bandcamp.com/album/a-{id}"),
                        cover_url: None,
                        art_id: None,
                    },
                )?;
                releases::update_metadata(
                    c,
                    &ReleaseMetadata {
                        key,
                        classification: Classification::Free,
                        pricing_json: Some(r#"{"minimum_price":0.0}"#),
                        is_preorder: false,
                        track_count: Some(3),
                        published_at: None,
                        modified_at: None,
                        art_id: None,
                    },
                )?;
                if downloaded {
                    releases::set_downloaded_manual(c, key, true)?;
                }
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();
        key
    }

    fn drain_events(rx: &mut mpsc::UnboundedReceiver<WorkerToUiEvent>) -> Vec<WorkerToUiEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    #[tokio::test]
    async fn test_3_enqueue_twice_while_paused_skips_in_flight() {
        let (dir, pool, mgr, mut rx, cancel) = setup_mgr().await;
        mgr.pause();
        let k1 = seed_release(&pool, 1, 101, false).await;
        let k2 = seed_release(&pool, 1, 102, false).await;
        let k3 = seed_release(&pool, 1, 103, false).await;
        let opts = DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: dir.path().join("out"),
            batch_limit: 50,
        };

        mgr.enqueue_downloads(vec![k1, k2, k3], opts.clone(), false, EnqueueMode::New)
            .await;
        let evs1 = drain_events(&mut rx);
        let queued1 = evs1
            .iter()
            .filter(|e| matches!(e, WorkerToUiEvent::JobQueued(_)))
            .count();
        assert_eq!(queued1, 3);
        assert_eq!(mgr.in_flight_len(), 3);

        mgr.enqueue_downloads(vec![k1, k2, k3], opts, false, EnqueueMode::New)
            .await;
        let evs2 = drain_events(&mut rx);
        let queued2 = evs2
            .iter()
            .filter(|e| matches!(e, WorkerToUiEvent::JobQueued(_)))
            .count();
        assert_eq!(queued2, 0);
        assert_eq!(mgr.in_flight_len(), 3);
        let summary2 = evs2
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::EnqueueSummary(s) => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!(summary2.requested, 3);
        assert_eq!(summary2.queued, 0);
        assert_eq!(summary2.skipped_in_flight, 3);

        mgr.cancel_all();
        assert_eq!(mgr.in_flight_len(), 0);
        cancel.cancel();
    }

    #[tokio::test]
    async fn test_4_in_flight_cleared_on_terminal_states_and_retry() {
        let (dir, pool, mgr, mut rx, cancel) = setup_mgr().await;
        mgr.pause();
        let k1 = seed_release(&pool, 1, 201, false).await;
        let k2 = seed_release(&pool, 1, 202, false).await;
        let k3 = seed_release(&pool, 2, 203, false).await;
        let opts = DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: dir.path().join("out"),
            batch_limit: 50,
        };

        mgr.enqueue_downloads(vec![k1, k2, k3], opts.clone(), false, EnqueueMode::New)
            .await;
        let evs = drain_events(&mut rx);
        let mut job_ids = Vec::new();
        for ev in evs {
            if let WorkerToUiEvent::JobQueued(j) = ev {
                job_ids.push((j.key, j.id));
            }
        }
        assert_eq!(mgr.in_flight_len(), 3);

        // Cancel single queued job
        let j1 = job_ids.iter().find(|(k, _)| *k == k1).unwrap().1;
        mgr.cancel_job(j1);
        assert!(!mgr.in_flight_contains(&k1));
        assert_eq!(mgr.in_flight_len(), 2);

        // Retry re-adds to in_flight
        mgr.retry_job(j1);
        assert!(mgr.in_flight_contains(&k1));
        assert_eq!(mgr.in_flight_len(), 3);

        // Block artist 1 drains artist 1's queued jobs (k1, k2)
        mgr.block_artist_for_30_mins(1);
        assert!(!mgr.in_flight_contains(&k1));
        assert!(!mgr.in_flight_contains(&k2));
        assert!(mgr.in_flight_contains(&k3));
        assert_eq!(mgr.in_flight_len(), 1);

        // Cancel all drains remaining (k3)
        mgr.cancel_all();
        assert_eq!(mgr.in_flight_len(), 0);

        // Failed job (active worker hitting closed port 127.0.0.1:9) leaves in_flight
        mgr.resume();
        mgr.enqueue_downloads(vec![k3], opts, false, EnqueueMode::New)
            .await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(WorkerToUiEvent::JobState {
                    state: JobState::Failed(_),
                    ..
                }) = rx.recv().await
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        // Wait for dispatcher cleanup to clear in_flight after JobState::Failed is emitted
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while mgr.in_flight_len() > 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert_eq!(mgr.in_flight_len(), 0);

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_5_new_skips_downloaded_force_queues_it() {
        let (dir, pool, mgr, mut rx, cancel) = setup_mgr().await;
        mgr.pause();
        let k1 = seed_release(&pool, 1, 301, true).await;
        let opts = DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: dir.path().join("out"),
            batch_limit: 50,
        };

        mgr.enqueue_downloads(vec![k1], opts.clone(), false, EnqueueMode::New)
            .await;
        let evs1 = drain_events(&mut rx);
        assert!(
            !evs1
                .iter()
                .any(|e| matches!(e, WorkerToUiEvent::JobQueued(_)))
        );
        assert_eq!(mgr.in_flight_len(), 0);

        mgr.enqueue_downloads(vec![k1], opts, false, EnqueueMode::Force)
            .await;
        let evs2 = drain_events(&mut rx);
        let queued2 = evs2
            .iter()
            .filter(|e| matches!(e, WorkerToUiEvent::JobQueued(_)))
            .count();
        assert_eq!(queued2, 1);
        assert_eq!(mgr.in_flight_len(), 1);

        mgr.cancel_all();
        assert_eq!(mgr.in_flight_len(), 0);
        cancel.cancel();
    }

    async fn seed_quota_usage(pool: &DbPool, band_id: i64, count: u32) {
        pool.writer
            .call(move |c| {
                for i in 0..count {
                    let key = ItemKey {
                        kind: ItemType::Album,
                        id: 90_000 + i64::from(i),
                    };
                    let hid = history::insert_started(
                        c,
                        &NewHistoryEntry {
                            key,
                            band_id,
                            format: AudioFormat::Flac,
                            source: DownloadSource::Api,
                            counts_against_quota: true,
                        },
                    )?;
                    history::finish(c, hid, HistoryStatus::Success, None, None)?;
                }
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_6_batch_limit_defers_excess_and_totals_match_requested() {
        let (dir, pool, mgr, mut rx, cancel) = setup_mgr().await;
        mgr.pause();
        let mut keys = Vec::new();
        for i in 0..40 {
            keys.push(seed_release(&pool, 1, 400 + i, false).await);
        }
        let opts = DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: dir.path().join("out"),
            batch_limit: 25,
        };

        mgr.enqueue_downloads(keys, opts, false, EnqueueMode::New)
            .await;
        let evs = drain_events(&mut rx);
        let summary = evs
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::EnqueueSummary(s) => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!(summary.requested, 40);
        assert_eq!(summary.queued, 25);
        assert_eq!(
            summary.deferred,
            vec![DeferredBatch {
                band_id: 1,
                count: 15,
                cause: DeferCause::BatchLimit,
            }]
        );
        let total = summary.queued
            + summary.skipped_downloaded
            + summary.skipped_in_flight
            + summary.skipped_ineligible
            + summary.awaiting_ack
            + summary.deferred_total();
        assert_eq!(total, summary.requested);

        mgr.cancel_all();
        cancel.cancel();
    }

    #[tokio::test]
    async fn test_7_hard_cap_and_blocked_30_min_deferrals() {
        let (dir, pool, mgr, mut rx, cancel) = setup_mgr().await;
        mgr.pause();
        seed_quota_usage(&pool, 1, 195).await;
        let mut keys = Vec::new();
        for i in 0..20 {
            keys.push(seed_release(&pool, 1, 500 + i, false).await);
        }
        let opts = DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: dir.path().join("out"),
            batch_limit: 50,
        };

        mgr.enqueue_downloads(keys.clone(), opts.clone(), true, EnqueueMode::New)
            .await;
        let evs = drain_events(&mut rx);
        let summary = evs
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::EnqueueSummary(s) => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!(summary.queued, 5);
        assert_eq!(
            summary.deferred,
            vec![DeferredBatch {
                band_id: 1,
                count: 15,
                cause: DeferCause::HardCap,
            }]
        );

        mgr.cancel_all();

        // At 200 used: decide returns Blocked -> HardCap deferral
        seed_quota_usage(&pool, 2, 200).await;
        let k200 = seed_release(&pool, 2, 600, false).await;
        mgr.enqueue_downloads(vec![k200], opts.clone(), true, EnqueueMode::New)
            .await;
        let evs2 = drain_events(&mut rx);
        let summary2 = evs2
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::EnqueueSummary(s) => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!(summary2.queued, 0);
        assert_eq!(
            summary2.deferred,
            vec![DeferredBatch {
                band_id: 2,
                count: 1,
                cause: DeferCause::HardCap,
            }]
        );

        // During 30-min block -> Blocked30Min deferral
        mgr.block_artist_for_30_mins(1);
        mgr.enqueue_downloads(vec![keys[0]], opts, true, EnqueueMode::New)
            .await;
        let evs3 = drain_events(&mut rx);
        let summary3 = evs3
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::EnqueueSummary(s) => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            summary3.deferred,
            vec![DeferredBatch {
                band_id: 1,
                count: 1,
                cause: DeferCause::Blocked30Min,
            }]
        );

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_8_soft_limit_awaiting_ack_then_ack_queues() {
        let (dir, pool, mgr, mut rx, cancel) = setup_mgr().await;
        mgr.pause();
        seed_quota_usage(&pool, 1, 145).await;
        let mut keys = Vec::new();
        for i in 0..10 {
            keys.push(seed_release(&pool, 1, 700 + i, false).await);
        }
        let opts = DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: dir.path().join("out"),
            batch_limit: 50,
        };

        mgr.enqueue_downloads(keys.clone(), opts.clone(), false, EnqueueMode::New)
            .await;
        let evs1 = drain_events(&mut rx);
        assert!(
            evs1.iter()
                .any(|e| matches!(e, WorkerToUiEvent::QuotaWarning { .. }))
        );
        let summary1 = evs1
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::EnqueueSummary(s) => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!(summary1.queued, 0);
        assert_eq!(summary1.awaiting_ack, 10);
        assert_eq!(mgr.in_flight_len(), 0);

        mgr.enqueue_downloads(keys, opts, true, EnqueueMode::New)
            .await;
        let evs2 = drain_events(&mut rx);
        let summary2 = evs2
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::EnqueueSummary(s) => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!(summary2.queued, 10);
        assert_eq!(summary2.awaiting_ack, 0);
        assert_eq!(mgr.in_flight_len(), 10);

        mgr.cancel_all();
        cancel.cancel();
    }

    async fn seed_release_with(
        pool: &DbPool,
        band_id: i64,
        id: i64,
        classification: Classification,
        is_preorder: bool,
        downloaded: bool,
    ) -> ItemKey {
        let key = ItemKey {
            kind: ItemType::Album,
            id,
        };
        pool.writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: &format!("a-{band_id}"),
                        name: &format!("Artist {band_id}"),
                        url: &format!("https://a-{band_id}.bandcamp.com"),
                        cover_url: None,
                    },
                )?;
                releases::upsert_discovered(
                    c,
                    &DiscoveredRelease {
                        key,
                        band_id,
                        title: &format!("Album {id}"),
                        artist_name: &format!("Artist {band_id}"),
                        url: &format!("https://a-{band_id}.bandcamp.com/album/a-{id}"),
                        cover_url: None,
                        art_id: None,
                    },
                )?;
                releases::update_metadata(
                    c,
                    &ReleaseMetadata {
                        key,
                        classification,
                        pricing_json: None,
                        is_preorder,
                        track_count: Some(3),
                        published_at: None,
                        modified_at: None,
                        art_id: None,
                    },
                )?;
                if downloaded {
                    releases::set_downloaded_manual(c, key, true)?;
                }
                Ok::<_, DbError>(())
            })
            .await
            .unwrap();
        key
    }

    #[tokio::test]
    async fn test_10_enqueue_remaining_artists_filters_to_candidates() {
        let (dir, pool, mgr, mut rx, cancel) = setup_mgr().await;
        mgr.pause();
        let k_free_new = seed_release_with(&pool, 1, 801, Classification::Free, false, false).await;
        let _k_free_dl = seed_release_with(&pool, 1, 802, Classification::Free, false, true).await;
        let _k_paid = seed_release_with(&pool, 1, 803, Classification::Paid, false, false).await;
        let _k_pre = seed_release_with(&pool, 1, 804, Classification::Free, true, false).await;
        let _k_email =
            seed_release_with(&pool, 1, 805, Classification::EmailRequired, false, false).await;
        let _other_artist =
            seed_release_with(&pool, 2, 806, Classification::Free, false, false).await;

        let opts = DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: dir.path().join("out"),
            batch_limit: 50,
        };
        mgr.enqueue_remaining(RemainingScope::Artists(vec![1]), opts, false)
            .await;

        let evs = drain_events(&mut rx);
        let queued: Vec<ItemKey> = evs
            .iter()
            .filter_map(|e| match e {
                WorkerToUiEvent::JobQueued(j) => Some(j.key),
                _ => None,
            })
            .collect();
        assert_eq!(queued, vec![k_free_new]);

        mgr.cancel_all();
        cancel.cancel();
    }

    #[tokio::test]
    async fn test_11_enqueue_remaining_library_twice_skips_in_flight() {
        let (dir, pool, mgr, mut rx, cancel) = setup_mgr().await;
        mgr.pause();
        let _k1 = seed_release_with(&pool, 1, 901, Classification::Free, false, false).await;
        let _k2 = seed_release_with(&pool, 2, 902, Classification::Nyp, false, false).await;
        let _k3 = seed_release_with(&pool, 3, 903, Classification::Free, false, false).await;
        let _k4 = seed_release_with(&pool, 3, 904, Classification::Free, false, true).await;

        let opts = DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: dir.path().join("out"),
            batch_limit: 50,
        };

        mgr.enqueue_remaining(RemainingScope::Library, opts.clone(), false)
            .await;
        let evs1 = drain_events(&mut rx);
        let s1 = evs1
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::EnqueueSummary(s) => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!(s1.queued, 3);
        assert_eq!(mgr.in_flight_len(), 3);

        mgr.enqueue_remaining(RemainingScope::Library, opts, false)
            .await;
        let evs2 = drain_events(&mut rx);
        let s2 = evs2
            .iter()
            .find_map(|e| match e {
                WorkerToUiEvent::EnqueueSummary(s) => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!(s2.queued, 0);
        assert_eq!(s2.skipped_in_flight, 3);

        mgr.cancel_all();
        cancel.cancel();
    }
}

use crate::{
    library::service::ReconcileReport,
    types::{
        ArtistFolder, ArtistView, DeletedPolicy, DownloadOpts, FolderState, ItemKey, JobId,
        JobState, JobView, LogEntry, LogLevel, MissingItem, RateLimitStatus, ReleaseFilter,
        ReleaseQuery, ReleaseRow, Settings, SortDir, SortKey, SyncMode, SyncProgress,
        UnavailableFolder, UnavailableRoot,
    },
    ui::{thumbs::ThumbCache, widgets::format_date},
    worker::messages::{UiToWorkerCommand, WorkerToUiEvent},
};
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug)]
pub struct CachedRow {
    pub row: ReleaseRow,
    pub artist_arc: Arc<str>,
    pub date_str: Arc<str>,
    pub tracks_str: Arc<str>,
}

impl CachedRow {
    pub fn from_row(row: ReleaseRow, artist_interner: &mut HashMap<String, Arc<str>>) -> Self {
        let artist_arc = artist_interner
            .entry(row.artist_name.clone())
            .or_insert_with(|| Arc::from(row.artist_name.as_str()))
            .clone();
        let date_str: Arc<str> = Arc::from(format_date(row.published_at).as_str());
        let tracks_str: Arc<str> = match row.track_count {
            Some(n) if n > 0 => Arc::from(n.to_string().as_str()),
            _ => Arc::from("-"),
        };
        Self {
            row,
            artist_arc,
            date_str,
            tracks_str,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct JobProgress {
    pub bytes: u64,
    pub total: Option<u64>,
    pub bytes_per_sec: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockTab {
    Queue,
    Errors,
}

#[derive(Clone, Debug)]
pub struct RedeemModalState {
    pub target: ItemKey,
    pub title: String,
    pub url_input: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct QuotaWarningModalState {
    pub band_id: i64,
    pub used_30d: u32,
    pub requested: u32,
    pub soft: u32,
    pub hard: u32,
    pub items: Vec<ItemKey>,
    pub opts: DownloadOpts,
}

#[derive(Clone, Debug)]
pub struct RenameArtistModalState {
    pub band_id: i64,
    pub original_name: String,
    pub name_input: String,
}

#[derive(Clone, Debug)]
pub struct LinkFolderModalState {
    pub band_id: Option<i64>,
    pub artist_input: String,
    pub folder_path: String,
    pub auto_apply_confident: bool,
    pub error: Option<String>,
    pub picked_folder: Arc<std::sync::Mutex<Option<std::path::PathBuf>>>,
    pub repoint_folder_id: Option<i64>,
}

impl LinkFolderModalState {
    pub fn new_empty(band_id: Option<i64>) -> Self {
        Self {
            band_id,
            artist_input: String::new(),
            folder_path: String::new(),
            auto_apply_confident: true,
            error: None,
            picked_folder: Arc::new(std::sync::Mutex::new(None)),
            repoint_folder_id: None,
        }
    }

    pub fn for_artist(
        band_id: i64,
        display_name: &str,
        existing_path: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            band_id: Some(band_id),
            artist_input: display_name.to_string(),
            folder_path: existing_path
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            auto_apply_confident: true,
            error: None,
            picked_folder: Arc::new(std::sync::Mutex::new(None)),
            repoint_folder_id: None,
        }
    }

    pub fn for_repoint(
        band_id: i64,
        display_name: &str,
        folder_id: i64,
        existing_path: &std::path::Path,
    ) -> Self {
        Self {
            band_id: Some(band_id),
            artist_input: display_name.to_string(),
            folder_path: existing_path.display().to_string(),
            auto_apply_confident: true,
            error: None,
            picked_folder: Arc::new(std::sync::Mutex::new(None)),
            repoint_folder_id: Some(folder_id),
        }
    }
}

pub fn worst_folder_state(folders: &[ArtistFolder]) -> Option<FolderState> {
    if folders.is_empty() {
        return None;
    }
    if folders.iter().any(|f| f.state == FolderState::Missing) {
        Some(FolderState::Missing)
    } else if folders.iter().any(|f| f.state == FolderState::Unreadable) {
        Some(FolderState::Unreadable)
    } else {
        Some(FolderState::Ok)
    }
}

pub fn folder_display_label(folder: &ArtistFolder, all_folders: &[ArtistFolder]) -> String {
    let base_name = folder
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.path.display().to_string());
    let duplicate_count = all_folders
        .iter()
        .filter(|f| f.path.file_name().map(|n| n.to_string_lossy()).as_deref() == Some(&base_name))
        .count();
    if duplicate_count > 1 {
        if let Some(parent) = folder.path.parent()
            && let Some(p_name) = parent.file_name()
        {
            format!("{}/{}", p_name.to_string_lossy(), base_name)
        } else {
            base_name
        }
    } else {
        base_name
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportTab {
    Matched,
    Review,
    PageOnly,
    LocalOnly,
    Conflicts,
    Restorable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalAction {
    Leave,
    Ignore,
    Attach(ItemKey),
}

#[derive(Clone, Debug)]
pub struct ReconcileReportModalState {
    pub report: Box<ReconcileReport>,
    pub active_tab: ReportTab,
    pub selected_matched: HashSet<ItemKey>,
    pub selected_review: HashMap<ItemKey, usize>,
    pub selected_restorable: HashSet<ItemKey>,
    pub local_decisions: HashMap<std::path::PathBuf, LocalAction>,
    pub local_attach_search: HashMap<std::path::PathBuf, String>,
}

impl ReconcileReportModalState {
    pub fn new(report: Box<ReconcileReport>) -> Self {
        let selected_matched = report.matched.iter().map(|m| m.release_key).collect();
        let selected_restorable = report.restorable.iter().map(|m| m.release_key).collect();
        let mut selected_review = HashMap::new();
        for r in &report.review {
            if !r.candidates.is_empty() {
                selected_review.insert(r.release_key, 0);
            }
        }
        let local_decisions = HashMap::new();
        let local_attach_search = HashMap::new();

        let active_tab = if !report.matched.is_empty() {
            ReportTab::Matched
        } else if !report.review.is_empty() {
            ReportTab::Review
        } else if !report.restorable.is_empty() {
            ReportTab::Restorable
        } else if !report.local_only.is_empty() {
            ReportTab::LocalOnly
        } else if !report.conflicts.is_empty() {
            ReportTab::Conflicts
        } else {
            ReportTab::PageOnly
        };

        Self {
            report,
            active_tab,
            selected_matched,
            selected_review,
            selected_restorable,
            local_decisions,
            local_attach_search,
        }
    }

    pub fn total_selected_count(&self) -> usize {
        let matched_cnt = self.selected_matched.len();
        let review_cnt = self.selected_review.len();
        let restorable_cnt = self.selected_restorable.len();
        let local_cnt = self
            .local_decisions
            .values()
            .filter(|a| !matches!(a, LocalAction::Leave))
            .count();
        matched_cnt + review_cnt + restorable_cnt + local_cnt
    }
}

#[derive(Clone, Debug)]
pub struct UndoToast {
    pub batch_id: u64,
    pub message: String,
    pub created_at: Instant,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MissingItemChoice {
    Moved(std::path::PathBuf),
    Deleted,
    Skip,
}

#[derive(Clone)]
pub struct ResolveMissingModalState {
    pub batch_id: u64,
    pub items: Vec<MissingItem>,
    pub mass_missing: bool,
    pub decisions: HashMap<ItemKey, MissingItemChoice>,
    pub deleted_policy: DeletedPolicy,
    pub make_default: bool,
    pub stop_asking: bool,
    pub picked_folder: Arc<std::sync::Mutex<Option<(ItemKey, std::path::PathBuf)>>>,
    pub repoint_picked_folder: Arc<std::sync::Mutex<Option<(i64, std::path::PathBuf)>>>,
    pub repoint_selected_band_id: Option<i64>,
}

impl ResolveMissingModalState {
    pub fn new(
        batch_id: u64,
        items: Vec<MissingItem>,
        mass_missing: bool,
        default_policy: DeletedPolicy,
    ) -> Self {
        let mut decisions = HashMap::new();
        for item in &items {
            if let Some(cand) = &item.best_candidate {
                decisions.insert(item.key, MissingItemChoice::Moved(cand.path.clone()));
            } else {
                decisions.insert(item.key, MissingItemChoice::Deleted);
            }
        }
        let repoint_selected_band_id = items.first().map(|it| it.band_id);
        Self {
            batch_id,
            items,
            mass_missing,
            decisions,
            deleted_policy: default_policy,
            make_default: false,
            stop_asking: false,
            picked_folder: Arc::new(std::sync::Mutex::new(None)),
            repoint_picked_folder: Arc::new(std::sync::Mutex::new(None)),
            repoint_selected_band_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnavailableCardState {
    pub folder: UnavailableFolder,
    pub policy: DeletedPolicy,
    pub unlink: bool,
    pub expanded: bool,
}

#[derive(Clone)]
pub struct UnavailableModalState {
    pub batch_id: u64,
    pub cards: Vec<UnavailableCardState>,
    pub picked_folder: Arc<std::sync::Mutex<Option<(i64, std::path::PathBuf)>>>,
}

impl UnavailableModalState {
    pub fn new(
        batch_id: u64,
        folders: Vec<UnavailableFolder>,
        default_policy: DeletedPolicy,
    ) -> Self {
        let cards = folders
            .into_iter()
            .map(|folder| {
                let unlink = matches!(folder.root, UnavailableRoot::Linked { .. });
                UnavailableCardState {
                    folder,
                    policy: default_policy,
                    unlink,
                    expanded: false,
                }
            })
            .collect();
        Self {
            batch_id,
            cards,
            picked_folder: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

pub struct UiState {
    pub artists: Vec<ArtistView>,
    pub artist_filter: String,
    pub selected_artists: BTreeSet<i64>,
    pub last_clicked_artist: Option<i64>,
    pub add_artist_input: String,
    pub add_artist_pending: bool,
    pub sync_mode: SyncMode,
    pub active_syncs: HashMap<i64, SyncProgress>,

    pub rows: Vec<CachedRow>,
    pub index: HashMap<ItemKey, usize>,
    pub artist_interner: HashMap<String, Arc<str>>,
    pub latest_request_id: u64,
    pub filter: ReleaseFilter,
    pub search: String,
    pub sort: SortKey,
    pub dir: SortDir,
    pub query_dirty_at: Option<Instant>,
    pub query_immediate: bool,

    pub selected: HashSet<ItemKey>,
    pub last_clicked_row: Option<usize>,

    pub jobs: Vec<JobView>,
    pub job_index: HashMap<JobId, usize>,
    pub active_item_jobs: HashSet<ItemKey>,
    pub progress: HashMap<JobId, JobProgress>,
    pub queue_paused: bool,

    pub dock_tab: DockTab,
    pub logs: VecDeque<LogEntry>,
    pub log_filter: Option<LogLevel>,
    pub rate_limit: RateLimitStatus,

    pub settings: Settings,
    pub settings_draft: Option<Settings>,
    pub folders: HashMap<i64, Vec<ArtistFolder>>,
    pub redeem_modal: Option<RedeemModalState>,
    pub quota_modal: Option<QuotaWarningModalState>,
    pub confirm_remove_artist: Option<(i64, String)>,
    pub rename_artist_modal: Option<RenameArtistModalState>,
    pub link_folder_modal: Option<LinkFolderModalState>,
    pub reconcile_report_modal: Option<ReconcileReportModalState>,
    pub undo_toast: Option<UndoToast>,
    pub resolve_missing_modal: Option<ResolveMissingModalState>,
    pub show_resolve_missing_window: bool,
    pub unavailable_modal: Option<UnavailableModalState>,
    pub show_unavailable_window: bool,
    pub relink_picked_folder: Arc<std::sync::Mutex<Option<(ItemKey, std::path::PathBuf)>>>,

    pub thumbs: ThumbCache,
    pub focus_search_next_frame: bool,
    pub show_debug_overlay: bool,
    pub last_event_backlog: usize,
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

impl UiState {
    pub fn new() -> Self {
        Self {
            artists: Vec::new(),
            artist_filter: String::new(),
            selected_artists: BTreeSet::new(),
            last_clicked_artist: None,
            add_artist_input: String::new(),
            add_artist_pending: false,
            sync_mode: SyncMode::Incremental,
            active_syncs: HashMap::new(),

            rows: Vec::new(),
            index: HashMap::new(),
            artist_interner: HashMap::new(),
            latest_request_id: 0,
            filter: ReleaseFilter::All,
            search: String::new(),
            sort: SortKey::Released,
            dir: SortDir::Desc,
            query_dirty_at: None,
            query_immediate: true,

            selected: HashSet::new(),
            last_clicked_row: None,

            jobs: Vec::new(),
            job_index: HashMap::new(),
            active_item_jobs: HashSet::new(),
            progress: HashMap::new(),
            queue_paused: false,

            dock_tab: DockTab::Queue,
            logs: VecDeque::with_capacity(2000),
            log_filter: None,
            rate_limit: RateLimitStatus::Idle,

            settings: Settings::default(),
            settings_draft: None,
            folders: HashMap::new(),
            redeem_modal: None,
            quota_modal: None,
            confirm_remove_artist: None,
            rename_artist_modal: None,
            link_folder_modal: None,
            reconcile_report_modal: None,
            undo_toast: None,
            resolve_missing_modal: None,
            show_resolve_missing_window: false,
            unavailable_modal: None,
            show_unavailable_window: false,
            relink_picked_folder: Arc::new(std::sync::Mutex::new(None)),

            thumbs: ThumbCache::new(),
            focus_search_next_frame: false,
            show_debug_overlay: false,
            last_event_backlog: 0,
        }
    }

    pub fn active_band_id(&self) -> Option<i64> {
        if self.selected_artists.len() == 1 {
            self.selected_artists.iter().next().copied()
        } else {
            None
        }
    }

    pub fn unavailable_folder_artists(&self) -> Vec<i64> {
        self.folders
            .iter()
            .filter_map(|(&band_id, folders)| {
                if folders.iter().any(|f| f.state != FolderState::Ok) {
                    Some(band_id)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn mark_query_immediate(&mut self) {
        self.query_immediate = true;
        self.query_dirty_at = None;
    }

    pub fn mark_query_debounced(&mut self) {
        self.query_dirty_at = Some(Instant::now());
    }

    pub fn flush_debounced_query(&mut self, cmd_tx: &UnboundedSender<UiToWorkerCommand>) {
        let should_send = self.query_immediate
            || self
                .query_dirty_at
                .is_some_and(|t| t.elapsed() >= Duration::from_millis(120));

        if should_send {
            self.query_immediate = false;
            self.query_dirty_at = None;
            self.latest_request_id = self.latest_request_id.wrapping_add(1);
            let query = ReleaseQuery {
                band_id: self.active_band_id(),
                filter: self.filter,
                search: self.search.clone(),
                sort: self.sort,
                dir: self.dir,
            };
            let _ = cmd_tx.send(UiToWorkerCommand::QueryReleases {
                request_id: self.latest_request_id,
                query,
            });
        }

        self.thumbs.flush_debounced(&self.settings, cmd_tx);
    }

    pub fn cooldown_visible(&self) -> bool {
        matches!(self.rate_limit, RateLimitStatus::Cooldown { .. })
    }

    pub fn push_log(&mut self, entry: LogEntry) {
        if self.logs.len() >= 2000 {
            self.logs.pop_front();
        }
        self.logs.push_back(entry);
    }

    pub fn apply_event(&mut self, ev: WorkerToUiEvent) {
        match ev {
            WorkerToUiEvent::Ready {
                artists,
                settings,
                folders,
            } => {
                self.artists = artists;
                self.settings = settings;
                self.folders.clear();
                for f in folders {
                    self.folders.entry(f.band_id).or_default().push(f);
                }
                self.mark_query_immediate();
            }
            WorkerToUiEvent::SettingsUpdated(settings) => {
                self.settings = settings;
            }
            WorkerToUiEvent::ArtistUpserted(view) => {
                self.add_artist_pending = false;
                if let Some(existing) = self.artists.iter_mut().find(|a| a.band_id == view.band_id)
                {
                    *existing = view;
                } else {
                    self.artists.push(view);
                }
                self.artists
                    .sort_by_key(|a| a.display_name().to_lowercase());
                self.mark_query_immediate();
            }
            WorkerToUiEvent::ArtistRemoved { band_id } => {
                self.artists.retain(|a| a.band_id != band_id);
                self.selected_artists.remove(&band_id);
                self.active_syncs.remove(&band_id);
                self.folders.remove(&band_id);
                self.mark_query_immediate();
            }
            WorkerToUiEvent::ArtistStats { band_id, stats } => {
                if let Some(a) = self.artists.iter_mut().find(|a| a.band_id == band_id) {
                    a.stats = stats;
                }
            }
            WorkerToUiEvent::ReleasesLoaded { request_id, rows } => {
                if request_id != self.latest_request_id {
                    return; // discard stale query result
                }
                self.rows.clear();
                self.index.clear();
                self.rows.reserve(rows.len());
                for (idx, r) in rows.into_iter().enumerate() {
                    let key = r.key;
                    let cached = CachedRow::from_row(r, &mut self.artist_interner);
                    self.rows.push(cached);
                    self.index.insert(key, idx);
                }
                // Keep selection restricted to existing keys or clear stale ones
                self.selected.retain(|k| self.index.contains_key(k));
                self.last_clicked_row = None;
            }
            WorkerToUiEvent::ReleaseUpdated(row) => {
                let key = row.key;
                if let Some(&idx) = self.index.get(&key) {
                    if let Some(slot) = self.rows.get_mut(idx) {
                        *slot = CachedRow::from_row(*row, &mut self.artist_interner);
                    }
                } else {
                    // New release discovered during sync; if it matches active artist scope and filter All, insert or trigger query refresh
                    let band_matches = self.active_band_id().is_none_or(|bid| bid == row.band_id);
                    if band_matches
                        && self.filter == ReleaseFilter::All
                        && self.search.trim().is_empty()
                    {
                        let idx = self.rows.len();
                        let cached = CachedRow::from_row(*row, &mut self.artist_interner);
                        self.rows.push(cached);
                        self.index.insert(key, idx);
                    } else if band_matches {
                        self.mark_query_debounced();
                    }
                }
            }
            WorkerToUiEvent::SyncProgress(prog) => {
                self.active_syncs.insert(prog.band_id, prog);
            }
            WorkerToUiEvent::SyncFinished { band_id, error, .. } => {
                self.add_artist_pending = false;
                self.active_syncs.remove(&band_id);
                if error.is_some() {
                    self.dock_tab = DockTab::Errors;
                }
            }
            WorkerToUiEvent::JobQueued(view) => {
                let id = view.id;
                let key = view.key;
                self.active_item_jobs.insert(key);
                let idx = self.jobs.len();
                self.jobs.push(view);
                self.job_index.insert(id, idx);
            }
            WorkerToUiEvent::JobState { job_id, state } => {
                if let Some(&idx) = self.job_index.get(&job_id)
                    && let Some(job) = self.jobs.get_mut(idx)
                {
                    let is_terminal = matches!(
                        state,
                        JobState::Done { .. } | JobState::Failed(_) | JobState::Cancelled
                    );
                    if is_terminal {
                        self.active_item_jobs.remove(&job.key);
                    } else {
                        self.active_item_jobs.insert(job.key);
                    }
                    job.state = state;
                }
            }
            WorkerToUiEvent::DownloadByteProgress {
                job_id,
                bytes,
                total,
                bytes_per_sec,
            } => {
                self.progress.insert(
                    job_id,
                    JobProgress {
                        bytes,
                        total,
                        bytes_per_sec,
                    },
                );
            }
            WorkerToUiEvent::QuotaStatus {
                band_id, used_30d, ..
            } => {
                if let Some(a) = self.artists.iter_mut().find(|a| a.band_id == band_id) {
                    a.stats.quota_used_30d = used_30d;
                }
            }
            WorkerToUiEvent::QuotaWarning {
                band_id,
                used_30d,
                requested,
                soft,
                hard,
                items,
                opts,
            } => {
                self.quota_modal = Some(QuotaWarningModalState {
                    band_id,
                    used_30d,
                    requested,
                    soft,
                    hard,
                    items,
                    opts,
                });
            }
            WorkerToUiEvent::RateLimit(status) => {
                self.rate_limit = status;
            }
            WorkerToUiEvent::ThumbReady { key, path } => {
                self.thumbs.on_thumb_ready(key, path);
            }
            WorkerToUiEvent::FolderStatus { band_id, folders } => {
                if folders.is_empty() {
                    self.folders.remove(&band_id);
                } else {
                    self.folders.insert(band_id, folders);
                }
            }
            WorkerToUiEvent::ReconcileReport { report } => {
                self.reconcile_report_modal = Some(ReconcileReportModalState::new(report));
            }
            WorkerToUiEvent::LibraryApplied { batch_id, summary } => {
                if batch_id > 0 {
                    let mut parts = Vec::new();
                    if summary.adopted > 0 {
                        parts.push(format!("{} adopted", summary.adopted));
                    }
                    if summary.relinked > 0 {
                        parts.push(format!("{} relinked", summary.relinked));
                    }
                    if summary.archived > 0 {
                        parts.push(format!("{} archived", summary.archived));
                    }
                    if summary.restored > 0 {
                        parts.push(format!("{} restored", summary.restored));
                    }
                    if summary.ignored > 0 {
                        parts.push(format!("{} ignored", summary.ignored));
                    }
                    if summary.unmarked > 0 {
                        parts.push(format!("{} unmarked", summary.unmarked));
                    }
                    let summary_str = if parts.is_empty() {
                        "Library changes applied".to_string()
                    } else {
                        parts.join(", ")
                    };
                    self.undo_toast = Some(UndoToast {
                        batch_id,
                        message: summary_str,
                        created_at: Instant::now(),
                    });
                }
            }
            WorkerToUiEvent::LibraryProgress { .. } => {}
            WorkerToUiEvent::MissingFound {
                batch_id,
                items,
                mass_missing,
            } => {
                if !items.is_empty() {
                    self.resolve_missing_modal = Some(ResolveMissingModalState::new(
                        batch_id,
                        items,
                        mass_missing,
                        self.settings.deleted_policy,
                    ));
                    self.show_resolve_missing_window = true;
                } else {
                    self.resolve_missing_modal = None;
                    self.show_resolve_missing_window = false;
                }
            }
            WorkerToUiEvent::FolderUnavailable {
                batch_id,
                folders,
                prompt,
            } => {
                if prompt && !folders.is_empty() {
                    self.unavailable_modal = Some(UnavailableModalState::new(
                        batch_id,
                        folders,
                        self.settings.deleted_policy,
                    ));
                    self.show_unavailable_window = true;
                }
            }
            WorkerToUiEvent::Log(entry) => {
                self.push_log(entry);
            }
        }
    }

    pub fn clear_finished_jobs(&mut self) {
        self.jobs.retain(|j| {
            !matches!(
                j.state,
                JobState::Done { .. } | JobState::Failed(_) | JobState::Cancelled
            )
        });
        self.job_index.clear();
        for (idx, j) in self.jobs.iter().enumerate() {
            self.job_index.insert(j.id, idx);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::{Classification, ItemType};

    #[test]
    fn discards_stale_release_query_results() {
        let mut st = UiState::new();
        st.latest_request_id = 5;

        let sample_row = ReleaseRow {
            key: ItemKey::new(ItemType::Album, 1),
            band_id: 1,
            title: "Title".to_string(),
            artist_name: "Artist".to_string(),
            url: "https://a.bandcamp.com/album/t".to_string(),
            cover_url: None,
            art_id: None,
            classification: Classification::Free,
            is_preorder: false,
            track_count: Some(3),
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
        };

        // Stale request_id = 4 is ignored
        st.apply_event(WorkerToUiEvent::ReleasesLoaded {
            request_id: 4,
            rows: vec![sample_row.clone()],
        });
        assert!(st.rows.is_empty());

        // Matching request_id = 5 is applied
        st.apply_event(WorkerToUiEvent::ReleasesLoaded {
            request_id: 5,
            rows: vec![sample_row],
        });
        assert_eq!(st.rows.len(), 1);
    }

    #[test]
    fn test_folders_lifecycle_in_state() {
        use crate::types::{ArtistFolder, FolderState};
        use std::path::PathBuf;

        let mut st = UiState::new();
        assert!(st.folders.is_empty());

        let f1 = ArtistFolder {
            id: 1,
            band_id: 10,
            path: PathBuf::from("/music/artist10"),
            state: FolderState::Ok,
        };
        let f2 = ArtistFolder {
            id: 2,
            band_id: 20,
            path: PathBuf::from("/music/artist20"),
            state: FolderState::Missing,
        };

        // 1. Ready populates folders
        st.apply_event(WorkerToUiEvent::Ready {
            artists: vec![],
            settings: Settings::default(),
            folders: vec![f1.clone(), f2.clone()],
        });
        assert_eq!(st.folders.len(), 2);
        assert_eq!(st.folders.get(&10).unwrap()[0].state, FolderState::Ok);
        assert_eq!(st.folders.get(&20).unwrap()[0].state, FolderState::Missing);

        // 2. FolderStatus updates an existing folder state
        let f2_updated = ArtistFolder {
            id: 2,
            band_id: 20,
            path: PathBuf::from("/music/artist20"),
            state: FolderState::Ok,
        };
        st.apply_event(WorkerToUiEvent::FolderStatus {
            band_id: 20,
            folders: vec![f2_updated],
        });
        assert_eq!(st.folders.get(&20).unwrap()[0].state, FolderState::Ok);

        // 3. FolderStatus with empty folders removes the folder (unlink)
        st.apply_event(WorkerToUiEvent::FolderStatus {
            band_id: 10,
            folders: vec![],
        });
        assert!(!st.folders.contains_key(&10));
        assert_eq!(st.folders.len(), 1);

        // 4. ArtistRemoved cleans up remaining linked folder
        st.apply_event(WorkerToUiEvent::ArtistRemoved { band_id: 20 });
        assert!(!st.folders.contains_key(&20));
        assert!(st.folders.is_empty());
    }

    #[test]
    fn test_library_settings_defaults_and_clamping() {
        let mut s = Settings::default();
        assert!(s.verify_on_sync);
        assert!(s.rescan_linked_on_sync);
        assert!(s.auto_relink);
        assert_eq!(s.on_missing, crate::types::OnMissing::Ask);
        assert_eq!(s.deleted_policy, crate::types::DeletedPolicy::KeepMarked);
        assert_eq!(s.auto_match_min, 0.92);
        assert_eq!(s.review_match_min, 0.75);

        // Clamping review_match_min to auto_match_min
        s.auto_match_min = 0.60;
        s.review_match_min = 0.80;
        s.review_match_min = s.review_match_min.min(s.auto_match_min);
        assert_eq!(s.review_match_min, 0.60);

        // JSON serialization roundtrip
        let json = serde_json::to_string(&s).unwrap();
        let deserialized: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.verify_on_sync, s.verify_on_sync);
        assert_eq!(deserialized.auto_match_min, 0.60);
        assert_eq!(deserialized.review_match_min, 0.60);
    }

    #[test]
    fn test_link_folder_modal_state() {
        use std::path::PathBuf;

        let empty = LinkFolderModalState::new_empty(None);
        assert!(empty.band_id.is_none());
        assert!(empty.artist_input.is_empty());
        assert!(empty.folder_path.is_empty());
        assert!(empty.auto_apply_confident);

        let for_art =
            LinkFolderModalState::for_artist(42, "Aphex Twin", Some(PathBuf::from("/music/aphex")));
        assert_eq!(for_art.band_id, Some(42));
        assert_eq!(for_art.artist_input, "Aphex Twin");
        assert_eq!(for_art.folder_path, "/music/aphex");
        assert!(for_art.auto_apply_confident);

        // Async picked folder communication
        {
            let mut lock = for_art.picked_folder.lock().unwrap();
            *lock = Some(PathBuf::from("/new/path"));
        }
        let retrieved = for_art.picked_folder.lock().unwrap().take();
        assert_eq!(retrieved, Some(PathBuf::from("/new/path")));
        assert!(for_art.picked_folder.lock().unwrap().is_none());

        let repoint = LinkFolderModalState::for_repoint(
            42,
            "Aphex Twin",
            7,
            &PathBuf::from("/music/aphex_old"),
        );
        assert_eq!(repoint.band_id, Some(42));
        assert_eq!(repoint.repoint_folder_id, Some(7));
        assert_eq!(repoint.folder_path, "/music/aphex_old");
    }

    #[test]
    fn test_worst_folder_state_and_display_label() {
        use std::path::PathBuf;

        // 1. worst_folder_state
        assert_eq!(worst_folder_state(&[]), None);

        let f_ok = ArtistFolder {
            id: 1,
            band_id: 10,
            path: PathBuf::from("/ambient/Music"),
            state: FolderState::Ok,
        };
        let f_unreadable = ArtistFolder {
            id: 2,
            band_id: 10,
            path: PathBuf::from("/techno/Music"),
            state: FolderState::Unreadable,
        };
        let f_missing = ArtistFolder {
            id: 3,
            band_id: 10,
            path: PathBuf::from("/other/Archive"),
            state: FolderState::Missing,
        };

        assert_eq!(
            worst_folder_state(std::slice::from_ref(&f_ok)),
            Some(FolderState::Ok)
        );
        assert_eq!(
            worst_folder_state(&[f_ok.clone(), f_unreadable.clone()]),
            Some(FolderState::Unreadable)
        );
        assert_eq!(
            worst_folder_state(&[f_ok.clone(), f_missing.clone()]),
            Some(FolderState::Missing)
        );
        assert_eq!(
            worst_folder_state(&[f_missing.clone(), f_unreadable.clone()]),
            Some(FolderState::Missing)
        );

        // 2. folder_display_label
        // Single or distinct basename
        assert_eq!(
            folder_display_label(&f_missing, &[f_ok.clone(), f_missing.clone()]),
            "Archive"
        );

        // Duplicate basename: uses parent/basename
        let all = vec![f_ok.clone(), f_unreadable.clone()];
        assert_eq!(folder_display_label(&f_ok, &all), "ambient/Music");
        assert_eq!(folder_display_label(&f_unreadable, &all), "techno/Music");
    }

    #[test]
    fn test_reconcile_report_modal_state() {
        use crate::library::matcher::{MatchProposal, ReviewItem, Score, ScoredCandidate};
        use crate::types::{AudioFormat, BatchSummary, ItemType};
        use std::path::PathBuf;

        let key1 = ItemKey::new(ItemType::Album, 100);
        let key2 = ItemKey::new(ItemType::Album, 200);

        let report = Box::new(ReconcileReport {
            batch_id: 7,
            band_id: 42,
            matched: vec![MatchProposal {
                release_key: key1,
                release_title: "Selected Ambient Works".to_string(),
                candidate_path: PathBuf::from("/music/saw"),
                candidate_name: "Selected Ambient Works".to_string(),
                score: Score {
                    value: 0.98,
                    signals: vec![],
                },
                format_guess: Some(AudioFormat::Flac),
            }],
            review: vec![ReviewItem {
                release_key: key2,
                release_title: "Drukqs".to_string(),
                candidates: vec![ScoredCandidate {
                    candidate_path: PathBuf::from("/music/drukqs"),
                    candidate_name: "Drukqs".to_string(),
                    score: Score {
                        value: 0.85,
                        signals: vec![],
                    },
                    format_guess: Some(AudioFormat::Mp3_320),
                }],
            }],
            page_only: vec![],
            local_only: vec![],
            conflicts: vec![],
            restorable: vec![],
            linked: vec![],
            unavailable_folders: vec![],
        });

        let mut st = UiState::new();
        st.apply_event(WorkerToUiEvent::ReconcileReport { report });
        assert!(st.reconcile_report_modal.is_some());

        let modal = st.reconcile_report_modal.as_ref().unwrap();
        assert_eq!(modal.active_tab, ReportTab::Matched);
        assert!(modal.selected_matched.contains(&key1));
        assert_eq!(modal.selected_review.get(&key2), Some(&0));
        assert_eq!(modal.total_selected_count(), 2);

        // Apply event creates undo toast
        let summary = BatchSummary {
            adopted: 2,
            relinked: 0,
            archived: 0,
            unmarked: 0,
            restored: 0,
            ignored: 0,
        };
        st.apply_event(WorkerToUiEvent::LibraryApplied {
            batch_id: 7,
            summary,
        });
        assert!(st.undo_toast.is_some());
        let toast = st.undo_toast.as_ref().unwrap();
        assert_eq!(toast.batch_id, 7);
        assert!(toast.message.contains("2 adopted"));
    }

    #[test]
    fn test_resolve_missing_modal_state() {
        use crate::types::{DeletedPolicy, ItemType, MissingCandidate, MissingItem};
        use std::path::PathBuf;

        let key1 = ItemKey::new(ItemType::Album, 100);
        let key2 = ItemKey::new(ItemType::Album, 200);

        let items = vec![
            MissingItem {
                key: key1,
                band_id: 42,
                artist_name: "Aphex Twin".to_string(),
                title: "SAW II".to_string(),
                last_known_path: Some(PathBuf::from("/music/saw2")),
                best_candidate: Some(MissingCandidate {
                    path: PathBuf::from("/music/saw2_new"),
                    score: 0.75,
                }),
            },
            MissingItem {
                key: key2,
                band_id: 42,
                artist_name: "Aphex Twin".to_string(),
                title: "Syro".to_string(),
                last_known_path: Some(PathBuf::from("/music/syro")),
                best_candidate: None,
            },
        ];

        let mut st = UiState::new();
        st.apply_event(WorkerToUiEvent::MissingFound {
            batch_id: 12,
            items,
            mass_missing: true,
        });

        assert!(st.show_resolve_missing_window);
        assert!(st.resolve_missing_modal.is_some());

        let modal = st.resolve_missing_modal.as_ref().unwrap();
        assert_eq!(modal.batch_id, 12);
        assert!(modal.mass_missing);
        assert_eq!(modal.deleted_policy, DeletedPolicy::KeepMarked);
        assert_eq!(
            modal.decisions.get(&key1),
            Some(&MissingItemChoice::Moved(PathBuf::from("/music/saw2_new")))
        );
        assert_eq!(
            modal.decisions.get(&key2),
            Some(&MissingItemChoice::Deleted)
        );

        // Empty items clears modal and hides window
        st.apply_event(WorkerToUiEvent::MissingFound {
            batch_id: 13,
            items: vec![],
            mass_missing: false,
        });
        assert!(!st.show_resolve_missing_window);
        assert!(st.resolve_missing_modal.is_none());
    }

    #[test]
    fn test_unavailable_modal_state_and_chip() {
        use crate::types::{
            DeletedPolicy, FolderState, ItemType, UnavailableFolder, UnavailableRelease,
            UnavailableRoot,
        };
        use std::path::PathBuf;

        let mut st = UiState::new();

        let folder = UnavailableFolder {
            root: UnavailableRoot::Linked { folder_id: 42 },
            path: PathBuf::from("/music/aphex"),
            state: FolderState::Missing,
            releases: vec![UnavailableRelease {
                key: ItemKey::new(ItemType::Album, 100),
                band_id: 42,
                artist_name: "Aphex Twin".to_string(),
                title: "SAW II".to_string(),
                last_known_path: Some(PathBuf::from("/music/aphex/saw2")),
            }],
        };

        // 1. prompt: false does NOT open the modal
        st.apply_event(WorkerToUiEvent::FolderUnavailable {
            batch_id: 50,
            folders: vec![folder.clone()],
            prompt: false,
        });
        assert!(!st.show_unavailable_window);
        assert!(st.unavailable_modal.is_none());

        // 2. prompt: true opens the modal
        st.apply_event(WorkerToUiEvent::FolderUnavailable {
            batch_id: 51,
            folders: vec![folder],
            prompt: true,
        });
        assert!(st.show_unavailable_window);
        assert!(st.unavailable_modal.is_some());
        let modal = st.unavailable_modal.as_ref().unwrap();
        assert_eq!(modal.batch_id, 51);
        assert_eq!(modal.cards.len(), 1);
        assert!(modal.cards[0].unlink, "Linked roots default unlink to true");
        assert_eq!(modal.cards[0].policy, DeletedPolicy::KeepMarked);

        // 3. unavailable_folder_artists tracks state.folders with state != Ok
        assert_eq!(st.unavailable_folder_artists().len(), 0);
        st.folders.insert(
            42,
            vec![crate::types::ArtistFolder {
                id: 1,
                band_id: 42,
                path: PathBuf::from("/music/aphex"),
                state: FolderState::Missing,
            }],
        );
        st.folders.insert(
            43,
            vec![crate::types::ArtistFolder {
                id: 2,
                band_id: 43,
                path: PathBuf::from("/music/boc"),
                state: FolderState::Ok,
            }],
        );
        assert_eq!(st.unavailable_folder_artists(), vec![42]);
    }

    #[test]
    fn test_22_ui_state_multi_folder_lifecycle() {
        let mut st = UiState::default();
        let f1_a1 = ArtistFolder {
            id: 1,
            band_id: 10,
            path: std::path::PathBuf::from("/music/artist10_a"),
            state: FolderState::Ok,
        };
        let f2_a1 = ArtistFolder {
            id: 2,
            band_id: 10,
            path: std::path::PathBuf::from("/music/artist10_b"),
            state: FolderState::Missing,
        };
        let f3_a2 = ArtistFolder {
            id: 3,
            band_id: 20,
            path: std::path::PathBuf::from("/music/artist20"),
            state: FolderState::Ok,
        };

        // Ready with 3 folders over 2 artists
        st.apply_event(WorkerToUiEvent::Ready {
            artists: vec![],
            settings: Settings::default(),
            folders: vec![f1_a1.clone(), f2_a1.clone(), f3_a2.clone()],
        });

        assert_eq!(st.folders.get(&10).unwrap().len(), 2);
        assert_eq!(st.folders.get(&20).unwrap().len(), 1);

        // FolderStatus with shorter list for artist 10
        st.apply_event(WorkerToUiEvent::FolderStatus {
            band_id: 10,
            folders: vec![f1_a1],
        });
        assert_eq!(st.folders.get(&10).unwrap().len(), 1);
        assert_eq!(st.folders.get(&20).unwrap().len(), 1);

        // FolderStatus with empty list removes artist 10
        st.apply_event(WorkerToUiEvent::FolderStatus {
            band_id: 10,
            folders: vec![],
        });
        assert!(!st.folders.contains_key(&10));
        assert_eq!(st.folders.get(&20).unwrap().len(), 1);
    }
}

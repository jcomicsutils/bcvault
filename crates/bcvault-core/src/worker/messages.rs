use crate::types::{
    ArtistStats, ArtistView, DownloadOpts, ItemKey, JobId, JobState, JobView, LogEntry,
    RateLimitStatus, ReleaseQuery, ReleaseRow, Settings, SyncMode, SyncProgress, SyncScope,
};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum UiToWorkerCommand {
    // artists
    AddArtist {
        input: String,
    },
    RemoveArtist {
        band_id: i64,
    },
    RenameArtist {
        band_id: i64,
        custom_name: Option<String>,
    },
    // sync
    Sync {
        scope: SyncScope,
        mode: SyncMode,
    },
    CancelSync,
    // table data (answered on the reader connection)
    QueryReleases {
        request_id: u64,
        query: ReleaseQuery,
    },
    RequestThumbs {
        keys: Vec<ItemKey>,
    },
    // downloads
    EnqueueDownloads {
        items: Vec<ItemKey>,
        opts: DownloadOpts,
        ack_soft_quota: bool,
    },
    RedeemUrl {
        target: ItemKey,
        url: String,
        opts: DownloadOpts,
    },
    SetReleaseDownloaded {
        key: ItemKey,
        downloaded: bool,
    },
    CancelJob(JobId),
    RetryJob(JobId),
    CancelAllJobs,
    PauseQueue,
    ResumeQueue,
    ClearFinished,
    // library
    LinkArtistFolder {
        band_id: i64,
        path: PathBuf,
        auto_apply_confident: bool,
    },
    UnlinkArtistFolder {
        band_id: i64,
    },
    MatchArtistFolder {
        band_id: i64,
        auto_apply_confident: bool,
    },
    ApplyReconcile {
        batch_id: u64,
        decisions: Vec<crate::types::MatchDecision>,
    },
    UndoBatch {
        batch_id: u64,
    },
    CancelLibrary,
    // misc
    UpdateSettings(Settings),
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum WorkerToUiEvent {
    Ready {
        artists: Vec<ArtistView>,
        settings: Settings,
    },
    SettingsUpdated(Settings),
    ArtistUpserted(ArtistView),
    ArtistRemoved {
        band_id: i64,
    },
    ArtistStats {
        band_id: i64,
        stats: ArtistStats,
    },
    ReleasesLoaded {
        request_id: u64,
        rows: Vec<ReleaseRow>,
    },
    ReleaseUpdated(Box<ReleaseRow>),
    SyncProgress(SyncProgress),
    SyncFinished {
        band_id: i64,
        new_items: u32,
        updated: u32,
        error: Option<String>,
    },
    JobQueued(JobView),
    JobState {
        job_id: JobId,
        state: JobState,
    },
    DownloadByteProgress {
        job_id: JobId,
        bytes: u64,
        total: Option<u64>,
        bytes_per_sec: f64,
    },
    QuotaStatus {
        band_id: i64,
        used_30d: u32,
        soft: u32,
        hard: u32,
    },
    QuotaWarning {
        band_id: i64,
        used_30d: u32,
        requested: u32,
        soft: u32,
        hard: u32,
        items: Vec<ItemKey>,
        opts: DownloadOpts,
    },
    RateLimit(RateLimitStatus),
    ThumbReady {
        key: ItemKey,
        path: PathBuf,
    },
    LibraryProgress {
        phase: crate::types::LibraryPhase,
        done: u32,
        total: u32,
        current: Option<String>,
    },
    ReconcileReport {
        report: Box<crate::library::ReconcileReport>,
    },
    FolderStatus {
        band_id: i64,
        state: crate::types::FolderState,
    },
    LibraryApplied {
        batch_id: u64,
        summary: crate::types::BatchSummary,
    },
    Log(LogEntry),
}

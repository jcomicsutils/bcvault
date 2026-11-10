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
    /// Link an artist folder (adds a linked folder to the artist).
    LinkArtistFolder {
        band_id: i64,
        path: PathBuf,
        auto_apply_confident: bool,
    },
    /// Add an artist and link a folder (adds a linked folder to the artist).
    AddAndLinkArtistFolder {
        input: String,
        path: PathBuf,
        auto_apply_confident: bool,
    },
    UnlinkArtistFolder {
        folder_id: i64,
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
    VerifyFiles {
        scope: SyncScope,
    },
    ResolveMissing {
        batch_id: u64,
        decisions: Vec<crate::types::MissingDecision>,
        remember: Option<crate::types::RememberChoice>,
    },
    ResolveUnavailable {
        batch_id: u64,
        decisions: Vec<crate::types::UnavailableDecision>,
    },
    RelinkRelease {
        key: ItemKey,
        path: PathBuf,
    },
    MarkArchived {
        keys: Vec<ItemKey>,
    },
    UnmarkDownloaded {
        keys: Vec<ItemKey>,
    },
    IgnoreLocal {
        band_id: i64,
        paths: Vec<PathBuf>,
    },
    RepointArtistFolder {
        folder_id: i64,
        new_path: PathBuf,
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
        folders: Vec<crate::types::ArtistFolder>,
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
        adopted: u32,
        missing_local: u32,
        relinked: u32,
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
        folders: Vec<crate::types::ArtistFolder>,
    },
    LibraryApplied {
        batch_id: u64,
        summary: crate::types::BatchSummary,
    },
    MissingFound {
        batch_id: u64,
        items: Vec<crate::types::MissingItem>,
        mass_missing: bool,
    },
    FolderUnavailable {
        batch_id: u64,
        folders: Vec<crate::types::UnavailableFolder>,
        prompt: bool,
    },
    Log(LogEntry),
}

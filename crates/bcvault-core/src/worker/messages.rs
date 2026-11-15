use crate::groups::{ActiveGrouping, ArtistGroup, ArtistGroupSet, PinTarget};
use crate::types::{
    ArtistStats, ArtistView, DownloadOpts, EnqueueMode, ItemKey, JobId, JobState, JobView,
    LogEntry, RateLimitStatus, ReleaseQuery, ReleaseRow, RemainingScope, Settings, SyncMode,
    SyncProgress, SyncScope,
};
use std::collections::HashMap;
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
        mode: EnqueueMode,
    },
    EnqueueRemaining {
        scope: RemainingScope,
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
    SetReleaseUnavailable {
        keys: Vec<ItemKey>,
        unavailable: bool,
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
    // artist groups
    SetActiveGrouping {
        grouping: ActiveGrouping,
    },
    CreateArtistGroupSet {
        name: String,
    },
    RenameArtistGroupSet {
        id: i64,
        name: String,
    },
    DeleteArtistGroupSet {
        id: i64,
    },
    CreateArtistGroup {
        set_id: Option<i64>,
        name: String,
        member_ids: Vec<i64>,
    },
    UpdateArtistGroup {
        id: i64,
        name: String,
        member_ids: Vec<i64>,
    },
    DeleteArtistGroup {
        id: i64,
    },
    ReorderArtistGroups {
        set_id: i64,
        ids: Vec<i64>,
    },
    SetArtistGroupCollapsed {
        id: i64,
        collapsed: bool,
    },
    PinArtists {
        set_id: i64,
        band_ids: Vec<i64>,
        target: PinTarget,
    },
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
        active_grouping: ActiveGrouping,
        group_sets: Vec<ArtistGroupSet>,
        groups: Vec<ArtistGroup>,
        pins: HashMap<i64, i64>,
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
    EnqueueSummary(EnqueueSummary),
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
    ArtistGroupsChanged {
        active_grouping: ActiveGrouping,
        group_sets: Vec<ArtistGroupSet>,
        groups: Vec<ArtistGroup>,
        pins: HashMap<i64, i64>,
    },
    ArtistGroupError {
        message: String,
    },
    Log(LogEntry),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeferCause {
    BatchLimit,
    HardCap,
    Blocked30Min,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeferredBatch {
    pub band_id: i64,
    pub count: u32,
    pub cause: DeferCause,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnqueueSummary {
    pub requested: u32,
    pub queued: u32,
    pub skipped_downloaded: u32,
    pub skipped_in_flight: u32,
    pub skipped_ineligible: u32,
    pub awaiting_ack: u32,
    pub deferred: Vec<DeferredBatch>,
}

impl EnqueueSummary {
    pub fn deferred_total(&self) -> u32 {
        self.deferred.iter().map(|d| d.count).sum()
    }

    pub fn format_summary(&self) -> String {
        if self.requested == 0 {
            return "Nothing to queue".to_string();
        }
        let mut parts = Vec::new();
        if self.queued > 0
            || (self.skipped_downloaded == 0
                && self.skipped_in_flight == 0
                && self.skipped_ineligible == 0
                && self.awaiting_ack == 0
                && self.deferred.is_empty())
        {
            parts.push(format!("Queued {}", self.queued));
        }
        if self.skipped_downloaded > 0 {
            parts.push(format!("{} already downloaded", self.skipped_downloaded));
        }
        if self.skipped_in_flight > 0 {
            parts.push(format!("{} already queued", self.skipped_in_flight));
        }
        if self.skipped_ineligible > 0 {
            parts.push(format!("{} ineligible", self.skipped_ineligible));
        }
        if self.awaiting_ack > 0 {
            parts.push(format!("{} awaiting quota confirmation", self.awaiting_ack));
        }
        if !self.deferred.is_empty() {
            let total = self.deferred_total();
            let artists: std::collections::HashSet<i64> =
                self.deferred.iter().map(|d| d.band_id).collect();
            let artist_word = if artists.len() == 1 {
                "artist"
            } else {
                "artists"
            };
            parts.push(format!(
                "{total} deferred ({} {artist_word} reached their limit)",
                artists.len()
            ));
        }
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_17_enqueue_summary_formatting() {
        assert_eq!(
            EnqueueSummary::default().format_summary(),
            "Nothing to queue"
        );

        let single_deferred = EnqueueSummary {
            requested: 40,
            queued: 25,
            deferred: vec![DeferredBatch {
                band_id: 1,
                count: 15,
                cause: DeferCause::BatchLimit,
            }],
            ..EnqueueSummary::default()
        };
        assert_eq!(
            single_deferred.format_summary(),
            "Queued 25 · 15 deferred (1 artist reached their limit)"
        );

        let mixed = EnqueueSummary {
            requested: 133,
            queued: 87,
            skipped_downloaded: 12,
            skipped_in_flight: 3,
            deferred: vec![
                DeferredBatch {
                    band_id: 1,
                    count: 20,
                    cause: DeferCause::BatchLimit,
                },
                DeferredBatch {
                    band_id: 2,
                    count: 11,
                    cause: DeferCause::HardCap,
                },
            ],
            ..EnqueueSummary::default()
        };
        assert_eq!(
            mixed.format_summary(),
            "Queued 87 · 12 already downloaded · 3 already queued · 31 deferred (2 artists reached their limit)"
        );
    }
}

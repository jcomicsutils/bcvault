pub use crate::library::FolderStyle;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type JobId = u64;
pub type Ts = i64; // unix seconds, UTC

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ItemType {
    Album,
    Track,
}

impl ItemType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Track => "track",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "album" | "a" => Some(Self::Album),
            "track" | "t" => Some(Self::Track),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Album => "Album",
            Self::Track => "Track",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ItemKey {
    pub kind: ItemType,
    pub id: i64,
}

impl ItemKey {
    pub const fn new(kind: ItemType, id: i64) -> Self {
        Self { kind, id }
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Default, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum Classification {
    #[default]
    Unknown,
    Free,
    Nyp,
    EmailRequired,
    Paid,
}

impl Classification {
    pub const fn auto_downloadable(self) -> bool {
        matches!(self, Self::Free | Self::Nyp)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Free => "free",
            Self::Nyp => "nyp",
            Self::EmailRequired => "email_required",
            Self::Paid => "paid",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "unknown" => Some(Self::Unknown),
            "free" => Some(Self::Free),
            "nyp" => Some(Self::Nyp),
            "email_required" => Some(Self::EmailRequired),
            "paid" => Some(Self::Paid),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::Free => "FREE",
            Self::Nyp => "NYP",
            Self::EmailRequired => "EMAIL",
            Self::Paid => "PAID",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioFormat {
    Flac,
    Mp3_320,
    Mp3V0,
    Wav,
    AiffLossless,
    Alac,
    Vorbis,
    AacHi,
}

impl AudioFormat {
    pub const ALL: [Self; 8] = [
        Self::Flac,
        Self::Mp3_320,
        Self::Mp3V0,
        Self::Wav,
        Self::AiffLossless,
        Self::Alac,
        Self::Vorbis,
        Self::AacHi,
    ];

    /// Exact keys of Bandcamp's `downloads` map.
    pub const fn api_key(self) -> &'static str {
        match self {
            Self::Flac => "flac",
            Self::Mp3_320 => "mp3-320",
            Self::Mp3V0 => "mp3-v0",
            Self::Wav => "wav",
            Self::AiffLossless => "aiff-lossless",
            Self::Alac => "alac",
            Self::Vorbis => "vorbis",
            Self::AacHi => "aac-hi",
        }
    }

    pub const fn as_str(self) -> &'static str {
        self.api_key()
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "flac" => Some(Self::Flac),
            "mp3-320" => Some(Self::Mp3_320),
            "mp3-v0" => Some(Self::Mp3V0),
            "wav" => Some(Self::Wav),
            "aiff-lossless" => Some(Self::AiffLossless),
            "alac" => Some(Self::Alac),
            "vorbis" => Some(Self::Vorbis),
            "aac-hi" => Some(Self::AacHi),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Flac => "FLAC",
            Self::Mp3_320 => "MP3 320",
            Self::Mp3V0 => "MP3 V0",
            Self::Wav => "WAV",
            Self::AiffLossless => "AIFF",
            Self::Alac => "ALAC",
            Self::Vorbis => "Vorbis",
            Self::AacHi => "AAC",
        }
    }

    pub const fn track_ext(self) -> &'static str {
        match self {
            Self::Flac => "flac",
            Self::Mp3_320 | Self::Mp3V0 => "mp3",
            Self::Wav => "wav",
            Self::AiffLossless => "aiff",
            Self::Alac | Self::AacHi => "m4a",
            Self::Vorbis => "ogg",
        }
    }

    pub const fn is_lossless(self) -> bool {
        matches!(
            self,
            Self::Flac | Self::Wav | Self::AiffLossless | Self::Alac
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LocalStatus {
    Unverified,
    Present,
    Missing,
    Archived,
}

impl LocalStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::Present => "present",
            Self::Missing => "missing",
            Self::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "unverified" => Some(Self::Unverified),
            "present" => Some(Self::Present),
            "missing" => Some(Self::Missing),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Origin {
    App,
    Imported,
    Redeem,
    Manual,
}

impl Origin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Imported => "imported",
            Self::Redeem => "redeem",
            Self::Manual => "manual",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "app" => Some(Self::App),
            "imported" => Some(Self::Imported),
            "redeem" => Some(Self::Redeem),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnMissing {
    #[default]
    Ask,
    Auto,
}

impl OnMissing {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Auto => "auto",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ask" => Some(Self::Ask),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeletedPolicy {
    #[default]
    KeepMarked,
    MarkNotDownloaded,
}

impl DeletedPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KeepMarked => "keep_marked",
            Self::MarkNotDownloaded => "mark_not_downloaded",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "keep_marked" => Some(Self::KeepMarked),
            "mark_not_downloaded" => Some(Self::MarkNotDownloaded),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FolderState {
    Ok,
    Missing,
    Unreadable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibraryPhase {
    Scanning,
    Matching,
    Verifying,
    Applying,
}

impl LibraryPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scanning => "scanning",
            Self::Matching => "matching",
            Self::Verifying => "verifying",
            Self::Applying => "applying",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Scanning => "Scanning folder",
            Self::Matching => "Matching releases",
            Self::Verifying => "Verifying files",
            Self::Applying => "Applying changes",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistFolder {
    pub band_id: i64,
    pub path: PathBuf,
    pub state: FolderState,
}

pub use crate::db::repo::library::LibraryTimelineEvent;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchSummary {
    pub adopted: u32,
    pub relinked: u32,
    pub archived: u32,
    pub unmarked: u32,
    pub restored: u32,
    pub ignored: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchDecision {
    Adopt {
        key: ItemKey,
        path: PathBuf,
        format: Option<AudioFormat>,
        folder_mtime: Option<Ts>,
    },
    Restore {
        key: ItemKey,
        path: PathBuf,
    },
    Attach {
        key: ItemKey,
        path: PathBuf,
        format: Option<AudioFormat>,
        folder_mtime: Option<Ts>,
    },
    Ignore {
        band_id: i64,
        path: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MissingCandidate {
    pub path: PathBuf,
    pub score: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MissingItem {
    pub key: ItemKey,
    pub band_id: i64,
    pub artist_name: String,
    pub title: String,
    pub last_known_path: Option<PathBuf>,
    pub best_candidate: Option<MissingCandidate>,
}

/// Action to apply when resolving a missing release.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MissingAction {
    /// Relink to a newly located folder or archive.
    Moved(PathBuf),
    /// Treat as deleted according to the chosen policy.
    Deleted(DeletedPolicy),
    /// Skip acknowledging absence; do not re-ask on sync.
    Skip,
}

/// Decision made by user or policy for a missing release.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MissingDecision {
    pub key: ItemKey,
    pub action: MissingAction,
}

/// User preference choices to remember when resolving missing albums.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RememberChoice {
    pub set_default_deleted_policy: Option<DeletedPolicy>,
    pub stop_asking: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RescanOutcome {
    pub adopted: u32,
    pub review_count: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VerifyOutcome {
    pub missing_count: u32,
    pub relinked_count: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArtistStats {
    pub downloaded: u32,
    pub eligible_downloaded: u32,
    pub non_eligible_downloaded: u32,
    pub eligible: u32,
    pub total: u32, // eligible = free + nyp
    pub email_required: u32,
    pub unknown: u32,
    pub quota_used_30d: u32,
    pub archived: u32,
    pub missing_files: u32,
}

impl ArtistStats {
    pub fn format_sidebar_badge(&self) -> String {
        if self.non_eligible_downloaded > 0 {
            format!(
                "{} (+{})/{}/{}",
                self.eligible_downloaded, self.non_eligible_downloaded, self.eligible, self.total
            )
        } else {
            format!(
                "{}/{}/{}",
                self.eligible_downloaded, self.eligible, self.total
            )
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtistView {
    pub band_id: i64,
    pub slug: String,
    pub name: String,
    pub custom_name: Option<String>,
    pub url: String,
    pub last_synced_at: Option<Ts>,
    pub stats: ArtistStats,
}

impl ArtistView {
    pub fn display_name(&self) -> &str {
        self.custom_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&self.name)
    }

    pub fn matches_input(&self, input: &str) -> bool {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return false;
        }
        if let Ok(norm) = crate::bandcamp::input::normalize_artist_input(trimmed)
            && (self.slug.eq_ignore_ascii_case(&norm.slug)
                || self
                    .url
                    .trim_end_matches('/')
                    .eq_ignore_ascii_case(norm.base_url.trim_end_matches('/')))
        {
            return true;
        }
        let lower = trimmed.to_lowercase();
        self.slug.to_lowercase() == lower
            || self.name.to_lowercase() == lower
            || self.display_name().to_lowercase() == lower
            || self.url.to_lowercase() == lower
            || lower.contains(&self.url.to_lowercase())
            || self.url.to_lowercase().contains(&lower)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateReason {
    TracksAdded,
    Modified,
}

impl UpdateReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TracksAdded => "tracks_added",
            Self::Modified => "modified",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tracks_added" => Some(Self::TracksAdded),
            "modified" => Some(Self::Modified),
            _ => None,
        }
    }

    pub const fn badge(self) -> &'static str {
        match self {
            Self::TracksAdded => "+TRACKS",
            Self::Modified => "MOD",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseRow {
    pub key: ItemKey,
    pub band_id: i64,
    pub title: String,
    pub artist_name: String,
    pub url: String,
    pub cover_url: Option<String>,
    pub art_id: Option<i64>,
    pub classification: Classification,
    pub is_preorder: bool,
    pub track_count: Option<u32>,
    pub published_at: Option<Ts>,
    pub modified_at: Option<Ts>,
    pub downloaded: bool,
    pub downloaded_at: Option<Ts>,
    pub downloaded_format: Option<AudioFormat>,
    pub local_path: Option<PathBuf>,
    pub local_status: Option<LocalStatus>,
    pub origin: Option<Origin>,
    pub missing_ack_at: Option<Ts>,
    pub has_remote_update: bool,
    pub remote_update_reason: Option<UpdateReason>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseQuery {
    pub band_id: Option<i64>,
    pub filter: ReleaseFilter,
    pub search: String,
    pub sort: SortKey,
    pub dir: SortDir,
}

impl Default for ReleaseQuery {
    fn default() -> Self {
        Self {
            band_id: None,
            filter: ReleaseFilter::All,
            search: String::new(),
            sort: SortKey::Released,
            dir: SortDir::Desc,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReleaseFilter {
    All,
    Free,
    Nyp,
    EmailRequired,
    Paid,
    NotDownloaded,
    Updated,
    Archived,
    MissingFiles,
}

impl ReleaseFilter {
    pub const ALL: [Self; 9] = [
        Self::All,
        Self::Free,
        Self::Nyp,
        Self::EmailRequired,
        Self::Paid,
        Self::NotDownloaded,
        Self::Updated,
        Self::Archived,
        Self::MissingFiles,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Free => "Free",
            Self::Nyp => "NYP",
            Self::EmailRequired => "Email Required",
            Self::Paid => "Paid",
            Self::NotDownloaded => "Not Downloaded",
            Self::Updated => "Updated/Modified",
            Self::Archived => "Archived",
            Self::MissingFiles => "Missing Files",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortKey {
    Artist,
    Title,
    Type,
    Released,
    Tracks,
    Classification,
    Format,
    Status,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub const fn toggle(self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormatFallback {
    Strict,
    Nearest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputLayout {
    Flat,
    ByArtist,
    ByPageArtist,
}

impl OutputLayout {
    pub const ALL: [Self; 3] = [Self::Flat, Self::ByArtist, Self::ByPageArtist];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::ByArtist => "By Album Artist",
            Self::ByPageArtist => "By Page Artist",
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_auto_match_min() -> f32 {
    0.92
}

fn default_review_match_min() -> f32 {
    0.75
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub out_dir: PathBuf,
    pub format: AudioFormat,
    pub format_fallback: FormatFallback,
    pub layout: OutputLayout,
    #[serde(default)]
    pub filename_pattern: String,
    pub auto_extract: bool,
    pub delete_zip_after_extract: bool,
    pub batch_limit: u32, // 25 | 50 | 100 | 150 | 200, clamped to quota_hard
    pub quota_soft: u32,
    pub quota_hard: u32, // 150 / 200 per artist per rolling 30 days
    pub page_delay_ms: u64,
    pub page_jitter_pct: u8,          // 1500 / 25
    pub max_concurrent_downloads: u8, // 2
    pub show_thumbnails: bool,
    pub user_agent: String,

    #[serde(default = "default_true")]
    pub verify_on_sync: bool,
    #[serde(default = "default_true")]
    pub rescan_linked_on_sync: bool,
    #[serde(default)]
    pub on_missing: OnMissing,
    #[serde(default)]
    pub deleted_policy: DeletedPolicy,
    #[serde(default = "default_true")]
    pub auto_relink: bool,
    #[serde(default = "default_auto_match_min")]
    pub auto_match_min: f32,
    #[serde(default = "default_review_match_min")]
    pub review_match_min: f32,
    #[serde(default)]
    pub folder_style: FolderStyle,
}

impl Settings {
    pub fn default_with_dir(out_dir: PathBuf) -> Self {
        Self {
            out_dir,
            format: AudioFormat::Flac,
            format_fallback: FormatFallback::Nearest,
            layout: OutputLayout::Flat,
            filename_pattern: String::new(),
            auto_extract: true,
            delete_zip_after_extract: false,
            batch_limit: 150,
            quota_soft: 150,
            quota_hard: 200,
            page_delay_ms: 1500,
            page_jitter_pct: 25,
            max_concurrent_downloads: 2,
            show_thumbnails: false,
            user_agent: format!(
                "bcvault/{} (personal archiving tool)",
                env!("CARGO_PKG_VERSION")
            ),
            verify_on_sync: true,
            rescan_linked_on_sync: true,
            on_missing: OnMissing::Ask,
            deleted_policy: DeletedPolicy::KeepMarked,
            auto_relink: true,
            auto_match_min: 0.92,
            review_match_min: 0.75,
            folder_style: FolderStyle::default(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::default_with_dir(PathBuf::from("downloads"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadOpts {
    pub format: AudioFormat,
    pub out_dir: PathBuf,
    pub batch_limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobView {
    pub id: JobId,
    pub key: ItemKey,
    pub band_id: i64,
    pub label: String, // "Artist - Title"
    pub format: AudioFormat,
    pub state: JobState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Resolving,
    Downloading,
    Extracting,
    Done { path: PathBuf, bytes: u64 },
    Failed(JobError),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobError {
    Network(String),
    ApiRejected(String),
    EmailRequired,
    QuotaExceeded,
    FormatUnavailable,
    Io(String),
    RateLimited { retry_after_s: u64 },
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(s) => write!(f, "Network error: {s}"),
            Self::ApiRejected(s) => write!(f, "API rejected: {s}"),
            Self::EmailRequired => write!(f, "Email address required"),
            Self::QuotaExceeded => write!(f, "Artist download quota exceeded"),
            Self::FormatUnavailable => write!(f, "Requested audio format unavailable"),
            Self::Io(s) => write!(f, "I/O error: {s}"),
            Self::RateLimited { retry_after_s } => {
                write!(f, "Rate limited (retry after {retry_after_s}s)")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncPhase {
    Resolving,
    Discovering,
    Diffing,
    FetchingMetadata,
    Finalising,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncProgress {
    pub band_id: i64,
    pub phase: SyncPhase,
    pub done: u32,
    pub total: u32,
    pub current: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMode {
    Incremental,
    CheckDownloaded,
    FullRefresh,
}

impl SyncMode {
    pub const ALL: [Self; 3] = [Self::Incremental, Self::CheckDownloaded, Self::FullRefresh];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Incremental => "Incremental",
            Self::CheckDownloaded => "Check downloaded",
            Self::FullRefresh => "Full refresh",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncScope {
    All,
    Selected(Vec<i64>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RateLimitStatus {
    Idle,
    Pacing { next_in_ms: u64 },
    Cooldown { until: Ts, reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub ts: Ts,
    pub level: LogLevel,
    pub scope: &'static str,
    pub message: String,
    pub item: Option<ItemKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryStatus {
    Started,
    Success,
    Failed,
    Cancelled,
}

impl HistoryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "started" => Some(Self::Started),
            "success" => Some(Self::Success),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadSource {
    Api,
    Redeem,
}

impl DownloadSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Redeem => "redeem",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "api" => Some(Self::Api),
            "redeem" => Some(Self::Redeem),
            _ => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn item_type_roundtrip() {
        for kind in [ItemType::Album, ItemType::Track] {
            assert_eq!(ItemType::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ItemType::parse("a"), Some(ItemType::Album));
        assert_eq!(ItemType::parse("t"), Some(ItemType::Track));
        assert_eq!(ItemType::parse("invalid"), None);
    }

    #[test]
    fn classification_roundtrip() {
        let all = [
            Classification::Unknown,
            Classification::Free,
            Classification::Nyp,
            Classification::EmailRequired,
            Classification::Paid,
        ];
        for c in all {
            assert_eq!(Classification::parse(c.as_str()), Some(c));
        }
        assert!(Classification::Free.auto_downloadable());
        assert!(Classification::Nyp.auto_downloadable());
        assert!(!Classification::Unknown.auto_downloadable());
        assert!(!Classification::EmailRequired.auto_downloadable());
        assert!(!Classification::Paid.auto_downloadable());
        assert_eq!(Classification::parse("bogus"), None);
    }

    #[test]
    fn audio_format_roundtrip() {
        for fmt in AudioFormat::ALL {
            assert_eq!(AudioFormat::parse(fmt.as_str()), Some(fmt));
            assert_eq!(fmt.as_str(), fmt.api_key());
            assert!(!fmt.track_ext().is_empty());
        }
        assert_eq!(AudioFormat::parse("unknown"), None);
    }

    #[test]
    fn update_reason_roundtrip() {
        for r in [UpdateReason::TracksAdded, UpdateReason::Modified] {
            assert_eq!(UpdateReason::parse(r.as_str()), Some(r));
        }
        assert_eq!(UpdateReason::parse("other"), None);
    }

    #[test]
    fn history_status_and_source_roundtrip() {
        for s in [
            HistoryStatus::Started,
            HistoryStatus::Success,
            HistoryStatus::Failed,
            HistoryStatus::Cancelled,
        ] {
            assert_eq!(HistoryStatus::parse(s.as_str()), Some(s));
        }
        for src in [DownloadSource::Api, DownloadSource::Redeem] {
            assert_eq!(DownloadSource::parse(src.as_str()), Some(src));
        }
    }

    #[test]
    fn library_types_and_defaults_roundtrip() {
        for status in [
            LocalStatus::Unverified,
            LocalStatus::Present,
            LocalStatus::Missing,
            LocalStatus::Archived,
        ] {
            assert_eq!(LocalStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(LocalStatus::parse("bogus"), None);

        for origin in [
            Origin::App,
            Origin::Imported,
            Origin::Redeem,
            Origin::Manual,
        ] {
            assert_eq!(Origin::parse(origin.as_str()), Some(origin));
        }
        assert_eq!(Origin::parse("bogus"), None);

        for om in [OnMissing::Ask, OnMissing::Auto] {
            assert_eq!(OnMissing::parse(om.as_str()), Some(om));
        }
        assert_eq!(OnMissing::default(), OnMissing::Ask);

        for dp in [DeletedPolicy::KeepMarked, DeletedPolicy::MarkNotDownloaded] {
            assert_eq!(DeletedPolicy::parse(dp.as_str()), Some(dp));
        }
        assert_eq!(DeletedPolicy::default(), DeletedPolicy::KeepMarked);

        for phase in [
            LibraryPhase::Scanning,
            LibraryPhase::Matching,
            LibraryPhase::Verifying,
            LibraryPhase::Applying,
        ] {
            assert!(!phase.as_str().is_empty());
            assert!(!phase.label().is_empty());
        }

        let folder = ArtistFolder {
            band_id: 42,
            path: PathBuf::from("/music/artist"),
            state: FolderState::Ok,
        };
        assert_eq!(folder.band_id, 42);
        assert_eq!(folder.state, FolderState::Ok);

        for filter in ReleaseFilter::ALL {
            assert!(!filter.label().is_empty());
        }
        assert!(ReleaseFilter::ALL.contains(&ReleaseFilter::Archived));
        assert!(ReleaseFilter::ALL.contains(&ReleaseFilter::MissingFiles));

        // Test that old Settings JSON without new fields deserializes with defaults
        let old_json = r#"{
            "out_dir": "downloads",
            "format": "Flac",
            "format_fallback": "Nearest",
            "layout": "Flat",
            "filename_pattern": "",
            "auto_extract": true,
            "delete_zip_after_extract": false,
            "batch_limit": 150,
            "quota_soft": 150,
            "quota_hard": 200,
            "page_delay_ms": 1500,
            "page_jitter_pct": 25,
            "max_concurrent_downloads": 2,
            "show_thumbnails": false,
            "user_agent": "bcvault/0.1.0"
        }"#;
        let s: Settings = serde_json::from_str(old_json).unwrap();
        assert!(s.verify_on_sync);
        assert!(s.rescan_linked_on_sync);
        assert_eq!(s.on_missing, OnMissing::Ask);
        assert_eq!(s.deleted_policy, DeletedPolicy::KeepMarked);
        assert!(s.auto_relink);
        assert!((s.auto_match_min - 0.92).abs() < 1e-4);
        assert!((s.review_match_min - 0.75).abs() < 1e-4);
    }

    #[test]
    fn test_artist_view_matches_input() {
        let artist = ArtistView {
            band_id: 123,
            slug: "areyoudoneyet".to_string(),
            name: "Are You Done Yet".to_string(),
            custom_name: None,
            url: "https://areyoudoneyet.bandcamp.com".to_string(),
            last_synced_at: None,
            stats: ArtistStats::default(),
        };

        // Exact slug
        assert!(artist.matches_input("areyoudoneyet"));
        assert!(artist.matches_input("AreYouDoneYet"));
        // URL variants
        assert!(artist.matches_input("https://areyoudoneyet.bandcamp.com"));
        assert!(artist.matches_input("https://areyoudoneyet.bandcamp.com/"));
        assert!(artist.matches_input("https://areyoudoneyet.bandcamp.com/music"));
        assert!(artist.matches_input("areyoudoneyet.bandcamp.com"));
        // Display name
        assert!(artist.matches_input("Are You Done Yet"));
        assert!(artist.matches_input("are you done yet"));
        // Mismatches
        assert!(!artist.matches_input("otherartist"));
        assert!(!artist.matches_input("https://otherartist.bandcamp.com"));
        assert!(!artist.matches_input(""));
    }

    #[test]
    fn test_artist_stats_format_sidebar_badge() {
        // Case 1: Standard clean badge with zero non-eligible downloads
        let clean_stats = ArtistStats {
            downloaded: 10,
            eligible_downloaded: 10,
            non_eligible_downloaded: 0,
            eligible: 15,
            total: 20,
            ..Default::default()
        };
        assert_eq!(clean_stats.format_sidebar_badge(), "10/15/20");

        // Case 2: Nigeria Futebol Clube (3 on page, 2 free, 1 paid, all 3 downloaded)
        let nfc_stats = ArtistStats {
            downloaded: 3,
            eligible_downloaded: 2,
            non_eligible_downloaded: 1,
            eligible: 2,
            total: 3,
            ..Default::default()
        };
        assert_eq!(nfc_stats.format_sidebar_badge(), "2 (+1)/2/3");

        // Case 3: Leichen (40 free on page, all downloaded + 2 extra downloaded off-page)
        let leichen_stats = ArtistStats {
            downloaded: 42,
            eligible_downloaded: 40,
            non_eligible_downloaded: 2,
            eligible: 40,
            total: 40,
            ..Default::default()
        };
        assert_eq!(leichen_stats.format_sidebar_badge(), "40 (+2)/40/40");
    }

    #[test]
    fn test_23_settings_json_without_folder_style_deserializes_to_default_and_roundtrips() {
        let minimal_json = r#"{
            "out_dir": "/tmp/music",
            "format": "Flac",
            "format_fallback": "Nearest",
            "layout": "Flat",
            "auto_extract": true,
            "delete_zip_after_extract": false,
            "batch_limit": 150,
            "quota_soft": 150,
            "quota_hard": 200,
            "page_delay_ms": 1500,
            "page_jitter_pct": 25,
            "max_concurrent_downloads": 2,
            "show_thumbnails": false,
            "user_agent": "test"
        }"#;

        let s: Settings = serde_json::from_str(minimal_json).expect("deserialize settings");
        assert_eq!(s.folder_style, FolderStyle::default());
        assert_eq!(s.folder_style.templates, vec!["{title}".to_string()]);
        assert!(s.folder_style.noise.is_empty());

        let serialized = serde_json::to_string(&s).expect("serialize settings");
        let roundtrip: Settings = serde_json::from_str(&serialized).expect("roundtrip settings");
        assert_eq!(roundtrip, s);
    }

    #[test]
    fn test_31_settings_json_with_folder_style_without_noise() {
        let json_with_style_no_noise = r#"{
            "out_dir": "/tmp/music",
            "format": "Flac",
            "format_fallback": "Nearest",
            "layout": "Flat",
            "auto_extract": true,
            "delete_zip_after_extract": false,
            "batch_limit": 150,
            "quota_soft": 150,
            "quota_hard": 200,
            "page_delay_ms": 1500,
            "page_jitter_pct": 25,
            "max_concurrent_downloads": 2,
            "show_thumbnails": false,
            "user_agent": "test",
            "folder_style": {
                "templates": ["{artist} - {title}"]
            }
        }"#;

        let s: Settings =
            serde_json::from_str(json_with_style_no_noise).expect("deserialize settings");
        assert_eq!(
            s.folder_style.templates,
            vec!["{artist} - {title}".to_string()]
        );
        assert!(s.folder_style.noise.is_empty());
    }
}

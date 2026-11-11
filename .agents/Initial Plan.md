# Bandcamp Free/NYP Discography Manager: Agent-Ready Implementation Plan

*Rust 2024 · eframe/egui · tokio · reqwest · rusqlite (WAL) · working name `bcvault` · prepared 6 Nov 2026*

> Status 11 Nov 2026: **implemented**, tagged `v0.1.0` (current workspace version `v0.1.3`). Later work moved the code into a workspace (`crates/bcvault-core`, `crates/bcvault`, `crates/bcvault-fx`) and added main-DB migrations `0003_custom_artist_name` to `0008_rejected_matches` (including `Classification::Unavailable` in `0007`); paths in this plan are pre-split. The code is authoritative where it differs.

## 0. How to use this plan

**Goal.** A single-binary desktop app that imports Bandcamp artists, tracks their discographies in SQLite, classifies every release (Free / NYP / Email-gated / Paid), and downloads the free and name-your-price ones in a chosen audio format, behind a sharp, square-cornered `egui` interface that never stalls on I/O.

**Execution protocol for the implementing agent**

1. Work phase by phase (§8). Do not start phase N+1 until every box in phase N's verification milestone is green. Keep a `PROGRESS.md` with the current task ID and open questions.
2. Tests first for every parser and every SQL query. Bandcamp-facing code is developed against saved fixtures; the live site is touched only by `examples/capture_fixtures.rs` and the opt-in `live-tests` feature.
3. `#![forbid(unsafe_code)]`. No `unwrap()`, `expect()` or `panic!` outside tests and `main` bootstrap. `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` pass at every commit.
4. **UI-thread purity.** `App::update` may only draw, read in-memory state, send commands and `try_recv` events. No `block_on`, no `rusqlite`, no `reqwest`, no file I/O, no blocking `rfd` dialogs.
5. **One gate.** Every request to a Bandcamp host goes through `BandcampGate` (§6.7). No module builds its own `reqwest` call.
6. `scraper::Html` is `!Send`. Parse inside plain synchronous functions that return owned structs; never keep an `Html` alive across an `.await`.
7. Versions in §2 are a known-good baseline. If an API has drifted (egui moves fast), fix the call site rather than downgrading. All `egui*` / `eframe` crates must share one minor version.

**Intentional deviations from the brief** (each is a correctness fix, flagged so you can veto it)

| Brief | Plan | Why |
| --- | --- | --- |
| `releases.item_id` as sole PRIMARY KEY | `PRIMARY KEY (item_type, item_id)` | Album and track IDs come from separate sequences, so a bare ID can collide. |
| `classification` has four values | Adds `unknown` | Rows found on the `/music` grid exist before their page has been fetched. |
| Only the listed columns | Adds `art_id`, `pricing_json`, `is_preorder`, `metadata_fetched_at`, `first_seen_at`, `last_seen_at`, `downloaded_track_count`, `downloaded_modified_at`, `remote_update_reason`, and `download_history.source` / `counts_against_quota` | Re-classification without refetching, resumable syncs, bonus-track detection, accurate quota math. |
| 150 / 200 cap treated as ground truth | Local 30-day estimate plus reaction to API rejections | The artist's free-download allowance is shared by every fan; the app only sees its own downloads. Real exhaustion shows up as an API rejection and halts that artist's queue. |
| Endpoints treated as stable | Isolated behind `bandcamp/` with versioned constants and fixture contract tests | `band_details` (v24) and `buy_for_nothing` (v22) are undocumented and can change without notice. |

## 1. Architecture at a glance

```text
┌──────────── main thread ─────────────┐         ┌──────── tokio multi-thread runtime ─────────┐
│ eframe::App::update()                │  cmds   │ Supervisor (command router, CancellationTokens)│
│  · try_recv() WorkerToUiEvent (≤256) │ ──────► │  ├─ SyncService    (one task per artist)       │
│  · draws sharp egui UI               │         │  ├─ DownloadManager (Semaphore + JoinSet)      │
│  · send() UiToWorkerCommand          │ ◄────── │  ├─ BandcampClient + BandcampGate              │
└──────────────────────────────────────┘ events  │  └─ DbPool { writer thread, reader thread }    │
        ▲  ctx.request_repaint()  ◄───────────── └───────────────────────────────────────────────┘
```

- **Runtime.** Built in `main()` (`Builder::new_multi_thread().worker_threads(4).enable_all()`), kept alive by `App`, shut down with `shutdown_timeout(2s)`. The worker is a set of tasks spawned on it; the UI thread owns no async code.
- **Channels.** `UnboundedSender<UiToWorkerCommand>` (UI to worker; `send` is synchronous so the UI never awaits) and `UnboundedSender<WorkerToUiEvent>` (worker to UI). State events are lossless. `DownloadByteProgress` is throttled at the source (at most 10 Hz per job plus one final event) and coalesced per `job_id` in the UI.
- **Repaint.** `UiBridge` wraps the event sender plus a cloned `egui::Context`; `emit()` sends then calls `ctx.request_repaint()`. egui coalesces repaints, so an idle app uses near-zero CPU. While a cooldown timer is visible, the UI additionally calls `request_repaint_after(250ms)`.
- **Database.** Two SQLite connections in WAL mode, each owned by its own OS thread (the `Db` actor, §4.4): one writer and one `query_only` reader. Table queries use the reader, so a long write transaction can never stall scrolling.
- **Stale results.** Every `QueryReleases` carries a `request_id`; the UI discards `ReleasesLoaded` events that are not the latest.
- **Shutdown.** `Shutdown` command trips the root cancellation token; in-flight downloads leave `.part` files that are swept on next boot.

## 2. Crate and dependency list (`Cargo.toml`)

```toml
[package]
name = "bcvault"
version = "0.1.0"
edition = "2024"
rust-version = "1.88"
description = "Track and download free / name-your-price Bandcamp discographies"

[dependencies]
# GUI
eframe = { version = "0.33", default-features = false, features = [
    "default_fonts", "glow", "persistence", "wayland", "x11",
] }
egui = "0.33"
egui_extras = { version = "0.33", features = ["image", "file"] }   # TableBuilder + file:// image loader
image = { version = "0.25", default-features = false, features = ["jpeg", "png"] }
rfd = { version = "0.15", default-features = false, features = ["xdg-portal", "tokio", "common-controls-v6"] }
open = "5"

# Async / network / parsing
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time", "fs", "io-util"] }
tokio-util = "0.7"            # CancellationToken
futures-util = "0.3"          # StreamExt for bytes_stream()
reqwest = { version = "0.12", default-features = false, features = [
    "rustls-tls", "json", "stream", "gzip", "brotli", "cookies", "http2",
] }
scraper = "0.23"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
url = "2"
percent-encoding = "2"        # RFC 5987 filename*=
rand = "0.9"                  # request jitter
chrono = { version = "0.4", default-features = false, features = ["std", "clock"] }  # RFC 2822 dates

# Storage / files
rusqlite = { version = "0.37", features = ["bundled"] }
directories = "6"
zip = { version = "2", default-features = false, features = ["deflate"] }

# Errors / logging
thiserror = "2"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
wiremock = "0.6"
tempfile = "3"
pretty_assertions = "1"

[features]
live-tests = []   # opt-in tests that hit bandcamp.com; never run in CI

[lints.rust]
unsafe_code = "forbid"
[lints.clippy]
all = { level = "warn", priority = -1 }
unwrap_used = "warn"
expect_used = "warn"

[profile.dev]
opt-level = 1
[profile.dev.package."*"]
opt-level = 3      # egui, image and SQLite are painfully slow at opt-level 0
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
```

**Why these flags**

- `eframe/glow`: smallest binary and lowest latency for a 2D table UI. Switch to `wgpu` only if a target machine has GL problems. `persistence` stores window geometry only; all app settings live in SQLite so the worker can read them too.
- `egui_extras/image` + `file`: renders cached covers via `file://` URIs. The `http` loader is deliberately **not** enabled because it would bypass `BandcampGate`.
- `reqwest/rustls-tls`: no OpenSSL dependency. `stream` gives `bytes_stream()` for chunked downloads, `cookies` supports download-page sessions, `gzip`/`brotli` shrink HTML pages.
- `rusqlite/bundled`: a static, recent SQLite, so `unixepoch()`, `STRICT` tables, partial indexes and `RETURNING` are guaranteed. No `chrono` feature: timestamps are `INTEGER` unix seconds (UTC), formatted only in the UI.
- `rfd` with `tokio` + `xdg-portal`: avoids a GTK dependency on Linux. Always call the `AsyncFileDialog` API from a spawned task and send the chosen path back as a command.
- `zip` with `deflate` only: Bandcamp archives are deflate-compressed.

## 3. Module and directory tree

```text
bcvault/
├── Cargo.toml  rust-toolchain.toml  rustfmt.toml  PROGRESS.md
├── migrations/                 0001_init.sql  0002_indexes.sql
├── examples/
│   ├── capture_fixtures.rs     polite one-shot fixture grabber (writes tests/fixtures/*)
│   └── headless.rs             sync + download without the GUI (CLI smoke harness)
├── tests/
│   ├── fixtures/               manifest.toml + saved .html/.json (see Phase 2)
│   └── db_concurrency.rs  sync_pipeline.rs  download_pipeline.rs
└── src/
    ├── main.rs                 tokio runtime, tracing, eframe::run_native
    ├── app.rs                  App: eframe::App; owns UiState, cmd sender, event receiver
    ├── types.rs                ItemKey, ItemType, Classification, AudioFormat, rows, Settings
    ├── error.rs                thiserror enums: DbError, BcError, DownloadError
    ├── paths.rs                directories::ProjectDirs → db path, cover cache, logs
    ├── ui/
    │   ├── theme.rs            apply(ctx): zero radius, palette, 1px borders, solid scrollbars
    │   ├── state.rs            UiState: filters, sort, selection, artists, jobs, log ring
    │   ├── sidebar.rs          artist filter, list (show_rows), badges, add bar, sync buttons
    │   ├── toolbar.rs          filter chips, format, batch limit, output dir, bulk actions
    │   ├── release_table.rs    TableBuilder, sorting, selection, row actions
    │   ├── bottom_dock.rs      queue tab (progress bars), errors tab, rate-limit chip
    │   ├── modals.rs           redeem URL, quota warning, settings, confirm remove
    │   ├── thumbs.rs           optional cover thumbnails from the cache dir
    │   └── widgets.rs          sharp_button, status_glyph (painter-drawn), badge, byte formatting
    ├── db/
    │   ├── mod.rs              Db actor (std thread + oneshot), DbPool { writer, reader }
    │   ├── migrations.rs       user_version runner over include_str! migrations
    │   └── repo/               artists.rs  releases.rs  history.rs  settings.rs (typed queries only)
    ├── bandcamp/
    │   ├── mod.rs              BandcampClient facade
    │   ├── gate.rs             BandcampGate: pacing, jitter, 429 cooldown, semaphores
    │   ├── input.rs            URL / slug normalisation
    │   ├── discography.rs      /music grid + data-client-items + band_details → DiscoveredItem
    │   ├── tralbum.rs          data-tralbum → Tralbum (lenient serde)
    │   ├── classify.rs         pricing signals → Classification (pure function)
    │   ├── download_api.rs     buy_for_nothing, format choice, URL resolution
    │   ├── redeem.rs           #pagedata[data-blob] → download_items
    │   ├── filenames.rs        Content-Disposition (RFC 5987), sanitising, reserved names
    │   └── errors.rs           API rejection → typed reason (string heuristics isolated here)
    └── worker/
        ├── mod.rs              spawn_worker(rt, ctx) -> WorkerHandle
        ├── messages.rs         UiToWorkerCommand, WorkerToUiEvent (§4.3)
        ├── bridge.rs           UiBridge { tx, ctx }.emit()
        ├── supervisor.rs       command router, JoinSet, CancellationTokens
        ├── sync.rs             SyncService: resolve → discover → diff → metadata queue
        ├── downloads.rs        DownloadManager: queue, semaphores, pause / cancel / retry
        ├── stream.rs           chunked streaming to .part, progress, speed, atomic rename
        ├── extract.rs          zip extraction (spawn_blocking, zip-slip safe)
        ├── quota.rs            30-day usage, soft / hard cap decisions
        └── thumbs.rs           cover fetch → cache dir, ThumbReady events
```

## 4. Data structures and channel messages

### 4.1 Core types (`src/types.rs`)

```rust
pub type JobId = u64;
pub type Ts = i64; // unix seconds, UTC

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemType { Album, Track }            // DB and API strings: "album" | "track"

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemKey { pub kind: ItemType, pub id: i64 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Classification { #[default] Unknown, Free, Nyp, EmailRequired, Paid }
impl Classification {
    pub fn auto_downloadable(self) -> bool { matches!(self, Self::Free | Self::Nyp) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AudioFormat { Flac, Mp3_320, Mp3V0, Wav, AiffLossless, Alac, Vorbis, AacHi }
impl AudioFormat {
    pub const ALL: [Self; 8] = [Self::Flac, Self::Mp3_320, Self::Mp3V0, Self::Wav,
                                Self::AiffLossless, Self::Alac, Self::Vorbis, Self::AacHi];
    /// Exact keys of Bandcamp's `downloads` map.
    pub const fn api_key(self) -> &'static str {
        match self {
            Self::Flac => "flac", Self::Mp3_320 => "mp3-320", Self::Mp3V0 => "mp3-v0",
            Self::Wav => "wav", Self::AiffLossless => "aiff-lossless", Self::Alac => "alac",
            Self::Vorbis => "vorbis", Self::AacHi => "aac-hi",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ArtistStats {
    pub downloaded: u32, pub eligible: u32, pub total: u32,   // eligible = free + nyp
    pub email_required: u32, pub unknown: u32, pub quota_used_30d: u32,
}

#[derive(Clone, Debug)]
pub struct ArtistView { pub band_id: i64, pub slug: String, pub name: String, pub url: String,
                        pub last_synced_at: Option<Ts>, pub stats: ArtistStats }

#[derive(Clone, Debug)]
pub struct ReleaseRow {
    pub key: ItemKey, pub band_id: i64, pub title: String, pub artist_name: String,
    pub url: String, pub cover_url: Option<String>, pub classification: Classification,
    pub track_count: Option<u32>, pub published_at: Option<Ts>, pub modified_at: Option<Ts>,
    pub downloaded: bool, pub downloaded_at: Option<Ts>, pub downloaded_format: Option<AudioFormat>,
    pub local_path: Option<std::path::PathBuf>,
    pub has_remote_update: bool, pub remote_update_reason: Option<UpdateReason>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum UpdateReason { TracksAdded, Modified }

#[derive(Clone, Debug)]
pub struct ReleaseQuery { pub band_id: Option<i64>, pub filter: ReleaseFilter, pub search: String,
                          pub sort: SortKey, pub dir: SortDir }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseFilter { All, Free, Nyp, EmailRequired, Paid, NotDownloaded, Updated }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey { Artist, Title, Type, Released, Tracks, Classification, Format, Status }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum SortDir { Asc, Desc }

#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum FormatFallback { Strict, Nearest }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum OutputLayout { Flat, ByArtist }

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub out_dir: std::path::PathBuf,
    pub format: AudioFormat, pub format_fallback: FormatFallback, pub layout: OutputLayout,
    pub auto_extract: bool, pub delete_zip_after_extract: bool,
    pub batch_limit: u32,                         // 25 | 50 | 100 | 150 | 200, clamped to quota_hard
    pub quota_soft: u32, pub quota_hard: u32,     // 150 / 200 per artist per rolling 30 days
    pub page_delay_ms: u64, pub page_jitter_pct: u8,      // 1500 / 25
    pub max_concurrent_downloads: u8,             // 2
    pub show_thumbnails: bool, pub user_agent: String,
}
```

(`AudioFormat`, `FormatFallback`, `OutputLayout` also derive `Serialize` / `Deserialize`; omitted above for brevity.)

### 4.2 Jobs, progress and status

```rust
#[derive(Clone, Debug)]
pub struct DownloadOpts { pub format: AudioFormat, pub out_dir: std::path::PathBuf,
                          pub batch_limit: u32 }

#[derive(Clone, Debug)]
pub struct JobView { pub id: JobId, pub key: ItemKey, pub label: String,   // "Artist - Title"
                     pub format: AudioFormat, pub state: JobState }
#[derive(Clone, Debug)]
pub enum JobState {
    Queued, Resolving, Downloading, Extracting,
    Done { path: std::path::PathBuf, bytes: u64 },
    Failed(JobError), Cancelled,
}
#[derive(Clone, Debug)]
pub enum JobError {
    Network(String), ApiRejected(String), EmailRequired, QuotaExceeded,
    FormatUnavailable, Io(String), RateLimited { retry_after_s: u64 },
}

#[derive(Clone, Copy, Debug)]
pub enum SyncPhase { Resolving, Discovering, Diffing, FetchingMetadata, Finalising }
#[derive(Clone, Debug)]
pub struct SyncProgress { pub band_id: i64, pub phase: SyncPhase, pub done: u32, pub total: u32,
                          pub current: Option<String> }
#[derive(Clone, Copy, Debug)] pub enum SyncMode { Incremental, CheckDownloaded, FullRefresh }
#[derive(Clone, Debug)]       pub enum SyncScope { All, Selected(Vec<i64>) }

#[derive(Clone, Debug)]
pub enum RateLimitStatus { Idle, Pacing { next_in_ms: u64 },
                           Cooldown { until: Ts, reason: String } }
#[derive(Clone, Copy, Debug)] pub enum LogLevel { Info, Warn, Error }
#[derive(Clone, Debug)]
pub struct LogEntry { pub ts: Ts, pub level: LogLevel, pub scope: &'static str,
                      pub message: String, pub item: Option<ItemKey> }
```

### 4.3 Channel messages (`src/worker/messages.rs`)

```rust
pub enum UiToWorkerCommand {
    // artists
    AddArtist { input: String },                  // URL, custom domain or bare slug
    RemoveArtist { band_id: i64 },                // DB rows only; never deletes files
    // sync
    Sync { scope: SyncScope, mode: SyncMode },
    CancelSync,
    // table data (answered on the reader connection)
    QueryReleases { request_id: u64, query: ReleaseQuery },
    RequestThumbs { keys: Vec<ItemKey> },         // visible rows only, debounced
    // downloads
    EnqueueDownloads { items: Vec<ItemKey>, opts: DownloadOpts, ack_soft_quota: bool },
    RedeemUrl { target: ItemKey, url: String, opts: DownloadOpts },
    CancelJob(JobId), RetryJob(JobId), CancelAllJobs,
    PauseQueue, ResumeQueue, ClearFinished,
    // misc
    UpdateSettings(Settings),
    Shutdown,
}

pub enum WorkerToUiEvent {
    Ready { artists: Vec<ArtistView>, settings: Settings },
    ArtistUpserted(ArtistView), ArtistRemoved { band_id: i64 },
    ArtistStats { band_id: i64, stats: ArtistStats },
    ReleasesLoaded { request_id: u64, rows: Vec<ReleaseRow> },
    ReleaseUpdated(Box<ReleaseRow>),              // patched in place by key
    SyncProgress(SyncProgress),
    SyncFinished { band_id: i64, new_items: u32, updated: u32, error: Option<String> },
    JobQueued(JobView), JobState { job_id: JobId, state: JobState },
    DownloadByteProgress { job_id: JobId, bytes: u64, total: Option<u64>, bytes_per_sec: f64 },
    QuotaStatus { band_id: i64, used_30d: u32, soft: u32, hard: u32 },
    QuotaWarning { band_id: i64, used_30d: u32, requested: u32, soft: u32, hard: u32 },
    RateLimit(RateLimitStatus),
    ThumbReady { key: ItemKey, path: std::path::PathBuf },
    Log(LogEntry),
}
```

### 4.4 Database actor (`src/db/mod.rs`)

`rusqlite::Connection` is `Send` but not `Sync`, so each connection lives on a dedicated std thread and is reached through a job channel. This is the whole async bridge.

```rust
type Job = Box<dyn FnOnce(&mut rusqlite::Connection) + Send + 'static>;

#[derive(Clone)]
pub struct Db { tx: std::sync::mpsc::Sender<Job> }

impl Db {
    /// Spawns the owning thread, applies PRAGMAs for the role, then loops over jobs.
    pub fn open(path: &std::path::Path, role: Role) -> Result<Self, DbError> { /* ... */ }

    pub async fn call<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut rusqlite::Connection) -> Result<R, DbError> + Send + 'static,
    ) -> Result<R, DbError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx.send(Box::new(move |c| { let _ = tx.send(f(c)); })).map_err(|_| DbError::Closed)?;
        rx.await.map_err(|_| DbError::Closed)?
    }
}
pub enum Role { Writer, Reader }
pub struct DbPool { pub writer: Db, pub reader: Db }
```

The writer runs migrations at open. The reader sets `PRAGMA query_only = ON`. Multi-statement writes use `conn.transaction()` inside a single `call` so a download commit (release update + history row) is atomic.

## 5. SQLite schema and indexing strategy

### 5.1 Connection setup and migrations

```sql
PRAGMA journal_mode = WAL;     -- persistent; set once by the writer
PRAGMA synchronous  = NORMAL;  -- safe under WAL, far fewer fsyncs
PRAGMA foreign_keys = ON;      -- per connection
PRAGMA busy_timeout = 5000;
PRAGMA temp_store   = MEMORY;
PRAGMA cache_size   = -20000;  -- ~20 MB page cache per connection
PRAGMA query_only   = ON;      -- reader connection only
```

Migrations are `(version, include_str!(...))` pairs. The runner applies every version greater than `PRAGMA user_version` in order, each inside its own transaction, then bumps `user_version`. It refuses to open a database newer than the binary (`DbError::TooNew`). A shipped migration is never edited; changes go in a new file. On boot the writer also runs `UPDATE download_history SET status='failed', error_message='interrupted' WHERE status='started'` and sweeps orphan `*.part` files.

### 5.2 `migrations/0001_init.sql`

```sql
CREATE TABLE artists (
    band_id        INTEGER PRIMARY KEY,
    slug           TEXT    NOT NULL,
    name           TEXT    NOT NULL,
    url            TEXT    NOT NULL,   -- canonical base URL, custom domains allowed, no trailing slash
    cover_url      TEXT,
    last_synced_at INTEGER,            -- unix seconds UTC
    created_at     INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE releases (
    item_type              TEXT    NOT NULL CHECK (item_type IN ('album','track')),
    item_id                INTEGER NOT NULL,
    band_id                INTEGER NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE,
    title                  TEXT    NOT NULL,
    artist_name            TEXT    NOT NULL,   -- per release: label pages host many artists
    url                    TEXT    NOT NULL,   -- absolute release URL
    cover_url              TEXT,
    art_id                 INTEGER,
    classification         TEXT    NOT NULL DEFAULT 'unknown'
                           CHECK (classification IN ('unknown','free','nyp','email_required','paid')),
    pricing_json           TEXT,               -- raw pricing signals, enables re-classification offline
    is_preorder            INTEGER NOT NULL DEFAULT 0 CHECK (is_preorder IN (0,1)),
    track_count            INTEGER,
    published_at           INTEGER,
    modified_at            INTEGER,
    metadata_fetched_at    INTEGER,            -- NULL until the release page was parsed
    first_seen_at          INTEGER NOT NULL DEFAULT (unixepoch()),
    last_seen_at           INTEGER NOT NULL DEFAULT (unixepoch()),
    downloaded             INTEGER NOT NULL DEFAULT 0 CHECK (downloaded IN (0,1)),
    downloaded_at          INTEGER,
    downloaded_format      TEXT,
    downloaded_track_count INTEGER,            -- snapshot used to detect bonus tracks
    downloaded_modified_at INTEGER,
    local_path             TEXT,
    has_remote_update      INTEGER NOT NULL DEFAULT 0 CHECK (has_remote_update IN (0,1)),
    remote_update_reason   TEXT CHECK (remote_update_reason IN ('tracks_added','modified')),
    PRIMARY KEY (item_type, item_id)
) STRICT;

CREATE TABLE download_history (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    item_type            TEXT    NOT NULL CHECK (item_type IN ('album','track')),
    item_id              INTEGER NOT NULL,
    band_id              INTEGER NOT NULL,     -- deliberately NOT a FK: history and quota math survive artist removal
    started_at           INTEGER NOT NULL DEFAULT (unixepoch()),
    downloaded_at        INTEGER,              -- set when status becomes 'success'
    format               TEXT    NOT NULL,
    bytes_size           INTEGER,
    status               TEXT    NOT NULL CHECK (status IN ('started','success','failed','cancelled')),
    source               TEXT    NOT NULL DEFAULT 'api' CHECK (source IN ('api','redeem')),
    counts_against_quota INTEGER NOT NULL DEFAULT 0 CHECK (counts_against_quota IN (0,1)),
    error_message        TEXT
) STRICT;

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;  -- JSON values
```

### 5.3 `migrations/0002_indexes.sql`

```sql
CREATE UNIQUE INDEX idx_artists_url  ON artists(url);
CREATE INDEX idx_artists_name        ON artists(name COLLATE NOCASE);

-- Table view: per-artist browsing, default sort newest first
CREATE INDEX idx_releases_band_pub   ON releases(band_id, published_at DESC, item_type, item_id);
CREATE INDEX idx_releases_pub        ON releases(published_at DESC, item_type, item_id);
CREATE INDEX idx_releases_class_dl   ON releases(classification, downloaded, band_id);

-- Small, hot partial indexes
CREATE INDEX idx_releases_todo       ON releases(band_id, published_at DESC)
    WHERE downloaded = 0 AND classification IN ('free','nyp');
CREATE INDEX idx_releases_updates    ON releases(band_id) WHERE has_remote_update = 1;
CREATE INDEX idx_releases_unknown    ON releases(band_id) WHERE classification = 'unknown';
CREATE INDEX idx_releases_downloaded ON releases(band_id) WHERE downloaded = 1;

-- Text sorts
CREATE INDEX idx_releases_title      ON releases(title COLLATE NOCASE);
CREATE INDEX idx_releases_artistname ON releases(artist_name COLLATE NOCASE, title COLLATE NOCASE);

-- History and quota math
CREATE INDEX idx_hist_quota ON download_history(band_id, started_at) WHERE counts_against_quota = 1;
CREATE INDEX idx_hist_item  ON download_history(item_type, item_id, started_at DESC);
```

**Index rationale.** Low-cardinality sort columns (type, tracks, format, status) get no index; SQLite sorts the already-filtered set, which is fine at 100k rows. The partial indexes stay tiny because they only cover the rows the UI keeps asking about (to-do, updated, unknown, downloaded). `ORDER BY` always ends with `item_type, item_id` so ordering is deterministic and virtualized scrolling never reshuffles rows.

### 5.4 Key statements (`db/repo/*`)

```sql
-- Sidebar badges (one pass, all artists). Eligible = free + nyp.
SELECT band_id, COUNT(*) AS total, SUM(downloaded) AS downloaded,
       SUM(classification IN ('free','nyp'))   AS eligible,
       SUM(classification = 'email_required')  AS email_required,
       SUM(classification = 'unknown')         AS unknown
FROM releases GROUP BY band_id;

-- Rolling 30-day quota usage for one artist (served by idx_hist_quota)
SELECT COUNT(*) FROM download_history
WHERE band_id = ?1 AND counts_against_quota = 1
  AND started_at >= unixepoch('now', '-30 days');

-- Discovery upsert: never touches classification, downloaded*, local_path
INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, cover_url, art_id)
VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
ON CONFLICT(item_type, item_id) DO UPDATE SET
    last_seen_at = unixepoch(), title = excluded.title, url = excluded.url,
    cover_url = COALESCE(excluded.cover_url, cover_url),
    art_id    = COALESCE(excluded.art_id, art_id);

-- Metadata write after a page was parsed ...
UPDATE releases SET classification=?3, pricing_json=?4, is_preorder=?5, track_count=?6,
    published_at=?7, modified_at=?8, art_id=COALESCE(?9, art_id), metadata_fetched_at=unixepoch()
WHERE item_type=?1 AND item_id=?2;
-- ... followed in the same transaction by remote-update detection:
UPDATE releases SET
  has_remote_update = CASE WHEN downloaded=1 AND (
        track_count > COALESCE(downloaded_track_count, track_count)
     OR modified_at > COALESCE(downloaded_modified_at, modified_at)) THEN 1 ELSE 0 END,
  remote_update_reason = CASE
     WHEN downloaded=1 AND track_count > COALESCE(downloaded_track_count, track_count) THEN 'tracks_added'
     WHEN downloaded=1 AND modified_at > COALESCE(downloaded_modified_at, modified_at) THEN 'modified'
  END
WHERE item_type=?1 AND item_id=?2;

-- Commit a finished download (one transaction with the history update)
UPDATE releases SET downloaded=1, downloaded_at=unixepoch(), downloaded_format=?3,
    downloaded_track_count=track_count, downloaded_modified_at=modified_at,
    local_path=?4, has_remote_update=0, remote_update_reason=NULL
WHERE item_type=?1 AND item_id=?2;
UPDATE download_history SET status='success', downloaded_at=unixepoch(), bytes_size=?5 WHERE id=?6;
```

**Table query builder.** `ORDER BY` comes from an exhaustive `match (SortKey, SortDir)` returning `&'static str` (never from user text). `WHERE` fragments are fixed strings per `ReleaseFilter`, with all values bound:

| Filter | Fragment |
| --- | --- |
| Free / NYP / Email Required / Paid | `classification = 'free'` / `'nyp'` / `'email_required'` / `'paid'` |
| Not Downloaded | `downloaded = 0 AND classification IN ('free','nyp','email_required','unknown')` (everything obtainable without paying) |
| Updated / Modified | `has_remote_update = 1` (label shows `+TRACKS` for the strong signal, `MOD` for the weak one) |
| Search | `(title LIKE ?n ESCAPE '!' OR artist_name LIKE ?n ESCAPE '!')` with `%`, `_`, `!` escaped by `!` |

`tracks_added` is a strong signal; `modified` is weak because `mod_date` changes on any edit, so the UI treats them differently.

**Plan tests.** `EXPLAIN QUERY PLAN` assertions: the per-artist default-sort query uses `idx_releases_band_pub`, the quota query uses `idx_hist_quota`, and neither contains `SCAN releases`.

## 6. Bandcamp client contract (`src/bandcamp/`)

All selectors and JSON field names below come from the brief and common page structure. Treat them as hypotheses until the Phase 2 fixtures confirm them, and make each parser tolerant (`Option` everywhere, `#[serde(default)]`).

### 6.1 Input normalisation (`input.rs`)

| Input | Normalised base URL |
| --- | --- |
| `artistname` (matches `^[a-z0-9][a-z0-9-]{0,62}$`) | `https://artistname.bandcamp.com` |
| `https://artistname.bandcamp.com/music` | `https://artistname.bandcamp.com` |
| `http://artistname.bandcamp.com/album/x?from=embed` | `https://artistname.bandcamp.com` |
| `https://music.example.com/` (custom domain) | `https://music.example.com` |
| `bandcamp.com`, `www.bandcamp.com`, `daily.bandcamp.com` | rejected: not an artist page |
| empty, whitespace, non-http(s) scheme | typed `InputError` |

Parse with `url::Url`, upgrade to https, lowercase the host, drop path, query and fragment.

### 6.2 Artist resolution and discovery (`discography.rs`)

1. `GET {base}/music` through the gate. If the final URL path starts with `/album/` or `/track/` (single-release artists are redirected), treat the response as a release page and take the band from its embedded data.
2. **`band_id`**, in priority order: `li.music-grid-item[data-band-id]`; `meta[name="bc-page-properties"]` JSON `page_id`; `script[data-band]` JSON `id`. If none match, return `BandIdNotFound` (each branch has its own fixture test).
3. **Name and cover**: `meta[property="og:site_name"]` then `#band-name-location .title`; cover from `og:image`.
4. **Grid items**: every `li.music-grid-item` yields `data-item-id` (`album-123` or `track-456`, split on the first `-`), its own `data-band-id`, `a[href]` joined to the base (so custom domains work), the title from `p.title` excluding a nested `.artist-override` span (label pages), and the art from `img` `src` or `data-original` (lazy-loaded).
5. **Overflow items**: `ol#music-grid[data-client-items]` holds a JSON array of items beyond the initially rendered ones (expect fields like `id`, `type`, `title`, `page_url`, `art_id`, `artist`, `band_id`). Parse it and merge with the grid, de-duplicating by `ItemKey`. Missing this is the classic bug that silently truncates large discographies, so it has a dedicated fixture (`music_grid_overflow.html`).
6. **`band_details`**: `POST https://bandcamp.com/api/mobile/24/band_details` with `{"band_id": N}` is used as a cross-check on item counts and as a fallback when the grid is empty. The HTML grid stays primary because it provides page URLs and works for custom domains. A Phase 2 spike decides whether the API can replace it.
7. **Label pages**: `releases.band_id` is the imported page's id (this is also what the quota is counted against); `artist_name` is stored per release.

### 6.3 Smart incremental diff (`worker/sync.rs`)

```text
sync(band, mode):
  emit Resolving      -> reuse stored band_id/base URL; re-resolve only on AddArtist or FullRefresh
  emit Discovering    -> discovered = fetch_discography(base)           // 1 request, overflow is in the same page
  known = repo.known_keys(band_id)                                      // HashSet<ItemKey>
  new   = discovered - known   -> upsert immediately as 'unknown' (the UI shows them at once)
  gone  = known - discovered   -> never deleted; last_seen_at simply stops advancing
  queue = match mode {
      Incremental     => new + repo.unknown_keys(band_id),             // also resumes interrupted syncs
      CheckDownloaded => new + unknown + repo.downloaded_keys(band_id),// detects bonus tracks
      FullRefresh     => every discovered key,
  }
  for key in queue (sequential, each behind gate.acquire(Page)):
      fetch page -> parse tralbum -> classify -> metadata UPDATE + remote-update flag (one tx)
      emit ReleaseUpdated + SyncProgress
  finalise: artists.last_synced_at = now; emit ArtistStats, SyncFinished
```

Properties to test: idempotent; cancel-safe (each item commits on its own); crash-safe (leftover `unknown` rows are picked up next run); an `Incremental` sync of an unchanged artist costs exactly one HTTP request.

### 6.4 Release page parsing and classification (`tralbum.rs`, `classify.rs`)

Selector `script[data-tralbum]`; the attribute is HTML-escaped JSON, which `scraper` decodes. Deserialize leniently:

```rust
#[derive(Deserialize, Default)]
pub struct Tralbum {
    #[serde(default)] pub id: Option<i64>,
    #[serde(default)] pub item_type: Option<String>,        // else current.type ("a" | "t")
    #[serde(default, rename = "freeDownloadPage")] pub free_download_page: Option<String>,
    #[serde(default)] pub art_id: Option<i64>,
    #[serde(default)] pub trackinfo: Vec<serde_json::Value>,
    #[serde(default)] pub current: Current,
}
// Current: minimum_price (lenient f64), set_price, is_set_price (Flag), require_email (Flag),
//          publish_date, mod_date, band_id, is_preorder (Flag) ... all Option
// Flag = lenient bool: accepts null, 0/1, true/false, "0"/"1"
```

Derivations: `item_type` from the top level or `current.type`; `track_count = trackinfo.len()` (tracks count as 1); dates parsed with `DateTime::parse_from_rfc2822` (`20 Aug 2021 00:00:00 GMT`), failure becomes `None` plus a warning; `published_at` from `publish_date` (fallback `album_release_date`), `modified_at` from `mod_date`; `pricing_json` stores the raw signals so classification can be redone without refetching.

`classify(&PricingSignals) -> Classification` is a pure function:

| `minimum_price` | `require_email` | Free-vs-NYP signal | Result |
| --- | --- | --- | --- |
| missing | any | any | `Unknown` (log, keep `pricing_json`) |
| `> 0` | any | any | `Paid` |
| `== 0` | truthy | any | `EmailRequired` |
| `== 0` | falsy | `freeDownloadPage` present, or fixed price of 0 | `Free` |
| `== 0` | falsy | otherwise (buyer may pay more) | `Nyp` |

Compare floats with an epsilon (`< 0.005`). Pre-orders keep their class but the row's download action is disabled with a tooltip. **Phase 2 task 2.1 must capture at least one fixture per class and pin the expected class in `manifest.toml`; if the fixtures disagree with the Free-vs-NYP discriminator above, the fixtures win.**

### 6.5 Direct download (`download_api.rs`, `worker/downloads.rs`)

Request: `POST https://bandcamp.com/api/mobile/22/buy_for_nothing` with `{"item_type": "album", "item_id": "123"}` (the ID as a string). Require `ok == true`, then read `downloads[format.api_key()].url`. One job runs:

1. **Quota check** (§6.8). Blocked or needs acknowledgement stops here.
2. `buy_for_nothing` through `gate.acquire(Api)`. On `ok:true`, insert the history row (`started`, `counts_against_quota = 1`), because the call itself consumes allowance whether or not the file arrives.
3. `choose_format(downloads, wanted, fallback)`. `Strict` fails with `FormatUnavailable`; `Nearest` walks flac, alac, wav, aiff-lossless, mp3-320, mp3-v0 (lossy requests walk the reverse way). The row's Format column always shows what was actually saved.
4. **Resolve and stream** through `gate.acquire(Download)`. If the response `Content-Type` is `text/html` or `application/json`, it is not the file: run `resolve_download_url()`, which handles the website's "statdownload" handshake (swap `/download/` for `/statdownload/`, add `.vrs=1`, read `download_url` or poll `retry_url` at most 10 times, 2 s apart). Confirm this against a live capture in Phase 3; the function boundary keeps it replaceable.
5. Filename from §6.6; destination by layout (`Flat`: `{out}/...`, `ByArtist`: `{out}/{Artist}/...`).
6. Commit and emit events (below).

**Rejections** (`errors.rs`, the only place with string heuristics): message mentions "email" gives `EmailRequired` and re-classifies the row; mentions "limit", "quota" or "exceed" gives `QuotaExceeded`; anything else is `ApiRejected(message)`. Each variant has a fixture.

**Streaming (`worker/stream.rs`).**

- Write to `{final}.part` in the destination directory through `BufWriter::with_capacity(64 * 1024, file)`; reqwest chunks are network-sized, the 64 KiB buffer is the write granularity.
- Emit `DownloadByteProgress` on a 100 ms timer; speed is an EMA over roughly one second.
- If `Content-Length` is known, require an exact match at the end (`Io("truncated")` otherwise).
- `tokio::select!` against the job's `CancellationToken`; on cancel or error delete the `.part` file.
- No resume in v1 (download URLs are signed and expire). On success: flush, `sync_all`, atomic rename, appending `  (1) ` if the name exists.

**Tracks.** Create `{Artist} - {Title}/`, save the audio file inside, and fetch `cover.jpg` from `https://f4.bcbits.com/img/a{art_id}_10.jpg` (verify the size suffix on a live capture) through the `Cdn` gate class. A cover failure is a warning, not a job failure.

**Albums.** Save the `.zip`. If `auto_extract`, extract in `spawn_blocking` into a folder named after the zip stem using `ZipFile::enclosed_name()` (zip-slip safe) with caps on entry count and total uncompressed size; optionally delete the zip. `local_path` is the folder if extracted, else the zip.

**On success:** one writer transaction (statements in §5.4), then `ReleaseUpdated`, `JobState::Done`, `ArtistStats`, `QuotaStatus`.

### 6.6 Filenames (`filenames.rs`)

`parse_content_disposition(&str) -> Option<String>`: prefer `filename*=` (`charset'lang'pct-encoded`, accept UTF-8 and ISO-8859-1, decode with `percent_decode_str`, fall through on invalid UTF-8), then `filename=` (quoted with backslash escapes, or bare token), else the fallback `sanitize("{Artist} - {Title}")` plus an extension (albums `.zip`; tracks by format: `flac`, `mp3`, `wav`, `aiff`, `m4a` for alac and aac-hi, `ogg` for vorbis).

The sanitiser keeps only the last path component (split on `/` and backslash), replaces control characters and `< > : " / | ? *` with `_`, trims trailing dots and spaces, prefixes `_` to Windows reserved stems (CON, PRN, AUX, NUL, COM1-9, LPT1-9), substitutes `download` for an empty result, and truncates to 180 UTF-8 bytes on a char boundary while preserving the extension.

| Header | Expected |
| --- | --- |
| `attachment; filename*=UTF-8''Sigur%20R%C3%B3s%20-%20%C3%81g%C3%A6tis.zip` | `Sigur Rós - Ágætis.zip` |
| `attachment; filename="A - B.zip"` | `A - B.zip` |
| `filename="fallback.zip"; filename*=UTF-8''real%20name.zip` | `real name.zip` (star form wins) |
| `attachment; filename="../../evil.zip"` | `evil.zip` |
| `attachment; filename="CON.zip"` | `_CON.zip` |
| header absent | `Artist - Title.zip` |

### 6.7 `BandcampGate` (`gate.rs`)

```rust
pub enum GateClass { Page, Api, Download, Cdn }
impl BandcampGate {
    pub async fn acquire(&self, class: GateClass, band_id: Option<i64>) -> GatePermit;
    pub fn report(&self, status: reqwest::StatusCode, retry_after: Option<std::time::Duration>);
}
```

- **Pacing:** `Page` minimum gap is `page_delay_ms` with `page_jitter_pct` jitter (default 1500 ms, plus or minus 25%); `Api` 1000 ms; `Cdn` 250 ms; `Download` is a `Semaphore` of `max_concurrent_downloads` (2) plus one permit per `band_id`.
- **Cooldown:** on 429 or 503, honour `Retry-After` (seconds or HTTP date), else back off exponentially from 30 s to a 15 min cap with 20% jitter. A global `cooldown_until` makes every `acquire` wait, and `RateLimit(Cooldown { .. })` is emitted for the dock. At most 5 retries per request, then `JobError::RateLimited`. Any success resets the backoff.
- **Clients:** one pooled `reqwest::Client` for pages and API (`connect_timeout` 10 s, `timeout` 60 s) and a second for downloads (no total timeout, `read_timeout` 30 s). The User-Agent defaults to `bcvault/{version} (personal archiving tool)`, is user-configurable, and never impersonates a browser.

### 6.8 Quota guard (`worker/quota.rs`)

`decide(used_30d, requested, batch_limit, soft, hard, ack_soft) -> QuotaDecision`:

- `remaining = hard - used`. If it is 0, the result is `Blocked`.
- `n = min(requested, batch_limit, remaining)`; `Truncate` is reported when `n < requested`.
- If `used + n > soft` and the user has not acknowledged, emit `QuotaWarning` and wait; the UI modal confirms "download n more for this artist?".
- Re-check right before each `buy_for_nothing` call, because several batches can be queued concurrently.
- The batch-limit selector offers 25, 50, 100, 150 (default) and 200; values above the soft limit render in the warning colour, and the artist row shows `used/hard`.
- An API-reported exhaustion overrides the local count: the artist is blocked for 30 minutes and its queued jobs fail as `QuotaExceeded`. The local number is only an estimate of the shared allowance, so the UI labels it "your downloads (30d)".

### 6.9 Email-gated releases (`redeem.rs`)

On `EmailRequired` rows the UI offers two actions. **Open in Browser** calls `open::that_detached(url)` directly from the UI (non-blocking). **Redeem URL...** opens a modal whose text field sends `RedeemUrl { target, url, opts }`. The worker then:

1. Validates the URL: `https`, host `bandcamp.com` or `*.bandcamp.com`, path `/download`. Anything else is `InvalidRedeemUrl`, so the app never fetches arbitrary hosts.
2. Fetches the page through the gate and parses `#pagedata[data-blob]` into `download_items` (each with `item_id`, type, `title`, `artist`, `art_id` and a `downloads` map of format to `{ url, size_mb, description }`). An empty list means "link expired or already used".
3. Picks the item matching `target.id` (or the only item) and the format as in §6.5.
4. Streams through the same `stream.rs`, extraction and commit path, with a history row of `source = 'redeem'` and `counts_against_quota = 0` (nothing was spent locally).
5. Marks the release `downloaded` with its `local_path`; the classification stays `email_required`.

## 7. UI specification (`src/ui/`)

### 7.1 Zero-radius theme (`theme.rs`), applied once at startup

```rust
use egui::{epaint::Shadow, style::{HandleShape, ScrollStyle}, Color32, CornerRadius, Stroke, Style, ThemePreference};

pub mod pal {
    use egui::Color32;
    pub const BG0: Color32       = Color32::from_rgb(0x0A, 0x0A, 0x0B); // central panel, text inputs
    pub const BG1: Color32       = Color32::from_rgb(0x12, 0x13, 0x15); // side/top/bottom panels, stripes
    pub const BG2: Color32       = Color32::from_rgb(0x1B, 0x1D, 0x20); // hover, selected row
    pub const BORDER: Color32    = Color32::from_rgb(0x3A, 0x3D, 0x42);
    pub const BORDER_HI: Color32 = Color32::from_rgb(0x8A, 0x8F, 0x98);
    pub const TEXT: Color32      = Color32::from_rgb(0xEC, 0xEC, 0xEC);
    pub const TEXT_DIM: Color32  = Color32::from_rgb(0x9A, 0xA0, 0xA6);
    pub const ACCENT: Color32    = Color32::from_rgb(0x00, 0xD1, 0xB2);
    pub const OK: Color32        = Color32::from_rgb(0x3D, 0xDC, 0x84);
    pub const WARN: Color32      = Color32::from_rgb(0xFF, 0xB0, 0x20);
    pub const ERR: Color32       = Color32::from_rgb(0xFF, 0x4D, 0x4F);
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(ThemePreference::Dark);
    ctx.all_styles_mut(sharpen);          // both themes, so a stray theme switch cannot reintroduce rounding
}

fn sharpen(style: &mut Style) {
    let v = &mut style.visuals;
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius   = CornerRadius::ZERO;   // combo popups, context menus, tooltips
    v.window_shadow = Shadow::NONE;
    v.popup_shadow  = Shadow::NONE;
    v.window_stroke = Stroke::new(1.0, pal::BORDER_HI);
    v.window_fill = pal::BG1;  v.panel_fill = pal::BG0;
    v.extreme_bg_color = pal::BG0;  v.faint_bg_color = pal::BG1;
    v.override_text_color = Some(pal::TEXT);
    v.handle_shape = HandleShape::Rect { aspect_ratio: 0.5 };
    v.selection.bg_fill = pal::ACCENT.gamma_multiply(0.25);
    v.selection.stroke  = Stroke::new(1.0, pal::ACCENT);
    let w = &mut v.widgets;
    for ws in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        ws.corner_radius = CornerRadius::ZERO;
        ws.expansion = 0.0;                         // no grow-on-hover
        ws.bg_stroke = Stroke::new(1.0, pal::BORDER);
    }
    w.hovered.bg_stroke = Stroke::new(1.0, pal::BORDER_HI);
    w.active.bg_stroke  = Stroke::new(1.0, pal::ACCENT);
    style.spacing.scroll = ScrollStyle::solid();    // solid bars; handle radius follows widget visuals
}
```

**Enforcement.**

- Unit test `theme_is_zero_radius`: call `apply`, then for both `Theme::Dark` and `Theme::Light` read `ctx.style_of(theme)` and assert `window_corner_radius`, `menu_corner_radius` and all five `widgets.*.corner_radius` equal `CornerRadius::ZERO`.
- Custom painting goes through `widgets::sharp_rect(painter, rect, fill, stroke)`, which always passes `CornerRadius::ZERO` (check the `Painter::rect` signature for the pinned egui version; recent versions also take a `StrokeKind`, use `Inside` so 1 px borders never bleed outward).
- CI grep guard: `rg -n 'corner_radius\(' src/ui | rg -v 'CornerRadius::ZERO'` must print nothing.
- Sort arrows and status glyphs are drawn with the painter (triangles, squares, crosses), not font glyphs, so they never depend on font coverage.
- Out of scope: the OS-drawn window frame (for example Windows 11 rounding top-level windows) is not controlled by egui. Keep native decorations.

### 7.2 Layout and frame loop (`app.rs`)

Panels must be added in this order so top and bottom span the full width: top, bottom, left, central.

```rust
fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
    self.drain_events();                              // <= 256 try_recv; patches UiState in place
    self.state.flush_debounced_query(&self.cmd_tx);   // sends QueryReleases when filter/sort/search changed
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| ui::toolbar::show(ui, &mut self.state, &self.cmd_tx));
    egui::TopBottomPanel::bottom("dock").resizable(true).default_height(180.0)
        .show(ctx, |ui| ui::bottom_dock::show(ui, &mut self.state, &self.cmd_tx));
    egui::SidePanel::left("artists").resizable(true).default_width(280.0)
        .show(ctx, |ui| ui::sidebar::show(ui, &mut self.state, &self.cmd_tx));
    egui::CentralPanel::default().show(ctx, |ui| ui::release_table::show(ui, &mut self.state, &self.cmd_tx));
    ui::modals::show(ctx, &mut self.state, &self.cmd_tx);
    if self.state.cooldown_visible() { ctx.request_repaint_after(std::time::Duration::from_millis(250)); }
}
```

`UiState` holds `rows: Vec<ReleaseRow>`, `index: HashMap<ItemKey, usize>` (rebuilt on `ReleasesLoaded`, patched on `ReleaseUpdated`), `latest_request_id`, the filter and sort state, `selected: HashSet<ItemKey>`, `selected_artists: BTreeSet<i64>`, `jobs: Vec<JobView>` with a `HashMap<JobId, Progress>`, a 2000-entry log ring buffer, and the latest `Settings`.

Add `WorkerToUiEvent::SettingsUpdated(Settings)` to §4.3: the worker persists `UpdateSettings` and echoes the result, which is how the folder-picker result reaches the UI. The UI may hold a `tokio::runtime::Handle` solely to `spawn` the `rfd::AsyncFileDialog` task, which then sends `UpdateSettings` through a cloned command sender.

### 7.3 Regions

| Region | Contents and behaviour |
| --- | --- |
| **Sidebar** | Artist filter `TextEdit` (in-memory). Virtualized list via `ScrollArea::show_rows(26.0, ..)` with an "All artists" row on top. Each row: clipped name plus a right-aligned monospace badge `downloaded/eligible/total` (tooltip adds email-gated and pending-metadata counts). Click, Ctrl-click and Shift-click multi-select; selected rows get a `BG2` fill and a 1 px accent bar. Add bar: `TextEdit` with hint "Add Artist (URL or Slug)", Enter or `Add` sends `AddArtist`, disabled while pending. Footer: `Sync Selected`, `Sync All`, and a mode `ComboBox` (Incremental, Check downloaded, Full refresh). Active syncs show a thin sharp progress bar under the artist row. |
| **Toolbar** | Row 1: filter chips (`selectable_value`: All, Free, NYP, Email Required, Paid, Not Downloaded, Updated/Modified) and a table search box. Row 2: format `ComboBox` (8 formats), batch-limit `ComboBox` (25/50/100/150/200, values above the soft limit in warning colour), output folder button with truncated path, `Download Selected (n)`, `Download All Eligible` for the selected artist, and the `used/hard (30d)` readout. |
| **Central table** | `egui_extras::TableBuilder` (below). |
| **Bottom dock** | Tabs `Queue` and `Errors`. Queue row: label, state, `ProgressBar` with `.corner_radius(CornerRadius::ZERO)` and text `412.3 / 980.0 MB  3.20 MB/s`, cancel/retry button. Header chip shows gate state: `OK`, `PACING 1.2 s`, or `COOLDOWN 00:41 (HTTP 429)`. Errors: virtualized table from the ring buffer, level filter, copy-to-clipboard. |
| **Modals** | `Window` with `collapsible(false)`, `resizable(false)`, centered, over a dimmed backdrop; Esc closes. Redeem URL (text field, validation message), quota warning (`n more for this artist? used/soft/hard`), settings (delays, user-agent, concurrency, extraction, layout, fallback, thresholds), confirm artist removal. |
| **Thumbnails** (optional, off by default) | `egui_extras::install_image_loaders(ctx)` at startup; visible-row keys are debounced into `RequestThumbs`; the worker caches covers (CDN gate class) and emits `ThumbReady`; the table renders `Image::new("file://...")` at 20 px. |

**Release table**

```rust
TableBuilder::new(ui)
    .striped(true).resizable(true).sense(egui::Sense::click())
    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
    .column(Column::exact(28.0))                              // checkbox
    .column(Column::exact(28.0))                              // status glyph
    .column(Column::initial(160.0).at_least(80.0).clip(true)) // artist
    .column(Column::initial(300.0).at_least(200.0).clip(true))// title
    .column(Column::exact(60.0))                              // type
    .column(Column::exact(92.0))                              // released
    .column(Column::exact(56.0))                              // tracks
    .column(Column::exact(110.0))                             // classification
    .column(Column::exact(96.0))                              // downloaded format
    .column(Column::remainder().at_least(170.0))              // actions
    .header(24.0, |mut h| { /* clickable headers toggle (SortKey, SortDir); painted arrow */ })
    .body(|body| body.rows(24.0, state.rows.len(), |mut row| { /* draw state.rows[row.index()] */ }));
```

- Header click toggles sort and re-queries immediately. The header checkbox selects all visible auto-downloadable rows. Shift-click selects a range from the last clicked row.
- Row actions: Free or NYP show `Download`; `EmailRequired` shows `Open in Browser` and `Redeem URL...`; `Paid` shows `Open`; downloaded rows show `Open Folder` and `Re-download`; rows with `has_remote_update` show a `+TRACKS` or `MOD` badge and `Update`. Pre-orders disable download with a tooltip.
- Performance: no per-frame sorting or filtering (SQLite does it), pre-formatted date and track strings cached beside each row, `Arc<str>` where strings repeat, and only visible rows are built by `rows()`. Target: frame time under 4 ms (release build) with 100k rows. An F3 overlay shows `unstable_dt`, row count and event backlog.

## 8. Implementation phases and verification milestones

Each task is a commit-sized unit with its own tests. A phase is done only when every box in its milestone is checked.

### Phase 0: Scaffold (about 0.5 day)

- 0.1 Create the crate, add dependencies from §2, `rust-toolchain.toml`, and `scripts/check.sh` (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`).
- 0.2 `main.rs`: tracing init, `ProjectDirs`, multi-thread runtime, `eframe::run_native` with `ViewportBuilder::default().with_inner_size([1360.0, 820.0]).with_min_inner_size([960.0, 560.0])` and `persist_window: true`.
- 0.3 `ui::theme::apply` plus the zero-radius test; empty four-panel layout.

**Milestone**

- [ ] Window opens with four empty, square-cornered panels; `scripts/check.sh` is green.
- [ ] `theme_is_zero_radius` passes and the CI grep guard is wired.

### Phase 1: Database and core types (1 to 1.5 days)

- 1.1 `types.rs`: all §4.1 and §4.2 types, with `as_str` / `parse` for DB enums and round-trip tests.
- 1.2 `error.rs`, `paths.rs`.
- 1.3 `Db` actor and `DbPool` (§4.4) with role-specific PRAGMAs.
- 1.4 Migrations `0001` and `0002` plus the runner (idempotent, `TooNew`).
- 1.5 Repos: artists (upsert, list, delete); releases (discovery upsert, metadata update with flagging, `known_keys`, `unknown_keys`, `downloaded_keys`, `query_releases`, `artist_stats`, `commit_download`); history (`insert_started`, `finish`, `quota_used_30d`, `recover_interrupted`); settings (JSON get/set).
- 1.6 `quota::decide` as a pure function with table-driven tests.
- 1.7 Test support: `seed(n)` helper that generates up to 100k releases.

**Milestone**

- [ ] Fresh DB lands at `user_version = 2`; reopening is a no-op; a DB at version 99 returns `TooNew`.
- [ ] `journal_mode` reports `wal` and `foreign_keys` is on for both connections.
- [ ] `tests/db_concurrency.rs`: while the writer holds a 500 ms transaction, the reader answers `query_releases` in under 50 ms.
- [ ] A discovery upsert on a downloaded row leaves `classification`, `downloaded*` and `local_path` untouched.
- [ ] Raising `track_count` on a downloaded row sets `has_remote_update = 1` with `tracks_added`; `commit_download` clears it.
- [ ] Quota query respects the 30-day boundary and ignores `counts_against_quota = 0` rows.
- [ ] `EXPLAIN QUERY PLAN` assertions from §5.4 pass.
- [ ] With 100k seeded rows, the default per-artist query and the stats query run comfortably under 20 ms and 30 ms (assert 5x looser in CI).

### Phase 2: Scraper and API client (2 to 3 days)

- 2.1 `examples/capture_fixtures.rs`: polite one-shot grabber (it uses the gate) for a handful of artists passed on the command line. Saves `/music`, one release page per class (free, NYP, email-gated, paid, a track), a redeem page supplied by the owner, and the `band_details` and `buy_for_nothing` responses. Sanitise cookies, signed URLs and email addresses, and write `manifest.toml` with the expected parse of each file. This is the only task that talks to the live site.
- 2.2 `input.rs` with the §6.1 table as tests.
- 2.3 `gate.rs` with unit tests using `tokio::time::pause()` (add `test-util` to the dev-dependency `tokio` features).
- 2.4 `discography.rs`: grid, `data-client-items` overflow, label pages, single-release redirect, `band_details` cross-check.
- 2.5 `tralbum.rs` and `classify.rs`: lenient serde, per-class fixtures, null / string / number variants.
- 2.6 `BandcampClient` facade: `resolve_artist`, `fetch_discography`, `fetch_release`, `band_details`, with injectable base URLs for tests.
- 2.7 `redeem.rs` page parsing.
- 2.8 `worker/sync.rs`, `bridge.rs` and a supervisor skeleton handling `AddArtist`, `Sync` and `QueryReleases`.

**Milestone**

- [ ] Every fixture parses to the expectation pinned in `manifest.toml` (class, item type, track count, dates, band id).
- [ ] The overflow fixture yields grid plus `data-client-items` entries with no duplicates.
- [ ] wiremock: adding a 20-release artist yields 20 classified rows, with page requests spaced by at least the jittered minimum gap.
- [ ] A second `Incremental` sync issues exactly one request. After the mock adds one release, the next sync issues exactly two.
- [ ] A 429 with `Retry-After: 2` pauses all gate traffic for at least 2 s, emits `RateLimit` events, then continues.
- [ ] Cancelling mid-sync and re-running processes only the remaining `unknown` rows.
- [ ] `CheckDownloaded` flags a downloaded album whose `trackinfo` grew as `tracks_added`.
- [ ] A compile-time `assert_send` over the sync future proves no `Html` crosses an `.await`.
- [ ] Manual, `--features live-tests`: `cargo run --example headless -- add <slug>` prints a class summary using at most (discography size + 3) requests.

### Phase 3: Async download engine (2 to 3 days)

- 3.1 `filenames.rs` with the §6.6 vectors.
- 3.2 `download_api.rs`: `buy_for_nothing`, `choose_format`, `resolve_download_url`, rejection mapping, fixtures for every variant.
- 3.3 `stream.rs`: `.part` file, 64 KiB `BufWriter`, 100 ms progress throttle, EMA speed, cancel, size check, atomic rename.
- 3.4 `extract.rs`: zip-slip-safe extraction with entry and size caps.
- 3.5 `quota.rs` wired to history rows and the `QuotaWarning` flow.
- 3.6 `downloads.rs`: FIFO per artist, global semaphore, pause and resume, cancel, retry with backoff on `RateLimited`, full event emission.
- 3.7 Track layout with `cover.jpg`, album layout with optional extraction.
- 3.8 `RedeemUrl` path.
- 3.9 `examples/headless.rs download` subcommand.

**Milestone**

- [ ] wiremock album end-to-end: `buy_for_nothing`, then a zip served with a `filename*` header. The file is named correctly, its SHA-256 matches the served bytes, the release row has format, path and track snapshot, and the history row is `success` with `counts_against_quota = 1`.
- [ ] Track end-to-end: `{Artist} - {Title}/` contains the audio and `cover.jpg`; a cover 404 logs a warning and the job still completes.
- [ ] Cancel mid-stream removes the `.part`, ends in `Cancelled`, and leaves the row not downloaded. A truncated body fails with `Io` and leaves no final file.
- [ ] Quota: with 199 counted rows a batch of 5 is truncated to 1; at 200 it is `Blocked`; crossing 150 without acknowledgement emits `QuotaWarning` and makes zero HTTP calls.
- [ ] An `ok:false` email rejection re-classifies the row as `email_required`; a quota rejection blocks that artist for 30 minutes.
- [ ] A zip containing `../evil.txt` is rejected; `delete_zip_after_extract` is honoured.
- [ ] Progress events measure at most 12 per second per job, and with `max_concurrent_downloads = 2` never more than two jobs are `Downloading`.
- [ ] Redeem: a non-Bandcamp host is rejected; the valid fixture page downloads and marks its row.
- [ ] Manual: streaming a 1 GB body keeps RSS under 50 MB above baseline.

### Phase 4: Zero-radius `egui` table and state wiring (3 to 4 days)

- 4.1 `app.rs`: `App::new` (image loaders, theme, initial `QueryReleases`), event drain, debounced queries with request ids, `UiBridge` wiring.
- 4.2 Sidebar. 4.3 Toolbar. 4.4 Release table (sorting, selection, row actions). 4.5 Bottom dock. 4.6 Modals. 4.7 Optional thumbnails. 4.8 Settings window. 4.9 Keyboard shortcuts (Ctrl+F search, Space toggle, Enter primary action) and the F3 debug overlay.

**Milestone**

- [ ] Adding an artist by slug, by URL, by `/music` URL and by custom domain shows rows appearing incrementally while the sync runs. Dragging the window during a sync stays smooth (frame time under 8 ms in the overlay).
- [ ] Rapid typing in the search box applies only the newest result (request-id test); filter and sort changes render within a frame of the result arriving.
- [ ] 100k-row synthetic DB: release-build frame time p95 under 4 ms while scrolling; memory under 200 MB.
- [ ] Download 3 selected releases: bars update at about 10 Hz with MB, total and MB/s; cancel works; finished rows flip to downloaded in place without a table reload.
- [ ] `EmailRequired` row: `Open in Browser` launches the default browser; the redeem modal accepts a valid link and rejects a foreign host.
- [ ] The quota modal appears above 150 and refuses beyond 200.
- [ ] **Visual audit, saved as `docs/visual-audit.png`:** window, panels, buttons in all four states, text edit, combo box and popup, tooltip, context menu, checkbox, progress bar, scrollbar track and handle, selection highlight, modal windows, table header, resize handles. Every corner is square; the grep guard prints nothing.
- [ ] Killing the process mid-download leaves a clean state: the next boot removes `.part` files and marks the history row `interrupted`.

### Phase 5: Hardening and release (1 to 2 days)

- Windows extended-length paths and case-insensitive collision handling for the output tree; the `ByArtist` layout; settings export.
- Packaging (`cargo-dist` or `cargo-bundle`), licence file, README with the politeness and quota notes, log rotation.
- `live-tests` contract suite for drift detection (run manually, for example monthly) and `docs/endpoints.md` recording the payloads actually observed.

## 9. Risks and open questions

| Risk | Mitigation |
| --- | --- |
| Undocumented endpoints change | Isolated in `bandcamp/`, fixture contract tests, `live-tests` suite, typed errors that show "Bandcamp changed its response" with the raw payload in the log. |
| Rate limiting or blocking | One gate for all traffic, sequential page fetches, jittered delays, honoured `Retry-After`, exponential cooldown, conservative defaults. |
| Shared per-artist allowance is invisible locally | Local 30-day estimate labelled as such, soft warning and hard stop, and an API-reported exhaustion overrides the count. |
| Free vs NYP vs email-gated semantics are ambiguous (pre-orders, streaming-only) | Fixtures per class, raw `pricing_json`, an `unknown` state, offline re-classification. |
| Terms and artist intent | The app only calls the download endpoint for releases classified Free or NYP, never tries to bypass an email gate or payment, and ships polite defaults. Keep it that way. |
| Huge discographies and label pages | Sequential, resumable metadata queue with progress; per-release `artist_name`. |
| macOS requires dialogs on the main thread | Use `rfd::AsyncFileDialog` only; never the blocking API from a worker thread. |
| Case-insensitive filesystems collide on names | De-duplicate with `  (1) ` suffixing at rename time. |

**Defaults assumed; confirm or override:**

1. "Eligible" in the sidebar badge means Free plus NYP; email-gated counts appear in the tooltip.
2. Output layout defaults to `Flat` (as in the brief); `ByArtist` is a setting.
3. Format fallback defaults to `Nearest`, because a `Strict` failure would still have spent an allowance slot. The Format column always shows what was actually saved.

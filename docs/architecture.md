# Architecture and Developer Guide

`bcvault` is a workspace of three Rust crates (`Edition 2024`, MSRV `1.89`) designed around strict separation between a headless asynchronous engine (`bcvault-core`) and two immediate-mode desktop front-ends (`bcvault` Classic and `bcvault-fx` Eye-Candy Edition).

---

## 1. Workspace Structure

```text
bcvault/
├── crates/
│   ├── bcvault-core/        # Headless engine: scraping, BandcampGate, SQLite WAL, worker services, library sync
│   ├── bcvault/             # Classic high-density table GUI (eframe/egui)
│   └── bcvault-fx/          # Eye-Candy GUI: Cover Wall, spring physics, OKLab themes, sidecar DB, stats views
├── docs/
│   ├── architecture.md      # System architecture, concurrency, worker protocol, and developer guide
│   ├── database.md          # Complete SQLite schema reference (bcvault.db v8 & fx.sqlite v1)
│   ├── endpoints.md         # Bandcamp scraping, mobile API, checkout, redeem, and CDN protocol reference
│   ├── library.md           # Artist folder links, exact/fuzzy matching, templates, and disk verification
│   └── user-guide.md        # End-to-end user manual for Classic, FX, and Headless CLI
├── examples/
│   ├── headless.rs          # Terminal CLI for adding artists, syncing, and batch downloading
│   └── capture_fixtures.rs  # Maintainer utility for capturing sanitized test fixtures
└── scripts/
    ├── check.sh             # Full quality gate (fmt, clippy, architectural guards, workspace tests)
    ├── guards.sh            # Static analysis guards enforcing UI purity, zero rounding, and gate discipline
    └── install-fx.sh        # Local desktop binary installer
```

### Crate Responsibilities

| Crate | Role | Key Modules |
| :--- | :--- | :--- |
| **`bcvault-core`** | Pure domain logic, HTTP client, rate limiter, SQLite persistence, local filesystem scanner/matcher, and background worker supervisor. Zero GUI dependencies. | `bandcamp`, `db`, `library`, `worker`, `paths`, `types`, `error` |
| **`bcvault`** | High-density spreadsheet-style desktop UI optimized for low latency and rapid triage across 100,000+ releases. | `app`, `ui::{toolbar, sidebar, release_table, bottom_dock, modals, thumbs, theme, state, widgets}` |
| **`bcvault-fx`** | Visual archival desktop UI featuring a virtualized Cover Wall, FLIP layout animations, OKLab perceptual color engine, slide-over Detail Drawer, fuzzy Command Palette, and interactive Stats visualizations. | `app`, `state`, `views::{wall, table, artists, stats, sidebar, toolbar, dock}`, `overlays::{drawer, modals, palette, toasts}`, `services::{thumbs, details, search}`, `motion`, `paint`, `theme`, `db` |

---

## 2. Hard Architectural Invariants

Every crate in the workspace is governed by eight non-negotiable invariants enforced by `clippy` and `scripts/guards.sh`:

1. **Memory Safety & Panic Freedom (`#![forbid(unsafe_code)]`)**:
   - `unsafe` blocks are forbidden workspace-wide.
   - `unwrap()`, `expect()`, and `panic!` are denied by clippy outside unit tests and `main` bootstrap initialization.
2. **UI-Thread Purity**:
   - `App::update` (and every module under `ui/`, `views/`, `overlays/`, `paint/`, `motion/`, and `theme/`) may only read in-memory state, emit draw primitives, send non-blocking channel commands, and drain `try_recv` events.
   - Blocking calls (`block_on`), database queries (`rusqlite`), network requests (`reqwest`), synchronous filesystem I/O (`std::fs`, `tokio::fs`), and synchronous file dialogs (`rfd::FileDialog`) are strictly forbidden on the UI thread (verified by `scripts/guards.sh`). File pickers run via `rfd::AsyncFileDialog` inside spawned tokio tasks.
3. **Single Bandcamp Network Gate (`BandcampGate`)**:
   - Every HTTP request to a Bandcamp or `bcbits.com` host must acquire a permit from `BandcampGate`.
   - `reqwest::Client` instances may only be constructed inside `crates/bcvault-core/src/bandcamp/gate.rs` (and `BandcampClient::new`).
4. **`!Send` HTML Parsing Discipline**:
   - `scraper::Html` is `!Send`. All HTML parsing is isolated inside synchronous functions (`parse_discography_html`, `parse_release_html`, `parse_redeem_html`) that return owned domain structs before any `.await` point.
5. **Sharp Square Geometry**:
   - Both UIs enforce a sharp editorial aesthetic with `CornerRadius::ZERO` everywhere. Round primitives (`Shape::circle`, non-zero `corner_radius`, round spinners) are rejected by `scripts/guards.sh`.
6. **Zero-Cost Motion at Rest**:
   - In `bcvault-fx`, spring animations (`motion`) only request `ctx.request_repaint()` while actively moving. Once all springs settle within epsilon, the UI idles at 0% CPU until the next input or worker event. All animations honor `EffectsLevel` (`Full`, `Reduced`, `Off`) and OS `reduced_motion`.
7. **Non-Destructive Library Operations**:
   - `bcvault` never modifies, renames, moves, or deletes user audio files in linked artist folders.
   - Every database state change from library sync, verification, or reconciliation is recorded in `library_batches` and `library_events` with full before/after JSON snapshots and one-click undo.
8. **No Secrets on Disk**:
   - Signed download tokens (`sig=`), payment IDs, cookies, and credentials are never written to SQLite, settings JSON, log files, or committed test fixtures.

---

## 3. Concurrency and Worker Architecture

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                             UI Thread (egui)                            │
│  • Renders immediate-mode views at 60+ FPS from in-memory state         │
│  • Sends UiToWorkerCommand (mpsc::unbounded_channel)                    │
│  • Polls WorkerToUiEvent via try_recv() on repaint                      │
└───────────────┬─────────────────────────────────────────▲───────────────┘
                │ UiToWorkerCommand                       │ WorkerToUiEvent + RepaintHook
                ▼                                         │
┌─────────────────────────────────────────────────────────────────────────┐
│                       Core Supervisor (Tokio Runtime)                   │
│                                                                         │
│  ┌─────────────────┐  ┌──────────────────┐  ┌────────────────────────┐  │
│  │   SyncService   │  │ DownloadManager  │  │     LibraryService     │  │
│  │ (Discography &  │  │ (Checkout, Zip   │  │ (Scan, Match, Verify,  │  │
│  │  Release Pages) │  │  Extract, Quota) │  │  Relink, Batch Undo)   │  │
│  └────────┬────────┘  └────────┬─────────┘  └───────────┬────────────┘  │
│           │                    │                        │               │
│           ▼                    ▼                        ▼               │
│  ┌───────────────────────────────────┐     ┌─────────────────────────┐  │
│  │           BandcampGate            │     │         DbPool          │  │
│  │ (Page / Api / Cdn / Download      │     │ • writer: AsyncConn     │  │
│  │  pacing, jitter & 429 cooldowns)  │     │ • reader: AsyncConn     │  │
│  └───────────────────────────────────┘     └─────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────┘
```

### 3.1 Database Connection Pool (`DbPool`)

`DbPool` (`crates/bcvault-core/src/db/mod.rs`) wraps two dedicated background SQLite threads over `bcvault.db` in WAL (`Write-Ahead Logging`) mode:
- **`writer` (`AsyncConnection`)**: Serializes all `INSERT`, `UPDATE`, `DELETE`, and schema migration transactions.
- **`reader` (`AsyncConnection`)**: Opens a read-only WAL connection (`SQLITE_OPEN_READ_ONLY`) so UI table filtering, sorting, and search queries (`UiToWorkerCommand::QueryReleases`) execute in parallel without blocking behind active syncs or download writes.

### 3.2 UI-to-Worker Message Protocol

All interaction between the UI and `bcvault-core` flows through two typed enums in `crates/bcvault-core/src/worker/messages.rs`:

- **`UiToWorkerCommand`**:
  - **Artist Management**: `AddArtist`, `RemoveArtist`, `RenameArtist`
  - **Synchronization**: `Sync { scope, mode }`, `CancelSync`
  - **Catalog Queries & Thumbnails**: `QueryReleases { request_id, query }`, `RequestThumbs { keys }`
  - **Downloads & Quotas**: `EnqueueDownloads`, `RedeemUrl`, `SetReleaseDownloaded`, `SetReleaseUnavailable`, `CancelJob`, `RetryJob`, `CancelAllJobs`, `PauseQueue`, `ResumeQueue`, `ClearFinished`
  - **Library Sync**: `LinkArtistFolder`, `AddAndLinkArtistFolder`, `UnlinkArtistFolder`, `RepointArtistFolder`, `MatchArtistFolder`, `ApplyReconcile`, `UndoBatch`, `VerifyFiles`, `ResolveMissing`, `ResolveUnavailable`, `RelinkRelease`, `MarkArchived`, `UnmarkDownloaded`, `IgnoreLocal`, `CancelLibrary`
  - **System**: `UpdateSettings`, `Shutdown`

- **`WorkerToUiEvent`**:
  - **Lifecycle & State**: `Ready`, `SettingsUpdated`, `ArtistUpserted`, `ArtistRemoved`, `ArtistStats`, `ReleasesLoaded`, `ReleaseUpdated`
  - **Sync & Network**: `SyncProgress`, `SyncFinished`, `RateLimit`, `ThumbReady`
  - **Downloads**: `JobQueued`, `JobState`, `DownloadByteProgress`, `QuotaStatus`, `QuotaWarning`
  - **Library**: `LibraryProgress`, `ReconcileReport`, `FolderStatus`, `LibraryApplied`, `MissingFound`, `FolderUnavailable`
  - **Diagnostics**: `Log(LogEntry)`

When the worker emits an event through `UiBridge`, it invokes the registered `RepaintHook` (`ctx.request_repaint()`), waking the GUI event loop immediately.

### 3.3 `bcvault-fx` Sidecar Services

In addition to the core worker, `bcvault-fx` spawns three specialized background services (`crates/bcvault-fx/src/services/mod.rs`) that operate on the attached `fx.sqlite` sidecar database:
1. **`ThumbService` (`services/thumbs.rs`)**:
   - Fetches `_10.jpg` (1200×1200) or `_16.jpg` (700×700) artwork via `BandcampGate` (`GateClass::Cdn`).
   - Generates multi-resolution disk cache tiers (`T64`, `T192`, `T384`, `Hero`) in `{cache_dir}/covers/{art_id % 256:02x}/`.
   - Computes an 8×8 RGB preview (`tiny8`, 192 bytes) for instant zero-latency placeholders, extracts 5 dominant color swatches, and computes perceptual OKLCH `hue`, `chroma`, and `luma` values stored in `fx.sqlite`.
2. **`DetailService` (`services/details.rs`)**:
   - Triggered when the slide-over Detail Drawer rests on a release for `>= 400 ms`.
   - Lazily fetches and caches tracklists, durations, `about` liner notes, `credits`, and Bandcamp genre tags into `fx.sqlite` (`release_detail`).
   - Streams 128 kbps MP3 track previews on demand.
3. **`SearchService` (`services/search.rs`)**:
   - Runs fuzzy matching via `nucleo-matcher` across artists, releases, and commands for the `Ctrl+K` Command Palette.

---

## 4. `BandcampGate` Rate Limiting & Politeness

`BandcampGate` (`crates/bcvault-core/src/bandcamp/gate.rs`) is a shared, cloneable concurrency and rate-limiting coordinator with four traffic classes:

| `GateClass` | Default Pacing | Jitter | Concurrency | Used For |
| :--- | :--- | :--- | :--- | :--- |
| **`Page`** | `1500 ms` (configurable `500..=10000 ms`) | `±25%` (configurable `0..=50%`) | `1` (serialized) | Artist `/music` pages, release pages, redeem pages |
| **`Api`** | `1000 ms` | `0%` | `1` (serialized) | `band_details`, `buy_for_nothing`, `/statdownload/` polling |
| **`Cdn`** | `250 ms` | `0%` | `1` (serialized) | Cover art downloads from `f4.bcbits.com` |
| **`Download`** | Cooldown-gated | `0%` | Global semaphore (`max_concurrent_downloads`, default `2`) + **per-artist semaphore (`1`)** | Binary audio `.zip` / track stream transfers |

### Cooldown and Backoff Behavior
- When any request receives HTTP `429 Too Many Requests` or `503 Service Unavailable`:
  1. `BandcampGate` parses the `Retry-After` header (supporting both integer seconds and RFC 2822 HTTP dates).
  2. If `Retry-After` is absent, it computes an exponential backoff: `min(30s * 2^(consecutive_429s - 1), 900s)` with `±20%` jitter.
  3. All four gate classes pause until the cooldown expires, and `WorkerToUiEvent::RateLimit(RateLimitStatus::Cooldown { until, reason })` is broadcast to the UI status dock.
- A subsequent `2xx` response resets `consecutive_429s` to `0` and restores `RateLimitStatus::Idle`.

---

## 5. Application Paths, Locking, and Logging

`AppPaths` (`crates/bcvault-core/src/paths.rs`) resolves platform directories via `directories::ProjectDirs::from("org", "bcvault", "bcvault")`, with support for `--data-dir=<path>` CLI arguments and the `BCVAULT_DATA_DIR` environment variable:

- **`bcvault.lock`**: On startup, both `bcvault` and `bcvault-fx` acquire an exclusive OS file lock on `{data_dir}/bcvault.lock`. Launching a second instance while one is running exits cleanly with an `ErrorKind::WouldBlock` message to prevent concurrent corruption or conflicting background jobs.
- **Log Rotation**: Logs are written to `{data_dir}/logs/bcvault.log` and rotated automatically at startup whenever `bcvault.log` exceeds `5 MiB` (retaining up to 5 rotated files: `bcvault.log.1` .. `bcvault.log.5`).
- **Windows Long Path Support**: `to_os_safe_path` automatically prefixes paths `>= 240` characters with `\\?\` on Windows so deeply nested artist/album directories never fail `MAX_PATH` (`260` characters), while `dunce` strips `\\?\` prefixes for clean display and canonical comparisons.

---

## 6. Quality Gate and Testing

All tests in the workspace run 100% offline using `wiremock` HTTP servers, embedded HTML/JSON/audio fixtures (`crates/bcvault-core/tests/fixtures/`), and temporary directories (`tempfile`).

### Running the Quality Gate

```bash
# Run formatting check, clippy (-D warnings), architectural guards, and all unit/integration tests
./scripts/check.sh

# Fast pre-commit check (fmt + clippy + guards without full test suite)
./scripts/check.sh --fast

# Run architectural static guards directly
./scripts/guards.sh
```

### Commit Convention

Commits follow **Conventional Commits 1.0.0** with mandatory scopes (`workspace`, `core`, `db`, `bandcamp`, `gate`, `worker`, `sync`, `downloads`, `quota`, `types`, `classic`, `fx`, `library`, `mail`, `tooling`, `deps`, `docs`, `plans`, `release`) and a `Task:` trailer (`Task: <ID>` or `Task: none`). Database migrations that bump `MAX_VERSION` require `!` after the scope and a `BREAKING CHANGE:` footer.

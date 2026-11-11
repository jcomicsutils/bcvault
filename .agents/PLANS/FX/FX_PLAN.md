# bcvault-fx: Eye-Candy Edition, Agent-Ready Implementation Plan

*Rust 2024 · eframe/egui (same pinned minor as the classic app) · working name `bcvault-fx` · prepared 6 Nov 2026 · companion to `Initial Plan.md`*

> Status 11 Nov 2026: **implemented** (F0 to F8; tracked in `.agents/PROGRESS.md`). Later plans amended the main schema (migrations `0003` to `0008`) and added library UI to FX; the code is authoritative where it differs.

## 0. How to use this plan

**Goal.** A second, separately installable front-end for the same engine: a cover-wall library browser with fluid, interruptible animations, real album thumbnails, a detail drawer, a command palette, history-style navigation, a stats view, switchable themes and an optional shader "atmosphere" layer. **Every corner stays square.** The classic `bcvault` app keeps working untouched; both share one sync/download engine.

**Execution protocol for the implementing agent**

1. Rules 2 to 7 of `Initial Plan.md §0` still apply unchanged: fixtures for Bandcamp-facing code, `#![forbid(unsafe_code)]`, no `unwrap`/`expect`/`panic!` outside tests and bootstrap, **UI-thread purity**, **one `BandcampGate`**, `scraper::Html` never across an `.await`.
2. Work phase by phase (§14). Keep `PROGRESS.md` with the current task ID and open questions. A phase is done only when every box in its milestone is checked.
3. **Squareness is a hard constraint.** The grep guard in §3.4 runs in `scripts/check.sh` and CI. Nothing round ships, including spinners, avatars, dots, radio buttons and scrollbars.
4. **Motion costs nothing at rest.** Animations are springs that retarget mid-flight (interruptible) and stop requesting repaints when settled. A test asserts the idle repaint delay (§13).
5. **Every effect checks `EffectsLevel`** (`Off` / `Low` / `Full`) and `reduced_motion`. `Off` must look and perform like the classic app.
6. **Phase F0 is a pure refactor.** No behaviour change to the classic app. All its tests stay green before F1 starts.
7. **Versions.** `egui`, `eframe`, `egui_extras` and `egui_glow` are pinned once in `[workspace.dependencies]`, shared by both apps. At the time of writing crates.io shows egui far ahead of the 0.33 baseline (0.36.x, which also needs a much newer Rust toolchain), so upgrading is its own chore after this plan, not part of it. Any third-party egui add-on must resolve to the same egui; check with `cargo tree -d` that exactly one egui is in the graph.

**Decisions made for you (veto any of them)**

| Decision | Why |
| --- | --- |
| Cargo workspace: `bcvault-core` (engine) + `bcvault` (classic UI) + `bcvault-fx` (new UI) | Classic stays as the safe fallback; both get every future sync and download fix. |
| **Main DB schema is not touched** (`user_version` stays 2). FX-only data lives in a sidecar `fx.sqlite` | The classic app refuses newer schemas (`TooNew`). A sidecar means either app can open the shared DB. |
| FX reads the main DB through a read-only `ATTACH` on its own reader connection | SQL joins between releases and cover palettes (sort by hue, filter by colour) without copying data. |
| Thumbnails on by default, one CDN fetch per cover, smaller tiers derived locally | Politeness (fewer requests) and speed (small tiers decode in microseconds). |
| Track lists, tags and descriptions are fetched **lazily** when a drawer is opened | Sync stays at one request per release; no extra background crawling. |
| Charts hand-painted, no `egui_plot` | Square bars, treemaps and heat squares are easy to paint and one less version-coupled dependency. |
| One `FxAction` enum drives keyboard, palette, clicks and tests | Single dispatcher, shortcuts and palette can never disagree. |

## 1. Feature catalogue

| Feature | What the user sees | Level |
| --- | --- | --- |
| **Cover Wall** | Virtualized grid of real covers, hover lift with hard offset shadow, in-frame parallax, FLIP animation when filters/sort/size change, marquee selection | Low+ |
| **Table** | Classic table plus 24 px thumbnails, sliding hover highlight, row flash when updated, animated sort arrow | Low+ |
| **Artists** | Banner per artist with a cover mosaic and odometer counts; click zooms the banner into that artist's wall | Low+ |
| **Stats** | Squarified treemap, download heat-calendar, releases-per-year bars, segmented quota gauges, rolling-digit counters | Low+ |
| **Detail drawer** | Slides in from the right: hero cover, palette tint, tracklist, history, actions, prev/next with slide transition | Low+ |
| **Command palette** (Ctrl+K) | Fuzzy jump to artists, releases, filters and commands, with thumbnails and match highlighting | Off+ |
| **Navigation** | Breadcrumbs, back/forward history, saved and smart views, A-Z / year scrub rail, colour filter | Off+ |
| **Facets** | Filter chips with live, tweening counts and a sliding segmented indicator | Off+ |
| **Dock and toasts** | Eased progress bars with hatch shimmer, speed sparkline, drawn-on check on completion, sliding toasts | Low+ |
| **Themes** | Carbon, Paper, Terminal, Vapor, plus "adaptive accent" from the hovered cover; 300 ms colour cross-fade | Low+ |
| **Atmosphere** | Pixel-block ambient glow, static grain and vignette, optional scanlines (shader layer, always optional) | Full |
| *Stretch* | Audio preview of a track from the drawer | n/a |

## 2. Workspace and install strategy

```text
bcvault/                       (workspace root)
├── Cargo.toml                 [workspace] resolver = "3", members = ["crates/*"], [workspace.dependencies], [workspace.lints]
├── scripts/check.sh           fmt, clippy -D warnings, test, grep guards (--workspace)
├── crates/
│   ├── bcvault-core/          lib: types, error, paths, db/, bandcamp/, worker/, migrations/, tests/, fixtures
│   ├── bcvault/               bin: classic UI (ui/, app.rs, main.rs, theme test)
│   └── bcvault-fx/            bin: new UI (§5 onward)
│       ├── assets/            fonts/ (+ licences), shaders/*.glsl, icons (if drawn)
│       ├── migrations/        0001_fx_init.sql ...
│       └── src/ main.rs app.rs actions.rs state.rs nav.rs
│              motion/ theme/ paint/ db/ services/ views/ overlays/ shader/ debug.rs
```

**F0 move map.** `types`, `error`, `paths`, `db`, `bandcamp`, `worker` and `migrations/` go to `bcvault-core`; `ui/`, `app.rs`, `main.rs` and the `theme_is_zero_radius` test go to `bcvault`; integration tests and `tests/fixtures` go with the core. Only `use` paths and `include_str!` paths may change.

**Core API surface the FX app needs (additive only):**

- `spawn_worker(rt, repaint_hook) -> WorkerHandle { cmd_tx, events, db: DbPool, gate: Arc<BandcampGate>, client: BandcampClient }`. The repaint hook is a `Fn() + Send + Sync` so core does not depend on egui.
- `BandcampClient::fetch_release_raw(url) -> Result<serde_json::Value>`: the `data-tralbum` JSON, fetched through the gate, so FX can read `trackinfo`, tags and description without a second parser.
- `BandcampGate::fetch_bytes(class, url) -> Result<Bytes>`: for cover downloads (`GateClass::Cdn`). FX never builds its own `reqwest` call.
- `Db::open_with(path, role, extra_pragmas, attach: &[(&Path, &str)])`, and the migration runner parameterised by `&[(u32, &str)]` and a `user_version` namespace, so the sidecar reuses the same code.
- `paths::acquire_instance_lock() -> Result<InstanceLock>` using `std::fs::File::try_lock` on `{data_dir}/bcvault.lock` (stable since Rust 1.89; bump the toolchain or use `fs4` if yours is older). **Both** apps call it, so two writers can never run at once and the classic and FX apps cannot be open together.

**Data sharing**

| Data | Location | Owner |
| --- | --- | --- |
| `bcvault.sqlite` (releases, artists, history, settings: out dir, format, quota) | Shared `ProjectDirs` data dir | Core worker is the only writer |
| `fx.sqlite` (cover palettes, tiny previews, lazy details, saved views, prefs, recents) | Same dir | FX only |
| Cover cache `covers/` | `ProjectDirs` cache dir, FX only | FX thumbnail service |
| eframe persistence (window geometry) | Separate app id `bcvault-fx` | FX |

Startup contract check: FX verifies via `PRAGMA table_info` that every column its queries use exists in the main DB. If a future classic migration breaks that, FX shows "schema mismatch, update bcvault-fx" instead of crashing. `--data-dir <path>` / `BCVAULT_DATA_DIR` allow a fully isolated FX profile for experiments.

**Packaging.** Separate binary `bcvault-fx`, its own app id, window title and icon, installed side by side with `bcvault`.

## 3. Visual language

### 3.1 Grid, type, icons

- 4 px base unit, 1 px borders, 2 px accent bars, gaps 8/12/16. Pixel-snap rects when **settled** (`ctx.round_to_pixel`); allow sub-pixel while moving.
- **Fonts, bundled (OFL, licences in `assets/licenses/`):** a squared UI family (for example Chakra Petch for display/headers, Inter or IBM Plex Sans for body) and JetBrains Mono for numerals and badges. Non-Latin artist names are common on Bandcamp, so at startup also load the first existing OS fallback font (Windows: `msyh`/`YuGothic`; macOS: PingFang; Linux: Noto CJK) with `std::fs::read` as lowest-priority fallbacks. No extra dependency.
- **Icons.** Prefer `egui-phosphor` (regular + bold variants; the 0.11 line dates from Nov 2025, so verify it resolves to your egui, otherwise skip it). Fallback: painter-drawn icons in `paint/icons.rs` (search, folder, download, check, cross, arrows, sort, view modes) from lines, rects and triangles only. Status glyphs and sort arrows are always painter-drawn, as in the classic app.

### 3.2 Themes

`FxTheme { bg0, bg1, bg2, border, border_hi, text, text_dim, accent, accent2, ok, warn, err }`, applied via the same `sharpen()` as the classic app plus the palette. Switching animates every colour through OKLab over 300 ms (re-applied to `Style` each frame only while the transition runs).

| Theme | bg0 / bg1 / bg2 | border / hi | text / dim | accent / accent2 |
| --- | --- | --- | --- | --- |
| **Carbon** (default) | `#0A0A0B` / `#121315` / `#1B1D20` | `#3A3D42` / `#8A8F98` | `#ECECEC` / `#9AA0A6` | `#00D1B2` / `#3DDC84` |
| **Paper** | `#F2F0EA` / `#E8E5DC` / `#DAD6CA` | `#1A1A1A` / `#000000` | `#111111` / `#555555` | `#E4002B` / `#0044FF` |
| **Terminal** | `#050805` / `#0A100A` / `#142014` | `#1F3B1F` / `#39FF14` | `#B6FFB0` / `#5C8A58` | `#39FF14` / `#FFD400` |
| **Vapor** | `#0D0A1A` / `#15102B` / `#1F1840` | `#3B2F73` / `#8C7BFF` | `#F1EAFF` / `#A79CD6` | `#FF4FD8` / `#3DE3FF` |

**Adaptive accent** (toggle): the accent follows the hovered or open cover's palette swatch, clamped in OKLab so text on it keeps contrast. A unit test checks every theme pair (text on bg0/bg1/bg2, accent on bg1) reaches contrast ratio 4.5.

### 3.3 Motion language

| Role | Spring preset | Feel | Used for |
| --- | --- | --- | --- |
| `SNAP` (k 900, c 60, critical) | about 120 ms, no overshoot | immediate feedback | hover, press, focus frame, chip indicator |
| `SMOOTH` (k 320, c 34) | about 250 ms | default transition | FLIP moves, drawer, view wipe, tweening numbers |
| `SOFT` (k 140, c 24) | about 450 ms | large, calm moves | theme/ambient glow, card-size change, zoom hero |
| `POP` (k 500, c 26) | about 10% overshoot | one-shot delight | selection tick, completion flash, toast in |

Rules: data never bounces (counts, bars use `SMOOTH`); only `POP` overshoots, and only on things the user just did. Springs retarget with velocity preserved, so interrupting is always fluid. Entrances stagger 12 ms per item, capped at 240 ms total. With `reduced_motion` every spring snaps to its target in one frame.

### 3.4 Squareness enforcement

- `fx/paint/` is the only module that issues shape calls; every helper takes no radius parameter and passes `CornerRadius::ZERO` with `StrokeKind::Inside`: `rect`, `rect_outline`, `hard_shadow(rect, offset, color)` (a solid offset rectangle, the only "shadow"), `glow_rect` (a vertex-coloured gradient `Mesh` with square edges), `hatch(rect, phase)` (45° stripes clipped to the rect), `diamond`, `busy_squares` (3×3 chase loader replacing `egui::Spinner`).
- Banned everywhere in `bcvault-fx`: circles, rounded shapes, `egui::Spinner`, radio-button visuals, anything with a non-zero corner radius. Round cursors and the OS window frame are out of scope.
- CI guard (any hit fails):
  `rg -n 'circle_|Shape::circle|rounding|corner_radius|CornerRadius::(same|from|new)|Spinner' crates/bcvault-fx/src | rg -v 'CornerRadius::ZERO'`
- Unit tests: `fx_theme_is_zero_radius` (both themes, all widget states, menus, windows, tooltips) and `no_round_primitives` (paints every `paint::*` helper into a `Shape` list and asserts there are no `CircleShape`, and every `RectShape` has zero radius).
- Visual audit saved as `docs/visual-audit-fx.png` (wall, table, drawer, palette, toasts, stats, scrollbars, tooltips, combos, modals, in all four themes).

## 4. Dependencies (`crates/bcvault-fx/Cargo.toml`)

```toml
[package]
name = "bcvault-fx"
version = "0.1.0"
edition = "2024"

[dependencies]
bcvault-core = { path = "../bcvault-core" }
eframe       = { workspace = true }             # glow backend, same features as classic
egui         = { workspace = true }
egui_extras  = { workspace = true }             # TableBuilder only; image loaders are NOT used here
image        = { workspace = true }             # jpeg + png decode, resize, JPEG encode of tiers
tokio        = { workspace = true }
tokio-util   = { workspace = true }
serde        = { workspace = true }
serde_json   = { workspace = true }
rusqlite     = { workspace = true }             # bundled; needs SQLITE_OPEN_URI for ATTACH
chrono       = { workspace = true }
directories  = { workspace = true }
open         = { workspace = true }
rfd          = { workspace = true }
tracing      = { workspace = true }
anyhow       = { workspace = true }
thiserror    = { workspace = true }
nucleo-matcher = "0.3"                          # fuzzy palette search; check the latest API (Matcher, Pattern)
egui-phosphor  = { version = "0.11", optional = true }   # verify it matches the workspace egui, else drop

[features]
default = ["phosphor-icons", "shaders"]
phosphor-icons = ["dep:egui-phosphor"]
shaders = []                                    # glow PaintCallback overlays; runtime-disabled if compile fails
audio-preview = []                              # stretch S1 only

[dev-dependencies]
wiremock = "0.6"
tempfile = "3"
pretty_assertions = "1"
# optional: egui_kittest (same minor as egui) for headless interaction tests
```

Notes: `eframe::egui_glow` and `eframe::glow` are re-exports, no direct dependency needed. The classic app's `http` image loader stays disabled; FX uploads textures itself, so every network byte passes the gate. Texture LRU, OKLab math, median-cut palette and the spring integrator are written in-tree (small, testable, no extra versions to track).

## 5. Architecture

```text
┌────────────────────── UI thread (FxApp::update) ──────────────────────┐
│ Motion::begin_frame → drain events (≤256 core + ≤256 fx, ≤4 uploads)  │
│ FxAction dispatcher ← keyboard / palette / clicks                      │
│ Layout: toolbar · sidebar · view (Wall|Table|Artists|Stats) · dock     │
│ Overlays: drawer · palette · toasts · modals · atmosphere (glow cb)    │
│ Motion::end_frame → request_repaint() only if something is moving      │
└───────────────┬───────────────────────────────────────▲────────────────┘
        FxCommand│ + core UiToWorkerCommand               │FxEvent + core WorkerToUiEvent
┌───────────────▼───────────────────────────────────────┴────────────────┐
│ tokio runtime (shared with core worker)                                 │
│  core worker (unchanged): sync, downloads, quota, gate                  │
│  FxServices: ThumbService · DetailService · QueryService · PrefsService │
│  FxDb: reader (sidecar + ATTACH main ro) · writer (sidecar only)        │
└─────────────────────────────────────────────────────────────────────────┘
```

- FX services receive `Arc<BandcampGate>` from the core handle. Cover and detail fetches obey the same pacing, cooldown and cancellation as everything else.
- Two receivers (core events, fx events) are drained in one `drain_events()`; both senders call the same repaint hook.
- Texture uploads (`ctx.load_texture`) happen on the UI thread but are budgeted: at most 4 per frame and about 2 ms; the remainder waits for the next frame with `request_repaint()`.

## 6. Motion engine (`motion/`)

```rust
#[derive(Clone, Copy)] pub struct SpringCfg { pub k: f32, pub c: f32, pub eps: f32 }
impl SpringCfg {
    pub const SNAP:   Self = Self { k: 900.0, c: 60.0, eps: 0.001 };
    pub const SMOOTH: Self = Self { k: 320.0, c: 34.0, eps: 0.001 };
    pub const SOFT:   Self = Self { k: 140.0, c: 24.0, eps: 0.001 };
    pub const POP:    Self = Self { k: 500.0, c: 26.0, eps: 0.001 };
}
#[derive(Clone, Copy)] pub struct Spring { pub x: f32, pub v: f32, pub target: f32 }
impl Spring {
    /// Fixed 1/240 s sub-steps => identical result at 30, 60 or 144 fps. Returns true while moving.
    pub fn step(&mut self, cfg: SpringCfg, dt: f32) -> bool {
        let mut left = dt.min(0.05);
        while left > 0.0 {
            let h = left.min(1.0 / 240.0);
            let a = -cfg.k * (self.x - self.target) - cfg.c * self.v;
            self.v += a * h; self.x += self.v * h; left -= h;
        }
        if (self.x - self.target).abs() < cfg.eps && self.v.abs() < cfg.eps * 10.0 {
            self.x = self.target; self.v = 0.0; false
        } else { true }
    }
}
pub struct Motion { /* HashMap<egui::Id, Entry>, frame counter, dt, active, reduced, level */ }
impl Motion {
    pub fn begin_frame(&mut self, ctx: &egui::Context);              // dt from stable_dt, GC entries untouched for 120 frames
    pub fn val(&mut self, id: impl std::hash::Hash, target: f32, cfg: SpringCfg) -> f32;           // first sight starts AT target
    pub fn val_from(&mut self, id: impl std::hash::Hash, from: f32, target: f32, cfg: SpringCfg) -> f32; // entrances
    pub fn vec2(&mut self, id: impl std::hash::Hash, target: egui::Vec2, cfg: SpringCfg) -> egui::Vec2;
    pub fn color(&mut self, id: impl std::hash::Hash, target: egui::Color32, cfg: SpringCfg) -> egui::Color32; // via OKLab
    pub fn end_frame(&self, ctx: &egui::Context);                    // request_repaint() iff active > 0
}
```

Helpers: `stagger(index, per_item_ms, cap_ms)`, `oklab` conversion and lerp, easing curves for the few time-based effects (shimmer phase, toast timer bar, wipe edge).

**Tests:** convergence for every preset; frame-rate independence (final value at dt 1/30, 1/60, 1/144 within 1e-3); `SNAP`/`SMOOTH` do not overshoot by more than 0.5%; retargeting keeps velocity continuous; `reduced_motion` returns the target immediately; `Motion` entries are garbage collected.

## 7. Sidecar DB and queries (`db/`)

### 7.1 `migrations/0001_fx_init.sql` (own `user_version`)

```sql
CREATE TABLE cover_meta (
    art_id         INTEGER PRIMARY KEY,
    tiny8          BLOB,                 -- 8x8 RGB = 192 bytes: instant blocky preview
    swatches       BLOB,                 -- 5 x RGB (15 bytes), by population
    accent_rgb     INTEGER,              -- 0xRRGGBB, most vivid swatch with usable contrast
    hue            INTEGER,              -- 0..359 (OKLCH), NULL if achromatic
    chroma         INTEGER, luma INTEGER,-- 0..100
    src_fetched_at INTEGER, failed_at INTEGER, fail_count INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX idx_cover_hue ON cover_meta(hue) WHERE hue IS NOT NULL;

CREATE TABLE release_detail (
    item_type TEXT NOT NULL, item_id INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL, tracks_json TEXT, about TEXT, credits TEXT, tags_json TEXT, duration_s INTEGER,
    PRIMARY KEY (item_type, item_id)
) STRICT;

CREATE TABLE saved_views (id INTEGER PRIMARY KEY, name TEXT NOT NULL, state_json TEXT NOT NULL, pos INTEGER NOT NULL) STRICT;
CREATE TABLE recents     (kind TEXT NOT NULL, ref TEXT NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY (kind, ref)) STRICT;
CREATE TABLE prefs       (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;   -- theme, effects, view, card size, ...
```

### 7.2 Reader with ATTACH

The FX reader opens `fx.sqlite` with `SQLITE_OPEN_URI` and runs `ATTACH DATABASE 'file:<main path>?mode=ro' AS m;` plus `PRAGMA query_only = ON`. Queries use explicit column lists on `m.releases` / `m.artists` / `m.download_history` (the contract check in §2 guards drift). `ORDER BY` and `WHERE` fragments remain `&'static str` per enum value with bound parameters, exactly as in the classic query builder.

```rust
pub struct FxQuery { pub base: ReleaseQuery, pub group: GroupBy, pub hue: Option<(u16, u16)>,
                     pub years: Option<(i32, i32)>, pub only_with_art: bool }
pub enum GroupBy { None, Year, Artist, Classification, Initial }
// SortKey gains: Hue, Added (first_seen_at DESC). Default wall sort: published_at DESC.
pub struct WallRow { pub rel: ReleaseRow, pub art_id: Option<i64>, pub hue: Option<u16>,
                     pub accent: Option<[u8; 3]>, pub added_at: Ts }
```

```sql
-- Wall / table rows (stable order, ends with item_type, item_id)
SELECT r.item_type, r.item_id, r.band_id, r.title, r.artist_name, r.url, r.art_id, r.classification,
       r.track_count, r.published_at, r.first_seen_at, r.downloaded, r.downloaded_format, r.local_path,
       r.has_remote_update, r.remote_update_reason, c.hue, c.accent_rgb
FROM m.releases r LEFT JOIN cover_meta c ON c.art_id = r.art_id
WHERE <static filter fragments> ORDER BY <static fragment>, r.item_type, r.item_id;

-- Facet counts, one pass, ignoring the facet's own dimension in the caller
SELECT classification, COUNT(*), SUM(downloaded = 0), SUM(has_remote_update) FROM m.releases WHERE <band+search> GROUP BY classification;

-- Stats (all local time so a São Paulo evening lands on the right day)
SELECT strftime('%Y-%m-%d', downloaded_at, 'unixepoch', 'localtime') d, COUNT(*), SUM(bytes_size)
  FROM m.download_history WHERE status = 'success' AND downloaded_at >= unixepoch('now','-370 days') GROUP BY d;
SELECT strftime('%Y', published_at, 'unixepoch') y, classification, COUNT(*) FROM m.releases WHERE published_at IS NOT NULL GROUP BY y, classification;
```

**Progressive load.** `QueryWall` streams `WallLoaded { request_id, chunk, last, rows }` in chunks of 500 so the first paint is immediate at 100k rows. The UI discards events whose `request_id` is stale, as in the classic app.

**Tests:** migrations idempotent; ATTACH is read-only (a write attempt errors); `EXPLAIN QUERY PLAN` for the default per-artist wall query still uses `idx_releases_band_pub` (no `SCAN r`); hue range filter uses `idx_cover_hue`; contract check detects a dropped column; 100k seeded rows query under 60 ms first chunk.

## 8. Thumbnail and palette pipeline (`services/thumbs.rs`)

**Tiers** (pixel edge of the square tier image; the tier is chosen from `display_size × pixels_per_point`, rounded up):

| Tier | Size | Used for | Source |
| --- | --- | --- | --- |
| `T64` | 64 px | table rows, sidebar mosaic, palette rows, queue | derived from the base fetch |
| `T192` | 192 px | wall cards (S/M), artist mosaics | derived from the base fetch |
| `T384` | 384 px | wall cards (L/XL) | derived from the base fetch |
| `Hero` | about 700 px | drawer hero | separate fetch, only when a drawer opens |

**Fetch once.** Per `art_id` the service downloads a single base image (`https://f4.bcbits.com/img/a{art_id}_{N}.jpg`; this plan's working guess is a size around 350 to 400 px, but **capture task F2.1 must confirm which `N` values exist and their pixel sizes** and record them in `docs/endpoints.md`; if `art_id` is missing, fall back to `releases.cover_url`). Through `GateClass::Cdn`. Tiers T64/T192/T384 are center-cropped to square and resized locally.

**Disk cache** (`covers/{art_id % 256:02x}/{art_id}.{tier}.jpg`, JPEG q85): derived tiers are small, so decode costs well under a millisecond each. The base file is kept so tiers can be regenerated without the network. Settings page: cache size readout and "purge covers" button. Covers are artists' artwork: local cache for personal use only, never redistributed.

**Pipeline**

1. UI sends `EnsureThumbs { generation, items: Vec<(art_id, Tier, priority)> }` for visible cards, plus a ±1 screen prefetch at lower priority, debounced by 80 ms. Each scroll settle or query change bumps `generation`; queued jobs of an older generation that have not started are dropped.
2. A priority queue (distance to viewport centre) feeds: disk hit → decode, or miss → gate fetch → derive all tiers → write disk → decode the requested tier. Decoding runs in `spawn_blocking` behind a `Semaphore(cpus - 1)`.
3. Result event: `ThumbReady { art_id, tier, image: Arc<egui::ColorImage> }`. The UI uploads within its per-frame budget into `TextureCache` and fades the image in (alpha spring over about 180 ms).
4. On first decode of a cover, `PaletteReady { art_id, tiny8, swatches, accent, hue, chroma, luma }` is computed from the 64 px image (median cut in OKLab) and persisted to `cover_meta`.
5. Failures: negative cache (`failed_at`, `fail_count`), retry after 24 h with backoff; 404 is final until `FullRefresh` changes the `art_id`.

**Instant placeholders.** All `tiny8` blobs (about 2 MB for 10k covers) are loaded into memory at startup. Before the real image arrives, a card draws its 8×8 mosaic as 64 solid square quads, then cross-fades to the sharp image: a blocky, on-brand blur-up. If even that is missing, draw a deterministic geometric tile from a hash of the item id.

**Memory.** `TextureCache` is an in-tree LRU keyed by `(art_id, tier)` with a byte budget (default 192 MB, settings-adjustable); entries in the current visible set are pinned. `Painter::image` with a shrunken `uv` rect gives the hover zoom and parallax without re-uploading anything.

**Tests:** wiremock-served JPEG fixtures (square, wide, tall, tiny, corrupt) produce exact tier dimensions with centre crop; second request hits disk (zero HTTP calls); cancelled generation never starts; LRU respects the byte budget and pins; palette of synthetic solid and two-colour images returns the expected hue; negative cache honoured; every network call went through the gate (assert on a counting gate).

## 9. Views and components

### 9.1 Shell and view switching
Top toolbar (breadcrumbs, search, view segmented control, card-size slider), left sidebar, central view, bottom dock; panels are added in the classic order (top, bottom, left, central). View switch uses a **hard wipe**: the incoming view is drawn clipped to a rect whose edge sweeps across with `SMOOTH`, the outgoing view stays beneath (frozen, read-only) and a 2 px accent line marks the edge. Total about 220 ms; with `Off` it cuts instantly.

### 9.2 Cover Wall (`views/wall.rs`)

- **Layout**: `WallLayout { cols, card, gap, label_h, groups: Vec<Group> }` computed from available width and card size (S 120, M 168, L 232, XL 320). Pure functions `pos(index)`, `visible_range(viewport_y)` and `index_at(point)` are unit-tested. Groups (Year, Artist, Class, Initial) add sticky headers with big typographic labels.
- **Virtualization**: `ScrollArea::show_viewport`; only cards in the viewport (plus a small margin) are built. Text galleys are cached per card width (`LayoutJob` with max 2 rows).
- **Card**: square cover, title and artist below, classification tag (square chip), downloaded tick, `+TRACKS`/`MOD` badge. States: *hover* lifts by 4 px with an accent hard shadow (`SNAP`), cover zooms to 1.04 and shifts up to 3% with the pointer (parallax, uv clamped inside 0..1), border takes the cover's accent; *selected* adds a 2 px accent frame and a drawn check (`POP`); *focused* is one shared frame that slides between cards (`SNAP`).
- **FLIP**: on filter, sort, group or size change, cards present in both the old and new visible sets animate from their old rect to the new one (`SMOOTH`, capped at 200 movers); new cards enter with a stagger (scale 0.92 to 1 from centre plus fade); cards that leave fade out for 120 ms as ghosts (capped at 40).
- **Interaction**: click selects, Ctrl-click toggles, Shift-click ranges, marquee drag selects by rectangle, double-click or Enter opens the drawer, arrow keys move focus (the view scrolls to keep it visible), Space toggles selection, `D` downloads, `O` opens in the browser.
- **Scrub rail** (right edge): ticks for years or initials; hover shows a large square label, drag scrubs, click jumps with `ui.scroll_to_rect` (egui's own scroll animation, tuned through `Style::scroll_animation`).

### 9.3 Table (`views/table.rs`)
Port the classic `TableBuilder` view (copy, do not depend on the classic crate) and add: a `T64` thumbnail column (24 px square), a single hover rect that springs between rows, a drawn sort arrow that rotates 180° (polygon vertices lerped), a 600 ms accent flash fading out on rows changed by `ReleaseUpdated`, and live search highlight (matched ranges drawn as accent underlay). Keep 4 ms frame budget at 100k rows.

### 9.4 Sidebar (`views/sidebar.rs`)
Virtualized artist list with a 2×2 `T64` mosaic per artist, badge numbers that tween, a sliding accent selection bar, a hard-edged light sweep along the bottom of a syncing artist, FLIP when the list re-sorts, and collapse to a 56 px icon rail (Ctrl+B, width spring). Above the list: **Smart views** (Recently added, Free and not downloaded, Updated, Pre-orders, Needs metadata) and **Saved views**.

### 9.5 Toolbar and facets (`views/toolbar.rs`)
Segmented filter control with a sliding indicator (`SNAP`) and live count per segment (tweening digits, from the facet query). Search field expands on focus, shows result count, debounced 120 ms through `request_id`. Colour filter: a row of 12 hue swatches that filter by `hue` range. Format, batch limit and output-folder controls as in the classic app, plus the `used/hard (30d)` quota readout as a segmented square gauge.

### 9.6 Detail drawer (`overlays/drawer.rs`)
`egui::Area` anchored right, 460 px, slides on x with `SMOOTH` over a scrim fading to 35%. Contents: hero cover (`Hero` tier, `T384` shown until it arrives), a mesh gradient header tinted by the cover palette, title, artist, classification chip, tracklist (staggered in), tags, description (collapsible), local path and format, history timeline of `download_history` rows as square dots, and actions (Download, Open Folder, Open in Browser, Redeem URL, Re-download, Update). Left/Right keys step through the current result set with a slide transition between releases. **Lazy detail fetch:** `DetailService::ensure(key)` fires only after the drawer has stayed on one release for 400 ms, uses `fetch_release_raw` through the gate, stores the result in `release_detail` and never fetches the same release twice. Rows classified `Unknown` or pre-orders still open; the tracklist shows "not loaded" with a Fetch button.

### 9.7 Artists view (`views/artists.rs`)
Wide banners: cover mosaic strip, name, odometer counts (total, eligible, downloaded), last sync time. Clicking a banner runs a shared-element transition: the banner rect springs to fill the view while the artist's wall fades in beneath, and Esc reverses it. This is the "dive into a crate" move.

### 9.8 Stats (`views/stats.rs`, all hand-painted)
Squarified treemap (library by classification, then artist; rects spring to new positions when data changes; algorithm of Bruls et al. with property tests: areas sum to the container, no overlaps, all inside), 53×7 download heat-calendar (accent intensity by count, tooltip with date, count and bytes), releases-per-year stacked bars, per-artist segmented quota gauges (soft threshold tick, hard end), and odometer counters (each digit is a clipped cell scrolling a vertical strip by spring).

### 9.9 Dock and toasts (`views/dock.rs`, `overlays/toasts.rs`)
Queue rows with `T64` thumbnail, label, state, a progress bar whose displayed fraction is a `SMOOTH` spring of the real fraction (jumps become glides) with moving 45° hatch shimmer while downloading, text `412.3 / 980.0 MB  3.20 MB/s`, and a 30 s speed sparkline of stepped square bars. Completion: bar flashes accent, a check mark draws on stroke by stroke (`POP`). Gate chip: `OK` / `PACING` / `COOLDOWN 00:41`, the cooldown shown as a draining bar. Toasts: bottom-right stack of at most 4, slide in (`POP`), square timer bar, errors persist until clicked.

### 9.10 Command palette (`overlays/palette.rs`)
Ctrl+K or Ctrl+P. Centred `Area`, 640 px wide, opens with scale 0.97 to 1 plus fade. Prefix scoping: `@` artists, `#` filters/classes, `>` commands; none searches everything. Matching with `nucleo-matcher` over an in-memory index (artists, release titles, commands) on a worker task with `request_id` and cancellation; show the top 50 with `T64` thumbnails, matched characters highlighted, key hints right-aligned, a selection rect that slides between rows, and **recents** when empty. Enter runs the item's `FxAction`; Tab cycles scope.

## 10. Navigation model

```rust
pub enum FxAction {
    SetView(ViewMode), OpenPalette, ClosePalette, FocusSearch,
    SetFilter(ReleaseFilter), SetSort(SortKey, SortDir), SetGroup(GroupBy), SetHue(Option<(u16, u16)>),
    SelectArtist(Option<i64>), OpenDrawer(ItemKey), CloseDrawer, DrawerStep(i32),
    ToggleSelect(ItemKey), SelectRange(ItemKey), ClearSelection,
    DownloadSelected, DownloadKey(ItemKey), OpenInBrowser(ItemKey), OpenFolder(ItemKey), RedeemDialog(ItemKey),
    SyncSelected(SyncMode), SyncAll(SyncMode), CardSize(i8), ToggleSidebar,
    SetTheme(ThemeId), SetEffects(EffectsLevel), NavBack, NavForward, SaveCurrentView, ApplySavedView(i64), ToggleDebug,
}
pub struct ViewState { pub view: ViewMode, pub query: FxQuery, pub scroll_y: f32, pub focus: Option<ItemKey> }
```

`nav.rs` keeps a bounded back/forward stack of `ViewState` (pushed when query, view or artist changes, not on scroll); Alt+Left/Right restore it including scroll position. Breadcrumbs (`All artists › Artist › Free › "search"`) are clickable, each segment animating its width. Esc closes the topmost layer: toast, palette, drawer, selection, in that order.

| Keys | Action |
| --- | --- |
| Ctrl+K / Ctrl+P | Command palette |
| `/` or Ctrl+F | Focus search |
| `1` `2` `3` `4` | Wall, Table, Artists, Stats |
| Arrows or `H J K L` | Move focus |
| Enter / Space | Open drawer / toggle select |
| `D` / `O` | Download focused / open in browser |
| `[` `]` | Previous / next artist |
| `+` `-` | Card size |
| Alt+Left / Alt+Right | Back / forward |
| Ctrl+B / F3 / F11 | Sidebar / debug overlay / fullscreen |

## 11. Atmosphere layer (`shader/`, feature `shaders`, level `Full`)

All effects are overlays or backdrops drawn through `egui_glow` paint callbacks, and the app is complete without them.

- **Grain + vignette overlay**: full-window quad on the foreground layer, 2 px block grain (square on purpose) and a soft vignette, dark only, grain alpha at most 6% and vignette alpha at most 30% (both user-adjustable down to zero). Static by default (drawn once per repaint, no continuous repaint). "Animated grain" is a separate off-by-default toggle capped at 30 fps and paused when the window is unfocused or minimised.
- **Ambient backdrop**: behind the wall header, a blocky "aurora" quantised to a 48×27 cell grid, tinted by the hovered/open cover's two swatches (colour springs `SOFT`), slow drift, 30 fps cap only while visible.
- **Scanlines** (Terminal theme option): a generated 1×3 px tile texture repeated at 4% alpha; no shader needed.
- **Compile safety**: build the program at startup with the shader version header from `egui_glow::ShaderVersion` (needs GL 3.3 core or GLES 3.0; on GLSL 100/ES2 disable the feature). Any compile or link failure logs once, disables the layer for the session and never blocks startup.
- **Fragment sketch** (overlay):
  ```glsl
  uniform vec2 u_res; uniform float u_time; uniform float u_strength; in vec2 v_uv; out vec4 o_color;
  float h(vec2 p) { return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453); }
  void main() {
      vec2 px = floor(gl_FragCoord.xy / 2.0);                       // 2 px square grain
      float g = abs(h(px + floor(u_time * 24.0)) - 0.5) * u_strength;
      float vig = smoothstep(0.55, 1.15, length(v_uv - 0.5) * 1.6) * 0.30;
      o_color = vec4(0.0, 0.0, 0.0, clamp(g + vig, 0.0, 0.06 + 0.30)); // premultiplied black
  }
  ```

## 12. FX messages (`services/messages.rs`)

```rust
pub enum FxCommand {
    QueryWall { request_id: u64, query: FxQuery }, QueryFacets { request_id: u64, query: FxQuery }, QueryStats { request_id: u64 },
    Search { request_id: u64, text: String, scope: SearchScope },
    EnsureThumbs { generation: u64, items: Vec<(i64, Tier, u32)> }, EnsureHero { art_id: i64 },
    EnsureDetail { key: ItemKey },
    SaveView { name: String, state: ViewState }, DeleteView(i64), SetPref { key: String, value: String }, TouchRecent { kind: String, r#ref: String },
    PurgeCovers, Shutdown,
}
pub enum FxEvent {
    Ready { prefs: Prefs, saved_views: Vec<SavedView>, tiny: Vec<(i64, [u8; 192])>, palettes: Vec<(i64, Palette)> },
    WallLoaded { request_id: u64, chunk: u32, last: bool, rows: Vec<WallRow> },
    FacetsLoaded { request_id: u64, facets: Facets }, StatsLoaded { request_id: u64, stats: Stats },
    SearchResults { request_id: u64, hits: Vec<SearchHit> },
    ThumbReady { art_id: i64, tier: Tier, image: std::sync::Arc<egui::ColorImage> },
    PaletteReady { art_id: i64, palette: Palette },
    DetailLoaded { key: ItemKey, detail: Box<ReleaseDetail> },
    ThumbFailed { art_id: i64 }, Notice(Toast),
}
```

## 13. Performance budgets and testing

| Budget | Target |
| --- | --- |
| Wall, 60 visible cards, release build | frame p95 under 6 ms; table under 4 ms at 100k rows |
| Texture uploads | at most 4 per frame and about 2 ms; texture memory under the 192 MB cap |
| Idle | after all springs settle, root `repaint_delay` is unlimited (animated grain off) |
| Startup | first paint under 400 ms with 100k rows (first chunk arrives async) |
| Memory with 100k rows | under 300 MB total, excluding the texture cache |
| Network | one CDN request per cover, one page request per opened drawer, all via the gate |

**F3 overlay** extends the classic one: frame time, active springs, texture bytes, thumb queue depth, uploads per frame, event backlog.

**Test plan**
- Pure unit tests: springs, OKLab, FLIP mapping, wall layout and hit-testing, treemap, palette extraction, LRU, nav stack, palette ranking, `FxAction` dispatch table.
- **Idle test**: run an `egui::Context` headlessly for 300 frames (animation kicked at frame 1) and assert the final `FullOutput` viewport `repaint_delay` is unlimited (check the field name in the pinned egui).
- Service tests with wiremock and `tempfile` (thumbs, details, sidecar migrations, ATTACH read-only).
- Optional `egui_kittest` (same minor as egui) for keyboard-driven interaction tests; snapshot tests only if software rendering works on your machine.
- `examples/fx_bench.rs`: opens the app on a 100k seeded DB, auto-scrolls the wall for 20 s and writes frame times to CSV; `--check` fails above budget.
- Contrast test for every theme (§3.2) and the squareness tests (§3.4).

## 14. Implementation phases and verification milestones

### Phase F0: Workspace split, pure refactor (0.5 to 1 day)
- 0.1 Root workspace with `[workspace.dependencies]` (versions from the classic `Cargo.toml`) and `[workspace.lints]`.
- 0.2 Move code per the §2 move map; fix `use` and `include_str!` paths only.
- 0.3 Add the §2 core API surface (additive) and the instance lock; call the lock from the classic `main`.
- 0.4 `scripts/check.sh` runs `--workspace`; the classic grep guard path is updated.

**Milestone**
- [ ] The workspace test count equals the old crate's test count and all pass; clippy and fmt clean.
- [ ] `cargo run -p bcvault`: add an artist, sync, download one release (5-minute manual smoke test) behaves as before.
- [ ] A second launched instance (classic or FX skeleton) exits with a clear "already running" message.
- [ ] `git diff` outside moves shows only imports and the additive API.

### Phase F1: Foundations (2 days)
- 1.1 `bcvault-fx` skeleton, `FxApp`, two-receiver event drain, `FxAction` dispatcher with an empty view set.
- 1.2 Sidecar DB, migrations, reader with ATTACH, contract check, `Prefs`.
- 1.3 `motion/` (springs, OKLab, `Motion`) with tests.
- 1.4 Theme system (4 themes, transitions), fonts and OS fallback, icons, `paint/` helpers, squareness tests and grep guard.
- 1.5 F3 overlay, `EffectsLevel` and `reduced_motion` plumbing.

**Milestone**
- [ ] Window opens with themed, square empty panels; theme switch cross-fades; all themes pass the contrast and zero-radius tests.
- [ ] Spring tests pass; idle test shows unlimited repaint delay after a settled animation.
- [ ] FX reads the real shared DB (artist count shown), never writes it, and the classic app still opens it afterwards.

### Phase F2: Thumbnail and palette pipeline (2 to 3 days)
- 2.1 Capture task (live, polite, one-off): record the valid `a{art_id}_{N}.jpg` sizes and a handful of sample covers as fixtures; write `docs/endpoints.md`.
- 2.2 Tier derivation, disk cache, `TextureCache`, priority queue and generations.
- 2.3 Palette extraction, `tiny8`, persistence, startup preload.
- 2.4 Placeholder rendering and fade-in widget `paint::cover(ui, key, rect, tier)`.

**Milestone**
- [ ] All §8 tests pass; a counting gate proves exactly one network request per new cover and zero on the second run.
- [ ] A scratch view with 5,000 seeded covers scrolls at fling speed with no stutter (uploads capped, queue drops stale work).
- [ ] Texture memory never exceeds the configured budget; palettes persist across restarts.

### Phase F3: Cover Wall (3 to 4 days)
- 3.1 `QueryWall` with chunked streaming and `FxQuery`; layout math.
- 3.2 Card rendering, hover/parallax/lift, selection, focus frame.
- 3.3 FLIP engine, entrances, ghosts; card-size change.
- 3.4 Groups, sticky headers, scrub rail; marquee selection; keyboard navigation.

**Milestone**
- [ ] 100k-row DB: first rows on screen under 400 ms, frame p95 under 6 ms while scrolling, memory under budget.
- [ ] Changing filter, sort and card size animates FLIP without a frame over 12 ms; interrupting an animation mid-flight is smooth (no jumps).
- [ ] Rapid typing in search applies only the newest result (request-id test); keyboard-only navigation works.
- [ ] With `EffectsLevel::Off` and `reduced_motion` the wall is fully functional with zero animation.

### Phase F4: Table, sidebar, toolbar, dock (3 days)
- 4.1 Table port with thumbnails and sliding highlight; 4.2 sidebar with mosaics, smart views and collapse; 4.3 toolbar facets and colour filter; 4.4 dock, progress springs, sparkline, toasts, gate chip.

**Milestone**
- [ ] Downloading 3 releases shows eased bars with hatch shimmer, correct MB and MB/s at about 10 Hz, completion check drawn, finished cards flip to downloaded in place without a reload.
- [ ] Facet counts tween and match the SQL; the sliding indicator follows keyboard and mouse.
- [ ] Table frame time under 4 ms at 100k rows.

### Phase F5: Drawer, navigation, palette (3 to 4 days)
- 5.1 Drawer with hero tier and slide transitions; 5.2 `DetailService` (lazy, deduplicated) and `release_detail`; 5.3 `nav.rs` history, breadcrumbs, saved views; 5.4 palette with scopes, recents and thumbnails.

**Milestone**
- [ ] Opening a drawer and pressing Right ten times quickly issues zero detail requests; resting 400 ms on a release issues one; reopening it issues none.
- [ ] Alt+Left/Right restore view, query and scroll position; saved views survive restart.
- [ ] Palette finds an artist in a 100k-row library in under 50 ms perceived, with correct match highlighting and Esc layering.

### Phase F6: Artists and Stats views (2 to 3 days)
- 6.1 Artists banners and shared-element dive; 6.2 treemap, heat-calendar, yearly bars, quota gauges, odometer.

**Milestone**
- [ ] Treemap property tests pass; stats numbers match direct SQL counts on a seeded DB.
- [ ] Banner-to-wall transition is reversible with Esc and stays square throughout.

### Phase F7: Atmosphere and polish (2 days)
- 7.1 Grain + vignette overlay, ambient backdrop, scanline tile, compile-failure fallback; 7.2 adaptive accent; 7.3 micro-polish pass (hover states, empty states, loading states built from `busy_squares`); 7.4 `fx_bench`.

**Milestone**
- [ ] Forcing a shader compile error disables the layer cleanly with one log line and the app keeps running.
- [ ] Idle CPU near zero at `Full` with animated grain off; unfocused window requests no repaints.
- [ ] `docs/visual-audit-fx.png` captured for all four themes; grep guard prints nothing.

### Phase F8: Hardening and packaging (1 to 2 days)
- Windows extended paths in the cover cache, case-insensitive collisions; cache purge and size readout; settings export; separate icon, app id and installer entry (`cargo-dist` or `cargo-bundle`); README section on running both apps; low-GPU guidance (`wgpu` fallback if glow misbehaves).

**Milestone**
- [ ] Killing the process mid-download and mid-thumbnail leaves both DBs consistent and no partial cache files remain after boot.
- [ ] Fresh install of FX on a machine that already has the classic DB opens it, and the classic app still starts afterwards.

### Stretch (only after F8)
- **S1 Audio preview**: play `mp3-128` preview URLs from `trackinfo` with `symphonia` + `cpal`, through the gate (`Download` class), with a square waveform strip and a 30 s buffer; off by default.
- **S2 wgpu backend** build for real blur/backdrop effects if you decide round-free frosted panels are worth the switch.

## 15. Risks and open questions

| Risk | Mitigation |
| --- | --- |
| egui moves fast; add-ons lag behind | One workspace pin, `cargo tree -d` check, in-tree helpers for LRU, OKLab, springs and charts, upgrade as its own chore. |
| Cover image sizes are a hypothesis | F2.1 capture decides the base size; everything else derives locally. |
| Shared DB while the classic schema evolves | Sidecar for FX data, column contract check at startup, instance lock, read-only ATTACH. |
| GPU or driver trouble with shaders | Feature flag, runtime compile fallback, `Off`/`Low` levels, optional `wgpu` build. |
| Animation cost on laptops | Springs stop at rest, 30 fps caps on ambient layers, pause when unfocused, `reduced_motion`. |
| Texture memory growth | Byte-budget LRU with pinning, small tiers only, hero loaded on demand. |
| Non-Latin names rendering as boxes | OS fallback fonts loaded at startup. |
| Extra traffic from thumbnails and details | Everything through the gate, one fetch per cover and per opened release, dwell delay, negative cache, resumable queue. |
| Scope creep | Effects levels and per-feature flags; phases F3 to F6 each ship usable on their own. |
| Artwork rights | Local personal cache, purge button, no redistribution or export of covers. |

**Defaults assumed; confirm or override:**

1. Shared main DB plus FX sidecar, one app open at a time.
2. Binary name `bcvault-fx`, installed beside `bcvault`.
3. Default theme Carbon, default effects `Full`, animated grain off.
4. Thumbnails on by default (the classic app keeps them off).
5. Cover wall as the default view, table one keypress away.
6. Fonts bundled (squared display face plus a neutral body face plus mono).

## 16. Kickoff prompt for the agent

> Read `Initial Plan.md` and `FX_PLAN.md`. Start with Phase F0 only: convert the repository into the workspace described in §2 as a pure refactor, keep every existing test green, and stop when the F0 milestone boxes are all checked. Record progress in `PROGRESS.md`. Do not start F1 until I confirm.

# Database Schema and Persistence Reference

`bcvault` uses SQLite in `WAL` (Write-Ahead Logging) mode with `STRICT` tables and foreign-key enforcement. Persistence is split across two SQLite database files in `data_dir`:

1. **`bcvault.db`** (Main Database, Schema Version **8**): Stores artists, releases, download history, settings, linked artist folders, ignored/rejected match pairs, and undoable library batch events. Shared between `bcvault-core`, `bcvault`, and `bcvault-fx`.
2. **`fx.sqlite`** (Sidecar Database, Schema Version **1**): Used exclusively by `bcvault-fx` (via SQLite `ATTACH DATABASE`) to store perceptual OKLab cover metadata, 8×8 RGB blur previews, lazily fetched release tracklists/liner notes, saved views, recent items, and UI preferences.

---

## 1. Connection Pragmas and Migration Runner

Every database connection opened by `DbPool` (`crates/bcvault-core/src/db/mod.rs`) configures:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
```

### Schema Versioning (`PRAGMA user_version`)
Migrations in `crates/bcvault-core/src/db/migrations.rs` run inside individual transactions at startup:
- Each migration `v` executes its SQL script and sets `PRAGMA user_version = v`.
- **Forward-Compatibility Guard (`DbError::TooNew`)**: If a database file has `user_version > MAX_VERSION` (currently `8`), `bcvault` refuses to open it rather than risking silent corruption from an older binary.
- **Sidecar Contract Check**: On startup, `bcvault-fx` verifies that all expected columns on `main.artists` and `main.releases` exist before attaching `fx.sqlite`.

---

## 2. Main Database Schema (`bcvault.db` — v8)

### 2.1 `artists`
Stores tracked Bandcamp artist and label pages.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `band_id` | `INTEGER` | `PRIMARY KEY` | Bandcamp's numeric band/label identifier. |
| `slug` | `TEXT` | `NOT NULL` | Subdomain slug or normalized domain identifier. |
| `name` | `TEXT` | `NOT NULL` | Artist or label name scraped from the Bandcamp page. |
| `custom_name` | `TEXT` | nullable | Optional user-defined display name override (added in `0003`). |
| `url` | `TEXT` | `NOT NULL`, `UNIQUE` | Canonical base URL without trailing slash (e.g., `https://slug.bandcamp.com`). |
| `cover_url` | `TEXT` | nullable | Artist avatar or header image URL. |
| `last_synced_at` | `INTEGER` | nullable | UTC Unix timestamp (seconds) of the last completed discography sync. |
| `created_at` | `INTEGER` | `NOT NULL DEFAULT (unixepoch())` | UTC Unix timestamp when the artist was added. |

**Indexes:**
- `idx_artists_url`: `UNIQUE` on `artists(url)`
- `idx_artists_name`: on `artists(name COLLATE NOCASE)`

---

### 2.2 `releases`
Stores every album and standalone track discovered across tracked artists.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `item_type` | `TEXT` | `NOT NULL CHECK IN ('album','track')` | Release kind (`album` or `track`). Part of composite PK. |
| `item_id` | `INTEGER` | `NOT NULL` | Bandcamp's numeric album/track ID. Part of composite PK. |
| `band_id` | `INTEGER` | `NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE` | Owning artist/label page ID. |
| `title` | `TEXT` | `NOT NULL` | Release title. |
| `artist_name` | `TEXT` | `NOT NULL` | Per-release artist name (differs from `artists.name` on compilation/label pages). |
| `url` | `TEXT` | `NOT NULL` | Absolute Bandcamp release URL. |
| `cover_url` | `TEXT` | nullable | Artwork URL. |
| `art_id` | `INTEGER` | nullable | Numeric Bandcamp artwork ID (`a{art_id:010}_{variant}.jpg`). |
| `classification` | `TEXT` | `NOT NULL DEFAULT 'unknown' CHECK IN ('unknown','free','nyp','email_required','paid','unavailable')` | Download eligibility classification (`unavailable` added in `0007`). |
| `pricing_json` | `TEXT` | nullable | Raw pricing signals JSON from `TralbumData` for offline auditing. |
| `is_preorder` | `INTEGER` | `NOT NULL DEFAULT 0 CHECK IN (0,1)` | `1` if the release is currently an unreleased pre-order. |
| `track_count` | `INTEGER` | nullable | Number of tracks on the release page (`0` triggers automatic `unavailable` classification). |
| `published_at` | `INTEGER` | nullable | UTC Unix timestamp of initial release publication. |
| `modified_at` | `INTEGER` | nullable | UTC Unix timestamp of last Bandcamp page modification. |
| `metadata_fetched_at` | `INTEGER` | nullable | UTC Unix timestamp when the individual release page was scraped. |
| `first_seen_at` | `INTEGER` | `NOT NULL DEFAULT (unixepoch())` | UTC Unix timestamp when `bcvault` first discovered the item. |
| `last_seen_at` | `INTEGER` | `NOT NULL DEFAULT (unixepoch())` | UTC Unix timestamp when the item was last seen on the discography grid. |
| `downloaded` | `INTEGER` | `NOT NULL DEFAULT 0 CHECK IN (0,1)` | `1` if downloaded or linked to a local library folder. |
| `downloaded_at` | `INTEGER` | nullable | UTC Unix timestamp when the release was downloaded or adopted. |
| `downloaded_format` | `TEXT` | nullable | Audio format key (`flac`, `mp3-320`, `mp3-v0`, `wav`, `aiff-lossless`, `alac`, `vorbis`, `aac-hi`). |
| `downloaded_track_count` | `INTEGER` | nullable | Snapshot of `track_count` at download time (used to detect `+TRACKS` updates). |
| `downloaded_modified_at` | `INTEGER` | nullable | Snapshot of `modified_at` at download time (used to detect `MOD` updates). |
| `local_path` | `TEXT` | nullable | Absolute path to the local album folder, `.zip`, or audio file. |
| `has_remote_update` | `INTEGER` | `NOT NULL DEFAULT 0 CHECK IN (0,1)` | `1` if remote changes were detected since download. |
| `remote_update_reason` | `TEXT` | `CHECK IN ('tracks_added','modified')` | Reason for `has_remote_update` (`+TRACKS` vs `MOD`). |
| `local_status` | `TEXT` | `CHECK IS NULL OR IN ('unverified','present','missing','archived')` | Local disk verification status (added in `0004`). |
| `local_checked_at` | `INTEGER` | nullable | UTC Unix timestamp of last local file verification. |
| `missing_ack_at` | `INTEGER` | nullable | UTC Unix timestamp when the user clicked "Skip for now" on a missing file prompt. |
| `origin` | `TEXT` | `CHECK IS NULL OR IN ('app','imported','redeem','manual')` | How the release was acquired (`app` download, `imported` folder link, `redeem` URL, or `manual` toggle). |

**Primary Key:** `PRIMARY KEY (item_type, item_id)`

**Indexes:**
- `idx_releases_band_pub`: `(band_id, published_at DESC, item_type, item_id)`
- `idx_releases_pub`: `(published_at DESC, item_type, item_id)`
- `idx_releases_class_dl`: `(classification, downloaded, band_id)`
- `idx_releases_todo`: `(band_id, published_at DESC) WHERE downloaded = 0 AND classification IN ('free','nyp')`
- `idx_releases_updates`: `(band_id) WHERE has_remote_update = 1`
- `idx_releases_unknown`: `(band_id) WHERE classification = 'unknown'`
- `idx_releases_downloaded`: `(band_id) WHERE downloaded = 1`
- `idx_releases_attention`: `(band_id) WHERE local_status = 'missing'`
- `idx_releases_archived`: `(band_id) WHERE local_status = 'archived'`
- `idx_releases_title`: `(title COLLATE NOCASE)`
- `idx_releases_artistname`: `(artist_name COLLATE NOCASE, title COLLATE NOCASE)`

---

### 2.3 `download_history`
Immutable ledger of download attempts and rolling 30-day quota usage.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | `INTEGER` | `PRIMARY KEY AUTOINCREMENT` | Unique history entry ID. |
| `item_type` | `TEXT` | `NOT NULL CHECK IN ('album','track')` | Release kind. |
| `item_id` | `INTEGER` | `NOT NULL` | Release ID. |
| `band_id` | `INTEGER` | `NOT NULL` | Artist ID (intentionally **not** a foreign key so 30-day quota math survives if an artist is removed and re-added). |
| `started_at` | `INTEGER` | `NOT NULL DEFAULT (unixepoch())` | UTC Unix timestamp when the job started. |
| `downloaded_at` | `INTEGER` | nullable | UTC Unix timestamp when the transfer completed (`status = 'success'`). |
| `format` | `TEXT` | `NOT NULL` | Audio format key. |
| `bytes_size` | `INTEGER` | nullable | Total bytes written to disk. |
| `status` | `TEXT` | `NOT NULL CHECK IN ('started','success','failed','cancelled')` | Outcome of the transfer. |
| `source` | `TEXT` | `NOT NULL DEFAULT 'api' CHECK IN ('api','redeem')` | Whether initiated via `buy_for_nothing` (`api`) or a redeemed link (`redeem`). |
| `counts_against_quota` | `INTEGER` | `NOT NULL DEFAULT 0 CHECK IN (0,1)` | `1` for `NYP` zero-dollar checkouts that consume the artist's monthly free download pool. |
| `error_message` | `TEXT` | nullable | Diagnostic message when `status = 'failed'`. |

**Indexes:**
- `idx_hist_quota`: `(band_id, started_at) WHERE counts_against_quota = 1`
- `idx_hist_item`: `(item_type, item_id, started_at DESC)`

---

### 2.4 `settings`
Single-row JSON configuration store (`key = 'app'`).

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `key` | `TEXT` | `PRIMARY KEY` | Setting key (`'app'`). |
| `value` | `TEXT` | `NOT NULL` | Serialized `Settings` JSON object (uses `#[serde(default)]` for backwards compatibility). |

---

### 2.5 `artist_folders`
Stores N:1 local music folder links bound to tracked artists (generalized in `0006`).

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | `INTEGER` | `PRIMARY KEY AUTOINCREMENT` | Unique folder link ID. |
| `band_id` | `INTEGER` | `NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE` | Linked artist ID. |
| `path` | `TEXT` | `NOT NULL` | Canonical absolute filesystem path. |
| `path_key` | `TEXT` | `NOT NULL UNIQUE` | Case- and Unicode-folded path key enforcing one-artist-per-folder uniqueness. |
| `added_at` | `INTEGER` | `NOT NULL DEFAULT (unixepoch())` | UTC Unix timestamp when linked. |
| `last_scan_at` | `INTEGER` | nullable | UTC Unix timestamp of the most recent scan. |
| `local_only_count` | `INTEGER` | `NOT NULL DEFAULT 0` | Count of unignored local-only orphan folders inside this root (added in `0005`). |

**Indexes:**
- `idx_artist_folders_band`: on `artist_folders(band_id)`

---

### 2.6 `artist_folder_ignored`
Stores local-only candidate folders that the user chose to ignore in future scans.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `band_id` | `INTEGER` | `NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE` | Artist ID. |
| `path_key` | `TEXT` | `NOT NULL` | Normalized candidate path key. |

**Primary Key:** `PRIMARY KEY (band_id, path_key)`

---

### 2.7 `artist_folder_rejected`
Stores permanently rejected `(release, folder)` review pairings ("Wrong, don't ask again", added in `0008`).

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `band_id` | `INTEGER` | `NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE` | Artist ID. |
| `item_type` | `TEXT` | `NOT NULL CHECK IN ('album','track')` | Release kind. |
| `item_id` | `INTEGER` | `NOT NULL` | Release ID. |
| `path_key` | `TEXT` | `NOT NULL` | Normalized candidate folder path key. |
| `created_at` | `INTEGER` | `NOT NULL DEFAULT (unixepoch())` | UTC Unix timestamp when rejected. |

**Constraints & Indexes:**
- `PRIMARY KEY (band_id, item_type, item_id, path_key)`
- `FOREIGN KEY (item_type, item_id) REFERENCES releases(item_type, item_id) ON DELETE CASCADE`
- `idx_artist_folder_rejected_band`: on `artist_folder_rejected(band_id)`

---

### 2.8 `library_batches` and `library_events`
Transactional audit log backing non-destructive library operations, release timeline history, and one-click batch Undo.

#### `library_batches`
| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | `INTEGER` | `PRIMARY KEY AUTOINCREMENT` | Batch ID. |
| `ts` | `INTEGER` | `NOT NULL DEFAULT (unixepoch())` | UTC Unix timestamp of the batch. |
| `kind` | `TEXT` | `NOT NULL` | Batch origin (`adopt`, `verify`, `resolve`, `manual`, `sync`). |
| `summary_json` | `TEXT` | `NOT NULL` | Serialized `BatchSummary` counts (`adopted`, `relinked`, `archived`, `unmarked`, `restored`, `ignored`). |
| `undone_at` | `INTEGER` | nullable | UTC Unix timestamp if the batch was undone (`NULL` if active). |

#### `library_events`
| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | `INTEGER` | `PRIMARY KEY AUTOINCREMENT` | Event ID. |
| `batch_id` | `INTEGER` | `NOT NULL REFERENCES library_batches(id) ON DELETE CASCADE` | Parent batch ID. |
| `item_type` | `TEXT` | `NOT NULL` | Release kind (`album`, `track`, or `local` for folder-level ignores). |
| `item_id` | `INTEGER` | `NOT NULL` | Release ID (or `band_id` for `ignored` events). |
| `kind` | `TEXT` | `NOT NULL` | Event type (`adopted`, `attached`, `relinked`, `archived`, `unmarked`, `restored`, `ignored`, `rejected`). |
| `old_json` | `TEXT` | nullable | Serialized `ReleaseSnapshot` before the change (restored on `undo_batch`). |
| `new_json` | `TEXT` | nullable | Serialized `ReleaseSnapshot` (or `path_key` for `ignored`/`rejected`) after the change. |

---

## 3. Sidecar Database Schema (`fx.sqlite` — v1)

`bcvault-fx` maintains `fx.sqlite` (`crates/bcvault-fx/migrations/0001_fx_init.sql`) for visual and interactive metadata:

### 3.1 `cover_meta`
| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `art_id` | `INTEGER` | `PRIMARY KEY` | Bandcamp artwork ID. |
| `tiny8` | `BLOB` | nullable | `8×8` RGB raw pixel buffer (`192` bytes) for instant blur placeholders. |
| `swatches` | `BLOB` | nullable | `5 × RGB` (`15` bytes) dominant color palette ordered by population. |
| `accent_rgb` | `INTEGER` | nullable | `0xRRGGBB` most vivid swatch passing contrast checks. |
| `hue` | `INTEGER` | nullable | Perceptual OKLCH hue angle (`0..=359`, `NULL` if achromatic). Indexed by `idx_cover_hue`. |
| `chroma` | `INTEGER` | nullable | OKLCH chroma (`0..=100`). |
| `luma` | `INTEGER` | nullable | OKLCH lightness (`0..=100`). |
| `src_fetched_at` | `INTEGER` | nullable | UTC Unix timestamp when the source cover was fetched. |
| `failed_at` | `INTEGER` | nullable | UTC Unix timestamp of last fetch failure (negative cache for 24h). |
| `fail_count` | `INTEGER` | `NOT NULL DEFAULT 0` | Consecutive download failures. |

### 3.2 `release_detail`
| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `item_type` | `TEXT` | `NOT NULL` | `album` or `track` (composite PK with `item_id`). |
| `item_id` | `INTEGER` | `NOT NULL` | Release ID. |
| `fetched_at` | `INTEGER` | `NOT NULL` | UTC Unix timestamp when scraped. |
| `tracks_json` | `TEXT` | nullable | JSON array of track rows (`num`, `title`, `duration_s`, `stream_url`). |
| `about` | `TEXT` | nullable | Release description / liner notes. |
| `credits` | `TEXT` | nullable | Release credits text. |
| `tags_json` | `TEXT` | nullable | JSON array of Bandcamp genre/location tags. |
| `duration_s` | `INTEGER` | nullable | Total release duration in seconds. |

### 3.3 `saved_views`, `recents`, and `prefs`
- **`saved_views`**: User-saved filter/sort presets (`id`, `name`, `state_json`, `pos`).
- **`recents`**: Recently visited artists and releases (`kind`, `ref`, `ts`) for the Command Palette.
- **`prefs`**: Key-value JSON preferences specific to `bcvault-fx` (`theme`, `effects_level`, `card_size`, `sidebar_open`).

---

## 4. Migration History (`crates/bcvault-core/migrations/`)

| Version | File | Summary |
| :--- | :--- | :--- |
| **1** | `0001_init.sql` | Initial `artists`, `releases`, `download_history`, and `settings` tables. |
| **2** | `0002_indexes.sql` | B-tree and partial indexes for release queries, sorting, and rolling quota lookups. |
| **3** | `0003_custom_artist_name.sql` | Adds `artists.custom_name` for user-defined artist display names. |
| **4** | `0004_library.sql` | Adds `local_status`, `local_checked_at`, `missing_ack_at`, `origin` on `releases`; creates `artist_folders`, `artist_folder_ignored`, `library_batches`, and `library_events`. |
| **5** | `0005_artist_folder_local_only.sql` | Adds `artist_folders.local_only_count` for sidebar local-only orphan tracking. |
| **6** | `0006_artist_folders_multi.sql` | Generalizes `artist_folders` from `1:1` to `N:1` (surrogate `id PRIMARY KEY AUTOINCREMENT`, multiple folders per artist). |
| **7** | `0007_unavailable.sql` | Extends `releases.classification` CHECK constraint with `'unavailable'` and backfills existing `0`-track releases. |
| **8** | `0008_rejected_matches.sql` | Creates `artist_folder_rejected` for persistent "Wrong, don't ask again" review pair rejection. |

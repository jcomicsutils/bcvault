# Plan: selectable release archive (text and covers, JPEG XL optional)

For the coding agent. Repo: bcvault (Rust workspace). Scope: `bcvault-core` (archive schema, capture during sync, cover fetching and encoding, jobs), the `bcvault-fx` and `bcvault` UIs (settings, drawer, cover fallback), and docs. Builds on the auto-sync plan.

## 0. Ground rules

- **Branch.** Create `feat/archive` from the repository default branch (`git symbolic-ref refs/remotes/origin/HEAD`). Never force-push.
- **Merge order.** This plan merges after `feat/auto-sync` and rebases onto it. Expect conflicts in `worker/sync.rs` (the new fetch call) and in the Settings modals; keep modal changes in new functions or files.
- **Migrations.** The schema is at version 8 (`0008_rejected_matches.sql`) and `AUTO_SYNC.md` / `ARTIST_GROUPS.md` take the next free numbers (`0009`/`0010`), so this plan uses `0010_archive.sql` (or `0011_archive.sql` — take the lowest free number at merge time per `ROADMAP.md` §5; never renumber a merged migration).
- Conventional Commits, one commit per step in section 5 (`Task: none`). Every commit compiles and passes tests. Run `scripts/check.sh` before each commit. Non-test code must not use `unwrap` or `expect`. The fx bans apply (`circle_`, `Shape::circle`, `rounding`, non-zero `corner_radius`, `Spinner`).
- **Politeness.** Page data is extracted from the HTML that sync already fetches, so archiving text adds zero page requests. Covers go through `GateClass::Cdn`. Never add a request path that bypasses the gate.
- **Archival rule.** Archived data is never deleted or shrunk automatically. Turning an option off, removing an artist, flagging a release removed, a failed fetch, or a page that now parses empty must not delete or blank anything. Only the explicit purge actions in 3.9 may delete archive data.
- **Untrusted input.** Pages and images come from the network. Enforce the limits in 3.5 and 3.11.
- **Licensing.** The JPEG XL encoder crate is AGPL-3.0 (or commercial). Stop for the user's confirmation before commit 5 is merged (see 3.7).
- Read first: `bandcamp/mod.rs` (`fetch_release`, `fetch_release_raw`, `fetch_bytes_gated`), `bandcamp/tralbum.rs`, `worker/sync.rs`, `worker/thumbs.rs`, `db/repo/releases.rs`, `db/mod.rs` and the migration runner, `paths.rs`, `types.rs` (`Settings`); in fx: `services/details.rs`, `services/thumbs.rs`, `db/mod.rs` (`release_detail`, `cover_meta`), `overlays/drawer.rs`, `overlays/modals.rs`; in the classic UI: the thumbnail path and `ui/modals.rs`; `docs/endpoints.md`. Line numbers in this plan come from a snapshot, so grep.

## 1. Current state (verified by reading the code)

- **The release page is fetched twice, by two unrelated paths.** Sync calls `fetch_release`, which parses classification, track count and dates and throws the HTML away. The fx drawer separately calls `fetch_release_raw` after the drawer has been open for 400 ms and parses tracks, about, credits and tags with `parse_raw_detail` in the fx crate.
- **Detail lives only in `fx.sqlite`** (`release_detail`), which is the UI sidecar. It exists only for releases whose drawer was opened, is written once and never refreshed, and the classic UI never has it. A release removed from Bandcamp before its drawer was opened has no tracklist, about text, credits or tags stored anywhere.
- `release_detail` also stores each track's `mp3-128` preview URL. That is a signed, expiring stream URL, so it is not worth archiving.
- **Covers exist only as cache files** under `cache_dir`. In fx: `covers/{art_id % 256:02x}/{art_id}.{tier}.jpg` for tiers T64, T192 and T384, all derived from one `_10` request. In the classic UI: `{kind}_{id}.jpg` from `ReleaseRow.cover_url`. `cover_meta` keeps only an 8x8 preview, swatches and accent. A cleared cache, or a dead URL after a page is removed, loses the cover.
- `docs/endpoints.md` lists the artwork sizes: `https://f4.bcbits.com/img/a{art_id:010}_{N}.jpg` with `_0` = original upload (JPEG or PNG), `_10` = 1200 px JPEG, `_16` = 700 px JPEG, `_2` = 350 px JPEG, `_7` = 150 px JPEG. I could not verify how the extension of `_0` is signaled; see 3.5.
- The workspace has `image` 0.25 with only the `jpeg` and `png` features, `rusqlite` (bundled), `BandcampClient::fetch_bytes_gated`, and no compression crate.

## 2. Goals and non-goals

Goals:

- The user chooses which text categories are archived: tracklist, tags (genres), about, credits, and optionally the raw page data.
- The user chooses cover size and cover format, including JPEG XL.
- Archived data lives in the databases, belongs to the vault rather than the UI, and survives releases being removed from Bandcamp.
- Both UIs fall back to archived covers, and the fx drawer shows archived text.

Non-goals:

- New network sources, audio, or per-artist archive settings.
- Exporting the whole archive to a folder (only Save cover as in this plan).
- Fetching a larger cover for an already archived one.
- Showing archived text in the classic UI (it has no detail pane).

## 3. Specification

### 3.1 Settings

`ArchiveSettings` inside `Settings`, with `#[serde(default)]` on everything:

| Field | Values | Default |
| --- | --- | --- |
| `tracklist`, `tags`, `about`, `credits` | bool | true |
| `raw_page` | bool (advanced) | false |
| `cover_size` | Off, S150, S350, S700, S1200, Original | S700 |
| `cover_format` | KeepOriginal, JxlLossless, JxlLossy | KeepOriginal |
| `jxl_distance` | float, clamped to 0.5 to 5.0, used only by JxlLossy | 1.0 |

Size mapping: S150 is `_7`, S350 is `_2`, S700 is `_16`, S1200 is `_10`, Original is `_0`.

### 3.2 Text archive (main database, migration `0010_archive.sql` or next free number)

```sql
CREATE TABLE release_archive (
    item_type TEXT NOT NULL,
    item_id INTEGER NOT NULL,
    captured_mask INTEGER NOT NULL DEFAULT 0,
    first_archived_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    tracks_json TEXT,
    tags_json TEXT,
    about TEXT,
    credits TEXT,
    duration_s INTEGER,
    raw_deflate BLOB,
    PRIMARY KEY (item_type, item_id)
) STRICT;
```

- **No foreign key to `releases`.** The archive survives removing an artist (which cascades to releases), the same way `download_history` does. If the artist is added again, the data reattaches through the item key.
- `captured_mask` bits: 1 tracklist, 2 tags, 4 about, 8 credits, 16 raw. A set bit means the category was parsed from a page; the value may then be NULL, meaning the page had none. This distinguishes not archived from archived and empty.
- `tracks_json` is an array of `{track_num, title, duration_s, artist}`. `artist` is stored only when the page provides one (compilations). Never store the per-track stream `file` URLs.
- `raw_deflate` holds the parsed tralbum JSON plus the DOM extras that `fetch_release_raw` builds, with every `file` object removed, compressed with `flate2` (pure Rust backend). It exists so fields not parsed today can be extracted later from a page that no longer exists.
- **Merge rule per category:** replace only when the new value is non-empty and different; never replace a non-empty value with empty or missing; set the mask bit whenever the category was parsed; bump `updated_at` only when something changed.

### 3.3 One fetch, two uses

- Add `BandcampClient::fetch_release_full(url, band_id) -> Result<ReleasePageFull, BcError>` returning the existing `ParsedReleasePage` plus an `ArchiveCapture` (the raw JSON with DOM extras), both parsed from a single HTML response. Keep `fetch_release` and `fetch_release_raw` as thin wrappers.
- Move the extraction logic of `parse_raw_detail` into core (`archive::extract`). The fx function becomes a thin call into core so nothing breaks.
- Sync uses `fetch_release_full` and calls `archive::capture(conn, key, &settings, &capture)` inside the same writer closure that calls `update_metadata`, so metadata and archive commit together. A failed fetch writes nothing.
- The fx `DetailService` reads the core archive first. If a wanted category is missing it calls `fetch_release_full`, stores the enabled categories, and keeps disabled ones in memory for the session only. A release whose enabled categories are already archived is never fetched again.

### 3.4 Legacy import

On the first launch after upgrade, fx copies every `release_detail` row into `release_archive` (all four categories, mask set accordingly) through the main database writer. It is idempotent and recorded under the settings key `archive_legacy_import_v1`. The old fx table stays in place and is never dropped.

### 3.5 Cover storage and fetching

Covers go in a separate SQLite file, `archive.sqlite` in the data directory (`AppPaths.archive_db_path`), with its own small migration set and the usual writer and reader pool. Reasons: covers are the only bulky data, so the main database stays small and quick to back up, and text stays in the main database where joins and search work.

```sql
CREATE TABLE cover (
    art_id INTEGER NOT NULL,
    size TEXT NOT NULL CHECK (size IN ('150','350','700','1200','orig')),
    format TEXT NOT NULL CHECK (format IN ('jpeg','png','jxl_jpeg','jxl_lossless','jxl_lossy')),
    width INTEGER NOT NULL, height INTEGER NOT NULL,
    source_format TEXT NOT NULL, source_bytes INTEGER NOT NULL,
    distance REAL,
    data BLOB NOT NULL,
    stored_at INTEGER NOT NULL,
    PRIMARY KEY (art_id, size)
) STRICT;
CREATE TABLE cover_fail (
    art_id INTEGER NOT NULL, size TEXT NOT NULL,
    failed_at INTEGER NOT NULL, fail_count INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (art_id, size)
) STRICT;
```

Keyed by `art_id` (an album and its tracks share one). No foreign keys.

Fetching:

- New `bandcamp::images::url(art_id, size)` builds `https://f4.bcbits.com/img/a{art_id:010}_{N}.jpg`.
- For Original, request `.jpg` first and `.png` on a 404, and remember the working extension in memory per `art_id`. Decide the real type from the magic bytes, never from the extension or content type. This is unverified: test it against a mock server now and against a live request before release; if neither extension works, record a failure and tell the user.
- Use `fetch_bytes_gated(GateClass::Cdn, url)` for pacing. Accept at most 40 MiB, read the image header and refuse more than 100 megapixels before decoding, decode on a blocking thread inside `catch_unwind`, and treat corrupt images as a failure, never a panic.
- A 404 or other failure writes `cover_fail`; do not retry the same `(art_id, size)` for 24 hours.
- When the configured size is S1200, optionally hand the fetched bytes to the existing fx tier derivation (T64, T192, T384) so one request serves both. Skip this if it is too invasive and note it.

### 3.6 Cover formats

| Format | JPEG source | PNG source |
| --- | --- | --- |
| KeepOriginal | Store the fetched bytes unchanged (`jpeg`) | Store unchanged (`png`) |
| JxlLossless | Reversible JPEG transcode (`jxl_jpeg`): the original JPEG can be rebuilt byte for byte. Verify by rebuilding and comparing before storing | Modular lossless (`jxl_lossless`). Verify decoded pixels equal the source |
| JxlLossy | Decode, encode at `jxl_distance` (`jxl_lossy`) | Same; use RGBA only if the PNG has alpha |

Rules that apply to every format:

- **Never apply pixel-lossless encoding to a JPEG source.** The decoded JPEG artifacts compress badly, so the result is usually bigger than the JPEG itself. Lossless for JPEG sources always means the reversible transcode.
- Store the encoded form only if it is strictly smaller than the original bytes; otherwise store the original with its true format.
- Any encode, verify or decode failure, or a panic, stores the original. A cover is never lost to a conversion problem.
- JxlLossy is irreversible. For JxlLossy, verify that the decoded dimensions match and that PSNR against the source is at least 30 dB. A source with an embedded ICC profile is kept as the original, because the lossy path would drop the profile.
- Quality presets shown in the UI (`distance`): Visually lossless 1.0 (default), Balanced 2.0, Small 3.0, plus a custom slider from 0.5 to 5.0. The encoder crate documents distance 1.0 as visually lossless.
- `archive::cover::decode(&StoredCover) -> image::RgbaImage` handles jpeg and png through `image` and the JXL formats through `jxl-oxide`. `reconstruct_original(&StoredCover) -> Option<Vec<u8>>` returns the original JPEG for `jxl_jpeg`, the PNG for `png` and `jxl_lossless`, and the bytes for `jpeg`.

### 3.7 Dependencies and licensing

- `jxl-encoder` (pure Rust encoder, lossy and lossless, with the `jpeg-reencoding` feature; young 0.x crate; dual-licensed AGPL-3.0 or commercial) behind a core cargo feature `jxl-encode`, on by default.
- `jxl-oxide` (pure Rust decoder, MIT or Apache-2.0, with the `image` feature) always on, so anything already stored can be displayed.
- `flate2` for the raw page data.
- bcvault is GPL-3.0-or-later. GPLv3 section 13 allows combining with AGPLv3 code, but the AGPL obligations then apply to that part. Record the dependency and its terms in the README license notes, check what CI runs for licenses, and confirm with the user before merging. This is not legal advice.
- Alternatives if the user declines: `jpegxl-rs` (GPL-3.0-or-later wrapper over the BSD-licensed libjxl; needs a vendored C++ build, and its minimum Rust version may exceed 1.89) or lossless-only formats.
- Check that the chosen crate versions compile on the project MSRV (1.89). Without the `jxl-encode` feature the UI hides the JXL formats and stored JXL covers still display.

### 3.8 Jobs (core worker)

New commands `ArchiveBackfill { text, covers, scope }`, `ArchiveConvert`, and cancel, with progress and summary events. All are cancelable and resumable, because each run recomputes what is missing.

- **Backfill text:** releases that lack a captured bit for any enabled category, excluding releases flagged `removed` by the auto-sync plan (their pages are gone). Fetches pages through `fetch_release_full`, gate-paced, newest first. The UI shows the count first and asks for confirmation above 100 pages.
- **Backfill covers:** releases with an `art_id` and no stored cover at the configured size, skipping anything inside the `cover_fail` window. At most two blocking encode workers.
- **Convert:** local only. For stored `jpeg`, `png`, `jxl_jpeg` and `jxl_lossless` covers, convert to the configured format. Never touch `jxl_lossy` rows, replace a row only if the result is smaller, one transaction per row.
- Changing settings affects future captures only. Nothing is deleted when an option is turned off.

### 3.9 Purge (the only deletions)

Two explicit actions behind a confirmation dialog that states what will be removed: Purge covers (all rows in `cover`) and Purge orphaned data (archive rows whose release no longer exists). No automatic path calls them, and there is no purge for archived text.

### 3.10 Display and fallback

- **fx drawer:** shows the archived tracklist, tags, about and credits and an Archived on line. Releases flagged removed or unlisted show their archived content, never a blank drawer.
- **Cover fallback in both UIs:** when the CDN fetch for a cover fails, try `archive.sqlite` before showing a placeholder, then feed the decoded image into the existing tier derivation (fx) or the cache file writer (classic). Preferring the archive over the network is a later optimization.
- **Save cover as** in the fx drawer writes `reconstruct_original` (an exact JPEG or PNG when available), or the stored `.jxl` for lossy covers.

### 3.11 Settings UI (both UIs)

An Archive section:

- Checkboxes: Tracklist, Tags (genres), About, Credits, and Raw page data (advanced).
- Cover size dropdown: Off, 150, 350, 700, 1200, Original.
- Format radio: Keep original, JPEG XL lossless, JPEG XL lossy. For lossy: the presets and the custom slider.
- Plain help text: Bandcamp covers are JPEGs. Lossless is exact, reversible and usually somewhat smaller. Lossy is smaller still but cannot be undone. Most other programs cannot open JXL files, so use Save cover as.
- A status line: releases with archived text, covers stored, total size in MB, and a projected total once at least 20 covers are stored (average size times releases that have an `art_id`).
- Buttons: Backfill text (with its page count), Fetch missing covers, Convert stored covers, Cancel, Purge covers, Purge orphaned data. State that turning options off never deletes data.

### 3.12 Genre artist grouping integration (`bcvault-core::groups`)

Once `release_archive.tags_json` is in place, add `BuiltinGrouping::ByGenre` to `bcvault-core::groups` (and expose it in the sidebar Grouping switcher in both `bcvault-fx` and `bcvault`):

- For each artist, aggregate all tags from `release_archive.tags_json` joined with `releases` on `(item_type, item_id)` for `band_id`.
- Bucket each artist by its most frequent tag (normalized/title-cased, tie-broken alphabetically), falling back to `"Untagged"` when an artist has no archived tags yet.
- Sort genre buckets alphabetically, with `"Untagged"` last.
- Keep `ByGenre` strictly non-destructive (computed on the fly, never modifying any custom Group Set).

## 4. Tests

| # | Area | Scenario | Expected |
| --- | --- | --- | --- |
| 1 | text | Parse fixtures with each category on and off | Only enabled categories stored; mask bits correct; archived-empty is NULL with the bit set |
| 2 | text | Merge rule: empty or missing over non-empty; changed non-empty value | Old kept; changed value updates and `updated_at` bumps |
| 3 | text | Stream URLs | Neither `tracks_json` nor the raw data contains any `file` or `mp3-128` entry |
| 4 | requests | Sync one release with the archive on; then open its drawer | Exactly one page request in total |
| 5 | survive | Delete an artist; add it again | Archive rows remain; the drawer shows them without a fetch |
| 6 | legacy | Import from `release_detail` twice | Idempotent; old table untouched |
| 7 | removed | Drawer for a removed release; refetch fails | Archived content shown; nothing overwritten |
| 8 | covers | Size to URL mapping; Original tries `.jpg` then `.png`; type decided by magic bytes | Correct |
| 9 | covers | 404; oversized body; over 100 megapixels; corrupt image | Failure recorded, 24-hour retry window, no panic |
| 10 | formats | KeepOriginal | Byte-identical |
| 11 | formats | JxlLossless on a JPEG fixture | `jxl_jpeg`; rebuilt bytes equal the source; falls back if larger |
| 12 | formats | JxlLossless on a PNG fixture | `jxl_lossless`; decoded pixels equal the source |
| 13 | formats | JxlLossy at 1.0, 2.0, 3.0 | Decodes, PSNR at least 30 dB, smaller than source (else original stored) |
| 14 | formats | Source with an ICC profile and JxlLossy; forced encoder error; forced panic | Original stored in each case |
| 15 | convert | Mixed stored formats | `jxl_lossy` untouched; replace only when smaller; resumable |
| 16 | backfill | Missing text and covers; removed releases; cancel and resume | Only missing items; removed skipped; resumes |
| 17 | fallback | CDN 404 with an archived cover | Both UIs produce a thumbnail from the archive |
| 18 | purge | Purge actions | Only run when invoked; counts match |
| 19 | settings | Old settings JSON without `archive`; build without the `jxl-encode` feature | Defaults load; build passes; stored JXL still displays |
| 20 | ui | State tests for the Archive section in both UIs | Pass |

## 5. Commit sequence

Each step leaves the workspace compiling with tests green.

1. `feat(core)!: add archive settings, schema and archive database` adds `ArchiveSettings`, migration `0010` (or lowest free number, with `BREAKING CHANGE:` footer), `archive.sqlite`, its path, pool and migrations.
2. `feat(core): archive release details during the sync fetch` adds `fetch_release_full`, `archive::extract`, `archive::capture`, the merge rule and tests 1 to 4.
3. `refactor(fx): read release details from the core archive` changes `DetailService`, adds the legacy import, and handles removed releases (tests 5 to 7).
4. `feat(core): archive covers in their original format` covers URLs, fetching with limits, storage, the failure cache and the fallback in both UIs (tests 8, 9, 10, 17).
5. `feat(core): add JPEG XL cover formats` adds the dependencies and feature, encode and verify and fallback rules, decode and reconstruct, and the license notes (tests 11 to 14). Pause for the user's confirmation here.
6. `feat(worker): add archive backfill and convert jobs` (tests 15, 16).
7. `feat(fx): archive settings, status and save cover as`, including purge actions (tests 18, 20).
8. `feat(ui): archive settings in the classic UI`.
9. `feat(groups): add built-in ByGenre artist grouping from release_archive.tags_json`.
10. `docs(archive): document what is archived and the cover options` and update the README.

## 6. Definition of done

- `scripts/check.sh` passes on every commit.
- Syncing a release with archiving on makes one page request, proved by a request-count test.
- A removed release still shows its tracklist, about, credits, tags and cover.
- No code path other than the two purge actions deletes archive data, and the tests prove it.
- A build without the encoder feature compiles and still displays stored JXL covers.

## 7. Decisions already made (do not re-litigate; flag if impossible)

- Defaults: tracklist, tags, about and credits on; raw page data off; cover size 700 px; keep original format.
- Text in the main database, covers in `archive.sqlite`, no foreign keys.
- For JPEG sources, lossless means the reversible transcode, never pixel-lossless.
- Lossy is opt-in, irreversible, and defaults to distance 1.0.
- The encoder license needs the user's confirmation before merge.
- The old fx `release_detail` table is left in place.

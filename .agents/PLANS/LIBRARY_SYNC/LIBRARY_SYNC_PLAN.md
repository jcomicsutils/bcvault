# Artist Folder Links: tie one folder to one artist page, adopt what is in it, verify it on sync

*Addendum to `Initial Plan.md` (and compatible with `FX_PLAN.md`) · lives in `bcvault-core`, UI in both front-ends · prepared 6 Nov 2026, revised 7 Nov 2026 (scope narrowed to one folder per artist)*

> [!NOTE]
> Status 8 Nov 2026: **implemented** (L0 to L6 on `main`). Differences from this text: the migration is `0004_library.sql` (`0003` was taken by `custom_artist_name`) and `0005_artist_folder_local_only.sql` adds `artist_folders.local_only_count`; normalisation lives in `library/textmatch.rs` (not `normalize.rs`); §5 normalisation and §6 scoring were replaced by `MATCH-FIX.md` (exact-first matching, `library/folder_style.rs` templates, no NFKC noise stripping, no Jaro-Winkler). Treat the code and `MATCH-FIX.md` as authoritative where they differ.

## 0. How to use this plan

**Scope, in one paragraph.** The user picks **one folder** and tells the app **which artist page** it belongs to. That is the whole feature. The app reads that folder, matches the albums in it to that artist's releases, marks them downloaded, and from then on checks that those albums are still on disk. It never scans a library, never looks at any folder the user did not link, and never guesses the Bandcamp page: the page always comes from the user (an already-tracked artist, or a pasted URL).

**What this adds**

1. **Link a folder to an artist page.** User chooses a folder and the artist page (existing tracked artist, or a pasted Bandcamp URL that goes through the normal add-artist flow). One folder per artist, one artist per folder.
2. **Adopt what is in that folder.** Album folders (and zips) inside the linked folder are matched to that artist's releases only, so albums you already have get marked downloaded without re-downloading. Uncertain matches go to a short review list.
3. **Say what did not match, inside that one artist.** Releases with nothing in the folder ("on page only") and folders with no release ("local only").
4. **Verify on every sync.** When an artist is synced, its downloaded albums are checked on disk. A missing album triggers *was it deleted or moved?* (ask), or an automatic policy: **mark not downloaded** or **keep marked as downloaded** (your "I archived it, then deleted it" case).
5. **Relink moved albums.** A moved album is searched for inside the artist's linked folder; if it is elsewhere, you point to it or re-point the artist's folder.

**Explicitly not in this feature** (so nobody adds them back by accident)
- No library roots, no `~/Music` scan, no global "Reconcile library" action, no cross-artist index.
- No guessing the artist page from a folder name, tag or path. Tags are read only to match albums *within* an already-chosen artist.
- No "gone from Bandcamp page" detection (it was in the previous draft; it is a remote-side concern, not a folder concern).
- No Bandcamp requests of any kind caused by this feature.

**Execution protocol.** Rules 2 to 7 of `Initial Plan.md §0` still apply (tests first, `forbid(unsafe_code)`, no `unwrap`/`expect`, UI-thread purity, one `BandcampGate`, no `Html` across `.await`). Additional rules:

1. Everything here is **local-only I/O**: no new Bandcamp requests. An `Incremental` sync of an unchanged artist must still cost exactly one HTTP request (existing milestone stays green).
2. **Never destroy information silently.** Every automatic change is a *batch* with before/after values in `library_events`, shown in a summary toast, and undoable.
3. **Never delete or modify user files.** This feature reads the filesystem only (stat, directory listing, tag reads), and only inside linked folders and the app's own `out_dir`.
4. **Absence of evidence is not deletion.** An unmounted drive, a permission error or a mass disappearance must never turn into "missing" (§8.2).
5. **One folder, one artist, enforced.** A linked folder may not equal, contain or sit inside another artist's linked folder (§5.1).
6. Prerequisite: Phase F0 of `FX_PLAN.md` (workspace split) is done, so core code is shared by both apps. Run the core phases (L0 to L3) in any order relative to the FX phases; the UI phases L4 and L5 follow the front-ends.

**Workflow.** Follow `AGENTS.md` (standing rules, Conventional Commits, quality gate) and `ROADMAP.md` (order and migration numbering). Task IDs for this plan are `L<phase>.<n>[letter]`. The task list and subjects in `COMMIT_PLAN.md` were written for the previous, wider scope and must be regenerated from this revision (the phase names below are unchanged, their contents are smaller).

**Decisions made for you (veto any)**

| Decision | Why |
| --- | --- |
| `releases.downloaded` keeps its meaning: "I have or had this". A new `local_status` says whether files exist now | Your archive workflow: downloaded, archived, deleted, still marked. |
| A link is `artist ↔ folder`, stored in a new `artist_folders` table, not in `Settings` | It is per-artist data; it must survive settings resets and be queryable per artist. |
| Default `deleted_policy = KeepMarked` | That is your stated use. The other option is one dropdown away. |
| Default `on_missing = Ask`, with an **Auto** mode and an automatic relink attempt first | Prompts only when the app cannot work it out itself. |
| Matching uses release URL tag first, folder name second, fuzzy last, review for anything not certain | Wrong "downloaded" marks are worse than a short review list. |
| Linking runs one match immediately and shows the report. Later syncs of that artist re-read the folder and **auto-adopt only certain matches** (setting `rescan_linked_on_sync`, default on); uncertain ones just raise a `REVIEW n` chip | "Tracked" folder should pick up albums you add later without a dialog. Turn off to make it link-time only. |
| Schema change in the **main** DB (migration `0004_library.sql`) | The sync service in core needs it. This amends the FX plan's "main schema untouched" rule; see §4.4. |
| Adoption sets `downloaded_track_count = track_count` of the page | Avoids false `+TRACKS` flags on albums where you pruned tracks. |

## 1. Vocabulary and states

| Field | Values | Meaning |
| --- | --- | --- |
| `downloaded` | 0 / 1 | You have, or had, this release. Used by "Not Downloaded", sidebar counts, quota-free logic. |
| `local_status` (only when `downloaded = 1`, else `NULL`) | `unverified` | Marked downloaded but files never checked (imported rows, upgrades). |
| | `present` | Files verified at `local_path`. |
| | `missing` | Files not found and not yet resolved by you. |
| | `archived` | You confirmed they were deleted (or policy did), and you want it kept as "done". |
| `origin` | `app`, `imported`, `redeem`, `manual` | How the downloaded mark arose (`NULL` = `app`). |

Invariant (enforced in the repo layer and by a test): `downloaded = 0 ⇒ local_status IS NULL`.

Glyphs and chips (painter-drawn squares, theme colours): present = filled square with check; archived = hollow square with a horizontal bar plus `ARCHIVED` chip; missing = square with `!` in warning colour plus `MISSING`; imported = tiny `I` tag in the drawer, not the table.

## 2. Case matrix: what the app does (always inside one artist and its linked folder)

| # | Item in the linked folder | Release on the artist's page | DB row | Meaning | Action |
| --- | --- | --- | --- | --- | --- |
| 1 | found, confident match | yes | not downloaded | existing download | **Adopt** (auto if certain, else review) |
| 2 | found, low confidence / several candidates | yes | not downloaded | uncertain | **Review list** (pick or skip) |
| 3 | none | yes | not downloaded | normal | nothing; listed as *on page only* in the report |
| 4 | found at `local_path` | yes | downloaded | normal | `present` |
| 5 | **not at `local_path`**, found elsewhere in the linked folder | yes | downloaded | **moved** | **Relink** (silent if unique and confident, else ask) |
| 6 | **not found** | yes | downloaded | **deleted** or on an unmounted disk | **Ask**, or apply `on_missing` / `deleted_policy` (§8) |
| 7 | found | **no matching release** | none | **local only** (not yet synced, different edition, not this artist's) | *Local only* list: attach manually, ignore, or leave |
| 8 | found again | yes | archived | restored | propose *restore link* (status back to `present`) |

Releases of artists **without** a linked folder are only ever touched by row 4/6 logic using their existing `local_path` (albums this app downloaded into `out_dir`). Adoption (rows 1, 2, 7) never runs for them.

## 3. Settings, links and when policies fire

New `Settings` fields (all `#[serde(default)]` so existing stored settings keep loading). There are **no** library roots and no scan depth setting:

```rust
pub verify_on_sync: bool,            // true
pub rescan_linked_on_sync: bool,     // true: re-read each synced artist's linked folder, auto-adopt certain matches
pub on_missing: OnMissing,           // Ask | Auto            (default Ask)
pub deleted_policy: DeletedPolicy,   // KeepMarked | MarkNotDownloaded   (default KeepMarked)
pub auto_relink: bool,               // true
pub auto_match_min: f32,             // 0.92
pub review_match_min: f32,           // 0.75
```

| Trigger | Network | Local work | Policies applied |
| --- | --- | --- | --- |
| `LinkArtistFolder` (user links a folder) | none | validate, scan that folder, match, report (apply certain ones only if the user ticked it; default on) | none |
| `Sync` Incremental, CheckDownloaded, FullRefresh (selected artists or all) | as before | after the network phase, **limited to the synced artists**: if `rescan_linked_on_sync` and the artist has a link, rescan that one folder and auto-adopt certain matches; then, if `verify_on_sync`, verify downloaded rows | `auto_relink`, then `on_missing`, then `deleted_policy` |
| `MatchArtistFolder` (button) | none | rescan that artist's folder and show the full report | none until accepted |
| `VerifyFiles` (button) | none | verification only, for the selected artists or all | same, and it **re-prompts** items you earlier skipped |

## 4. Data model

### 4.1 `migrations/0004_library.sql` (planned as `0003`; that number was already taken)

```sql
ALTER TABLE releases ADD COLUMN local_status TEXT
    CHECK (local_status IS NULL OR local_status IN ('unverified','present','missing','archived'));
ALTER TABLE releases ADD COLUMN local_checked_at INTEGER;
ALTER TABLE releases ADD COLUMN missing_ack_at   INTEGER;     -- user chose "skip for now"
ALTER TABLE releases ADD COLUMN origin TEXT
    CHECK (origin IS NULL OR origin IN ('app','imported','redeem','manual'));
UPDATE releases SET local_status = 'unverified' WHERE downloaded = 1;

-- One folder per artist, one artist per folder.
CREATE TABLE artist_folders (
    band_id      INTEGER PRIMARY KEY REFERENCES bands(band_id) ON DELETE CASCADE,  -- use the real artists table/column from 0001
    path         TEXT NOT NULL,                -- canonical absolute path
    path_key     TEXT NOT NULL UNIQUE,         -- case/Unicode-folded key for comparisons
    added_at     INTEGER NOT NULL DEFAULT (unixepoch()),
    last_scan_at INTEGER
) STRICT;

-- "Local only" folders the user chose to hide, per artist.
CREATE TABLE artist_folder_ignored (
    band_id  INTEGER NOT NULL REFERENCES artist_folders(band_id) ON DELETE CASCADE,
    path_key TEXT NOT NULL,
    PRIMARY KEY (band_id, path_key)
) STRICT;

CREATE TABLE library_batches (
    id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL DEFAULT (unixepoch()),
    kind TEXT NOT NULL,                        -- adopt | verify | resolve | manual | sync
    summary_json TEXT NOT NULL, undone_at INTEGER
) STRICT;
CREATE TABLE library_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT, batch_id INTEGER NOT NULL REFERENCES library_batches(id) ON DELETE CASCADE,
    item_type TEXT NOT NULL, item_id INTEGER NOT NULL,
    kind TEXT NOT NULL,                        -- adopted | relinked | archived | unmarked | restored | ignored | linked | unlinked
    old_json TEXT, new_json TEXT
) STRICT;
CREATE INDEX idx_libev_item ON library_events(item_type, item_id);

CREATE INDEX idx_releases_attention ON releases(band_id) WHERE local_status = 'missing';
CREATE INDEX idx_releases_archived  ON releases(band_id) WHERE local_status = 'archived';
```

There is deliberately **no persistent index of local files**: an artist folder holds tens of albums, so it is scanned in memory each time it is needed.

### 4.2 Types (`types.rs` additions)

```rust
pub enum LocalStatus { Unverified, Present, Missing, Archived }
pub enum Origin { App, Imported, Redeem, Manual }
pub enum OnMissing { Ask, Auto }
pub enum DeletedPolicy { KeepMarked, MarkNotDownloaded }
pub struct ArtistFolder { pub band_id: i64, pub path: PathBuf, pub state: FolderState }   // FolderState: Ok | Missing | Unreadable
// ReleaseRow gains: local_status: Option<LocalStatus>, origin: Option<Origin>
// ReleaseFilter gains: Archived, MissingFiles.   Where fragments are fixed strings:
//   Archived: local_status = 'archived'   MissingFiles: local_status = 'missing'
// ArtistStats gains: archived, missing_files (tooltip only; `downloaded` still counts archived).
```

### 4.3 Repo functions (`db/repo/library.rs`)
`link_folder(band_id, path)` (enforces §5.1 inside one transaction), `unlink_folder(band_id)`, `get_folder(band_id)`, `set_local_status`, `adopt(key, path, format, origin, batch)`, `relink(key, path, batch)`, `archive(keys, batch)`, `unmark(keys, batch)`, `restore(key, path, batch)`, `ignore_local(band_id, path_key)`, `undo_batch(batch_id)`, `batch_summary`. Every mutating function takes a `batch_id` and writes `library_events` in the same transaction. Unlinking removes only the link row; downloaded marks and `local_path` values stay.

### 4.4 Compatibility with the FX plan and old binaries
FX's startup contract check must add the new columns and tables it uses. An already-installed older `bcvault` binary will refuse the migrated DB (`TooNew`), so rebuild and reinstall both apps from the same workspace revision when this lands. Existing `commit_download` sets `local_status = 'present'`, `origin = 'app'` (or `'redeem'` for the redeem path).

## 5. Linking a folder and scanning it (`library/link.rs`, `scan.rs`, `tags.rs`, `textmatch.rs`)

### 5.1 Linking rules
- The user supplies **both** inputs: a folder (picker) and an artist page. The page is either an already-tracked artist (dropdown) or a pasted Bandcamp URL, which goes through the existing add-artist resolve/sync path. If the artist has not finished its first sync there are no releases to match yet: the link is stored and the first match runs automatically after that sync.
- **The folder name, tags and path are never used to look up, suggest or pre-fill an artist.** No search box seeded from them, no "did you mean".
- Validation (`link_folder`): path exists, is a directory, is readable; canonicalised (`dunce::canonicalize` on Windows); `path_key` is unique; the folder may not equal, contain or sit inside another artist's linked folder; it may not equal or contain `out_dir`. One folder per artist: linking again replaces the old link after confirmation (old marks are untouched).
- Soft warning (does not block): if most direct children contain no audio but themselves contain album folders, the folder looks like it holds several artists; the dialog says "Link one artist's folder, not a library" and asks for confirmation.

### 5.2 What counts as an album candidate
Only inside the linked folder, looking at its **direct children** plus one level for disc subfolders:
- A **folder** that directly contains at least one audio file (`flac mp3 ogg opus m4a wav aif aiff`). Multi-disc layouts (`CD1/`, `Disc 2/`) are merged into their parent: if a child has no audio but all its children are named like discs, the child is the candidate. If the linked folder itself directly contains audio, it is treated as one album candidate (single-album artists).
- A **zip** file (the app's own `.zip` downloads). Matched by name only; optionally list entry names from the central directory (no extraction) to count audio entries.
- Nothing deeper is read. Folders like `Artist/2019/Album` are not discovered; if you use that layout, link the folder that directly holds the albums.

**Walk rules.** Do not follow symlinks (record and skip); skip hidden directories and `_to_delete`-style names; respect cancellation between directories; run in `spawn_blocking` (a few hundred directory reads at most).

**Tags** (`lofty`, check the current 0.2x API with `cargo add lofty`): read the *first* audio file of each candidate (plus one more if the first has no tags) for `ALBUM`, `DATE`/`YEAR`, any Bandcamp release URL in URL/comment tags (confirm on fixtures whether your downloads carry it), codec, bitrate and VBR flag. `ARTIST` is read for display only; the artist is already known. Reads are bounded (timeout per file, max file size), and failures leave the fields `NULL`.

**Normalisation** (implemented as `textmatch.rs`, later reduced by `MATCH-FIX.md`; shared; also used by the mail plan): Unicode NFKC, case fold, map `&` to `and`, strip diacritics for comparison only, drop punctuation, collapse whitespace; remove noise tokens such as `[FLAC]`, `[WEB]`, `(Free Download)`, `(FLAC)`, `V0`, `320`, and a trailing `(YYYY)`. Keep "deluxe/remaster/edition" words (they distinguish releases). Also exports `name_variants(artist, title)` = `{artist} - {title}`, `{title}`, `{artist}/{title}`.

**Format guess from tags** (`downloaded_format`, best effort): `flac`→Flac, `wav`→Wav, `aiff`→AiffLossless, `ogg`→Vorbis, MP4 ALAC→Alac, MP4 AAC→AacHi, MP3 CBR 310 to 320 kbps→Mp3_320, MP3 VBR average 220 to 260→Mp3V0, otherwise `NULL`.

**Folder probe.** The linked folder is `Ok`, `Missing` (path absent) or `Unreadable` (permission/IO error). For rows with no link, `out_dir` is probed. Only `Ok` can produce evidence of absence.

## 6. Matching (`library/matcher.rs`, pure functions)

Matching is **one artist's releases against one folder's candidates**. Both sides are small, so no blocking or indexing is needed.

```rust
pub enum Signal { UrlTag, AlbumTag, FolderName, TrackCount, Year }
pub struct Score { pub value: f32, pub signals: Vec<Signal> }
pub fn score(local: &LocalEntry, rel: &ReleaseInfo) -> Score;
```

| Signal | Effect |
| --- | --- |
| Tag URL equals the release URL (normalised host and path) | score 1.00, certain |
| `tag_album` ≈ `title` (Jaro-Winkler on normalised) | base = similarity |
| Folder or zip `name_key` equals a `name_variants` entry | base 0.95 (exact), else fuzzy similarity |
| `audio_count == track_count` (albums) | +0.05; mismatch by more than 2 tracks: −0.10 (bonus tracks tolerated) |
| `tag_year` equals release year | +0.02 |
| Version word in only one side (`deluxe`, `remastered`, `live`) | cap at 0.80 so it goes to review |

**Assignment.** Greedy one-to-one assignment by descending score. A pair is **auto** if `score ≥ auto_match_min` and it beats the runner-up for both sides by at least 0.05; **review** if `≥ review_match_min` or ambiguous; otherwise unmatched. One local folder matching several releases (or the reverse) goes to *conflicts*, never auto. Duplicate local copies (the same album in FLAC and MP3 folders) are a conflict with both paths so you choose which to link.

**Single tracks** (`item_type = track`): a candidate is a subfolder with exactly one audio file, or `{Artist} - {Title}/` produced by the app; same scoring with `audio_count == 1`.

**Tests** (fixture builder creates temp trees; tiny committed `silence.mp3` and `silence.flac` get tags written by `lofty` in test setup): exact name, renamed folder with correct album tag, URL tag only, zip-only, Deluxe vs standard, label page with `artist_name` per release, NFC vs NFD names, trailing-year noise, bracket noise, duplicates in two formats, orphan folder, multi-disc, single-album artist folder, symlink loop, and the linking rules of §5.1 (nested, containing, duplicate link, `out_dir` overlap, looks-like-a-library warning).

## 7. Match and adopt flow

`LinkArtistFolder` and `MatchArtistFolder { band_id, auto_apply_confident }`:

1. Scan the artist's linked folder (progress events).
2. Load that artist's releases, compute proposals with §6.
3. Build the report and send it; **apply nothing** unless the user accepted, except `auto` proposals when "apply confident matches automatically" is ticked (default on at link time and on sync rescans).

```rust
pub struct ReconcileReport {
    pub batch_id: u64, pub band_id: i64,
    pub matched: Vec<MatchProposal>,        // auto-confident
    pub review: Vec<ReviewItem>,            // pick one of several, or skip
    pub page_only: Vec<ItemKey>,            // on the artist's page, nothing in the folder
    pub local_only: Vec<LocalOrphan>,       // in the folder, no release
    pub conflicts: Vec<Conflict>,
    pub restorable: Vec<MatchProposal>,     // archived rows whose files reappeared
}
```

**Apply** (`ApplyReconcile { batch_id, decisions }`): one writer transaction per decision group: `downloaded = 1`, `downloaded_at = folder mtime`, `downloaded_format` guess, `local_path`, `local_status = 'present'`, `origin = 'imported'`, snapshot `downloaded_track_count = track_count`, `downloaded_modified_at = modified_at`, clear update flags; write events. Then `ReleaseUpdated` per row and `ArtistStats`. **Local-only** items offer *Attach to release…* (search box over that artist's releases), *Ignore* (row in `artist_folder_ignored`, never shown again), *Leave*. A toast offers **Undo** for the batch.

On later syncs (§3) the same scan runs without a dialog: certain matches are adopted as a `sync` batch (summary toast with Undo), everything else only updates a `REVIEW n` chip on the artist.

Eligibility note: adoption works for any classification (you may own a Paid release bought elsewhere); it never downloads anything.

## 8. Verify flow (missing, moved, deleted)

### 8.1 Per-release check (`library/verify.rs`)
For every `downloaded = 1` release in scope:
- Folder: exists, is a directory, and contains at least one audio file (depth 2). A folder holding only `cover.jpg` counts as **missing**.
- Zip: exists, is a file, size greater than 0.
- Result: `Present` (update `local_status = 'present'`, `local_checked_at`), `Missing`, or `Unknown` (folder probe not `Ok`; leave unchanged and warn "folder unavailable: <path>").
- Rows `unverified` with `local_path IS NULL` and `origin = 'imported'` are skipped (nothing to check); `archived` rows are re-checked only for *restore* detection (§7).

### 8.2 Safeguards
- **Unavailable folder.** If the artist's linked folder (or `out_dir` for unlinked rows) probes `Missing`/`Unreadable`, every row under it is `Unknown`, never `Missing`. An unmounted drive therefore cannot mark an artist's whole discography missing.
- **Mass-missing guard.** If more than 30% of the checked releases (at least 20) come back missing in one pass, the app assumes a disk problem: nothing is applied automatically, `on_missing` is treated as `Ask` for this pass, and the dialog opens with a banner "Many albums are missing: is a drive disconnected?".
- **Verification is read-only and cheap** (stat plus a small directory listing); run in `spawn_blocking`, cancellable, with progress events.

### 8.3 Resolution order for each missing album
1. **Auto-relink** (if `auto_relink` and the artist has a link): rescan the artist's linked folder, then look for a unique candidate with `score ≥ auto_match_min` that is not already linked to another release. If found: relink, `present`, event `relinked` (no prompt). Artists without a link skip this step.
2. **Policy**
   - `on_missing = Ask` → collect into a `MissingFound` batch and show the dialog (below). The sync itself never waits for the answer.
   - `on_missing = Auto` → treat as deleted: apply `deleted_policy`:
     - `KeepMarked`: `local_status = 'archived'`, `downloaded` stays 1, `local_path` kept as "last known path".
     - `MarkNotDownloaded`: `downloaded = 0`, clear `downloaded_at/format/track snapshot/local_path/update flags`, `local_status = NULL`.
3. **Resolve dialog** (Ask mode), one row per album:
   - Columns: cover (FX), `Artist - Title`, last known path, best candidate from the artist's folder (if any, below the confident threshold), radio: **Moved** (use candidate / Browse… to the new folder) · **Deleted** · **Skip for now**.
   - Bulk buttons: *Mark all as deleted*, *The artist's folder moved…* (pick the new folder; it **replaces that artist's link** subject to §5.1, rescans, and retries relinking for that artist's rows), *Skip all*.
   - Footer: "When deleted:" combo (Keep marked as downloaded / Mark as not downloaded), defaulting to the setting, plus checkboxes "Make this my default" and "Stop asking (treat missing as deleted automatically)" which write `deleted_policy` and `on_missing = Auto`.
   - **Skip** sets `local_status = 'missing'` and `missing_ack_at = now`; the row is not asked again during syncs, only on a manual *Verify files* or when its path becomes reachable.

### 8.4 Manual actions (table and drawer, bulk-capable)
*Mark as archived*, *Mark as not downloaded*, *Relink to folder…* (validates that it contains audio), *Open folder* (disabled when missing), *Re-download* (allowed for archived and missing; warns that it uses the artist's free-download allowance).

## 9. Worker integration

```rust
// UiToWorkerCommand additions
LinkArtistFolder { band_id: i64, path: PathBuf, auto_apply_confident: bool },
UnlinkArtistFolder { band_id: i64 },
MatchArtistFolder { band_id: i64, auto_apply_confident: bool },
VerifyFiles { scope: SyncScope },
ApplyReconcile { batch_id: u64, decisions: Vec<MatchDecision> },
ResolveMissing { batch_id: u64, decisions: Vec<MissingDecision>, remember: Option<RememberChoice> },
RelinkRelease { key: ItemKey, path: PathBuf }, MarkArchived { keys: Vec<ItemKey> }, UnmarkDownloaded { keys: Vec<ItemKey> },
IgnoreLocal { band_id: i64, paths: Vec<PathBuf> }, UndoBatch { batch_id: u64 }, CancelLibrary,
// WorkerToUiEvent additions
LibraryProgress { phase: LibraryPhase, done: u32, total: u32, current: Option<String> },
ReconcileReport { report: Box<ReconcileReport> },
MissingFound { batch_id: u64, items: Vec<MissingItem>, mass_missing: bool },
LibraryApplied { batch_id: u64, summary: BatchSummary },
FolderStatus { band_id: i64, state: FolderState },
// SyncFinished gains: adopted: u32, missing_local: u32, relinked: u32
```

`SyncService` order per artist: resolve → discover → diff → metadata (network, unchanged) → **linked-folder rescan and certain-match adopt** (local, if linked and enabled) → **verify** (local, if enabled) → finalise. A `LibraryService` task owns the scanner, runs at most one library job at a time, and shares the writer `Db`. All DB writes go through the writer actor in single transactions per batch step.

## 10. UI specification (classic app; FX gets the same flows, restyled)

| Place | Addition |
| --- | --- |
| Link dialog | Two fields only: **Folder** (picker via `rfd::AsyncFileDialog` from a spawned task) and **Artist page** (dropdown of tracked artists, or paste a Bandcamp URL). Checkbox "Adopt confident matches automatically". Reached from: sidebar artist context menu *Link folder…* (artist preset, only the folder is asked), the add-artist dialog (optional Folder field), and a toolbar *Link folder to artist page…* (both fields). No folder-name-based suggestions anywhere |
| Sidebar artist context menu and footer | *Link folder…* / *Change folder…* / *Unlink folder*, *Match folder* (when linked), *Verify files*. Linked artists show a small folder glyph with a status dot (ok / missing / unreadable); the path is in the tooltip |
| Match report dialog (per artist) | Tabs: **Matched** (n) · **Review** (n) · **On page only** (n) · **Local only** (n) · **Conflicts** (n) · **Found again** (n). Checkbox lists with select-all; each row shows both sides (`Artist - Title` on the page vs local path and tag summary, score and signals as tooltip). Footer: `Apply selected (n)`, `Cancel`. After apply: toast with **Undo** |
| Resolve-missing dialog | As in §8.3, non-modal queue, also reachable from a dock chip `MISSING 12` |
| Table | New glyphs and chips (§1); filters *Archived*, *Missing files*; row actions per §8.4; a *Local path* line in tooltips |
| Settings, "Library" section | *Verify files after every sync*, *Re-read linked folders on sync*, *When files are missing* (Ask / Handle automatically), *If deleted* (Keep marked as downloaded / Mark as not downloaded), *Try to find moved albums automatically*, match thresholds (advanced). A read-only list of linked folders with status dots and an Unlink button each. No roots list |
| Drawer / history (FX) | `library_events` shown as a timeline ("linked", "adopted", "relinked", "archived", "restored"); the match report shows cover thumbnails of both the page and the local `cover.jpg` when present |

All dialogs keep the square look and the UI-thread rules.

## 11. Implementation phases and milestones

### Phase L0: Model and repos (1 day)
Migration `0004`, types, settings fields with serde defaults, `artist_folders` and link validation, repo functions with batch/event writing, filters and fragments, stats, index and query-plan tests.

**Milestone**
- [ ] Fresh DB lands at the new `user_version`; migrating an existing DB leaves every row intact and gives downloaded rows `unverified`.
- [ ] Invariant test (`downloaded = 0 ⇒ local_status IS NULL`) holds across every repo function; `undo_batch` restores exact previous values for adopt, relink, archive, unmark.
- [ ] Link rules (§5.1): duplicate, nested, containing and `out_dir`-overlapping links are rejected; unlinking leaves marks intact.
- [ ] `EXPLAIN QUERY PLAN`: the two new filters use `idx_releases_attention` / `idx_releases_archived` (no `SCAN releases`).
- [ ] Existing Phase 1 to 3 tests still pass.

### Phase L1: Normalisation, tags, single-folder scanner (1 day)
**Milestone**
- [ ] All §5 fixture trees produce the expected candidates for one linked folder; nothing outside the folder is ever opened (test with a sibling folder made unreadable).
- [ ] Symlink loop, NFC/NFD, Windows long path and unreadable-subdirectory cases do not crash and are reported.
- [ ] Scan is cancellable within 200 ms.

### Phase L2: Matcher, report, apply, undo (2 days)
**Milestone**
- [ ] Table-driven matcher tests cover every row of the §6 signal table and the ambiguity rules.
- [ ] End-to-end against a temp artist folder plus wiremock-served discography: adopt marks the right releases, `page_only`/`local_only`/`conflicts` are exactly as built in the fixture, and an `Incremental` sync afterwards still issues one request.
- [ ] Linking a folder for an artist that has not synced yet queues the match until after its first sync.
- [ ] Undo of an adopt batch returns every touched row to its prior state.

### Phase L3: Verify, policies, relink, sync integration (2 days)
**Milestone**
- [ ] Deleting one album folder and syncing: `Ask` produces one `MissingFound`; `Auto` + `KeepMarked` yields `archived` with `downloaded = 1`; `Auto` + `MarkNotDownloaded` yields `downloaded = 0` with fields cleared.
- [ ] Moving an album to another place inside the linked folder and syncing relinks it silently; two identical candidates ask instead.
- [ ] Removing the linked folder (temp dir removed) produces `Unknown` and a warning, never `missing`; removing 30% of 40 albums triggers the mass-missing guard.
- [ ] A newly added album folder is auto-adopted on the next sync when certain, and only raises `REVIEW n` when not; with `rescan_linked_on_sync` off nothing is adopted on sync.
- [ ] *The artist's folder moved…* replaces the link and relinks that artist's rows.
- [ ] `Skip` is not re-asked on the next sync but is re-asked on `VerifyFiles`.

### Phase L4: Classic UI (1 to 2 days)
**Milestone**
- [ ] Walk-through on a real folder: *Link folder…* for one artist shows the report, applying marks rows in place, Undo works.
- [ ] Delete an album folder, sync, answer the dialog both ways (including "the artist's folder moved"); each outcome matches the chosen policy.
- [ ] Settings persist; the linked-folders list shows live status; frame time unaffected.

### Phase L5: FX integration and polish (1 day)
Link and resolve dialogs restyled with covers and animated transitions, smart views *Archived* and *Missing files*, drawer timeline from `library_events`, palette commands.

**Milestone**
- [ ] FX contract check includes the new columns and tables; both apps open the same DB and show identical statuses.
- [ ] `docs/visual-audit-fx.png` updated with the new dialogs; square-look guard still prints nothing.

### Phase L6: Hardening (1 day)
Windows extended paths and drive-letter probing, macOS case-insensitive comparisons, network-share timeouts, README section on folder links, the archive workflow and policies, `docs/library.md`.

## 12. Risks and open questions

| Risk | Mitigation |
| --- | --- |
| Wrong auto-match marks an album downloaded | Certain only on URL tag or two agreeing strong signals plus runner-up margin; everything else goes to review; every batch is undoable; matching is limited to one artist's releases, which removes most look-alike collisions. |
| User links a folder that actually holds several artists | Soft "looks like a library" warning (§5.1); nothing is matched outside the chosen artist's releases anyway, so the result is unmatched folders, not wrong marks. |
| Unmounted drive causes mass "missing" | Folder probing and the mass-missing guard (§8.2). |
| Tag quality varies (bootlegs, retagged files, no URL tag) | Folder-name and fuzzy fallbacks; review list; fixtures decide what Bandcamp downloads actually carry. |
| Albums nested deeper than one level are not found | Documented in §5.2; link the folder that directly holds the albums. Revisit only if it proves common. |
| `archived` rows hide real gaps | Filters and smart view *Archived*; stats tooltip shows archived count separately. |
| Silent adoption on sync surprises the user | Only certain matches, summary toast with Undo, and a setting to turn it off. |
| Schema change breaks the older binary | §4.4: rebuild both apps together; `TooNew` message names the fix. |

**Defaults assumed; confirm or override:**
1. A link is exactly one folder plus one artist page that the user supplies; nothing is ever inferred.
2. `deleted_policy = KeepMarked` (confirmed by you), `on_missing = Ask`, `verify_on_sync = true`, `auto_relink = true`.
3. `rescan_linked_on_sync = true`: later syncs auto-adopt certain matches from the linked folder (veto to make linking a one-time action).
4. Adoption snapshots the page's track count (no false `+TRACKS`).
5. Gone-from-page detection is dropped from this feature.

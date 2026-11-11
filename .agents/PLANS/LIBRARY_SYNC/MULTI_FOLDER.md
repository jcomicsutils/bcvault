# Plan: several linked folders per artist

For the coding agent. Repo: bcvault (Rust workspace). Place this file at `.agents/PLANS/LIBRARY_SYNC/MULTI_FOLDER.md`. It amends `LIBRARY_SYNC_PLAN.md` §5 ("one folder per artist, one artist per folder") to **many folders per artist, one artist per folder**.

> Status 11 Nov 2026: **implemented** on `main` (Steps 1 to 6, migration `0006_artist_folders_multi.sql`, shipped in `v0.1.3`; `Task: none` commits recorded in `PROGRESS.md`).
> **Prerequisite:** `MISSING-FOLDER-FIX.md` has landed. This plan generalises its `UnavailableRoot::Linked { band_id }` to `Linked { folder_id }` and its single-link per-release root attribution to "longest matching link among N". If that fix has not landed, do it first and do not merge the two plans.
> **Out of scope:** downloading into a linked folder (downloads keep using `out_dir` and the layout setting), per-folder scan depth, per-link folder style, link labels.

## 0. Ground rules

- Follow `AGENTS.md`: Conventional Commits with a scope from §5.2 and a `Task: none` trailer, `scripts/check.sh` before each commit, no `unwrap`/`expect` outside tests, UI-thread purity, square look.
- Library sync reads the filesystem only, inside linked folders and `out_dir`. Nothing in this plan writes to user folders.
- Migrations are immutable once on `main`. This plan adds exactly one new migration file. Take the lowest free number at merge time (at the time of writing that is `0006`; `AUTO_SYNC` and `ARCHIVE` are planned for `0006`/`0007` and must be renumbered if this lands first, per `ROADMAP.md` §5, including their plan text). Update the `ROADMAP.md` §5 table in the docs step.
- This is a breaking migration for older binaries (`feat(db)!`). Classic and FX share the main DB, so they ship together.
- Line numbers below come from a snapshot. Grep, do not trust them.
- Read before editing: `migrations/0004_library.sql`, `0005_artist_folder_local_only.sql`, `db/migrations.rs` (how migrations run: inside a transaction, with `foreign_keys = ON` set in `db/mod.rs`), `db/repo/library.rs` (`link_folder`, `unlink_folder`, `get_folder`, `all_folders`, `update_folder_local_only_count`, `ignore_local`, `unignore_local`, `get_ignored_local`), `db/repo/releases.rs` (the stats SQL around the `LEFT JOIN artist_folders` and the scalar `local_only_count` subquery), `library/link.rs`, `library/service.rs` (`run_scan_and_match`, `linked_releases_under`), `library/scan.rs`, `library/verify.rs`, `worker/library.rs` (about 15 non-test call sites of the folder API between the top of the file and the tests), `worker/{messages,supervisor,sync}.rs`, and the UI code listed in §3.7.
- If something below turns out unworkable, stop and report instead of improvising.

## 1. Why

A Bandcamp page can hold several unrelated projects of one artist (for example one page, many projects, each kept in its own local folder, possibly in different places). Today:

- `artist_folders.band_id` is the primary key, and `link_folder` replaces any existing link, so only one folder per artist.
- `scan_linked_folder` reads the direct children of one root only. `Artist/Project/Album` is not found, and a project folder that is itself an album can't be linked together with others.
- Verify, relink, rescan-on-sync and the folder-moved flow all call `get_folder(band_id)` (one folder).
- Both UIs keep `HashMap<i64, ArtistFolder>`.

The matcher does not care: `assign_matches` takes a flat list of candidates and a list of releases. The change is plumbing around it, and the matcher stays untouched.

## 2. Goals and non-goals

Goals:

- An artist can have any number of linked folders. A folder (and any folder nested in or containing it) still belongs to exactly one artist.
- Scan, match, adopt, relink and verify work over the union of an artist's folders.
- Per-folder state (`Ok`/`Missing`/`Unreadable`) is tracked; one offline folder never blocks the others and never causes a wrong "missing" conclusion.
- Existing databases upgrade with no data loss; single-folder behaviour is unchanged for artists with one link.

Non-goals: see the "Out of scope" note at the top. Also no change to matcher scoring, folder style templates, `Settings`, or the album-level missing flow other than the partial-availability rule in §3.5.

## 3. Specification

### 3.1 Schema (new migration, `0006_artist_folders_multi.sql` or next free number)

SQLite cannot change a primary key, so rebuild. `artist_folder_ignored` must be rebuilt too: its foreign key points at `artist_folders(band_id)`, which stops being unique, and SQLite then fails DML with "foreign key mismatch". Re-point it at `artists(band_id)`.

```sql
CREATE TABLE artist_folders_new (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    band_id          INTEGER NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE,
    path             TEXT NOT NULL,
    path_key         TEXT NOT NULL UNIQUE,
    added_at         INTEGER NOT NULL DEFAULT (unixepoch()),
    last_scan_at     INTEGER,
    local_only_count INTEGER NOT NULL DEFAULT 0
) STRICT;
INSERT INTO artist_folders_new (band_id, path, path_key, added_at, last_scan_at, local_only_count)
    SELECT band_id, path, path_key, added_at, last_scan_at, local_only_count FROM artist_folders;

CREATE TABLE artist_folder_ignored_new (
    band_id  INTEGER NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE,
    path_key TEXT NOT NULL,
    PRIMARY KEY (band_id, path_key)
) STRICT;
INSERT INTO artist_folder_ignored_new SELECT band_id, path_key FROM artist_folder_ignored;

-- Drop the child first: with foreign_keys ON, dropping a parent that still has
-- referencing rows runs an implicit DELETE and would cascade.
DROP TABLE artist_folder_ignored;
DROP TABLE artist_folders;
ALTER TABLE artist_folders_new RENAME TO artist_folders;
ALTER TABLE artist_folder_ignored_new RENAME TO artist_folder_ignored;
CREATE INDEX idx_artist_folders_band ON artist_folders(band_id);
```

Verify against the real runner: the runner uses a transaction and `PRAGMA foreign_keys` cannot be changed inside one, so the order above must not rely on it. Register the file in `db/migrations.rs`. `local_only_count` stays per folder (§3.4). No `label` column (out of scope).

FX's schema contract check (`bcvault-fx/src/db/mod.rs`, the `artist_folders` entry listing `band_id`, `path`, `path_key`) stays valid. Its test fixture (`CREATE TABLE m.artist_folders (...)`) may stay minimal.

### 3.2 Types (`types.rs`)

```rust
pub struct ArtistFolder {
    pub id: i64,            // new: stable row id
    pub band_id: i64,
    pub path: PathBuf,
    pub state: FolderState,
}
```

### 3.3 Repo API (`db/repo/library.rs`)

| New | Behaviour |
| --- | --- |
| `add_folder(conn, band_id, path, out_dir) -> Result<ArtistFolder, LinkFolderError>` | Insert. Same validations as today (exists, is dir, readable, canonicalised, not overlapping `out_dir`, not overlapping **another artist's** folders), plus the same-artist rules below. |
| `replace_folder(conn, folder_id, new_path, out_dir)` | Same validations, ignoring the folder itself. Keeps `id`, resets `local_only_count` to 0. Ignored rows are kept (they were kept before too). |
| `unlink_folder(conn, folder_id) -> Result<bool, DbError>` | Delete the row and the artist's ignored rows that lie under its `path_key`. Other folders of the artist are untouched. |
| `get_folders(conn, band_id) -> Vec<ArtistFolder>` | Ordered by `added_at, id`. Probes each state. |
| `get_folder_by_id`, `all_folders` | `all_folders` returns one entry per row. |
| `owning_folder(folders: &[ArtistFolder], path: &Path) -> Option<&ArtistFolder>` | Pure: the folder whose `path_key` equals the path key or is a `/`-prefix of it; if several, the longest. |
| `update_folder_local_only_count(conn, folder_id, count)` | Keyed by folder id. |
| `ignore_local` / `unignore_local` | Keep `(band_id, path_key)`; adjust `local_only_count` of the **owning folder** only. |

Same-artist overlap rules (new `LinkFolderError` variants, and the matching `FolderLinkError` variants in `library/link.rs`):

- Equal to, or inside, an existing link of the same artist: `AlreadyCovered { path }` ("already covered by <path>").
- Contains an existing link of the same artist: `ContainsOwnFolder { path }` ("contains <path>; unlink that first"). No auto-merge.
- Overlap with another artist's folder: unchanged (`OverlapsExisting`).

Prefix comparisons on `path_key` in SQL must not use `LIKE` (a folder named `100%_pure` would act as a wildcard). Use `path_key = ?1 OR substr(path_key, 1, length(?1) + 1) = ?1 || '/'`.

`link_folder` and `get_folder` (single) are replaced. Keep thin wrappers in the first commit only (§5, step 1) so the repo compiles while callers migrate; delete them in step 3.

Stats SQL (`db/repo/releases.rs`) must aggregate instead of joining rows, otherwise artists with two links appear twice and the scalar subquery silently returns the first row:

- Replace `LEFT JOIN artist_folders af ON af.band_id = s.band_id` with a join on `(SELECT band_id, SUM(local_only_count) AS local_only_count FROM artist_folders GROUP BY band_id)`.
- Replace `(SELECT local_only_count FROM artist_folders WHERE band_id = ?1)` with `(SELECT COALESCE(SUM(local_only_count), 0) FROM artist_folders WHERE band_id = ?1)`.
- Check the `INSERT INTO artist_folders ...` near the end of `releases.rs` (test helper or real code) and `bcvault-fx` for other readers.

### 3.4 Scan and match (`library/service.rs`)

`run_scan_and_match(conn, band_id, ...)`:

1. `folders = get_folders(conn, band_id)`. None: return `Ok(None)`.
2. Probe each. If none is `Ok`: return `Ok(None)`. Otherwise scan each `Ok` folder with `scan_linked_folder`, concatenate the candidates and sort them by `path_key` (deterministic). Progress reports the folder being scanned.
3. `linked` and `active_linked_keys` are computed over all the artist's releases as today; `linked_releases_under` is applied per available folder and merged (dedupe by key).
4. Matching is unchanged. The same album found in two folders becomes the existing duplicate `Conflict`.
5. Local-only counts: attribute each `assignment.local_only` entry to its folder with `owning_folder`, then `update_folder_local_only_count` for every **available** folder (including 0). Counts of unavailable folders are not touched (their orphans are unknown).
6. `ReconcileReport` gains `unavailable_folders: Vec<i64>` (folder ids skipped this run). Matches are positive evidence and stay safe to auto-apply; `page_only` is informational. The UI shows "N of M folders unavailable; results may be incomplete" when the list is not empty.

`scan_linked_folder` and `is_audio_file` are unchanged. Each folder keeps depth 1 (direct children, with disc subfolders handled as today); a project folder that is itself an album works because `SingleAlbumRoot` already applies.

### 3.5 Verify and relink (`worker/library.rs`)

- **Root attribution** (generalising `MISSING-FOLDER-FIX` §3.2): each downloaded release belongs to the artist's link that `owning_folder` returns for its `local_path`; else `OutDir` when it has a path; else (no path) the artist's first link if any, else `OutDir`. Probe each root once per run. Unavailable roots produce `FolderUnavailable` per folder id (`UnavailableRoot::Linked { folder_id }`; update `types.rs`, messages, both dialogs, and the tests of that plan accordingly). `ResolveUnavailable`'s `unlink` unlinks that one folder.
- **Auto-relink (step 8 of `verify_files`)**: scan every `Ok` folder of the artist (not just one), merge candidates, and run `find_relink_candidates` once over the merged list. An album that moved from one project folder to another is relinked silently, exactly like a move within one folder. If no folder is `Ok`, behave as before (skip).
- **Partial availability rule:** if an artist has at least one folder that is not `Ok`, missing albums of that artist that remain unresolved after relink are **never auto-resolved**, even with `on_missing = Auto`; they go to the Ask list (`MissingFound`) for that pass, because the album may live in the offline folder. This does not affect other artists.
- `rescan_and_adopt_on_sync`: uses `run_scan_and_match` as is (union, partial allowed).
- Mass-missing ratio: unchanged (unavailable-root releases remain `Unknown` and are not counted).

### 3.6 Worker, commands and events

| Today | After |
| --- | --- |
| `LinkArtistFolder { band_id, path }` (replaces) | Same name and fields; now means **add**. Doc comment says so. Errors surface as log lines and in the link modal through the existing error path |
| `AddAndLinkArtistFolder { ... }` | Same, add semantics |
| `UnlinkArtistFolder { band_id }` | `UnlinkArtistFolder { folder_id }` |
| `RepointArtistFolder { band_id, new_path }` | `RepointArtistFolder { folder_id, new_path }` |
| `FolderStatus { band_id, folder: Option<ArtistFolder> }` | `FolderStatus { band_id, folders: Vec<ArtistFolder> }`: always the full current list for the artist; an empty list means none linked |
| `Ready { ..., folders: Vec<ArtistFolder> }` | Unchanged shape; now holds every row |

`MatchArtistFolder { band_id, .. }` stays artist-level. `link_artist_folder` runs the first match after adding (including adding a second folder), exactly as it does today. `unlink_artist_folder` looks up the `band_id` from the folder row first, then emits `FolderStatus` with the remaining list, `ArtistUpserted` and `ArtistStats`, and clears `pending_first_matches` only when no folder remains. `repoint_artist_folder` keeps its post-repoint verify. `ignore_local` finds the owning folder for each path.

### 3.7 UI (both front-ends; classic first, FX restyled, same flows)

State: `folders: HashMap<i64 /*band_id*/, Vec<ArtistFolder>>`. `Ready` groups by `band_id`; `FolderStatus` replaces the artist's vector (removes the key when empty).

- **Sidebar artist glyph and tooltip:** one glyph showing the worst state among the artist's folders (`Missing`/`Unreadable` beat `Ok`); tooltip lists each folder with its state. The "local only" badge uses the aggregated count from stats (already summed in §3.3).
- **Artist context menu:** "Link folder..." becomes "Add folder..." when the artist already has links. Per folder: "Unlink <last path component>..." and "Folder moved: change <last path component>..." (a submenu when there is more than one folder; flat when there is one, so single-folder users see no change). Use the last two path components when two links share a basename.
- **Link modal:** when the artist already has links, the title is "Add another folder for <artist>" and existing links are listed read-only. The new `AlreadyCovered` and `ContainsOwnFolder` errors show inline with the path in the message. The soft "looks like a library" warning is unchanged.
- **Settings, linked folders list:** one row per folder (artist name, path, state, Unlink).
- **Unlinked local folders / reconcile views:** when the artist has more than one folder, show each orphan's path relative to its owning folder, prefixed by that folder's last component (`ProjectB / Album`). Show the partial-results banner when `unavailable_folders` is not empty.
- **Both UIs' state tests** follow the shape of the existing `test_folder_status` style tests (see `state.rs` around "Ready populates folders").

## 4. Tests

Write each test first.

| # | Area | Scenario | Expected |
| --- | --- | --- | --- |
| 1 | Migration | Populated 0005 DB (folders with `local_only_count`, ignored rows, library events) upgraded | All rows preserved; `PRAGMA foreign_key_check` empty; `integrity_check` ok; inserting two folders for one band works; deleting an artist cascades folders and ignores |
| 2 | Repo | Two folders for one artist | Both returned in `added_at, id` order; each probed |
| 3 | Repo | Same path added twice; path inside own folder; path containing own folder | `AlreadyCovered` / `ContainsOwnFolder`; no row written |
| 4 | Repo | Second folder of artist A overlapping artist B's folder; folder overlapping `out_dir` | Still rejected |
| 5 | Repo | `replace_folder` | Keeps id, validates ignoring itself, resets count; rejects overlap with another artist |
| 6 | Repo | `unlink_folder` with two folders and ignores under both | Only the unlinked folder's ignores deleted; other folder and its ignores intact |
| 7 | Repo | Folder name containing `%` and `_` next to a similarly named sibling | Prefix logic does not match the sibling (no wildcard behaviour) |
| 8 | Repo | `owning_folder` with nested-name collisions (`/a/b` vs `/a/bc`) | Prefix is `/`-aware; longest wins |
| 9 | Stats | Artist with two links and local-only counts 2 and 3 | One row for the artist; `local_only` is 5; `artist_stats_for_one` agrees; artists without links unchanged |
| 10 | Service | Candidates in two folders | Report matches releases from both; candidates sorted deterministically |
| 11 | Service | Same album folder name in both | One duplicate `Conflict`, nothing auto-adopted for it |
| 12 | Service | Local-only orphans in each folder | Per-folder counts match; ignoring one decrements only its folder |
| 13 | Service | One folder `Missing`, one `Ok` | Result from the available one; `unavailable_folders` lists the other; the unavailable folder's count is unchanged |
| 14 | Service | All folders unavailable | `Ok(None)` |
| 15 | Verify | Release path under folder B while folder A is deleted | Verified normally against B; A's releases go to `FolderUnavailable { Linked { folder_id: A } }` |
| 16 | Verify | Album moved from folder A to folder B | Silently relinked (`local_path` switches), no event |
| 17 | Verify | Missing album, artist has an offline second folder, `on_missing = Auto` | Goes to `MissingFound` (Ask); nothing auto-archived. Another artist with all folders online still auto-resolves |
| 18 | Worker | `LinkArtistFolder` twice for one artist | `FolderStatus` with two folders; first-match flow runs for the new folder |
| 19 | Worker | `UnlinkArtistFolder { folder_id }` | Remaining folder kept; `FolderStatus` has one; stats re-emitted; `pending_first_matches` cleared only when none remain |
| 20 | Worker | `RepointArtistFolder { folder_id, .. }` | That folder replaced; verify runs |
| 21 | Worker | `ResolveUnavailable` with `unlink: true` | Unlinks only that folder |
| 22 | UI state (classic and FX) | `Ready` with 3 folders over 2 artists; `FolderStatus` with a shorter list and with an empty list | Vectors replaced; key removed when empty |
| 23 | Invariants | `assert_library_invariants` after 16, 18 to 21 | Passes |
| 24 | Regression | Single-folder artist flows (existing tests) | Unchanged results |

Also run the full workspace test suite, FX's schema contract test, and grep for remaining uses of `get_folder(` and the single `link_folder(`; none may remain after step 3.

## 5. Decisions made (veto any; flag if impossible)

- **Many folders per artist, one artist per folder.** Nested or equal links of the same artist are rejected, not merged.
- **Depth stays 1.** To cover several projects, link each project folder. A per-link depth option is a possible later addition.
- **`local_only_count` stays per folder** and is summed for the badge.
- **No label column**; folders are identified by their last path component(s).
- **Adding a folder runs the same "first match" flow** as the first link.
- **Unlinking one folder keeps the others**, and removes only the ignores under that folder. Marks and `local_path` values are never touched.
- **Partial availability never auto-archives** that artist's missing albums (Ask instead).
- **Folder style stays global.** If project folders use different naming, rely on the template list and the raw-folder-name fallback; a per-link style is out of scope.
- **Downloading is unaffected**: new downloads still go to `out_dir`; the user moves them, and relink picks them up in whichever linked folder they land.

## 6. Commit sequence

Each step compiles with all tests green across the workspace.

1. `feat(db)!: allow several folders per artist` adds the migration, `ArtistFolder.id`, the repo API of §3.3, the stats SQL fix, and thin wrappers (`link_folder` = replace the artist's first folder or add; `get_folder` = first folder) so existing callers keep their behaviour. Tests 1 to 9, 24.
2. `feat(library): scan, match and verify across an artist's folders` implements §3.4 and §3.5 in core, using `get_folders`. Tests 10 to 17. No message shape change yet.
3. `feat(worker)!: address linked folders by id` implements §3.6 (commands, events, flows), removes the wrappers, and makes the minimal adaptations in both UI crates so the workspace compiles (state maps become `Vec`, existing UX still shows the first folder). Tests 18 to 22, 23.
4. `feat(classic): add, unlink and change several folders per artist` implements §3.7 in the classic UI (sidebar, menu, modals, settings list, banners), with its state tests.
5. `feat(fx): add, unlink and change several folders per artist` same for `bcvault-fx`.
6. `docs(library): document several folders per artist` rewrites "One folder per artist" in `docs/library.md` to "many folders per artist, one artist per folder" and documents the partial-availability rule, adds this plan to the `AGENTS.md` §1 table, updates `ROADMAP.md` (status snapshot, §5 migration row, §6 "Artist folder links" row), and updates `PROGRESS.md`.

## 7. Definition of done

- `scripts/check.sh` passes.
- A database created before this change opens and upgrades with all links, counts and ignored rows intact.
- Manual check (ask the user to perform it): link two project folders of one artist (from different parent directories), run **Match Folder**; albums from both are adopted, orphans show their folder, and one **Add folder** error message is correct when selecting a nested path. Delete or rename one folder: the other keeps working, results show the partial banner, and no missing album is auto-archived. Move an album from one project folder to the other: it relinks without a prompt.
- Single-folder artists see no UI change.

# Plan: persistent review rejection and searchable slug/URL release matching

For the coding agent. Repo: bcvault (Rust workspace). Scope: `crates/bcvault-core/src/migrations/`, `crates/bcvault-core/src/db/{migrations.rs,repo/library.rs}`, `crates/bcvault-core/src/library/{matcher,verify,service}.rs`, `crates/bcvault-core/src/worker/{library,messages,supervisor}.rs`, `crates/bcvault-core/src/types.rs`, both UIs (`crates/bcvault/src/ui/modals.rs` and `crates/bcvault-fx/src/overlays/modals.rs`), and docs. Place this file at `.agents/PLANS/LIBRARY_SYNC/REVIEW-AND-MATCH-FIX.md`.

> Status 10 Nov 2026: **implemented** on `main` (`Task: none` commits, migration `0008_rejected_matches.sql`). Steps are `Task: none` commits (step numbers go in `PROGRESS.md`), like `MATCH-FIX.md` and `DOWNLOAD-DIR-FIX.md`. Amends `LIBRARY_SYNC_PLAN.md` §5, §6, and §10, and `MATCH-FIX.md` §3.5.

## 0. Ground rules

- Follow `AGENTS.md`: stay on `main` (or a `fix/review-and-match-ux` branch fast-forwarded to `main`), use Conventional Commits with a scope from §5.2 and a `Task: none` trailer, one commit per step in section 6, run `scripts/check.sh` before each commit, no `unwrap`/`expect` outside tests, UI-thread purity, and zero corner radius (`CornerRadius::ZERO`).
- Library sync is local-only I/O. Never modify, rename, move, or delete user files.
- Migrations are immutable once on `main`. Add one new migration file taking the lowest free number (`0008_rejected_matches.sql`, after `0007_unavailable.sql`). Because a new main-DB migration bumps `MAX_VERSION`, mark the migration commit with `!` and a `BREAKING CHANGE:` footer, and ship `bcvault` and `bcvault-fx` together.
- Read before editing:
  - `crates/bcvault-core/src/library/matcher.rs` (`assign_matches`, `fuzzy_sim`, `ReviewItem`, `ScoredCandidate`, `Conflict`, `LocalOrphan`, `normalize_bandcamp_url`)
  - `crates/bcvault-core/src/library/verify.rs` (`find_relink_candidates`)
  - `crates/bcvault-core/src/library/service.rs` (`ReconcileReport`, `run_scan_and_match`)
  - `crates/bcvault-core/src/db/repo/library.rs` (`ignore_local`, `unignore_local`, `get_ignored_local`, `unlink_folder`, `create_batch`, `update_batch_summary`, `undo_batch`)
  - `crates/bcvault-core/src/worker/library.rs` (`match_artist_folder_internal`, `rescan_and_adopt_on_sync`, `apply_reconcile_internal`)
  - `crates/bcvault-core/src/types.rs` (`MatchDecision`, `BatchSummary`)
  - `crates/bcvault/src/ui/modals.rs` (`show_reconcile_report_modal`, `ReconcileReportModalState`, `ReportTab`, `LocalAction`)
  - `crates/bcvault-fx/src/overlays/modals.rs` (`show_reconcile_report_modal`, `FxReconcileModal`, `FxReportTab`, `FxLocalAction`)
- Line numbers below come from a snapshot; grep rather than trusting them blindly.

## 1. Problem

Two closely related UX and matching defects make linking and reconciling real-world artist folders painful:

1. **No way to say "wrong, don't ask again" in the Review tab.**
   When linking a folder (for example an artist folder containing an album titled `"Wall Split Action 001"`), the containment rule in `matcher::fuzzy_sim` (`ta.is_subset(&tb) || tb.is_subset(&ta)`) scores `"Wall Split Action 001"` at `0.85` against every release on the artist's Bandcamp page titled `"split"` (9 different releases). Because `0.85 >= review_match_min` (`0.60`) and `is_contained` forces `auto_eligible = false`, all 9 `"split"` releases land in the **Review** tab proposing `"Wall Split Action 001"`.
   - The Review tab only offers candidate radio buttons and `"Skip for now"`.
   - `"Skip for now"` persists nothing in the database (`modal.selected_review.remove(&r.release_key)`).
   - Every time the user links another folder for that artist, clicks **Match Folder Now**, or syncs (`rescan_and_adopt_on_sync`), the exact same 9 wrong review prompts reappear.
   - Worse, while `"Wall Split Action 001"` is referenced by any `ReviewItem` (`review_cand_paths` in `assign_matches`), it is excluded from `local_only`, so the user cannot even ignore or manually attach `"Wall Split Action 001"` from the **Local only** tab until every false review pairing is dismissed.
   - And if a report contains *only* false review items and the user leaves them all skipped, `total_selected_count()` is `0`, leaving **Apply selected (0)** disabled.

2. **No search, disambiguation, or slug/URL input when matching Local Only albums or resolving Conflicts.**
   - In the **Local only** tab, choosing `"Attach to release..."` opens a plain `egui::ComboBox` populated from `state.rows` showing only `release.title`:
     - There is no search or filter box inside the picker.
     - When an artist has 10 releases titled `"split"` (or `"Untitled"`, `"Demo"`, `"Live"`), all 10 entries in the dropdown render identically as `"split"`, making it impossible to know which one is which.
     - There is no way to paste or type the Bandcamp release **slug** (e.g. `split-3` or `album/split-3`) or **URL** (e.g. `https://artist.bandcamp.com/album/split-3`) to guarantee matching the folder to the exact page release.
     - Populating the dropdown from `state.rows` is also brittle: if the main window has an active search filter, classification filter, or another selected artist, `state.rows` may not even contain all releases of `report.band_id`.
   - In the **Conflicts** tab (`ReportTab::Conflicts` / `FxReportTab::Conflicts`), the UI is currently read-only: it prints `c.description` and `Paths involved` with **no resolution controls at all**, forcing the user to rename folders on disk or leave conflicts permanently unresolved.
   - In the **Review** tab, each card header shows only `r.release_title`, so 9 review cards for `"split"` also look identical without their slug, year, or track count.

## 2. Causes found by reading the code

1. `artist_folder_ignored` (`0004_library.sql`, `0006_artist_folders_multi.sql`) only stores `(band_id, path_key)`, which permanently hides a local folder from all matching. There is no table or repo API to record that a specific `(release_key, candidate_path_key)` pair was rejected by the user.
2. `assign_matches` in `crates/bcvault-core/src/library/matcher.rs` only accepts `ignored_path_keys: &HashSet<String>` (folder-wide ignore) and has no `rejected_pairs: &HashSet<(ItemKey, String)>` filter.
3. `MatchDecision` in `crates/bcvault-core/src/types.rs` has only `Adopt`, `Attach`, `Restore`, and `Ignore { band_id, path }`. There is no `Reject` variant for rejecting a `(release_key, path)` pair.
4. `ReconcileReport` in `crates/bcvault-core/src/library/service.rs` does not carry the artist's catalog metadata (`Vec<ReleaseRow>` or `Vec<ReleasePickerItem>`), forcing UI modals to look up titles in `state.rows`, and `ReviewItem` only carries `release_key` and `release_title` (no `url`, `published_at`, `track_count`, or `classification`).
5. `show_reconcile_report_modal` in both `crates/bcvault/src/ui/modals.rs` and `crates/bcvault-fx/src/overlays/modals.rs` uses an unsearchable `egui::ComboBox` for `LocalAction::Attach` and renders `ReportTab::Conflicts` with zero action widgets.

## 3. Goals and non-goals

Goals:

- Let the user mark any Review proposal (a `(release, candidate)` pair or all candidates under a `ReviewItem`) as **"Wrong, don't ask again"**, persisted in SQLite so future folder links, manual matches, sync rescans, and auto-relinks never propose that pair again.
- Provide a one-click batch action in the Review tab (**"Mark all skipped as wrong (don't ask again)"**) so a user facing 9 false `"split"` review cards can reject all of them at once.
- When all review pairings for a candidate folder are rejected and it has no other match, move that candidate immediately into **Local only** on subsequent scans (and allow rejecting it in the current modal session).
- Replace the plain title-only dropdown in **Local only** with a **searchable release picker + slug/URL input** that:
  - Accepts a full Bandcamp URL (`https://artist.bandcamp.com/album/split-3`), a typed path (`album/split-3`, `track/split-3`), or a bare slug (`split-3`) for deterministic 1-to-1 matching.
  - Filters the artist's releases live by title, slug, year, classification, or track count.
  - Displays disambiguating metadata on every row: `{title} — /{kind}/{slug} ({year}, {tracks} trk, {class})` plus an **Open** button to inspect the release page in the browser.
- Add full interactive resolution controls to the **Conflicts** tab using the same searchable release picker + slug/URL input (per candidate folder in the conflict: **Leave**, **Ignore folder**, or **Match to release...**).
- Show disambiguating release metadata (`/{kind}/{slug}`, year, track count, and an **Open** link button) on every **Review**, **On page only**, and **Conflicts** card so duplicate titles like `"split"` are always distinguishable.
- Support full batch **Undo** for rejected review pairs when undoing a reconcile batch.

Non-goals:

- Changing the containment score cap (`0.85`) or digit guard in `matcher::fuzzy_sim` (containment is useful when legitimately matching `Album` to `Album (Deluxe)`; persistent rejection solves false positives cleanly without breaking true containment reviews).
- Making network requests during matching or inside the Match Report modal. All slug/URL resolution runs against the artist's synced `releases` rows in SQLite.
- Modifying or deleting any files on disk.

## 4. Specification

### 4.1 Schema (migration `0008_rejected_matches.sql` or next free number)

Add `crates/bcvault-core/migrations/0008_rejected_matches.sql` and register version `8` in `crates/bcvault-core/src/db/migrations.rs` (`MAX_VERSION = 8`):

```sql
CREATE TABLE artist_folder_rejected (
    band_id    INTEGER NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE,
    item_type  TEXT    NOT NULL CHECK (item_type IN ('album','track')),
    item_id    INTEGER NOT NULL,
    path_key   TEXT    NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (band_id, item_type, item_id, path_key),
    FOREIGN KEY (item_type, item_id) REFERENCES releases(item_type, item_id) ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_artist_folder_rejected_band ON artist_folder_rejected(band_id);
```

- `path_key` is normalized with `library::scan::normalize_candidate_path_key` / `library::link::normalize_path_key` (identical to `artist_folder_ignored.path_key`).
- In `db/repo/library.rs::unlink_folder(conn, folder_id)`, also delete rejected pairs belonging to the unlinked folder's `path_key` prefix:
  ```sql
  DELETE FROM artist_folder_rejected
  WHERE band_id = ?1
    AND (path_key = ?2 OR substr(path_key, 1, length(?2) + 1) = ?2 || '/');
  ```
  (Use `substr` prefix comparison, never `LIKE`, per `MULTI_FOLDER.md` §3.3.)

### 4.2 Repository API (`crates/bcvault-core/src/db/repo/library.rs`)

Add:

| Function | Behaviour |
| --- | --- |
| `reject_match_pair(conn, band_id: i64, key: ItemKey, path_key: &str, batch_id: Option<i64>) -> Result<bool, DbError>` | `INSERT OR IGNORE INTO artist_folder_rejected (band_id, item_type, item_id, path_key) VALUES (?1, ?2, ?3, ?4)`. If `batch_id` is `Some(bid)` and a row was inserted (`affected > 0`), record a `library_events` entry with `kind = 'rejected'` and `new_json = Some(path_key_json)` so `undo_batch` can reverse it. Returns `Ok(affected > 0)`. |
| `unreject_match_pair(conn, band_id: i64, key: ItemKey, path_key: &str) -> Result<bool, DbError>` | `DELETE FROM artist_folder_rejected WHERE band_id = ?1 AND item_type = ?2 AND item_id = ?3 AND path_key = ?4`. Called automatically when `adopt`, `attach`, or `relink` binds `key` to `path`. |
| `get_rejected_pairs(conn, band_id: i64) -> Result<HashSet<(ItemKey, String)>, DbError>` | Returns all `(ItemKey, path_key)` pairs rejected for `band_id`. |

Update `undo_batch(conn, batch_id)`:
- When reversing a `library_events` row with `kind = "rejected"`, parse the `path_key` from `new_json` and delete that `(item_type, item_id, path_key)` row from `artist_folder_rejected`.
- Update `library_events` CHECK constraint only if one exists (check `0004_library.sql`: `library_events.kind` has no CHECK constraint, so `'rejected'` works without rebuilding `library_events`).

### 4.3 Types (`crates/bcvault-core/src/types.rs`)

1. Expand `MatchDecision`:
   ```rust
   pub enum MatchDecision {
       Adopt {
           key: ItemKey,
           path: PathBuf,
           format: Option<AudioFormat>,
           folder_mtime: Option<Ts>,
       },
       Attach {
           key: ItemKey,
           path: PathBuf,
           format: Option<AudioFormat>,
           folder_mtime: Option<Ts>,
       },
       Restore {
           key: ItemKey,
           path: PathBuf,
       },
       Ignore {
           band_id: i64,
           path: PathBuf,
       },
       /// Permanently rejects a (release, candidate folder) pair so it is never proposed again.
       Reject {
           band_id: i64,
           key: ItemKey,
           path: PathBuf,
       },
   }
   ```
2. Expand `BatchSummary`:
   - Add `#[serde(default)] pub rejected: u32` to `BatchSummary` so toast messages and batch summaries report rejected pairings (e.g. `"Adopted 2, rejected 9 wrong matches"`).

### 4.4 Matcher, URL/Slug Resolver, and Service (`crates/bcvault-core/src/library/{matcher,verify,service}.rs`)

1. **Rejected Pair Filtering in `assign_matches` and `find_relink_candidates`**:
   - Add `assign_matches_with_rejected` (or add `rejected_pairs: &HashSet<(ItemKey, String)>` parameter to `assign_matches` and `find_relink_candidates`, updating callers):
   - In `build_exact_edges_refs`: skip any `(cand, rel)` pair where `rejected_pairs.contains(&(rel.key, cand.path_key))` unless the edge has `Signal::UrlTag` (or skip unconditionally so a user's explicit rejection always wins).
   - In the fuzzy score matrix loop of `assign_matches` (and `find_relink_candidates` in `verify.rs`): if `rejected_pairs.contains(&(rel.key, cand.path_key))`, set its score to `(Score { value: 0.0, signals: Vec::new() }, false)`.
   - Consequence: once all false review pairings for `"Wall Split Action 001"` are rejected, `"Wall Split Action 001"` is no longer in `review_cand_paths` and automatically appears in `assignment.local_only`!

2. **Pure URL / Slug & Search Helpers in `matcher.rs`**:
   - `pub fn extract_release_slug(url: &str) -> Option<(ItemType, &str)>`:
     - Parses normalized or raw Bandcamp URLs (`https://artist.bandcamp.com/album/split-3`, `https://custom.example.com/track/ noise-track ?from=fanitem`) and shorthand inputs (`album/split-3`, `/album/split-3`, `track/foo`).
     - Returns `Some((ItemType::Album, "split-3"))` or `Some((ItemType::Track, "foo"))`.
   - `pub fn resolve_release_by_query(input: &str, releases: &[ReleaseRow]) -> Option<ItemKey>`:
     - Trims `input`. Returns `None` if empty.
     - **Pass 1 (Full URL)**: If `input` contains `"://"` or `".bandcamp.com"` or starts with `"http"`, normalize with `normalize_bandcamp_url(input)` and match against `normalize_bandcamp_url(&r.url)`.
     - **Pass 2 (Typed slug `album/<slug>` or `track/<slug>`)**: If `extract_release_slug(input)` returns `Some((kind, slug))`, match the release whose `r.key.kind == kind` and whose URL slug equals `slug` (case-insensitive).
     - **Pass 3 (Bare slug `<slug>`)**: Compare `input.trim_matches('/').to_lowercase()` against the trailing slug segment of `normalize_bandcamp_url(&r.url)`. If **exactly one** release of the artist has that slug, return `Some(r.key)`. (If both an `/album/foo` and `/track/foo` exist, require `album/foo` or `track/foo` to disambiguate.)
   - `pub fn filter_releases_for_picker<'a>(query: &str, releases: &'a [ReleaseRow]) -> Vec<&'a ReleaseRow>`:
     - If `query.trim()` is empty, returns all `releases` ordered by `title` (case-insensitive), then `published_at DESC`, then `key.id`.
     - If `resolve_release_by_query(query, releases)` matches a specific `key`, puts that exact release first.
     - Otherwise splits `query.to_lowercase()` into whitespace tokens and keeps every `ReleaseRow` where all query tokens appear in the concatenation of `title.to_lowercase()`, the URL slug (`album/<slug>`), the publication year, `classification.label().to_lowercase()`, or `key.id.to_string()`.
   - `pub fn format_release_picker_label(rel: &ReleaseRow) -> String`:
     - Formats a release with disambiguating slug, year, track count, and classification so identical titles like `"split"` are always distinct:
       - Extract slug from `rel.url` (e.g. `album/split-3` or `track/split`).
       - Year from `rel.published_at` (`"2023"` or `"----"`).
       - Tracks from `rel.track_count` (`"4 trk"` or `"- trk"`).
       - Returns e.g. `"split  [album/split-3 · 2023 · 4 trk · FREE]"`.

3. **Catalog Metadata on `ReconcileReport` (`library/service.rs`)**:
   - Add `pub catalog: Vec<ReleaseRow>` to `ReconcileReport` (populated from `all_releases` in `run_scan_and_match`, `rescan_and_adopt_on_sync`, and `match_artist_folder_internal`, sorted by `title COLLATE NOCASE ASC, published_at DESC`).
   - This guarantees both UIs have the complete, unfiltered release list (including `url`, `published_at`, `track_count`, `classification`, `art_id`, and `downloaded`) for `report.band_id`, even if the main window is currently filtered or viewing another artist.

### 4.5 Worker Integration (`crates/bcvault-core/src/worker/library.rs`)

1. In `rescan_and_adopt_on_sync` and `match_artist_folder_internal` (and `service::run_scan_and_match`):
   - Load `let rejected = lib_repo::get_rejected_pairs(c, band_id)?;` alongside `get_ignored_local`.
   - Pass `&rejected` to `assign_matches`.
   - When `rescan_and_adopt_on_sync` runs after a sync, `assignment.review` now excludes all previously rejected pairs, so the Match Report modal never pops up for already-rejected pairs.
2. In `apply_reconcile_internal`:
   - Handle `MatchDecision::Reject { band_id, key, path }`:
     - Compute `let path_key = crate::library::scan::normalize_candidate_path_key(&path);`.
     - Call `lib_repo::reject_match_pair(c, band_id, key, &path_key, Some(actual_batch_id))?`.
     - Increment `summary.rejected += 1;` and insert `band_id` into `touched_bands`.
   - In `MatchDecision::Adopt` and `MatchDecision::Attach`:
     - Also call `lib_repo::unreject_match_pair` so an explicit adoption clears any prior rejection for that pair.
   - After applying `MatchDecision::Reject` (or `Ignore` / `Adopt` / `Attach`), recompute `local_only_count` for the artist's available folders (or let a follow-up scan update it) and emit `ArtistStats` and `LibraryApplied`.

### 4.6 UI Specification (Classic `crates/bcvault/src/ui/modals.rs` & FX `crates/bcvault-fx/src/overlays/modals.rs`)

#### A. Review Tab (`ReportTab::Review` / `FxReportTab::Review`)

1. **Per-Item State**:
   Replace `selected_review: HashMap<ItemKey, usize>` with:
   ```rust
   #[derive(Debug, Clone, PartialEq, Eq)]
   pub enum ReviewChoice {
       Skip,
       Adopt(usize),          // index into r.candidates
       RejectAll,             // "Wrong, don't ask again" for all candidates on this ReviewItem
       RejectSet(BTreeSet<usize>), // optional per-candidate rejection when r.candidates.len() > 1
   }
   ```
   Default when not present in the map is `ReviewChoice::Skip`.

2. **Top Bulk Action Bar in Review Tab**:
   When `!modal.report.review.is_empty()`, show a horizontal bar at the top of the Review tab:
   - Button **"Mark all skipped as wrong (don't ask again)"**: sets every `ReviewItem` currently in `ReviewChoice::Skip` to `ReviewChoice::RejectAll`.
   - Button **"Skip all for now"**: resets all `ReviewChoice::RejectAll` / `Adopt` entries back to `ReviewChoice::Skip`.
   - Summary label: `"{adopted} to adopt · {rejected} marked wrong · {skipped} skipped"`.

3. **Disambiguated Review Card Header**:
   For each `ReviewItem`:
   - Look up the full `ReleaseRow` in `modal.report.catalog` by `r.release_key`.
   - Display:
     - **Title** (bold), followed by its **slug badge** (`album/<slug>` or `track/<slug>` in monospace dim text), **year**, **track count** (`4 trk`), and **classification badge**.
     - An **"Open"** button on the right of the header that opens `rel.url` in the browser (`open::that_detached(&rel.url)`), with hover tooltip showing the full URL.
   - For each candidate `c` in `r.candidates`:
     - Radio button `ReviewChoice::Adopt(c_idx)` + candidate folder name + format badge + score percentage with signal tooltip.
   - Below the candidates, two radio options side-by-side:
     - `◉ Skip for now` (`ReviewChoice::Skip`)
     - `◉ Wrong, don't ask again` (`ReviewChoice::RejectAll`) — styled in warning/error accent when selected, with hover text: *"Never propose matching this release to these folder(s) again"*.
   - When building `Vec<MatchDecision>` on **Apply selected**:
     - `ReviewChoice::Adopt(c_idx)` -> emits `MatchDecision::Adopt` for `r.candidates[c_idx]`.
     - `ReviewChoice::RejectAll` -> emits `MatchDecision::Reject { band_id: modal.report.band_id, key: r.release_key, path: c.candidate_path.clone() }` for every `c` in `r.candidates`.
   - `modal.total_selected_count()` includes the count of `ReviewItem`s set to `ReviewChoice::Adopt(_)` or `ReviewChoice::RejectAll`, so **Apply selected (N)** is enabled when rejecting wrong proposals even if nothing is adopted.

#### B. Searchable Release Picker + Slug/URL Input (Shared Widget for `Local only` and `Conflicts`)

Add per-path search query state to the modal struct:
- `pub picker_queries: HashMap<PathBuf, String>`

When a folder row is in `Attach(current_key)` mode (in **Local only** or **Conflicts**):
1. **Search / Slug / URL TextEdit**:
   - Render a single-line `TextEdit` (`desired_width(260.0)`) bound to `modal.picker_queries.entry(path.clone()).or_default()` with hint text:
     `"Search title, slug, or paste Bandcamp URL..."`
   - Whenever the text changes, call `resolve_release_by_query(q, &modal.report.catalog)`:
     - If it returns `Some(matched_key)`, automatically update the selected `Attach(matched_key)` target!
     - Otherwise, if the current `current_key` is not in `filter_releases_for_picker(q, &modal.report.catalog)` and the filtered list is non-empty, update `current_key` to the first filtered result.
2. **Direct URL/Slug Match Indicator**:
   - If `resolve_release_by_query(q, &modal.report.catalog)` is `Some(k)`, render a confirmation label (`"✓ Exact URL/slug match"` in `pal::OK` / `th.ok`).
   - If `q` looks like a URL (`q.contains("://")` or `q.contains("bandcamp.com")`) and `resolve_release_by_query` returns `None`, render a warning (`"⚠ No release on this artist page matches that URL/slug"` in `pal::WARN` / `th.warn`).
3. **Filtered Dropdown (`ComboBox`) with Disambiguated Labels**:
   - Render an `egui::ComboBox` whose selected text is `format_release_picker_label(selected_rel)` (e.g., `"split  [album/split-3 · 2023 · 4 trk · FREE]"`).
   - Inside the dropdown popup, list `filter_releases_for_picker(q, &modal.report.catalog)` (or show `"No releases match filter"` if empty).
   - Each item in the dropdown displays `format_release_picker_label(rel)` with hover tooltip showing the full `rel.url` and download status (`"[Already downloaded]"` if `rel.downloaded`).
4. **Quick Browser Verification**:
   - Next to the `ComboBox`, render a small **"Open"** button that opens the currently selected release's `url` in the browser so the user can verify the Bandcamp page in one click.

#### C. Interactive Conflict Resolution (`ReportTab::Conflicts` / `FxReportTab::Conflicts`)

Add `pub conflict_decisions: HashMap<PathBuf, LocalAction>` (or `FxLocalAction`) to the modal state:
1. For each `Conflict` card in `modal.report.conflicts`:
   - Show `c.description` at the top.
   - If `!c.release_keys.is_empty()`, list the colliding Bandcamp releases with their disambiguated labels (`format_release_picker_label`) and an **"Open"** button per release.
   - For each candidate folder `p` in `&c.candidate_paths`:
     - Show the folder name (and relative path when the artist has multiple linked folders) with track count / format if available.
     - Show the three action choices (identical to `Local only`):
       - **Leave unresolved** (`LocalAction::Leave`)
       - **Ignore in future** (`LocalAction::Ignore`)
       - **Match to release...** (`LocalAction::Attach(default_key)`, defaulting to the first key in `c.release_keys` that isn't already claimed by another folder, or the first release in `modal.report.catalog`).
     - When **Match to release...** is active, render the **Searchable Release Picker + Slug/URL Input** from §4.6.B (with the releases in `c.release_keys` listed first when the search box is empty, followed by a separator and the rest of `modal.report.catalog`).
2. Include `conflict_decisions` in `modal.total_selected_count()` and emit `MatchDecision::Ignore` or `MatchDecision::Attach` on **Apply selected**.

## 5. Tests

Write the tests alongside the code in each step:

| # | Scenario | Expected |
| --- | --- | --- |
| 1 | Migration `0008_rejected_matches`: upgrade from v7 to v8, insert and query `artist_folder_rejected`, cascade on artist/release delete, `unlink_folder` cleans up rejected rows under that folder's `path_key` only | Passes; `PRAGMA foreign_key_check` empty |
| 2 | Regression ("Wall Split Action 001" vs 9 `"split"` releases): candidate `"Wall Split Action 001"` and 9 releases titled `"split"` (`album/split-1` .. `album/split-9`). Initial `assign_matches` puts all 9 in `review` and 0 in `local_only`. Reject all 9 pairs via `reject_match_pair`, then re-run `assign_matches` | `review` is empty (`0` items); `"Wall Split Action 001"` now appears in `local_only` (`1` item) |
| 3 | Rejecting `(rel_a, folder_x)` does not block `(rel_b, folder_x)` or `(rel_a, folder_y)` | `folder_x` still matches `rel_b` and `folder_y` still matches `rel_a` |
| 4 | Undo batch after applying `MatchDecision::Reject` | `undo_batch` removes the `artist_folder_rejected` rows and restores prior state |
| 5 | Explicit `MatchDecision::Attach` or `Adopt` on a previously rejected `(key, path)` pair | Clears the rejection row and adopts/attaches the folder cleanly |
| 6 | `extract_release_slug` and `resolve_release_by_query`: full URL (`https://artist.bandcamp.com/album/split-3?from=fanitem`), custom domain URL (`https://music.example.com/track/ test-track /`), typed slug (`album/split-3`), and bare slug (`split-3`) among 10 releases titled `"split"` | Resolves deterministically to the exact `ItemKey` for `split-3` |
| 7 | Ambiguous bare slug (`album/split` and `track/split` both exist on the same artist) | Bare `"split"` returns `None` (requires `"album/split"` or `"track/split"`), while `"album/split"` resolves to the album and `"track/split"` resolves to the track |
| 8 | `filter_releases_for_picker` with 10 releases titled `"split"` (`split-1`..`split-10`, various years) | Searching `"split-7"` or `"2021"` or pasting the URL narrows the picker to the matching release(s); `format_release_picker_label` produces distinct labels for all 10 |
| 9 | Classic and FX modal state tests: selecting **"Mark all skipped as wrong (don't ask again)"** on 9 review items sets `total_selected_count() == 9` and builds 9 `MatchDecision::Reject` decisions; resolving a conflict via URL/slug builds `MatchDecision::Attach` for the resolved key | Passes |

## 6. Commit sequence

Each step must compile and pass `scripts/check.sh`:

1. `feat(db)!: add artist_folder_rejected table for rejected review pairs` (`Task: none`, `BREAKING CHANGE: database schema v8`). Adds migration `0008_rejected_matches.sql`, bumps `MAX_VERSION` to `8`, adds `reject_match_pair`, `unreject_match_pair`, `get_rejected_pairs`, `unlink_folder` cleanup, `undo_batch` support, `MatchDecision::Reject`, `BatchSummary.rejected`, and test 1 + test 4.
2. `feat(library): filter rejected pairs and resolve releases by url or slug` (`Task: none`). Threads `rejected_pairs` through `assign_matches`, `find_relink_candidates`, `run_scan_and_match`, and `worker/library.rs` (`rescan_and_adopt_on_sync`, `match_artist_folder_internal`, `apply_reconcile_internal`); adds `ReconcileReport.catalog`, `extract_release_slug`, `resolve_release_by_query`, `filter_releases_for_picker`, `format_release_picker_label`, and tests 2, 3, 5, 6, 7, 8.
3. `feat(classic): add review rejection, conflict resolution, and url/slug picker` (`Task: none`). Updates `crates/bcvault/src/ui/modals.rs` with `ReviewChoice::RejectAll`, batch "Mark all skipped as wrong (don't ask again)", disambiguated release headers with Open buttons, searchable + URL/slug release picker in `Local only`, and interactive resolution controls in `Conflicts`, plus classic state test 9.
4. `feat(fx): add review rejection, conflict resolution, and url/slug picker` (`Task: none`). Same UX additions in `crates/bcvault-fx/src/overlays/modals.rs` and `state.rs` obeying zero corner radius and UI-thread purity guards, plus FX state test 9.
5. `docs(library): document review rejection, conflict resolution, and slug matching` (`Task: none`). Updates `docs/library.md` §4.3 and `PROGRESS.md` / `ROADMAP.md`.

## 7. Verification

- `scripts/check.sh` green after every commit.
- Manual check (ask the user):
  1. Link an artist folder containing `"Wall Split Action 001"` to an artist with multiple `"split"` releases.
  2. In the Match Report **Review** tab, verify each `"split"` card shows its slug (`album/split-...`), year, track count, and **Open** button.
  3. Click **Mark all skipped as wrong (don't ask again)** and click **Apply selected**.
  4. Click **Match Folder Now** (or link another folder): verify none of the rejected `"split"` releases appear in **Review** again, and `"Wall Split Action 001"` now appears under **Local only**.
  5. In **Local only** (or **Conflicts**), choose **Attach to release...**, paste a release's Bandcamp URL or slug into the search box, and verify it immediately selects that exact release and attaches cleanly on Apply.

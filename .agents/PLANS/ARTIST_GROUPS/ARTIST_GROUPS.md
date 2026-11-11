# Plan: artist groups in the sidebar (manual and rule-based)

For the coding agent. Repo: bcvault (Rust workspace). Scope: `bcvault-core` (pure group rules and assignment, a new migration, repo, worker commands and events, an additive release-query filter), the `bcvault` and `bcvault-fx` sidebars, a group editor in each UI, and docs.

## 0. Ground rules

- **Branch.** `feat/artist-groups` from the repository default branch. Never force-push.
- **Merge order.** After `SETTINGS_TABS.md` and `DOWNLOAD_FLOW.md`; before `KEYBOARD.md`, which then covers the final sidebar row model. `AUTO_SYNC.md` also touches the sidebar and the Artists view; whichever merges second rebases. Add UI inside new functions and files, not into the existing list loops.
- **Migration.** One new migration, **numbered by landing order** (`ROADMAP.md` §5): the lowest free number at merge time. At the time of writing `0007` is reserved by `AUTO_SYNC.md` and `0008` by `ARCHIVE.md`; if this lands first it takes `0007` and you renumber those unmerged plans and the ROADMAP table in the same docs commit. Never edit a merged migration. It is breaking for older binaries (`feat(db)!`, `BREAKING CHANGE:` footer; ship both apps together).
- Conventional Commits, one commit per step in section 7, `Task: none`. Every commit compiles and passes `scripts/check.sh`. Non-test code must not use `unwrap` or `expect`.
- **Never destroy information silently** (`AGENTS.md` rule 8). Groups are user-curated data: deleting a group asks for confirmation and states how many manual placements it holds. Groups never change releases, library state or files.
- **UI-thread purity and square look** apply. Group evaluation is pure in-memory code and runs on the UI thread only over data already in state; it never touches the database.
- Read first: `types.rs` (`ArtistView`, `ArtistStats`, `ArtistFolder`, `ReleaseQuery`), `db/repo/artists.rs` (`list_views`), `db/migrations.rs`, `worker/messages.rs`, `worker/supervisor.rs`, `library/textmatch.rs` (name keys); in fx: `views/sidebar.rs` (virtualised `show_rows`, fixed 28 px rows), `views/artists.rs`, `state.rs`, `actions.rs`, `services/search.rs`, `db/mod.rs` (schema contract check); in classic: `ui/sidebar.rs` (virtualised, fixed 26 px rows, multi-selection `selected_artists`), `ui/state.rs`, `ui/modals.rs`. Line numbers come from a snapshot taken before `MULTI_FOLDER` landed (folders are a `Vec` per artist now), so grep.

## 1. Current state (verified by reading the code)

- The sidebar is one flat, virtualised list: a row 0 "All artists" followed by artists in the order of `state.artists`, with a text filter box. Rows have a fixed height, which makes a flattened row model easy.
- `ArtistView` has `band_id`, `slug`, `name`, `custom_name`, `url`, `last_synced_at` and `stats`. `ArtistStats` has `downloaded`, `eligible_downloaded`, `non_eligible_downloaded`, `eligible`, `total`, `email_required`, `unknown`, `quota_used_30d`, `archived`, `missing_files`. `artists` has no tags, country or genre columns (the archive plan may add release tags later; not used here). Linked folders and their state are in `state.folders` in both UIs.
- Classic supports sidebar multi-selection (`selected_artists`), but the release query only uses it when exactly one artist is selected (`active_band_id`); with several selected, the table still shows every artist. `ReleaseQuery` has a single `band_id: Option<i64>`.
- `SyncScope::Selected(Vec<i64>)` already exists, so "sync this group" needs no core work. `RemoveArtist` deletes the artist row and cascades.
- fx keeps UI-only data in `fx.sqlite`; the classic app has no UI preference store. Group data must be visible to both apps, so it belongs in the main database.
- Sidebar rows are painted by hand in both UIs (custom `draw_artist_row`), so headers are new painted rows, not egui collapsing headers.

## 2. Goals and non-goals

Goals:

- Sidebar **groups**: named, ordered, collapsible sections. A group is either **manual** (the user puts artists in it) or **automatic** (rules decide), and an artist can also be pinned by hand into any group, overriding rules.
- A group editor with a live preview of who matches; presets for common rules.
- Group actions: sync the group, download remaining for the group (uses `DOWNLOAD_FLOW.md`), select all its artists, view its releases.
- With no groups defined the sidebar is **identical to today**.

Non-goals: nested groups, an artist in several groups at once, groups on release rows, tags fetched from Bandcamp, importing or exporting groups, rules on release-level data, per-group sync schedules, themes or colours per group.

## 3. Specification

### 3.1 Model (new pure module `bcvault-core/src/groups.rs`)

```rust
pub struct ArtistGroup { pub id: i64, pub name: String, pub pos: i32, pub collapsed: bool, pub rule: Option<GroupRule> }
pub struct GroupRule { pub mode: MatchMode /* All | Any */, pub conditions: Vec<Condition> }
pub struct Condition { pub negate: bool, pub kind: ConditionKind }
pub enum ConditionKind {
    NameContains(String), NameStartsWith(String),
    NameInitial { from: char, to: char },   // 'a'..'f'; '#' = starts with a digit or symbol
    Downloads(DownloadState),               // Complete | Partial | NoneYet | NothingEligible | HasRemaining
    Library(LibraryState),                  // HasFolder | NoFolder | FolderProblem | HasMissingFiles | HasArchived
    SyncedOlderThanDays(u32), NeverSynced,
    TotalReleases { at_least: Option<u32>, at_most: Option<u32> },
    CustomDomain(bool),                     // url host is not *.bandcamp.com
}
```

- Name conditions compare with `library::textmatch::key` (case and accent insensitive, same as the library matcher) against `display_name()`.
- `DownloadState` is computed from `ArtistStats` only: `Complete` = `eligible > 0 && eligible_downloaded >= eligible`; `Partial` = `0 < eligible_downloaded < eligible`; `NoneYet` = `eligible > 0 && eligible_downloaded == 0`; `NothingEligible` = `eligible == 0`; `HasRemaining` = `eligible_downloaded < eligible`. `LibraryState` uses the artist's folders (`FolderProblem` = any folder not `Ok`) and `ArtistStats`.
- **An empty rule (no conditions) matches nobody.** A rule with more than 8 conditions, a string longer than 64 characters or a `from > to` initial range is invalid (typed `GroupError`).
- Evaluation input: `ArtistFacts { view: &ArtistView, folders: &[ArtistFolder] }` and `now: Ts`.
- **Placement** (exclusive, deterministic) by `assign(artists, groups, pins, folders, now) -> Assignment`:
  1. If the artist has a **pin**: `Some(g)` puts it in group `g`; `None` ("pinned ungrouped") keeps it out of every group. A pin to a group that no longer exists is ignored.
  2. Otherwise the first group in `pos` order whose rule matches.
  3. Otherwise **Ungrouped**.
  Within a group, artists keep the order of the input slice (the order the sidebar already uses).
- `Assignment { groups: Vec<GroupMembers>, ungrouped: Vec<i64> }`, and `build_sidebar_rows(&Assignment, &[ArtistGroup], filter: &str, ...) -> Vec<SidebarRow>` with `SidebarRow { AllArtists, GroupHeader { id, name, count, collapsed }, UngroupedHeader { count, collapsed }, Artist { band_id } }`. With no groups, it returns exactly `AllArtists` plus the artists (no `Ungrouped` header). With at least one group, `Ungrouped` is shown only when it is not empty. A non-empty filter hides groups without matches and shows matching groups expanded without changing stored `collapsed`.
- Validation: name trimmed, 1 to 40 characters, unique case-insensitively; at most 64 groups.

### 3.2 Storage (new migration)

```sql
CREATE TABLE artist_groups (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    name      TEXT    NOT NULL COLLATE NOCASE UNIQUE,
    pos       INTEGER NOT NULL,
    collapsed INTEGER NOT NULL DEFAULT 0 CHECK (collapsed IN (0,1)),
    rule_json TEXT
) STRICT;

CREATE TABLE artist_group_pins (
    band_id  INTEGER PRIMARY KEY REFERENCES artists(band_id)   ON DELETE CASCADE,
    group_id INTEGER          REFERENCES artist_groups(id)     ON DELETE CASCADE  -- NULL = pinned ungrouped
) STRICT;
```

`band_id` as primary key makes "one pin per artist" a schema fact. Deleting a group deletes its pins, so those artists fall back to rules or Ungrouped. `rule_json` is the serde form of `GroupRule` with a `"v": 1` field; an unreadable or invalid value loads as `rule = None` plus a warning log, never an error that blocks startup. Register the file in `db/migrations.rs`; check whether fx's schema contract check lists main-DB tables it reads (it should not need these, because groups reach fx through core events), and update it only if needed.

### 3.3 Repo, commands and events

- Repo functions with the validations of §3.1: `list_groups`, `list_pins`, `create_group`, `update_group` (name and rule), `delete_group`, `reorder_groups(&[i64])`, `set_collapsed`, `set_pins(&[i64], PinTarget)`.
- Commands: `CreateArtistGroup { name, rule }`, `UpdateArtistGroup { id, name, rule }`, `DeleteArtistGroup { id }`, `ReorderArtistGroups { ids }`, `SetArtistGroupCollapsed { id, collapsed }`, `PinArtists { band_ids, target }` with `PinTarget { Group(i64), Ungrouped, Auto }` (`Auto` removes the pin).
- Events: `Ready` carries `groups` and `pins`; every write answers with `ArtistGroupsChanged { groups, pins }` (a full snapshot; both lists are small). Errors come back as a `LogLine` warning and an `ArtistGroupError { message }` event the editor can show inline.

### 3.4 Sidebar behaviour (both UIs)

- State per UI: `groups`, `pins`, a cached `Assignment` and `Vec<SidebarRow>`, recomputed **at most once per frame and only when dirty**: on artists, stats, folders, groups, pins or filter changes, and once a minute for age-based rules. Stats events are frequent during syncs; the recompute must be linear in artists times groups and allocation-light. Add a bench-style test with 10 000 artists and 20 groups asserting it stays well under a frame (generous bound, not a flaky microbenchmark).
- Rows keep the fixed height. A **group header** shows a chevron, the name, the member count and (muted) the downloaded and eligible totals of its members; click toggles `collapsed` (persisted through `SetArtistGroupCollapsed`). The `Ungrouped` header behaves the same but is not editable.
- **Header context menu:** Rename..., Edit rule..., Sync group, Download remaining for group (after `DOWNLOAD_FLOW.md`; a `RemainingScope::Artists(members)`), Select artists in group, View releases of group, Move up, Move down, Delete group... (confirm; shows pinned count).
- **Artist context menu additions:** Move to group > (each group, "Ungrouped", "Automatic (remove manual placement)"), New group from selection...
- **Selection scope:** with several artists selected (classic today; fx gains artist multi-select only if it already exists, otherwise act on the single selected artist), the same menu applies to all of them.
- **Drag and drop** (step 9, optional): dragging artist rows onto a group header pins them there, using egui's drag and drop payload API. It is a mouse convenience only; the menu path above is the accessible one.
- Palette (fx) and menu actions are all `FxAction`s so `KEYBOARD.md` can bind them: `NewGroup`, `EditGroup(id)`, `DeleteGroup(id)`, `ToggleGroup(id)`, `CollapseAllGroups`, `ExpandAllGroups`, `MoveArtistsToGroup(ids, PinTarget)`, `SyncGroup(id)`, `DownloadRemainingGroup(id)`, `ViewGroup(id)`.

### 3.5 Group editor (modal, both UIs)

- Fields: Name; kind selector **Manual** or **Automatic**. Manual has no rule (members come only from pins). Automatic shows **Match all / any** and a list of condition rows (kind combo, value editor, a `not` checkbox, remove), `+ Add condition` (max 8), and a presets menu: *Starts with A to F*, *Fully downloaded*, *Has remaining releases*, *No linked folder*, *Folder problems*, *Not synced in 30 days*, *Custom domain*.
- **Live preview** line: `Matches 37 artists (5 are placed elsewhere by hand)`, plus the first 8 names, computed with the pure `assign` over current state; no I/O.
- Help text: "Artists you move by hand always stay where you put them. Groups higher in the list win when several rules match."
- Validation errors from §3.1 shown inline; Save disabled while invalid.

### 3.6 Viewing a group's releases (additive core change; step 8)

Add `band_ids: Vec<i64>` to `ReleaseQuery` (`#[serde(default)]`; empty means no restriction; when both `band_id` and `band_ids` are set both must hold, so saved views and old code keep working). SQL: `r.band_id IN (SELECT value FROM json_each(?))` with one JSON-array parameter (no parameter-count limit; JSON functions are available in the bundled SQLite; verify). The sidebar uses it for **View releases of group** and, in classic, to make sidebar multi-selection actually filter the table. fx stores it in `FxQuery.base`, so navigation history and saved views include it.

## 4. Tests

| # | Area | Scenario | Expected |
| --- | --- | --- | --- |
| 1 | groups | Each `ConditionKind` with matching, non-matching and negated artists (stats and folders fixtures) | As §3.1 |
| 2 | groups | Empty rule, 9 conditions, 65-character string, reversed initial range | Typed `GroupError`; empty rule matches nobody |
| 3 | groups | Precedence: pin beats rules; `None` pin beats rules; first matching group in `pos` order wins; pin to a deleted group ignored | Deterministic `Assignment` |
| 4 | groups | `build_sidebar_rows` with no groups, with groups and an empty Ungrouped, with a filter, with collapsed groups | No groups gives exactly today's rows; filter expands matches without changing `collapsed` |
| 5 | groups | Name matching with accents and case (`Émilie` vs `emilie`) | Same key as the library matcher |
| 6 | groups | 10 000 artists, 20 groups with rules | Completes under the agreed bound |
| 7 | db | Migration on a populated database | Existing data untouched; tables created; `foreign_key_check` clean |
| 8 | db | Cascades: delete group removes pins; delete artist removes its pin; duplicate name differing only by case | As §3.2 |
| 9 | db | Bad `rule_json` | Loads with `rule = None` and a warning, no panic |
| 10 | repo | Validation (name length, uniqueness, 64-group cap), reorder, collapsed, `set_pins` for all three targets | As §3.1 and §3.3 |
| 11 | worker | Each command answers with `ArtistGroupsChanged`; `Ready` includes groups and pins; errors surface as `ArtistGroupError` | As §3.3 |
| 12 | worker | `RemoveArtist` for a pinned artist | Pin gone; snapshot event consistent |
| 13 | query | `band_ids` alone, with `band_id`, empty, 5 000 ids | Correct rows; no parameter-limit failure; saved-view JSON without the field still deserialises |
| 14 | ui state (both) | Dirty-flag recompute (counter), collapse persistence command, filter behaviour, editor preview, menu actions send the right commands | As §3.4 and §3.5 |
| 15 | ui state (both) | Editor validation and Save gating | Save disabled while invalid |

## 5. Decisions made for you (change only through `AGENTS.md` §8)

- **Exclusive placement**: an artist appears in exactly one section. (A tag-like model with duplicates was rejected for the sidebar; "View releases of group" gives the collection view.)
- Manual pins always beat rules; an artist can be pinned to Ungrouped to escape every rule.
- No sidebar "groups on/off" toggle; with no groups nothing changes, and collapsing hides content.
- Groups live in the main database so both apps share them; `collapsed` is stored there too.
- Rule set is intentionally small and uses only data the app already has.

## 6. Interaction with other plans

- `DOWNLOAD_FLOW.md`: group download uses `RemainingScope::Artists`.
- `AUTO_SYNC.md`: its per-artist "excluded from scheduled sync" marker is drawn by the artist-row renderer; a group sync is a user action and does not override an exclusion unless that plan says so (check its rules when implementing `SyncGroup`).
- `KEYBOARD.md`: the sidebar row model of §3.1 is what keyboard navigation walks; headers support Left, Right and Enter to collapse and expand.
- `SETTINGS_TABS.md`: no dependency.

## 7. Commit sequence

One commit per step, every commit green.

1. `feat(core): add group rules, artist placement and sidebar rows` (pure module, tests 1 to 6).
2. `feat(db)!: add artist groups and manual placements` (migration, repo, validation, tests 7 to 10; `BREAKING CHANGE:` footer).
3. `feat(worker): manage artist groups` (commands, events, `Ready`, tests 11 and 12).
4. `feat(classic/sidebar): render groups with a menu for moving artists` (rows, headers, menus, dirty-flag cache, test 14).
5. `feat(classic/groups): add the group editor` (§3.5, test 15).
6. `feat(fx/sidebar): render groups with actions and palette commands`.
7. `feat(fx/groups): add the group editor`.
8. `feat(core): filter releases by several artists` (§3.6, test 13) and `feat(classic): view the releases of a group` and `feat(fx): view the releases of a group` as separate commits if the diff exceeds the size guideline.
9. `feat(fx,classic): drag artists onto group headers` (optional; skip if it risks the schedule and record it in `PROGRESS.md`).
10. `docs(groups): document groups and rules` (README, `docs/groups.md` rule reference, ROADMAP migration row, `PROGRESS.md`).

## 8. Definition of done

- `scripts/check.sh` green; with zero groups the sidebar renders exactly the rows of the previous release (test 4).
- Create a manual group, an automatic group with two conditions and a "not synced in 30 days" group; move artists by menu; restart both apps and see the same groups, order and collapsed state; delete a group and see its artists fall back.
- **Manual (ask the user):** with the user's own artist list, build the groups they want (for example "A to F", "Fully downloaded", "Has remaining") and confirm the live preview matches what they expect.
- `PROGRESS.md` updated in every commit; tag per `AGENTS.md` rule 13 when released; both apps ship together.

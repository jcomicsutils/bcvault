# Plan: ask the user when a whole linked folder is gone

For the coding agent. Repo: bcvault (Rust workspace). Scope: `crates/bcvault-core/src/worker/{library,sync,supervisor,messages}.rs`, `types.rs`, `library/verify.rs` (only if needed), both UIs (state, dock, sidebar, one new dialog each), and docs. Place this file at `.agents/LIBRARY_SYNC/MISSING-FOLDER-FIX.md`.

> Status: not started. Steps are `Task: none` commits (step numbers go in `PROGRESS.md`), like `MATCH-FIX.md`.

## 0. Ground rules

- Follow `AGENTS.md`: Conventional Commits with a scope from §5.2 and a `Task: none` trailer, one commit per step in section 6, `scripts/check.sh` before each commit, no `unwrap`/`expect` outside tests, UI-thread purity, square look.
- Library sync is local-only I/O and never modifies or deletes user files. Nothing here writes to user folders.
- **No migration.** No new column, no new `local_status` value (the CHECK constraint stays as it is). Do not edit merged migrations.
- Keep these shapes unchanged: `MissingFound`, `MissingItem`, `MissingDecision`, `MissingAction`, `ResolveMissing`, `RememberChoice`, `VerifyOutcome` (adding fields to `VerifyOutcome` is allowed, removing is not).
- Line numbers below come from a snapshot. Grep, do not trust them.
- Read before editing: `worker/library.rs` (`verify_files`, `resolve_missing`, `repoint_artist_folder`, `rescan_and_adopt_on_sync`, `unlink_artist_folder`), `library/verify.rs` (`verify_single_release`), `worker/sync.rs` (the `verify_files(..., true, ...)` call), `worker/supervisor.rs` (`VerifyFiles`, `ResolveMissing`), `worker/messages.rs`, `db/repo/library.rs` (`archive`, `unmark`, `unlink_folder`, `probe_folder_state`), the Resolve Missing dialog and `MissingFound` handling in `crates/bcvault/src/ui/{modals,state,bottom_dock,sidebar}.rs` and `crates/bcvault-fx/src/{state.rs,overlays/modals.rs,views/sidebar.rs,views/dock.rs}`.
- If you find something that makes a rule below unworkable (for example a sync path that cannot know whether the user started it), stop and report instead of improvising.

## 1. Problem

Reported: an artist's releases were downloaded, the user archived the files elsewhere and deleted the artist's whole linked folder. Settings were `When Files Missing = Ask`. Neither **Verify Local Library Files** nor syncing the artist asked anything. The releases still show as downloaded, and the sidebar shows the folder glyph with `Status: Missing`.

Cause, read from the code:

1. `verify_files` probes the artist's linked folder (`probe_folder`). If it is `Missing` or `Unreadable`, `verify_single_release` returns `Unknown` for every release of that artist. This is the unmounted-drive safeguard and must stay.
2. `Unknown` results are only logged (`Folder unavailable for artist ...` and the final `N unknown` line). No event is emitted and nothing is persisted, so the user gets no prompt and no way to say "I deleted it on purpose".
3. An existing test asserts exactly this (search `Because folder is unavailable, outcome is Unknown, NEVER Missing`). The safeguard is right; the silence is the bug.
4. Related defect found while reading: step 3 of `verify_files` builds one probe result per artist (`folder_probe_ok: HashMap<band_id, bool>`) and applies it to every downloaded release of that artist, including releases whose `local_path` is under `out_dir` and not under the linked folder. A deleted linked folder therefore also hides real missing albums stored elsewhere.

## 2. Goals and non-goals

Goals:

- When a linked folder (or `out_dir`) is unavailable, **ask the user** whether the drive is unmounted, the folder was moved, or it was deleted on purpose.
- Nothing is ever applied without an answer, in every mode, including `on_missing = Auto`.
- Releases outside the unavailable folder are still verified normally.
- Answering "deleted on purpose" applies the user's chosen policy (archive or unmark) to the affected releases as one undoable batch.

Non-goals:

- No auto-detection of "unplugged versus deleted". The app cannot know; the user says.
- No persisted "skipped" state (no migration). Dismissals live in memory for the session.
- No change to album-level missing handling (`MissingFound`, `ResolveMissing`, auto-relink, mass-missing guard).
- No multi-folder links (a later plan generalises the per-release attribution of §3.2).
- Releases stored at paths that are under neither the linked folder nor `out_dir` keep the current behaviour (probed against `out_dir`). Known limit.

## 3. Specification

### 3.1 Types (`types.rs`) and messages (`worker/messages.rs`)

```rust
pub enum UnavailableRoot { Linked { band_id: i64 }, OutDir }

pub struct UnavailableRelease {
    pub key: ItemKey,
    pub band_id: i64,
    pub artist_name: String,
    pub title: String,
    pub last_known_path: Option<PathBuf>,
}

pub struct UnavailableFolder {
    pub root: UnavailableRoot,
    pub path: PathBuf,          // linked folder, or out_dir
    pub state: FolderState,     // Missing | Unreadable (never Ok)
    pub releases: Vec<UnavailableRelease>,
}

pub enum UnavailableAction {
    /// Drive unplugged, will reconnect: change nothing.
    Keep,
    /// Deleted on purpose: apply `policy` to `keys`; optionally unlink the folder.
    Deleted { policy: DeletedPolicy, unlink: bool },   // `unlink` is ignored for OutDir
}

pub struct UnavailableDecision {
    pub root: UnavailableRoot,
    pub keys: Vec<ItemKey>,     // exactly what the user saw
    pub action: UnavailableAction,
}

// UiToWorkerCommand
ResolveUnavailable { batch_id: u64, decisions: Vec<UnavailableDecision> },
// WorkerToUiEvent
FolderUnavailable { batch_id: u64, folders: Vec<UnavailableFolder>, prompt: bool },
```

"I moved the folder" needs no new command: the dialog reuses `RepointArtistFolder`, which already relinks and re-verifies (`repoint_artist_folder` calls `verify_files` at its end). It applies to `Linked` roots only.

### 3.2 Verify changes (`worker/library.rs`)

1. **Trigger instead of a bool.** Replace `is_sync: bool` in `verify_files` with
   ```rust
   pub enum VerifyTrigger { Manual, SyncInteractive, SyncBackground }
   // is_sync() = !Manual        prompts() = !SyncBackground
   ```
   `VerifyFiles` command and `repoint_artist_folder` use `Manual`. `sync.rs` passes `SyncInteractive` when the user started the sync for selected artists (`SyncScope::Selected`) and `SyncBackground` for `SyncScope::All` and any scheduled or automatic sync (the auto-sync plan will rely on this). If `sync.rs` cannot see the scope at that call, thread it through the sync job context. Update the existing test call sites mechanically (`false` becomes `VerifyTrigger::Manual`).
2. **Per-release root attribution.** Replace `folder_probe_ok` with a function `root_for(rel, linked: Option<&ArtistFolder>, out_dir_state) -> Root`:
   - `local_path` under the linked folder (compare with `normalize_path_key`, `==` or prefix plus `/`): root is that link, state is the link's state.
   - `local_path` present and not under the link: root is `OutDir`, state is the probed `out_dir` state.
   - `local_path` is `None`: root is the link if the artist has one, else `OutDir` (current behaviour).
   Probe each folder once per run. Emit `FolderStatus` for each probed linked folder so the sidebar glyph is fresh after a manual verify (today only the rescan path emits it).
3. **Unknown handling.** After `verify_single_release`, take the `Unknown` releases, drop `local_status == Archived` ones (already archived, nothing to decide), and group the rest by root into `Vec<UnavailableFolder>`. Put the grouping in a pure function so it is unit-testable. `Unknown` releases are not counted in the mass-missing ratio (already true; keep a test).
4. **Emit.** If the grouping is non-empty, create a batch (`create_batch(c, "verify", ...)`, as the `MissingFound` path does) and emit `FolderUnavailable { batch_id, folders, prompt: trigger.prompts() && !acked }` where `acked` is true when every folder in the list is in the session set `acked_unavailable: Mutex<HashSet<(String /*root key*/, ...)>>` on `LibraryService` and the trigger is not `Manual`. A `Manual` trigger always sets `prompt: true` and clears the acks for those roots. Never write to the DB at this point.
5. **`on_missing` does not apply.** Even with `on_missing = Auto`, unavailable roots are never auto-resolved. Keep the existing `Auto` behaviour for ordinary missing albums unchanged.
6. **Recovery.** A root that probes `Ok` again is removed from `acked_unavailable`, and its releases verify normally in that run.
7. Releases whose root is available keep going through the existing present / missing / auto-relink / `on_missing` path untouched.

### 3.3 Resolve (`resolve_unavailable`, called from `supervisor.rs`)

One writer transaction per decision group, all under the event's `batch_id`:

- `Keep`: add the root to `acked_unavailable`; write nothing.
- `Deleted { policy: KeepMarked }`: `lib_repo::archive(c, &keys, batch_id)`.
- `Deleted { policy: MarkNotDownloaded }`: `lib_repo::unmark(c, &keys, batch_id)`.
- `unlink == true` and `root == Linked`: run the same code path as `UnlinkArtistFolder` (`unlink_artist_folder`), which removes only the link row and ignore rows and emits `FolderStatus { folder: None }`. Marks and `local_path` values are untouched.
- Only act on keys that are still `downloaded = 1` and not already archived at apply time; skip the rest silently.
- Afterwards emit `ReleaseUpdated` per row, `ArtistStats` per touched artist and `LibraryApplied { batch_id, summary }`, exactly as `resolve_missing` does, so the existing **Undo** toast works. Archive and unmark are undoable through the batch; unlinking is not (re-link is one click, marks were never touched). Say so in the dialog.

### 3.4 UI (classic first, then FX; same flows, FX restyled)

State: `unavailable_modal: Option<UnavailableModalState>` with one card per `UnavailableFolder` (per-card choice, policy combo defaulting to `settings.deleted_policy`, `unlink` checkbox defaulting to **on** for `Linked`). `FolderUnavailable` handling: if `prompt` is false do nothing (the chip below covers it); if true, replace the open dialog and show it. Non-modal window, like Resolve Missing.

Dialog, per card:

- Title: `<Artist>: linked folder unavailable` (or `Download folder unavailable`).
- Text by state: `Missing` is "This folder no longer exists." `Unreadable` is "This folder or its drive can't be read." Then the path and "N downloaded releases were last seen here. Their files can't be checked."
- Three choices:
  1. **The drive is disconnected, I'll reconnect it.** Button "Keep as is". Sends `Keep`. Nothing changes.
  2. **I moved the folder.** Button "Choose new folder...". Uses `rfd::AsyncFileDialog` from a spawned task, then sends the existing `RepointArtistFolder`. Linked roots only; for `OutDir` the button is absent (the user changes the download folder in Settings).
  3. **I deleted it on purpose.** Combo "Then: Keep marked as downloaded (archived) / Mark as not downloaded", checkbox "Also unlink this folder" (Linked only), button "Apply".
- Footer: `Close` (same as Keep for all unanswered cards, for this session).
- Expandable list of the releases in the card (title and last known path).

Sidebar and dock:

- Sidebar artist context menu: when the linked folder state is not `Ok`, add **Resolve unavailable folder...**, which sends `VerifyFiles { scope: Selected([band_id]) }` (manual, so it prompts).
- Dock: a warning chip `FOLDER n` (next to `MISSING n`) where n = artists whose entry in `state.folders` has `state != Ok`. It is derived from state, needs no new event, and clears when the glyph returns to `Ok`. Clicking it sends `VerifyFiles` for those artists.
- Tooltips on the sidebar glyph: for `Missing` add "Right-click to resolve".

Release table: no new status. Rows whose artist's folder is not `Ok` get a derived `FOLDER MISSING` chip in the warning colour (UI-side only, computed from `state.folders` and `local_path` prefix) so they stop looking healthy.

### 3.5 Settings wording

In both settings modals, the `If Deleted` row gets the hover text and label suffix "(used when handled automatically, and as the default in the dialog)". The `When Files Missing` hover text says that an unavailable folder always asks. No setting changes.

## 4. Tests

Write the regression first and confirm it fails on the current code.

| # | Scenario | Expected |
| --- | --- | --- |
| 1 | Regression: artist with a linked folder and 3 adopted releases; delete the folder; `verify_files(Manual)`; `on_missing = Ask` | One `FolderUnavailable` with `prompt: true`, root `Linked`, state `Missing`, 3 releases; no `MissingFound`; DB rows unchanged |
| 2 | Same with `on_missing = Auto`, `KeepMarked` | Same event; rows still `present`/unchanged; never auto-archived |
| 3 | `SyncInteractive` | Event with `prompt: true` |
| 4 | `SyncBackground` | Event with `prompt: false`, nothing else |
| 5 | After `Keep` (ack), `SyncInteractive` again | `prompt: false`; then `Manual` verify prompts again |
| 6 | Folder recreated with the album present | No event; release `present`; ack cleared |
| 7 | Mixed artist: one release under the deleted linked folder, one under `out_dir` that is deleted | Linked one in `FolderUnavailable`; the `out_dir` one in `MissingFound` |
| 8 | Archived releases under the unavailable folder | Not listed |
| 9 | `out_dir` itself missing, unlinked artists | `FolderUnavailable` with root `OutDir`, releases of all affected artists |
| 10 | `Unreadable` state (`#[cfg(unix)]`, chmod 000) and a pure grouping test with a given `FolderState` | Card state `Unreadable`; never `Missing` for the releases |
| 11 | `ResolveUnavailable` `Deleted { KeepMarked, unlink: false }` | Releases `archived`, `downloaded = 1`; link kept; one batch |
| 12 | `Deleted { MarkNotDownloaded, unlink: true }` | `downloaded = 0`, fields cleared; link removed; `FolderStatus { folder: None }`; other artists untouched |
| 13 | `UndoBatch` after 11 and 12 | Release rows restored; `assert_library_invariants` passes |
| 14 | Stale keys (release already archived or re-downloaded between event and answer) | Skipped without error |
| 15 | Mass-missing ratio with many `Unknown` rows | Guard does not trigger |
| 16 | Existing "unavailable folder" test | Updated to expect `FolderUnavailable` and no `MissingFound`, status unchanged |
| 17 | Classic and FX `state.rs`: `prompt: false` does not open the dialog; `prompt: true` replaces it; chip count follows `folders` | Pure state tests |

Also run the full workspace tests and grep `tests/sync_pipeline.rs` for any dependence on `verify_files` signatures or on silent `Unknown`.

## 5. Decisions made (veto any; flag if impossible)

- **Always ask for an unavailable root; never auto-resolve**, including `on_missing = Auto`. An unmounted drive must never become "deleted" without an answer.
- **Sync of all artists and any scheduled sync do not open the dialog**; they leave the `FOLDER n` chip. Sync of selected artists and manual verify do open it.
- **"Also unlink" defaults to on** when answering "deleted on purpose", because a link to a deleted folder only leaves a permanent warning glyph. Marks are untouched either way.
- **Dismissal is in memory only**; after a restart a manual verify asks again.
- **No new `local_status`.** Releases under an unavailable folder keep their stored status; the table chip is derived.
- Albums at paths under neither the link nor `out_dir` keep the old probing (known limit; multi-folder work revisits it).

## 6. Commit sequence

Each step compiles with all tests green.

1. `fix(library): report unavailable folders instead of staying silent` adds the types and messages (§3.1), `VerifyTrigger`, per-release root attribution, grouping, `FolderUnavailable` emission and `FolderStatus` on verify (§3.2), and tests 1 to 10, 15 and 16. Includes the regression written first.
2. `feat(worker): resolve unavailable folders as one undoable batch` adds `ResolveUnavailable`, `resolve_unavailable`, the supervisor arm and tests 11 to 14.
3. `feat(classic): ask what happened when a linked folder is gone` adds the dialog, `FOLDER n` chip, sidebar menu item, table chip and tooltips (§3.4), plus test 17 for the classic state.
4. `feat(fx): ask what happened when a linked folder is gone` is the same for `bcvault-fx`.
5. `docs(library): document unavailable folder handling` rewrites the `Drive & Volume Probing` and `Mass-Missing Guard` sections of `docs/library.md` (an unavailable folder always asks; releases elsewhere are still verified), applies §3.5, and adds this plan to the `AGENTS.md` §1 table and the `ROADMAP.md` status snapshot. Update `PROGRESS.md`.

## 7. Definition of done

- `scripts/check.sh` passes.
- The regression fails before step 1 and passes after.
- Manual check (ask the user to perform it): link a real artist folder, adopt albums, delete the folder, click **Verify Local Library Files**. The dialog appears. Choosing "deleted on purpose" with `Keep marked` archives the albums, the sidebar glyph disappears when "Also unlink" is ticked, and **Undo** restores them. Repeat with "Sync artist" for a selected artist.
- No path writes to the DB when the user has not answered.
- Old settings and databases load unchanged (no migration).

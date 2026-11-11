# Plan: relink and adopt albums moved from the download directory on verify

For the coding agent. Repo: bcvault (Rust workspace). Scope: `crates/bcvault-core/src/types.rs`, `crates/bcvault-core/src/worker/library.rs`, `crates/bcvault/src/ui/modals.rs`, `crates/bcvault-fx/src/overlays/modals.rs`, and docs. Place this file at `.agents/PLANS/LIBRARY_SYNC/VERIFY-DOWNLOAD-DIR-FIX.md`.

> Status 11 Nov 2026: **implemented** on `main` (`Task: none` commits). Steps are `Task: none` commits (step numbers go in `PROGRESS.md`), like `DOWNLOAD-DIR-FIX.md` and `MISSING-FOLDER-FIX.md`. Amends `LIBRARY_SYNC_PLAN.md` §6, `MISSING-FOLDER-FIX.md` §3, and `DOWNLOAD-DIR-FIX.md` §3.

## 0. Ground rules

- Follow `AGENTS.md`: Conventional Commits with a valid scope from §5.2 and a `Task: none` trailer, one commit per step in §4, `scripts/check.sh` green before every commit, `#![forbid(unsafe_code)]`, no `unwrap()`/`expect()`/`panic!` outside tests, strict UI-thread purity, and zero corner radius (`CornerRadius::ZERO`).
- Library sync is local-only I/O and never modifies or deletes user files.
- **No migration.** Schema version stays at `8`.

## 1. Problem

Reported user workflow:

1. An artist folder is linked with 5 existing local albums that are not on Bandcamp (`0 (+5)/6/6`).
2. The user queues all 6 Bandcamp releases for download through the app. They download into `out_dir`.
3. The user moves all 6 downloaded album folders from `out_dir` into the artist's linked folder (removing or emptying `out_dir` in the process) and clicks **Verify files**, expecting the app to detect the 6 albums in the linked folder.
4. **Bug A**: Instead of relinking the 6 albums to the linked folder, **Verify files** opens the **Resolve Unavailable Folder** dialog saying `Download folder unavailable: This folder no longer exists`, offering only two options (*"The drive is disconnected, I'll reconnect it"* and *"I deleted it on purpose"*) with no *"Moved to linked folder"* option.
5. **Bug B**: When the user selects *"I deleted it on purpose"* (marking the releases as deleted/archived or not downloaded) and presses **Verify files** again, nothing happens: the 6 albums sitting in the linked folder are still not matched or adopted until the user unlinks and re-links the folder.

## 2. Causes found in the code

1. **`unknown_rels` under `UnavailableRoot::OutDir` bypass auto-relink even when the artist has a reachable (`Ok`) linked folder.**
   In `LibraryService::verify_files`, when `out_dir` no longer exists (`out_dir_state != FolderState::Ok`), `attribute_release_root` attributes every release whose `local_path` was inside `out_dir` to `UnavailableRoot::OutDir` with `is_ok = false`, so `verify_single_release` returns `VerifyResult::Unknown`. Step 8 (`Auto-relink moved albums`) only runs on `missing_rels` and skips `unknown_rels`. Even though the artist's linked folder is `FolderState::Ok` and contains the moved albums, `verify_files` never scans the linked folder and jumps straight to emitting `FolderUnavailable` for `OutDir`.
2. **Empty `UnavailableRoot::OutDir` card pushed unconditionally on `SyncScope::All`.**
   Lines 1268–1280 of `crates/bcvault-core/src/worker/library.rs` push `UnavailableFolder { root: UnavailableRoot::OutDir, releases: Vec::new() }` whenever `out_dir_state != FolderState::Ok && is_all_scope`. While an empty linked folder (`UnavailableRoot::Linked`) needs a card so the user can unlink or repoint the dead link, `OutDir` cannot be unlinked or repointed from that modal. Pushing a 0-release `OutDir` card prompts the user even when zero active downloads remain in `out_dir`.
3. **`Resolve Unavailable Folder` has no *"Moved to linked folder"* option for `OutDir`.**
   In `show_unavailable_modal` (both Classic and FX), Choice 2 (*"I moved the folder"*) is rendered only for `UnavailableRoot::Linked`. When `UnavailableRoot::OutDir` is shown (e.g., if `auto_relink` is disabled or a moved folder was renamed below `auto_match_min`), the user has no button to scan and match against the artist's linked folder.
4. **Manual `VerifyFiles` never rescans linked folders to adopt or restore albums, and `rescan_and_adopt_on_sync` does not auto-restore `restorable` matches.**
   - `verify_files` only inspects `downloaded = 1` rows at their old `local_path`. When the user marks missing `out_dir` releases as `MarkNotDownloaded` (`downloaded = 0`) or `KeepMarked` (`local_status = 'archived'` with `local_path` still pointing at the deleted `out_dir` path) and presses **Verify files** again, `downloaded = 0` rows are ignored and `archived` rows only check whether the old `out_dir` path reappeared. Unlike `Sync` (which calls `verify_files` followed by `rescan_and_adopt_on_sync`), manual `VerifyFiles` never runs `rescan_and_adopt_on_sync` on `Ok` linked folders in scope.
   - Inside `rescan_and_adopt_on_sync` (and `match_artist_folder_internal` when `auto_apply_confident` is true), `assign_matches` places exact matches for `Archived` releases into `assignment.restorable`, but only `assignment.matched` was auto-applied (`MatchDecision::Adopt`), leaving `assignment.restorable` unapplied.

## 3. Specification

### 3.1 Auto-relink `unknown_rels` against `Ok` linked folders & suppress empty `OutDir` cards (`worker/library.rs`)

1. In `verify_files` Step 8 (`if !mass_missing && st.auto_relink`):
   - Collect relink candidates not only from `missing_rels`, but also from any entry in `unknown_rels` whose artist (`rel.band_id`) has at least one linked folder with `state == FolderState::Ok`.
   - Run `find_relink_candidates_with_rejected` against the artist's `Ok` linked folders:
     - Any release from `unknown_rels` that is confidently relinked (`outcome.relinked`) is updated via `lib_repo::relink` (`local_status = 'present'`, `local_path = <linked_folder_candidate>`) and removed from `unknown_rels`.
     - Any release from `unknown_rels` that is *not* relinked remains in `unknown_rels` (preserving the unmounted-drive safeguard when the album was not moved into an `Ok` linked folder).
2. In `verify_files` Step 10:
   - Remove the `if out_dir_state != FolderState::Ok && is_all_scope` block that pushes a 0-release `UnavailableRoot::OutDir` card. `UnavailableRoot::OutDir` is emitted only when `group_unavailable_releases` produces a non-empty `releases` list for `OutDir`.

### 3.2 Rescan `Ok` linked folders on manual `VerifyFiles` & auto-restore `restorable` matches (`worker/library.rs`)

1. In `rescan_and_adopt_on_sync` and `match_artist_folder_internal` (when `auto_apply_confident` is true):
   - Auto-apply both `assignment.matched` (`MatchDecision::Adopt`) and `assignment.restorable` (`MatchDecision::Restore`), moving both into `linked` so `Archived` releases whose files are found in a linked folder are automatically restored and re-pointed to the linked folder path.
2. At the end of `verify_files`:
   - When `trigger == VerifyTrigger::Manual` and `st.rescan_linked_on_sync` is true, for every `band_id` in scope that has at least one `FolderState::Ok` linked folder, run a linked-folder rescan pass (`rescan_linked_folder_after_verify`) that scans `Ok` linked folders, matches adoptable releases (`downloaded = 0`, `Archived`, or `downloaded = 1` outside linked folders via `split_matchable_releases`), auto-applies confident `matched` and `restorable` proposals, updates `local_only_count`, and emits `ArtistStats` (and `ReconcileReport` if review/conflict items remain).

### 3.3 Add `UnavailableAction::MovedToLinked` and UI button (`types.rs`, `worker/library.rs`, both UIs)

1. In `crates/bcvault-core/src/types.rs`:
   ```rust
   pub enum UnavailableAction {
       /// Drive unplugged, will reconnect: change nothing.
       Keep,
       /// Files were moved into the artist's linked folder(s): scan and match linked folders.
       MovedToLinked,
       /// Deleted on purpose: apply `policy` to `keys`; optionally unlink the folder.
       Deleted { policy: DeletedPolicy, unlink: bool },
   }
   ```
2. In `LibraryService::resolve_unavailable`:
   - When `UnavailableAction::MovedToLinked` is received, collect the unique `band_id`s for `dec.keys` that have linked folders, unmark/archive or allow `match_artist_folder_internal` to match them (for releases whose old path is in an unavailable `OutDir`, `split_matchable_releases` already treats them as adoptable because their `local_path` is outside every linked folder!), and call `self.match_artist_folder_internal(band_id, true, CancellationToken::new()).await`.
3. In `crates/bcvault/src/ui/modals.rs` and `crates/bcvault-fx/src/overlays/modals.rs` (`show_unavailable_modal`):
   - On an `UnavailableRoot::OutDir` card, if any release in `card.folder.releases` belongs to an artist that has a linked folder in `state.folders`, render Choice 2:
     - Label: `"I moved the files to the linked folder:"`
     - Button: `"Scan linked folder"` -> sends `UnavailableDecision { root: card.folder.root, keys, action: UnavailableAction::MovedToLinked }` and removes the card.

## 4. Commit sequence

1. `fix(library): relink out_dir releases to linked folders and rescan on verify` (`Task: none`)
   - Auto-relink `unknown_rels` whose artist has an `Ok` linked folder in `verify_files`.
   - Stop pushing empty `UnavailableRoot::OutDir` cards when 0 releases are affected.
   - Auto-restore `assignment.restorable` in `rescan_and_adopt_on_sync` and `match_artist_folder_internal`.
   - Rescan `Ok` linked folders at the end of manual `verify_files` so unmarked (`downloaded = 0`), archived (`KeepMarked`), and `out_dir`-resident releases are matched and adopted/restored into linked folders.
   - Add unit tests covering:
     1. Downloading 6 releases to `out_dir`, moving them to the linked folder and deleting `out_dir`, then running `verify_files(Manual)` -> all 6 silently relink to the linked folder with zero `FolderUnavailable` events and `6 (+5)/6/6` stats.
     2. Marking `out_dir` releases as deleted (`MarkNotDownloaded` or `KeepMarked`) and pressing `verify_files(Manual)` again with the files in the linked folder -> all 6 are adopted/restored into the linked folder.
2. `feat(library): add MovedToLinked action to resolve_unavailable and UI modals` (`Task: none`)
   - Add `UnavailableAction::MovedToLinked` to `types.rs` and handle it in `LibraryService::resolve_unavailable`.
   - Add `"I moved the files to the linked folder: [Scan linked folder]"` to `show_unavailable_modal` in both `bcvault` and `bcvault-fx`.
   - Add unit test verifying `UnavailableAction::MovedToLinked` adopts/relinks moved releases in the artist's linked folder.
3. `docs(library): document download-dir verify relink and rescan workflow` (`Task: none`)
   - Update `docs/library.md`, `.agents/PLANS/LIBRARY_SYNC/VERIFY-DOWNLOAD-DIR-FIX.md`, `.agents/AGENTS.md`, `.agents/ROADMAP.md`, and `.agents/PROGRESS.md`.

## 5. Verification

- `./scripts/check.sh` passes with zero warnings and all tests green.

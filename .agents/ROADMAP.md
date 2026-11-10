# ROADMAP: order of work across plans

*Companion to `AGENTS.md`. Prepared 7 Nov 2026, revised 10 Nov 2026 (synced with the codebase: library L0 to L6, MATCH-FIX, badge rework, MISSING-FOLDER-FIX, and MULTI_FOLDER landed; auto-sync, archive, mail-redeem, and file-themes plans tracked).*

## 1. Status snapshot (live status is in `PROGRESS.md`)

| Plan | State on 10 Nov 2026 |
| --- | --- |
| Base engine (`Initial Plan.md`) | Implemented (Phases 0 to 5) in `v0.1.0` |
| FX (`PLANS/FX/FX_PLAN.md`) | Implemented (Phases F0 to F8) in `v0.1.0` |
| Library (`PLANS/LIBRARY_SYNC/LIBRARY_SYNC_PLAN.md`) | Implemented (Phases L0 to L6) in `v0.1.0` |
| Exact matching (`PLANS/LIBRARY_SYNC/MATCH-FIX.md`) | Implemented (Steps 1 to 8) in `v0.1.0` |
| Badge reform (`PROGRESS.md`, migration `0005`) | Implemented (Steps 1 to 6) in `v0.1.0` |
| Missing folder handling (`PLANS/LIBRARY_SYNC/MISSING-FOLDER-FIX.md`) | Implemented (Steps 1 to 5) in `v0.1.1` and `v0.1.2` |
| Multiple linked folders (`PLANS/LIBRARY_SYNC/MULTI_FOLDER.md`) | Implemented (Steps 1 to 6) in `main` (migration `0006_artist_folders_multi.sql`) |
| Download dir link guard and local-only fix (`PLANS/LIBRARY_SYNC/DOWNLOAD-DIR-FIX.md`) | Planned (Steps 1 and 2, no migration) |
| Tooling (T0) | In progress: architectural guards, `scripts/check.sh`, and `.githooks/pre-push` with auto `install-fx.sh` landed; commit-msg hooks and cliff optional. |
| Auto sync (`PLANS/AUTO_SYNC/AUTO_SYNC.md`) | Planned, not started; scheduled sync, quick check, removed-release tracking, change feed. |
| Archive (`PLANS/ARCHIVE/ARCHIVE.md`) | Planned, not started; selectable release archive (text & covers). Depends on auto sync. |
| Mail (`PLANS/MAIL_REDEEM/MAIL_REDEEM_PLAN.md`) | Planned, not started (Phases M0 to M6); mailbox redeem and cleanup. First real email fixture received. |
| File themes (`PLANS/USER_THEMES/USER_THEMES.md`) | Planned, not started; file-based themes for `bcvault-fx` (FX only, independent). |

## 2. Dependency graph

```text
T0 tooling ──┬──► MULTI_FOLDER (done)
             │
             ├──► AUTO_SYNC ──► ARCHIVE
             │
             ├──► M0 ► M1 ► M2 ► M3 ► M4 ──┬──► M5 (classic UI)
             │    (core mail; reuses matcher)└──► M6 (FX UI)
             │
             └──► USER_THEMES (FX only, independent)
```

F0 to F8, L0 to L6, MATCH-FIX, BADGE-REFORM, MISSING-FOLDER-FIX, and MULTI_FOLDER are done and not part of the active graph.
- `MULTI_FOLDER.md` generalised `MISSING-FOLDER-FIX`'s root attribution to "longest matching link among N" and is completed.
- M0.1 reuses `bcvault_core::library::textmatch` and the matcher's scoring (both exist). M0.4's follow-up updates FX's schema contract check.
- `ARCHIVE.md` rebases onto `AUTO_SYNC.md` (both edit `worker/sync.rs`).
- `USER_THEMES.md` touches only `bcvault-fx` (with a minor `AppPaths` addition in core) and has no migration dependencies.

## 3. Recommended order

| Step | Work | Why now |
| --- | --- | --- |
| 1 | **AUTO_SYNC** | Unblocked. Takes migration `0007_auto_sync.sql`, verifying and rescanning multiple folders cleanly. |
| 2 | **ARCHIVE** | Merges after `AUTO_SYNC` and rebases onto it (both touch `worker/sync.rs`). |
| 3 | **M0 to M4** mail core | Needs the real-mailbox fixture and cock.li check. Takes the next free migration number when it lands. |
| 4 | **M5 and M6** mail UI in both front-ends | Both apps exist; wires mail redeem into classic and FX. |
| any | **USER_THEMES** | FX-only, no migration; can interleave with anything. |
| any | **T0** remaining tooling | Commit message checks, `.gitmessage`, and changelog generation with `git-cliff`. |

Core phases never touch UI crates, so they interleave safely with UI work of another plan.

## 4. Parallel work

A single agent works one phase or plan at a time. If you run two agents, split by crate: one on `bcvault-core` (M0 to M4, auto-sync and archive core), one on the UI crates. Never two agents on the same phase or the same migration number. Every in-flight plan edits the Settings modals; keep modal changes in new functions or files. Shared files: `PROGRESS.md`, `Cargo.lock`, `crates/bcvault-core/src/types.rs`; rebase often.

## 5. Migration numbering (main DB, `crates/bcvault-core/migrations/`)

| Number | Content | Origin | Binary compatibility |
| --- | --- | --- | --- |
| `0001`, `0002` | Base schema and indexes | Base | n/a |
| `0003_custom_artist_name` | `artists.custom_name` (page artist renaming) | Base follow-up, before the workspace split | Breaking for older binaries |
| `0004_library` | Library columns on `releases`, `artist_folders`, `artist_folder_ignored`, `library_batches`/`library_events` | L0.1 | Breaking (`feat(db)!`) |
| `0005_artist_folder_local_only` | `artist_folders.local_only_count` | Badge rework (`Task: none`) | Breaking (`feat(library)!`) |
| `0006_artist_folders_multi` | Multi-folder links per artist (`artist_folders` multi-link schema) | `MULTI_FOLDER.md` (`Task: none`) | Breaking (`feat(db)!`) |
| `0007` (lowest free) | `0007_auto_sync.sql` | `AUTO_SYNC.md` | Breaking (`feat(db)!`) |
| `0008` (or next free) | `0008_archive.sql` | `ARCHIVE.md` | Breaking (`feat(db)!`) |
| next free | Mail accounts, messages, links, wait list | `MAIL_REDEEM_PLAN.md` (M0.4) | Breaking (`feat(db)!`) |

Number by landing order: whichever plan lands first takes the lowest free number; rename the unmerged file (and the plan text) before merging. The runner list is in `crates/bcvault-core/src/db/migrations.rs`. FX's sidecar database has its own numbering (`fx.sqlite`, `0001_fx_init.sql`) and is unaffected.

## 6. Cross-plan integration points

| Topic | Owner | Consumers |
| --- | --- | --- |
| `library::textmatch` (`tokens`, `key`) and `library::matcher` scoring (`score_candidate`, `normalize_bandcamp_url`) | L1.1 / L2.1, reworked by MATCH-FIX (exact-first; no noise-token or diacritic stripping) | Mail M0.1, M3.3 |
| `library::folder_style` (`FolderStyle` templates, noise list) | MATCH-FIX | Library matcher; mail may reuse for local lookups |
| `Artist folder links (multiple folders per artist)` | `MULTI_FOLDER.md` (done) | `library::service`, `library::verify`, `worker/library`, `AUTO_SYNC`, both UIs, sidebar stats |
| Unavailable folder handling (`FolderUnavailable`, `ResolveUnavailable`) | MISSING-FOLDER-FIX | Verify, relink, `MULTI_FOLDER`, both UIs |
| `validate_redeem_url` | Base 3.8, refactored in M1.4 | Manual redeem modal, mail links |
| `fetch_release_raw`, gate `fetch_bytes`, `WorkerHandle` | F0.3 (done) | FX services, archive |
| Instance lock | F0.3 (done) | Classic and FX `main` |
| FX schema contract check | F1.2 (done), updated by L0.7 | Follow-up to M0.4, auto-sync, archive and multi-folder migrations whenever they add/alter columns or tables FX queries |
| `Settings` new fields | L0.2, MATCH-FIX (folder style) | Must use `#[serde(default)]`; M phases keep secrets out of `Settings` |
| `ArtistStats` (eligible and non-eligible downloaded counts) | Badge rework | Both sidebars; auto-sync removed-release tracking and multi-folder aggregation must keep counts consistent |
| `validate_redeem_url` | Base 3.8, refactored in M1.4 | Manual redeem modal, mail links |
| `fetch_release_raw`, gate `fetch_bytes`, `WorkerHandle` | F0.3 (done) | FX services, archive |
| Instance lock | F0.3 (done) | Classic and FX `main` |
| FX schema contract check | F1.2 (done), updated by L0.7 | Follow-up to M0.4, auto-sync, archive and multi-folder migrations whenever they add/alter columns or tables FX queries |
| `Settings` new fields | L0.2, MATCH-FIX (folder style) | Must use `#[serde(default)]`; M phases keep secrets out of `Settings` |
| `ArtistStats` (eligible and non-eligible downloaded counts) | Badge rework | Both sidebars; auto-sync removed-release tracking and multi-folder aggregation must keep counts consistent |
| Redeem job refactor | M0.2, M4.1 | Existing `RedeemUrl` command keeps working |
| Status glyphs, filters | L4.5 (classic), L5.2 (FX) | Same enums from `types.rs` |

## 7. Merge gates

Each phase ends with: all milestone boxes true, full gate green, rebase on `main`, fast-forward merge, tag `vMAJOR.MINOR.PATCH`. Manual milestone boxes (real mailbox, a real artist folder, visual audit) are confirmed by you before tagging.

## 8. Versions and releases

All crates share `workspace.package.version`. While `0.x`: `feat` bumps minor, `fix` patch, a `!` commit bumps minor and the release note starts with its `BREAKING CHANGE:` text. Generate `CHANGELOG.md` with `git-cliff` (config in `AGENTS.md` Appendix A). Release CI (`.github/workflows/ci.yml`) runs on `v*` tags and on manual dispatch.

| Tag | Content |
| --- | --- |
| `v0.1.0` | Base engine, FX and library (L0 to L6, MATCH-FIX, badge rework); migrations `0001` to `0005`. |
| `v0.1.1` | Unavailable folder handling fixes (`MISSING-FOLDER-FIX`). |
| `v0.1.2` | Unavailable folder resolution for empty/archived releases, archived visual differentiation, and pre-push hook. |
| following releases (`v0.2.0`, ...) | Multi-folder, auto sync, archive, mail, file themes, in landing order |

Every release that includes a new main-DB migration must ship `bcvault` and `bcvault-fx` together.

## 9. Kickoff prompts (paste one per phase or group)

**Multi-folder library sync**
> Read `AGENTS.md`, `ROADMAP.md`, `PLANS/LIBRARY_SYNC/MULTI_FOLDER.md`. Execute the 6 steps in §6 in order, one commit per step, keeping tests green at every step. Generalise linked folders from one-per-artist to many-per-artist while maintaining one-artist-per-folder isolation and updating the stats aggregation, verify attribution, and both front-ends.

**Auto sync**
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/AUTO_SYNC/AUTO_SYNC.md`. Execute the steps in section 9 in order. Add the `Auto` and `QuickCheck` sync modes, scheduled execution, per-artist sync exclusion, removed-release tracking, and the persistent change feed across `bcvault-core` and both front-ends. Never bypass `BandcampGate` and never delete releases that disappeared from Bandcamp.

**Archive**
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/ARCHIVE/ARCHIVE.md`. Make sure `AUTO_SYNC` has landed first. Execute the steps in section 8 in order. Implement text and artwork archiving into the database, cover fallback in both front-ends, and the detail view in the FX drawer. Stop for confirmation before merging JPEG XL AGPL-licensed dependencies.

**Mail core**
> Read `AGENTS.md`, `ROADMAP.md`, `PLANS/MAIL_REDEEM/MAIL_REDEEM_PLAN.md` and the M table of `COMMIT_PLAN.md`. Execute phases M0 to M4. For M1.1 ask me to supply the sanitised `.eml` files (one real sample already exists; I will provide a cock.li-forwarded one). Never log secrets or signed links; never run against a real mailbox without my explicit go-ahead.

**Mail UI (both front-ends)**
> Read `AGENTS.md`, `PLANS/MAIL_REDEEM/MAIL_REDEEM_PLAN.md` §9 and the M5 and M6 rows of `COMMIT_PLAN.md`. Execute M5 in the classic app, then M6 in `bcvault-fx`, keeping the square look and UI-thread purity guards green.

**File themes (bcvault-fx)**
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/USER_THEMES/USER_THEMES.md`. Execute the steps in section 7 in order. Add file-based TOML theme definitions, custom theme editor in the FX settings modal, import/export, and embed the 10 built-in themes. Maintain zero corner radius and strict UI-thread purity.

**Tooling**
> Read `AGENTS.md` and `ROADMAP.md`. Implement remaining T0 items from `AGENTS.md` Appendix A: commit-msg check (`scripts/check-commit-msg.sh`), commit range validation (`scripts/check-commits.sh`), `.gitmessage` template, and `cliff.toml` changelog config.

**Library core** (done; kept for reference)
> Read `AGENTS.md`, `ROADMAP.md`, `LIBRARY_SYNC_PLAN.md` and the L table of `COMMIT_PLAN.md`. Execute phases L0 to L3 in order, one branch per phase, one commit per row, merging each phase only when its milestone boxes pass.

**Library UI (both front-ends)** (done; kept for reference)
> Read `AGENTS.md`, `LIBRARY_SYNC_PLAN.md` §10 and the L4 to L6 rows of `COMMIT_PLAN.md`. Execute L4 in the classic app, then L5 in `bcvault-fx`, then L6. Keep the square look and UI-thread purity guards green, and use `rfd::AsyncFileDialog` from a spawned task for the folder picker.

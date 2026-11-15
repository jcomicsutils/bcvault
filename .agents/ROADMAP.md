# ROADMAP: order of work across plans

*Companion to `AGENTS.md`. Prepared 7 Nov 2026, revised 11 Nov 2026 (synced with the codebase at `v0.1.3`: library L0 to L6, MATCH-FIX, badge rework, MISSING-FOLDER-FIX, MULTI_FOLDER, DOWNLOAD-DIR-FIX, unavailable classification, REVIEW-AND-MATCH-FIX, VERIFY-DOWNLOAD-DIR-FIX, and project docs landed; settings-tabs, download-flow, artist-groups, keyboard, auto-sync, archive, mail-redeem, and file-themes plans tracked).*

## 1. Status snapshot (live status is in `PROGRESS.md`)

| Plan | State on 11 Nov 2026 |
| --- | --- |
| Base engine (`Initial Plan.md`) | Implemented (Phases 0 to 5) in `v0.1.0` |
| FX (`PLANS/FX/FX_PLAN.md`) | Implemented (Phases F0 to F8) in `v0.1.0` |
| Library (`PLANS/LIBRARY_SYNC/LIBRARY_SYNC_PLAN.md`) | Implemented (Phases L0 to L6) in `v0.1.0` |
| Exact matching (`PLANS/LIBRARY_SYNC/MATCH-FIX.md`) | Implemented (Steps 1 to 8) in `v0.1.0` |
| Badge reform (`PROGRESS.md`, migration `0005`) | Implemented (Steps 1 to 6) in `v0.1.0` |
| Missing folder handling (`PLANS/LIBRARY_SYNC/MISSING-FOLDER-FIX.md`) | Implemented (Steps 1 to 5) in `v0.1.1` and `v0.1.2` |
| Multiple linked folders (`PLANS/LIBRARY_SYNC/MULTI_FOLDER.md`) | Implemented (Steps 1 to 6, migration `0006_artist_folders_multi.sql`) in `v0.1.3` |
| Download dir link guard and local-only fix (`PLANS/LIBRARY_SYNC/DOWNLOAD-DIR-FIX.md`) | Implemented (Steps 1 and 2, no migration) in `v0.1.3` |
| Unavailable classification (`0007_unavailable.sql`) | Implemented (0-track detection, manual unavailable toggle, and folder matching support) in `v0.1.3` |
| Review rejection & URL/slug match picker (`PLANS/LIBRARY_SYNC/REVIEW-AND-MATCH-FIX.md`) | Implemented (Steps 1 to 5, migration `0008_rejected_matches.sql`) in `v0.1.3` |
| Verify download-dir relink & rescan (`PLANS/LIBRARY_SYNC/VERIFY-DOWNLOAD-DIR-FIX.md`) | Implemented (Steps 1 to 3, no migration) in `v0.1.3` |
| Tooling (T0) | Active: architectural guards (`scripts/guards.sh`), `scripts/check.sh`, and `.githooks/pre-push` with auto `install-fx.sh` landed; commit-msg hooks and cliff optional. |
| Tabbed settings & ZIP options (`PLANS/SETTINGS_TABS/SETTINGS_TABS.md`) | Implemented (Steps 1 to 7) on `feat/settings-tabs`; 6-tab settings modal, `Settings::sanitize`, ZIP extraction/cleanup parity, and coverage tests. |
| Bulk selection & download remaining (`PLANS/DOWNLOAD_FLOW/DOWNLOAD_FLOW.md`) | Planned, not started; select all/remaining/invert, idempotent `in_flight` enqueue, `EnqueueSummary`, soft-quota parity in FX, and bulk confirmation. |
| Artist groups (`PLANS/ARTIST_GROUPS/ARTIST_GROUPS.md`) | Planned, not started; manual and rule-based collapsible sidebar groups (`0009_artist_groups.sql` or next free migration). |
| Complete keyboard operation (`PLANS/KEYBOARD/KEYBOARD.md`) | Planned, not started; per-UI shortcut registry, `F6` focus regions, spatial wall/table/sidebar/dock navigation, modal focus scope, and `?`/`F1` help overlay. |
| Auto sync (`PLANS/AUTO_SYNC/AUTO_SYNC.md`) | Planned, not started; scheduled sync, quick check, removed-release tracking, change feed (`0009_auto_sync.sql` or next free migration). |
| Archive (`PLANS/ARCHIVE/ARCHIVE.md`) | Planned, not started; selectable release archive (text & covers). Depends on auto sync (`0010_archive.sql` or next free migration). |
| Mail (`PLANS/MAIL_REDEEM/MAIL_REDEEM_PLAN.md`) | Planned, not started (Phases M0 to M6); mailbox redeem and cleanup. First real email fixture received. |
| File themes (`PLANS/USER_THEMES/USER_THEMES.md`) | Planned, not started; file-based TOML themes for `bcvault-fx` (FX only, independent). |

## 2. Dependency graph

```text
T0 tooling (done) ──┬──► SETTINGS_TABS ──► DOWNLOAD_FLOW ──► ARTIST_GROUPS ──► KEYBOARD
                    │
                    ├──► AUTO_SYNC ──► ARCHIVE
                    │
                    ├──► M0 ► M1 ► M2 ► M3 ► M4 ──┬──► M5 (classic UI)
                    │    (core mail; reuses matcher)└──► M6 (FX UI)
                    │
                    └──► USER_THEMES (FX only, independent)
```

F0 to F8, L0 to L6, MATCH-FIX, BADGE-REFORM, MISSING-FOLDER-FIX, MULTI_FOLDER, DOWNLOAD-DIR-FIX, unavailable classification (`0007`), REVIEW-AND-MATCH-FIX (`0008`), and VERIFY-DOWNLOAD-DIR-FIX are done and shipped through `v0.1.3`.
- `SETTINGS_TABS.md` merges first among the pending UI plans so `AUTO_SYNC`, `ARCHIVE`, `USER_THEMES` and `MAIL_REDEEM` each add or extend one settings tab file instead of conflicting in a monolithic modal.
- `DOWNLOAD_FLOW.md` precedes `ARTIST_GROUPS.md` (which reuses `RemainingScope::Artists` for group downloads), and `KEYBOARD.md` lands after `SETTINGS_TABS`, `DOWNLOAD_FLOW`, and `ARTIST_GROUPS` to bind and audit their final controls.
- M0.1 reuses `bcvault_core::library::textmatch` and the matcher's scoring (both exist). M0.4's follow-up updates FX's schema contract check.
- `ARCHIVE.md` rebases onto `AUTO_SYNC.md` (both edit `worker/sync.rs`).
- `USER_THEMES.md` touches only `bcvault-fx` (with a minor `AppPaths` addition in core) and has no migration dependencies.

## 3. Recommended order

| Step | Work | Why now |
| --- | --- | --- |
| 1 | **SETTINGS_TABS** | Unblocked, no migration. Splits the monolithic settings modals into per-tab files, adds `Settings::sanitize`, and brings ZIP extraction parity to FX before other plans add settings. |
| 2 | **DOWNLOAD_FLOW** | Unblocked, no migration. Adds idempotent enqueue, `SelectAll`/`SelectRemaining`, `Download remaining`, and soft-quota dialog parity in FX. |
| 3 | **ARTIST_GROUPS** | Unblocked after `DOWNLOAD_FLOW`. Adds manual and rule-based sidebar artist groups and multi-artist release filtering (takes next free migration). |
| 4 | **KEYBOARD** | Audits and binds the final UI from steps 1–3 into a single shortcut registry with focus regions, spatial navigation, and modal focus scopes. |
| 5 | **AUTO_SYNC** | Unblocked. Takes next free migration (`0009_auto_sync.sql` or next free), verifying and rescanning multiple folders cleanly. |
| 6 | **ARCHIVE** | Merges after `AUTO_SYNC` and rebases onto it (both touch `worker/sync.rs`; takes next free migration). |
| 7 | **M0 to M4** mail core | Needs the real-mailbox fixture and cock.li check. Takes the next free migration number when it lands. |
| 8 | **M5 and M6** mail UI in both front-ends | Both apps exist; wires mail redeem into classic and FX. |
| any | **USER_THEMES** | FX-only, no migration; extends the `Appearance` tab and can interleave with anything. |
| any | **T0** remaining tooling | Optional commit message checks, `.gitmessage`, and changelog generation with `git-cliff`. |

Core phases never touch UI crates, so they interleave safely with UI work of another plan.

## 4. Parallel work

A single agent works one phase or plan at a time. If you run two agents, split by crate: one on `bcvault-core` (M0 to M4, auto-sync and archive core), one on the UI crates. Never two agents on the same phase or the same migration number. Every in-flight plan edits the Settings modals; once `SETTINGS_TABS.md` lands, each plan edits its own tab file. Shared files: `PROGRESS.md`, `Cargo.lock`, `crates/bcvault-core/src/types.rs`; rebase often.

## 5. Migration numbering (main DB, `crates/bcvault-core/migrations/`)

| Number | Content | Origin | Binary compatibility |
| --- | --- | --- | --- |
| `0001`, `0002` | Base schema and indexes | Base (`v0.1.0`) | n/a |
| `0003_custom_artist_name` | `artists.custom_name` (page artist renaming) | Base follow-up (`v0.1.0`) | Breaking for older binaries |
| `0004_library` | Library columns on `releases`, `artist_folders`, `artist_folder_ignored`, `library_batches`/`library_events` | L0.1 (`v0.1.0`) | Breaking (`feat(db)!`) |
| `0005_artist_folder_local_only` | `artist_folders.local_only_count` | Badge rework (`v0.1.0`, `Task: none`) | Breaking (`feat(library)!`) |
| `0006_artist_folders_multi` | Multi-folder links per artist (`artist_folders` multi-link schema) | `MULTI_FOLDER.md` (`v0.1.3`, `Task: none`) | Breaking (`feat(db)!`) |
| `0007_unavailable` | `'unavailable'` release classification CHECK constraint and 0-track backfill | Unavailable release fix (`v0.1.3`, `Task: none`) | Breaking (`feat(core)!`) |
| `0008_rejected_matches` | `artist_folder_rejected` table for persistent rejected review pairs | `REVIEW-AND-MATCH-FIX.md` (`v0.1.3`, `Task: none`) | Breaking (`feat(db)!`) |
| `0009_artist_groups` | `artist_groups` and `artist_group_pins` tables | `ARTIST_GROUPS.md` (`Task: none`) | Breaking (`feat(db)!`) |
| `0010` (lowest free) | `auto_sync` / `sync_events` (or next landing plan) | `AUTO_SYNC.md` | Breaking (`feat(db)!`) |
| `0011` (or next free) | `release_archive` table (text archive in main DB; covers in `archive.sqlite`) | `ARCHIVE.md` | Breaking (`feat(core)!` / `feat(db)!`) |
| next free | Mail accounts, messages, links, wait list | `MAIL_REDEEM_PLAN.md` (M0.4) | Breaking (`feat(db)!`) |

Number by landing order: whichever plan lands next takes the lowest free number (currently `0010`); rename the unmerged file (and the plan text) before merging. The runner list is in `crates/bcvault-core/src/db/migrations.rs` (`MAX_VERSION = 9`). FX's sidecar database has its own numbering (`fx.sqlite`, `0001_fx_init.sql`) and is unaffected.

## 6. Cross-plan integration points

| Topic | Owner | Consumers |
| --- | --- | --- |
| `library::textmatch` (`tokens`, `key`) and `library::matcher` scoring (`score_candidate`, `normalize_bandcamp_url`) | L1.1 / L2.1, reworked by MATCH-FIX (exact-first; no noise-token or diacritic stripping) | Mail M0.1, M3.3, `ARTIST_GROUPS.md`, `KEYBOARD.md` type-ahead |
| `library::folder_style` (`FolderStyle` templates, noise list) | MATCH-FIX (done) | Library matcher; mail may reuse for local lookups |
| `Artist folder links (multiple folders per artist)` | `MULTI_FOLDER.md` (done) | `library::service`, `library::verify`, `worker/library`, `AUTO_SYNC`, `SETTINGS_TABS`, `ARTIST_GROUPS`, both UIs, sidebar stats |
| Unavailable folder handling (`FolderUnavailable`, `ResolveUnavailable`, `MovedToLinked`) | `MISSING-FOLDER-FIX`, `VERIFY-DOWNLOAD-DIR-FIX` (done) | Verify, relink, `MULTI_FOLDER`, both UIs |
| Rejected review pairs (`artist_folder_rejected`) and URL/slug release resolution | `REVIEW-AND-MATCH-FIX` (done) | `library::matcher`, `library::verify`, `library::service`, both UIs |
| `SettingsTab` registry & `Settings::sanitize` | `SETTINGS_TABS.md` | `AUTO_SYNC` (`Sync` tab), `ARCHIVE` (`Archive` tab), `USER_THEMES` (`Appearance` tab), `MAIL_REDEEM` (`Mail` tab), `KEYBOARD` |
| `ReleaseRow::is_download_candidate`, `RemainingScope`, and idempotent `in_flight` enqueue | `DOWNLOAD_FLOW.md` | Both UIs, `ARTIST_GROUPS` (group download remaining), `AUTO_SYNC` (excludes `removed` releases), `KEYBOARD` |
| `SidebarRow` flattened model and `ReleaseQuery.band_ids` | `ARTIST_GROUPS.md` | Both sidebars, `KEYBOARD` sidebar cursor navigation |
| Shortcut registry (`keys.rs`), focus regions (`F6`), and `modal_scope` | `KEYBOARD.md` | Both UIs, all modals and future panels (`AGENTS.md` rule 15) |
| `validate_redeem_url` | Base 3.8, refactored in M1.4 | Manual redeem modal, mail links |
| `fetch_release_raw`, gate `fetch_bytes`, `WorkerHandle` | F0.3 (done) | FX services, archive |
| Instance lock | F0.3 (done) | Classic and FX `main` |
| FX schema contract check | F1.2 (done), updated by L0.7 | Follow-up to M0.4, auto-sync, archive, artist-groups, and library migrations whenever they add/alter columns or tables FX queries |
| `Settings` new fields | L0.2, MATCH-FIX (folder style) | Must use `#[serde(default)]` and be classified in `SettingsTab` (`AGENTS.md` rule 16); M phases keep secrets out of `Settings` |
| `ArtistStats` (eligible and non-eligible downloaded counts) | Badge rework (done) | Both sidebars, `ARTIST_GROUPS`; auto-sync removed-release tracking and multi-folder aggregation must keep counts consistent |
| Redeem job refactor | M0.2, M4.1 | Existing `RedeemUrl` command keeps working |
| Status glyphs, filters | L4.5 (classic), L5.2 (FX) | Same enums from `types.rs` |

## 7. Merge gates

Each phase or plan ends with: all milestone boxes true, full gate green, rebase on `main`, fast-forward merge, tag `vMAJOR.MINOR.PATCH`. Manual milestone boxes (real mailbox, a real artist folder, visual audit) are confirmed by you before tagging.

## 8. Versions and releases

All crates share `workspace.package.version`. While `0.x`: `feat` bumps minor (or patch when grouped in point releases per user direction), `fix` patch, a `!` commit includes its `BREAKING CHANGE:` text. Release CI (`.github/workflows/ci.yml`) runs on `v*` tags and on manual dispatch.

| Tag | Content |
| --- | --- |
| `v0.1.0` | Base engine, FX and library (L0 to L6, MATCH-FIX, badge rework); migrations `0001` to `0005`. |
| `v0.1.1` | Unavailable folder handling fixes (`MISSING-FOLDER-FIX`). |
| `v0.1.2` | Unavailable folder resolution for empty/archived releases, archived visual differentiation, and pre-push hook. |
| `v0.1.3` | Multi-folder links (`0006`), download-dir guard & local-only matching fix (`DOWNLOAD-DIR-FIX`), `unavailable` release classification (`0007`), persistent review rejection & URL/slug release picker (`REVIEW-AND-MATCH-FIX`, `0008`), verify download-dir relink & rescan (`VERIFY-DOWNLOAD-DIR-FIX`), complete `docs/` suite, and `.agents/` plan synchronization. |
| following releases (`v0.2.0`, ...) | Settings tabs, download flow, artist groups, keyboard operation, auto sync, archive, mail, file themes, in landing order. |

Every release that includes a new main-DB migration must ship `bcvault` and `bcvault-fx` together.

## 9. Kickoff prompts (paste one per phase or group)

**Tabbed settings and ZIP extraction options**
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/SETTINGS_TABS/SETTINGS_TABS.md`. Execute the 7 steps in §9 in order, one commit per step, keeping `scripts/check.sh` green at every step. Add `Settings::sanitize`, split both UIs' settings dialogs into the 6 tabs of §3.1, add the missing download/ZIP/network settings to FX and classic, and add the settings parity coverage test in both UI crates.

**Select all, download remaining, and safer bulk downloads**
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/DOWNLOAD_FLOW/DOWNLOAD_FLOW.md`. Execute the 9 steps in §8 in order, one commit per step, keeping `scripts/check.sh` green. Add `ReleaseRow::is_download_candidate`, pure selection helpers, idempotent `in_flight` tracking with `EnqueueMode` and `EnqueueSummary`, `EnqueueRemaining`, bulk quota preview, selection bar, and soft-quota dialog parity in FX.

**Artist groups in the sidebar**
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/ARTIST_GROUPS/ARTIST_GROUPS.md`. Execute the steps in §7 in order, one commit per step, keeping `scripts/check.sh` green. Add pure group evaluation and sidebar row building, migration `0009_artist_groups.sql` (or lowest free number), worker group management, collapsible group headers and group editor modals in both UIs, and multi-artist `ReleaseQuery.band_ids` filtering.

**Complete keyboard operation**
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/KEYBOARD/KEYBOARD.md`. Execute the 14 steps in §9 in order, one commit per step, keeping `scripts/check.sh` green. Move shortcuts into a per-crate registry, add `F6` focus regions, spatial wall/table/sidebar/dock navigation with scroll-into-view, keyboard context actions, modal focus scopes, and the `?`/`F1` help overlay in both front-ends.

**Auto sync**
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/AUTO_SYNC/AUTO_SYNC.md`. Execute the steps in section 9 in order. Add the `Auto` and `QuickCheck` sync modes, scheduled execution, per-artist sync exclusion, removed-release tracking (using the lowest free migration number), and the persistent change feed across `bcvault-core` and both front-ends. Never bypass `BandcampGate` and never delete releases that disappeared from Bandcamp.

**Archive**
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/ARCHIVE/ARCHIVE.md`. Make sure `AUTO_SYNC` has landed first. Execute the steps in section 5 in order (using the lowest free migration number). Implement text and artwork archiving into the database, cover fallback in both front-ends, and the detail view in the FX drawer. Stop for confirmation before merging JPEG XL AGPL-licensed dependencies.

**Mail core**
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/MAIL_REDEEM/MAIL_REDEEM_PLAN.md`. Execute phases M0 to M4 (using the lowest free migration number). For M1.1 ask me to supply the sanitised `.eml` files (one real sample already exists; I will provide a cock.li-forwarded one). Never log secrets or signed links; never run against a real mailbox without my explicit go-ahead.

**Mail UI (both front-ends)**
> Read `AGENTS.md` and `PLANS/MAIL_REDEEM/MAIL_REDEEM_PLAN.md` §9 and §12. Execute M5 in the classic app, then M6 in `bcvault-fx`, keeping the square look and UI-thread purity guards green.

**File themes (bcvault-fx)**
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/USER_THEMES/USER_THEMES.md`. Execute the steps in section 7 in order. Add file-based TOML theme definitions, custom theme editor in the FX settings modal, import/export, and embed the 10 built-in themes. Maintain zero corner radius and strict UI-thread purity.

**Tooling**
> Read `AGENTS.md` and `ROADMAP.md`. Implement remaining optional T0 items from `AGENTS.md` Appendix A: commit-msg check (`scripts/check-commit-msg.sh`), commit range validation (`scripts/check-commits.sh`), `.gitmessage` template, and `cliff.toml` changelog config.

**Review rejection and URL/slug match picker** (done; kept for reference)
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/LIBRARY_SYNC/REVIEW-AND-MATCH-FIX.md`. Execute the 5 steps in §6 in order, one commit per step, keeping tests green at every step.

**Multi-folder library sync** (done; kept for reference)
> Read `AGENTS.md`, `ROADMAP.md`, `PLANS/LIBRARY_SYNC/MULTI_FOLDER.md`. Execute the 6 steps in §6 in order, one commit per step, keeping tests green at every step.

**Library core & UI** (done; kept for reference)
> Read `AGENTS.md`, `ROADMAP.md`, and `PLANS/LIBRARY_SYNC/LIBRARY_SYNC_PLAN.md`. Execute phases L0 to L6 in order.

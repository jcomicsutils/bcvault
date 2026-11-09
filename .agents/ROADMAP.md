# ROADMAP: order of work across plans

*Companion to `AGENTS.md`. Prepared 7 Nov 2026, revised 8 Nov 2026 (synced with the codebase: library L0 to L6, MATCH-FIX and badge rework landed; auto-sync, archive and file-themes plans added).*

## 1. Status snapshot (live status is in `PROGRESS.md`)

| Plan | State on 8 Nov 2026 |
| --- | --- |
| Base engine (`Initial Plan.md`) | Implemented |
| FX (`FX/FX_PLAN.md`) | Implemented (F0 to F8) |
| Library (`LIBRARY_SYNC/LIBRARY_SYNC_PLAN.md`) | Implemented (L0 to L6) on `main`, plus `MATCH-FIX.md`, `MISSING-FOLDER-FIX.md` and the sidebar badge rework. All of base, FX and library ship in `v0.1.0` (the tag points at the current `main` head, `781d5bd`) |
| Tooling (T0) | Not done: only an older `scripts/check.sh` exists |
| Mail (`MAIL_REDEEM/MAIL_REDEEM_PLAN.md`) | Planned, not started; first real email fixture received |
| Auto sync (`AUTO_SYNC/AUTO_SYNC.md`) | Planned, not started |
| Archive (`ARCHIVE/ARCHIVE.md`) | Planned, not started; depends on auto sync |
| File themes (`USER_THEMES/USER_THEMES.md`) | Planned, not started; FX only, independent |

## 2. Dependency graph

```text
T0 tooling ──┬──► M0 ► M1 ► M2 ► M3 ► M4 ──┬──► M5 (classic UI)
             │   (core mail; reuses library::textmatch)  └──► M6 (FX UI)
             ├──► AUTO_SYNC ──► ARCHIVE
             └──► USER_THEMES (FX only)
```

F0 to F8 and L0 to L6 are done and are not part of the graph. M0.1 reuses `bcvault_core::library::textmatch` and the matcher's scoring (both exist). M0.4's follow-up updates FX's schema contract check. `ARCHIVE.md` rebases onto `AUTO_SYNC.md` (both edit `worker/sync.rs`).

## 3. Recommended order

| Step | Work | Why now |
| --- | --- | --- |
| 1 | **T0** tooling (hooks, guards, commit checks) | The history so far was not validated (unscoped and phase-sized commits exist). Validate from here on; do not rewrite merged history. |
| 2 | **AUTO_SYNC**, then **ARCHIVE** | Migrations `0006`, `0007` (see §5). |
| 3 | **M0 to M4** mail core | Needs the real-mailbox fixture and your cock.li check. Takes the next free migration number when it lands. |
| 4 | **M5 and M6** mail UI in both front-ends | Both apps exist. |
| any | **USER_THEMES** | FX-only, no migration; can interleave with anything. |

Core phases never touch UI crates, so they interleave safely with UI work of another plan.

## 4. Parallel work

A single agent works one phase or plan at a time. If you run two agents, split by crate: one on `bcvault-core` (M0 to M4, auto-sync and archive core), one on the UI crates. Never two agents on the same phase or the same migration number. Every in-flight plan edits the Settings modals; keep modal changes in new functions or files. Shared files: `PROGRESS*.md`, `Cargo.lock`, `crates/bcvault-core/src/types.rs`; rebase often.

## 5. Migration numbering (main DB, `crates/bcvault-core/migrations/`)

| Number | Content | Origin | Binary compatibility |
| --- | --- | --- | --- |
| `0001`, `0002` | Base schema and indexes | Base | n/a |
| `0003_custom_artist_name` | `artists.custom_name` (page artist renaming) | Base follow-up, before the workspace split | Breaking for older binaries |
| `0004_library` | Library columns on `releases`, `artist_folders`, `artist_folder_ignored`, `library_batches`/`library_events` | L0.1 | Breaking (`feat(db)!`) |
| `0005_artist_folder_local_only` | `artist_folders.local_only_count` | Badge rework (`Task: none`) | Breaking (`feat(library)!`) |
| `0006_auto_sync` (planned) | Auto-sync, missing-release and sync-event schema | `AUTO_SYNC.md` | Breaking |
| `0007_archive` (planned) | Text archive | `ARCHIVE.md` | Breaking |
| next free (planned) | Mail accounts, messages, links, wait list | M0.4 | Breaking (`feat(db)!`) |

Number by landing order: whichever plan lands first takes the lowest free number; rename the unmerged file (and the plan text) before merging. The runner list is in `crates/bcvault-core/src/db/migrations.rs`. FX's sidecar database has its own numbering (`fx.sqlite`, `0001_fx_init.sql`) and is unaffected.

## 6. Cross-plan integration points

| Topic | Owner | Consumers |
| --- | --- | --- |
| `library::textmatch` (`tokens`, `key`) and `library::matcher` scoring (`score_candidate`, `normalize_bandcamp_url`) | L1.1 / L2.1, reworked by MATCH-FIX (exact-first; no noise-token or diacritic stripping) | Mail M0.1, M3.3 |
| `library::folder_style` (`FolderStyle` templates, noise list) | MATCH-FIX | Library matcher; mail may reuse for local lookups |
| `validate_redeem_url` | Base 3.8, refactored in M1.4 | Manual redeem modal, mail links |
| `fetch_release_raw`, gate `fetch_bytes`, `WorkerHandle` | F0.3 (done) | FX services, archive |
| Instance lock | F0.3 (done) | Classic and FX `main` |
| FX schema contract check | F1.2 (done), updated by L0.7 | Follow-up to M0.4, auto-sync and archive migrations whenever they add columns or tables FX queries |
| `Settings` new fields | L0.2, MATCH-FIX (folder style) | Must use `#[serde(default)]`; M phases keep secrets out of `Settings` |
| Artist folder links (`artist_folders`) | L0.3 | Library service, sync service, both UIs, sidebar badge (`local_only_count`) |
| `ArtistStats` (eligible and non-eligible downloaded counts) | Badge rework | Both sidebars; auto-sync removed-release tracking must keep the counts consistent |
| Redeem job refactor | M0.2, M4.1 | Existing `RedeemUrl` command keeps working |
| Status glyphs, filters | L4.5 (classic), L5.2 (FX) | Same enums from `types.rs` |

## 7. Merge gates

Each phase ends with: all milestone boxes true, full gate green, rebase on `main`, fast-forward merge, tag `milestone/<ID>`. Manual milestone boxes (real mailbox, a real artist folder, visual audit) are confirmed by you before the tag. The library phases were merged without `milestone/L*` tags (see `PROGRESS.md`); do not tag them retroactively unless you ask for it.

## 8. Versions and releases

All crates share `workspace.package.version`. While `0.x`: `feat` bumps minor, `fix` patch, a `!` commit bumps minor and the release note starts with its `BREAKING CHANGE:` text. Generate `CHANGELOG.md` with `git-cliff` (config in `AGENTS.md` Appendix A). Release CI (`.github/workflows/ci.yml`) runs on `v*` tags and on manual dispatch.

| Tag | Content |
| --- | --- |
| `v0.1.0` | Base engine, FX and library (L0 to L6, MATCH-FIX, badge rework); migrations `0001` to `0005`. FX and library were never tagged separately. Do not retag |
| following minors (`v0.2.0`, ...) | Auto sync, archive, mail, file themes, in landing order |

Every release that includes a new main-DB migration must ship `bcvault` and `bcvault-fx` together.

## 9. Kickoff prompts (paste one per phase or group)

**Tooling**
> Read `AGENTS.md` and `ROADMAP.md`. Implement T0.1 to T0.6 from `COMMIT_PLAN.md` (only an older `scripts/check.sh` exists; replace it with the Appendix A version): create the scripts, hooks, `.gitmessage` and `cliff.toml` from `AGENTS.md` Appendix A, install the hooks, and make the existing repository pass `scripts/check.sh`. Commit with the messages in the table. Stop when `scripts/check-commits.sh` passes on the commits after the T0 work begins; older history predates the convention and is not rewritten.

**Library core** (done; kept for reference)
> Read `AGENTS.md`, `ROADMAP.md`, `LIBRARY_SYNC_PLAN.md` and the L table of `COMMIT_PLAN.md`. Execute phases L0 to L3 in order, one branch per phase, one commit per row, merging each phase only when its milestone boxes pass. The feature is one folder linked to one artist page that the user supplies: never scan anything outside a linked folder or `out_dir`, never guess an artist page, never add library roots. Ask me before changing any default listed under "Defaults assumed".

**Library UI (both front-ends)** (done; kept for reference)
> Read `AGENTS.md`, `LIBRARY_SYNC_PLAN.md` §10 and the L4 to L6 rows of `COMMIT_PLAN.md`. Execute L4 in the classic app, then L5 in `bcvault-fx`, then L6. Keep the square look and UI-thread purity guards green, and use `rfd::AsyncFileDialog` from a spawned task for the folder picker.

**Mail core**
> Read `AGENTS.md`, `ROADMAP.md`, `MAIL_REDEEM_PLAN.md` and the M table of `COMMIT_PLAN.md`. Execute phases M0 to M4. For M1.1 ask me to supply the sanitised `.eml` files (one real sample already exists; I will provide a cock.li-forwarded one). Never log secrets or signed links; never run against a real mailbox without my explicit go-ahead.

**Mail UI (both front-ends)**
> Read `AGENTS.md`, `MAIL_REDEEM_PLAN.md` §9 and the M5 and M6 rows of `COMMIT_PLAN.md`. Execute M5 in the classic app, then M6 in `bcvault-fx`, keeping the square look and UI-thread purity guards green.

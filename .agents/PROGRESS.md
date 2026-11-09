# PROGRESS

## Current
Task: none (idle; next: AUTO_SYNC or MULTI_FOLDER, see `ROADMAP.md` §3)  ·  Branch: main  ·  Updated: 2026-11-09

## Done
- B0.1 to B5.x: base engine (phase-sized commits, before the commit convention)
- F0 to F8: FX front-end (one squashed commit plus fixes; details in `FX/PROGRESS-FX.md`)
- L0.1: add library status columns and artist folder link schema
- L0.2: add library statuses, settings fields and filters
- L0.3: add artist folder links with overlap validation
- L0.4: add library repository with batches, events and undo
- L0.5: add archived and missing filters and stats
- L0.6: assert query plans for library indexes
- L0.7: include library columns in the contract check
- L1.1: add unicode text normalisation
- L1.2: read audio tags and guess download format
- L1.3: scan one linked folder for album candidates
- L1.4: add folder link validation and folder probe
- L2.1: score local albums against releases
- L2.2: greedy 1-to-1 candidate assignment
- L2.3: add library service with link, match and report
- L2.4: apply reconcile decisions with undo
- L2.5: queue the first match until the artist has synced
- L3.1: verify downloaded releases against disk
- L3.2: relink moved albums inside the linked folder
- L3.3: apply missing-album policies and resolution commands
- L3.4: rescan linked folders and adopt certain matches
- L3.5: verify files after every sync
- L4.1: add library section to settings
- L4.2: add link folder dialog and sidebar actions
- L4.3: add match report dialog
- L4.4: add resolve-missing dialog
- L4.5: add library status glyphs, filters and row actions
- L5.1: add link folder, match report and resolve dialogs
- L5.2: add library smart views and drawer timeline
- L6.1: handle windows extended paths and drive probing
- L6.2: document folder links and the archive workflow
- MATCH-FIX 1: add accent-insensitive name keys and folder style templates
- MATCH-FIX 2: persist folder style in settings
- MATCH-FIX 3: resolve exact matches before fuzzy scoring
- MATCH-FIX 4: use exact-first matching when relinking missing albums
- MATCH-FIX 5: remove noise-token and diacritic-table text handling
- MATCH-FIX 6: add folder naming settings to the classic UI
- MATCH-FIX 7: add folder naming settings to the fx UI
- MATCH-FIX 8: document folder styles and exact-first matching
- BADGE-REFORM 1: expand ArtistStats with eligible and non-eligible downloaded fields
- BADGE-REFORM 2: calculate on-page eligible and non-eligible download counts in queries
- BADGE-REFORM 3: render reformatted download badge in classic sidebar
- BADGE-REFORM 4: animate and render reformatted download badge in fx sidebar
- BADGE-REFORM 5: document reformatted sidebar download badge
- BADGE-REFORM 6: count local-only orphan albums in non-eligible downloads (migration `0005`, breaking)
- none: resolve Windows UNC prefix and 8.3 short paths in tests; handle root bypass when probing unreadable directories
- none: streamline release workflow
- none: single-pass `all_artist_stats` aggregation
- MISSING-FOLDER-FIX 1: report unavailable folders instead of staying silent
- MISSING-FOLDER-FIX 2: resolve unavailable folders as one undoable batch
- MISSING-FOLDER-FIX 3: ask what happened when a linked folder is gone (classic)
- MISSING-FOLDER-FIX 4: ask what happened when a linked folder is gone (fx)
- MISSING-FOLDER-FIX 5: document unavailable folder handling
- none: resolve unavailable folder when all releases are archived or empty
- none: visually differentiate archived releases on wall, drawer, and table; add sidebar archive icon
- none: release v0.1.1 bump
- none: release v0.1.2 bump
- none: pre-push hook installing bcvault-fx to ~/.local/bin and updating desktop entry; tracked .agents in git
- none: sync ROADMAP.md and AGENTS.md to list and track all plans including MULTI_FOLDER

## Milestones
- [x] Base engine (Phase 0 - 5)
- [x] FX (Phase F0 - F8)
- [x] L0: Model and repos
- [x] L1: Normalisation, tags, single-folder scanner
- [x] L2: Matcher, report, apply, undo
- [x] L3: Verify, policies, relink, sync integration
- [x] L4: Classic UI
- [x] L5: FX integration and polish
- [x] L6: Hardening
- [x] T0: guard checks updated (commit hook ceremony skipped per user decision)
- Note: no `milestone/*` tags exist; releases follow SemVer version tags (`v0.1.0` covers base, FX and library; `v0.1.1` and `v0.1.2` cover unavailable folder fixes, archived release display, pre-push tooling, and `.agents` tracking; current HEAD is tagged `v0.1.2`).

## Open questions (needs the user)
- 2026-11-08: create `milestone/F*` and `milestone/L*` tags retroactively, or record skipping them? (blocking: no)

## Decisions and deviations
- 2026-11-07: Fixed prefix bias in candidate matching by comparing folder title parts without artist prefix and replacing Jaro-Winkler with normalized Levenshtein and token-set Jaccard; tightened multi-folder conflict detection to prevent false cascades; displayed already-linked releases in ReconcileReport Matched tab.
- 2026-11-07: Clamped modal dialog windows to viewport bounds with max_size, truncated long titles and folder paths across report tabs with hover tooltips, and wired the Escape key to close the topmost open modal dialog.
- 2026-11-08: Applied MATCH-FIX plan: replaced NFKC/heuristic noise-stripping with exact-first matching architecture and user-declared FolderStyle templates + noise list. Exact matches resolve prior to fuzzy scoring via authoritative URL tags and connected components (only true duplicates/disagreements produce conflicts). Fuzzy scoring runs only on leftover items with digit multiset guard and containment capped at 0.85 (never auto-adopted). Persisted folder style in settings and exposed configuration editors in both Classic and FX UIs.
- 2026-11-08: Replaced `{downloaded}/{eligible}/{total}` sidebar badge with `{eligible dl} (+ {non-eligible dl})/eligible/page total`. Updated SQL queries to determine current page releases via latest sync timestamps and split downloaded releases into eligible vs. non-eligible (paid or off-page). Added spring motion in FX and detailed hover tooltips across both front-ends.
- 2026-11-08: Library work was committed directly on `main` (no `phase/L*` branches, no milestone tags). Migration numbers differ from the plan: `0003` was already taken by `custom_artist_name`, so library is `0004_library.sql`, and the badge rework added `0005_artist_folder_local_only.sql`. Planned: `0006` auto sync, `0007` archive, mail takes the next free number.
- 2026-11-08: L2.2 was committed as `feat(library): greedy 1-to-1 candidate assignment` (plan: `assign matches one-to-one and detect conflicts`); L1.5 (fixture builder) was folded into L1.3, as the plan allowed. `COMMIT_PLAN.md` updated to match.
- 2026-11-09: `.agents/` is now tracked in git (previously excluded via `.git/info/exclude`). Added `.githooks/pre-push` to automatically copy the newest compiled `bcvault-fx` binary to `~/.local/bin/` and generate/update `bcvault-fx.desktop` with standard audio category and generic icon.
- 2026-11-08: T0 tooling: integrated architectural guards (UI purity, square look, single gate, secrets in logs, migration immutability) into scripts/check.sh and scripts/guards.sh; skipped commit hook and commit message enforcement ceremony. Fixed synchronous settings JSON export in bcvault-fx to run off the UI thread via spawn_blocking.
- 2026-11-08: Applied MISSING-FOLDER-FIX plan: when an artist's linked folder or out_dir is unavailable (Missing or Unreadable), verify emits FolderUnavailable instead of silently keeping Unknown releases. Attribution checks each release against its true root so releases elsewhere verify normally. Unavailable roots are never auto-resolved, even with on_missing = Auto; the Resolve Unavailable Folder dialog provides Keep, Repoint, and Deleted policies with single-batch undo and optional unlinking. Added FOLDER n warning dock chips, Resolve unavailable folder... sidebar actions, and derived FOLDER MISSING badges in both classic and FX front-ends. Excluded Unknown releases from mass-missing ratio calculations.
- 2026-11-09: Fixed unavailable folder resolution stalling when releases under a missing folder are already archived or empty: `verify_files` now includes all unavailable linked folders in scope so `FolderUnavailable` is reliably emitted, allowing unlinking and repointing via the modal. Added monospace rectangular `ARCHIVED` badges on Wall view cards (matching `UPDATE`/`MISSING`), `ARCHIVED (FILES REMOVED)` status in detail drawer with suppressed "Open Folder" action, dimmed format labels in table view, and geometric `Archive` box icons in the artist sidebar for artists with archived releases.

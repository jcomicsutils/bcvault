# Plan: tabbed settings, settings parity and ZIP extraction options

For the coding agent. Repo: bcvault (Rust workspace). Scope: `bcvault-fx` (the settings modal moves out of `overlays/modals.rs`), `bcvault` (the settings dialog moves out of `ui/modals.rs`), a small addition to `bcvault-core` (`Settings` limits and `sanitize`), tests and docs. No migration, no new `Settings` field.

## 0. Ground rules

- **Branch.** `feat/settings-tabs` from the repository default branch. Never force-push.
- **Merge order.** This plan merges **first** among the pending plans. `AUTO_SYNC`, `ARCHIVE`, `USER_THEMES` and the mail UI each add settings controls, and all of them warned about conflicts in the monolithic modal. After this plan they add one tab file each (see §3.1 and `ROADMAP.md` §6).
- Conventional Commits, one commit per step in section 9, `Task: none`. Every commit compiles and passes `scripts/check.sh`. Non-test code must not use `unwrap` or `expect`.
- **UI-thread purity.** No `std::fs`, `rfd::FileDialog`, `block_on`, `rusqlite` or `reqwest` in UI directories (`scripts/guards.sh`). Folder pickers use `rfd::AsyncFileDialog` from a spawned task and return the result by channel, as the link-folder dialog already does. Tests that need the README or other files use `include_str!`, never `std::fs`.
- **Square look.** Zero corner radius; in `bcvault-fx` use only `fx/paint` helpers. The fx bans (`circle_`, `Shape::circle`, `rounding`, non-zero `corner_radius`, `Spinner`) apply to the tab strip and every new widget. Do not write the word `rounding` in fx source, comments included (the guard is a grep).
- **Keep behaviour.** Draft, Save and Cancel semantics, existing defaults, `#[serde(default)]` on every field, and the FX visual prefs (stored in `fx.sqlite`, applied immediately) stay as they are.
- Read first: `bcvault-fx/src/overlays/modals.rs` (the block from `if state.settings_modal_open` to the Save, Cancel and Export footer), `bcvault-fx/src/state.rs` (`settings_modal_open`, `settings_draft`, `close_top_modal`, `filename_pattern_input`), `bcvault-fx/src/views/toolbar.rs` (opens the dialog), `bcvault-fx/src/db/mod.rs` (`Prefs`), `bcvault/src/ui/modals.rs` (`show_settings_modal`), `bcvault/src/ui/state.rs`, `bcvault-core/src/types.rs` (`Settings`, `default_with_dir`), `worker/downloads.rs` and `worker/extract.rs` (extraction), `bcvault-core/tests/download_pipeline.rs`. Line numbers come from a snapshot taken before `MULTI_FOLDER` landed (its linked-folders list is not in this snapshot), so grep and re-read.

## 1. Current state (verified by reading the code)

- **fx**: one `egui::Window` ("bcvault-fx Settings", fixed 540 px, one `ScrollArea` capped at 540 px) built inline inside `show_modals`, about 470 lines. Sections in order: VISUALS & MOTION (theme, effects level, adaptive accent, reduced motion, grain, scanlines, texture budget; these write FX prefs immediately), DOWNLOADS & QUOTA (output directory as a plain text box, layout, filename pattern, preferred format, batch limit / soft / hard as three `DragValue`s), LIBRARY BEHAVIOUR, MATCHING THRESHOLDS, FOLDER NAMING, a linked-folders list. Footer: Save, Cancel, Export Settings JSON.
- **classic**: `show_settings_modal`, about 380 lines: Downloads & Storage (layout, filename pattern, format fallback, **Archive Extraction** checkbox, **Cleanup ZIP** checkbox that is disabled while extraction is off, cover thumbnails, page delay, page jitter, max concurrent downloads, soft quota, hard quota, user agent), Library (including thresholds), Folder Naming, Linked Artist Folders. Output folder, format and batch limit are in the toolbar, not in this dialog. Footer: Save, Copy Settings JSON, Cancel.
- **Parity gap.** The fx dialog cannot edit `format_fallback`, `auto_extract`, `delete_zip_after_extract`, `show_thumbnails` (fx never reads it), `page_delay_ms`, `page_jitter_pct`, `max_concurrent_downloads` or `user_agent`. So **fx users cannot turn ZIP extraction on or off at all**, although core honours both options.
- **Inconsistent limits.** fx edits `batch_limit` with `DragValue` range 1..=100, but the default is 150 and the documented set is 25, 50, 100, 150, 200 (classic uses a combo). Soft and hard quota ranges differ too (fx 1..=200 each, classic 25..=200 and 50..=300). Save clamps differ per UI (`quota_soft <= quota_hard`, `batch_limit <= quota_hard`, `review_match_min <= auto_match_min`, `folder_style.sanitize()` in fx; check classic).
- **Extraction behaviour in core** (`worker/downloads.rs`, `worker/extract.rs`): for albums, when `settings.auto_extract` is true the job state becomes `Extracting`, the ZIP is extracted next to itself into a unique directory (entry count and size caps, path-traversal checks), `local_path` becomes that directory, and when `delete_zip_after_extract` is true the ZIP is removed after a successful extraction. When `auto_extract` is false the release is marked downloaded with `local_path` pointing at the ZIP file; library scan and verify already understand ZIPs (`library/scan.rs`, `library/verify.rs`). Defaults: `auto_extract = true`, `delete_zip_after_extract = false`. If extraction fails the job fails and the ZIP stays on disk (the release is not marked downloaded); that behaviour is **not** changed here (open question in §8).
- Tracks are not ZIPs: single-track downloads ignore both options.
- `download_pipeline.rs` has tests that set `auto_extract = false`; none asserts `delete_zip_after_extract = true`.

## 2. Goals and non-goals

Goals:

- Both UIs show settings in **tabs**, each tab in its own file, with a small registry so later plans add a tab without touching others.
- Every user-editable `Settings` field is editable in **both** UIs (except fields explicitly listed as not shown, with a reason), and a test fails when a new field is added without being classified.
- fx gets the ZIP options (and the other missing controls). ZIP help text is explicit about what gets deleted.
- One `Settings::sanitize` in core replaces per-UI clamps; shared limit constants.
- Keyboard-friendly shell (tab switching, focus, Enter and Esc rules) so `KEYBOARD.md` only extends it.

Non-goals: new settings or changed defaults; per-artist or per-download overrides; a quick extraction toggle in the toolbar; changing what happens when extraction fails; theming content (`USER_THEMES.md` owns the Appearance tab's theme picker); moving FX prefs into `Settings`; localisation.

## 3. Specification

### 3.1 Tab registry (per UI crate)

Each crate gets a module (`bcvault-fx/src/overlays/settings/`, `bcvault/src/ui/settings/`) with `mod.rs` (shell: window, tab strip, footer, dirty state), one file per tab, and a registry:

```rust
pub enum SettingsTab { Appearance, Downloads, Network, Library, Folders, Advanced }
impl SettingsTab {
    pub const ALL: &[Self];            // display order; later plans append variants here
    pub const fn label(self) -> &'static str;
    pub const fn fields(self) -> &'static [&'static str]; // Settings JSON keys this tab edits
}
```

A tab file exposes `pub fn show(ui, ctx: &mut TabCtx)` where `TabCtx` carries the draft `Settings`, the theme/palette, the command senders and (fx) prefs. Adding a tab later is: one variant, one file, one `match` arm. Do not add generic plugin machinery.

| Tab | fx | classic | Contents |
| --- | --- | --- | --- |
| Appearance | yes | yes | fx: theme, effects, adaptive accent, reduced motion, grain, scanlines, texture budget (the current VISUALS & MOTION block, moved as is; `USER_THEMES.md` extends it). classic: cover thumbnails (`show_thumbnails`). |
| Downloads | yes | yes | Output folder (text box plus **Browse...**), layout, filename pattern with its tag help, preferred format, format fallback, and a **ZIP files** group: *Extract downloaded ZIP archives* (`auto_extract`) and *Delete the ZIP after successful extraction* (`delete_zip_after_extract`; disabled with a hint while extraction is off). Batch limit as the preset combo (25, 50, 100, 150, 200 clamped to the hard quota). |
| Network & quota | yes | yes | Soft and hard 30-day quota, page request delay, jitter, max concurrent downloads, user agent. Keep the conservative defaults; show the default next to each control and a **Reset** button per control group. |
| Library | yes | yes | Verify after sync, re-read linked folders, auto-relink, when files are missing, if deleted, and a collapsed **Matching thresholds (advanced)** group (auto and review minimum scores). |
| Folders | yes | yes | Folder naming (templates, noise list, presets, validation messages) and the linked-folders list, with a filter box and a virtualised list (`show_rows`) because it can hold hundreds of artists; with `MULTI_FOLDER` an artist can appear several times. Unlink buttons keep their current commands. |
| Advanced | yes | yes | Export or copy settings JSON (fx: the export runs through the existing off-thread helper), data and cache folder paths (read-only labels). |

Help text for the ZIP group, verbatim (fx and classic): *"Bandcamp album downloads arrive as .zip files. When extraction is off, the ZIP itself is kept as your download."* and *"Only the downloaded .zip is removed, and only after every file was extracted successfully. Off by default."*

### 3.2 Shell behaviour (both UIs)

- A horizontal strip of square tab buttons above a scroll area; Save, Cancel and the footer buttons stay outside the scroll area. Window width about 680, content height capped at about 80 percent of the screen.
- The active tab is remembered for the session. fx also persists it as a pref (`settings_tab`, through `FxCommand::SetPref`; unknown values fall back to Downloads).
- **Dirty marker.** A tab whose fields differ between draft and saved `Settings` shows a `•` after its label. A pure function compares the draft against `state.settings` per tab using `SettingsTab::fields`.
- **Esc.** With a clean draft, Esc closes as today. With a dirty draft the first Esc shows an inline bar "Unsaved changes: Save / Discard / Keep editing" (focus on Keep editing); Enter on the focused button activates it, and a second Esc means Keep editing. Never silently discard edits.
- **Keys.** `Ctrl+Tab` and `Ctrl+Shift+Tab` (also `Ctrl+PageDown` and `Ctrl+PageUp`) switch tabs while the dialog is open; `Left` and `Right` move between tab buttons when the strip has focus; opening the dialog and switching tabs focuses the first control of the tab; `Enter` on a single-line field does not trigger Save (it only commits the field); Save is reached with Tab or `Ctrl+S`. These are registered where the shell handles them so `KEYBOARD.md` can lift them into its registry.
- While the dialog is open the global single-key shortcuts are already suppressed (`has_open_modal`); keep that.

### 3.3 Core changes

- `types.rs`: constants `BATCH_LIMIT_PRESETS: [u32; 5]`, `QUOTA_SOFT_RANGE`, `QUOTA_HARD_RANGE`, `PAGE_DELAY_RANGE`, `PAGE_JITTER_RANGE`, `MAX_CONCURRENT_RANGE` and `Settings::sanitize(&mut self)` applying: every numeric field clamped to its range, `quota_soft <= quota_hard`, `batch_limit` snapped to a preset not above `quota_hard`, `review_match_min <= auto_match_min`, `folder_style.sanitize()`, empty `user_agent` replaced by the default, a blank `out_dir` rejected (left unchanged). Both UIs call it on Save and the worker calls it on `UpdateSettings` (defence in depth). Choose ranges that include every default (150 batch limit, 150 soft, 200 hard) and every value either UI could previously produce; check existing saved settings round-trip unchanged.
- No other core change. Do not touch the downloader.

### 3.4 Coverage test (the parity guard)

In each UI crate, a test serialises `Settings::default()` to a JSON object and compares its key set with the union of `SettingsTab::ALL[..].fields()` and a `NOT_SHOWN: &[(&str, &str)]` table of key plus reason (for example classic `out_dir` is shown in Downloads and the toolbar, so it is shown; a field intentionally absent must say why). The test fails on a missing, duplicated or unknown key. Adding a `Settings` field therefore requires deciding where it appears, in both crates, in the same commit (`AGENTS.md` rule 16).

## 4. Tests

| # | Area | Scenario | Expected |
| --- | --- | --- | --- |
| 1 | core | `sanitize` on out-of-range values, soft above hard, odd batch limit, blank user agent | Clamped and snapped as in §3.3; idempotent; defaults unchanged |
| 2 | core | Old saved JSON (without any new key) deserialises and `sanitize` leaves it equal | No behaviour change for existing users |
| 3 | downloads | `auto_extract = true`, `delete_zip_after_extract = false` | Directory exists, ZIP exists, `local_path` is the directory |
| 4 | downloads | `auto_extract = true`, `delete_zip_after_extract = true` | Directory exists, ZIP removed |
| 5 | downloads | `auto_extract = false` | ZIP kept, `local_path` is the ZIP, the release is marked downloaded |
| 6 | downloads | `delete_zip_after_extract = true` with a corrupt ZIP | Job fails, ZIP not deleted |
| 7 | downloads | Settings updated through `UpdateSettings` while a job is queued | The next job to start uses the new value (document the exact timing found in code) |
| 8 | ui state (both) | Tab switching, remembered tab, unknown persisted tab, dirty marker per tab, Esc state machine (clean, dirty, keep editing, discard, save) | As in §3.2 |
| 9 | ui state (both) | Opening the dialog copies `settings` into the draft; Save sends `UpdateSettings` with a sanitised copy; Cancel sends nothing | As today |
| 10 | coverage (both) | §3.4 | Passes; fails when a key is added to `Settings` |
| 11 | classic | Cleanup ZIP control is disabled while extraction is off and enabled when on | As today, now in the Downloads tab |

## 5. Decisions made for you (change only through `AGENTS.md` §8)

- Six tabs as in §3.1; Appearance exists in classic even though it holds one setting, so later plans have a home.
- `batch_limit` becomes the preset combo in both settings dialogs (the classic toolbar combo stays).
- Folder picker for the output folder is added to the Downloads tab (it was a bare text box in fx).
- Matching thresholds stay editable but collapsed under Library.

## 6. Interaction with other plans

- `AUTO_SYNC.md` adds a **Sync** tab (one variant, one file). `ARCHIVE.md` adds an **Archive** tab. `USER_THEMES.md` extends **Appearance**. The mail plan's settings dialog can stay a separate dialog opened from a **Mail** tab (one button plus status line) or become that tab; decide when it lands. Their plan files carry a short amendment saying so.
- `KEYBOARD.md` takes over the keys in §3.2 into its registry and adds focus-scope rules for modals.

## 7. Risks

- The fx modal mixes immediate prefs (FX) with the draft (`Settings`). Keep both mechanisms exactly as they are: tabs only change where controls are drawn.
- Moving 850 lines is a large diff. Do it in the move-only commits of §9 (no behaviour change), then add parity in separate commits so review stays tractable. Keep each commit under about 400 changed lines excluding pure moves, as `AGENTS.md` §5.4 allows.

## 8. Open questions (record in `PROGRESS.md`, not blocking)

- When ZIP extraction fails, the download is lost to the user's view (job failed, ZIP left on disk, release not marked downloaded, and the failed attempt is recorded in history). Should the release instead be marked downloaded with `local_path` at the ZIP and a warning? Not changed by this plan.
- Should extraction have a per-download override later? Not planned.

## 9. Commit sequence

One commit per step, every commit green.

1. `feat(types): sanitise settings in one place` adds the constants and `Settings::sanitize`, calls it from `UpdateSettings` handling, with tests 1 and 2.
2. `test(downloads): cover zip extraction and cleanup settings` adds tests 3 to 7 (reuse the existing wiremock pipeline fixtures).
3. `refactor(classic/settings): split the settings dialog into tabs` moves the existing controls into the registry and files with no behaviour change, adds the shell of §3.2 (tabs, dirty marker, Esc rules, keys), tests 8, 9 and 11 for classic.
4. `refactor(fx/settings): split the settings modal into tabs` the same for fx; VISUALS & MOTION becomes Appearance; persists `settings_tab`.
5. `feat(fx/settings): add the missing download and network settings` adds `format_fallback`, `auto_extract`, `delete_zip_after_extract`, `show_thumbnails` (shown in Appearance only if fx honours it, otherwise listed in `NOT_SHOWN` with that reason), page delay, jitter, concurrency, user agent, the batch-limit presets, the output-folder Browse button, plus the coverage test for fx.
6. `feat(classic/settings): add output folder, format and batch limit to Downloads` plus the coverage test for classic and its `NOT_SHOWN` table.
7. `docs(settings): document the tabs and ZIP options` updates `README.md`, `docs/` (a short Settings section), `AGENTS.md` rule references, `ROADMAP.md`, `PROGRESS.md`.

## 10. Definition of done

- `scripts/check.sh` green; test 10 fails when a field is added to `Settings` without classification.
- Both UIs open Settings on the last tab used; all six tabs render; Save persists and Cancel discards; Esc with edits asks first.
- **Manual (ask the user):** in each UI, toggle *Extract downloaded ZIP archives* off, download one small free release the user chooses, confirm a ZIP remains and the release shows as downloaded; toggle it on with *Delete the ZIP...* on, download another, confirm only the extracted folder remains. Never touch the live site without the user's go-ahead (`AGENTS.md` §8).
- `PROGRESS.md` updated in every commit; tag per `AGENTS.md` rule 13 when released.

# Plan: file-based themes (create, save, share) for bcvault-fx

For the coding agent. Repo: bcvault (Rust workspace). Scope: `crates/bcvault-fx` (theme module, settings UI, command palette, prefs), a small addition to `AppPaths` in `bcvault-core`, and docs. The classic `bcvault` UI is out of scope (see section 2).

## 0. Ground rules

- **Branch.** Create `feat/file-themes` from the repository default branch (find it with `git symbolic-ref refs/remotes/origin/HEAD`), not from another plan's branch, so the efforts stay independent. Never force-push or touch other branches.
- **Expected merge conflict.** The Folder naming section from the library-matching plan is already in `overlays/modals.rs` on `main`; the auto-sync and archive plans also edit the Settings modals. Keep this plan's modal changes inside the VISUALS & MOTION block and in new functions or files so conflicts stay small. Whichever branch merges second rebases.
- Conventional Commits, one commit per step in section 7. Every commit compiles and passes tests.
- Run `scripts/check.sh` before each commit. Non-test code must not use `unwrap` or `expect`.
- `scripts/check.sh` bans these in `crates/bcvault-fx/src`, and that includes every swatch and editor widget you add: `circle_`, `Shape::circle`, `rounding`, `corner_radius` (except `CornerRadius::ZERO`), `CornerRadius::(same|from|new)`, `Spinner`. Zero corner radius is a product rule, so theme files never carry radius settings.
- Theme files are untrusted input because users will share them. Follow the safety rules in 5.4.
- Read before editing: `theme/mod.rs`; `db/mod.rs` (`Prefs`, `get_prefs`); `actions.rs`; `state.rs` (`FxState::new`, the `SetTheme` handler, `active_theme`); `app.rs` (the block that builds the animated theme); `overlays/modals.rs` (theme selector, and how existing file dialogs with `rfd` are opened); `services/search.rs` (palette entries); `shader/mod.rs`; the tests in `views/sidebar.rs` and `views/toolbar.rs` that use `FxTheme::PAPER`; `bcvault-core/src/paths.rs`; `README.md`.
- Line numbers in this plan come from a snapshot. Grep, do not trust them.

## 1. Current state (verified by reading the code)

- `bcvault-fx/src/theme/mod.rs` defines `enum ThemeId { Carbon, Paper, Terminal, Vapor }` with `ALL`, `as_str`, `label` and `parse`, plus `FxTheme`, a `Copy` struct of 12 colors and `is_dark`, with four `const` palettes and `for_id`.
- `FxTheme` also provides `with_adaptive_accent` (clamps the accent to 4.5:1 against `bg1`) and `animate_through` (springs each color through `Motion` under `th_*` keys). Keep both behaviors unchanged.
- `ThemeId` is hard-wired in: `db/mod.rs` (`Prefs.theme`, `get_prefs` parses the `theme` key), `actions.rs` (`FxAction::SetTheme(ThemeId)`), `state.rs` (`new` and the `SetTheme` handler, which persists through `FxCommand::SetPref`), `app.rs` (`FxTheme::for_id(prefs.theme)`), `overlays/modals.rs` (loops `ThemeId::ALL`), `services/search.rs` (four hard-coded palette commands), and `shader/mod.rs`.
- `shader/mod.rs` forces the scanline overlay with `state.prefs.theme == ThemeId::Terminal`. That must become a theme property.
- Prefs are key/value rows in `fx.sqlite`. The existing `theme` values are `carbon`, `paper`, `terminal` and `vapor`, and they must keep working.
- There is no `toml` dependency (only `serde_json`), and `AppPaths` has no themes directory (`data_dir` honors `--data-dir` and `BCVAULT_DATA_DIR`).
- The classic UI has a single hard-coded dark palette (`pal::*`, 143 references) and no theme choice.
- The README says there are four built-in themes.

## 2. Goals and non-goals

Goals:

- Every theme is its own file. The 10 built-ins live in `themes/` and are embedded in the binary.
- Users can create, edit, save, delete, import and export their own themes, and share one by sending a single `.toml` file.
- A bad or hostile shared file can never crash the app or break the UI.
- Existing stored `theme` preference values keep working.

Non-goals:

- Making the classic `bcvault` UI themeable (a separate, larger refactor).
- Fonts, spacing, corner radius or motion in theme files.
- Theme inheritance (`extends`), a file watcher, an online gallery, per-artist themes, or importing from the clipboard.

## 3. Theme file format

TOML, one theme per file, format version 1. Example (this is `carbon.toml`, with the values from today's `FxTheme::CARBON`):

```toml
# bcvault theme, format 1. Share this file as is.
format = 1
id = "carbon"
name = "Carbon"
description = "Dark neutral slate theme"
author = "bcvault"
dark = true
scanlines = false

[colors]
bg0 = "#0A0A0B"
bg1 = "#121315"
bg2 = "#1B1D20"
border = "#3A3D42"
border_hi = "#8A8F98"
text = "#ECECEC"
text_dim = "#9AA0A6"
accent = "#00D1B2"
accent2 = "#3DDC84"
ok = "#3DDC84"
warn = "#FFB300"
err = "#FF5252"
```

Validation (errors reject the file; the message must name the key and the reason):

- At most 16 KiB, valid UTF-8, valid TOML.
- `format` must equal 1. A larger value gives the message that the theme was made by a newer bcvault.
- `id`: 1 to 32 characters from `a-z`, `0-9` and `-`, with no leading, trailing or doubled hyphen.
- `name`: 1 to 40 characters after trimming, no control characters. `description` (optional) at most 160 characters, `author` (optional) at most 40.
- `dark`: bool, required. `scanlines`: bool, optional, default false (the overlay that today is hard-wired to Terminal).
- `[colors]`: all 12 keys present, each exactly `#RRGGBB` (case-insensitive, no alpha, no shorthand).
- Unknown keys are ignored and reported as a warning, which keeps old versions able to open newer files.

Non-blocking hints (shown in the editor and in the load report; never block loading a user theme; the same rules are hard test failures for built-ins):

- `text` and `text_dim` against each of `bg0`, `bg1`, `bg2` at least 4.5:1.
- `accent` against `bg1` at least 4.5:1.
- `accent2`, `ok`, `warn`, `err` against `bg1` at least 3.0:1.
- `dark` should agree with the background: relative luminance of `bg0` below 0.4 means dark.

Contrast uses the existing `motion::oklab::contrast_ratio`. Serialization writes a fixed key order with uppercase hex and the header comment above, so export and re-import round-trips exactly.

## 4. Architecture

Split `theme/` into modules. Keep `FxTheme`, `apply_theme` and `install_fonts` in `theme/mod.rs`.

- `theme/file.rs`: `ThemeFile` (serde model), `parse(&str) -> Result<ThemeFile, ThemeError>`, `to_toml_string`, validation, hints, `slugify`.
- `theme/builtin.rs`: the ordered list of `include_str!` lines for the 10 files in `crates/bcvault-fx/themes/`. The files live inside the crate so `include_str!` paths stay crate-relative.
- `theme/registry.rs`: `ThemeRegistry`, `ThemeEntry { id, name, description, author, source: Builtin | User(PathBuf), theme: FxTheme, hints }`, and `LoadIssue { path, message, severity }`.

Registry rules:

- Built-ins come first in a fixed order: carbon, paper, terminal, vapor, midnight, ember, dusk, contrast, daylight, sage. User themes follow, sorted by name (case-insensitive), then by id.
- `get(id)` returns an entry or nothing. `resolve(id)` falls back to `carbon`. An unknown stored id falls back silently, without rewriting the preference, so a restored file works again.
- A user theme whose id equals a built-in id, or equals an earlier user file (files sorted by name), is skipped with a `LoadIssue`. A user directory holds at most 100 themes; extra files are skipped with an issue.
- `ThemeRegistry::builtin_only()` does no I/O, and `load_user_dir(&Path) -> Vec<LoadIssue>` scans `*.toml` only (non-recursive, symlinks ignored, size checked before reading).
- Keep a `FxTheme::FALLBACK` const (Carbon's colors) so a theme always exists even if embedded parsing ever failed, and test that `carbon.toml` equals it.

Other changes:

- `FxTheme` gains `scanlines: bool`. `animate_through` copies it like `is_dark`. Remove the four consts and `for_id` once nothing uses them. Provide a test helper so the sidebar and toolbar tests can build the Paper theme from the registry.
- `Prefs.theme` becomes `String` (default `carbon`). `get_prefs` accepts any non-empty string up to 32 characters, and the stored key stays `theme`, so old databases keep working. `FxAction::SetTheme` takes a `String`; check whether `FxAction` derives `Copy` and adjust.
- `FxState` gets `themes: ThemeRegistry` (built with `builtin_only()` in `FxState::new`, so tests and `fx_bench` need no I/O) and `theme_preview: Option<FxTheme>`. One function resolves the target theme as preview if set, otherwise the registry entry for `prefs.theme`. `app.rs` feeds that result into the existing adaptive-accent and `animate_through` path. Adaptive accent is skipped while a preview is active.
- `AppPaths` gains `themes_dir = data_dir.join("themes")`, created by the existing directory-creation step. Fix any struct literals or tests that construct `AppPaths` directly.
- At startup the app calls `load_user_dir` once. File I/O happens on the UI thread only at boot and on explicit user actions (reload, save, import, export, delete), and the files are tiny. Use the existing `rfd` pattern for dialogs.
- `shader/mod.rs` uses `state.active_theme.scanlines` instead of the Terminal check.
- `services/search.rs` builds one `Theme: <name>` palette command per registry entry (the description comes from the file) instead of four literals. Thread the registry through if the builder lacks access.

## 5. User-facing behavior

### 5.1 Theme picker (Settings, VISUALS & MOTION)

- Replace the `ThemeId::ALL` row with a wrapped grid. Each entry is a selectable item with three square swatches (`bg1`, `accent`, `accent2`) and the name. The tooltip shows description and author. Sections are labelled Built-in and Yours.
- Buttons: New theme (duplicates the active theme into the editor), Edit (user themes) or Duplicate (built-ins), Import, Open themes folder (use the `open` crate that is already a dependency), Reload.
- If loading produced issues, show a collapsible list `N theme files had problems` with the file name and message of each.

### 5.2 Theme editor

A modal window in the existing style.

- **Fields:** name, description, author, a `dark` toggle and a `scanlines` toggle. The id is derived from the name on creation (`slugify`: lowercase, non-alphanumeric runs become `-`, trimmed, at most 32 characters, empty becomes `theme`). If it collides with any existing id, append `-2`, `-3` and so on, shortening the base to stay within 32 characters. The id is immutable after creation and shown read-only.
- **Colors:** one row per color with a label, an `egui` color button and a hex text field. An invalid hex is flagged inline and never applied.
- **Live preview:** valid edits update `state.theme_preview`, so the whole app re-themes as the user edits.
- **Hints:** contrast and `dark` hints appear inline per rule.
- **Buttons:** Save, Cancel (clears the preview), Export, and Delete for an existing user theme.

### 5.3 Save and delete

- Save re-validates, serializes with `to_toml_string`, writes to a temporary file in `themes_dir`, then renames over `<id>.toml` (atomic replace). Editing a user theme that was loaded from a differently named file saves back to that file's path. After saving, reload the registry and select the theme.
- Delete asks for confirmation and removes the file. If the deleted theme was active, dispatch `SetTheme("carbon")` so the fallback is persisted.
- Built-ins are read-only: no edit or delete, only Duplicate and Export.

### 5.4 Import, export, share, and safety

- **Export** opens a save dialog with the default name `<id>.toml` and writes `to_toml_string`. It works for built-ins too. **Copy as text** puts the same TOML on the clipboard (`ctx.copy_text`).
- **Import** opens a multi-select file dialog for `.toml`. Each file is size-checked, parsed and validated, then saved to `themes_dir` under its own id by re-serializing the parsed values, never copying raw bytes. Import never overwrites: if the id exists (built-in or user), import under a new id (`-2`, `-3`) and say so in the result. Finish with a summary such as `Imported 3, skipped 1 (see problems)`.
- **Safety:** a theme file carries only text and colors. It contains no paths and nothing is executed. Names and descriptions are drawn as plain text. The id is the only thing used in a file name, and it is limited to `[a-z0-9-]`, so path traversal is impossible. Reject files over 16 KiB before reading all of it, and ignore anything that is not a regular `.toml` file.
- **Sharing** means sending the `.toml` file. `docs/themes.md` explains where the folder is (`<data_dir>/themes`) and how `--data-dir` and `BCVAULT_DATA_DIR` change it.

## 6. Built-in themes (10)

The four existing themes move into files with exactly today's values: `carbon.toml`, `paper.toml`, `terminal.toml` (with `scanlines = true`, which preserves today's behavior) and `vapor.toml`. Keep the current descriptions: Dark neutral slate theme; High-contrast warm paper theme; Phosphor green terminal theme; Deep violet and neon cyan theme. Built-in author is `bcvault`.

Six new themes. I checked all of these against the rules in section 3 using the same contrast formula as `contrast_ratio` (minimum text contrast 12.2:1, minimum accent contrast 5.1:1). Use the values as given.

| Key | midnight | ember | dusk | contrast | daylight | sage |
| --- | --- | --- | --- | --- | --- | --- |
| name | Midnight | Ember | Dusk | Contrast | Daylight | Sage |
| description | Deep navy with a clear blue accent | Warm charcoal with a molten orange accent | Dark plum with rose and violet accents | Maximum-contrast black and white with a yellow accent | Cool light theme with a blue accent | Soft green-grey light theme with a forest accent |
| dark | true | true | true | true | false | false |
| bg0 | #070B14 | #0F0A07 | #120A12 | #000000 | #F7F9FC | #F3F6F0 |
| bg1 | #0D1424 | #1A110C | #1C101D | #0A0A0A | #EAEFF6 | #E5ECE0 |
| bg2 | #16203A | #271A12 | #2A1A2C | #1A1A1A | #DCE4EF | #D5DFCD |
| border | #2A3A5E | #4A3022 | #4D2F50 | #B0B0B0 | #9BA8BC | #8FA284 |
| border\_hi | #5C7BC0 | #C27A3E | #C084C8 | #FFFFFF | #3B4A63 | #2F4A2A |
| text | #E6ECFA | #F5E9DF | #F4E8F4 | #FFFFFF | #14202E | #172016 |
| text\_dim | #98A6C8 | #B59C8A | #B89BBA | #D0D0D0 | #4A5A70 | #4B5D47 |
| accent | #4DA3FF | #FF8A3D | #FF7AA8 | #FFD60A | #0B5FD0 | #1F6B3A |
| accent2 | #7CE0D3 | #FFC24D | #B794F6 | #00E5FF | #0A6F65 | #8A5A00 |
| ok | #4ADE80 | #A3E635 | #6EE7B7 | #00FF7F | #17702F | #17692F |
| warn | #FBBF24 | #FBBF24 | #FCD34D | #FFB000 | #9A5B00 | #8F5A00 |
| err | #FB7185 | #FF6B6B | #FF6B81 | #FF5C5C | #C62828 | #B3261E |

`scanlines` is false for all six. The full set is seven dark and three light themes.

## 7. Commit sequence

Each step leaves the workspace compiling with all tests green.

1. `feat(theme): add theme file format with parsing and validation` adds the `toml` dependency (workspace and `bcvault-fx`), `theme/file.rs`, `slugify` and hints, with tests, alongside the old code.
2. `feat(theme): add ten built-in theme files and registry` adds `crates/bcvault-fx/themes/*.toml` (4 migrated plus 6 new), `builtin.rs`, `registry.rs` with `builtin_only()`, `FxTheme.scanlines` and `FALLBACK`. A temporary test compares the four migrated files with the old consts and is deleted in step 3.
3. `refactor(fx): resolve themes through the registry instead of ThemeId` replaces the enum with string ids across `db`, `actions`, `state`, `app`, the settings selector, the palette (one command per theme), the shader scanlines flag and the two view tests. It removes `ThemeId` and the consts and updates the README count to ten.
4. `feat(theme): load user themes from the data directory` adds `AppPaths.themes_dir`, `load_user_dir`, load issues, collision and limit rules, and boot-time loading.
5. `feat(fx): theme picker with swatches, reload and open folder` implements 5.1.
6. `feat(fx): theme editor with live preview and save` implements 5.2 and 5.3.
7. `feat(fx): import, export, copy and delete user themes` implements 5.4 and the delete flow.
8. `docs(theme): document theme files and sharing` adds `docs/themes.md` (format reference, folder location, how to share, the ten themes) and links it from the README.
9. Optional: `feat(fx): import themes dropped onto the window` using the files dropped on the egui window, through the same import path.

## 8. Tests

| # | Area | Scenario | Expected |
| --- | --- | --- | --- |
| 1 | file | Parse the carbon example; serialize and parse again | Equal values; output is byte-stable |
| 2 | file | Each failure: missing key, bad hex (`#FFF`, `red`, alpha), wrong type, id with uppercase or `/` or `..`, empty or over-long name, format 2, over 16 KiB, invalid UTF-8 | Rejected with a message naming the key |
| 3 | file | Unknown extra key | Accepted, one warning |
| 4 | file | `slugify` cases: spaces, symbols, accents, empty, over 32 characters; collision suffixes | Valid ids within 32 characters, unique |
| 5 | file | Hints: low-contrast text; `dark` set against a light background | Hints produced; loading still succeeds |
| 6 | builtin | Every `themes/*.toml` in the directory is registered (read the directory with `CARGO_MANIFEST_DIR`) | No file is missing from `builtin.rs`, and no entry lacks a file |
| 7 | builtin | Ids equal file stems, ids and names unique, exactly 10, fixed order | Passes |
| 8 | builtin | Contrast rules from section 3 for all 10; `dark` agrees with `bg0` luminance | Passes |
| 9 | builtin | `scanlines` is true only for `terminal`; `carbon.toml` equals `FxTheme::FALLBACK` | Passes |
| 10 | builtin | Zero corner radius after `apply_theme` for every theme (extend the existing test) | Passes |
| 11 | registry | User dir with a valid theme, an invalid theme, a built-in id clash, two files with the same id, a non-`.toml` file, a subfolder, a symlink, and 101 files | Valid one loaded; the rest skipped with issues; limit enforced |
| 12 | registry | Ordering and `resolve` fallback for an unknown id | Built-ins first, user sorted; fallback to carbon without changing prefs |
| 13 | prefs | Old stored values `paper`, `terminal` and an unknown string | Resolve to the right themes or the fallback |
| 14 | save | Save then reload; the atomic temporary file is gone; editing a differently named file saves in place | Passes |
| 15 | import | Valid file; id clash with a built-in; id clash with a user theme; batch with one bad file | Never overwrites; new id suffix; summary counts correct |
| 16 | export | Export then import round trip | Identical colors and metadata |
| 17 | delete | Delete the active user theme | File gone; active falls back to carbon and the pref is persisted |
| 18 | palette | One `Theme:` command per registry entry | Count and labels correct |
| 19 | state | Preview overrides the resolved theme; cancelling restores it; adaptive accent is off during a preview | Passes |

Also run the whole workspace test suite, `cargo run --release -p bcvault-fx --example fx_bench -- --check`, and grep the README and the other docs for the old four-theme claims.

## 9. Definition of done

- `scripts/check.sh` passes on every commit.
- Ten theme files exist in `crates/bcvault-fx/themes/` and nothing else defines their colors.
- A user can create, edit, save, delete, import, export and share a theme without leaving the app, and a malformed file never crashes the app or blocks the others.
- Databases from before the change load with their theme intact.
- Terminal still shows scanlines, now because its file says so.

## 10. Decisions already made (do not re-litigate; flag if impossible)

- Scope is `bcvault-fx` only. The classic UI keeps its fixed palette.
- TOML format with the new `toml` dependency. A theme file has exactly the 12 colors plus `dark`, `scanlines` and metadata, and no inheritance.
- Built-in files live in `crates/bcvault-fx/themes/` (inside the crate, so embedding paths stay crate-relative). User files live in `<data_dir>/themes/`.
- Ids are immutable after creation. Import never overwrites. Built-ins are read-only.
- Contrast and `dark` checks are warnings for user themes and hard test failures for built-ins.
- The stored preference key stays `theme` and holds the theme id.

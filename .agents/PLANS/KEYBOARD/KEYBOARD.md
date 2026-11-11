# Plan: complete keyboard operation

For the coding agent. Repo: bcvault (Rust workspace). Scope: `bcvault-fx` (most of the work), `bcvault` (parity for tables, sidebar, dialogs), no `bcvault-core` change beyond what a step names, tests and docs. No migration.

Goal: every operation the app offers can be done without a mouse, discoverably, in both front-ends.

## 0. Ground rules

- **Branch.** `feat/keyboard` from the repository default branch. Never force-push.
- **Merge order.** **Last** of the four plans written together (`SETTINGS_TABS.md`, `DOWNLOAD_FLOW.md`, `ARTIST_GROUPS.md`, this one), so it audits their final UI: the settings tabs, the selection bar and quota dialogs, the grouped sidebar rows. Those plans each exposed their operations as `FxAction`s and palette commands on purpose; this plan binds, completes and verifies them. If this plan lands earlier, the later plans must register their keys in the registry of §3.1 and satisfy `AGENTS.md` rule 15.
- Conventional Commits, one commit per step in section 9, `Task: none`. Every commit compiles and passes `scripts/check.sh`. Non-test code must not use `unwrap` or `expect`.
- **UI-thread purity and square look** apply (`scripts/guards.sh`). Focus rings and highlights use `fx/paint` helpers, zero corner radius, no circles; do not write the word `rounding` in fx source. README checks use `include_str!`, never `std::fs`.
- **Do not fight egui.** Text inputs, combo boxes, buttons and Tab traversal keep egui's behaviour. Our shortcuts act only when no text input has keyboard focus (the existing `ctx.wants_keyboard_input()` guard) and never steal `Tab`, `Enter` or `Space` from a focused widget.
- Read first: `bcvault-fx/src/app.rs` (`handle_keyboard`, the `Esc` layering, `has_open_modal`), `nav.rs`, `actions.rs`, `state.rs` (`apply_action`, `focused_key`, `focus_move`, `pending_scroll_to_y`, `close_top_modal`), `services/search.rs` and `overlays/palette.rs` (commands, scopes, hints), `views/{wall,table,artists,sidebar,toolbar,dock}.rs`, `overlays/{drawer,modals,toasts}.rs`; in classic `app.rs`, `ui/{release_table,sidebar,toolbar,bottom_dock,modals,state}.rs`; `README.md` shortcut tables. Line numbers come from a snapshot, so grep.

## 1. Current state (verified by reading the code)

**fx** has a real shortcut layer, hard-coded in `handle_keyboard`: `Ctrl+K` or `Ctrl+P` palette, `Ctrl+B` sidebar, `F3` debug, `F11` fullscreen, `Alt+Left` and `Alt+Right` history, `Esc` layered (palette, drawer, modal, selection), `/` or `Ctrl+F` search, `1` to `4` views, `+` and `-` card size, `[` and `]` previous and next artist, arrows or `hjkl` move the focused wall card, `Enter` opens the drawer, `Space` toggles selection, `D` downloads, `O` opens in the browser. Shortcuts run only when no modal or palette is open and no text widget wants input. The palette has scopes (`Tab` cycles) and a static command list with hint strings written by hand.

Gaps, all verified:

- **Wall navigation** assumes four columns: Up and Down move the focus by a fixed 4 cards regardless of the real column count or grouped layouts (`GroupBy` restarts rows per section). The focused card is **not scrolled into view**. No `Home`, `End`, `PageUp`, `PageDown`, no `Shift+Arrow` range select, no `Ctrl+A`.
- **Table, Artists and Stats views** have no focus ring and no keyboard navigation at all (`focused_key` is used only by the wall).
- **Sidebar**: rows are painted widgets with no arrow navigation, no visible keyboard focus, and `[` `]` changes the artist without scrolling the sidebar to it.
- **Context menus** (wall cards, table rows, sidebar rows, dock jobs) open only on secondary click; there is no keyboard route to their actions.
- **Toolbar and dock**: no shortcuts for sort, filter, group, sync, settings or for choosing and cancelling queue jobs.
- **Modals** (about a dozen `egui::Window`s): `Esc` closes the top one, but nothing sets initial focus, `Enter` rarely confirms, and background widgets stay focusable, so `Tab` can walk into hidden UI behind a dialog.
- **Docs drift**: `README.md` says `[` and `]` change card size and that `Ctrl+Enter` enqueues; the code uses `+` and `-`, `[` and `]` for artists, and has no `Ctrl+Enter`. Palette hints are separate hand-written strings.
- **classic** has only `F3`, `Ctrl+F`, `Space` (toggle the last clicked row) and `Enter` (download the selection). No arrow navigation, row focus ring, sidebar navigation, palette or shortcut help.

## 2. Goals and non-goals

Goals:

1. A **shortcut registry** per UI crate as the single source of truth for keys, palette hints, the in-app help and the README table, with a conflict test.
2. **Focus regions** (sidebar, toolbar, content, dock, drawer) with `F6` cycling and a visible focus ring.
3. Full **navigation** in the wall, table, Artists view, sidebar (including groups), dock and drawer; scroll-into-view everywhere; multi-select keys.
4. **Item actions** (everything the right-click menus offer) reachable from the keyboard, generated from one list so menus and keyboard cannot drift.
5. **Modal focus rules**: initial focus, `Enter` and `Esc`, no focus leaking to the background, focus restored on close.
6. A `?` / `F1` **help overlay** generated from the registry.
7. **classic parity** for tables, sidebar, dialogs and global shortcuts.
8. A keyboard-only walkthrough that is the acceptance test for the whole app.

Non-goals: user-remappable keys (the registry makes it possible later); a screen-reader audit (note any AccessKit gaps in `PROGRESS.md` only); touch gestures; a palette in classic; changing existing key meanings except where §3 says so.

## 3. Specification

### 3.1 Shortcut registry

Per crate (`bcvault-fx/src/keys.rs`, `bcvault/src/ui/keys.rs`):

```rust
pub struct Chord { pub key: Key, pub ctrl: bool, pub shift: bool, pub alt: bool }
pub enum KeyContext { Global, Content, Sidebar, Dock, Drawer, Modal, Palette }
pub struct Binding { pub action: ActionId, pub chords: &'static [Chord], pub context: KeyContext,
                     pub label: &'static str, pub group: &'static str }
pub fn resolve(events: &[Chord], ctx: KeyContext, text_focus: bool) -> Option<ActionId>;   // pure
```

- `resolve` is a pure function over a small chord type; the egui `InputState` is converted at the edge. A binding with no modifier is ignored when `text_focus` is true; chorded bindings with `Ctrl` still work in text fields only if they cannot collide with text editing (`Ctrl+A`, `Ctrl+C`, `Ctrl+V`, `Ctrl+X`, `Ctrl+Z` are never taken from a focused text input).
- `handle_keyboard` becomes a thin loop over `resolve`. Every existing binding moves into the table first, with identical behaviour (commit 1 is a pure refactor).
- **Conflict test:** no chord is bound twice in overlapping contexts (`Global` overlaps every other context).
- **Palette coverage test:** every `ActionId` that is user-visible has a palette command, and palette hints are produced from the registry (delete the hand-written hint strings).
- **README block:** the shortcut tables in `README.md` between `<!-- keys:start -->` and `<!-- keys:end -->` are generated from the registry; a test compares them with the registry using `include_str!("../../../README.md")` and fails on drift. Fix the existing drift as part of this.

### 3.2 Focus regions

`Region { Sidebar, Toolbar, Content, Dock, Drawer }`. Each owns one `egui::Id`; "the region has focus" means egui's focus is parked on that id, so Tab, `F6` and our arrows share one focus system (clicking a region, opening the drawer or pressing `F6` parks focus there). `F6` and `Shift+F6` cycle the visible regions in the order Sidebar, Toolbar, Content, Dock; `Esc` in the sidebar, dock or drawer returns to Content. The active region draws a 2 px square outline in the accent colour (disabled-motion safe: no animation). Inside Toolbar and modals egui's own `Tab` traversal applies. Pure state machine for the cycling and `Esc` rules, unit-tested.

### 3.3 Content navigation

- **Wall:** spatial navigation using the layout's card rectangles: Left and Right move by one card (wrapping to the previous or next row), Up and Down pick the card in the adjacent row with the greatest horizontal overlap (works in grouped layouts and with a partial last row); `Home` and `End` first and last; `PageUp` and `PageDown` move by the viewport's row count. After every move, **scroll the focused card into view** (reuse `pending_scroll_to_y`; respect reduced motion by snapping). `hjkl` stay as aliases.
- **Selection keys:** `Space` toggles the focused item; `Shift+Arrow`, `Shift+PageUp`, `Shift+PageDown`, `Shift+Home`, `Shift+End` extend the selection from an anchor (core `selection::range_keys`); `Ctrl+Arrow` moves focus without changing selection; `Ctrl+A` select all, `Esc` clear (both from `DOWNLOAD_FLOW.md`).
- **Table:** a focus row with the same outline, Up and Down by one, `Home`, `End`, `PageUp`, `PageDown`, scroll via `TableBuilder::scroll_to_row`, same selection keys. **Artists view:** spatial navigation, `Enter` dives into the artist. **Drawer:** `Left` and `Right` step releases (existing `DrawerStep`), `Tab` walks its buttons, `D`, `O` and open-folder keys act on the shown release, `Esc` closes.
- **Download keys:** `D` downloads the selection (or the focused item when nothing is selected), `Shift+D` downloads remaining in the view (confirmation rules from `DOWNLOAD_FLOW.md`), `Ctrl+Shift+D` opens the "Download remaining for all artists" confirmation. Re-download has no single-key binding (palette and menu only).

### 3.4 Sidebar navigation (uses the row model of `ARTIST_GROUPS.md`)

Region Sidebar: Up and Down move a cursor over `SidebarRow`s (All artists, group headers, artists), with scroll-into-view; `Enter` selects the artist (or "All artists"); on a group header `Left` collapses, `Right` expands and `Enter` toggles; `Space` toggles the artist in the multi-selection (classic) or selects it (fx); `Home` and `End`; **type-ahead**: letters typed within 600 ms jump to the next artist whose display name starts with them (name keys from `textmatch::key`); `/` focuses the artist filter box while the region is Sidebar (otherwise it focuses release search); `[` and `]` still change artist from anywhere and now scroll the sidebar to it. A pure cursor-movement function over `Vec<SidebarRow>` is unit-tested including collapsed groups and filters.

### 3.5 Item actions without right-click

- One function per target builds the action list that right-click menus show today: `context_actions(target: Target, state) -> Vec<ItemAction>` where `Target { Release(ItemKey), Artist(i64), Job(JobId), Group(i64) }` and `ItemAction { label, action: FxAction, enabled }`. The existing right-click menus are rewritten to render this list (so the two cannot differ).
- **fx:** `.` or `Shift+F10` (and the Menu key if egui exposes it) opens the palette in a new scope **Actions** listing `context_actions` for the focused target (the focused release in Content, the cursor row in Sidebar, the selected job in Dock). Typing filters; `Enter` runs.
- **classic:** `Shift+F10` opens a small "Actions" window with the same list (`Up`, `Down`, `Enter`, `Esc`); right-click keeps the popup menu.

### 3.6 Dock

Region Dock: Up and Down select a job, `C` or `Delete` cancels it, `R` retries, `X` clears finished, `Tab` cycles the Queue, Log and Quota tabs, `Enter` on a job opens its release in the drawer. The fx dock must not clone the job vector per frame while doing this (carry the cursor by `JobId`).

### 3.7 Global operations

Bind (and add palette commands for) the remaining operations so nothing needs the pointer: `Ctrl+,` settings, `Ctrl+N` focus the add-artist box (or open the add-artist dialog), `Ctrl+R` sync the active artist (incremental), `Ctrl+Shift+R` sync all (incremental; the existing politeness gates apply), `Ctrl+L` link or add a folder for the active artist, `Ctrl+Shift+V` verify local files for the active artist, `Ctrl+Q` quit, `Ctrl+Tab` and `Ctrl+Shift+Tab` for tab strips inside dialogs. Sort, filter, group-by and card size remain palette commands (`#` filters exist) with direct keys only where a registry slot is free: `S` cycles sort, `G` cycles group-by, `F` cycles filter in Content. Resolve the final chords in the registry conflict test; if any suggestion above collides, keep the existing binding and use the palette.

### 3.8 Modal focus scope

A helper per crate, `modal_scope(ui, id, default_focus, primary, cancel)`:

- On open, focus goes to `default_focus` (first text field, otherwise the primary button; **safe actions default to Cancel** for confirmation dialogs that can change data: delete group, quota continue, library resolve, bulk confirm).
- `Enter` runs the primary action when the focused widget is not a multi-line editor and has not consumed it; `Esc` runs cancel (the existing `close_top_modal` stays the dispatcher).
- While any modal is open the main panels are drawn disabled (`ui.add_enabled_ui(false, ..)`), so their widgets are not focusable and `Tab` stays inside the dialog; custom-painted widgets must allocate with `Sense::hover()` when `!ui.is_enabled()`.
- When the last modal closes, focus returns to the region and item that had it.
- Inventory to convert (one row each in the tests): settings (tabs, `Ctrl+Tab`), link folder, reconcile report, resolve missing, resolve unavailable folder, unlinked-folders match, redeem URL, quota list, bulk confirm, remove artist, rename artist, group editor, help overlay, palette.

### 3.9 Help overlay

`?` or `F1` (fx and classic) opens a searchable list of every binding grouped by context, generated from the registry; it is itself a modal under §3.8.

### 3.10 Walkthrough (acceptance)

A checklist table in `docs/keyboard.md`, each row a task with the key sequence, executed by the agent in both UIs and by the user at the end: add an artist; search; change sort, filter and group-by; walk the wall and the table; select several and select all; download selected; download remaining; open the drawer and step releases; mark downloaded; open the folder; link a folder; resolve a missing-files dialog; change a setting on each tab and save; sync an artist and sync all; select a job, cancel and retry; read the log; create a group, move an artist, collapse it; open the help overlay; quit. Any step that needs the mouse is a bug to fix before release.

## 4. Tests

Pure logic gets unit tests; egui behaviour gets a small number of end-to-end tests if `egui_kittest` (same minor version as `egui`) works as a dev-dependency here without a GPU; if it does not, keep the egui layer thin, test the pure parts and record the limitation in `PROGRESS.md`.

| # | Area | Scenario | Expected |
| --- | --- | --- | --- |
| 1 | registry | Conflict test over all bindings and contexts | None; fails when a chord is double-bound |
| 2 | registry | Every `ActionId` has a palette command; hints come from the registry | Complete; no hand-written hints remain |
| 3 | registry | README block equals the generated block | Passes; fails on drift |
| 4 | registry | `resolve` with `text_focus = true` for plain and `Ctrl` chords | Plain keys ignored; `Ctrl+A` and friends never taken from text inputs |
| 5 | regions | `F6` and `Shift+F6` cycling with hidden sidebar or dock; `Esc` returns to Content | As §3.2 |
| 6 | wall | Spatial neighbour function: ragged last row, grouped layout with restarted rows, 1 to 12 columns, empty view | Correct index; never out of range |
| 7 | wall | Page moves and `Home` and `End`; focus rectangle outside the viewport | Scroll request issued; snap with reduced motion |
| 8 | selection | `Shift+Arrow` range from the anchor in both directions; `Ctrl+Arrow` leaves selection | As §3.3 |
| 9 | table | Row cursor, paging, `scroll_to_row` request | As §3.3 |
| 10 | sidebar | Cursor over rows with collapsed groups and a filter; type-ahead with accents and a timeout | As §3.4 |
| 11 | actions | `context_actions` for each target; right-click menu and palette scope render the same list | Identical |
| 12 | dock | Cursor by `JobId` survives job updates and removals | No jump, no clone per frame |
| 13 | modal | Each inventory item: initial focus target, `Enter`, `Esc`, background disabled, focus restored | As §3.8 |
| 14 | help | Overlay lists every registry binding once | Complete |
| 15 | classic | Table cursor, selection keys, sidebar cursor, `Shift+F10` actions window, global keys | As §3.3 to §3.7 |
| 16 | e2e (if kittest) | Keyboard-only: open the app with fixture data, move, select, open the drawer, queue a download, open settings, switch a tab, close | Passes |

## 5. Decisions made for you (change only through `AGENTS.md` §8)

- `F6` for region cycling (browser and file-manager convention), `?` or `F1` for help.
- Context actions through the palette in fx and a small window in classic.
- Existing key meanings stay (`hjkl`, `D`, `O`, `+` and `-`, `[` and `]`); README is corrected to match the code, not the other way round.
- `S`, `G`, `F` cycle sort, group-by and filter only if free in the registry.
- Modal confirmation dialogs focus Cancel first.

## 6. Interaction with other plans

- `SETTINGS_TABS.md`: tab keys of §3.2 there move into the registry (`Modal` context).
- `DOWNLOAD_FLOW.md`: `Ctrl+A`, `Esc`, `D` and the selection bar and dialogs are audited here.
- `ARTIST_GROUPS.md`: row model, header keys and group actions.
- `AUTO_SYNC.md`, `ARCHIVE.md`, mail UI: their new panels must satisfy `AGENTS.md` rule 15 (focusable, reachable, activatable, closable, exposed as `FxAction`s) and add their rows to the walkthrough.

## 7. Risks

- Spatial navigation and scroll-into-view touch the wall layout code that also drives the scrubber and marquee; keep the neighbour function pure and driven by the layout's rectangles so the painting code does not change.
- Disabling background panels while a modal is open changes visuals (egui dims disabled widgets). Check both themes and the effects levels; if the dimming looks wrong, draw our own scrim instead of relying on the disabled style.
- Region focus parked on egui ids must not break text editing; test the search box, rename and add-artist fields explicitly.

## 8. Open questions (record in `PROGRESS.md`, not blocking)

- Should keys become remappable later (store overrides in `fx.sqlite`)?
- Does the user want a Vim-style leader or modal mode? Not planned.

## 9. Commit sequence

One commit per step, every commit green.

1. `refactor(fx/keys): move shortcuts into a registry` (pure refactor, no behaviour change; tests 1 and 4).
2. `feat(fx/keys): generate palette hints and README keys from the registry` (tests 2 and 3, README corrected).
3. `feat(fx/keys): add focus regions with F6 cycling` (test 5).
4. `feat(fx/wall): navigate cards spatially with scrolling and range keys` (tests 6 to 8).
5. `feat(fx/table): add a row cursor with paging and scrolling` (test 9), then `feat(fx/artists): navigate the artist grid`.
6. `feat(fx/sidebar): navigate artists and groups by keyboard` (test 10).
7. `feat(fx/dock): select, cancel and retry jobs by keyboard` (test 12).
8. `feat(fx/actions): open item actions from the keyboard` (§3.5, test 11).
9. `feat(fx/modals): add a focus scope to every dialog` (test 13; split by dialog group if over the size guideline) and `feat(fx): add the keyboard help overlay` (test 14).
10. `feat(fx/keys): bind global operations` (§3.7).
11. `feat(classic/keys): add a registry, table and sidebar navigation` (test 15 part 1).
12. `feat(classic/modals): add focus scope, actions window and help` (test 15 part 2).
13. `test(fx): add a keyboard-only walkthrough test` (test 16 if feasible).
14. `docs(keyboard): document keyboard operation` (`docs/keyboard.md` walkthrough table, README, `PROGRESS.md`).

## 10. Definition of done

- `scripts/check.sh` green; tests 1 to 3 make shortcut drift impossible.
- The walkthrough of §3.10 is completed in fx and classic without touching the mouse, by the agent and then by the user.
- Every dialog opens with focus on the right control, `Enter` and `Esc` behave as specified, and `Tab` never reaches hidden UI behind a dialog.
- `PROGRESS.md` updated in every commit; tag per `AGENTS.md` rule 13 when released.

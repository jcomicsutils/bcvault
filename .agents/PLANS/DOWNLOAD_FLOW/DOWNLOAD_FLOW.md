# Plan: select all, download remaining and safer bulk downloads

For the coding agent. Repo: bcvault (Rust workspace). Scope: `bcvault-core` (download candidates, selection helpers, enqueue idempotency, summary event, remaining scopes, quota preview), the `bcvault-fx` and `bcvault` UIs (selection, toolbar, dialogs, palette), and docs. No migration.

## 0. Ground rules

- **Branch.** `feat/download-flow` from the repository default branch. Never force-push.
- **Merge order.** After `SETTINGS_TABS.md` (independent of it in code, but both edit the UIs' state and toolbars; rebase often). Before `ARTIST_GROUPS.md`, which adds a "download remaining for this group" scope on top of `RemainingScope`. Independent of `AUTO_SYNC.md`; if that plan lands first, its "removed or unlisted" flag must be excluded in `is_download_candidate` (§3.1).
- Conventional Commits, one commit per step in section 8, `Task: none`. Every commit compiles and passes `scripts/check.sh`. Non-test code must not use `unwrap` or `expect`.
- **Politeness is a product rule** (`AGENTS.md` rule 12). Every request still goes through `BandcampGate`. This plan must not raise any limit, remove a quota check or add a request path. Bulk actions respect `batch_limit`, the soft limit and the hard cap, and the user sees what was deferred.
- **UI-thread purity and square look** apply (`scripts/guards.sh`). Counts shown in toolbars must not iterate all rows every frame (§3.5).
- Read first: `worker/downloads.rs` (`enqueue_downloads`, `push_spec`, dispatcher, terminal job states, `block_artist_for_30_mins`, retry and cancel paths), `worker/quota.rs` (`decide`), `worker/supervisor.rs` (`EnqueueDownloads` arm), `worker/messages.rs`, `types.rs` (`ReleaseRow`, `Classification::auto_downloadable`, `JobState`, `DownloadOpts`, `ReleaseQuery`), `db/repo/releases.rs` (`query_releases`, `get_release`); in fx: `state.rs` (`selected_keys`, `ToggleSelect`, `SelectRange`, `DownloadSelected`, `DownloadKey`, the no-op `QuotaWarning` arm), `actions.rs`, `views/wall.rs` (click, marquee, card painting), `views/table.rs`, `views/toolbar.rs`, `services/search.rs` (palette commands), `overlays/toasts.rs`; in classic: `ui/state.rs` (`selected`, `quota_modal`), `ui/release_table.rs`, `ui/toolbar.rs`, `ui/modals.rs` (quota modal), `app.rs`. Line numbers come from a snapshot, so grep.

## 1. Current state (verified by reading the code)

- **fx selection**: `selected_keys: BTreeSet<ItemKey>`. A plain click on a wall card replaces the selection with that card; Ctrl or Cmd click toggles; Shift click selects a range (`SelectRange`); dragging on the wall background selects the cards in the visible range (marquee); table rows have a checkbox that toggles one row. There is **no select all, no invert, no select remaining, no header checkbox in the table and no Ctrl+A**. `Esc` clears the selection. Selecting many releases therefore means clicking each one with a modifier.
- **fx download buttons**: toolbar `Download (N)` and key `D` send `DownloadSelected`; with an empty selection that falls back to the focused card. There is no "download all" or "download remaining" action in fx at all.
- **classic**: only rows that are not pre-orders and are free or name-your-price can be selected; a header checkbox toggles all such visible rows; Shift click selects a range; Space toggles the last clicked row; Enter downloads the selection. The toolbar has `Download Selected (N)` and `Download All Eligible (M)`, where M counts visible rows that are not downloaded, not pre-orders and free or NYP. Both send `ack_soft_quota: false`.
- Both UIs keep the **entire query result** in memory (`ReleasesLoaded { rows }`, no `LIMIT`), so selection over everything in the view is cheap; only drawing is virtualised.
- **Worker** `enqueue_downloads`: loads the rows, drops pre-orders and anything not free or NYP, groups by artist, and per artist calls `quota::decide(used_30d, requested, min(batch_limit, hard), soft, hard, ack_soft)`. Results: `Blocked` (every item is reported as a failed job), `Warn` (emits `QuotaWarning` with the allowed subset), `Allow` (queues the first `count` items). When `count < requested` the rest is **dropped with only a log line**; the UI is never told.
- **No idempotency.** Nothing skips a release that is already queued or running, and nothing skips a release that is already downloaded. Pressing Download twice queues duplicates and every completed duplicate counts against the 30-day quota. Selecting downloaded releases and pressing Download re-downloads them silently.
- **fx bypasses the soft quota**: it always sends `ack_soft_quota: true` and ignores the `QuotaWarning` event (it is in the no-op arm of `apply_core_event`). Classic shows a quota modal, but holds only one warning (`quota_modal: Option<...>`), so several artists warning in one bulk call overwrite each other.
- Docks are virtualised (`show_rows`), so hundreds of queued jobs are fine. The fx dock clones the job vector every frame; watch it with a thousand jobs.
- README lists `Ctrl+Enter` as "enqueue selected"; the code has no such binding (README drift is owned by `KEYBOARD.md`).

## 2. Goals and non-goals

Goals:

1. **Selection:** select all, select none, invert, select remaining (not downloaded and downloadable), in both UIs, by keyboard (`Ctrl+A`) and by buttons; a visible selection bar with counts; a checkbox on wall cards so no modifier key is needed; a header checkbox in the fx table.
2. **Download remaining:** one action that enqueues every downloadable, not-yet-downloaded, not-already-queued release in a scope: the current view, the current artist(s), or all artists.
3. **Safe bulk behaviour:** idempotent enqueue, an explicit summary of what was queued, skipped and deferred, a confirmation with a per-artist quota preview for large or library-wide runs, soft-quota parity in fx.

Non-goals: new quota rules or defaults; automatic or scheduled downloading (never automatic, see `AUTO_SYNC.md`); a persisted "deferred" queue (pressing the button again later picks up whatever is still remaining, which is the point of idempotency); email-required, paid and pre-order releases (still excluded; the redeem flow is separate); a re-download UX beyond one explicit action.

## 3. Specification

### 3.1 Core: candidates and selection helpers (`types.rs`, new `selection.rs`)

- `ReleaseRow::is_download_candidate(&self) -> bool`: `!downloaded && !is_preorder && classification.auto_downloadable()`. If `AUTO_SYNC.md` has landed, also `&& !removed` (its flag). The worker, both UIs and every count use this one function; delete the inline copies (classic toolbar and table, fx wall, worker filter).
- `selection.rs` (pure, no egui): `SelectScope { All, Downloadable, Remaining }`; `select<'a>(rows: impl Iterator<Item = &'a ReleaseRow>, scope) -> BTreeSet<ItemKey>`; `invert(rows, &selected, scope)`; `range_keys(rows: &[&ReleaseRow], a, b, scope)`. `Downloadable` = free or NYP and not a pre-order (what classic lets you select); `Remaining` = `is_download_candidate`; `All` = every row. fx uses `All` for Ctrl+A (it already lets any row be selected) and classic uses `Downloadable`.

### 3.2 Core: idempotent enqueue, modes and summary (`worker/downloads.rs`, `messages.rs`)

- `EnqueueMode { New, Force }`. `EnqueueDownloads { items, opts, ack_soft_quota, mode }` gains the field (compiler finds the call sites). `New` skips releases that are `downloaded`; `Force` keeps today's behaviour and is used only by an explicit **Re-download** action on a downloaded release (fx drawer and context menu, classic row menu). All other senders use `New`.
- `ManagerState.in_flight: HashSet<ItemKey>`: insert in the one place jobs are queued (`push_spec`, also used by retry), remove at **every** terminal transition (Done, Failed, Cancelled, drained by `block_artist_for_30_mins`, cancel all, shutdown). In both modes a key already in `in_flight` is skipped, never queued twice. Add a debug assertion in tests that `in_flight` is empty once all jobs are terminal.
- `WorkerToUiEvent::EnqueueSummary(EnqueueSummary)` emitted once per enqueue call:
  ```rust
  pub struct EnqueueSummary {
      pub requested: u32,           // distinct keys received
      pub queued: u32,
      pub skipped_downloaded: u32,  // New mode only
      pub skipped_in_flight: u32,
      pub skipped_ineligible: u32,  // pre-order, paid, email required, unknown, row missing
      pub awaiting_ack: u32,        // sent to QuotaWarning, not queued yet
      pub deferred: Vec<DeferredBatch>,
  }
  pub struct DeferredBatch { pub band_id: i64, pub count: u32, pub cause: DeferCause }
  pub enum DeferCause { BatchLimit, HardCap, Blocked30Min }
  ```
  `deferred` replaces the log-only truncation: whatever `decide` cut (batch limit or remaining hard-cap headroom) and whatever was reported as `Blocked` is counted here. Keep the existing log lines.
- `quota.rs`: `preview_bulk(per_artist: &[(i64, u32, u32)], batch_limit, soft, hard, ack_soft) -> Vec<BulkPreviewRow>` where each row is `{ band_id, requested, used_30d, allowed, deferred, will_warn, blocked }`, implemented by calling `decide` per artist so UI previews cannot drift from the worker. Constant `BULK_CONFIRM_MIN: u32 = 25`.

### 3.3 Core: remaining scopes

`UiToWorkerCommand::EnqueueRemaining { scope: RemainingScope, opts: DownloadOpts, ack_soft_quota: bool }` with `RemainingScope { Artists(Vec<i64>), Library }`. The worker resolves keys on the reader connection with the existing `query_releases` per artist (filter `NotDownloaded`, newest first) and `is_download_candidate`, then calls the same internal enqueue as `EnqueueDownloads` with mode `New`, so quota handling and the summary are shared. No new SQL. Artists and library scopes ignore view filters on purpose.

The **View** scope is not a worker scope: the UI computes the keys from the rows it is showing, because fx-only filters (hue, years, art-only, smart views) are applied in the UI and the worker cannot reproduce them. It sends `EnqueueDownloads { items, mode: New }`.

### 3.4 Quota dialogs (both UIs)

- Both UIs collect `QuotaWarning` events into a list (not a single `Option`), de-duplicated by artist, and show **one** dialog listing each artist (used of soft, requested), with `Continue` (one `EnqueueDownloads` with all listed items and `ack_soft_quota: true`) and `Cancel`. Enter confirms only when Continue has focus; Esc cancels; initial focus is on Cancel (a safe default). fx gets this dialog for the first time and stops passing `ack_soft_quota: true` for selection, remaining and single-item downloads.
- **Bulk confirmation dialog** ("Download remaining"): shown when the scope is Library or more than one artist, or when the candidates exceed `BULK_CONFIRM_MIN`, or when `preview_bulk` reports any deferral, warning or block. A virtualised table of artist, new releases, will queue, deferred (with the cause), plus totals and the line "Deferred releases stay remaining; press Download remaining again later." Buttons: Queue N, Cancel (initial focus on Cancel). Below the threshold and with no deferrals the action runs directly and the summary toast confirms it.

### 3.5 UI behaviour

Shared state, per UI: `SelectionStats { selected, selected_downloadable, remaining_in_view, total_in_view }`, recomputed only when `rows`, the selection or `ReleaseUpdated` events change (dirty flag), never per frame.

**fx**
- Actions: `SelectAll`, `InvertSelection`, `SelectRemaining`, `DownloadRemaining(RemainingTarget)` with `RemainingTarget { View, ThisArtist, AllArtists }`, `Redownload(ItemKey)`. `ClearSelection` exists. Bind `Ctrl+A` to `SelectAll` only when no text input has focus (the existing `wants_keyboard_input` guard) and `Esc` keeps clearing.
- **Selection bar** under the toolbar whenever the selection is not empty: `N selected (M downloadable)`, `Download N`, `Select all (T)`, `Select remaining (R)`, `Invert`, `Clear`. Square buttons through `fx/paint`.
- **Wall card checkbox**: a square box in the card's top-left corner, drawn while the card is hovered, selected, or any card is selected; clicking it toggles that card (`ToggleSelect`) and must not also fire the card click. Plain click semantics are unchanged.
- **Table**: a tri-state header checkbox (none, some, all of the view) and the existing row checkboxes.
- **Toolbar**: `Download (N)` stays and is disabled with an empty selection (the `D` key keeps its focused-card fallback); new `Download remaining (M)` where M counts candidates in the view, with a `▾` menu: *In this view*, *This artist* (enabled when an artist is selected), *All artists...*.
- **Palette** commands (all also bound as `FxAction`s): Select all releases in view, Select remaining, Invert selection, Clear selection, Download selected, Download remaining in this view, Download remaining for this artist, Download remaining for all artists.
- **Toast** on `EnqueueSummary`: `Queued 87 · 12 already downloaded · 3 already queued · 31 deferred (2 artists reached their limit)`; show the deferred line as a warning toast. The per-artist detail goes to the log.

**classic**
- Header checkbox and `Ctrl+A` select `Downloadable`; `Esc` clears the selection when no dialog is open. The existing second toolbar row gets `Select remaining`, `Clear` and the renamed `Download remaining (M)` with a `▾` scope menu: *In this view*, *Selected artists* (the sidebar multi-selection, or the single active artist), *All artists...*.
- `EnqueueSummary` becomes a status note in the toolbar for about 10 seconds plus a log entry (classic has no toast layer).

### 3.6 Behaviour changes to call out in the README

- Download and Download remaining skip releases that are already downloaded; a release is re-downloaded only through **Re-download**.
- Pressing a download action twice never queues a release twice.
- fx now warns at the soft limit like classic.

## 4. Tests

| # | Area | Scenario | Expected |
| --- | --- | --- | --- |
| 1 | core | `is_download_candidate` over free, NYP, paid, email-required, unknown, pre-order, downloaded, archived (downloaded) rows | Only free or NYP, non-pre-order, not downloaded |
| 2 | core | `select` and `invert` for each `SelectScope`; `range_keys` forward and backward | As §3.1; pure and deterministic |
| 3 | downloads | Enqueue the same 3 keys twice with the queue paused | 3 queued then `skipped_in_flight = 3`; `in_flight` has 3 keys |
| 4 | downloads | After each terminal state (Done, Failed, Cancelled, hard-cap drain, cancel all) | Key leaves `in_flight`; can be queued again; retry path re-adds it |
| 5 | downloads | `New` with a downloaded release; `Force` with the same | `New` skips it (`skipped_downloaded`), `Force` queues it |
| 6 | downloads | 40 keys, `batch_limit = 25` | 25 queued, `deferred = [{count 15, BatchLimit}]`, summary totals add up to `requested` |
| 7 | downloads | Artist with 195 of 200 used and 20 requested | 5 allowed, 15 deferred with `HardCap`; at 200 used everything `Blocked30Min` or `HardCap` as `decide` says |
| 8 | downloads | Soft limit exceeded, `ack_soft_quota = false` | `QuotaWarning` emitted, `awaiting_ack` counted, nothing queued; resending with `true` queues them |
| 9 | quota | `preview_bulk` against `decide` for a grid of (used, requested, limits, ack) | Identical outcomes for every row |
| 10 | downloads | `EnqueueRemaining { Artists([a]) }` where the artist has 2 free (1 downloaded), 1 paid, 1 pre-order, 1 email-required | Only the not-downloaded free release is queued |
| 11 | downloads | `EnqueueRemaining { Library }` over three artists with the queue paused, run twice | Second run queues nothing (`skipped_in_flight`) |
| 12 | pipeline (wiremock) | Download remaining for an artist with three releases, one already downloaded | Two files downloaded, one history row each, quota counted once per download |
| 13 | fx state | `SelectAll`, `SelectRemaining`, `InvertSelection`, header tri-state, stats cache invalidation on `ReleaseUpdated` | Selection and `SelectionStats` as expected; no per-frame recount (counter in the test) |
| 14 | fx state | Several `QuotaWarning` events then Continue and Cancel | One dialog state listing all artists; Continue sends one command with all items and `ack_soft_quota = true` |
| 15 | classic state | Same as 14, plus `Ctrl+A` and `Esc` handling, header checkbox | As specified |
| 16 | both | Bulk confirmation threshold: 24 items without deferral, 25 items, library scope, any deferral | Direct run or dialog exactly as §3.4 |
| 17 | both | `EnqueueSummary` formatting for all-zero, mixed and deferred cases | Exact strings, singular and plural |

## 5. Decisions made for you (change only through `AGENTS.md` §8)

- Download actions never re-download implicitly (`EnqueueMode::New`); re-download is explicit.
- fx honours the soft quota from now on. This adds prompts that did not exist; it follows `AGENTS.md` rule 12.
- Click semantics on wall cards are unchanged; a hover checkbox is added instead of a "selection mode".
- The View scope is computed in the UI from visible rows; Artists and Library scopes are resolved by the worker from the database and ignore view filters.
- No persisted deferred queue.
- `Download (N)` is disabled with an empty selection; `D` keeps its focused-card fallback.

## 6. Interaction with other plans

- `ARTIST_GROUPS.md` adds a group to `RemainingTarget` (as `Artists(ids)` resolved in the UI from the group's members).
- `KEYBOARD.md` moves the keys listed here (`Ctrl+A`, `Esc`, `D`) into its registry and covers the new bar, dialogs and checkboxes in its walkthrough.
- `AUTO_SYNC.md`: when a release is flagged removed or unlisted it must not be a candidate (§3.1 hook).
- `SETTINGS_TABS.md`: no code dependency. The toolbar format combo stays.

## 7. Risks

- The summary event and the in-flight set touch the hottest path of the downloader. Keep the dispatcher's fairness (round-robin by artist) and the existing per-artist block logic exactly as they are; test 4 guards the in-flight set.
- A library-wide button invites mistakes. The confirmation dialog defaults to Cancel and always appears for library scope.

## 8. Commit sequence

One commit per step, every commit green.

1. `feat(types): add download candidates, selection helpers and bulk preview` (§3.1, `quota::preview_bulk`), tests 1, 2, 9.
2. `feat(downloads): skip downloaded and in-flight releases when enqueueing` (`EnqueueMode`, `in_flight`), updates every `EnqueueDownloads` sender with `mode`, tests 3 to 5.
3. `feat(downloads): report an enqueue summary with quota deferrals` (event, counts), tests 6 to 8.
4. `feat(downloads): queue remaining releases for artists or the library` (§3.3), tests 10 to 12.
5. `feat(classic): add bulk selection, quota list dialog and download remaining` (§3.4, §3.5 classic), tests 15 to 17 for classic.
6. `feat(fx): show the quota warning dialog and enqueue summaries` (fx quota list dialog, summary toasts, stop forcing `ack_soft_quota`), tests 14 and 17 for fx.
7. `feat(fx/selection): add select all, remaining, invert and a selection bar` (actions, bar, card checkbox, table header, `Ctrl+A`), test 13.
8. `feat(fx/toolbar): add download remaining with scope menu and confirmation` (toolbar button, `▾` menu, confirmation dialog, palette commands), test 16 for fx.
9. `docs(downloads): document selection and download remaining` (README, `docs/`, behaviour changes of §3.6, `PROGRESS.md`).

## 9. Definition of done

- `scripts/check.sh` green; tests 3, 4 and 11 prove double presses and repeated bulk runs never duplicate work.
- In both UIs: `Ctrl+A`, Select remaining, Download remaining for the view, for the artist(s) and for all artists work; the summary shows what was skipped and deferred; large runs show the confirmation; fx warns at the soft limit.
- **Manual (ask the user):** with a test artist that has several free releases, run Download remaining twice in a row and confirm the second queues nothing; run it for an artist near the soft limit and confirm the warning; confirm Re-download works on a downloaded release. Never touch the live site without the user's go-ahead.
- `PROGRESS.md` updated in every commit; tag per `AGENTS.md` rule 13 when released.

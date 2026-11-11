# Plan: scheduled auto sync, quick check, and removed-release tracking

For the coding agent. Repo: bcvault (Rust workspace). Scope: `bcvault-core` (sync modes, scheduler, DB, settings), the `bcvault-fx` and `bcvault` UIs (settings, tags, change feed, artist exclusion), and docs. A companion plan, the release archive, builds on this one.

## 0. Ground rules

- **Branch.** Create `feat/auto-sync` from the repository default branch (`git symbolic-ref refs/remotes/origin/HEAD`), not from `feat/file-themes` (the library work is already on `main`). Never force-push.
- **Merge order.** This plan merges before the archive plan, which rebases onto it (both touch `worker/sync.rs`). Every in-flight plan edits the Settings modals, so keep modal changes in new functions or files (or add a `Sync` tab if `SETTINGS_TABS.md` has landed). Migration numbering: the schema is at version 8 (`0008_rejected_matches.sql`), so this plan uses `0009_auto_sync.sql` (or the lowest free number at merge time per `ROADMAP.md` §5; never renumber a merged migration).
- Conventional Commits, one commit per step in section 9. Every commit compiles and passes tests. Run `scripts/check.sh` before each commit. Non-test code must not use `unwrap` or `expect`. The fx bans from `scripts/check.sh` apply (`circle_`, `Shape::circle`, `rounding`, non-zero `corner_radius`, `Spinner`), including any badge you draw.
- **Politeness is a product rule.** Every request goes through the existing client and `BandcampGate`. Never add a request path that bypasses the gate, and never bypass email or payment gates.
- **Archival is a product rule.** No code path may delete, blank or overwrite a release, its metadata, its classification or its download state because it disappeared from Bandcamp or because a fetch failed. Missing releases are flagged only.
- Read first: `worker/sync.rs` (`sync_resolved`), `worker/supervisor.rs` (the `Sync` command arm), `worker/messages.rs`, `types.rs` (`SyncMode`, `SyncScope`, `Settings`), `db/repo/releases.rs`, `db/repo/artists.rs`, `bandcamp/mod.rs`, `bandcamp/errors.rs`, `bandcamp/gate.rs`, `error.rs`; in fx: `state.rs`, `actions.rs`, `app.rs`, `overlays/modals.rs`, `overlays/drawer.rs`, `views/artists.rs`, `views/table.rs`, `views/wall.rs`, `services/search.rs`; in the classic UI: `ui/modals.rs`, `ui/release_table.rs`. Line numbers here come from a snapshot, so grep.

## 1. Current state (verified by reading the code)

- `SyncMode` has three variants: `Incremental`, `CheckDownloaded`, `FullRefresh`. Every mode starts with `fetch_discography` (the artist page), upserts all discovered releases, then builds a queue of release pages to fetch: Incremental = new plus all `unknown`; CheckDownloaded = those plus every downloaded release; FullRefresh = every release.
- The supervisor `Sync { scope, mode }` command spawns a task that loops over artist ids. `SyncScope::All` means every artist from `artists::list_views`. There is no exclusion, no scheduler, and nothing prevents two sync tasks running at once.
- `releases.last_seen_at` is set on every discovery, but nothing reads it. **Removed releases are not detected at all.**
- Release pages are parsed in `fetch_release`. The row read before the fetch (`get_release`) already holds the old `classification`, so classification changes can be detected without extra queries.
- Only the success branch writes metadata; a failed fetch is logged and ignored (keep it that way).
- The only delete touching releases is `DELETE FROM artists`, which cascades. That is a user action and stays unchanged.
- `Settings` is a serde struct stored as JSON in the `settings` table, and new fields must use `#[serde(default)]`.
- I did not verify how HTTP 404 or 410 on a release page is reported by `get_text_gated`; check `bandcamp/errors.rs` and add a typed variant if there is none.

## 2. Goals and non-goals

Goals:

- A cheap scheduled sync that only needs the artist page plus the pages of brand-new releases.
- A manual Quick check that re-fetches paid releases to see whether they became free.
- Full refresh stays manual and unchanged.
- Per-artist exclusion from scheduled and bulk sync.
- Removed or unlisted releases are flagged, never deleted.
- A persistent change feed.

Non-goals:

- Changing Incremental, Check downloaded or Full refresh behavior.
- A background service that runs while the app is closed.
- Auto-downloading anything.
- Per-artist intervals.

## 3. Sync modes

Add two variants and keep the existing three untouched. Every mode (old and new) gets removed-release detection from section 4, because they all run discovery.

| Mode | Trigger | Fetches |
| --- | --- | --- |
| `Auto` | Scheduler, and Run now | Artist page. Pages of newly discovered releases only, so they get classified: at most `new_pages_per_artist` (default 10) per artist per run; the rest stay `unknown` for a manual Incremental. No `unknown` backlog, no downloaded rechecks. |
| `QuickCheck` | Manual only | Everything `Auto` does, plus the release pages of paid releases that are not downloaded and whose `metadata_fetched_at` is empty or older than `quick_recheck_days` (default 7), oldest first. Also re-verifies releases currently flagged `unlisted` (section 4). |
| `Incremental`, `CheckDownloaded` | Manual | Unchanged |
| `FullRefresh` | Manual only | Unchanged. Never scheduled, and the UI labels it as fetching every release page. |

Reclassification events: when a release page is fetched, compare the old `classification` (already read before the fetch) with the parsed one. If the old value was not `unknown` and differs, write a `reclassified` event with detail like `paid -> free`. This applies to every mode.

## 4. Removed and unlisted releases

Migration `0009_auto_sync.sql` (or next free number; verify that `ALTER TABLE ... ADD COLUMN` with `CHECK` works on these STRICT tables, and adapt if not):

```sql
ALTER TABLE artists ADD COLUMN auto_sync INTEGER NOT NULL DEFAULT 1 CHECK (auto_sync IN (0,1));
ALTER TABLE releases ADD COLUMN missing_since INTEGER;
ALTER TABLE releases ADD COLUMN missing_kind TEXT CHECK (missing_kind IN ('unlisted','removed'));
ALTER TABLE releases ADD COLUMN missing_checked_at INTEGER;
CREATE TABLE sync_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL DEFAULT (unixepoch()),
    band_id INTEGER NOT NULL,
    item_type TEXT, item_id INTEGER,
    kind TEXT NOT NULL CHECK (kind IN ('new','removed','unlisted','restored','reclassified','anomaly')),
    detail TEXT,
    seen INTEGER NOT NULL DEFAULT 0 CHECK (seen IN (0,1))
) STRICT;
CREATE INDEX idx_sync_events_unseen ON sync_events(seen, ts);
```

`sync_events` has no foreign key to `releases` or `artists`, so history survives artist removal, in line with `download_history`.

Detection runs inside the same writer closure that already calls `upsert_discovered_batch`:

1. Releases of that band seen in the discovery clear `missing_since`, `missing_kind` and `missing_checked_at`. If they were flagged before, write a `restored` event.
2. The candidates for flagging are the band's releases that were not seen and have `missing_since IS NULL`.
3. **Guard.** Do not flag anything, and write one `anomaly` event, if the known count is above zero and nothing was discovered, or if more than 3 releases are missing and they exceed 30 percent of the known releases. This protects against a bad page parse.
4. Otherwise set `missing_since` to now and `missing_kind` to `unlisted`.

After that step, **verify each newly flagged release once** (at most 20 per artist per run) by fetching its release page through the gate:

- HTTP 404 or 410: set `missing_kind = 'removed'`, set `missing_checked_at`, write a `removed` event.
- Page loads normally: leave it as `unlisted` and write an `unlisted` event.
- Any other error (network, 429, 5xx): leave it as `unlisted` with `missing_checked_at` empty, and retry at the next QuickCheck or FullRefresh.

`QuickCheck` and `FullRefresh` also re-verify releases that are currently `unlisted` (a page fetch each). That fetch can turn `unlisted` into `removed`, or clear the flag if the release is listed again.

Hard rules: a failed verification never changes metadata. `update_metadata` runs only on a successful parse. A removed release keeps everything it had, including `downloaded`, `local_path` and its classification.

User-facing behavior:

- **Removed on Bandcamp** (kind `removed`) and **Unlisted** (kind `unlisted`) badges with the date from `missing_since`, in the table, on the wall and as a banner in the drawer.
- Filters `#removed` and `#unlisted` in the palette, and a count in stats.
- Download is disabled with a tooltip for `removed` releases that are not already downloaded. `unlisted` releases still download.
- The classic UI gets a status label in the release table and a filter entry. Do not build a detail pane there.

## 5. Excluding artists

- `artists.auto_sync` (default 1). A new worker command `SetArtistAutoSync { band_id, enabled }` and an `ArtistUpserted`-style refresh event.
- `SyncScope::All` skips artists with `auto_sync = 0` and reports the skipped count in the finished event. The scheduler uses the same filter. `SyncScope::Selected` always includes the artists named, so you can still sync one excluded artist on purpose.
- UI: a toggle in the artist context menu and the drawer, an Excluded badge and filter in the Artists view, and `N artists excluded` in the sync menu.
- **Dormant suggestions** in the Artists view: artists not excluded whose newest `published_at` is older than 24 months, and who have metadata for every release (artists with unknown dates are never suggested). A single Exclude all dormant button with a confirmation listing the count.
- **Remove artist confirmation** states how many releases and downloaded releases would be deleted, and offers Exclude from sync as the non-destructive alternative. The deletion behavior itself does not change.

## 6. Scheduler

New `worker/scheduler.rs`, owned by the supervisor so both UIs and any future CLI share it.

Settings (serde defaults, in a new `AutoSyncSettings` struct inside `Settings`):

| Field | Default | Range |
| --- | --- | --- |
| `enabled` | false |  |
| `interval_hours` | 24 | 6 to 168 |
| `run_if_due_on_launch` | true |  |
| `startup_delay_s` | 45 | 0 to 600 |
| `quick_recheck_days` | 7 | 1 to 90 |
| `new_pages_per_artist` | 10 | 1 to 50 |

Runtime state is stored under the `settings` table key `auto_sync_state` as JSON: `last_complete_at`, `last_partial_at`, `last_summary`.

Behavior:

- Check once a minute while the app runs. A run starts when it is enabled, due (`now - last_complete_at >= interval`), and nothing else is syncing. At launch it waits `startup_delay_s` first.
- Introduce a shared run guard (counter or mutex) so a scheduled run never overlaps a manual sync, and a manual sync started during a run waits or refuses with a clear message.
- Artists are processed one at a time, with `auto_sync = 1` only, ordered by `last_synced_at` ascending (never-synced first), using `SyncMode::Auto` through the existing `sync_artist` path. Per-artist failures are logged and the run continues.
- A rate-limit cooldown or cancellation ends the run as partial: record `last_partial_at` and retry after the smaller of one hour and the interval. A fully completed run sets `last_complete_at`.
- Linked-folder rescans follow the existing `rescan_linked_on_sync` setting, unchanged.
- Events: `AutoSyncStarted`, `AutoSyncProgress { done, total }`, `AutoSyncFinished { summary }`, plus the existing per-artist `SyncProgress`. A Run now button starts the same run on demand, and Cancel uses the existing `CancelSync`.

## 7. Change feed and UI

- **Feed.** A Changes panel (fx: a view or modal reachable from the sidebar and the palette; classic: a simple list modal) with events grouped by run, newest first. It shows kind, artist, release and detail; clicking an event jumps to the release. Mark all read, and an unread badge on the sidebar entry. Events are never pruned.
- **Toast.** After each scheduled run: `Auto sync: 3 new, 1 now free, 2 removed (4 artists skipped)`.
- **Settings.** An Automatic sync section in both settings modals: enable, interval presets (6 hours, 12 hours, 24 hours, 3 days, 7 days), run on launch if due, Run now, last run summary, `quick_recheck_days`.
- **Sync menu.** Add Quick check (paid releases, looks for free ones). Keep Full refresh with its warning that it fetches every release page.

## 8. Tests

| # | Area | Scenario | Expected |
| --- | --- | --- | --- |
| 1 | modes | Auto with 2 new and 5 `unknown` releases | Pages fetched for the 2 new ones only (assert the mock server request count) |
| 2 | modes | Auto with 15 new releases and `new_pages_per_artist = 10` | 10 fetched, 5 remain `unknown` |
| 3 | modes | QuickCheck with paid releases (one downloaded, one checked yesterday, two stale), free releases, `unknown` | Only the two stale paid ones are fetched |
| 4 | modes | Existing Incremental, CheckDownloaded and FullRefresh tests | Unchanged |
| 5 | reclass | Paid release now free; unknown to paid; free to paid | Events written only for known-to-different transitions |
| 6 | missing | Release disappears from discovery; its page returns 404 | Flag `removed`, event written, no row or field deleted |
| 7 | missing | Disappears but page loads | Flag `unlisted` |
| 8 | missing | Disappears and verification gets 500 or 429 | Stays `unlisted` unchecked; retried by QuickCheck |
| 9 | missing | Reappears | Flags cleared, `restored` event |
| 10 | guard | Empty discovery for a known artist; 40 percent missing with more than 3 | No flags, one `anomaly` event |
| 11 | guard | 2 of 3 releases removed on a tiny catalog | Flagged (the 3-item floor allows it) |
| 12 | safety | Failed release fetch and failed verification | Existing metadata and classification untouched |
| 13 | safety | Downloaded release becomes removed | `downloaded`, `local_path`, `downloaded_at` untouched |
| 14 | exclusion | `SyncScope::All` with one excluded artist | Skipped, count reported; `Selected` still syncs it |
| 15 | scheduler | Due, not due, disabled, cooldown active, manual sync running | Starts only when due, enabled and idle; partial run retry timing |
| 16 | scheduler | Artist ordering and per-artist error isolation | Oldest-synced first; one failure does not stop the run |
| 17 | settings | Old settings JSON without the new fields | Deserializes to defaults |
| 18 | feed | Unread counts, mark read, event survives artist removal | Passes |
| 19 | dormant | Artist with no release in 24 months; artist with unknown dates | Only the first is suggested |
| 20 | ui | fx and classic state tests for badges, filters and exclusion toggle | Pass |

## 9. Commit sequence

Each step leaves the workspace compiling with tests green.

1. `feat(db)!: add auto-sync, missing-release and sync-event schema` adds migration `0009` (or lowest free number, with `BREAKING CHANGE:` footer), repo functions and types.
2. `feat(sync): flag releases missing from the artist page` covers detection, guard, verification fetch, the typed 404 and 410 error, and `restored` events, for all modes.
3. `feat(sync): add auto and quick-check sync modes` adds the variants, queue building and reclassification events.
4. `feat(sync): allow excluding artists from automatic and bulk sync` adds the command, scope filtering and skipped counts.
5. `feat(worker): add the sync scheduler` adds settings, scheduler task, run guard and events.
6. `feat(fx): automatic sync settings, quick check and artist exclusion` covers the settings section, sync menu, Artists view toggle and filter, dormant suggestions and the remove-artist dialog.
7. `feat(fx): removed-on-bandcamp tags and change feed` covers badges, filters, drawer banner, Changes panel and toast.
8. `feat(ui): auto sync settings, tags and exclusion in the classic UI`.
9. `docs(sync): document sync modes, scheduling and removed-release tracking` and update the README sync section.

## 10. Definition of done

- `scripts/check.sh` passes on every commit.
- A scheduled run on a library of N non-excluded artists makes N artist-page requests plus only the new-release pages, which tests prove with request counts.
- Nothing in any sync mode deletes or blanks release data; the safety tests prove it.
- Full refresh is still manual only and behaves as before.
- Old databases and old settings load without changes.

## 11. Decisions already made (do not re-litigate; flag if impossible)

- Auto fetches pages of new releases only (capped per artist), never the unclassified backlog.
- Quick check means paid, not downloaded, and stale (7 days by default), plus re-verifying unlisted releases.
- Removed and unlisted are flags with dates. The Unlisted-versus-Removed page check and the dormant suggestions and remove-artist dialog are included by default; drop them only if the user says so.
- The scheduler runs only while the app is open, and is off by default.
- Events are never pruned, and have no foreign keys.

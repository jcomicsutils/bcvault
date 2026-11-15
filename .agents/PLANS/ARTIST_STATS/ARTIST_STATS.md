# Plan: artist stats page (per-artist statistics from the local database)

For the coding agent. Repo: bcvault (Rust workspace). Scope: `bcvault-core` (a pure `insights` module that turns database rows into statistics, plus a read-only loader), `bcvault-fx` (a new "Artist stats" view, a query service, state and actions) and docs. No migration, no new settings, no network. A companion plan, the release archive, feeds it.

## 0. Ground rules

- **Branch.** `feat/artist-stats` from the repository default branch (`git symbolic-ref refs/remotes/origin/HEAD`). Never force-push.
- **Merge order.** After `ARCHIVE.md` (which itself merges after `AUTO_SYNC.md`). Track lengths, track titles and tags only exist in the database once the archive has captured them, and the loader reads `release_archive` and the `missing_*` columns that those plans add. `ARTIST_GROUPS.md` and `KEYBOARD.md` also touch the view switcher and the sidebar menu: whichever merges second rebases. Add UI in new functions and files, not into existing loops.
- **No migration.** Everything is derived on read from existing tables. If a commit seems to need one, stop and follow `AGENTS.md` §8.
- Conventional Commits, one commit per step in section 7, `Task: none`. Every commit compiles and passes `scripts/check.sh`. Non-test code must not use `unwrap` or `expect`. The fx bans apply (`circle_`, `Shape::circle`, `rounding`, non-zero `corner_radius`, `Spinner`).
- **Database only (product rule).** Every number on this page comes from rows already in `bcvault.db` or `fx.sqlite`. Opening the page, changing a toggle or switching artist never sends a request to Bandcamp, never queues a sync, archive backfill, thumbnail or any other worker job, and never writes to a database. When data is not stored, the page says so (3.3) and offers at most a navigation action to the Archive settings. Section 7 step 1 adds a guard for this.
- **UI-thread purity** applies. Rows are loaded and statistics computed off the UI thread; the view only paints state and sends commands.
- **Honest numbers.** A statistic never presents partial data as complete (3.3), and never counts a release it could not read as if it were zero.
- **Naming.** The sidebar already has `ArtistStats` (badge counts) and fx already has a library-wide `Stats` view. To avoid confusion the new code uses `insights` everywhere internally (`bcvault_core::insights`, `ViewMode::Insights`, `views/artist_stats.rs`, `FxCommand::QueryInsights`), and the user-facing label is "Artist stats".
- Read first: `types.rs` (`ArtistView`, `ArtistStats`, `Classification::is_eligible`, `ReleaseRow`, `ItemKey`), `db/repo/releases.rs` (`all_artist_stats` for the sidebar's definition of eligible), `bandcamp/tralbum.rs` (`parse_tralbum_json_with_dom`, `track_count`, `published_at`), `bandcamp/classify.rs` (`PricingSignals`), `library/textmatch.rs` (`key`), `text.rs`, the archive plan's `release_archive` schema (`ARCHIVE.md` 3.2) and the auto-sync plan's `missing_kind` columns (`AUTO_SYNC.md` section 4), `db/mod.rs` (`DbPool` readers); in fx: `views/stats.rs` (`paint_odometer_u32`, bars), `views/artists.rs`, `views/toolbar.rs` (view tabs), `actions.rs` (`ViewMode`), `state.rs` (`SelectArtist`, `active_stats_req_id`, `dispatch`), `app.rs` (number keys), `services/mod.rs`, `services/details.rs` (service pattern), `services/messages.rs`, `db/mod.rs` (`query_stats`, `cover_meta`), `overlays/palette.rs`, `views/sidebar.rs` (artist context menu), `paint/`. Line numbers come from a snapshot, so grep.

## 1. Current state (verified by reading the code)

- fx has one **library-wide** Stats view (`views/stats.rs`): odometers, a classification treemap, a download heat-calendar and releases per year. Its data comes from `db::query_stats` on the fx reader (main database attached as `m`), through `FxCommand::QueryStats` and `FxEvent::StatsLoaded`, with a stale-response guard (`active_stats_req_id`). `ViewMode::ALL` has four entries, keys `1` to `4` in `app.rs` switch views, the toolbar iterates `ViewMode::ALL`, and `ViewMode` is stored in the fx prefs.
- `ArtistStats` (in `types.rs`) holds only sidebar badge counts. It has no durations, titles or dates.
- **Main database today** (`releases`): `title`, `artist_name` (per-release artist, differs from the page artist on label and compilation pages), `item_type`, `classification`, `pricing_json`, `is_preorder`, `track_count` (NULL until the release page was inspected; 1 for track pages, the `trackinfo` length for albums, 0 means `unavailable`), `published_at`, `metadata_fetched_at`, `downloaded`, `art_id`. `published_at` comes from `current.publish_date` with `album_release_date` as fallback, so it is Bandcamp's page publish date, not necessarily the date the artist printed on the release. `download_history.bytes_size` records bytes written by the app.
- `pricing_json` stores `PricingSignals` (`minimum_price`, `set_price`, `is_set_price`, `require_email`, ...). It has **no currency**, so prices from different artists or releases cannot be compared or summed. That is why v1 shows the classification mix and not average or highest price.
- **Durations and track titles are not in the main database today.** They exist only in `fx.sqlite` `release_detail` for releases whose drawer was opened. `ARCHIVE.md` moves them into `release_archive` (main database): `tracks_json` (`{track_num, title, duration_s, artist}` as ARCHIVE.md specifies it today, which has no per-track slug; section 6 makes ARCHIVE.md add one), `tags_json`, `about`, `credits`, `captured_mask` (bit 1 tracklist, 2 tags, 4 about, 8 credits, 16 raw; a set bit with a NULL value means archived and empty), no foreign key to `releases`, and it survives artist removal. Capture is opt-in per category and backfill is a user-confirmed job, so on a real library some releases will lack it.
- **Cover colors** are in `fx.sqlite` `cover_meta` keyed by `art_id`: `swatches` (5 RGB by population), `accent_rgb`, `hue`, `chroma`, `luma`. A row exists only for covers the UI has already processed.
- Name keys for exact-first matching are `library::textmatch::key` (case, diacritics and punctuation folded; no noise-token stripping).
- `ArtistView` has `name` and `custom_name`; `display_name()` prefers the custom one. `releases.artist_name` is the credit shown for a release; a per-track `artist` exists in the archive only when the page provided one.
- The sidebar's definition of eligible is `Classification::is_eligible` (free, nyp, email_required), as in `all_artist_stats`. Email-required releases count as eligible by design. The comment on `ArtistStats.total` (`eligible = free + nyp`) is wrong; fix it in the step 3 commit and follow the code.
- **Not verified.** The fixtures in `crates/bcvault-core/tests/fixtures` have `trackinfo` entries with titles only, so real `duration` values, per-track `artist` and the per-track link used for dedupe (expected as `trackinfo[].title_link`, a path like `/track/<slug>`) are unverified. The first step that parses them must be developed against one captured real album page, sanitised per `AGENTS.md` §10 (fixtures beat assumptions). If the archive plan changes the shape of `release_archive`, adapt only the loader (3.8).

## 2. Goals and non-goals

Goals:

- A page "Artist stats" for the selected artist, with the 13 v1 cards in 3.6.
- Strictly database-only. Missing information is reported in plain words, never fetched.
- Partial coverage is visible on every card ("based on 40 of 60 releases").
- A duplicate-track flow with its own card, so totals do not count a track twice. A duplicate is proven by an exact slug match (a standalone track page whose slug is now a track of an album on the same page) and by nothing else.
- A per-stat "same artist" button for statistics that single out one release or track, for pages where the item is credited to someone else (splits, compilations).
- Every idea not in v1 is recorded in section 9 with what it needs, so it can be added later without redesign.

Non-goals:

- Any network access, sync, archive job or thumbnail request triggered from this page.
- Splitting a label or compilation page into separate artists. One Bandcamp page is one artist here, always.
- Group, multi-artist or library-wide aggregation (the existing Stats view stays library-wide).
- The classic UI, exporting, historical trends, charts beyond 3.10, new `Settings` fields, new migrations, writing to any database.

## 3. Specification

### 3.1 Architecture

- **Pure core** `bcvault-core/src/insights/` (no I/O, no clock, no network): `mod.rs` (types and `compute_insights`), `facts.rs` (inputs), `credit.rs` (page-artist matching), `dedupe.rs`, `stats.rs` (the statistics), `fmt.rs` (`fmt_duration`).
- **Read-only loader** `bcvault-core/src/db/repo/insights.rs`: `load_artist_facts(conn, band_id) -> Result<ArtistFacts, DbError>`. It reads the main database only.
- **fx service** `services/insights.rs` (pattern of `services/details.rs`): reads facts through the main reader pool, reads `cover_meta` through the fx reader, calls `compute_insights`, and emits `FxEvent::InsightsLoaded`. It holds no `BandcampClient`.
- **fx view** `views/artist_stats.rs`: paints `state.insights` only. It is under `views/`, so the UI-purity guard applies to it.
- Data flow: `FxAction::SetView(Insights)` / artist change / toggle / invalidating event, then `FxCommand::QueryInsights { request_id, band_id, options }`, then service, then `FxEvent::InsightsLoaded { request_id, band_id, insights }`, then state, then paint. A response whose `request_id` is not the active one is dropped.

### 3.2 Facts (the only input of the computation)

```rust
pub struct ArtistFacts {
    pub band_id: i64,
    pub page_names: Vec<String>,        // artists.name and non-empty custom_name
    pub history_bytes: u64,             // download_history success bytes for this band_id
    pub releases: Vec<ReleaseFacts>,
}
pub struct ReleaseFacts {
    pub key: ItemKey,
    pub title: String,
    pub slug: Option<String>,           // track releases only: path segment after /track/ in releases.url
    pub credit: String,                 // releases.artist_name
    pub classification: Classification,
    pub is_preorder: bool,
    pub track_count: Option<u32>,
    pub published_at: Option<Ts>,
    pub inspected: bool,                // metadata_fetched_at IS NOT NULL
    pub downloaded: bool,
    pub missing_kind: Option<MissingKind>, // from AUTO_SYNC: Unlisted | Removed
    pub art_id: Option<i64>,
    pub tracks: Archived<Vec<TrackFacts>>, // release_archive.tracks_json
    pub tags: Archived<Vec<String>>,       // release_archive.tags_json
}
pub enum Archived<T> { NotCaptured, Captured(T) } // Captured(empty) = archived and empty
pub struct TrackFacts { pub num: u32, pub title: String, pub duration_s: Option<u32>, pub artist: Option<String>, pub slug: Option<String> }
pub struct CoverColor { pub art_id: i64, pub rgb: Option<[u8; 3]> } // swatches[0], None if absent
pub struct InsightOptions { pub dedupe: bool, pub include_removed: bool, pub now: Ts }
```

`Archived::NotCaptured` means the category's `captured_mask` bit is clear, or there is no `release_archive` row, or the stored JSON failed to parse. A set bit with a NULL value is `Captured(vec![])`. `compute_insights(&ArtistFacts, &[CoverColor], &InsightOptions) -> ArtistInsights` is a pure function; `now` is passed in so tests are deterministic.

### 3.3 Missing and partial information (the rule behind every card)

Each statistic declares the **eligible set** (releases that could contribute, after the exclusions in 3.6) and the **category** it needs. The **used set** is the eligible releases whose data is usable.

```rust
pub enum Missing {
    NotArchived(ArchiveCategory), // Tracklist | Tags (later: About, Credits)
    NotSynced,                    // release pages not inspected yet
    CoversNotLoaded,
    NoData,                       // everything that could be read is empty, or nothing qualifies
}
pub struct Coverage { pub used: u32, pub eligible: u32 }
pub enum Stat<T> { Ready { value: T, coverage: Coverage, excluded: Excluded }, Missing(Missing) }
pub struct Excluded { pub preorders: u32, pub unavailable: u32, pub no_duration: u32, pub duplicates: u32, pub removed: u32, pub undated: u32, pub not_inspected: u32 }
```

Decision procedure, in order:

1. Eligible set empty: `Missing::NoData`.
2. Used set empty and at least one eligible release lacks the needed category: `NotArchived(category)` (or `NotSynced` / `CoversNotLoaded` for those sources).
3. Used set empty and every eligible release was captured but nothing is usable (all tracklists empty, no durations): `NoData`.
4. Otherwise `Ready`. When `coverage.used < coverage.eligible` the card shows "Based on {used} of {eligible} releases." and any total that would be understated is prefixed with "at least".

Copy (constants in one place in fx, tested):

- `NotArchived(Tracklist)`: "Info not archived. Archive tracklists to see this stat."
- `NotArchived(Tags)`: "Info not archived. Archive tags to see this stat."
- `NotSynced`: "Not synced yet. Sync this artist to see this stat."
- `CoversNotLoaded`: "No cover colors yet. Covers are processed as you browse the wall."
- `NoData`: "Nothing to show for this artist." (cards may use a more specific second line, for example "No release has track durations.")

Cards with `NotArchived` also show a button "Open archive settings", which only navigates to the Archive tab of Settings. Nothing is started from the page.

### 3.4 Page artist and the "same artist" button

One page is one artist. The page artist is `ArtistFacts.page_names`. The matching helper in `credit.rs` is exact-first and intentionally small:

- `PageArtist { keys: HashSet<String> }` holds `textmatch::key(name)` for every page name; empty keys are dropped. If no key remains, the page credits nothing.
- The **credit** of a release is `ReleaseFacts.credit`. The credit of a track is `TrackFacts.artist` when present, else its release's credit.
- `is_page_artist(credit) = keys.contains(key(credit))`. Equality of keys only: no containment, no splitting of "A / B", no noise-word stripping (a multi-credit match is in section 9).

Record statistics (the ones that single out one release, track or name: see 3.6 H, I, J) return both variants, computed eagerly in core:

```rust
pub struct Record<T> {
    pub page_wide: T,              // best over everything on the page (the default)
    pub same_artist: Option<T>,    // best over items credited to the page artist, if any exist
    pub differs: bool,             // the page_wide item is not credited to the page artist
}
// every T carries `credited_to: String` and the ItemKey(s) to open
```

UI rules:

- `differs == false`: show the page-wide item and **nothing else**.
- `differs == true` and `same_artist.is_some()`: show the page-wide item with a caption "Credited to {credited_to}" and a button "Stat artist differs from page artist, show stat from same artist instead". Pressed, the card shows the same-artist item and the button text becomes "Showing page artist only, show page-wide stat". The toggle is per card and lasts for the session.
- `differs == true` and `same_artist.is_none()` (a label page, or nothing credited to the page artist in that domain): show the page-wide item with the caption and **no button**. The page never tries to work out who the other artists are.
- If the page credits nothing at all, every `same_artist` is `None`, so no button appears anywhere and nothing fails.
- Aggregate cards (totals, averages, histogram, cadence, mix, tags) are always page-wide and never have the button.

### 3.5 Duplicate tracks and the dedupe flow

A standalone track can end up as a track of an album on the same page: the artist published it as a single, you synced, the artist moved it into an album, you synced again. The standalone row stays in your database while the album now lists the same track, so counting both would double count it. This is the only case treated as a duplicate, and the finding is itself a statistic.

- **Pair rule** (`dedupe.rs`): exact slug equality and nothing else. A release R of kind `track` with slug `s` is a duplicate when any album A on the same page has a captured track whose `slug` equals `s`. No title, duration, credit or fuzzy comparison is ever used, so pages whose tracks share names and lengths (drone, noise, loops, many "Untitled") are never touched. Slugs are compared as exact strings: `R.slug` is the path segment after `/track/` in `releases.url`, and the album side is the same segment of the track link archived in `tracks_json` (3.8, section 6).
- **No proof, no match.** If an album has no captured tracklist, or its entries carry no slug (for example rows imported from the legacy `release_detail`), nothing can be proven for it and no duplicate is reported on its account. The card shows how many albums could be checked.
- One standalone track found on several albums is one duplicate; all those albums are listed, earliest published first.
- `DedupeReport { pairs: Vec<DuplicatePair { standalone: ItemKey, albums: Vec<ItemKey> }>, albums_checked: u32, albums_total: u32 }` is always computed and shown by card M, whatever the option.
- **Option** `dedupe` (default **on**): the standalone release of each pair is dropped from every track-based computation (total length, total tracks, track-length mean, median and histogram, track-title records, most reused name); the album copy stays. The standalone is still a release: cadence, classification mix, release-title records and the downloaded share keep it. Album statistics are never affected. `Excluded.duplicates` reports how many were dropped.
- The toggle lives in the page header: "Count duplicate tracks once". It is session state; changing it re-queries.

### 3.6 The v1 statistics

Global exclusions, always reported in `Excluded` and never silently dropped: `unavailable` classification (0 tracks), releases flagged `removed` only when `include_removed` is off (default on, because the archive keeps them and they are part of the discography), and pre-orders in every length statistic (their tracklists are often partial).

| Card | Content | Needs | Eligible set | Missing handling |
| --- | --- | --- | --- | --- |
| **A. Overview** | Releases (albums and tracks), total tracks (sum of `track_count`, minus confirmed duplicates when dedupe is on), total length of the discography (sum of complete albums plus non-duplicate standalone tracks, shown as `3 d 4 h 12 m`) | Counts: sync. Length: tracklist | Counts: inspected releases. Length: non-pre-order releases with all durations | Counts show `NotSynced` if none inspected; length follows 3.3 and is prefixed "at least" when partial |
| **B. Album length** | Mean and median album length | Tracklist | Albums (kind `album`) whose tracks all have durations | 3.3; shows `no_duration` count |
| **C. Track length** | Mean and median track length across album tracks and non-duplicate standalone tracks | Tracklist | Releases with a captured tracklist | 3.3 |
| **D. Track-length histogram** | Track counts per bucket: under 1, 1-2, 2-3, 3-4, 4-5, 5-7, 7-10, 10-15, 15-30, over 30 minutes (lower bound inclusive) | Tracklist | Same tracks as C | 3.3; bucket counts sum to the track count in C |
| **E. Cadence** | First and latest release date, span in years, releases per year (bars), average gap and longest hiatus between consecutive distinct publish dates (both endpoints open the release drawer), days since the latest release | `published_at` | Releases with `published_at`, not in the future (future-dated ones are reported as upcoming) | `NotSynced` when none has a date; `undated` count shown. Label: "Bandcamp page publish date" |
| **F. Price mix** | Releases per classification (free, name your price, email required, paid, unavailable) and share; `unknown` is shown as "not inspected" | `classification` | All releases | `NotSynced` when every release is `unknown` |
| **G. Your library** | Downloaded share of all releases and of eligible releases (eligible as in the sidebar), eligible remaining, bytes downloaded through the app (`history_bytes`) | none beyond sync | All releases | `NoData` when the artist has no releases |
| **H. Title records** | Longest and shortest **release** title; longest and shortest **track** title (each shows the title, its length in characters, the credit and the release, and opens the drawer) | Release titles: none. Track titles: tracklist | All releases; tracks as C | Release records always available; track records follow 3.3. Same-artist button (3.4) |
| **I. Track-count records** | Album with the most and the fewest tracks (`track_count`, albums only, at least 1 track, not pre-orders) | sync | Inspected albums | `NotSynced`; same-artist button |
| **J. Most reused track name** | The track name that appears most often across releases (names compared by `textmatch::key`; titles with no alphabetic character are ignored; needs 2 or more occurrences; shows the most common spelling, the count and the releases) | Tracklist | Tracks as C | 3.3; `NoData` with "No track name is reused." when nothing repeats; same-artist button (the button appears when any occurrence of the winning name is not credited to the page artist) |
| **K. Cover color strip** | One tile per release with a cover, in publish-date order (undated last), colored by the cover's most populous swatch; an outlined tile when the cover is not processed; click opens the drawer. Reads colors only, never images | `cover_meta` | Releases with an `art_id` | `CoversNotLoaded` when none has a row; coverage "N of M covers processed" |
| **L. Top tags** | The 12 most common tags by number of releases carrying them (grouped by `textmatch::key`, shown in the most common spelling), plus the number of distinct tags | Tags | Releases with captured tags | `NotArchived(Tags)`; 3.3 |
| **M. Duplicate tracks** | Number of standalone tracks that are now also a track of an album (matched by slug), the pairs (click opens either release), and how many albums could be checked | Tracklist with track slugs | Standalone track releases; coverage is albums checked over all albums | `NotArchived(Tracklist)` when no album has a captured tracklist with slugs; `NoData` with "No duplicate tracks found." when albums were checked and none matched |

### 3.7 Details that must be deterministic

- **Ties** in every record: earliest `published_at` first (undated last), then smaller `item_id`. The card shows "+N more with the same value" and opens the list on hover.
- **Title length** is the number of Unicode scalar values of the trimmed title (no grapheme segmentation, so an emoji sequence counts as several). Empty titles are ignored. Display goes through `paint::display_layout_job` so invisible characters stay marked when `Settings::mark_invisible_chars` is on.
- **Median** of an even count is the mean of the two middle values. Durations are whole seconds as archived.
- **Years** in cadence use UTC.
- `fmt_duration(secs)` formats `3 d 4 h 12 m` from one day up, `4 h 12 m` from one hour, `12 m 07 s` otherwise; tested at every boundary.
- Anything computed from a partial set is prefixed "at least" only where the true value can only be larger (sums and totals). Means, medians and records show the coverage line instead.

### 3.8 Loader

`load_artist_facts` runs on the main reader pool. It selects the band's releases with `LEFT JOIN release_archive` on `(item_type, item_id)` (and the `missing_kind` column), the band's names from `artists`, and `SUM(bytes_size)` from `download_history` where `status = 'success'` for the band. `tracks_json` and `tags_json` are parsed inside the reader closure. `ReleaseFacts.slug` is derived from `releases.url` for `track` releases, and `TrackFacts.slug` is read from each `tracks_json` entry (`None` when absent, as in rows imported from the legacy `release_detail`). A mask bit that is set with a NULL value becomes `Captured(empty)`. A clear bit, a missing row or JSON that fails to parse becomes `NotCaptured` and increments a counter that is logged (counts only, never content). The query uses `idx_releases_band_pub`. The archive survives artist removal and re-adding because it has no foreign key; the loader therefore needs nothing special for that.

### 3.9 fx wiring

- `ViewMode::Insights` (label "Artist stats", slug `insights`), `ViewMode::ALL` grows to five, key `5` in `app.rs`, a palette command, and an "Artist stats" entry in the artist context menu of the sidebar (it selects the artist and switches view). `ViewMode::parse` keeps accepting every old value; old prefs load unchanged. Check that the toolbar tabs still fit at the minimum window width.
- Subject: `state.query.base.band_id`. With "All artists" selected, or a group selection (`band_ids` not empty), the view shows "Select one artist in the sidebar to see its stats." and sends nothing.
- State: `insights: Option<Box<ArtistInsights>>`, `insights_band: Option<i64>`, `insights_loading: bool`, `active_insights_req_id: u64`, `insights_dedupe: bool` (default true), `insights_include_removed: bool` (default true), `insights_same_artist: HashSet<RecordId>`. All session-only.
- Actions (each an `FxAction`, so `AGENTS.md` rule 15 is satisfied from the start and tests can call `dispatch`): `InsightsToggleDedupe`, `InsightsToggleRemoved`, `InsightsToggleSameArtist(RecordId)`, `InsightsOpenArchiveSettings`. `RecordId` is an enum with one variant per record in H, I and J.
- Re-query on: opening the view, artist change, either option, `WorkerToUiEvent::SyncFinished`, `WorkerToUiEvent::ReleaseUpdated` for the shown artist, and the archive job's progress and summary events once they exist (name them when implementing). Coalesce bursts: at most one in-flight query per view; a new trigger marks the state dirty and re-queries when the previous response arrives.
- Cover colors: the service reads `cover_meta` for the facts' `art_id`s in chunks of 500 ids. The page never sends `EnsureThumbs` or any thumbnail command.
- `InsightsOpenArchiveSettings` ends in `FxState::open_settings_modal` (the existing opener in `state.rs`), selecting the Archive tab that `ARCHIVE.md` adds; if the opener takes no tab yet, add a tab argument rather than a second code path.

### 3.10 Page layout

- Header: artist display name, a coverage line ("Tracklists archived: 40 of 60 releases. Tags archived: 12 of 60.") with the Open archive settings action when below 100 percent, and the two toggles ("Count duplicate tracks once", "Include releases removed from Bandcamp").
- Body: a single scrolling column. Overview odometer strip (card A, reusing `paint_odometer_u32` for counts), then cards B to M in the table's order, two per row when the width allows. Bars and tiles use `paint::rect` and `paint::rect_filled` with theme colors (`th.accent`, `class_color` for the mix); cover tiles use the artwork's own RGB. Every card has a title in the style of `views/stats.rs`, its coverage line and the excluded breakdown on hover. Zero corner radius everywhere.
- Motion honours `EffectsLevel` and `reduced_motion` (the odometer already does). Nothing animates at rest.

## 4. Tests

| # | Area | Scenario | Expected |
| --- | --- | --- | --- |
| 1 | credit | Keys across case and diacritics; `name` versus `custom_name`; page name whose key is empty; credit "A / B" for page artist "A" | Same key rule as the library matcher; empty keys dropped; "A / B" does not match |
| 2 | missing | None archived; some archived; all archived but no durations; no eligible releases; none inspected | `NotArchived`; `Ready` with partial coverage; `NoData`; `NoData`; `NotSynced` |
| 3 | missing | Archived-empty (bit set, NULL) versus not archived | Counts as captured with zero tracks, never `NotArchived` |
| 4 | fmt | `fmt_duration` at 0, 59 s, 1 m, 59 m 59 s, 1 h, 23 h 59 m, 1 d, 400 d | Exact strings from 3.7 |
| 5 | length | Album with one missing duration; pre-order; standalone track; one 40-minute track among short ones | Album excluded and counted in `no_duration`; pre-order excluded; track in C but not B; mean and median differ as expected |
| 6 | length | Median of even and odd counts; totals with partial coverage | Per 3.7; total flagged partial |
| 7 | histogram | Durations 59 s, 60 s, 7 min, 30 min | Correct buckets; bucket sum equals track count in C |
| 8 | records | Track-count and title ties; fewest ignores 0-track and pre-orders; empty titles | Deterministic winner and "+N more" |
| 9 | cadence | Same-day bulk upload, future-dated, undated, one release only | Distinct-date gaps; upcoming and undated counted separately; no gap for one release; hiatus endpoints correct |
| 10 | mix | Counts per class; all `unknown`; `unavailable` | Correct; `NotSynced`; shown separately |
| 11 | library | Downloaded share and eligible share on a fixture | Equal to the sidebar `ArtistStats` for the same data |
| 12 | titles | Accents, emoji, invisible characters, 1-character and very long titles | Scalar-count length; ties per 3.7 |
| 13 | reuse | Same name with different case and accents; digit-only titles; a single occurrence | Grouped by key; digit-only ignored; `NoData` |
| 14 | same-artist | Winner credited to the page artist | `differs == false` |
| 15 | same-artist | Winner credited to someone else, page artist has own items | `differs == true`, `same_artist` is the best page-credited item |
| 16 | same-artist | Label page where nothing is credited to the page artist | `same_artist` is `None` everywhere, no panic, captions name the credits |
| 17 | same-artist | Track `artist` overrides the release credit; split release | Credit resolution per 3.4 |
| 18 | tags | Grouping by key, display spelling, distinct count, none archived, archived-empty | Per 3.6 L and 3.3 |
| 19 | colors | Order, undated last, missing `cover_meta`, none loaded, achromatic cover | Per 3.6 K; `CoversNotLoaded` when none |
| 20 | dedupe | Standalone slug equals an album track slug; same title, same duration and different slug (drone-page case); album without a captured tracklist or without slugs; one standalone on two albums | Matched; not matched; not matched and reflected in `albums_checked`; one duplicate with both albums listed |
| 21 | dedupe | Option on and off | Off changes nothing; on drops duplicates only from track-based stats; cadence, mix, release titles and album stats unchanged |
| 22 | removed | `include_removed` on and off with `removed` and `unlisted` releases | Counts and `Excluded.removed` change as specified; archive rows still used |
| 23 | loader | Temp database with an archived, an archived-empty and a non-archived release, a second band, bad `tracks_json` | Facts match; bad JSON becomes `NotCaptured`; other band untouched |
| 24 | loader | Remove the artist and add it again | Archive rows reattach; facts complete without a fetch |
| 25 | perf | 5 000 releases with 12 tracks each | Completes under 250 ms in release mode |
| 26 | service | Stale response; artist change in flight; more than 500 cover ids | Stale dropped; latest wins; all colors found |
| 27 | state | `SetView(Insights)` with no artist, with an artist, with a group selection | No query; one query; no query |
| 28 | state | Each toggle action | Flips state, re-queries once, `insights_same_artist` per `RecordId` |
| 29 | side effects | Open the view, switch artist, toggle everything | `core_tx` receives nothing; no `EnsureThumbs`; no database write |
| 30 | view | `ViewMode` parse and `as_str` round trip, old prefs, `ALL` length, key `5` | Pass |
| 31 | guard | Add `reqwest` or `BandcampClient` text under the insights paths | `scripts/guards.sh` fails |

## 5. Decisions made for you (change only through `AGENTS.md` §8)

- One page is one artist, whatever it contains. There is no logic that tries to identify other artists on a label or compilation page.
- Record statistics default to page-wide. The same-artist alternative is a per-card button, shown only when the winner is credited to someone else **and** a page-credited alternative exists. A label page where nothing matches shows no button and no error; each record still names who it is credited to. Matching is exact on `textmatch::key`; multi-credit matching is in section 9.
- Aggregate cards are always page-wide.
- Database only. Missing data produces a message, never a fetch. The only action offered is navigating to the Archive settings.
- Dedupe is on by default and rests on exact slug equality only (a standalone track whose slug is now a track of an album on the same page). Titles and durations are never compared, and what cannot be proven is not reported.
- Removed and unlisted releases are included by default.
- Pre-orders are excluded from length statistics.
- Price mix means classification only in v1, because the stored pricing signals have no currency.
- All toggles are session-only. No `Settings` field, no preference, no migration, so the settings parity rule (`AGENTS.md` rule 16) is untouched.
- fx only. The classic UI has no archived-text display and gets no stats page.
- Internal name `insights`, user-facing label "Artist stats".

## 6. Interaction with other plans

- `ARCHIVE.md`: this plan reads `release_archive` (`captured_mask`, `tracks_json`, `tags_json`). Keep those column names and meanings, or update only the loader (3.8). **Required change in `ARCHIVE.md`** (already applied to 3.2 and test 1 of that plan): each `tracks_json` entry also stores `slug`, the track page slug from the `trackinfo` entry's link, when the page provides one. Dedupe (3.5) cannot work without it, and adding it after the archive is populated means refetching pages. The archive's `raw_page` and cover table are not used in v1.
- `AUTO_SYNC.md`: reads `missing_kind` for `include_removed`; invalidates on `SyncFinished`.
- `ARTIST_GROUPS.md`: a group selection shows the "select one artist" state; group statistics are in section 9.
- `KEYBOARD.md`: the view, toggles and record buttons are `FxAction`s from the start. If `KEYBOARD.md` has landed, register them in the shortcut registry and the walkthrough in the same commits (rule 15); if not, its audit covers them.
- `SETTINGS_TABS.md`: no dependency. `Open archive settings` uses the Archive tab added by `ARCHIVE.md`.
- `USER_THEMES.md`: use only theme tokens, so every built-in and user theme works.

## 7. Commit sequence

One commit per step, every commit green. Split a step along its seam if it exceeds about 400 changed lines.

1. `ci(tooling): forbid network access in insights code` adds a `guard` in `scripts/guards.sh` that fails on `reqwest`, `BandcampClient`, `bandcamp::` or `fetch_` under `crates/bcvault-core/src/insights`, `crates/bcvault-core/src/db/repo/insights.rs` and `crates/bcvault-fx/src/services/insights.rs` (paths that do not exist yet simply print nothing). Test 31.
2. `feat(insights): add facts, missing-info states and page-artist matching` covers 3.2, 3.3 (types and decision procedure), `credit.rs`, `fmt_duration`, `Excluded` (tests 1 to 4).
3. `feat(insights): compute length, cadence, mix and library stats` covers cards A to G (tests 5 to 11, 22), and corrects the comment on `ArtistStats.total`. Develop the length parsing against the captured real album fixture.
4. `feat(insights): compute title, track-count, reuse, tag and color stats` covers cards H to L with `Record<T>` and both variants (tests 12 to 19).
5. `feat(insights): detect duplicate tracks and apply the dedupe flow` covers 3.5 and card M (tests 20 and 21), plus `compute_insights` and the performance test 25.
6. `feat(db): load artist facts from the main database` covers 3.8 (tests 23 and 24). No migration.
7. `feat(fx): query artist insights off the UI thread` adds the service, commands, events, state fields, actions, request guard and cover chunking (tests 26 to 29).
8. `feat(fx/stats): add the artist stats view` adds `ViewMode::Insights`, key `5`, toolbar tab, palette and sidebar entries, header with toggles, the empty and missing-info states, and cards A to G (test 30).
9. `feat(fx/stats): add record, tag, color and duplicate cards` adds cards H to M, the same-artist buttons and captions, and the Open archive settings action.
10. `docs(insights): document artist stats` adds `docs/artist-stats.md` (every statistic's definition, eligible set and exclusions, the missing-info messages, the same-artist button and the dedupe rule), the view and key `5` in `docs/user-guide.md`, the README feature list, the `ROADMAP.md` status and `PROGRESS.md`.

## 8. Definition of done

- `scripts/check.sh` green on every commit; the guard of step 1 is active.
- With the archive fully captured, every card shows a value and coverage "N of N". With nothing archived, cards that need it show the "Info not archived" message and the others still work. With a partly archived artist, cards show partial coverage and "at least" on totals.
- Opening the page and using every control provably sends no worker command and no network request (test 29).
- On a label or split page, record cards show "Credited to X"; the same-artist button appears only where a page-credited alternative exists, and a page that credits nothing shows no button.
- Old databases and old fx prefs load unchanged; no migration was added.
- **Manual (ask the user):** open the page for (a) a fully archived artist, (b) a partly archived one, (c) a label or split-heavy page, (d) an artist with a standalone track that is also on an album if one exists, and confirm the numbers against what they know.
- `PROGRESS.md` updated in every commit; tag per `AGENTS.md` rule 13 when released.

## 9. Backlog: add once v1 works

Each item names what it needs. None is part of this plan's commits.

**Length and catalog**
- Longest and shortest album; longest and shortest track (records with the same-artist button). Needs the tracklist.
- Average tracks per album; album with the widest gap between its longest and shortest track; tracks under 10 seconds; single-track albums; album closest to an exact length. Needs the tracklist.
- Single, EP and LP split by track count and length (a labelled heuristic). Needs the tracklist.

**Cadence**
- Busiest year and month; most common release weekday. Needs only `published_at` (remember it is the page publish date, so back-catalog uploads cluster).

**Price and library**
- Average and highest price. Needs the currency stored in the pricing signals at sync time (a sync change, then a backfill by re-inspection).
- Downloaded formats mix, bytes per format, missing and archived-file counts. Uses `downloaded_format`, `download_history`, `local_status`.

**Titles and text**
- Most common word in titles; self-titled releases; counts of "(Remix)", "(Live)", "(Demo)" and "feat."; digit-only titles (the stat J ignores); non-Latin or emoji titles.
- Longest liner notes by word count; share of releases with credits; releases with no about text. Needs archived `about` and `credits` (`ArchiveCategory::About`, `Credits`).

**Covers**
- Most vivid, darkest and brightest cover from `cover_meta` (`chroma`, `luma`).
- Original upload resolution. Needs covers archived at Original size (`archive.sqlite`, a second reader).

**Tags**
- Separate location tags from genre tags. Needs the archive to store the tag kind.
- Tag drift by year. Needs tags and `published_at`.

**Tracks and credits**
- Most frequent collaborator parsed from credits. Fuzzy; likely skip.

**Scope and polish**
- Multi-credit matching for the same-artist button ("A / B", "A & B", "feat."), with a documented separator list.
- A page-wide same-artist scope for aggregate cards.
- Group-level and library-wide versions of these statistics.
- Persist the toggles in the fx prefs; copy the page as text; export.

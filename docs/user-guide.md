# User Guide

`bcvault` provides two desktop graphical applications and a headless CLI that share the same database (`bcvault.db`) and background engine (`bcvault-core`):

- **`bcvault-fx` (Eye-Candy Edition)**: Visual archival interface with a virtualized Cover Wall, FLIP spring layout animations, OKLab perceptual color filtering, slide-over Detail Drawer with audio preview streaming, fuzzy Command Palette (`Ctrl+K`), and interactive collection statistics.
- **`bcvault` (Classic Edition)**: High-density, low-latency spreadsheet table interface designed for rapidly triaging, filtering, and batch-downloading large discographies.
- **`headless` CLI Example**: Command-line interface for adding artists, syncing catalogs, and batch-downloading releases without a GUI.

---

## 1. Launching and Data Directory Configuration

Only one instance of `bcvault` or `bcvault-fx` may run against the same data directory at a time (enforced via `{data_dir}/bcvault.lock`).

### Default Platform Paths
| Platform | Data Directory (`bcvault.db`, `fx.sqlite`, `logs/`) | Cache Directory (`covers/`) | Default Download Directory (`out_dir`) |
| :--- | :--- | :--- | :--- |
| **Linux** | `~/.local/share/bcvault/` | `~/.cache/bcvault/covers/` | `~/Music/bcvault/` (or `~/Downloads/bcvault/`) |
| **Windows** | `%APPDATA%\bcvault\bcvault\data\` | `%LOCALAPPDATA%\bcvault\bcvault\cache\covers\` | `%USERPROFILE%\Music\bcvault\` |

### Custom Data Directory Override
You can isolate multiple vaults or run portable installations via CLI flag or environment variable:

```bash
# Using CLI flag
bcvault-fx --data-dir /path/to/custom_vault
bcvault --data-dir=/path/to/custom_vault

# Using environment variable
BCVAULT_DATA_DIR=/path/to/custom_vault bcvault-fx
```

---

## 2. Managing Artists

### Adding an Artist
Click **+ Add** in the top toolbar (or press `Ctrl+K` -> `Add Artist...` in `bcvault-fx`) and enter any of the following:
- **Artist Slug**: `sigur-ros`
- **Bandcamp URL**: `https://sigur-ros.bandcamp.com` or `https://sigur-ros.bandcamp.com/music`
- **Custom Domain**: `https://music.example.com`

You can also choose **Add Artist + Link Folder...** to add a Bandcamp artist and immediately bind one of your local music folders to it in a single operation.

### Renaming and Removing Artists
Right-click any artist in the left sidebar:
- **Rename Artist...**: Sets a custom display name (`custom_name`) in the sidebar and views without altering the underlying Bandcamp artist metadata. Leave blank to restore the scraped Bandcamp name.
- **Remove Artist...**: Deletes the artist and its catalog rows from `bcvault.db` (local audio files on disk are never touched, and rolling 30-day quota records in `download_history` are preserved).

---

## 3. Synchronizing Discographies

Select an artist in the sidebar (or choose **All Artists**) and click **Sync** after choosing a **Sync Mode**:

| Sync Mode | Behavior | When to Use |
| :--- | :--- | :--- |
| **Incremental** *(default)* | Fetches `/music` to discover new releases and inspects only releases whose metadata has not yet been fetched (`classification = 'unknown'`). | Routine daily/weekly checks for new releases. |
| **Check Downloaded** | Performs an Incremental sync **plus** re-fetches metadata for all releases marked `downloaded = 1` to detect bonus tracks (`+TRACKS` badge) or page modifications (`MOD` badge). | Checking if artists added bonus tracks or remastered existing albums in your collection. |
| **Full Refresh** | Re-fetches every release page in the artist's discography while preserving your `downloaded` flags, `local_path` links, and library history. | Re-auditing pricing changes across an entire catalog. |

### Automatic Post-Sync Library Checks
When an artist has linked local folders:
- **`verify_on_sync`** *(default: on)*: Verifies that all downloaded releases still exist on disk before syncing.
- **`rescan_linked_on_sync`** *(default: on)*: Rescans linked local folders after syncing, automatically adopting high-confidence (`>= 92%`) matches and opening the **Match Report** if any items require manual review.

---

## 4. Release Classifications and Manual Overrides

Every release is assigned one of six classifications:

| Badge | Meaning | Counts Toward `Eligible`? | Auto-Downloadable? |
| :--- | :--- | :---: | :---: |
| **`FREE`** | Direct free download without checkout or email requirement. | Yes | Yes |
| **`NYP`** | Name-Your-Price (`$0.00` minimum); downloadable via zero-dollar checkout. | Yes | Yes |
| **`EMAIL`** | Free download gated behind an email address submission (`require_email`). | Yes | Via **Redeem URL** |
| **`PAID`** | Fixed-price commercial release (`minimum_price > 0`). | No | No |
| **`UNAVAILABLE`** | Release has `0` tracks on Bandcamp (no download button exists on Bandcamp, e.g., external-link showcase pages) or was manually marked unavailable by the user. | No | No *(Folder matching still supported)* |
| **`UNKNOWN`** | Discovered on `/music` grid; awaiting release page inspection. | No | After sync |

### Manual Status & Availability Toggles
Right-click any release row/card (or use the Detail Drawer in `bcvault-fx`):
- **Mark as Downloaded / Mark as Not Downloaded**: Manually toggles the `downloaded` state (`origin = 'manual'`).
- **Mark as Unavailable**: Marks one or more selected releases as `UNAVAILABLE` so they no longer count toward `eligible` downloads (useful for broken or external-only releases).
- **Re-evaluate Availability (Unmark Unavailable)**: Restores the release's classification offline from its cached pricing signals (provided the release has `> 0` tracks).

---

## 5. Downloading Releases and Quota Protection

### Enqueuing Downloads
- **Download Selected**: Select rows/cards and click **Download Selected** (or press `Ctrl+Enter` in `bcvault-fx`).
- **Download All Eligible**: Enqueues all undownloaded `FREE` and `NYP` releases in the current view (excluding pre-orders), capped at `batch_limit` per batch.

### Rolling 30-Day Quota Protection
Bandcamp allocates each artist a limited pool of free `NYP` downloads per month. To protect artists from exhausting their free download credits:
- `bcvault` tracks `NYP` checkouts per artist over a rolling 30-day window in `download_history`.
- **Soft Warning (`quota_soft`, default `150`)**: Displays a confirmation modal before enqueuing jobs that would cross the soft threshold.
- **Hard Cap (`quota_hard`, default `200`)**: Prevents enqueuing beyond the hard limit for that artist.
- **Server-Side Exhaustion**: If Bandcamp returns a quota-exhausted error during `buy_for_nothing`, `bcvault` immediately halts remaining queued downloads for that artist.

### Redeeming Email-Gated Releases (`EMAIL`)
`bcvault` never spoofs or bypasses email gates. To download an `EMAIL` release:
1. Click **Open Page** on the release to request the download link on Bandcamp in your browser.
2. Copy the download link from your email (`https://bandcamp.com/download?...`).
3. Click **Redeem URL** in `bcvault`, paste the link, and `bcvault` will validate the link, download and extract the audio archive in your preferred format, and link it to the release (`origin = 'redeem'`).

### Output Layouts and Custom Filename Patterns
In **Settings**, configure where downloaded archives and extracted folders are placed:
- **Layout**:
  - `Flat`: `{out_dir}/{stem}`
  - `By Album Artist`: `{out_dir}/{release_artist}/{stem}`
  - `By Page Artist`: `{out_dir}/{page_artist}/{stem}`
- **Custom Filename Pattern (`filename_pattern`)**:
  - Leave blank to use Bandcamp's `Content-Disposition` filename (or `{artist} - {title}`).
  - Supported placeholders (case-insensitive, supports both `{tag}` and `%tag%` syntax):
    - `{artist}` / `{album_artist}`: Release artist name
    - `{page_artist}`: Bandcamp page / label name
    - `{album}` / `{title}`: Release title
    - `{year}`: 4-digit release year (`YYYY`)
    - `{date}`: Release date (`YYYY-MM-DD`)
    - `{format}`: Audio format label (`FLAC`, `MP3 320`, etc.)
    - `{type}`: `album` or `track`
    - `{id}`: Numeric Bandcamp item ID
  - Empty brackets `()`, `[]`, and dangling ` - ` separators are automatically collapsed if a field (such as `{year}`) is missing.

---

## 6. Linking Local Folders and Reconciling Matches

See [`docs/library.md`](library.md) for the complete technical specification of the library sync engine.

### Quick Workflow
1. **Link Folder**: Right-click an artist in the sidebar and choose **Link Folder...** (or **Add Folder...** to attach additional folders across drives).
2. **Match Report Dialog**:
   - **Matched**: Exact matches and high-confidence (`>= 92%`) fuzzy matches, pre-selected for adoption.
   - **Review**: Fuzzy or containment proposals (such as a folder named `"Wall Split Action 001"` matching releases titled `"split"`).
     - Each release header shows its disambiguating Bandcamp slug (`[album/split-3 · 2023 · 4 trk · FREE]`) and an **Open** button.
     - Choose a candidate radio button to adopt, **Skip for now**, or **Wrong, don't ask again** (individually or in bulk via **Mark all as wrong (don't ask again)** / **Mark skipped as wrong**).
     - Rejected pairs are stored in `artist_folder_rejected` so future scans never ask about them again, and unlinked folders whose false review pairings were rejected fall through to **Local Only**.
   - **Local Only**: Local folders that did not match any release automatically.
     - Choose **Leave**, **Ignore in future**, or **Attach to release...**.
     - **Searchable URL / Slug Picker**: When attaching a folder, type in the **Search / URL / Slug** box to filter by title, year, or slug, or paste a full Bandcamp URL (`https://artist.bandcamp.com/album/split-3`) or slug (`album/split-3` or `split-3`) to guarantee matching the exact release page even when an artist has 10 albums titled `"split"`.
   - **Conflicts**: Colliding exact matches (e.g., two local format copies or identical album titles). Each local folder in a conflict can be left unresolved, ignored, or matched to a specific release using the same searchable URL/slug release picker.
   - **Found Again**: Previously `archived` releases whose files reappeared on disk.
3. **Undo**: After applying any library batch, click **Undo** in the status toast/dock to revert all changes from that batch.

---

## 7. `bcvault-fx` Eye-Candy Edition Guide

### Primary Views (`1` / `2` / `3` / `4`)
1. **Cover Wall (`1`)**:
   - Virtualized masonry/grid rendering 100,000+ covers with zero off-screen overhead.
   - Card sizes: `S`, `M`, `L`, `XL` (adjust with `[` and `]`).
   - **OKLCH Color Strip**: Click any hue swatch in the filter bar to filter releases by dominant cover artwork color.
   - **FLIP Animations**: Filtering, sorting, and resizing smoothly animate visible cards to their new coordinates.
2. **Table View (`2`)**:
   - High-density sortable table with inline cover thumbnails, classification pills, format badges, and library status indicators.
3. **Artists View (`3`)**:
   - Artist cards with cover mosaics, completeness bars, quota meters, and folder health badges.
4. **Stats View (`4`)**:
   - **Collection Treemap**: Proportional breakdown of your vault by artist, classification, and download state.
   - **Release Calendar Heatmap**: Publication and acquisition density across years and months.

### Slide-Over Detail Drawer (`Enter`)
Press `Enter` on any focused release (or double-click a card) to open the Detail Drawer:
- Displays high-resolution `Hero` (`700×700`) artwork with an adaptive OKLab background wash.
- After `400 ms`, lazily loads the full tracklist, track durations, `about` liner notes, `credits`, and genre tags.
- Click the play button next to any track to stream a 128 kbps preview directly inside the drawer.
- Displays the release's **Library History** timeline (`ADOPTED`, `RELINKED`, `ARCHIVED`, `RESTORED`, `UNMARKED`).

### Command Palette (`Ctrl+K` / `Ctrl+P`)
- Type freely to fuzzy-search releases, artists, and actions.
- Prefix with `@` to filter artists (`@sigur`).
- Prefix with `#` to apply classification/status filters (`#free`, `#unavailable`, `#missing`).
- Prefix with `>` to run application commands (`>sync`, `>verify`, `>theme`).

---

## 8. Complete Settings Reference

| Setting | Default | Description |
| :--- | :--- | :--- |
| `out_dir` | Platform Music/Downloads `bcvault/` | Global directory where newly downloaded releases are saved. Cannot overlap with any linked artist folder. |
| `format` | `Flac` | Preferred download audio format (`Flac`, `Mp3_320`, `Mp3V0`, `Wav`, `AiffLossless`, `Alac`, `Vorbis`, `AacHi`). |
| `format_fallback` | `Nearest` | `Nearest` selects the closest lossless/lossy format if preferred is unavailable; `Strict` fails the job. |
| `layout` | `Flat` | Subdirectory structure under `out_dir` (`Flat`, `ByArtist`, `ByPageArtist`). |
| `filename_pattern` | `""` (server default) | Custom stem template (`{year} - {artist} - {album}`, etc.). |
| `auto_extract` | `true` | Automatically extract album `.zip` archives into folders. |
| `delete_zip_after_extract` | `false` | Delete the `.zip` archive after successful extraction. |
| `batch_limit` | `150` | Maximum jobs enqueued in a single batch (`25`, `50`, `100`, `150`, `200`). |
| `quota_soft` | `150` | Rolling 30-day per-artist NYP download warning threshold. |
| `quota_hard` | `200` | Rolling 30-day per-artist NYP download hard cap. |
| `page_delay_ms` | `1500` | Base delay (`ms`) between HTML page requests in `BandcampGate`. |
| `page_jitter_pct` | `25` | Random jitter percentage (`±%`) applied to `page_delay_ms`. |
| `max_concurrent_downloads` | `2` | Maximum simultaneous binary downloads across different artists (`1` per artist). |
| `show_thumbnails` | `false` | Enable inline cover thumbnails in the Classic `bcvault` table view. |
| `user_agent` | `bcvault/<ver> (personal archiving tool)` | Custom HTTP `User-Agent` string. |
| `verify_on_sync` | `true` | Verify local files before syncing an artist with linked folders. |
| `rescan_linked_on_sync` | `true` | Rescan linked folders for new local albums after syncing. |
| `on_missing` | `Ask` | Action when verified files are missing on a mounted drive (`Ask` or `Auto`). |
| `deleted_policy` | `KeepMarked` | How deleted local albums are treated (`KeepMarked` = archive workflow; `MarkNotDownloaded` = reset). |
| `auto_relink` | `true` | Silently relink missing albums found moved/renamed inside the artist's linked folders. |
| `auto_match_min` | `0.92` (92%) | Minimum fuzzy score (`0.80..=0.98`) for automatic adoption (with `>= 5%` runner-up margin). |
| `review_match_min` | `0.75` (75%) | Minimum fuzzy score (`0.50..=0.90`) to propose a candidate in the **Review** tab. |
| `folder_style` | Templates: `["{title}"]`, Noise: `[]` | Custom folder naming templates (`{artist}`, `{year}`, `{title}`, `{noise}`) and explicit noise tokens. |

---

## 9. Headless CLI Usage

Run headless commands via `cargo run --release -p bcvault --example headless -- <command>`:

```bash
# 1. Add and sync an artist by slug or URL
cargo run --release -p bcvault --example headless -- add https://artist.bandcamp.com

# 2. Download all eligible undownloaded releases for band_id in FLAC to ./music
cargo run --release -p bcvault --example headless -- download 123456789 flac ./music
```

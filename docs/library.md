# Artist Folder Links and Library Sync

`bcvault` includes an offline, non-destructive library sync engine that binds local music folders to Bandcamp artist pages. It scans local audio files, matches them against tracked discographies, tracks file availability on disk, and supports an archive workflow when albums are pruned or moved.

---

## 1. Core Principles

1. **Local-Only I/O**: Scanning, tag reading, matching, and disk verification perform filesystem operations only. No network requests are made to Bandcamp for library synchronization.
2. **Never Modify or Delete User Files**: `bcvault` strictly reads filesystem metadata (file existence, directory listings, audio tags). It never creates, alters, renames, moves, or deletes user music files.
3. **Many Folders per Artist, One Artist per Folder (N:1 Binding)**: A tracked artist page can link to multiple local folders (e.g., across multiple drives, separate album roots, or project directories). However, each folder belongs strictly to one artist: linked folders may not equal, contain, or sit inside any other linked folder (belonging to another artist or the same artist) or the global download directory (`out_dir`).
4. **Absence of Evidence is Not Deletion**: If an external hard drive, USB volume, or network share is unmounted or unreachable, releases under it are reported as `Unknown`, never `Missing`.
5. **Non-Destructive Batch Changes with Undo**: Every automatic or manual status modification creates a batch entry in `library_batches` with before/after audit entries in `library_events`. Any batch can be undone with a single click.

---

## 2. Linking Artist Folders

You can link local music folders to an artist page from either front-end:

- **Sidebar Context Menu**: Right-click an artist in the left sidebar and select **Link Folder...** (or **Add Folder...** if already linked to one or more folders). Per linked folder, submenus allow **Folder moved: change <folder>...** and **Unlink <folder>...** (disambiguated by parent path if folder basenames collide).
- **Toolbar**: Click the **Link Folder...** button in the top toolbar.
- **Settings**: Open Settings to view all currently linked artist folders with live health status dots and individual unlink controls.
- **Command Palette (`bcvault-fx`)**: Press `Ctrl+K` and type `Link Folder to Artist...`.

### Validation Rules
When linking or changing a folder, `bcvault` verifies:
- The path exists, is a directory, and is readable.
- The path is canonicalized (stripping Windows extended-length `\\?\` prefixes via `dunce`).
- The path does not overlap with `out_dir` (it cannot be equal to, inside, or a parent of `out_dir`). The check uses the configured download directory, or the application's default download directory until settings are first saved, so the download directory can never be linked to an artist.
- The path does not overlap with any folder belonging to another artist.
- The path does not overlap with any other folder already linked to the same artist (`AlreadyCovered` or `ContainsOwnFolder`).
- **Soft Warning**: If the folder contains subdirectories that look like separate artists rather than albums, the dialog displays a non-blocking warning: *"Link one artist's folder, not a library"*.

---

## 3. Candidate Scanning & Format Guessing

When a linked folder is scanned, the engine identifies **album candidates**:
- **Direct Child Folders**: Folders directly within the linked directory containing at least one supported audio file (`.flac`, `.mp3`, `.ogg`, `.opus`, `.m4a`, `.wav`, `.aif`, `.aiff`).
- **Multi-Disc Sets**: Folders where direct children are disc subfolders (e.g., `CD1/`, `Disc 2/`) are automatically collapsed into a single album candidate.
- **Zip Archives**: Standalone album archives containing audio files.
- **Single-Album Roots**: If the linked folder itself directly contains audio tracks (common for single-album artists), the linked folder itself is treated as a candidate.

### Format Guessing
Audio tags and stream headers are inspected using `lofty` to guess the download format:
- `FLAC` -> `Flac`
- `WAV` -> `Wav`
- `AIFF` -> `AiffLossless`
- `OGG` -> `Vorbis`
- `M4A` (ALAC) -> `Alac`
- `M4A` (AAC) -> `AacHi`
- `MP3` (CBR ~320 kbps) -> `Mp3_320`
- `MP3` (VBR ~220-260 kbps) -> `Mp3V0`

### Text Normalization & Name Keys
Folder names and release titles are converted to canonical name keys:
1. Converted to lowercase.
2. Decomposed to canonical NFD with combining accent marks removed.
3. Non-decomposing European letters mapped (e.g., `ø` -> `o`, `ł` -> `l`, `ß` -> `ss`, `æ` -> `ae`, `œ` -> `oe`).
4. Apostrophes deleted (`Don't` -> `dont`).
5. All other non-alphanumeric characters become whitespace separators.
6. Split into tokens, preserving Unicode alphanumeric words (including CJK and Cyrillic).

### Folder Style & Templates
Instead of guessing or implicitly stripping noise and tags, `bcvault` lets you declare how folders are named via **Folder Style**:
- **Placeholders**: `{title}` (required), `{artist}`, `{year}`, and `{noise}`.
- Punctuation, dashes, and brackets in templates are treated as separators; only the token sequence matters. For instance, `{title} ({year})` matches `Album [2019]`, `Album - 2019`, and `Album (2019)`.
- **Noise List**: Decorations like audio formats or bitrates are declared explicitly in the noise list (e.g., `FLAC`, `MP3`, `320`, `WEB FLAC`) and recognized only through the `{noise}` placeholder. Noise is never stripped implicitly.
- If a folder name does not match any custom template, the raw folder name key is always tested as a fallback.

---

## 4. Matching & Scoring

Matching uses a deterministic **exact-first** architecture, followed by fuzzy scoring only for leftover unmatched releases and candidates.

### 4.1 Exact-First Pass
Before calculating fuzzy Levenshtein distances or running scoring heuristics, the engine builds exact match edges:
- **Bandcamp URL Tag**: Audio file comment or tag contains the exact Bandcamp release URL.
- **Exact Folder Name**: The folder name (or extracted title from a matching folder template) exactly equals the release title key.
- **Exact Album Tag**: The embedded album tag exactly equals the release title key.

#### Exact Match Resolution
1. **Authoritative URL Tags**: If a candidate and a release share a unique, 1:1 Bandcamp URL tag, they match decisively and are removed immediately.
2. **Connected Components**: For remaining exact edges:
   - A single-edge component (exactly one candidate matching one release) is decisively **Matched** (or **Restorable** if already downloaded and archived).
   - Multi-edge components (e.g., duplicate formats on disk, or a folder matching release A by folder name but release B by album tag) produce a **Conflict** with an explicit explanatory description.
3. Matched and conflicted nodes are completely excluded from the fuzzy pass. Exact matches never compete with near-misses, and runner-up margins play no role in exact matching.

### 4.2 Fuzzy Scoring Pass (Leftovers Only)
Any releases and candidates not resolved in the exact pass are evaluated with fuzzy scoring:
- **Fuzzy Token Similarity**: Normalized Levenshtein distance on token sequences.
- **Digit Guard**: If both names contain numeric tokens (e.g., `Volume 1` vs `Volume 2`, or `Leiche 73` vs `Leiche 75`), their multisets of digit tokens must match identically. If numbers differ, similarity is clamped to `0.0`. Numbered series can never falsely match or create conflict cascades.
- **Containment Rule**: If one title is a strict token subset of the other (e.g., `Album` vs `Album Deluxe`), similarity is capped at `0.85` and flagged as `auto_eligible = false`. It will appear in **Review** and is never adopted automatically, regardless of score bonuses or slider settings.
- **Track Count & Year Signals**: Small score bonuses or penalties for matching track counts and release years.

### 4.3 Match Thresholds & Report Tabs
The settings sliders (`Auto-Match Min Score` and `Review Min Score`) control the acceptance of **non-exact (fuzzy) matches only**:
1. **Matched**: Exact matches from step 1, plus fuzzy matches scoring >= `auto_match_min` (default 92%) that beat their runner-up by >= 5%. Pre-selected for automatic adoption.
2. **Review**: Fuzzy proposals scoring >= `review_match_min` (default 60%) or containment matches requiring manual confirmation.
3. **On Page Only**: Tracked releases with no local files.
4. **Local Only**: Discovered folders that do not match any release on the artist's page.
5. **Conflicts**: Genuine duplicate formats or conflicting tag/folder identities from the exact pass. Fuzzy scoring never creates conflicts.
6. **Found Again (Restorable)**: Releases marked as `archived` whose audio files reappeared on disk.

---

## 5. Verification & Safeguards

During background synchronization (or on demand via **Verify Local Library Files**), `bcvault` verifies that all adopted releases are reachable on disk.

### Drive & Volume Probing
Before checking individual files, `bcvault` probes each storage root once per run:
- Each release is accurately attributed to its true root (`Linked { folder_id }` for whichever linked folder contains its path, or `OutDir` if stored under the global download directory). Releases outside an unavailable folder are still verified normally.
- Folders evaluate to one of three states:
  - **`FolderState::Ok`**: Path exists and is readable. File checks proceed normally.
  - **`FolderState::Missing`**: Folder no longer exists.
  - **`FolderState::Unreadable`**: Storage drive root is unmounted, volume is disconnected, or access timed out.

> [!IMPORTANT]
> When a folder probes `Missing` or `Unreadable`, releases under that folder evaluate to `VerifyResult::Unknown` instead of `Missing`.
> An unavailable folder **never auto-resolves**, even with `on_missing = Auto`. Instead, `bcvault` asks the user via the **Resolve Unavailable Folder** dialog:
> 1. **Drive disconnected (`Keep as is`)**: dismisses for the session without writing to disk or database.
> 2. **Moved folder (`Choose new folder...`)**: re-points the artist link and re-verifies.
> 3. **Deleted on purpose (`Apply`)**: applies the chosen deletion policy (`KeepMarked` or `MarkNotDownloaded`) as an undoable batch, with optional folder unlinking.
>
> Background syncs of all artists leave a `FOLDER n` warning chip in the status dock without interrupting work; clicking the chip or manual verification opens the dialog.

### Multiple Folders & Partial Availability Safeguard
When an artist links several folders (e.g., across internal and external storage) and one becomes offline while others remain online:
- Releases stored under the reachable folder are verified as usual.
- Releases under the unavailable folder evaluate to `FolderUnavailable`.
- **Partial-Availability Rule**: If an artist has at least one unavailable folder, missing albums under their available folders are **never auto-archived**, even when `on_missing = Auto`. Instead, they are routed to `MissingFound` (Ask) to safeguard against cross-volume confusion or unmounted partitions.
- Reconcile reports display a warning banner: `N of M folders unavailable; results may be incomplete`.
- In Reconcile reports, unassigned local folders (orphans) display their path relative to their owning folder (e.g. `ProjectB / Album`) whenever an artist has multiple linked folders.

### Mass-Missing Guard
If more than 30% of verified releases (minimum 20 albums) disappear in a single pass:
- Automatic policies are immediately halted.
- No database marks are applied automatically.
- The **Resolve Missing** dialog opens with an alert: *"Many albums are missing: is a drive disconnected?"*.
- Releases from unavailable folders (`Unknown`) are excluded from mass-missing ratio calculations, ensuring unmounted drives do not trigger false alarms for available files.

---

## 6. Moved Albums & Resolution Policies

When an album folder disappears, `bcvault` attempts resolution in the following order:

1. **Automatic Relinking**:
   If `auto_relink` is enabled (default), the artist's linked folders are rescanned. If the album was moved to another subfolder, renamed with score >= 92%, or moved from one linked folder of the artist to another linked folder of the same artist, `bcvault` relinks it silently with no user prompt.
2. **Missing Album Policy (`on_missing`)**:
   - **`Ask`** (default): Collects missing albums into the non-modal **Resolve Missing** queue and shows a warning chip in the status dock (`MISSING n`). Note that an unavailable folder always asks regardless of this setting.
   - **`Auto`**: Automatically applies the configured `deleted_policy`.
3. **Deleted Policy (`deleted_policy`)**:
   - Used when handled automatically, and as the default in resolution dialogs.
   - **`KeepMarked`** (default, Archive Workflow): Retains `downloaded = 1` and marks `local_status = 'archived'`. The album stays counted in your collection statistics and is visible in the *Archived* smart view, but is excluded from active disk checks.
   - **`MarkNotDownloaded`**: Clears `downloaded = 0`, `local_path = NULL`, and `local_status = NULL`.

---

## 7. Status Glyphs and Indicators

| Status | Classic Glyph | FX Indicator | Meaning |
| :--- | :--- | :--- | :--- |
| **`Present`** | Green filled square with checkmark `[✓]` | Accent color badge | Files verified on disk with audio tracks. |
| **`Archived`** | Hollow square with horizontal bar `[-]` | Dim `ARCHIVED` chip | Pruned from local disk but kept marked as downloaded. |
| **`Missing`** | Warning square with exclamation `[!]` | Warning `MISSING` chip | Path unreachable on disk while folder is mounted. |
| **`Folder Missing`** | Warning `FOLDER MISSING` badge | Warning `FOLDER MISSING` chip / card badge | Album's linked artist folder is missing or unreadable. |
| **`Unverified`** | Solid neutral square `[■]` | Default row style | Awaiting local disk check. |

### Sidebar Download Badges

Each artist row in the sidebar displays a download count badge summarizing collection completeness:
- **Standard Format (`{eligible dl}/{eligible}/{total}`)**: When an artist has only eligible (Free, NYP, Email Required) downloads, the badge shows `X/Y/Z` (e.g., `20/20/20`).
- **Expanded Format (`{eligible dl} (+{non-eligible dl})/eligible/page total`)**: When the user has downloaded paid releases or extra albums not listed on the live Bandcamp page, the non-eligible downloads are highlighted with `(+N)`.
  - Example: `2 (+1)/2/3` indicates 2 free releases downloaded, 1 paid release downloaded, out of 2 eligible and 3 total on the page.
  - Example: `40 (+2)/40/40` indicates all 40 free releases downloaded, plus 2 extra downloaded off-page albums.
Hovering over the badge displays a breakdown of eligible downloads, paid/extra downloads, eligible catalog total, and total page releases.

---

## 8. Smart Views and Timeline History

In `bcvault-fx`:
- **Smart Views**:
  - **Archived**: Filters catalog to releases with `local_status = 'archived'`.
  - **Missing files**: Filters catalog to releases with `local_status = 'missing'`.
- **Drawer Timeline**: Opening a release's detail drawer displays its complete `LIBRARY HISTORY` timeline (`ADOPTED`, `RELINKED`, `ARCHIVED`, `RESTORED`, `UNMARKED`) with exact timestamps and batch associations.

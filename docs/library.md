# Artist Folder Links and Library Sync

`bcvault` includes an offline, non-destructive library sync engine that binds local music folders to Bandcamp artist pages. It scans local audio files, matches them against tracked discographies, tracks file availability on disk, and supports an archive workflow when albums are pruned or moved.

---

## 1. Core Principles

1. **Local-Only I/O**: Scanning, tag reading, matching, and disk verification perform filesystem operations only. No network requests are made to Bandcamp for library synchronization.
2. **Never Modify or Delete User Files**: `bcvault` strictly reads filesystem metadata (file existence, directory listings, audio tags). It never creates, alters, renames, moves, or deletes user music files.
3. **One Folder per Artist (1:1 Binding)**: Each tracked artist page links to at most one local directory. A linked folder may not equal, contain, or sit inside another linked artist folder or the global download directory (`out_dir`).
4. **Absence of Evidence is Not Deletion**: If an external hard drive, USB volume, or network share is unmounted or unreachable, releases under it are reported as `Unknown`, never `Missing`.
5. **Non-Destructive Batch Changes with Undo**: Every automatic or manual status modification creates a batch entry in `library_batches` with before/after audit entries in `library_events`. Any batch can be undone with a single click.

---

## 2. Linking an Artist Folder

You can link a local folder to an artist page from either front-end:

- **Sidebar Context Menu**: Right-click an artist in the left sidebar and select **Link Folder...** (or **Change Folder...** if already linked).
- **Toolbar**: Click the **Link Folder...** button in the top toolbar.
- **Settings**: Open Settings to view all currently linked artist folders with live health status dots and unlink controls.
- **Command Palette (`bcvault-fx`)**: Press `Ctrl+K` and type `Link Folder to Artist...`.

### Validation Rules
When linking a folder, `bcvault` verifies:
- The path exists, is a directory, and is readable.
- The path is canonicalized (stripping Windows extended-length `\\?\` prefixes via `dunce`).
- The path does not overlap with `out_dir` (it cannot be equal to, inside, or a parent of `out_dir`).
- The path does not overlap with any other artist's linked folder.
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

### Text Normalization
Album names and folder names are normalized using Unicode NFKC, case folding, mapping `&` to `and`, stripping diacritics, removing punctuation, collapsing whitespace, and stripping common release noise tags (such as `[FLAC]`, `(320)`, `[WEB]`, `(Free Download)`, and trailing `(YYYY)` release years). Release-distinguishing tokens like `Deluxe`, `Remaster`, and `Edition` are preserved.

---

## 4. Matching & Scoring

The matcher executes a greedy 1-to-1 assignment between the artist's Bandcamp releases and the discovered local candidates.

### Scoring Signals
- **Exact URL Tag (`1.0`)**: If any audio file in the folder contains a Bandcamp release URL tag in its comments/metadata matching the release URL, the score is certain (`100%`).
- **Album Tag & Folder Name**: High-confidence agreement between audio tags and normalized folder names.
- **Track Count**: Penalties applied for large discrepancies in track counts.
- **Year Tag**: Agreement between tag release year and Bandcamp publication date.

### Match Report Tabs
1. **Matched**: High-confidence matches (score >= 92% and beating runner-up by >= 5%). Selected by default for immediate adoption.
2. **Review**: Ambiguous matches or candidates below the confidence threshold. Allows choosing between candidate variants or skipping.
3. **On Page Only**: Releases present on Bandcamp that have no corresponding local files.
4. **Local Only**: Local audio folders that could not be mapped to any release on the artist's page. Can be marked as *Leave unmatched*, *Ignore forever*, or manually attached to a release.
5. **Conflicts**: Ambiguities where multiple formats or candidate folders compete for the same release.
6. **Found Again (Restorable)**: Releases previously marked as `archived` whose audio files were detected on disk again.

---

## 5. Verification & Safeguards

During background synchronization (or on demand via **Verify Local Library Files**), `bcvault` verifies that all adopted releases are reachable on disk.

### Drive & Volume Probing
Before checking individual files, `bcvault` probes the artist's linked folder:
- **`FolderState::Ok`**: Path exists and is readable. File checks proceed normally.
- **`FolderState::Missing`**: Folder was deleted or moved.
- **`FolderState::Unreadable`**: Storage drive root is unmounted, volume is disconnected, or access timed out.

> [!IMPORTANT]
> If a folder probes `Missing` or `Unreadable`, all releases under that artist evaluate to `VerifyResult::Unknown`. They are **never** marked `missing`. An unmounted drive cannot mark your music collection as missing.

### Mass-Missing Guard
If more than 30% of verified releases (minimum 20 albums) disappear in a single pass:
- Automatic policies are immediately halted.
- No database marks are applied automatically.
- The **Resolve Missing** dialog opens with an alert: *"Many albums are missing: is a drive disconnected?"*.

---

## 6. Moved Albums & Resolution Policies

When an album folder disappears, `bcvault` attempts resolution in the following order:

1. **Automatic Relinking**:
   If `auto_relink` is enabled (default), the linked artist folder is rescanned. If the album was moved to another subfolder or renamed with score >= 92%, `bcvault` relinks it silently with no user prompt.
2. **Missing Album Policy (`on_missing`)**:
   - **`Ask`** (default): Collects missing albums into the non-modal **Resolve Missing** queue and shows a warning chip in the status dock (`MISSING n`).
   - **`Auto`**: Automatically applies the configured `deleted_policy`.
3. **Deleted Policy (`deleted_policy`)**:
   - **`KeepMarked`** (default, Archive Workflow): Retains `downloaded = 1` and marks `local_status = 'archived'`. The album stays counted in your collection statistics and is visible in the *Archived* smart view, but is excluded from active disk checks.
   - **`MarkNotDownloaded`**: Clears `downloaded = 0`, `local_path = NULL`, and `local_status = NULL`.

---

## 7. Status Glyphs and Indicators

| Status | Classic Glyph | FX Indicator | Meaning |
| :--- | :--- | :--- | :--- |
| **`Present`** | Green filled square with checkmark `[✓]` | Accent color badge | Files verified on disk with audio tracks. |
| **`Archived`** | Hollow square with horizontal bar `[-]` | Dim `ARCHIVED` chip | Pruned from local disk but kept marked as downloaded. |
| **`Missing`** | Warning square with exclamation `[!]` | Warning `MISSING` chip | Path unreachable on disk while folder is mounted. |
| **`Unverified`** | Solid neutral square `[■]` | Default row style | Awaiting local disk check. |

---

## 8. Smart Views and Timeline History

In `bcvault-fx`:
- **Smart Views**:
  - **Archived**: Filters catalog to releases with `local_status = 'archived'`.
  - **Missing files**: Filters catalog to releases with `local_status = 'missing'`.
- **Drawer Timeline**: Opening a release's detail drawer displays its complete `LIBRARY HISTORY` timeline (`ADOPTED`, `RELINKED`, `ARCHIVED`, `RESTORED`, `UNMARKED`) with exact timestamps and batch associations.

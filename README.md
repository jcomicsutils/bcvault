# bcvault

<div align="center">

[![GitHub Release](https://img.shields.io/github/v/release/jcomicsutils/bcvault?style=flat-square)](https://github.com/jcomicsutils/bcvault/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/jcomicsutils/bcvault/ci.yml?style=flat-square&label=CI)](https://github.com/jcomicsutils/bcvault/actions/workflows/ci.yml)
[![Rust 2024](https://img.shields.io/badge/rust-2024%20(MSRV%201.89)-orange.svg?style=flat-square)](https://www.rust-lang.org)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg?style=flat-square)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20Windows-lightgrey.svg?style=flat-square)](https://github.com/jcomicsutils/bcvault/releases)

**A high-performance Bandcamp vault archiver and discography manager for free and Name-Your-Price releases.**  

</div>

> [!WARNING]
> This is a personal project developed entirely with LLMs. Run at your own risk and do not trust that it does what it says it does.

---

## Documentation Index

| Document | Description |
| :--- | :--- |
| **[User Guide](docs/user-guide.md)** | End-to-end manual for `bcvault-fx` (Eye-Candy Edition), `bcvault` (Classic Edition), and the headless CLI, including sync modes, tabbed settings & ZIP options, filename patterns, quota rules, and all settings. |
| **[Library Sync & Folder Links](docs/library.md)** | Deep dive into N:1 artist folder linking, candidate scanning, exact-first and fuzzy matching, folder style templates, review rejection, URL/slug release matching, and disk verification safeguards. |
| **[System Architecture](docs/architecture.md)** | Crate boundaries, UI-thread purity invariants, worker supervisor & message protocol, `BandcampGate` rate-limiting, sidecar services, and developer workflow. |
| **[Database Schema Reference](docs/database.md)** | Complete SQLite schema specification for `bcvault.db` (v8) and `fx.sqlite` (v1), partial indexes, migration history, and batch undo audit log. |
| **[Bandcamp Endpoints & Scraping](docs/endpoints.md)** | Technical reference for `/music` grid scraping, `script[data-tralbum]` parsing, classification decision tree, `buy_for_nothing` checkout, `/statdownload/` polling, redeem validation, and artwork CDN tiers. |
| **[Custom Themes (`bcvault-fx`)](docs/themes.md)** | File-based TOML theme format reference, `<data_dir>/themes` location, live theme editor, sharing/importing, and the 13 built-in themes. |
| **[Artist Groups](docs/groups.md)** | Manual and rule-based collapsible sidebar artist groups, placement precedence, rule conditions/presets, drag-and-drop, and multi-artist release filtering. |

---

## Releases

Pre-compiled release archives are available on the [GitHub Releases](https://github.com/jcomicsutils/bcvault/releases) page for every tagged release:

- **Linux (x86_64)**: `bcvault-linux-x86_64.tar.gz` (contains both `bcvault` and `bcvault-fx`)
- **Windows (x86_64)**: `bcvault-windows-x86_64.zip` (contains both `bcvault.exe` and `bcvault-fx.exe`)

---

## Key Features

### Discography Tracking and Classification
- **Universal Resolver**: Ingests artist slugs (`artist-slug`), full Bandcamp URLs (`https://artist-slug.bandcamp.com`), and custom artist domains (`https://custom-domain.example.com`).
- **Comprehensive Scraper**: Parses both initial `/music` release grids and dynamic `data-client-items` overflow batches, with automatic fallback to `band_details`.
- **Release Classification**:
  - `FREE`: Direct download available without payment or email requirement.
  - `NYP` (Name-Your-Price): Free download eligible via zero-dollar checkout.
  - `EMAIL`: Free download gated behind an email submission.
  - `PAID`: Fixed-price commercial releases (flagged, never scraped or bypassed).
  - `UNAVAILABLE`: Releases with no Bandcamp download available (e.g., 0-track showcase/external-link pages or releases manually marked unavailable by the user; excluded from eligible download counts while still supporting local folder matching).
  - `UNKNOWN`: Unresolved release awaiting metadata inspection.

### Smart Incremental Synchronization
- **Incremental Sync**: Only queries newly discovered releases or previously interrupted catalog items.
- **Check Downloaded**: Re-inspects existing downloaded releases to detect newly appended bonus tracks (`+TRACKS`) or remote modifications (`MOD`).
- **Full Refresh**: Force-updates the complete discography metadata while preserving local download history and custom tags.

### Polite Scraping and Traffic Guard
- **`BandcampGate` Pacing**: Centralized rate limiter with configurable delays, random jitter, and per-artist concurrency limits.
- **Cooldown and Retry Enforcement**: Respects HTTP `429 Too Many Requests`, `503 Service Unavailable`, and `Retry-After` headers with exponential backoff.
- **Rolling Quota Protection**: Tracks rolling 30-day downloads per artist locally (default: `150` soft warning / `200` hard cap) and halts queues if Bandcamp reports free monthly download exhaustion.
- **Ethical Email Gate Handling**: Never attempts to spoof or bypass email verification or payment barriers. Provides direct browser redirection and a one-click **Redeem URL** intake dialog.

### Artist Folder Links and Library Sync
- **Multi-Folder Artist Binding (N:1)**: Link one or more local music folders (across drives or directories) to any Bandcamp artist page with non-destructive candidate scanning, format guessing, and declarative folder style templates (see [`docs/library.md`](docs/library.md)).
- **Persistent Review Rejection & URL/Slug Matching**: Dismiss false fuzzy/containment review proposals permanently with **Wrong, don't ask again**, resolve conflicts interactively, and attach local folders to exact releases by searching or pasting a Bandcamp URL or slug (`album/split-3`).
- **Disk Verification & Safeguards**: Probes volume roots and storage devices to ensure unmounted drives never trigger accidental deletions. Mass-missing guard halts automated operations if >30% of albums disappear in a single pass.
- **Archive Workflow & Auto-Relink**: Silently relinks albums moved within or across an artist's linked folders. Supports keeping deleted albums marked as downloaded (`archived`) or unmarking them.
- **Full Non-Destructive Undo**: Every library action creates a batch transaction with before/after audit entries in `library_events` and instant undo support.

### Design
- **Adaptive Dynamic Themes**: OKLab contrast-safe color generation across 13 built-in themes:
  - **Carbon**: Dark neutral slate and high-tech cyan/mint accents.
  - **Paper**: High-contrast editorial ink on warm newsprint.
  - **Terminal**: Monochromatic amber and green phosphor CRT glow.
  - **Vapor**: Twilight lavender with neon synthwave contrast.
  - **Midnight**: Deep navy with a clear blue accent.
  - **Ember**: Warm charcoal with a molten orange accent.
  - **Catppuccin Mocha**: Soothing pastel dark theme with mauve accent.
  - **Catppuccin Macchiato**: Soothing pastel dark theme with mauve accent.
  - **Catppuccin Frappe**: Soothing pastel medium-dark theme with mauve accent.
  - **Catppuccin Latte**: Soothing pastel light theme with mauve accent.
  - **Contrast**: Maximum-contrast black and white with a yellow accent.
  - **Daylight**: Cool light theme with a blue accent.
  - **Sage**: Soft green-grey light theme with a forest accent.

---

## Workspace Crates

| Crate | Path | Description |
| :--- | :--- | :--- |
| **`bcvault-core`** | [`crates/bcvault-core`](crates/bcvault-core) | Headless scraping engine, `BandcampGate` rate-limiting, release classification, audio format negotiation, stream extraction, local library scanner/matcher, and SQLite WAL database repository. |
| **`bcvault-fx`** | [`crates/bcvault-fx`](crates/bcvault-fx) | **Eye-Candy Edition GUI**: Virtualized Cover Wall, spring physics, dynamic OKLab theming, slide-over Detail Drawer, fuzzy Command Palette (`Ctrl+K`), Treemap and Calendar stats visualizations, and sidecar database. |
| **`bcvault`** | [`crates/bcvault`](crates/bcvault) | **Classic Desktop GUI**: High-density spreadsheet table interface with virtualized rendering for 100,000+ release collections. |

---

## Installation and Building

### Prerequisites

A modern Rust toolchain is required (MSRV **1.89+** / Edition **2024**).

#### Arch Linux
```bash
sudo pacman -Syu --needed base-devel git rust mesa \
  libx11 libxcursor libxrandr libxi libxinerama wayland libxkbcommon
```

#### Ubuntu / Debian
```bash
sudo apt-get update
sudo apt-get install -y build-essential git libx11-dev libxcursor-dev \
  libxrandr-dev libxi-dev libxinerama-dev libgl1-mesa-dev libwayland-dev libxkbcommon-dev
```

#### Fedora
```bash
sudo dnf install -y gcc gcc-c++ git mesa-libGL-devel \
  libX11-devel libXcursor-devel libXrandr-devel libXi-devel libXinerama-devel wayland-devel libxkbcommon-devel
```

#### Windows
Install Rust using [rustup.rs](https://rustup.rs/). The default MSVC toolchain (`x86_64-pc-windows-msvc`) and C++ build tools are supported out-of-the-box.

---

### Building from Source

```bash
git clone https://github.com/jcomicsutils/bcvault.git
cd bcvault
cargo build --workspace --release
```

Compiled binaries will be located in `target/release/`:
- `target/release/bcvault-fx` (or `bcvault-fx.exe` on Windows)
- `target/release/bcvault` (or `bcvault.exe` on Windows)

---

## Usage

### 1. Eye-Candy Edition (`bcvault-fx`)

```bash
cargo run --release -p bcvault-fx
```
Includes the virtualized Cover Wall, FLIP layout animations, interactive OKLab color filtering, artist banners, and library statistics.

### 2. Classic Table GUI (`bcvault`)

```bash
cargo run --release -p bcvault
```
High-density, low-latency table view optimized for rapidly triaging and batch-downloading thousands of items.

### 3. Headless CLI (`bcvault`)

Run operations from the terminal without graphical dependencies:

```bash
# Add and sync an artist discography
cargo run --release -p bcvault --example headless -- add <artist_slug_or_url>

# Batch-download eligible releases (formats: flac, mp3-320, mp3-v0, aac-hi, vorbis, alac, wav, aiff-lossless)
cargo run --release -p bcvault --example headless -- download <band_id> flac ./my_music
```

### 4. High-Load Benchmark

Benchmark the virtualized wall renderer under a synthetic 100,000-release library:

```bash
cargo run --release -p bcvault-fx --example fx_bench -- --check
```

---

## Keyboard Shortcuts

### Global Navigation (`bcvault-fx`)

| Shortcut | Action |
| :--- | :--- |
| <kbd>Ctrl</kbd> + <kbd>K</kbd> / <kbd>Ctrl</kbd> + <kbd>P</kbd> | Open **Command Palette** (`@` artists, `#` filters, `>` commands) |
| <kbd>/</kbd> or <kbd>Ctrl</kbd> + <kbd>F</kbd> | Focus toolbar release search input |
| <kbd>Ctrl</kbd> + <kbd>B</kbd> | Toggle left sidebar visibility |
| <kbd>Ctrl</kbd> + <kbd>A</kbd> | Select all releases in current view |
| <kbd>1</kbd> / <kbd>2</kbd> / <kbd>3</kbd> / <kbd>4</kbd> | Switch primary view (**Wall**, **Table**, **Artists**, **Stats**) |
| <kbd>Alt</kbd> + <kbd>Left</kbd> | Navigate view history backward |
| <kbd>Alt</kbd> + <kbd>Right</kbd> | Navigate view history forward |
| <kbd>F3</kbd> | Toggle live performance diagnostics overlay |
| <kbd>Esc</kbd> | Close active drawer, palette, modal, or clear selection |

### Cover Wall Navigation

| Shortcut | Action |
| :--- | :--- |
| <kbd>Arrow Keys</kbd> | Move card focus across the cover grid |
| <kbd>[</kbd> / <kbd>]</kbd> | Decrease / increase cover card size (`S`, `M`, `L`, `XL`) |
| <kbd>Space</kbd> | Toggle selection checkbox on focused release |
| <kbd>Enter</kbd> | Slide open the **Detail Drawer** for focused release |
| <kbd>Ctrl</kbd> + <kbd>Enter</kbd> | Immediately enqueue selected releases for download |

---

## Architecture and Storage

### Storage Layout

`bcvault` follows standard OS platform directory conventions via the `directories` crate:

| Platform | Database and Logs (`data_dir`) | Thumbnails and Cache (`cache_dir`) |
| :--- | :--- | :--- |
| **Linux** | `~/.local/share/bcvault/` | `~/.cache/bcvault/covers/` |
| **Windows** | `%APPDATA%\bcvault\bcvault\data\` | `%LOCALAPPDATA%\bcvault\bcvault\cache\covers\` |

The storage location can be overridden at runtime by setting the `BCVAULT_DATA_DIR=/custom/path` environment variable or launching with `--data-dir=/custom/path`.

### Database Files

- **`bcvault.db`**: Primary transactional SQLite store operating in WAL mode (Schema v8). Contains normalized artist records, release catalogs, download history, quota ledgers, linked artist folders, rejected review pairs, and library undo batches.
- **`fx.sqlite`**: Attached sidecar database (Schema v1) tailored for fast, zero-allocation UI reads, caching OKLab dominant hues, 8×8 RGB blur hashes (`tiny8`), cover dimensions, and lazy release details.

---

## License

Distributed under the **GNU General Public License v3.0 or later** (`GPL-3.0-or-later`).  
See [LICENSE](LICENSE) for full details.

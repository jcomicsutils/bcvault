# bcvault

A desktop application for tracking and downloading free and Name-Your-Price (NYP) Bandcamp discographies, built with Rust 2024, `eframe`/`egui` (zero-radius dark theme), `tokio`, `reqwest`, and `rusqlite` (WAL mode).

## Features

- **Discography Tracking & Classification**: Resolves artist slugs, Bandcamp URLs, and custom domains; parses both initial `/music` grids and `data-client-items` overflow lists; classifies every release as `FREE`, `NYP`, `EMAIL`, `PAID`, or `UNKNOWN`.
- **Smart Incremental Sync**:
  - *Incremental*: Fetches only newly discovered or previously interrupted (`unknown`) releases.
  - *Check downloaded*: Re-inspects downloaded releases to flag bonus tracks (`+TRACKS`) or remote modifications (`MOD`).
  - *Full refresh*: Re-fetches metadata for the entire discography.
- **Polite Traffic Gate (`BandcampGate`)**: All HTTP traffic passes through a single rate-limiting gate with configurable request delays, random jitter, per-artist concurrency limits, `Retry-After` handling, and exponential 429/503 cooldowns.
- **Shared Quota Guard**: Tracks rolling 30-day downloads per artist locally (`150` soft warning / `200` hard cap default) and immediately halts an artist's queue for 30 minutes if Bandcamp's API reports monthly free-download exhaustion.
- **Email-Gated Redeem Flow**: Never attempts to bypass email gates or paid releases. Provides one-click browser launch and a Redeem URL modal for `https://*.bandcamp.com/download` links.
- **Zero-Radius UI**: Strict square-cornered (`CornerRadius::ZERO`) dark interface with virtualized scrolling for 100k+ release rows and painter-drawn glyphs.

## Workspace Crates

- `crates/bcvault-core`: Shared sync, classification, download, `BandcampGate`, and SQLite WAL engine.
- `crates/bcvault`: Classic zero-radius table UI.
- `crates/bcvault-fx`: Eye-Candy Edition (`bcvault-fx`) — virtualized Cover Wall, FLIP animations, spring physics (`1/240s` fixed sub-steps), OKLab palette extraction & adaptive themes (`Carbon`, `Paper`, `Terminal`, `Vapor`), lazy slide-over Detail Drawer, fuzzy Command Palette (`Ctrl+K`), Artists banners, and Stats visualizations (squarified treemap, 53×7 heat-calendar, rolling odometer counters), backed by a read-only attached sidecar database (`fx.sqlite`).

## Usage

```bash
# Run the classic desktop GUI
cargo run --release -p bcvault

# Run the bcvault-fx Eye-Candy Edition GUI
cargo run --release -p bcvault-fx

# Run the headless CLI smoke harness
cargo run -p bcvault-core --example headless -- add <artist_slug_or_url>
cargo run -p bcvault-core --example headless -- download <band_id> [flac|mp3-320|...] [out_dir]

# Run the 100k-row headless wall scroll benchmark
cargo run --release -p bcvault-fx --example fx_bench -- --check

# Run formatting, clippy, unit/integration tests, and zero-radius UI checks
./scripts/check.sh
```

## Keyboard Shortcuts (`bcvault-fx`)

- `Ctrl+K` / `Ctrl+P`: Open Command Palette (`@` artists, `#` filters, `>` commands)
- `/` or `Ctrl+F`: Focus toolbar search box
- `1` / `2` / `3` / `4`: Switch view (`Wall`, `Table`, `Artists`, `Stats`)
- `[` / `]`: Decrease / increase Cover Wall card size (`S`, `M`, `L`, `XL`)
- `Ctrl+B`: Collapse / expand left sidebar
- `Alt+Left` / `Alt+Right`: Navigate view history back / forward
- `Arrow Keys`: Move focus on the Cover Wall (or step prev/next in Detail Drawer)
- `Space`: Toggle selection on focused release
- `Enter`: Open/close Detail Drawer (`Ctrl+Enter`: enqueue selected for download)
- `Esc`: Close Command Palette, Detail Drawer, or clear selection
- `F3`: Toggle diagnostics overlay (frame time, active springs, texture LRU RAM, event drain)

## License

GPL-3.0-or-later

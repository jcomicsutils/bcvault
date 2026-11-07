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

## Usage

```bash
# Run the desktop GUI
cargo run --release

# Run the headless CLI smoke harness
cargo run --example headless -- add <artist_slug_or_url>
cargo run --example headless -- download <band_id> [flac|mp3-320|...] [out_dir]

# Run formatting, clippy, unit/integration tests, and zero-radius UI checks
./scripts/check.sh
```

## Keyboard Shortcuts

- `Ctrl+F`: Focus the release table search box
- `Space`: Toggle selection on the last clicked eligible row
- `Enter`: Enqueue selected releases for download
- `Esc`: Close any open modal window
- `F3`: Toggle the frame-time and event-backlog debug overlay

## License

GPL-3.0-or-later

# Bandcamp CDN & Page Endpoints (`bcvault-fx`)

## 1. Artwork CDN (`https://f4.bcbits.com/img/a{art_id:010}_{N}.jpg`)

Confirmed from live Bandcamp release pages (e.g. `a0236863520_16.jpg`):

| Variant `_{N}` | Dimensions | Format | Usage in `bcvault-fx` |
| --- | --- | --- | --- |
| `_0` (`.jpg` / `.png`) | Original uploaded resolution (unscaled) | Original (`JPEG`/`PNG`) | Full-resolution archival / original source |
| `_10.jpg` | 1200 × 1200 px | JPEG | High-res square source; base cover fetch used to derive local `T384` (384 px), `T192` (192 px), and `T64` (64 px) tiers in a single CDN request |
| `_16.jpg` | 700 × 700 px (square) | JPEG | Primary `Hero` tier (700 px) |
| `_2.jpg` | 350 × 350 px (square) | JPEG | Medium square variant on Bandcamp discography grids |
| `_7.jpg` | 150 × 150 px (square) | JPEG | Small square variant; not fetched directly (`T64` and `T192` are derived locally from the single base fetch) |

### Politeness & Caching Contract
- Every cover fetch passes through `BandcampGate` (`GateClass::Cdn`).
- Exactly **one** network request is made per `art_id` for standard tiers (`T64`, `T192`, `T384`), saved under `{cache_dir}/covers/{art_id % 256:02x}/{art_id}.{tier}.jpg`.
- Failures are recorded in `fx.sqlite` (`cover_meta.failed_at`, `fail_count`) and negatively cached for 24 hours.

## 2. Lazy Release Details (`script[data-tralbum]`)
- Triggered only when the detail drawer remains open on a single release for at least **400 ms**.
- Uses `BandcampClient::fetch_release_raw` through `GateClass::Page`.
- Extracts `trackinfo` (track number, title, duration in seconds, optional `file["mp3-128"]` stream preview URL), `current.about` / `.tralbum-about`, `current.credits` / `.tralbum-credits`, and `.tag` elements, persisting them to `fx.sqlite` (`release_detail`) so a release is never fetched twice.

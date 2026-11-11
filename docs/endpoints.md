# Bandcamp Protocol, Scraping, and Endpoints Reference

All network communication with Bandcamp and its artwork CDN (`bcbits.com`) is isolated in `crates/bcvault-core/src/bandcamp/` and rate-limited through `BandcampGate`.

---

## 1. Artist Input Normalization (`bandcamp::input`)

`normalize_artist_input` accepts three input forms and normalizes them into a canonical `(slug, base_url)` pair:

1. **Bare Artist Slug**:
   - Input: `sigur-ros`
   - Normalized: `slug = "sigur-ros"`, `base_url = "https://sigur-ros.bandcamp.com"`
2. **Bandcamp Subdomain URL**:
   - Input: `https://sigur-ros.bandcamp.com/music` or `sigur-ros.bandcamp.com/album/foo`
   - Normalized: `slug = "sigur-ros"`, `base_url = "https://sigur-ros.bandcamp.com"`
3. **Custom Domain URL**:
   - Input: `https://music.example.org/music`
   - Normalized: `slug = "music.example.org"`, `base_url = "https://music.example.org"`

### Security Validation
- Only `http://` and `https://` schemes are permitted (bare domains default to `https://`).
- Localhost, loopback (`127.0.0.0/8`, `::1`), link-local, and private RFC 1918 IP literals are rejected unless `allow_local_mock = true` is explicitly set in offline `wiremock` integration tests.

---

## 2. Discography Discovery (`bandcamp::discography`)

### 2.1 Primary HTML Grid (`GET {base_url}/music`)
- **Gate Class**: `GateClass::Page`
- **Parsed Elements**:
  1. **Band ID Resolution** (in priority order):
     - `<script data-band="{...}">` JSON (`id` field)
     - `<meta property="og:site_name">` / `<meta name="bandcamp:band_id">` / `#band-name-location` attributes
     - `data-band-follow-info` JSON (`band_id` field)
     - Page inline JSON blobs (`band_id` / `selling_band_id`)
  2. **Initial Release Grid (`#music-grid li.music-grid-item` or `.music-grid-item`)**:
     - Extracts `data-item-id` (`album-<id>` or `track-<id>`), relative or absolute `<a href>`, release title (`.title`), per-release artist override (`.artist-override`), and cover artwork URL (`<img>` `src` or `data-original`).
  3. **Overflow Catalog (`ol#music-grid[data-client-items]`)**:
     - Bandcamp pages with large discographies embed the remaining items as a JSON array in the `data-client-items` attribute on `#music-grid`.
     - Each JSON entry provides `id`, `type` (`"album"` or `"track"`), `title`, `band_name`, `page_url`, and `art_id`.
  4. **Single-Release Redirect Fallback**:
     - If `/music` redirects directly to a single `/album/...` or `/track/...` page containing `script[data-tralbum]`, `parse_discography_html` extracts the band metadata and single release item directly from `TralbumData`.

### 2.2 Mobile API Fallback (`POST /api/mobile/24/band_details`)
- **Gate Class**: `GateClass::Api`
- **Request Payload**: `{"band_id": <i64>}`
- **When Used**: Triggered automatically when `parse_discography_html` resolves a `band_id` but finds an empty HTML release grid (common on certain custom-layout pages).
- **Response**: Returns `discography` array with `item_type`, `item_id`, `title`, `band_name`, `art_id`, and `tralbum_url`.

---

## 3. Release Page Inspection & Classification (`bandcamp::tralbum`, `bandcamp::classify`)

### 3.1 `script[data-tralbum]` Extraction (`GET {release_url}`)
- **Gate Class**: `GateClass::Page`
- Every Bandcamp album or track page embeds a `<script data-tralbum="{...}">` attribute containing `TralbumData` JSON:
  - `id` and `item_type` (`"album"` or `"track"`)
  - `current`:
    - `title`, `band_id`, `selling_band_id`
    - `minimum_price` (`f64`), `minimum_price_nonzero` (`f64`), `is_set_price` (`bool`)
    - `require_email` (`Option<i64>` / `bool`), `is_purchasable` (`bool`), `is_preorder` (`bool`)
    - `publish_date`, `mod_date` (RFC 2822 timestamps)
    - `about`, `credits`
  - `freeDownloadPage` (`Option<String>`)
  - `album_is_preorder` (`Option<bool>`)
  - `art_id` (`Option<i64>`)
  - `trackinfo` (`Vec<TrackInfo>`): track number, title, duration (`f64`), and `file["mp3-128"]` preview stream URL.

### 3.2 Classification Decision Tree (`classify::classify_release`)

`bcvault` classifies each inspected release deterministically using `PricingSignals` and `track_count`:

```text
1. Is track_count == Some(0)?
   └── YES ──► Classification::Unavailable
               (Bandcamp never renders a download button when a release has 0 tracks,
                even if minimum_price == 0.0 or freeDownloadPage is present.)

2. Is freeDownloadPage present (non-empty URL)?
   └── YES ──► Classification::Free

3. Is is_purchasable == true AND minimum_price == 0.0 AND is_set_price == false?
   ├── require_email == true  ──► Classification::EmailRequired
   └── require_email == false ──► Classification::Nyp

4. Is is_purchasable == true AND (minimum_price > 0.0 OR minimum_price_nonzero > 0.0)?
   └── YES ──► Classification::Paid

5. Is is_purchasable == false AND freeDownloadPage is None?
   ├── Has positive price signal ──► Classification::Paid
   └── Otherwise                 ──► Classification::Unknown
```

> [!NOTE]
> Users can also manually mark or unmark any release as `Classification::Unavailable` via `UiToWorkerCommand::SetReleaseUnavailable`. When unmarking, `bcvault` re-evaluates the stored `pricing_json` (and `track_count`) offline to restore the original classification without making a network request.

---

## 4. Zero-Dollar Checkout & Download Pipeline (`bandcamp::download_api`)

### 4.1 Direct Free Download (`Classification::Free`)
If the release page exposes a `freeDownloadPage` URL, `DownloadManager` fetches that page (`GateClass::Page`), extracts the `#download-items` / `data-blob` format map, and selects the target `AudioFormat` without touching the artist's monthly NYP quota (`counts_against_quota = 0`).

### 4.2 Zero-Dollar NYP Checkout (`POST /api/mobile/22/buy_for_nothing`)
- **Gate Class**: `GateClass::Api`
- **Request Payload**:
  ```json
  {
    "item_type": "album",
    "item_id": "1234567890"
  }
  ```
- **Quota Accounting**: Counts against the artist's rolling 30-day download quota (`counts_against_quota = 1`).
- **Response Handling**:
  - `ok: true` with `downloads` (or `item.downloads`) map of `{ "<format_key>": { "url": "https://..." } }`.
  - `ok: false`: Mapped by `bandcamp::errors::map_api_rejection` to:
    - `BcError::EmailRequired` (if Bandcamp now requires an email address; the release's DB classification is automatically updated to `email_required`).
    - `BcError::QuotaExceeded` (if the artist's monthly free download pool is exhausted; halts remaining queued jobs for that artist).
    - `BcError::ApiRejected(reason)` for other server errors.

### 4.3 `/statdownload/` Handshake Resolution
Bandcamp download URLs (`https://popplers5.bandcamp.com/download/album?...`) occasionally return an HTML/JS wrapper or JSON status payload while the archive is being prepared on a poppler server:
1. If the initial `GET` response (`GateClass::Download`) has a `text/*` or `application/json` `Content-Type` instead of binary audio/zip data, `resolve_download_url` inspects the body for `download_url`.
2. If not immediately ready, it rewrites `/download/` to `/statdownload/` with `&.vrs=1` and polls up to 10 times (2-second intervals via `GateClass::Api`), following any `retry_url` tokens until `result == "ok"` yields the final binary `download_url`.

### 4.4 Audio Format Negotiation (`choose_format`)
Bandcamp format keys (`flac`, `mp3-320`, `mp3-v0`, `wav`, `aiff-lossless`, `alac`, `vorbis`, `aac-hi`) are matched against the user's preferred `AudioFormat`:
- **`FormatFallback::Strict`**: Fails with `BcError::FormatUnavailable` if the exact format is absent.
- **`FormatFallback::Nearest`**:
  - Lossless preference order: `FLAC -> ALAC -> WAV -> AIFF -> MP3 320 -> MP3 V0 -> AAC -> Vorbis`
  - Lossy preference order: `MP3 320 -> MP3 V0 -> AAC -> Vorbis -> FLAC -> ALAC -> WAV -> AIFF`

---

## 5. Email Redeem Page Validation (`bandcamp::redeem`)

When a user submits a link received via Bandcamp email in the **Redeem URL** dialog (`validate_redeem_url`):
1. **Strict Host Allowlist**: The URL must use `https://` and match `bandcamp.com` or `*.bandcamp.com` (or `*.bcbits.com`).
2. **Path Validation**: Must point to `/download` or `/statdownload`.
3. **HTML Blob Parsing (`parse_redeem_html`)**: Extracts `#pagedata[data-blob]` JSON (`digital_items` array), resolving each item's `item_type`, `item_id`, `title`, `artist`, and per-format download URLs.
4. **Privacy**: Query parameters containing `sig`, `payment_id`, or `upn` are never logged.

---

## 6. Artwork CDN & Lazy Details (`bcvault-fx`)

### 6.1 Artwork CDN (`https://f4.bcbits.com/img/a{art_id:010}_{N}.jpg`)

| Variant `_{N}` | Dimensions | Format | Usage in `bcvault-fx` |
| :--- | :--- | :--- | :--- |
| `_0` (`.jpg` / `.png`) | Original uploaded resolution | `JPEG` / `PNG` | Archival original source |
| `_10.jpg` | `1200 × 1200 px` | `JPEG` | High-res square source; fetched once to derive local `T384` (384 px), `T192` (192 px), and `T64` (64 px) cache files |
| `_16.jpg` | `700 × 700 px` | `JPEG` | Primary `Hero` tier (700 px) for the Detail Drawer |
| `_2.jpg` | `350 × 350 px` | `JPEG` | Medium square variant on Bandcamp discography grids |
| `_7.jpg` | `150 × 150 px` | `JPEG` | Small square variant (used by Classic UI thumbnail worker) |

- **Politeness & Caching**:
  - Every cover request acquires a `GateClass::Cdn` permit (`250 ms` spacing).
  - Standard tiers (`T64`, `T192`, `T384`) are derived locally from a single `_10.jpg` download and stored under `{cache_dir}/covers/{art_id % 256:02x}/{art_id}.{tier}.jpg`.
  - Failed cover fetches are recorded in `fx.sqlite` (`cover_meta.failed_at`, `fail_count`) and negatively cached for 24 hours.

### 6.2 Lazy Release Details (`DetailService`)
- Triggered only when the `bcvault-fx` Detail Drawer stays open on a single release for at least **400 ms**.
- Calls `BandcampClient::fetch_release_raw` (`GateClass::Page`).
- Extracts `trackinfo` (track number, title, duration in seconds, optional `file["mp3-128"]` stream preview URL), `current.about` / `.tralbum-about`, `current.credits` / `.tralbum-credits`, and `.tag` elements, persisting them to `fx.sqlite` (`release_detail`) so a release is never fetched twice.

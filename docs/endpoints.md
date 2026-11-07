# Bandcamp HTML & API Endpoint Contract (`bcvault`)

This document records the HTML selectors, embedded JSON attributes, and mobile API payloads isolated inside `src/bandcamp/`.

## 1. Discography Discovery (`GET {base}/music`)

- **Single-release redirect**: When an artist has only one release, Bandcamp may redirect `/music` to `/album/{slug}` or `/track/{slug}`. `discography::parse_discography_html` inspects the final URL and falls back to `parse_release_html`.
- **`band_id` resolution order**:
  1. `li.music-grid-item[data-band-id]`
  2. `meta[name="bc-page-properties"]` (`content` JSON -> `page_id`)
  3. `script[data-band]` (`data-band` JSON -> `id`)
- **Grid items**: `li.music-grid-item`
  - `data-item-id`: `"album-{id}"` or `"track-{id}"`
  - `a[href]`: relative or absolute release URL
  - `p.title`: release title (nested `.artist-override` holds per-release artist on label pages)
  - `img[data-original]` or `img[src]`: cover image URL (ignoring `/img/0.gif` placeholders)
- **Overflow items**: `ol#music-grid[data-client-items]`
  - JSON array of objects: `{ id, type, title, page_url, art_id, artist, band_id }`.
  - Merged and deduplicated by `ItemKey(kind, id)`.

## 2. Mobile Discography Fallback (`POST /api/mobile/24/band_details`)

- **Request**: `{"band_id": 440011}`
- **Response**:
  ```json
  {
    "band_id": 440011,
    "name": "Artist Name",
    "discography": [
      {
        "item_id": 1001,
        "item_type": "album",
        "title": "Release Title",
        "band_name": "Artist Name",
        "art_id": 11223344
      }
    ]
  }
  ```

## 3. Release Metadata (`GET /album/{slug}` or `/track/{slug}`)

- **Selector**: `script[data-tralbum]`
- **JSON payload (`Tralbum`)**:
  - `id`: integer or string
  - `item_type`: `"album"` or `"track"`
  - `freeDownloadPage`: nullable URL string (non-empty indicates direct free download page)
  - `art_id`: integer
  - `artist`: release artist string
  - `trackinfo`: array of track objects
  - `current`:
    - `band_id`: integer
    - `minimum_price`: float or string (`0.0` for Free/NYP/EmailRequired, `> 0.0` for Paid)
    - `set_price`: float or string
    - `is_set_price`: boolean/integer/string flag (`0`/`1`/`true`/`false`/`null`)
    - `require_email`: boolean/integer/string flag
    - `publish_date` / `mod_date`: RFC 2822 timestamp (`"20 Aug 2021 00:00:00 GMT"`)
    - `is_preorder`: boolean/integer/string flag

## 4. Direct Free / NYP Download (`POST /api/mobile/22/buy_for_nothing`)

- **Request**: `{"item_type": "album", "item_id": "1001"}`
- **Success Response**:
  ```json
  {
    "ok": true,
    "downloads": {
      "flac": { "url": "https://popplers5.bandcamp.com/download/album?..." },
      "mp3-320": { "url": "https://popplers5.bandcamp.com/download/album?..." }
    }
  }
  ```
- **Error Response**:
  ```json
  {
    "ok": false,
    "error_message": "An email address is required to download this release."
  }
  ```
- **Statdownload Handshake**: If the download URL returns `text/html` or `application/json`, `resolve_download_url` replaces `/download/` with `/statdownload/`, appends `.vrs=1`, and reads `download_url` (or polls `retry_url`).

## 5. Email Redeem Page (`GET https://*.bandcamp.com/download?...`)

- **Selector**: `#pagedata[data-blob]`
- **JSON payload**:
  - `download_items`: array of `{ item_id, item_type, title, artist, art_id, downloads: { "<format>": { url, size_mb, description } } }`.

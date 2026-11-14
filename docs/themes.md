# Custom Themes (`bcvault-fx`)

`bcvault-fx` uses single-file TOML themes. Every built-in theme and every user-created theme uses the same format.

## Built-in Themes (13)

| ID | Name | Dark | Scanlines | Description |
| :--- | :--- | :--- | :--- | :--- |
| `carbon` | Carbon | Yes | No | Dark neutral slate theme (default fallback) |
| `paper` | Paper | No | No | High-contrast warm paper theme |
| `terminal` | Terminal | Yes | Yes | Phosphor green terminal theme |
| `vapor` | Vapor | Yes | No | Deep violet and neon cyan theme |
| `midnight` | Midnight | Yes | No | Deep navy with a clear blue accent |
| `ember` | Ember | Yes | No | Warm charcoal with a molten orange accent |
| `catppuccin-mocha` | Catppuccin Mocha | Yes | No | Soothing pastel dark theme with mauve accent |
| `catppuccin-macchiato` | Catppuccin Macchiato | Yes | No | Soothing pastel dark theme with mauve accent |
| `catppuccin-frappe` | Catppuccin Frappe | Yes | No | Soothing pastel medium-dark theme with mauve accent |
| `catppuccin-latte` | Catppuccin Latte | No | No | Soothing pastel light theme with mauve accent |
| `contrast` | Contrast | Yes | No | Maximum-contrast black and white with a yellow accent |
| `daylight` | Daylight | No | No | Cool light theme with a blue accent |
| `sage` | Sage | No | No | Soft green-grey light theme with a forest accent |

## Themes Folder Location

User themes live in `<data_dir>/themes/*.toml` (non-recursive; symlinks and files over 16 KiB are ignored; up to 100 user themes):

- **Linux**: `~/.local/share/bcvault/themes/`
- **macOS**: `~/Library/Application Support/org.bcvault.bcvault/themes/`
- **Windows**: `%APPDATA%\bcvault\bcvault\data\themes\`

Passing `--data-dir <PATH>` or setting `BCVAULT_DATA_DIR=<PATH>` moves the themes folder to `<PATH>/themes/`. You can also open the folder directly from **Settings → Appearance → Open themes folder**.

## Creating, Editing, and Sharing

1. **Create or Duplicate**: In **Settings → Appearance**, click **New theme** (duplicates the active theme) or **Edit** / **Duplicate** on any theme. Changes preview live across the entire window.
2. **Export / Copy**: Click **Export** in Settings or the Theme Editor to save a `.toml` file, or **Copy as text** to copy the TOML to your clipboard.
3. **Import**: Click **Import** in **Settings → Appearance** and select one or more `.toml` files (or drop `.toml` files onto the `bcvault-fx` window). Imports never overwrite existing themes; colliding IDs are suffixed with `-2`, `-3`, etc.
4. **Delete**: Open a user theme in the editor and click **Delete** (with confirmation). If the active theme is deleted, `bcvault-fx` falls back to `carbon`.

## File Format (Version 1)

```toml
# bcvault theme, format 1. Share this file as is.
format = 1
id = "carbon"
name = "Carbon"
description = "Dark neutral slate theme"
author = "bcvault"
dark = true
scanlines = false

[colors]
bg0 = "#0A0A0B"
bg1 = "#121315"
bg2 = "#1B1D20"
border = "#3A3D42"
border_hi = "#8A8F98"
text = "#ECECEC"
text_dim = "#9AA0A6"
accent = "#00D1B2"
accent2 = "#3DDC84"
ok = "#3DDC84"
warn = "#FFB300"
err = "#FF5252"
```

### Rules & Hints

- **Size & Encoding**: UTF-8 TOML, at most 16 KiB.
- **`format`**: Must be `1`.
- **`id`**: 1–32 characters from `a-z`, `0-9`, and `-` (no leading, trailing, or doubled hyphens).
- **`name`**: 1–40 characters (required). **`description`**: up to 160 characters (optional). **`author`**: up to 40 characters (optional).
- **`dark`**: Required boolean. **`scanlines`**: Optional boolean (defaults to `false`).
- **`[colors]`**: All 12 keys required as exact `#RRGGBB` hex strings (no alpha or 3-digit shorthand).
- **Contrast Hints** (non-blocking warnings shown in the editor): `text`/`text_dim` ≥ 4.5:1 on `bg0`/`bg1`/`bg2`, `accent` ≥ 4.5:1 on `bg1`, `accent2`/`ok`/`warn`/`err` ≥ 3.0:1 on `bg1`, and `dark` matching `bg0` relative luminance (< 0.4).

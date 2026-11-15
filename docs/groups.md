# Artist Groups

`bcvault` and `bcvault-fx` support collapsible sidebar artist groups backed by `bcvault.db` (schema v9: `artist_groups` and `artist_group_pins`). Groups can be populated manually, automatically via live rules, or with a combination of both.

---

## Placement Precedence

Every artist appears in at most one sidebar section, evaluated in strict order:

1. **Manual group pin** (`artist_group_pins` with a `group_id`) — always places the artist in that group, overriding any rules.
2. **Manual Ungrouped pin** (`artist_group_pins` with `group_id = NULL`) — keeps the artist in **Ungrouped**, even if a rule would otherwise match it.
3. **First matching rule** — groups are evaluated in their `sort_order` (then `id`); the first group whose `GroupRule` matches the artist claims it.
4. **Ungrouped** — fallback when no pin or rule matches.

Removing an artist automatically removes any manual pin for that artist (`ON DELETE CASCADE`).

---

## Rule Conditions & Presets

Each group can optionally carry a `GroupRule` with:
- **Match mode**: `All` (every condition must match) or `Any` (at least one condition must match).
- **Conditions** (each supports a `Not` / `negate` toggle):
  - **Name contains** / **Name starts with**: accent- and case-insensitive substring/prefix match against the artist's display name.
  - **URL contains**: case-insensitive substring match against the artist's Bandcamp URL.
  - **Download state**: `NoneDownloaded`, `HasRemaining` (eligible releases left to download), or `AllDownloaded`.
  - **Library state**: `Unlinked` (no linked folders), `Linked` (at least one linked folder), `HasIssues` (missing, unavailable, or pending review), or `Clean` (linked with no issues).
  - **At least N releases**: matches artists with `total >= min`.

The Group Editor also includes one-click presets:
- **Needs Attention** — artists with library issues (`LibraryState::HasIssues`).
- **Remaining Free** — artists with eligible releases not yet downloaded (`DownloadState::HasRemaining`).
- **Unlinked Artists** — artists without linked local folders (`LibraryState::Unlinked`).

---

## Using Groups in the UI

- **Create / Edit**: Use **+ Group** in the sidebar (or `New artist group` / `Edit artist group` in the `bcvault-fx` command palette). The Group Editor shows a live preview of which artists match the current rule and notes any manually pinned artists.
- **Move artists**: Right-click an artist row to **Move to group…**, **Move to Ungrouped**, or **Clear manual placement** (returning rule control). You can also drag-and-drop an artist row directly onto any group header or the **Ungrouped** header.
- **Group actions**: Right-click any group header (or **Ungrouped**) to:
  - **Sync group** (syncs all current members)
  - **Download remaining** (queues remaining eligible releases for all members, subject to quota warnings)
  - **View group releases** (filters the release list/wall to all members via `ReleaseQuery.band_ids`; in Classic you can also multi-select artists in a group)
  - **Edit group** / **Delete group** (deleting a group removes its pins and returns rule-matched artists to the remaining groups or Ungrouped)

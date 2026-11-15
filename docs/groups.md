# Artist Groups & Grouping Sets

`bcvault` and `bcvault-fx` support non-destructive sidebar **Grouping Sets** and **Built-in Automatic Groupings** backed by `bcvault.db` (schema v9: `artist_group_sets`, `artist_groups`, and `artist_group_pins`).

---

## Sidebar Grouping Switcher

Use the **Grouping** dropdown at the top of the sidebar to switch non-destructively at any time between:

- **No Grouping** — flat artist list with no group headers.
- **Built-in (Automatic)** — zero-setup dynamic groupings computed on the fly:
  - **First Letter (A–Z)** — collapsible `A`–`Z` and `#` buckets (accent-folded).
  - **Download Status** — `Remaining Free`, `All Downloaded`, `Nothing Downloaded`.
  - **Library Status** — `Needs Attention` (missing files or folder errors), `Linked & Clean`, `Unlinked`.
  - **Pricing Makeup** — `100% Free / NYP`, `Email-Gated`, `Mixed (Free + Paid)`, `Paid Only`, `Empty / Unsynced`.
  - **Discography Size** — `Large (20+ releases)`, `Medium (5–19 releases)`, `Small (1–4 releases)`, `Empty (0 releases)`.
  - **Sync Freshness** — `Never Synced`, `Recent (<= 7 days)`, `7–30 Days Ago`, `Stale (> 30 days)`.
  - **Archive Status** — `Fully Archived`, `Partially Archived`, `Active on Disk`, `Nothing Downloaded`.
- **Custom Group Sets** — user-created named sets (for example, a `"Workflow"` set with `"Finished"`, `"Planned"`, and `"Waiting for Free Downloads"`). Switching to a built-in mode or another Group Set never modifies or deletes your saved groups or artist placements.

---

## Multi-Artist Selection & Group Actions

In both `bcvault-fx` and `bcvault` (Classic):

- **Multi-select artists**: Use `Ctrl+Click` (or `Cmd+Click`) to toggle individual artists, and `Shift+Click` to select a range of visible artists.
- **Move artists**: Drag selected artist row(s) onto any group header or **Ungrouped**, right-click to use **Move to group**, or use the searchable artist checklist inside **+Grp** / **Edit group & members...**.
- **Group actions**: Right-click any built-in or custom group header to **Sync group**, **Download remaining for group**, **Select artists in group**, or **View releases of group** (filters the release list/wall to all members via `ReleaseQuery.band_ids`). Custom group headers also support **Edit group & members...**, **Move up** / **Move down**, and **Delete group...**.

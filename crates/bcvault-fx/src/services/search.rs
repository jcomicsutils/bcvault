use crate::{
    actions::{FxAction, ViewMode},
    motion::EffectsLevel,
    services::messages::{SearchHit, SearchHitKind, SearchScope},
    theme::ThemeId,
};
use bcvault_core::types::{ItemKey, ItemType, ReleaseFilter, SyncMode};
use rusqlite::Connection;

/// Fast subsequence fuzzy matcher returning `(score, matched_char_indices)`.
pub fn fuzzy_match(pattern: &str, candidate: &str) -> Option<(i64, Vec<usize>)> {
    let pat_chars: Vec<char> = pattern
        .trim()
        .chars()
        .flat_map(|c| c.to_lowercase())
        .collect();
    if pat_chars.is_empty() {
        return Some((0, Vec::new()));
    }

    let cand_chars: Vec<char> = candidate.chars().collect();
    let cand_lower: Vec<char> = cand_chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();

    let mut indices = Vec::with_capacity(pat_chars.len());
    let mut score = 0_i64;
    let mut p_idx = 0;
    let mut prev_match: Option<usize> = None;

    for (c_idx, &ch) in cand_lower.iter().enumerate() {
        if p_idx < pat_chars.len() && ch == pat_chars[p_idx] {
            indices.push(c_idx);
            score += 10;
            if c_idx == 0 {
                score += 25;
            } else if matches!(
                cand_chars[c_idx - 1],
                ' ' | '-' | '_' | '/' | '.' | ':' | '('
            ) {
                score += 18;
            }
            if let Some(prev) = prev_match {
                if prev + 1 == c_idx {
                    score += 15;
                } else {
                    score -= (c_idx - prev) as i64;
                }
            }
            prev_match = Some(c_idx);
            p_idx += 1;
        }
    }

    if p_idx == pat_chars.len() {
        score -= (cand_chars.len().saturating_sub(pat_chars.len())) as i64;
        Some((score, indices))
    } else {
        None
    }
}

pub fn run_palette_search(
    conn: &Connection,
    raw_text: &str,
    default_scope: SearchScope,
) -> Vec<SearchHit> {
    let trimmed = raw_text.trim();
    let (scope, query_text) = if let Some(rest) = trimmed.strip_prefix('@') {
        (SearchScope::Artists, rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix('#') {
        (SearchScope::Filters, rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix('>') {
        (SearchScope::Commands, rest.trim())
    } else {
        (default_scope, trimmed)
    };

    let mut hits = Vec::new();

    if matches!(scope, SearchScope::All | SearchScope::Commands) {
        let commands: &[(&str, &str, FxAction, &'static str)] = &[
            (
                "View: Cover Wall",
                "Switch to cover wall grid",
                FxAction::SetView(ViewMode::Wall),
                "1",
            ),
            (
                "View: Release Table",
                "Switch to compact table view",
                FxAction::SetView(ViewMode::Table),
                "2",
            ),
            (
                "View: Artists",
                "Switch to artist banner gallery",
                FxAction::SetView(ViewMode::Artists),
                "3",
            ),
            (
                "View: Stats",
                "Switch to library statistics",
                FxAction::SetView(ViewMode::Stats),
                "4",
            ),
            (
                "Sync All Artists (Incremental)",
                "Check all artists for new releases",
                FxAction::SyncAll(SyncMode::Incremental),
                "",
            ),
            (
                "Sync All Artists (Full Refresh)",
                "Re-fetch metadata for all artists",
                FxAction::SyncAll(SyncMode::FullRefresh),
                "",
            ),
            (
                "Download Selected Releases",
                "Enqueue currently selected releases",
                FxAction::DownloadSelected,
                "D",
            ),
            (
                "Toggle Sidebar",
                "Collapse or expand the left sidebar",
                FxAction::ToggleSidebar,
                "Ctrl+B",
            ),
            (
                "Theme: Carbon",
                "Dark neutral slate theme",
                FxAction::SetTheme(ThemeId::Carbon),
                "",
            ),
            (
                "Theme: Paper",
                "High-contrast warm paper theme",
                FxAction::SetTheme(ThemeId::Paper),
                "",
            ),
            (
                "Theme: Terminal",
                "Phosphor green terminal theme",
                FxAction::SetTheme(ThemeId::Terminal),
                "",
            ),
            (
                "Theme: Vapor",
                "Deep violet and neon cyan theme",
                FxAction::SetTheme(ThemeId::Vapor),
                "",
            ),
            (
                "Effects: Full",
                "Enable all animations and atmosphere",
                FxAction::SetEffects(EffectsLevel::Full),
                "",
            ),
            (
                "Effects: Low",
                "Enable springs without shader atmosphere",
                FxAction::SetEffects(EffectsLevel::Low),
                "",
            ),
            (
                "Effects: Off",
                "Instant transitions, zero animations",
                FxAction::SetEffects(EffectsLevel::Off),
                "",
            ),
            (
                "Purge Cover Cache",
                "Delete cached cover thumbnails from disk",
                FxAction::PurgeCovers,
                "",
            ),
            (
                "Toggle Debug Overlay",
                "Show frame time, springs, and VRAM stats",
                FxAction::ToggleDebug,
                "F3",
            ),
            (
                "Link Folder to Artist...",
                "Link a local music folder to a Bandcamp artist",
                FxAction::OpenLinkFolderModal(None),
                "",
            ),
            (
                "Verify Local Library Files",
                "Check that all adopted local files are present on disk",
                FxAction::VerifyFiles,
                "",
            ),
        ];

        for (title, sub, act, hint) in commands {
            if let Some((sc, idx)) = fuzzy_match(query_text, title) {
                hits.push(SearchHit {
                    title: (*title).to_string(),
                    subtitle: (*sub).to_string(),
                    match_indices: idx,
                    score: sc + 5,
                    kind: SearchHitKind::Command(act.clone(), hint),
                });
            }
        }
    }

    if matches!(scope, SearchScope::All | SearchScope::Filters) {
        for f in ReleaseFilter::ALL {
            let label = format!("Filter: {}", f.label());
            if let Some((sc, idx)) = fuzzy_match(query_text, &label) {
                hits.push(SearchHit {
                    title: label,
                    subtitle: "Filter releases by classification".to_string(),
                    match_indices: idx,
                    score: sc,
                    kind: SearchHitKind::Filter(f),
                });
            }
        }
    }

    if matches!(scope, SearchScope::All | SearchScope::Artists)
        && let Ok(mut stmt) = conn.prepare(
            "SELECT band_id, COALESCE(custom_name, name), slug FROM m.artists ORDER BY name COLLATE NOCASE ASC",
        )
        && let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
    {
        for row in rows.flatten() {
            let (band_id, name, slug) = row;
            if let Some((sc, idx)) = fuzzy_match(query_text, &name) {
                hits.push(SearchHit {
                    title: name,
                    subtitle: format!("Artist (@{slug})"),
                    match_indices: idx,
                    score: sc + 15,
                    kind: SearchHitKind::Artist { band_id },
                });
            }
        }
    }

    if matches!(scope, SearchScope::All) && !query_text.is_empty() {
        let like_pat = format!("%{}%", query_text.replace('%', ""));
        if let Ok(mut stmt) = conn.prepare(
            "SELECT item_type, item_id, title, artist_name, art_id
             FROM m.releases
             WHERE title LIKE ?1 OR artist_name LIKE ?1
             LIMIT 250",
        ) && let Ok(rows) = stmt.query_map([&like_pat], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        }) {
            for row in rows.flatten() {
                let (t_str, id, title, artist, art_id) = row;
                let kind = ItemType::parse(&t_str).unwrap_or(ItemType::Album);
                if let Some((sc, idx)) = fuzzy_match(query_text, &title) {
                    hits.push(SearchHit {
                        title,
                        subtitle: artist,
                        match_indices: idx,
                        score: sc,
                        kind: SearchHitKind::Release {
                            key: ItemKey { kind, id },
                            art_id,
                        },
                    });
                }
            }
        }
    }

    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
    hits.truncate(50);
    hits
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matcher_ranks_prefix_higher_and_tracks_indices() {
        let (s1, idx1) = fuzzy_match("mod", "Modular Horizons").unwrap();
        let (s2, _) = fuzzy_match("mod", "The Modern Echo").unwrap();
        assert!(s1 > s2);
        assert_eq!(idx1, vec![0, 1, 2]);
        assert!(fuzzy_match("xyz", "Modular Horizons").is_none());
    }
}

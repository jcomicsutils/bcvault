use std::collections::BTreeSet;

use crate::types::{ItemKey, ReleaseRow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectScope {
    All,
    Downloadable,
    Remaining,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionStats {
    pub selected: usize,
    pub selected_downloadable: usize,
    pub remaining_in_view: usize,
    pub total_in_view: usize,
}

pub const fn row_in_scope(row: &ReleaseRow, scope: SelectScope) -> bool {
    match scope {
        SelectScope::All => true,
        SelectScope::Downloadable => row.is_downloadable(),
        SelectScope::Remaining => row.is_download_candidate(),
    }
}

pub fn select<'a>(
    rows: impl IntoIterator<Item = &'a ReleaseRow>,
    scope: SelectScope,
) -> BTreeSet<ItemKey> {
    rows.into_iter()
        .filter(|row| row_in_scope(row, scope))
        .map(|row| row.key)
        .collect()
}

pub fn invert<'a>(
    rows: impl IntoIterator<Item = &'a ReleaseRow>,
    selected: &BTreeSet<ItemKey>,
    scope: SelectScope,
) -> BTreeSet<ItemKey> {
    rows.into_iter()
        .filter(|row| row_in_scope(row, scope) && !selected.contains(&row.key))
        .map(|row| row.key)
        .collect()
}

pub fn range_keys(
    rows: &[&ReleaseRow],
    a: ItemKey,
    b: ItemKey,
    scope: SelectScope,
) -> BTreeSet<ItemKey> {
    let Some(ia) = rows.iter().position(|r| r.key == a) else {
        return BTreeSet::new();
    };
    let Some(ib) = rows.iter().position(|r| r.key == b) else {
        return BTreeSet::new();
    };
    let (lo, hi) = if ia <= ib { (ia, ib) } else { (ib, ia) };
    rows[lo..=hi]
        .iter()
        .filter(|row| row_in_scope(row, scope))
        .map(|row| row.key)
        .collect()
}

pub fn compute_stats<'a>(
    rows: impl IntoIterator<Item = &'a ReleaseRow>,
    selected: &BTreeSet<ItemKey>,
) -> SelectionStats {
    let mut stats = SelectionStats::default();
    for row in rows {
        stats.total_in_view += 1;
        if row.is_download_candidate() {
            stats.remaining_in_view += 1;
        }
        if selected.contains(&row.key) {
            stats.selected += 1;
            if row.is_downloadable() {
                stats.selected_downloadable += 1;
            }
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Classification, ItemType, LocalStatus};

    fn make_row(
        id: i64,
        classification: Classification,
        is_preorder: bool,
        downloaded: bool,
        local_status: Option<LocalStatus>,
    ) -> ReleaseRow {
        ReleaseRow {
            key: ItemKey {
                kind: ItemType::Album,
                id,
            },
            band_id: 1,
            title: format!("Album {id}"),
            artist_name: "Artist".into(),
            url: format!("https://artist.bandcamp.com/album/a-{id}"),
            cover_url: None,
            art_id: None,
            classification,
            is_preorder,
            track_count: Some(5),
            published_at: None,
            modified_at: None,
            downloaded,
            downloaded_at: None,
            downloaded_format: None,
            local_path: None,
            local_status,
            origin: None,
            missing_ack_at: None,
            has_remote_update: false,
            remote_update_reason: None,
        }
    }

    #[test]
    fn is_download_candidate_matrix() {
        let free = make_row(1, Classification::Free, false, false, None);
        let nyp = make_row(2, Classification::Nyp, false, false, None);
        let paid = make_row(3, Classification::Paid, false, false, None);
        let email = make_row(4, Classification::EmailRequired, false, false, None);
        let unknown = make_row(5, Classification::Unknown, false, false, None);
        let unavailable = make_row(6, Classification::Unavailable, false, false, None);
        let preorder = make_row(7, Classification::Free, true, false, None);
        let downloaded = make_row(8, Classification::Free, false, true, None);
        let archived = make_row(
            9,
            Classification::Free,
            false,
            true,
            Some(LocalStatus::Archived),
        );

        assert!(free.is_download_candidate());
        assert!(nyp.is_download_candidate());
        assert!(!paid.is_download_candidate());
        assert!(!email.is_download_candidate());
        assert!(!unknown.is_download_candidate());
        assert!(!unavailable.is_download_candidate());
        assert!(!preorder.is_download_candidate());
        assert!(!downloaded.is_download_candidate());
        assert!(!archived.is_download_candidate());
    }

    #[test]
    fn select_invert_and_range_scopes() {
        let rows = [
            make_row(1, Classification::Free, false, false, None),
            make_row(2, Classification::Nyp, false, true, None),
            make_row(3, Classification::Paid, false, false, None),
            make_row(4, Classification::Free, true, false, None),
            make_row(5, Classification::Nyp, false, false, None),
        ];
        let k = |id| ItemKey {
            kind: ItemType::Album,
            id,
        };

        assert_eq!(
            select(rows.iter(), SelectScope::All),
            [k(1), k(2), k(3), k(4), k(5)].into_iter().collect()
        );
        assert_eq!(
            select(rows.iter(), SelectScope::Downloadable),
            [k(1), k(2), k(5)].into_iter().collect()
        );
        assert_eq!(
            select(rows.iter(), SelectScope::Remaining),
            [k(1), k(5)].into_iter().collect()
        );

        let sel: BTreeSet<ItemKey> = [k(1), k(3)].into_iter().collect();
        assert_eq!(
            invert(rows.iter(), &sel, SelectScope::All),
            [k(2), k(4), k(5)].into_iter().collect()
        );
        assert_eq!(
            invert(rows.iter(), &sel, SelectScope::Downloadable),
            [k(2), k(5)].into_iter().collect()
        );
        assert_eq!(
            invert(rows.iter(), &sel, SelectScope::Remaining),
            [k(5)].into_iter().collect()
        );

        let refs: Vec<&ReleaseRow> = rows.iter().collect();
        assert_eq!(
            range_keys(&refs, k(2), k(5), SelectScope::All),
            [k(2), k(3), k(4), k(5)].into_iter().collect()
        );
        assert_eq!(
            range_keys(&refs, k(5), k(2), SelectScope::Downloadable),
            [k(2), k(5)].into_iter().collect()
        );
        assert_eq!(
            range_keys(&refs, k(5), k(1), SelectScope::Remaining),
            [k(1), k(5)].into_iter().collect()
        );
        assert!(range_keys(&refs, k(99), k(1), SelectScope::All).is_empty());

        let stats = compute_stats(rows.iter(), &sel);
        assert_eq!(
            stats,
            SelectionStats {
                selected: 2,
                selected_downloadable: 1,
                remaining_in_view: 2,
                total_in_view: 5,
            }
        );
    }
}

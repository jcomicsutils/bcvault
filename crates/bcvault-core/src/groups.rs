use crate::library::textmatch;
use crate::types::{ArtistFolder, ArtistView, FolderState, Ts};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use thiserror::Error;

pub const MAX_GROUPS: usize = 64;
pub const MAX_GROUP_SETS: usize = 32;
pub const MAX_GROUP_NAME_CHARS: usize = 40;

const SECONDS_PER_DAY: i64 = 86_400;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum GroupError {
    #[error("Name cannot be empty.")]
    EmptyName,
    #[error("Name cannot exceed {max} characters.")]
    NameTooLong { max: usize },
    #[error("A group named '{0}' already exists.")]
    DuplicateName(String),
    #[error("A grouping set named '{0}' already exists.")]
    DuplicateSetName(String),
    #[error("Cannot create more than {max} groups in a set.")]
    TooManyGroups { max: usize },
    #[error("Cannot create more than {max} grouping sets.")]
    TooManyGroupSets { max: usize },
    #[error("Group {0} was not found.")]
    GroupNotFound(i64),
    #[error("Grouping set {0} was not found.")]
    GroupSetNotFound(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinTarget {
    Group(i64),
    Ungrouped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistGroupSet {
    pub id: i64,
    pub name: String,
    pub pos: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistGroup {
    pub id: i64,
    pub set_id: i64,
    pub name: String,
    pub pos: i32,
    pub collapsed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinGrouping {
    FirstLetter,
    DownloadStatus,
    LibraryStatus,
    PricingMakeup,
    DiscographySize,
    SyncFreshness,
    ArchiveStatus,
}

impl BuiltinGrouping {
    pub const ALL: [Self; 7] = [
        Self::FirstLetter,
        Self::DownloadStatus,
        Self::LibraryStatus,
        Self::PricingMakeup,
        Self::DiscographySize,
        Self::SyncFreshness,
        Self::ArchiveStatus,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::FirstLetter => "First Letter (A–Z)",
            Self::DownloadStatus => "Download Status",
            Self::LibraryStatus => "Library Status",
            Self::PricingMakeup => "Pricing Makeup",
            Self::DiscographySize => "Discography Size",
            Self::SyncFreshness => "Sync Freshness",
            Self::ArchiveStatus => "Archive Status",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ActiveGrouping {
    #[default]
    None,
    Builtin(BuiltinGrouping),
    CustomSet(i64),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupMembers {
    pub group_id: i64,
    pub band_ids: Vec<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assignment {
    pub groups: Vec<GroupMembers>,
    pub ungrouped: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarRow {
    AllArtists,
    GroupHeader {
        id: i64,
        name: String,
        count: usize,
        collapsed: bool,
    },
    UngroupedHeader {
        count: usize,
        collapsed: bool,
    },
    Artist {
        band_id: i64,
    },
}

pub fn validate_group_name(name: &str) -> Result<String, GroupError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(GroupError::EmptyName);
    }
    if trimmed.chars().count() > MAX_GROUP_NAME_CHARS {
        return Err(GroupError::NameTooLong {
            max: MAX_GROUP_NAME_CHARS,
        });
    }
    Ok(trimmed.to_string())
}

pub fn assign_custom(
    artists: &[ArtistView],
    groups: &[ArtistGroup],
    pins: &HashMap<i64, i64>,
) -> Assignment {
    let mut group_members: Vec<GroupMembers> = groups
        .iter()
        .map(|g| GroupMembers {
            group_id: g.id,
            band_ids: Vec::new(),
        })
        .collect();
    let mut ungrouped = Vec::new();

    if groups.is_empty() {
        ungrouped.extend(artists.iter().map(|a| a.band_id));
        return Assignment {
            groups: group_members,
            ungrouped,
        };
    }

    let group_index_by_id: HashMap<i64, usize> = groups
        .iter()
        .enumerate()
        .map(|(idx, g)| (g.id, idx))
        .collect();

    for artist in artists {
        if let Some(gid) = pins.get(&artist.band_id)
            && let Some(&idx) = group_index_by_id.get(gid)
        {
            group_members[idx].band_ids.push(artist.band_id);
        } else {
            ungrouped.push(artist.band_id);
        }
    }

    Assignment {
        groups: group_members,
        ungrouped,
    }
}

fn first_letter_bucket(display_name: &str) -> (i64, &'static str) {
    let normalized = textmatch::key(display_name);
    if let Some(ch) = normalized.chars().next()
        && ch.is_ascii_alphabetic()
    {
        let idx = (ch.to_ascii_lowercase() as u8 - b'a') as i64;
        const LETTERS: [&str; 26] = [
            "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q",
            "R", "S", "T", "U", "V", "W", "X", "Y", "Z",
        ];
        return (-1 - idx, LETTERS[idx as usize]);
    }
    (-27, "#")
}

pub fn assign_builtin(
    kind: BuiltinGrouping,
    artists: &[ArtistView],
    folders: &HashMap<i64, Vec<ArtistFolder>>,
    now: Ts,
    collapsed_builtin: &BTreeSet<i64>,
) -> (Vec<ArtistGroup>, Assignment) {
    let specs: Vec<(i64, &str)> = match kind {
        BuiltinGrouping::FirstLetter => {
            let mut v: Vec<(i64, &str)> = vec![
                (-1, "A"),
                (-2, "B"),
                (-3, "C"),
                (-4, "D"),
                (-5, "E"),
                (-6, "F"),
                (-7, "G"),
                (-8, "H"),
                (-9, "I"),
                (-10, "J"),
                (-11, "K"),
                (-12, "L"),
                (-13, "M"),
                (-14, "N"),
                (-15, "O"),
                (-16, "P"),
                (-17, "Q"),
                (-18, "R"),
                (-19, "S"),
                (-20, "T"),
                (-21, "U"),
                (-22, "V"),
                (-23, "W"),
                (-24, "X"),
                (-25, "Y"),
                (-26, "Z"),
                (-27, "#"),
            ];
            v.shrink_to_fit();
            v
        }
        BuiltinGrouping::DownloadStatus => vec![
            (-101, "Remaining Free"),
            (-102, "All Downloaded"),
            (-103, "Nothing Downloaded"),
        ],
        BuiltinGrouping::LibraryStatus => vec![
            (-201, "Needs Attention"),
            (-202, "Linked & Clean"),
            (-203, "Unlinked"),
        ],
        BuiltinGrouping::PricingMakeup => vec![
            (-301, "100% Free / NYP"),
            (-302, "Email-Gated"),
            (-303, "Mixed (Free + Paid)"),
            (-304, "Paid Only"),
            (-305, "Empty / Unsynced"),
        ],
        BuiltinGrouping::DiscographySize => vec![
            (-401, "Large (20+ releases)"),
            (-402, "Medium (5–19 releases)"),
            (-403, "Small (1–4 releases)"),
            (-404, "Empty (0 releases)"),
        ],
        BuiltinGrouping::SyncFreshness => vec![
            (-501, "Never Synced"),
            (-502, "Recent (<= 7 days)"),
            (-503, "7–30 Days Ago"),
            (-504, "Stale (> 30 days)"),
        ],
        BuiltinGrouping::ArchiveStatus => vec![
            (-601, "Fully Archived"),
            (-602, "Partially Archived"),
            (-603, "Active on Disk"),
            (-604, "Nothing Downloaded"),
        ],
    };

    let mut buckets: Vec<Vec<i64>> = vec![Vec::new(); specs.len()];
    let index_by_id: HashMap<i64, usize> = specs
        .iter()
        .enumerate()
        .map(|(idx, (id, _))| (*id, idx))
        .collect();

    for artist in artists {
        let stats = artist.stats;
        let bucket_id = match kind {
            BuiltinGrouping::FirstLetter => first_letter_bucket(artist.display_name()).0,
            BuiltinGrouping::DownloadStatus => {
                if stats.eligible > stats.eligible_downloaded {
                    -101
                } else if (stats.eligible > 0 && stats.eligible_downloaded >= stats.eligible)
                    || (stats.eligible == 0 && stats.downloaded > 0)
                {
                    -102
                } else {
                    -103
                }
            }
            BuiltinGrouping::LibraryStatus => {
                let artist_folders = folders
                    .get(&artist.band_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                if stats.missing_files > 0
                    || artist_folders.iter().any(|f| f.state != FolderState::Ok)
                {
                    -201
                } else if !artist_folders.is_empty() {
                    -202
                } else {
                    -203
                }
            }
            BuiltinGrouping::PricingMakeup => {
                if stats.total == 0 {
                    -305
                } else if stats.email_required > 0 {
                    -302
                } else if stats.eligible == stats.total {
                    -301
                } else if stats.eligible > 0 {
                    -303
                } else {
                    -304
                }
            }
            BuiltinGrouping::DiscographySize => {
                if stats.total >= 20 {
                    -401
                } else if stats.total >= 5 {
                    -402
                } else if stats.total >= 1 {
                    -403
                } else {
                    -404
                }
            }
            BuiltinGrouping::SyncFreshness => match artist.last_synced_at {
                None => -501,
                Some(ts) => {
                    let age = now.saturating_sub(ts);
                    if age <= 7 * SECONDS_PER_DAY {
                        -502
                    } else if age <= 30 * SECONDS_PER_DAY {
                        -503
                    } else {
                        -504
                    }
                }
            },
            BuiltinGrouping::ArchiveStatus => {
                if stats.downloaded == 0 {
                    -604
                } else if stats.archived >= stats.downloaded {
                    -601
                } else if stats.archived > 0 {
                    -602
                } else {
                    -603
                }
            }
        };

        if let Some(&idx) = index_by_id.get(&bucket_id) {
            buckets[idx].push(artist.band_id);
        }
    }

    let mut groups = Vec::new();
    let mut group_members = Vec::new();
    for (pos, ((id, name), band_ids)) in specs.into_iter().zip(buckets).enumerate() {
        if band_ids.is_empty() {
            continue;
        }
        groups.push(ArtistGroup {
            id,
            set_id: 0,
            name: name.to_string(),
            pos: pos as i32,
            collapsed: collapsed_builtin.contains(&id),
        });
        group_members.push(GroupMembers {
            group_id: id,
            band_ids,
        });
    }

    (
        groups,
        Assignment {
            groups: group_members,
            ungrouped: Vec::new(),
        },
    )
}

pub fn artist_matches_filter(artist: &ArtistView, filter_lower: &str) -> bool {
    filter_lower.is_empty()
        || artist.display_name().to_lowercase().contains(filter_lower)
        || artist.name.to_lowercase().contains(filter_lower)
        || artist.slug.to_lowercase().contains(filter_lower)
}

pub fn build_sidebar_rows(
    assignment: &Assignment,
    groups: &[ArtistGroup],
    artists_by_id: &HashMap<i64, usize>,
    artists: &[ArtistView],
    filter: &str,
    ungrouped_collapsed: bool,
) -> Vec<SidebarRow> {
    let filter_lower = filter.trim().to_lowercase();
    let filtering = !filter_lower.is_empty();

    let matches = |band_id: i64| -> bool {
        if !filtering {
            return true;
        }
        artists_by_id
            .get(&band_id)
            .and_then(|&idx| artists.get(idx))
            .is_some_and(|a| artist_matches_filter(a, &filter_lower))
    };

    let mut rows = Vec::with_capacity(1 + artists.len() + groups.len() + 1);
    rows.push(SidebarRow::AllArtists);

    if groups.is_empty() {
        for &band_id in &assignment.ungrouped {
            if matches(band_id) {
                rows.push(SidebarRow::Artist { band_id });
            }
        }
        return rows;
    }

    for (group, members) in groups.iter().zip(&assignment.groups) {
        let visible_ids: Vec<i64> = members
            .band_ids
            .iter()
            .copied()
            .filter(|&id| matches(id))
            .collect();

        if filtering && visible_ids.is_empty() {
            continue;
        }

        let collapsed = if filtering { false } else { group.collapsed };
        rows.push(SidebarRow::GroupHeader {
            id: group.id,
            name: group.name.clone(),
            count: visible_ids.len(),
            collapsed,
        });
        if !collapsed {
            for band_id in visible_ids {
                rows.push(SidebarRow::Artist { band_id });
            }
        }
    }

    let visible_ungrouped: Vec<i64> = assignment
        .ungrouped
        .iter()
        .copied()
        .filter(|&id| matches(id))
        .collect();

    if !visible_ungrouped.is_empty() {
        let collapsed = if filtering {
            false
        } else {
            ungrouped_collapsed
        };
        rows.push(SidebarRow::UngroupedHeader {
            count: visible_ungrouped.len(),
            collapsed,
        });
        if !collapsed {
            for band_id in visible_ungrouped {
                rows.push(SidebarRow::Artist { band_id });
            }
        }
    }

    rows
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::ArtistStats;

    fn mk_artist(
        id: i64,
        name: &str,
        stats: ArtistStats,
        last_synced_at: Option<Ts>,
    ) -> ArtistView {
        ArtistView {
            band_id: id,
            slug: name.to_lowercase().replace(' ', "-"),
            name: name.to_string(),
            custom_name: None,
            url: format!("https://artist{id}.bandcamp.com"),
            last_synced_at,
            stats,
        }
    }

    fn mk_folder(band_id: i64, state: FolderState) -> ArtistFolder {
        ArtistFolder {
            id: band_id * 10,
            band_id,
            path: std::path::PathBuf::from(format!("/music/{band_id}")),
            state,
        }
    }

    #[test]
    fn test_builtin_first_letter_and_accent_folding() {
        let artists = vec![
            mk_artist(1, "Éclair", ArtistStats::default(), None),
            mk_artist(2, "99 Luftballons", ArtistStats::default(), None),
            mk_artist(3, "Alpha", ArtistStats::default(), None),
        ];
        let folders = HashMap::new();
        let collapsed = BTreeSet::from([-1]); // A collapsed
        let (groups, assign) = assign_builtin(
            BuiltinGrouping::FirstLetter,
            &artists,
            &folders,
            1000,
            &collapsed,
        );
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].name, "A");
        assert!(groups[0].collapsed);
        assert_eq!(groups[1].name, "E");
        assert_eq!(groups[2].name, "#");
        assert_eq!(assign.groups[0].band_ids, vec![3]);
        assert_eq!(assign.groups[1].band_ids, vec![1]);
        assert_eq!(assign.groups[2].band_ids, vec![2]);
        assert!(assign.ungrouped.is_empty());
    }

    #[test]
    fn test_builtin_download_and_library_and_archive_and_size_and_freshness() {
        let now = 100 * SECONDS_PER_DAY;
        let artists = vec![
            mk_artist(
                1,
                "A",
                ArtistStats {
                    total: 25,
                    eligible: 25,
                    eligible_downloaded: 10,
                    downloaded: 10,
                    archived: 10,
                    missing_files: 0,
                    ..Default::default()
                },
                Some(now - 2 * SECONDS_PER_DAY),
            ),
            mk_artist(
                2,
                "B",
                ArtistStats {
                    total: 8,
                    eligible: 8,
                    eligible_downloaded: 8,
                    downloaded: 8,
                    archived: 0,
                    missing_files: 1,
                    ..Default::default()
                },
                Some(now - 15 * SECONDS_PER_DAY),
            ),
            mk_artist(
                3,
                "C",
                ArtistStats {
                    total: 2,
                    eligible: 1,
                    email_required: 1,
                    eligible_downloaded: 0,
                    downloaded: 0,
                    ..Default::default()
                },
                None,
            ),
        ];
        let mut folders = HashMap::new();
        folders.insert(1, vec![mk_folder(1, FolderState::Ok)]);
        folders.insert(2, vec![mk_folder(2, FolderState::Ok)]);

        let (dl_g, dl_a) = assign_builtin(
            BuiltinGrouping::DownloadStatus,
            &artists,
            &folders,
            now,
            &BTreeSet::new(),
        );
        assert_eq!(
            dl_g.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec!["Remaining Free", "All Downloaded"]
        );
        assert_eq!(dl_a.groups[0].band_ids, vec![1, 3]);
        assert_eq!(dl_a.groups[1].band_ids, vec![2]);

        let (lib_g, lib_a) = assign_builtin(
            BuiltinGrouping::LibraryStatus,
            &artists,
            &folders,
            now,
            &BTreeSet::new(),
        );
        assert_eq!(
            lib_g.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec!["Needs Attention", "Linked & Clean", "Unlinked"]
        );
        assert_eq!(lib_a.groups[0].band_ids, vec![2]);
        assert_eq!(lib_a.groups[1].band_ids, vec![1]);
        assert_eq!(lib_a.groups[2].band_ids, vec![3]);

        let (sz_g, _) = assign_builtin(
            BuiltinGrouping::DiscographySize,
            &artists,
            &folders,
            now,
            &BTreeSet::new(),
        );
        assert_eq!(
            sz_g.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec![
                "Large (20+ releases)",
                "Medium (5–19 releases)",
                "Small (1–4 releases)"
            ]
        );

        let (fr_g, _) = assign_builtin(
            BuiltinGrouping::SyncFreshness,
            &artists,
            &folders,
            now,
            &BTreeSet::new(),
        );
        assert_eq!(
            fr_g.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec!["Never Synced", "Recent (<= 7 days)", "7–30 Days Ago"]
        );

        let (ar_g, _) = assign_builtin(
            BuiltinGrouping::ArchiveStatus,
            &artists,
            &folders,
            now,
            &BTreeSet::new(),
        );
        assert_eq!(
            ar_g.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec!["Fully Archived", "Active on Disk", "Nothing Downloaded"]
        );

        let (pr_g, _) = assign_builtin(
            BuiltinGrouping::PricingMakeup,
            &artists,
            &folders,
            now,
            &BTreeSet::new(),
        );
        assert_eq!(
            pr_g.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec!["100% Free / NYP", "Email-Gated"]
        );
    }

    #[test]
    fn test_custom_assignment_and_sidebar_rows() {
        let artists = vec![
            mk_artist(1, "Finished Band", ArtistStats::default(), None),
            mk_artist(2, "Unplaced Band", ArtistStats::default(), None),
        ];
        let groups = vec![ArtistGroup {
            id: 10,
            set_id: 1,
            name: "Finished".to_string(),
            pos: 0,
            collapsed: false,
        }];
        let pins = HashMap::from([(1, 10)]);
        let assign = assign_custom(&artists, &groups, &pins);
        assert_eq!(assign.groups[0].band_ids, vec![1]);
        assert_eq!(assign.ungrouped, vec![2]);

        let by_id: HashMap<i64, usize> = artists
            .iter()
            .enumerate()
            .map(|(i, a)| (a.band_id, i))
            .collect();
        let rows = build_sidebar_rows(&assign, &groups, &by_id, &artists, "", false);
        assert_eq!(
            rows,
            vec![
                SidebarRow::AllArtists,
                SidebarRow::GroupHeader {
                    id: 10,
                    name: "Finished".to_string(),
                    count: 1,
                    collapsed: false,
                },
                SidebarRow::Artist { band_id: 1 },
                SidebarRow::UngroupedHeader {
                    count: 1,
                    collapsed: false,
                },
                SidebarRow::Artist { band_id: 2 },
            ]
        );
    }
}

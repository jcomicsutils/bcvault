use crate::library::textmatch;
use crate::types::{ArtistFolder, ArtistView, FolderState, Ts};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

pub const MAX_GROUPS: usize = 64;
pub const MAX_GROUP_NAME_CHARS: usize = 40;
pub const MAX_RULE_CONDITIONS: usize = 8;
pub const MAX_CONDITION_STRING_CHARS: usize = 64;
pub const RULE_JSON_VERSION: u32 = 1;

const SECONDS_PER_DAY: i64 = 86_400;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum GroupError {
    #[error("Group name cannot be empty.")]
    EmptyName,
    #[error("Group name cannot exceed {max} characters.")]
    NameTooLong { max: usize },
    #[error("A group named '{0}' already exists.")]
    DuplicateName(String),
    #[error("Cannot create more than {max} groups.")]
    TooManyGroups { max: usize },
    #[error("A rule cannot have more than {max} conditions.")]
    TooManyConditions { max: usize },
    #[error("Condition text cannot exceed {max} characters.")]
    ConditionStringTooLong { max: usize },
    #[error("Invalid initial range '{from}' to '{to}'.")]
    InvalidInitialRange { from: char, to: char },
    #[error("Invalid release count range: min ({at_least}) exceeds max ({at_most}).")]
    InvalidReleaseRange { at_least: u32, at_most: u32 },
    #[error("Group {0} was not found.")]
    GroupNotFound(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinTarget {
    Group(i64),
    Ungrouped,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchMode {
    All,
    Any,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadState {
    Complete,
    Partial,
    NoneYet,
    NothingEligible,
    HasRemaining,
}

impl DownloadState {
    pub const ALL: [Self; 5] = [
        Self::Complete,
        Self::Partial,
        Self::NoneYet,
        Self::NothingEligible,
        Self::HasRemaining,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Complete => "Fully downloaded",
            Self::Partial => "Partially downloaded",
            Self::NoneYet => "None downloaded yet",
            Self::NothingEligible => "Nothing eligible",
            Self::HasRemaining => "Has remaining",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibraryState {
    HasFolder,
    NoFolder,
    FolderProblem,
    HasMissingFiles,
    HasArchived,
}

impl LibraryState {
    pub const ALL: [Self; 5] = [
        Self::HasFolder,
        Self::NoFolder,
        Self::FolderProblem,
        Self::HasMissingFiles,
        Self::HasArchived,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::HasFolder => "Has linked folder",
            Self::NoFolder => "No linked folder",
            Self::FolderProblem => "Folder problem",
            Self::HasMissingFiles => "Has missing files",
            Self::HasArchived => "Has archived releases",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionKind {
    NameContains(String),
    NameStartsWith(String),
    NameInitial {
        from: char,
        to: char,
    },
    Downloads(DownloadState),
    Library(LibraryState),
    SyncedOlderThanDays(u32),
    NeverSynced,
    TotalReleases {
        at_least: Option<u32>,
        at_most: Option<u32>,
    },
    CustomDomain(bool),
}

impl ConditionKind {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::NameContains(_) => "Name contains",
            Self::NameStartsWith(_) => "Name starts with",
            Self::NameInitial { .. } => "Name initial range",
            Self::Downloads(_) => "Downloads",
            Self::Library(_) => "Library",
            Self::SyncedOlderThanDays(_) => "Synced older than (days)",
            Self::NeverSynced => "Never synced",
            Self::TotalReleases { .. } => "Total releases",
            Self::CustomDomain(_) => "Custom domain",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    pub negate: bool,
    pub kind: ConditionKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupRule {
    pub mode: MatchMode,
    pub conditions: Vec<Condition>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistGroup {
    pub id: i64,
    pub name: String,
    pub pos: i32,
    pub collapsed: bool,
    pub rule: Option<GroupRule>,
}

pub struct ArtistFacts<'a> {
    pub view: &'a ArtistView,
    pub folders: &'a [ArtistFolder],
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RulePreview {
    pub total_matching: usize,
    pub pinned_elsewhere: usize,
    pub sample_names: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulePreset {
    StartsWithAToF,
    FullyDownloaded,
    HasRemaining,
    NoLinkedFolder,
    FolderProblems,
    NotSynced30Days,
    CustomDomain,
}

impl RulePreset {
    pub const ALL: [Self; 7] = [
        Self::StartsWithAToF,
        Self::FullyDownloaded,
        Self::HasRemaining,
        Self::NoLinkedFolder,
        Self::FolderProblems,
        Self::NotSynced30Days,
        Self::CustomDomain,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::StartsWithAToF => "Starts with A to F",
            Self::FullyDownloaded => "Fully downloaded",
            Self::HasRemaining => "Has remaining releases",
            Self::NoLinkedFolder => "No linked folder",
            Self::FolderProblems => "Folder problems",
            Self::NotSynced30Days => "Not synced in 30 days",
            Self::CustomDomain => "Custom domain",
        }
    }

    pub fn build(self) -> (&'static str, GroupRule) {
        let one = |kind: ConditionKind| GroupRule {
            mode: MatchMode::All,
            conditions: vec![Condition {
                negate: false,
                kind,
            }],
        };
        match self {
            Self::StartsWithAToF => (
                "A–F",
                one(ConditionKind::NameInitial { from: 'a', to: 'f' }),
            ),
            Self::FullyDownloaded => (
                "Fully downloaded",
                one(ConditionKind::Downloads(DownloadState::Complete)),
            ),
            Self::HasRemaining => (
                "Has remaining",
                one(ConditionKind::Downloads(DownloadState::HasRemaining)),
            ),
            Self::NoLinkedFolder => (
                "No linked folder",
                one(ConditionKind::Library(LibraryState::NoFolder)),
            ),
            Self::FolderProblems => (
                "Folder problems",
                one(ConditionKind::Library(LibraryState::FolderProblem)),
            ),
            Self::NotSynced30Days => (
                "Not synced 30d",
                GroupRule {
                    mode: MatchMode::Any,
                    conditions: vec![
                        Condition {
                            negate: false,
                            kind: ConditionKind::SyncedOlderThanDays(30),
                        },
                        Condition {
                            negate: false,
                            kind: ConditionKind::NeverSynced,
                        },
                    ],
                },
            ),
            Self::CustomDomain => ("Custom domain", one(ConditionKind::CustomDomain(true))),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct StoredGroupRule {
    v: u32,
    #[serde(flatten)]
    rule: GroupRule,
}

pub fn serialize_rule_json(rule: &GroupRule) -> Result<String, serde_json::Error> {
    serde_json::to_string(&StoredGroupRule {
        v: RULE_JSON_VERSION,
        rule: rule.clone(),
    })
}

pub fn deserialize_rule_json(json: &str) -> Result<GroupRule, String> {
    let stored: StoredGroupRule =
        serde_json::from_str(json).map_err(|e| format!("invalid rule JSON: {e}"))?;
    if stored.v != RULE_JSON_VERSION {
        return Err(format!("unsupported rule version {}", stored.v));
    }
    stored.rule.validate().map_err(|e| e.to_string())?;
    Ok(stored.rule)
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

impl ConditionKind {
    pub fn validate(&self) -> Result<(), GroupError> {
        match self {
            Self::NameContains(s) | Self::NameStartsWith(s) => {
                if s.chars().count() > MAX_CONDITION_STRING_CHARS {
                    return Err(GroupError::ConditionStringTooLong {
                        max: MAX_CONDITION_STRING_CHARS,
                    });
                }
            }
            Self::NameInitial { from, to } => {
                if *from == '#' && *to == '#' {
                    return Ok(());
                }
                if !from.is_ascii_alphabetic()
                    || !to.is_ascii_alphabetic()
                    || from.to_ascii_lowercase() > to.to_ascii_lowercase()
                {
                    return Err(GroupError::InvalidInitialRange {
                        from: *from,
                        to: *to,
                    });
                }
            }
            Self::TotalReleases {
                at_least: Some(lo),
                at_most: Some(hi),
            } if lo > hi => {
                return Err(GroupError::InvalidReleaseRange {
                    at_least: *lo,
                    at_most: *hi,
                });
            }
            Self::Downloads(_)
            | Self::Library(_)
            | Self::SyncedOlderThanDays(_)
            | Self::NeverSynced
            | Self::TotalReleases { .. }
            | Self::CustomDomain(_) => {}
        }
        Ok(())
    }

    pub fn matches(&self, facts: &ArtistFacts<'_>, now: Ts) -> bool {
        self.matches_with_key(facts, now, None)
    }

    fn matches_with_key(
        &self,
        facts: &ArtistFacts<'_>,
        now: Ts,
        cached_name_key: Option<&str>,
    ) -> bool {
        let view = facts.view;
        let stats = &view.stats;
        match self {
            Self::NameContains(needle) => {
                let needle_key = textmatch::key(needle);
                if needle_key.is_empty() {
                    return false;
                }
                match cached_name_key {
                    Some(k) => k.contains(&needle_key),
                    None => textmatch::key(view.display_name()).contains(&needle_key),
                }
            }
            Self::NameStartsWith(prefix) => {
                let prefix_key = textmatch::key(prefix);
                if prefix_key.is_empty() {
                    return false;
                }
                match cached_name_key {
                    Some(k) => k.starts_with(&prefix_key),
                    None => textmatch::key(view.display_name()).starts_with(&prefix_key),
                }
            }
            Self::NameInitial { from, to } => {
                let first_char = match cached_name_key {
                    Some(k) => k.chars().next(),
                    None => textmatch::key(view.display_name()).chars().next(),
                };
                if *from == '#' && *to == '#' {
                    return match first_char {
                        Some(c) => !c.is_ascii_lowercase(),
                        None => !view.display_name().trim().is_empty(),
                    };
                }
                let Some(c) = first_char else {
                    return false;
                };
                let lo = from.to_ascii_lowercase();
                let hi = to.to_ascii_lowercase();
                c.is_ascii_lowercase() && (lo..=hi).contains(&c)
            }
            Self::Downloads(state) => match state {
                DownloadState::Complete => {
                    stats.eligible > 0 && stats.eligible_downloaded >= stats.eligible
                }
                DownloadState::Partial => {
                    stats.eligible_downloaded > 0 && stats.eligible_downloaded < stats.eligible
                }
                DownloadState::NoneYet => stats.eligible > 0 && stats.eligible_downloaded == 0,
                DownloadState::NothingEligible => stats.eligible == 0,
                DownloadState::HasRemaining => stats.eligible_downloaded < stats.eligible,
            },
            Self::Library(state) => match state {
                LibraryState::HasFolder => !facts.folders.is_empty(),
                LibraryState::NoFolder => facts.folders.is_empty(),
                LibraryState::FolderProblem => {
                    facts.folders.iter().any(|f| f.state != FolderState::Ok)
                }
                LibraryState::HasMissingFiles => stats.missing_files > 0,
                LibraryState::HasArchived => stats.archived > 0,
            },
            Self::SyncedOlderThanDays(days) => {
                let threshold = i64::from(*days).saturating_mul(SECONDS_PER_DAY);
                matches!(
                    view.last_synced_at,
                    Some(ts) if now.saturating_sub(ts) >= threshold
                )
            }
            Self::NeverSynced => view.last_synced_at.is_none(),
            Self::TotalReleases { at_least, at_most } => {
                at_least.is_none_or(|min| stats.total >= min)
                    && at_most.is_none_or(|max| stats.total <= max)
            }
            Self::CustomDomain(expected) => is_custom_domain(&view.url) == *expected,
        }
    }
}

fn is_custom_domain(url_str: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url_str) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host_lower = host.to_ascii_lowercase();
    !(host_lower == "bandcamp.com" || host_lower.ends_with(".bandcamp.com"))
}

impl Condition {
    pub fn matches(&self, facts: &ArtistFacts<'_>, now: Ts) -> bool {
        self.matches_with_key(facts, now, None)
    }

    fn matches_with_key(
        &self,
        facts: &ArtistFacts<'_>,
        now: Ts,
        cached_name_key: Option<&str>,
    ) -> bool {
        let raw = self.kind.matches_with_key(facts, now, cached_name_key);
        if self.negate { !raw } else { raw }
    }
}

impl GroupRule {
    pub fn validate(&self) -> Result<(), GroupError> {
        if self.conditions.len() > MAX_RULE_CONDITIONS {
            return Err(GroupError::TooManyConditions {
                max: MAX_RULE_CONDITIONS,
            });
        }
        for cond in &self.conditions {
            cond.kind.validate()?;
        }
        Ok(())
    }

    pub fn uses_name_key(&self) -> bool {
        self.conditions.iter().any(|c| {
            matches!(
                c.kind,
                ConditionKind::NameContains(_)
                    | ConditionKind::NameStartsWith(_)
                    | ConditionKind::NameInitial { .. }
            )
        })
    }

    pub fn matches(&self, facts: &ArtistFacts<'_>, now: Ts) -> bool {
        self.matches_with_key(facts, now, None)
    }

    fn matches_with_key(
        &self,
        facts: &ArtistFacts<'_>,
        now: Ts,
        cached_name_key: Option<&str>,
    ) -> bool {
        if self.conditions.is_empty() {
            return false;
        }
        match self.mode {
            MatchMode::All => self
                .conditions
                .iter()
                .all(|c| c.matches_with_key(facts, now, cached_name_key)),
            MatchMode::Any => self
                .conditions
                .iter()
                .any(|c| c.matches_with_key(facts, now, cached_name_key)),
        }
    }
}

pub fn assign(
    artists: &[ArtistView],
    groups: &[ArtistGroup],
    pins: &HashMap<i64, Option<i64>>,
    folders: &HashMap<i64, Vec<ArtistFolder>>,
    now: Ts,
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

    let mut pos_order: Vec<usize> = (0..groups.len()).collect();
    pos_order.sort_by_key(|&idx| (groups[idx].pos, groups[idx].id));

    let any_name_rule = groups
        .iter()
        .any(|g| g.rule.as_ref().is_some_and(GroupRule::uses_name_key));

    for artist in artists {
        if let Some(pin) = pins.get(&artist.band_id) {
            match pin {
                Some(gid) => {
                    if let Some(&idx) = group_index_by_id.get(gid) {
                        group_members[idx].band_ids.push(artist.band_id);
                        continue;
                    }
                    // Pin to a deleted group is ignored; fall through to rules.
                }
                None => {
                    ungrouped.push(artist.band_id);
                    continue;
                }
            }
        }

        let artist_folders = folders
            .get(&artist.band_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let facts = ArtistFacts {
            view: artist,
            folders: artist_folders,
        };
        let name_key = if any_name_rule {
            Some(textmatch::key(artist.display_name()))
        } else {
            None
        };

        let matched_idx = pos_order.iter().copied().find(|&idx| {
            groups[idx]
                .rule
                .as_ref()
                .is_some_and(|rule| rule.matches_with_key(&facts, now, name_key.as_deref()))
        });

        match matched_idx {
            Some(idx) => group_members[idx].band_ids.push(artist.band_id),
            None => ungrouped.push(artist.band_id),
        }
    }

    Assignment {
        groups: group_members,
        ungrouped,
    }
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

pub fn preview_rule(
    rule: &GroupRule,
    artists: &[ArtistView],
    pins: &HashMap<i64, Option<i64>>,
    folders: &HashMap<i64, Vec<ArtistFolder>>,
    now: Ts,
    editing_group_id: Option<i64>,
) -> RulePreview {
    if rule.conditions.is_empty() || rule.validate().is_err() {
        return RulePreview::default();
    }

    let mut preview = RulePreview::default();
    let uses_name = rule.uses_name_key();

    for artist in artists {
        let artist_folders = folders
            .get(&artist.band_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let facts = ArtistFacts {
            view: artist,
            folders: artist_folders,
        };
        let name_key = if uses_name {
            Some(textmatch::key(artist.display_name()))
        } else {
            None
        };
        if rule.matches_with_key(&facts, now, name_key.as_deref()) {
            preview.total_matching += 1;
            if let Some(pin) = pins.get(&artist.band_id) {
                let elsewhere = match pin {
                    None => true,
                    Some(gid) => editing_group_id != Some(*gid),
                };
                if elsewhere {
                    preview.pinned_elsewhere += 1;
                }
            }
            if preview.sample_names.len() < 8 {
                preview.sample_names.push(artist.display_name().to_string());
            }
        }
    }

    preview
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::ArtistStats;
    use std::path::PathBuf;
    use std::time::Instant;

    fn make_artist(band_id: i64, name: &str) -> ArtistView {
        ArtistView {
            band_id,
            slug: format!("artist-{band_id}"),
            name: name.to_string(),
            custom_name: None,
            url: format!("https://artist-{band_id}.bandcamp.com"),
            last_synced_at: Some(1_700_000_000),
            stats: ArtistStats::default(),
        }
    }

    fn make_folder(band_id: i64, state: FolderState) -> ArtistFolder {
        ArtistFolder {
            id: band_id * 10,
            band_id,
            path: PathBuf::from(format!("/music/{band_id}")),
            state,
        }
    }

    // Test 1: Each ConditionKind with matching, non-matching and negated artists
    #[test]
    fn test_1_each_condition_kind_and_negation() {
        let now = 1_700_000_000 + 40 * SECONDS_PER_DAY;

        let mut a1 = make_artist(1, "Émilie Simon");
        a1.stats = ArtistStats {
            eligible: 5,
            eligible_downloaded: 5,
            total: 10,
            missing_files: 0,
            archived: 0,
            ..Default::default()
        };
        a1.last_synced_at = Some(1_700_000_000); // 40 days ago
        a1.url = "https://music.emiliesimon.com".to_string();

        let mut a2 = make_artist(2, "Boards of Canada");
        a2.stats = ArtistStats {
            eligible: 4,
            eligible_downloaded: 2,
            total: 4,
            missing_files: 1,
            archived: 1,
            ..Default::default()
        };
        a2.last_synced_at = Some(now - 5 * SECONDS_PER_DAY); // 5 days ago

        let mut a3 = make_artist(3, "Zebrahead");
        a3.stats = ArtistStats {
            eligible: 3,
            eligible_downloaded: 0,
            total: 3,
            ..Default::default()
        };
        a3.last_synced_at = None;

        let mut a4 = make_artist(4, "100 gecs");
        a4.stats = ArtistStats {
            eligible: 0,
            eligible_downloaded: 0,
            total: 2,
            ..Default::default()
        };

        let f1 = [make_folder(1, FolderState::Ok)];
        let f2 = [make_folder(2, FolderState::Missing)];
        let empty: [ArtistFolder; 0] = [];

        let facts1 = ArtistFacts {
            view: &a1,
            folders: &f1,
        };
        let facts2 = ArtistFacts {
            view: &a2,
            folders: &f2,
        };
        let facts3 = ArtistFacts {
            view: &a3,
            folders: &empty,
        };
        let facts4 = ArtistFacts {
            view: &a4,
            folders: &empty,
        };

        let eval = |kind: ConditionKind, negate: bool, f: &ArtistFacts<'_>| {
            Condition { negate, kind }.matches(f, now)
        };

        // NameContains / NameStartsWith
        assert!(eval(
            ConditionKind::NameContains("simon".into()),
            false,
            &facts1
        ));
        assert!(!eval(
            ConditionKind::NameContains("simon".into()),
            true,
            &facts1
        ));
        assert!(!eval(
            ConditionKind::NameContains("simon".into()),
            false,
            &facts2
        ));
        assert!(eval(
            ConditionKind::NameStartsWith("emilie".into()),
            false,
            &facts1
        ));
        assert!(!eval(
            ConditionKind::NameStartsWith("canada".into()),
            false,
            &facts2
        ));

        // NameInitial
        assert!(eval(
            ConditionKind::NameInitial { from: 'a', to: 'f' },
            false,
            &facts1
        ));
        assert!(eval(
            ConditionKind::NameInitial { from: 'a', to: 'f' },
            false,
            &facts2
        ));
        assert!(!eval(
            ConditionKind::NameInitial { from: 'a', to: 'f' },
            false,
            &facts3
        ));
        assert!(eval(
            ConditionKind::NameInitial { from: '#', to: '#' },
            false,
            &facts4
        ));
        assert!(!eval(
            ConditionKind::NameInitial { from: '#', to: '#' },
            false,
            &facts1
        ));

        // Downloads
        assert!(eval(
            ConditionKind::Downloads(DownloadState::Complete),
            false,
            &facts1
        ));
        assert!(!eval(
            ConditionKind::Downloads(DownloadState::Complete),
            false,
            &facts2
        ));
        assert!(eval(
            ConditionKind::Downloads(DownloadState::Partial),
            false,
            &facts2
        ));
        assert!(eval(
            ConditionKind::Downloads(DownloadState::NoneYet),
            false,
            &facts3
        ));
        assert!(eval(
            ConditionKind::Downloads(DownloadState::NothingEligible),
            false,
            &facts4
        ));
        assert!(eval(
            ConditionKind::Downloads(DownloadState::HasRemaining),
            false,
            &facts2
        ));
        assert!(!eval(
            ConditionKind::Downloads(DownloadState::HasRemaining),
            false,
            &facts1
        ));

        // Library
        assert!(eval(
            ConditionKind::Library(LibraryState::HasFolder),
            false,
            &facts1
        ));
        assert!(eval(
            ConditionKind::Library(LibraryState::NoFolder),
            false,
            &facts3
        ));
        assert!(eval(
            ConditionKind::Library(LibraryState::FolderProblem),
            false,
            &facts2
        ));
        assert!(!eval(
            ConditionKind::Library(LibraryState::FolderProblem),
            false,
            &facts1
        ));
        assert!(eval(
            ConditionKind::Library(LibraryState::HasMissingFiles),
            false,
            &facts2
        ));
        assert!(eval(
            ConditionKind::Library(LibraryState::HasArchived),
            false,
            &facts2
        ));

        // SyncedOlderThanDays / NeverSynced
        assert!(eval(ConditionKind::SyncedOlderThanDays(30), false, &facts1));
        assert!(!eval(
            ConditionKind::SyncedOlderThanDays(30),
            false,
            &facts2
        ));
        assert!(!eval(
            ConditionKind::SyncedOlderThanDays(30),
            false,
            &facts3
        ));
        assert!(eval(ConditionKind::NeverSynced, false, &facts3));
        assert!(!eval(ConditionKind::NeverSynced, false, &facts1));

        // TotalReleases
        assert!(eval(
            ConditionKind::TotalReleases {
                at_least: Some(5),
                at_most: Some(15)
            },
            false,
            &facts1
        ));
        assert!(!eval(
            ConditionKind::TotalReleases {
                at_least: Some(5),
                at_most: Some(15)
            },
            false,
            &facts2
        ));

        // CustomDomain
        assert!(eval(ConditionKind::CustomDomain(true), false, &facts1));
        assert!(!eval(ConditionKind::CustomDomain(true), false, &facts2));
        assert!(eval(ConditionKind::CustomDomain(false), false, &facts2));
    }

    // Test 2: Empty rule, 9 conditions, 65-character string, reversed initial range
    #[test]
    fn test_2_rule_validation_and_empty_rule() {
        let a = make_artist(1, "Alpha");
        let facts = ArtistFacts {
            view: &a,
            folders: &[],
        };

        let empty_rule = GroupRule {
            mode: MatchMode::All,
            conditions: Vec::new(),
        };
        assert!(empty_rule.validate().is_ok());
        assert!(!empty_rule.matches(&facts, 1_700_000_000));

        let nine_conds = GroupRule {
            mode: MatchMode::All,
            conditions: vec![
                Condition {
                    negate: false,
                    kind: ConditionKind::NeverSynced,
                };
                9
            ],
        };
        assert_eq!(
            nine_conds.validate(),
            Err(GroupError::TooManyConditions { max: 8 })
        );

        let long_str = "x".repeat(65);
        let long_rule = GroupRule {
            mode: MatchMode::All,
            conditions: vec![Condition {
                negate: false,
                kind: ConditionKind::NameContains(long_str),
            }],
        };
        assert_eq!(
            long_rule.validate(),
            Err(GroupError::ConditionStringTooLong { max: 64 })
        );

        let reversed_range = GroupRule {
            mode: MatchMode::All,
            conditions: vec![Condition {
                negate: false,
                kind: ConditionKind::NameInitial { from: 'f', to: 'a' },
            }],
        };
        assert_eq!(
            reversed_range.validate(),
            Err(GroupError::InvalidInitialRange { from: 'f', to: 'a' })
        );
    }

    // Test 3: Precedence: pin beats rules; None pin beats rules; first matching group in pos order wins; pin to deleted group ignored
    #[test]
    fn test_3_assignment_precedence() {
        let artists = vec![
            make_artist(1, "Alpha"),
            make_artist(2, "Beta"),
            make_artist(3, "Gamma"),
            make_artist(4, "Delta"),
            make_artist(5, "Zeta"),
        ];

        let rule_a_to_g = GroupRule {
            mode: MatchMode::All,
            conditions: vec![Condition {
                negate: false,
                kind: ConditionKind::NameInitial { from: 'a', to: 'g' },
            }],
        };
        let rule_all_names = GroupRule {
            mode: MatchMode::All,
            conditions: vec![Condition {
                negate: false,
                kind: ConditionKind::NameInitial { from: 'a', to: 'z' },
            }],
        };

        let groups = vec![
            ArtistGroup {
                id: 10,
                name: "First".into(),
                pos: 0,
                collapsed: false,
                rule: Some(rule_a_to_g),
            },
            ArtistGroup {
                id: 20,
                name: "Second".into(),
                pos: 1,
                collapsed: false,
                rule: Some(rule_all_names),
            },
            ArtistGroup {
                id: 30,
                name: "Manual".into(),
                pos: 2,
                collapsed: false,
                rule: None,
            },
        ];

        let mut pins = HashMap::new();
        pins.insert(1, Some(30)); // Alpha pinned to Manual (beats First & Second)
        pins.insert(2, None); // Beta pinned to Ungrouped (beats First & Second)
        pins.insert(4, Some(999)); // Delta pinned to deleted group -> falls through to First

        let folders = HashMap::new();
        let res = assign(&artists, &groups, &pins, &folders, 1_700_000_000);

        assert_eq!(res.groups[0].band_ids, vec![3, 4]); // Gamma, Delta
        assert_eq!(res.groups[1].band_ids, vec![5]); // Zeta
        assert_eq!(res.groups[2].band_ids, vec![1]); // Alpha
        assert_eq!(res.ungrouped, vec![2]); // Beta
    }

    // Test 4: build_sidebar_rows with no groups, with groups and empty Ungrouped, with filter, with collapsed groups
    #[test]
    fn test_4_build_sidebar_rows() {
        let artists = vec![
            make_artist(1, "Alpha"),
            make_artist(2, "Beta"),
            make_artist(3, "Gamma"),
        ];
        let by_id: HashMap<i64, usize> = artists
            .iter()
            .enumerate()
            .map(|(i, a)| (a.band_id, i))
            .collect();
        let empty_pins = HashMap::new();
        let empty_folders = HashMap::new();

        // No groups: identical to flat list (AllArtists + artists, no Ungrouped header)
        let assign_none = assign(&artists, &[], &empty_pins, &empty_folders, 0);
        let rows_none = build_sidebar_rows(&assign_none, &[], &by_id, &artists, "", false);
        assert_eq!(
            rows_none,
            vec![
                SidebarRow::AllArtists,
                SidebarRow::Artist { band_id: 1 },
                SidebarRow::Artist { band_id: 2 },
                SidebarRow::Artist { band_id: 3 },
            ]
        );

        // Groups covering all artists: empty Ungrouped is omitted
        let groups = vec![
            ArtistGroup {
                id: 10,
                name: "Collapsed Group".into(),
                pos: 0,
                collapsed: true,
                rule: Some(GroupRule {
                    mode: MatchMode::All,
                    conditions: vec![Condition {
                        negate: false,
                        kind: ConditionKind::NameStartsWith("a".into()),
                    }],
                }),
            },
            ArtistGroup {
                id: 20,
                name: "Open Group".into(),
                pos: 1,
                collapsed: false,
                rule: Some(GroupRule {
                    mode: MatchMode::All,
                    conditions: vec![Condition {
                        negate: false,
                        kind: ConditionKind::NameInitial { from: 'b', to: 'z' },
                    }],
                }),
            },
        ];
        let assign_all = assign(&artists, &groups, &empty_pins, &empty_folders, 0);
        let rows_all = build_sidebar_rows(&assign_all, &groups, &by_id, &artists, "", false);
        assert_eq!(
            rows_all,
            vec![
                SidebarRow::AllArtists,
                SidebarRow::GroupHeader {
                    id: 10,
                    name: "Collapsed Group".into(),
                    count: 1,
                    collapsed: true,
                },
                SidebarRow::GroupHeader {
                    id: 20,
                    name: "Open Group".into(),
                    count: 2,
                    collapsed: false,
                },
                SidebarRow::Artist { band_id: 2 },
                SidebarRow::Artist { band_id: 3 },
            ]
        );

        // Filter hides non-matching groups and expands collapsed matching groups without mutating stored collapsed
        let rows_filtered =
            build_sidebar_rows(&assign_all, &groups, &by_id, &artists, "alp", false);
        assert_eq!(
            rows_filtered,
            vec![
                SidebarRow::AllArtists,
                SidebarRow::GroupHeader {
                    id: 10,
                    name: "Collapsed Group".into(),
                    count: 1,
                    collapsed: false,
                },
                SidebarRow::Artist { band_id: 1 },
            ]
        );
        assert!(groups[0].collapsed);
    }

    // Test 5: Name matching with accents and case (Émilie vs emilie)
    #[test]
    fn test_5_accent_and_case_insensitive_name_matching() {
        let artist = make_artist(1, "Émilie Simon");
        let facts = ArtistFacts {
            view: &artist,
            folders: &[],
        };
        assert!(
            ConditionKind::NameStartsWith("emilie".into()).matches(&facts, 0),
            "Émilie should match emilie"
        );
        assert!(
            ConditionKind::NameContains("EMILIE".into()).matches(&facts, 0),
            "Émilie should match EMILIE"
        );
    }

    // Test 6: 10 000 artists, 20 groups with rules completes well under a frame
    #[test]
    fn test_6_performance_10k_artists_20_groups() {
        let artists: Vec<ArtistView> = (0..10_000)
            .map(|i| make_artist(i, &format!("Artist Number {i}")))
            .collect();
        let groups: Vec<ArtistGroup> = (0..20)
            .map(|g| ArtistGroup {
                id: g + 1,
                name: format!("Group {g}"),
                pos: g as i32,
                collapsed: false,
                rule: Some(GroupRule {
                    mode: MatchMode::All,
                    conditions: vec![
                        Condition {
                            negate: false,
                            kind: ConditionKind::NameContains(format!("number {g}")),
                        },
                        Condition {
                            negate: false,
                            kind: ConditionKind::CustomDomain(false),
                        },
                    ],
                }),
            })
            .collect();
        let pins = HashMap::new();
        let folders = HashMap::new();

        let start = Instant::now();
        let assignment = assign(&artists, &groups, &pins, &folders, 1_700_000_000);
        let elapsed = start.elapsed();

        assert_eq!(assignment.groups.len(), 20);
        // Generous bound (250 ms) so debug/CI builds don't flake; typically takes a few ms.
        assert!(
            elapsed.as_millis() < 250,
            "assign took too long: {elapsed:?}"
        );
    }
}

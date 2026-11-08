use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::Datelike;
use strsim::normalized_levenshtein;

use crate::library::folder_style::FolderStyle;
use crate::library::scan::AlbumCandidate;
use crate::library::textmatch::key;
use crate::types::{ItemType, ReleaseRow};

/// Matching signal observed between a local candidate and a Bandcamp release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Signal {
    /// Tag URL equals the release URL (normalised host and path).
    UrlTag,
    /// Tag album title matches release title.
    AlbumTag,
    /// Folder or zip name matches a release name variant.
    FolderName,
    /// Track counts match between local audio files and release.
    TrackCount,
    /// Tag release year matches release year.
    Year,
}

/// Match score with contributing signals.
#[derive(Debug, Clone, PartialEq)]
pub struct Score {
    /// Numeric score between 0.0 and 1.0.
    pub value: f32,
    /// Signals contributing to this score.
    pub signals: Vec<Signal>,
}

/// An automated confident match proposal.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchProposal {
    pub release_key: crate::types::ItemKey,
    pub release_title: String,
    pub candidate_path: std::path::PathBuf,
    pub candidate_name: String,
    pub score: Score,
    pub format_guess: Option<crate::types::AudioFormat>,
}

/// A candidate scored against a release for review.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredCandidate {
    pub candidate_path: std::path::PathBuf,
    pub candidate_name: String,
    pub score: Score,
    pub format_guess: Option<crate::types::AudioFormat>,
}

/// A release requiring user review among multiple candidate options.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewItem {
    pub release_key: crate::types::ItemKey,
    pub release_title: String,
    pub candidates: Vec<ScoredCandidate>,
}

/// A local album candidate with no matching Bandcamp release.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalOrphan {
    pub path: std::path::PathBuf,
    pub name: String,
    pub track_count: usize,
    pub format_guess: Option<crate::types::AudioFormat>,
}

/// A conflict where multiple candidates or releases collide.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub description: String,
    pub release_keys: Vec<crate::types::ItemKey>,
    pub candidate_paths: Vec<std::path::PathBuf>,
}

/// Complete outcome of assigning candidates to releases.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MatchAssignment {
    /// Auto-confident 1-to-1 matches.
    pub matched: Vec<MatchProposal>,
    /// Ambiguous or below-auto matches for user review.
    pub review: Vec<ReviewItem>,
    /// Releases on artist's page with no corresponding folder.
    pub page_only: Vec<crate::types::ItemKey>,
    /// Folders with no matching release.
    pub local_only: Vec<LocalOrphan>,
    /// Duplicate format or multi-release collisions.
    pub conflicts: Vec<Conflict>,
    /// Previously archived releases whose files reappeared.
    pub restorable: Vec<MatchProposal>,
}

/// An exact-match edge between a local candidate and a Bandcamp release.
#[derive(Debug, Clone)]
pub struct ExactEdge {
    pub cand_idx: usize,
    pub rel_idx: usize,
    pub signals: BTreeSet<Signal>,
}

/// Result of resolving exact matches in the bipartite graph.
pub struct ExactResolution {
    pub matched: Vec<(usize, usize, BTreeSet<Signal>)>,
    pub conflicts: Vec<Conflict>,
    pub matched_cands: HashSet<usize>,
    pub matched_rels: HashSet<usize>,
    pub conflicted_cands: HashSet<usize>,
    pub conflicted_rels: HashSet<usize>,
}

struct DisjointSet {
    parent: Vec<usize>,
}

impl DisjointSet {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, i: usize) -> usize {
        let mut root = i;
        while root != self.parent[root] {
            root = self.parent[root];
        }
        let mut curr = i;
        while curr != root {
            let next = self.parent[curr];
            self.parent[curr] = root;
            curr = next;
        }
        root
    }

    fn union(&mut self, i: usize, j: usize) {
        let ri = self.find(i);
        let rj = self.find(j);
        if ri != rj {
            self.parent[ri] = rj;
        }
    }
}

/// Builds exact-match edges between candidates and releases based on:
/// - Tag URL equals release URL (normalized)
/// - Any folder name variant equals key(release.title) (or raw string if key is empty)
/// - Album tag equals key(release.title) (or raw string if key is empty)
pub fn build_exact_edges(
    candidates: &[AlbumCandidate],
    releases: &[ReleaseRow],
    style: &FolderStyle,
) -> Vec<ExactEdge> {
    let refs: Vec<&AlbumCandidate> = candidates.iter().collect();
    build_exact_edges_refs(&refs, releases, style)
}

/// Builds exact-match edges for candidate references.
pub fn build_exact_edges_refs(
    candidates: &[&AlbumCandidate],
    releases: &[ReleaseRow],
    style: &FolderStyle,
) -> Vec<ExactEdge> {
    let mut edges = Vec::new();
    for (cand_idx, cand) in candidates.iter().enumerate() {
        for (rel_idx, rel) in releases.iter().enumerate() {
            let mut signals = BTreeSet::new();

            // 1. Tag URL equals release URL (normalized host and path)
            if cand
                .metadata
                .as_ref()
                .and_then(|m| m.bandcamp_url.as_deref())
                .is_some_and(|u| normalize_bandcamp_url(u) == normalize_bandcamp_url(&rel.url))
            {
                signals.insert(Signal::UrlTag);
            }

            let rel_title_key = key(&rel.title);

            // 2. Folder name equals key(release.title) (or raw fallback for empty keys)
            let cand_names = style.candidate_names(&cand.name, &rel.artist_name);
            let folder_exact = if !rel_title_key.is_empty() {
                cand_names.iter().any(|n| n == &rel_title_key)
            } else {
                let cand_trimmed = cand.name.trim();
                let rel_trimmed = rel.title.trim();
                !cand_trimmed.is_empty() && cand_trimmed.eq_ignore_ascii_case(rel_trimmed)
            };
            if folder_exact {
                signals.insert(Signal::FolderName);
            }

            // 3. Album tag equals key(release.title) (or raw fallback for empty keys)
            if let Some(album_tag) = cand.metadata.as_ref().and_then(|m| m.album.as_deref()) {
                let tag_key = key(album_tag);
                let album_exact = if !rel_title_key.is_empty() {
                    !tag_key.is_empty() && tag_key == rel_title_key
                } else {
                    let tag_trimmed = album_tag.trim();
                    let rel_trimmed = rel.title.trim();
                    !tag_trimmed.is_empty() && tag_trimmed.eq_ignore_ascii_case(rel_trimmed)
                };
                if album_exact {
                    signals.insert(Signal::AlbumTag);
                }
            }

            if !signals.is_empty() {
                edges.push(ExactEdge {
                    cand_idx,
                    rel_idx,
                    signals,
                });
            }
        }
    }
    edges
}

/// Resolves exact-match edges into 1-to-1 matches and multi-edge conflicts according to §3.4:
/// 1. URL tags first: authoritative when candidate and release both have exactly one UrlTag edge.
/// 2. Connected components over remaining edges: single-edge -> matched, multi-edge -> conflict.
pub fn resolve_exact_matches(
    candidates: &[&AlbumCandidate],
    releases: &[ReleaseRow],
    edges: &[ExactEdge],
) -> ExactResolution {
    let mut matched = Vec::new();
    let mut conflicts = Vec::new();
    let mut matched_cands = HashSet::new();
    let mut matched_rels = HashSet::new();
    let mut conflicted_cands = HashSet::new();
    let mut conflicted_rels = HashSet::new();

    // 1. URL tags first:
    let mut cand_url_counts = vec![0usize; candidates.len()];
    let mut rel_url_counts = vec![0usize; releases.len()];
    for e in edges {
        if e.signals.contains(&Signal::UrlTag) {
            cand_url_counts[e.cand_idx] += 1;
            rel_url_counts[e.rel_idx] += 1;
        }
    }

    for e in edges {
        if e.signals.contains(&Signal::UrlTag)
            && cand_url_counts[e.cand_idx] == 1
            && rel_url_counts[e.rel_idx] == 1
            && matched_cands.insert(e.cand_idx)
            && matched_rels.insert(e.rel_idx)
        {
            matched.push((e.cand_idx, e.rel_idx, e.signals.clone()));
        }
    }

    let mut remaining_edges = Vec::new();
    for e in edges {
        if !matched_cands.contains(&e.cand_idx) && !matched_rels.contains(&e.rel_idx) {
            remaining_edges.push(e.clone());
        }
    }

    // 2. Connected components over remaining edges using DisjointSet
    let total_nodes = candidates.len() + releases.len();
    let mut ds = DisjointSet::new(total_nodes);

    for e in &remaining_edges {
        ds.union(e.cand_idx, candidates.len() + e.rel_idx);
    }

    let mut groups: BTreeMap<usize, Vec<&ExactEdge>> = BTreeMap::new();
    for e in &remaining_edges {
        let root = ds.find(e.cand_idx);
        groups.entry(root).or_default().push(e);
    }

    for (_root, comp_edges) in groups {
        let mut comp_cands = BTreeSet::new();
        let mut comp_rels = BTreeSet::new();
        for e in &comp_edges {
            comp_cands.insert(e.cand_idx);
            comp_rels.insert(e.rel_idx);
        }

        if comp_edges.len() == 1 {
            let e = comp_edges[0];
            matched_cands.insert(e.cand_idx);
            matched_rels.insert(e.rel_idx);
            matched.push((e.cand_idx, e.rel_idx, e.signals.clone()));
        } else if comp_edges.len() > 1 {
            let cands_sorted: Vec<usize> = comp_cands.into_iter().collect();
            let rels_sorted: Vec<usize> = comp_rels.into_iter().collect();

            conflicted_cands.extend(cands_sorted.iter().copied());
            conflicted_rels.extend(rels_sorted.iter().copied());

            let description = if rels_sorted.len() == 1 {
                format!(
                    "{} folders match release '{}' exactly",
                    cands_sorted.len(),
                    releases[rels_sorted[0]].title
                )
            } else if cands_sorted.len() == 1 {
                let cand_name = &candidates[cands_sorted[0]].name;
                if rels_sorted.len() == 2 {
                    let r0 = rels_sorted[0];
                    let r1 = rels_sorted[1];
                    let e0 = comp_edges.iter().find(|e| e.rel_idx == r0);
                    let e1 = comp_edges.iter().find(|e| e.rel_idx == r1);
                    if let (Some(e0), Some(e1)) = (e0, e1) {
                        if e0.signals.contains(&Signal::FolderName)
                            && e1.signals.contains(&Signal::AlbumTag)
                        {
                            format!(
                                "Folder name matches '{}' but album tag matches '{}'",
                                releases[r0].title, releases[r1].title
                            )
                        } else if e1.signals.contains(&Signal::FolderName)
                            && e0.signals.contains(&Signal::AlbumTag)
                        {
                            format!(
                                "Folder name matches '{}' but album tag matches '{}'",
                                releases[r1].title, releases[r0].title
                            )
                        } else {
                            format!(
                                "Folder '{}' matches {} releases exactly",
                                cand_name,
                                rels_sorted.len()
                            )
                        }
                    } else {
                        format!(
                            "Folder '{}' matches {} releases exactly",
                            cand_name,
                            rels_sorted.len()
                        )
                    }
                } else {
                    format!(
                        "Folder '{}' matches {} releases exactly",
                        cand_name,
                        rels_sorted.len()
                    )
                }
            } else {
                format!(
                    "{} folders and {} releases collide exactly",
                    cands_sorted.len(),
                    rels_sorted.len()
                )
            };

            let release_keys: Vec<crate::types::ItemKey> =
                rels_sorted.iter().map(|&ri| releases[ri].key).collect();
            let candidate_paths: Vec<std::path::PathBuf> = cands_sorted
                .iter()
                .map(|&ci| candidates[ci].path.clone())
                .collect();

            conflicts.push(Conflict {
                description,
                release_keys,
                candidate_paths,
            });
        }
    }

    ExactResolution {
        matched,
        conflicts,
        matched_cands,
        matched_rels,
        conflicted_cands,
        conflicted_rels,
    }
}

/// Matches candidates to releases using an exact-first pass followed by fuzzy scoring for leftovers.
pub fn assign_matches(
    candidates: &[AlbumCandidate],
    releases: &[ReleaseRow],
    auto_match_min: f32,
    review_match_min: f32,
    ignored_path_keys: &HashSet<String>,
    style: &FolderStyle,
) -> MatchAssignment {
    let mut assignment = MatchAssignment::default();

    // Filter out candidates ignored by user
    let active_candidates: Vec<&AlbumCandidate> = candidates
        .iter()
        .filter(|c| !ignored_path_keys.contains(&c.path_key))
        .collect();

    if releases.is_empty() {
        for c in active_candidates {
            assignment.local_only.push(LocalOrphan {
                path: c.path.clone(),
                name: c.name.clone(),
                track_count: c.track_count,
                format_guess: c.metadata.as_ref().and_then(|m| m.format_guess),
            });
        }
        return assignment;
    }

    if active_candidates.is_empty() {
        for r in releases {
            assignment.page_only.push(r.key);
        }
        return assignment;
    }

    // 1. Exact Pass
    let exact_edges = build_exact_edges_refs(&active_candidates, releases, style);
    let resolution = resolve_exact_matches(&active_candidates, releases, &exact_edges);

    assignment.conflicts = resolution.conflicts;

    for (cand_idx, rel_idx, signals) in resolution.matched {
        let cand = active_candidates[cand_idx];
        let rel = &releases[rel_idx];
        let proposal = MatchProposal {
            release_key: rel.key,
            release_title: rel.title.clone(),
            candidate_path: cand.path.clone(),
            candidate_name: cand.name.clone(),
            score: Score {
                value: 1.00,
                signals: signals.into_iter().collect(),
            },
            format_guess: cand.metadata.as_ref().and_then(|m| m.format_guess),
        };

        if rel.downloaded && rel.local_status == Some(crate::types::LocalStatus::Archived) {
            assignment.restorable.push(proposal);
        } else {
            assignment.matched.push(proposal);
        }
    }

    let mut matched_cands = resolution.matched_cands;
    let mut matched_rels = resolution.matched_rels;
    let conflicted_cands = resolution.conflicted_cands;
    let conflicted_rels = resolution.conflicted_rels;

    // 2. Fuzzy Pass (only for leftovers)
    let leftover_cands: Vec<usize> = (0..active_candidates.len())
        .filter(|ci| !matched_cands.contains(ci) && !conflicted_cands.contains(ci))
        .collect();
    let leftover_rels: Vec<usize> = (0..releases.len())
        .filter(|ri| !matched_rels.contains(ri) && !conflicted_rels.contains(ri))
        .collect();

    if leftover_cands.is_empty() || leftover_rels.is_empty() {
        for &ri in &leftover_rels {
            assignment.page_only.push(releases[ri].key);
        }
        for &ci in &leftover_cands {
            let cand = active_candidates[ci];
            assignment.local_only.push(LocalOrphan {
                path: cand.path.clone(),
                name: cand.name.clone(),
                track_count: cand.track_count,
                format_guess: cand.metadata.as_ref().and_then(|m| m.format_guess),
            });
        }
        return assignment;
    }

    // Compute pairwise score matrix for leftovers
    let mut matrix: Vec<Vec<(Score, bool)>> = Vec::with_capacity(active_candidates.len());
    for (cand_idx, cand) in active_candidates.iter().enumerate() {
        let mut row = Vec::with_capacity(releases.len());
        if !leftover_cands.contains(&cand_idx) {
            row.resize(
                releases.len(),
                (
                    Score {
                        value: 0.0,
                        signals: Vec::new(),
                    },
                    false,
                ),
            );
        } else {
            for (rel_idx, rel) in releases.iter().enumerate() {
                if !leftover_rels.contains(&rel_idx) {
                    row.push((
                        Score {
                            value: 0.0,
                            signals: Vec::new(),
                        },
                        false,
                    ));
                } else {
                    let release_year = rel.published_at.and_then(|ts| {
                        chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.year() as u32)
                    });
                    row.push(score_candidate_internal(
                        cand,
                        &rel.artist_name,
                        &rel.title,
                        &rel.url,
                        &rel.key.kind,
                        rel.track_count,
                        release_year,
                        style,
                    ));
                }
            }
        }
        matrix.push(row);
    }

    struct Pair {
        cand_idx: usize,
        rel_idx: usize,
        score: Score,
        auto_eligible: bool,
    }

    let mut pairs = Vec::new();
    for &cand_idx in &leftover_cands {
        for &rel_idx in &leftover_rels {
            let (ref s, auto_el) = matrix[cand_idx][rel_idx];
            if s.value >= review_match_min {
                pairs.push(Pair {
                    cand_idx,
                    rel_idx,
                    score: s.clone(),
                    auto_eligible: auto_el,
                });
            }
        }
    }

    pairs.sort_by(|a, b| {
        b.score
            .value
            .partial_cmp(&a.score.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for pair in pairs {
        if matched_cands.contains(&pair.cand_idx) || matched_rels.contains(&pair.rel_idx) {
            continue;
        }

        let cand_runner_up = leftover_rels
            .iter()
            .copied()
            .filter(|&ri| ri != pair.rel_idx)
            .map(|ri| matrix[pair.cand_idx][ri].0.value)
            .fold(0.0f32, f32::max);

        let rel_runner_up = leftover_cands
            .iter()
            .copied()
            .filter(|&ci| ci != pair.cand_idx)
            .map(|ci| matrix[ci][pair.rel_idx].0.value)
            .fold(0.0f32, f32::max);

        let cand = active_candidates[pair.cand_idx];
        let rel = &releases[pair.rel_idx];

        let is_auto = pair.auto_eligible
            && pair.score.value >= auto_match_min
            && (pair.score.value - cand_runner_up) >= 0.05
            && (pair.score.value - rel_runner_up) >= 0.05;

        if is_auto {
            matched_cands.insert(pair.cand_idx);
            matched_rels.insert(pair.rel_idx);

            let proposal = MatchProposal {
                release_key: rel.key,
                release_title: rel.title.clone(),
                candidate_path: cand.path.clone(),
                candidate_name: cand.name.clone(),
                score: pair.score,
                format_guess: cand.metadata.as_ref().and_then(|m| m.format_guess),
            };

            if rel.downloaded && rel.local_status == Some(crate::types::LocalStatus::Archived) {
                assignment.restorable.push(proposal);
            } else {
                assignment.matched.push(proposal);
            }
        }
    }

    // 3. Review items
    for &rel_idx in &leftover_rels {
        if matched_rels.contains(&rel_idx) {
            continue;
        }

        let rel = &releases[rel_idx];
        let mut review_cands = Vec::new();
        for &cand_idx in &leftover_cands {
            if matched_cands.contains(&cand_idx) {
                continue;
            }
            let (ref s, _) = matrix[cand_idx][rel_idx];
            if s.value >= review_match_min {
                let cand = active_candidates[cand_idx];
                review_cands.push(ScoredCandidate {
                    candidate_path: cand.path.clone(),
                    candidate_name: cand.name.clone(),
                    score: s.clone(),
                    format_guess: cand.metadata.as_ref().and_then(|m| m.format_guess),
                });
            }
        }

        if !review_cands.is_empty() {
            review_cands.sort_by(|a, b| {
                b.score
                    .value
                    .partial_cmp(&a.score.value)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            assignment.review.push(ReviewItem {
                release_key: rel.key,
                release_title: rel.title.clone(),
                candidates: review_cands,
            });
            matched_rels.insert(rel_idx);
        }
    }

    // 4. Page only: releases with no match and not in review
    for &rel_idx in &leftover_rels {
        if !matched_rels.contains(&rel_idx) {
            assignment.page_only.push(releases[rel_idx].key);
        }
    }

    // 5. Local only: candidates with no match and not in review
    let review_cand_paths: HashSet<std::path::PathBuf> = assignment
        .review
        .iter()
        .flat_map(|r| r.candidates.iter().map(|c| c.candidate_path.clone()))
        .collect();

    for &cand_idx in &leftover_cands {
        let cand = active_candidates[cand_idx];
        if !matched_cands.contains(&cand_idx) && !review_cand_paths.contains(&cand.path) {
            assignment.local_only.push(LocalOrphan {
                path: cand.path.clone(),
                name: cand.name.clone(),
                track_count: cand.track_count,
                format_guess: cand.metadata.as_ref().and_then(|m| m.format_guess),
            });
        }
    }

    assignment
}

/// Scores a local album candidate against a `ReleaseRow`.
pub fn score_release(
    candidate: &AlbumCandidate,
    release: &ReleaseRow,
    style: &FolderStyle,
) -> Score {
    let release_year = release
        .published_at
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.year() as u32));

    score_candidate(
        candidate,
        &release.artist_name,
        &release.title,
        &release.url,
        &release.key.kind,
        release.track_count,
        release_year,
        style,
    )
}

/// Scores a local candidate against release attributes.
#[allow(clippy::too_many_arguments)]
pub fn score_candidate(
    candidate: &AlbumCandidate,
    artist_name: &str,
    title: &str,
    url: &str,
    item_type: &ItemType,
    track_count: Option<u32>,
    release_year: Option<u32>,
    style: &FolderStyle,
) -> Score {
    score_candidate_internal(
        candidate,
        artist_name,
        title,
        url,
        item_type,
        track_count,
        release_year,
        style,
    )
    .0
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SimResult {
    sim: f32,
    is_contained: bool,
}

fn digit_tokens(s: &str) -> Vec<&str> {
    s.split_whitespace()
        .filter(|tok| tok.chars().any(|c| c.is_ascii_digit()))
        .collect()
}

/// Computes fuzzy similarity between two normalized strings with the digit guard:
/// if both contain numeric tokens and their multisets differ, returns 0.0.
fn fuzzy_sim(a: &str, b: &str) -> SimResult {
    if a.is_empty() || b.is_empty() {
        return SimResult {
            sim: 0.0,
            is_contained: false,
        };
    }
    if a == b {
        return SimResult {
            sim: 1.0,
            is_contained: false,
        };
    }

    let mut da = digit_tokens(a);
    let mut db = digit_tokens(b);
    da.sort_unstable();
    db.sort_unstable();
    if !da.is_empty() && !db.is_empty() && da != db {
        return SimResult {
            sim: 0.0,
            is_contained: false,
        };
    }

    let lev = normalized_levenshtein(a, b) as f32;
    let ta: BTreeSet<&str> = a.split_whitespace().collect();
    let tb: BTreeSet<&str> = b.split_whitespace().collect();
    let inter = ta.intersection(&tb).count() as f32;
    let union = ta.union(&tb).count() as f32;
    let jac = if union > 0.0 { inter / union } else { 0.0 };

    let is_subset = (ta.is_subset(&tb) || tb.is_subset(&ta)) && ta != tb;
    let contained = if is_subset { 0.85 } else { 0.0 };

    let non_contained = lev.max(jac);
    if contained > non_contained {
        SimResult {
            sim: contained.min(0.99),
            is_contained: true,
        }
    } else {
        SimResult {
            sim: non_contained.min(0.99),
            is_contained: false,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn score_candidate_internal(
    candidate: &AlbumCandidate,
    artist_name: &str,
    title: &str,
    url: &str,
    item_type: &ItemType,
    track_count: Option<u32>,
    release_year: Option<u32>,
    style: &FolderStyle,
) -> (Score, bool) {
    let mut signals = BTreeSet::new();

    // 1. Tag URL equals release URL (normalized host and path) -> 1.00, certain
    let tag_url_matches = candidate
        .metadata
        .as_ref()
        .and_then(|m| m.bandcamp_url.as_deref())
        .is_some_and(|u| normalize_bandcamp_url(u) == normalize_bandcamp_url(url));

    if tag_url_matches {
        return (
            Score {
                value: 1.00,
                signals: vec![Signal::UrlTag],
            },
            true,
        );
    }

    let rel_title_key = key(title);
    let cand_folder_names = style.candidate_names(&candidate.name, artist_name);

    // Folder similarity: max over candidate names
    let mut best_folder_sim = SimResult {
        sim: 0.0,
        is_contained: false,
    };
    if !rel_title_key.is_empty() {
        for fn_key in &cand_folder_names {
            let sim = fuzzy_sim(fn_key, &rel_title_key);
            if sim.sim > best_folder_sim.sim {
                best_folder_sim = sim;
            }
        }
    } else {
        let cand_trimmed = candidate.name.trim();
        let rel_trimmed = title.trim();
        if !cand_trimmed.is_empty() && cand_trimmed.eq_ignore_ascii_case(rel_trimmed) {
            best_folder_sim = SimResult {
                sim: 1.0,
                is_contained: false,
            };
        }
    }

    // Album tag similarity
    let album_sim = candidate
        .metadata
        .as_ref()
        .and_then(|m| m.album.as_deref())
        .map(|a| {
            if !rel_title_key.is_empty() {
                fuzzy_sim(&key(a), &rel_title_key)
            } else {
                let tag_trimmed = a.trim();
                let rel_trimmed = title.trim();
                if !tag_trimmed.is_empty() && tag_trimmed.eq_ignore_ascii_case(rel_trimmed) {
                    SimResult {
                        sim: 1.0,
                        is_contained: false,
                    }
                } else {
                    SimResult {
                        sim: 0.0,
                        is_contained: false,
                    }
                }
            }
        });

    if best_folder_sim.sim >= 0.80 {
        signals.insert(Signal::FolderName);
    }
    if album_sim.as_ref().is_some_and(|s| s.sim >= 0.80) {
        signals.insert(Signal::AlbumTag);
    }

    let folder_val = best_folder_sim.sim;
    let album_val = album_sim.as_ref().map(|s| s.sim).unwrap_or(0.0);
    let mut base_score = folder_val.max(album_val);

    let is_contained_winner = if folder_val >= album_val {
        best_folder_sim.is_contained
    } else {
        album_sim.as_ref().is_some_and(|s| s.is_contained)
    };
    let auto_eligible = !is_contained_winner;

    let name_evidence = base_score >= 0.60;

    let expected_tracks = if *item_type == ItemType::Track {
        Some(1)
    } else {
        track_count.map(|c| c as usize)
    };

    if let Some(expected) = expected_tracks.filter(|_| name_evidence) {
        if candidate.track_count == expected {
            base_score += 0.05;
            signals.insert(Signal::TrackCount);
        } else if candidate.track_count.abs_diff(expected) > 2 {
            base_score -= 0.10;
        }
    }

    let cand_year = style
        .extracted_year(&candidate.name, artist_name)
        .or_else(|| candidate.metadata.as_ref().and_then(|m| m.year));

    let year_matches =
        name_evidence && release_year.is_some_and(|ry| cand_year.is_some_and(|cy| ry == cy));
    if year_matches {
        base_score += 0.02;
        signals.insert(Signal::Year);
    }

    (
        Score {
            value: base_score.clamp(0.0, 1.0),
            signals: signals.into_iter().collect(),
        },
        auto_eligible,
    )
}

/// Normalizes a Bandcamp URL by lowercasing, stripping scheme, www, query parameters and trailing slashes.
pub fn normalize_bandcamp_url(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(rest) = s.strip_prefix("https://") {
        s = rest;
    } else if let Some(rest) = s.strip_prefix("http://") {
        s = rest;
    }
    if let Some(rest) = s.strip_prefix("www.") {
        s = rest;
    }
    if let Some((clean, _)) = s.split_once('?') {
        s = clean;
    }
    if let Some((clean, _)) = s.split_once('#') {
        s = clean;
    }
    s.trim_end_matches('/').to_lowercase()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::library::scan::CandidateKind;
    use crate::library::tags::AudioMetadata;
    use crate::types::{Classification, ItemKey};
    use std::path::PathBuf;

    fn make_candidate(
        name: &str,
        track_count: usize,
        album_tag: Option<&str>,
        year_tag: Option<u32>,
        url_tag: Option<&str>,
    ) -> AlbumCandidate {
        AlbumCandidate {
            name: name.to_string(),
            path: PathBuf::from(format!("/music/{}", name)),
            path_key: name.to_lowercase(),
            kind: CandidateKind::Folder,
            track_count,
            metadata: Some(AudioMetadata {
                artist: Some("Aphex Twin".to_string()),
                album: album_tag.map(|s| s.to_string()),
                year: year_tag,
                bandcamp_url: url_tag.map(|s| s.to_string()),
                codec: Some("flac".to_string()),
                bitrate_kbps: None,
                is_vbr: None,
                format_guess: None,
            }),
        }
    }

    fn make_release(
        id: i64,
        title: &str,
        track_count: u32,
        local_status: Option<crate::types::LocalStatus>,
    ) -> ReleaseRow {
        ReleaseRow {
            key: ItemKey {
                kind: ItemType::Album,
                id,
            },
            band_id: 100,
            title: title.to_string(),
            artist_name: "Aphex Twin".to_string(),
            url: format!(
                "https://aphextwin.bandcamp.com/album/{}",
                title.to_lowercase().replace(' ', "-")
            ),
            cover_url: None,
            art_id: None,
            classification: Classification::Free,
            is_preorder: false,
            track_count: Some(track_count),
            published_at: None,
            modified_at: None,
            downloaded: local_status.is_some(),
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
    fn test_01_leiche_regression() {
        for variant in &["tag_is_title", "tag_is_folder", "no_tag"] {
            let mut candidates = Vec::new();
            let mut releases = Vec::new();

            for (idx, i) in (1..=75).step_by(2).enumerate() {
                let j = i + 1;
                let bc_title = format!("Leiche {} / Leiche {}", i, j);
                let folder_name = format!("Leiche {} - Leiche {}", i, j);

                let album_tag = match *variant {
                    "tag_is_title" => Some(bc_title.as_str()),
                    "tag_is_folder" => Some(folder_name.as_str()),
                    "no_tag" => None,
                    _ => unreachable!(),
                };

                let mut cand = make_candidate(&folder_name, 2, album_tag, Some(2020), None);
                if let Some(m) = cand.metadata.as_mut() {
                    m.artist = Some("Leichen".to_string());
                }
                candidates.push(cand);

                releases.push(ReleaseRow {
                    key: ItemKey {
                        kind: ItemType::Album,
                        id: (idx + 1) as i64,
                    },
                    band_id: 10,
                    title: bc_title,
                    artist_name: "Leichen".to_string(),
                    url: format!(
                        "https://leichen.bandcamp.com/album/leiche-{}-leiche-{}",
                        i, j
                    ),
                    cover_url: None,
                    art_id: None,
                    classification: Classification::Free,
                    is_preorder: false,
                    track_count: Some(2),
                    published_at: Some(1577836800), // 2020
                    modified_at: None,
                    downloaded: false,
                    downloaded_at: None,
                    downloaded_format: None,
                    local_path: None,
                    local_status: None,
                    origin: None,
                    missing_ack_at: None,
                    has_remote_update: false,
                    remote_update_reason: None,
                });
            }

            let ignored = HashSet::new();
            let style = FolderStyle::default();
            let res = assign_matches(&candidates, &releases, 0.92, 0.75, &ignored, &style);

            assert!(
                res.conflicts.is_empty(),
                "variant {} produced conflicts: {:?}",
                variant,
                res.conflicts
            );
            assert!(res.review.is_empty(), "variant {} produced review", variant);
            assert!(
                res.local_only.is_empty(),
                "variant {} produced local_only",
                variant
            );
            assert!(
                res.page_only.is_empty(),
                "variant {} produced page_only",
                variant
            );
            assert_eq!(res.matched.len(), 38, "variant {} matched count", variant);
        }
    }

    #[test]
    fn test_02_leiche_missing_release_is_local_only() {
        let mut candidates = Vec::new();
        let mut releases = Vec::new();

        for (idx, i) in (1..=75).step_by(2).enumerate() {
            let j = i + 1;
            let bc_title = format!("Leiche {} / Leiche {}", i, j);
            let folder_name = format!("Leiche {} - Leiche {}", i, j);

            let mut cand = make_candidate(&folder_name, 2, Some(&folder_name), Some(2020), None);
            if let Some(m) = cand.metadata.as_mut() {
                m.artist = Some("Leichen".to_string());
            }
            candidates.push(cand);

            // Skip the last release (Leiche 75 / Leiche 76)
            if i < 75 {
                releases.push(ReleaseRow {
                    key: ItemKey {
                        kind: ItemType::Album,
                        id: (idx + 1) as i64,
                    },
                    band_id: 10,
                    title: bc_title,
                    artist_name: "Leichen".to_string(),
                    url: format!(
                        "https://leichen.bandcamp.com/album/leiche-{}-leiche-{}",
                        i, j
                    ),
                    cover_url: None,
                    art_id: None,
                    classification: Classification::Free,
                    is_preorder: false,
                    track_count: Some(2),
                    published_at: Some(1577836800),
                    modified_at: None,
                    downloaded: false,
                    downloaded_at: None,
                    downloaded_format: None,
                    local_path: None,
                    local_status: None,
                    origin: None,
                    missing_ack_at: None,
                    has_remote_update: false,
                    remote_update_reason: None,
                });
            }
        }

        let ignored = HashSet::new();
        let style = FolderStyle::default();
        let res = assign_matches(&candidates, &releases, 0.92, 0.75, &ignored, &style);

        assert!(res.conflicts.is_empty());
        assert!(res.review.is_empty());
        assert_eq!(res.matched.len(), 37);
        assert_eq!(res.local_only.len(), 1);
        assert_eq!(res.local_only[0].name, "Leiche 75 - Leiche 76");
    }

    #[test]
    fn test_03_leiche_missing_folder_is_page_only() {
        let mut candidates = Vec::new();
        let mut releases = Vec::new();

        for (idx, i) in (1..=75).step_by(2).enumerate() {
            let j = i + 1;
            let bc_title = format!("Leiche {} / Leiche {}", i, j);
            let folder_name = format!("Leiche {} - Leiche {}", i, j);

            // Skip folder for Leiche 75 - Leiche 76
            if i < 75 {
                let mut cand =
                    make_candidate(&folder_name, 2, Some(&folder_name), Some(2020), None);
                if let Some(m) = cand.metadata.as_mut() {
                    m.artist = Some("Leichen".to_string());
                }
                candidates.push(cand);
            }

            releases.push(ReleaseRow {
                key: ItemKey {
                    kind: ItemType::Album,
                    id: (idx + 1) as i64,
                },
                band_id: 10,
                title: bc_title,
                artist_name: "Leichen".to_string(),
                url: format!(
                    "https://leichen.bandcamp.com/album/leiche-{}-leiche-{}",
                    i, j
                ),
                cover_url: None,
                art_id: None,
                classification: Classification::Free,
                is_preorder: false,
                track_count: Some(2),
                published_at: Some(1577836800),
                modified_at: None,
                downloaded: false,
                downloaded_at: None,
                downloaded_format: None,
                local_path: None,
                local_status: None,
                origin: None,
                missing_ack_at: None,
                has_remote_update: false,
                remote_update_reason: None,
            });
        }

        let ignored = HashSet::new();
        let style = FolderStyle::default();
        let res = assign_matches(&candidates, &releases, 0.92, 0.75, &ignored, &style);

        assert!(res.conflicts.is_empty());
        assert!(res.review.is_empty());
        assert_eq!(res.matched.len(), 37);
        assert_eq!(res.page_only.len(), 1);
        assert_eq!(res.page_only[0].id, 38);
    }

    #[test]
    fn test_04_title_punctuation_and_separators_exact_match() {
        let cand = make_candidate("Foo - Bar_Baz", 5, None, None, None);
        let rel = make_release(1, "Foo / Bar: Baz?", 5, None);

        let res = assign_matches(
            &[cand],
            &[rel],
            0.92,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert_eq!(res.matched.len(), 1);
        assert_eq!(res.matched[0].release_key.id, 1);
        assert!(res.matched[0].score.signals.contains(&Signal::FolderName));
    }

    #[test]
    fn test_05_title_and_artist_folder_style_both_exact() {
        let style = FolderStyle {
            templates: vec!["{artist} - {title}".to_string(), "{title}".to_string()],
            noise: Vec::new(),
        };

        let cand1 = make_candidate("Artist - Foo - Bar", 5, None, None, None);
        let cand2 = make_candidate("Foo - Bar", 5, None, None, None);

        let mut rel1 = make_release(1, "Foo - Bar", 5, None);
        rel1.artist_name = "Artist".to_string();

        let res1 = assign_matches(
            &[cand1],
            &[rel1.clone()],
            0.92,
            0.75,
            &HashSet::new(),
            &style,
        );
        assert_eq!(res1.matched.len(), 1);

        let res2 = assign_matches(&[cand2], &[rel1], 0.92, 0.75, &HashSet::new(), &style);
        assert_eq!(res2.matched.len(), 1);
    }

    #[test]
    fn test_06_folder_style_artist_does_not_fit_raw_folder_still_matches() {
        let style = FolderStyle {
            templates: vec!["{artist} - {title}".to_string()],
            noise: Vec::new(),
        };

        let mut cand = make_candidate("Leiche 75 - Leiche 76", 2, None, None, None);
        cand.metadata.as_mut().unwrap().artist = Some("Leichen".to_string());

        let mut rel = make_release(1, "Leiche 75 / Leiche 76", 2, None);
        rel.artist_name = "Leichen".to_string();

        let res = assign_matches(&[cand], &[rel], 0.92, 0.75, &HashSet::new(), &style);
        assert_eq!(res.matched.len(), 1);
        assert!(res.matched[0].score.signals.contains(&Signal::FolderName));
    }

    #[test]
    fn test_07_default_style_album_with_year_fuzzy_containment_review() {
        let cand = make_candidate("Album (2019)", 8, None, None, None);
        let mut rel = make_release(1, "Album", 8, None);
        rel.track_count = None;

        let res = assign_matches(
            &[cand],
            &[rel],
            0.92,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert!(res.matched.is_empty(), "containment must not auto-adopt");
        assert_eq!(res.review.len(), 1);
        assert_eq!(res.review[0].candidates.len(), 1);
        assert_eq!(res.review[0].candidates[0].score.value, 0.85);
    }

    #[test]
    fn test_08_style_title_year_exact_match_and_year_extracted() {
        let style = FolderStyle {
            templates: vec!["{title} ({year})".to_string()],
            noise: Vec::new(),
        };

        let cand = make_candidate("Album [2019]", 8, None, None, None);
        let mut rel = make_release(1, "Album", 8, None);
        rel.published_at = Some(1577836800); // 2020, but exact pass doesn't check year!

        let res = assign_matches(&[cand], &[rel], 0.92, 0.75, &HashSet::new(), &style);
        assert_eq!(res.matched.len(), 1);
        assert!(res.matched[0].score.signals.contains(&Signal::FolderName));
    }

    #[test]
    fn test_09_accents_and_special_letters_exact_match() {
        let cases = [
            ("Sigur Ros", "Sigur Rós"),
            ("Cafe", "Caf\u{0065}\u{0301}"), // NFD Cafe
            ("Bjork", "Björk"),
            ("Strasse", "Straße"),
            ("Ornen", "Ørnen"),
            ("Lodz", "Łódź"),
        ];

        for (folder, title) in cases {
            let cand = make_candidate(folder, 5, None, None, None);
            let rel = make_release(1, title, 5, None);

            let res = assign_matches(
                &[cand],
                &[rel],
                0.92,
                0.75,
                &HashSet::new(),
                &FolderStyle::default(),
            );
            assert_eq!(res.matched.len(), 1, "failed for {} vs {}", folder, title);
        }
    }

    #[test]
    fn test_10_flac_and_mp3_duplicate_exact_folders_produce_one_conflict() {
        let style = FolderStyle {
            templates: vec!["{title} ({noise})".to_string()],
            noise: vec!["FLAC".to_string(), "MP3".to_string()],
        };

        let c_flac = make_candidate("Selected Ambient Works [FLAC]", 13, None, None, None);
        let c_mp3 = make_candidate("Selected Ambient Works [MP3]", 13, None, None, None);
        let c_other = make_candidate("Drukqs", 30, None, None, None);

        let r1 = make_release(1, "Selected Ambient Works", 13, None);
        let r2 = make_release(2, "Drukqs", 30, None);

        let res = assign_matches(
            &[c_flac, c_mp3, c_other],
            &[r1, r2],
            0.92,
            0.75,
            &HashSet::new(),
            &style,
        );
        assert_eq!(res.matched.len(), 1);
        assert_eq!(res.matched[0].release_key.id, 2);
        assert_eq!(res.conflicts.len(), 1);
        assert_eq!(res.conflicts[0].candidate_paths.len(), 2);
        assert_eq!(res.conflicts[0].release_keys.len(), 1);
        assert_eq!(res.conflicts[0].release_keys[0].id, 1);
        assert!(
            res.conflicts[0]
                .description
                .contains("2 folders match release")
        );
    }

    #[test]
    fn test_11_folder_exact_for_album_and_single_conflict_or_url_tag_resolves() {
        let cand = make_candidate("Avril 14th", 1, None, None, None);
        let r_album = make_release(1, "Avril 14th", 1, None);
        let mut r_single = make_release(2, "Avril 14th", 1, None);
        r_single.key.kind = ItemType::Track;
        r_single.url = "https://aphextwin.bandcamp.com/track/avril-14th".to_string();

        // Without URL tag -> conflict
        let res_no_url = assign_matches(
            std::slice::from_ref(&cand),
            &[r_album.clone(), r_single.clone()],
            0.92,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert_eq!(res_no_url.matched.len(), 0);
        assert_eq!(res_no_url.conflicts.len(), 1);
        assert!(
            res_no_url.conflicts[0]
                .description
                .contains("matches 2 releases exactly")
        );

        // With URL tag pointing to single -> resolves to single!
        let mut cand_with_url = cand;
        cand_with_url.metadata.as_mut().unwrap().bandcamp_url = Some(r_single.url.clone());

        let res_url = assign_matches(
            &[cand_with_url],
            &[r_album, r_single],
            0.92,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert_eq!(res_url.conflicts.len(), 0);
        assert_eq!(res_url.matched.len(), 1);
        assert_eq!(res_url.matched[0].release_key.id, 2);
        assert_eq!(res_url.page_only.len(), 1);
        assert_eq!(res_url.page_only[0].id, 1);
    }

    #[test]
    fn test_12_folder_name_matches_a_album_tag_matches_b_disagreement_conflict() {
        let cand = make_candidate("Album A", 10, Some("Album B"), None, None);
        let r_a = make_release(1, "Album A", 10, None);
        let r_b = make_release(2, "Album B", 10, None);

        let res = assign_matches(
            &[cand],
            &[r_a, r_b],
            0.92,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert_eq!(res.matched.len(), 0);
        assert_eq!(res.conflicts.len(), 1);
        assert!(
            res.conflicts[0]
                .description
                .contains("Folder name matches 'Album A' but album tag matches 'Album B'")
        );
        assert_eq!(res.conflicts[0].release_keys.len(), 2);
    }

    #[test]
    fn test_13_exact_folder_name_junk_album_tag_matches() {
        let cand = make_candidate("Album A", 10, Some("Random Nonexistent Album"), None, None);
        let r_a = make_release(1, "Album A", 10, None);

        let res = assign_matches(
            &[cand],
            &[r_a],
            0.92,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert_eq!(res.matched.len(), 1);
        assert_eq!(res.matched[0].release_key.id, 1);
        assert!(res.conflicts.is_empty());
    }

    #[test]
    fn test_14_symbol_only_titles_match_raw_fallback_never_cross() {
        let c1 = make_candidate("!!!", 5, None, None, None);
        let c2 = make_candidate("△", 5, None, None, None);

        let r1 = make_release(1, "!!!", 5, None);
        let r2 = make_release(2, "△", 5, None);

        let res = assign_matches(
            &[c1, c2],
            &[r1, r2],
            0.92,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert_eq!(res.matched.len(), 2);
        assert_eq!(res.conflicts.len(), 0);
        for m in &res.matched {
            if m.candidate_name == "!!!" {
                assert_eq!(m.release_key.id, 1);
            } else if m.candidate_name == "△" {
                assert_eq!(m.release_key.id, 2);
            } else {
                panic!("unexpected match");
            }
        }
    }

    #[test]
    fn test_15_fuzzy_series_numbers_mismatch_not_a_candidate() {
        let cand1 = make_candidate("Volume 1", 5, None, None, None);
        let rel1 = make_release(1, "Volume 2", 5, None);

        let res1 = assign_matches(
            &[cand1],
            &[rel1],
            0.92,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert!(res1.matched.is_empty());
        assert!(res1.review.is_empty());
        assert_eq!(res1.local_only.len(), 1);
        assert_eq!(res1.page_only.len(), 1);

        let cand2 = make_candidate("Part 1", 5, None, None, None);
        let rel2 = make_release(2, "Part 2", 5, None);

        let res2 = assign_matches(
            &[cand2],
            &[rel2],
            0.92,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert!(res2.matched.is_empty());
        assert!(res2.review.is_empty());
        assert_eq!(res2.local_only.len(), 1);
        assert_eq!(res2.page_only.len(), 1);
    }

    #[test]
    fn test_16_album_vs_album_deluxe_containment_never_auto_even_low_slider() {
        let cand = make_candidate("Album", 10, None, Some(2020), None);
        let mut rel = make_release(1, "Album Deluxe", 10, None);
        rel.published_at = Some(1577836800); // 2020

        // Auto slider lowered to 0.80
        let res = assign_matches(
            &[cand],
            &[rel],
            0.80,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert!(
            res.matched.is_empty(),
            "containment must never be auto-adopted"
        );
        assert_eq!(res.review.len(), 1);
        assert_eq!(res.review[0].release_key.id, 1);
    }

    #[test]
    fn test_17_fuzzy_near_tie_produces_review_no_conflict() {
        let c1 = make_candidate("Live at the Fillmore West", 10, None, None, None);
        let c2 = make_candidate("Live at the Fillmore East", 10, None, None, None);
        let rel = make_release(1, "Live at the Fillmore", 10, None);

        let res = assign_matches(
            &[c1, c2],
            &[rel],
            0.92,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert!(
            res.conflicts.is_empty(),
            "fuzzy near ties must not produce conflicts"
        );
        assert_eq!(res.matched.len(), 0);
        assert_eq!(res.review.len(), 1);
        assert_eq!(res.review[0].candidates.len(), 2);
    }

    #[test]
    fn test_18_archived_downloaded_exact_match_goes_to_restorable() {
        let c = make_candidate("Selected Ambient Works", 13, None, None, None);
        let r = make_release(
            1,
            "Selected Ambient Works",
            13,
            Some(crate::types::LocalStatus::Archived),
        );

        let res = assign_matches(
            &[c],
            &[r],
            0.92,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert_eq!(res.matched.len(), 0);
        assert_eq!(res.restorable.len(), 1);
        assert_eq!(res.restorable[0].release_key.id, 1);
    }

    #[test]
    fn test_19_ignored_path_keys_skipped() {
        let c = make_candidate("Selected Ambient Works", 13, None, None, None);
        let r = make_release(1, "Selected Ambient Works", 13, None);

        let mut ignored = HashSet::new();
        ignored.insert(c.path_key.clone());

        let res = assign_matches(&[c], &[r], 0.92, 0.75, &ignored, &FolderStyle::default());
        assert_eq!(res.matched.len(), 0);
        assert_eq!(res.page_only.len(), 1);
        assert_eq!(res.local_only.len(), 0);
    }

    #[test]
    fn test_20_zip_candidate_stem_exact_match() {
        let mut cand = make_candidate("Selected Ambient Works 85-92", 13, None, None, None);
        cand.kind = CandidateKind::Zip;

        let rel = make_release(1, "Selected Ambient Works 85-92", 13, None);

        let res = assign_matches(
            &[cand],
            &[rel],
            0.92,
            0.75,
            &HashSet::new(),
            &FolderStyle::default(),
        );
        assert_eq!(res.matched.len(), 1);
        assert_eq!(res.matched[0].release_key.id, 1);
    }

    #[test]
    fn test_32_mixed_flac_mp3_ogg_folder_style_noise_and_fuzzy_fallback() {
        let style = FolderStyle {
            templates: vec!["{title} ({noise})".to_string()],
            noise: vec!["FLAC".to_string(), "MP3".to_string()],
        };

        let c_flac = make_candidate("Album One [FLAC]", 10, None, None, None);
        let c_mp3 = make_candidate("Album Two [MP3]", 10, None, None, None);
        let c_ogg = make_candidate("Album Three [OGG]", 10, None, None, None);

        let r1 = make_release(1, "Album One", 10, None);
        let r2 = make_release(2, "Album Two", 10, None);
        let mut r3 = make_release(3, "Album Three", 10, None);
        r3.track_count = None;

        let res = assign_matches(
            &[c_flac, c_mp3, c_ogg],
            &[r1, r2, r3],
            0.92,
            0.75,
            &HashSet::new(),
            &style,
        );

        assert_eq!(res.matched.len(), 2);
        assert!(res.matched.iter().any(|m| m.release_key.id == 1));
        assert!(res.matched.iter().any(|m| m.release_key.id == 2));

        // Album Three [OGG] did not fit the template (OGG not in noise list),
        // so it fell back to fuzzy containment: in review at 0.85
        assert_eq!(res.review.len(), 1);
        assert_eq!(res.review[0].release_key.id, 3);
        assert_eq!(res.review[0].candidates.len(), 1);
        assert_eq!(res.review[0].candidates[0].score.value, 0.85);
    }

    #[test]
    fn url_tag_match_yields_certain_score_1() {
        let cand = make_candidate(
            "Some Random Folder Name",
            10,
            None,
            None,
            Some("https://aphextwin.bandcamp.com/album/selected-ambient-works-85-92?from=search"),
        );

        let score = score_candidate(
            &cand,
            "Aphex Twin",
            "Selected Ambient Works 85-92",
            "http://aphextwin.bandcamp.com/album/selected-ambient-works-85-92/",
            &ItemType::Album,
            Some(10),
            Some(1992),
            &FolderStyle::default(),
        );

        assert_eq!(score.value, 1.00);
        assert_eq!(score.signals, vec![Signal::UrlTag]);
    }

    #[test]
    fn exact_folder_name_and_track_count_scores_high() {
        let cand = make_candidate(
            "Aphex Twin - Selected Ambient Works 85-92",
            13,
            Some("Selected Ambient Works 85-92"),
            Some(1992),
            None,
        );

        let style = FolderStyle {
            templates: vec!["{artist} - {title}".to_string()],
            noise: Vec::new(),
        };

        let score = score_candidate(
            &cand,
            "Aphex Twin",
            "Selected Ambient Works 85-92",
            "https://aphextwin.bandcamp.com/album/selected-ambient-works-85-92",
            &ItemType::Album,
            Some(13),
            Some(1992),
            &style,
        );

        assert_eq!(score.value, 1.00);
        assert!(score.signals.contains(&Signal::FolderName));
        assert!(score.signals.contains(&Signal::AlbumTag));
        assert!(score.signals.contains(&Signal::TrackCount));
        assert!(score.signals.contains(&Signal::Year));
    }

    #[test]
    fn shared_artist_prefix_singles_match_one_to_one_without_conflicts() {
        let titles = [
            "Une nouvelle étoile",
            "Crevée",
            "L'heure du choix",
            "Sous les décombres",
            "Quand la danseuse tourne",
            "Le son des corps agonisants",
            "Born Again",
            "Split w/ Gvillotina",
            "Hurler sur les sourds",
            "La mue d'une danseuse",
        ];
        let candidates: Vec<AlbumCandidate> = titles
            .iter()
            .map(|t| {
                let folder = format!("Deaf Ballerina - {}", t.replace('/', "-"));
                let mut cand = make_candidate(&folder, 1, Some(t), Some(2017), None);
                cand.metadata.as_mut().unwrap().artist = Some("Deaf Ballerina".to_string());
                cand
            })
            .collect();
        let releases: Vec<ReleaseRow> = titles
            .iter()
            .enumerate()
            .map(|(i, t)| ReleaseRow {
                key: ItemKey {
                    kind: ItemType::Album,
                    id: i as i64 + 1,
                },
                band_id: 1,
                title: t.to_string(),
                artist_name: "Deaf Ballerina".to_string(),
                url: format!("https://deafballerina.bandcamp.com/album/a{i}"),
                cover_url: None,
                art_id: None,
                classification: Classification::Free,
                is_preorder: false,
                track_count: Some(1),
                published_at: None,
                modified_at: None,
                downloaded: false,
                downloaded_at: None,
                downloaded_format: None,
                local_path: None,
                local_status: None,
                origin: None,
                missing_ack_at: None,
                has_remote_update: false,
                remote_update_reason: None,
            })
            .collect();

        let style = FolderStyle {
            templates: vec!["{artist} - {title}".to_string(), "{title}".to_string()],
            noise: Vec::new(),
        };

        let res = assign_matches(&candidates, &releases, 0.92, 0.75, &HashSet::new(), &style);
        assert!(res.conflicts.is_empty(), "{:?}", res.conflicts);
        assert!(res.page_only.is_empty());
        assert!(res.review.is_empty());
        assert_eq!(res.matched.len(), titles.len());
        for m in &res.matched {
            let rel = releases.iter().find(|r| r.key == m.release_key).unwrap();
            assert!(m.candidate_name.ends_with(&rel.title.replace('/', "-")));
        }
    }

    #[test]
    fn track_count_penalty_for_large_mismatch() {
        let cand = make_candidate(
            "Selected Ambient Works",
            5,
            Some("Selected Ambient Works"),
            None,
            None,
        );
        let score = score_candidate(
            &cand,
            "Aphex Twin",
            "Selected Ambient Works",
            "https://aphextwin.bandcamp.com/album/saw",
            &ItemType::Album,
            Some(13), // 5 vs 13 is diff > 2 tracks: -0.10 penalty
            None,
            &FolderStyle::default(),
        );

        assert_eq!(score.value, 0.90);
    }

    #[test]
    fn normalizes_bandcamp_urls() {
        assert_eq!(
            normalize_bandcamp_url("https://artist.bandcamp.com/album/xyz/"),
            "artist.bandcamp.com/album/xyz"
        );
        assert_eq!(
            normalize_bandcamp_url("http://www.artist.bandcamp.com/album/xyz?ref=test#section"),
            "artist.bandcamp.com/album/xyz"
        );
    }

    #[test]
    fn assigns_auto_page_only_and_local_only() {
        let c1 = make_candidate(
            "Aphex Twin - Selected Ambient Works",
            13,
            Some("Selected Ambient Works"),
            None,
            None,
        );
        let c2 = make_candidate("Unrelated Bootleg", 5, None, None, None);

        let r1 = make_release(1, "Selected Ambient Works", 13, None);
        let r2 = make_release(2, "Drukqs", 30, None);

        let ignored = HashSet::new();
        let res = assign_matches(
            &[c1, c2],
            &[r1, r2],
            0.85,
            0.70,
            &ignored,
            &FolderStyle::default(),
        );

        assert_eq!(res.matched.len(), 1);
        assert_eq!(res.matched[0].release_key.id, 1);

        assert_eq!(res.page_only.len(), 1);
        assert_eq!(res.page_only[0].id, 2);

        assert_eq!(res.local_only.len(), 1);
        assert_eq!(res.local_only[0].name, "Unrelated Bootleg");
    }
}

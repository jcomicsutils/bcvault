use std::collections::BTreeSet;

use chrono::Datelike;
use strsim::normalized_levenshtein;

use crate::library::scan::AlbumCandidate;
use crate::library::textmatch::{name_variant_keys, normalize};
use crate::types::{ItemType, ReleaseRow};

/// Known version tokens that distinguish special editions from standard releases.
const VERSION_WORDS: &[&str] = &[
    "deluxe",
    "remaster",
    "remastered",
    "edition",
    "live",
    "anniversary",
    "expanded",
    "reissue",
];

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

/// Greedy 1-to-1 candidate assignment according to §6 rules:
/// - A pair is auto if score >= auto_match_min and it beats runner-up for both sides by >= 0.05
/// - Review if score >= review_match_min or ambiguous
/// - Conflicts if duplicate local copies (e.g. FLAC and MP3) or multiple releases match same folder
/// - Otherwise page_only or local_only
pub fn assign_matches(
    candidates: &[AlbumCandidate],
    releases: &[ReleaseRow],
    auto_match_min: f32,
    review_match_min: f32,
    ignored_path_keys: &std::collections::HashSet<String>,
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

    // Compute pairwise score matrix
    // matrix[cand_idx][rel_idx] = Score
    let mut matrix: Vec<Vec<Score>> = Vec::with_capacity(active_candidates.len());
    for c in &active_candidates {
        let mut row = Vec::with_capacity(releases.len());
        for r in releases {
            row.push(score_release(c, r));
        }
        matrix.push(row);
    }

    // 1. Detect conflicts:
    // Check if multiple candidates score >= review_match_min on the same release with close scores (difference < 0.05)
    let mut conflicted_rel_indices = std::collections::HashSet::new();
    let mut conflicted_cand_indices = std::collections::HashSet::new();

    for (rel_idx, r) in releases.iter().enumerate() {
        let mut high_scoring_cands = Vec::new();
        for (cand_idx, _) in active_candidates.iter().enumerate() {
            let score_val = matrix[cand_idx][rel_idx].value;
            if score_val >= review_match_min {
                high_scoring_cands.push((cand_idx, score_val));
            }
        }

        if high_scoring_cands.len() > 1 {
            high_scoring_cands
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            // If top two are close (< 0.05 difference), it's a conflict (e.g. FLAC and MP3 duplicates)
            // A conflict needs two strong, near-equal candidates (e.g. FLAC and MP3
            // copies). A weak runner-up is handled by greedy assignment and review.
            if high_scoring_cands[1].1 >= auto_match_min
                && (high_scoring_cands[0].1 - high_scoring_cands[1].1) < 0.05
            {
                conflicted_rel_indices.insert(rel_idx);
                let paths: Vec<std::path::PathBuf> = high_scoring_cands
                    .iter()
                    .map(|(ci, _)| {
                        conflicted_cand_indices.insert(*ci);
                        active_candidates[*ci].path.clone()
                    })
                    .collect();

                assignment.conflicts.push(Conflict {
                    description: format!(
                        "Multiple local folders match release '{}' (duplicate formats or identical titles)",
                        r.title
                    ),
                    release_keys: vec![r.key],
                    candidate_paths: paths,
                });
            }
        }
    }

    // Check if one candidate matches multiple releases with close scores
    for (cand_idx, c) in active_candidates.iter().enumerate() {
        if conflicted_cand_indices.contains(&cand_idx) {
            continue;
        }
        let mut high_scoring_rels = Vec::new();
        for (rel_idx, _) in releases.iter().enumerate() {
            let score_val = matrix[cand_idx][rel_idx].value;
            if score_val >= review_match_min {
                high_scoring_rels.push((rel_idx, score_val));
            }
        }

        if high_scoring_rels.len() > 1 {
            high_scoring_rels
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            if high_scoring_rels[1].1 >= auto_match_min
                && (high_scoring_rels[0].1 - high_scoring_rels[1].1) < 0.05
            {
                conflicted_cand_indices.insert(cand_idx);
                let rel_keys: Vec<crate::types::ItemKey> = high_scoring_rels
                    .iter()
                    .map(|(ri, _)| {
                        conflicted_rel_indices.insert(*ri);
                        releases[*ri].key
                    })
                    .collect();

                assignment.conflicts.push(Conflict {
                    description: format!(
                        "Folder '{}' matches multiple releases with equal confidence",
                        c.name
                    ),
                    release_keys: rel_keys,
                    candidate_paths: vec![c.path.clone()],
                });
            }
        }
    }

    // 2. Greedy assignment of non-conflicted pairs
    // Collect all valid pairs (cand_idx, rel_idx, score)
    struct Pair {
        cand_idx: usize,
        rel_idx: usize,
        score: Score,
    }

    let mut pairs = Vec::new();
    for (cand_idx, _) in active_candidates.iter().enumerate() {
        if conflicted_cand_indices.contains(&cand_idx) {
            continue;
        }
        for (rel_idx, _) in releases.iter().enumerate() {
            if conflicted_rel_indices.contains(&rel_idx) {
                continue;
            }
            let s = matrix[cand_idx][rel_idx].clone();
            if s.value >= review_match_min {
                pairs.push(Pair {
                    cand_idx,
                    rel_idx,
                    score: s,
                });
            }
        }
    }

    // Sort descending by score value
    pairs.sort_by(|a, b| {
        b.score
            .value
            .partial_cmp(&a.score.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut assigned_cands = std::collections::HashSet::new();
    let mut assigned_rels = std::collections::HashSet::new();

    for pair in pairs {
        if assigned_cands.contains(&pair.cand_idx) || assigned_rels.contains(&pair.rel_idx) {
            continue;
        }

        // Runner up check for cand_idx
        let cand_runner_up = (0..releases.len())
            .filter(|&ri| ri != pair.rel_idx && !conflicted_rel_indices.contains(&ri))
            .map(|ri| matrix[pair.cand_idx][ri].value)
            .fold(0.0f32, f32::max);

        // Runner up check for rel_idx
        let rel_runner_up = (0..active_candidates.len())
            .filter(|&ci| ci != pair.cand_idx && !conflicted_cand_indices.contains(&ci))
            .map(|ci| matrix[ci][pair.rel_idx].value)
            .fold(0.0f32, f32::max);

        let cand = active_candidates[pair.cand_idx];
        let rel = &releases[pair.rel_idx];

        let is_auto = pair.score.value >= auto_match_min
            && (pair.score.value - cand_runner_up) >= 0.05
            && (pair.score.value - rel_runner_up) >= 0.05;

        if is_auto {
            assigned_cands.insert(pair.cand_idx);
            assigned_rels.insert(pair.rel_idx);

            let proposal = MatchProposal {
                release_key: rel.key,
                release_title: rel.title.clone(),
                candidate_path: cand.path.clone(),
                candidate_name: cand.name.clone(),
                score: pair.score,
                format_guess: cand.metadata.as_ref().and_then(|m| m.format_guess),
            };

            // If the release was already downloaded and is marked archived, it is restorable
            if rel.downloaded && rel.local_status == Some(crate::types::LocalStatus::Archived) {
                assignment.restorable.push(proposal);
            } else {
                assignment.matched.push(proposal);
            }
        }
    }

    // 3. Review items:
    // Any unassigned release that has candidates with score >= review_match_min
    for (rel_idx, rel) in releases.iter().enumerate() {
        if assigned_rels.contains(&rel_idx) || conflicted_rel_indices.contains(&rel_idx) {
            continue;
        }

        let mut review_cands = Vec::new();
        for (cand_idx, cand) in active_candidates.iter().enumerate() {
            if assigned_cands.contains(&cand_idx) || conflicted_cand_indices.contains(&cand_idx) {
                continue;
            }
            let s = &matrix[cand_idx][rel_idx];
            if s.value >= review_match_min {
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
            assigned_rels.insert(rel_idx);
        }
    }

    // 4. Page only: releases with no match and not in review/conflict
    for (rel_idx, rel) in releases.iter().enumerate() {
        if !assigned_rels.contains(&rel_idx) && !conflicted_rel_indices.contains(&rel_idx) {
            assignment.page_only.push(rel.key);
        }
    }

    // 5. Local only: candidates with no match and not in review/conflict
    let review_cand_paths: std::collections::HashSet<std::path::PathBuf> = assignment
        .review
        .iter()
        .flat_map(|r| r.candidates.iter().map(|c| c.candidate_path.clone()))
        .collect();

    for (cand_idx, cand) in active_candidates.iter().enumerate() {
        if !assigned_cands.contains(&cand_idx)
            && !conflicted_cand_indices.contains(&cand_idx)
            && !review_cand_paths.contains(&cand.path)
        {
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
pub fn score_release(candidate: &AlbumCandidate, release: &ReleaseRow) -> Score {
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
    )
}

/// Scores a local candidate against release attributes according to §6 rules:
/// - URL tag match: score 1.00 (certain)
/// - Tag album ≈ title: Jaro-Winkler on normalized string
/// - Folder/zip name matches name_variants: 0.95 (exact), else fuzzy similarity
/// - Track count match: +0.05; mismatch > 2 tracks: -0.10
/// - Year match: +0.02
/// - Version word in only one side (e.g. deluxe, live, remastered): capped at 0.80
pub fn score_candidate(
    candidate: &AlbumCandidate,
    artist_name: &str,
    title: &str,
    url: &str,
    item_type: &ItemType,
    track_count: Option<u32>,
    release_year: Option<u32>,
) -> Score {
    let mut signals = BTreeSet::new();

    // 1. Tag URL equals the release URL (normalised host and path) -> score 1.00, certain
    let tag_url_matches = candidate
        .metadata
        .as_ref()
        .and_then(|m| m.bandcamp_url.as_deref())
        .is_some_and(|u| normalize_bandcamp_url(u) == normalize_bandcamp_url(url));

    if tag_url_matches {
        return Score {
            value: 1.00,
            signals: vec![Signal::UrlTag],
        };
    }

    let norm_title = normalize(title);
    let norm_folder = normalize(&candidate.name);
    let variant_keys = name_variant_keys(artist_name, title);

    // 2. Folder name. Folders are usually "{artist} - {title}"; comparing the
    // whole string lets the shared artist prefix dominate any fuzzy metric, so
    // the title part after the last " - " is compared on its own as well.
    // Fuzzy similarity uses normalized Levenshtein, which (unlike Jaro-Winkler)
    // does not reward a common prefix.
    let folder_title = candidate
        .name
        .rsplit_once(" - ")
        .map(|(_, t)| normalize(t))
        .unwrap_or_else(|| norm_folder.clone());

    let folder_exact = variant_keys.contains(&norm_folder) || folder_title == norm_title;
    let folder_sim = if folder_exact {
        1.0
    } else {
        title_similarity(&folder_title, &norm_title)
    };
    if folder_sim >= 0.80 {
        signals.insert(Signal::FolderName);
    }

    // 3. Album tag against title.
    let album_sim = candidate
        .metadata
        .as_ref()
        .and_then(|m| m.album.as_deref())
        .map(|a| title_similarity(&normalize(a), &norm_title));
    let album_exact = album_sim.is_some_and(|s| s >= 1.0);
    if album_sim.is_some_and(|s| s >= 0.80) {
        signals.insert(Signal::AlbumTag);
    }

    let mut base_score = match (folder_exact, album_exact) {
        (true, true) => 1.0,
        (false, true) => 0.97,
        (true, false) => 0.95,
        (false, false) => folder_sim.max(album_sim.unwrap_or(0.0)),
    };
    // Two independent signals disagreeing strongly (album tag names another
    // release) lowers confidence in a folder-name-only match.
    if folder_exact && album_sim.is_some_and(|s| s < 0.5) {
        base_score = base_score.min(0.80);
    }

    // Track count and year only refine a name-based match; on their own they
    // say nothing (every single has one track, many releases share a year).
    let name_evidence = base_score >= 0.60;

    // 4. Track count signal
    // For single tracks: audio_count == 1 is treated as match
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

    // 5. Release year match
    let year_matches = name_evidence
        && release_year.is_some_and(|ry| {
            candidate
                .metadata
                .as_ref()
                .and_then(|m| m.year)
                .is_some_and(|ty| ry == ty)
        });
    if year_matches {
        base_score += 0.02;
        signals.insert(Signal::Year);
    }

    // 6. Version word mismatch check:
    // If a version word exists on only one side (e.g. "deluxe", "remastered"), cap at 0.80
    let candidate_text = format!(
        "{} {}",
        candidate.name,
        candidate
            .metadata
            .as_ref()
            .and_then(|m| m.album.as_deref())
            .unwrap_or("")
    );

    if has_version_word_mismatch(&candidate_text, title) {
        base_score = base_score.min(0.80);
    }

    Score {
        value: base_score.clamp(0.0, 1.0),
        signals: signals.into_iter().collect(),
    }
}

/// Similarity of two normalized titles in `[0, 1]`: the better of character-level
/// normalized Levenshtein and token-set overlap (handles reordered words).
fn title_similarity(a: &str, b: &str) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let lev = normalized_levenshtein(a, b) as f32;
    let ta: BTreeSet<&str> = a.split_whitespace().collect();
    let tb: BTreeSet<&str> = b.split_whitespace().collect();
    let inter = ta.intersection(&tb).count() as f32;
    let union = ta.union(&tb).count() as f32;
    let jac = if union > 0.0 { inter / union } else { 0.0 };
    // One title fully contained in the other ("Album" vs "Album (Deluxe)") is a
    // likely but not certain match: high enough for review, below auto.
    let contained = if ta.is_subset(&tb) || tb.is_subset(&ta) {
        0.85
    } else {
        0.0
    };
    lev.max(jac).max(contained).min(0.99)
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

/// Detects if version words exist on one side of a match but not on the other.
fn has_version_word_mismatch(local_text: &str, remote_text: &str) -> bool {
    let local_v = extract_version_words(local_text);
    let remote_v = extract_version_words(remote_text);

    local_v.iter().any(|w| !remote_v.contains(w)) || remote_v.iter().any(|w| !local_v.contains(w))
}

fn extract_version_words(text: &str) -> Vec<&'static str> {
    let norm = text.to_lowercase();
    let words: Vec<&str> = norm.split(|c: char| !c.is_alphanumeric()).collect();
    let mut found = Vec::new();
    for v in VERSION_WORDS {
        if words.contains(v) {
            found.push(*v);
        }
    }
    found
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::library::scan::CandidateKind;
    use crate::library::tags::AudioMetadata;
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

        let score = score_candidate(
            &cand,
            "Aphex Twin",
            "Selected Ambient Works 85-92",
            "https://aphextwin.bandcamp.com/album/selected-ambient-works-85-92",
            &ItemType::Album,
            Some(13),
            Some(1992),
        );

        // 0.95 (folder) + 0.05 (track count) + 0.02 (year) clamped to 1.0
        assert_eq!(score.value, 1.00);
        assert!(score.signals.contains(&Signal::FolderName));
        assert!(score.signals.contains(&Signal::AlbumTag));
        assert!(score.signals.contains(&Signal::TrackCount));
        assert!(score.signals.contains(&Signal::Year));
    }

    #[test]
    fn shared_artist_prefix_singles_match_one_to_one_without_conflicts() {
        use crate::types::{Classification, ItemKey};
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
                make_candidate(&folder, 1, Some(t), Some(2017), None)
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

        let res = assign_matches(
            &candidates,
            &releases,
            0.92,
            0.75,
            &std::collections::HashSet::new(),
        );
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
    fn version_word_mismatch_caps_score_at_80() {
        let cand = make_candidate(
            "Selected Ambient Works 85-92 (Deluxe Edition)",
            15,
            Some("Selected Ambient Works 85-92 (Deluxe Edition)"),
            Some(2021),
            None,
        );

        let score = score_candidate(
            &cand,
            "Aphex Twin",
            "Selected Ambient Works 85-92",
            "https://aphextwin.bandcamp.com/album/selected-ambient-works-85-92",
            &ItemType::Album,
            Some(13),
            Some(1992),
        );

        assert!(score.value <= 0.80);
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

    fn make_release(
        id: i64,
        title: &str,
        track_count: u32,
        local_status: Option<crate::types::LocalStatus>,
    ) -> ReleaseRow {
        ReleaseRow {
            key: crate::types::ItemKey {
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
            classification: crate::types::Classification::Free,
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

        let ignored = std::collections::HashSet::new();
        let res = assign_matches(&[c1, c2], &[r1, r2], 0.85, 0.70, &ignored);

        assert_eq!(res.matched.len(), 1);
        assert_eq!(res.matched[0].release_key.id, 1);

        assert_eq!(res.page_only.len(), 1);
        assert_eq!(res.page_only[0].id, 2);

        assert_eq!(res.local_only.len(), 1);
        assert_eq!(res.local_only[0].name, "Unrelated Bootleg");
    }

    #[test]
    fn duplicate_formats_detected_as_conflict() {
        let c_flac = make_candidate(
            "Selected Ambient Works [FLAC]",
            13,
            Some("Selected Ambient Works"),
            None,
            None,
        );
        let c_mp3 = make_candidate(
            "Selected Ambient Works [MP3]",
            13,
            Some("Selected Ambient Works"),
            None,
            None,
        );

        let r = make_release(1, "Selected Ambient Works", 13, None);

        let ignored = std::collections::HashSet::new();
        let res = assign_matches(&[c_flac, c_mp3], &[r], 0.85, 0.70, &ignored);

        assert_eq!(res.matched.len(), 0);
        assert_eq!(res.conflicts.len(), 1);
        assert_eq!(res.conflicts[0].candidate_paths.len(), 2);
    }

    #[test]
    fn archived_release_matched_goes_to_restorable() {
        let c = make_candidate(
            "Selected Ambient Works",
            13,
            Some("Selected Ambient Works"),
            None,
            None,
        );
        let r = make_release(
            1,
            "Selected Ambient Works",
            13,
            Some(crate::types::LocalStatus::Archived),
        );

        let ignored = std::collections::HashSet::new();
        let res = assign_matches(&[c], &[r], 0.85, 0.70, &ignored);

        assert_eq!(res.matched.len(), 0);
        assert_eq!(res.restorable.len(), 1);
        assert_eq!(res.restorable[0].release_key.id, 1);
    }
}

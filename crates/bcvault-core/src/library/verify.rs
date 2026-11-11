use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::library::folder_style::FolderStyle;
use crate::library::matcher::{build_exact_edges_refs, resolve_exact_matches, score_release};
use crate::library::scan::{AlbumCandidate, is_audio_file};
use crate::types::{
    ArtistFolder, AudioFormat, FolderState, ItemKey, LocalStatus, MissingCandidate, Origin,
    ReleaseRow, UnavailableFolder, UnavailableRelease, UnavailableRoot,
};

/// Verification status for a downloaded release on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyResult {
    /// Local files exist and contain valid audio (or non-empty zip).
    Present,
    /// Local files are absent or contain no audio files.
    Missing,
    /// Root or artist folder is inaccessible (drive unmounted or unreadable); absence not confirmed.
    Unknown,
}

/// Aggregated report of a verification pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyReport {
    pub present: Vec<ItemKey>,
    pub missing: Vec<ItemKey>,
    pub unknown: Vec<ItemKey>,
    /// Set true when >= 20 releases and > 30% of checked releases are missing in one pass.
    pub mass_missing: bool,
}

/// Checks whether a local path on disk contains audio files or is a non-empty audio archive.
/// For directories, searches direct children and one subfolder level (depth 2).
/// Folders containing only metadata or covers (e.g. `cover.jpg`) return false.
pub fn check_disk_release_path(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }

    if path.is_file() {
        return path.metadata().is_ok_and(|m| m.len() > 0);
    }

    if !path.is_dir() {
        return false;
    }

    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };

    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && is_audio_file(&p) {
            return true;
        }
        if p.is_dir()
            && let Ok(sub_entries) = std::fs::read_dir(&p)
        {
            for sub in sub_entries.flatten() {
                let sub_p = sub.path();
                if sub_p.is_file() && is_audio_file(&sub_p) {
                    return true;
                }
            }
        }
    }

    false
}

/// Verifies a single release against the disk and folder health state.
/// Returns `None` if the release should be skipped (e.g. not downloaded or unverified without a path).
pub fn verify_single_release(
    rel: &ReleaseRow,
    is_folder_probe_ok: bool,
    is_sync: bool,
) -> Option<VerifyResult> {
    if !rel.downloaded {
        return None;
    }

    // Unavailable folder safeguard: if folder is missing or unreadable, mark Unknown, never Missing.
    if !is_folder_probe_ok {
        return Some(VerifyResult::Unknown);
    }

    let Some(path) = &rel.local_path else {
        // Rows unverified with local_path IS NULL and origin = 'imported' are skipped
        if rel.local_status == Some(LocalStatus::Unverified) && rel.origin == Some(Origin::Imported)
        {
            return None;
        }
        if rel.local_status == Some(LocalStatus::Archived) {
            return None;
        }
        return Some(VerifyResult::Missing);
    };

    // Archived rows are re-checked only for restore detection (if files reappeared)
    if rel.local_status == Some(LocalStatus::Archived) {
        if check_disk_release_path(path) {
            return Some(VerifyResult::Present);
        }
        return None;
    }

    if check_disk_release_path(path) {
        Some(VerifyResult::Present)
    } else {
        // Skip is not re-asked on sync if already acknowledged (missing_ack_at is set),
        // but is re-asked on manual VerifyFiles (is_sync == false).
        if is_sync && rel.local_status == Some(LocalStatus::Missing) && rel.missing_ack_at.is_some()
        {
            return None;
        }
        Some(VerifyResult::Missing)
    }
}

/// Computes the mass-missing guard: true if >= 20 releases and > 30% of checked releases are missing.
pub fn is_mass_missing(missing_count: usize, total_checked: usize) -> bool {
    if total_checked == 0 || missing_count < 20 {
        return false;
    }
    (missing_count as f32 / total_checked as f32) > 0.30
}

/// Determines the root folder and health state for a release during verification.
pub fn attribute_release_root(
    rel: &ReleaseRow,
    linked_folders: &[ArtistFolder],
    out_dir: &Path,
    out_dir_state: FolderState,
) -> (UnavailableRoot, PathBuf, FolderState) {
    if let Some(local_p) = &rel.local_path {
        if let Some(folder) = crate::db::repo::library::owning_folder(linked_folders, local_p) {
            return (
                UnavailableRoot::Linked {
                    folder_id: folder.id,
                },
                folder.path.clone(),
                folder.state,
            );
        }
    } else if let Some(folder) = linked_folders.first() {
        return (
            UnavailableRoot::Linked {
                folder_id: folder.id,
            },
            folder.path.clone(),
            folder.state,
        );
    }
    (
        UnavailableRoot::OutDir,
        out_dir.to_path_buf(),
        out_dir_state,
    )
}

/// Groups `Unknown` releases by their unavailable root (linked folder or out_dir).
/// Drops releases whose `local_status` is already `Archived`.
pub fn group_unavailable_releases(
    items: Vec<(ReleaseRow, UnavailableRoot, PathBuf, FolderState)>,
) -> Vec<UnavailableFolder> {
    let mut groups: Vec<(
        UnavailableRoot,
        PathBuf,
        FolderState,
        Vec<UnavailableRelease>,
    )> = Vec::new();

    for (rel, root, path, state) in items {
        if rel.local_status == Some(LocalStatus::Archived) {
            continue;
        }
        if state == FolderState::Ok {
            continue;
        }

        let release_info = UnavailableRelease {
            key: rel.key,
            band_id: rel.band_id,
            artist_name: rel.artist_name,
            title: rel.title,
            last_known_path: rel.local_path,
        };

        if let Some(existing) = groups
            .iter_mut()
            .find(|(r, p, _, _)| *r == root && *p == path)
        {
            existing.3.push(release_info);
        } else {
            groups.push((root, path, state, vec![release_info]));
        }
    }

    groups
        .into_iter()
        .map(|(root, path, state, releases)| UnavailableFolder {
            root,
            path,
            state,
            releases,
        })
        .collect()
}

/// Relink proposal for a missing release that confidently matches a local folder.
#[derive(Debug, Clone, PartialEq)]
pub struct RelinkProposal {
    pub key: ItemKey,
    pub path: PathBuf,
    pub score: f32,
    pub format: Option<AudioFormat>,
}

/// Outcome of searching for relink candidates for missing releases.
#[derive(Debug, Clone, PartialEq)]
pub struct RelinkOutcome {
    /// Confident, unique 1-to-1 relink proposals.
    pub relinked: Vec<RelinkProposal>,
    /// Releases that could not be auto-relinked, along with their best candidate (if any).
    pub unresolved: Vec<(ReleaseRow, Option<MissingCandidate>)>,
}

/// Evaluates candidates in an artist's linked folder to find confident, unique relink proposals
/// for missing releases.
///
/// Rules:
/// - Candidates already linked to other releases (or ignored) are excluded.
/// - A candidate qualifies if `score >= auto_match_min` and beats runner-up by >= 0.05.
/// - If two or more identical/near-identical candidates compete for a release, it is NOT auto-relinked (asks instead).
/// - If two releases compete for the same candidate, neither is auto-relinked.
pub fn find_relink_candidates(
    missing_releases: &[ReleaseRow],
    candidates: &[AlbumCandidate],
    excluded_path_keys: &HashSet<String>,
    auto_match_min: f32,
    style: &FolderStyle,
) -> RelinkOutcome {
    find_relink_candidates_with_rejected(
        missing_releases,
        candidates,
        excluded_path_keys,
        &HashSet::new(),
        auto_match_min,
        style,
    )
}

pub fn find_relink_candidates_with_rejected(
    missing_releases: &[ReleaseRow],
    candidates: &[AlbumCandidate],
    excluded_path_keys: &HashSet<String>,
    rejected_pairs: &HashSet<(ItemKey, String)>,
    auto_match_min: f32,
    style: &FolderStyle,
) -> RelinkOutcome {
    let available_candidates: Vec<&AlbumCandidate> = candidates
        .iter()
        .filter(|c| !excluded_path_keys.contains(&c.path_key))
        .collect();

    if available_candidates.is_empty() {
        return RelinkOutcome {
            relinked: Vec::new(),
            unresolved: missing_releases
                .iter()
                .cloned()
                .map(|r| (r, None))
                .collect(),
        };
    }

    // 1. Exact pass
    let exact_edges: Vec<_> = build_exact_edges_refs(&available_candidates, missing_releases, style)
        .into_iter()
        .filter(|e| {
            let rel_key = missing_releases[e.rel_idx].key;
            let cand_pk = &available_candidates[e.cand_idx].path_key;
            !rejected_pairs.contains(&(rel_key, cand_pk.clone()))
        })
        .collect();
    let resolution = resolve_exact_matches(&available_candidates, missing_releases, &exact_edges);

    let mut relinked = Vec::new();
    let mut unresolved_releases: Vec<(&ReleaseRow, Option<MissingCandidate>)> = Vec::new();

    for (cand_idx, rel_idx, _) in resolution.matched {
        let cand = available_candidates[cand_idx];
        let rel = &missing_releases[rel_idx];
        relinked.push(RelinkProposal {
            key: rel.key,
            path: cand.path.clone(),
            score: 1.0,
            format: cand.metadata.as_ref().and_then(|m| m.format_guess),
        });
    }

    // Releases involved in exact conflicts are unresolved (asks instead), with best candidate attached
    for conflict in &resolution.conflicts {
        for &rel_key in &conflict.release_keys {
            if let Some(rel) = missing_releases.iter().find(|r| r.key == rel_key) {
                let best_cand = conflict.candidate_paths.first().map(|p| MissingCandidate {
                    path: p.clone(),
                    score: 1.0,
                });
                unresolved_releases.push((rel, best_cand));
            }
        }
    }

    let matched_cands = resolution.matched_cands;
    let matched_rels = resolution.matched_rels;
    let conflicted_cands = resolution.conflicted_cands;
    let conflicted_rels = resolution.conflicted_rels;

    // 2. Fuzzy pass for leftover releases and leftover candidates
    let leftover_cands: Vec<&AlbumCandidate> = available_candidates
        .iter()
        .enumerate()
        .filter(|(ci, _)| !matched_cands.contains(ci) && !conflicted_cands.contains(ci))
        .map(|(_, &c)| c)
        .collect();

    let leftover_rels: Vec<&ReleaseRow> = missing_releases
        .iter()
        .enumerate()
        .filter(|(ri, _)| !matched_rels.contains(ri) && !conflicted_rels.contains(ri))
        .map(|(_, r)| r)
        .collect();

    if leftover_rels.is_empty() {
        return RelinkOutcome {
            relinked,
            unresolved: unresolved_releases
                .into_iter()
                .map(|(r, opt)| (r.clone(), opt))
                .collect(),
        };
    }

    struct Scored<'a> {
        candidate: &'a AlbumCandidate,
        score: f32,
    }

    let mut per_release_scores: Vec<(&ReleaseRow, Vec<Scored>)> = Vec::new();

    for rel in leftover_rels {
        let mut scored = Vec::new();
        for &cand in &leftover_cands {
            if rejected_pairs.contains(&(rel.key, cand.path_key.clone())) {
                continue;
            }
            let s = score_release(cand, rel, style);
            scored.push(Scored {
                candidate: cand,
                score: s.value,
            });
        }
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        per_release_scores.push((rel, scored));
    }

    let mut potential_matches: Vec<(&ReleaseRow, &AlbumCandidate, f32)> = Vec::new();

    for (rel, scored) in per_release_scores {
        if scored.is_empty() {
            unresolved_releases.push((rel, None));
            continue;
        }

        let best = &scored[0];
        let best_candidate_info = if best.score > 0.0 {
            Some(MissingCandidate {
                path: best.candidate.path.clone(),
                score: best.score,
            })
        } else {
            None
        };

        if best.score >= auto_match_min {
            let beats_runner_up = if scored.len() > 1 {
                (best.score - scored[1].score) >= 0.05
            } else {
                true
            };

            if beats_runner_up {
                potential_matches.push((rel, best.candidate, best.score));
            } else {
                unresolved_releases.push((rel, best_candidate_info));
            }
        } else {
            unresolved_releases.push((rel, best_candidate_info));
        }
    }

    // Ensure 1-to-1 uniqueness
    let mut candidate_claim_count: HashMap<PathBuf, usize> = HashMap::new();
    for (_, cand, _) in &potential_matches {
        *candidate_claim_count.entry(cand.path.clone()).or_insert(0) += 1;
    }

    for (rel, cand, score) in potential_matches {
        if candidate_claim_count.get(&cand.path).copied().unwrap_or(0) == 1 {
            relinked.push(RelinkProposal {
                key: rel.key,
                path: cand.path.clone(),
                score,
                format: cand.metadata.as_ref().and_then(|m| m.format_guess),
            });
        } else {
            unresolved_releases.push((
                rel,
                Some(MissingCandidate {
                    path: cand.path.clone(),
                    score,
                }),
            ));
        }
    }

    RelinkOutcome {
        relinked,
        unresolved: unresolved_releases
            .into_iter()
            .map(|(r, opt)| (r.clone(), opt))
            .collect(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    use crate::types::{AudioFormat, ItemKey, ItemType, LocalStatus, Origin};

    fn make_test_row(
        key: ItemKey,
        downloaded: bool,
        path: Option<std::path::PathBuf>,
    ) -> ReleaseRow {
        ReleaseRow {
            key,
            band_id: 1,
            title: "Test Album".to_string(),
            artist_name: "Artist".to_string(),
            url: "https://example.com".to_string(),
            cover_url: None,
            art_id: None,
            classification: crate::types::Classification::Free,
            is_preorder: false,
            track_count: Some(10),
            published_at: None,
            modified_at: None,
            downloaded,
            downloaded_at: None,
            downloaded_format: Some(AudioFormat::Flac),
            local_path: path,
            local_status: if downloaded {
                Some(LocalStatus::Present)
            } else {
                None
            },
            origin: Some(Origin::App),
            missing_ack_at: None,
            has_remote_update: false,
            remote_update_reason: None,
        }
    }

    #[test]
    fn check_disk_path_detects_files_and_directories() {
        let dir = tempdir().unwrap();

        // Nonexistent
        let bogus = dir.path().join("missing");
        assert!(!check_disk_release_path(&bogus));

        // Zip file empty vs with content
        let empty_zip = dir.path().join("empty.zip");
        fs::write(&empty_zip, b"").unwrap();
        assert!(!check_disk_release_path(&empty_zip));

        let non_empty_zip = dir.path().join("full.zip");
        fs::write(&non_empty_zip, b"zipcontent").unwrap();
        assert!(check_disk_release_path(&non_empty_zip));

        // Directory with only cover.jpg
        let cover_only = dir.path().join("CoverOnly");
        fs::create_dir(&cover_only).unwrap();
        fs::write(cover_only.join("cover.jpg"), b"image").unwrap();
        assert!(!check_disk_release_path(&cover_only));

        // Directory with audio directly
        let with_audio = dir.path().join("WithAudio");
        fs::create_dir(&with_audio).unwrap();
        fs::write(with_audio.join("track.flac"), b"audio").unwrap();
        assert!(check_disk_release_path(&with_audio));

        // Directory with audio in subfolder (multi-disc)
        let disc_dir = dir.path().join("MultiDisc");
        let disc1 = disc_dir.join("CD1");
        fs::create_dir_all(&disc1).unwrap();
        fs::write(disc1.join("01.mp3"), b"audio").unwrap();
        assert!(check_disk_release_path(&disc_dir));
    }

    #[test]
    fn verify_single_release_safeguards_and_unverified() {
        let dir = tempdir().unwrap();
        let key = ItemKey::new(ItemType::Album, 100);

        let album_dir = dir.path().join("Album");
        fs::create_dir(&album_dir).unwrap();
        fs::write(album_dir.join("01.mp3"), b"audio").unwrap();

        let row = make_test_row(key, true, Some(album_dir));

        // 1. Folder probe is Ok -> Present
        assert_eq!(
            verify_single_release(&row, true, false),
            Some(VerifyResult::Present)
        );

        // 2. Folder probe is NOT Ok (unavailable drive) -> Unknown, NEVER Missing
        assert_eq!(
            verify_single_release(&row, false, false),
            Some(VerifyResult::Unknown)
        );

        // 3. File removed, probe Ok -> Missing
        let missing_row = make_test_row(key, true, Some(dir.path().join("deleted")));
        assert_eq!(
            verify_single_release(&missing_row, true, false),
            Some(VerifyResult::Missing)
        );

        // 4. File removed, probe NOT Ok -> Unknown
        assert_eq!(
            verify_single_release(&missing_row, false, false),
            Some(VerifyResult::Unknown)
        );

        // 5. Unverified row with no local_path is skipped
        let mut unverified_row = make_test_row(key, true, None);
        unverified_row.local_status = Some(LocalStatus::Unverified);
        unverified_row.origin = Some(Origin::Imported);
        assert_eq!(verify_single_release(&unverified_row, true, false), None);
    }

    #[test]
    fn skip_not_reasked_on_sync_but_reasked_on_manual_verify() {
        let key = ItemKey::new(ItemType::Album, 10);
        let mut row = make_test_row(key, true, Some(PathBuf::from("/music/deleted")));
        row.local_status = Some(LocalStatus::Missing);
        row.missing_ack_at = Some(1_700_000_000);

        // During sync: skipped -> None
        assert_eq!(verify_single_release(&row, true, true), None);

        // During manual verify: re-asked -> Some(VerifyResult::Missing)
        assert_eq!(
            verify_single_release(&row, true, false),
            Some(VerifyResult::Missing)
        );
    }

    #[test]
    fn mass_missing_guard_thresholds() {
        // Less than 20 missing -> false even if 100%
        assert!(!is_mass_missing(19, 19));
        // 20 missing out of 100 (20%) -> false
        assert!(!is_mass_missing(20, 100));
        // 20 missing out of 50 (40%) -> true
        assert!(is_mass_missing(20, 50));
        // 31 missing out of 100 (31%) -> true
        assert!(is_mass_missing(31, 100));
    }

    fn make_test_candidate(name: &str, path: PathBuf, track_count: usize) -> AlbumCandidate {
        AlbumCandidate {
            name: name.to_string(),
            path_key: name.to_lowercase(),
            path,
            kind: crate::library::scan::CandidateKind::Folder,
            track_count,
            metadata: None,
        }
    }

    #[test]
    fn find_relink_single_candidate_auto_relinks() {
        let key = ItemKey::new(ItemType::Album, 10);
        let rel = make_test_row(key, true, Some(PathBuf::from("/music/old_path")));
        let cand = make_test_candidate("Test Album", PathBuf::from("/music/new_path"), 10);

        let outcome = find_relink_candidates(
            &[rel],
            &[cand],
            &HashSet::new(),
            0.90,
            &FolderStyle::default(),
        );
        assert_eq!(outcome.relinked.len(), 1);
        assert_eq!(outcome.relinked[0].key, key);
        assert_eq!(outcome.relinked[0].path, PathBuf::from("/music/new_path"));
        assert!(outcome.unresolved.is_empty());
    }

    #[test]
    fn find_relink_two_identical_candidates_asks_instead() {
        let key = ItemKey::new(ItemType::Album, 10);
        let rel = make_test_row(key, true, Some(PathBuf::from("/music/old_path")));
        let cand1 = make_test_candidate("Test Album", PathBuf::from("/music/path_a"), 10);
        let mut cand2 = make_test_candidate("Test Album", PathBuf::from("/music/path_b"), 10);
        cand2.path_key = "test_album_b".to_string();

        let outcome = find_relink_candidates(
            &[rel],
            &[cand1, cand2],
            &HashSet::new(),
            0.90,
            &FolderStyle::default(),
        );
        assert!(outcome.relinked.is_empty());
        assert_eq!(outcome.unresolved.len(), 1);
        assert!(outcome.unresolved[0].1.is_some());
    }

    #[test]
    fn find_relink_below_threshold_asks_instead() {
        let key = ItemKey::new(ItemType::Album, 10);
        let rel = make_test_row(key, true, Some(PathBuf::from("/music/old_path")));
        let cand = make_test_candidate("Completely Different", PathBuf::from("/music/diff"), 1);

        let outcome = find_relink_candidates(
            &[rel],
            &[cand],
            &HashSet::new(),
            0.90,
            &FolderStyle::default(),
        );
        assert!(outcome.relinked.is_empty());
        assert_eq!(outcome.unresolved.len(), 1);
    }

    #[test]
    fn find_relink_already_linked_candidate_ignored() {
        let key = ItemKey::new(ItemType::Album, 10);
        let rel = make_test_row(key, true, Some(PathBuf::from("/music/old_path")));
        let cand = make_test_candidate("Test Album", PathBuf::from("/music/new_path"), 10);

        let mut excluded = HashSet::new();
        excluded.insert("test album".to_string());

        let outcome =
            find_relink_candidates(&[rel], &[cand], &excluded, 0.90, &FolderStyle::default());
        assert!(outcome.relinked.is_empty());
        assert_eq!(outcome.unresolved.len(), 1);
        assert_eq!(outcome.unresolved[0].1, None);
    }

    #[test]
    fn find_relink_competing_releases_collide() {
        let key1 = ItemKey::new(ItemType::Album, 10);
        let key2 = ItemKey::new(ItemType::Album, 20);
        let rel1 = make_test_row(key1, true, Some(PathBuf::from("/music/old1")));
        let rel2 = make_test_row(key2, true, Some(PathBuf::from("/music/old2")));
        let cand = make_test_candidate("Test Album", PathBuf::from("/music/cand"), 10);

        let outcome = find_relink_candidates(
            &[rel1, rel2],
            &[cand],
            &HashSet::new(),
            0.90,
            &FolderStyle::default(),
        );
        assert!(outcome.relinked.is_empty());
        assert_eq!(outcome.unresolved.len(), 2);
    }

    #[test]
    fn test_attribute_release_root_under_link_and_outside() {
        let band_id = 42;
        let link_path = PathBuf::from("/music/Artist");
        let linked = ArtistFolder {
            id: 1,
            band_id,
            path: link_path.clone(),
            state: FolderState::Missing,
        };
        let out_dir = PathBuf::from("/downloads");

        // 1. Release under linked folder
        let key1 = ItemKey::new(ItemType::Album, 1);
        let rel1 = make_test_row(key1, true, Some(link_path.join("Album 1")));
        let (root1, p1, st1) = attribute_release_root(
            &rel1,
            std::slice::from_ref(&linked),
            &out_dir,
            FolderState::Ok,
        );
        assert_eq!(
            root1,
            UnavailableRoot::Linked {
                folder_id: linked.id
            }
        );
        assert_eq!(p1, link_path);
        assert_eq!(st1, FolderState::Missing);

        // 2. Release outside linked folder (e.g. in out_dir)
        let key2 = ItemKey::new(ItemType::Album, 2);
        let rel2 = make_test_row(key2, true, Some(out_dir.join("Album 2")));
        let (root2, p2, st2) = attribute_release_root(
            &rel2,
            std::slice::from_ref(&linked),
            &out_dir,
            FolderState::Ok,
        );
        assert_eq!(root2, UnavailableRoot::OutDir);
        assert_eq!(p2, out_dir);
        assert_eq!(st2, FolderState::Ok);

        // 3. Release without path but artist has link
        let key3 = ItemKey::new(ItemType::Album, 3);
        let rel3 = make_test_row(key3, true, None);
        let (root3, p3, st3) = attribute_release_root(
            &rel3,
            std::slice::from_ref(&linked),
            &out_dir,
            FolderState::Ok,
        );
        assert_eq!(
            root3,
            UnavailableRoot::Linked {
                folder_id: linked.id
            }
        );
        assert_eq!(p3, link_path);
        assert_eq!(st3, FolderState::Missing);

        // 4. Release without path and no link
        let (root4, p4, st4) =
            attribute_release_root(&rel3, &[], &out_dir, FolderState::Unreadable);
        assert_eq!(root4, UnavailableRoot::OutDir);
        assert_eq!(p4, out_dir);
        assert_eq!(st4, FolderState::Unreadable);
    }

    #[test]
    fn test_group_unavailable_releases_pure() {
        let key1 = ItemKey::new(ItemType::Album, 1);
        let mut rel1 = make_test_row(key1, true, Some(PathBuf::from("/music/Artist/Album 1")));
        rel1.title = "Album 1".to_string();
        rel1.artist_name = "Artist".to_string();

        let key2 = ItemKey::new(ItemType::Album, 2);
        let mut rel2 = make_test_row(key2, true, Some(PathBuf::from("/music/Artist/Album 2")));
        rel2.title = "Album 2".to_string();
        rel2.artist_name = "Artist".to_string();

        // Archived release: should be dropped
        let key3 = ItemKey::new(ItemType::Album, 3);
        let mut rel3 = make_test_row(key3, true, Some(PathBuf::from("/music/Artist/Album 3")));
        rel3.local_status = Some(LocalStatus::Archived);

        let key4 = ItemKey::new(ItemType::Album, 4);
        let mut rel4 = make_test_row(key4, true, Some(PathBuf::from("/downloads/Other")));
        rel4.title = "Other".to_string();
        rel4.artist_name = "Other Artist".to_string();

        let root_linked = UnavailableRoot::Linked { folder_id: 10 };
        let root_out = UnavailableRoot::OutDir;

        let items = vec![
            (
                rel1,
                root_linked,
                PathBuf::from("/music/Artist"),
                FolderState::Missing,
            ),
            (
                rel2,
                root_linked,
                PathBuf::from("/music/Artist"),
                FolderState::Missing,
            ),
            (
                rel3,
                root_linked,
                PathBuf::from("/music/Artist"),
                FolderState::Missing,
            ),
            (
                rel4,
                root_out,
                PathBuf::from("/downloads"),
                FolderState::Unreadable,
            ),
        ];

        let grouped = group_unavailable_releases(items);
        assert_eq!(grouped.len(), 2);

        // Group 1: Linked Missing with 2 releases (archived excluded)
        assert_eq!(grouped[0].root, root_linked);
        assert_eq!(grouped[0].state, FolderState::Missing);
        assert_eq!(grouped[0].releases.len(), 2);
        assert_eq!(grouped[0].releases[0].key, key1);
        assert_eq!(grouped[0].releases[1].key, key2);

        // Group 2: OutDir Unreadable with 1 release
        assert_eq!(grouped[1].root, root_out);
        assert_eq!(grouped[1].state, FolderState::Unreadable);
        assert_eq!(grouped[1].releases.len(), 1);
        assert_eq!(grouped[1].releases[0].key, key4);
    }
}

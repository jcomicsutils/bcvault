use std::collections::BTreeSet;

use chrono::Datelike;
use strsim::jaro_winkler;

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

    let mut base_score = 0.0f32;

    // 2. Folder name match against name variants:
    // If exact variant match -> base 0.95, else fuzzy Jaro-Winkler
    if variant_keys.contains(&norm_folder) {
        base_score = 0.95;
        signals.insert(Signal::FolderName);
    } else {
        let best_folder_sim = variant_keys
            .iter()
            .map(|k| jaro_winkler(&norm_folder, k) as f32)
            .fold(0.0f32, f32::max);

        if best_folder_sim >= 0.80 {
            signals.insert(Signal::FolderName);
        }
        base_score = base_score.max(best_folder_sim);
    }

    // 3. Tag album similarity against title
    if let Some(tag_album) = candidate.metadata.as_ref().and_then(|m| m.album.as_deref()) {
        let norm_tag_album = normalize(tag_album);
        let album_sim = jaro_winkler(&norm_tag_album, &norm_title) as f32;

        if album_sim >= 0.80 {
            signals.insert(Signal::AlbumTag);
        }
        base_score = base_score.max(album_sim);
    }

    // 4. Track count signal
    // For single tracks: audio_count == 1 is treated as match
    let expected_tracks = if *item_type == ItemType::Track {
        Some(1)
    } else {
        track_count.map(|c| c as usize)
    };

    if let Some(expected) = expected_tracks {
        if candidate.track_count == expected {
            base_score += 0.05;
            signals.insert(Signal::TrackCount);
        } else if candidate.track_count.abs_diff(expected) > 2 {
            base_score -= 0.10;
        }
    }

    // 5. Release year match
    let year_matches = release_year.is_some_and(|ry| {
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
}

pub mod folder_style;
pub mod link;
pub mod matcher;
pub mod scan;
pub mod service;
pub mod tags;
pub mod textmatch;
pub mod verify;

pub use folder_style::{Extracted, FolderStyle};
pub use link::{
    FolderLinkError, FolderWarning, ValidatedLink, check_looks_like_library, normalize_path_key,
    probe_folder, validate_folder_link,
};
pub use matcher::{
    Conflict, LocalOrphan, MatchAssignment, MatchProposal, ReviewItem, Score, ScoredCandidate,
    Signal, assign_matches, assign_matches_with_rejected, extract_release_slug,
    filter_releases_for_picker, format_release_picker_label, normalize_bandcamp_url,
    release_url_slug, resolve_release_by_query, score_candidate, score_release,
};
pub use scan::{
    AUDIO_EXTENSIONS, AlbumCandidate, CandidateKind, ScanReport, is_audio_file,
    is_disc_folder_name, normalize_candidate_path_key, scan_linked_folder,
};
pub use service::{LibraryProgressCallback, ReconcileReport, run_scan_and_match};
pub use tags::{AudioMetadata, guess_audio_format, read_audio_tags};
pub use textmatch::{key, tokens};
pub use verify::{
    VerifyReport, VerifyResult, attribute_release_root, check_disk_release_path,
    group_unavailable_releases, is_mass_missing, verify_single_release,
};

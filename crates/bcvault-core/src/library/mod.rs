pub mod link;
pub mod matcher;
pub mod scan;
pub mod tags;
pub mod textmatch;

pub use link::{
    FolderLinkError, FolderWarning, ValidatedLink, check_looks_like_library, normalize_path_key,
    probe_folder, validate_folder_link,
};
pub use matcher::{
    Conflict, LocalOrphan, MatchAssignment, MatchProposal, ReviewItem, Score, ScoredCandidate,
    Signal, assign_matches, normalize_bandcamp_url, score_candidate, score_release,
};
pub use scan::{
    AUDIO_EXTENSIONS, AlbumCandidate, CandidateKind, ScanReport, is_audio_file,
    is_disc_folder_name, normalize_candidate_path_key, scan_linked_folder,
};
pub use tags::{AudioMetadata, guess_audio_format, read_audio_tags};
pub use textmatch::{
    name_variant_keys, name_variants, normalize, strip_diacritics, strip_noise_tokens,
};

pub mod scan;
pub mod tags;
pub mod textmatch;

pub use scan::{
    AUDIO_EXTENSIONS, AlbumCandidate, CandidateKind, ScanReport, is_audio_file,
    is_disc_folder_name, normalize_candidate_path_key, scan_linked_folder,
};
pub use tags::{AudioMetadata, guess_audio_format, read_audio_tags};
pub use textmatch::{
    name_variant_keys, name_variants, normalize, strip_diacritics, strip_noise_tokens,
};

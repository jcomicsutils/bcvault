use std::fs::File;
use std::path::Path;

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::mp4::{Mp4Codec, Mp4File};
use lofty::tag::{Accessor, ItemKey, Tag};

use crate::types::AudioFormat;

/// Maximum audio file size to attempt parsing tags from (1 GiB).
const MAX_TAG_FILE_SIZE: u64 = 1024 * 1024 * 1024;

/// Extracted metadata from an audio file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioMetadata {
    /// Artist name (used for display only; the artist is already known).
    pub artist: Option<String>,
    /// Album or release title.
    pub album: Option<String>,
    /// Release year.
    pub year: Option<u32>,
    /// Bandcamp release or track URL embedded in comment, description or URL tags.
    pub bandcamp_url: Option<String>,
    /// Codec name (e.g. "flac", "mpeg", "mp4").
    pub codec: Option<String>,
    /// Estimated bitrate in kbps.
    pub bitrate_kbps: Option<u32>,
    /// Whether the file is detected as variable bitrate.
    pub is_vbr: Option<bool>,
    /// Best-effort guess of Bandcamp `downloaded_format`.
    pub format_guess: Option<AudioFormat>,
}

/// Reads tags and properties from an audio file.
/// Returns `None` if the file does not exist, is too large, or cannot be parsed as audio.
pub fn read_audio_tags(path: &Path) -> Option<AudioMetadata> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_TAG_FILE_SIZE {
        return None;
    }

    let tagged_file = lofty::read_from_path(path).ok()?;
    let file_type = tagged_file.file_type();

    let mut artist = None;
    let mut album = None;
    let mut year = None;
    let mut bandcamp_url = None;

    for tag in tagged_file.tags() {
        if artist.is_none() {
            artist = tag
                .artist()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }
        if album.is_none() {
            album = tag
                .album()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }
        if year.is_none() {
            year = extract_year(tag);
        }
        if bandcamp_url.is_none() {
            bandcamp_url = extract_bandcamp_url(tag);
        }
    }

    // ID3v2 in MP3 files may retain unmapped frames (like TXXX:comment) that generic Tag skips
    if bandcamp_url.is_none() && file_type == FileType::Mpeg {
        bandcamp_url = extract_bandcamp_from_mpeg(path);
    }

    let props = tagged_file.properties();
    let bitrate_kbps = props.audio_bitrate().or_else(|| props.overall_bitrate());

    let codec_name = format!("{:?}", file_type).to_lowercase();
    let format_guess = guess_audio_format(path, file_type, bitrate_kbps);

    // Determine VBR heuristic for MP3: if bitrate is between 200 and 280, it's typically VBR.
    let is_vbr = match file_type {
        FileType::Mpeg => bitrate_kbps.map(|b| (200..=280).contains(&b)),
        _ => None,
    };

    Some(AudioMetadata {
        artist,
        album,
        year,
        bandcamp_url,
        codec: Some(codec_name),
        bitrate_kbps,
        is_vbr,
        format_guess,
    })
}

/// Fallback extractor for Bandcamp URLs stored in non-standard ID3v2 frames in MP3 files.
fn extract_bandcamp_from_mpeg(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mpeg_file = lofty::mpeg::MpegFile::read_from(&mut file, ParseOptions::new()).ok()?;
    let id3v2 = mpeg_file.id3v2()?;

    for frame in id3v2.iter() {
        let content_opt = match frame {
            lofty::id3::v2::Frame::Comment(c) => Some(c.content.as_ref()),
            lofty::id3::v2::Frame::Text(t) => Some(t.value.as_ref()),
            lofty::id3::v2::Frame::UserText(u) => Some(u.content.as_ref()),
            lofty::id3::v2::Frame::Url(u) => Some(u.url()),
            lofty::id3::v2::Frame::UserUrl(u) => Some(u.content.as_ref()),
            lofty::id3::v2::Frame::UnsynchronizedText(u) => Some(u.content.as_ref()),
            _ => None,
        };
        if let Some(url) = content_opt.and_then(find_bandcamp_url) {
            return Some(url);
        }
    }

    None
}

/// Extracts a 4-digit release year from tag accessors or item keys.
fn extract_year(tag: &Tag) -> Option<u32> {
    if let Some(date) = tag.date().filter(|d| (1900..=2099).contains(&d.year)) {
        return Some(date.year as u32);
    }

    let candidate_keys = [
        ItemKey::Year,
        ItemKey::RecordingDate,
        ItemKey::OriginalReleaseDate,
    ];

    for key in candidate_keys {
        if let Some(y) = tag
            .get(key)
            .and_then(|item| item.value().text())
            .and_then(parse_year_from_str)
        {
            return Some(y);
        }
    }

    None
}

/// Parses the first 4-digit year in range 1900..=2099 from a string.
fn parse_year_from_str(s: &str) -> Option<u32> {
    for token in s.split(|c: char| !c.is_ascii_digit()) {
        if token.len() == 4
            && let Ok(y @ 1900..=2099) = token.parse::<u32>()
        {
            return Some(y);
        }
    }
    None
}

/// Extracts a Bandcamp release or track URL embedded in tag comments, locators, or text fields.
fn extract_bandcamp_url(tag: &Tag) -> Option<String> {
    if let Some(url) = tag.comment().as_deref().and_then(find_bandcamp_url) {
        return Some(url);
    }

    for item in tag.items() {
        let text_opt = match item.value() {
            lofty::tag::ItemValue::Text(s) | lofty::tag::ItemValue::Locator(s) => Some(s.as_str()),
            _ => None,
        };
        if let Some(url) = text_opt.and_then(find_bandcamp_url) {
            return Some(url);
        }
    }

    None
}

/// Scans text for any `bandcamp.com` URL.
fn find_bandcamp_url(text: &str) -> Option<String> {
    if !text.contains("bandcamp.com") {
        return None;
    }

    for word in text.split_whitespace() {
        let clean = word.trim_matches(|c: char| {
            matches!(
                c,
                '<' | '>' | '"' | '\'' | '(' | ')' | '[' | ']' | ',' | ';'
            )
        });
        if (clean.starts_with("http://") || clean.starts_with("https://"))
            && clean.contains("bandcamp.com")
        {
            return Some(clean.to_string());
        }
    }

    None
}

/// Guesses the `AudioFormat` according to the §5.2 rules:
/// - `flac` -> Flac
/// - `wav` -> Wav
/// - `aiff` -> AiffLossless
/// - `ogg` -> Vorbis
/// - MP4 ALAC -> Alac
/// - MP4 AAC -> AacHi
/// - MP3 CBR 310 to 320 kbps -> Mp3_320
/// - MP3 VBR average 220 to 260 -> Mp3V0
/// - Otherwise -> None
pub fn guess_audio_format(
    path: &Path,
    file_type: FileType,
    bitrate_kbps: Option<u32>,
) -> Option<AudioFormat> {
    match file_type {
        FileType::Flac => Some(AudioFormat::Flac),
        FileType::Wav => Some(AudioFormat::Wav),
        FileType::Aiff => Some(AudioFormat::AiffLossless),
        FileType::Vorbis | FileType::Opus => Some(AudioFormat::Vorbis),
        FileType::Mp4 => guess_mp4_format(path).or(Some(AudioFormat::AacHi)),
        FileType::Mpeg => guess_mpeg_format(bitrate_kbps),
        _ => {
            // Fallback to file extension
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_lowercase())?;
            match ext.as_str() {
                "flac" => Some(AudioFormat::Flac),
                "wav" => Some(AudioFormat::Wav),
                "aif" | "aiff" => Some(AudioFormat::AiffLossless),
                "ogg" | "opus" => Some(AudioFormat::Vorbis),
                "m4a" => guess_mp4_format(path).or(Some(AudioFormat::AacHi)),
                "mp3" => guess_mpeg_format(bitrate_kbps),
                _ => None,
            }
        }
    }
}

/// Distinguishes Apple Lossless (ALAC) from AAC in an MP4 container.
fn guess_mp4_format(path: &Path) -> Option<AudioFormat> {
    let mut file = File::open(path).ok()?;
    let mp4_file = Mp4File::read_from(&mut file, ParseOptions::new()).ok()?;
    match mp4_file.properties().codec()? {
        Mp4Codec::ALAC => Some(AudioFormat::Alac),
        Mp4Codec::AAC => Some(AudioFormat::AacHi),
        _ => None,
    }
}

/// Classifies MP3 bitrates into CBR 320 or V0.
fn guess_mpeg_format(bitrate_kbps: Option<u32>) -> Option<AudioFormat> {
    let kbps = bitrate_kbps?;
    if (310..=335).contains(&kbps) {
        Some(AudioFormat::Mp3_320)
    } else if (210..=270).contains(&kbps) {
        Some(AudioFormat::Mp3V0)
    } else {
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_path(name: &str) -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("tests");
        path.push("fixtures");
        path.push(name);
        path
    }

    #[test]
    fn reads_flac_tags_and_guesses_format() {
        let path = fixture_path("silence.flac");
        let meta = read_audio_tags(&path).expect("should read silence.flac tags");

        assert_eq!(meta.artist.as_deref(), Some("Test Artist"));
        assert_eq!(meta.album.as_deref(), Some("Test Album"));
        assert_eq!(meta.year, Some(2021));
        assert_eq!(
            meta.bandcamp_url.as_deref(),
            Some("https://testartist.bandcamp.com/album/test-album")
        );
        assert_eq!(meta.format_guess, Some(AudioFormat::Flac));
    }

    #[test]
    fn reads_mp3_tags_and_guesses_cbr_320() {
        let path = fixture_path("silence.mp3");
        let meta = read_audio_tags(&path).expect("should read silence.mp3 tags");

        assert_eq!(meta.artist.as_deref(), Some("Test Artist"));
        assert_eq!(meta.album.as_deref(), Some("Test Album"));
        assert_eq!(meta.year, Some(2021));
        assert_eq!(
            meta.bandcamp_url.as_deref(),
            Some("https://testartist.bandcamp.com/album/test-album")
        );
        assert_eq!(meta.format_guess, Some(AudioFormat::Mp3_320));
    }

    #[test]
    fn guesses_format_from_bitrate_matrix() {
        let dummy = Path::new("test.mp3");
        assert_eq!(
            guess_audio_format(dummy, FileType::Mpeg, Some(320)),
            Some(AudioFormat::Mp3_320)
        );
        assert_eq!(
            guess_audio_format(dummy, FileType::Mpeg, Some(315)),
            Some(AudioFormat::Mp3_320)
        );
        assert_eq!(
            guess_audio_format(dummy, FileType::Mpeg, Some(245)),
            Some(AudioFormat::Mp3V0)
        );
        assert_eq!(guess_audio_format(dummy, FileType::Mpeg, Some(192)), None);
        assert_eq!(guess_audio_format(dummy, FileType::Mpeg, Some(128)), None);
    }

    #[test]
    fn non_audio_or_missing_file_returns_none() {
        assert_eq!(read_audio_tags(Path::new("non_existent_file.flac")), None);
        let manifest = fixture_path("manifest.toml");
        assert_eq!(read_audio_tags(&manifest), None);
    }
}

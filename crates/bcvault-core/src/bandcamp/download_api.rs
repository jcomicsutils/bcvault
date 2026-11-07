use crate::{
    bandcamp::{BUY_FOR_NOTHING_PATH, BandcampClient, errors::map_api_rejection, gate::GateClass},
    error::BcError,
    types::{AudioFormat, FormatFallback, ItemKey},
};
use serde::Deserialize;
use std::{collections::HashMap, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChosenDownload {
    pub format: AudioFormat,
    pub url: String,
}

#[derive(Deserialize)]
struct BuyForNothingResponse {
    #[serde(default)]
    ok: Option<bool>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
    #[serde(default)]
    downloads: HashMap<String, FormatUrlEntry>,
    #[serde(default)]
    item: Option<NestedItemDownloads>,
}

#[derive(Deserialize)]
struct NestedItemDownloads {
    #[serde(default)]
    downloads: HashMap<String, FormatUrlEntry>,
}

#[derive(Deserialize)]
struct FormatUrlEntry {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct StatDownloadJson {
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    retry_url: Option<String>,
}

const LOSSLESS_PREF: [AudioFormat; 8] = [
    AudioFormat::Flac,
    AudioFormat::Alac,
    AudioFormat::Wav,
    AudioFormat::AiffLossless,
    AudioFormat::Mp3_320,
    AudioFormat::Mp3V0,
    AudioFormat::AacHi,
    AudioFormat::Vorbis,
];

const LOSSY_PREF: [AudioFormat; 8] = [
    AudioFormat::Mp3_320,
    AudioFormat::Mp3V0,
    AudioFormat::AacHi,
    AudioFormat::Vorbis,
    AudioFormat::Flac,
    AudioFormat::Alac,
    AudioFormat::Wav,
    AudioFormat::AiffLossless,
];

pub fn choose_format(
    available: &HashMap<AudioFormat, String>,
    wanted: AudioFormat,
    fallback: FormatFallback,
) -> Result<ChosenDownload, BcError> {
    if let Some(url) = available.get(&wanted) {
        return Ok(ChosenDownload {
            format: wanted,
            url: url.clone(),
        });
    }

    if fallback == FormatFallback::Strict {
        return Err(BcError::FormatUnavailable);
    }

    let order = if wanted.is_lossless() {
        &LOSSLESS_PREF
    } else {
        &LOSSY_PREF
    };

    for &fmt in order {
        if let Some(url) = available.get(&fmt) {
            return Ok(ChosenDownload {
                format: fmt,
                url: url.clone(),
            });
        }
    }

    Err(BcError::FormatUnavailable)
}

pub fn parse_buy_for_nothing_json(json_str: &str) -> Result<HashMap<AudioFormat, String>, BcError> {
    let resp: BuyForNothingResponse = serde_json::from_str(json_str)?;

    if resp.ok != Some(true) {
        let msg = resp
            .error_message
            .or(resp.error)
            .unwrap_or_else(|| "buy_for_nothing rejected request".to_string());
        return Err(map_api_rejection(&msg));
    }

    let raw_map = if !resp.downloads.is_empty() {
        resp.downloads
    } else if let Some(item) = resp.item {
        item.downloads
    } else {
        HashMap::new()
    };

    let mut out = HashMap::new();
    for (k, entry) in raw_map {
        if let (Some(fmt), Some(url)) = (AudioFormat::parse(&k), entry.url)
            && !url.trim().is_empty()
        {
            out.insert(fmt, url);
        }
    }

    if out.is_empty() {
        return Err(BcError::FormatUnavailable);
    }

    Ok(out)
}

impl BandcampClient {
    pub async fn buy_for_nothing(
        &self,
        key: ItemKey,
        band_id: i64,
    ) -> Result<HashMap<AudioFormat, String>, BcError> {
        let url = format!("{}{BUY_FOR_NOTHING_PATH}", self.api_base_url);
        let payload = serde_json::json!({
            "item_type": key.kind.as_str(),
            "item_id": key.id.to_string(),
        });
        let text = self
            .post_json_gated(&url, &payload, GateClass::Api, Some(band_id))
            .await?;
        parse_buy_for_nothing_json(&text)
    }

    /// Resolves Bandcamp's `/statdownload/` handshake when a download URL returns HTML or JSON
    /// instead of the binary stream.
    pub async fn resolve_download_url(
        &self,
        initial_url: &str,
        body_text: &str,
        band_id: Option<i64>,
    ) -> Result<String, BcError> {
        if let Some(resolved) = extract_statdownload_url(body_text) {
            return Ok(resolved);
        }

        let mut poll_url = if initial_url.contains("/download/") {
            let swapped = initial_url.replacen("/download/", "/statdownload/", 1);
            if swapped.contains('?') {
                format!("{swapped}&.vrs=1")
            } else {
                format!("{swapped}?.vrs=1")
            }
        } else {
            initial_url.to_string()
        };

        for attempt in 0..10 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            let (text, _) = self
                .get_text_gated(&poll_url, GateClass::Api, band_id)
                .await?;
            if let Some(dl_url) = extract_statdownload_url(&text) {
                return Ok(dl_url);
            }
            if let Some(next_retry) = extract_retry_url(&text) {
                poll_url = next_retry;
            }
        }

        Err(BcError::UnexpectedResponse(
            "Timed out resolving statdownload URL".to_string(),
        ))
    }
}

fn extract_statdownload_url(text: &str) -> Option<String> {
    let trimmed = text.trim();
    // Strip optional JS variable assignment wrapper (e.g. `if ( window.Downloads ) { ... }`)
    let json_slice = extract_json_object(trimmed)?;
    let parsed: StatDownloadJson = serde_json::from_str(json_slice).ok()?;
    if parsed.result.as_deref() == Some("err") {
        return None;
    }
    parsed.download_url.filter(|s| !s.trim().is_empty())
}

fn extract_retry_url(text: &str) -> Option<String> {
    let json_slice = extract_json_object(text.trim())?;
    let parsed: StatDownloadJson = serde_json::from_str(json_slice).ok()?;
    parsed.retry_url.filter(|s| !s.trim().is_empty())
}

fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end >= start {
        Some(&s[start..=end])
    } else {
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn format_fallback_selection() {
        let mut map = HashMap::new();
        map.insert(AudioFormat::Alac, "https://example.com/alac".to_string());
        map.insert(AudioFormat::Mp3V0, "https://example.com/v0".to_string());

        // Strict fails when Flac missing
        assert!(matches!(
            choose_format(&map, AudioFormat::Flac, FormatFallback::Strict),
            Err(BcError::FormatUnavailable)
        ));

        // Nearest picks Alac when Flac requested
        let c1 = choose_format(&map, AudioFormat::Flac, FormatFallback::Nearest).unwrap();
        assert_eq!(c1.format, AudioFormat::Alac);

        // Nearest picks Mp3V0 when Mp3_320 requested
        let c2 = choose_format(&map, AudioFormat::Mp3_320, FormatFallback::Nearest).unwrap();
        assert_eq!(c2.format, AudioFormat::Mp3V0);
    }
}

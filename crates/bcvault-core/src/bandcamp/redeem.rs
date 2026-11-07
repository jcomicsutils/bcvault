use crate::{
    error::BcError,
    types::{AudioFormat, ItemKey, ItemType},
};
use scraper::{Html, Selector};
use serde::Deserialize;
use std::collections::HashMap;
use url::Url;

#[derive(Clone, Debug, PartialEq)]
pub struct RedeemDownloadInfo {
    pub url: String,
    pub size_mb: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RedeemItem {
    pub key: ItemKey,
    pub title: String,
    pub artist: String,
    pub art_id: Option<i64>,
    pub downloads: HashMap<AudioFormat, RedeemDownloadInfo>,
}

#[derive(Deserialize)]
struct PageBlob {
    #[serde(default)]
    download_items: Vec<RawRedeemItem>,
}

#[derive(Deserialize)]
struct RawRedeemItem {
    #[serde(default)]
    item_id: Option<i64>,
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    item_type: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    art_id: Option<i64>,
    #[serde(default)]
    downloads: HashMap<String, RawFormatEntry>,
}

#[derive(Deserialize)]
struct RawFormatEntry {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    size_mb: Option<serde_json::Value>,
    #[serde(default)]
    description: Option<String>,
}

pub fn validate_redeem_url(raw_url: &str, allow_local_mock: bool) -> Result<Url, BcError> {
    let parsed =
        Url::parse(raw_url.trim()).map_err(|_| BcError::InvalidRedeemUrl(raw_url.to_string()))?;

    let host = parsed
        .host_str()
        .ok_or_else(|| BcError::InvalidRedeemUrl(raw_url.to_string()))?
        .to_ascii_lowercase();

    let is_local = allow_local_mock && (host == "localhost" || host.starts_with("127."));

    if !is_local {
        if parsed.scheme() != "https" {
            return Err(BcError::InvalidRedeemUrl(
                "Redeem URL must use https".to_string(),
            ));
        }
        if host != "bandcamp.com" && !host.ends_with(".bandcamp.com") {
            return Err(BcError::InvalidRedeemUrl(format!(
                "Host '{host}' is not bandcamp.com"
            )));
        }
    }

    if parsed.path() != "/download" {
        return Err(BcError::InvalidRedeemUrl(
            "Redeem URL path must be /download".to_string(),
        ));
    }

    Ok(parsed)
}

pub fn parse_redeem_html(html_text: &str) -> Result<Vec<RedeemItem>, BcError> {
    let doc = Html::parse_document(html_text);
    let sel = Selector::parse("#pagedata[data-blob]")
        .map_err(|e| BcError::UnexpectedResponse(format!("selector err: {e:?}")))?;

    let raw_blob = doc
        .select(&sel)
        .find_map(|el| el.value().attr("data-blob"))
        .ok_or(BcError::RedeemExpiredOrEmpty)?;

    let blob: PageBlob = serde_json::from_str(raw_blob)?;
    if blob.download_items.is_empty() {
        return Err(BcError::RedeemExpiredOrEmpty);
    }

    let mut out = Vec::new();
    for raw in blob.download_items {
        let Some(id) = raw.item_id.or(raw.id) else {
            continue;
        };
        let kind = raw
            .item_type
            .as_deref()
            .and_then(ItemType::parse)
            .or_else(|| raw.kind.as_deref().and_then(ItemType::parse))
            .unwrap_or(ItemType::Album);

        let mut downloads = HashMap::new();
        for (fmt_key, entry) in raw.downloads {
            if let (Some(fmt), Some(url)) = (AudioFormat::parse(&fmt_key), entry.url) {
                let size_mb = entry.size_mb.map(|v| match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                });
                downloads.insert(
                    fmt,
                    RedeemDownloadInfo {
                        url,
                        size_mb,
                        description: entry.description,
                    },
                );
            }
        }

        out.push(RedeemItem {
            key: ItemKey { kind, id },
            title: raw.title.unwrap_or_else(|| "Untitled".to_string()),
            artist: raw.artist.unwrap_or_else(|| "Unknown Artist".to_string()),
            art_id: raw.art_id,
            downloads,
        });
    }

    if out.is_empty() {
        return Err(BcError::RedeemExpiredOrEmpty);
    }

    Ok(out)
}

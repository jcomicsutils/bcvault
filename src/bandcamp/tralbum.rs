use crate::{
    bandcamp::classify::{PricingSignals, classify},
    error::BcError,
    types::{Classification, ItemType, Ts},
};
use scraper::{Html, Selector};
use serde::{Deserialize, Deserializer};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flag(pub Option<bool>);

impl<'de> Deserialize<'de> for Flag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let val = Option::<serde_json::Value>::deserialize(deserializer)?;
        let res = match val {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::Bool(b)) => Some(b),
            Some(serde_json::Value::Number(n)) => n.as_i64().map(|i| i != 0),
            Some(serde_json::Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" => Some(true),
                "0" | "false" | "no" | "" => Some(false),
                _ => None,
            },
            _ => None,
        };
        Ok(Flag(res))
    }
}

fn deserialize_lenient_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let val = Option::<serde_json::Value>::deserialize(deserializer)?;
    let res = match val {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Number(n)) => n.as_f64(),
        Some(serde_json::Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    Ok(res)
}

fn deserialize_lenient_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let val = Option::<serde_json::Value>::deserialize(deserializer)?;
    let res = match val {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Number(n)) => n.as_i64(),
        Some(serde_json::Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    };
    Ok(res)
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Current {
    #[serde(default, deserialize_with = "deserialize_lenient_i64")]
    pub id: Option<i64>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_i64")]
    pub band_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub minimum_price: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub set_price: Option<f64>,
    #[serde(default)]
    pub is_set_price: Flag,
    #[serde(default)]
    pub require_email: Flag,
    #[serde(default)]
    pub publish_date: Option<String>,
    #[serde(default)]
    pub mod_date: Option<String>,
    #[serde(default)]
    pub is_preorder: Flag,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Tralbum {
    #[serde(default, deserialize_with = "deserialize_lenient_i64")]
    pub id: Option<i64>,
    #[serde(default)]
    pub item_type: Option<String>,
    #[serde(default, rename = "freeDownloadPage")]
    pub free_download_page: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_i64")]
    pub art_id: Option<i64>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album_release_date: Option<String>,
    #[serde(default)]
    pub is_preorder: Flag,
    #[serde(default)]
    pub trackinfo: Vec<serde_json::Value>,
    #[serde(default)]
    pub current: Current,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedReleasePage {
    pub item_id: Option<i64>,
    pub item_type: Option<ItemType>,
    pub band_id: Option<i64>,
    pub title: Option<String>,
    pub artist_name: Option<String>,
    pub art_id: Option<i64>,
    pub track_count: u32,
    pub published_at: Option<Ts>,
    pub modified_at: Option<Ts>,
    pub is_preorder: bool,
    pub signals: PricingSignals,
    pub pricing_json: String,
    pub classification: Classification,
}

pub fn parse_release_html(html_text: &str) -> Result<ParsedReleasePage, BcError> {
    let doc = Html::parse_document(html_text);
    let sel = Selector::parse("script[data-tralbum]")
        .map_err(|e| BcError::UnexpectedResponse(format!("selector err: {e:?}")))?;

    let raw_attr = doc
        .select(&sel)
        .find_map(|el| el.value().attr("data-tralbum"))
        .ok_or(BcError::TralbumNotFound)?;

    parse_tralbum_json(raw_attr)
}

pub fn parse_tralbum_json(json_str: &str) -> Result<ParsedReleasePage, BcError> {
    let tr: Tralbum = serde_json::from_str(json_str)?;

    let item_id = tr.id.or(tr.current.id);
    let item_type = tr
        .item_type
        .as_deref()
        .and_then(ItemType::parse)
        .or_else(|| tr.current.kind.as_deref().and_then(ItemType::parse));

    let track_count = if item_type == Some(ItemType::Track) {
        1
    } else if !tr.trackinfo.is_empty() {
        tr.trackinfo.len() as u32
    } else {
        0
    };

    let published_at = tr
        .current
        .publish_date
        .as_deref()
        .and_then(parse_bc_date)
        .or_else(|| tr.album_release_date.as_deref().and_then(parse_bc_date));

    let modified_at = tr.current.mod_date.as_deref().and_then(parse_bc_date);

    let is_preorder = tr
        .current
        .is_preorder
        .0
        .or(tr.is_preorder.0)
        .unwrap_or(false);

    let has_free_download_page = tr
        .free_download_page
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty());

    let signals = PricingSignals {
        minimum_price: tr.current.minimum_price,
        set_price: tr.current.set_price,
        is_set_price: tr.current.is_set_price.0,
        require_email: tr.current.require_email.0,
        has_free_download_page,
    };

    let pricing_json = serde_json::to_string(&signals)?;
    let classification = classify(&signals);

    Ok(ParsedReleasePage {
        item_id,
        item_type,
        band_id: tr.current.band_id,
        title: tr.current.title,
        artist_name: tr.artist,
        art_id: tr.art_id,
        track_count,
        published_at,
        modified_at,
        is_preorder,
        signals,
        pricing_json,
        classification,
    })
}

pub fn parse_bc_date(s: &str) -> Option<Ts> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(trimmed) {
        return Some(dt.timestamp());
    }
    // Fallback for "+0000" instead of "GMT"
    let normalized = trimmed.replace("GMT", "+0000");
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(&normalized) {
        return Some(dt.timestamp());
    }
    tracing::warn!("Could not parse Bandcamp date string: {trimmed}");
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn lenient_tralbum_variants() {
        let json = r#"{
            "id": "555",
            "item_type": "album",
            "freeDownloadPage": null,
            "art_id": 999,
            "artist": "Test Artist",
            "trackinfo": [{}, {}, {}],
            "current": {
                "band_id": "42",
                "title": "Test Album",
                "minimum_price": "0.0",
                "set_price": 7,
                "is_set_price": 0,
                "require_email": "1",
                "publish_date": "20 Aug 2021 00:00:00 GMT",
                "mod_date": "21 Aug 2021 12:00:00 GMT",
                "is_preorder": "0"
            }
        }"#;
        let parsed = parse_tralbum_json(json).unwrap();
        assert_eq!(parsed.item_id, Some(555));
        assert_eq!(parsed.item_type, Some(ItemType::Album));
        assert_eq!(parsed.band_id, Some(42));
        assert_eq!(parsed.track_count, 3);
        assert_eq!(parsed.classification, Classification::EmailRequired);
        assert_eq!(parsed.published_at, Some(1_629_417_600));
        assert!(!parsed.is_preorder);
    }
}

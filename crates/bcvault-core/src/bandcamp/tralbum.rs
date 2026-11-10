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
    #[serde(default, deserialize_with = "deserialize_lenient_i64")]
    pub download_pref: Option<i64>,
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
    let nyp_sel = Selector::parse(".buyItemNyp")
        .map_err(|e| BcError::UnexpectedResponse(format!("selector err: {e:?}")))?;

    let raw_attr = doc
        .select(&sel)
        .find_map(|el| el.value().attr("data-tralbum"))
        .ok_or(BcError::TralbumNotFound)?;
    let has_nyp_badge = doc.select(&nyp_sel).next().is_some();

    parse_tralbum_json_with_dom(raw_attr, has_nyp_badge)
}

pub fn parse_release_raw_json(html_text: &str) -> Result<serde_json::Value, BcError> {
    let doc = Html::parse_document(html_text);
    let sel = Selector::parse("script[data-tralbum]")
        .map_err(|e| BcError::UnexpectedResponse(format!("selector err: {e:?}")))?;

    let raw_attr = doc
        .select(&sel)
        .find_map(|el| el.value().attr("data-tralbum"))
        .ok_or(BcError::TralbumNotFound)?;

    let mut val: serde_json::Value = serde_json::from_str(raw_attr)?;

    if let Some(obj) = val.as_object_mut() {
        if let Ok(tag_sel) = Selector::parse("a.tag") {
            let tags: Vec<String> = doc
                .select(&tag_sel)
                .map(|el| el.text().collect::<String>().trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !tags.is_empty() {
                obj.insert("_dom_tags".to_string(), serde_json::json!(tags));
            }
        }
        if let Ok(about_sel) = Selector::parse(".tralbumData.tralbum-about")
            && let Some(el) = doc.select(&about_sel).next()
        {
            let text = el.text().collect::<String>().trim().to_string();
            if !text.is_empty() {
                obj.insert("_dom_about".to_string(), serde_json::Value::String(text));
            }
        }
        if let Ok(cred_sel) = Selector::parse(".tralbumData.tralbum-credits")
            && let Some(el) = doc.select(&cred_sel).next()
        {
            let text = el.text().collect::<String>().trim().to_string();
            if !text.is_empty() {
                obj.insert("_dom_credits".to_string(), serde_json::Value::String(text));
            }
        }
    }

    Ok(val)
}

pub fn parse_tralbum_json(json_str: &str) -> Result<ParsedReleasePage, BcError> {
    parse_tralbum_json_with_dom(json_str, false)
}

fn parse_tralbum_json_with_dom(
    json_str: &str,
    has_nyp_badge: bool,
) -> Result<ParsedReleasePage, BcError> {
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
        download_pref: tr.current.download_pref,
        has_nyp_badge,
        track_count: Some(track_count),
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

    #[test]
    fn nyp_html_with_free_download_page_classifies_as_nyp() {
        let html = r#"<!doctype html>
<html>
<head>
  <script data-tralbum='{
    "FREE": 1,
    "PAID": 2,
    "id": 1112467332,
    "item_type": "album",
    "freeDownloadPage": "https://bandcamp.com/download?fsig=abc&id=1112467332&ts=1775866775&type=album",
    "art_id": 236863520,
    "artist": "Bird Watcher",
    "trackinfo": [{"title": "Acanthiza chrysorrhoa"}],
    "current": {
      "id": 1112467332,
      "type": "album",
      "band_id": 2847952267,
      "title": "Acanthiza chrysorrhoa",
      "download_pref": 2,
      "minimum_price": 0.0,
      "set_price": 9.0,
      "is_set_price": null,
      "require_email": null,
      "publish_date": "13 Feb 2023 09:05:31 GMT",
      "mod_date": "23 Feb 2023 23:25:34 GMT",
      "is_preorder": null
    }
  }'></script>
</head>
<body>
  <h4 class="ft compound-button main-button">
    <button class="download-link buy-link" type="button">Buy Digital Album</button>
    <span class="buyItemExtra buyItemNyp secondaryText">name your price</span>
  </h4>
</body>
</html>"#;
        let parsed = parse_release_html(html).unwrap();
        assert_eq!(parsed.item_id, Some(1_112_467_332));
        assert_eq!(parsed.classification, Classification::Nyp);
        assert_eq!(parsed.signals.download_pref, Some(2));
        assert!(parsed.signals.has_nyp_badge);
    }

    #[test]
    fn zero_track_album_with_free_pricing_classifies_as_unavailable() {
        let html = r#"<!doctype html>
<html>
<head>
  <script data-tralbum='{
    "FREE": 1,
    "PAID": 2,
    "id": 4112009107,
    "item_type": "album",
    "freeDownloadPage": "https://bandcamp.com/download?fsig=00000000000000000000000000000000&id=4112009107&ts=1794351138.0&type=album",
    "art_id": 3549813623,
    "artist": "Burial Garden",
    "trackinfo": [],
    "current": {
      "id": 4112009107,
      "type": "album",
      "band_id": 1467807624,
      "title": "i usually don&#39;t like going too loud when mixing stuff but i felt like i had to make an exception for this one",
      "download_pref": 2,
      "minimum_price": 0.0,
      "set_price": 9.0,
      "is_set_price": null,
      "require_email": null,
      "publish_date": "01 Oct 2025 19:50:58 GMT",
      "mod_date": "01 Oct 2025 20:00:19 GMT",
      "is_preorder": false
    }
  }'></script>
</head>
<body>
  <ul class="tralbumCommands">
    <li id="collect-item-placeholder"></li>
  </ul>
  <table class="track_list track_table" id="track_table"></table>
</body>
</html>"#;
        let parsed = parse_release_html(html).unwrap();
        assert_eq!(parsed.item_id, Some(4_112_009_107));
        assert_eq!(parsed.item_type, Some(ItemType::Album));
        assert_eq!(parsed.track_count, 0);
        assert_eq!(parsed.classification, Classification::Unavailable);
        assert!(!parsed.classification.auto_downloadable());
    }
}

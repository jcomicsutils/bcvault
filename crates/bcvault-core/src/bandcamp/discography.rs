use crate::{
    bandcamp::tralbum::parse_release_html,
    error::BcError,
    types::{ItemKey, ItemType},
};
use scraper::{Html, Selector};
use serde::Deserialize;
use std::collections::HashSet;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredItem {
    pub key: ItemKey,
    pub band_id: i64,
    pub title: String,
    pub artist_name: String,
    pub url: String,
    pub cover_url: Option<String>,
    pub art_id: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscographyPage {
    pub band_id: i64,
    pub name: String,
    pub cover_url: Option<String>,
    pub items: Vec<DiscoveredItem>,
}

#[derive(Deserialize)]
struct PageProperties {
    #[serde(default)]
    page_id: Option<i64>,
}

#[derive(Deserialize)]
struct DataBand {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct ClientItem {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    page_url: Option<String>,
    #[serde(default)]
    art_id: Option<i64>,
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    band_id: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct BandDetailsResponse {
    #[serde(default)]
    pub band_id: Option<i64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub discography: Vec<BandDetailsItem>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BandDetailsItem {
    #[serde(default)]
    pub item_id: Option<i64>,
    #[serde(default)]
    pub item_type: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub band_name: Option<String>,
    #[serde(default)]
    pub art_id: Option<i64>,
}

pub fn parse_discography_html(
    html_text: &str,
    base_url: &str,
    final_url: Option<&str>,
    fallback_slug: &str,
) -> Result<DiscographyPage, BcError> {
    let base = Url::parse(base_url).map_err(|e| BcError::UnexpectedResponse(e.to_string()))?;

    // Check if single-release redirect occurred (final URL path starts with /album/ or /track/)
    if let Some(f_url) = final_url
        && let Ok(parsed_final) = Url::parse(f_url)
    {
        let p = parsed_final.path();
        if (p.starts_with("/album/") || p.starts_with("/track/"))
            && let Ok(rel) = parse_release_html(html_text)
        {
            let doc = Html::parse_document(html_text);
            let band_id = rel
                .band_id
                .or_else(|| extract_band_id(&doc))
                .ok_or(BcError::BandIdNotFound)?;
            let name = rel
                .artist_name
                .clone()
                .or_else(|| extract_artist_name(&doc))
                .unwrap_or_else(|| fallback_slug.to_string());
            let cover_url = extract_cover_url(&doc).or_else(|| rel.art_id.map(art_id_to_url));
            let mut items = Vec::new();
            if let (Some(kind), Some(id)) = (rel.item_type, rel.item_id) {
                items.push(DiscoveredItem {
                    key: ItemKey { kind, id },
                    band_id,
                    title: rel.title.unwrap_or_else(|| "Untitled".to_string()),
                    artist_name: name.clone(),
                    url: parsed_final.to_string(),
                    cover_url: rel.art_id.map(art_id_to_url).or_else(|| cover_url.clone()),
                    art_id: rel.art_id,
                });
            }
            return Ok(DiscographyPage {
                band_id,
                name,
                cover_url,
                items,
            });
        }
    }

    let doc = Html::parse_document(html_text);
    let band_id = extract_band_id(&doc).ok_or(BcError::BandIdNotFound)?;
    let artist_name = extract_artist_name(&doc).unwrap_or_else(|| fallback_slug.to_string());
    let cover_url = extract_cover_url(&doc);

    let mut items = Vec::new();
    let mut seen = HashSet::new();

    // 1. Parse li.music-grid-item
    if let Ok(li_sel) = Selector::parse("li.music-grid-item") {
        let a_sel = Selector::parse("a").ok();
        let p_sel = Selector::parse("p.title").ok();
        let override_sel = Selector::parse(".artist-override").ok();
        let img_sel = Selector::parse("img").ok();

        for li in doc.select(&li_sel) {
            let Some(raw_item_id) = li.value().attr("data-item-id") else {
                continue;
            };
            let Some(key) = parse_data_item_id(raw_item_id) else {
                continue;
            };

            let href = a_sel
                .as_ref()
                .and_then(|s| li.select(s).next())
                .and_then(|a| a.value().attr("href"));
            let Some(href) = href else {
                continue;
            };
            let Ok(abs_url) = base.join(href) else {
                continue;
            };

            let (title, item_artist) =
                extract_grid_title_and_artist(&li, &p_sel, &override_sel, &artist_name);
            let (img_url, art_id) = extract_grid_image(&li, &img_sel);

            if seen.insert(key) {
                items.push(DiscoveredItem {
                    key,
                    band_id,
                    title,
                    artist_name: item_artist,
                    url: abs_url.to_string(),
                    cover_url: img_url,
                    art_id,
                });
            }
        }
    }

    // 2. Parse overflow items from ol#music-grid[data-client-items] or [data-client-items]
    if let Ok(grid_sel) = Selector::parse("[data-client-items]") {
        for grid_el in doc.select(&grid_sel) {
            if let Some(raw_json) = grid_el.value().attr("data-client-items")
                && let Ok(client_items) = serde_json::from_str::<Vec<ClientItem>>(raw_json)
            {
                for ci in client_items {
                    let Some(id) = ci.id else { continue };
                    let Some(kind) = ci.kind.as_deref().and_then(ItemType::parse) else {
                        continue;
                    };
                    let key = ItemKey { kind, id };
                    if !seen.insert(key) {
                        continue;
                    }
                    let Some(page_url) = ci.page_url.as_deref() else {
                        continue;
                    };
                    let Ok(abs_url) = base.join(page_url) else {
                        continue;
                    };
                    let title = ci
                        .title
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("Untitled")
                        .to_string();
                    let item_artist = ci
                        .artist
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&artist_name)
                        .to_string();
                    let cover = ci.art_id.map(art_id_to_url);
                    let _ = ci.band_id; // label pages still count against the imported band_id

                    items.push(DiscoveredItem {
                        key,
                        band_id,
                        title,
                        artist_name: item_artist,
                        url: abs_url.to_string(),
                        cover_url: cover,
                        art_id: ci.art_id,
                    });
                }
            }
        }
    }

    Ok(DiscographyPage {
        band_id,
        name: artist_name,
        cover_url,
        items,
    })
}

pub fn merge_band_details_fallback(
    page: &mut DiscographyPage,
    details: &BandDetailsResponse,
    base_url: &str,
) {
    if !page.items.is_empty() {
        return;
    }
    let Ok(base) = Url::parse(base_url) else {
        return;
    };
    let mut seen = HashSet::new();
    for item in &details.discography {
        let Some(id) = item.item_id else { continue };
        let Some(kind) = item.item_type.as_deref().and_then(ItemType::parse) else {
            continue;
        };
        let key = ItemKey { kind, id };
        if !seen.insert(key) {
            continue;
        }
        let title = item
            .title
            .as_deref()
            .unwrap_or("Untitled")
            .trim()
            .to_string();
        let artist_name = item
            .band_name
            .as_deref()
            .unwrap_or(&page.name)
            .trim()
            .to_string();
        let path = format!("/{}/{}", kind.as_str(), id);
        let url = base
            .join(&path)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("{base_url}{path}"));
        page.items.push(DiscoveredItem {
            key,
            band_id: page.band_id,
            title,
            artist_name,
            url,
            cover_url: item.art_id.map(art_id_to_url),
            art_id: item.art_id,
        });
    }
}

pub fn art_id_to_url(art_id: i64) -> String {
    format!("https://f4.bcbits.com/img/a{art_id:010}_10.jpg")
}

fn parse_data_item_id(raw: &str) -> Option<ItemKey> {
    let (prefix, id_str) = raw.trim().split_once('-')?;
    let kind = ItemType::parse(prefix)?;
    let id = id_str.parse::<i64>().ok()?;
    Some(ItemKey { kind, id })
}

fn extract_band_id(doc: &Html) -> Option<i64> {
    // 1. li.music-grid-item[data-band-id]
    if let Ok(sel) = Selector::parse("li.music-grid-item[data-band-id]") {
        for el in doc.select(&sel) {
            if let Some(val) = el.value().attr("data-band-id")
                && let Ok(id) = val.trim().parse::<i64>()
            {
                return Some(id);
            }
        }
    }

    // 2. meta[name="bc-page-properties"] JSON page_id
    if let Ok(sel) = Selector::parse(r#"meta[name="bc-page-properties"]"#) {
        for el in doc.select(&sel) {
            if let Some(content) = el.value().attr("content")
                && let Ok(props) = serde_json::from_str::<PageProperties>(content)
                && let Some(id) = props.page_id
            {
                return Some(id);
            }
        }
    }

    // 3. script[data-band] JSON id
    if let Ok(sel) = Selector::parse("script[data-band]") {
        for el in doc.select(&sel) {
            if let Some(attr) = el.value().attr("data-band")
                && let Ok(band) = serde_json::from_str::<DataBand>(attr)
                && let Some(id) = band.id
            {
                return Some(id);
            }
        }
    }

    None
}

fn extract_artist_name(doc: &Html) -> Option<String> {
    if let Ok(sel) = Selector::parse(r#"meta[property="og:site_name"]"#) {
        for el in doc.select(&sel) {
            if let Some(c) = el.value().attr("content") {
                let t = c.trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    if let Ok(sel) = Selector::parse("#band-name-location .title") {
        for el in doc.select(&sel) {
            let t = el.text().collect::<String>();
            let trimmed = t.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    if let Ok(sel) = Selector::parse("script[data-band]") {
        for el in doc.select(&sel) {
            if let Some(attr) = el.value().attr("data-band")
                && let Ok(band) = serde_json::from_str::<DataBand>(attr)
                && let Some(name) = band.name
            {
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

fn extract_cover_url(doc: &Html) -> Option<String> {
    if let Ok(sel) = Selector::parse(r#"meta[property="og:image"]"#) {
        for el in doc.select(&sel) {
            if let Some(c) = el.value().attr("content") {
                let t = c.trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    None
}

fn extract_grid_title_and_artist(
    li: &scraper::ElementRef<'_>,
    p_sel: &Option<Selector>,
    override_sel: &Option<Selector>,
    default_artist: &str,
) -> (String, String) {
    let Some(p_s) = p_sel else {
        return ("Untitled".to_string(), default_artist.to_string());
    };
    let Some(p_el) = li.select(p_s).next() else {
        return ("Untitled".to_string(), default_artist.to_string());
    };

    let override_artist = override_sel
        .as_ref()
        .and_then(|s| p_el.select(s).next())
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty());

    // Direct text nodes inside p.title exclude nested .artist-override spans
    let direct_title: String = p_el
        .children()
        .filter_map(|node| node.value().as_text().map(|t| t.as_ref()))
        .collect::<Vec<_>>()
        .join(" ");
    let clean_title = direct_title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let title = if !clean_title.is_empty() {
        clean_title
    } else {
        p_el.text()
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };

    let final_title = if title.is_empty() {
        "Untitled".to_string()
    } else {
        title
    };
    let final_artist = override_artist.unwrap_or_else(|| default_artist.to_string());
    (final_title, final_artist)
}

fn extract_grid_image(
    li: &scraper::ElementRef<'_>,
    img_sel: &Option<Selector>,
) -> (Option<String>, Option<i64>) {
    let Some(s) = img_sel else {
        return (None, None);
    };
    let Some(img) = li.select(s).next() else {
        return (None, None);
    };

    let url = img
        .value()
        .attr("data-original")
        .or_else(|| img.value().attr("src"))
        .map(str::trim)
        .filter(|u| !u.is_empty() && !u.ends_with("/0.gif"))
        .map(ToString::to_string);

    let art_id = url.as_deref().and_then(parse_art_id_from_img_url);
    (url, art_id)
}

fn parse_art_id_from_img_url(url: &str) -> Option<i64> {
    let filename = url.rsplit('/').next()?;
    let rest = filename.strip_prefix('a')?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse::<i64>().ok()
    }
}

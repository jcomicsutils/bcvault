pub mod classify;
pub mod discography;
pub mod download_api;
pub mod errors;
pub mod filenames;
pub mod gate;
pub mod input;
pub mod redeem;
pub mod tralbum;

use crate::{
    bandcamp::{
        discography::{
            BandDetailsResponse, DiscographyPage, merge_band_details_fallback,
            parse_discography_html,
        },
        gate::{BandcampGate, GateClass, parse_retry_after},
        redeem::{RedeemItem, parse_redeem_html, validate_redeem_url},
        tralbum::{ParsedReleasePage, parse_release_html},
    },
    error::BcError,
};
use std::time::Duration;

pub const BAND_DETAILS_PATH: &str = "/api/mobile/24/band_details";
pub const BUY_FOR_NOTHING_PATH: &str = "/api/mobile/22/buy_for_nothing";
const MAX_RETRIES: usize = 5;

#[derive(Clone)]
pub struct BandcampClient {
    pub gate: BandcampGate,
    pub page_client: reqwest::Client,
    pub dl_client: reqwest::Client,
    pub api_base_url: String,
    pub cdn_base_url: String,
    pub allow_local_mock: bool,
}

impl BandcampClient {
    pub fn new(gate: BandcampGate, user_agent: &str) -> Result<Self, BcError> {
        let ua = if user_agent.trim().is_empty() {
            format!(
                "bcvault/{} (personal archiving tool)",
                env!("CARGO_PKG_VERSION")
            )
        } else {
            user_agent.trim().to_string()
        };

        let page_client = reqwest::Client::builder()
            .user_agent(&ua)
            .cookie_store(true)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()?;

        let dl_client = reqwest::Client::builder()
            .user_agent(&ua)
            .cookie_store(true)
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self {
            gate,
            page_client,
            dl_client,
            api_base_url: "https://bandcamp.com".to_string(),
            cdn_base_url: "https://f4.bcbits.com".to_string(),
            allow_local_mock: false,
        })
    }

    pub fn with_base_urls(
        mut self,
        api_base_url: impl Into<String>,
        cdn_base_url: impl Into<String>,
    ) -> Self {
        self.api_base_url = api_base_url.into().trim_end_matches('/').to_string();
        self.cdn_base_url = cdn_base_url.into().trim_end_matches('/').to_string();
        self.allow_local_mock = true;
        self
    }

    pub fn cover_url_for_art_id(&self, art_id: i64) -> String {
        format!("{}/img/a{art_id:010}_10.jpg", self.cdn_base_url)
    }

    pub async fn get_text_gated(
        &self,
        url: &str,
        class: GateClass,
        band_id: Option<i64>,
    ) -> Result<(String, String), BcError> {
        let mut last_wait = Duration::from_secs(30);
        for attempt in 0..=MAX_RETRIES {
            let _permit = self.gate.acquire(class, band_id).await;
            let resp = self.page_client.get(url).send().await?;
            let status = resp.status();
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_retry_after);

            if let Some(wait_dur) = self.gate.report(status, retry_after) {
                last_wait = wait_dur;
                if attempt == MAX_RETRIES {
                    return Err(BcError::RateLimited {
                        retry_after_s: last_wait.as_secs().max(1),
                    });
                }
                continue;
            }

            let resp = resp.error_for_status()?;
            let final_url = resp.url().to_string();
            let text = resp.text().await?;
            return Ok((text, final_url));
        }

        Err(BcError::RateLimited {
            retry_after_s: last_wait.as_secs().max(1),
        })
    }

    pub async fn post_json_gated<B: serde::Serialize>(
        &self,
        url: &str,
        body: &B,
        class: GateClass,
        band_id: Option<i64>,
    ) -> Result<String, BcError> {
        let mut last_wait = Duration::from_secs(30);
        for attempt in 0..=MAX_RETRIES {
            let _permit = self.gate.acquire(class, band_id).await;
            let resp = self.page_client.post(url).json(body).send().await?;
            let status = resp.status();
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_retry_after);

            if let Some(wait_dur) = self.gate.report(status, retry_after) {
                last_wait = wait_dur;
                if attempt == MAX_RETRIES {
                    return Err(BcError::RateLimited {
                        retry_after_s: last_wait.as_secs().max(1),
                    });
                }
                continue;
            }

            let resp = resp.error_for_status()?;
            let text = resp.text().await?;
            return Ok(text);
        }

        Err(BcError::RateLimited {
            retry_after_s: last_wait.as_secs().max(1),
        })
    }

    pub async fn fetch_discography(
        &self,
        base_url: &str,
        fallback_slug: &str,
    ) -> Result<DiscographyPage, BcError> {
        let music_url = format!("{}/music", base_url.trim_end_matches('/'));
        let (html_text, final_url) = self
            .get_text_gated(&music_url, GateClass::Page, None)
            .await?;
        let mut page =
            parse_discography_html(&html_text, base_url, Some(&final_url), fallback_slug)?;

        if page.items.is_empty()
            && let Ok(details) = self.band_details(page.band_id).await
        {
            merge_band_details_fallback(&mut page, &details, base_url);
        }

        Ok(page)
    }

    pub async fn band_details(&self, band_id: i64) -> Result<BandDetailsResponse, BcError> {
        let url = format!("{}{BAND_DETAILS_PATH}", self.api_base_url);
        let payload = serde_json::json!({ "band_id": band_id });
        let text = self
            .post_json_gated(&url, &payload, GateClass::Api, Some(band_id))
            .await?;
        let parsed: BandDetailsResponse = serde_json::from_str(&text)?;
        Ok(parsed)
    }

    pub async fn fetch_release(
        &self,
        release_url: &str,
        band_id: Option<i64>,
    ) -> Result<ParsedReleasePage, BcError> {
        let (html_text, _) = self
            .get_text_gated(release_url, GateClass::Page, band_id)
            .await?;
        parse_release_html(&html_text)
    }

    pub async fn fetch_redeem_page(&self, redeem_url: &str) -> Result<Vec<RedeemItem>, BcError> {
        let validated = validate_redeem_url(redeem_url, self.allow_local_mock)?;
        let (html_text, _) = self
            .get_text_gated(validated.as_str(), GateClass::Page, None)
            .await?;
        parse_redeem_html(&html_text)
    }
}

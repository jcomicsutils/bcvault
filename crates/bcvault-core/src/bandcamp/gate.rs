use crate::types::RateLimitStatus;
use rand::Rng;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use tokio::{
    sync::{Mutex, OwnedSemaphorePermit, Semaphore},
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateClass {
    Page,
    Api,
    Download,
    Cdn,
}

pub struct GatePermit {
    _global_dl: Option<OwnedSemaphorePermit>,
    _band_dl: Option<OwnedSemaphorePermit>,
}

struct GateInner {
    page_delay_ms: u64,
    page_jitter_pct: u8,
    api_delay_ms: u64,
    cdn_delay_ms: u64,
    cooldown_until: Option<(Instant, i64, String)>,
    consecutive_429s: u32,
}

#[derive(Clone)]
pub struct BandcampGate {
    inner: Arc<StdMutex<GateInner>>,
    page_lock: Arc<Mutex<Option<Instant>>>,
    api_lock: Arc<Mutex<Option<Instant>>>,
    cdn_lock: Arc<Mutex<Option<Instant>>>,
    dl_sem: Arc<Semaphore>,
    band_sems: Arc<StdMutex<HashMap<i64, Arc<Semaphore>>>>,
    status_cb: Option<Arc<dyn Fn(RateLimitStatus) + Send + Sync>>,
}

impl BandcampGate {
    pub fn new(
        page_delay_ms: u64,
        page_jitter_pct: u8,
        max_concurrent_downloads: u8,
        status_cb: Option<Arc<dyn Fn(RateLimitStatus) + Send + Sync>>,
    ) -> Self {
        Self::with_delays(
            page_delay_ms,
            page_jitter_pct,
            1000,
            250,
            max_concurrent_downloads,
            status_cb,
        )
    }

    pub fn with_delays(
        page_delay_ms: u64,
        page_jitter_pct: u8,
        api_delay_ms: u64,
        cdn_delay_ms: u64,
        max_concurrent_downloads: u8,
        status_cb: Option<Arc<dyn Fn(RateLimitStatus) + Send + Sync>>,
    ) -> Self {
        let permits = usize::from(max_concurrent_downloads.max(1));
        Self {
            inner: Arc::new(StdMutex::new(GateInner {
                page_delay_ms,
                page_jitter_pct,
                api_delay_ms,
                cdn_delay_ms,
                cooldown_until: None,
                consecutive_429s: 0,
            })),
            page_lock: Arc::new(Mutex::new(None)),
            api_lock: Arc::new(Mutex::new(None)),
            cdn_lock: Arc::new(Mutex::new(None)),
            dl_sem: Arc::new(Semaphore::new(permits)),
            band_sems: Arc::new(StdMutex::new(HashMap::new())),
            status_cb,
        }
    }

    pub fn update_config(&self, page_delay_ms: u64, page_jitter_pct: u8) {
        if let Ok(mut g) = self.inner.lock() {
            g.page_delay_ms = page_delay_ms;
            g.page_jitter_pct = page_jitter_pct;
        }
    }

    fn emit(&self, status: RateLimitStatus) {
        if let Some(cb) = &self.status_cb {
            cb(status);
        }
    }

    async fn wait_cooldown(&self) {
        loop {
            let wait_info = {
                let Ok(mut g) = self.inner.lock() else {
                    break;
                };
                if let Some((until_inst, until_ts, ref reason)) = g.cooldown_until {
                    let now = Instant::now();
                    if until_inst > now {
                        Some((until_inst - now, until_ts, reason.clone()))
                    } else {
                        g.cooldown_until = None;
                        None
                    }
                } else {
                    None
                }
            };

            if let Some((dur, until_ts, reason)) = wait_info {
                self.emit(RateLimitStatus::Cooldown {
                    until: until_ts,
                    reason,
                });
                tokio::time::sleep(dur).await;
            } else {
                break;
            }
        }
    }

    pub async fn acquire(&self, class: GateClass, band_id: Option<i64>) -> GatePermit {
        self.wait_cooldown().await;

        match class {
            GateClass::Page => {
                let mut guard = self.page_lock.lock().await;
                let (base_ms, jitter_pct) = {
                    let g = self.inner.lock().ok();
                    g.map(|x| (x.page_delay_ms, x.page_jitter_pct))
                        .unwrap_or((1500, 25))
                };
                let gap = jittered_duration(base_ms, jitter_pct);
                if let Some(last) = *guard {
                    let elapsed = last.elapsed();
                    if gap > elapsed {
                        let rem = gap - elapsed;
                        self.emit(RateLimitStatus::Pacing {
                            next_in_ms: rem.as_millis() as u64,
                        });
                        tokio::time::sleep(rem).await;
                    }
                }
                self.wait_cooldown().await;
                *guard = Some(Instant::now());
                self.emit(RateLimitStatus::Idle);
                GatePermit {
                    _global_dl: None,
                    _band_dl: None,
                }
            }
            GateClass::Api => {
                let mut guard = self.api_lock.lock().await;
                let api_ms = self
                    .inner
                    .lock()
                    .ok()
                    .map(|g| g.api_delay_ms)
                    .unwrap_or(1000);
                let gap = Duration::from_millis(api_ms);
                if let Some(last) = *guard {
                    let elapsed = last.elapsed();
                    if gap > elapsed {
                        let rem = gap - elapsed;
                        self.emit(RateLimitStatus::Pacing {
                            next_in_ms: rem.as_millis() as u64,
                        });
                        tokio::time::sleep(rem).await;
                    }
                }
                self.wait_cooldown().await;
                *guard = Some(Instant::now());
                self.emit(RateLimitStatus::Idle);
                GatePermit {
                    _global_dl: None,
                    _band_dl: None,
                }
            }
            GateClass::Cdn => {
                let mut guard = self.cdn_lock.lock().await;
                let cdn_ms = self
                    .inner
                    .lock()
                    .ok()
                    .map(|g| g.cdn_delay_ms)
                    .unwrap_or(250);
                let gap = Duration::from_millis(cdn_ms);
                if let Some(last) = *guard {
                    let elapsed = last.elapsed();
                    if gap > elapsed {
                        tokio::time::sleep(gap - elapsed).await;
                    }
                }
                self.wait_cooldown().await;
                *guard = Some(Instant::now());
                GatePermit {
                    _global_dl: None,
                    _band_dl: None,
                }
            }
            GateClass::Download => {
                let global_permit = self.dl_sem.clone().acquire_owned().await.ok();
                let band_permit = if let Some(bid) = band_id {
                    let sem = {
                        let mut map = self.band_sems.lock().unwrap_or_else(|e| e.into_inner());
                        map.entry(bid)
                            .or_insert_with(|| Arc::new(Semaphore::new(1)))
                            .clone()
                    };
                    sem.acquire_owned().await.ok()
                } else {
                    None
                };
                self.wait_cooldown().await;
                GatePermit {
                    _global_dl: global_permit,
                    _band_dl: band_permit,
                }
            }
        }
    }

    pub fn report(
        &self,
        status: reqwest::StatusCode,
        retry_after: Option<Duration>,
    ) -> Option<Duration> {
        let Ok(mut g) = self.inner.lock() else {
            return None;
        };

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
        {
            g.consecutive_429s = g.consecutive_429s.saturating_add(1);
            let wait_dur = match retry_after {
                Some(d) => d,
                None => {
                    let shift = (g.consecutive_429s - 1).min(5);
                    let base_s = (30_u64.saturating_mul(1_u64 << shift)).min(900);
                    jittered_duration(base_s * 1000, 20)
                }
            };
            let until_inst = Instant::now() + wait_dur;
            let now_unix = chrono::Utc::now().timestamp();
            let until_ts = now_unix + (wait_dur.as_secs() as i64).max(1);
            let reason = format!("HTTP {}", status.as_u16());
            g.cooldown_until = Some((until_inst, until_ts, reason.clone()));
            drop(g);
            self.emit(RateLimitStatus::Cooldown {
                until: until_ts,
                reason,
            });
            Some(wait_dur)
        } else if status.is_success() {
            let had_cooldown = g.cooldown_until.is_some() || g.consecutive_429s > 0;
            g.consecutive_429s = 0;
            g.cooldown_until = None;
            drop(g);
            if had_cooldown {
                self.emit(RateLimitStatus::Idle);
            }
            None
        } else {
            None
        }
    }

    pub async fn fetch_bytes(
        &self,
        client: &reqwest::Client,
        class: GateClass,
        url: &str,
    ) -> Result<Vec<u8>, crate::error::BcError> {
        let mut last_wait = Duration::from_secs(30);
        for attempt in 0..=5 {
            let _permit = self.acquire(class, None).await;
            let resp = client.get(url).send().await?;
            let status = resp.status();
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_retry_after);

            if let Some(wait_dur) = self.report(status, retry_after) {
                last_wait = wait_dur;
                if attempt == 5 {
                    return Err(crate::error::BcError::RateLimited {
                        retry_after_s: last_wait.as_secs().max(1),
                    });
                }
                continue;
            }

            let resp = resp.error_for_status()?;
            let bytes = resp.bytes().await?;
            return Ok(bytes.to_vec());
        }

        Err(crate::error::BcError::RateLimited {
            retry_after_s: last_wait.as_secs().max(1),
        })
    }
}

pub fn parse_retry_after(header_val: &str) -> Option<Duration> {
    let trimmed = header_val.trim();
    if let Ok(secs) = trimmed.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(trimmed) {
        let now = chrono::Utc::now();
        let diff = dt.with_timezone(&chrono::Utc) - now;
        if diff.num_seconds() > 0 {
            return Some(Duration::from_secs(diff.num_seconds() as u64));
        }
    }
    None
}

fn jittered_duration(base_ms: u64, jitter_pct: u8) -> Duration {
    if base_ms == 0 || jitter_pct == 0 {
        return Duration::from_millis(base_ms);
    }
    let delta = (base_ms as f64) * (f64::from(jitter_pct) / 100.0);
    let min_ms = ((base_ms as f64) - delta).max(0.0);
    let max_ms = (base_ms as f64) + delta;
    let chosen = rand::rng().random_range(min_ms..=max_ms);
    Duration::from_millis(chosen.round() as u64)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn page_pacing_enforces_minimum_gap() {
        tokio::time::pause();
        let gate = BandcampGate::new(1500, 0, 2, None);

        let t0 = Instant::now();
        let _p1 = gate.acquire(GateClass::Page, None).await;
        assert_eq!(t0.elapsed(), Duration::ZERO);

        let _p2 = gate.acquire(GateClass::Page, None).await;
        assert!(t0.elapsed() >= Duration::from_millis(1500));
    }

    #[tokio::test]
    async fn cooldown_blocks_all_traffic_and_emits_events() {
        tokio::time::pause();
        let events = Arc::new(StdMutex::new(Vec::new()));
        let events_clone = events.clone();
        let gate = BandcampGate::new(
            100,
            0,
            2,
            Some(Arc::new(move |st| {
                events_clone.lock().unwrap().push(st);
            })),
        );

        gate.report(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            Some(Duration::from_secs(2)),
        );

        let t0 = Instant::now();
        let _p = gate.acquire(GateClass::Api, None).await;
        assert!(t0.elapsed() >= Duration::from_secs(2));

        let recorded = events.lock().unwrap();
        assert!(
            recorded
                .iter()
                .any(|e| matches!(e, RateLimitStatus::Cooldown { .. }))
        );
    }
}

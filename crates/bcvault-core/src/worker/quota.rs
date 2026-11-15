pub const BULK_CONFIRM_MIN: u32 = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaDecision {
    Allow { count: u32, truncated: bool },
    Warn { count: u32, truncated: bool },
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BulkPreviewRow {
    pub band_id: i64,
    pub requested: u32,
    pub used_30d: u32,
    pub allowed: u32,
    pub deferred: u32,
    pub will_warn: bool,
    pub blocked: bool,
}

pub fn decide(
    used_30d: u32,
    requested: u32,
    batch_limit: u32,
    soft: u32,
    hard: u32,
    ack_soft: bool,
) -> QuotaDecision {
    let remaining = hard.saturating_sub(used_30d);
    if remaining == 0 || requested == 0 {
        return QuotaDecision::Blocked;
    }

    let n = requested.min(batch_limit).min(remaining);
    if n == 0 {
        return QuotaDecision::Blocked;
    }
    let truncated = n < requested;

    if used_30d + n > soft && !ack_soft {
        QuotaDecision::Warn {
            count: n,
            truncated,
        }
    } else {
        QuotaDecision::Allow {
            count: n,
            truncated,
        }
    }
}

/// Preview quota outcomes for multiple artists using `decide`, keeping UI previews aligned with
/// the worker. Each entry in `per_artist` is `(band_id, requested, used_30d)`.
pub fn preview_bulk(
    per_artist: &[(i64, u32, u32)],
    batch_limit: u32,
    soft: u32,
    hard: u32,
    ack_soft: bool,
) -> Vec<BulkPreviewRow> {
    let effective_batch = batch_limit.min(hard);
    per_artist
        .iter()
        .map(|&(band_id, requested, used_30d)| {
            match decide(used_30d, requested, effective_batch, soft, hard, ack_soft) {
                QuotaDecision::Allow { count, .. } => BulkPreviewRow {
                    band_id,
                    requested,
                    used_30d,
                    allowed: count,
                    deferred: requested.saturating_sub(count),
                    will_warn: false,
                    blocked: false,
                },
                QuotaDecision::Warn { count, .. } => BulkPreviewRow {
                    band_id,
                    requested,
                    used_30d,
                    allowed: count,
                    deferred: requested.saturating_sub(count),
                    will_warn: true,
                    blocked: false,
                },
                QuotaDecision::Blocked => BulkPreviewRow {
                    band_id,
                    requested,
                    used_30d,
                    allowed: 0,
                    deferred: requested,
                    will_warn: false,
                    blocked: true,
                },
            }
        })
        .collect()
}

pub fn should_confirm_bulk(is_library_or_multi_artist: bool, preview: &[BulkPreviewRow]) -> bool {
    if is_library_or_multi_artist {
        return true;
    }
    let total_requested: u32 = preview.iter().map(|r| r.requested).sum();
    if total_requested >= BULK_CONFIRM_MIN {
        return true;
    }
    preview
        .iter()
        .any(|r| r.deferred > 0 || r.will_warn || r.blocked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_decisions_table_driven() {
        // Under soft limit, no truncation
        assert_eq!(
            decide(10, 5, 150, 150, 200, false),
            QuotaDecision::Allow {
                count: 5,
                truncated: false
            }
        );

        // Batch limit truncates request
        assert_eq!(
            decide(10, 50, 25, 150, 200, false),
            QuotaDecision::Allow {
                count: 25,
                truncated: true
            }
        );

        // Exactly hitting soft limit (150) does not warn
        assert_eq!(
            decide(140, 10, 150, 150, 200, false),
            QuotaDecision::Allow {
                count: 10,
                truncated: false
            }
        );

        // Crossing soft limit without ack warns
        assert_eq!(
            decide(145, 10, 150, 150, 200, false),
            QuotaDecision::Warn {
                count: 10,
                truncated: false
            }
        );

        // Crossing soft limit with ack allows
        assert_eq!(
            decide(145, 10, 150, 150, 200, true),
            QuotaDecision::Allow {
                count: 10,
                truncated: false
            }
        );

        // At 199 used, batch of 5 is truncated to 1
        assert_eq!(
            decide(199, 5, 150, 150, 200, true),
            QuotaDecision::Allow {
                count: 1,
                truncated: true
            }
        );

        // At 199 used without ack, warns with truncated count = 1
        assert_eq!(
            decide(199, 5, 150, 150, 200, false),
            QuotaDecision::Warn {
                count: 1,
                truncated: true
            }
        );

        // At 200 used (hard cap), blocked
        assert_eq!(decide(200, 1, 150, 150, 200, true), QuotaDecision::Blocked);
        assert_eq!(decide(250, 1, 150, 150, 200, true), QuotaDecision::Blocked);
    }

    #[test]
    fn preview_bulk_matches_decide_grid() {
        let per_artist = [
            (1, 5, 10),   // Allow 5, deferred 0
            (2, 40, 10),  // Batch limit 25 -> Allow 25, deferred 15
            (3, 10, 145), // Crosses soft 150 -> Warn (or Allow if ack)
            (4, 20, 195), // Near hard 200 -> Warn/Allow 5, deferred 15
            (5, 8, 200),  // At hard 200 -> Blocked, deferred 8
        ];
        for ack_soft in [false, true] {
            let rows = preview_bulk(&per_artist, 25, 150, 200, ack_soft);
            assert_eq!(rows.len(), per_artist.len());
            for (row, &(band_id, requested, used_30d)) in rows.iter().zip(per_artist.iter()) {
                assert_eq!(row.band_id, band_id);
                assert_eq!(row.requested, requested);
                assert_eq!(row.used_30d, used_30d);
                match decide(used_30d, requested, 25, 150, 200, ack_soft) {
                    QuotaDecision::Allow { count, .. } => {
                        assert_eq!(row.allowed, count);
                        assert_eq!(row.deferred, requested - count);
                        assert!(!row.will_warn);
                        assert!(!row.blocked);
                    }
                    QuotaDecision::Warn { count, .. } => {
                        assert_eq!(row.allowed, count);
                        assert_eq!(row.deferred, requested - count);
                        assert!(row.will_warn);
                        assert!(!row.blocked);
                    }
                    QuotaDecision::Blocked => {
                        assert_eq!(row.allowed, 0);
                        assert_eq!(row.deferred, requested);
                        assert!(!row.will_warn);
                        assert!(row.blocked);
                    }
                }
            }
        }
    }
}

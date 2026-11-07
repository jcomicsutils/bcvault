#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaDecision {
    Allow { count: u32, truncated: bool },
    Warn { count: u32, truncated: bool },
    Blocked,
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
}

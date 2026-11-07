use crate::types::Classification;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PricingSignals {
    pub minimum_price: Option<f64>,
    pub set_price: Option<f64>,
    pub is_set_price: Option<bool>,
    pub require_email: Option<bool>,
    pub has_free_download_page: bool,
    #[serde(default)]
    pub download_pref: Option<i64>,
    #[serde(default)]
    pub has_nyp_badge: bool,
}

const EPSILON: f64 = 0.005;

pub fn classify(signals: &PricingSignals) -> Classification {
    let Some(min_price) = signals.minimum_price else {
        return Classification::Unknown;
    };

    if min_price > EPSILON {
        return Classification::Paid;
    }

    if signals.require_email == Some(true) {
        return Classification::EmailRequired;
    }

    // Bandcamp sets `download_pref = 2` (PAID) and renders `.buyItemNyp` ("name your price")
    // on NYP releases even when `freeDownloadPage` is populated in `data-tralbum`.
    if signals.has_nyp_badge || signals.download_pref == Some(2) {
        return Classification::Nyp;
    }

    if signals.download_pref == Some(1) {
        return Classification::Free;
    }

    let is_fixed_zero =
        signals.is_set_price == Some(true) && signals.set_price.is_none_or(|sp| sp.abs() < EPSILON);

    if is_fixed_zero {
        return Classification::Free;
    }

    // In Bandcamp's `tralbum.current`, NYP releases have `minimum_price == 0.0`,
    // `is_set_price != true`, and a positive default/suggested `set_price` (e.g. 7.0 or 9.0),
    // even when `freeDownloadPage` is present.
    let has_suggested_nyp_price =
        signals.is_set_price != Some(true) && signals.set_price.is_some_and(|sp| sp > EPSILON);

    if has_suggested_nyp_price {
        return Classification::Nyp;
    }

    if signals.has_free_download_page {
        Classification::Free
    } else {
        Classification::Nyp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_matrix() {
        // Missing minimum_price -> Unknown
        assert_eq!(
            classify(&PricingSignals {
                minimum_price: None,
                ..Default::default()
            }),
            Classification::Unknown
        );

        // Paid (> 0)
        assert_eq!(
            classify(&PricingSignals {
                minimum_price: Some(7.0),
                require_email: Some(true),
                has_free_download_page: true,
                ..Default::default()
            }),
            Classification::Paid
        );

        // Email required (min == 0, require_email == true)
        assert_eq!(
            classify(&PricingSignals {
                minimum_price: Some(0.0),
                require_email: Some(true),
                has_free_download_page: true,
                ..Default::default()
            }),
            Classification::EmailRequired
        );

        // Free via freeDownloadPage
        assert_eq!(
            classify(&PricingSignals {
                minimum_price: Some(0.0),
                require_email: Some(false),
                has_free_download_page: true,
                ..Default::default()
            }),
            Classification::Free
        );

        // Free via fixed price of 0
        assert_eq!(
            classify(&PricingSignals {
                minimum_price: Some(0.0),
                set_price: Some(0.0),
                is_set_price: Some(true),
                require_email: Some(false),
                has_free_download_page: false,
                ..Default::default()
            }),
            Classification::Free
        );

        // NYP (min == 0, buyer can pay more, no freeDownloadPage)
        assert_eq!(
            classify(&PricingSignals {
                minimum_price: Some(0.0),
                set_price: Some(5.0),
                is_set_price: Some(false),
                require_email: Some(false),
                has_free_download_page: false,
                ..Default::default()
            }),
            Classification::Nyp
        );

        // NYP on real Bandcamp page where freeDownloadPage is also populated
        assert_eq!(
            classify(&PricingSignals {
                minimum_price: Some(0.0),
                set_price: Some(9.0),
                is_set_price: None,
                require_email: None,
                has_free_download_page: true,
                download_pref: Some(2),
                has_nyp_badge: true,
            }),
            Classification::Nyp
        );

        // NYP from legacy cached pricing_json (where download_pref was not yet stored)
        assert_eq!(
            classify(&PricingSignals {
                minimum_price: Some(0.0),
                set_price: Some(9.0),
                is_set_price: None,
                require_email: None,
                has_free_download_page: true,
                download_pref: None,
                has_nyp_badge: false,
            }),
            Classification::Nyp
        );
    }
}

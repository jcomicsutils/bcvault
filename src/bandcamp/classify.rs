use crate::types::Classification;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PricingSignals {
    pub minimum_price: Option<f64>,
    pub set_price: Option<f64>,
    pub is_set_price: Option<bool>,
    pub require_email: Option<bool>,
    pub has_free_download_page: bool,
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

    let is_fixed_zero =
        signals.is_set_price == Some(true) && signals.set_price.is_none_or(|sp| sp.abs() < EPSILON);

    if signals.has_free_download_page || is_fixed_zero {
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
            }),
            Classification::Nyp
        );
    }
}

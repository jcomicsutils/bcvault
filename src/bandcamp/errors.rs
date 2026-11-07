use crate::error::BcError;

pub fn map_api_rejection(message: &str) -> BcError {
    let lower = message.to_ascii_lowercase();
    if lower.contains("email") {
        BcError::EmailRequired
    } else if lower.contains("limit") || lower.contains("quota") || lower.contains("exceed") {
        BcError::QuotaExceeded
    } else {
        BcError::ApiRejected(message.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_rejection_heuristics() {
        assert!(matches!(
            map_api_rejection("An email address is required to download this release."),
            BcError::EmailRequired
        ));
        assert!(matches!(
            map_api_rejection("Free download limit exceeded for this artist."),
            BcError::QuotaExceeded
        ));
        assert!(matches!(
            map_api_rejection("Monthly quota reached"),
            BcError::QuotaExceeded
        ));
        assert!(matches!(
            map_api_rejection("Item is not available for free"),
            BcError::ApiRejected(_)
        ));
    }
}

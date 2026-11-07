use crate::error::InputError;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedArtistInput {
    pub base_url: String,
    pub slug: String,
}

const REJECTED_HOSTS: &[&str] = &[
    "bandcamp.com",
    "www.bandcamp.com",
    "daily.bandcamp.com",
    "bcbits.com",
    "f4.bcbits.com",
];

pub fn normalize_artist_input(raw: &str) -> Result<NormalizedArtistInput, InputError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(InputError::Empty);
    }

    // Check if bare slug: ^[a-z0-9][a-z0-9-]{0,62}$
    let lower = trimmed.to_ascii_lowercase();
    if !lower.contains("://") && !lower.contains('.') && !lower.contains('/') {
        if is_valid_slug(&lower) {
            return Ok(NormalizedArtistInput {
                base_url: format!("https://{lower}.bandcamp.com"),
                slug: lower,
            });
        }
        return Err(InputError::InvalidUrl(trimmed.to_string()));
    }

    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    let parsed = Url::parse(&candidate).map_err(|_| InputError::InvalidUrl(trimmed.to_string()))?;

    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(InputError::UnsupportedScheme(other.to_string())),
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| InputError::InvalidUrl(trimmed.to_string()))?
        .to_ascii_lowercase();

    if REJECTED_HOSTS.contains(&host.as_str()) {
        return Err(InputError::NotAnArtistHost(host));
    }

    let slug = if let Some(sub) = host.strip_suffix(".bandcamp.com") {
        if !is_valid_slug(sub) {
            return Err(InputError::InvalidUrl(trimmed.to_string()));
        }
        sub.to_string()
    } else {
        // Custom domain or localhost (for wiremock tests)
        if !host.contains('.') && host != "localhost" && !host.starts_with("127.") {
            return Err(InputError::InvalidUrl(trimmed.to_string()));
        }
        host.clone()
    };

    let base_url = match parsed.port() {
        Some(port) => {
            // Preserve http scheme only for localhost/127.0.0.1 in tests; upgrade everything else to https
            let scheme = if host == "localhost" || host.starts_with("127.") {
                parsed.scheme()
            } else {
                "https"
            };
            format!("{scheme}://{host}:{port}")
        }
        None => format!("https://{host}"),
    };

    Ok(NormalizedArtistInput { base_url, slug })
}

fn is_valid_slug(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 63 {
        return false;
    }
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return false;
    }
    bytes
        .iter()
        .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn normalises_slug_and_urls() {
        let a = normalize_artist_input("artistname").unwrap();
        assert_eq!(a.base_url, "https://artistname.bandcamp.com");
        assert_eq!(a.slug, "artistname");

        let b = normalize_artist_input("https://artistname.bandcamp.com/music").unwrap();
        assert_eq!(b.base_url, "https://artistname.bandcamp.com");
        assert_eq!(b.slug, "artistname");

        let c = normalize_artist_input("http://artistname.bandcamp.com/album/x?from=embed#track1")
            .unwrap();
        assert_eq!(c.base_url, "https://artistname.bandcamp.com");
        assert_eq!(c.slug, "artistname");

        let d = normalize_artist_input("https://music.example.com/").unwrap();
        assert_eq!(d.base_url, "https://music.example.com");
        assert_eq!(d.slug, "music.example.com");
    }

    #[test]
    fn rejects_invalid_inputs() {
        for host in [
            "https://bandcamp.com",
            "https://www.bandcamp.com/tag/ambient",
            "https://daily.bandcamp.com",
            "bandcamp.com",
        ] {
            let err = normalize_artist_input(host).unwrap_err();
            assert!(
                matches!(err, InputError::NotAnArtistHost(_)),
                "Expected NotAnArtistHost for {host}, got {err:?}"
            );
        }

        assert!(matches!(
            normalize_artist_input("   "),
            Err(InputError::Empty)
        ));
        assert!(matches!(
            normalize_artist_input("ftp://artist.bandcamp.com"),
            Err(InputError::UnsupportedScheme(_))
        ));
        assert!(matches!(
            normalize_artist_input("-invalid-slug"),
            Err(InputError::InvalidUrl(_))
        ));
    }
}

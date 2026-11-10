use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Database actor channel closed")]
    Closed,
    #[error("Database schema version {found} is newer than supported version {max_supported}")]
    TooNew { found: i32, max_supported: i32 },
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid enum value in database: {0}")]
    InvalidEnum(String),
    #[error("Schema contract mismatch: {0}")]
    SchemaMismatch(String),
    #[error("Folder link error: {0}")]
    LinkFolder(#[from] LinkFolderError),
}

#[derive(Debug, Error)]
pub enum InputError {
    #[error("Artist input cannot be empty")]
    Empty,
    #[error("Invalid URL or artist slug: {0}")]
    InvalidUrl(String),
    #[error("Unsupported URL scheme '{0}' (expected http or https)")]
    UnsupportedScheme(String),
    #[error("Host '{0}' is a root Bandcamp site, not an artist page")]
    NotAnArtistHost(String),
}

#[derive(Debug, Error)]
pub enum BcError {
    #[error("Input error: {0}")]
    Input(#[from] InputError),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Rate limited after retries (retry after {retry_after_s}s)")]
    RateLimited { retry_after_s: u64 },
    #[error("Could not find band_id on page")]
    BandIdNotFound,
    #[error("Could not find data-tralbum attribute on release page")]
    TralbumNotFound,
    #[error("Failed to parse JSON payload: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid redeem URL: {0}")]
    InvalidRedeemUrl(String),
    #[error("Redeem link expired or contains no download items")]
    RedeemExpiredOrEmpty,
    #[error("API rejected request: {0}")]
    ApiRejected(String),
    #[error("Email required for download")]
    EmailRequired,
    #[error("Artist download quota exceeded")]
    QuotaExceeded,
    #[error("Requested format unavailable")]
    FormatUnavailable,
    #[error("Unexpected response from Bandcamp: {0}")]
    UnexpectedResponse(String),
}

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("Bandcamp error: {0}")]
    Bandcamp(#[from] BcError),
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Zip extraction error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("Unsafe or invalid zip entry path: {0}")]
    UnsafeZipEntry(String),
    #[error("Zip archive exceeds safety limits: {0}")]
    ZipLimitExceeded(String),
    #[error("Download truncated: expected {expected} bytes, received {actual}")]
    Truncated { expected: u64, actual: u64 },
    #[error("Download cancelled")]
    Cancelled,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LinkFolderError {
    #[error("Path does not exist: {0}")]
    NotFound(std::path::PathBuf),
    #[error("Path is not a directory: {0}")]
    NotADirectory(std::path::PathBuf),
    #[error("Path is not readable: {0}")]
    Unreadable(std::path::PathBuf),
    #[error("Folder overlaps with output directory: {0}")]
    OverlapsOutDir(std::path::PathBuf),
    #[error("Folder overlaps with existing linked folder for artist {band_id}: {path}")]
    OverlapsExisting {
        band_id: i64,
        path: std::path::PathBuf,
    },
    #[error("already covered by {path}")]
    AlreadyCovered { path: std::path::PathBuf },
    #[error("contains {path}; unlink that first")]
    ContainsOwnFolder { path: std::path::PathBuf },
    #[error("Database error: {0}")]
    Db(String),
}

impl From<rusqlite::Error> for LinkFolderError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e.to_string())
    }
}

impl From<DbError> for LinkFolderError {
    fn from(e: DbError) -> Self {
        Self::Db(e.to_string())
    }
}

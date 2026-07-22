use thiserror::Error;

#[derive(Debug, Error)]
pub enum DriveError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error {status}: {message}")]
    Api { status: u16, message: String },
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Auth error: {0}")]
    Auth(#[from] tuxdrive_auth::AuthError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Rate limited, retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    #[error("Not found: {0}")]
    NotFound(String),
}

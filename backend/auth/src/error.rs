use thiserror::Error;

/// Top-level error type for the `tuxdrive-auth` crate.
#[derive(Debug, Error)]
pub enum AuthError {
    /// General OAuth protocol error (e.g. `access_denied`, `invalid_grant`).
    #[error("OAuth error: {0}")]
    OAuthError(String),

    /// The access token has expired and a refresh attempt failed or was not
    /// possible (no refresh token present).
    #[error("token is expired and could not be refreshed")]
    TokenExpired,

    /// No token record was found for the requested account.
    #[error("no token found for account")]
    NoToken,

    /// An error originating from the OS secret store (keyring).
    #[error("keyring error: {0}")]
    KeyringError(#[from] keyring::Error),

    /// An HTTP-level error from `reqwest`.
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),

    /// JSON serialisation / deserialisation error.
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// The redirect URI provided in the OAuth config is invalid.
    #[error("invalid redirect URI")]
    InvalidRedirect,

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

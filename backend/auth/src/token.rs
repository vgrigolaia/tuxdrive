use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A complete set of OAuth2 tokens associated with one Google account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    /// The short-lived Bearer token used to call Google APIs.
    pub access_token: String,

    /// The long-lived token used to obtain a new `access_token`.
    /// `None` when the authorisation server did not return one (e.g. implicit
    /// flow or device-flow without `offline` scope).
    pub refresh_token: Option<String>,

    /// UTC instant after which the `access_token` must be considered invalid.
    pub expires_at: DateTime<Utc>,

    /// Token type — almost always `"Bearer"`.
    pub token_type: String,

    /// Space-separated list of scopes granted by the user.
    pub scope: String,

    /// The Google account e-mail address that owns these tokens.
    pub account_email: String,
}

impl TokenSet {
    /// Returns `true` when the access token is expired or will expire within
    /// the next 60 seconds (a safety buffer to avoid using a token that expires
    /// in transit).
    pub fn is_expired(&self) -> bool {
        let buffer = chrono::Duration::seconds(60);
        Utc::now() + buffer >= self.expires_at
    }
}

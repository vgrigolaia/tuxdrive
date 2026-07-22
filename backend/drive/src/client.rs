use std::sync::Arc;

use tracing::warn;

use crate::error::DriveError;

pub(crate) const DRIVE_API: &str = "https://www.googleapis.com/drive/v3";
pub(crate) const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3";

pub struct DriveClient {
    pub http: reqwest::Client,
    pub(crate) auth: Arc<tuxdrive_auth::AuthManager>,
    /// Shared with the daemon's account-email state so a login completed
    /// after this client was constructed is picked up on the next call.
    pub account_email: Arc<parking_lot::RwLock<String>>,
}

impl DriveClient {
    pub fn new(auth: Arc<tuxdrive_auth::AuthManager>, account_email: Arc<parking_lot::RwLock<String>>) -> Self {
        Self {
            http: reqwest::Client::new(),
            auth,
            account_email,
        }
    }

    pub(crate) async fn get_token(&self) -> Result<String, DriveError> {
        let email = self.account_email.read().clone();
        let token = self.auth.get_valid_token(&email).await?;
        Ok(token)
    }

    pub(crate) async fn check_response(
        resp: reqwest::Response,
    ) -> Result<reqwest::Response, DriveError> {
        let status = resp.status();

        if status.as_u16() == 429 {
            // Try to extract Retry-After header; fall back to 60 s.
            let retry_after_secs = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(60);
            warn!(retry_after_secs, "Drive API rate limited");
            return Err(DriveError::RateLimited { retry_after_secs });
        }

        if status.is_client_error() || status.is_server_error() {
            let status_code = status.as_u16();
            // Attempt to read JSON error body; fall back to plain text.
            let body = resp.text().await.unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .map(str::to_owned)
                })
                .unwrap_or(body);
            return Err(DriveError::Api {
                status: status_code,
                message,
            });
        }

        Ok(resp)
    }
}

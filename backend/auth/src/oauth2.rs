use chrono::Utc;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::Duration;
use tracing::info;
use uuid::Uuid;

use crate::{AuthError, TokenSet};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const SCOPES: &str =
    "https://www.googleapis.com/auth/drive email profile openid";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// OAuth2 application credentials.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

// ---------------------------------------------------------------------------
// Internal token-endpoint response
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawTokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    token_type: Option<String>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    email: String,
}

// ---------------------------------------------------------------------------
// Loopback redirect server
// ---------------------------------------------------------------------------

/// Binds to `127.0.0.1:0`, generates the Google authorization URL, and waits
/// for the browser redirect that delivers the authorization code.
pub struct LoopbackServer {
    listener: TcpListener,
    pub auth_url: String,
    pub redirect_uri: String,
    state: String,
}

impl LoopbackServer {
    /// Block until Google redirects to this server with an authorization code.
    ///
    /// Times out after 5 minutes.  Sends a "Login successful" HTML page to the
    /// browser before returning.
    pub async fn wait_for_code(self) -> Result<String, AuthError> {
        tokio::time::timeout(Duration::from_secs(300), async move {
            loop {
                let (mut socket, _) = self.listener.accept().await.map_err(|e| {
                    AuthError::OAuthError(format!("loopback accept: {e}"))
                })?;

                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);

                let first_line = request.lines().next().unwrap_or("");
                let path = first_line.split_whitespace().nth(1).unwrap_or("/");
                let query = path.split('?').nth(1).unwrap_or("");

                let mut code = None;
                let mut returned_state = None;
                for pair in query.split('&') {
                    let mut kv = pair.splitn(2, '=');
                    match (kv.next(), kv.next()) {
                        (Some("code"), Some(v)) => code = Some(url_decode(v)),
                        (Some("state"), Some(v)) => returned_state = Some(url_decode(v)),
                        _ => {}
                    }
                }

                if let Some(code_val) = code {
                    if returned_state.as_deref() != Some(self.state.as_str()) {
                        let _ = socket
                            .write_all(
                                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n",
                            )
                            .await;
                        return Err(AuthError::OAuthError("OAuth state mismatch".into()));
                    }
                    let body = b"<html><body><h2>Login successful!</h2>\
                        <p>You can close this tab and return to the terminal.</p>\
                        </body></html>";
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body).await;
                    return Ok(code_val);
                }

                // favicon / other browser noise — 404 and keep listening
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\
                          Connection: close\r\n\r\n",
                    )
                    .await;
            }
        })
        .await
        .map_err(|_| AuthError::OAuthError("login timed out (5 minutes)".into()))?
    }
}

/// Minimal percent-decoding for OAuth redirect query parameters.
fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(char::from(h * 16 + l));
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { ' ' } else { char::from(bytes[i]) });
        i += 1;
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// OAuthClient
// ---------------------------------------------------------------------------

pub struct OAuthClient {
    config: OAuthConfig,
    http: reqwest::Client,
}

impl OAuthClient {
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Loopback redirect flow
    // -----------------------------------------------------------------------

    /// Bind a loopback server and build the Google authorization URL.
    ///
    /// Show `server.auth_url` to the user (or open it with `xdg-open`), then
    /// call `server.wait_for_code()` to receive the authorization code.
    pub async fn start_loopback_server(&self) -> Result<LoopbackServer, AuthError> {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| {
            AuthError::OAuthError(format!("failed to bind loopback server: {e}"))
        })?;
        let port = listener
            .local_addr()
            .map_err(|e| AuthError::OAuthError(format!("failed to get port: {e}")))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}");
        let state = Uuid::new_v4().to_string().replace('-', "");

        let mut url =
            url::Url::parse(GOOGLE_AUTH_URL).expect("static GOOGLE_AUTH_URL is valid");
        url.query_pairs_mut()
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", SCOPES)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent")
            .append_pair("state", &state);

        Ok(LoopbackServer {
            listener,
            auth_url: url.to_string(),
            redirect_uri,
            state,
        })
    }

    /// Exchange an authorization code (from the loopback redirect) for tokens.
    pub async fn exchange_code(
        &self,
        code: &str,
        redirect_uri: &str,
    ) -> Result<TokenSet, AuthError> {
        let params = [
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ];

        let resp = self
            .http
            .post(GOOGLE_TOKEN_URL)
            .form(&params)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;

        if !status.is_success() {
            return Err(AuthError::OAuthError(format!(
                "code exchange failed ({status}): {body}"
            )));
        }

        let raw: RawTokenResponse = serde_json::from_str(&body)?;
        if let Some(ref err) = raw.error {
            return Err(AuthError::OAuthError(
                raw.error_description.unwrap_or_else(|| err.clone()),
            ));
        }

        self.raw_to_token_set(raw, "")
    }

    // -----------------------------------------------------------------------
    // Refresh token grant
    // -----------------------------------------------------------------------

    pub async fn refresh_token(&self, refresh_token: &str) -> Result<TokenSet, AuthError> {
        let params = [
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];

        let resp = self
            .http
            .post(GOOGLE_TOKEN_URL)
            .form(&params)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;

        if !status.is_success() {
            return Err(AuthError::OAuthError(format!(
                "token refresh failed ({status}): {body}"
            )));
        }

        let raw: RawTokenResponse = serde_json::from_str(&body)?;
        if let Some(ref err) = raw.error {
            return Err(AuthError::OAuthError(
                raw.error_description.unwrap_or_else(|| err.clone()),
            ));
        }

        let mut token = self.raw_to_token_set(raw, "")?;
        if token.refresh_token.is_none() {
            token.refresh_token = Some(refresh_token.to_owned());
        }
        info!("access token refreshed");
        Ok(token)
    }

    // -----------------------------------------------------------------------
    // Userinfo
    // -----------------------------------------------------------------------

    pub async fn get_user_email(&self, access_token: &str) -> Result<String, AuthError> {
        let resp = self
            .http
            .get("https://www.googleapis.com/oauth2/v2/userinfo")
            .bearer_auth(access_token)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await?;
            return Err(AuthError::OAuthError(format!(
                "userinfo request failed ({status}): {body}"
            )));
        }

        let info: UserInfo = resp.json().await?;
        info!(email = %info.email, "fetched user email");
        Ok(info.email)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn raw_to_token_set(
        &self,
        raw: RawTokenResponse,
        email: &str,
    ) -> Result<TokenSet, AuthError> {
        let access_token = raw.access_token.ok_or_else(|| {
            AuthError::OAuthError("token response missing access_token".into())
        })?;
        let expires_in = raw.expires_in.unwrap_or(3600);
        let expires_at = Utc::now() + chrono::Duration::seconds(expires_in as i64);
        Ok(TokenSet {
            access_token,
            refresh_token: raw.refresh_token,
            expires_at,
            token_type: raw.token_type.unwrap_or_else(|| "Bearer".into()),
            scope: raw.scope.unwrap_or_else(|| SCOPES.into()),
            account_email: email.to_owned(),
        })
    }
}

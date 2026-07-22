use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tracing::{error, info, warn};

use crate::{AuthError, LoopbackServer, OAuthClient, TokenSet, TokenStore};

/// High-level authentication manager.
///
/// Combines an [`OAuthClient`] for obtaining / refreshing tokens with a
/// [`TokenStore`] for persistent storage and an in-process cache so that
/// repeat calls within the same process do not hit the keyring on every
/// request.
pub struct AuthManager {
    oauth: OAuthClient,
    store: Arc<dyn TokenStore>,
    /// In-process cache: `email -> TokenSet`.
    cache: RwLock<HashMap<String, TokenSet>>,
}

impl AuthManager {
    /// Create a new [`AuthManager`].
    pub fn new(oauth: OAuthClient, store: Arc<dyn TokenStore>) -> Self {
        Self {
            oauth,
            store,
            cache: RwLock::new(HashMap::new()),
        }
    }

    // -----------------------------------------------------------------------
    // Login — loopback redirect flow
    // -----------------------------------------------------------------------

    /// Bind the loopback server and build the authorization URL.
    ///
    /// Fast and non-blocking — the returned [`LoopbackServer`] must be passed
    /// to [`Self::complete_login`] to actually wait for the browser redirect.
    /// Split out from [`Self::login`] so callers (e.g. the daemon's IPC layer)
    /// can hand the URL back to a caller immediately instead of blocking for
    /// up to 5 minutes before responding.
    pub async fn start_login(&self) -> Result<LoopbackServer, AuthError> {
        self.oauth.start_loopback_server().await
    }

    /// Block (up to 5 minutes) waiting for the browser redirect on an
    /// already-started loopback server. Returns the redirect URI (needed by
    /// [`Self::exchange_and_save`]) and the authorization code.
    ///
    /// Split out from the old monolithic `complete_login` so callers (e.g.
    /// the daemon's IPC layer) can distinguish "still waiting on the user's
    /// browser" from "actively exchanging the code" instead of eagerly
    /// reporting the latter before the former has actually happened.
    pub async fn await_browser_redirect(
        &self,
        server: LoopbackServer,
    ) -> Result<(String, String), AuthError> {
        let redirect_uri = server.redirect_uri.clone();
        let code = server.wait_for_code().await?;
        Ok((redirect_uri, code))
    }

    /// Exchange an authorization code for a token, resolve the account
    /// email, and persist the result to the token store and in-process
    /// cache. Fast — call after [`Self::await_browser_redirect`] resolves.
    pub async fn exchange_and_save(
        &self,
        redirect_uri: &str,
        code: &str,
    ) -> Result<TokenSet, AuthError> {
        let mut token = self.oauth.exchange_code(code, redirect_uri).await?;

        match self.oauth.get_user_email(&token.access_token).await {
            Ok(email) => {
                token.account_email = email.clone();
                info!(email = %email, "login successful");
            }
            Err(e) => {
                warn!(error = %e, "could not fetch user email after login");
            }
        }

        self.store.save(&token).await?;
        self.cache
            .write()
            .insert(token.account_email.clone(), token.clone());

        Ok(token)
    }

    /// Wait for the browser redirect on an already-started loopback server,
    /// exchange the code for a token, resolve the account email, and persist
    /// the result to the token store and in-process cache.
    pub async fn complete_login(&self, server: LoopbackServer) -> Result<TokenSet, AuthError> {
        let (redirect_uri, code) = self.await_browser_redirect(server).await?;
        self.exchange_and_save(&redirect_uri, &code).await
    }

    /// Start an interactive browser-based login flow.
    ///
    /// Binds a local HTTP server, prints the authorization URL (and tries to
    /// open it with `xdg-open`), then waits for Google to redirect back with
    /// the authorization code.  On success the resulting [`TokenSet`] is saved
    /// to the store and returned.
    ///
    /// This is the terminal (`tuxdrive-daemon login`) entry point; it composes
    /// [`Self::start_login`] and [`Self::complete_login`] with the CLI's
    /// printed prompts. GUI callers should use those two methods directly.
    pub async fn login(&self) -> Result<TokenSet, AuthError> {
        let server = self.start_login().await?;

        println!("\nOpen this URL in your browser to log in:\n");
        println!("  {}\n", server.auth_url);
        info!(auth_url = %server.auth_url, "loopback login started");

        // Try to open the browser automatically; ignore errors (headless envs).
        let _ = std::process::Command::new("xdg-open")
            .arg(&server.auth_url)
            .spawn();

        println!("Waiting for browser login (5-minute timeout)...\n");

        self.complete_login(server).await
    }

    // -----------------------------------------------------------------------
    // Get a valid (non-expired) access token
    // -----------------------------------------------------------------------

    /// Return a valid access token for `email`, refreshing automatically when
    /// needed.
    ///
    /// Lookup order:
    /// 1. In-process cache (fast path).
    /// 2. Persistent store (keyring).
    ///
    /// If the cached / stored token is expired and a refresh token is present,
    /// the token is refreshed, the cache and store are updated, and the new
    /// access token is returned.
    pub async fn get_valid_token(&self, email: &str) -> Result<String, AuthError> {
        // --- Step 1: read from cache ----------------------------------------
        // Clone out whatever we have (or None) while holding only a read lock.
        let cached: Option<TokenSet> = self.cache.read().get(email).cloned();

        let token: TokenSet = match cached {
            // Fast path: token is in cache and still valid.
            Some(ref t) if !t.is_expired() => return Ok(t.access_token.clone()),

            // Cache hit but expired — use the cached entry for the refresh attempt.
            Some(t) => t,

            // Not in cache at all — load from the persistent store.
            None => match self.store.load(email).await {
                Ok(t) => {
                    // Warm the cache so the next call skips the keyring.
                    self.cache.write().insert(email.to_owned(), t.clone());
                    // Not expired: return immediately.
                    if !t.is_expired() {
                        return Ok(t.access_token.clone());
                    }
                    t
                }
                Err(AuthError::NoToken) => return Err(AuthError::NoToken),
                Err(e) => return Err(e),
            },
        };

        // --- Step 2: token is expired — attempt refresh ---------------------
        let refresh = token.refresh_token.as_deref().ok_or_else(|| {
            warn!(email = %email, "token expired and no refresh token available");
            AuthError::TokenExpired
        })?;

        let mut refreshed = self.oauth.refresh_token(refresh).await.map_err(|e| {
            error!(email = %email, error = %e, "token refresh failed");
            AuthError::TokenExpired
        })?;

        // Preserve the email; the refresh response may not echo it back.
        refreshed.account_email = email.to_owned();

        // Update persistent store and in-process cache atomically (best-effort).
        self.store.save(&refreshed).await?;
        let access = refreshed.access_token.clone();
        self.cache.write().insert(email.to_owned(), refreshed);

        info!(email = %email, "access token refreshed and cached");
        Ok(access)
    }

    // -----------------------------------------------------------------------
    // Logout
    // -----------------------------------------------------------------------

    /// Remove all stored credentials for `email`.
    ///
    /// Clears both the in-process cache and the persistent store entry.
    pub async fn logout(&self, email: &str) -> Result<(), AuthError> {
        self.cache.write().remove(email);
        self.store.delete(email).await?;
        info!(email = %email, "logged out");
        Ok(())
    }
}

use async_trait::async_trait;
use keyring::Entry;
use tracing::{info, warn};

use crate::{AuthError, TokenSet};

/// The keyring service name used for every secret entry.
const SERVICE: &str = "tuxdrive";

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Async interface for persisting, loading, and removing [`TokenSet`]s.
#[async_trait]
pub trait TokenStore: Send + Sync {
    /// Persist `token` in the secret store, keyed by its account e-mail.
    async fn save(&self, token: &TokenSet) -> Result<(), AuthError>;

    /// Retrieve the [`TokenSet`] previously saved for `email`.
    ///
    /// Returns [`AuthError::NoToken`] when no entry exists.
    async fn load(&self, email: &str) -> Result<TokenSet, AuthError>;

    /// Remove the [`TokenSet`] for `email` from the secret store.
    ///
    /// This is a no-op if no entry exists (returns `Ok(())`).
    async fn delete(&self, email: &str) -> Result<(), AuthError>;
}

// ---------------------------------------------------------------------------
// KeyringTokenStore
// ---------------------------------------------------------------------------

/// [`TokenStore`] implementation backed by the OS secret store via the
/// `keyring` crate (libsecret / Keychain / Windows Credential Manager).
///
/// Tokens are JSON-serialised and stored under the key
/// `"tuxdrive:{email}"` within the `"tuxdrive"` service namespace.
#[derive(Debug, Default, Clone)]
pub struct KeyringTokenStore;

impl KeyringTokenStore {
    /// Construct a new [`KeyringTokenStore`].
    pub fn new() -> Self {
        Self
    }

    /// Build the keyring username from an e-mail address.
    ///
    /// Using a prefixed key lets the keyring entry be identified easily via
    /// system tools and avoids clashing with other services sharing the same
    /// keyring namespace.
    fn key(email: &str) -> String {
        format!("tuxdrive:{email}")
    }
}

#[async_trait]
impl TokenStore for KeyringTokenStore {
    async fn save(&self, token: &TokenSet) -> Result<(), AuthError> {
        let key = Self::key(&token.account_email);
        let json = serde_json::to_string(token)?;
        let entry = Entry::new(SERVICE, &key)?;
        entry.set_password(&json)?;
        info!(email = %token.account_email, "token saved to keyring");
        Ok(())
    }

    async fn load(&self, email: &str) -> Result<TokenSet, AuthError> {
        let key = Self::key(email);
        let entry = Entry::new(SERVICE, &key)?;
        match entry.get_password() {
            Ok(json) => {
                let token: TokenSet = serde_json::from_str(&json)?;
                info!(email = %email, "token loaded from keyring");
                Ok(token)
            }
            Err(keyring::Error::NoEntry) => {
                warn!(email = %email, "no token entry found in keyring");
                Err(AuthError::NoToken)
            }
            Err(e) => Err(AuthError::KeyringError(e)),
        }
    }

    async fn delete(&self, email: &str) -> Result<(), AuthError> {
        let key = Self::key(email);
        let entry = Entry::new(SERVICE, &key)?;
        match entry.delete_credential() {
            Ok(()) => {
                info!(email = %email, "token deleted from keyring");
                Ok(())
            }
            // Already gone — treat as success.
            Err(keyring::Error::NoEntry) => {
                warn!(email = %email, "attempted to delete non-existent keyring entry");
                Ok(())
            }
            Err(e) => Err(AuthError::KeyringError(e)),
        }
    }
}

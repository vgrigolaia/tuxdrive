use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Bundled OAuth2 credentials
//
// These are compiled in at build time via environment variables:
//   TUXDRIVE_CLIENT_ID
//   TUXDRIVE_CLIENT_SECRET
//
// When distributing the binary to end users they never need to touch these.
// Users who build from source must supply their own credentials (see README).
//
// For a desktop/installed app, Google considers the client_secret "not secret"
// (it is a "public client"); the real security comes from the per-user OAuth
// tokens stored in the OS keyring, not from keeping the client_secret hidden.
// ---------------------------------------------------------------------------
const BUNDLED_CLIENT_ID: &str = env!(
    "TUXDRIVE_CLIENT_ID",
    "Set TUXDRIVE_CLIENT_ID=<your-client-id> when running cargo build"
);
const BUNDLED_CLIENT_SECRET: &str = env!(
    "TUXDRIVE_CLIENT_SECRET",
    "Set TUXDRIVE_CLIENT_SECRET=<your-client-secret> when running cargo build"
);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub sync: SyncConfig,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
}

/// OAuth2 credentials section — entirely optional in `config.toml`.
///
/// When absent the binary's compiled-in credentials are used, which is the
/// normal path for end users.  Power users (e.g. corporate) can override with
/// their own Google Cloud project by adding `[auth]` to their config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

impl AuthConfig {
    /// Return the effective client_id: config override → bundled default.
    pub fn effective_client_id(&self) -> &str {
        self.client_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(BUNDLED_CLIENT_ID)
    }

    /// Return the effective client_secret: config override → bundled default.
    pub fn effective_client_secret(&self) -> &str {
        self.client_secret
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(BUNDLED_CLIENT_SECRET)
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            client_id: None,
            client_secret: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// May contain `~` which is expanded at runtime.
    pub local_root: String,
    pub poll_interval_secs: u64,
    pub chunk_size_bytes: u64,
    pub max_concurrent_transfers: usize,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            local_root: "~/TuxDrive".to_string(),
            poll_interval_secs: 30,
            chunk_size_bytes: 8 * 1024 * 1024,
            max_concurrent_transfers: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// One of: "trace", "debug", "info", "warn", "error"
    pub level: String,
    /// Optional path to a log file; if absent, logging goes to stderr only.
    pub file: Option<String>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            file: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Unix socket path. Default: `~/.local/share/tuxdrive/daemon.sock`
    pub socket_path: Option<String>,
    /// SQLite database path. Default: `~/.local/share/tuxdrive/tuxdrive.db`
    pub db_path: Option<String>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: None,
            db_path: None,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auth: AuthConfig::default(),
            sync: SyncConfig::default(),
            log: LogConfig::default(),
            daemon: DaemonConfig::default(),
        }
    }
}

impl Config {
    fn config_path() -> PathBuf {
        Self::expand_path("~/.config/tuxdrive/config.toml")
    }

    /// Load config from `~/.config/tuxdrive/config.toml`, falling back to
    /// defaults if the file is absent.  Returns an error only if the file
    /// exists but cannot be parsed.
    pub fn load() -> anyhow::Result<Self> {
        let config_path = Self::config_path();

        if !config_path.exists() {
            return Ok(Config::default());
        }

        let content = std::fs::read_to_string(&config_path)?;
        let config: Config = toml::from_str(&content)?;
        tracing::info!(path = %config_path.display(), "loaded config");
        Ok(config)
    }

    /// Persist this config back to `~/.config/tuxdrive/config.toml`.
    pub fn save(&self) -> anyhow::Result<()> {
        let config_path = Self::config_path();
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;
        tracing::info!(path = %config_path.display(), "saved config");
        Ok(())
    }

    /// Expand a leading `~` or `~/` in `path` using [`dirs::home_dir`].
    pub fn expand_path(path: &str) -> PathBuf {
        if path == "~" {
            return dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        }
        if let Some(rest) = path.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(rest);
            }
        }
        PathBuf::from(path)
    }

    /// Resolved local sync root path (with `~` expanded).
    pub fn sync_root(&self) -> PathBuf {
        Self::expand_path(&self.sync.local_root)
    }

    /// Resolved Unix socket path.
    pub fn socket_path(&self) -> PathBuf {
        match &self.daemon.socket_path {
            Some(p) => Self::expand_path(p),
            None => Self::expand_path("~/.local/share/tuxdrive/daemon.sock"),
        }
    }

    /// Resolved SQLite database path.
    pub fn db_path(&self) -> PathBuf {
        match &self.daemon.db_path {
            Some(p) => Self::expand_path(p),
            None => Self::expand_path("~/.local/share/tuxdrive/tuxdrive.db"),
        }
    }
}

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

use crate::login_flow::{self, ConflictAction, LoginState};
use crate::sync_resolve::ConflictSummary;

// ---------------------------------------------------------------------------
// Protocol types
// ---------------------------------------------------------------------------

/// Commands sent from a frontend client to the daemon.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum IpcCommand {
    GetStatus,
    Pause,
    Resume,
    Logout { email: String },
    ListFiles { folder_path: String },
    GetLogs { lines: usize },
    Shutdown,
    /// Begin a GUI-driven OAuth login. Returns immediately with the URL to
    /// open in a browser; poll `GetLoginStatus` for progress.
    StartLogin,
    GetLoginStatus,
    /// Resolve a pending sync-history conflict surfaced via `GetLoginStatus`.
    ResolveSyncConflict { action: ConflictAction },
    /// Abort an in-progress login (e.g. the user closed the browser tab).
    CancelLogin,
    /// Return the current sync folder location.
    GetSyncSettings,
    /// Relocate the local sync folder. Moves all existing synced files to
    /// `path` (refusing if it's non-empty), persists the new location to
    /// config.toml, then restarts the daemon so it picks up the new root.
    SetSyncFolder { path: String },
}

/// Responses produced by the daemon and sent back to the client.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcResponse {
    Status {
        status: String,
        queued: usize,
        account_email: String,
        paused: bool,
        /// Estimated seconds remaining to drain the queue, based on recent
        /// throughput. `None` when there's not enough recent history yet.
        #[serde(skip_serializing_if = "Option::is_none")]
        eta_seconds: Option<u64>,
    },
    Paused,
    Resumed,
    LoggedOut,
    Files {
        entries: Vec<FileEntry>,
    },
    Logs {
        lines: Vec<String>,
    },
    ShuttingDown,
    Error {
        message: String,
    },
    LoginStarted {
        auth_url: String,
    },
    LoginStatus {
        /// One of: idle, awaiting_browser, exchanging_code, conflict_pending,
        /// resolving_conflict, complete, failed.
        phase: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        account_email: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        known_count: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        missing_count: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        missing_paths: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        resolved_done: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        resolved_total: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    SyncConflictResolving,
    LoginCancelled,
    SyncSettings {
        local_root: String,
    },
    /// Files moved and config saved; the daemon is about to restart itself
    /// to apply the new sync root.
    SyncFolderChanged {
        local_root: String,
    },
}

#[derive(Debug, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub relative_path: String,
    pub size: i64,
    pub sync_status: String,
    pub is_folder: bool,
}

// ---------------------------------------------------------------------------
// Shared daemon state
// ---------------------------------------------------------------------------

/// State shared across the daemon main loop and every IPC connection handler.
pub struct DaemonState {
    pub sync_engine: Arc<tuxdrive_sync::SyncEngine>,
    pub scheduler: Arc<tuxdrive_scheduler::Scheduler>,
    pub db: Arc<tuxdrive_database::Database>,
    pub auth: Arc<tuxdrive_auth::AuthManager>,
    pub drive: Arc<tuxdrive_drive::DriveClient>,
    pub cfg: crate::config::Config,
    /// Shared with `DriveClient` and the `Scheduler` so a login completed via
    /// IPC after the daemon started is picked up immediately by both.
    pub account_email: Arc<RwLock<String>>,
    pub paused: RwLock<bool>,
    /// Circular buffer of recent log lines (newest at the back).
    pub log_buffer: RwLock<VecDeque<String>>,
    /// Progress of an in-flight (or just-completed) GUI-driven login.
    pub login_state: RwLock<LoginState>,
    /// Full conflict details stashed between `ConflictPending` and the
    /// `ResolveSyncConflict` call that consumes them.
    pub pending_conflict: RwLock<Option<ConflictSummary>>,
    /// The background task running the current login, if any — aborted by
    /// `CancelLogin`.
    pub login_task: RwLock<Option<tokio::task::JoinHandle<()>>>,
}

impl DaemonState {
    pub fn new(
        sync_engine: Arc<tuxdrive_sync::SyncEngine>,
        scheduler: Arc<tuxdrive_scheduler::Scheduler>,
        db: Arc<tuxdrive_database::Database>,
        auth: Arc<tuxdrive_auth::AuthManager>,
        drive: Arc<tuxdrive_drive::DriveClient>,
        cfg: crate::config::Config,
        account_email: Arc<RwLock<String>>,
    ) -> Self {
        Self {
            sync_engine,
            scheduler,
            db,
            auth,
            drive,
            cfg,
            account_email,
            paused: RwLock::new(false),
            log_buffer: RwLock::new(VecDeque::with_capacity(1000)),
            login_state: RwLock::new(LoginState::default()),
            pending_conflict: RwLock::new(None),
            login_task: RwLock::new(None),
        }
    }

    /// Append a line to the rolling log buffer (capped at 1 000 entries).
    pub fn push_log(&self, line: String) {
        let mut buf = self.log_buffer.write();
        if buf.len() >= 1000 {
            buf.pop_front();
        }
        buf.push_back(line);
    }
}

// ---------------------------------------------------------------------------
// IPC server
// ---------------------------------------------------------------------------

pub struct IpcServer {
    socket_path: PathBuf,
    state: Arc<DaemonState>,
}

impl IpcServer {
    pub fn new(socket_path: PathBuf, state: Arc<DaemonState>) -> Self {
        Self { socket_path, state }
    }

    /// Accept connections until `shutdown` is notified.
    ///
    /// Each connection is handled concurrently in its own Tokio task.
    pub async fn run(self, shutdown: Arc<tokio::sync::Notify>) -> anyhow::Result<()> {
        // Remove a stale socket file left over from a previous run.
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        // Ensure the parent directory exists.
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;

        // Set restrictive permissions: owner read/write only (0o600).
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&self.socket_path, perms)?;
        }

        info!(socket = %self.socket_path.display(), "IPC server listening");

        // Pin a shutdown future that persists across select! iterations so
        // that a notification fired while we're inside the accept() branch
        // is not missed.
        let shutdown_signal = shutdown.notified();
        tokio::pin!(shutdown_signal);

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, _addr)) => {
                            let state = Arc::clone(&self.state);
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, state).await {
                                    warn!("IPC connection error: {e}");
                                }
                            });
                        }
                        Err(e) => {
                            error!("IPC accept error: {e}");
                        }
                    }
                }
                _ = &mut shutdown_signal => {
                    info!("IPC server shutting down");
                    break;
                }
            }
        }

        // Clean up the socket file on graceful shutdown.
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Per-connection handler
// ---------------------------------------------------------------------------

async fn handle_connection(
    stream: UnixStream,
    state: Arc<DaemonState>,
) -> anyhow::Result<()> {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut lines = BufReader::new(read_half).lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_owned();
        if line.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<IpcCommand>(&line) {
            Ok(cmd) => dispatch_command(cmd, Arc::clone(&state)).await,
            Err(e) => IpcResponse::Error {
                message: format!("invalid command JSON: {e}"),
            },
        };

        let mut json = serde_json::to_string(&response)?;
        json.push('\n');
        write_half.write_all(json.as_bytes()).await?;

        // After ShuttingDown the client knows the daemon is going away.
        if matches!(response, IpcResponse::ShuttingDown) {
            break;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

async fn dispatch_command(cmd: IpcCommand, state: Arc<DaemonState>) -> IpcResponse {
    match cmd {
        // ---- Status ----------------------------------------------------------
        IpcCommand::GetStatus => {
            let paused = *state.paused.read();
            let queued = state.sync_engine.queue_len();
            let account_email = state.account_email.read().clone();
            let eta_seconds = state.sync_engine.eta_seconds();

            let status = if paused {
                "paused".to_string()
            } else if queued > 0 {
                "syncing".to_string()
            } else {
                "synced".to_string()
            };

            IpcResponse::Status {
                status,
                queued,
                account_email,
                paused,
                eta_seconds,
            }
        }

        // ---- Pause / Resume -------------------------------------------------
        IpcCommand::Pause => {
            state.scheduler.pause();
            *state.paused.write() = true;
            info!("sync paused via IPC");
            IpcResponse::Paused
        }

        IpcCommand::Resume => {
            state.scheduler.resume();
            *state.paused.write() = false;
            info!("sync resumed via IPC");
            IpcResponse::Resumed
        }

        // ---- Sync folder location --------------------------------------------
        IpcCommand::GetSyncSettings => IpcResponse::SyncSettings {
            local_root: state.cfg.sync_root().to_string_lossy().into_owned(),
        },

        IpcCommand::SetSyncFolder { path } => {
            let new_root = std::path::PathBuf::from(&path);
            if !new_root.is_absolute() {
                return IpcResponse::Error {
                    message: "Please choose an absolute folder path.".into(),
                };
            }

            let old_root = state.cfg.sync_root();
            if old_root == new_root {
                return IpcResponse::SyncSettings {
                    local_root: new_root.to_string_lossy().into_owned(),
                };
            }

            info!(
                from = %old_root.display(),
                to = %new_root.display(),
                "changing sync folder location"
            );

            // Pause everything before touching files on disk — workers must
            // not be mid-upload/download while we move the tree out from
            // under them.
            state.scheduler.pause();
            *state.paused.write() = true;

            if let Err(e) = crate::sync_folder::move_sync_folder(&old_root, &new_root) {
                // Nothing durable changed — safe to just resume normally.
                state.scheduler.resume();
                *state.paused.write() = false;
                return IpcResponse::Error { message: e.to_string() };
            }

            let mut new_cfg = state.cfg.clone();
            new_cfg.sync.local_root = new_root.to_string_lossy().into_owned();
            if let Err(e) = new_cfg.save() {
                return IpcResponse::Error {
                    message: format!(
                        "Files were moved to \"{}\", but saving the new config failed: {e}. \
                         Edit ~/.config/tuxdrive/config.toml's [sync] local_root manually, then restart the daemon.",
                        new_root.display()
                    ),
                };
            }

            // The sync engine and filesystem watcher aren't built to hot-swap
            // their root path — restart the whole process so it re-reads
            // config fresh. Give this response time to reach the client
            // first. Relies on the systemd unit's `Restart=on-failure`
            // (RestartPreventExitStatus only excludes exit code 2).
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                info!("restarting daemon to apply new sync folder location");
                std::process::exit(75);
            });

            IpcResponse::SyncFolderChanged {
                local_root: new_root.to_string_lossy().into_owned(),
            }
        }

        // ---- Logout ---------------------------------------------------------
        IpcCommand::Logout { email } => {
            match state.auth.logout(&email).await {
                Ok(()) => {
                    info!(email = %email, "user logged out via IPC");
                    // Clear account email if it matches.
                    let mut stored = state.account_email.write();
                    if *stored == email {
                        stored.clear();
                    }
                    IpcResponse::LoggedOut
                }
                Err(e) => IpcResponse::Error {
                    message: format!("logout failed: {e}"),
                },
            }
        }

        // ---- List files -----------------------------------------------------
        IpcCommand::ListFiles { folder_path } => {
            match state.db.list_all_files().await {
                Ok(records) => {
                    let entries: Vec<FileEntry> = records
                        .into_iter()
                        .filter(|r| {
                            // Empty folder_path → return everything.
                            if folder_path.is_empty() {
                                return true;
                            }
                            // Match files whose local_path starts with folder_path.
                            r.local_path
                                .as_deref()
                                .map(|p| p.starts_with(&folder_path))
                                .unwrap_or(false)
                        })
                        .map(|r| FileEntry {
                            name: r.name.clone(),
                            relative_path: r.local_path.unwrap_or(r.name),
                            size: r.size,
                            sync_status: r.sync_status,
                            is_folder: r.is_folder != 0,
                        })
                        .collect();
                    IpcResponse::Files { entries }
                }
                Err(e) => IpcResponse::Error {
                    message: format!("db error: {e}"),
                },
            }
        }

        // ---- Logs -----------------------------------------------------------
        IpcCommand::GetLogs { lines } => {
            let buf = state.log_buffer.read();
            let skip = buf.len().saturating_sub(lines);
            let log_lines: Vec<String> = buf.iter().skip(skip).cloned().collect();
            IpcResponse::Logs { lines: log_lines }
        }

        // ---- Shutdown -------------------------------------------------------
        IpcCommand::Shutdown => {
            info!("shutdown requested via IPC");
            // Kick off an async shutdown without blocking the IPC response.
            let scheduler = Arc::clone(&state.scheduler);
            let engine = Arc::clone(&state.sync_engine);
            tokio::spawn(async move {
                scheduler.shutdown().await;
                engine.shutdown().await;
            });
            IpcResponse::ShuttingDown
        }

        // ---- GUI-driven login -------------------------------------------------
        IpcCommand::StartLogin => {
            let in_progress = matches!(
                *state.login_state.read(),
                LoginState::AwaitingBrowser { .. }
                    | LoginState::ExchangingCode
                    | LoginState::ResolvingConflict { .. }
            );
            if in_progress {
                return IpcResponse::Error {
                    message: "a login is already in progress".into(),
                };
            }

            match state.auth.start_login().await {
                Ok(server) => {
                    let auth_url = server.auth_url.clone();
                    *state.login_state.write() = LoginState::AwaitingBrowser {
                        auth_url: auth_url.clone(),
                    };
                    let handle = login_flow::spawn_login_completion(Arc::clone(&state), server);
                    *state.login_task.write() = Some(handle);
                    IpcResponse::LoginStarted { auth_url }
                }
                Err(e) => IpcResponse::Error {
                    message: format!("could not start login: {e}"),
                },
            }
        }

        IpcCommand::GetLoginStatus => {
            let phase = state.login_state.read().clone();
            login_status_response(phase)
        }

        IpcCommand::ResolveSyncConflict { action } => {
            if !matches!(*state.login_state.read(), LoginState::ConflictPending { .. }) {
                return IpcResponse::Error {
                    message: "no sync conflict is currently pending".into(),
                };
            }
            login_flow::spawn_conflict_resolution(Arc::clone(&state), action);
            IpcResponse::SyncConflictResolving
        }

        IpcCommand::CancelLogin => {
            if let Some(handle) = state.login_task.write().take() {
                handle.abort();
            }
            *state.login_state.write() = LoginState::Idle;
            info!("login cancelled via IPC");
            IpcResponse::LoginCancelled
        }
    }
}

/// Translate a [`LoginState`] snapshot into the wire-level [`IpcResponse`].
fn login_status_response(state: LoginState) -> IpcResponse {
    let phase;
    let mut auth_url = None;
    let mut account_email = None;
    let mut known_count = None;
    let mut missing_count = None;
    let mut missing_paths = None;
    let mut resolved_done = None;
    let mut resolved_total = None;
    let mut error = None;

    match state {
        LoginState::Idle => phase = "idle".into(),
        LoginState::AwaitingBrowser { auth_url: url } => {
            phase = "awaiting_browser".into();
            auth_url = Some(url);
        }
        LoginState::ExchangingCode => phase = "exchanging_code".into(),
        LoginState::ConflictPending {
            known_count: known,
            missing,
        } => {
            phase = "conflict_pending".into();
            known_count = Some(known);
            missing_count = Some(missing.len());
            missing_paths = Some(missing);
        }
        LoginState::ResolvingConflict { done, total } => {
            phase = "resolving_conflict".into();
            resolved_done = Some(done);
            resolved_total = Some(total);
        }
        LoginState::Complete {
            account_email: email,
        } => {
            phase = "complete".into();
            account_email = Some(email);
        }
        LoginState::Failed { message } => {
            phase = "failed".into();
            error = Some(message);
        }
    }

    IpcResponse::LoginStatus {
        phase,
        auth_url,
        account_email,
        known_count,
        missing_count,
        missing_paths,
        resolved_done,
        resolved_total,
        error,
    }
}

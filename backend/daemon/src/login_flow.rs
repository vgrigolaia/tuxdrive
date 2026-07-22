use std::sync::Arc;

use serde::Deserialize;
use tracing::{info, warn};

use tuxdrive_auth::LoopbackServer;

use crate::ipc::DaemonState;
use crate::sync_resolve::{check_sync_conflict, download_missing_files, ConflictSummary};

/// Progress of an in-flight (or completed) GUI-driven login, polled by the
/// frontend via `IpcCommand::GetLoginStatus`.
#[derive(Clone, Debug, Default)]
pub enum LoginState {
    #[default]
    Idle,
    AwaitingBrowser {
        auth_url: String,
    },
    ExchangingCode,
    ConflictPending {
        known_count: usize,
        /// Relative paths only, capped — the full records stay server-side
        /// in `DaemonState::pending_conflict`.
        missing: Vec<String>,
    },
    ResolvingConflict {
        done: usize,
        total: usize,
    },
    Complete {
        account_email: String,
    },
    Failed {
        message: String,
    },
}

/// How the user chose to resolve a detected sync-history mismatch.
#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ConflictAction {
    Download,
    DeleteConfirmed,
}

/// Cap on how many missing-file paths are sent to the GUI in one shot.
const MAX_MISSING_PATHS_ON_WIRE: usize = 200;

/// Spawned right after `IpcCommand::StartLogin` obtains a [`LoopbackServer`]
/// and returns its `auth_url` to the caller. Awaits the browser redirect,
/// exchanges the code, then checks for a sync-history conflict before
/// declaring the login complete.
pub fn spawn_login_completion(
    state: Arc<DaemonState>,
    server: LoopbackServer,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Stay in `AwaitingBrowser` for the (potentially minutes-long) wait —
        // only flip to `ExchangingCode` once the redirect has actually
        // arrived, so the GUI never skips past showing the auth URL.
        let (redirect_uri, code) = match state.auth.await_browser_redirect(server).await {
            Ok(pair) => pair,
            Err(e) => {
                warn!(error = %e, "login failed while waiting for browser redirect");
                *state.login_state.write() = LoginState::Failed {
                    message: e.to_string(),
                };
                return;
            }
        };

        *state.login_state.write() = LoginState::ExchangingCode;

        let token = match state.auth.exchange_and_save(&redirect_uri, &code).await {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "login failed during token exchange");
                *state.login_state.write() = LoginState::Failed {
                    message: e.to_string(),
                };
                return;
            }
        };

        // Flip the shared account-email cell before any Drive calls, so the
        // scheduler and DriveClient start using it immediately.
        *state.account_email.write() = token.account_email.clone();
        if let Err(e) = crate::save_account_email(&token.account_email) {
            warn!(error = %e, "failed to persist account.json after login");
            *state.login_state.write() = LoginState::Failed {
                message: e.to_string(),
            };
            return;
        }

        info!(email = %token.account_email, "login completed via IPC");

        match check_sync_conflict(&state.cfg, &state.db).await {
            Ok(Some(summary)) => {
                let missing_paths = summary
                    .missing
                    .iter()
                    .map(|f| f.local_path.clone().unwrap_or_default())
                    .take(MAX_MISSING_PATHS_ON_WIRE)
                    .collect();
                let known_count = summary.known_count;
                *state.pending_conflict.write() = Some(summary);
                *state.login_state.write() = LoginState::ConflictPending {
                    known_count,
                    missing: missing_paths,
                };
            }
            Ok(None) => {
                *state.login_state.write() = LoginState::Complete {
                    account_email: token.account_email,
                };
            }
            Err(e) => {
                warn!(error = %e, "sync conflict check failed");
                *state.login_state.write() = LoginState::Failed {
                    message: e.to_string(),
                };
            }
        }
    })
}

/// Spawned by `IpcCommand::ResolveSyncConflict`'s handler once a conflict is
/// `ConflictPending`. Resolves it per `action` and moves `login_state` to
/// `Complete` (or `Failed`).
pub fn spawn_conflict_resolution(
    state: Arc<DaemonState>,
    action: ConflictAction,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let Some(summary): Option<ConflictSummary> = state.pending_conflict.write().take() else {
            return;
        };
        let email = state.account_email.read().clone();

        match action {
            ConflictAction::Download => {
                let total = summary.missing.len();
                *state.login_state.write() = LoginState::ResolvingConflict { done: 0, total };

                let progress_state = Arc::clone(&state);
                let result = download_missing_files(
                    &state.cfg,
                    &state.drive,
                    &summary.missing,
                    |done, total, _path| {
                        *progress_state.login_state.write() =
                            LoginState::ResolvingConflict { done, total };
                    },
                )
                .await;

                match result {
                    Ok((_ok, 0)) => {
                        *state.login_state.write() = LoginState::Complete {
                            account_email: email,
                        };
                    }
                    Ok((_ok, failed)) => {
                        *state.login_state.write() = LoginState::Failed {
                            message: format!(
                                "{failed} file(s) could not be re-downloaded — resolve manually, \
                                 otherwise they will be deleted from Google Drive on next sync."
                            ),
                        };
                    }
                    Err(e) => {
                        *state.login_state.write() = LoginState::Failed {
                            message: e.to_string(),
                        };
                    }
                }
            }
            ConflictAction::DeleteConfirmed => {
                // Matches the CLI's typed-DELETE semantics: no deletion is
                // performed here — this only unblocks progress. Any actual
                // deletion happens later, if at all, via the live filesystem
                // watcher noticing the files are genuinely absent.
                info!(
                    known_count = summary.known_count,
                    missing_count = summary.missing.len(),
                    "user confirmed proceeding without re-downloading missing files"
                );
                *state.login_state.write() = LoginState::Complete {
                    account_email: email,
                };
            }
        }
    })
}

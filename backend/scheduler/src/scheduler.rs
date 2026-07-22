use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tracing::{error, info, warn};

use tuxdrive_auth::AuthManager;
use tuxdrive_database::Database;
use tuxdrive_drive::{
    changes::{get_start_page_token, list_changes},
    DriveClient,
};
use tuxdrive_sync::SyncEngine;

use crate::retry::{with_retry, RetryPolicy};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Runtime configuration for the [`Scheduler`].
pub struct SchedulerConfig {
    /// How often to poll Drive for remote changes (seconds). Default: 30.
    pub poll_interval_secs: u64,
    /// How many seconds before token expiry to trigger a proactive refresh.
    /// Default: 900 (15 min).
    pub token_refresh_lead_secs: u64,
    /// The Google account e-mail whose credentials are managed. Shared with
    /// the daemon's account-email state so a login completed after the
    /// scheduler was constructed is picked up on the next loop iteration.
    pub account_email: Arc<RwLock<String>>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 30,
            token_refresh_lead_secs: 900,
            account_email: Arc::new(RwLock::new(String::new())),
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// Owns all periodic background tasks for the TuxDrive daemon.
///
/// Call [`Scheduler::start`] (on an `Arc<Scheduler>`) to launch the loops,
/// then use [`pause`](Scheduler::pause) / [`resume`](Scheduler::resume) /
/// [`shutdown`](Scheduler::shutdown) to control them.
pub struct Scheduler {
    config: SchedulerConfig,
    db: Arc<Database>,
    drive: Arc<DriveClient>,
    sync_engine: Arc<SyncEngine>,
    auth: Arc<AuthManager>,
    /// Signals all background loops to exit.
    shutdown: Arc<tokio::sync::Notify>,
    /// When `true`, the remote-polling loop skips work but keeps running.
    paused: Arc<RwLock<bool>>,
}

impl Scheduler {
    /// Construct a new [`Scheduler`].
    pub fn new(
        config: SchedulerConfig,
        db: Arc<Database>,
        drive: Arc<DriveClient>,
        sync_engine: Arc<SyncEngine>,
        auth: Arc<AuthManager>,
    ) -> Self {
        Self {
            config,
            db,
            drive,
            sync_engine,
            auth,
            shutdown: Arc::new(tokio::sync::Notify::new()),
            paused: Arc::new(RwLock::new(false)),
        }
    }

    // -----------------------------------------------------------------------
    // Control API
    // -----------------------------------------------------------------------

    /// Pause all sync operations without stopping the background tasks.
    pub fn pause(&self) {
        *self.paused.write() = true;
        self.sync_engine.pause();
        info!("scheduler paused");
    }

    /// Resume all sync operations.
    pub fn resume(&self) {
        *self.paused.write() = false;
        self.sync_engine.resume();
        info!("scheduler resumed");
    }

    /// Trigger a graceful shutdown; awaits until the signal is delivered.
    pub async fn shutdown(&self) {
        info!("scheduler shutdown requested");
        self.sync_engine.shutdown().await;
        self.shutdown.notify_waiters();
    }

    // -----------------------------------------------------------------------
    // Start
    // -----------------------------------------------------------------------

    /// Spawn all background loops and return a [`JoinHandle`] for the outer
    /// supervisor task.
    ///
    /// The returned handle completes when either all inner tasks finish or
    /// [`shutdown`](Scheduler::shutdown) is called.
    pub fn start(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut set = tokio::task::JoinSet::new();

            // --- Remote-changes polling loop ----------------------------------
            {
                let s = self.clone();
                set.spawn(async move { s.run_poll_loop().await });
            }

            // --- Token-refresh loop -------------------------------------------
            {
                let s = self.clone();
                set.spawn(async move { s.run_token_refresh_loop().await });
            }

            // Wait for shutdown signal; then abort all tasks.
            self.shutdown.notified().await;
            info!("scheduler shutting down — aborting background tasks");
            set.abort_all();

            // Drain to completion (tasks may still be mid-await).
            while set.join_next().await.is_some() {}
            info!("scheduler stopped");
        })
    }

    // -----------------------------------------------------------------------
    // Remote-changes polling loop
    // -----------------------------------------------------------------------

    async fn run_poll_loop(&self) {
        let retry = RetryPolicy::default_policy();
        let poll_duration = Duration::from_secs(self.config.poll_interval_secs);

        info!(
            interval_secs = self.config.poll_interval_secs,
            "remote poll loop started"
        );

        loop {
            // Honour pause state.
            if *self.paused.read() {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }

            // Re-read on every iteration: a login completed via IPC after the
            // scheduler started must be picked up without a daemon restart.
            let email = self.config.account_email.read().clone();
            if email.is_empty() {
                tokio::time::sleep(poll_duration).await;
                continue;
            }
            let email = &email;

            // ---- Fetch the current page token (or run initial sync). ----------
            let page_token = match self.db.get_change_token(email).await {
                Ok(Some(t)) => t,
                Ok(None) => {
                    // First run — enumerate all existing Drive files first, then
                    // get a start page token so future changes are tracked from
                    // this point forward.
                    info!("no change token found — running initial sync");
                    if let Err(e) = self.sync_engine.initial_sync().await {
                        error!(error = %e, "initial sync failed; will retry next cycle");
                        tokio::time::sleep(poll_duration).await;
                        continue;
                    }

                    match with_retry(&retry, "get_start_page_token", || {
                        let drive = self.drive.clone();
                        async move { get_start_page_token(&drive).await }
                    })
                    .await
                    {
                        Ok(t) => {
                            if let Err(e) = self.db.save_change_token(email, &t).await {
                                warn!(error = %e, "failed to persist start page token");
                            }
                            t
                        }
                        Err(e) => {
                            error!(error = %e, "could not fetch start page token; retrying next cycle");
                            tokio::time::sleep(poll_duration).await;
                            continue;
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "DB error reading change token; retrying next cycle");
                    tokio::time::sleep(poll_duration).await;
                    continue;
                }
            };

            // ---- Poll Drive for changes. -------------------------------------
            let change_list = match with_retry(&retry, "list_changes", || {
                let drive = self.drive.clone();
                let token = page_token.clone();
                async move { list_changes(&drive, &token).await }
            })
            .await
            {
                Ok(cl) => cl,
                Err(e) => {
                    error!(error = %e, "list_changes failed after retries");
                    tokio::time::sleep(poll_duration).await;
                    continue;
                }
            };

            // ---- Hand changes to the sync engine. ---------------------------
            if !change_list.changes.is_empty() {
                info!(count = change_list.changes.len(), "received remote changes");
                if let Err(e) = self
                    .sync_engine
                    .handle_remote_changes(change_list.changes)
                    .await
                {
                    error!(error = %e, "sync engine failed to handle remote changes");
                }
            }

            // ---- Persist the new page token. --------------------------------
            // Prefer newStartPageToken (final page) over nextPageToken (more pages).
            let next_token = change_list
                .new_start_page_token
                .or(change_list.next_page_token);

            if let Some(t) = next_token {
                if let Err(e) = self.db.save_change_token(email, &t).await {
                    warn!(error = %e, "failed to persist new page token");
                }
            }

            tokio::time::sleep(poll_duration).await;
        }
    }

    // -----------------------------------------------------------------------
    // Token refresh loop
    // -----------------------------------------------------------------------

    async fn run_token_refresh_loop(&self) {
        info!("token refresh loop started");

        loop {
            // Check every 60 s; the AuthManager only actually calls the network
            // when the token is near expiry.
            tokio::time::sleep(Duration::from_secs(60)).await;

            let email = self.config.account_email.read().clone();
            if email.is_empty() {
                continue;
            }
            let email = &email;

            match self.auth.get_valid_token(email).await {
                Ok(_) => {
                    // Token is valid (and refreshed if needed) — nothing to do.
                }
                Err(e) => {
                    // Non-fatal: log and carry on.  The next poll cycle will
                    // fail with an auth error if the token truly cannot be
                    // renewed, which will surface to the caller there.
                    warn!(
                        email = %email,
                        error = %e,
                        "background token refresh failed; will retry"
                    );
                }
            }
        }
    }
}

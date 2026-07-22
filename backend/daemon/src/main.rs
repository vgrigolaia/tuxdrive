mod config;
mod ipc;
mod logging;
mod login_flow;
mod service;
mod sync_folder;
mod sync_resolve;

use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing::{error, info, warn};

use config::Config;
use ipc::{DaemonState, IpcServer};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "tuxdrive-daemon", about = "Google Drive sync daemon for Linux")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to the config file (default: ~/.config/tuxdrive/config.toml)
    #[arg(long)]
    config: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Start the daemon (default behaviour when no subcommand is given)
    Start,
    /// Authenticate with Google (loopback/browser-redirect flow — prints a
    /// URL to open in any browser, then waits for the redirect)
    Login,
    /// Revoke credentials and log out
    Logout,
    /// Install a systemd user service for auto-start
    InstallService,
    /// Remove the systemd user service
    UninstallService,
    /// Print daemon status by querying the running daemon over IPC
    Status,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Load configuration.
    let cfg = match &cli.config {
        Some(path) => {
            let content = std::fs::read_to_string(path)?;
            toml::from_str::<Config>(&content)?
        }
        None => Config::load()?,
    };

    match cli.command {
        // ------------------------------------------------------------------
        None | Some(Command::Start) => run_daemon(cfg).await?,

        // ------------------------------------------------------------------
        Some(Command::Login) => do_login(cfg).await?,

        // ------------------------------------------------------------------
        Some(Command::Logout) => do_logout(cfg).await?,

        // ------------------------------------------------------------------
        Some(Command::InstallService) => {
            let exec = std::env::current_exe()?;
            service::install_systemd_service(&exec)?;
            println!("tuxdrive-daemon systemd user service installed and enabled.");
        }

        // ------------------------------------------------------------------
        Some(Command::UninstallService) => {
            service::uninstall_systemd_service()?;
            println!("tuxdrive-daemon systemd user service removed.");
        }

        // ------------------------------------------------------------------
        Some(Command::Status) => {
            query_status(&cfg).await?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Status query (thin client side)
// ---------------------------------------------------------------------------

async fn query_status(cfg: &Config) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let socket_path = cfg.socket_path();
    let mut stream = UnixStream::connect(&socket_path).await.map_err(|e| {
        anyhow::anyhow!(
            "Could not connect to daemon socket at {}: {}",
            socket_path.display(),
            e
        )
    })?;

    let cmd = serde_json::json!({"cmd": "get_status"});
    let mut line = serde_json::to_string(&cmd)?;
    line.push('\n');
    stream.write_all(line.as_bytes()).await?;

    let (read_half, _write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half).lines();
    if let Some(response_line) = reader.next_line().await? {
        println!("{response_line}");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Daemon run loop
// ---------------------------------------------------------------------------

async fn run_daemon(cfg: Config) -> anyhow::Result<()> {
    // 1. Initialise logging.
    let log_buffer = logging::init_logging(&cfg.log.level, cfg.log.file.as_deref())?;
    info!("tuxdrive-daemon starting");

    // 2. Acquire an exclusive lock file so only one daemon instance runs.
    //    Exit code 2 maps to RestartPreventExitStatus=2 in the service unit —
    //    systemd will not restart a daemon that exits because it's already running.
    let lock_path = Config::expand_path("~/.local/share/tuxdrive/daemon.lock");
    if let Some(p) = lock_path.parent() { std::fs::create_dir_all(p)?; }
    let lock_file = std::fs::OpenOptions::new()
        .write(true).create(true).open(&lock_path)?;
    use std::os::unix::io::AsRawFd;
    let rc = unsafe {
        libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB)
    };
    if rc != 0 {
        eprintln!("tuxdrive-daemon is already running (lock: {})", lock_path.display());
        std::process::exit(2);
    }
    // Keep `lock_file` alive for the process lifetime — dropping it releases the lock.
    let _lock_guard = lock_file;

    // 3. Ensure the sync root directory exists.
    let sync_root = cfg.sync_root();
    if !sync_root.exists() {
        std::fs::create_dir_all(&sync_root)?;
        info!(path = %sync_root.display(), "created sync root directory");
    }

    // 3. Open the database.
    let db_path = cfg.db_path();
    info!(path = %db_path.display(), "opening database");
    let db = Arc::new(tuxdrive_database::Database::open(&db_path).await?);

    // 4. Build OAuth config and auth manager (uses bundled credentials unless
    //    the user has overridden them in config.toml).
    let oauth_config = tuxdrive_auth::OAuthConfig {
        client_id:     cfg.auth.effective_client_id().to_owned(),
        client_secret: cfg.auth.effective_client_secret().to_owned(),
    };
    let token_store = Arc::new(tuxdrive_auth::KeyringTokenStore::new());
    let oauth_client = tuxdrive_auth::OAuthClient::new(oauth_config);
    let auth = Arc::new(tuxdrive_auth::AuthManager::new(oauth_client, token_store));

    // 5. Load account email from the account file (may be empty on first run),
    //    wrapped in a shared cell so a login completed later via IPC is picked
    //    up immediately by the Drive client and scheduler below.
    let account_email_cell = Arc::new(parking_lot::RwLock::new(load_account_email()));
    info!(
        email = %account_email_cell.read(),
        "loaded account email (empty = first run)"
    );

    // 6. Build the Drive client, sharing the same cell.
    let drive = Arc::new(tuxdrive_drive::DriveClient::new(
        Arc::clone(&auth),
        Arc::clone(&account_email_cell),
    ));

    // 7. Build the sync engine.
    let sync_config = tuxdrive_sync::SyncConfig {
        sync_root: sync_root.clone(),
        account_email: account_email_cell.read().clone(),
        chunk_size: cfg.sync.chunk_size_bytes,
        max_concurrent: cfg.sync.max_concurrent_transfers,
    };
    let sync_engine = Arc::new(tuxdrive_sync::SyncEngine::new(
        sync_config,
        Arc::clone(&drive),
        Arc::clone(&db),
    ));

    // 8. Build the filesystem watcher and start it.
    let (mut watcher, watcher_rx) = tuxdrive_watcher::FsWatcher::new(sync_root.clone());
    watcher.start().expect("failed to start filesystem watcher");

    // 9. Build the scheduler, sharing the same cell.
    let scheduler_config = tuxdrive_scheduler::SchedulerConfig {
        poll_interval_secs: cfg.sync.poll_interval_secs,
        token_refresh_lead_secs: 900,
        account_email: Arc::clone(&account_email_cell),
    };
    let scheduler = Arc::new(tuxdrive_scheduler::Scheduler::new(
        scheduler_config,
        Arc::clone(&db),
        Arc::clone(&drive),
        Arc::clone(&sync_engine),
        Arc::clone(&auth),
    ));

    // 10. Build shared state and IPC server.
    let shutdown_notify = Arc::new(tokio::sync::Notify::new());
    let daemon_state = Arc::new(DaemonState::new(
        Arc::clone(&sync_engine),
        Arc::clone(&scheduler),
        Arc::clone(&db),
        Arc::clone(&auth),
        Arc::clone(&drive),
        cfg.clone(),
        account_email_cell,
        log_buffer,
    ));
    let ipc_server = IpcServer::new(cfg.socket_path(), Arc::clone(&daemon_state));

    // Subscribe BEFORE spawning the signal handler so that a very fast signal
    // cannot slip between the spawn and the .await below.
    let shutdown_waiter = shutdown_notify.notified();

    // 11. Set up signal handlers (SIGTERM + SIGINT → graceful shutdown).
    let shutdown_for_signals = Arc::clone(&shutdown_notify);
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        info!("shutdown signal received");
        shutdown_for_signals.notify_waiters();
    });

    // 12. Spawn background tasks.

    // a) Filesystem watcher event loop.
    {
        let engine = Arc::clone(&sync_engine);
        let mut rx = watcher_rx;
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                engine.handle_local_event(event);
            }
        });
    }

    // b) Sync engine workers.
    sync_engine.start_workers();

    // c) Scheduler (remote polling + token refresh).
    Arc::clone(&scheduler).start();

    // d) IPC server.
    let shutdown_for_ipc = Arc::clone(&shutdown_notify);
    let ipc_handle = tokio::spawn(async move {
        if let Err(e) = ipc_server.run(shutdown_for_ipc).await {
            error!("IPC server error: {e}");
        }
    });

    info!("tuxdrive-daemon running — waiting for shutdown signal");

    // 13. Wait for the shutdown signal (SIGTERM / SIGINT).
    shutdown_waiter.await;

    info!("initiating graceful shutdown");

    // 14. Graceful shutdown.
    scheduler.shutdown().await;
    sync_engine.shutdown().await;
    ipc_handle.await.ok();

    // Stop the watcher (drops the debouncer).
    drop(watcher);

    info!("tuxdrive-daemon stopped");
    Ok(())
}

// ---------------------------------------------------------------------------
// Account email persistence
// ---------------------------------------------------------------------------

/// Path to the simple JSON file that caches the active account email.
fn account_file_path() -> std::path::PathBuf {
    Config::expand_path("~/.local/share/tuxdrive/account.json")
}

/// Read the account email from `~/.local/share/tuxdrive/account.json`.
/// Returns an empty string if the file does not exist or cannot be parsed.
fn load_account_email() -> String {
    let path = account_file_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(v) => v
            .get("email")
            .and_then(|e| e.as_str())
            .unwrap_or("")
            .to_owned(),
        Err(e) => {
            warn!(error = %e, "failed to parse account.json");
            String::new()
        }
    }
}

/// Persist the active account email to `~/.local/share/tuxdrive/account.json`.
pub fn save_account_email(email: &str) -> anyhow::Result<()> {
    let path = account_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::json!({"email": email});
    std::fs::write(&path, serde_json::to_string_pretty(&json)?)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Login / Logout subcommands
// ---------------------------------------------------------------------------

/// Build an OAuthConfig from the loaded Config (always uses effective credentials).
fn make_oauth_config(cfg: &Config) -> tuxdrive_auth::OAuthConfig {
    tuxdrive_auth::OAuthConfig {
        client_id:     cfg.auth.effective_client_id().to_owned(),
        client_secret: cfg.auth.effective_client_secret().to_owned(),
    }
}

/// Interactive Google Device Code login. Does not require the daemon to be running.
async fn do_login(cfg: Config) -> anyhow::Result<()> {
    let oauth_config = make_oauth_config(&cfg);
    let token_store  = std::sync::Arc::new(tuxdrive_auth::KeyringTokenStore::new());
    let oauth_client = tuxdrive_auth::OAuthClient::new(oauth_config);
    let auth         = std::sync::Arc::new(tuxdrive_auth::AuthManager::new(oauth_client, token_store));

    println!("\nLogging in to Google Drive...");

    let token = auth.login().await?;

    // Persist the email so the daemon finds it on next start.
    save_account_email(&token.account_email)?;

    println!("Logged in as: {}", token.account_email);
    println!("Token stored in GNOME Keyring / KWallet.");

    // Check the local folder against sync history before the daemon ever
    // starts watching it — catches the case where local files went missing
    // (wiped folder, reinstall, wrong path) so we don't mistake that for an
    // intentional deletion and mirror it to Google Drive.
    let db = tuxdrive_database::Database::open(&cfg.db_path()).await?;
    let drive = tuxdrive_drive::DriveClient::new(
        std::sync::Arc::clone(&auth),
        std::sync::Arc::new(parking_lot::RwLock::new(token.account_email.clone())),
    );
    sync_resolve::resolve_sync_direction(&cfg, &db, &drive).await?;

    println!("Start the daemon:  tuxdrive-daemon start\n");

    Ok(())
}

/// Revoke credentials and remove the account file.
async fn do_logout(cfg: Config) -> anyhow::Result<()> {
    let email = load_account_email();
    if email.is_empty() {
        println!("No account is currently logged in.");
        return Ok(());
    }

    let oauth_config = make_oauth_config(&cfg);
    let token_store  = std::sync::Arc::new(tuxdrive_auth::KeyringTokenStore::new());
    let oauth_client = tuxdrive_auth::OAuthClient::new(oauth_config);
    let auth         = std::sync::Arc::new(tuxdrive_auth::AuthManager::new(oauth_client, token_store));

    auth.logout(&email).await?;

    // Remove the account file.
    let path = account_file_path();
    let _ = std::fs::remove_file(&path);

    println!("Logged out from {}.", email);
    Ok(())
}

// ---------------------------------------------------------------------------
// Signal handling
// ---------------------------------------------------------------------------

/// Resolves when SIGTERM *or* SIGINT is received.
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm = signal(SignalKind::terminate())
        .expect("failed to install SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt())
        .expect("failed to install SIGINT handler");

    tokio::select! {
        _ = sigterm.recv() => {
            info!("received SIGTERM");
        }
        _ = sigint.recv() => {
            info!("received SIGINT");
        }
    }
}

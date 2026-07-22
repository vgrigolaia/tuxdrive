# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project identity

This project was renamed from **gcloud** to **TuxDrive** (the name collided
with Google's own CLI). The user is buying `tuxdrive.com` and building this
into a real, distributable public product — not just a personal dev-machine
tool. Treat feature/architecture decisions accordingly: favor choices that
hold up for real end users (clean installers, cross-distro support) over
quick personal-use shortcuts. Installer support spans both Debian/Ubuntu
(apt) and Fedora/RHEL (dnf) families — see the Fedora/RHEL section under
`install.sh` below.

## Build Commands

```bash
# Install Rust (not present by default on this machine)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Check all crates compile without building binaries
cargo check --workspace

# Build all crates (debug)
cargo build --workspace

# Build release binary — OAuth2 credentials MUST be supplied via env vars.
# These are compiled in via env!() (backend/daemon/src/config.rs) so end
# users never need their own Google Cloud project. install.sh falls back to
# a bundled default Client ID/Secret when these are unset (see below).
TUXDRIVE_CLIENT_ID="your-client-id.apps.googleusercontent.com" \
TUXDRIVE_CLIENT_SECRET="your-client-secret" \
cargo build --package tuxdrive-daemon --release

# Run all unit tests across workspace
cargo test --workspace

# Run a single integration test file
cargo test --test test_database
cargo test --test test_queue
cargo test --test test_checksum
cargo test --test test_conflict
cargo test --test test_retry
cargo test --test test_watcher_filter

# Run a single test by name within a test file
cargo test --test test_database upsert_and_get_file_by_id

# Lint
cargo clippy --workspace -- -D warnings

# Format check / auto-format
cargo fmt --check
cargo fmt

# Flutter frontend
cd frontend/flutter && flutter pub get && flutter build linux --release
flutter analyze   # lint
```

`cargo check`/`clippy` on `tuxdrive-daemon` fails without `TUXDRIVE_CLIENT_ID`/`TUXDRIVE_CLIENT_SECRET` set (the `env!()` macro is a compile-time hard requirement, not a runtime default) — the other six workspace crates check fine without them.

### `install.sh`

The real end-user installation path, not just a convenience script — `./install.sh --uninstall` then `./install.sh` is the standard "test as a fresh new user" cycle. It installs system/Flutter build deps, builds the daemon + `tuxdrive-ctl` + `tuxdrive-indicator` + the Flutter GUI, installs everything under `/opt/tuxdrive` and `/usr/local/bin`, writes the systemd user units, and launches the GUI. OAuth credentials resolve in this order: `$TUXDRIVE_CLIENT_ID`/`$TUXDRIVE_CLIENT_SECRET` env vars → `[auth]` section in `config.toml` → the hardcoded `DEFAULT_CLIENT_ID`/`DEFAULT_CLIENT_SECRET` constants — so a fresh install never has to prompt for credentials. `--uninstall` removes everything under `/opt/tuxdrive`, `~/.local/share/tuxdrive`, `~/.config/tuxdrive`, and the systemd units, but leaves `~/TuxDrive/*` (the synced files) untouched.

## Architecture

The project is a bidirectional Google Drive sync daemon for Linux. It is split into a Rust backend (multi-crate workspace) and a Flutter desktop frontend.

### Crate dependency graph (bottom → top)

```
tuxdrive-auth       ← OAuth2 (loopback/browser-redirect) + GNOME Keyring token storage
tuxdrive-database   ← SQLite via sqlx (no external deps on other workspace crates)
tuxdrive-drive      ← Drive REST API client (depends on tuxdrive-auth)
tuxdrive-watcher    ← inotify via notify + notify-debouncer-full (no workspace deps)
tuxdrive-sync       ← Sync engine (depends on all four above)
tuxdrive-scheduler  ← Periodic polling + retry (depends on tuxdrive-sync, tuxdrive-auth, tuxdrive-drive, tuxdrive-database)
tuxdrive-daemon     ← Binary; wires everything together, owns IPC server
```

### How a sync cycle works

**Local → remote:** `FsWatcher` (inotify, 500 ms debounce) emits `LocalEvent` → `SyncEngine::handle_local_event` → enqueues `SyncTask` in `SyncQueue` → worker calls `execute_upload` (simple upload < 5 MB, resumable chunked upload otherwise) → updates `files` + `sync_state` tables. Directory create/modify events are filtered out here rather than queued as (always-failing) uploads — see Watcher below for why that check is needed.

**Remote → local:** `Scheduler` polls `tuxdrive-drive::changes::list_changes` every 30 s using a persisted `page_token` from the `change_tokens` table → `SyncEngine::handle_remote_changes` → enqueues `DownloadTask` / `DeleteLocalTask` → worker calls `execute_download` (conflict check first) → updates DB.

**Conflict:** when both local checksum ≠ last-synced checksum AND remote `modifiedTime` ≠ last-synced remote mod, the local copy is renamed to `<stem>.conflict.<ISO8601compact>.<ext>` and re-queued for upload; the remote version is downloaded as the canonical file.

**Known gap:** `handle_local_event` mirrors a local `Deleted` event straight to a remote trash task with no confirmation — if local sync history and local disk ever disagree while the daemon is already running (as opposed to at login, see below), a local wipe can cascade into mass remote deletion. There is no in-engine safeguard against this; the login-time conflict check below is the only mitigation in place today.

### Folder hierarchy resolution

`SyncEngine::resolve_local_path`/`resolve_folder_path` (`backend/sync/src/engine.rs`) build each Drive item's local relative path from its real Drive parent chain instead of using the bare filename, so the local tree mirrors Drive's actual folder structure rather than dumping every file flat into the sync root. `resolve_folder_path` self-heals against the Drive list/Changes APIs returning items in arbitrary order: if an item's parent folder isn't cached yet, it fetches that folder directly and recurses, caching the result in the `files` table as it goes. `drive_root_id` (resolved once via `files.get?fileId=root`, cached for the engine's lifetime) identifies the Drive account root so top-level items resolve to the sync root itself instead of nesting under a synthetic folder for it.

### Google Docs Editors files (Docs/Sheets/Slides/Drawings)

These have no raw binary content — `alt=media` downloads always 403 with "Use Export with Docs Editors files". `tuxdrive_drive::files::google_export_target(mime_type)` maps native Google mimeTypes to an export mimeType + file extension (Docs→`.docx`, Sheets→`.xlsx`, Slides→`.pptx`, Drawings→`.png`); `execute_download` calls `export_file` (`GET /files/{id}/export`) instead of `download_file` for these, and the local filename gets the export extension appended. Native types with no sensible local equivalent (Forms, Sites, Apps Script, Shortcuts, ...) are recorded with `sync_status = "unsupported"` and never queued for download at all — `is_google_native()` detects the whole `application/vnd.google-apps.*` family for this check.

### Throughput / ETA

`SyncEngine` keeps a rolling window (`RATE_WINDOW = 100`) of recent task-completion timestamps; `eta_seconds()` extrapolates seconds-remaining from that window's throughput, discarding the estimate once the newest completion is >60 s stale (so an idle daemon doesn't show a misleading number left over from an old burst). Surfaced via `IpcResponse::Status.eta_seconds` → `tuxdrive-ctl status`, the tray indicator, and the Flutter "N queued" chip.

### Login-time sync-history safety net

`backend/daemon/src/sync_resolve.rs` compares local disk state against the DB's sync history at login time (`check_sync_conflict`); if files that should exist per DB history are missing locally (e.g. after a local wipe or reinstall), the GUI surfaces a choice instead of silently re-syncing — default is to re-download from Drive (`download_missing_files`), with a destructive "proceed with deletion instead" path gated behind typing `DELETE` in the Flutter dialog (`sync_conflict_dialog.dart`). This mitigates the local-delete-mirroring gap above but only runs at login, not continuously.

### IPC protocol

The daemon listens on a Unix domain socket (`~/.local/share/tuxdrive/daemon.sock`, permissions 0600). The protocol is **newline-delimited JSON**: one JSON object per line in each direction, FIFO-paired request/response.

Commands (client → daemon, see `IpcCommand` in `backend/daemon/src/ipc.rs`): `get_status`, `pause`, `resume`, `logout`, `list_files`, `get_logs`, `shutdown`, plus the GUI-driven login flow — `start_login` (returns the browser URL immediately), `get_login_status` (poll target; phases: idle → awaiting_browser → exchanging_code → conflict_pending → resolving_conflict → complete/failed), `resolve_sync_conflict` (answers a pending conflict from the safety net above), `cancel_login`. `DaemonState` holds the in-flight `login_state`/`pending_conflict`/`login_task` for this flow. Responses are tagged with `"type"`.

Full gRPC (tonic + prost from `shared/proto/tuxdrive.proto`) is planned for Phase 2. The `tonic`/`prost` deps are in `Cargo.toml` but the daemon currently uses only the Unix socket.

### Database

`tuxdrive-database` uses `sqlx` with **runtime string queries** (not `query!` macros), because compile-time verification requires a `DATABASE_URL` environment variable pointing to a real database during `cargo build`. Do not switch to `query!` macros unless you also wire up `DATABASE_URL` in CI and the sqlx prepare cache.

Migrations live in `backend/database/migrations/` and are embedded via `sqlx::migrate!("./migrations")` which runs automatically on `Database::open`.

Key tables: `files` (Drive metadata mirror), `sync_state` (per-file checksum + last-synced timestamps for conflict detection), `change_tokens` (persisted Drive Changes API page token per account), `upload_sessions` (resumable upload session URIs for crash recovery), `selective_sync` (schema + full CRUD already exist in `repository.rs`, but nothing in the sync engine calls it yet — selective sync is unimplemented).

### Auth flow

`AuthManager` uses an OAuth2 **loopback/browser-redirect** flow — a local `LoopbackServer` receives the redirect after the user approves in their browser. It is not device-code flow, despite what stray comments elsewhere in the codebase may claim. `login()` is split for the GUI-driven flow: `start_login()` opens the loopback server and returns it; `await_browser_redirect(server)` waits for the redirect and returns `(redirect_uri, code)`; `exchange_and_save(redirect_uri, code)` does the token exchange and keyring write; `complete_login(server)` composes the latter two for the plain-CLI path. Keep the state-transition ordering in mind if touching this: setting a "waiting"/"in-progress" login phase *before* `await_browser_redirect` resolves creates a race where the UI shows "finishing sign-in" before the browser step has even happened.

The resulting `TokenSet` is JSON-serialised into GNOME Keyring / KWallet under the key `tuxdrive:{email}` via the `keyring` crate. `get_valid_token(email)` checks the in-process cache first, then keyring; if the token is within 60 s of expiry it calls `refresh_token`.

The active account email is shared live across `DriveClient`/`SchedulerConfig`/`DaemonState` as `Arc<parking_lot::RwLock<String>>` (not a plain `String`), so a fresh login updates every consumer without a daemon restart. It's also persisted separately at `~/.local/share/tuxdrive/account.json` (written by the daemon after login via IPC; read at startup).

### Watcher filter rules

`EventFilter::should_ignore` returns `true` (ignored) for: any path component starting with `.tuxdrive-`, temp extensions (`.tmp`, `.tuxdrive-tmp`, `.crdownload`, `.part`), hidden files (leading `.`), Office temp prefix `~$`, `.Trash`/`.trash` directories, and the literal `.` path. This filter runs on file *paths*, not file *types* — the watcher never distinguishes a directory event from a file event (`notify::EventKind::Create` fires identically for both), so `SyncEngine::handle_local_event` has to check `is_dir()` itself before treating a `Created`/`Modified` event as an upload. Creating a *new local folder* (as opposed to one the daemon creates itself while mirroring a remote folder) still isn't synced to Drive — there's no folder-creation path wired up from local filesystem events.

### Runtime file locations

| Path | Purpose |
|---|---|
| `~/.config/tuxdrive/config.toml` | Main config (client_id, client_secret, sync_root, etc.) |
| `~/.local/share/tuxdrive/tuxdrive.db` | SQLite metadata database |
| `~/.local/share/tuxdrive/daemon.sock` | Unix domain socket (IPC) |
| `~/.local/share/tuxdrive/account.json` | Cached active account email |
| `~/.config/systemd/user/tuxdrive.service` | Systemd user service unit (installed by `install.sh`) |
| `/opt/tuxdrive/tuxdrive_flutter` | Installed Flutter GUI binary |
| `/usr/local/bin/{tuxdrive-daemon,tuxdrive-ctl,tuxdrive-indicator}` | Installed binaries/scripts |

If a fix doesn't seem to take effect after rebuilding, check for a stale binary earlier on `PATH` (e.g. `~/.local/bin/tuxdrive-daemon` shadowing `/usr/local/bin/tuxdrive-daemon`) via `readlink -f /proc/<pid>/exe` before assuming the code change is wrong.

### Shared state pattern in daemon

All subsystems are wrapped in `Arc<T>` and constructed in `run_daemon()` in `backend/daemon/src/main.rs`. `DaemonState` (in `ipc.rs`) holds `Arc` references to `SyncEngine`, `Scheduler`, `Database`, `AuthManager`, and `DriveClient`, plus the login-flow state (`login_state`, `pending_conflict`, `login_task`), so IPC handlers can call any subsystem without holding locks; `dispatch_command` takes `Arc<DaemonState>`. Shutdown is co-ordinated via a single `Arc<tokio::sync::Notify>` that all loops `select!` against.

### Flutter frontend

Located at `frontend/flutter/`. `DaemonClient` (`lib/ipc/daemon_client.dart`) connects to the daemon socket, sends JSON commands, and pairs responses to `Completer` objects via a FIFO queue — decode the stream with `utf8.decoder.bind(_socket!)`, not `_socket!.transform(utf8.decoder)` (the latter doesn't compile under current Dart's stream generic variance). `SyncProvider` (`lib/providers/sync_provider.dart`) polls `get_status` every 3 s and reconnects automatically, plus runs a separate 1 s poll of `get_login_status` while a login is in flight. `LoginScreen` (`lib/screens/login_screen.dart`) drives the entire OAuth + sync-conflict flow from the GUI with zero terminal interaction, switching on `sync.loginPhase`; the app routes there when the daemon is unreachable or no account is logged in, and to `MainScreen` otherwise.

## Phase roadmap

- **Phase 1 (current):** bidirectional folder sync, OAuth2, SQLite, inotify, JSON IPC, Flutter GUI
- **Phase 2:** selective sync UI (schema/CRUD already exist, unwired), shared drives, gRPC IPC, SQLCipher, bandwidth throttling, certificate pinning
- **Phase 3:** `tuxdrive-fuse` crate — FUSE virtual filesystem with on-demand file download (Files On Demand)

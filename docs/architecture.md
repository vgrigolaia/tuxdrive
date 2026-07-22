# TuxDrive — Architecture Documentation

## Overview

TuxDrive is a production-grade Google Drive desktop synchronization daemon for Linux, modelled after Microsoft OneDrive on Windows. It provides automatic, bidirectional synchronization between a local folder and the user's Google Drive, with a native GTK/Flutter GUI, system-tray integration, and a Phase 3 FUSE virtual filesystem for on-demand file access.

---

## High-Level Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│  Flutter Desktop Frontend  (frontend/flutter/)                   │
│  ┌──────────────┐  ┌────────────────┐  ┌───────────────────┐    │
│  │  Setup Wizard │  │  Main Window   │  │  System Tray Icon │    │
│  │  OAuth2 Login │  │  File Explorer │  │  Pause / Resume   │    │
│  └──────────────┘  └────────────────┘  └───────────────────┘    │
│              gRPC / Unix Domain Socket (IPC)                      │
└────────────────────────────┬─────────────────────────────────────┘
                             │
┌────────────────────────────▼─────────────────────────────────────┐
│  tuxdrive-daemon  (backend/daemon/)                                 │
│  ┌──────────────────────────────────────────────────────────┐    │
│  │  Config   │  IPC Server   │  Signal Handling  │  Logging │    │
│  └──────────────────────────────────────────────────────────┘    │
│       │              │                │                           │
│  ┌────▼──────┐  ┌────▼────────┐  ┌───▼──────────────────────┐   │
│  │ tuxdrive    │  │ tuxdrive      │  │ tuxdrive-sync              │   │
│  │ -auth     │  │ -scheduler  │  │ ┌────────────────────┐   │   │
│  │           │  │ ┌─────────┐ │  │ │  Sync Engine       │   │   │
│  │ OAuth2    │  │ │ Retry / │ │  │ │  ┌──────────────┐  │   │   │
│  │ Token     │  │ │ Backoff │ │  │ │  │ Upload Queue │  │   │   │
│  │ Refresh   │  │ └─────────┘ │  │ │  │ Download Q   │  │   │   │
│  └────┬──────┘  └─────────────┘  │ │  │ Conflict     │  │   │   │
│       │                           │ │  │ Resolution   │  │   │   │
│  ┌────▼──────────────────────┐    │ │  └──────────────┘  │   │   │
│  │  tuxdrive-database          │    │ └────────────────────┘   │   │
│  │  SQLite via sqlx           │    │                          │   │
│  │  ┌────────────────────┐   │    └──────────────────────────┘   │
│  │  │ files / sync_state │   │           │            │           │
│  │  │ change_tokens       │   │    ┌──────▼──┐   ┌────▼────┐     │
│  │  │ selective_sync      │   │    │ tuxdrive  │   │ tuxdrive  │     │
│  │  └────────────────────┘   │    │ -drive  │   │ -watcher│     │
│  └───────────────────────────┘    │ REST API│   │ inotify │     │
│                                   │ Changes │   └─────────┘     │
│                                   └─────────┘                   │
└──────────────────────────────────────────────────────────────────┘
```

---

## Component Breakdown

### `tuxdrive-auth`
Handles OAuth2 device-code and PKCE flows against Google Identity Platform. Stores and refreshes access/refresh tokens using the OS Secret Service (GNOME Keyring via the `keyring` crate). Exposes a simple `AuthManager` trait returning a valid `BearerToken` on demand.

### `tuxdrive-drive`
Thin async HTTP client wrapping the Google Drive v3 REST API:
- Files: list, get, create, update, delete, copy
- Changes: startPageToken, list (polling loop)
- Resumable upload sessions for files > 5 MB
- Exponential-backoff retries for 5xx / 429

### `tuxdrive-database`
SQLite layer using `sqlx` with compile-time query verification. Stores:
- `files` — mirrors drive metadata for every tracked file/folder
- `sync_state` — per-file sync status, local/remote checksums, conflict flags
- `change_tokens` — Drive Changes API page token persistence
- `upload_sessions` — resumable upload session URLs for crash recovery
- `selective_sync` — which remote folders the user wants mirrored locally

Migrations are embedded via `sqlx::migrate!()`.

### `tuxdrive-watcher`
Wraps the Linux `notify` crate (inotify backend) to recursively watch the local sync root. Produces a debounced stream of `LocalEvent` (Created, Modified, Renamed, Deleted) mapped to canonical relative paths, filtering `.tmp`, `.tuxdrive-conflict`, and hidden system files.

### `tuxdrive-sync`
Core synchronisation engine:
- **SyncQueue** — priority work-stealing queue of `SyncTask` items
- **SyncEngine** — consumes tasks, orchestrates uploads/downloads
- **ConflictResolver** — detects write-write conflicts (both sides modified since last sync) and renames the older copy with a `.conflict.<timestamp>` suffix
- **Checksum** — SHA-256 of local files, compared against Drive `md5Checksum`
- **ChunkedUpload** — 8 MB chunks with resumable sessions persisted in the DB
- **ChunkedDownload** — HTTP range requests written atomically via temp file + rename

### `tuxdrive-scheduler`
Async task runner using `tokio`:
- Periodic remote-change polling (configurable interval, default 30 s)
- Token refresh loop (15 min before expiry)
- Retry queue with truncated exponential backoff (1 s → 512 s, jitter)
- Pause / Resume signals forwarded from IPC

### `tuxdrive-daemon`
Binary entry point:
- Reads `~/.config/tuxdrive/config.toml`
- Starts all subsystems via dependency injection (Arc-wrapped shared state)
- Listens on a Unix domain socket for IPC commands from the Flutter frontend
- Handles SIGTERM / SIGHUP gracefully (flush queue, close DB)
- Installs itself as a systemd user service or XDG autostart entry

### Flutter Frontend
Dart/Flutter desktop application:
- **Setup Wizard** — first-run OAuth2 flow, sync-root picker, selective-folder chooser
- **Main Window** — activity feed, file tree, sync status
- **System Tray** — sync-in-progress spinner, pause/resume, open folder, quit
- Connects to the daemon over a Unix socket using the generated gRPC stubs

---

## Data Flow: Local Change → Drive

```
File written on disk
      │
      ▼
tuxdrive-watcher (inotify event)
      │ debounce 500 ms
      ▼
LocalEvent emitted
      │
      ▼
tuxdrive-sync: compute SHA-256 checksum
      │ compare with last-known checksum in DB
      │ if changed → enqueue UploadTask
      ▼
SyncQueue
      │
      ▼
SyncEngine: acquire token (tuxdrive-auth)
      │ choose simple upload (<5 MB) or resumable
      ▼
tuxdrive-drive: upload to Google Drive
      │
      ▼
tuxdrive-database: update files, sync_state (new checksum, modifiedTime)
```

## Data Flow: Remote Change → Local

```
tuxdrive-scheduler: poll Changes API every 30 s
      │
      ▼
tuxdrive-drive: list changes since saved pageToken
      │ save new pageToken to DB
      ▼
For each changed file → enqueue DownloadTask (or DeleteTask, MoveTask)
      │
      ▼
SyncEngine:
      │ check local checksum vs DB checksum
      │ if local also changed → ConflictResolver
      ▼
tuxdrive-drive: download file (range requests)
      │ write to .tuxdrive-tmp, rename atomically
      ▼
tuxdrive-database: update sync_state
```

---

## Conflict Resolution Strategy

1. Detect: remote `modifiedTime` > DB `lastSyncedRemoteModifiedTime` **AND** local `checksum` ≠ DB `lastSyncedChecksum`.
2. Rename local copy to `<name>.conflict.<ISO-timestamp>.<ext>`.
3. Download the remote version as the canonical file.
4. Queue the conflict copy for upload so neither version is lost.
5. Notify the user via the system tray.

---

## SQLite Schema (summary)

```sql
CREATE TABLE files (
    id              TEXT PRIMARY KEY,   -- Drive file ID
    name            TEXT NOT NULL,
    mime_type       TEXT NOT NULL,
    parent_id       TEXT,
    local_path      TEXT,               -- Relative to sync root
    size            INTEGER,
    drive_checksum  TEXT,               -- Drive md5Checksum
    local_checksum  TEXT,               -- SHA-256 of local file
    drive_modified  TEXT,               -- RFC3339
    local_modified  INTEGER,            -- Unix timestamp
    sync_status     TEXT NOT NULL DEFAULT 'synced',
    is_folder       INTEGER NOT NULL DEFAULT 0,
    trashed         INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE sync_state (
    local_path              TEXT PRIMARY KEY,
    drive_file_id           TEXT,
    last_synced_checksum    TEXT,
    last_synced_remote_mod  TEXT,
    conflict                INTEGER NOT NULL DEFAULT 0,
    sync_direction          TEXT     -- 'upload', 'download', 'conflict'
);

CREATE TABLE change_tokens (
    account_email   TEXT PRIMARY KEY,
    page_token      TEXT NOT NULL,
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE upload_sessions (
    local_path      TEXT PRIMARY KEY,
    session_uri     TEXT NOT NULL,
    offset          INTEGER NOT NULL DEFAULT 0,
    total_size      INTEGER NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE selective_sync (
    drive_folder_id TEXT PRIMARY KEY,
    folder_path     TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1
);
```

---

## Security Model

| Concern | Mechanism |
|---|---|
| OAuth2 tokens | Stored in OS Secret Service (GNOME Keyring / KWallet) via `keyring` crate |
| Client secret | Embedded at compile time via `TUXDRIVE_CLIENT_SECRET` env var; never written to disk |
| IPC channel | Unix domain socket with filesystem permissions (0600, owned by the user) |
| TLS | All HTTP via `reqwest` with native-tls; certificate pinning for Drive API endpoints (Phase 2) |
| DB encryption | SQLCipher optional (Phase 2) |

---

## Phased Delivery Plan

### Phase 1 (this document) — Reliable Two-Way Sync
- Full OAuth2 login / token refresh
- SQLite metadata store
- inotify local watcher
- Drive REST + Changes API client
- Bidirectional sync engine with basic conflict rename
- Daemon + Flutter setup wizard

### Phase 2 — Robustness & Features
- Selective folder sync UI
- Shared Drives support
- Systemd user service integration
- Certificate pinning
- Bandwidth throttling
- SQLCipher encryption
- Comprehensive test suite (unit + integration + E2E with Drive sandbox)

### Phase 3 — Files On Demand (FUSE)
- `tuxdrive-fuse` crate using `fuser`
- Virtual filesystem: files appear as placeholders, downloaded on `open(2)`
- Persistent inode table in SQLite
- Pinned vs. online-only per-file attribute

---

## Technology Stack

| Layer | Technology |
|---|---|
| Daemon language | Rust (2021 edition) |
| Async runtime | Tokio |
| HTTP client | reqwest (async) |
| SQLite | sqlx (compile-time checked, async) |
| Filesystem events | notify (inotify backend) |
| Crypto / checksum | sha2, ring |
| Secret storage | keyring |
| IPC | Unix socket + protobuf (prost) |
| Frontend language | Dart / Flutter |
| IPC client | Generated from .proto via `protoc` |
| Build system | Cargo workspaces + `just` |

---

## Directory Structure

```
tuxdrive/
├── Cargo.toml                  # Workspace root
├── rust-toolchain.toml
├── .gitignore
├── README.md
├── docs/
│   └── architecture.md         # This file
├── shared/
│   └── proto/
│       └── tuxdrive.proto        # IPC API definition
├── backend/
│   ├── auth/                   # tuxdrive-auth crate
│   ├── drive/                  # tuxdrive-drive crate
│   ├── database/               # tuxdrive-database crate
│   ├── watcher/                # tuxdrive-watcher crate
│   ├── sync/                   # tuxdrive-sync crate
│   ├── scheduler/              # tuxdrive-scheduler crate
│   └── daemon/                 # tuxdrive-daemon binary
├── frontend/
│   └── flutter/                # Flutter desktop app
└── tests/
    ├── integration/            # Rust integration tests
    └── e2e/                    # End-to-end tests
```

# TuxDrive — Google Drive Sync for Linux

A production-quality, bidirectional Google Drive desktop synchronization daemon for Linux, inspired by Microsoft OneDrive on Windows.

## Features

- **Automatic two-way sync** — local changes upload; remote changes download
- **Conflict resolution** — write-write conflicts renamed with `.conflict.<timestamp>` suffix
- **Resumable transfers** — chunked uploads/downloads survive crashes and reboots
- **Offline support** — queued changes sync when connectivity returns
- **inotify-based watching** — instant local change detection, debounced
- **Changes API polling** — efficient remote change detection (no full listing)
- **Secure token storage** — OAuth2 tokens in GNOME Keyring / KWallet
- **Selective folder sync** — choose which Drive folders to mirror
- **System tray** — progress spinner, pause/resume, notifications
- **Auto-start** — installs as a systemd user service or XDG autostart entry
- **Modern GUI** — Flutter desktop frontend (setup wizard + activity feed)
- **SQLite metadata** — fast local state, survives restarts

## Quick Start

### Prerequisites

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# System libraries
sudo apt install libsqlite3-dev libssl-dev pkg-config libsecret-1-dev

# Flutter (for the frontend)
# See https://docs.flutter.dev/get-started/install/linux
```

### Build

```bash
git clone https://github.com/you/tuxdrive
cd tuxdrive

# Build all backend crates
cargo build --workspace --release

# Build Flutter frontend
cd frontend/flutter
flutter pub get
flutter build linux --release
```

### Configure

Create `~/.config/tuxdrive/config.toml`:

```toml
[auth]
client_id     = "YOUR_GOOGLE_CLIENT_ID"
client_secret = "YOUR_GOOGLE_CLIENT_SECRET"

[sync]
local_root    = "~/TuxDrive"
poll_interval_secs = 30
chunk_size_bytes   = 8388608  # 8 MB

[log]
level = "info"
file  = "~/.local/share/tuxdrive/tuxdrive.log"
```

### Run

```bash
# Start daemon
~/.cargo/bin/tuxdrive-daemon &

# Launch GUI
frontend/flutter/build/linux/x64/release/bundle/tuxdrive_flutter
```

### Install as systemd service

```bash
tuxdrive-daemon --install-service
systemctl --user enable --now tuxdrive
```

## Architecture

See [docs/architecture.md](docs/architecture.md) for the full architecture document.

## Development

```bash
# Run all tests
cargo test --workspace

# Check formatting
cargo fmt --check

# Lint
cargo clippy --workspace -- -D warnings

# Integration tests (requires TUXDRIVE_TEST_CLIENT_* env vars)
cargo test --workspace --test '*' -- --include-ignored
```

## License

MIT

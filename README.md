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
- **System tray** — progress spinner, pause/resume, notifications
- **Auto-start** — installs as a systemd user service
- **Modern GUI** — Flutter desktop frontend that drives the entire OAuth sign-in flow, no terminal required
- **SQLite metadata** — fast local state, survives restarts
- **Sign in with just your Google account** — TuxDrive ships with a bundled OAuth client, so there's no Google Cloud project to create for normal use. Power users can swap in their own client via the GUI's "Advanced" login option or `config.toml`.

## Install

### Recommended: one-command installer

Works out of the box on both **Debian/Ubuntu** (apt) and **Fedora/RHEL** (dnf) — the installer detects your package manager automatically.

```bash
git clone https://github.com/vgrigolaia/tuxdrive.git
cd tuxdrive
./install.sh
```

This installs system and Flutter build dependencies, builds the daemon, `tuxdrive-ctl`, `tuxdrive-indicator`, and the Flutter GUI, installs everything under `/opt/tuxdrive` and `/usr/local/bin`, sets up the systemd user services, and launches the GUI. Sign in with your Google account when the GUI opens — that's it.

To remove everything TuxDrive installed (your synced files in `~/TuxDrive` are left untouched):

```bash
./install.sh --uninstall
```

### Alternative: .deb / .rpm package

Prefer your package manager over a build-and-install script? Grab the latest `.deb` or `.rpm` from [GitHub Releases](https://github.com/vgrigolaia/tuxdrive/releases) and install it directly:

```bash
# Debian/Ubuntu
sudo dpkg -i tuxdrive_<version>_amd64.deb

# Fedora/RHEL
sudo rpm -i tuxdrive-<version>-1.x86_64.rpm
```

Then, per user who wants to sync:

```bash
systemctl --user enable --now tuxdrive tuxdrive-indicator
```

and open TuxDrive from the applications menu. These packages are built by [`packaging/build-packages.sh`](packaging/build-packages.sh) — see that script if you want to build one yourself instead of downloading a release.

### Build from source manually (for development)

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# System libraries — Debian/Ubuntu:
sudo apt install libsqlite3-dev libssl-dev pkg-config libsecret-1-dev libgtk-3-dev

# System libraries — Fedora/RHEL:
sudo dnf install sqlite-devel openssl-devel pkgconf-pkg-config libsecret-devel gtk3-devel

# Flutter (for the frontend)
# See https://docs.flutter.dev/get-started/install/linux
```

```bash
git clone https://github.com/vgrigolaia/tuxdrive.git
cd tuxdrive

# OAuth2 credentials are compiled in at build time (see backend/daemon/src/config.rs).
# Use your own Google Cloud OAuth client here, or the codebase's bundled default
# (see install.sh) if you're just testing locally.
TUXDRIVE_CLIENT_ID="your-client-id.apps.googleusercontent.com" \
TUXDRIVE_CLIENT_SECRET="your-client-secret" \
cargo build --workspace --release

# Build Flutter frontend
cd frontend/flutter
flutter pub get
flutter build linux --release
```

```bash
# Start daemon
./target/release/tuxdrive-daemon &

# Launch GUI
frontend/flutter/build/linux/x64/release/bundle/tuxdrive_flutter
```

To run the daemon as a systemd user service instead of manually:

```bash
./target/release/tuxdrive-daemon install-service
```

This writes and enables `~/.config/systemd/user/tuxdrive.service` and starts it immediately — no separate `systemctl` step needed.

## Architecture

See [docs/architecture.md](docs/architecture.md) for the full architecture document.

## Development

```bash
# Check all crates compile without building binaries
TUXDRIVE_CLIENT_ID=dummy TUXDRIVE_CLIENT_SECRET=dummy cargo check --workspace

# Run all tests
cargo test --workspace

# Check formatting
cargo fmt --check

# Lint
cargo clippy --workspace -- -D warnings
```

### Releasing a new version

```bash
./scripts/bump-version.sh 0.1.4   # syncs Cargo.toml, pubspec.yaml, lib/version.dart
# add an entry to CHANGELOG.md
git commit -am "Release 0.1.4"
git tag v0.1.4
git push && git push --tags
```

Pushing the tag triggers [`.github/workflows/release.yml`](.github/workflows/release.yml), which builds and attaches `.deb`/`.rpm` packages to a new GitHub Release automatically.

## Roadmap

- **Phase 1 (current):** bidirectional folder sync, OAuth2, SQLite, inotify, JSON IPC, Flutter GUI
- **Phase 2:** selective sync (choose which Drive folders to mirror), shared drives, gRPC IPC, SQLCipher, bandwidth throttling, certificate pinning
- **Phase 3:** FUSE virtual filesystem with on-demand file download (Files On Demand)

## Changelog

See [CHANGELOG.md](CHANGELOG.md).

## License

[MIT](LICENSE)

# Changelog

All notable changes to TuxDrive are documented here. Versions follow
`scripts/bump-version.sh`, which keeps `Cargo.toml`, `pubspec.yaml`, and
`lib/version.dart` in sync.

## 0.1.3

- Add `.deb` and `.rpm` packaging (`packaging/build-packages.sh`) as an
  alternative to `install.sh` — both stage the same file layout.
- Add a GitHub Actions release workflow that builds and attaches both
  package formats to a GitHub Release on every `vX.Y.Z` tag push.
- Add `scripts/bump-version.sh` to keep the version in sync across
  `Cargo.toml`, `pubspec.yaml`, and `lib/version.dart` in one step.
- `install.sh` and the new packaging script now share one source of truth
  for the default OAuth credentials (`packaging/default-credentials.sh`).

## 0.1.2

- Fix `install.sh` failing on a genuinely fresh Fedora install with
  `linker \`cc\` not found` — neither the Debian nor Fedora package lists
  actually guaranteed a C compiler was present.
- Fix the Fedora tray-icon package name
  (`libayatana-appindicator3-gtk3-devel` doesn't exist under that name).
- Fix the Activity tab's Daemon Logs panel always being empty — the
  `log_buffer` was fed by an unused method nothing called; wire an
  in-memory `tracing_subscriber` layer instead so every log line shows up.
- Add a lightweight update-check banner in the GUI (checks GitHub Releases
  on startup, links out — no auto-download/install).
- Add an "Open TuxDrive Folder" button to the main AppBar.
- Add advanced/self-host OAuth client override (GUI + `SetAuthConfig` IPC
  command), for use while the bundled OAuth client is still awaiting
  Google's verification review.
- Add `LICENSE` (MIT, as already stated in the README).
- Fix stale docs: README pointed at manual `cargo build` steps instead of
  `install.sh`, `docs/user-guide.md` told every user to create their own
  Google Cloud project and documented a device-code login flow that was
  never how login actually works, `docs/architecture.md` described gRPC
  IPC and FUSE as already implemented rather than planned.

## 0.1.0 – 0.1.1

- Initial bidirectional Google Drive sync daemon: OAuth2 (loopback/
  browser-redirect flow), SQLite metadata store, inotify-based local
  watching, Drive Changes API polling, conflict resolution, resumable
  uploads, Flutter desktop GUI, systemd user service.

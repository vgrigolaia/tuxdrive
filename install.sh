#!/usr/bin/env bash
# TuxDrive — one-command installer
#
# Usage:
#   ./install.sh                  # interactive, installs to /usr/local/bin
#   ./install.sh --uninstall      # remove everything
#
# Environment overrides:
#   TUXDRIVE_CLIENT_ID / TUXDRIVE_CLIENT_SECRET  — bake your OAuth credentials in
set -euo pipefail

BOLD='\033[1m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
info()  { echo -e "${GREEN}[tuxdrive]${NC} $*"; }
warn()  { echo -e "${YELLOW}[warn]${NC}  $*"; }
step()  { echo -e "\n${BOLD}── $* ──${NC}"; }
die()   { echo -e "${RED}[error]${NC} $*" >&2; exit 1; }

# ── Package-manager detection (Debian/Ubuntu-family vs Fedora/RHEL-family) ──
if command -v apt-get &>/dev/null; then
    PKG_FAMILY="debian"
elif command -v dnf &>/dev/null; then
    PKG_FAMILY="fedora"
else
    die "Unsupported distro: neither apt-get nor dnf found."
fi

pkg_installed() {
    case "$PKG_FAMILY" in
        debian) dpkg -l "$1" &>/dev/null ;;
        fedora) rpm -q "$1"  &>/dev/null ;;
    esac
}
pkg_install_all() {
    case "$PKG_FAMILY" in
        debian) sudo apt-get install -y "$@" ;;
        fedora) sudo dnf install -y "$@" ;;
    esac
}

UNINSTALL=0
[[ "${1:-}" == "--uninstall" ]] && UNINSTALL=1

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="/usr/local/bin"
DATA_DIR="${HOME}/.local/share/tuxdrive"
CFG_DIR="${HOME}/.config/tuxdrive"
SYSTEMD_DIR="${HOME}/.config/systemd/user"

# shellcheck disable=SC1091
source "${REPO_DIR}/packaging/default-credentials.sh"

# ── Uninstall ────────────────────────────────────────────────────────────────
# Removes everything except the synced files themselves (~/TuxDrive/*) —
# database, cached account, config, credentials, systemd units, binaries, and
# the installed GUI app. This is a full reset back to "never installed."
if [[ $UNINSTALL -eq 1 ]]; then
    step "Uninstalling TuxDrive"

    # Revoke the OAuth credential from the system keyring while the binary
    # and account.json still exist — otherwise it's just orphaned there.
    if [[ -f "${DATA_DIR}/account.json" ]] && command -v tuxdrive-daemon &>/dev/null; then
        tuxdrive-daemon logout 2>/dev/null || true
    fi

    systemctl --user stop  tuxdrive tuxdrive-indicator 2>/dev/null || true
    systemctl --user disable tuxdrive tuxdrive-indicator 2>/dev/null || true
    rm -f "${SYSTEMD_DIR}/tuxdrive.service" "${SYSTEMD_DIR}/tuxdrive-indicator.service"
    systemctl --user daemon-reload 2>/dev/null || true

    sudo rm -f "${BIN_DIR}/tuxdrive-daemon" "${BIN_DIR}/tuxdrive-ctl" "${BIN_DIR}/tuxdrive-indicator"
    sudo rm -rf /opt/tuxdrive

    rm -f "${HOME}/.local/share/applications/tuxdrive.desktop"
    sed -i '/TuxDrive/d' "${HOME}/.config/gtk-3.0/bookmarks" 2>/dev/null || true

    # Database, sync history, cached account email, socket, icon, and config
    # (including any custom OAuth override) — everything but the files.
    rm -rf "${DATA_DIR}" "${CFG_DIR}"

    info "Uninstalled. Your synced files in ~/TuxDrive are untouched — everything else has been removed."
    exit 0
fi

# ── Banner ───────────────────────────────────────────────────────────────────
echo -e "${BOLD}"
cat << 'BANNER'
  ╔════════════════════════════════════════════╗
  ║   TuxDrive — Google Drive Sync for Linux   ║
  ╚════════════════════════════════════════════╝
BANNER
echo -e "${NC}"

# ── Step 1: System packages ──────────────────────────────────────────────────
step "1 / 5  System packages"
info "Detected package manager: ${PKG_FAMILY}"

# Core packages — required on every distro, names mapped per family.
# A C compiler/linker is required unconditionally: several Rust crates in
# this workspace (openssl-sys, libsqlite3-sys, ...) compile small C shims via
# build scripts, which fails with "linker `cc` not found" on a genuinely
# fresh machine that has never had a C toolchain installed.
CORE_PKGS=()
case "$PKG_FAMILY" in
    debian)
        command -v cc &>/dev/null   || pkg_installed build-essential || CORE_PKGS+=(build-essential)
        pkg_installed libssl-dev      || CORE_PKGS+=(libssl-dev)
        pkg_installed pkg-config      || CORE_PKGS+=(pkg-config)
        pkg_installed libsecret-1-dev || CORE_PKGS+=(libsecret-1-dev)
        ;;
    fedora)
        command -v cc &>/dev/null   || pkg_installed gcc || CORE_PKGS+=(gcc)
        pkg_installed openssl-devel      || CORE_PKGS+=(openssl-devel)
        pkg_installed pkgconf-pkg-config || CORE_PKGS+=(pkgconf-pkg-config)
        pkg_installed libsecret-devel    || CORE_PKGS+=(libsecret-devel)
        ;;
esac

# Only needed to build the Flutter GUI — skip if Flutter isn't installed
# (the daemon + CLI still work fine without it).
if command -v flutter &>/dev/null; then
    pkg_installed clang       || CORE_PKGS+=(clang)
    pkg_installed cmake       || CORE_PKGS+=(cmake)
    pkg_installed ninja-build || CORE_PKGS+=(ninja-build)
    case "$PKG_FAMILY" in
        debian) pkg_installed libgtk-3-dev || CORE_PKGS+=(libgtk-3-dev) ;;
        fedora) pkg_installed gtk3-devel   || CORE_PKGS+=(gtk3-devel) ;;
    esac
fi

if [[ ${#CORE_PKGS[@]} -gt 0 ]]; then
    info "Installing: ${CORE_PKGS[*]}"
    pkg_install_all "${CORE_PKGS[@]}"
else
    info "All core packages present."
fi

# Tray-icon packages (AppIndicator GIR bindings + GNOME Shell extension) —
# installed separately and non-fatally. Package availability/naming here is
# the least certain across distros, especially Fedora/RHEL, where the GNOME
# Shell extension often isn't in the default repos (may need a COPR, or
# RHEL/CentOS may need EPEL enabled first). The daemon and CLI work fine
# without the tray icon if this step fails.
case "$PKG_FAMILY" in
    debian)
        TRAY_PKGS=()
        pkg_installed gir1.2-ayatanaappindicator3-0.1 || \
          pkg_installed gir1.2-appindicator3-0.1 || \
          TRAY_PKGS+=(gir1.2-ayatanaappindicator3-0.1)
        pkg_installed gnome-shell-extension-appindicator || \
          TRAY_PKGS+=(gnome-shell-extension-appindicator)
        if [[ ${#TRAY_PKGS[@]} -gt 0 ]]; then
            pkg_install_all "${TRAY_PKGS[@]}" || \
                warn "Could not install tray-icon packages (${TRAY_PKGS[*]}) — TuxDrive will still work, just without a system-tray icon."
        fi
        ;;
    fedora)
        # The devel package's name varies by Fedora/RHEL release and which
        # fork is packaged — try the actively-maintained Ayatana fork, then
        # the older libappindicator, rather than guessing one name and
        # failing outright if that release uses the other.
        if ! pkg_installed libayatana-appindicator-gtk3-devel && \
           ! pkg_installed libappindicator-gtk3-devel; then
            sudo dnf install -y libayatana-appindicator-gtk3-devel &>/dev/null || \
              sudo dnf install -y libappindicator-gtk3-devel &>/dev/null || \
              warn "Could not install an AppIndicator devel package (tried libayatana-appindicator-gtk3-devel, libappindicator-gtk3-devel) — TuxDrive will still work, just without a system-tray icon."
        fi
        pkg_installed gnome-shell-extension-appindicator || \
          sudo dnf install -y gnome-shell-extension-appindicator &>/dev/null || \
          warn "Could not install gnome-shell-extension-appindicator — this may need a COPR repo (or EPEL, on RHEL/CentOS) enabled first. TuxDrive will still work, just without a system-tray icon."
        ;;
esac

# ── Step 2: Rust toolchain ───────────────────────────────────────────────────
step "2 / 5  Rust toolchain"
if ! command -v cargo &>/dev/null; then
    if [[ -f "${HOME}/.cargo/env" ]]; then
        # shellcheck disable=SC1091
        source "${HOME}/.cargo/env"
    fi
fi
if ! command -v cargo &>/dev/null; then
    info "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
    source "${HOME}/.cargo/env"
fi
info "Rust $(rustc --version)"

# ── Step 3: OAuth credentials ─────────────────────────────────────────────
step "3 / 5  OAuth credentials"
CLIENT_ID="${TUXDRIVE_CLIENT_ID:-}"
CLIENT_SECRET="${TUXDRIVE_CLIENT_SECRET:-}"

if [[ -z "$CLIENT_ID" || -z "$CLIENT_SECRET" ]]; then
    CFG="${CFG_DIR}/config.toml"
    if [[ -f "$CFG" ]]; then
        CLIENT_ID=$(grep -E '^\s*client_id\s*=' "$CFG" 2>/dev/null \
            | head -1 | sed 's/.*"\(.*\)".*/\1/' || true)
        CLIENT_SECRET=$(grep -E '^\s*client_secret\s*=' "$CFG" 2>/dev/null \
            | head -1 | sed 's/.*"\(.*\)".*/\1/' || true)
    fi
fi

if [[ -z "$CLIENT_ID" || -z "$CLIENT_SECRET" ]]; then
    CLIENT_ID="$DEFAULT_CLIENT_ID"
    CLIENT_SECRET="$DEFAULT_CLIENT_SECRET"
fi
info "Client ID: ${CLIENT_ID:0:40}..."

# ── Step 4: Build ─────────────────────────────────────────────────────────
step "4 / 5  Building"
cd "${REPO_DIR}"
TUXDRIVE_CLIENT_ID="${CLIENT_ID}" \
TUXDRIVE_CLIENT_SECRET="${CLIENT_SECRET}" \
    cargo build --package tuxdrive-daemon --release

# Install daemon binary system-wide
sudo install -m 755 target/release/tuxdrive-daemon "${BIN_DIR}/tuxdrive-daemon"
info "Installed: ${BIN_DIR}/tuxdrive-daemon"

# Install helper scripts
sudo install -m 755 scripts/tuxdrive-ctl       "${BIN_DIR}/tuxdrive-ctl"       2>/dev/null || true
sudo install -m 755 scripts/tuxdrive-indicator "${BIN_DIR}/tuxdrive-indicator" 2>/dev/null || true
info "Installed: ${BIN_DIR}/tuxdrive-ctl  ${BIN_DIR}/tuxdrive-indicator"

# Install Flutter itself if missing, via snap — the same install method
# already used in this project's own dev/CI environment. Never fatal: if any
# step here fails, we fall through to the existing "Flutter not found"
# warning below and the daemon/CLI still install fine without the GUI.
if ! command -v flutter &>/dev/null; then
    info "Flutter not found — installing via snap..."
    if ! command -v snap &>/dev/null; then
        pkg_install_all snapd || warn "Could not install snapd."
        [[ -e /snap ]] || sudo ln -s /var/lib/snapd/snap /snap 2>/dev/null || true
        sudo systemctl enable --now snapd.socket &>/dev/null || true
    fi
    if command -v snap &>/dev/null; then
        sudo snap install flutter --classic || \
            warn "snap install flutter failed — install manually: https://docs.flutter.dev/get-started/install/linux, then re-run ./install.sh."
        # Make it usable in *this* run without needing a fresh login shell —
        # snapd's own PATH setup (/etc/profile.d/snapd.sh) only applies to
        # new sessions.
        export PATH="/snap/bin:${PATH}"
    else
        warn "snapd was just installed but isn't ready yet (needs a fresh login session for its socket/PATH setup)."
        warn "Log out and back in, then run: sudo snap install flutter --classic, then re-run ./install.sh."
    fi
fi

# Build and install the Flutter GUI — this is what end users actually run;
# everything past this point (login, sync-conflict resolution, status,
# pause/resume) happens in the app, not the terminal.
APP_DIR=""
if command -v flutter &>/dev/null; then
    info "Building TuxDrive GUI (Flutter)..."
    (cd "${REPO_DIR}/frontend/flutter" && flutter pub get && flutter build linux --release)
    APP_DIR="/opt/tuxdrive"
    sudo mkdir -p "${APP_DIR}"
    sudo rm -rf "${APP_DIR:?}"/*
    sudo cp -r "${REPO_DIR}/frontend/flutter/build/linux/x64/release/bundle/." "${APP_DIR}/"
    info "Installed GUI: ${APP_DIR}/tuxdrive_flutter"
else
    warn "Flutter not found — skipping GUI build."
    warn "Install Flutter (https://flutter.dev) and re-run ./install.sh to get the desktop app."
    warn "Until then, use 'tuxdrive-daemon login' in a terminal to connect an account."
fi

# ── Step 5: System integration ──────────────────────────────────────────────
step "5 / 5  System integration"

mkdir -p "${SYSTEMD_DIR}" "${DATA_DIR}" "${CFG_DIR}" "${HOME}/.config/gtk-3.0"

# Create default config if absent
if [[ ! -f "${CFG_DIR}/config.toml" ]]; then
    cat > "${CFG_DIR}/config.toml" << TOML
[sync]
local_root               = "~/TuxDrive"
poll_interval_secs       = 30
chunk_size_bytes         = 8388608
max_concurrent_transfers = 4

[log]
level = "info"
TOML
    info "Created default config: ${CFG_DIR}/config.toml"
fi

# Daemon systemd user service
cat > "${SYSTEMD_DIR}/tuxdrive.service" << UNIT
[Unit]
Description=TuxDrive Google Drive Sync Daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${BIN_DIR}/tuxdrive-daemon
Restart=on-failure
RestartSec=5s
RestartPreventExitStatus=2
Environment=RUST_LOG=info
StandardOutput=journal
StandardError=journal
SyslogIdentifier=tuxdrive

[Install]
WantedBy=default.target
UNIT

# Tray indicator systemd user service
cat > "${SYSTEMD_DIR}/tuxdrive-indicator.service" << UNIT
[Unit]
Description=TuxDrive sync tray indicator
After=graphical-session.target tuxdrive.service

[Service]
ExecStart=${BIN_DIR}/tuxdrive-indicator
Restart=on-failure
RestartSec=5s
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=graphical-session.target
UNIT

systemctl --user daemon-reload
systemctl --user enable tuxdrive.service
systemctl --user enable tuxdrive-indicator.service

# Enable linger so services start at boot, not just at graphical login
loginctl enable-linger "$(id -un)" 2>/dev/null && \
    info "Lingering enabled — daemon starts at boot." || \
    warn "Could not enable linger (non-fatal)."

# Enable GNOME AppIndicator extension — this commonly fails on a first
# install even though the package installed fine, because GNOME Shell only
# discovers newly-dropped-in system extensions after a restart (X11: Alt+F2,
# r, Enter; Wayland: full log out/in — Shell can't restart in place there).
# `gnome-extensions enable` before that point reports "no such extension"
# even though the files are on disk.
if gnome-extensions enable appindicatorsupport@rgcjonas.gmail.com 2>/dev/null; then
    info "AppIndicator GNOME extension enabled."
else
    warn "Could not enable the AppIndicator GNOME extension yet — this is expected on a first install."
    warn "Log out and back in, then run: gnome-extensions enable appindicatorsupport@rgcjonas.gmail.com"
    warn "(or: Extensions app → AppIndicator and KStatusNotifierItem Support → toggle on)"
fi

# Nautilus sidebar bookmark
grep -q "TuxDrive" "${HOME}/.config/gtk-3.0/bookmarks" 2>/dev/null || \
    echo "file://${HOME}/TuxDrive TuxDrive" >> "${HOME}/.config/gtk-3.0/bookmarks"
info "Sidebar bookmark added."

# Desktop launcher for the GUI (applications menu entry + icon)
if [[ -n "$APP_DIR" ]]; then
    ICON_DIR="${DATA_DIR}/icons"
    mkdir -p "${ICON_DIR}" "${HOME}/.local/share/applications"
    cp "${REPO_DIR}/frontend/flutter/assets/icons/app_icon.png" "${ICON_DIR}/tuxdrive.png"

    cat > "${HOME}/.local/share/applications/tuxdrive.desktop" << DESKTOP
[Desktop Entry]
Type=Application
Name=TuxDrive
Comment=Google Drive sync for Linux
Exec=${APP_DIR}/tuxdrive_flutter
Icon=${ICON_DIR}/tuxdrive.png
Terminal=false
Categories=Utility;Network;
StartupWMClass=tuxdrive_flutter
DESKTOP
    info "Desktop launcher installed — TuxDrive now appears in your applications menu."
fi

# ── Done ─────────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}${GREEN}  ✓  Installation complete!${NC}"
echo ""

# The daemon is safe to run before login (it just waits, showing the login
# screen in the GUI) — start it now instead of waiting for next boot/login.
systemctl --user start tuxdrive.service tuxdrive-indicator.service 2>/dev/null || true

EMAIL=""
if [[ -f "${DATA_DIR}/account.json" ]]; then
    EMAIL=$(python3 -c "import json; d=json.load(open('${DATA_DIR}/account.json')); print(d.get('email',''))" 2>/dev/null || true)
fi

if [[ -n "$EMAIL" ]]; then
    info "Already logged in as: ${EMAIL}"
    echo ""
    echo "  Daemon is running. Your files sync to ~/TuxDrive/"
elif [[ -n "$APP_DIR" ]]; then
    echo "  Next: connect your Google Drive account — open TuxDrive from your"
    echo "  applications menu (or the tray icon) and click \"Connect Google Drive\"."
else
    echo "  Next: log in to Google Drive"
    echo ""
    echo "    tuxdrive-daemon login"
fi

if [[ -n "$APP_DIR" ]]; then
    info "Launching TuxDrive..."
    nohup "${APP_DIR}/tuxdrive_flutter" >/dev/null 2>&1 &
    disown
fi

echo ""
echo "  Logs:    journalctl --user -u tuxdrive -f"
echo "  Status:  tuxdrive-ctl status"
echo ""

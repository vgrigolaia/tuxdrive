#!/usr/bin/env bash
# TuxDrive — build .deb and/or .rpm packages from a release build.
#
# Usage:
#   packaging/build-packages.sh deb        # build only the .deb
#   packaging/build-packages.sh rpm        # build only the .rpm
#   packaging/build-packages.sh all         # build both (default)
#
# Environment overrides:
#   TUXDRIVE_CLIENT_ID / TUXDRIVE_CLIENT_SECRET  — bake your own OAuth
#     credentials in instead of the shared default (see default-credentials.sh)
#
# Output: dist/tuxdrive_<version>_amd64.deb, dist/tuxdrive-<version>-1.x86_64.rpm
#
# This intentionally hand-rolls the staged file tree (rather than using
# cargo-deb/cargo-generate-rpm) so the exact same, already-verified layout is
# used for both formats — see install.sh for the equivalent live-filesystem
# version of this same layout.
set -euo pipefail

BOLD='\033[1m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
info()  { echo -e "${GREEN}[tuxdrive]${NC} $*"; }
warn()  { echo -e "${YELLOW}[warn]${NC}  $*"; }
step()  { echo -e "\n${BOLD}── $* ──${NC}"; }
die()   { echo -e "${RED}[error]${NC} $*" >&2; exit 1; }

TARGET="${1:-all}"
case "$TARGET" in
    deb|rpm|all) ;;
    *) die "Usage: $0 [deb|rpm|all]" ;;
esac

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"
# shellcheck disable=SC1091
source "${REPO_DIR}/packaging/default-credentials.sh"

VERSION="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')"
[[ -n "$VERSION" ]] || die "Could not read version from Cargo.toml"
info "Packaging TuxDrive v${VERSION}"

ARCH="$(uname -m)"
case "$ARCH" in
    x86_64) DEB_ARCH="amd64"; RPM_ARCH="x86_64" ;;
    *) die "Unsupported architecture: ${ARCH} (only x86_64 is packaged today)" ;;
esac

command -v flutter &>/dev/null || die "Flutter is required to build the GUI bundle these packages ship — install it first (https://docs.flutter.dev/get-started/install/linux)."

# ── Build ────────────────────────────────────────────────────────────────────
step "Building release binaries"
CLIENT_ID="${TUXDRIVE_CLIENT_ID:-$DEFAULT_CLIENT_ID}"
CLIENT_SECRET="${TUXDRIVE_CLIENT_SECRET:-$DEFAULT_CLIENT_SECRET}"
TUXDRIVE_CLIENT_ID="${CLIENT_ID}" \
TUXDRIVE_CLIENT_SECRET="${CLIENT_SECRET}" \
    cargo build --package tuxdrive-daemon --release

step "Building GUI (Flutter)"
(cd "${REPO_DIR}/frontend/flutter" && flutter pub get && flutter build linux --release)
GUI_BUNDLE="${REPO_DIR}/frontend/flutter/build/linux/x64/release/bundle"
[[ -f "${GUI_BUNDLE}/tuxdrive_flutter" ]] || die "Flutter build did not produce ${GUI_BUNDLE}/tuxdrive_flutter"

# ── Stage the common file tree shared by both package formats ───────────────
step "Staging package contents"
DIST_DIR="${REPO_DIR}/dist"
STAGE="${DIST_DIR}/stage"
rm -rf "$STAGE"
mkdir -p \
    "${STAGE}/usr/bin" \
    "${STAGE}/opt/tuxdrive" \
    "${STAGE}/usr/share/applications" \
    "${STAGE}/usr/share/icons/hicolor/256x256/apps" \
    "${STAGE}/usr/lib/systemd/user" \
    "${STAGE}/usr/share/doc/tuxdrive"

install -m 755 "${REPO_DIR}/target/release/tuxdrive-daemon" "${STAGE}/usr/bin/tuxdrive-daemon"
install -m 755 "${REPO_DIR}/scripts/tuxdrive-ctl"            "${STAGE}/usr/bin/tuxdrive-ctl"
install -m 755 "${REPO_DIR}/scripts/tuxdrive-indicator"      "${STAGE}/usr/bin/tuxdrive-indicator"

cp -r "${GUI_BUNDLE}/." "${STAGE}/opt/tuxdrive/"
chmod 755 "${STAGE}/opt/tuxdrive/tuxdrive_flutter"

install -m 644 "${REPO_DIR}/packaging/tuxdrive.desktop"           "${STAGE}/usr/share/applications/tuxdrive.desktop"
install -m 644 "${REPO_DIR}/packaging/tuxdrive.service"           "${STAGE}/usr/lib/systemd/user/tuxdrive.service"
install -m 644 "${REPO_DIR}/packaging/tuxdrive-indicator.service" "${STAGE}/usr/lib/systemd/user/tuxdrive-indicator.service"
install -m 644 "${REPO_DIR}/frontend/flutter/assets/icons/app_icon.png" \
    "${STAGE}/usr/share/icons/hicolor/256x256/apps/tuxdrive.png"
install -m 644 "${REPO_DIR}/README.md" "${STAGE}/usr/share/doc/tuxdrive/README.md"
[[ -f "${REPO_DIR}/CHANGELOG.md" ]] && \
    install -m 644 "${REPO_DIR}/CHANGELOG.md" "${STAGE}/usr/share/doc/tuxdrive/CHANGELOG.md"

mkdir -p "$DIST_DIR"

# ── .deb ─────────────────────────────────────────────────────────────────────
build_deb() {
    step "Building .deb"
    local deb_root="${DIST_DIR}/deb-root"
    rm -rf "$deb_root"
    cp -r "$STAGE" "$deb_root"
    mkdir -p "${deb_root}/DEBIAN"

    cat > "${deb_root}/DEBIAN/control" <<EOF
Package: tuxdrive
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${DEB_ARCH}
Depends: libc6, libgtk-3-0, libsecret-1-0, libssl3 | libssl3t64, ca-certificates
Maintainer: TuxDrive <noreply@tuxdrive.com>
Homepage: https://github.com/vgrigolaia/tuxdrive
Description: Bidirectional Google Drive sync for Linux
 Automatic two-way sync between a local folder and Google Drive, with
 conflict resolution, resumable transfers, and a Flutter desktop GUI.
EOF

    install -m 755 "${REPO_DIR}/packaging/postinst.sh" "${deb_root}/DEBIAN/postinst"
    install -m 755 "${REPO_DIR}/packaging/postrm.sh"    "${deb_root}/DEBIAN/postrm"

    local out="${DIST_DIR}/tuxdrive_${VERSION}_${DEB_ARCH}.deb"
    dpkg-deb --build --root-owner-group "$deb_root" "$out"
    rm -rf "$deb_root"
    info "Built: ${out}"
}

# ── .rpm ─────────────────────────────────────────────────────────────────────
build_rpm() {
    step "Building .rpm"
    command -v rpmbuild &>/dev/null || die "rpmbuild not found — install rpm-build (Fedora) or rpm (Debian) to build the .rpm."

    local rpm_topdir="${DIST_DIR}/rpm-topdir"
    rm -rf "$rpm_topdir"
    mkdir -p "${rpm_topdir}"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

    # /opt/tuxdrive is package-specific (unlike /usr/bin, /usr/share/... which
    # are owned by base system packages already) — explicitly own its
    # directories so `rpm -e` cleans them up instead of leaving them behind.
    local dir_list file_list
    dir_list="$(cd "$STAGE" && find opt/tuxdrive -type d | sed 's|^|%dir /|' | sort)"
    file_list="$(cd "$STAGE" && find . -type f -o -type l | sed 's|^\.||' | sort)"

    {
        echo "Name: tuxdrive"
        echo "Version: ${VERSION}"
        echo "Release: 1"
        echo "Summary: Bidirectional Google Drive sync for Linux"
        echo "License: MIT"
        echo "URL: https://github.com/vgrigolaia/tuxdrive"
        echo "BuildArch: ${RPM_ARCH}"
        echo "Requires: glibc, gtk3, libsecret, openssl-libs, ca-certificates"
        echo
        echo "%description"
        echo "Automatic two-way sync between a local folder and Google Drive, with"
        echo "conflict resolution, resumable transfers, and a Flutter desktop GUI."
        echo
        echo "%post"
        cat "${REPO_DIR}/packaging/postinst.sh"
        echo
        echo "%postun"
        cat "${REPO_DIR}/packaging/postrm.sh"
        echo
        echo "%files"
        echo "$dir_list"
        echo "$file_list"
    } > "${rpm_topdir}/SPECS/tuxdrive.spec"

    rpmbuild -bb \
        --define "_topdir ${rpm_topdir}" \
        --buildroot "$STAGE" \
        "${rpm_topdir}/SPECS/tuxdrive.spec"

    local built
    built="$(find "${rpm_topdir}/RPMS" -name '*.rpm' | head -1)"
    [[ -n "$built" ]] || die "rpmbuild did not produce an .rpm"
    local out="${DIST_DIR}/tuxdrive-${VERSION}-1.${RPM_ARCH}.rpm"
    cp "$built" "$out"
    rm -rf "$rpm_topdir"
    info "Built: ${out}"
}

case "$TARGET" in
    deb) build_deb ;;
    rpm) build_rpm ;;
    all) build_deb; build_rpm ;;
esac

rm -rf "$STAGE"
info "Done. Packages in ${DIST_DIR}/"

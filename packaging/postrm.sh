#!/bin/sh
# Shared post-removal steps for both the .deb and .rpm package. Never touches
# user data — ~/.config/tuxdrive, ~/.local/share/tuxdrive, and the synced
# ~/TuxDrive folder are left in place, same as install.sh --uninstall.
set -e

command -v update-desktop-database >/dev/null 2>&1 && \
    update-desktop-database -q /usr/share/applications 2>/dev/null || true

command -v gtk-update-icon-cache >/dev/null 2>&1 && \
    gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true

exit 0

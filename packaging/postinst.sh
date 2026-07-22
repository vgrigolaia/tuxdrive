#!/bin/sh
# Shared post-install steps for both the .deb and .rpm package. Runs as root
# during package install/upgrade — deliberately does NOT touch `systemctl
# --user` here, since that would act on root's own session, not the actual
# end user's; enabling the daemon is a one-time step each user runs for
# themselves (see the message below).
set -e

command -v update-desktop-database >/dev/null 2>&1 && \
    update-desktop-database -q /usr/share/applications 2>/dev/null || true

command -v gtk-update-icon-cache >/dev/null 2>&1 && \
    gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true

cat <<'MSG'

TuxDrive is installed. To finish setup, each user who wants to sync runs:

  systemctl --user enable --now tuxdrive tuxdrive-indicator

Then open TuxDrive from the applications menu and sign in with Google.

MSG

exit 0

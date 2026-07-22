# TuxDrive — User Guide

## Getting started (recommended)

1. Install TuxDrive — see the [README](../README.md#install) for the one-command installer, which works on both Debian/Ubuntu and Fedora/RHEL.
2. The Flutter GUI opens automatically after install. Pick a sync folder (defaults to `~/TuxDrive`) and click **Connect Google Drive**.
3. Your browser opens and asks you to approve access to your Google account — approve it, then return to the GUI.

That's it — no Google Cloud project, no client ID, no config file editing. TuxDrive ships with a bundled OAuth client shared by every install, so signing in is just the normal Google consent flow.

The first sync downloads everything already in your Drive to the sync folder, then keeps both sides in sync from there:

- **Any file you add or change locally** uploads within seconds (inotify-based watching).
- **Any file added or changed in Drive** downloads within about 30 seconds (Changes API polling).
- **Conflicts** (edited in both places since the last sync) get the local copy renamed, e.g. `file.conflict.20260721T120000Z.docx`, and the remote version downloaded as the canonical file.

### Advanced: using your own OAuth client

If you'd rather not use the bundled client (e.g. self-hosting, corporate policy, or the bundled client is temporarily capped while awaiting Google's app verification), expand **Advanced** on the sign-in screen and enter your own Client ID / Client Secret from a Google Cloud project with the Drive API enabled. This persists to `config.toml`'s `[auth]` section and restarts the daemon to apply it — see [Configure a custom OAuth client](#configure-a-custom-oauth-client-optional) below for how to create one.

---

## Command-line usage

Everything above also works from the terminal, useful for headless machines or scripting.

### Step 1 — Log in

```bash
tuxdrive-daemon login
```

```
Logging in to Google Drive...

Open this URL in your browser to log in:

  https://accounts.google.com/o/oauth2/v2/auth?...

Waiting for browser login (5-minute timeout)...

Logged in as: yourname@gmail.com
Token stored in GNOME Keyring / KWallet.
```

Open the printed URL in any browser, approve access, and the command returns once the browser redirect completes — no device code to type in.

### Configure a custom OAuth client (optional)

Only needed if you want to use your own Google Cloud project instead of the bundled default:

1. Go to https://console.cloud.google.com/, create or select a project.
2. **APIs & Services → Library** → search **Google Drive API** → **Enable**.
3. **APIs & Services → OAuth consent screen** → User type **External** → fill in an app name and your email → add scope `https://www.googleapis.com/auth/drive` → add your account under **Test users** (required until the app passes Google's verification review).
4. **APIs & Services → Credentials → + Create Credentials → OAuth client ID** → Application type **Desktop app** → copy the **Client ID** and **Client Secret**.
5. Add them to `~/.config/tuxdrive/config.toml`:

```toml
[auth]
client_id     = "YOUR_CLIENT_ID_HERE.apps.googleusercontent.com"
client_secret = "YOUR_CLIENT_SECRET_HERE"
```

Restart the daemon to pick up the change.

### Step 2 — Start the daemon

```bash
tuxdrive-daemon start &
```

```
INFO  tuxdrive_daemon: opening database
INFO  tuxdrive_daemon: tuxdrive-daemon running — waiting for shutdown signal
INFO  tuxdrive_scheduler::scheduler: remote poll loop started interval_secs=30
```

(Skip this if you installed via `install.sh` or `tuxdrive-daemon install-service` — the daemon already runs as a systemd user service.)

### Step 3 — Check status and control sync

`tuxdrive-ctl` is the simplest way to talk to the running daemon:

```bash
tuxdrive-ctl status   # current sync status, account, queue, ETA
tuxdrive-ctl pause     # pause sync (e.g. on a slow connection)
tuxdrive-ctl resume    # resume sync
tuxdrive-ctl files     # list synced files and their status
tuxdrive-ctl logs 50   # last 50 log lines
tuxdrive-ctl stop      # shut down the daemon cleanly
```

```
  ✅  SYNCED
  Account : yourname@gmail.com
  Queued  : 0 item(s)
```

To log out entirely (revokes the local token):

```bash
tuxdrive-daemon logout
```

### Auto-start on login

```bash
tuxdrive-daemon install-service
```

Writes, enables, and immediately starts `~/.config/systemd/user/tuxdrive.service` — the daemon then starts automatically on every login (no separate `systemctl enable` needed). `install.sh` does this for you already.

```bash
systemctl --user status tuxdrive     # check service status
journalctl --user -u tuxdrive -f     # live logs
tuxdrive-daemon uninstall-service    # remove the service
```

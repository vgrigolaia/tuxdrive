# TuxDrive — User Guide

## Step 1 — Get Google API credentials (one-time)

1. Go to https://console.cloud.google.com/
2. Create a new project (or select an existing one).
3. In the left menu: **APIs & Services → Library**
4. Search for **"Google Drive API"**, click it, click **Enable**.
5. Go to **APIs & Services → OAuth consent screen**
   - User type: **External**
   - Fill in App name (e.g. "TuxDrive Sync"), your email, save.
   - Add scope: `https://www.googleapis.com/auth/drive`
   - Under **Test users**, add your Gmail address.
6. Go to **APIs & Services → Credentials**
   - Click **+ Create Credentials → OAuth client ID**
   - Application type: **Desktop app**
   - Name: "TuxDrive Desktop"
   - Click **Create**
   - Copy the **Client ID** and **Client Secret** — you'll need them next.

---

## Step 2 — Configure TuxDrive

Edit `~/.config/tuxdrive/config.toml`:

```toml
[auth]
client_id     = "YOUR_CLIENT_ID_HERE.apps.googleusercontent.com"
client_secret = "YOUR_CLIENT_SECRET_HERE"

[sync]
local_root               = "~/TuxDrive"
poll_interval_secs       = 30
chunk_size_bytes         = 8388608
max_concurrent_transfers = 4

[log]
level = "info"
```

---

## Step 3 — Log in to Google

```bash
tuxdrive-daemon login
```

You will see something like:

```
🔐  Logging in to Google Drive…

  Visit:  https://google.com/device
  Enter code:  ABCD-EFGH

  Waiting for approval…
```

1. Open the URL in any browser (phone is fine).
2. Enter the code shown in the terminal.
3. Grant access to the app.
4. The terminal will print:

```
✅  Logged in as  yourname@gmail.com
    Token stored in GNOME Keyring / KWallet.
    Start the daemon:  tuxdrive-daemon start
```

---

## Step 4 — Start the daemon

```bash
tuxdrive-daemon start
```

Or in the background (recommended):

```bash
tuxdrive-daemon start &
```

You will see log output like:
```
INFO  tuxdrive-daemon: opening database
INFO  tuxdrive-daemon: tuxdrive-daemon running — waiting for shutdown signal
INFO  tuxdrive_scheduler: remote poll loop started interval_secs=30
INFO  tuxdrive_sync: starting initial sync
```

The first run downloads all your Drive files to `~/TuxDrive/`.

---

## Step 5 — Check status

In another terminal:

```bash
tuxdrive-daemon status
```

Output:
```json
{"type":"status","status":"syncing","queued":42,"account_email":"you@gmail.com","paused":false}
```

| Field | Meaning |
|---|---|
| `status` | `synced` / `syncing` / `paused` / `error` |
| `queued` | Files waiting to upload/download |
| `account_email` | Logged-in account |
| `paused` | Whether sync is paused |

---

## Step 6 — Use your sync folder

```bash
ls ~/TuxDrive/
```

- **Any file you copy here** is automatically uploaded to Drive within seconds.
- **Any file you add/change in Drive** is automatically downloaded within 30 s.
- **Conflicts** (edited in both places) get renamed: `file.conflict.20260721T120000Z.docx`

---

## Useful commands

```bash
# Pause sync (e.g. on a slow connection)
echo '{"cmd":"pause"}' | socat - UNIX-CONNECT:~/.local/share/tuxdrive/daemon.sock

# Resume sync
echo '{"cmd":"resume"}' | socat - UNIX-CONNECT:~/.local/share/tuxdrive/daemon.sock

# Check daemon logs (last 50 lines)
echo '{"cmd":"get_logs","lines":50}' | socat - UNIX-CONNECT:~/.local/share/tuxdrive/daemon.sock | python3 -m json.tool

# List synced files
echo '{"cmd":"list_files","folder_path":""}' | socat - UNIX-CONNECT:~/.local/share/tuxdrive/daemon.sock | python3 -m json.tool

# Stop the daemon cleanly
echo '{"cmd":"shutdown"}' | socat - UNIX-CONNECT:~/.local/share/tuxdrive/daemon.sock
# or: kill $(pgrep tuxdrive-daemon)

# Log out
tuxdrive-daemon logout
```

---

## Auto-start on login

```bash
tuxdrive-daemon install-service
```

This installs a systemd user service. After a reboot (or `systemctl --user start tuxdrive`), the daemon starts automatically every time you log in.

Check service status:
```bash
systemctl --user status tuxdrive
journalctl --user -u tuxdrive -f   # live logs
```

---

## Watch live sync activity

```bash
tail -f ~/.local/share/tuxdrive/tuxdrive.log   # if log.file is set in config
# or watch the daemon start output directly
RUST_LOG=info tuxdrive-daemon start
```

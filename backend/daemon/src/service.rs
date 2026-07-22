use std::path::Path;
use std::process::Command;

use crate::config::Config;

const SERVICE_NAME: &str = "tuxdrive";

/// Install a systemd **user** service that starts automatically on login.
///
/// - Writes `~/.config/systemd/user/tuxdrive.service`
/// - Enables + starts the service immediately
/// - Enables lingering so the service survives across login sessions
pub fn install_systemd_service(exec_path: &Path) -> anyhow::Result<()> {
    let service_dir = Config::expand_path("~/.config/systemd/user");
    std::fs::create_dir_all(&service_dir)?;

    let service_file = service_dir.join(format!("{SERVICE_NAME}.service"));
    let exec_path_str = exec_path.display();

    let unit = format!(
        "[Unit]\n\
         Description=TuxDrive Google Drive Sync Daemon\n\
         Documentation=man:tuxdrive-daemon(1)\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exec_path_str}\n\
         Restart=on-failure\n\
         RestartSec=5s\n\
         RestartPreventExitStatus=2\n\
         Environment=RUST_LOG=info\n\
         StandardOutput=journal\n\
         StandardError=journal\n\
         SyslogIdentifier=tuxdrive\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    );

    std::fs::write(&service_file, &unit)?;
    tracing::info!(path = %service_file.display(), "wrote systemd unit file");

    run_systemctl(&["--user", "daemon-reload"])?;
    run_systemctl(&["--user", "enable", SERVICE_NAME])?;

    // Enable lingering: service persists across login/logout and starts at boot.
    let username = std::env::var("USER").or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    if !username.is_empty() {
        let _ = Command::new("loginctl")
            .args(["enable-linger", &username])
            .status();
    }

    // Start the service right now — no reboot or re-login needed.
    let _ = run_systemctl(&["--user", "start", SERVICE_NAME]);

    tracing::info!("systemd user service installed, enabled, and started");
    Ok(())
}

/// Remove the systemd user service.
pub fn uninstall_systemd_service() -> anyhow::Result<()> {
    // Disable first (ignore errors if it was never enabled).
    let _ = run_systemctl(&["--user", "disable", SERVICE_NAME]);

    let service_file =
        Config::expand_path(&format!("~/.config/systemd/user/{SERVICE_NAME}.service"));
    if service_file.exists() {
        std::fs::remove_file(&service_file)?;
        tracing::info!(path = %service_file.display(), "removed systemd unit file");
    }

    let _ = run_systemctl(&["--user", "daemon-reload"]);

    tracing::info!("systemd user service uninstalled");
    Ok(())
}

/// Install an XDG autostart `.desktop` file (fallback for non-systemd desktops).
pub fn install_xdg_autostart(exec_path: &Path) -> anyhow::Result<()> {
    let autostart_dir = Config::expand_path("~/.config/autostart");
    std::fs::create_dir_all(&autostart_dir)?;

    let desktop_file = autostart_dir.join(format!("{SERVICE_NAME}.desktop"));
    let exec_path_str = exec_path.display();

    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=TuxDrive Sync\n\
         Comment=Google Drive synchronization daemon\n\
         Exec={exec_path_str}\n\
         Hidden=false\n\
         NoDisplay=false\n\
         X-GNOME-Autostart-enabled=true\n"
    );

    std::fs::write(&desktop_file, entry)?;
    tracing::info!(path = %desktop_file.display(), "wrote XDG autostart desktop file");
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn run_systemctl(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("systemctl").args(args).status()?;
    if !status.success() {
        anyhow::bail!(
            "systemctl {} exited with status {}",
            args.join(" "),
            status
        );
    }
    Ok(())
}

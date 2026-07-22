use std::io::Write;

use tuxdrive_database::{Database, FileRecord};
use tuxdrive_drive::DriveClient;

use crate::config::Config;

/// Result of comparing sync history against what's actually on disk.
pub struct ConflictSummary {
    pub known_count: usize,
    pub missing: Vec<FileRecord>,
}

/// Compares what the sync database expects to exist locally against what's
/// actually on disk. Pure comparison — no prompts, no I/O side effects beyond
/// reading the database and the filesystem.
///
/// Returns `None` when there's nothing to reconcile: either there's no prior
/// sync history (a genuinely fresh setup downloads everything naturally via
/// the Changes API once polling starts), or every known file is present.
///
/// If there's prior history but files are missing, callers should offer to
/// re-download them from Drive. Without this check, the daemon's filesystem
/// watcher would interpret their absence as an intentional local deletion and
/// mirror it to Google Drive by trashing the remote copies — exactly the
/// failure mode that motivated this module.
pub async fn check_sync_conflict(
    cfg: &Config,
    db: &Database,
) -> anyhow::Result<Option<ConflictSummary>> {
    let sync_root = cfg.sync_root();
    std::fs::create_dir_all(&sync_root)?;

    let known_files: Vec<FileRecord> = db
        .list_all_files()
        .await?
        .into_iter()
        .filter(|f| f.is_folder == 0 && f.trashed == 0 && f.local_path.is_some())
        .collect();

    if known_files.is_empty() {
        return Ok(None);
    }

    let missing: Vec<FileRecord> = known_files
        .iter()
        .filter(|f| {
            let rel = f.local_path.as_deref().unwrap_or_default();
            !sync_root.join(rel).exists()
        })
        .cloned()
        .collect();

    if missing.is_empty() {
        return Ok(None);
    }

    Ok(Some(ConflictSummary {
        known_count: known_files.len(),
        missing,
    }))
}

/// Downloads every file in `missing` from Drive, invoking `on_progress` after
/// each attempt (success or failure) with `(done, total, relative_path)`.
/// Returns `(ok_count, failed_count)` — never fails outright on a per-file
/// download error; the caller decides what a partial failure means.
pub async fn download_missing_files(
    cfg: &Config,
    drive: &DriveClient,
    missing: &[FileRecord],
    mut on_progress: impl FnMut(usize, usize, &str),
) -> anyhow::Result<(usize, usize)> {
    let sync_root = cfg.sync_root();
    let total = missing.len();
    let mut ok = 0usize;
    let mut failed = 0usize;

    for record in missing {
        let rel = record.local_path.as_deref().unwrap_or_default();
        let dest = sync_root.join(rel);
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match tuxdrive_drive::files::download_file(drive, &record.id, &dest, cfg.sync.chunk_size_bytes)
            .await
        {
            Ok(()) => ok += 1,
            Err(_) => failed += 1,
        }
        on_progress(ok + failed, total, rel);
    }

    Ok((ok, failed))
}

/// Interactive CLI entry point — used by `tuxdrive-daemon login` when run in a
/// terminal. Prompts on `stdin`/`stdout`; GUI callers should use
/// [`check_sync_conflict`] and [`download_missing_files`] directly instead
/// (see `login_flow.rs`).
pub async fn resolve_sync_direction(
    cfg: &Config,
    db: &Database,
    drive: &DriveClient,
) -> anyhow::Result<()> {
    let Some(summary) = check_sync_conflict(cfg, db).await? else {
        return Ok(());
    };
    let missing = &summary.missing;
    let sync_root = cfg.sync_root();

    println!("\n⚠️  Sync history mismatch detected");
    println!(
        "    {} file(s) are recorded as already synced to {}, but {} of them are missing locally.",
        summary.known_count,
        sync_root.display(),
        missing.len()
    );
    println!("    This usually means the local folder was cleared, moved, or reinstalled —");
    println!("    not that you deleted these files on purpose.\n");
    println!("    If the daemon starts now without fixing this, it will treat the missing");
    println!("    files as intentional local deletions and remove them from Google Drive too.\n");
    print!(
        "Re-download the {} missing file(s) from Google Drive now? [Y/n] ",
        missing.len()
    );
    std::io::stdout().flush()?;

    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    let answer = answer.trim().to_lowercase();

    if answer.is_empty() || answer == "y" || answer == "yes" {
        println!("\nRe-downloading {} file(s)...", missing.len());
        let (ok, failed) = download_missing_files(cfg, drive, missing, |done, total, path| {
            println!("  [{done}/{total}] downloaded {path}");
        })
        .await?;
        println!("\nDone: {ok} downloaded, {failed} failed.\n");
        if failed > 0 {
            anyhow::bail!(
                "{failed} file(s) could not be re-downloaded — resolve manually before starting \
                 the daemon, otherwise they will be deleted from Google Drive on next sync."
            );
        }
        return Ok(());
    }

    // Declined — require an explicit, hard-to-fat-finger confirmation before
    // letting the daemon start and propagate deletions to Drive.
    println!(
        "\nTyping anything other than the exact phrase below will abort so nothing is deleted by accident."
    );
    print!(
        "Type DELETE to confirm you want tuxdrive to remove {} file(s) from Google Drive: ",
        missing.len()
    );
    std::io::stdout().flush()?;
    let mut confirm = String::new();
    std::io::stdin().read_line(&mut confirm)?;
    if confirm.trim() == "DELETE" {
        println!("Understood — starting the daemon will remove these files from Google Drive.\n");
        Ok(())
    } else {
        anyhow::bail!("Aborted — nothing was changed. Restore the local folder and run `tuxdrive-daemon login` again to continue.");
    }
}

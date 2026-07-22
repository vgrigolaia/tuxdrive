use std::path::Path;

/// Move all contents of `old_root` to `new_root`, refusing if the
/// destination already contains anything (never merges/overwrites). Uses a
/// same-filesystem `rename` when possible; falls back to a verified
/// recursive copy for cross-filesystem moves, only removing the source
/// after the copy has fully succeeded.
pub fn move_sync_folder(old_root: &Path, new_root: &Path) -> anyhow::Result<()> {
    if old_root == new_root {
        return Ok(());
    }

    if new_root.exists() {
        let has_entries = std::fs::read_dir(new_root)?.next().is_some();
        if has_entries {
            anyhow::bail!(
                "\"{}\" is not empty. Choose an empty or non-existent folder.",
                new_root.display()
            );
        }
        // rename() can replace an existing directory target only if it's
        // empty; remove it so the rename below always has a clean target.
        std::fs::remove_dir(new_root)?;
    } else if let Some(parent) = new_root.parent() {
        std::fs::create_dir_all(parent)?;
    }

    match std::fs::rename(old_root, new_root) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            copy_dir_recursive(old_root, new_root)?;
            std::fs::remove_dir_all(old_root)?;
            Ok(())
        }
        Err(e) => Err(e).map_err(|e: std::io::Error| {
            anyhow::anyhow!(
                "failed to move \"{}\" to \"{}\": {e}",
                old_root.display(),
                new_root.display()
            )
        }),
    }
}

/// Recursively copy `src` into `dst` (created if missing). Regular files
/// and directories only — symlinks are skipped rather than followed or
/// duplicated, since they shouldn't normally appear inside a synced folder.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

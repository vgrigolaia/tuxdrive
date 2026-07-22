use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::error::SyncError;

/// Rename a local file to a conflict copy and return the new path.
///
/// Naming scheme: `<stem>.conflict.<ISO8601-compact-timestamp>.<ext>`
/// Example: `report.conflict.20260715T143022Z.docx`
#[tracing::instrument(skip_all, fields(relative_path = %relative_path))]
pub async fn rename_to_conflict(
    sync_root: &Path,
    relative_path: &str,
) -> Result<PathBuf, SyncError> {
    let original = sync_root.join(relative_path);

    // Build the conflict filename.
    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();

    let conflict_name = {
        let stem = original
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let ext = original.extension().and_then(|s| s.to_str());

        match ext {
            Some(e) => format!("{}.conflict.{}.{}", stem, timestamp, e),
            None => format!("{}.conflict.{}", stem, timestamp),
        }
    };

    let conflict_path = original
        .parent()
        .map(|p| p.join(&conflict_name))
        .unwrap_or_else(|| PathBuf::from(&conflict_name));

    tokio::fs::rename(&original, &conflict_path).await?;

    tracing::info!(
        original = %original.display(),
        conflict = %conflict_path.display(),
        "renamed file to conflict copy"
    );

    Ok(conflict_path)
}

/// Return `true` if both local and remote changed since the last sync.
///
/// - Local changed: `local_checksum != last_synced_checksum`
/// - Remote changed: `remote_modified != last_synced_remote_mod`
pub async fn is_conflict(
    _sync_root: &Path,
    _relative_path: &str,
    local_checksum: &str,
    last_synced_checksum: &str,
    remote_modified: &str,
    last_synced_remote_mod: &str,
) -> bool {
    let local_changed = local_checksum != last_synced_checksum;
    let remote_changed = remote_modified != last_synced_remote_mod;
    local_changed && remote_changed
}

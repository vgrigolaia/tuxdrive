use std::fmt;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// SyncStatus enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    Uploading,
    Downloading,
    Conflict,
    Error,
    Pending,
}

impl fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            SyncStatus::Synced => "synced",
            SyncStatus::Uploading => "uploading",
            SyncStatus::Downloading => "downloading",
            SyncStatus::Conflict => "conflict",
            SyncStatus::Error => "error",
            SyncStatus::Pending => "pending",
        };
        f.write_str(s)
    }
}

impl FromStr for SyncStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "synced" => Ok(SyncStatus::Synced),
            "uploading" => Ok(SyncStatus::Uploading),
            "downloading" => Ok(SyncStatus::Downloading),
            "conflict" => Ok(SyncStatus::Conflict),
            "error" => Ok(SyncStatus::Error),
            "pending" => Ok(SyncStatus::Pending),
            other => Err(format!("unknown sync status: {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// FileRecord — mirrors the `files` table
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct FileRecord {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub parent_id: Option<String>,
    pub local_path: Option<String>,
    /// File size in bytes.
    pub size: i64,
    pub drive_checksum: Option<String>,
    pub local_checksum: Option<String>,
    /// RFC-3339 / ISO-8601 string from Google Drive.
    pub drive_modified: Option<String>,
    /// Unix timestamp (seconds) of the local file's mtime.
    pub local_modified: Option<i64>,
    /// Stored as TEXT in SQLite; corresponds to [`SyncStatus`].
    pub sync_status: String,
    /// SQLite stores booleans as INTEGER (0/1).
    pub is_folder: i64,
    pub trashed: i64,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// SyncState — mirrors the `sync_state` table
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct SyncState {
    pub local_path: String,
    pub drive_file_id: Option<String>,
    pub last_synced_checksum: Option<String>,
    pub last_synced_remote_mod: Option<String>,
    pub conflict: i64,
    pub sync_direction: Option<String>,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// ChangeToken — mirrors the `change_tokens` table
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct ChangeToken {
    pub account_email: String,
    pub page_token: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// UploadSession — mirrors the `upload_sessions` table
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct UploadSession {
    pub local_path: String,
    pub session_uri: String,
    pub offset: i64,
    pub total_size: i64,
    pub drive_file_id: Option<String>,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// SelectiveSync — mirrors the `selective_sync` table
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct SelectiveSync {
    pub drive_folder_id: String,
    pub folder_path: String,
    pub enabled: i64,
}

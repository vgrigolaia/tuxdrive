use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Drive error: {0}")]
    Drive(#[from] tuxdrive_drive::DriveError),
    #[error("Auth error: {0}")]
    Auth(#[from] tuxdrive_auth::AuthError),
    #[error("Database error: {0}")]
    Database(#[from] tuxdrive_database::DbError),
    #[error("Watcher error: {0}")]
    Watcher(#[from] tuxdrive_watcher::WatcherError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Conflict: {path}")]
    Conflict { path: String },
    #[error("Task cancelled")]
    Cancelled,
    #[error("Queue full")]
    QueueFull,
    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
}

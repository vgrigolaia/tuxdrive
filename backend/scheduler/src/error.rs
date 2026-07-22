use thiserror::Error;

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("Sync error: {0}")]
    Sync(String),
    #[error("Auth error: {0}")]
    Auth(#[from] tuxdrive_auth::AuthError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}

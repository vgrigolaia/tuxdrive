use thiserror::Error;

#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("Notify error: {0}")]
    Notify(#[from] notify::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Channel send error")]
    Send,
    #[error("Watcher already running")]
    AlreadyRunning,
    #[error("Watcher not running")]
    NotRunning,
}

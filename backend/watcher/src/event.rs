use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    Created,
    Modified,
    Deleted,
    Renamed { from: PathBuf, to: PathBuf },
}

#[derive(Debug, Clone)]
pub struct LocalEvent {
    /// Path relative to the sync root
    pub relative_path: PathBuf,
    /// Absolute path
    pub absolute_path: PathBuf,
    pub kind: EventKind,
    pub timestamp: std::time::SystemTime,
}

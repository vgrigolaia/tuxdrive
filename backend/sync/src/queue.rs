use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// SyncDirection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SyncDirection {
    Upload,
    Download,
    /// Delete remote file (local file was deleted).
    Delete,
    /// Delete local file (remote was deleted or trashed).
    DeleteLocal,
    CreateFolder,
    Move { old_path: String, new_path: String },
}

// ---------------------------------------------------------------------------
// SyncTask
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SyncTask {
    pub id: Uuid,
    /// Path relative to the sync root.
    pub local_path: String,
    pub drive_file_id: Option<String>,
    pub direction: SyncDirection,
    pub mime_type: String,
    pub size: u64,
    /// 0 = highest priority.
    pub priority: u8,
    pub created_at: DateTime<Utc>,
    pub retry_count: u32,
}

impl SyncTask {
    pub fn upload(local_path: String, size: u64, mime_type: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            local_path,
            drive_file_id: None,
            direction: SyncDirection::Upload,
            mime_type,
            size,
            priority: 1,
            created_at: Utc::now(),
            retry_count: 0,
        }
    }

    pub fn download(local_path: String, drive_file_id: String, size: u64) -> Self {
        Self {
            id: Uuid::new_v4(),
            local_path,
            drive_file_id: Some(drive_file_id),
            direction: SyncDirection::Download,
            mime_type: String::new(),
            size,
            priority: 1,
            created_at: Utc::now(),
            retry_count: 0,
        }
    }

    pub fn delete_remote(local_path: String, drive_file_id: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            local_path,
            drive_file_id: Some(drive_file_id),
            direction: SyncDirection::Delete,
            mime_type: String::new(),
            size: 0,
            priority: 2,
            created_at: Utc::now(),
            retry_count: 0,
        }
    }

    pub fn delete_local(local_path: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            local_path,
            drive_file_id: None,
            direction: SyncDirection::DeleteLocal,
            mime_type: String::new(),
            size: 0,
            priority: 2,
            created_at: Utc::now(),
            retry_count: 0,
        }
    }

    /// `drive_file_id`: `Some(id)` when this folder already exists on Drive
    /// (the task only needs to create the local mirror directory); `None`
    /// when it's a genuinely new local folder that still needs to be created
    /// on Drive too.
    pub fn create_folder(local_path: String, mime_type: String, drive_file_id: Option<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            local_path,
            drive_file_id,
            direction: SyncDirection::CreateFolder,
            mime_type,
            size: 0,
            priority: 0,
            created_at: Utc::now(),
            retry_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// SyncQueue
// ---------------------------------------------------------------------------

/// Thread-safe work queue with path-level deduplication.
pub struct SyncQueue {
    inner: parking_lot::Mutex<VecDeque<SyncTask>>,
    /// Maps `local_path` → `task_id` to prevent duplicate queuing.
    dedup: dashmap::DashMap<String, Uuid>,
}

impl SyncQueue {
    pub fn new() -> Self {
        Self {
            inner: parking_lot::Mutex::new(VecDeque::new()),
            dedup: dashmap::DashMap::new(),
        }
    }

    /// Enqueue a task. Returns `false` if a task for the same path is already
    /// queued (deduplication), `true` if the task was accepted.
    pub fn enqueue(&self, task: SyncTask) -> bool {
        // Use `entry` to atomically check-and-insert in the dedup map.
        use dashmap::mapref::entry::Entry;
        match self.dedup.entry(task.local_path.clone()) {
            Entry::Occupied(_) => false,
            Entry::Vacant(slot) => {
                slot.insert(task.id);
                self.inner.lock().push_back(task);
                true
            }
        }
    }

    /// Dequeue the next task, removing it from the dedup map as well.
    pub fn dequeue(&self) -> Option<SyncTask> {
        let task = self.inner.lock().pop_front()?;
        // Remove only if the stored id still matches this task (a re-enqueue
        // after remove_path would have a different id).
        self.dedup.remove_if(&task.local_path, |_, v| *v == task.id);
        Some(task)
    }

    /// Return the number of queued tasks.
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    /// Return `true` if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove any queued task for the given local path.
    pub fn remove_path(&self, local_path: &str) {
        // Remove from dedup map first so the id is no longer tracked.
        self.dedup.remove(local_path);
        // Then sweep the queue.
        let mut guard = self.inner.lock();
        guard.retain(|t| t.local_path != local_path);
    }
}

impl Default for SyncQueue {
    fn default() -> Self {
        Self::new()
    }
}

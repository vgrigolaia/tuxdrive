use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use chrono::Utc;
use tracing::{error, info, instrument, warn};

use tuxdrive_database::{
    models::{FileRecord, SyncState, UploadSession},
    Database,
};
use tuxdrive_drive::{files as drive_files, DriveClient};
use tuxdrive_watcher::{EventKind, LocalEvent};

use crate::error::SyncError;
use crate::queue::{SyncDirection, SyncQueue, SyncTask};

// ---------------------------------------------------------------------------
// SyncConfig
// ---------------------------------------------------------------------------

pub struct SyncConfig {
    pub sync_root: PathBuf,
    /// Currently unused by `SyncEngine` itself — Drive authentication for all
    /// engine operations flows through the `Arc<DriveClient>` it holds, whose
    /// account email is independently kept live (see `DriveClient`).
    pub account_email: String,
    /// Upload chunk size in bytes. Default: 8 MB.
    pub chunk_size: u64,
    /// Maximum number of concurrent worker tasks. Default: 4.
    pub max_concurrent: usize,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            sync_root: PathBuf::from("."),
            account_email: String::new(),
            chunk_size: 8 * 1024 * 1024,
            max_concurrent: 4,
        }
    }
}

// ---------------------------------------------------------------------------
// SyncEngine
// ---------------------------------------------------------------------------

/// A boxed, `Send` future — needed because `resolve_local_path` and
/// `resolve_folder_path` recurse into each other across `.await` points,
/// which `async fn` cannot do directly (infinitely-sized future).
type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

pub struct SyncEngine {
    config: SyncConfig,
    drive: Arc<DriveClient>,
    db: Arc<Database>,
    queue: Arc<SyncQueue>,
    paused: Arc<parking_lot::RwLock<bool>>,
    /// Set to `true` once `shutdown()` is called; workers check this flag.
    shutdown_flag: Arc<AtomicBool>,
    /// Notified on shutdown so sleeping workers wake up promptly.
    shutdown: Arc<tokio::sync::Notify>,
    /// Cached id of the Drive account's own root folder ("My Drive"), so
    /// top-level items resolve to the sync root instead of being nested
    /// under a synthetic folder for it.
    drive_root_id: tokio::sync::OnceCell<String>,
    /// Timestamps of the most recent task completions (success or permanent
    /// failure), used to estimate a live throughput rate for `eta_seconds`.
    completions: parking_lot::Mutex<VecDeque<Instant>>,
}

/// Number of recent completions kept for the throughput estimate.
const RATE_WINDOW: usize = 100;

impl SyncEngine {
    pub fn new(config: SyncConfig, drive: Arc<DriveClient>, db: Arc<Database>) -> Self {
        Self {
            config,
            drive,
            db,
            queue: Arc::new(SyncQueue::new()),
            paused: Arc::new(parking_lot::RwLock::new(false)),
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(tokio::sync::Notify::new()),
            drive_root_id: tokio::sync::OnceCell::new(),
            completions: parking_lot::Mutex::new(VecDeque::with_capacity(RATE_WINDOW)),
        }
    }

    // -----------------------------------------------------------------------
    // Public interface
    // -----------------------------------------------------------------------

    /// Translate a local filesystem event into a queued sync task.
    pub fn handle_local_event(&self, event: LocalEvent) {
        let rel = match event.relative_path.to_str() {
            Some(s) => s.to_owned(),
            None => {
                warn!("ignoring event with non-UTF-8 path");
                return;
            }
        };

        let task = match event.kind {
            EventKind::Created | EventKind::Modified => {
                // The watcher doesn't distinguish directory events from file
                // events (notify::EventKind::Create fires identically for
                // both). Treating a directory as an upload makes execute_upload
                // fail with "Is a directory" on every retry — skip it here.
                // This also fires harmlessly for directories *we* just
                // created while mirroring a remote folder locally.
                if event.absolute_path.is_dir() {
                    tracing::debug!(path = %rel, "ignoring directory create/modify event");
                    return;
                }
                // We don't know the size yet without a stat — use 0 as placeholder;
                // execute_upload will read the actual size from disk.
                SyncTask::upload(rel.clone(), 0, "application/octet-stream".into())
            }
            EventKind::Deleted => {
                // The drive_file_id is resolved during execution from the DB.
                SyncTask::delete_remote(rel.clone(), String::new())
            }
            EventKind::Renamed { from, to } => {
                let from_str = from.to_string_lossy().into_owned();
                let to_str = to.to_string_lossy().into_owned();
                SyncTask {
                    id: uuid::Uuid::new_v4(),
                    local_path: to_str.clone(),
                    drive_file_id: None,
                    direction: SyncDirection::Move {
                        old_path: from_str,
                        new_path: to_str,
                    },
                    mime_type: "application/octet-stream".into(),
                    size: 0,
                    priority: 1,
                    created_at: Utc::now(),
                    retry_count: 0,
                }
            }
        };

        if !self.queue.enqueue(task) {
            tracing::debug!(path = %rel, "task already queued, skipping duplicate");
        }
    }

    /// Queue remote Drive changes for sync.
    #[instrument(skip_all)]
    pub async fn handle_remote_changes(
        &self,
        changes: Vec<tuxdrive_drive::DriveChange>,
    ) -> Result<(), SyncError> {
        for change in changes {
            let file_id = match &change.file_id {
                Some(id) => id.clone(),
                None => continue,
            };

            // File was removed or trashed.
            if change.removed.unwrap_or(false) {
                // Determine local path from DB.
                if let Ok(record) = self.db.get_file_by_id(&file_id).await {
                    if let Some(local_path) = record.local_path {
                        let task = SyncTask::delete_local(local_path);
                        self.queue.enqueue(task);
                    }
                    if let Err(e) = self.db.delete_file(&file_id).await {
                        warn!(file_id = %file_id, error = %e, "failed to delete file record");
                    }
                }
                continue;
            }

            let drive_file = match &change.file {
                Some(f) => f.clone(),
                None => continue,
            };

            // Skip trashed files.
            if drive_file.trashed.unwrap_or(false) {
                if let Ok(record) = self.db.get_file_by_id(&file_id).await {
                    if let Some(local_path) = record.local_path {
                        let task = SyncTask::delete_local(local_path);
                        self.queue.enqueue(task);
                    }
                }
                continue;
            }

            let size: u64 = drive_file
                .size
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            let is_folder =
                drive_file.mime_type == "application/vnd.google-apps.folder";
            let export_target = drive_files::google_export_target(&drive_file.mime_type);
            let is_unsupported_native =
                !is_folder && drive_files::is_google_native(&drive_file.mime_type) && export_target.is_none();

            // Determine local path for this file, mirroring its actual
            // position in the Drive folder hierarchy (not just its bare name).
            let mut local_path = self
                .resolve_local_path(&drive_file.name, &drive_file.parents)
                .await;

            // Native Google Docs/Sheets/Slides/Drawings have no raw binary
            // content and must be exported to a real file format — give the
            // local copy the matching extension.
            if let Some((_, ext)) = export_target {
                if !local_path.ends_with(&format!(".{ext}")) {
                    local_path = format!("{local_path}.{ext}");
                }
            }

            if is_folder {
                let task = SyncTask::create_folder(
                    local_path.clone(),
                    drive_file.mime_type.clone(),
                    Some(file_id.clone()),
                );
                self.queue.enqueue(task);
            } else if is_unsupported_native {
                // Forms, Sites, Apps Script, Shortcuts, ... — no sensible
                // local file equivalent. Record it (so it shows up labeled
                // correctly instead of stuck "pending" forever) but don't
                // queue a download: it would just 403 permanently.
            } else {
                let mut task = SyncTask::download(local_path.clone(), file_id.clone(), size);
                if export_target.is_some() {
                    task.mime_type = drive_file.mime_type.clone();
                }
                self.queue.enqueue(task);
            }

            // Upsert the DB record.
            let now = Utc::now().to_rfc3339();
            let record = FileRecord {
                id: file_id.clone(),
                name: drive_file.name.clone(),
                mime_type: drive_file.mime_type.clone(),
                parent_id: drive_file
                    .parents
                    .as_ref()
                    .and_then(|p| p.first())
                    .cloned(),
                local_path: Some(local_path.clone()),
                size: size as i64,
                drive_checksum: drive_file.md5_checksum.clone(),
                local_checksum: None,
                drive_modified: drive_file.modified_time.clone(),
                local_modified: None,
                sync_status: if is_unsupported_native {
                    "unsupported".into()
                } else {
                    "pending".into()
                },
                is_folder: if is_folder { 1 } else { 0 },
                trashed: 0,
                created_at: now.clone(),
                updated_at: now,
            };

            if let Err(e) = self.db.upsert_file(&record).await {
                warn!(file_id = %file_id, error = %e, "failed to upsert file record for remote change");
            }
        }

        Ok(())
    }

    /// Spawn `config.max_concurrent` worker tasks and return a join handle that
    /// resolves when all workers have exited.
    pub fn start_workers(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let n = self.config.max_concurrent;
        let engine = Arc::clone(self);

        tokio::spawn(async move {
            let mut handles = Vec::with_capacity(n);

            for worker_id in 0..n {
                let eng = Arc::clone(&engine);
                let handle = tokio::spawn(async move {
                    eng.run_worker(worker_id).await;
                });
                handles.push(handle);
            }

            for h in handles {
                if let Err(e) = h.await {
                    error!(error = %e, "worker task panicked");
                }
            }
        })
    }

    /// Pause sync (workers will idle until resumed).
    pub fn pause(&self) {
        *self.paused.write() = true;
        info!("sync paused");
    }

    /// Resume sync.
    pub fn resume(&self) {
        *self.paused.write() = false;
        info!("sync resumed");
    }

    /// Return the number of tasks currently waiting in the sync queue.
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Record that a task just left the queue for good (succeeded, or failed
    /// permanently) — feeds the throughput estimate behind `eta_seconds`.
    fn record_completion(&self) {
        let mut window = self.completions.lock();
        window.push_back(Instant::now());
        if window.len() > RATE_WINDOW {
            window.pop_front();
        }
    }

    /// Estimate seconds remaining to drain the current queue, based on
    /// recent completion throughput. Returns `None` when there isn't enough
    /// recent history to extrapolate from (just started, or been idle for a
    /// while — an old rate isn't a fair estimate for a fresh batch).
    pub fn eta_seconds(&self) -> Option<u64> {
        let window = self.completions.lock();
        if window.len() < 5 {
            return None;
        }
        let newest = *window.back()?;
        if newest.elapsed() > Duration::from_secs(60) {
            return None;
        }
        let oldest = *window.front()?;
        let elapsed = newest.duration_since(oldest).as_secs_f64();
        if elapsed < 1.0 {
            return None;
        }
        let rate = (window.len() - 1) as f64 / elapsed; // tasks per second
        if rate <= 0.0 {
            return None;
        }
        let remaining = self.queue.len() as f64;
        Some((remaining / rate).round() as u64)
    }

    /// Signal all workers to stop. Workers exit after completing their current task.
    pub async fn shutdown(&self) {
        info!("sync engine shutting down");
        self.shutdown_flag.store(true, Ordering::Relaxed);
        // Wake any workers sleeping in `sleep` inside `run_worker`.
        self.shutdown.notify_waiters();
    }

    /// Run an initial full sync:
    /// 1. Paginate all Drive files → upsert DB, queue missing downloads.
    /// 2. Walk local sync root → queue uploads for files not in DB.
    #[instrument(skip_all)]
    pub async fn initial_sync(&self) -> Result<(), SyncError> {
        info!(sync_root = %self.config.sync_root.display(), "starting initial sync");

        // --- Phase 1: Drive → local ------------------------------------------
        let mut page_token: Option<String> = None;

        loop {
            let list = match drive_files::list_files(
                &self.drive,
                None,
                page_token.as_deref(),
            )
            .await
            {
                Ok(l) => l,
                Err(e) => {
                    error!(error = %e, "initial_sync: failed to list Drive files");
                    break;
                }
            };

            for drive_file in &list.files {
                let is_folder =
                    drive_file.mime_type == "application/vnd.google-apps.folder";
                let export_target = drive_files::google_export_target(&drive_file.mime_type);
                let is_unsupported_native = !is_folder
                    && drive_files::is_google_native(&drive_file.mime_type)
                    && export_target.is_none();
                let size: u64 = drive_file
                    .size
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);

                let mut local_path = self
                    .resolve_local_path(&drive_file.name, &drive_file.parents)
                    .await;
                if let Some((_, ext)) = export_target {
                    if !local_path.ends_with(&format!(".{ext}")) {
                        local_path = format!("{local_path}.{ext}");
                    }
                }
                let now = Utc::now().to_rfc3339();

                // Upsert the file record.
                let record = FileRecord {
                    id: drive_file.id.clone(),
                    name: drive_file.name.clone(),
                    mime_type: drive_file.mime_type.clone(),
                    parent_id: drive_file
                        .parents
                        .as_ref()
                        .and_then(|p| p.first())
                        .cloned(),
                    local_path: Some(local_path.clone()),
                    size: size as i64,
                    drive_checksum: drive_file.md5_checksum.clone(),
                    local_checksum: None,
                    drive_modified: drive_file.modified_time.clone(),
                    local_modified: None,
                    sync_status: if is_unsupported_native {
                        "unsupported".into()
                    } else {
                        "pending".into()
                    },
                    is_folder: if is_folder { 1 } else { 0 },
                    trashed: 0,
                    created_at: now.clone(),
                    updated_at: now,
                };

                if let Err(e) = self.db.upsert_file(&record).await {
                    warn!(
                        file_id = %drive_file.id,
                        error = %e,
                        "initial_sync: failed to upsert file record"
                    );
                    continue;
                }

                if is_unsupported_native {
                    // Forms, Sites, Apps Script, Shortcuts, ... — no sensible
                    // local file equivalent; already recorded above, nothing
                    // to download (it would just 403 permanently).
                    continue;
                }

                // If the file is not present locally, queue a download.
                let abs_path = self.config.sync_root.join(&local_path);
                if !abs_path.exists() {
                    if is_folder {
                        let task = SyncTask::create_folder(
                            local_path,
                            drive_file.mime_type.clone(),
                            Some(drive_file.id.clone()),
                        );
                        self.queue.enqueue(task);
                    } else {
                        let mut task = SyncTask::download(
                            local_path,
                            drive_file.id.clone(),
                            size,
                        );
                        if export_target.is_some() {
                            task.mime_type = drive_file.mime_type.clone();
                        }
                        self.queue.enqueue(task);
                    }
                }
            }

            match list.next_page_token {
                Some(tok) => page_token = Some(tok),
                None => break,
            }
        }

        // --- Phase 2: local → Drive ------------------------------------------
        self.walk_local_for_uploads().await;

        info!("initial sync queuing complete");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Worker loop
    // -----------------------------------------------------------------------

    async fn run_worker(&self, worker_id: usize) {
        loop {
            // Check the shutdown flag first (non-blocking, no await).
            if self.shutdown_flag.load(Ordering::Relaxed) {
                info!(worker_id, "worker received shutdown signal, exiting");
                return;
            }

            // Pause check.
            if *self.paused.read() {
                // Sleep interruptibly so shutdown wakes us up.
                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {}
                    _ = self.shutdown.notified() => {
                        info!(worker_id, "worker woken by shutdown during pause");
                        return;
                    }
                }
                continue;
            }

            // Pop a task.
            match self.queue.dequeue() {
                None => {
                    // No work: sleep briefly, interruptibly.
                    tokio::select! {
                        _ = tokio::time::sleep(tokio::time::Duration::from_millis(200)) => {}
                        _ = self.shutdown.notified() => {
                            info!(worker_id, "worker woken by shutdown while idle");
                            return;
                        }
                    }
                    continue;
                }
                Some(task) => {
                    let result = self.execute_task(&task).await;
                    match result {
                        Ok(()) => self.record_completion(),
                        Err(e) => {
                            if self.is_retryable(&e) && task.retry_count < 3 {
                                warn!(
                                    worker_id,
                                    path = %task.local_path,
                                    retry = task.retry_count + 1,
                                    error = %e,
                                    "task failed, re-enqueuing for retry"
                                );
                                let mut retry = task.clone();
                                retry.id = uuid::Uuid::new_v4();
                                retry.retry_count += 1;
                                self.queue.enqueue(retry);
                                // Not recorded as a completion yet — the task
                                // is still in flight (back in the queue).
                            } else {
                                error!(
                                    worker_id,
                                    path = %task.local_path,
                                    error = %e,
                                    "task failed permanently"
                                );
                                self.record_completion();
                            }
                        }
                    }
                }
            }
        }
    }

    async fn execute_task(&self, task: &SyncTask) -> Result<(), SyncError> {
        match &task.direction {
            SyncDirection::Upload => self.execute_upload(task).await,
            SyncDirection::Download => self.execute_download(task).await,
            SyncDirection::Delete => self.execute_delete_remote(task).await,
            SyncDirection::DeleteLocal => self.execute_delete_local(task).await,
            SyncDirection::CreateFolder => self.execute_create_folder(task).await,
            SyncDirection::Move { old_path, new_path } => {
                // Treat a move as: delete remote old + upload new.
                // Re-queue both sub-tasks.
                if let Ok(record) = self.db.get_file_by_path(old_path).await {
                    let del = SyncTask::delete_remote(
                        old_path.clone(),
                        record.id.clone(),
                    );
                    self.queue.enqueue(del);
                }
                let upload = SyncTask::upload(
                    new_path.clone(),
                    task.size,
                    task.mime_type.clone(),
                );
                self.queue.enqueue(upload);
                Ok(())
            }
        }
    }

    // -----------------------------------------------------------------------
    // Task executors
    // -----------------------------------------------------------------------

    #[instrument(skip(self), fields(path = %task.local_path))]
    async fn execute_upload(&self, task: &SyncTask) -> Result<(), SyncError> {
        let abs_path = self.config.sync_root.join(&task.local_path);

        // Compute checksum of the local file.
        let local_checksum = crate::checksum::file_checksum(&abs_path).await?;

        // If the checksum matches what we last synced, skip.
        if let Ok(Some(state)) = self.db.get_sync_state(&task.local_path).await {
            if let Some(ref last) = state.last_synced_checksum {
                if *last == local_checksum {
                    info!(path = %task.local_path, "upload skipped: checksum unchanged");
                    return Ok(());
                }
            }
        }

        let file_name = abs_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&task.local_path);

        let metadata = tokio::fs::metadata(&abs_path).await?;
        let file_size = metadata.len();

        const SIMPLE_UPLOAD_LIMIT: u64 = 5 * 1024 * 1024; // 5 MB

        let drive_file = if file_size < SIMPLE_UPLOAD_LIMIT {
            // Simple multipart upload.
            let data = tokio::fs::read(&abs_path).await?;
            let mime = if task.mime_type.is_empty() {
                "application/octet-stream"
            } else {
                &task.mime_type
            };
            drive_files::simple_upload(
                &self.drive,
                file_name,
                None,
                mime,
                Bytes::from(data),
            )
            .await?
        } else {
            // Resumable upload — check for an existing session.
            let completed_file = match self.db.get_upload_session(&task.local_path).await {
                Ok(Some(session)) => {
                    // Resume from where we left off.
                    let offset =
                        drive_files::query_upload_status(
                            &self.drive,
                            &session.session_uri,
                            session.total_size as u64,
                        )
                        .await
                        .unwrap_or(0);

                    self.upload_chunks_from_offset(
                        &abs_path,
                        &session.session_uri,
                        offset,
                        file_size,
                        &task.local_path,
                    )
                    .await?
                }
                _ => {
                    // Initiate a new resumable upload session.
                    let mime = if task.mime_type.is_empty() {
                        "application/octet-stream"
                    } else {
                        &task.mime_type
                    };
                    let upload_session_uri = drive_files::initiate_resumable_upload(
                        &self.drive,
                        file_name,
                        None,
                        mime,
                        file_size,
                    )
                    .await?;

                    // Persist the session so it can be resumed after a crash.
                    let now = Utc::now().to_rfc3339();
                    let session_record = UploadSession {
                        local_path: task.local_path.clone(),
                        session_uri: upload_session_uri.clone(),
                        offset: 0,
                        total_size: file_size as i64,
                        drive_file_id: None,
                        created_at: now,
                    };
                    if let Err(e) = self.db.save_upload_session(&session_record).await {
                        warn!(error = %e, "failed to persist upload session");
                    }

                    self.upload_chunks_from_offset(
                        &abs_path,
                        &upload_session_uri,
                        0,
                        file_size,
                        &task.local_path,
                    )
                    .await?
                }
            };

            completed_file
        };

        // Clean up upload session record (if any).
        let _ = self.db.delete_upload_session(&task.local_path).await;

        // Update DB file record.
        let now = Utc::now().to_rfc3339();
        let record = FileRecord {
            id: drive_file.id.clone(),
            name: drive_file.name.clone(),
            mime_type: drive_file.mime_type.clone(),
            parent_id: drive_file.parents.as_ref().and_then(|p| p.first()).cloned(),
            local_path: Some(task.local_path.clone()),
            size: file_size as i64,
            drive_checksum: drive_file.md5_checksum.clone(),
            local_checksum: Some(local_checksum.clone()),
            drive_modified: drive_file.modified_time.clone(),
            local_modified: None,
            sync_status: "synced".into(),
            is_folder: 0,
            trashed: 0,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        if let Err(e) = self.db.upsert_file(&record).await {
            warn!(error = %e, "failed to upsert file record after upload");
        }

        // Update sync state.
        let state = SyncState {
            local_path: task.local_path.clone(),
            drive_file_id: Some(drive_file.id.clone()),
            last_synced_checksum: Some(local_checksum),
            last_synced_remote_mod: drive_file.modified_time.clone(),
            conflict: 0,
            sync_direction: Some("upload".into()),
            updated_at: now,
        };
        if let Err(e) = self.db.upsert_sync_state(&state).await {
            warn!(error = %e, "failed to upsert sync state after upload");
        }

        info!(path = %task.local_path, "upload complete");
        Ok(())
    }

    #[instrument(skip(self), fields(path = %task.local_path))]
    async fn execute_download(&self, task: &SyncTask) -> Result<(), SyncError> {
        let file_id = match &task.drive_file_id {
            Some(id) => id.clone(),
            None => {
                warn!(path = %task.local_path, "download task missing drive_file_id");
                return Ok(());
            }
        };

        let abs_path = self.config.sync_root.join(&task.local_path);

        // Check for conflict if the file already exists locally.
        if abs_path.exists() {
            let local_checksum =
                crate::checksum::file_checksum(&abs_path).await.unwrap_or_default();

            if let Ok(Some(state)) = self.db.get_sync_state(&task.local_path).await {
                let last_checksum = state.last_synced_checksum.as_deref().unwrap_or("");
                let last_remote_mod = state.last_synced_remote_mod.as_deref().unwrap_or("");

                // Fetch remote metadata to get modified time.
                let remote_modified = match drive_files::get_file(&self.drive, &file_id).await {
                    Ok(f) => f.modified_time.unwrap_or_default(),
                    Err(e) => {
                        warn!(error = %e, "failed to get remote file metadata for conflict check");
                        String::new()
                    }
                };

                if crate::conflict::is_conflict(
                    &self.config.sync_root,
                    &task.local_path,
                    &local_checksum,
                    last_checksum,
                    &remote_modified,
                    last_remote_mod,
                )
                .await
                {
                    warn!(path = %task.local_path, "conflict detected");

                    // Rename local copy to conflict file.
                    let conflict_path = crate::conflict::rename_to_conflict(
                        &self.config.sync_root,
                        &task.local_path,
                    )
                    .await?;

                    // Queue the conflict copy for upload.
                    let conflict_rel = conflict_path
                        .strip_prefix(&self.config.sync_root)
                        .unwrap_or(&conflict_path)
                        .to_string_lossy()
                        .into_owned();

                    let conflict_size = tokio::fs::metadata(&conflict_path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0);

                    let upload_task = SyncTask::upload(
                        conflict_rel,
                        conflict_size,
                        "application/octet-stream".into(),
                    );
                    self.queue.enqueue(upload_task);

                    // Update sync state to reflect the conflict.
                    let now = Utc::now().to_rfc3339();
                    let conflict_state = SyncState {
                        local_path: task.local_path.clone(),
                        drive_file_id: Some(file_id.clone()),
                        last_synced_checksum: state.last_synced_checksum.clone(),
                        last_synced_remote_mod: state.last_synced_remote_mod.clone(),
                        conflict: 1,
                        sync_direction: Some("download".into()),
                        updated_at: now,
                    };
                    let _ = self.db.upsert_sync_state(&conflict_state).await;
                }
            }
        }

        // Ensure the parent directory exists.
        if let Some(parent) = abs_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Native Google Docs/Sheets/Slides/Drawings have no raw binary
        // content — `alt=media` always 403s for them. Export instead.
        match drive_files::google_export_target(&task.mime_type) {
            Some((export_mime, _ext)) => {
                drive_files::export_file(&self.drive, &file_id, export_mime, &abs_path).await?;
            }
            None => {
                drive_files::download_file(
                    &self.drive,
                    &file_id,
                    &abs_path,
                    self.config.chunk_size,
                )
                .await?;
            }
        }

        // Compute local checksum of the downloaded file.
        let local_checksum = crate::checksum::file_checksum(&abs_path).await?;

        // Fetch remote metadata to get the modified time.
        let remote_modified = match drive_files::get_file(&self.drive, &file_id).await {
            Ok(f) => f.modified_time,
            Err(e) => {
                warn!(error = %e, "failed to fetch remote file metadata after download");
                None
            }
        };

        // Update DB file record.
        let now = Utc::now().to_rfc3339();
        if let Ok(mut record) = self.db.get_file_by_id(&file_id).await {
            record.local_path = Some(task.local_path.clone());
            record.local_checksum = Some(local_checksum.clone());
            record.sync_status = "synced".into();
            record.updated_at = now.clone();
            if let Err(e) = self.db.upsert_file(&record).await {
                warn!(error = %e, "failed to update file record after download");
            }
        }

        // Update sync state.
        let state = SyncState {
            local_path: task.local_path.clone(),
            drive_file_id: Some(file_id),
            last_synced_checksum: Some(local_checksum),
            last_synced_remote_mod: remote_modified,
            conflict: 0,
            sync_direction: Some("download".into()),
            updated_at: now,
        };
        if let Err(e) = self.db.upsert_sync_state(&state).await {
            warn!(error = %e, "failed to upsert sync state after download");
        }

        info!(path = %task.local_path, "download complete");
        Ok(())
    }

    #[instrument(skip(self), fields(path = %task.local_path))]
    async fn execute_delete_remote(&self, task: &SyncTask) -> Result<(), SyncError> {
        // Resolve drive_file_id from task or DB.
        let file_id = match &task.drive_file_id {
            Some(id) if !id.is_empty() => id.clone(),
            _ => {
                // Look up from DB.
                match self.db.get_file_by_path(&task.local_path).await {
                    Ok(record) => record.id,
                    Err(e) => {
                        warn!(path = %task.local_path, error = %e, "delete_remote: file not found in DB");
                        return Ok(());
                    }
                }
            }
        };

        drive_files::trash_file(&self.drive, &file_id).await?;
        self.db.delete_file(&file_id).await?;

        info!(path = %task.local_path, file_id = %file_id, "remote file trashed");
        Ok(())
    }

    #[instrument(skip(self), fields(path = %task.local_path))]
    async fn execute_delete_local(&self, task: &SyncTask) -> Result<(), SyncError> {
        let abs_path = self.config.sync_root.join(&task.local_path);

        if abs_path.exists() {
            // Attempt XDG trash move first, fall back to permanent deletion.
            let trash_dir = dirs_trash_path();
            if let Some(trash) = trash_dir {
                if let Err(e) = tokio::fs::create_dir_all(&trash).await {
                    warn!(error = %e, "failed to create trash dir, will delete permanently");
                    tokio::fs::remove_file(&abs_path).await?;
                } else {
                    let file_name = abs_path
                        .file_name()
                        .unwrap_or(abs_path.as_os_str());
                    let dest = trash.join(file_name);
                    if let Err(e) = tokio::fs::rename(&abs_path, &dest).await {
                        // Cross-device rename may fail; fall back to delete.
                        warn!(error = %e, "failed to move file to trash, deleting permanently");
                        tokio::fs::remove_file(&abs_path).await?;
                    }
                }
            } else {
                tokio::fs::remove_file(&abs_path).await?;
            }
        }

        // Remove DB record for this path.
        if let Ok(record) = self.db.get_file_by_path(&task.local_path).await {
            if let Err(e) = self.db.delete_file(&record.id).await {
                warn!(error = %e, "failed to delete DB record during delete_local");
            }
        }

        info!(path = %task.local_path, "local file deleted");
        Ok(())
    }

    #[instrument(skip(self), fields(path = %task.local_path))]
    async fn execute_create_folder(&self, task: &SyncTask) -> Result<(), SyncError> {
        let abs_path = self.config.sync_root.join(&task.local_path);
        tokio::fs::create_dir_all(&abs_path).await?;

        // If we already know the Drive folder id, this folder mirrors an
        // existing Drive folder (remote -> local direction) and its DB
        // record was already upserted by the caller — there's nothing left
        // to do. Without this check, every remote folder got a brand-new
        // duplicate created on Drive on each sync pass.
        if task.drive_file_id.is_some() {
            info!(path = %task.local_path, "local folder created (mirrors existing Drive folder)");
            return Ok(());
        }

        // Otherwise this is a genuinely new local folder — create it on Drive too.
        let folder_name = abs_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&task.local_path);

        let result = drive_files::create_folder(&self.drive, folder_name, None).await;
        match result {
            Ok(drive_folder) => {
                let now = Utc::now().to_rfc3339();
                let record = FileRecord {
                    id: drive_folder.id.clone(),
                    name: drive_folder.name.clone(),
                    mime_type: drive_folder.mime_type.clone(),
                    parent_id: drive_folder
                        .parents
                        .as_ref()
                        .and_then(|p| p.first())
                        .cloned(),
                    local_path: Some(task.local_path.clone()),
                    size: 0,
                    drive_checksum: None,
                    local_checksum: None,
                    drive_modified: drive_folder.modified_time.clone(),
                    local_modified: None,
                    sync_status: "synced".into(),
                    is_folder: 1,
                    trashed: 0,
                    created_at: now.clone(),
                    updated_at: now,
                };
                if let Err(e) = self.db.upsert_file(&record).await {
                    warn!(error = %e, "failed to upsert folder record");
                }
            }
            Err(e) => {
                warn!(error = %e, path = %task.local_path, "failed to create Drive folder");
            }
        }

        info!(path = %task.local_path, "folder created");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Resolve the id of the Drive account's own root folder ("My Drive"),
    /// caching it after the first lookup.
    async fn drive_root_id(&self) -> Option<String> {
        if let Some(id) = self.drive_root_id.get() {
            return Some(id.clone());
        }
        match drive_files::get_file(&self.drive, "root").await {
            Ok(f) => {
                let _ = self.drive_root_id.set(f.id.clone());
                Some(f.id)
            }
            Err(e) => {
                warn!(error = %e, "failed to resolve Drive account root folder id");
                None
            }
        }
    }

    /// Resolve the full local relative path for a Drive item, mirroring its
    /// actual position in the Drive folder hierarchy instead of flattening
    /// everything into the sync root by bare name. Top-level items (whose
    /// parent is the account's own root folder, or which have no parent at
    /// all) resolve to just their own name.
    fn resolve_local_path<'a>(
        &'a self,
        name: &'a str,
        parents: &'a Option<Vec<String>>,
    ) -> BoxFuture<'a, String> {
        Box::pin(async move {
            let parent_id = match parents.as_ref().and_then(|p| p.first()) {
                Some(id) => id,
                None => return name.to_string(),
            };

            if let Some(root_id) = self.drive_root_id().await {
                if parent_id == &root_id {
                    return name.to_string();
                }
            }

            match self.resolve_folder_path(parent_id).await {
                Some(parent_path) if !parent_path.is_empty() => {
                    format!("{parent_path}/{name}")
                }
                _ => name.to_string(),
            }
        })
    }

    /// Resolve (and cache in the DB) the local relative path of a Drive
    /// folder by its id, recursively resolving its own ancestor chain if not
    /// already known. Self-healing against Drive's `files.list`/Changes API
    /// returning items in arbitrary order (a child can be seen before its
    /// parent folder) by fetching the parent directly from Drive when it
    /// isn't cached yet.
    fn resolve_folder_path<'a>(&'a self, folder_id: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            if let Ok(record) = self.db.get_file_by_id(folder_id).await {
                if let Some(path) = record.local_path {
                    return Some(path);
                }
            }

            let folder = match drive_files::get_file(&self.drive, folder_id).await {
                Ok(f) => f,
                Err(e) => {
                    warn!(folder_id = %folder_id, error = %e, "failed to resolve ancestor folder from Drive");
                    return None;
                }
            };

            let path = self.resolve_local_path(&folder.name, &folder.parents).await;

            let now = Utc::now().to_rfc3339();
            let record = FileRecord {
                id: folder_id.to_string(),
                name: folder.name.clone(),
                mime_type: folder.mime_type.clone(),
                parent_id: folder.parents.as_ref().and_then(|p| p.first()).cloned(),
                local_path: Some(path.clone()),
                size: 0,
                drive_checksum: None,
                local_checksum: None,
                drive_modified: folder.modified_time.clone(),
                local_modified: None,
                sync_status: "synced".into(),
                is_folder: 1,
                trashed: 0,
                created_at: now.clone(),
                updated_at: now,
            };
            if let Err(e) = self.db.upsert_file(&record).await {
                warn!(folder_id = %folder_id, error = %e, "failed to cache ancestor folder record");
            }

            Some(path)
        })
    }

    /// Upload file chunks starting from `start_offset`. Returns the completed
    /// [`tuxdrive_drive::DriveFile`] on success.
    async fn upload_chunks_from_offset(
        &self,
        abs_path: &std::path::Path,
        session_uri: &str,
        start_offset: u64,
        total_size: u64,
        local_path: &str,
    ) -> Result<tuxdrive_drive::DriveFile, SyncError> {
        use tokio::io::AsyncReadExt as _;
        use tokio::io::AsyncSeekExt as _;

        let mut file = tokio::fs::File::open(abs_path).await?;
        if start_offset > 0 {
            file.seek(std::io::SeekFrom::Start(start_offset)).await?;
        }

        let chunk_size = self.config.chunk_size;
        let mut offset = start_offset;
        let mut buf = vec![0u8; chunk_size as usize];

        loop {
            let n = file.read(&mut buf).await?;
            if n == 0 {
                return Err(SyncError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "unexpected EOF during resumable upload",
                )));
            }

            let chunk = Bytes::copy_from_slice(&buf[..n]);
            let (new_offset, maybe_file) = tuxdrive_drive::files::upload_chunk(
                &self.drive,
                session_uri,
                chunk,
                offset,
                total_size,
            )
            .await?;

            offset = new_offset;

            // Persist the updated offset so we can resume after a crash.
            if let Ok(Some(mut session)) = self.db.get_upload_session(local_path).await {
                session.offset = offset as i64;
                let _ = self.db.save_upload_session(&session).await;
            }

            if let Some(drive_file) = maybe_file {
                return Ok(drive_file);
            }
        }
    }

    /// Walk the local sync root and enqueue upload tasks for files not in DB.
    async fn walk_local_for_uploads(&self) {
        let sync_root = self.config.sync_root.clone();

        // Collect entries using blocking std::fs::read_dir wrapped in spawn_blocking.
        let entries: Vec<PathBuf> = match tokio::task::spawn_blocking(move || {
            collect_files_recursive(&sync_root)
        })
        .await
        {
            Ok(Ok(paths)) => paths,
            Ok(Err(e)) => {
                warn!(error = %e, "initial_sync: failed to walk local sync root");
                return;
            }
            Err(e) => {
                warn!(error = %e, "initial_sync: spawn_blocking panicked");
                return;
            }
        };

        for abs_path in entries {
            let rel = match abs_path.strip_prefix(&self.config.sync_root) {
                Ok(r) => r.to_string_lossy().into_owned(),
                Err(_) => continue,
            };

            // Check if already in DB.
            match self.db.get_file_by_path(&rel).await {
                Ok(_) => {
                    // Already known — skip.
                }
                Err(tuxdrive_database::DbError::NotFound(_)) => {
                    // Not in DB → queue for upload.
                    let size = tokio::fs::metadata(&abs_path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0);
                    let task = SyncTask::upload(rel, size, "application/octet-stream".into());
                    self.queue.enqueue(task);
                }
                Err(e) => {
                    warn!(path = %rel, error = %e, "initial_sync: DB lookup failed");
                }
            }
        }
    }

    /// Return `true` for errors that are worth retrying (network / rate limit).
    fn is_retryable(&self, err: &SyncError) -> bool {
        match err {
            SyncError::Drive(tuxdrive_drive::DriveError::RateLimited { .. }) => true,
            SyncError::Drive(tuxdrive_drive::DriveError::Http(_)) => true,
            SyncError::Io(_) => true,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// Resolve the XDG trash directory: `~/.local/share/Trash/files`.
fn dirs_trash_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local/share/Trash/files"))
}

/// Recursively collect all regular-file paths under `dir`.
fn collect_files_recursive(dir: &std::path::Path) -> std::io::Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    collect_recursive_inner(dir, &mut result)?;
    Ok(result)
}

fn collect_recursive_inner(dir: &std::path::Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            collect_recursive_inner(&path, out)?;
        } else if ft.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

use std::path::Path;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use tracing::{debug, instrument};

use crate::error::DbError;
use crate::models::{ChangeToken, FileRecord, SelectiveSync, SyncState, UploadSession};

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

/// SQLite-backed metadata store for TuxDrive.
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Open (or create) the SQLite database at `path`, running embedded
    /// migrations automatically.
    #[instrument(skip_all, fields(path = %path.display()))]
    pub async fn open(path: &Path) -> Result<Self, DbError> {
        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let connect_opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(16)
            .connect_with(connect_opts)
            .await?;

        // Run embedded migrations from the `./migrations` directory.
        sqlx::migrate!("./migrations").run(&pool).await?;

        debug!("database opened and migrations applied");
        Ok(Self { pool })
    }

    // -----------------------------------------------------------------------
    // Files
    // -----------------------------------------------------------------------

    /// Insert or replace a file record.
    #[instrument(skip(self, file), fields(id = %file.id))]
    pub async fn upsert_file(&self, file: &FileRecord) -> Result<(), DbError> {
        sqlx::query(
            r#"
            INSERT INTO files (
                id, name, mime_type, parent_id, local_path, size,
                drive_checksum, local_checksum, drive_modified, local_modified,
                sync_status, is_folder, trashed, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name            = excluded.name,
                mime_type       = excluded.mime_type,
                parent_id       = excluded.parent_id,
                local_path      = excluded.local_path,
                size            = excluded.size,
                drive_checksum  = excluded.drive_checksum,
                local_checksum  = excluded.local_checksum,
                drive_modified  = excluded.drive_modified,
                local_modified  = excluded.local_modified,
                sync_status     = excluded.sync_status,
                is_folder       = excluded.is_folder,
                trashed         = excluded.trashed,
                updated_at      = excluded.updated_at
            "#,
        )
        .bind(&file.id)
        .bind(&file.name)
        .bind(&file.mime_type)
        .bind(&file.parent_id)
        .bind(&file.local_path)
        .bind(file.size)
        .bind(&file.drive_checksum)
        .bind(&file.local_checksum)
        .bind(&file.drive_modified)
        .bind(file.local_modified)
        .bind(&file.sync_status)
        .bind(file.is_folder)
        .bind(file.trashed)
        .bind(&file.created_at)
        .bind(&file.updated_at)
        .execute(&self.pool)
        .await?;

        debug!(id = %file.id, "upserted file record");
        Ok(())
    }

    /// Fetch a single file record by its Drive file ID.
    #[instrument(skip(self), fields(id = %id))]
    pub async fn get_file_by_id(&self, id: &str) -> Result<FileRecord, DbError> {
        let row = sqlx::query_as::<_, FileRecord>("SELECT * FROM files WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        row.ok_or_else(|| DbError::NotFound(format!("file id={id}")))
    }

    /// Fetch a single file record by its local filesystem path.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn get_file_by_path(&self, path: &str) -> Result<FileRecord, DbError> {
        let row =
            sqlx::query_as::<_, FileRecord>("SELECT * FROM files WHERE local_path = ?")
                .bind(path)
                .fetch_optional(&self.pool)
                .await?;

        row.ok_or_else(|| DbError::NotFound(format!("file path={path}")))
    }

    /// Return all files whose `sync_status` is not `synced`.
    #[instrument(skip(self))]
    pub async fn list_files_needing_sync(&self) -> Result<Vec<FileRecord>, DbError> {
        let rows = sqlx::query_as::<_, FileRecord>(
            "SELECT * FROM files WHERE sync_status != 'synced' AND trashed = 0",
        )
        .fetch_all(&self.pool)
        .await?;

        debug!(count = rows.len(), "listed files needing sync");
        Ok(rows)
    }

    /// Mark a file as `synced` and update its local checksum.
    #[instrument(skip(self, local_checksum), fields(id = %id))]
    pub async fn mark_file_synced(
        &self,
        id: &str,
        local_checksum: &str,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE files SET sync_status = 'synced', local_checksum = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(local_checksum)
        .bind(id)
        .execute(&self.pool)
        .await?;

        debug!(id = %id, "marked file synced");
        Ok(())
    }

    /// Delete a file record by ID.
    #[instrument(skip(self), fields(id = %id))]
    pub async fn delete_file(&self, id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM files WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        debug!(id = %id, "deleted file record");
        Ok(())
    }

    /// Return every file record (non-trashed).
    #[instrument(skip(self))]
    pub async fn list_all_files(&self) -> Result<Vec<FileRecord>, DbError> {
        let rows = sqlx::query_as::<_, FileRecord>(
            "SELECT * FROM files WHERE trashed = 0 ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;

        debug!(count = rows.len(), "listed all files");
        Ok(rows)
    }

    // -----------------------------------------------------------------------
    // Sync state
    // -----------------------------------------------------------------------

    /// Insert or update a sync-state row for a local path.
    #[instrument(skip(self, state), fields(path = %state.local_path))]
    pub async fn upsert_sync_state(&self, state: &SyncState) -> Result<(), DbError> {
        sqlx::query(
            r#"
            INSERT INTO sync_state (
                local_path, drive_file_id, last_synced_checksum,
                last_synced_remote_mod, conflict, sync_direction, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(local_path) DO UPDATE SET
                drive_file_id           = excluded.drive_file_id,
                last_synced_checksum    = excluded.last_synced_checksum,
                last_synced_remote_mod  = excluded.last_synced_remote_mod,
                conflict                = excluded.conflict,
                sync_direction          = excluded.sync_direction,
                updated_at              = excluded.updated_at
            "#,
        )
        .bind(&state.local_path)
        .bind(&state.drive_file_id)
        .bind(&state.last_synced_checksum)
        .bind(&state.last_synced_remote_mod)
        .bind(state.conflict)
        .bind(&state.sync_direction)
        .bind(&state.updated_at)
        .execute(&self.pool)
        .await?;

        debug!(path = %state.local_path, "upserted sync state");
        Ok(())
    }

    /// Retrieve the sync state for a local path, if it exists.
    #[instrument(skip(self), fields(path = %local_path))]
    pub async fn get_sync_state(
        &self,
        local_path: &str,
    ) -> Result<Option<SyncState>, DbError> {
        let row = sqlx::query_as::<_, SyncState>(
            "SELECT * FROM sync_state WHERE local_path = ?",
        )
        .bind(local_path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // -----------------------------------------------------------------------
    // Change tokens
    // -----------------------------------------------------------------------

    /// Retrieve the stored Drive change page-token for an account.
    #[instrument(skip(self), fields(email = %email))]
    pub async fn get_change_token(&self, email: &str) -> Result<Option<String>, DbError> {
        let row = sqlx::query_as::<_, ChangeToken>(
            "SELECT * FROM change_tokens WHERE account_email = ?",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.page_token))
    }

    /// Persist (or overwrite) the Drive change page-token for an account.
    #[instrument(skip(self, token), fields(email = %email))]
    pub async fn save_change_token(&self, email: &str, token: &str) -> Result<(), DbError> {
        sqlx::query(
            r#"
            INSERT INTO change_tokens (account_email, page_token, updated_at)
            VALUES (?, ?, datetime('now'))
            ON CONFLICT(account_email) DO UPDATE SET
                page_token = excluded.page_token,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(email)
        .bind(token)
        .execute(&self.pool)
        .await?;

        debug!(email = %email, "saved change token");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Upload sessions
    // -----------------------------------------------------------------------

    /// Persist (or overwrite) a resumable-upload session record.
    #[instrument(skip(self, session), fields(path = %session.local_path))]
    pub async fn save_upload_session(&self, session: &UploadSession) -> Result<(), DbError> {
        sqlx::query(
            r#"
            INSERT INTO upload_sessions (
                local_path, session_uri, offset, total_size, drive_file_id, created_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(local_path) DO UPDATE SET
                session_uri   = excluded.session_uri,
                offset        = excluded.offset,
                total_size    = excluded.total_size,
                drive_file_id = excluded.drive_file_id
            "#,
        )
        .bind(&session.local_path)
        .bind(&session.session_uri)
        .bind(session.offset)
        .bind(session.total_size)
        .bind(&session.drive_file_id)
        .bind(&session.created_at)
        .execute(&self.pool)
        .await?;

        debug!(path = %session.local_path, "saved upload session");
        Ok(())
    }

    /// Retrieve a resumable-upload session by local path, if present.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn get_upload_session(
        &self,
        path: &str,
    ) -> Result<Option<UploadSession>, DbError> {
        let row = sqlx::query_as::<_, UploadSession>(
            "SELECT * FROM upload_sessions WHERE local_path = ?",
        )
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Remove a resumable-upload session record.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn delete_upload_session(&self, path: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM upload_sessions WHERE local_path = ?")
            .bind(path)
            .execute(&self.pool)
            .await?;

        debug!(path = %path, "deleted upload session");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Selective sync
    // -----------------------------------------------------------------------

    /// List all folder entries in the selective-sync table.
    #[instrument(skip(self))]
    pub async fn get_selective_sync_folders(&self) -> Result<Vec<SelectiveSync>, DbError> {
        let rows = sqlx::query_as::<_, SelectiveSync>(
            "SELECT * FROM selective_sync ORDER BY folder_path",
        )
        .fetch_all(&self.pool)
        .await?;

        debug!(count = rows.len(), "listed selective sync folders");
        Ok(rows)
    }

    /// Insert or update whether a Drive folder should be synced locally.
    #[instrument(skip(self), fields(folder_id = %folder_id, enabled = %enabled))]
    pub async fn set_folder_sync(
        &self,
        folder_id: &str,
        path: &str,
        enabled: bool,
    ) -> Result<(), DbError> {
        sqlx::query(
            r#"
            INSERT INTO selective_sync (drive_folder_id, folder_path, enabled)
            VALUES (?, ?, ?)
            ON CONFLICT(drive_folder_id) DO UPDATE SET
                folder_path = excluded.folder_path,
                enabled     = excluded.enabled
            "#,
        )
        .bind(folder_id)
        .bind(path)
        .bind(enabled as i64)
        .execute(&self.pool)
        .await?;

        debug!(folder_id = %folder_id, enabled = %enabled, "set folder sync preference");
        Ok(())
    }
}

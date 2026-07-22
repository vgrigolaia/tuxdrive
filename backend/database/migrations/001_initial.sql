CREATE TABLE IF NOT EXISTS files (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    mime_type       TEXT NOT NULL DEFAULT '',
    parent_id       TEXT,
    local_path      TEXT,
    size            INTEGER NOT NULL DEFAULT 0,
    drive_checksum  TEXT,
    local_checksum  TEXT,
    drive_modified  TEXT,
    local_modified  INTEGER,
    sync_status     TEXT NOT NULL DEFAULT 'synced',
    is_folder       INTEGER NOT NULL DEFAULT 0,
    trashed         INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_files_local_path ON files(local_path);
CREATE INDEX IF NOT EXISTS idx_files_parent_id ON files(parent_id);
CREATE INDEX IF NOT EXISTS idx_files_sync_status ON files(sync_status);

CREATE TABLE IF NOT EXISTS sync_state (
    local_path              TEXT PRIMARY KEY,
    drive_file_id           TEXT,
    last_synced_checksum    TEXT,
    last_synced_remote_mod  TEXT,
    conflict                INTEGER NOT NULL DEFAULT 0,
    sync_direction          TEXT,
    updated_at              TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS change_tokens (
    account_email   TEXT PRIMARY KEY,
    page_token      TEXT NOT NULL,
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS upload_sessions (
    local_path      TEXT PRIMARY KEY,
    session_uri     TEXT NOT NULL,
    offset          INTEGER NOT NULL DEFAULT 0,
    total_size      INTEGER NOT NULL,
    drive_file_id   TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS selective_sync (
    drive_folder_id TEXT PRIMARY KEY,
    folder_path     TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1
);

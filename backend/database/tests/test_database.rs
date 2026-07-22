/// Integration tests for the SQLite database layer.
///
/// These create a real temporary SQLite database on disk.
/// Run with:
///   cargo test --test test_database
use tuxdrive_database::{Database, FileRecord, SyncState};
use tempfile::tempdir;

async fn open_db() -> (Database, tempfile::TempDir) {
    let dir = tempdir().expect("temp dir");
    let db_path = dir.path().join("test.db");
    let db = Database::open(&db_path).await.expect("open database");
    (db, dir)
}

fn sample_file(id: &str, name: &str) -> FileRecord {
    FileRecord {
        id: id.to_owned(),
        name: name.to_owned(),
        mime_type: "application/pdf".to_owned(),
        parent_id: None,
        local_path: Some(name.to_owned()),
        size: 1024,
        drive_checksum: Some("abc123".to_owned()),
        local_checksum: Some("def456".to_owned()),
        drive_modified: Some("2026-01-15T12:00:00Z".to_owned()),
        local_modified: Some(1_705_312_800),
        sync_status: "synced".to_owned(),
        is_folder: 0,
        trashed: 0,
        created_at: "2026-01-15T12:00:00Z".to_owned(),
        updated_at: "2026-01-15T12:00:00Z".to_owned(),
    }
}

#[tokio::test]
async fn upsert_and_get_file_by_id() {
    let (db, _dir) = open_db().await;
    let file = sample_file("file-001", "report.pdf");

    db.upsert_file(&file).await.expect("upsert");
    let fetched = db.get_file_by_id("file-001").await.expect("get by id");

    assert_eq!(fetched.id, "file-001");
    assert_eq!(fetched.name, "report.pdf");
    assert_eq!(fetched.size, 1024);
    assert_eq!(fetched.sync_status, "synced");
}

#[tokio::test]
async fn get_file_by_path() {
    let (db, _dir) = open_db().await;
    db.upsert_file(&sample_file("file-002", "notes.txt"))
        .await
        .expect("upsert");

    let fetched = db.get_file_by_path("notes.txt").await.expect("get by path");
    assert_eq!(fetched.id, "file-002");
}

#[tokio::test]
async fn list_all_files() {
    let (db, _dir) = open_db().await;
    db.upsert_file(&sample_file("f1", "a.txt")).await.unwrap();
    db.upsert_file(&sample_file("f2", "b.txt")).await.unwrap();
    db.upsert_file(&sample_file("f3", "c.txt")).await.unwrap();

    let files = db.list_all_files().await.expect("list");
    assert_eq!(files.len(), 3);
}

#[tokio::test]
async fn delete_file_removes_record() {
    let (db, _dir) = open_db().await;
    db.upsert_file(&sample_file("del-1", "to_delete.txt"))
        .await
        .unwrap();

    db.delete_file("del-1").await.expect("delete");

    let result = db.get_file_by_id("del-1").await;
    assert!(result.is_err(), "should be gone after delete");
}

#[tokio::test]
async fn upsert_is_idempotent() {
    let (db, _dir) = open_db().await;
    let mut file = sample_file("idem-1", "idem.txt");
    db.upsert_file(&file).await.unwrap();

    // Update the file and upsert again.
    file.sync_status = "uploading".to_owned();
    file.size = 2048;
    db.upsert_file(&file).await.unwrap();

    let fetched = db.get_file_by_id("idem-1").await.unwrap();
    assert_eq!(fetched.sync_status, "uploading");
    assert_eq!(fetched.size, 2048);
}

#[tokio::test]
async fn change_token_roundtrip() {
    let (db, _dir) = open_db().await;

    assert!(db.get_change_token("user@example.com").await.unwrap().is_none());

    db.save_change_token("user@example.com", "token-abc")
        .await
        .expect("save token");
    let token = db.get_change_token("user@example.com").await.unwrap();
    assert_eq!(token.as_deref(), Some("token-abc"));

    // Overwrite.
    db.save_change_token("user@example.com", "token-xyz")
        .await
        .unwrap();
    let token2 = db.get_change_token("user@example.com").await.unwrap();
    assert_eq!(token2.as_deref(), Some("token-xyz"));
}

#[tokio::test]
async fn sync_state_roundtrip() {
    let (db, _dir) = open_db().await;

    let state = SyncState {
        local_path: "docs/project.md".to_owned(),
        drive_file_id: Some("drive-111".to_owned()),
        last_synced_checksum: Some("checksum-aaa".to_owned()),
        last_synced_remote_mod: Some("2026-03-01T00:00:00Z".to_owned()),
        conflict: 0,
        sync_direction: Some("download".to_owned()),
        updated_at: "2026-03-01T00:00:00Z".to_owned(),
    };

    db.upsert_sync_state(&state).await.expect("upsert state");

    let fetched = db.get_sync_state("docs/project.md").await.unwrap();
    let fetched = fetched.expect("state must exist");
    assert_eq!(fetched.drive_file_id, Some("drive-111".to_owned()));
    assert_eq!(fetched.conflict, 0);
}

#[tokio::test]
async fn selective_sync_roundtrip() {
    let (db, _dir) = open_db().await;

    db.set_folder_sync("folder-123", "Work/Projects", true)
        .await
        .expect("set");

    let folders = db.get_selective_sync_folders().await.expect("get");
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].drive_folder_id, "folder-123");
    assert_eq!(folders[0].folder_path, "Work/Projects");
    assert_eq!(folders[0].enabled, 1);

    // Disable it.
    db.set_folder_sync("folder-123", "Work/Projects", false)
        .await
        .unwrap();
    let folders2 = db.get_selective_sync_folders().await.unwrap();
    assert_eq!(folders2[0].enabled, 0);
}

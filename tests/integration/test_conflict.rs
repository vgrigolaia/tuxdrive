/// Integration tests for conflict detection and rename logic.
///
/// Run with:
///   cargo test --test test_conflict
use tuxdrive_sync::{is_conflict, rename_to_conflict};
use std::io::Write;
use tempfile::{tempdir, NamedTempFile};

#[tokio::test]
async fn no_conflict_when_neither_changed() {
    let root = tempdir().expect("temp dir");
    let result = is_conflict(
        root.path(),
        "file.txt",
        "aabbcc",  // local checksum
        "aabbcc",  // last synced checksum — SAME → local unchanged
        "2026-01-01T00:00:00Z",  // remote modified
        "2026-01-01T00:00:00Z",  // last synced remote mod — SAME → remote unchanged
    )
    .await;
    assert!(!result, "should not be a conflict");
}

#[tokio::test]
async fn no_conflict_when_only_local_changed() {
    let root = tempdir().expect("temp dir");
    let result = is_conflict(
        root.path(),
        "file.txt",
        "newchecksum",   // local checksum changed
        "oldchecksum",   // last synced checksum
        "2026-01-01T00:00:00Z",  // remote modified
        "2026-01-01T00:00:00Z",  // last synced — same
    )
    .await;
    assert!(!result, "only local changed — not a conflict, just an upload");
}

#[tokio::test]
async fn no_conflict_when_only_remote_changed() {
    let root = tempdir().expect("temp dir");
    let result = is_conflict(
        root.path(),
        "file.txt",
        "samechecksum",  // local checksum unchanged
        "samechecksum",  // last synced
        "2026-06-01T00:00:00Z",  // remote modified NEWER
        "2026-01-01T00:00:00Z",  // last synced remote mod — different
    )
    .await;
    assert!(!result, "only remote changed — not a conflict");
}

#[tokio::test]
async fn conflict_when_both_changed() {
    let root = tempdir().expect("temp dir");
    let result = is_conflict(
        root.path(),
        "file.txt",
        "local-new-checksum",   // local changed
        "old-checksum",         // last synced
        "2026-06-01T00:00:00Z", // remote changed
        "2026-01-01T00:00:00Z", // last synced remote mod
    )
    .await;
    assert!(result, "both changed → conflict");
}

#[tokio::test]
async fn rename_to_conflict_renames_file() {
    let root = tempdir().expect("temp dir");
    let file_path = root.path().join("report.docx");
    std::fs::write(&file_path, b"original content").expect("write");

    let conflict_path = rename_to_conflict(root.path(), "report.docx")
        .await
        .expect("rename_to_conflict");

    // Original must be gone.
    assert!(!file_path.exists(), "original must have been renamed");

    // Conflict copy must exist.
    assert!(conflict_path.exists(), "conflict copy must exist");

    // Conflict file name must contain "conflict".
    let name = conflict_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(
        name.contains("conflict"),
        "conflict filename must contain 'conflict': {name}"
    );

    // Extension must be preserved.
    assert!(
        name.ends_with(".docx"),
        "extension must be preserved: {name}"
    );
}

#[tokio::test]
async fn rename_to_conflict_no_extension() {
    let root = tempdir().expect("temp dir");
    let file_path = root.path().join("Makefile");
    std::fs::write(&file_path, b"content").expect("write");

    let conflict_path = rename_to_conflict(root.path(), "Makefile")
        .await
        .expect("rename_to_conflict");

    assert!(!file_path.exists());
    assert!(conflict_path.exists());
    let name = conflict_path.file_name().unwrap().to_string_lossy().to_string();
    assert!(name.starts_with("Makefile.conflict"), "bad name: {name}");
}

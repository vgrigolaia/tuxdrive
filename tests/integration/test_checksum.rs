/// Integration tests for the checksum module.
///
/// Run with:
///   cargo test --test test_checksum
use tuxdrive_sync::bytes_checksum;
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn known_sha256_empty() {
    // SHA-256 of empty string
    let hash = bytes_checksum(&[]);
    assert_eq!(
        hash,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn known_sha256_hello() {
    let hash = bytes_checksum(b"hello");
    assert_eq!(
        hash,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

#[test]
fn bytes_checksum_deterministic() {
    let data = b"deterministic test data";
    assert_eq!(bytes_checksum(data), bytes_checksum(data));
}

#[test]
fn different_inputs_different_hashes() {
    assert_ne!(bytes_checksum(b"aaa"), bytes_checksum(b"bbb"));
}

#[tokio::test]
async fn file_checksum_matches_bytes_checksum() {
    let content = b"file content for checksum test";
    let mut tmp = NamedTempFile::new().expect("temp file");
    tmp.write_all(content).expect("write");
    tmp.flush().expect("flush");

    let from_file = tuxdrive_sync::file_checksum(tmp.path())
        .await
        .expect("file_checksum");
    let from_bytes = bytes_checksum(content);

    assert_eq!(from_file, from_bytes);
}

#[tokio::test]
async fn file_checksum_missing_file_returns_error() {
    let result = tuxdrive_sync::file_checksum(std::path::Path::new("/nonexistent/path/file.txt"))
        .await;
    assert!(result.is_err(), "expected error for missing file");
}

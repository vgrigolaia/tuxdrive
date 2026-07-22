use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::io::AsyncReadExt;

use crate::error::SyncError;

const CHUNK_SIZE: usize = 64 * 1024; // 64 KB

/// Compute SHA-256 hex digest of a file, reading in 64 KB chunks.
#[tracing::instrument(skip_all, fields(path = %path.display()))]
pub async fn file_checksum(path: &Path) -> Result<String, SyncError> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_SIZE];

    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    let digest = hasher.finalize();
    Ok(hex::encode(digest))
}

/// Compute SHA-256 hex digest of in-memory bytes.
pub fn bytes_checksum(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

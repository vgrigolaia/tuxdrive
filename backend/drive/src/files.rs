use std::path::Path;

use bytes::Bytes;
use tracing::{debug, info, warn};

use crate::{
    client::{DriveClient, DRIVE_API, UPLOAD_API},
    error::DriveError,
    models::{DriveFile, FileList},
};

const FILE_FIELDS: &str =
    "id,name,mimeType,size,md5Checksum,parents,modifiedTime,createdTime,trashed,webViewLink";

// ---------------------------------------------------------------------------
// List files
// ---------------------------------------------------------------------------

pub async fn list_files(
    client: &DriveClient,
    folder_id: Option<&str>,
    page_token: Option<&str>,
) -> Result<FileList, DriveError> {
    let token = client.get_token().await?;

    let q = match folder_id {
        Some(id) => format!("trashed=false and '{}' in parents", id),
        None => "trashed=false".to_owned(),
    };

    let fields = format!("files({}),nextPageToken,incompleteSearch", FILE_FIELDS);

    let mut req = client
        .http
        .get(format!("{}/files", DRIVE_API))
        .bearer_auth(&token)
        .query(&[
            ("fields", fields.as_str()),
            ("pageSize", "1000"),
            ("q", q.as_str()),
        ]);

    if let Some(pt) = page_token {
        req = req.query(&[("pageToken", pt)]);
    }

    debug!(folder_id = ?folder_id, "listing Drive files");
    let resp = req.send().await?;
    let resp = DriveClient::check_response(resp).await?;
    let list: FileList = resp.json().await?;
    debug!(count = list.files.len(), "received file list");
    Ok(list)
}

// ---------------------------------------------------------------------------
// Get file metadata
// ---------------------------------------------------------------------------

pub async fn get_file(client: &DriveClient, file_id: &str) -> Result<DriveFile, DriveError> {
    let token = client.get_token().await?;

    let url = format!("{}/files/{}", DRIVE_API, file_id);
    debug!(file_id, "fetching file metadata");

    let resp = client
        .http
        .get(&url)
        .bearer_auth(&token)
        .query(&[("fields", FILE_FIELDS)])
        .send()
        .await?;

    let resp = DriveClient::check_response(resp).await?;
    let file: DriveFile = resp.json().await?;
    Ok(file)
}

// ---------------------------------------------------------------------------
// Create folder
// ---------------------------------------------------------------------------

pub async fn create_folder(
    client: &DriveClient,
    name: &str,
    parent_id: Option<&str>,
) -> Result<DriveFile, DriveError> {
    let token = client.get_token().await?;

    let mut meta = serde_json::json!({
        "name": name,
        "mimeType": "application/vnd.google-apps.folder",
    });

    if let Some(pid) = parent_id {
        meta["parents"] = serde_json::json!([pid]);
    }

    debug!(name, parent_id = ?parent_id, "creating folder");

    let resp = client
        .http
        .post(format!("{}/files", DRIVE_API))
        .bearer_auth(&token)
        .query(&[("fields", FILE_FIELDS)])
        .json(&meta)
        .send()
        .await?;

    let resp = DriveClient::check_response(resp).await?;
    let file: DriveFile = resp.json().await?;
    info!(id = %file.id, name, "folder created");
    Ok(file)
}

// ---------------------------------------------------------------------------
// Trash a file
// ---------------------------------------------------------------------------

pub async fn trash_file(client: &DriveClient, file_id: &str) -> Result<(), DriveError> {
    let token = client.get_token().await?;

    let url = format!("{}/files/{}", DRIVE_API, file_id);
    debug!(file_id, "trashing file");

    let resp = client
        .http
        .patch(&url)
        .bearer_auth(&token)
        .json(&serde_json::json!({ "trashed": true }))
        .send()
        .await?;

    DriveClient::check_response(resp).await?;
    info!(file_id, "file trashed");
    Ok(())
}

// ---------------------------------------------------------------------------
// Permanently delete a file
// ---------------------------------------------------------------------------

pub async fn delete_file(client: &DriveClient, file_id: &str) -> Result<(), DriveError> {
    let token = client.get_token().await?;

    let url = format!("{}/files/{}", DRIVE_API, file_id);
    debug!(file_id, "permanently deleting file");

    let resp = client
        .http
        .delete(&url)
        .bearer_auth(&token)
        .send()
        .await?;

    DriveClient::check_response(resp).await?;
    info!(file_id, "file permanently deleted");
    Ok(())
}

// ---------------------------------------------------------------------------
// Simple upload (< 5 MB)
// ---------------------------------------------------------------------------

pub async fn simple_upload(
    client: &DriveClient,
    name: &str,
    parent_id: Option<&str>,
    mime_type: &str,
    data: Bytes,
) -> Result<DriveFile, DriveError> {
    let token = client.get_token().await?;

    let mut meta = serde_json::json!({ "name": name });
    if let Some(pid) = parent_id {
        meta["parents"] = serde_json::json!([pid]);
    }

    let meta_bytes = serde_json::to_vec(&meta)?;
    let data_len = data.len();

    // Multipart body: metadata part + media part
    let boundary = "TUXDRIVE_DRIVE_BOUNDARY_01234567890";
    let mut body: Vec<u8> = Vec::new();

    // -- metadata part
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(&meta_bytes);
    body.extend_from_slice(b"\r\n");

    // -- media part
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        format!("Content-Type: {}\r\n\r\n", mime_type).as_bytes(),
    );
    body.extend_from_slice(&data);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

    let content_type = format!("multipart/related; boundary={}", boundary);

    debug!(name, data_len, "simple multipart upload");

    let resp = client
        .http
        .post(format!("{}/files", UPLOAD_API))
        .bearer_auth(&token)
        .query(&[("uploadType", "multipart"), ("fields", FILE_FIELDS)])
        .header("Content-Type", content_type)
        .body(body)
        .send()
        .await?;

    let resp = DriveClient::check_response(resp).await?;
    let file: DriveFile = resp.json().await?;
    info!(id = %file.id, name, "simple upload complete");
    Ok(file)
}

// ---------------------------------------------------------------------------
// Initiate a resumable upload session
// ---------------------------------------------------------------------------

pub async fn initiate_resumable_upload(
    client: &DriveClient,
    name: &str,
    parent_id: Option<&str>,
    mime_type: &str,
    total_size: u64,
) -> Result<String, DriveError> {
    let token = client.get_token().await?;

    let mut meta = serde_json::json!({ "name": name });
    if let Some(pid) = parent_id {
        meta["parents"] = serde_json::json!([pid]);
    }

    debug!(name, total_size, "initiating resumable upload");

    let resp = client
        .http
        .post(format!("{}/files", UPLOAD_API))
        .bearer_auth(&token)
        .query(&[("uploadType", "resumable"), ("fields", FILE_FIELDS)])
        .header("X-Upload-Content-Type", mime_type)
        .header("X-Upload-Content-Length", total_size.to_string())
        .json(&meta)
        .send()
        .await?;

    let resp = DriveClient::check_response(resp).await?;

    let session_uri = resp
        .headers()
        .get("Location")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .ok_or_else(|| DriveError::Api {
            status: 200,
            message: "Missing Location header in resumable upload response".into(),
        })?;

    info!(name, session_uri, "resumable upload session initiated");
    Ok(session_uri)
}

// ---------------------------------------------------------------------------
// Upload a chunk to a resumable session
// ---------------------------------------------------------------------------

pub async fn upload_chunk(
    client: &DriveClient,
    session_uri: &str,
    data: Bytes,
    offset: u64,
    total_size: u64,
) -> Result<(u64, Option<DriveFile>), DriveError> {
    let token = client.get_token().await?;
    let chunk_len = data.len() as u64;
    let end = offset + chunk_len - 1;
    let content_range = format!("bytes {}-{}/{}", offset, end, total_size);

    debug!(offset, chunk_len, total_size, "uploading chunk");

    let resp = client
        .http
        .put(session_uri)
        .bearer_auth(&token)
        .header("Content-Range", &content_range)
        .header("Content-Length", chunk_len.to_string())
        .body(data)
        .send()
        .await?;

    let status = resp.status().as_u16();

    // 308 Resume Incomplete — chunk accepted, not yet finished.
    if status == 308 {
        // Range header in response tells us how many bytes the server has received.
        let uploaded = resp
            .headers()
            .get("Range")
            .and_then(|v| v.to_str().ok())
            .and_then(|r| r.split('-').nth(1))
            .and_then(|s| s.parse::<u64>().ok())
            .map(|last| last + 1)
            .unwrap_or(offset + chunk_len);
        debug!(uploaded, "chunk accepted, upload incomplete");
        return Ok((uploaded, None));
    }

    // 200 / 201 — upload complete, body contains DriveFile.
    if status == 200 || status == 201 {
        let file: DriveFile = resp.json().await?;
        info!(id = %file.id, "resumable upload complete");
        return Ok((total_size, Some(file)));
    }

    // Anything else is an error — delegate to check_response for a tidy message.
    Err(DriveClient::check_response(resp).await.unwrap_err())
}

// ---------------------------------------------------------------------------
// Query upload status (for resuming after crash)
// ---------------------------------------------------------------------------

pub async fn query_upload_status(
    client: &DriveClient,
    session_uri: &str,
    total_size: u64,
) -> Result<u64, DriveError> {
    let token = client.get_token().await?;

    debug!(session_uri, "querying resumable upload status");

    let resp = client
        .http
        .put(session_uri)
        .bearer_auth(&token)
        .header("Content-Range", format!("bytes */{}", total_size))
        .header("Content-Length", "0")
        .send()
        .await?;

    let status = resp.status().as_u16();

    if status == 308 {
        // Upload still incomplete; Range header tells us what's been received.
        let next_byte = resp
            .headers()
            .get("Range")
            .and_then(|v| v.to_str().ok())
            .and_then(|r| r.split('-').nth(1))
            .and_then(|s| s.parse::<u64>().ok())
            .map(|last| last + 1)
            .unwrap_or(0);
        debug!(next_byte, "upload status: incomplete");
        return Ok(next_byte);
    }

    if status == 200 || status == 201 {
        // Already complete.
        info!("upload already complete");
        return Ok(total_size);
    }

    // 404 means the session has expired.
    if status == 404 {
        warn!(session_uri, "resumable upload session expired");
        return Err(DriveError::NotFound("Upload session expired".into()));
    }

    Err(DriveClient::check_response(resp).await.unwrap_err())
}

// ---------------------------------------------------------------------------
// Export native Google Docs Editors files (Docs, Sheets, Slides, Drawings)
// ---------------------------------------------------------------------------

/// Google's native Docs Editors formats (`application/vnd.google-apps.*`,
/// excluding folders) have no raw binary content — downloading them via
/// `alt=media` always 403s with "Use Export with Docs Editors files". They
/// have to be exported to a concrete format instead.
///
/// Returns `(export mime type, local file extension)` for the types we know
/// a sensible export for, or `None` for types with no useful local file
/// equivalent (Forms, Sites, Apps Script, Shortcuts, Fusion Tables, ...) —
/// those are skipped rather than synced.
pub fn google_export_target(mime_type: &str) -> Option<(&'static str, &'static str)> {
    match mime_type {
        "application/vnd.google-apps.document" => Some((
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "docx",
        )),
        "application/vnd.google-apps.spreadsheet" => Some((
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "xlsx",
        )),
        "application/vnd.google-apps.presentation" => Some((
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "pptx",
        )),
        "application/vnd.google-apps.drawing" => Some(("image/png", "png")),
        _ => None,
    }
}

/// Returns `true` for any Google-native type (Docs, Sheets, Slides, Forms,
/// Sites, Apps Script, Shortcuts, ...) other than folders — i.e. anything
/// with no raw binary content, exportable or not.
pub fn is_google_native(mime_type: &str) -> bool {
    mime_type.starts_with("application/vnd.google-apps.")
        && mime_type != "application/vnd.google-apps.folder"
}

/// Export a native Google Docs Editors file to `export_mime_type` and write
/// the result to `dest`. Exports are returned in a single response (no range
/// support), which is fine since these documents are typically small.
pub async fn export_file(
    client: &DriveClient,
    file_id: &str,
    export_mime_type: &str,
    dest: &Path,
) -> Result<(), DriveError> {
    let token = client.get_token().await?;

    let url = format!("{}/files/{}/export", DRIVE_API, file_id);
    debug!(file_id, export_mime_type, "exporting Google Docs Editors file");

    let resp = client
        .http
        .get(&url)
        .bearer_auth(&token)
        .query(&[("mimeType", export_mime_type)])
        .send()
        .await?;

    let resp = DriveClient::check_response(resp).await?;
    let bytes = resp.bytes().await?;

    let tmp_path = {
        let mut p = dest.as_os_str().to_owned();
        p.push(".tuxdrive-tmp");
        std::path::PathBuf::from(p)
    };
    std::fs::write(&tmp_path, &bytes)?;
    std::fs::rename(&tmp_path, dest)?;

    info!(file_id, dest = %dest.display(), "file exported");
    Ok(())
}

// ---------------------------------------------------------------------------
// Download a file using HTTP range requests
// ---------------------------------------------------------------------------

pub async fn download_file(
    client: &DriveClient,
    file_id: &str,
    dest: &Path,
    chunk_size: u64,
) -> Result<(), DriveError> {
    use std::io::Write;

    let url = format!("{}/files/{}?alt=media", DRIVE_API, file_id);

    // Write to a temp file alongside the destination.
    let tmp_path = {
        let mut p = dest.as_os_str().to_owned();
        p.push(".tuxdrive-tmp");
        std::path::PathBuf::from(p)
    };

    let mut tmp_file = std::fs::File::create(&tmp_path)?;

    let mut offset: u64 = 0;
    loop {
        // Refresh the token each iteration so long downloads don't use stale tokens.
        let token = client.get_token().await?;
        let end = offset + chunk_size - 1;
        let range_header = format!("bytes={}-{}", offset, end);

        debug!(file_id, offset, chunk_size, "downloading chunk");

        let resp = client
            .http
            .get(&url)
            .bearer_auth(&token)
            .header("Range", &range_header)
            .send()
            .await?;

        let status = resp.status().as_u16();

        // 416 Range Not Satisfiable means we've read past the end — done.
        if status == 416 {
            debug!(file_id, "download complete (416 end-of-file)");
            break;
        }

        // 206 Partial Content or 200 OK (server may ignore range on small files).
        if status != 206 && status != 200 {
            DriveClient::check_response(resp).await?;
            break;
        }

        let chunk = resp.bytes().await?;
        if chunk.is_empty() {
            debug!(file_id, "download complete (empty chunk)");
            break;
        }

        let bytes_read = chunk.len() as u64;
        tmp_file.write_all(&chunk)?;
        offset += bytes_read;

        // If the server returned fewer bytes than requested, we're at EOF.
        if bytes_read < chunk_size {
            debug!(file_id, offset, "download complete (short chunk)");
            break;
        }
    }

    // Flush and close before renaming.
    tmp_file.flush()?;
    drop(tmp_file);

    std::fs::rename(&tmp_path, dest)?;
    info!(file_id, dest = %dest.display(), "file downloaded");
    Ok(())
}

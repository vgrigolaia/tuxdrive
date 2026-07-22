use tracing::debug;

use crate::{
    client::{DriveClient, DRIVE_API},
    error::DriveError,
    models::ChangeList,
};

const CHANGE_FIELDS: &str =
    "changes(changeType,time,fileId,file(id,name,mimeType,size,md5Checksum,parents,modifiedTime,createdTime,trashed),removed),nextPageToken,newStartPageToken";

// ---------------------------------------------------------------------------
// Get start page token
// ---------------------------------------------------------------------------

pub async fn get_start_page_token(client: &DriveClient) -> Result<String, DriveError> {
    let token = client.get_token().await?;

    debug!("fetching changes start page token");

    let resp = client
        .http
        .get(format!("{}/changes/startPageToken", DRIVE_API))
        .bearer_auth(&token)
        .send()
        .await?;

    let resp = DriveClient::check_response(resp).await?;
    let body: crate::models::StartPageTokenResponse = resp.json().await?;
    Ok(body.start_page_token)
}

// ---------------------------------------------------------------------------
// List changes since a page token
// ---------------------------------------------------------------------------

pub async fn list_changes(
    client: &DriveClient,
    page_token: &str,
) -> Result<ChangeList, DriveError> {
    let token = client.get_token().await?;

    debug!(page_token, "listing Drive changes");

    let resp = client
        .http
        .get(format!("{}/changes", DRIVE_API))
        .bearer_auth(&token)
        .query(&[
            ("pageToken", page_token),
            ("fields", CHANGE_FIELDS),
            ("spaces", "drive"),
            ("includeItemsFromAllDrives", "false"),
            ("pageSize", "100"),
        ])
        .send()
        .await?;

    let resp = DriveClient::check_response(resp).await?;
    let change_list: ChangeList = resp.json().await?;
    debug!(
        count = change_list.changes.len(),
        next_page_token = ?change_list.next_page_token,
        new_start_page_token = ?change_list.new_start_page_token,
        "received change list"
    );
    Ok(change_list)
}

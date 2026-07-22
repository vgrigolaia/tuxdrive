use tracing::debug;

use crate::{
    client::DriveClient,
    error::DriveError,
    models::DriveAbout,
};

pub async fn get_about(client: &DriveClient) -> Result<DriveAbout, DriveError> {
    let token = client.get_token().await?;

    debug!("fetching Drive about info");

    let resp = client
        .http
        .get("https://www.googleapis.com/drive/v3/about")
        .bearer_auth(&token)
        .query(&[("fields", "user,storageQuota")])
        .send()
        .await?;

    let resp = DriveClient::check_response(resp).await?;
    let about: DriveAbout = resp.json().await?;
    Ok(about)
}

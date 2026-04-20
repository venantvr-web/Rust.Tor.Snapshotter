use anyhow::{Context, Result};
use reqwest::Client;
use std::path::Path;
use yup_oauth2::ServiceAccountAuthenticator;

const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
const UPLOAD_URL: &str = "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart";

pub async fn get_token(sa_path: &Path) -> Result<String> {
    let key = yup_oauth2::read_service_account_key(sa_path)
        .await
        .with_context(|| format!("lecture service account: {}", sa_path.display()))?;
    let auth = ServiceAccountAuthenticator::builder(key).build().await?;
    let tok = auth.token(&[DRIVE_SCOPE]).await?;
    tok.token().map(String::from).context("token vide")
}

pub async fn upload(
    http: &Client,
    token: &str,
    folder_id: &str,
    filename: &str,
    body: &[u8],
) -> Result<String> {
    let meta = serde_json::json!({
        "name": filename,
        "parents": [folder_id],
        "mimeType": "text/html",
    });

    let boundary = "----TorSnap-7a9f4c2e";
    let mut payload: Vec<u8> = Vec::with_capacity(body.len() + 512);
    payload.extend_from_slice(
        format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes(),
    );
    payload.extend_from_slice(meta.to_string().as_bytes());
    payload.extend_from_slice(
        format!("\r\n--{boundary}\r\nContent-Type: text/html\r\n\r\n").as_bytes(),
    );
    payload.extend_from_slice(body);
    payload.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let resp = http
        .post(UPLOAD_URL)
        .bearer_auth(token)
        .header(
            "Content-Type",
            format!("multipart/related; boundary={boundary}"),
        )
        .body(payload)
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("Drive upload {status}: {text}");
    }
    let v: serde_json::Value = serde_json::from_str(&text)?;
    Ok(v.get("id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string())
}

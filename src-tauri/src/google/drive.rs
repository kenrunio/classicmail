//! Google Drive v3 — only what sending large attachments as links needs (mail::compose::drive):
//! ClassicMail's own folder, resumable uploads into it, sharing a file with the recipients, the
//! folder's files (size, clean-up) and the Drive's storage. With the `drive.file` permission
//! Google shows ClassicMail only the files and folders it created itself.

use std::time::Duration;

use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Api, client::ApiError, seg};

pub const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
/// `appProperties` mark: our folder is `classicmail=uploads`, each file in it `classicmail=upload`
pub const MARK: &str = "classicmail";
const FILE_FIELDS: &str = "id,name,size,mimeType,createdTime,webViewLink,trashed";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    /// bytes, as a decimal string (absent for folders)
    pub size: Option<String>,
    pub mime_type: String,
    /// RFC 3339
    pub created_time: Option<String>,
    pub web_view_link: Option<String>,
    pub trashed: bool,
}

impl DriveFile {
    pub fn bytes(&self) -> i64 {
        self.size.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0)
    }
    /// The link sent to the recipients.
    pub fn link(&self) -> String {
        self.web_view_link.clone().unwrap_or_else(|| format!("https://drive.google.com/file/d/{}/view", self.id))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StorageQuota {
    /// absent for unlimited storage
    pub limit: Option<String>,
    pub usage: Option<String>,
}

/// Where a resumable upload stands.
pub enum UploadState {
    /// Google has the bytes before this offset
    Incomplete(u64),
    Done(DriveFile),
}

/// A `'` or `\` in a Drive query string literal.
fn q_lit(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

impl Api {
    /// Our folder (marked `classicmail=uploads`), if one exists: the oldest, when there are several.
    /// Found again after ClassicMail is installed afresh — the mark stays on the folder in Drive.
    pub async fn drive_find_folder(&self) -> anyhow::Result<Option<DriveFile>> {
        let q = format!("mimeType='{FOLDER_MIME}' and trashed=false and appProperties has {{ key='{MARK}' and value='uploads' }}");
        let files = self.drive_list(&q, Some("createdTime")).await?;
        Ok(files.into_iter().next())
    }

    pub async fn drive_get(&self, id: &str) -> anyhow::Result<DriveFile> {
        self.get(&format!("{}/files/{}", self.urls.drive, seg(id)), &[("fields", FILE_FIELDS.into())]).await
    }

    pub async fn drive_create_folder(&self, name: &str) -> anyhow::Result<DriveFile> {
        let body = json!({ "name": name, "mimeType": FOLDER_MIME, "appProperties": { MARK: "uploads" } });
        self.post(&format!("{}/files", self.urls.drive), &[("fields", FILE_FIELDS.into())], body).await
    }

    pub async fn drive_rename(&self, id: &str, name: &str) -> anyhow::Result<()> {
        let _: Value =
            self.patch(&format!("{}/files/{}", self.urls.drive, seg(id)), &[("fields", "id".into())], json!({ "name": name })).await?;
        Ok(())
    }

    /// To Drive's trash (kept there 30 days), not deleted outright.
    pub async fn drive_trash(&self, id: &str) -> anyhow::Result<()> {
        let _: Value =
            self.patch(&format!("{}/files/{}", self.urls.drive, seg(id)), &[("fields", "id".into())], json!({ "trashed": true })).await?;
        Ok(())
    }

    /// Every file matching the query `q` (all pages).
    pub async fn drive_list(&self, q: &str, order_by: Option<&str>) -> anyhow::Result<Vec<DriveFile>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page {
            #[serde(default)]
            files: Vec<DriveFile>,
            next_page_token: Option<String>,
        }
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![
                ("q", q.to_string()),
                ("fields", format!("nextPageToken,files({FILE_FIELDS})")),
                ("pageSize", "1000".into()),
                ("spaces", "drive".into()),
            ];
            if let Some(o) = order_by {
                query.push(("orderBy", o.into()));
            }
            if let Some(t) = &token {
                query.push(("pageToken", t.clone()));
            }
            let page: Page = self.get(&format!("{}/files", self.urls.drive), &query).await?;
            out.extend(page.files);
            match page.next_page_token {
                Some(t) if !t.is_empty() => token = Some(t),
                _ => break,
            }
        }
        Ok(out)
    }

    /// The files in a folder (not trashed), optionally only those created before `before` (RFC 3339).
    pub async fn drive_files_in(&self, folder_id: &str, before: Option<&str>) -> anyhow::Result<Vec<DriveFile>> {
        let mut q = format!("'{}' in parents and trashed=false and mimeType!='{FOLDER_MIME}'", q_lit(folder_id));
        if let Some(b) = before {
            q.push_str(&format!(" and createdTime < '{}'", q_lit(b)));
        }
        self.drive_list(&q, Some("createdTime")).await
    }

    /// Let `email` open the file. No e-mail from Google: the message itself carries the link.
    /// Google refuses (400 `invalidSharingRequest`) when the address has no Google account.
    pub async fn drive_share_with(&self, file_id: &str, email: &str) -> anyhow::Result<()> {
        let url = format!("{}/files/{}/permissions", self.urls.drive, seg(file_id));
        let q = [("sendNotificationEmail", "false".to_string()), ("fields", "id".into())];
        let _: Value = self.post(&url, &q, json!({ "type": "user", "role": "reader", "emailAddress": email })).await?;
        Ok(())
    }

    /// Let anyone with the link open the file.
    pub async fn drive_share_anyone(&self, file_id: &str) -> anyhow::Result<()> {
        let url = format!("{}/files/{}/permissions", self.urls.drive, seg(file_id));
        let _: Value = self.post(&url, &[("fields", "id".into())], json!({ "type": "anyone", "role": "reader" })).await?;
        Ok(())
    }

    pub async fn drive_storage(&self) -> anyhow::Result<StorageQuota> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct About {
            #[serde(default)]
            storage_quota: StorageQuota,
        }
        let a: About = self.get(&format!("{}/about", self.urls.drive), &[("fields", "storageQuota(limit,usage)".into())]).await?;
        Ok(a.storage_quota)
    }

    /// Start a resumable upload into `parent`; returns the session URL (valid about a week).
    pub async fn drive_start_upload(&self, name: &str, mime: &str, size: u64, parent: &str) -> anyhow::Result<String> {
        let url = format!("{}/files?uploadType=resumable&fields={}", self.urls.drive_upload, urlencoding::encode(FILE_FIELDS));
        let meta = json!({ "name": name, "mimeType": mime, "parents": [parent], "appProperties": { MARK: "upload" } });
        let headers = [
            ("Content-Type", "application/json; charset=UTF-8".to_string()),
            ("X-Upload-Content-Type", mime.to_string()),
            ("X-Upload-Content-Length", size.to_string()),
        ];
        let res = self.raw(Method::POST, &url, &headers, Some(serde_json::to_vec(&meta)?), Duration::from_secs(60)).await?;
        if !res.status().is_success() {
            return Err(Api::raw_error(res).await);
        }
        let loc = res.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).map(str::to_string);
        loc.ok_or_else(|| anyhow::anyhow!("Google Drive didn't start the upload (no session address)"))
    }

    /// Send `bytes` as the part of the file starting at `start` (of `total`).
    pub async fn drive_upload_chunk(&self, session: &str, start: u64, total: u64, bytes: Vec<u8>) -> anyhow::Result<UploadState> {
        let end = start + bytes.len() as u64;
        let range = if bytes.is_empty() { format!("bytes */{total}") } else { format!("bytes {start}-{}/{total}", end - 1) };
        // allow ~50 KB/s for the chunk on top of a minute
        let timeout = Duration::from_secs(60 + bytes.len() as u64 / 50_000);
        let res = self.raw(Method::PUT, session, &[("Content-Range", range)], Some(bytes), timeout).await?;
        upload_state(res).await
    }

    /// Ask a session how much Google already has (after a break in the connection or a restart).
    pub async fn drive_upload_status(&self, session: &str, total: u64) -> anyhow::Result<UploadState> {
        self.drive_upload_chunk(session, 0, total, Vec::new()).await
    }
}

async fn upload_state(res: reqwest::Response) -> anyhow::Result<UploadState> {
    match res.status().as_u16() {
        200 | 201 => Ok(UploadState::Done(res.json().await?)),
        308 => {
            // "Range: bytes=0-1048575": Google has bytes up to and including the last number
            let next = res
                .headers()
                .get(reqwest::header::RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|r| r.rsplit('-').next())
                .and_then(|n| n.trim().parse::<u64>().ok())
                .map(|last| last + 1)
                .unwrap_or(0);
            Ok(UploadState::Incomplete(next))
        }
        _ => Err(Api::raw_error(res).await),
    }
}

/// The Drive permission isn't there (signed in before ClassicMail asked for it, or unticked).
pub fn needs_sign_in(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ApiError>().is_some_and(|a| {
        a.status() == Some(403) && matches!(a.reason(), Some("insufficientPermissions" | "ACCESS_TOKEN_SCOPE_INSUFFICIENT"))
    }) || e.to_string().contains("insufficient authentication scopes")
}

/// The Google Drive API isn't turned on in the user's Google Cloud project.
pub fn api_disabled(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ApiError>()
        .is_some_and(|a| a.status() == Some(403) && matches!(a.reason(), Some("accessNotConfigured" | "SERVICE_DISABLED")))
}

/// Google refused to share with an address that has no Google account.
pub fn no_google_account(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ApiError>().is_some_and(|a| a.status() == Some(400) && a.reason() == Some("invalidSharingRequest"))
}

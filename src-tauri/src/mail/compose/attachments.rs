//! Getting attachment bytes: local files, forwarded attachments of existing messages, data URLs.

use std::{path::Path, sync::Arc};

use base64::Engine;
use serde::Serialize;

use super::{
    mime::{Part, guess_mime},
    model::AttachmentSpec,
};
use crate::ctx::AppCtx;

/// Gmail's limit for a whole message.
pub const MAX_MESSAGE_BYTES: usize = 25 * 1024 * 1024;

/// (filename, mime, bytes) of a local `attachments.id` (cached on disk, else from Gmail).
pub async fn attachment_data(ctx: &Arc<AppCtx>, attachment_id: i64) -> anyhow::Result<(String, String, Vec<u8>)> {
    crate::mail::attachment_bytes(ctx, attachment_id).await
}

pub fn decode_data_url(url: &str) -> anyhow::Result<(String, Vec<u8>)> {
    let rest = url.strip_prefix("data:").ok_or_else(|| anyhow::anyhow!("not a data URL"))?;
    let (meta, data) = rest.split_once(',').ok_or_else(|| anyhow::anyhow!("bad data URL"))?;
    let mime = meta.split(';').next().unwrap_or("application/octet-stream").to_string();
    let bytes = if meta.ends_with(";base64") {
        base64::engine::general_purpose::STANDARD.decode(data.trim())?
    } else {
        urlencoding::decode_binary(data.as_bytes()).into_owned()
    };
    Ok((mime, bytes))
}

pub fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

/// Resolve every attachment of a compose to bytes.
pub async fn resolve(ctx: &Arc<AppCtx>, specs: &[AttachmentSpec]) -> anyhow::Result<Vec<Part>> {
    let mut out = Vec::new();
    let mut total = 0usize;
    for s in specs {
        let part = if let Some(path) = &s.path {
            let bytes = tokio::fs::read(path)
                .await
                .map_err(|e| crate::user_err!("attach_missing", "Cannot find the attachment \"{}\". ({e})", file_name(path)))?;
            let name = s.name.clone().unwrap_or_else(|| file_name(path));
            let mime = s.mime_type.clone().unwrap_or_else(|| guess_mime(&name));
            Part { filename: name, mime, bytes, cid: s.cid.clone() }
        } else if let Some(id) = s.attachment_id {
            let (name, mime, bytes) = attachment_data(ctx, id).await?;
            Part { filename: s.name.clone().unwrap_or(name), mime: s.mime_type.clone().unwrap_or(mime), bytes, cid: s.cid.clone() }
        } else if let Some(url) = &s.data_url {
            let (mime, bytes) = decode_data_url(url)?;
            let name = s.name.clone().unwrap_or_else(|| "image.png".into());
            Part { filename: name, mime, bytes, cid: s.cid.clone() }
        } else {
            continue;
        };
        total += part.bytes.len();
        out.push(part);
    }
    if total > MAX_MESSAGE_BYTES {
        return Err(crate::user_err!(
            "too_large",
            "The attachment size exceeds the allowable limit. Gmail accepts messages up to 25 MB; these attachments total {:.1} MB.",
            total as f64 / 1_048_576.0
        ));
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub path: String,
    pub name: String,
    pub size: i64,
    pub mime_type: String,
    pub exists: bool,
}

pub async fn file_info(paths: Vec<String>) -> Vec<FileInfo> {
    let mut out = Vec::new();
    for p in paths {
        let meta = tokio::fs::metadata(&p).await.ok();
        let name = file_name(&p);
        out.push(FileInfo {
            mime_type: guess_mime(&name),
            name,
            size: meta.as_ref().map(|m| m.len() as i64).unwrap_or(0),
            exists: meta.map(|m| m.is_file()).unwrap_or(false),
            path: p,
        });
    }
    out
}

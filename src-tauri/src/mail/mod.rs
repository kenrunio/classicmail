//! Mail domain. Owned by the mail-core builder, except `compose/**` (mail-compose builder).
//!
//! Hook points both builders rely on (keep these signatures):
//! * `compose::register(r)` is called from `register`.
//! * `sync::sync_account` calls `compose::process_outbox(ctx, api)` before pulling.
//! * `ensure_body` / `attachment_bytes` are implemented by mail-core and used by compose
//!   (reply/forward quoting, forwarding attachments).
//!
//! Layout: `store` (folders/messages in SQLite), `schema` (mail tables), `sync` (Gmail full/incremental
//! sync, pending ops, body prefetch), `imap_sync` (fast first download of headers over IMAP), `body`
//! (format=full → bodies/attachments/invites), `render` (sanitising), `list` (message list queries),
//! `threads` (Show as Conversations), `search` (Instant Search syntax), `actions` (local-first
//! changes), `api` (rpc), `mime` (shared MIME helpers), `attach_policy` (opening / saving
//! attachments safely), `preview` (attachment previews), `save_folders` (where attachments were
//! saved per domain), `drag_out` (dragging attachments out), `merge` (mail merge).

pub mod actions;
pub mod api;
pub mod attach_policy;
pub mod body;
pub mod compose;
pub mod drag_out;
pub mod imap_sync;
pub mod list;
pub mod merge;
pub mod mime;
pub mod preview;
pub mod render;
pub mod save_folders;
pub mod schema;
pub mod search;
#[cfg(test)]
mod search_bench;
pub mod store;
pub mod sync;
pub mod threads;

#[cfg(test)]
pub mod hostile_tests;
#[cfg(test)]
mod imap_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::{ctx::AppCtx, rpc::Registry};

pub fn register(r: &mut Registry) {
    api::register(r);
    compose::register(r);
    merge::register(r);
    save_folders::register(r);
    drag_out::register(r);
}

/// Make sure `message_bodies` (and `attachments`) are filled for a local message id.
pub async fn ensure_body(ctx: &Arc<AppCtx>, message_id: i64) -> anyhow::Result<()> {
    body::ensure_body(ctx, message_id).await
}

/// Attachment content by local `attachments.id` → (filename, mime type, bytes).
pub async fn attachment_bytes(ctx: &Arc<AppCtx>, attachment_id: i64) -> anyhow::Result<(String, String, Vec<u8>)> {
    body::attachment_bytes(ctx, attachment_id).await
}

/// Serves `cmattach://localhost/<path>` (`http://cmattach.localhost/<path>` on Windows):
/// `/inline/<attachment id>` and `/att/<attachment id>/<name>` (attachments and inline images), and
/// `/pdf/<attachment id>` for the Reading Pane's PDF preview.
pub async fn protocol_response(ctx: &Arc<AppCtx>, path: &str) -> tauri::http::Response<Vec<u8>> {
    let resp = |status: u16, ct: &str, body: Vec<u8>| {
        // the type comes from the message: one that isn't a valid header value is served as bytes
        let ct = if tauri::http::HeaderValue::from_str(ct).is_ok() { ct } else { "application/octet-stream" };
        tauri::http::Response::builder()
            .status(status)
            .header("Content-Type", ct)
            // no CORS: only <img> loads need this, and no other origin should read attachments;
            // if one is ever navigated to, it can't run anything
            .header("Cache-Control", "private, max-age=86400")
            .header("X-Content-Type-Options", "nosniff")
            .header("Content-Security-Policy", "default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; sandbox")
            .body(body)
            .expect("constant headers and a checked content type")
    };
    let mut segs = path.trim_start_matches('/').split('/');
    let kind = segs.next().unwrap_or("");
    let id: Option<i64> = segs.next().and_then(|s| s.parse().ok());
    match (kind, id) {
        ("inline" | "att", Some(id)) => match attachment_bytes(ctx, id).await {
            Ok((_, mime, bytes)) => {
                // never let an attachment run as a page inside the app
                let ct = if mime.starts_with("image/") && mime != "image/svg+xml" { mime } else { "application/octet-stream".into() };
                resp(200, &ct, bytes)
            }
            Err(e) => {
                log::debug!("cmattach {kind} {id}: {}", crate::logging::err(&e));
                resp(404, "text/plain", b"not found".to_vec())
            }
        },
        // a PDF for the WebView's own viewer (no sandbox: the viewer is a plugin, and it doesn't
        // run a PDF's scripts); only real PDFs, never served as anything else
        ("pdf", Some(id)) => match attachment_bytes(ctx, id).await {
            Ok((_, _, bytes)) if bytes.starts_with(b"%PDF-") => tauri::http::Response::builder()
                .status(200)
                .header("Content-Type", "application/pdf")
                .header("Cache-Control", "private, max-age=86400")
                .header("X-Content-Type-Options", "nosniff")
                .body(bytes)
                .unwrap(),
            Ok(_) => resp(415, "text/plain", b"not a PDF".to_vec()),
            Err(e) => {
                log::debug!("cmattach {kind} {id}: {}", crate::logging::err(&e));
                resp(404, "text/plain", b"not found".to_vec())
            }
        },
        _ => resp(404, "text/plain", b"not found".to_vec()),
    }
}

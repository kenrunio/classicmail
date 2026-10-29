//! Composing and sending: MIME building, Outbox, Gmail drafts, signatures, send-as identities,
//! recipient auto-complete cache. Owned by the mail-compose builder.
//!
//! rpc methods:
//! * `mail.compose.prepare { mode, id?, accountId? }` → initial window contents
//! * `mail.compose.send { compose }` → queue in the Outbox (sent by `process_outbox`)
//! * `mail.compose.saveDraft { compose }` / `discardDraft { accountId, draftId }` / `openDraft { id }`
//! * `mail.compose.fileInfo { paths }`, `readImage { path }`, `openAttachment { path? | attachmentId? }`,
//!   `saveAs { path, content }` (Office button › Save As, to a path the user picked)
//! * `mail.outbox.list { accountId? }`, `cancel { id }` (back to Drafts), `retry { id }`
//! * `mail.signatures.list / save / delete / setDefaults`, `mail.sendAs.list { accountId }`
//! * `mail.recipients.suggest { query, limit? }`, `mail.recipients.forget { email }`
//! * `mail.desktopAlert.test { count? }` — emits a `mail.newMail` for the desktop alert (demo/testing)
//!
//! Events: `mail.changed`, `mail.outbox { accountId }`, `mail.signatures {}`, `mail.sendAs { accountId }`.

pub mod attachments;
pub mod cleanup;
pub mod drafts;
pub mod dropped;
pub mod mime;
pub mod model;
pub mod outbox;
pub mod prepare;
pub mod recent;
pub mod recipients;
pub mod signatures;
pub mod store;
pub mod webpage;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ctx::AppCtx, google::Api, rpc::Registry};

pub fn register(r: &mut Registry) {
    r.add("mail.compose.prepare", prepare::prepare);
    r.add("mail.compose.send", outbox::send);
    r.add("mail.compose.saveDraft", drafts::save_rpc);
    r.add("mail.compose.discardDraft", drafts::discard);
    r.add("mail.compose.openDraft", drafts::open_rpc);
    r.add("mail.compose.fileInfo", file_info);
    r.add("mail.compose.readImage", read_image);
    r.add("mail.compose.openAttachment", open_attachment);
    r.add("mail.compose.saveAs", save_as);
    r.add("mail.outbox.list", outbox::list);
    r.add("mail.outbox.cancel", outbox::cancel);
    r.add("mail.outbox.retry", outbox::retry);
    r.add("mail.signatures.list", signatures::list);
    r.add("mail.signatures.save", signatures::save);
    r.add("mail.signatures.delete", signatures::delete);
    r.add("mail.signatures.setDefaults", signatures::set_defaults);
    r.add("mail.sendAs.list", signatures::send_as_list);
    r.add("mail.recipients.suggest", recipients::suggest);
    r.add("mail.recipients.usualGroup", recipients::usual_group);
    r.add("mail.compose.recentFiles", recent::list);
    r.add("mail.compose.folderFiles", recent::in_folder);
    r.add("mail.compose.webPage", webpage::web_page);
    r.add("mail.compose.readText", webpage::read_text);
    r.add("mail.compose.rememberAttachments", recent::remember);
    r.add("mail.compose.stageDropped", dropped::stage);
    r.add("mail.recipients.forget", recipients::forget);
    r.add("mail.desktopAlert.test", desktop_alert_test);
}

/// Send everything due in the Outbox for this account (called at the start of each mail sync).
/// An error means at least one message failed (they stay in the Outbox); the pull should still run.
pub async fn process_outbox(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    outbox::process(ctx, api).await
}

#[derive(Debug, Deserialize)]
struct PathsParams {
    paths: Vec<String>,
}

async fn file_info(_ctx: Arc<AppCtx>, p: PathsParams) -> anyhow::Result<Vec<attachments::FileInfo>> {
    Ok(attachments::file_info(p.paths).await)
}

#[derive(Debug, Deserialize)]
struct PathParams {
    path: String,
}

/// Insert › Picture: a local image as a data: URL for the editor.
async fn read_image(_ctx: Arc<AppCtx>, p: PathParams) -> anyhow::Result<Value> {
    let bytes = tokio::fs::read(&p.path)
        .await
        .map_err(|e| crate::user_err!("not_found", "Cannot open \"{}\": {e}", attachments::file_name(&p.path)))?;
    if bytes.len() > 10 * 1024 * 1024 {
        return Err(crate::user_err!("too_large", "The picture is too large to insert (over 10 MB)."));
    }
    let name = attachments::file_name(&p.path);
    let mime = mime::guess_mime(&name);
    if !mime.starts_with("image/") {
        return Err(crate::user_err!("bad_type", "\"{name}\" is not a picture."));
    }
    Ok(json!({
        "name": name,
        "dataUrl": format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenAttachmentParams {
    path: Option<String>,
    attachment_id: Option<i64>,
    /// the user answered "Open" to the "Opening Mail Attachment" question
    #[serde(default)]
    confirmed: bool,
}

/// Open an attachment of the compose window with its default program.
async fn open_attachment(ctx: Arc<AppCtx>, p: OpenAttachmentParams) -> anyhow::Result<()> {
    let path = match (p.path, p.attachment_id) {
        (Some(path), _) => {
            // our own file, or one a reopened draft brought back from Gmail: same rules
            crate::mail::attach_policy::check_open(&attachments::file_name(&path), p.confirmed)?;
            path
        }
        (None, Some(id)) => {
            // a forwarded attachment came from the internet: same rules as opening it from the message
            let (name, _, bytes) = attachments::attachment_data(&ctx, id).await?;
            crate::mail::attach_policy::check_open(&name, p.confirmed)?;
            let dir = ctx.data_dir.join("compose").join("open").join(uuid::Uuid::new_v4().simple().to_string());
            tokio::fs::create_dir_all(&dir).await?;
            let path = dir.join(crate::mail::attach_policy::safe_file_name(&name));
            crate::mail::attach_policy::write_marked(&path, &bytes)?;
            path.to_string_lossy().into_owned()
        }
        _ => return Err(crate::user_err!("bad_params", "nothing to open")),
    };
    if let Some(app) = &ctx.app {
        use tauri_plugin_opener::OpenerExt;
        app.opener().open_path(path, None::<&str>)?;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct SaveAsParams {
    path: String,
    content: String,
}

/// Office button › Save As: write the message as .htm / .txt where the user chose (save dialog).
async fn save_as(_ctx: Arc<AppCtx>, p: SaveAsParams) -> anyhow::Result<()> {
    let lower = p.path.to_lowercase();
    // .eml: a message's original source (View Source), saved as it came
    if ![".htm", ".html", ".txt", ".eml"].iter().any(|x| lower.ends_with(x)) {
        return Err(crate::user_err!("bad_type", "Messages can be saved as HTML (.htm), text (.txt) or e-mail (.eml) files."));
    }
    let bytes = if lower.ends_with(".txt") {
        // UTF-8 with BOM so Notepad shows Hebrew correctly
        let mut v = vec![0xEF, 0xBB, 0xBF];
        v.extend_from_slice(p.content.replace("\r\n", "\n").replace('\n', "\r\n").as_bytes());
        v
    } else {
        p.content.into_bytes()
    };
    tokio::fs::write(&p.path, bytes)
        .await
        .map_err(|e| crate::user_err!("write_failed", "Cannot save \"{}\": {e}", attachments::file_name(&p.path)))?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct AlertTestParams {
    count: Option<usize>,
}

/// Emit a `mail.newMail` for the newest Inbox messages (or a sample) so the alert can be seen.
async fn desktop_alert_test(ctx: Arc<AppCtx>, p: AlertTestParams) -> anyhow::Result<Value> {
    let n = p.count.unwrap_or(1).clamp(1, 20) as i64;
    let rows: Vec<(i64, i64, String, String, String)> = ctx
        .db
        .read(move |c| {
            let mut st = c.prepare(
                "SELECT m.id, m.account_id, CASE WHEN m.from_name<>'' THEN m.from_name ELSE m.from_email END, m.subject, m.snippet \
                 FROM messages m JOIN message_folders mf ON mf.message_id = m.id JOIN folders f ON f.id = mf.folder_id \
                 WHERE f.role='inbox' ORDER BY m.internal_date DESC LIMIT ?1",
            )?;
            let v = st.query_map([n], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await?;
    let payload = if rows.is_empty() {
        let accounts = crate::accounts::list(&ctx.db).await?;
        let account_id = accounts.first().map(|a| a.id).unwrap_or(0);
        let sample = [
            (
                "Sarah Cohen",
                "Q3 earnings call — investor questions",
                "Hi Kenny, ahead of Thursday's call, could you share the list of questions from the buy side?",
            ),
            ("Noa Shapiro", "דוח רבעוני – טיוטה לעיון", "שלום קני, מצורפת טיוטת הדוח הרבעוני לעיונך. נשמח להערות עד יום ראשון."),
            (
                "Chris Donovan",
                "Atlas Tech Conference: meeting schedule",
                "Attached is the updated 1x1 schedule for the Tel Aviv conference.",
            ),
        ];
        let messages: Vec<Value> =
            sample.iter().cycle().take(n as usize).map(|(f, s, pv)| json!({ "id": 0, "from": f, "subject": s, "preview": pv })).collect();
        json!({ "accountId": account_id, "messages": messages })
    } else {
        let account_id = rows[0].1;
        let messages: Vec<Value> =
            rows.iter().map(|(id, _, f, s, pv)| json!({ "id": id, "from": f, "subject": s, "preview": pv })).collect();
        json!({ "accountId": account_id, "messages": messages })
    };
    ctx.events.emit("mail.newMail", payload.clone());
    Ok(payload)
}

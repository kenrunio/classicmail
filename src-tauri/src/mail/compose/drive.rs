//! Attachments too large for Gmail (over 25 MB), sent as Google Drive links instead.
//!
//! * Attaching such a file starts an upload at once (`mail.drive.upload`) into the account's
//!   ClassicMail folder (drive_folder.rs). It runs in the background, survives closing the compose
//!   window and carries on after a break or a restart (Google's resumable upload: the session is
//!   kept in `drive_uploads`). Progress goes out as `drive.upload` events.
//! * The message gets a card per file (name, size, link) above the quoted original; the plain-text
//!   version a line with the link. A saved draft keeps the cards; opening it again turns them back
//!   into Drive attachments (`take_cards`).
//! * Sending waits for the uploads (the Outbox holds the message, `readiness`), then shares each
//!   file with every recipient, keeping it "only people with access" (`share`). Google refuses an
//!   address with no Google account: then, by the `drive.noGoogleAccount` setting, anyone with the
//!   link may open it ("anyone", the default) or it stays private ("private").
//!
//! rpc: `mail.drive.upload { accountId, path, temp? }` → Upload; `mail.drive.uploads { ids }`;
//! `mail.drive.retry { id }`; `mail.drive.cancel { id }`.

use std::{
    collections::HashSet,
    sync::{Arc, LazyLock},
    time::Duration,
};

use parking_lot::Mutex;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{
    mime::{guess_mime, html_to_text},
    model::{AttachmentSpec, Compose, DriveLink},
    prepare::ORIGINAL_MARKER,
};
use crate::{
    ctx::AppCtx,
    db::util::{get_setting, now_ms},
    google::{
        Api,
        client::{ApiError, api_status},
        drive::{self, DriveFile, UploadState},
    },
};

/// Gmail's limit per message: larger files (or what doesn't fit beside the rest) go as links.
pub const LINK_ABOVE: u64 = 25 * 1024 * 1024;
/// A multiple of 256 KiB, as Google asks.
const CHUNK: u64 = 8 * 1024 * 1024;
pub const NO_ACCOUNT_SETTING: &str = "drive.noGoogleAccount";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Upload {
    pub id: i64,
    pub account_id: i64,
    pub path: String,
    pub name: String,
    pub size: i64,
    pub sent: i64,
    /// "uploading" | "done" | "failed"
    pub status: String,
    pub error: Option<String>,
    pub file_id: Option<String>,
    pub link: Option<String>,
    #[serde(skip)]
    mime: String,
    #[serde(skip)]
    temp: bool,
    #[serde(skip)]
    pub(super) session_url: Option<String>,
}

const COLS: &str = "id, account_id, path, name, size, sent, status, error, file_id, link, mime, temp, session_url";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Upload> {
    Ok(Upload {
        id: r.get(0)?,
        account_id: r.get(1)?,
        path: r.get(2)?,
        name: r.get(3)?,
        size: r.get(4)?,
        sent: r.get(5)?,
        status: r.get(6)?,
        error: r.get(7)?,
        file_id: r.get(8)?,
        link: r.get(9)?,
        mime: r.get(10)?,
        temp: r.get::<_, i64>(11)? != 0,
        session_url: r.get(12)?,
    })
}

pub async fn load(ctx: &AppCtx, id: i64) -> anyhow::Result<Option<Upload>> {
    ctx.db.read(move |c| Ok(c.query_row(&format!("SELECT {COLS} FROM drive_uploads WHERE id=?1"), [id], from_row).optional()?)).await
}

fn emit(ctx: &AppCtx, u: &Upload) {
    ctx.events.emit("drive.upload", serde_json::to_value(u).unwrap_or_default());
}

// ---- rpc ------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadParams {
    account_id: i64,
    path: String,
    /// a copy ClassicMail made (a dropped file): deleted once uploaded
    #[serde(default)]
    temp: bool,
}

pub async fn upload_rpc(ctx: Arc<AppCtx>, p: UploadParams) -> anyhow::Result<Upload> {
    let name = super::attachments::file_name(&p.path);
    let meta = tokio::fs::metadata(&p.path).await.map_err(|e| crate::user_err!("attach_missing", "Cannot find \"{name}\". ({e})"))?;
    if !meta.is_file() || meta.len() == 0 {
        return Err(crate::user_err!("attach_missing", "\"{name}\" is empty or isn't a file."));
    }
    let (size, mime, account_id, path, temp) = (meta.len() as i64, guess_mime(&name), p.account_id, p.path.clone(), p.temp);
    let n2 = name.clone();
    let id = ctx
        .db
        .write(move |c| {
            c.execute(
                "INSERT INTO drive_uploads(account_id, path, name, mime, size, temp, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![account_id, path, n2, mime, size, temp as i64, now_ms()],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await?;
    spawn(ctx.clone(), id);
    load(&ctx, id).await?.ok_or_else(|| anyhow::anyhow!("upload {id} vanished"))
}

#[derive(Deserialize)]
pub struct IdsParams {
    ids: Vec<i64>,
}

pub async fn uploads_rpc(ctx: Arc<AppCtx>, p: IdsParams) -> anyhow::Result<Vec<Upload>> {
    let mut out = Vec::new();
    for id in p.ids {
        if let Some(u) = load(&ctx, id).await? {
            out.push(u);
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct IdParams {
    id: i64,
}

/// Try a failed upload again (it carries on where it stopped when Google still has the session).
pub async fn retry_rpc(ctx: Arc<AppCtx>, p: IdParams) -> anyhow::Result<()> {
    let n = ctx
        .db
        .write(move |c| Ok(c.execute("UPDATE drive_uploads SET status='uploading', error=NULL WHERE id=?1 AND status='failed'", [p.id])?))
        .await?;
    if n == 1 {
        spawn(ctx.clone(), p.id);
        if let Some(u) = load(&ctx, p.id).await? {
            emit(&ctx, &u);
        }
    }
    Ok(())
}

/// The attachment was removed before the upload finished: stop it (Google drops the partial file).
/// A finished file stays in Drive (the clean-up takes it in time): a saved draft may link to it.
pub async fn cancel_rpc(ctx: Arc<AppCtx>, p: IdParams) -> anyhow::Result<()> {
    let Some(u) = load(&ctx, p.id).await? else { return Ok(()) };
    if u.status == "done" {
        return Ok(());
    }
    ctx.db.write(move |c| Ok(c.execute("DELETE FROM drive_uploads WHERE id=?1", [p.id])?)).await?;
    if u.temp {
        remove_temp(&u.path);
    }
    if let Some(s) = u.session_url
        && let Ok(api) = ctx.api(u.account_id).await
    {
        // DELETE on the session cancels it; best effort
        let _ = api.raw(reqwest::Method::DELETE, &s, &[], None, Duration::from_secs(20)).await;
    }
    Ok(())
}

// ---- the upload ---------------------------------------------------------------------------

/// Uploads running in this process (one task per upload), by database (tests run several) and id.
static RUNNING: LazyLock<Mutex<HashSet<(usize, i64)>>> = LazyLock::new(Default::default);

fn spawn(ctx: Arc<AppCtx>, id: i64) {
    let key = (Arc::as_ptr(&ctx) as usize, id);
    if !RUNNING.lock().insert(key) {
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = run(&ctx, id).await {
            log::warn!("drive upload {id}: {}", crate::logging::err(&e));
            let msg = friendly(&ctx, id, &e).await;
            let m2 = msg.clone();
            let _ = ctx
                .db
                .write(move |c| {
                    Ok(c.execute("UPDATE drive_uploads SET status='failed', error=?2 WHERE id=?1 AND status='uploading'", params![id, m2])?)
                })
                .await;
            if let Ok(Some(u)) = load(&ctx, id).await {
                emit(&ctx, &u);
            }
        }
        RUNNING.lock().remove(&key);
    });
}

/// Carry on with every upload a restart interrupted (at start-up).
pub fn resume_all(ctx: Arc<AppCtx>) {
    tokio::spawn(async move {
        let ids: Vec<i64> = ctx
            .db
            .read(|c| {
                let mut st = c.prepare("SELECT id FROM drive_uploads WHERE status='uploading'")?;
                let v = st.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
                Ok(v)
            })
            .await
            .unwrap_or_default();
        for id in ids {
            spawn(ctx.clone(), id);
        }
    });
}

/// The reason an upload failed, in words for the compose window.
async fn friendly(ctx: &AppCtx, id: i64, e: &anyhow::Error) -> String {
    let email = match load(ctx, id).await {
        Ok(Some(u)) => crate::accounts::get(&ctx.db, u.account_id).await.map(|a| a.email).unwrap_or_default(),
        _ => String::new(),
    };
    explain(e, &email)
}

/// A Drive error in words (uploads, sharing, the Options tab).
pub fn explain(e: &anyhow::Error, email: &str) -> String {
    if drive::needs_sign_in(e) {
        return format!(
            "ClassicMail isn't allowed to use Google Drive for {email}. Sign in to it again (Tools › Account Settings) and allow Google Drive on Google's permission screen."
        );
    }
    if drive::api_disabled(e) {
        return "The Google Drive API is turned off in your Google Cloud project. Turn it on (APIs & Services › Library › Google Drive API), then try again.".into();
    }
    if crate::google::client::api_reason(e) == Some("storageQuotaExceeded") {
        return format!("The Google Drive of {email} is full.");
    }
    if let Some(u) = e.downcast_ref::<crate::error::UserError>() {
        return u.message.clone();
    }
    if matches!(e.downcast_ref::<ApiError>(), Some(ApiError::Network(_))) {
        return "No connection to Google Drive.".into();
    }
    format!("{e:#}")
}

/// Upload until done. Breaks in the connection are waited out (a few times); a refusal from Google
/// (no permission, Drive full) or a missing file ends it as failed.
async fn run(ctx: &Arc<AppCtx>, id: i64) -> anyhow::Result<()> {
    let Some(mut u) = load(ctx, id).await? else { return Ok(()) };
    if u.status != "uploading" {
        return Ok(());
    }
    let api = ctx.api(u.account_id).await?;
    let total = u.size as u64;
    let mut offset = 0u64;
    // where a kept session stands is asked first
    let mut known = false;
    let mut failures = 0u32;
    loop {
        // removed from the message meanwhile
        if load(ctx, id).await?.is_none_or(|x| x.status != "uploading") {
            return Ok(());
        }
        let step: anyhow::Result<Option<DriveFile>> = async {
            let session = match &u.session_url {
                Some(s) => s.clone(),
                None => {
                    let folder = super::drive_folder::ensure(ctx, &api).await?;
                    let s = api.drive_start_upload(&u.name, &u.mime, total, &folder).await?;
                    let s2 = s.clone();
                    ctx.db
                        .write(move |c| Ok(c.execute("UPDATE drive_uploads SET session_url=?2, sent=0 WHERE id=?1", params![id, s2])?))
                        .await?;
                    u.session_url = Some(s.clone());
                    offset = 0;
                    known = true;
                    s
                }
            };
            if !known {
                match api.drive_upload_status(&session, total).await? {
                    UploadState::Done(f) => return Ok(Some(f)),
                    UploadState::Incomplete(n) => offset = n,
                }
                known = true;
            }
            let bytes = read_chunk(&u.path, offset, CHUNK.min(total.saturating_sub(offset)))
                .await
                .map_err(|e| crate::user_err!("attach_missing", "Can't read \"{}\" (was it moved or deleted?): {e}", u.name))?;
            match api.drive_upload_chunk(&session, offset, total, bytes).await? {
                UploadState::Done(f) => Ok(Some(f)),
                UploadState::Incomplete(n) => {
                    offset = n;
                    let sent = n as i64;
                    ctx.db.write(move |c| Ok(c.execute("UPDATE drive_uploads SET sent=?2 WHERE id=?1", params![id, sent])?)).await?;
                    u.sent = sent;
                    emit(ctx, &u);
                    Ok(None)
                }
            }
        }
        .await;
        match step {
            Ok(Some(f)) => return finish(ctx, &u, f).await,
            Ok(None) => failures = 0,
            Err(e) => {
                // the session ran out (about a week) or was lost: start again
                if matches!(api_status(&e), Some(404 | 410)) && u.session_url.is_some() {
                    u.session_url = None;
                    failures += 1;
                } else if e.downcast_ref::<crate::error::UserError>().is_some()
                    || drive::needs_sign_in(&e)
                    || drive::api_disabled(&e)
                    || crate::google::client::is_permanent(&e)
                {
                    return Err(e);
                } else {
                    failures += 1;
                    known = false;
                }
                if failures > 8 {
                    return Err(e);
                }
                log::info!("drive upload {id}: {} — trying again", crate::logging::err(&e));
                tokio::time::sleep(Duration::from_secs((2u64 << failures.min(5)).min(60))).await;
            }
        }
    }
}

async fn read_chunk(path: &str, offset: u64, len: u64) -> std::io::Result<Vec<u8>> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut f = tokio::fs::File::open(path).await?;
    f.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn finish(ctx: &Arc<AppCtx>, u: &Upload, f: DriveFile) -> anyhow::Result<()> {
    let (id, link, file_id, size) = (u.id, f.link(), f.id.clone(), u.size);
    ctx.db
        .write(move |c| {
            Ok(c.execute(
                "UPDATE drive_uploads SET status='done', error=NULL, file_id=?2, link=?3, sent=?4, session_url=NULL WHERE id=?1",
                params![id, file_id, link, size],
            )?)
        })
        .await?;
    if u.temp {
        remove_temp(&u.path);
    }
    if let Some(done) = load(ctx, id).await? {
        emit(ctx, &done);
    }
    log::info!("drive upload {id}: done ({} bytes)", u.size);
    // a message waiting in the Outbox for this file can go now
    let waiting: i64 = ctx
        .db
        .read(|c| Ok(c.query_row("SELECT COUNT(*) FROM outbox WHERE status IN ('queued','failed')", [], |r| r.get(0))?))
        .await
        .unwrap_or(0);
    if waiting > 0 {
        ctx.sync_now(None);
    }
    Ok(())
}

/// A dropped file's copy (`<data>/dropin/<folder>/<file>`): the file and its folder.
fn remove_temp(path: &str) {
    let p = std::path::Path::new(path);
    let _ = std::fs::remove_file(p);
    if let Some(dir) = p.parent()
        && dir.parent().and_then(|d| d.file_name()).is_some_and(|n| n == "dropin")
    {
        let _ = std::fs::remove_dir(dir);
    }
}

// ---- the message ----------------------------------------------------------------------------

/// A Drive attachment as the message shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Linked {
    pub name: String,
    pub size: i64,
    /// None while still uploading
    pub link: Option<String>,
    pub upload_id: Option<i64>,
    pub file_id: Option<String>,
    /// the upload's account (sharing is done as it); None: the message's own
    pub account_id: Option<i64>,
    pub status: String,
    pub error: Option<String>,
}

/// The Drive attachments of a compose, with their uploads' state.
pub async fn linked(ctx: &AppCtx, specs: &[AttachmentSpec]) -> anyhow::Result<Vec<Linked>> {
    let mut out = Vec::new();
    for s in specs {
        let Some(d) = &s.drive else { continue };
        let name = s.name.clone().or_else(|| s.path.as_deref().map(super::attachments::file_name)).unwrap_or_else(|| "file".into());
        let size = s.size.unwrap_or(0);
        let up = match d.upload_id {
            Some(id) => load(ctx, id).await?,
            None => None,
        };
        out.push(match up {
            Some(u) => Linked {
                name: s.name.clone().unwrap_or(u.name),
                size: u.size,
                link: u.link.or_else(|| d.link.clone()),
                upload_id: Some(u.id),
                file_id: u.file_id.or_else(|| d.file_id.clone()),
                account_id: Some(u.account_id),
                status: u.status,
                error: u.error,
            },
            // a draft opened again (here, or written on another computer): the file is in Drive
            None if d.file_id.is_some() || d.link.is_some() => Linked {
                name,
                size,
                link: d.link.clone(),
                upload_id: None,
                file_id: d.file_id.clone(),
                account_id: None,
                status: "done".into(),
                error: None,
            },
            None => Linked {
                name,
                size,
                link: None,
                upload_id: d.upload_id,
                file_id: None,
                account_id: None,
                status: "failed".into(),
                error: Some("The upload was removed.".into()),
            },
        });
    }
    Ok(out)
}

/// Whether a message's Drive files are all uploaded.
pub enum Readiness {
    Ready,
    /// still uploading: what to show in the Outbox
    Waiting(String),
    Failed(String),
}

pub fn readiness(links: &[Linked]) -> Readiness {
    if let Some(l) = links.iter().find(|l| l.status == "failed") {
        return Readiness::Failed(format!("The upload of \"{}\" failed: {}", l.name, l.error.as_deref().unwrap_or("unknown error")));
    }
    let waiting: Vec<&Linked> = links.iter().filter(|l| l.status != "done").collect();
    if waiting.is_empty() {
        return Readiness::Ready;
    }
    let names: Vec<String> = waiting.iter().map(|l| format!("\"{}\"", l.name)).collect();
    Readiness::Waiting(format!("Waiting for the upload of {} to Google Drive.", names.join(", ")))
}

/// "700 MB", "1.2 GB" (as Explorer rounds).
pub fn size_text(bytes: i64) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB", b / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.0} MB", b / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", (b / 1024.0).max(1.0))
    }
}

const CARDS_START: &str = "<div data-cm-drives";
const CARDS_END: &str = "<!--/cm-drives-->";

fn attr(s: &str) -> String {
    crate::mail::mime::escape_html(s)
}

/// The cards: a small table per file (name as the link, size, "Google Drive"), styled inline so
/// every mail program shows them alike. The data-cm-* attributes bring them back as attachments
/// when the draft is opened again.
pub fn cards_html(links: &[Linked]) -> String {
    let mut out = format!("{CARDS_START} style=\"margin:12px 0\">");
    for l in links {
        let key = match (&l.file_id, l.upload_id) {
            (Some(f), _) => format!("f:{f}"),
            (None, Some(u)) => format!("u:{u}"),
            _ => String::new(),
        };
        let name = match &l.link {
            Some(href) => {
                format!("<a href=\"{}\" style=\"color:#1155cc;text-decoration:none;font-weight:bold\">{}</a>", attr(href), attr(&l.name))
            }
            None => format!("<b>{}</b>", attr(&l.name)),
        };
        out.push_str(&format!(
            "<table data-cm-drive=\"{}\" data-cm-name=\"{}\" data-cm-size=\"{}\" data-cm-link=\"{}\" cellpadding=\"0\" cellspacing=\"0\" \
             style=\"border:1px solid #dadce0;border-radius:6px;margin:4px 0;font-family:'Segoe UI',Arial,sans-serif;font-size:10pt\"><tr>\
             <td style=\"padding:8px 10px;background:#f1f3f4;border-radius:6px 0 0 6px;color:#1a73e8;font-size:14pt\">&#9650;</td>\
             <td style=\"padding:6px 14px 6px 10px\">{name}<br><span style=\"color:#5f6368;font-size:9pt\">{} &middot; Google Drive</span></td>\
             </tr></table>",
            attr(&key),
            attr(&l.name),
            l.size,
            attr(l.link.as_deref().unwrap_or("")),
            size_text(l.size),
        ));
    }
    out.push_str("</div>");
    out.push_str(CARDS_END);
    out
}

/// The plain-text version: a line per file with its link.
pub fn cards_text(links: &[Linked]) -> String {
    links
        .iter()
        .map(|l| match &l.link {
            Some(href) => format!("{} ({}, Google Drive): {href}", l.name, size_text(l.size)),
            None => format!("{} ({}, Google Drive)", l.name, size_text(l.size)),
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// The compose as sent: the Drive files as cards above the quoted original (at the end when
/// there is none), and as lines with their links in the plain-text version.
pub fn with_cards(c: &Compose, links: &[Linked]) -> Compose {
    let mut c = c.clone();
    if links.is_empty() {
        return c;
    }
    let lines = cards_text(links);
    if c.plain_text {
        let text = c.text.clone().unwrap_or_default();
        c.text = Some(format!("{}\r\n\r\n{lines}\r\n", text.trim_end()));
        return c;
    }
    let html = c.html.clone();
    let lower = html.to_ascii_lowercase();
    let at = html.find(ORIGINAL_MARKER).or_else(|| lower.rfind("</body>")).unwrap_or(html.len());
    let (before, after) = html.split_at(at);
    c.html = format!("{before}{}{after}", cards_html(links));
    if c.text.is_none() {
        // html2text drops the links: the text version gets them written out
        let t_before = html_to_text(before);
        let t_after = if after.trim().is_empty() { String::new() } else { html_to_text(after) };
        c.text = Some(format!("{}\r\n\r\n{lines}\r\n\r\n{t_after}", t_before.trim_end()).trim_end().to_string() + "\r\n");
    }
    c
}

/// A draft opened again: its cards come out of the text and back as Drive attachments.
pub fn take_cards(html: &str) -> (String, Vec<AttachmentSpec>) {
    let Some(start) = html.find(CARDS_START) else { return (html.to_string(), vec![]) };
    let Some(end) = html[start..].find(CARDS_END).map(|e| start + e + CARDS_END.len()) else { return (html.to_string(), vec![]) };
    let block = &html[start..end];
    static CARD: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"data-cm-drive="([^"]*)" data-cm-name="([^"]*)" data-cm-size="(\d*)" data-cm-link="([^"]*)""#).unwrap()
    });
    let specs = CARD
        .captures_iter(block)
        .map(|m| {
            let key = unescape(&m[1]);
            let link = Some(unescape(&m[4])).filter(|l| !l.is_empty());
            AttachmentSpec {
                name: Some(unescape(&m[2])),
                size: m[3].parse().ok(),
                mime_type: Some(guess_mime(&unescape(&m[2]))),
                drive: Some(DriveLink {
                    upload_id: key.strip_prefix("u:").and_then(|v| v.parse().ok()),
                    file_id: key.strip_prefix("f:").map(str::to_string),
                    link,
                }),
                ..Default::default()
            }
        })
        .collect();
    (format!("{}{}", &html[..start], &html[end..]), specs)
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// Let every recipient open the message's Drive files (before it goes). The files stay "only
/// people with access"; an address Google knows no account for gets "anyone with the link" or
/// nothing, by the `drive.noGoogleAccount` setting.
pub async fn share(ctx: &Arc<AppCtx>, c: &Compose, links: &[Linked]) -> anyhow::Result<()> {
    if links.is_empty() {
        return Ok(());
    }
    let anyone = ctx.db.read(|d| get_setting::<String>(d, NO_ACCOUNT_SETTING)).await?.as_deref() != Some("private");
    let mut seen = HashSet::new();
    let recipients: Vec<String> =
        c.all_recipients().map(|a| a.email.trim().to_lowercase()).filter(|e| !e.is_empty() && seen.insert(e.clone())).collect();
    for l in links {
        let Some(file) = &l.file_id else { continue };
        let api: Api = ctx.api(l.account_id.unwrap_or(c.account_id)).await?;
        let mut opened = false;
        for r in &recipients {
            if r.eq_ignore_ascii_case(&api.email) {
                continue;
            }
            match api.drive_share_with(file, r).await {
                Ok(()) => {}
                Err(e) if drive::no_google_account(&e) => {
                    if anyone && !opened {
                        api.drive_share_anyone(file).await.map_err(|e| share_err(&e, &l.name, &api.email))?;
                        opened = true;
                    }
                    log::info!(
                        "drive: a recipient has no Google account ({})",
                        if anyone { "anyone with the link" } else { "left private" }
                    );
                }
                Err(e) => return Err(share_err(&e, &l.name, &api.email)),
            }
        }
    }
    Ok(())
}

fn share_err(e: &anyhow::Error, name: &str, email: &str) -> anyhow::Error {
    // offline stays offline (the Outbox keeps the message queued instead of failed)
    if matches!(e.downcast_ref::<ApiError>(), Some(ApiError::Network(_))) {
        return ApiError::Network(format!("sharing \"{name}\"")).into();
    }
    crate::user_err!("drive_share", "\"{name}\" couldn't be shared with the recipients: {}", explain(e, email))
}

pub fn register(r: &mut crate::rpc::Registry) {
    r.add("mail.drive.upload", upload_rpc);
    r.add("mail.drive.uploads", uploads_rpc);
    r.add("mail.drive.retry", retry_rpc);
    r.add("mail.drive.cancel", cancel_rpc);
    super::drive_folder::register(r);
}

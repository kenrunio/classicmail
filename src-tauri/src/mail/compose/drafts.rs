//! Gmail drafts: save (create / update), discard, and reopen a draft in the compose window.

use std::sync::Arc;

use base64::Engine;
use mail_parser::{MessageParser, MimeHeaders};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{
    model::{Addr, AttachmentSpec, Compose, Importance, Mode},
    outbox,
    prepare::{ORIGINAL_MARKER, Prepared},
    store,
};
use crate::{ctx::AppCtx, db::util::now_ms};

#[derive(Debug, Deserialize)]
pub struct SaveParams {
    pub compose: Compose,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub draft_id: String,
    /// local messages.id of the Drafts row
    pub message_id: i64,
}

pub async fn save_rpc(ctx: Arc<AppCtx>, p: SaveParams) -> anyhow::Result<Saved> {
    save(&ctx, p.compose).await
}

/// Create or update the Gmail draft and the local Drafts row.
pub async fn save(ctx: &Arc<AppCtx>, c: Compose) -> anyhow::Result<Saved> {
    let built = outbox::build(ctx, &c).await?;
    let api = ctx.api(c.account_id).await?;
    let thread = built.thread_id.as_deref();
    let create = |thread: Option<&str>| {
        let api = api.clone();
        let raw = built.raw.clone();
        let thread = thread.map(str::to_string);
        async move {
            match api.gmail_create_draft(&raw, thread.as_deref()).await {
                // the conversation is gone: keep the draft as a new one
                Err(e) if thread.is_some() && matches!(crate::google::client::api_status(&e), Some(400 | 404)) => {
                    api.gmail_create_draft(&raw, None).await
                }
                r => r,
            }
        }
    };
    let draft = match &c.draft_id {
        Some(id) => match api.gmail_update_draft(id, &built.raw, thread).await {
            Ok(d) => d,
            Err(e) if crate::google::client::api_status(&e) == Some(404) => create(thread).await?,
            Err(e) => return Err(e),
        },
        None => create(thread).await?,
    };
    let account_id = c.account_id;
    let old = c.draft_id.clone();
    let draft_id = draft.id.clone();
    let gmail_id = draft.message.id.clone();
    let thread_id = if draft.message.thread_id.is_empty() { gmail_id.clone() } else { draft.message.thread_id.clone() };
    let size = built.raw.len() as i64;
    let message_id = ctx
        .db
        .write(move |db| {
            let tx = db.transaction()?;
            if let Some(o) = &old {
                store::delete_local_draft(&tx, account_id, o)?;
            }
            store::delete_local_draft(&tx, account_id, &draft_id)?;
            let refs = if built.references.is_empty() {
                None
            } else {
                Some(built.references.iter().map(|r| format!("<{r}>")).collect::<Vec<_>>().join(" "))
            };
            let irt = built.in_reply_to.as_ref().map(|r| format!("<{r}>"));
            let id = store::insert_local_draft(
                &tx,
                &store::LocalDraft {
                    account_id,
                    gmail_id: &gmail_id,
                    draft_id: &draft_id,
                    thread_id: &thread_id,
                    from: &built.from,
                    compose: &c,
                    html: &built.html,
                    text: &built.text,
                    message_id_header: &built.message_id,
                    in_reply_to: irt.as_deref(),
                    references: refs.as_deref(),
                    size,
                    attachments: &built.files,
                    now: now_ms(),
                },
            )?;
            tx.commit()?;
            Ok(id)
        })
        .await?;
    ctx.events.mail_changed(account_id, None);
    Ok(Saved { draft_id: draft.id, message_id })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardParams {
    pub account_id: i64,
    pub draft_id: String,
}

pub async fn discard(ctx: Arc<AppCtx>, p: DiscardParams) -> anyhow::Result<()> {
    let api = ctx.api(p.account_id).await?;
    if let Err(e) = api.gmail_delete_draft(&p.draft_id).await
        && crate::google::client::api_status(&e) != Some(404)
    {
        return Err(e);
    }
    let (a, d) = (p.account_id, p.draft_id.clone());
    ctx.db.write(move |db| store::delete_local_draft(db, a, &d)).await?;
    ctx.events.mail_changed(p.account_id, None);
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct OpenParams {
    pub id: i64,
}

pub async fn open_rpc(ctx: Arc<AppCtx>, p: OpenParams) -> anyhow::Result<Prepared> {
    open_draft(ctx, p.id).await
}

struct DraftRow {
    account_id: i64,
    gmail_id: String,
    draft_id: Option<String>,
    subject: String,
    to: Vec<Addr>,
    cc: Vec<Addr>,
    bcc: Vec<Addr>,
    from: String,
    html: Option<String>,
    text: Option<String>,
}

/// Compose data for a message in Drafts (by local message id).
pub async fn open_draft(ctx: Arc<AppCtx>, id: i64) -> anyhow::Result<Prepared> {
    let row: Option<DraftRow> = ctx
        .db
        .read(move |db| {
            Ok(db
                .query_row(
                    "SELECT m.account_id, m.gmail_id, m.gmail_draft_id, m.subject, m.to_json, m.cc_json, m.bcc_json, m.from_email, \
                            b.html, b.text FROM messages m LEFT JOIN message_bodies b ON b.message_id = m.id WHERE m.id=?1",
                    [id],
                    |r| {
                        Ok(DraftRow {
                            account_id: r.get(0)?,
                            gmail_id: r.get(1)?,
                            draft_id: r.get(2)?,
                            subject: r.get(3)?,
                            to: store::addrs(&r.get::<_, String>(4)?),
                            cc: store::addrs(&r.get::<_, String>(5)?),
                            bcc: store::addrs(&r.get::<_, String>(6)?),
                            from: r.get(7)?,
                            html: r.get(8)?,
                            text: r.get(9)?,
                        })
                    },
                )
                .optional()?)
        })
        .await?;
    let row = row.ok_or_else(|| crate::user_err!("not_found", "The draft could not be found."))?;
    let choices = ctx.db.read(store::from_choices).await?;

    // Prefer the server copy (complete MIME with attachments).
    let mut draft_id = row.draft_id.clone();
    let mut parsed: Option<Compose> = None;
    if let Ok(api) = ctx.api(row.account_id).await {
        if draft_id.is_none() {
            draft_id = find_draft_id(&api, &row.gmail_id).await.ok().flatten();
            if let (Some(d), a) = (&draft_id, row.account_id) {
                let (d, g) = (d.clone(), row.gmail_id.clone());
                let _ = ctx
                    .db
                    .write(move |db| {
                        db.execute("UPDATE messages SET gmail_draft_id=?1 WHERE account_id=?2 AND gmail_id=?3", params![d, a, g])?;
                        Ok(())
                    })
                    .await;
            }
        }
        if let Some(d) = &draft_id {
            match api.gmail_get_draft(d, "raw").await {
                Ok(dr) => {
                    if let Some(raw) = dr.message.raw.as_deref() {
                        let bytes = crate::google::gmail::b64url_decode(raw)?;
                        parsed = Some(parse_raw(&ctx, &bytes).await?);
                    }
                }
                Err(e) => log::warn!("draft {d}: {}", crate::logging::err(&e)),
            }
        }
    }
    let mut compose = match parsed {
        Some(c) => c,
        None => Compose {
            to: row.to.clone(),
            cc: row.cc.clone(),
            bcc: row.bcc.clone(),
            subject: row.subject.clone(),
            html: row.html.clone().unwrap_or_else(|| {
                row.text.as_deref().map(|t| format!("<p>{}</p>", super::prepare::esc(t).replace('\n', "<br>"))).unwrap_or_default()
            }),
            from: Some(row.from.clone()),
            ..Default::default()
        },
    };
    compose.account_id = row.account_id;
    compose.draft_id = draft_id;
    if compose
        .from
        .as_deref()
        .map(|f| !choices.iter().any(|c| c.account_id == row.account_id && c.email.eq_ignore_ascii_case(f)))
        .unwrap_or(true)
    {
        compose.from = choices.iter().find(|c| c.account_id == row.account_id && c.primary).map(|c| c.email.clone());
    }
    // which message is this a reply to? (In-Reply-To was kept in the draft)
    let html = std::mem::take(&mut compose.html);
    let (body_html, quoted_html) = split_quote(&html);
    // the quote goes into the compose window as raw HTML (innerHTML): drafts come from Gmail, so
    // sanitise it like a fresh quote (the editable part goes through TipTap's schema)
    let quoted_html = if quoted_html.is_empty() { quoted_html } else { super::prepare::sanitize(&quoted_html) };
    Ok(Prepared { compose, body_html, quoted_html, from_choices: choices, draft_message_id: Some(id), reply_targets: None })
}

/// Split off the quoted original (our marker) so it stays raw HTML in the editor.
pub fn split_quote(html: &str) -> (String, String) {
    let inner = body_inner(html);
    match inner.find(ORIGINAL_MARKER) {
        Some(i) => (inner[..i].to_string(), inner[i..].to_string()),
        None => (inner.to_string(), String::new()),
    }
}

/// The contents of <body> (or the input if it isn't a full document).
fn body_inner(html: &str) -> &str {
    let lower = html.to_ascii_lowercase();
    if let Some(b) = lower.find("<body")
        && let Some(gt) = lower[b..].find('>')
    {
        let start = b + gt + 1;
        let end = lower.rfind("</body>").filter(|e| *e >= start).unwrap_or(html.len());
        return &html[start..end];
    }
    html
}

async fn find_draft_id(api: &crate::google::Api, gmail_message_id: &str) -> anyhow::Result<Option<String>> {
    let mut token: Option<String> = None;
    loop {
        let page = api.gmail_list_drafts(token.as_deref()).await?;
        if let Some(d) = page.drafts.iter().find(|d| d.message.id == gmail_message_id) {
            return Ok(Some(d.id.clone()));
        }
        match page.next_page_token {
            Some(t) => token = Some(t),
            None => return Ok(None),
        }
    }
}

fn addr_list(a: Option<&mail_parser::Address>) -> Vec<Addr> {
    a.map(|a| {
        a.iter().filter_map(|x| x.address().map(|e| Addr { name: x.name().unwrap_or("").to_string(), email: e.to_string() })).collect()
    })
    .unwrap_or_default()
}

/// Turn a draft's MIME back into a compose. Attachments are written to a temp folder so they can
/// be re-sent; inline images go back into the HTML as data: URLs.
pub async fn parse_raw(ctx: &Arc<AppCtx>, raw: &[u8]) -> anyhow::Result<Compose> {
    let m = MessageParser::default().parse(raw).ok_or_else(|| anyhow::anyhow!("draft is not a valid message"))?;
    let mut c = Compose {
        from: m.from().and_then(|f| f.first()).and_then(|a| a.address()).map(str::to_string),
        to: addr_list(m.to()),
        cc: addr_list(m.cc()),
        bcc: addr_list(m.bcc()),
        reply_to: addr_list(m.reply_to()),
        subject: m.subject().unwrap_or("").to_string(),
        ..Default::default()
    };
    let has_html = m.html_body_count() > 0 && m.html_part(0).map(|p| p.is_text_html()).unwrap_or(false);
    c.html = if has_html {
        m.body_html(0).map(|h| h.into_owned()).unwrap_or_default()
    } else {
        c.plain_text = true;
        let t = m.body_text(0).map(|t| t.into_owned()).unwrap_or_default();
        t.split('\n').map(|l| format!("<p>{}</p>", super::prepare::esc(l.trim_end_matches('\r')))).collect()
    };
    c.importance = match m.header_raw("X-Priority").map(str::trim) {
        Some(p) if p.starts_with('1') || p.starts_with('2') => Importance::High,
        Some(p) if p.starts_with('4') || p.starts_with('5') => Importance::Low,
        _ => match m.header_raw("Importance").map(|s| s.trim().to_ascii_lowercase()) {
            Some(s) if s == "high" => Importance::High,
            Some(s) if s == "low" => Importance::Low,
            _ => Importance::Normal,
        },
    };
    c.request_read_receipt = m.header_raw("Disposition-Notification-To").is_some();
    c.request_delivery_receipt = m.header_raw("Return-Receipt-To").is_some();
    c.flag_for_recipients = m.header_raw("X-Message-Flag").map(|s| s.trim().to_string());
    // reply / forward source
    if let Some(irt) = m.in_reply_to().as_text().map(|s| s.to_string()) {
        let wanted = [format!("<{irt}>"), irt.clone()];
        let src: Option<i64> = ctx
            .db
            .read(move |db| {
                Ok(db
                    .query_row(
                        "SELECT id FROM messages WHERE message_id_header IN (?1, ?2) ORDER BY is_draft, id LIMIT 1",
                        params![wanted[0], wanted[1]],
                        |r| r.get(0),
                    )
                    .optional()?)
            })
            .await?;
        if let Some(s) = src {
            c.source_message_id = Some(s);
            let subj = c.subject.trim_start().to_ascii_lowercase();
            c.mode = if subj.starts_with("fw:") || subj.starts_with("fwd:") { Mode::Forward } else { Mode::Reply };
        }
    }
    // attachments
    let dir = ctx.data_dir.join("compose").join(uuid::Uuid::new_v4().simple().to_string());
    for part in m.attachments() {
        let bytes = part.contents();
        let mime = part
            .content_type()
            .map(|ct| match ct.subtype() {
                Some(s) => format!("{}/{}", ct.ctype(), s),
                None => ct.ctype().to_string(),
            })
            .unwrap_or_else(|| "application/octet-stream".into());
        if let Some(cid) = part.content_id() {
            let needle = format!("cid:{cid}");
            if c.html.contains(&needle) {
                let url = format!("data:{};base64,{}", mime, base64::engine::general_purpose::STANDARD.encode(bytes));
                c.html = c.html.replace(&needle, &url);
                continue;
            }
        }
        let name = part.attachment_name().map(crate::mail::attach_policy::safe_file_name).unwrap_or_else(|| "attachment".into());
        tokio::fs::create_dir_all(&dir).await?;
        let path = unique_path(&dir, &name);
        tokio::fs::write(&path, bytes).await?;
        crate::mail::attach_policy::mark_from_internet(&path);
        c.attachments.push(AttachmentSpec {
            path: Some(path.to_string_lossy().into_owned()),
            name: Some(name),
            size: Some(bytes.len() as i64),
            mime_type: Some(mime),
            ..Default::default()
        });
    }
    Ok(c)
}

fn unique_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    (2..).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|p| !p.exists()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quote() {
        let html = "<html><head></head><body><p>hi</p><div id=\"cm-original\"><p>old</p></div></body></html>";
        let (a, b) = split_quote(html);
        assert_eq!(a, "<p>hi</p>");
        assert!(b.starts_with("<div id=\"cm-original\""));
        let (a, b) = split_quote("<p>only</p>");
        assert_eq!(a, "<p>only</p>");
        assert!(b.is_empty());
    }
}

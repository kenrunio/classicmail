//! `mail.*` rpc methods (and `demo.deliverMail`).

use std::{collections::HashMap, sync::Arc};

use futures::StreamExt;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    actions, attach_policy,
    body::{self, Invite},
    list::{self, ListParams},
    mime::{self, Addr},
    render, search, store,
};
use crate::{
    accounts,
    ctx::AppCtx,
    db::util::get_setting,
    rpc::{NoParams, Registry},
};

pub fn register(r: &mut Registry) {
    r.add("mail.listFolders", list_folders);
    r.add("mail.listMessages", list_messages);
    r.add("mail.listThread", list_thread);
    r.add("mail.search", search_messages);
    r.add("mail.searchServer", search_server);
    r.add("mail.downloadEstimate", download_estimate);
    r.add("mail.previewAttachment", super::preview::preview_attachment);
    r.add("mail.getMessage", get_message);
    r.add("mail.getSource", get_source);
    r.add("mail.getHeaders", get_headers);
    r.add("mail.adjacent", adjacent);
    r.add("mail.markRead", mark_read);
    r.add("mail.markAllRead", mark_all_read);
    r.add("mail.setFlag", set_flag);
    r.add("mail.listFlagged", list_flagged);
    r.add("mail.move", move_messages);
    r.add("mail.archive", archive_messages);
    r.add("mail.copy", copy_messages);
    r.add("mail.deleteImported", delete_imported);
    r.add("mail.delete", delete_messages);
    r.add("mail.deleteInfo", delete_info);
    r.add("mail.restore", restore);
    r.add("mail.emptyFolder", empty_folder);
    r.add("mail.markJunk", mark_junk);
    r.add("mail.notJunk", not_junk);
    r.add("mail.createFolder", create_folder);
    r.add("mail.renameFolder", rename_folder);
    r.add("mail.moveFolder", move_folder);
    r.add("mail.deleteFolder", delete_folder);
    r.add("mail.saveAttachment", save_attachment);
    r.add("mail.saveAllAttachments", save_all_attachments);
    r.add("mail.openAttachment", open_attachment);
    r.add("mail.addSafeSender", add_safe_sender);
    r.add("mail.listSafeSenders", list_safe_senders);
    r.add("mail.removeSafeSender", remove_safe_sender);
    r.add("demo.deliverMail", deliver_mail);
}

// ---- folders ----------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MailboxFolders {
    account_id: i64,
    email: String,
    display_name: String,
    folders: Vec<store::FolderRow>,
    /// search folders
    unread_mail: i64,
    follow_up: i64,
    large_mail: i64,
}

async fn list_folders(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<Vec<MailboxFolders>> {
    let accts = accounts::list(&ctx.db).await?;
    ctx.db
        .read(move |c| {
            let mut out = Vec::new();
            for a in accts.into_iter().filter(|a| a.enabled) {
                // every whole-mailbox count in one pass over the messages_counts index
                let (all, unread, follow_up, large_mail): (i64, i64, i64, i64) = c.query_row(
                    "SELECT COUNT(*) FILTER (WHERE is_draft=0), COALESCE(SUM(is_read=0 AND is_draft=0),0), \
                       COALESCE(SUM(is_flagged=1 OR flag_status='flagged'),0), COALESCE(SUM(size>102400),0) \
                     FROM messages WHERE account_id=?1 AND location=0",
                    [a.id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?;
                let mut folders = store::folders(c, a.id)?;
                for f in folders.iter_mut() {
                    if f.role == store::ROLE_OUTBOX {
                        f.total = c.query_row("SELECT COUNT(*) FROM outbox WHERE account_id=?1", [a.id], |r| r.get(0)).unwrap_or(0);
                    }
                    if f.role == store::ROLE_ALL {
                        f.total = all;
                        f.unread = unread;
                    }
                }
                out.push(MailboxFolders {
                    account_id: a.id,
                    email: a.email.clone(),
                    display_name: a.display_name.clone(),
                    folders,
                    unread_mail: unread,
                    follow_up,
                    large_mail,
                });
            }
            Ok(out)
        })
        .await
}

async fn list_messages(ctx: Arc<AppCtx>, p: ListParams) -> anyhow::Result<list::ListResult> {
    ctx.db.read(move |c| list::list(c, &p)).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadParams {
    account_id: i64,
    thread_id: String,
    /// the list's params (source / search): marks which messages are in the view
    #[serde(default)]
    view: Option<ListParams>,
}

/// Every message of a conversation across folders, newest first (expanded conversation rows).
async fn list_thread(ctx: Arc<AppCtx>, p: ThreadParams) -> anyhow::Result<Vec<list::Row>> {
    ctx.db.read(move |c| super::threads::list_thread(c, p.account_id, &p.thread_id, p.view.as_ref())).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchParams {
    query: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    folder_id: Option<i64>,
    #[serde(default)]
    limit: Option<i64>,
}

async fn search_messages(ctx: Arc<AppCtx>, p: SearchParams) -> anyhow::Result<list::ListResult> {
    let lp = ListParams {
        source: list::Source { folder_id: p.folder_id, ..Default::default() },
        search: Some(p.query),
        scope: p.scope.or(Some(if p.folder_id.is_some() { "folder".into() } else { "all".into() })),
        limit: p.limit.unwrap_or(500),
        ..Default::default()
    };
    ctx.db.read(move |c| list::list(c, &lp)).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerSearch {
    query: String,
    account_id: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct EstimateParams {
    /// the "last N days" choice (the other choice is all mail)
    days: i64,
}

/// Options › Mail: what the background download would still fetch, for the last `days` days and
/// for all mail, estimated from this mailbox (average stored body; Gmail's message sizes for the
/// attachments).
async fn download_estimate(ctx: Arc<AppCtx>, p: EstimateParams) -> anyhow::Result<Value> {
    ctx.db
        .read(move |c| {
            let avg_body: f64 = c
                .query_row("SELECT AVG(length(coalesce(html,''))+length(coalesce(text,''))) FROM message_bodies", [], |r| r.get::<_, Option<f64>>(0))?
                .unwrap_or(72.0 * 1024.0);
            let range = |since: i64| -> anyhow::Result<Value> {
                let (left, n, gmail_bytes): (i64, i64, i64) = c.query_row(
                    "SELECT COALESCE(SUM(m.body_state=0),0), COUNT(*), COALESCE(SUM(m.size),0) FROM messages m JOIN accounts a ON a.id=m.account_id WHERE a.enabled=1 AND m.location=0 AND m.internal_date>=?1",
                    [since],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                let cached: i64 = c.query_row(
                    "SELECT COALESCE(SUM(t.size),0) FROM attachments t JOIN messages m ON m.id=t.message_id WHERE t.cached_path IS NOT NULL AND m.location=0 AND m.internal_date>=?1",
                    [since],
                    |r| r.get(0),
                )?;
                let attachments_bytes = (gmail_bytes - (n as f64 * avg_body) as i64 - cached).max(0);
                Ok(json!({ "bodiesLeft": left, "bodiesBytes": (left as f64 * avg_body) as i64, "attachmentsBytes": attachments_bytes }))
            };
            let recent = if p.days > 0 { range(crate::db::util::now_ms() - p.days * 86_400_000)? } else { json!({ "bodiesLeft": 0, "bodiesBytes": 0, "attachmentsBytes": 0 }) };
            Ok(json!({ "recent": recent, "all": range(i64::MIN)? }))
        })
        .await
}

/// Search Gmail itself (for mail not indexed locally); pulls missing messages in and returns the
/// matching local ids, newest first.
async fn search_server(ctx: Arc<AppCtx>, p: ServerSearch) -> anyhow::Result<Value> {
    let q = search::parse(&p.query).gmail_q();
    if q.trim().is_empty() {
        return Ok(json!({ "ids": [], "query": q, "extra": 0 }));
    }
    let accts: Vec<i64> = match p.account_id {
        Some(a) => vec![a],
        None => accounts::list(&ctx.db).await?.into_iter().filter(|a| a.enabled).map(|a| a.id).collect(),
    };
    // every account at once (one Gmail request each)
    let results = futures::future::join_all(accts.iter().map(|&a| server_ids(&ctx, a, &q))).await;
    let mut ids: Vec<i64> = Vec::new();
    let mut errors = Vec::new();
    for r in results {
        match r {
            Ok(v) => ids.extend(v),
            Err(e) => errors.push(e),
        }
    }
    if ids.is_empty()
        && let Some(e) = errors.into_iter().next()
    {
        return Err(e);
    }
    // how many of Gmail's matches the local search did not find (bodies not downloaded, text in
    // attachments, Gmail's own matching)
    let query = p.query.clone();
    let ids2 = ids.clone();
    let local: std::collections::HashSet<i64> = ctx
        .db
        .read(move |c| {
            let n = ids2.len() as i64;
            let lp = ListParams {
                source: list::Source { ids: Some(ids2), ..Default::default() },
                search: Some(query),
                limit: n.max(1),
                ..Default::default()
            };
            Ok(list::list(c, &lp)?.rows.into_iter().map(|r| r.id).collect())
        })
        .await?;
    // the ones only Gmail found: the list marks them, and can show just those
    let extra_ids: Vec<i64> = ids.iter().copied().filter(|id| !local.contains(id)).collect();
    Ok(json!({ "ids": ids, "query": q, "extra": extra_ids.len(), "extraIds": extra_ids }))
}

/// Gmail's matches for `q` in one account (newest 100) as local message ids; messages not in the
/// local store yet are pulled in.
async fn server_ids(ctx: &Arc<AppCtx>, account_id: i64, q: &str) -> anyhow::Result<Vec<i64>> {
    let api = ctx.api(account_id).await?;
    let res = api.gmail_list(Some(q), &[], None, 100, false).await?;
    let gids: Vec<String> = res.messages.iter().map(|m| m.id.clone()).collect();
    let gids2 = gids.clone();
    let have: HashMap<String, i64> = ctx
        .db
        .read(move |c| {
            let mut st = c.prepare("SELECT id FROM messages WHERE account_id=?1 AND gmail_id=?2")?;
            let mut m = HashMap::new();
            for g in &gids2 {
                if let Some(id) = st.query_row(params![account_id, g], |r| r.get::<_, i64>(0)).optional()? {
                    m.insert(g.clone(), id);
                }
            }
            Ok(m)
        })
        .await?;
    let missing: Vec<String> = gids.iter().filter(|g| !have.contains_key(*g)).cloned().collect();
    let results: Vec<_> = futures::stream::iter(missing.into_iter().map(|g| super::sync::fetch_meta(&api, g)))
        .buffer_unordered(super::sync::CONCURRENCY)
        .collect()
        .await;
    let mut fetched = Vec::new();
    for (g, r) in results {
        match r {
            Ok(m) => fetched.push(m),
            Err(e) if crate::google::client::api_status(&e) == Some(404) => {}
            Err(e) => log::warn!("mail: server search could not fetch {g}: {}", crate::logging::err(&e)),
        }
    }
    let new_ids: Vec<(String, i64)> = if fetched.is_empty() {
        vec![]
    } else {
        ctx.db
            .write(move |c| {
                let map = store::LabelMap::load(c, account_id)?;
                let tx = c.transaction()?;
                let mut out = Vec::new();
                for m in &fetched {
                    let (id, _) = store::upsert_meta(&tx, account_id, &store::meta_from(m), &map, 0)?;
                    out.push((m.id.clone(), id));
                }
                store::recount(&tx, account_id)?;
                tx.commit()?;
                Ok(out)
            })
            .await?
    };
    // only when something new came in: the folders and lists need refreshing
    if !new_ids.is_empty() {
        ctx.events.mail_changed(account_id, None);
    }
    let mut all = have;
    all.extend(new_ids);
    Ok(gids.iter().filter_map(|g| all.get(g).copied()).collect())
}

// ---- reading -----------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetMessage {
    id: i64,
    #[serde(default)]
    allow_remote: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderRef {
    id: i64,
    name: String,
    role: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AttachmentDto {
    id: i64,
    filename: String,
    mime_type: String,
    size: i64,
    is_inline: bool,
    content_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MessageView {
    id: i64,
    account_id: i64,
    account_email: String,
    gmail_id: String,
    thread_id: String,
    subject: String,
    from_name: String,
    from_email: String,
    to: Vec<Addr>,
    cc: Vec<Addr>,
    bcc: Vec<Addr>,
    reply_to: Vec<Addr>,
    sent_at: Option<i64>,
    received_at: i64,
    is_read: bool,
    is_flagged: bool,
    flag_status: Option<String>,
    flag_due: Option<String>,
    importance: i64,
    last_verb: Option<String>,
    last_verb_at: Option<i64>,
    is_draft: bool,
    size: i64,
    folders: Vec<FolderRef>,
    /// sanitised body HTML (fragment)
    html: String,
    is_plain: bool,
    /// plain text of the body (print / quoting)
    text: String,
    remote_blocked: bool,
    has_remote: bool,
    safe_sender: bool,
    attachments: Vec<AttachmentDto>,
    invite: Option<Invite>,
    body_error: Option<String>,
    message_id_header: Option<String>,
}

/// A sender whose pictures load without asking: someone in your contacts, or someone whose pictures
/// you chose to download on an earlier message.
fn is_known_sender(c: &rusqlite::Connection, email: &str) -> anyhow::Result<bool> {
    let e = email.trim().to_lowercase();
    if e.is_empty() {
        return Ok(false);
    }
    let before: bool = c
        .prepare_cached("SELECT EXISTS(SELECT 1 FROM messages WHERE allow_remote=1 AND lower(from_email)=?1)")?
        .query_row([&e], |r| r.get(0))?;
    if before {
        return Ok(true);
    }
    Ok(c.prepare_cached("SELECT EXISTS(SELECT 1 FROM contacts ct, json_each(ct.emails) j WHERE lower(j.value ->> 'value') = ?1)")?
        .query_row([&e], |r| r.get(0))
        .unwrap_or(false))
}

fn is_safe_sender(c: &rusqlite::Connection, email: &str) -> anyhow::Result<bool> {
    let e = email.to_lowercase();
    let domain = e.split('@').nth(1).map(|d| format!("@{d}")).unwrap_or_default();
    Ok(c.prepare("SELECT 1 FROM safe_senders WHERE lower(entry)=?1 OR lower(entry)=?2")?.exists(params![e, domain])?)
}

async fn get_message(ctx: Arc<AppCtx>, p: GetMessage) -> anyhow::Result<MessageView> {
    let id = p.id;
    let body_error = match body::ensure_body(&ctx, id).await {
        Ok(()) => None,
        Err(e) => {
            if e.downcast_ref::<crate::error::UserError>().map(|u| u.code == "not_found").unwrap_or(false) {
                return Err(e);
            }
            Some(format!("{e:#}"))
        }
    };
    let allow_param = p.allow_remote;
    if allow_param {
        // remember "Download Pictures" for this message (local only)
        ctx.db
            .write(move |c| {
                c.execute("UPDATE messages SET allow_remote=1 WHERE id=?1 AND allow_remote=0", [id])?;
                Ok(())
            })
            .await?;
    }
    ctx.db
        .read(move |c| {
            type R = (i64, String, String, String, String, String, String, String, String, String, Option<i64>, i64, i64, i64, Option<String>, Option<String>, i64, Option<String>, Option<i64>, i64, i64, String, Option<String>, i64);
            let r: R = c.query_row(
                "SELECT m.account_id, a.email, m.gmail_id, m.thread_id, m.subject, m.from_name, m.from_email, m.to_json, m.cc_json, m.bcc_json || '\u{1}' || m.reply_to_json, \
                 m.sent_date, m.internal_date, m.is_read, m.is_flagged, m.flag_status, m.flag_due, m.importance, m.last_verb, m.last_verb_at, m.is_draft, m.size, m.snippet, m.message_id_header, m.allow_remote \
                 FROM messages m JOIN accounts a ON a.id=m.account_id WHERE m.id=?1",
                [id],
                |r| {
                    Ok((
                        r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?, r.get(13)?, r.get(14)?, r.get(15)?,
                        r.get(16)?, r.get(17)?, r.get(18)?, r.get(19)?, r.get(20)?, r.get(21)?, r.get(22)?, r.get(23)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| crate::user_err!("not_found", "The message no longer exists."))?;
            let parse = |s: &str| serde_json::from_str::<Vec<Addr>>(s).unwrap_or_default();
            let (bcc_s, reply_s) = r.9.split_once('\u{1}').unwrap_or((&r.9, "[]"));
            let folders: Vec<FolderRef> = {
                let mut st = c.prepare("SELECT f.id, f.name, f.role FROM message_folders mf JOIN folders f ON f.id=mf.folder_id WHERE mf.message_id=?1 ORDER BY f.position, f.path")?;
                st.query_map([id], |r| Ok(FolderRef { id: r.get(0)?, name: r.get(1)?, role: r.get(2)? }))?.collect::<Result<_, _>>()?
            };
            let atts: Vec<AttachmentDto> = {
                let mut st = c.prepare("SELECT id, filename, mime_type, size, is_inline, content_id FROM attachments WHERE message_id=?1 ORDER BY id")?;
                st.query_map([id], |r| Ok(AttachmentDto { id: r.get(0)?, filename: r.get(1)?, mime_type: r.get(2)?, size: r.get(3)?, is_inline: r.get::<_, i64>(4)? != 0, content_id: r.get(5)? }))?
                    .collect::<Result<_, _>>()?
            };
            let bodyrow: Option<(Option<String>, Option<String>, Option<String>)> =
                c.query_row("SELECT html, text, calendar_ics FROM message_bodies WHERE message_id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
            let safe = is_safe_sender(c, &r.6)?;
            let block_setting: bool = get_setting(c, "mail.blockRemoteImages")?.unwrap_or(true);
            let own = r.6.eq_ignore_ascii_case(&r.1);
            // "Download Pictures" once on this message → remembered (set below when allowRemote is passed)
            let remembered = r.23 != 0;
            // senders you trust implicitly: in your contacts, or you've downloaded their pictures before
            let auto_known: bool = get_setting(c, "mail.autoPicturesKnownSenders")?.unwrap_or(true);
            let known = auto_known && !allow_param && !remembered && !safe && is_known_sender(c, &r.6)?;
            let allow = allow_param || remembered || safe || !block_setting || own || known;
            let cids: HashMap<String, i64> = atts.iter().filter_map(|a| a.content_id.as_ref().map(|cid| (cid.to_ascii_lowercase(), a.id))).collect();
            let (html, text, ics) = bodyrow.unwrap_or((None, None, None));
            let text = text.unwrap_or_default();
            let (html_out, plain, blocked, has_remote) = match &html {
                Some(h) if !h.trim().is_empty() => {
                    let rr = render::render_html(h, &cids, allow);
                    (rr.html, false, rr.blocked, rr.has_remote)
                }
                _ => {
                    let t = if text.is_empty() { r.21.clone() } else { text.clone() };
                    (mime::text_to_html(&t), true, false, false)
                }
            };
            let invite = ics.as_deref().and_then(body::parse_invite);
            Ok(MessageView {
                id,
                account_id: r.0,
                account_email: r.1,
                gmail_id: r.2,
                thread_id: r.3,
                subject: r.4,
                from_name: r.5,
                from_email: r.6,
                to: parse(&r.7),
                cc: parse(&r.8),
                bcc: parse(bcc_s),
                reply_to: parse(reply_s),
                sent_at: r.10,
                received_at: r.11,
                is_read: r.12 != 0,
                is_flagged: r.13 != 0,
                flag_status: r.14,
                flag_due: r.15,
                importance: r.16,
                last_verb: r.17,
                last_verb_at: r.18,
                is_draft: r.19 != 0,
                size: r.20,
                folders,
                html: html_out,
                is_plain: plain,
                text: if text.is_empty() { r.21 } else { text },
                remote_blocked: blocked,
                has_remote,
                safe_sender: safe,
                attachments: atts,
                invite,
                body_error,
                message_id_header: r.22,
            })
        })
        .await
}

#[derive(Deserialize)]
struct IdParam {
    id: i64,
}

#[derive(Serialize)]
struct MessageSource {
    /// the HTML body as it arrived (before ClassicMail made it safe to show), if it has one
    html: Option<String>,
    /// the whole message as sent: headers, every part, attachments encoded (fetched from Gmail)
    raw: String,
}

/// View Source: a message's HTML as it arrived, and the whole original message.
async fn get_source(ctx: Arc<AppCtx>, p: IdParam) -> anyhow::Result<MessageSource> {
    let id = p.id;
    let (account_id, gmail_id, html): (i64, String, Option<String>) = ctx
        .db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT m.account_id, m.gmail_id, b.html FROM messages m LEFT JOIN message_bodies b ON b.message_id = m.id WHERE m.id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await?;
    if gmail_id.is_empty() {
        return Err(crate::user_err!("no_source", "This message isn't on the server yet, so it has no source to show."));
    }
    let api = ctx.api(account_id).await?;
    let m = api.gmail_get(&gmail_id, "raw", &[]).await?;
    let bytes = crate::google::gmail::b64url_decode(m.raw.as_deref().unwrap_or_default())?;
    Ok(MessageSource { html: html.filter(|h| !h.trim().is_empty()), raw: String::from_utf8_lossy(&bytes).into_owned() })
}

/// Internet headers (Message Options).
async fn get_headers(ctx: Arc<AppCtx>, p: IdParam) -> anyhow::Result<String> {
    body::ensure_body(&ctx, p.id).await?;
    let raw: Option<String> = ctx
        .db
        .read(move |c| {
            Ok(c.query_row("SELECT headers_json FROM message_bodies WHERE message_id=?1", [p.id], |r| r.get(0)).optional()?.flatten())
        })
        .await?;
    let v: Vec<Value> = raw.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
    Ok(v.iter()
        .map(|h| format!("{}: {}", h["name"].as_str().unwrap_or(""), h["value"].as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\r\n"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdjacentParams {
    id: i64,
    folder_id: Option<i64>,
    /// "next" (older) | "previous" (newer)
    direction: String,
}

/// Previous / next item for the message window's arrows (by date within the folder).
async fn adjacent(ctx: Arc<AppCtx>, p: AdjacentParams) -> anyhow::Result<Option<i64>> {
    ctx.db
        .read(move |c| {
            let (date, folder): (i64, Option<i64>) = c.query_row(
                "SELECT m.internal_date, (SELECT folder_id FROM message_folders x JOIN folders f ON f.id=x.folder_id WHERE x.message_id=m.id ORDER BY f.position LIMIT 1) FROM messages m WHERE m.id=?1",
                [p.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let Some(folder) = p.folder_id.or(folder) else { return Ok(None) };
            let next = p.direction != "previous";
            let sql = if next {
                "SELECT message_id FROM message_folders WHERE folder_id=?1 AND (msg_date < ?2 OR (msg_date = ?2 AND message_id < ?3)) ORDER BY msg_date DESC, message_id DESC LIMIT 1"
            } else {
                "SELECT message_id FROM message_folders WHERE folder_id=?1 AND (msg_date > ?2 OR (msg_date = ?2 AND message_id > ?3)) ORDER BY msg_date ASC, message_id ASC LIMIT 1"
            };
            Ok(c.query_row(sql, params![folder, date, p.id], |r| r.get(0)).optional()?)
        })
        .await
}

// ---- actions -----------------------------------------------------------------------------------

#[derive(Deserialize)]
struct IdsParam {
    ids: Vec<i64>,
}

#[derive(Deserialize)]
struct MarkRead {
    ids: Vec<i64>,
    read: bool,
}

async fn mark_read(ctx: Arc<AppCtx>, p: MarkRead) -> anyhow::Result<()> {
    actions::mark_read(&ctx, p.ids, p.read).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FolderParam {
    folder_id: i64,
}

async fn mark_all_read(ctx: Arc<AppCtx>, p: FolderParam) -> anyhow::Result<usize> {
    let ids = actions::folder_message_ids(&ctx, p.folder_id, true).await?;
    let n = ids.len();
    actions::mark_read(&ctx, ids, true).await?;
    Ok(n)
}

#[derive(Deserialize)]
struct SetFlag {
    ids: Vec<i64>,
    flag: String,
    due: Option<String>,
}

async fn set_flag(ctx: Arc<AppCtx>, p: SetFlag) -> anyhow::Result<()> {
    const OK: &[&str] = &["none", "today", "tomorrow", "thisWeek", "nextWeek", "noDate", "custom", "complete"];
    if !OK.contains(&p.flag.as_str()) {
        return Err(crate::user_err!("bad_params", "unknown flag {}", p.flag));
    }
    if p.flag == "custom" && p.due.as_deref().map(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err()).unwrap_or(true) {
        return Err(crate::user_err!("bad_params", "custom flags need a due date (YYYY-MM-DD)"));
    }
    actions::set_flag(&ctx, p.ids, p.flag, p.due).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Flagged {
    id: i64,
    account_id: i64,
    subject: String,
    from: String,
    received_at: i64,
    flag_due: Option<String>,
    flag_status: String,
}

async fn list_flagged(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<Vec<Flagged>> {
    let since = crate::db::util::now_ms() - 14 * 86_400_000;
    ctx.db
        .read(move |c| {
            let mut st = c.prepare(
                "SELECT id, account_id, subject, CASE WHEN from_name<>'' THEN from_name ELSE from_email END, internal_date, flag_due, \
                 CASE WHEN flag_status='complete' AND is_flagged=0 THEN 'complete' ELSE 'flagged' END \
                 FROM messages WHERE location=0 AND (is_flagged=1 OR flag_status='flagged' OR (flag_status='complete' AND COALESCE(flag_completed_at,0) >= ?1)) \
                 ORDER BY COALESCE(flag_due, '9999'), internal_date DESC LIMIT 1000",
            )?;
            let v = st
                .query_map([since], |r| {
                    Ok(Flagged { id: r.get(0)?, account_id: r.get(1)?, subject: r.get(2)?, from: r.get(3)?, received_at: r.get(4)?, flag_due: r.get(5)?, flag_status: r.get(6)? })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveParams {
    ids: Vec<i64>,
    folder_id: i64,
    /// the folder the items are being moved out of (the one being viewed)
    from_folder_id: Option<i64>,
}

async fn move_messages(ctx: Arc<AppCtx>, p: MoveParams) -> anyhow::Result<actions::Transferred> {
    actions::move_to(&ctx, p.ids, p.folder_id, p.from_folder_id, false).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveParams {
    ids: Vec<i64>,
    /// the folder being viewed (its label is the one removed)
    from_folder_id: Option<i64>,
}

async fn archive_messages(ctx: Arc<AppCtx>, p: ArchiveParams) -> anyhow::Result<()> {
    actions::archive(&ctx, p.ids, p.from_folder_id).await
}

async fn copy_messages(ctx: Arc<AppCtx>, p: MoveParams) -> anyhow::Result<actions::Transferred> {
    actions::move_to(&ctx, p.ids, p.folder_id, p.from_folder_id, true).await
}

#[derive(Deserialize)]
struct ImportedParams {
    copies: Vec<actions::Imported>,
}

/// Undo of a move / copy between mailboxes (undo/inverse.rs): remove the copies it made.
async fn delete_imported(ctx: Arc<AppCtx>, p: ImportedParams) -> anyhow::Result<()> {
    actions::delete_imported(&ctx, p.copies).await
}

/// Before Delete: how many of the ids it would erase for good (the rest go to Deleted Items).
async fn delete_info(ctx: Arc<AppCtx>, p: IdsParam) -> anyhow::Result<Value> {
    let permanent = ctx.db.read(move |c| actions::permanent_count(c, &p.ids)).await?;
    Ok(json!({ "permanent": permanent }))
}

async fn delete_messages(ctx: Arc<AppCtx>, p: IdsParam) -> anyhow::Result<Value> {
    let permanent = actions::delete(&ctx, p.ids).await?;
    Ok(json!({ "permanent": permanent }))
}

async fn restore(ctx: Arc<AppCtx>, p: IdsParam) -> anyhow::Result<()> {
    actions::restore(&ctx, p.ids).await
}

async fn empty_folder(ctx: Arc<AppCtx>, p: FolderParam) -> anyhow::Result<usize> {
    actions::empty_folder(&ctx, p.folder_id).await
}

async fn mark_junk(ctx: Arc<AppCtx>, p: IdsParam) -> anyhow::Result<()> {
    actions::mark_junk(&ctx, p.ids, true).await
}

async fn not_junk(ctx: Arc<AppCtx>, p: IdsParam) -> anyhow::Result<()> {
    actions::mark_junk(&ctx, p.ids, false).await
}

// ---- folders (online: Gmail labels) ------------------------------------------------------------

fn valid_name(name: &str) -> anyhow::Result<String> {
    let n = name.trim();
    if n.is_empty() || n.contains('/') {
        return Err(crate::user_err!("bad_name", "Folder names can't be empty or contain \"/\"."));
    }
    Ok(n.to_string())
}

async fn refresh_labels(ctx: &Arc<AppCtx>, account_id: i64) -> anyhow::Result<()> {
    let api = ctx.api(account_id).await?;
    let labels = api.gmail_labels().await?;
    ctx.db
        .write(move |c| {
            let show_all: bool = get_setting(c, "mail.showAllMail")?.unwrap_or(false);
            store::sync_labels(c, account_id, &labels, show_all)?;
            let map = store::LabelMap::load(c, account_id)?;
            let tx = c.transaction()?;
            store::rederive_membership(&tx, account_id, &map)?;
            store::recount(&tx, account_id)?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    ctx.events.mail_changed(account_id, None);
    Ok(())
}

/// Google's label errors in words: 409 = taken, 400 = a name Gmail keeps for itself ("Important",
/// "Chats", ...; Gmail decides the list) — `name` is the folder name the user typed.
fn label_err(name: &str) -> impl Fn(anyhow::Error) -> anyhow::Error + '_ {
    move |e| match crate::google::client::api_status(&e) {
        Some(409) => crate::user_err!("exists", "A folder with this name already exists."),
        Some(400) => crate::user_err!(
            "bad_name",
            "Gmail uses the name \"{name}\" for one of its own labels, so a folder here can't have it. Choose another name, or put the folder inside another one (Inbox, for example)."
        ),
        _ => e,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateFolder {
    account_id: Option<i64>,
    parent_id: Option<i64>,
    name: String,
}

/// Where a folder made (or moved) under `parent` goes: its account and the label path prefix ("" =
/// top level). Inside Inbox is a label "Inbox/<name>" (as Outlook makes them over IMAP; the tree
/// shows them in the real Inbox) — the existing "Inbox" spelling of the mailbox is kept. Other
/// system folders can't hold labels, so their subfolders go to the top level.
async fn parent_prefix(ctx: &Arc<AppCtx>, parent_id: Option<i64>, account_id: Option<i64>) -> anyhow::Result<(i64, String)> {
    let Some(pid) = parent_id else {
        return Ok((account_id.ok_or_else(|| crate::user_err!("bad_params", "accountId or parentId is required"))?, String::new()));
    };
    let parent = ctx.db.read(move |c| store::folder(c, pid)).await?;
    let prefix = match parent.role.as_str() {
        store::ROLE_USER | store::ROLE_PLACEHOLDER => parent.path,
        store::ROLE_INBOX => {
            let acct = parent.account_id;
            ctx.db
                .read(move |c| {
                    Ok(c.query_row(
                        "SELECT path FROM folders WHERE account_id=?1 AND parent_id IS NULL AND role IN ('user','placeholder') AND lower(path)='inbox'",
                        [acct],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?)
                })
                .await?
                .unwrap_or_else(|| "Inbox".into())
        }
        _ => String::new(),
    };
    Ok((parent.account_id, prefix))
}

fn join_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") }
}

async fn create_folder(ctx: Arc<AppCtx>, p: CreateFolder) -> anyhow::Result<i64> {
    let name = valid_name(&p.name)?;
    let (account_id, prefix) = parent_prefix(&ctx, p.parent_id, p.account_id).await?;
    let path = join_path(&prefix, &name);
    let api = ctx.api(account_id).await?;
    let label = api.gmail_create_label(&path).await.map_err(label_err(&name))?;
    refresh_labels(&ctx, account_id).await?;
    let lid = label.id.clone();
    ctx.db
        .read(move |c| {
            Ok(c.query_row("SELECT id FROM folders WHERE account_id=?1 AND gmail_label_id=?2", params![account_id, lid], |r| r.get(0))?)
        })
        .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenameFolder {
    folder_id: i64,
    name: String,
}

async fn rename_folder(ctx: Arc<AppCtx>, p: RenameFolder) -> anyhow::Result<()> {
    let name = valid_name(&p.name)?;
    let f = ctx.db.read(move |c| store::folder(c, p.folder_id)).await?;
    if f.role != store::ROLE_USER {
        return Err(crate::user_err!("bad_folder", "This folder can't be renamed."));
    }
    let new_path = match f.path.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{name}"),
        None => name.clone(),
    };
    relabel(&ctx, &f, &new_path, &name).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveFolder {
    folder_id: i64,
    /// the new parent folder (a folder, Inbox) — or none for the top of `account_id`'s mailbox
    parent_id: Option<i64>,
    account_id: Option<i64>,
    /// instead of a parent: the label path to go under ("" = top level) — what Undo puts back
    parent_path: Option<String>,
}

/// Drag a folder onto another folder: Gmail has no folder move, so the label (and its sublabels)
/// get the new parent's path.
async fn move_folder(ctx: Arc<AppCtx>, p: MoveFolder) -> anyhow::Result<()> {
    let f = ctx.db.read(move |c| store::folder(c, p.folder_id)).await?;
    if f.role != store::ROLE_USER {
        return Err(crate::user_err!("bad_folder", "This folder can't be moved."));
    }
    let (account_id, prefix) = match p.parent_path {
        Some(pp) => (f.account_id, pp),
        None => parent_prefix(&ctx, p.parent_id, p.account_id.or(Some(f.account_id))).await?,
    };
    if account_id != f.account_id {
        return Err(crate::user_err!("bad_folder", "A folder can only be moved within its own mailbox."));
    }
    let lower = prefix.to_lowercase();
    let own = f.path.to_lowercase();
    if lower == own || lower.starts_with(&format!("{own}/")) {
        return Err(crate::user_err!("bad_folder", "A folder can't be moved into itself or one of its subfolders."));
    }
    let new_path = join_path(&prefix, &f.name);
    if new_path == f.path {
        return Ok(());
    }
    relabel(&ctx, &f, &new_path, &f.name).await
}

/// Give folder `f` the label path `new_path`, its subfolders following.
async fn relabel(ctx: &Arc<AppCtx>, f: &store::FolderRow, new_path: &str, name: &str) -> anyhow::Result<()> {
    let api = ctx.api(f.account_id).await?;
    // Gmail doesn't rename nested labels: rename the whole subtree
    let old_prefix = format!("{}/", f.path);
    let account_id = f.account_id;
    let subtree: Vec<(String, String)> = ctx
        .db
        .read(move |c| {
            let mut st =
                c.prepare("SELECT gmail_label_id, path FROM folders WHERE account_id=?1 AND gmail_label_id IS NOT NULL AND role='user'")?;
            let v = st.query_map([account_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(v.into_iter().filter(|(_, p)| p.starts_with(&old_prefix)).collect())
        })
        .await?;
    api.gmail_rename_label(f.gmail_label_id.as_deref().unwrap_or_default(), new_path).await.map_err(label_err(name))?;
    for (lid, path) in subtree {
        let rest = &path[f.path.len()..];
        api.gmail_rename_label(&lid, &format!("{new_path}{rest}")).await.map_err(label_err(name))?;
    }
    refresh_labels(ctx, f.account_id).await
}

/// Outlook semantics: the folder's messages go to Deleted Items, then the label (and sublabels) go.
async fn delete_folder(ctx: Arc<AppCtx>, p: FolderParam) -> anyhow::Result<()> {
    let f = ctx.db.read(move |c| store::folder(c, p.folder_id)).await?;
    if f.role != store::ROLE_USER && f.role != store::ROLE_PLACEHOLDER {
        return Err(crate::user_err!("bad_folder", "This folder can't be deleted."));
    }
    let account_id = f.account_id;
    let prefix = format!("{}/", f.path);
    let fpath = f.path.clone();
    let subtree: Vec<(i64, Option<String>, String)> = ctx
        .db
        .read(move |c| {
            let mut st =
                c.prepare("SELECT id, gmail_label_id, path FROM folders WHERE account_id=?1 AND role IN ('user','placeholder')")?;
            let v = st.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(v.into_iter().filter(|(_, _, path)| *path == fpath || path.starts_with(&prefix)).collect())
        })
        .await?;
    // messages that live only in these folders → Deleted Items
    let folder_ids: Vec<i64> = subtree.iter().map(|s| s.0).collect();
    let fids = folder_ids.clone();
    let only_here: Vec<i64> = ctx
        .db
        .read(move |c| {
            let list = fids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
            let mut st = c.prepare(&format!(
                "SELECT DISTINCT mf.message_id FROM message_folders mf WHERE mf.folder_id IN ({list}) \
                 AND NOT EXISTS (SELECT 1 FROM message_folders o WHERE o.message_id=mf.message_id AND o.folder_id NOT IN ({list}))"
            ))?;
            let v = st.query_map([], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?;
            Ok(v)
        })
        .await?;
    if !only_here.is_empty() {
        actions::delete(&ctx, only_here).await?;
        // push the trash ops before the labels disappear
        let api = ctx.api(account_id).await?;
        let _ = super::sync::push_now(&ctx, &api).await;
    }
    let api = ctx.api(account_id).await?;
    let mut labels: Vec<(Option<String>, String)> = subtree.into_iter().map(|s| (s.1, s.2)).collect();
    labels.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    for (lid, _) in labels {
        if let Some(lid) = lid {
            match api.gmail_delete_label(&lid).await {
                Ok(()) => {}
                Err(e) if crate::google::client::api_status(&e) == Some(404) => {}
                Err(e) => return Err(e),
            }
        }
    }
    refresh_labels(&ctx, account_id).await
}

// ---- attachments ------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveAttachment {
    attachment_id: i64,
    path: String,
}

async fn save_attachment(ctx: Arc<AppCtx>, p: SaveAttachment) -> anyhow::Result<String> {
    let (_, _, bytes) = body::attachment_bytes(&ctx, p.attachment_id).await?;
    let path = p.path.clone();
    tokio::task::spawn_blocking(move || attach_policy::write_marked(std::path::Path::new(&path), &bytes))
        .await?
        .map_err(|e| crate::user_err!("io", "Can't save {}: {e}", p.path))?;
    Ok(p.path)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveAll {
    message_id: i64,
    dir: String,
}

async fn save_all_attachments(ctx: Arc<AppCtx>, p: SaveAll) -> anyhow::Result<Vec<String>> {
    let mid = p.message_id;
    let ids: Vec<i64> = ctx
        .db
        .read(move |c| {
            let mut st = c.prepare("SELECT id FROM attachments WHERE message_id=?1 AND is_inline=0 ORDER BY id")?;
            let v = st.query_map([mid], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?;
            Ok(v)
        })
        .await?;
    let dir = std::path::PathBuf::from(&p.dir);
    let mut out = Vec::new();
    for id in ids {
        let (name, _, bytes) = body::attachment_bytes(&ctx, id).await?;
        let dir = dir.clone();
        let path = tokio::task::spawn_blocking(move || {
            let mut path = dir.join(attach_policy::safe_file_name(&name));
            let mut n = 1;
            while path.exists() {
                let stem = std::path::Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let ext = std::path::Path::new(&name).extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
                path = dir.join(attach_policy::safe_file_name(&format!("{stem} ({n}){ext}")));
                n += 1;
            }
            attach_policy::write_marked(&path, &bytes).map_err(|e| crate::user_err!("io", "Can't save {}: {e}", path.display()))?;
            anyhow::Ok(path)
        })
        .await??;
        out.push(path.to_string_lossy().into_owned());
    }
    Ok(out)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenAttachment {
    attachment_id: i64,
    /// the user answered "Open" to the "Opening Mail Attachment" question
    #[serde(default)]
    confirmed: bool,
    /// show Windows' "Open with" chooser instead of using the default program
    #[serde(default)]
    choose: bool,
}

/// Open with the default program (Outlook rules: blocked types refused, others may need a "yes").
async fn open_attachment(ctx: Arc<AppCtx>, p: OpenAttachment) -> anyhow::Result<String> {
    let (name, _, bytes) = body::attachment_bytes(&ctx, p.attachment_id).await?;
    attach_policy::check_open(&name, p.confirmed)?;
    let dir = std::env::temp_dir().join("ClassicMail").join(p.attachment_id.to_string());
    let path = dir.join(attach_policy::safe_file_name(&name));
    let p2 = path.clone();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&dir)?;
        attach_policy::write_marked(&p2, &bytes)
    })
    .await??;
    let s = path.to_string_lossy().into_owned();
    attach_policy::launch(ctx.app.as_ref(), &s, &name, p.choose)?;
    Ok(s)
}

// ---- safe senders -----------------------------------------------------------------------------

#[derive(Deserialize)]
struct AddressParam {
    address: String,
}

async fn add_safe_sender(ctx: Arc<AppCtx>, p: AddressParam) -> anyhow::Result<()> {
    let a = p.address.trim().to_lowercase();
    if a.is_empty() || !a.contains('@') {
        return Err(crate::user_err!("bad_params", "Enter an e-mail address or @domain."));
    }
    ctx.db
        .write(move |c| {
            c.execute("INSERT OR IGNORE INTO safe_senders(entry) VALUES (?1)", [a])?;
            Ok(())
        })
        .await?;
    ctx.events.emit("mail.changed", json!({ "accountId": null }));
    Ok(())
}

async fn list_safe_senders(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<Vec<String>> {
    ctx.db
        .read(|c| {
            let mut st = c.prepare("SELECT entry FROM safe_senders ORDER BY entry")?;
            let v = st.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
            Ok(v)
        })
        .await
}

async fn remove_safe_sender(ctx: Arc<AppCtx>, p: AddressParam) -> anyhow::Result<()> {
    let a = p.address.trim().to_lowercase();
    ctx.db
        .write(move |c| {
            c.execute("DELETE FROM safe_senders WHERE lower(entry)=?1", [a])?;
            Ok(())
        })
        .await?;
    ctx.events.emit("mail.changed", json!({ "accountId": null }));
    Ok(())
}

// ---- demo -------------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DeliverParams {
    account_id: Option<i64>,
}

/// Demo mode: a new message arrives in a demo mailbox (then a Send/Receive picks it up).
async fn deliver_mail(ctx: Arc<AppCtx>, p: DeliverParams) -> anyhow::Result<Value> {
    let accts = accounts::list(&ctx.db).await?;
    let acct = match p.account_id {
        Some(id) => accts.into_iter().find(|a| a.id == id),
        None => accts.into_iter().find(|a| a.provider == "demo" && a.enabled),
    }
    .ok_or_else(|| crate::user_err!("not_found", "No demo account to deliver to."))?;
    if acct.provider != "demo" {
        return Err(crate::user_err!("not_demo", "Only demo mailboxes can receive simulated mail."));
    }
    ctx.fake_base().await?;
    let gmail_id = super::sync::deliver_demo(&acct.email).ok_or_else(|| crate::user_err!("no_server", "The demo server isn't running."))?;
    ctx.sync_now(Some(acct.id));
    Ok(json!({ "accountId": acct.id, "gmailId": gmail_id }))
}

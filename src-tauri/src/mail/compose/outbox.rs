//! Sending: `mail.compose.send` queues the built message in the Outbox; `process_outbox` (start of
//! every mail sync) sends what is due with `messages.send`.

use std::sync::Arc;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    attachments,
    mime::{self, Envelope},
    model::{Compose, Mode, valid_email},
    prepare::set_last_verb,
    store::{self, SourceMsg},
};
use crate::{ctx::AppCtx, db::util::now_ms, google::Api};

/// A compose turned into RFC 822 bytes, with what sending / saving needs to know.
pub struct Built {
    pub raw: Vec<u8>,
    pub from: crate::mail::compose::model::Addr,
    pub message_id: String,
    pub thread_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    /// html / text as sent (for local Drafts rows)
    pub html: String,
    pub text: String,
    /// (filename, mime, size) of real attachments
    pub files: Vec<(String, String, i64)>,
}

/// Build the message for a compose (resolves From, threading headers and attachments).
pub async fn build(ctx: &Arc<AppCtx>, c: &Compose) -> anyhow::Result<Built> {
    let account_id = c.account_id;
    let from_req = c.from.clone();
    let source_id = c.source_message_id;
    let (from, src): (_, Option<SourceMsg>) = ctx
        .db
        .read(move |db| {
            let from = store::identity(db, account_id, from_req.as_deref())?;
            let src = match source_id {
                Some(id) => store::load_source(db, id)?,
                None => None,
            };
            Ok((from, src))
        })
        .await?;
    let mut in_reply_to = None;
    let mut references = Vec::new();
    let mut thread_id = None;
    if let Some(s) = &src {
        if let Some(mid) = s.message_id_header.as_deref().map(mime::split_ids).and_then(|v| v.into_iter().next()) {
            references = s.references_header.as_deref().map(mime::split_ids).unwrap_or_default();
            references.retain(|r| r != &mid);
            references.push(mid.clone());
            // keep References reasonably short (first + last 9, like most clients)
            if references.len() > 10 {
                let first = references[0].clone();
                let tail = references.split_off(references.len() - 9);
                references = std::iter::once(first).chain(tail).collect();
            }
            in_reply_to = Some(mid);
        }
        // Gmail thread ids are per mailbox
        if s.account_id == account_id && !s.thread_id.is_empty() {
            thread_id = Some(s.thread_id.clone());
        }
    }
    let parts = attachments::resolve(ctx, &c.attachments).await?;
    let files: Vec<(String, String, i64)> =
        parts.iter().filter(|p| p.cid.is_none()).map(|p| (p.filename.clone(), p.mime.clone(), p.bytes.len() as i64)).collect();
    let message_id = mime::new_message_id(&from.email);
    let env = Envelope {
        from: from.clone(),
        message_id: message_id.clone(),
        date: crate::clock::now_local().fixed_offset(),
        in_reply_to: in_reply_to.clone(),
        references: references.clone(),
    };
    let raw = mime::build(c, &env, parts)?;
    let text = c.text.clone().unwrap_or_else(|| mime::html_to_text(&c.html));
    Ok(Built { raw, from, message_id, thread_id, in_reply_to, references, html: c.html.clone(), text, files })
}

pub fn validate(c: &Compose) -> anyhow::Result<()> {
    if c.all_recipients().next().is_none() {
        return Err(crate::user_err!("no_recipients", "There must be at least one name or contact group in the To, Cc, or Bcc box."));
    }
    let bad: Vec<String> = c.all_recipients().filter(|a| !valid_email(&a.email)).map(|a| a.display()).collect();
    if !bad.is_empty() {
        return Err(crate::user_err!("bad_recipient", "ClassicMail does not recognize one or more names: {}", bad.join("; ")));
    }
    for a in &c.reply_to {
        if !valid_email(&a.email) {
            return Err(crate::user_err!("bad_recipient", "The \"Have replies sent to\" address {} is not valid.", a.display()));
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct SendParams {
    pub compose: Compose,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Queued {
    pub outbox_id: i64,
    pub send_at: i64,
}

pub async fn send(ctx: Arc<AppCtx>, p: SendParams) -> anyhow::Result<Queued> {
    let c = p.compose;
    let send_at = c.defer_until.unwrap_or(0);
    let mut q = queue_many(&ctx, vec![(c, send_at)]).await?;
    Ok(q.remove(0))
}

/// Queue several messages (each with its own "not before" time, epoch ms; past = now). All are
/// validated and built first, then inserted in one transaction: either all are queued or none.
/// All must be from the same account.
pub async fn queue_many(ctx: &Arc<AppCtx>, items: Vec<(Compose, i64)>) -> anyhow::Result<Vec<Queued>> {
    let Some(account_id) = items.first().map(|(c, _)| c.account_id) else { return Ok(vec![]) };
    anyhow::ensure!(items.iter().all(|(c, _)| c.account_id == account_id), "queue_many: mixed accounts");
    let now = now_ms();
    let mut rows = Vec::with_capacity(items.len());
    for (c, at) in items {
        validate(&c)?;
        let built = build(ctx, &c).await?;
        let send_at = if at > now { at } else { now };
        rows.push((serde_json::to_string(&c)?, built.raw, built.thread_id, c.source_message_id, c.mode.verb(), c.draft_id, send_at));
    }
    let any_now = rows.iter().any(|r| r.6 <= now);
    let (queued, folder) = ctx
        .db
        .write(move |db| {
            let tx = db.transaction()?;
            let mut out = Vec::with_capacity(rows.len());
            for (compose_json, raw, thread_id, source, verb, draft_id, send_at) in rows {
                tx.execute(
                    "INSERT INTO outbox(account_id, compose_json, raw, thread_id, reply_to_message_id, verb, gmail_draft_id, status,                        attempts, send_at, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,'queued',0,?8,?9)",
                    params![account_id, compose_json, raw, thread_id, source, verb, draft_id, send_at, now],
                )?;
                out.push(Queued { outbox_id: tx.last_insert_rowid(), send_at });
                // the draft leaves Drafts now (Outlook moves it to the Outbox); Gmail's copy goes once sent
                if let Some(d) = &draft_id {
                    store::delete_local_draft(&tx, account_id, d)?;
                }
            }
            let folder = store::refresh_outbox_count(&tx, account_id)?;
            tx.commit()?;
            Ok((out, folder))
        })
        .await?;
    ctx.events.mail_changed(account_id, folder.map(|f| vec![f]));
    ctx.events.emit("mail.outbox", json!({ "accountId": account_id }));
    if any_now {
        ctx.sync_now(Some(account_id));
    } else {
        ctx.wake_sync(account_id);
    }
    Ok(queued)
}

/// Queue a paced batch (mail merge): `items` in order, the first at `start`, then one every
/// `pace_ms`. Every message is validated and the first is built in full (identity, attachments),
/// but the bytes are not kept: each row is built again when it is sent, so memory and the database
/// hold one copy of the attachments, not one per recipient. All rows are inserted in one
/// transaction (all or none). `process` keeps the pace even when rows fall overdue together.
pub async fn queue_batch(ctx: &Arc<AppCtx>, items: Vec<Compose>, start: i64, pace_ms: i64) -> anyhow::Result<usize> {
    let Some(first) = items.first() else { return Ok(0) };
    let account_id = first.account_id;
    anyhow::ensure!(items.iter().all(|c| c.account_id == account_id), "queue_batch: mixed accounts");
    for c in &items {
        validate(c)?;
    }
    let size = build(ctx, first).await?.raw.len() as i64;
    let now = now_ms();
    let start = start.max(now);
    let batch = format!("{now:x}-{:08x}", rand::random::<u32>());
    let rows: Vec<(String, Option<String>)> =
        items.iter().map(|c| Ok((serde_json::to_string(c)?, c.mode.verb().map(str::to_string)))).collect::<anyhow::Result<_>>()?;
    let n = rows.len();
    let folder = ctx
        .db
        .write(move |db| {
            let tx = db.transaction()?;
            for (i, (compose_json, verb)) in rows.into_iter().enumerate() {
                tx.execute(
                    "INSERT INTO outbox(account_id, compose_json, raw, verb, status, attempts, send_at, created_at, batch, pace_ms, size) \
                     VALUES (?1,?2,NULL,?3,'queued',0,?4,?5,?6,?7,?8)",
                    params![account_id, compose_json, verb, start + i as i64 * pace_ms, now, batch, pace_ms, size],
                )?;
            }
            let folder = store::refresh_outbox_count(&tx, account_id)?;
            tx.commit()?;
            Ok(folder)
        })
        .await?;
    ctx.events.mail_changed(account_id, folder.map(|f| vec![f]));
    ctx.events.emit("mail.outbox", json!({ "accountId": account_id }));
    if start <= now {
        ctx.sync_now(Some(account_id));
    } else {
        ctx.wake_sync(account_id);
    }
    Ok(n)
}

/// Retry delay after a failed attempt.
fn backoff_ms(attempts: i64) -> i64 {
    match attempts {
        0 | 1 => 60_000,
        2 => 5 * 60_000,
        _ => 15 * 60_000,
    }
}

struct Due {
    id: i64,
    raw: Vec<u8>,
    thread_id: Option<String>,
    reply_to_message_id: Option<i64>,
    verb: Option<String>,
    gmail_draft_id: Option<String>,
    attempts: i64,
    created_at: i64,
    compose_json: String,
}

/// Send everything due for this account. Returns an error (after trying every item) if any
/// message failed, so the status bar shows it; failed items stay in the Outbox for a retry.
pub async fn process(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let account_id = api.account_id;
    if let Err(e) = super::signatures::maybe_sync_send_as(ctx, api).await {
        log::warn!("send-as sync for account {account_id}: {}", crate::logging::err(&e));
    }
    let now = now_ms();
    let due: Vec<Due> = ctx
        .db
        .write(move |db| {
            // items stuck in 'sending' (app quit mid-send) go back to the queue
            db.execute(
                "UPDATE outbox SET status='failed', last_error='Sending was interrupted.' \
                 WHERE account_id=?1 AND status='sending' AND send_at < ?2",
                params![account_id, now - 10 * 60_000],
            )?;
            // a paced batch (mail merge) with several rows overdue at once — the app was closed or
            // offline — is spread out again from now, so it never goes out in one burst
            db.execute(
                "WITH late AS (SELECT batch FROM outbox WHERE account_id=?1 AND status='queued' AND batch IS NOT NULL \
                   AND send_at<=?2 GROUP BY batch HAVING COUNT(*) > 1), \
                 r AS (SELECT id, pace_ms * (ROW_NUMBER() OVER (PARTITION BY batch ORDER BY send_at, id) - 1) AS delay \
                   FROM outbox WHERE account_id=?1 AND status='queued' AND batch IN (SELECT batch FROM late)) \
                 UPDATE outbox SET send_at = ?2 + (SELECT delay FROM r WHERE r.id = outbox.id) WHERE id IN (SELECT id FROM r)",
                params![account_id, now],
            )?;
            let mut st = db.prepare(
                "SELECT id, raw, thread_id, reply_to_message_id, verb, gmail_draft_id, attempts, created_at, compose_json \
                 FROM outbox WHERE account_id=?1 AND status IN ('queued','failed') AND send_at<=?2 ORDER BY send_at, id",
            )?;
            let v = st
                .query_map(params![account_id, now], |r| {
                    Ok(Due {
                        id: r.get(0)?,
                        raw: r.get::<_, Option<Vec<u8>>>(1)?.unwrap_or_default(),
                        thread_id: r.get(2)?,
                        reply_to_message_id: r.get(3)?,
                        verb: r.get(4)?,
                        gmail_draft_id: r.get(5)?,
                        attempts: r.get(6)?,
                        created_at: r.get(7)?,
                        compose_json: r.get(8)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await?;
    if due.is_empty() {
        return Ok(());
    }
    let total = due.len();
    let mut failures = Vec::new();
    let mut changed_folders = Vec::new();
    for (i, item) in due.into_iter().enumerate() {
        let claimed = ctx
            .db
            .write(move |db| {
                Ok(db.execute(
                    "UPDATE outbox SET status='sending', send_at=?2 WHERE id=?1 AND status IN ('queued','failed')",
                    params![item.id, now_ms()],
                )? == 1)
            })
            .await?;
        if !claimed {
            continue;
        }
        ctx.events.status(account_id, "sync", &format!("Sending {} of {}…", i + 1, total), Some((i as f64 + 0.5) / total as f64));
        let raw = if item.raw.is_empty() {
            // a batch row (mail merge) is built now, from its compose
            let c: Compose = serde_json::from_str(&item.compose_json).unwrap_or_default();
            build(ctx, &c).await.map(|b| b.raw)
        } else if now_ms() - item.created_at > 60_000 {
            // a deferred message is dated when it actually goes out
            Ok(mime::restamp_date(&item.raw, &crate::clock::now_local().fixed_offset()))
        } else {
            Ok(item.raw.clone())
        };
        let result = match raw {
            Ok(raw) => {
                let mut r = api.gmail_send(&raw, item.thread_id.as_deref()).await;
                if item.thread_id.is_some() && matches!(r.as_ref().err().and_then(crate::google::client::api_status), Some(400 | 404)) {
                    // the thread is gone (deleted conversation) or its id is not accepted: send as a new thread
                    r = api.gmail_send(&raw, None).await;
                }
                r
            }
            // e.g. an attachment file was deleted after the merge was queued
            Err(e) => Err(e),
        };
        match result {
            Ok(sent) => {
                log::info!("outbox {}: sent as {} (thread {})", item.id, sent.id, sent.thread_id);
                if let Some(d) = &item.gmail_draft_id
                    && let Err(e) = api.gmail_delete_draft(d).await
                    && crate::google::client::api_status(&e) != Some(404)
                {
                    log::warn!("deleting draft {d} after send: {}", crate::logging::err(&e));
                }
                let compose: Compose = serde_json::from_str(&item.compose_json).unwrap_or_default();
                let draft = item.gmail_draft_id.clone();
                let verb = item.verb.clone();
                let src = item.reply_to_message_id;
                let id = item.id;
                let (outbox_folder, src_account) = ctx
                    .db
                    .write(move |db| {
                        let tx = db.transaction()?;
                        let at = now_ms();
                        let mut src_account = None;
                        if let (Some(src), Some(verb)) = (src, verb.as_deref()) {
                            src_account = set_last_verb(&tx, src, verb, at)?;
                        }
                        if let Some(d) = &draft {
                            store::delete_local_draft(&tx, account_id, d)?;
                        }
                        store::bump_recipients(&tx, compose.all_recipients(), at)?;
                        tx.execute("DELETE FROM outbox WHERE id=?1", [id])?;
                        let f = store::refresh_outbox_count(&tx, account_id)?;
                        tx.commit()?;
                        Ok((f, src_account))
                    })
                    .await?;
                changed_folders.extend(outbox_folder);
                if let Some(a) = src_account.filter(|a| *a != account_id) {
                    ctx.events.mail_changed(a, None);
                }
            }
            Err(e) => {
                // offline (no answer from Google at all): not the message's fault — it stays queued and
                // goes with the next Send/Receive, like Outlook's Outbox
                let offline =
                    matches!(e.downcast_ref::<crate::google::client::ApiError>(), Some(crate::google::client::ApiError::Network(_)));
                let msg = if offline { "Waiting for a network connection.".to_string() } else { short_error(&e) };
                log::warn!("outbox {}: send failed: {}", item.id, crate::logging::err(&e));
                let id = item.id;
                let attempts = if offline { item.attempts } else { item.attempts + 1 };
                let (status, retry_at) = if offline { ("queued", now_ms()) } else { ("failed", now_ms() + backoff_ms(attempts)) };
                let m2 = msg.clone();
                let f = ctx
                    .db
                    .write(move |db| {
                        db.execute(
                            "UPDATE outbox SET status=?5, attempts=?2, last_error=?3, send_at=?4 WHERE id=?1",
                            params![id, attempts, m2, retry_at, status],
                        )?;
                        store::refresh_outbox_count(db, account_id)
                    })
                    .await?;
                changed_folders.extend(f);
                let subject = serde_json::from_str::<Compose>(&item.compose_json).map(|c| c.subject).unwrap_or_default();
                failures.push(if subject.is_empty() { msg } else { format!("\"{subject}\": {msg}") });
                if offline {
                    // the rest would only wait for the same network time-outs
                    break;
                }
            }
        }
    }
    changed_folders.dedup();
    ctx.events.mail_changed(account_id, if changed_folders.is_empty() { None } else { Some(changed_folders) });
    ctx.events.emit("mail.outbox", json!({ "accountId": account_id }));
    if !failures.is_empty() {
        anyhow::bail!("{} message(s) could not be sent and remain in the Outbox — {}", failures.len(), failures.join("; "));
    }
    Ok(())
}

fn short_error(e: &anyhow::Error) -> String {
    let s = match e.downcast_ref::<crate::google::client::ApiError>() {
        Some(crate::google::client::ApiError::Http { status, message, .. }) => format!("{message} ({status})"),
        _ => format!("{e:#}"),
    };
    if s.chars().count() > 200 { format!("{}…", s.chars().take(200).collect::<String>()) } else { s }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxItem {
    pub id: i64,
    pub account_id: i64,
    pub subject: String,
    pub to: Vec<crate::mail::compose::model::Addr>,
    pub from: Option<String>,
    pub status: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub send_at: i64,
    pub created_at: i64,
    pub size: i64,
    pub has_attachments: bool,
    pub importance: crate::mail::compose::model::Importance,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListParams {
    pub account_id: Option<i64>,
}

pub async fn list(ctx: Arc<AppCtx>, p: ListParams) -> anyhow::Result<Vec<OutboxItem>> {
    ctx.db
        .read(move |db| {
            let mut st = db.prepare(
                "SELECT id, account_id, compose_json, status, attempts, last_error, send_at, created_at, COALESCE(length(raw), size) \
                 FROM outbox WHERE (?1 IS NULL OR account_id=?1) ORDER BY created_at DESC, id DESC",
            )?;
            let v = st
                .query_map(params![p.account_id], |r| {
                    let c: Compose = serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default();
                    Ok(OutboxItem {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        subject: c.subject.clone(),
                        to: c.to.iter().chain(c.cc.iter()).cloned().collect(),
                        from: c.from.clone(),
                        status: r.get(3)?,
                        attempts: r.get(4)?,
                        last_error: r.get(5)?,
                        send_at: r.get(6)?,
                        created_at: r.get(7)?,
                        size: r.get::<_, Option<i64>>(8)?.unwrap_or(0),
                        has_attachments: c.attachments.iter().any(|a| a.cid.is_none()),
                        importance: c.importance,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await
}

#[derive(Debug, Deserialize)]
pub struct IdParams {
    pub id: i64,
}

/// "Send again now" for a failed or deferred item.
pub async fn retry(ctx: Arc<AppCtx>, p: IdParams) -> anyhow::Result<()> {
    let account_id: i64 = ctx
        .db
        .write(move |db| {
            let a: Option<i64> = db.query_row("SELECT account_id FROM outbox WHERE id=?1", [p.id], |r| r.get(0)).optional()?;
            let a = a.ok_or_else(|| crate::user_err!("not_found", "That message is no longer in the Outbox."))?;
            db.execute(
                "UPDATE outbox SET status='queued', send_at=?2, last_error=NULL WHERE id=?1 AND status<>'sending'",
                params![p.id, now_ms()],
            )?;
            Ok(a)
        })
        .await?;
    ctx.events.emit("mail.outbox", json!({ "accountId": account_id }));
    ctx.sync_now(Some(account_id));
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cancelled {
    pub draft_id: Option<String>,
    pub message_id: Option<i64>,
}

/// Take a message out of the Outbox and put it back in Drafts (to edit or keep it).
pub async fn cancel(ctx: Arc<AppCtx>, p: IdParams) -> anyhow::Result<Cancelled> {
    let row: Option<(i64, String, String)> = ctx
        .db
        .read(move |db| {
            Ok(db
                .query_row("SELECT account_id, compose_json, status FROM outbox WHERE id=?1", [p.id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .optional()?)
        })
        .await?;
    let (account_id, compose_json, status) =
        row.ok_or_else(|| crate::user_err!("not_found", "That message is no longer in the Outbox."))?;
    if status == "sending" {
        return Err(crate::user_err!("busy", "ClassicMail is already sending this message."));
    }
    let mut compose: Compose = serde_json::from_str(&compose_json)?;
    compose.defer_until = None;
    if compose.mode == Mode::Draft {
        compose.mode = Mode::New;
    }
    let saved = super::drafts::save(&ctx, compose).await?;
    let f = ctx
        .db
        .write(move |db| {
            db.execute("DELETE FROM outbox WHERE id=?1", [p.id])?;
            store::refresh_outbox_count(db, account_id)
        })
        .await?;
    ctx.events.mail_changed(account_id, f.map(|f| vec![f]));
    ctx.events.emit("mail.outbox", json!({ "accountId": account_id }));
    Ok(Cancelled { draft_id: Some(saved.draft_id), message_id: Some(saved.message_id) })
}

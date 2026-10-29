//! Signatures (Tools › Options › Mail Format › Signatures) and Gmail "send as" identities.

use std::sync::Arc;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    ctx::AppCtx,
    db::util::{get_sync_state, now_ms, set_sync_state},
    google::Api,
    rpc::NoParams,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signature {
    pub id: i64,
    pub name: String,
    pub html: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureDefaults {
    pub account_id: i64,
    pub new_signature_id: Option<i64>,
    pub reply_signature_id: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureList {
    pub signatures: Vec<Signature>,
    /// one entry per account (missing defaults are null)
    pub defaults: Vec<SignatureDefaults>,
}

pub async fn list(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<SignatureList> {
    ctx.db
        .read(|c| {
            let mut st = c.prepare("SELECT id, name, html FROM signatures ORDER BY name COLLATE NOCASE, id")?;
            let signatures =
                st.query_map([], |r| Ok(Signature { id: r.get(0)?, name: r.get(1)?, html: r.get(2)? }))?.collect::<Result<Vec<_>, _>>()?;
            let mut st = c.prepare(
                "SELECT a.id, d.new_signature_id, d.reply_signature_id FROM accounts a \
                 LEFT JOIN signature_defaults d ON d.account_id = a.id ORDER BY a.position, a.id",
            )?;
            let defaults = st
                .query_map([], |r| {
                    Ok(SignatureDefaults { account_id: r.get(0)?, new_signature_id: r.get(1)?, reply_signature_id: r.get(2)? })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(SignatureList { signatures, defaults })
        })
        .await
}

#[derive(Debug, Deserialize)]
pub struct SaveParams {
    pub id: Option<i64>,
    pub name: String,
    pub html: String,
}

pub async fn save(ctx: Arc<AppCtx>, p: SaveParams) -> anyhow::Result<Signature> {
    let name = p.name.trim().to_string();
    if name.is_empty() {
        return Err(crate::user_err!("bad_name", "Type a name for this signature."));
    }
    let sig = ctx
        .db
        .write(move |c| {
            let dup: Option<i64> = c
                .query_row("SELECT id FROM signatures WHERE lower(name)=lower(?1) AND id IS NOT ?2", params![name, p.id], |r| r.get(0))
                .optional()?;
            if dup.is_some() {
                return Err(crate::user_err!("duplicate", "A signature with this name already exists."));
            }
            let id = match p.id {
                Some(id) => {
                    let n = c.execute("UPDATE signatures SET name=?1, html=?2 WHERE id=?3", params![name, p.html, id])?;
                    if n == 0 {
                        return Err(crate::user_err!("not_found", "That signature no longer exists."));
                    }
                    id
                }
                None => {
                    c.execute("INSERT INTO signatures(name, html) VALUES (?1, ?2)", params![name, p.html])?;
                    c.last_insert_rowid()
                }
            };
            Ok(Signature { id, name, html: p.html })
        })
        .await?;
    ctx.events.emit("mail.signatures", json!({}));
    Ok(sig)
}

#[derive(Debug, Deserialize)]
pub struct IdParams {
    pub id: i64,
}

pub async fn delete(ctx: Arc<AppCtx>, p: IdParams) -> anyhow::Result<()> {
    ctx.db
        .write(move |c| {
            c.execute("DELETE FROM signatures WHERE id=?1", [p.id])?;
            Ok(())
        })
        .await?;
    ctx.events.emit("mail.signatures", json!({}));
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultsParams {
    pub account_id: i64,
    pub new_signature_id: Option<i64>,
    pub reply_signature_id: Option<i64>,
}

pub async fn set_defaults(ctx: Arc<AppCtx>, p: DefaultsParams) -> anyhow::Result<()> {
    ctx.db
        .write(move |c| {
            c.execute(
                "INSERT INTO signature_defaults(account_id, new_signature_id, reply_signature_id) VALUES (?1,?2,?3) \
                 ON CONFLICT(account_id) DO UPDATE SET new_signature_id=excluded.new_signature_id, \
                   reply_signature_id=excluded.reply_signature_id",
                params![p.account_id, p.new_signature_id, p.reply_signature_id],
            )?;
            Ok(())
        })
        .await?;
    ctx.events.emit("mail.signatures", json!({}));
    Ok(())
}

// ---- send-as -----------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendAsRow {
    pub email: String,
    pub display_name: String,
    pub is_default: bool,
    pub signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendAsParams {
    pub account_id: i64,
}

fn read_send_as(c: &rusqlite::Connection, account_id: i64) -> anyhow::Result<Vec<SendAsRow>> {
    let mut st =
        c.prepare("SELECT email, display_name, is_default, signature FROM send_as WHERE account_id=?1 ORDER BY is_default DESC, email")?;
    let v = st
        .query_map([account_id], |r| {
            Ok(SendAsRow { email: r.get(0)?, display_name: r.get(1)?, is_default: r.get::<_, i64>(2)? != 0, signature: r.get(3)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

pub async fn send_as_list(ctx: Arc<AppCtx>, p: SendAsParams) -> anyhow::Result<Vec<SendAsRow>> {
    let a = p.account_id;
    let rows = ctx.db.read(move |c| read_send_as(c, a)).await?;
    if !rows.is_empty() {
        return Ok(rows);
    }
    // never synced yet: fetch now (best effort)
    if let Ok(api) = ctx.api(a).await
        && let Err(e) = sync_send_as(&ctx, &api).await
    {
        log::warn!("send-as for account {a}: {}", crate::logging::err(&e));
    }
    ctx.db.read(move |c| read_send_as(c, a)).await
}

/// Refresh `send_as` from Gmail settings at most hourly (called from `process_outbox`).
pub async fn maybe_sync_send_as(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let a = api.account_id;
    let last: i64 = ctx.db.read(move |c| get_sync_state(c, a, "compose.sendAsAt")).await?.and_then(|s| s.parse().ok()).unwrap_or(0);
    if now_ms() - last < 3_600_000 {
        return Ok(());
    }
    sync_send_as(ctx, api).await
}

pub async fn sync_send_as(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let list = api.gmail_send_as().await?;
    let a = api.account_id;
    let changed = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let before = read_send_as(&tx, a)?;
            tx.execute("DELETE FROM send_as WHERE account_id=?1", [a])?;
            for s in &list {
                tx.execute(
                    "INSERT OR REPLACE INTO send_as(account_id, email, display_name, is_default, signature) VALUES (?1,?2,?3,?4,?5)",
                    params![a, s.send_as_email, s.display_name, s.is_default, s.signature],
                )?;
            }
            // Gmail signatures seed ours once per account (only if the user has none set up)
            let seeded = get_sync_state(&tx, a, "compose.sigSeeded")?.is_some();
            if !seeded {
                let has_default: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM signature_defaults WHERE account_id=?1 AND \
                           (new_signature_id IS NOT NULL OR reply_signature_id IS NOT NULL))",
                    [a],
                    |r| r.get(0),
                )?;
                if let Some(s) = list.iter().find(|s| s.is_default || s.is_primary).filter(|s| !s.signature.trim().is_empty())
                    && !has_default
                {
                    let name = s.send_as_email.clone();
                    let existing: Option<i64> =
                        tx.query_row("SELECT id FROM signatures WHERE name=?1", [&name], |r| r.get(0)).optional()?;
                    let id = match existing {
                        Some(id) => id,
                        None => {
                            tx.execute("INSERT INTO signatures(name, html) VALUES (?1, ?2)", params![name, s.signature])?;
                            tx.last_insert_rowid()
                        }
                    };
                    tx.execute(
                        "INSERT INTO signature_defaults(account_id, new_signature_id, reply_signature_id) VALUES (?1,?2,?2) \
                             ON CONFLICT(account_id) DO UPDATE SET new_signature_id=excluded.new_signature_id, \
                               reply_signature_id=excluded.reply_signature_id",
                        params![a, id],
                    )?;
                }
                set_sync_state(&tx, a, "compose.sigSeeded", Some("1"))?;
            }
            set_sync_state(&tx, a, "compose.sendAsAt", Some(&now_ms().to_string()))?;
            let after = read_send_as(&tx, a)?;
            tx.commit()?;
            Ok(before.len() != after.len()
                || before.iter().zip(after.iter()).any(|(x, y)| x.email != y.email || x.display_name != y.display_name))
        })
        .await?;
    if changed {
        ctx.events.emit("mail.sendAs", json!({ "accountId": a }));
    }
    Ok(())
}

//! SQL helpers used by compose (all synchronous; call inside `db.read` / `db.write`).

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::model::Addr;

/// A message being replied to / forwarded.
#[derive(Debug, Clone, Default)]
pub struct SourceMsg {
    pub id: i64,
    pub account_id: i64,
    pub gmail_id: String,
    pub thread_id: String,
    pub subject: String,
    pub from: Addr,
    pub to: Vec<Addr>,
    pub cc: Vec<Addr>,
    pub reply_to: Vec<Addr>,
    /// Date: header, else received (ms)
    pub date: i64,
    pub message_id_header: Option<String>,
    pub references_header: Option<String>,
    pub html: Option<String>,
    pub text: Option<String>,
    pub attachments: Vec<AttRow>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttRow {
    pub id: i64,
    pub message_id: i64,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub gmail_attachment_id: Option<String>,
    pub cached_path: Option<String>,
}

pub fn addrs(json: &str) -> Vec<Addr> {
    serde_json::from_str::<Vec<Addr>>(json).unwrap_or_default()
}

/// "Sarah Cohen; bob@x.com" for the To column of the message list.
fn to_display(to: &[Addr]) -> String {
    to.iter().map(|a| if a.name.trim().is_empty() { a.email.as_str() } else { a.name.as_str() }).collect::<Vec<_>>().join("; ")
}

pub fn addrs_json(v: &[Addr]) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

pub fn load_source(c: &Connection, id: i64) -> anyhow::Result<Option<SourceMsg>> {
    let row = c
        .query_row(
            "SELECT id, account_id, gmail_id, thread_id, subject, from_name, from_email, to_json, cc_json, reply_to_json, \
                    COALESCE(sent_date, internal_date), message_id_header, references_header \
             FROM messages WHERE id=?1",
            [id],
            |r| {
                Ok(SourceMsg {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    gmail_id: r.get(2)?,
                    thread_id: r.get(3)?,
                    subject: r.get(4)?,
                    from: Addr { name: r.get(5)?, email: r.get(6)? },
                    to: addrs(&r.get::<_, String>(7)?),
                    cc: addrs(&r.get::<_, String>(8)?),
                    reply_to: addrs(&r.get::<_, String>(9)?),
                    date: r.get(10)?,
                    message_id_header: r.get(11)?,
                    references_header: r.get(12)?,
                    ..Default::default()
                })
            },
        )
        .optional()?;
    let Some(mut m) = row else { return Ok(None) };
    if let Some((html, text)) = c
        .query_row("SELECT html, text FROM message_bodies WHERE message_id=?1", [id], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .optional()?
    {
        m.html = html.filter(|h| !h.trim().is_empty());
        m.text = text;
    }
    m.attachments = attachments(c, id)?;
    Ok(Some(m))
}

pub fn attachments(c: &Connection, message_id: i64) -> anyhow::Result<Vec<AttRow>> {
    let mut st = c.prepare(
        "SELECT id, message_id, filename, mime_type, size, content_id, is_inline, gmail_attachment_id, cached_path \
         FROM attachments WHERE message_id=?1 ORDER BY id",
    )?;
    let v = st.query_map([message_id], att_from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

fn att_from_row(r: &rusqlite::Row) -> rusqlite::Result<AttRow> {
    Ok(AttRow {
        id: r.get(0)?,
        message_id: r.get(1)?,
        filename: r.get(2)?,
        mime_type: r.get(3)?,
        size: r.get(4)?,
        content_id: r.get(5)?,
        is_inline: r.get::<_, i64>(6)? != 0,
        gmail_attachment_id: r.get(7)?,
        cached_path: r.get(8)?,
    })
}

/// Every address that is "me": account addresses + send-as aliases (lower case).
pub fn own_addresses(c: &Connection) -> anyhow::Result<HashSet<String>> {
    let mut set = HashSet::new();
    let mut st = c.prepare("SELECT lower(email) FROM accounts UNION SELECT lower(email) FROM send_as")?;
    for e in st.query_map([], |r| r.get::<_, String>(0))? {
        set.insert(e?);
    }
    Ok(set)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FromChoice {
    pub account_id: i64,
    pub email: String,
    pub name: String,
    /// the account's own (primary) address
    pub primary: bool,
}

/// Accounts + their send-as aliases, for the From ▾ button.
pub fn from_choices(c: &Connection) -> anyhow::Result<Vec<FromChoice>> {
    let mut out = Vec::new();
    let mut st = c.prepare("SELECT id, email, display_name FROM accounts WHERE enabled=1 ORDER BY position, id")?;
    let accts =
        st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
    let mut sa = c.prepare(
        "SELECT email, display_name FROM send_as WHERE account_id=?1 AND lower(email)<>lower(?2) ORDER BY is_default DESC, email",
    )?;
    for (id, email, name) in accts {
        out.push(FromChoice { account_id: id, email: email.clone(), name: name.clone(), primary: true });
        for row in sa.query_map(params![id, email], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (e, n) = row?;
            out.push(FromChoice { account_id: id, email: e, name: if n.is_empty() { name.clone() } else { n }, primary: false });
        }
    }
    Ok(out)
}

/// The From: address for sending as `from` through `account_id` (validated).
pub fn identity(c: &Connection, account_id: i64, from: Option<&str>) -> anyhow::Result<Addr> {
    let (email, name): (String, String) = c
        .query_row("SELECT email, display_name FROM accounts WHERE id=?1", [account_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?
        .ok_or_else(|| crate::user_err!("no_account", "The sending account no longer exists."))?;
    let Some(want) = from.map(str::trim).filter(|f| !f.is_empty() && !f.eq_ignore_ascii_case(&email)) else {
        // the account's default send-as display name wins (Gmail settings), else the account's
        let n: Option<String> = c
            .query_row(
                "SELECT display_name FROM send_as WHERE account_id=?1 AND lower(email)=lower(?2) AND display_name<>''",
                params![account_id, email],
                |r| r.get(0),
            )
            .optional()?;
        return Ok(Addr { name: n.unwrap_or(name), email });
    };
    let alias: Option<(String, String)> = c
        .query_row(
            "SELECT email, display_name FROM send_as WHERE account_id=?1 AND lower(email)=lower(?2)",
            params![account_id, want],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match alias {
        Some((e, n)) => Ok(Addr { name: if n.is_empty() { name } else { n }, email: e }),
        None => Err(crate::user_err!(
            "bad_from",
            "You can't send as {want} from {email}. Add it as a \"Send mail as\" address in Gmail settings first."
        )),
    }
}

pub fn folder_id(c: &Connection, account_id: i64, role: &str) -> anyhow::Result<Option<i64>> {
    Ok(c.query_row("SELECT id FROM folders WHERE account_id=?1 AND role=?2 ORDER BY id LIMIT 1", params![account_id, role], |r| r.get(0))
        .optional()?)
}

/// Keep the local Outbox folder's count in step with the outbox table; returns the folder id.
pub fn refresh_outbox_count(c: &Connection, account_id: i64) -> anyhow::Result<Option<i64>> {
    let Some(fid) = folder_id(c, account_id, "outbox")? else { return Ok(None) };
    // Outlook shows the Outbox count in bold like unread mail (number of items waiting)
    c.execute(
        "UPDATE folders SET total_count=(SELECT COUNT(*) FROM outbox WHERE account_id=?1), \
                            unread_count=(SELECT COUNT(*) FROM outbox WHERE account_id=?1) WHERE id=?2",
        params![account_id, fid],
    )?;
    Ok(Some(fid))
}

/// Update Outlook's auto-complete list after sending.
pub fn bump_recipients<'a>(c: &Connection, addrs: impl Iterator<Item = &'a Addr>, now: i64) -> anyhow::Result<()> {
    let mut st = c.prepare(
        "INSERT INTO recipient_cache(email, name, use_count, last_used) VALUES (lower(?1), ?2, 1, ?3) \
         ON CONFLICT(email) DO UPDATE SET use_count=use_count+1, last_used=excluded.last_used, \
           name=CASE WHEN excluded.name<>'' THEN excluded.name ELSE name END",
    )?;
    for a in addrs {
        if super::model::valid_email(&a.email) {
            st.execute(params![a.email.trim(), a.name.trim(), now])?;
        }
    }
    Ok(())
}

/// A draft row so the message shows in Drafts right away (mail sync later refreshes it).
pub struct LocalDraft<'a> {
    pub account_id: i64,
    pub gmail_id: &'a str,
    pub draft_id: &'a str,
    pub thread_id: &'a str,
    pub from: &'a Addr,
    pub compose: &'a super::model::Compose,
    pub html: &'a str,
    pub text: &'a str,
    pub message_id_header: &'a str,
    pub in_reply_to: Option<&'a str>,
    pub references: Option<&'a str>,
    pub size: i64,
    pub attachments: &'a [(String, String, i64)],
    pub now: i64,
}

pub fn insert_local_draft(c: &Connection, d: &LocalDraft) -> anyhow::Result<i64> {
    let snippet: String = d.text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect();
    let importance = match d.compose.importance {
        super::model::Importance::Low => 0,
        super::model::Importance::Normal => 1,
        super::model::Importance::High => 2,
    };
    c.execute(
        "INSERT INTO messages(account_id, gmail_id, thread_id, internal_date, sent_date, subject, from_name, from_email, \
            to_json, cc_json, bcc_json, reply_to_json, snippet, label_ids, is_read, is_draft, gmail_draft_id, has_attachments, \
            importance, size, message_id_header, in_reply_to, references_header, body_state, subject_norm, to_display) \
         VALUES (?1,?2,?3,?4,?4,?5,?6,?7,?8,?9,?10,?11,?12,'DRAFT',1,1,?13,?14,?15,?16,?17,?18,?19,1,?20,?21) \
         ON CONFLICT(account_id, gmail_id) DO UPDATE SET subject=excluded.subject, to_json=excluded.to_json, \
            subject_norm=excluded.subject_norm, to_display=excluded.to_display, \
            cc_json=excluded.cc_json, bcc_json=excluded.bcc_json, snippet=excluded.snippet, gmail_draft_id=excluded.gmail_draft_id, \
            internal_date=excluded.internal_date, is_draft=1, has_attachments=excluded.has_attachments, body_state=1",
        params![
            d.account_id,
            d.gmail_id,
            d.thread_id,
            d.now,
            d.compose.subject,
            d.from.name,
            d.from.email,
            addrs_json(&d.compose.to),
            addrs_json(&d.compose.cc),
            addrs_json(&d.compose.bcc),
            addrs_json(&d.compose.reply_to),
            snippet,
            d.draft_id,
            !d.attachments.is_empty(),
            importance,
            d.size,
            format!("<{}>", d.message_id_header),
            d.in_reply_to,
            d.references,
            crate::mail::mime::normalize_subject(&d.compose.subject),
            to_display(&d.compose.to),
        ],
    )?;
    let id: i64 =
        c.query_row("SELECT id FROM messages WHERE account_id=?1 AND gmail_id=?2", params![d.account_id, d.gmail_id], |r| r.get(0))?;
    c.execute(
        "INSERT INTO message_bodies(message_id, html, text, fetched_at) VALUES (?1,?2,?3,?4) \
         ON CONFLICT(message_id) DO UPDATE SET html=excluded.html, text=excluded.text, fetched_at=excluded.fetched_at",
        params![id, d.html, d.text, d.now],
    )?;
    // Instant Search finds drafts too
    let rcpts = d.compose.to.iter().chain(&d.compose.cc).map(|a| format!("{} {}", a.name, a.email)).collect::<Vec<_>>().join(" ");
    crate::mail::store::fts_put(c, id, &d.compose.subject, &format!("{} {}", d.from.name, d.from.email), &rcpts, d.text)?;
    c.execute("DELETE FROM attachments WHERE message_id=?1", [id])?;
    for (name, mime, size) in d.attachments {
        c.execute("INSERT INTO attachments(message_id, filename, mime_type, size) VALUES (?1,?2,?3,?4)", params![id, name, mime, size])?;
    }
    if let Some(fid) = folder_id(c, d.account_id, "drafts")? {
        c.execute(
            "INSERT INTO message_folders(message_id, folder_id, msg_date) VALUES (?1,?2,?3) ON CONFLICT DO UPDATE SET msg_date=excluded.msg_date",
            params![id, fid, d.now],
        )?;
        c.execute("UPDATE folders SET total_count=(SELECT COUNT(*) FROM message_folders WHERE folder_id=?1) WHERE id=?1", [fid])?;
    }
    Ok(id)
}

/// Remove local rows of a Gmail draft (after it was replaced, sent or discarded).
pub fn delete_local_draft(c: &Connection, account_id: i64, draft_id: &str) -> anyhow::Result<usize> {
    let ids: Vec<i64> = {
        let mut st = c.prepare("SELECT id FROM messages WHERE account_id=?1 AND gmail_draft_id=?2")?;
        st.query_map(params![account_id, draft_id], |r| r.get(0))?.collect::<Result<_, _>>()?
    };
    // through the mail store, so the full-text index rows go too
    crate::mail::store::delete_messages(c, &ids)?;
    let n = ids.len();
    if n > 0
        && let Some(fid) = folder_id(c, account_id, "drafts")?
    {
        c.execute("UPDATE folders SET total_count=(SELECT COUNT(*) FROM message_folders WHERE folder_id=?1) WHERE id=?1", [fid])?;
    }
    Ok(n)
}

pub fn table_exists(c: &Connection, name: &str) -> bool {
    c.query_row("SELECT 1 FROM sqlite_master WHERE type IN ('table','view') AND name=?1", [name], |_| Ok(()))
        .optional()
        .ok()
        .flatten()
        .is_some()
}

pub fn columns(c: &Connection, table: &str) -> Vec<String> {
    let Ok(mut st) = c.prepare(&format!("PRAGMA table_info(\"{}\")", table.replace('"', ""))) else { return vec![] };
    st.query_map([], |r| r.get::<_, String>(1)).map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
}

//! Running rules over stored messages: load message facts from the mail tables, plan with the pure
//! matcher (`model::plan`), and execute actions through mail's rpc methods (so moves, flags etc.
//! go through mail's local-first logic). Everything mail-specific goes through `MailPort`, so the
//! engine can be tested without the mail module.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};

use super::model::{Action, MsgFacts, Person, Plan, Rule, plan};
use crate::{
    ctx::AppCtx,
    mail::compose::prepare::{esc, outlook_date},
    rpc::Registry,
};

/// The mail operations rules need. Production: `RegistryPort`; tests: a recorder.
#[async_trait::async_trait]
pub trait MailPort: Send + Sync {
    /// Call an rpc method (`mail.move`, `mail.markRead`, ...).
    async fn call(&self, method: &str, params: Value) -> anyhow::Result<Value>;
    /// Make sure `message_bodies` is filled for a message (fetches from Gmail if needed).
    async fn ensure_body(&self, message_id: i64) -> anyhow::Result<()>;
}

/// Calls mail through the shared rpc registry (built once).
pub struct RegistryPort(pub Arc<AppCtx>);

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(crate::rpc::build)
}

#[async_trait::async_trait]
impl MailPort for RegistryPort {
    async fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        registry().call(self.0.clone(), method, params).await
    }
    async fn ensure_body(&self, message_id: i64) -> anyhow::Result<()> {
        crate::mail::ensure_body(&self.0, message_id).await
    }
}

// ---- loading facts --------------------------------------------------------------------------

fn people(json_text: &str) -> Vec<Person> {
    serde_json::from_str(json_text).unwrap_or_default()
}

/// The mailbox owner's addresses for each account: the account e-mail plus Gmail send-as aliases.
pub fn my_addresses(c: &Connection) -> anyhow::Result<HashMap<i64, Vec<String>>> {
    let mut out: HashMap<i64, Vec<String>> = HashMap::new();
    let mut st = c.prepare("SELECT id, email FROM accounts")?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
        let (id, e) = r?;
        out.entry(id).or_default().push(e.to_lowercase());
    }
    let mut st = c.prepare("SELECT account_id, email FROM send_as")?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
        let (id, e) = r?;
        let v = out.entry(id).or_default();
        let e = e.to_lowercase();
        if !v.contains(&e) {
            v.push(e);
        }
    }
    Ok(out)
}

/// Header-level facts for messages (no body). Missing ids are skipped.
pub fn load_facts(c: &Connection, ids: &[i64]) -> anyhow::Result<Vec<MsgFacts>> {
    let me = my_addresses(c)?;
    let mut st = c.prepare(
        "SELECT id, account_id, from_name, from_email, to_json, cc_json, subject, has_attachments, importance \
         FROM messages WHERE id=?1",
    )?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let row = st
            .query_row([id], |r| {
                Ok(MsgFacts {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    from_name: r.get(2)?,
                    from_email: r.get(3)?,
                    to: people(&r.get::<_, String>(4)?),
                    cc: people(&r.get::<_, String>(5)?),
                    subject: r.get(6)?,
                    has_attachments: r.get::<_, i64>(7)? != 0,
                    importance: r.get(8)?,
                    ..Default::default()
                })
            })
            .optional()?;
        if let Some(mut f) = row {
            f.me = me.get(&f.account_id).cloned().unwrap_or_default();
            out.push(f);
        }
    }
    Ok(out)
}

/// Body text + header lines from `message_bodies`, if fetched.
pub fn load_body(c: &Connection, id: i64) -> anyhow::Result<Option<(String, Option<String>)>> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> = c
        .query_row("SELECT text, html, headers_json FROM message_bodies WHERE message_id=?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let Some((text, html, headers)) = row else { return Ok(None) };
    let text = match text {
        Some(t) if !t.trim().is_empty() => t,
        _ => html.map(|h| crate::mail::mime::html_to_text(&h)).unwrap_or_default(),
    };
    let headers = headers.and_then(|h| serde_json::from_str::<Vec<Value>>(&h).ok()).map(|hs| {
        hs.iter()
            .map(|h| format!("{}: {}", h["name"].as_str().unwrap_or(""), h["value"].as_str().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("\n")
    });
    Ok(Some((text, headers)))
}

fn synth_headers(f: &MsgFacts) -> String {
    let addrs = |v: &[Person]| v.iter().map(|p| format!("{} <{}>", p.name, p.email)).collect::<Vec<_>>().join(", ");
    format!("From: {} <{}>\nTo: {}\nCc: {}\nSubject: {}", f.from_name, f.from_email, addrs(&f.to), addrs(&f.cc), f.subject)
}

/// Fill body + headers, fetching the body if needed. Failures leave the body empty (logged).
pub async fn fill_body(ctx: &Arc<AppCtx>, port: &dyn MailPort, f: &mut MsgFacts) {
    let id = f.id;
    let mut got = ctx.db.read(move |c| load_body(c, id)).await.ok().flatten();
    if got.is_none() {
        match port.ensure_body(id).await {
            Ok(()) => got = ctx.db.read(move |c| load_body(c, id)).await.ok().flatten(),
            Err(e) => log::warn!("rules: no body for message {id} (treating as empty): {}", crate::logging::err(&e)),
        }
    }
    let (text, headers) = got.unwrap_or_default();
    f.body = text;
    f.headers = headers.unwrap_or_else(|| synth_headers(f));
}

// ---- running --------------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunStats {
    /// messages examined
    pub processed: usize,
    /// messages at least one rule matched
    pub matched: usize,
    /// actions carried out successfully
    pub actions: usize,
    /// actions that failed (details in `errors`)
    pub failed: usize,
    pub errors: Vec<String>,
}

/// Apply `rules` (ordered; disabled ones are skipped) to messages. Never fails as a whole:
/// per-message problems are counted and logged.
pub async fn apply(
    ctx: &Arc<AppCtx>,
    port: &dyn MailPort,
    rules: &[Rule],
    ids: &[i64],
    status_account: Option<i64>,
) -> anyhow::Result<RunStats> {
    let mut stats = RunStats::default();
    let rules: Vec<Rule> = rules.iter().filter(|r| r.enabled).cloned().collect();
    if rules.is_empty() || ids.is_empty() {
        return Ok(stats);
    }
    let ids_v = ids.to_vec();
    let facts = ctx.db.read(move |c| load_facts(c, &ids_v)).await?;
    let total = facts.len();
    let mut alerts = Vec::new();
    for (i, mut f) in facts.into_iter().enumerate() {
        if let Some(acct) = status_account
            && i % 25 == 0
            && total > 50
        {
            ctx.events.status(acct, "rules", &format!("Running rules: {} of {}", i, total), Some(i as f64 / total as f64));
        }
        stats.processed += 1;
        if rules.iter().any(|r| r.applies_to_account(f.account_id) && r.needs_body()) {
            fill_body(ctx, port, &mut f).await;
        }
        // `plan` skips rules for other mailboxes itself
        let p = plan(&rules, &f);
        if p.matched.is_empty() {
            continue;
        }
        stats.matched += 1;
        // rule ids, not names (a name can be a person or a subject)
        log::info!("rules: message {} matched rules {:?} → {} action(s)", f.id, p.matched, p.actions.len());
        execute(ctx, port, &f, &p, &mut stats, &mut alerts).await;
    }
    if let Some(acct) = status_account
        && total > 50
    {
        ctx.events.status(acct, "idle", "", None);
    }
    if !alerts.is_empty() {
        desktop_alert(ctx, alerts);
    }
    Ok(stats)
}

/// Resolve a folder for this message's account: the stored id if it belongs to the account,
/// otherwise the folder with the same path in the message's account (rules for all mailboxes).
fn resolve_folder(c: &Connection, account_id: i64, folder_id: i64, path: &str) -> anyhow::Result<Option<i64>> {
    let owner: Option<(i64, String)> =
        c.query_row("SELECT account_id, path FROM folders WHERE id=?1", [folder_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let path = match &owner {
        Some((a, _)) if *a == account_id => return Ok(Some(folder_id)),
        Some((_, p)) => p.clone(),
        None => path.to_string(),
    };
    if path.is_empty() {
        return Ok(None);
    }
    Ok(c.query_row("SELECT id FROM folders WHERE account_id=?1 AND path=?2", params![account_id, path], |r| r.get(0)).optional()?)
}

async fn folder_for(ctx: &Arc<AppCtx>, f: &MsgFacts, folder_id: i64, path: &str) -> anyhow::Result<i64> {
    let (acct, path_s) = (f.account_id, path.to_string());
    let found = ctx.db.read(move |c| resolve_folder(c, acct, folder_id, &path_s)).await?;
    // Unknown folder ids are passed through (mail decides) when nothing better is known.
    Ok(found.unwrap_or(folder_id))
}

async fn execute(ctx: &Arc<AppCtx>, port: &dyn MailPort, f: &MsgFacts, p: &Plan, stats: &mut RunStats, alerts: &mut Vec<Value>) {
    let ids = json!([f.id]);
    for a in &p.actions {
        let res: anyhow::Result<()> = async {
            match a {
                Action::CopyToFolder { folder_id, folder_path } => {
                    let fid = folder_for(ctx, f, *folder_id, folder_path).await?;
                    port.call("mail.copy", json!({ "ids": ids, "folderId": fid })).await?;
                }
                Action::MoveToFolder { folder_id, folder_path } => {
                    let fid = folder_for(ctx, f, *folder_id, folder_path).await?;
                    port.call("mail.move", json!({ "ids": ids, "folderId": fid })).await?;
                }
                Action::Delete => {
                    port.call("mail.delete", json!({ "ids": ids })).await?;
                }
                Action::MarkRead => {
                    port.call("mail.markRead", json!({ "ids": ids, "read": true })).await?;
                }
                Action::Flag { flag } => {
                    port.call("mail.setFlag", json!({ "ids": ids, "flag": flag })).await?;
                }
                Action::SetImportance { level } => {
                    // Gmail has no importance field, so this is a local-only change.
                    let (id, v) = (f.id, level.as_i64());
                    ctx.db.write(move |c| Ok(c.execute("UPDATE messages SET importance=?1 WHERE id=?2", params![v, id])?)).await?;
                    ctx.events.mail_changed(f.account_id, None);
                }
                Action::ForwardTo { people } => forward(ctx, port, f, people).await?,
                Action::DisplayDesktopAlert => {
                    alerts.push(json!({
                        "id": f.id, "accountId": f.account_id, "from": if f.from_name.is_empty() { &f.from_email } else { &f.from_name },
                        "subject": f.subject, "preview": f.body.chars().take(140).collect::<String>(),
                        "rules": p.matched_names,
                    }));
                }
                Action::PlaySound { sound } => {
                    ctx.events.emit("rules.playSound", json!({ "sound": sound, "messageId": f.id }));
                }
                Action::StopProcessing => {}
            }
            Ok(())
        }
        .await;
        match res {
            Ok(()) => stats.actions += 1,
            Err(e) => {
                stats.failed += 1;
                log::warn!("rules: message {}: {:?}: {}", f.id, action_name(a), crate::logging::err(&e));
                let msg = format!("message {}: {:?}: {e:#}", f.id, action_name(a));
                if stats.errors.len() < 20 {
                    stats.errors.push(msg);
                }
            }
        }
    }
}

fn action_name(a: &Action) -> &'static str {
    match a {
        Action::MoveToFolder { .. } => "move",
        Action::CopyToFolder { .. } => "copy",
        Action::Delete => "delete",
        Action::MarkRead => "markRead",
        Action::Flag { .. } => "flag",
        Action::SetImportance { .. } => "setImportance",
        Action::ForwardTo { .. } => "forward",
        Action::DisplayDesktopAlert => "alert",
        Action::PlaySound { .. } => "sound",
        Action::StopProcessing => "stop",
    }
}

/// Forward through mail's compose/send path (Outbox), Outlook-style "FW:" with the original header block.
async fn forward(ctx: &Arc<AppCtx>, port: &dyn MailPort, f: &MsgFacts, to: &[Person]) -> anyhow::Result<()> {
    if f.me.iter().any(|m| m.eq_ignore_ascii_case(f.from_email.trim())) {
        // never forward our own mail (avoids loops between mailboxes)
        return Ok(());
    }
    let _ = port.ensure_body(f.id).await;
    let id = f.id;
    let (html, text, sent, atts) = ctx
        .db
        .read(move |c| {
            let body: Option<(Option<String>, Option<String>)> = c
                .query_row("SELECT html, text FROM message_bodies WHERE message_id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            let sent: i64 = c.query_row("SELECT COALESCE(sent_date, internal_date) FROM messages WHERE id=?1", [id], |r| r.get(0))?;
            let mut st = c.prepare("SELECT id FROM attachments WHERE message_id=?1 AND is_inline=0")?;
            let atts = st.query_map([id], |r| r.get::<_, i64>(0))?.collect::<Result<Vec<_>, _>>()?;
            let (html, text) = body.unwrap_or((None, None));
            Ok((html, text, sent, atts))
        })
        .await?;
    let original = match (html, text) {
        (Some(h), _) if !h.trim().is_empty() => h,
        (_, Some(t)) => format!("<pre style=\"font-family:inherit;white-space:pre-wrap\">{}</pre>", esc(&t)),
        _ => String::new(),
    };
    // the same "Sent:" line as a forward from the compose window (this computer's time zone)
    let when = outlook_date(sent);
    let list =
        |v: &[Person]| v.iter().map(|p| if p.name.is_empty() { p.email.clone() } else { p.name.clone() }).collect::<Vec<_>>().join("; ");
    let from = if f.from_name.is_empty() { f.from_email.clone() } else { format!("{} [mailto:{}]", f.from_name, f.from_email) };
    let mut header = format!(
        "<div><br></div><div style=\"border:none;border-top:solid #B5C4DF 1.0pt;padding:3.0pt 0 0 0\">\
         <b>From:</b> {}<br><b>Sent:</b> {}<br><b>To:</b> {}<br>",
        esc(&from),
        esc(&when),
        esc(&list(&f.to))
    );
    if !f.cc.is_empty() {
        header.push_str(&format!("<b>Cc:</b> {}<br>", esc(&list(&f.cc))));
    }
    header.push_str(&format!("<b>Subject:</b> {}</div><br>", esc(&f.subject)));
    let subject = if f.subject.to_lowercase().starts_with("fw:") { f.subject.clone() } else { format!("FW: {}", f.subject) };
    let compose = json!({
        "accountId": f.account_id,
        "to": to.iter().map(|p| json!({ "name": p.name, "email": p.email })).collect::<Vec<_>>(),
        "cc": [], "bcc": [],
        "subject": subject,
        "html": format!("{header}{original}"),
        "attachments": atts.iter().map(|a| json!({ "attachmentId": a })).collect::<Vec<_>>(),
    });
    port.call("mail.compose.send", json!({ "compose": compose })).await?;
    Ok(())
}

/// "Display a Desktop Alert": emits `rules.alert`; the main window shows it in the Outlook-style
/// desktop alert window (even when the regular new-mail alert is turned off).
fn desktop_alert(ctx: &Arc<AppCtx>, messages: Vec<Value>) {
    ctx.events.emit("rules.alert", json!({ "messages": messages }));
}

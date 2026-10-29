//! Local-first message actions (design §3): change SQLite immediately, queue a `pending_ops` row,
//! emit `mail.changed`, and push to Gmail in the background.

use std::{collections::HashMap, sync::Arc};

use rusqlite::{Connection, params};
use serde_json::json;

use super::{
    store::{self, LabelMap},
    sync::{IdsOp, ModifyOp, push_soon},
};
use crate::{ctx::AppCtx, sync::pending};

#[derive(Debug, Clone)]
pub struct MsgInfo {
    pub id: i64,
    pub account_id: i64,
    pub gmail_id: String,
    pub labels: Vec<String>,
    pub location: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Modify { add: Vec<String>, remove: Vec<String> },
    Trash,
    Untrash,
    Delete,
}

pub fn load_infos(c: &Connection, ids: &[i64]) -> anyhow::Result<Vec<MsgInfo>> {
    let mut st = c.prepare_cached("SELECT id, account_id, gmail_id, label_ids, location FROM messages WHERE id=?1")?;
    let mut out = Vec::new();
    for id in ids {
        if let Ok(i) = st.query_row([id], |r| {
            Ok(MsgInfo {
                id: r.get(0)?,
                account_id: r.get(1)?,
                gmail_id: r.get(2)?,
                labels: r.get::<_, String>(3)?.split_whitespace().map(str::to_string).collect(),
                location: r.get(4)?,
            })
        }) {
            out.push(i);
        }
    }
    Ok(out)
}

/// Apply per-message plans locally and queue them for Gmail. Returns the touched account ids.
pub fn execute(
    c: &mut Connection,
    infos: &[MsgInfo],
    plan: &dyn Fn(&MsgInfo, &LabelMap) -> anyhow::Result<Vec<Step>>,
) -> anyhow::Result<Vec<i64>> {
    let tx = c.transaction()?;
    let mut maps: HashMap<i64, LabelMap> = HashMap::new();
    // queued ops per account, in order; modify ops grouped by (add, remove)
    let mut queue: HashMap<i64, Vec<(String, serde_json::Value)>> = HashMap::new();
    let mut modify_groups: HashMap<(i64, Vec<String>, Vec<String>, usize), Vec<String>> = HashMap::new();
    let mut accounts = Vec::new();
    for info in infos {
        if let std::collections::hash_map::Entry::Vacant(e) = maps.entry(info.account_id) {
            e.insert(LabelMap::load(&tx, info.account_id)?);
        }
        let map = &maps[&info.account_id];
        let steps = plan(info, map)?;
        if steps.is_empty() {
            continue;
        }
        if !accounts.contains(&info.account_id) {
            accounts.push(info.account_id);
        }
        let mut labels = info.labels.clone();
        for (si, step) in steps.iter().enumerate() {
            match step {
                Step::Modify { add, remove } => {
                    let before = labels.clone();
                    labels.retain(|l| !remove.contains(l));
                    for a in add {
                        if !labels.contains(a) {
                            labels.push(a.clone());
                        }
                    }
                    if labels != before {
                        modify_groups.entry((info.account_id, add.clone(), remove.clone(), si)).or_default().push(info.gmail_id.clone());
                    }
                }
                Step::Trash => {
                    labels.retain(|l| l != "SPAM");
                    if !labels.iter().any(|l| l == "TRASH") {
                        labels.push("TRASH".into());
                    }
                    queue.entry(info.account_id).or_default().push(("trash".into(), json!([info.gmail_id])));
                }
                Step::Untrash => {
                    labels.retain(|l| l != "TRASH");
                    queue.entry(info.account_id).or_default().push(("untrash".into(), json!([info.gmail_id])));
                }
                Step::Delete => {
                    queue.entry(info.account_id).or_default().push(("delete".into(), json!([info.gmail_id])));
                }
            }
        }
        if steps.contains(&Step::Delete) {
            store::delete_messages(&tx, &[info.id])?;
        } else {
            store::apply_labels(&tx, info.id, &labels, map)?;
        }
    }
    // untrash before modifies (a move out of Deleted Items), trash/delete after
    for (acct, ops) in &queue {
        let mut by_kind: HashMap<&str, Vec<String>> = HashMap::new();
        for (k, v) in ops {
            by_kind.entry(k.as_str()).or_default().extend(v.as_array().unwrap().iter().filter_map(|x| x.as_str().map(str::to_string)));
        }
        if let Some(ids) = by_kind.get("untrash") {
            pending::enqueue(&tx, *acct, "mail", "untrash", &IdsOp { ids: ids.clone() })?;
        }
    }
    let mut groups: Vec<_> = modify_groups.into_iter().collect();
    groups.sort_by_key(|g| g.0.3);
    for ((acct, add, remove, _), ids) in groups {
        for chunk in ids.chunks(1000) {
            pending::enqueue(&tx, acct, "mail", "modify", &ModifyOp { ids: chunk.to_vec(), add: add.clone(), remove: remove.clone() })?;
        }
    }
    for (acct, ops) in &queue {
        for kind in ["trash", "delete"] {
            let ids: Vec<String> = ops
                .iter()
                .filter(|(k, _)| k == kind)
                .flat_map(|(_, v)| v.as_array().unwrap().iter().filter_map(|x| x.as_str().map(str::to_string)).collect::<Vec<_>>())
                .collect();
            if !ids.is_empty() {
                pending::enqueue(&tx, *acct, "mail", kind, &IdsOp { ids })?;
            }
        }
    }
    for a in &accounts {
        store::recount(&tx, *a)?;
    }
    tx.commit()?;
    Ok(accounts)
}

/// Run `plan` over messages `ids`, then notify and push.
pub async fn run<F>(ctx: &Arc<AppCtx>, ids: Vec<i64>, plan: F) -> anyhow::Result<Vec<i64>>
where
    F: Fn(&MsgInfo, &LabelMap) -> anyhow::Result<Vec<Step>> + Send + 'static,
{
    let accounts = ctx
        .db
        .write(move |c| {
            let infos = load_infos(c, &ids)?;
            execute(c, &infos, &plan)
        })
        .await?;
    after(ctx, &accounts);
    Ok(accounts)
}

pub fn after(ctx: &Arc<AppCtx>, accounts: &[i64]) {
    for a in accounts {
        ctx.events.mail_changed(*a, None);
        push_soon(ctx, *a);
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

pub async fn mark_read(ctx: &Arc<AppCtx>, ids: Vec<i64>, read: bool) -> anyhow::Result<()> {
    run(ctx, ids, move |i, _| {
        let unread = i.labels.iter().any(|l| l == "UNREAD");
        Ok(match (read, unread) {
            (true, true) => vec![Step::Modify { add: vec![], remove: s(&["UNREAD"]) }],
            (false, false) => vec![Step::Modify { add: s(&["UNREAD"]), remove: vec![] }],
            _ => vec![],
        })
    })
    .await?;
    Ok(())
}

/// Due date for an Outlook quick flag (local date). Work week ends on Friday.
pub fn flag_due(flag: &str, custom: Option<&str>, today: chrono::NaiveDate) -> Option<String> {
    use chrono::{Datelike, Duration};
    let fri_offset = (5 + 7 - today.weekday().num_days_from_sunday() as i64) % 7; // days until Friday
    let d = match flag {
        "today" => today,
        "tomorrow" => today + Duration::days(1),
        "thisWeek" => today + Duration::days(fri_offset.max(if today.weekday().num_days_from_sunday() == 6 { 6 } else { 0 })),
        "nextWeek" => today + Duration::days(fri_offset + 7),
        "custom" => return custom.map(str::to_string),
        _ => return None,
    };
    Some(d.format("%Y-%m-%d").to_string())
}

pub async fn set_flag(ctx: &Arc<AppCtx>, ids: Vec<i64>, flag: String, due: Option<String>) -> anyhow::Result<()> {
    let flag2 = flag.clone();
    let accounts = ctx
        .db
        .write(move |c| {
            let infos = load_infos(c, &ids)?;
            let flagging = !matches!(flag2.as_str(), "none" | "complete");
            let accounts = execute(c, &infos, &|i, _| {
                let starred = i.labels.iter().any(|l| l == "STARRED");
                Ok(if flagging && !starred {
                    vec![Step::Modify { add: s(&["STARRED"]), remove: vec![] }]
                } else if !flagging && starred {
                    vec![Step::Modify { add: vec![], remove: s(&["STARRED"]) }]
                } else {
                    vec![]
                })
            })?;
            let today = crate::clock::today();
            let due = flag_due(&flag2, due.as_deref(), today);
            for i in &infos {
                match flag2.as_str() {
                    "none" => {
                        c.execute("UPDATE messages SET flag_status=NULL, flag_due=NULL, flag_completed_at=NULL WHERE id=?1", [i.id])?
                    }
                    "complete" => c.execute(
                        "UPDATE messages SET flag_status='complete', flag_completed_at=?1 WHERE id=?2",
                        params![crate::db::util::now_ms(), i.id],
                    )?,
                    _ => c.execute(
                        "UPDATE messages SET flag_status='flagged', flag_due=?1, flag_completed_at=NULL WHERE id=?2",
                        params![due, i.id],
                    )?,
                };
            }
            let mut accts = accounts;
            for i in &infos {
                if !accts.contains(&i.account_id) {
                    accts.push(i.account_id);
                }
            }
            Ok(accts)
        })
        .await?;
    after(ctx, &accounts);
    Ok(())
}

/// Archive (Backspace), Gmail style: take the messages out of the folder being viewed — the Inbox
/// or a label — so they live on in All Mail only. From any other view (All Mail, search results,
/// Sent Items…) the Inbox label is removed. Messages in Deleted Items / Junk are left alone.
pub async fn archive(ctx: &Arc<AppCtx>, ids: Vec<i64>, from: Option<i64>) -> anyhow::Result<()> {
    let source = match from {
        Some(f) => ctx.db.read(move |c| store::folder(c, f)).await.ok(),
        None => None,
    };
    let label = match &source {
        Some(f) if f.role == store::ROLE_INBOX || f.role == store::ROLE_USER => f.gmail_label_id.clone().unwrap_or_else(|| "INBOX".into()),
        _ => "INBOX".to_string(),
    };
    run(ctx, ids, move |i, _| {
        if i.location != 0 || !i.labels.contains(&label) {
            return Ok(vec![]);
        }
        Ok(vec![Step::Modify { add: vec![], remove: vec![label.clone()] }])
    })
    .await?;
    Ok(())
}

/// Labels that make up "the folder(s) a message is in" for a move.
fn folder_labels(i: &MsgInfo, map: &LabelMap) -> Vec<String> {
    i.labels.iter().filter(|l| map.by_label.contains_key(*l) && !matches!(l.as_str(), "SENT" | "DRAFT" | "TRASH")).cloned().collect()
}

/// What a move / copy did with messages from other mailboxes (`transfer`): the copies made, and
/// the messages that couldn't be copied (their originals are left as they were).
#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transferred {
    pub imported: Vec<Imported>,
    pub failed: Vec<Failed>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub account_id: i64,
    pub gmail_id: String,
}

#[derive(Debug, serde::Serialize)]
pub struct Failed {
    pub id: i64,
    pub message: String,
}

/// Move (or copy) to a folder, Gmail label semantics. `from` = the folder the user is looking at.
/// Messages from another mailbox are copied there (`transfer`), as Outlook does between mailboxes.
pub async fn move_to(ctx: &Arc<AppCtx>, ids: Vec<i64>, folder_id: i64, from: Option<i64>, copy: bool) -> anyhow::Result<Transferred> {
    let target = ctx.db.read(move |c| store::folder(c, folder_id)).await?;
    let source = match from {
        Some(f) => ctx.db.read(move |c| store::folder(c, f)).await.ok(),
        None => None,
    };
    if matches!(target.role.as_str(), store::ROLE_DRAFTS | store::ROLE_OUTBOX | store::ROLE_PLACEHOLDER) {
        return Err(crate::user_err!("bad_target", "Items can't be moved to the folder \"{}\".", target.name));
    }
    if source.as_ref().map(|s| s.id) == Some(target.id) {
        return Ok(Transferred::default());
    }
    let all = ids.clone();
    let (here, elsewhere): (Vec<MsgInfo>, Vec<MsgInfo>) =
        ctx.db.read(move |c| load_infos(c, &all)).await?.into_iter().partition(|i| i.account_id == target.account_id);
    let transferred = if elsewhere.is_empty() { Transferred::default() } else { transfer(ctx, elsewhere, &target, copy).await? };
    let ids: Vec<i64> = here.iter().map(|i| i.id).collect();
    if ids.is_empty() {
        return Ok(transferred);
    }
    run(ctx, ids, move |i, map| {
        let mut steps = Vec::new();
        let in_trash = i.location == 1;
        match target.role.as_str() {
            store::ROLE_TRASH => {
                if !in_trash {
                    steps.push(Step::Trash);
                }
                return Ok(steps);
            }
            store::ROLE_SPAM => {
                if in_trash {
                    steps.push(Step::Untrash);
                }
                if i.location != 2 {
                    steps.push(Step::Modify { add: s(&["SPAM"]), remove: s(&["INBOX"]) });
                }
                return Ok(steps);
            }
            _ => {}
        }
        if in_trash {
            steps.push(Step::Untrash);
        }
        let target_label = target.gmail_label_id.clone();
        let mut remove: Vec<String> = Vec::new();
        if !copy {
            if i.location == 2 {
                remove.push("SPAM".into());
            } else if !in_trash {
                match &source {
                    Some(src) if src.role != store::ROLE_ALL && src.role != store::ROLE_TRASH => {
                        if let Some(l) = &src.gmail_label_id
                            && !matches!(l.as_str(), "SENT" | "DRAFT")
                        {
                            remove.push(l.clone());
                        }
                    }
                    Some(_) => {}
                    None => remove = folder_labels(i, map),
                }
            }
        } else if i.location == 2 {
            remove.push("SPAM".into());
        }
        let mut add = Vec::new();
        if let Some(l) = target_label {
            remove.retain(|r| *r != l);
            if !i.labels.contains(&l) || in_trash {
                add.push(l);
            }
        }
        if !add.is_empty() || !remove.is_empty() {
            steps.push(Step::Modify { add, remove });
        }
        Ok(steps)
    })
    .await?;
    Ok(transferred)
}

/// Gmail has no move between accounts: each message is downloaded whole (format=raw) from its own
/// mailbox and imported into `target`'s, keeping its date, attachments, read state and flag; for a
/// move the original then goes to its Deleted Items (one already there stays there, so Undo can
/// always bring it back). One at a time, with `mail.transfer {done, total}` progress; a message
/// that can't be copied is reported in `failed` and left alone. The copies appear with the target
/// mailbox's next sync, started at the end.
async fn transfer(ctx: &Arc<AppCtx>, infos: Vec<MsgInfo>, target: &store::FolderRow, copy: bool) -> anyhow::Result<Transferred> {
    let dest = ctx.api(target.account_id).await?;
    let mut base: Vec<String> = target.gmail_label_id.iter().filter(|l| !matches!(l.as_str(), "SENT" | "DRAFT")).cloned().collect();
    match target.role.as_str() {
        store::ROLE_TRASH if base.is_empty() => base.push("TRASH".into()),
        store::ROLE_SPAM if base.is_empty() => base.push("SPAM".into()),
        _ => {}
    }
    let total = infos.len();
    let mut out = Transferred::default();
    let mut moved = Vec::new();
    for (n, i) in infos.iter().enumerate() {
        ctx.events.emit("mail.transfer", json!({ "done": n, "total": total, "copy": copy }));
        let res: anyhow::Result<Imported> = async {
            if i.labels.iter().any(|l| l == "DRAFT") {
                return Err(crate::user_err!("draft", "Drafts can't be moved to another mailbox."));
            }
            let raw = ctx.api(i.account_id).await?.gmail_get(&i.gmail_id, "raw", &[]).await?.raw.unwrap_or_default();
            let raw = crate::google::gmail::b64url_decode(&raw)?;
            let mut labels = base.clone();
            for l in ["UNREAD", "STARRED"] {
                if i.labels.iter().any(|x| x == l) {
                    labels.push(l.into());
                }
            }
            let m = dest.gmail_import(&raw, &labels).await?;
            Ok(Imported { account_id: target.account_id, gmail_id: m.id })
        }
        .await;
        match res {
            Ok(c) => {
                out.imported.push(c);
                if !copy && i.location != 1 {
                    moved.push(i.id);
                }
            }
            Err(e) => {
                log::warn!("mail: message {} not copied to account {}: {}", i.id, target.account_id, crate::logging::err(&e));
                out.failed.push(Failed { id: i.id, message: crate::error::RpcError::from(e).message });
            }
        }
    }
    ctx.events.emit("mail.transfer", json!({ "done": total, "total": total, "copy": copy }));
    if !moved.is_empty() {
        run(ctx, moved, |_, _| Ok(vec![Step::Trash])).await?;
    }
    if !out.imported.is_empty() {
        ctx.sync_now(Some(target.account_id));
    }
    Ok(out)
}

/// Undo of a move / copy between mailboxes: the copies `transfer` made go for good (the originals
/// come back by their own undo step). A copy already synced goes through the usual delete; one not
/// synced yet is deleted at Gmail directly.
pub async fn delete_imported(ctx: &Arc<AppCtx>, copies: Vec<Imported>) -> anyhow::Result<()> {
    let keys = copies.clone();
    let local: Vec<(i64, String)> = ctx
        .db
        .read(move |c| {
            let mut st = c.prepare_cached("SELECT id FROM messages WHERE account_id=?1 AND gmail_id=?2")?;
            let mut out = Vec::new();
            for k in &keys {
                if let Ok(id) = st.query_row(params![k.account_id, k.gmail_id], |r| r.get::<_, i64>(0)) {
                    out.push((id, k.gmail_id.clone()));
                }
            }
            Ok(out)
        })
        .await?;
    for k in copies.iter().filter(|k| !local.iter().any(|(_, g)| *g == k.gmail_id)) {
        ctx.api(k.account_id).await?.gmail_delete(&k.gmail_id).await?;
    }
    let ids: Vec<i64> = local.iter().map(|(id, _)| *id).collect();
    if !ids.is_empty() {
        run(ctx, ids.clone(), |_, _| Ok(vec![Step::Delete])).await?;
        drop_cached_files(ctx, ids).await;
    }
    Ok(())
}

/// Does Delete erase this message for good? Only in Deleted Items; anywhere else (Junk too) it
/// moves there.
fn deletes_for_good(i: &MsgInfo) -> bool {
    i.location == 1
}

/// How many of `ids` Delete would erase for good (the warning before it, `mail.deleteInfo`).
pub fn permanent_count(c: &Connection, ids: &[i64]) -> anyhow::Result<usize> {
    Ok(load_infos(c, ids)?.iter().filter(|i| deletes_for_good(i)).count())
}

/// Delete: to Deleted Items; already there → permanently. Returns how many were deleted for good.
pub async fn delete(ctx: &Arc<AppCtx>, ids: Vec<i64>) -> anyhow::Result<usize> {
    let ids2 = ids.clone();
    let permanent: usize = ctx.db.read(move |c| permanent_count(c, &ids2)).await?;
    let doomed = ids.clone();
    run(ctx, ids, move |i, _| Ok(if deletes_for_good(i) { vec![Step::Delete] } else { vec![Step::Trash] })).await?;
    drop_cached_files(ctx, doomed).await;
    Ok(permanent)
}

/// Remove the cached attachment files of those of `ids` that no longer exist (deleted for good).
pub(super) async fn drop_cached_files(ctx: &Arc<AppCtx>, ids: Vec<i64>) {
    let gone: Vec<i64> = ctx
        .db
        .read(move |c| {
            let mut st = c.prepare_cached("SELECT 1 FROM messages WHERE id=?1")?;
            let mut out = Vec::new();
            for id in ids {
                if !st.exists([id])? {
                    out.push(id);
                }
            }
            Ok(out)
        })
        .await
        .unwrap_or_default();
    let root = ctx.data_dir.join("attachments");
    let _ = tokio::task::spawn_blocking(move || {
        for id in gone {
            let _ = std::fs::remove_dir_all(root.join(id.to_string()));
        }
    })
    .await;
}

pub async fn restore(ctx: &Arc<AppCtx>, ids: Vec<i64>) -> anyhow::Result<()> {
    run(ctx, ids, |i, map| {
        if i.location != 1 {
            return Ok(vec![]);
        }
        let mut steps = vec![Step::Untrash];
        let remaining: Vec<String> = folder_labels(i, map);
        let sent = i.labels.iter().any(|l| l == "SENT" || l == "DRAFT");
        if remaining.is_empty() && !sent {
            steps.push(Step::Modify { add: s(&["INBOX"]), remove: vec![] });
        }
        Ok(steps)
    })
    .await?;
    Ok(())
}

pub async fn mark_junk(ctx: &Arc<AppCtx>, ids: Vec<i64>, junk: bool) -> anyhow::Result<()> {
    run(ctx, ids, move |i, _| {
        let mut steps = Vec::new();
        if junk {
            if i.location == 1 {
                steps.push(Step::Untrash);
            }
            if i.location != 2 {
                steps.push(Step::Modify { add: s(&["SPAM"]), remove: s(&["INBOX"]) });
            }
        } else if i.location == 2 {
            steps.push(Step::Modify { add: s(&["INBOX"]), remove: s(&["SPAM"]) });
        }
        Ok(steps)
    })
    .await?;
    Ok(())
}

pub async fn folder_message_ids(ctx: &Arc<AppCtx>, folder_id: i64, unread_only: bool) -> anyhow::Result<Vec<i64>> {
    ctx.db
        .read(move |c| {
            let f = store::folder(c, folder_id)?;
            let sql = if f.role == store::ROLE_ALL {
                format!(
                    "SELECT id FROM messages WHERE account_id={} AND location=0{}",
                    f.account_id,
                    if unread_only { " AND is_read=0" } else { "" }
                )
            } else {
                format!(
                    "SELECT m.id FROM message_folders mf JOIN messages m ON m.id=mf.message_id WHERE mf.folder_id={folder_id}{}",
                    if unread_only { " AND m.is_read=0" } else { "" }
                )
            };
            let mut st = c.prepare(&sql)?;
            let v = st.query_map([], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?;
            Ok(v)
        })
        .await
}

pub async fn empty_folder(ctx: &Arc<AppCtx>, folder_id: i64) -> anyhow::Result<usize> {
    let f = ctx.db.read(move |c| store::folder(c, folder_id)).await?;
    if f.role != store::ROLE_TRASH && f.role != store::ROLE_SPAM {
        return Err(crate::user_err!("bad_folder", "Only Deleted Items and Junk E-mail can be emptied."));
    }
    let ids = folder_message_ids(ctx, folder_id, false).await?;
    let n = ids.len();
    run(ctx, ids.clone(), |_, _| Ok(vec![Step::Delete])).await?;
    drop_cached_files(ctx, ids).await;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_flag_dates() {
        let wed = chrono::NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
        assert_eq!(flag_due("today", None, wed).unwrap(), "2026-10-14");
        assert_eq!(flag_due("tomorrow", None, wed).unwrap(), "2026-10-15");
        assert_eq!(flag_due("thisWeek", None, wed).unwrap(), "2026-10-16");
        assert_eq!(flag_due("nextWeek", None, wed).unwrap(), "2026-10-23");
        assert_eq!(flag_due("noDate", None, wed), None);
        assert_eq!(flag_due("custom", Some("2026-12-01"), wed).unwrap(), "2026-12-01");
    }
}

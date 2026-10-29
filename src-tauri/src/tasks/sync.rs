//! Google Tasks sync for one account: push pending local ops, then pull.
//!
//! Pull: task lists are always listed in full (they are few). Tasks are listed per list — in full
//! the first time, then with `updatedMin` (plus `showDeleted`) for changes only. Outlook-only
//! columns are never touched by a pull, and tasks with unpushed local changes are left alone.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{DOMAIN, store};
use crate::{
    ctx::AppCtx,
    db::util::{get_sync_state, set_sync_state},
    google::{
        Api,
        client::api_status,
        tasks::{self as gt, ListOpts},
    },
    sync::pending::{self, PendingOp},
};

/// Overlap for `updatedMin` so writes that become visible a little late on Google's side are seen.
const UPDATED_MIN_MARGIN_MS: i64 = 120_000;

pub fn updated_min_key(list_gid: &str) -> String {
    format!("tasks.updatedMin:{list_gid}")
}

/// One push or pull at a time per account (the sync worker and `push_soon` share it), so a create
/// op is never sent twice and a pull never sees a task that is half-way through being pushed.
fn push_lock(account_id: i64) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(Default::default).lock().entry(account_id).or_default().clone()
}

pub async fn sync_account(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let lock = push_lock(api.account_id);
    let _g = lock.lock().await;
    push_locked(ctx, api).await?;
    pull_locked(ctx, api).await
}

/// Push pending ops right away after a local change (only once the app's sync has started;
/// tests drive sync directly).
pub fn push_soon(ctx: &Arc<AppCtx>, account_id: i64) {
    if ctx.sync.get().is_none() {
        return;
    }
    let ctx = ctx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let res = async {
            let api = ctx.api(account_id).await?;
            push(&ctx, &api).await
        }
        .await;
        if let Err(e) = res {
            log::warn!("tasks push for account {account_id}: {}", crate::logging::err(&e));
        }
    });
}

// ---- push -------------------------------------------------------------------------------------

pub async fn push(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let lock = push_lock(api.account_id);
    let _g = lock.lock().await;
    push_locked(ctx, api).await
}

/// `push` for a caller that holds the account's `push_lock`.
async fn push_locked(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let mut touched = false;
    let dropped = pending::drain(&ctx.db, api.account_id, DOMAIN, |op| {
        touched = true;
        push_op(ctx, api, op)
    })
    .await?;
    if dropped > 0 {
        // something could not be applied on Google: re-read everything from the server
        let acct = api.account_id;
        ctx.db
            .write(move |c| {
                c.execute("DELETE FROM sync_state WHERE account_id = ?1 AND kind LIKE 'tasks.updatedMin:%'", [acct])?;
                Ok(())
            })
            .await?;
    }
    if touched {
        ctx.events.emit("tasks.changed", json!({ "accountId": api.account_id }));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpPayload {
    list_id: Option<i64>,
    task_id: Option<i64>,
    google_id: Option<String>,
    list_google_id: Option<String>,
    from_list_google_id: Option<String>,
}

/// Google fields of a local task, for insert/patch.
struct Pushable {
    google_id: Option<String>,
    list_gid: Option<String>,
    parent_gid: Option<String>,
    previous_gid: Option<String>,
    body: Value,
}

fn pushable(c: &Connection, id: i64) -> anyhow::Result<Option<Pushable>> {
    let row = c
        .query_row(
            "SELECT t.google_id, l.google_id, p.google_id, t.title, t.notes, t.due, t.status, t.completed_at \
             FROM tasks t JOIN task_lists l ON l.id = t.list_id LEFT JOIN tasks p ON p.id = t.parent_id WHERE t.id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<i64>>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((gid, list_gid, parent_gid, title, notes, due, status, completed)) = row else { return Ok(None) };
    // previous sibling that already exists on Google
    let (list_id, parent_id): (i64, Option<i64>) =
        c.query_row("SELECT list_id, parent_id FROM tasks WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let sibs = store::siblings(c, list_id, parent_id, -1)?;
    let mut previous_gid = None;
    if let Some(i) = sibs.iter().position(|s| s.0 == id) {
        for (sid, _) in sibs[..i].iter().rev() {
            // Google refuses a hidden task (cleared / completed in its apps) as `previous`
            let g: Option<String> =
                c.query_row("SELECT CASE WHEN hidden THEN NULL ELSE google_id END FROM tasks WHERE id = ?1", [sid], |r| r.get(0))?;
            if g.is_some() {
                previous_gid = g;
                break;
            }
        }
    }
    let mut body = json!({
        "title": title,
        "notes": if notes.is_empty() { Value::Null } else { json!(notes) },
        "due": due.as_deref().map(gt::date_to_due),
        "status": status,
    });
    body["completed"] = match (status.as_str(), completed) {
        ("completed", Some(ms)) => json!(gt::fmt_ts(ms)),
        _ => Value::Null,
    };
    Ok(Some(Pushable { google_id: gid, list_gid, parent_gid, previous_gid, body }))
}

/// Store what Google returned for a task we pushed.
async fn store_remote(ctx: &AppCtx, id: i64, t: gt::Task) -> anyhow::Result<()> {
    ctx.db
        .write(move |c| {
            c.execute(
                "UPDATE tasks SET google_id = ?1, etag = ?2, position = COALESCE(?3, position), updated = COALESCE(?4, updated) WHERE id = ?5",
                params![t.id, t.etag, t.position, t.updated.as_deref().and_then(gt::parse_ts), id],
            )?;
            Ok(())
        })
        .await
}

fn not_found_ok(r: anyhow::Result<()>) -> anyhow::Result<()> {
    match r {
        Err(e) if matches!(api_status(&e), Some(404 | 410)) => Ok(()),
        other => other,
    }
}

async fn push_op(ctx: &Arc<AppCtx>, api: &Api, op: PendingOp) -> anyhow::Result<()> {
    let p: OpPayload = op.payload()?;
    match op.kind.as_str() {
        "createList" => {
            let id = p.list_id.unwrap_or_default();
            let row = ctx.db.read(move |c| store::get_list(c, id)).await?;
            let Some(l) = row else { return Ok(()) };
            if l.google_id.is_some() {
                return Ok(());
            }
            let r = gt::insert_tasklist(api, &l.title).await?;
            let upd = r.updated.as_deref().and_then(gt::parse_ts).unwrap_or(0);
            ctx.db
                .write(move |c| {
                    c.execute("UPDATE task_lists SET google_id = ?1, updated = ?2 WHERE id = ?3", params![r.id, upd, id])?;
                    Ok(())
                })
                .await
        }
        "renameList" => {
            let id = p.list_id.unwrap_or_default();
            let row = ctx.db.read(move |c| store::get_list(c, id)).await?;
            let Some(l) = row else { return Ok(()) };
            let Some(gid) = l.google_id else { return Ok(()) };
            gt::patch_tasklist(api, &gid, &l.title).await?;
            Ok(())
        }
        "deleteList" => {
            let Some(gid) = p.google_id else { return Ok(()) };
            not_found_ok(gt::delete_tasklist(api, &gid).await)
        }
        "create" => {
            let id = p.task_id.unwrap_or_default();
            let Some(t) = ctx.db.read(move |c| pushable(c, id)).await? else { return Ok(()) };
            if t.google_id.is_some() {
                return Ok(());
            }
            let Some(list_gid) = t.list_gid else { anyhow::bail!("task list not created on Google yet") };
            let r = gt::insert_task(api, &list_gid, t.body, t.parent_gid.as_deref(), t.previous_gid.as_deref()).await?;
            store_remote(ctx, id, r).await
        }
        "update" => {
            let id = p.task_id.unwrap_or_default();
            let Some(t) = ctx.db.read(move |c| pushable(c, id)).await? else { return Ok(()) };
            let (Some(gid), Some(list_gid)) = (t.google_id, t.list_gid) else { return Ok(()) };
            let r = gt::patch_task(api, &list_gid, &gid, t.body).await?;
            store_remote(ctx, id, r).await
        }
        "delete" => {
            let (Some(gid), Some(list_gid)) = (p.google_id, p.list_google_id) else { return Ok(()) };
            not_found_ok(gt::delete_task(api, &list_gid, &gid).await)
        }
        "move" => {
            let id = p.task_id.unwrap_or_default();
            let Some(t) = ctx.db.read(move |c| pushable(c, id)).await? else { return Ok(()) };
            let (Some(gid), Some(list_gid)) = (t.google_id, t.list_gid) else { return Ok(()) };
            let from = p.from_list_google_id.unwrap_or_else(|| list_gid.clone());
            let dest = if from != list_gid { Some(list_gid.as_str()) } else { None };
            let r = gt::move_task(api, &from, &gid, t.parent_gid.as_deref(), t.previous_gid.as_deref(), dest).await?;
            store_remote(ctx, id, r).await
        }
        other => {
            log::warn!("tasks: unknown pending op {other}");
            Ok(())
        }
    }
}

// ---- pull -------------------------------------------------------------------------------------

/// Local task ids that still have unpushed changes (left alone by the pull). Read inside the
/// pull's own transaction, so an edit made while the lists were downloading counts too.
fn dirty_tasks(c: &Connection, account_id: i64) -> anyhow::Result<HashSet<i64>> {
    let mut st = c.prepare("SELECT payload FROM pending_ops WHERE account_id = ?1 AND domain = ?2")?;
    let ops = st.query_map(params![account_id, DOMAIN], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(ops.iter().filter_map(|s| serde_json::from_str::<Value>(s).ok()).filter_map(|v| v["taskId"].as_i64()).collect())
}

pub async fn pull(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let lock = push_lock(api.account_id);
    let _g = lock.lock().await;
    pull_locked(ctx, api).await
}

/// `pull` for a caller that holds the account's `push_lock`.
async fn pull_locked(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let account_id = api.account_id;
    ctx.events.status(account_id, "sync", "Receiving tasks…", Some(0.8));
    let remote_lists = gt::list_tasklists(api).await?;

    // lists
    let rl = remote_lists.clone();
    let mut any_change = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let mut changed = false;
            let pending_renames: HashSet<i64> = {
                let mut st = tx.prepare("SELECT payload FROM pending_ops WHERE account_id = ?1 AND domain = ?2 AND kind = 'renameList'")?;
                st.query_map(params![account_id, DOMAIN], |r| r.get::<_, String>(0))?
                    .filter_map(|s| s.ok())
                    .filter_map(|s| serde_json::from_str::<Value>(&s).ok())
                    .filter_map(|v| v["listId"].as_i64())
                    .collect()
            };
            for (i, l) in rl.iter().enumerate() {
                let upd = l.updated.as_deref().and_then(gt::parse_ts).unwrap_or(0);
                let existing: Option<(i64, String, i64, i64)> = tx
                    .query_row(
                        "SELECT id, title, position, is_default FROM task_lists WHERE account_id = ?1 AND google_id = ?2",
                        params![account_id, l.id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()?;
                let is_default = (i == 0) as i64;
                match existing {
                    Some((id, title, pos, def)) => {
                        let keep_title = pending_renames.contains(&id);
                        if (!keep_title && title != l.title) || pos != i as i64 || def != is_default {
                            tx.execute(
                                "UPDATE task_lists SET title = CASE WHEN ?1 THEN title ELSE ?2 END, updated = ?3, position = ?4, is_default = ?5 WHERE id = ?6",
                                params![keep_title, l.title, upd, i as i64, is_default, id],
                            )?;
                            changed = true;
                        }
                    }
                    None => {
                        tx.execute(
                            "INSERT INTO task_lists(account_id, google_id, title, updated, position, is_default) VALUES (?1,?2,?3,?4,?5,?6)",
                            params![account_id, l.id, l.title, upd, i as i64, is_default],
                        )?;
                        changed = true;
                    }
                }
            }
            // lists deleted on Google (local-only lists waiting to be created are kept)
            let remote_ids: HashSet<&str> = rl.iter().map(|l| l.id.as_str()).collect();
            let mut st = tx.prepare("SELECT id, google_id FROM task_lists WHERE account_id = ?1 AND google_id IS NOT NULL")?;
            let gone: Vec<(i64, String)> = st
                .query_map([account_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|(_, g)| !remote_ids.contains(g.as_str()))
                .collect();
            drop(st);
            for (id, gid) in gone {
                tx.execute("DELETE FROM task_lists WHERE id = ?1", [id])?;
                set_sync_state(&tx, account_id, &updated_min_key(&gid), None)?;
                changed = true;
            }
            tx.commit()?;
            Ok(changed)
        })
        .await?;

    // tasks, list by list
    for l in &remote_lists {
        let gid = l.id.clone();
        let key = updated_min_key(&gid);
        let k2 = key.clone();
        let since = ctx.db.read(move |c| get_sync_state(c, account_id, &k2)).await?;
        let full = since.is_none();
        let opts = ListOpts { show_completed: true, show_hidden: true, show_deleted: !full, updated_min: since };
        let items = match gt::list_tasks(api, &gid, &opts).await {
            Ok(v) => v,
            Err(e) if api_status(&e) == Some(404) => continue, // list vanished meanwhile
            Err(e) => return Err(e),
        };
        let changed = ctx.db.write(move |c| apply_tasks(c, account_id, &gid, &key, items, full)).await?;
        any_change |= changed;
    }
    if any_change {
        ctx.events.emit("tasks.changed", json!({ "accountId": account_id }));
    }
    Ok(())
}

/// Upsert one list's pulled tasks. Returns whether anything changed locally.
fn apply_tasks(c: &mut Connection, account_id: i64, list_gid: &str, key: &str, items: Vec<gt::Task>, full: bool) -> anyhow::Result<bool> {
    let tx = c.transaction()?;
    let dirty = dirty_tasks(&tx, account_id)?;
    let list_id: i64 =
        tx.query_row("SELECT id FROM task_lists WHERE account_id = ?1 AND google_id = ?2", params![account_id, list_gid], |r| r.get(0))?;
    let mut changed = false;
    let mut max_updated: Option<i64> = None;
    let mut seen: HashSet<String> = HashSet::new();
    let mut parents: Vec<(i64, String)> = Vec::new();
    for t in &items {
        let upd = t.updated.as_deref().and_then(gt::parse_ts).unwrap_or(0);
        max_updated = Some(max_updated.map_or(upd, |m: i64| m.max(upd)));
        // cached statements: one lookup per pulled task
        let existing: Option<(i64, Option<String>, i64)> = tx
            .prepare_cached("SELECT id, etag, list_id FROM tasks WHERE account_id = ?1 AND google_id = ?2")?
            .query_row(params![account_id, t.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        if t.deleted == Some(true) {
            if let Some((id, _, lid)) = existing
                && lid == list_id
                && !dirty.contains(&id)
            {
                tx.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
                changed = true;
            }
            continue;
        }
        seen.insert(t.id.clone());
        let status = if t.status.as_deref() == Some("completed") { "completed" } else { "needsAction" };
        let completed = t.completed.as_deref().and_then(gt::parse_ts);
        let due = t.due.as_deref().and_then(gt::due_to_date);
        let title = t.title.clone().unwrap_or_default();
        let notes = t.notes.clone().unwrap_or_default();
        let position = t.position.clone().unwrap_or_default();
        let hidden = t.hidden.unwrap_or(false) as i64;
        let id = match existing {
            Some((id, _, _)) if dirty.contains(&id) => continue,
            Some((id, etag, lid)) => {
                if etag.is_some() && etag == t.etag && lid == list_id {
                    // unchanged
                    if let Some(p) = &t.parent {
                        parents.push((id, p.clone()));
                    } else {
                        parents.push((id, String::new()));
                    }
                    continue;
                }
                tx.execute(
                    "UPDATE tasks SET list_id = ?1, position = ?2, title = ?3, notes = ?4, due = ?5, completed_at = ?6, updated = ?7, \
                     etag = ?8, hidden = ?9, \
                     percent = CASE WHEN ?10 = 'completed' THEN 100 WHEN status = 'completed' THEN 0 ELSE percent END, \
                     outlook_status = CASE WHEN ?10 != 'completed' AND status = 'completed' THEN 'notStarted' ELSE outlook_status END, \
                     status = ?10 WHERE id = ?11",
                    params![list_id, position, title, notes, due, completed, upd, t.etag, hidden, status, id],
                )?;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO tasks(account_id, list_id, google_id, position, title, notes, due, status, completed_at, updated, etag, \
                     hidden, percent, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?10)",
                    params![
                        account_id,
                        list_id,
                        t.id,
                        position,
                        title,
                        notes,
                        due,
                        status,
                        completed,
                        upd,
                        t.etag,
                        hidden,
                        if status == "completed" { 100 } else { 0 }
                    ],
                )?;
                tx.last_insert_rowid()
            }
        };
        changed = true;
        parents.push((id, t.parent.clone().unwrap_or_default()));
    }
    // parents (after every task of the batch exists locally)
    for (id, pgid) in parents {
        let pid: Option<i64> = if pgid.is_empty() {
            None
        } else {
            tx.prepare_cached("SELECT id FROM tasks WHERE account_id = ?1 AND google_id = ?2")?
                .query_row(params![account_id, pgid], |r| r.get(0))
                .optional()?
        };
        let n = tx.prepare_cached("UPDATE tasks SET parent_id = ?1 WHERE id = ?2 AND parent_id IS NOT ?1")?.execute(params![pid, id])?;
        changed |= n > 0;
    }
    if full {
        // anything synced earlier that Google no longer has
        let mut st = tx.prepare("SELECT id, google_id FROM tasks WHERE list_id = ?1 AND google_id IS NOT NULL")?;
        let stale: Vec<i64> = st
            .query_map([list_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|(id, g)| !seen.contains(g) && !dirty.contains(id))
            .map(|(id, _)| id)
            .collect();
        drop(st);
        for id in stale {
            tx.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
            changed = true;
        }
    }
    // next pull asks only for changes; keep the old mark when nothing came back
    let prev = get_sync_state(&tx, account_id, key)?.and_then(|s| gt::parse_ts(&s));
    let mark = match (max_updated, prev) {
        (Some(m), _) => Some(m - UPDATED_MIN_MARGIN_MS),
        (None, Some(p)) => Some(p),
        // empty list on the first sync: everything from now on
        (None, None) => Some(crate::db::util::now_ms() - UPDATED_MIN_MARGIN_MS),
    };
    let mark = match (mark, prev) {
        (Some(m), Some(p)) => Some(m.max(p)),
        (m, _) => m,
    };
    if let Some(m) = mark {
        set_sync_state(&tx, account_id, key, Some(&gt::fmt_ts(m)))?;
    }
    tx.commit()?;
    Ok(changed)
}

//! Tasks domain: Google Tasks lists/tasks plus Outlook-only fields (start date, priority, % complete,
//! reminder, categories, private) kept locally. Every change is local-first: SQLite is updated at
//! once, `tasks.changed` is emitted and a pending op is queued for `sync` to push.
//!
//! rpc: tasks.lists, tasks.createList, tasks.renameList, tasks.deleteList, tasks.list, tasks.get,
//! tasks.create, tasks.update, tasks.delete, tasks.setComplete, tasks.move.

pub mod schema;
pub mod store;
pub mod sync;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use chrono::NaiveDate;
use rusqlite::{Connection, params};
use serde::{Deserialize, Deserializer};
use serde_json::json;

use crate::{
    ctx::AppCtx,
    db::util::now_ms,
    rpc::{NoParams, Registry},
    sync::pending,
};
use store::{TaskDto, TaskListDto};

pub const DOMAIN: &str = "tasks";

pub fn register(r: &mut Registry) {
    r.add("tasks.lists", lists);
    r.add("tasks.createList", create_list);
    r.add("tasks.renameList", rename_list);
    r.add("tasks.deleteList", delete_list);
    r.add("tasks.list", list);
    r.add("tasks.get", get);
    r.add("tasks.create", create);
    r.add("tasks.update", update);
    r.add("tasks.delete", delete);
    r.add("tasks.setComplete", set_complete);
    r.add("tasks.move", move_tasks);
}

/// `Option<Option<T>>` for patch params: absent = leave alone, null = clear, value = set.
fn double<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

fn changed(ctx: &Arc<AppCtx>, account_ids: impl IntoIterator<Item = i64>) {
    let mut ids: Vec<i64> = account_ids.into_iter().collect();
    ids.sort();
    ids.dedup();
    for a in ids {
        ctx.events.emit("tasks.changed", json!({ "accountId": a }));
        sync::push_soon(ctx, a);
    }
}

fn valid_date(s: &str) -> anyhow::Result<String> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map(|d| d.format("%Y-%m-%d").to_string())
        .map_err(|_| crate::user_err!("bad_params", "invalid date {s} (expected YYYY-MM-DD)"))
}

fn opt_date(s: Option<String>) -> anyhow::Result<Option<String>> {
    match s {
        Some(s) if !s.trim().is_empty() => Ok(Some(valid_date(s.trim())?)),
        _ => Ok(None),
    }
}

const OUTLOOK_STATUSES: &[&str] = &["notStarted", "inProgress", "completed", "waiting", "deferred"];

// ---- lists ------------------------------------------------------------------------------------

async fn lists(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<Vec<TaskListDto>> {
    ctx.db.read(store::list_lists).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateList {
    account_id: Option<i64>,
    title: String,
}

async fn create_list(ctx: Arc<AppCtx>, p: CreateList) -> anyhow::Result<TaskListDto> {
    let title = p.title.trim().to_string();
    if title.is_empty() {
        return Err(crate::user_err!("bad_params", "The folder needs a name."));
    }
    let (id, account_id) = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let account_id: i64 = match p.account_id {
                Some(a) => a,
                None => tx
                    .query_row("SELECT id FROM accounts WHERE enabled = 1 ORDER BY position, id LIMIT 1", [], |r| r.get(0))
                    .map_err(|_| crate::user_err!("no_account", "Add an account first."))?,
            };
            let pos: i64 =
                tx.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM task_lists WHERE account_id = ?1", [account_id], |r| r.get(0))?;
            tx.execute(
                "INSERT INTO task_lists(account_id, title, updated, position, is_default) VALUES (?1, ?2, ?3, ?4, 0)",
                params![account_id, title, now_ms(), pos],
            )?;
            let id = tx.last_insert_rowid();
            pending::enqueue(&tx, account_id, DOMAIN, "createList", &json!({ "listId": id }))?;
            tx.commit()?;
            Ok((id, account_id))
        })
        .await?;
    changed(&ctx, [account_id]);
    let all = ctx.db.read(store::list_lists).await?;
    all.into_iter().find(|l| l.id == id).ok_or_else(|| anyhow::anyhow!("list vanished"))
}

#[derive(Deserialize)]
struct RenameList {
    id: i64,
    title: String,
}

async fn rename_list(ctx: Arc<AppCtx>, p: RenameList) -> anyhow::Result<()> {
    let title = p.title.trim().to_string();
    if title.is_empty() {
        return Err(crate::user_err!("bad_params", "The folder needs a name."));
    }
    let account_id = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let l = store::get_list(&tx, p.id)?.ok_or_else(|| crate::user_err!("not_found", "task list {} not found", p.id))?;
            tx.execute("UPDATE task_lists SET title = ?1, updated = ?2 WHERE id = ?3", params![title, now_ms(), p.id])?;
            pending::enqueue(&tx, l.account_id, DOMAIN, "renameList", &json!({ "listId": p.id }))?;
            tx.commit()?;
            Ok(l.account_id)
        })
        .await?;
    changed(&ctx, [account_id]);
    Ok(())
}

#[derive(Deserialize)]
struct IdParam {
    id: i64,
}

async fn delete_list(ctx: Arc<AppCtx>, p: IdParam) -> anyhow::Result<()> {
    let account_id = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let l = store::get_list(&tx, p.id)?.ok_or_else(|| crate::user_err!("not_found", "task list {} not found", p.id))?;
            if l.is_default {
                return Err(crate::user_err!("forbidden", "The default Tasks folder can't be deleted."));
            }
            tx.execute("DELETE FROM task_lists WHERE id = ?1", [p.id])?;
            if let Some(gid) = &l.google_id {
                pending::enqueue(&tx, l.account_id, DOMAIN, "deleteList", &json!({ "googleId": gid }))?;
                crate::db::util::set_sync_state(&tx, l.account_id, &sync::updated_min_key(gid), None)?;
            }
            tx.commit()?;
            Ok(l.account_id)
        })
        .await?;
    changed(&ctx, [account_id]);
    Ok(())
}

// ---- queries ----------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ListParams {
    list_id: Option<i64>,
    account_id: Option<i64>,
    /// simple | detailed | active | next7 | overdue | completed | todo
    view: Option<String>,
    search: Option<String>,
    /// local "today" (YYYY-MM-DD); defaults to the machine's date
    today: Option<String>,
}

fn today_or(s: &Option<String>) -> NaiveDate {
    s.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).unwrap_or_else(crate::clock::today)
}

async fn list(ctx: Arc<AppCtx>, p: ListParams) -> anyhow::Result<Vec<TaskDto>> {
    let today = today_or(&p.today);
    ctx.db.read(move |c| query_tasks(c, &p, today)).await
}

fn query_tasks(c: &Connection, p: &ListParams, today: NaiveDate) -> anyhow::Result<Vec<TaskDto>> {
    let mut wh: Vec<String> = vec!["1=1".into()];
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(l) = p.list_id {
        args.push(l.into());
        wh.push(format!("t.list_id = ?{}", args.len()));
    }
    if let Some(a) = p.account_id {
        args.push(a.into());
        wh.push(format!("t.account_id = ?{}", args.len()));
    }
    let t0 = today.format("%Y-%m-%d").to_string();
    let t7 = (today + chrono::Duration::days(7)).format("%Y-%m-%d").to_string();
    match p.view.as_deref().unwrap_or("simple") {
        "simple" | "detailed" | "category" => {}
        "active" | "todo" => wh.push("t.status != 'completed'".into()),
        "next7" => {
            args.push(t0.clone().into());
            args.push(t7.into());
            wh.push(format!("t.status != 'completed' AND t.due >= ?{} AND t.due <= ?{}", args.len() - 1, args.len()));
        }
        "overdue" => {
            args.push(t0.into());
            wh.push(format!("t.status != 'completed' AND t.due < ?{}", args.len()));
        }
        "completed" => wh.push("t.status = 'completed'".into()),
        v => return Err(crate::user_err!("bad_params", "unknown view {v}")),
    }
    if let Some(s) = p.search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let like = format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        args.push(like.into());
        let n = args.len();
        wh.push(format!("(t.title LIKE ?{n} ESCAPE '\\' OR t.notes LIKE ?{n} ESCAPE '\\' OR t.categories LIKE ?{n} ESCAPE '\\')"));
    }
    let sql = format!(
        "{} JOIN accounts a ON a.id = t.account_id WHERE a.enabled = 1 AND {} ORDER BY a.position, l.is_default DESC, l.position, l.id, {}",
        store::TASK_SELECT,
        wh.join(" AND "),
        store::TASK_ORDER
    );
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map(rusqlite::params_from_iter(args), store::task_from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

async fn get(ctx: Arc<AppCtx>, p: IdParam) -> anyhow::Result<TaskDto> {
    ctx.db.read(move |c| store::get_task(c, p.id)?.ok_or_else(|| crate::user_err!("not_found", "This item no longer exists."))).await
}

// ---- create / update --------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CreateParams {
    pub list_id: Option<i64>,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub due: Option<String>,
    pub start: Option<String>,
    pub status: Option<String>,
    pub importance: Option<String>,
    pub percent: Option<i64>,
    pub reminder_at: Option<i64>,
    pub categories: Option<Vec<String>>,
    pub is_private: Option<bool>,
    pub parent_id: Option<i64>,
    /// place right after this sibling (default: first, like Google)
    pub previous_id: Option<i64>,
}

async fn create(ctx: Arc<AppCtx>, p: CreateParams) -> anyhow::Result<TaskDto> {
    let (id, account_id) = ctx.db.write(move |c| insert_task(c, p)).await?;
    changed(&ctx, [account_id]);
    get(ctx, IdParam { id }).await
}

/// Insert a task locally and queue its creation. Returns (id, account id).
pub fn insert_task(c: &mut Connection, p: CreateParams) -> anyhow::Result<(i64, i64)> {
    let due = opt_date(p.due)?;
    let start = opt_date(p.start)?;
    let status = p.status.unwrap_or_else(|| "notStarted".into());
    if !OUTLOOK_STATUSES.contains(&status.as_str()) {
        return Err(crate::user_err!("bad_params", "unknown status {status}"));
    }
    let complete = status == "completed" || p.percent == Some(100);
    let percent = if complete { 100 } else { p.percent.unwrap_or(0).clamp(0, 99) };
    let tx = c.transaction()?;
    let list_id = match p.list_id {
        Some(l) => l,
        None => store::default_list(&tx)?
            .ok_or_else(|| crate::user_err!("no_list", "There is no Tasks folder yet — wait for Send/Receive to finish."))?,
    };
    let list = store::get_list(&tx, list_id)?.ok_or_else(|| crate::user_err!("not_found", "task list {list_id} not found"))?;
    // subtasks only one level deep, in the same list
    let parent_id = match p.parent_id {
        Some(pid) => {
            let (pl, pp): (i64, Option<i64>) =
                tx.query_row("SELECT list_id, parent_id FROM tasks WHERE id = ?1", [pid], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if pl != list_id {
                return Err(crate::user_err!("bad_params", "parent is in another list"));
            }
            Some(pp.unwrap_or(pid))
        }
        None => None,
    };
    let position = store::position_after(&tx, list_id, parent_id, p.previous_id, -1)?;
    let now = now_ms();
    tx.execute(
        "INSERT INTO tasks(account_id, list_id, parent_id, position, title, notes, due, status, completed_at, updated, \
         start_date, outlook_status, importance, percent, reminder_at, categories, private, created_at) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?10)",
        params![
            list.account_id,
            list_id,
            parent_id,
            position,
            p.title.trim(),
            p.notes,
            due,
            if complete { "completed" } else { "needsAction" },
            if complete { Some(now) } else { None },
            now,
            start,
            if complete { "notStarted".to_string() } else { status },
            store::importance_num(p.importance.as_deref().unwrap_or("normal")),
            percent,
            p.reminder_at,
            serde_json::to_string(&p.categories.unwrap_or_default())?,
            p.is_private.unwrap_or(false) as i64,
        ],
    )?;
    let id = tx.last_insert_rowid();
    pending::enqueue(&tx, list.account_id, DOMAIN, "create", &json!({ "taskId": id }))?;
    tx.commit()?;
    Ok((id, list.account_id))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateParams {
    pub id: i64,
    pub title: Option<String>,
    pub notes: Option<String>,
    #[serde(default, deserialize_with = "double")]
    pub due: Option<Option<String>>,
    #[serde(default, deserialize_with = "double")]
    pub start: Option<Option<String>>,
    pub status: Option<String>,
    pub importance: Option<String>,
    pub percent: Option<i64>,
    #[serde(default, deserialize_with = "double")]
    pub reminder_at: Option<Option<i64>>,
    pub categories: Option<Vec<String>>,
    pub is_private: Option<bool>,
}

async fn update(ctx: Arc<AppCtx>, p: UpdateParams) -> anyhow::Result<TaskDto> {
    let id = p.id;
    let account_id = ctx.db.write(move |c| update_task(c, p)).await?;
    changed(&ctx, [account_id]);
    get(ctx, IdParam { id }).await
}

struct Current {
    account_id: i64,
    title: String,
    notes: String,
    due: Option<String>,
    complete: bool,
    outlook_status: String,
    percent: i64,
}

fn current(c: &Connection, id: i64) -> anyhow::Result<Current> {
    c.query_row("SELECT account_id, title, notes, due, status, outlook_status, percent FROM tasks WHERE id = ?1", [id], |r| {
        Ok(Current {
            account_id: r.get(0)?,
            title: r.get(1)?,
            notes: r.get(2)?,
            due: r.get(3)?,
            complete: r.get::<_, String>(4)? == "completed",
            outlook_status: r.get(5)?,
            percent: r.get(6)?,
        })
    })
    .map_err(|_| crate::user_err!("not_found", "This item no longer exists."))
}

/// Apply an update locally; queue a push when a field Google stores changed. Returns the account.
pub fn update_task(c: &mut Connection, p: UpdateParams) -> anyhow::Result<i64> {
    let tx = c.transaction()?;
    let cur = current(&tx, p.id)?;
    let mut google_changed = false;
    let now = now_ms();

    if let Some(t) = &p.title
        && t.trim() != cur.title
    {
        tx.execute("UPDATE tasks SET title = ?1 WHERE id = ?2", params![t.trim(), p.id])?;
        google_changed = true;
    }
    if let Some(n) = &p.notes
        && *n != cur.notes
    {
        tx.execute("UPDATE tasks SET notes = ?1 WHERE id = ?2", params![n, p.id])?;
        google_changed = true;
    }
    if let Some(d) = p.due {
        let d = opt_date(d)?;
        if d != cur.due {
            tx.execute("UPDATE tasks SET due = ?1 WHERE id = ?2", params![d, p.id])?;
            google_changed = true;
        }
    }
    if let Some(s) = p.start {
        tx.execute("UPDATE tasks SET start_date = ?1 WHERE id = ?2", params![opt_date(s)?, p.id])?;
    }
    if let Some(i) = &p.importance {
        tx.execute("UPDATE tasks SET importance = ?1 WHERE id = ?2", params![store::importance_num(i), p.id])?;
    }
    if let Some(r) = p.reminder_at {
        tx.execute("UPDATE tasks SET reminder_at = ?1 WHERE id = ?2", params![r, p.id])?;
    }
    if let Some(cats) = &p.categories {
        tx.execute("UPDATE tasks SET categories = ?1 WHERE id = ?2", params![serde_json::to_string(cats)?, p.id])?;
    }
    if let Some(b) = p.is_private {
        tx.execute("UPDATE tasks SET private = ?1 WHERE id = ?2", params![b as i64, p.id])?;
    }

    // status and % complete stay consistent the way Outlook keeps them
    if p.status.is_some() || p.percent.is_some() {
        if let Some(s) = &p.status
            && !OUTLOOK_STATUSES.contains(&s.as_str())
        {
            return Err(crate::user_err!("bad_params", "unknown status {s}"));
        }
        let (status, percent) = match (p.status.as_deref(), p.percent.map(|n| n.clamp(0, 100))) {
            (Some("completed"), _) => ("completed".to_string(), 100),
            (Some(s), Some(n)) => (s.to_string(), n.min(99)),
            (Some(s), None) => {
                let n = if s == "notStarted" || cur.complete { 0 } else { cur.percent };
                (s.to_string(), n)
            }
            (None, Some(100)) => ("completed".to_string(), 100),
            (None, Some(n)) => {
                let base = if cur.complete { "notStarted" } else { cur.outlook_status.as_str() };
                let s = match (base, n) {
                    ("notStarted", n) if n > 0 => "inProgress",
                    ("inProgress", 0) => "notStarted",
                    (b, _) => b,
                };
                (s.to_string(), n)
            }
            (None, None) => unreachable!(),
        };
        let complete = status == "completed";
        if complete != cur.complete {
            google_changed = true;
        }
        tx.execute(
            "UPDATE tasks SET status = ?1, completed_at = CASE WHEN ?1 = 'completed' THEN COALESCE(completed_at, ?2) ELSE NULL END, \
             outlook_status = ?3, percent = ?4, hidden = CASE WHEN ?1 = 'completed' THEN hidden ELSE 0 END WHERE id = ?5",
            params![
                if complete { "completed" } else { "needsAction" },
                now,
                if complete { cur.outlook_status.clone() } else { status },
                percent,
                p.id
            ],
        )?;
    }

    tx.execute("UPDATE tasks SET updated = ?1 WHERE id = ?2", params![now, p.id])?;
    if google_changed {
        pending::enqueue(&tx, cur.account_id, DOMAIN, "update", &json!({ "taskId": p.id }))?;
    }
    tx.commit()?;
    Ok(cur.account_id)
}

// ---- delete / complete / move -----------------------------------------------------------------

#[derive(Deserialize)]
struct IdsParam {
    ids: Vec<i64>,
}

async fn delete(ctx: Arc<AppCtx>, p: IdsParam) -> anyhow::Result<()> {
    let accounts = ctx.db.write(move |c| delete_tasks(c, &p.ids)).await?;
    changed(&ctx, accounts);
    Ok(())
}

pub fn delete_tasks(c: &mut Connection, ids: &[i64]) -> anyhow::Result<Vec<i64>> {
    let tx = c.transaction()?;
    let mut accounts = Vec::new();
    for id in ids {
        let row: Option<(i64, Option<String>, Option<String>)> = tx
            .query_row(
                "SELECT t.account_id, t.google_id, l.google_id FROM tasks t JOIN task_lists l ON l.id = t.list_id WHERE t.id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        let Some((account_id, gid, list_gid)) = row else { continue };
        // Google deletes subtasks with their parent; do the same locally
        tx.execute("DELETE FROM tasks WHERE parent_id = ?1", [id])?;
        tx.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
        if let (Some(gid), Some(list_gid)) = (gid, list_gid) {
            pending::enqueue(&tx, account_id, DOMAIN, "delete", &json!({ "googleId": gid, "listGoogleId": list_gid }))?;
        }
        accounts.push(account_id);
    }
    tx.commit()?;
    Ok(accounts)
}

#[derive(Deserialize)]
struct CompleteParams {
    ids: Vec<i64>,
    complete: bool,
}

async fn set_complete(ctx: Arc<AppCtx>, p: CompleteParams) -> anyhow::Result<()> {
    let accounts = ctx
        .db
        .write(move |c| {
            let mut acc = Vec::new();
            for id in p.ids {
                let status = if p.complete { "completed" } else { "notStarted" };
                acc.push(update_task(c, UpdateParams { id, status: Some(status.into()), ..Default::default() })?);
            }
            Ok(acc)
        })
        .await?;
    changed(&ctx, accounts);
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveParams {
    pub ids: Vec<i64>,
    /// destination list (default: stay in the current list)
    pub list_id: Option<i64>,
    /// new parent (null = top level; absent = top level too)
    pub parent_id: Option<i64>,
    /// place after this sibling (absent/null = first)
    pub previous_id: Option<i64>,
}

async fn move_tasks(ctx: Arc<AppCtx>, p: MoveParams) -> anyhow::Result<Vec<i64>> {
    let (accounts, ids) = ctx.db.write(move |c| move_local(c, p)).await?;
    changed(&ctx, accounts);
    Ok(ids)
}

/// A copy of task `id` (all its fields, completed or not) in `list_id`, under `parent_id`, after
/// `previous_id`. Returns the new id.
fn copy_task(c: &mut Connection, id: i64, list_id: i64, parent_id: Option<i64>, previous_id: Option<i64>) -> anyhow::Result<i64> {
    let t = store::get_task(c, id)?.ok_or_else(|| crate::user_err!("not_found", "This item no longer exists."))?;
    let (new_id, _) = insert_task(
        c,
        CreateParams {
            list_id: Some(list_id),
            title: t.title,
            notes: t.notes,
            due: t.due,
            start: t.start,
            status: Some(t.status),
            importance: Some(t.importance),
            percent: Some(t.percent),
            reminder_at: t.reminder_at,
            categories: Some(t.categories),
            is_private: Some(t.is_private),
            parent_id,
            previous_id,
        },
    )?;
    Ok(new_id)
}

/// Move tasks (in order, each after the previous one). Moving to another account copies the task
/// (with its subtasks) there and deletes the original. Returns (touched accounts, resulting task ids).
pub fn move_local(c: &mut Connection, p: MoveParams) -> anyhow::Result<(Vec<i64>, Vec<i64>)> {
    let mut accounts = Vec::new();
    let mut out_ids = Vec::new();
    let mut previous = p.previous_id;
    // subtasks already moved to another account with their parent: old id → new id
    let mut copied: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for id in p.ids {
        if let Some(&nk) = copied.get(&id) {
            out_ids.push(nk);
            continue;
        }
        let tx = c.transaction()?;
        let row: Option<(i64, i64, Option<String>, Option<String>)> = tx
            .query_row(
                "SELECT t.account_id, t.list_id, t.google_id, l.google_id FROM tasks t JOIN task_lists l ON l.id = t.list_id WHERE t.id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .ok();
        let Some((account_id, from_list, gid, from_list_gid)) = row else { continue };
        let to_list = p.list_id.unwrap_or(from_list);
        let dest = store::get_list(&tx, to_list)?.ok_or_else(|| crate::user_err!("not_found", "task list {to_list} not found"))?;
        let parent = match p.parent_id {
            Some(pid) if pid != id => {
                let (pl, pp): (i64, Option<i64>) =
                    tx.query_row("SELECT list_id, parent_id FROM tasks WHERE id = ?1", [pid], |r| Ok((r.get(0)?, r.get(1)?)))?;
                if pl != to_list {
                    return Err(crate::user_err!("bad_params", "parent is in another list"));
                }
                Some(pp.unwrap_or(pid))
            }
            _ => None,
        };
        if previous == Some(id) {
            previous = store::previous_sibling(&tx, id)?;
        }
        if dest.account_id != account_id {
            tx.commit()?;
            // copy into the other account (subtasks too, in their order), then delete here
            let new_id = copy_task(c, id, to_list, None, previous)?;
            let mut prev_kid = None;
            for (kid, _) in store::siblings(c, from_list, Some(id), -1)? {
                let nk = copy_task(c, kid, to_list, Some(new_id), prev_kid)?;
                copied.insert(kid, nk);
                prev_kid = Some(nk);
            }
            delete_tasks(c, &[id])?;
            accounts.extend([account_id, dest.account_id]);
            out_ids.push(new_id);
            previous = Some(new_id);
            continue;
        }
        let pos = store::position_after(&tx, to_list, parent, previous, id)?;
        tx.execute(
            "UPDATE tasks SET list_id = ?1, parent_id = ?2, position = ?3, updated = ?4 WHERE id = ?5",
            params![to_list, parent, pos, now_ms(), id],
        )?;
        if to_list != from_list {
            // subtasks travel with their parent; a moved subtask's own children don't exist (1 level)
            tx.execute("UPDATE tasks SET list_id = ?1 WHERE parent_id = ?2", params![to_list, id])?;
        }
        if gid.is_some() {
            pending::enqueue(&tx, account_id, DOMAIN, "move", &json!({ "taskId": id, "fromListGoogleId": from_list_gid }))?;
        }
        tx.commit()?;
        accounts.push(account_id);
        out_ids.push(id);
        previous = Some(id);
    }
    Ok((accounts, out_ids))
}

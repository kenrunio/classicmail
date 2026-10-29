//! SQLite access for task lists and tasks (shared by the rpc handlers and sync).

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskListDto {
    pub id: i64,
    pub account_id: i64,
    pub account_email: String,
    pub account_name: String,
    pub google_id: Option<String>,
    pub title: String,
    pub is_default: bool,
    pub position: i64,
    /// tasks not yet completed
    pub active_count: i64,
    pub total_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDto {
    pub id: i64,
    pub account_id: i64,
    pub list_id: i64,
    pub list_title: String,
    pub google_id: Option<String>,
    pub parent_id: Option<i64>,
    pub position: String,
    pub title: String,
    pub notes: String,
    pub due: Option<String>,
    pub start: Option<String>,
    /// Outlook status: notStarted | inProgress | completed | waiting | deferred
    pub status: String,
    pub complete: bool,
    pub completed_at: Option<i64>,
    pub updated: i64,
    /// low | normal | high
    pub importance: String,
    pub percent: i64,
    pub reminder_at: Option<i64>,
    pub categories: Vec<String>,
    pub is_private: bool,
    pub hidden: bool,
}

pub fn importance_str(i: i64) -> &'static str {
    match i {
        0 => "low",
        2 => "high",
        _ => "normal",
    }
}

pub fn importance_num(s: &str) -> i64 {
    match s {
        "low" => 0,
        "high" => 2,
        _ => 1,
    }
}

pub const TASK_SELECT: &str = "SELECT t.id, t.account_id, t.list_id, l.title AS list_title, t.google_id, t.parent_id, t.position, \
     t.title, t.notes, t.due, t.start_date, t.status, t.outlook_status, t.completed_at, t.updated, t.importance, t.percent, \
     t.reminder_at, t.categories, t.private, t.hidden \
     FROM tasks t JOIN task_lists l ON l.id = t.list_id LEFT JOIN tasks p ON p.id = t.parent_id";

/// Tree order inside a list: each top-level task followed by its subtasks.
pub const TASK_ORDER: &str = "COALESCE(p.position, t.position), t.parent_id IS NOT NULL, t.position, t.id";

pub fn task_from_row(r: &Row) -> rusqlite::Result<TaskDto> {
    let status: String = r.get("status")?;
    let complete = status == "completed";
    let cats: String = r.get("categories")?;
    Ok(TaskDto {
        id: r.get("id")?,
        account_id: r.get("account_id")?,
        list_id: r.get("list_id")?,
        list_title: r.get("list_title")?,
        google_id: r.get("google_id")?,
        parent_id: r.get("parent_id")?,
        position: r.get("position")?,
        title: r.get("title")?,
        notes: r.get("notes")?,
        due: r.get("due")?,
        start: r.get("start_date")?,
        status: if complete { "completed".into() } else { r.get("outlook_status")? },
        complete,
        completed_at: r.get("completed_at")?,
        updated: r.get("updated")?,
        importance: importance_str(r.get("importance")?).into(),
        percent: if complete { 100 } else { r.get("percent")? },
        reminder_at: r.get("reminder_at")?,
        categories: serde_json::from_str(&cats).unwrap_or_default(),
        is_private: r.get::<_, i64>("private")? != 0,
        hidden: r.get::<_, i64>("hidden")? != 0,
    })
}

pub fn get_task(c: &Connection, id: i64) -> anyhow::Result<Option<TaskDto>> {
    Ok(c.query_row(&format!("{TASK_SELECT} WHERE t.id = ?1"), [id], task_from_row).optional()?)
}

pub fn list_lists(c: &Connection) -> anyhow::Result<Vec<TaskListDto>> {
    let mut st = c.prepare(
        "SELECT l.id, l.account_id, a.email, a.display_name, l.google_id, l.title, l.is_default, l.position, \
         (SELECT COUNT(*) FROM tasks t WHERE t.list_id = l.id AND t.status != 'completed') AS active, \
         (SELECT COUNT(*) FROM tasks t WHERE t.list_id = l.id) AS total \
         FROM task_lists l JOIN accounts a ON a.id = l.account_id WHERE a.enabled = 1 \
         ORDER BY a.position, a.id, l.is_default DESC, l.position, l.id",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok(TaskListDto {
                id: r.get(0)?,
                account_id: r.get(1)?,
                account_email: r.get(2)?,
                account_name: r.get(3)?,
                google_id: r.get(4)?,
                title: r.get(5)?,
                is_default: r.get::<_, i64>(6)? != 0,
                position: r.get(7)?,
                active_count: r.get(8)?,
                total_count: r.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The list new tasks go to when none is given: the first account's default list.
pub fn default_list(c: &Connection) -> anyhow::Result<Option<i64>> {
    Ok(c.query_row(
        "SELECT l.id FROM task_lists l JOIN accounts a ON a.id = l.account_id WHERE a.enabled = 1 \
             ORDER BY a.position, a.id, l.is_default DESC, l.position, l.id LIMIT 1",
        [],
        |r| r.get(0),
    )
    .optional()?)
}

pub struct ListRow {
    pub id: i64,
    pub account_id: i64,
    pub google_id: Option<String>,
    pub title: String,
    pub is_default: bool,
}

pub fn get_list(c: &Connection, id: i64) -> anyhow::Result<Option<ListRow>> {
    Ok(c.query_row("SELECT id, account_id, google_id, title, is_default FROM task_lists WHERE id = ?1", [id], |r| {
        Ok(ListRow { id: r.get(0)?, account_id: r.get(1)?, google_id: r.get(2)?, title: r.get(3)?, is_default: r.get::<_, i64>(4)? != 0 })
    })
    .optional()?)
}

// ---- positions --------------------------------------------------------------------------------

/// Spacing between locally generated positions (same scheme as Google's 20-digit strings).
pub const GAP: u128 = 1 << 32;

pub fn parse_pos(s: &str) -> u128 {
    s.trim().parse().unwrap_or(0)
}

pub fn fmt_pos(p: u128) -> String {
    format!("{p:020}")
}

/// Siblings (same list + parent) in order, as (id, position), excluding `exclude`.
pub fn siblings(c: &Connection, list_id: i64, parent_id: Option<i64>, exclude: i64) -> anyhow::Result<Vec<(i64, u128)>> {
    let mut st = c.prepare("SELECT id, position FROM tasks WHERE list_id = ?1 AND parent_id IS ?2 AND id != ?3 ORDER BY position, id")?;
    let mut v = st
        .query_map(params![list_id, parent_id, exclude], |r| Ok((r.get::<_, i64>(0)?, parse_pos(&r.get::<_, String>(1)?))))?
        .collect::<Result<Vec<_>, _>>()?;
    v.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    Ok(v)
}

/// A position right after `previous` (or first when None) among the siblings; renumbers the
/// siblings locally when there is no room left.
pub fn position_after(c: &Connection, list_id: i64, parent_id: Option<i64>, previous: Option<i64>, exclude: i64) -> anyhow::Result<String> {
    for attempt in 0..2 {
        let sibs = siblings(c, list_id, parent_id, exclude)?;
        let (lo, hi) = match previous.and_then(|p| sibs.iter().position(|s| s.0 == p)) {
            Some(i) => (sibs[i].1, sibs.get(i + 1).map(|s| s.1)),
            None => (0, sibs.first().map(|s| s.1)),
        };
        let pos = match hi {
            None => Some(lo + GAP),
            Some(h) if h > lo + 1 => Some(lo + (h - lo) / 2),
            _ => None,
        };
        if let Some(p) = pos {
            return Ok(fmt_pos(p));
        }
        if attempt == 0 {
            for (n, (id, _)) in sibs.iter().enumerate() {
                c.execute("UPDATE tasks SET position = ?1 WHERE id = ?2", params![fmt_pos((n as u128 + 1) * GAP), id])?;
            }
        }
    }
    anyhow::bail!("could not find a position")
}

/// The sibling just before `id` (same list + parent) in local order.
pub fn previous_sibling(c: &Connection, id: i64) -> anyhow::Result<Option<i64>> {
    let (list_id, parent_id): (i64, Option<i64>) =
        c.query_row("SELECT list_id, parent_id FROM tasks WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let sibs = siblings(c, list_id, parent_id, -1)?;
    Ok(sibs.iter().position(|s| s.0 == id).and_then(|i| if i > 0 { Some(sibs[i - 1].0) } else { None }))
}

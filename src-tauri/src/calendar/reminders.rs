//! Reminders: a background loop finds items whose reminder time has passed (and that weren't
//! dismissed / are not snoozed) and emits `reminder.due { items }` when new ones come due.
//! Dismiss / snooze state is local, per occurrence (event id + start).

use std::{
    collections::HashSet,
    sync::{Arc, OnceLock},
    time::Duration,
};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{ctx::AppCtx, db::util::now_ms, rpc::NoParams};

/// Overdue reminders are kept this long after the item started.
const KEEP_OVERDUE_MS: i64 = 6 * 3_600_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderItem {
    /// "<eventId>:<startMs>" for appointments, "task:<taskId>:<reminderAt>" for tasks
    pub key: String,
    /// "event" | "task"
    pub kind: String,
    pub event_id: i64,
    pub summary: String,
    pub location: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub reminder_at: i64,
    pub is_meeting: bool,
    pub calendar_color: String,
}

pub async fn due_items(ctx: &AppCtx) -> anyhow::Result<Vec<ReminderItem>> {
    let now = now_ms();
    ctx.db
        .read(move |c| {
            let mut st = c.prepare(
                "SELECT e.*, c.color AS cal_color FROM events e JOIN calendars c ON c.id=e.calendar_id JOIN accounts a ON a.id=e.account_id \
                 LEFT JOIN reminder_state rs ON rs.event_id=e.id AND rs.start_ms=e.start_ms \
                 WHERE a.enabled=1 AND c.access_role IN ('owner','writer') AND e.is_master=0 AND e.status!='cancelled' \
                 AND e.reminder_min IS NOT NULL AND (e.my_response IS NULL OR e.my_response!='declined') \
                 AND e.start_ms - e.reminder_min*60000 <= ?1 AND e.start_ms > ?2 \
                 AND COALESCE(rs.dismissed,0)=0 AND (rs.snooze_until IS NULL OR rs.snooze_until <= ?1) \
                 ORDER BY e.start_ms LIMIT 200",
            )?;
            let rows = st
                .query_map(params![now, now - KEEP_OVERDUE_MS], |r| {
                    let e = super::model::EventRow::from_row(r)?;
                    let color: String = r.get("cal_color")?;
                    Ok(ReminderItem {
                        key: format!("{}:{}", e.id, e.start_ms),
                        kind: "event".into(),
                        event_id: e.id,
                        is_meeting: e.is_meeting(),
                        summary: e.summary.clone(),
                        location: e.location.clone(),
                        start: e.start_ms,
                        end: e.end_ms,
                        all_day: e.all_day,
                        reminder_at: e.start_ms - e.reminder_min.unwrap_or(0) * 60_000,
                        calendar_color: color,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut rows = rows;
            rows.extend(task_items(c, now)?);
            rows.sort_by_key(|r| r.start);
            Ok(rows)
        })
        .await
}

/// Task reminders (the tasks domain stores `tasks.reminder_at`; dismiss/snooze is kept here).
fn task_items(c: &rusqlite::Connection, now: i64) -> anyhow::Result<Vec<ReminderItem>> {
    let has_tasks: bool =
        c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='tasks')", [], |r| r.get(0))?;
    if !has_tasks {
        return Ok(vec![]);
    }
    let mut st = c.prepare(
        "SELECT t.id, t.title, t.reminder_at FROM tasks t JOIN accounts a ON a.id=t.account_id          LEFT JOIN task_reminder_state rs ON rs.task_id=t.id AND rs.reminder_at=t.reminder_at          WHERE a.enabled=1 AND t.reminder_at IS NOT NULL AND t.reminder_at <= ?1 AND t.reminder_at > ?2 AND t.status != 'completed'          AND COALESCE(rs.dismissed,0)=0 AND (rs.snooze_until IS NULL OR rs.snooze_until <= ?1) ORDER BY t.reminder_at LIMIT 100",
    )?;
    let rows = st
        .query_map(params![now, now - 7 * 86_400_000], |r| {
            let id: i64 = r.get(0)?;
            let at: i64 = r.get(2)?;
            Ok(ReminderItem {
                key: format!("task:{id}:{at}"),
                kind: "task".into(),
                event_id: id,
                summary: r.get(1)?,
                location: String::new(),
                start: at,
                end: at,
                all_day: false,
                reminder_at: at,
                is_meeting: false,
                calendar_color: String::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

static STARTED: OnceLock<()> = OnceLock::new();

/// Start the reminder loop (once per process). Called lazily from calendar rpc methods; the
/// integrator may also call it at startup.
pub fn start_reminders(ctx: Arc<AppCtx>) {
    if ctx.app.is_none() || STARTED.set(()).is_err() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut last: HashSet<String> = HashSet::new();
        loop {
            match due_items(&ctx).await {
                Ok(items) => {
                    let keys: HashSet<String> = items.iter().map(|i| i.key.clone()).collect();
                    if keys.iter().any(|k| !last.contains(k)) {
                        ctx.events.emit("reminder.due", json!({ "items": items }));
                    }
                    last = keys;
                }
                Err(e) => log::warn!("reminders: {}", crate::logging::err(&e)),
            }
            // every 10 s so a reminder pops up close to its time (the query is cheap)
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
}

pub fn ensure_started(ctx: &Arc<AppCtx>) {
    if STARTED.get().is_none() {
        start_reminders(ctx.clone());
    }
}

pub async fn due_reminders(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<Vec<ReminderItem>> {
    ensure_started(&ctx);
    due_items(&ctx).await
}

#[derive(Deserialize)]
pub struct KeysParams {
    keys: Vec<String>,
    #[serde(default)]
    minutes: Option<i64>,
}

fn parse_key(k: &str) -> Option<(i64, i64)> {
    let (a, b) = k.split_once(':')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

async fn set_state(ctx: &AppCtx, keys: Vec<String>, dismissed: bool, snooze_until: Option<i64>) -> anyhow::Result<()> {
    ctx.db
        .write(move |c| {
            let tx = c.transaction()?;
            for k in keys {
                if let Some(rest) = k.strip_prefix("task:") {
                    let Some((id, at)) = parse_key(rest) else { continue };
                    tx.execute(
                        "INSERT INTO task_reminder_state(task_id, reminder_at, dismissed, snooze_until) VALUES (?1,?2,?3,?4)                          ON CONFLICT(task_id, reminder_at) DO UPDATE SET dismissed=excluded.dismissed, snooze_until=excluded.snooze_until",
                        params![id, at, dismissed as i64, snooze_until],
                    )?;
                    continue;
                }
                let Some((id, start)) = parse_key(&k) else { continue };
                let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE id=?1)", [id], |r| r.get(0))?;
                if !exists {
                    continue;
                }
                tx.execute(
                    "INSERT INTO reminder_state(event_id, start_ms, dismissed, snooze_until) VALUES (?1,?2,?3,?4) \
                     ON CONFLICT(event_id, start_ms) DO UPDATE SET dismissed=excluded.dismissed, snooze_until=excluded.snooze_until",
                    params![id, start, dismissed as i64, snooze_until],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    ctx.events.emit("calendar.changed", json!({ "reminders": true }));
    Ok(())
}

/// `{ keys }` — Dismiss / Dismiss All.
pub async fn dismiss(ctx: Arc<AppCtx>, p: KeysParams) -> anyhow::Result<()> {
    set_state(&ctx, p.keys, true, None).await
}

/// `{ keys, minutes }` — Snooze (Outlook's 5 minutes … 2 weeks; 0 = until the item starts).
pub async fn snooze(ctx: Arc<AppCtx>, p: KeysParams) -> anyhow::Result<()> {
    let minutes = p.minutes.unwrap_or(5).max(0);
    let now = now_ms();
    // "0 minutes before start" style snoozes are relative to each item's start
    if minutes == 0 {
        for k in p.keys {
            if let Some((_, start)) = parse_key(k.strip_prefix("task:").unwrap_or(&k)) {
                set_state(&ctx, vec![k], false, Some(start.max(now + 60_000))).await?;
            }
        }
        return Ok(());
    }
    set_state(&ctx, p.keys, false, Some(now + minutes * 60_000)).await
}

//! Calendar sync (design §4): push local pending ops first, then for every calendar replace the
//! local −12 / +24 month window with `events.list(singleEvents=true)` (plus the recurring
//! masters, so series details are available offline) — in full once a day, otherwise only the
//! events changed since the last pull. Rows with unpushed local changes are left alone until
//! their ops are pushed.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
};

use chrono::Duration;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::model::{CalRow, EventRow, get_cal};
use crate::{
    ctx::AppCtx,
    google::{
        Api,
        calendar::{self as gcal, Event, ListEvents},
    },
    sync::pending::{self, PendingOp},
};

pub const DOMAIN: &str = "calendar";

/// Window kept in sync (months back / ahead).
pub const MONTHS_BACK: i64 = 12;
pub const MONTHS_AHEAD: i64 = 24;

pub fn window() -> (i64, i64) {
    let now = crate::clock::now();
    ((now - Duration::days(MONTHS_BACK * 31)).timestamp_millis(), (now + Duration::days(MONTHS_AHEAD * 31)).timestamp_millis())
}

fn rfc(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---- pending op payloads ---------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateOp {
    pub event_id: i64,
    #[serde(default)]
    pub send_updates: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchOp {
    pub event_id: i64,
    /// "all" = every editable field, "response" = attendees only (my response)
    pub what: String,
    #[serde(default)]
    pub send_updates: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOp {
    pub calendar_gcal_id: String,
    pub gcal_id: String,
    #[serde(default)]
    pub send_updates: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveOp {
    pub event_id: i64,
    pub gcal_id: String,
    pub from_calendar_gcal_id: String,
}

/// Serialise pushes per account (the worker and the "push soon after an edit" task).
fn account_lock(account_id: i64) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<parking_lot::Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(Default::default).lock().entry(account_id).or_default().clone()
}

pub async fn sync_account(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let lock = account_lock(api.account_id);
    let _g = lock.lock().await;
    let account_id = api.account_id;
    ctx.events.status(account_id, "sync", "Sending calendar changes…", Some(0.1));
    push(ctx, api).await?;
    ctx.events.status(account_id, "sync", "Receiving calendars…", Some(0.2));
    let cals = sync_calendars(ctx, api).await?;
    let n = cals.len().max(1) as f64;
    let mut changed = 0;
    for (i, cal) in cals.iter().enumerate() {
        ctx.events.status(account_id, "sync", &format!("Receiving calendar: {}…", cal.summary), Some(0.3 + 0.7 * i as f64 / n));
        match pull_calendar(ctx, api, cal).await {
            Ok(c) => changed += c,
            // one subscribed calendar Google won't list (removed, access revoked) must not stop
            // the others from syncing
            Err(e) if crate::google::client::is_permanent(&e) => {
                // the local id: a Google calendar id is often an address
                log::warn!("calendar {} for account {account_id}: {}", cal.id, crate::logging::err(&e));
            }
            Err(e) => return Err(e),
        }
    }
    if changed > 0 {
        ctx.events.emit("calendar.changed", json!({ "accountId": account_id }));
    }
    Ok(())
}

/// Push pending ops now (after a local edit), then refresh calendars whose series changed.
pub async fn push_soon(ctx: Arc<AppCtx>, account_id: i64, pull_calendars: Vec<i64>) {
    let res: anyhow::Result<()> = async {
        let api = ctx.api(account_id).await?;
        let lock = account_lock(account_id);
        let _g = lock.lock().await;
        push(&ctx, &api).await?;
        for cid in pull_calendars {
            let cal = ctx.db.read(move |c| get_cal(c, cid)).await?;
            pull_calendar(&ctx, &api, &cal).await?;
        }
        ctx.events.emit("calendar.changed", json!({ "accountId": account_id }));
        Ok(())
    }
    .await;
    if let Err(e) = res {
        log::warn!("calendar push for account {account_id}: {}", crate::logging::err(&e));
    }
}

// ---- push ------------------------------------------------------------------------------------

pub async fn push(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let account_id = api.account_id;
    let dropped = pending::drain(&ctx.db, account_id, DOMAIN, |op| push_one(ctx, api, op)).await?;
    if dropped > 0 {
        ctx.events.status(account_id, "error", "Some calendar changes could not be saved to Google and were undone.", None);
    }
    reconcile(ctx, account_id).await
}

async fn load(ctx: &AppCtx, id: i64) -> anyhow::Result<Option<(EventRow, CalRow)>> {
    ctx.db
        .read(move |c| {
            let Some(row) = c.query_row("SELECT * FROM events WHERE id=?1", [id], EventRow::from_row).optional()? else { return Ok(None) };
            let cal = get_cal(c, row.calendar_id)?;
            Ok(Some((row, cal)))
        })
        .await
}

/// Store the server's copy of an event we just pushed (keeps local dirty state for reconcile).
async fn store_pushed(ctx: &AppCtx, mut row: EventRow, cal: CalRow, ev: Event, email: String) -> anyhow::Result<()> {
    ctx.db
        .write(move |c| {
            // the row may have been edited again meanwhile: keep local content, take server ids
            let cur = c.query_row("SELECT * FROM events WHERE id=?1", [row.id], EventRow::from_row).optional()?;
            let Some(cur) = cur else { return Ok(()) };
            let was_master = cur.is_master;
            let tx = c.transaction()?;
            if cur.updated_ms == row.updated_ms && cur.summary == row.summary && cur.start_ms == row.start_ms && cur.end_ms == row.end_ms {
                row.apply_google(&ev, &cal, &email);
                row.local_dirty = cur.local_dirty;
                // a series master stays a master even if Google echoes it without instances
                row.is_master = row.is_master || was_master;
                row.save(&tx)?;
            } else {
                tx.execute(
                    "UPDATE events SET gcal_id=?2, etag=?3, ical_uid=COALESCE(ical_uid, ?4), html_link=?5 WHERE id=?1",
                    params![row.id, ev.id, ev.etag, ev.ical_uid, ev.html_link],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
}

async fn push_one(ctx: &Arc<AppCtx>, api: &Api, op: PendingOp) -> anyhow::Result<()> {
    match op.kind.as_str() {
        "create" => {
            let p: CreateOp = op.payload()?;
            let Some((row, cal)) = load(ctx, p.event_id).await? else { return Ok(()) };
            if row.gcal_id.is_some() {
                return Ok(());
            }
            let send = p.send_updates.as_deref().unwrap_or("none");
            let mut body = row.to_google(&cal);
            if let Some(uid) = &row.ical_uid {
                body["iCalUID"] = json!(uid);
            }
            let ev = gcal::insert_event(api, &cal.gcal_id, body, send).await?;
            store_pushed(ctx, row, cal, ev, api.email.clone()).await
        }
        "patch" => {
            let p: PatchOp = op.payload()?;
            let Some((row, cal)) = load(ctx, p.event_id).await? else { return Ok(()) };
            let Some(gid) = row.gcal_id.clone() else { return Ok(()) }; // the create op will carry it
            let send = p.send_updates.as_deref().unwrap_or("none");
            let body: Value = if p.what == "response" {
                json!({ "attendees": super::model::attendees_body(&row.attendees) })
            } else if p.what == "cosmetic" {
                row.cosmetic_body()
            } else {
                row.to_google(&cal)
            };
            let ev = gcal::patch_event(api, &cal.gcal_id, &gid, body, send).await?;
            store_pushed(ctx, row, cal, ev, api.email.clone()).await
        }
        "move" => {
            // events.move to another calendar of the same account (the row already points there)
            let p: MoveOp = op.payload()?;
            let Some((row, cal)) = load(ctx, p.event_id).await? else { return Ok(()) };
            let ev = match gcal::move_event(api, &p.from_calendar_gcal_id, &p.gcal_id, &cal.gcal_id).await {
                Ok(ev) => ev,
                // already moved (a retry after a lost response)
                Err(e) if matches!(crate::google::client::api_status(&e), Some(404)) => {
                    gcal::get_event(api, &cal.gcal_id, &p.gcal_id).await?
                }
                Err(e) => return Err(e),
            };
            // later ops (a new time in the same drag) carry the local content: take only server ids
            let id = row.id;
            ctx.db
                .write(move |c| {
                    c.execute(
                        "UPDATE events SET etag=?2, html_link=COALESCE(?3, html_link) WHERE id=?1",
                        rusqlite::params![id, ev.etag, ev.html_link],
                    )?;
                    Ok(())
                })
                .await
        }
        "delete" => {
            let p: DeleteOp = op.payload()?;
            gcal::delete_event(api, &p.calendar_gcal_id, &p.gcal_id, p.send_updates.as_deref().unwrap_or("none")).await
        }
        other => {
            log::warn!("unknown calendar op {other}");
            Ok(())
        }
    }
}

/// After pushing: clear `local_dirty` on rows no op refers to any more, drop local creations
/// whose op was given up on, and drop placeholder instances of series that are now on the server.
async fn reconcile(ctx: &AppCtx, account_id: i64) -> anyhow::Result<()> {
    ctx.db
        .write(move |c| {
            let tx = c.transaction()?;
            tx.execute_batch("CREATE TEMP TABLE IF NOT EXISTS cal_pending_ids(id INTEGER PRIMARY KEY); DELETE FROM cal_pending_ids;")?;
            tx.execute(
                "INSERT OR IGNORE INTO cal_pending_ids(id) SELECT json_extract(payload,'$.eventId') FROM pending_ops \
                 WHERE account_id=?1 AND domain='calendar' AND json_extract(payload,'$.eventId') IS NOT NULL",
                [account_id],
            )?;
            tx.execute(
                "UPDATE events SET local_dirty=0 WHERE account_id=?1 AND local_dirty=1 AND gcal_id IS NOT NULL AND id NOT IN (SELECT id FROM cal_pending_ids)",
                [account_id],
            )?;
            // creations that were given up on
            tx.execute(
                "DELETE FROM events WHERE account_id=?1 AND gcal_id IS NULL AND parent_local_id IS NULL AND id NOT IN (SELECT id FROM cal_pending_ids)",
                [account_id],
            )?;
            // placeholders whose master is gone or already on the server and settled
            tx.execute(
                "DELETE FROM events WHERE account_id=?1 AND parent_local_id IS NOT NULL AND parent_local_id NOT IN \
                 (SELECT id FROM events WHERE account_id=?1 AND (gcal_id IS NULL OR local_dirty=1))",
                [account_id],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
}

// ---- pull ------------------------------------------------------------------------------------

async fn sync_calendars(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<Vec<CalRow>> {
    let list = gcal::list_calendars(api).await?;
    let account_id = api.account_id;
    let changed = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let mut seen = Vec::new();
            let mut changed = 0;
            for (pos, e) in list.iter().enumerate() {
                // hidden in Google Calendar, or only free/busy visible (events.list gives no details)
                if e.hidden == Some(true) || e.access_role == "freeBusyReader" {
                    continue;
                }
                seen.push(e.id.clone());
                let summary = e.summary_override.clone().unwrap_or_else(|| e.summary.clone());
                changed += tx.execute(
                    "INSERT INTO calendars(account_id, gcal_id, summary, description, color, fg_color, selected, access_role, is_primary, time_zone, default_reminders, position) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) \
                     ON CONFLICT(account_id, gcal_id) DO UPDATE SET summary=excluded.summary, description=excluded.description, color=excluded.color, \
                     fg_color=excluded.fg_color, access_role=excluded.access_role, is_primary=excluded.is_primary, time_zone=excluded.time_zone, \
                     default_reminders=excluded.default_reminders, position=excluded.position \
                     WHERE calendars.summary IS NOT excluded.summary OR calendars.color IS NOT excluded.color OR calendars.access_role IS NOT excluded.access_role \
                     OR calendars.default_reminders IS NOT excluded.default_reminders OR calendars.position IS NOT excluded.position \
                     OR calendars.time_zone IS NOT excluded.time_zone OR calendars.is_primary IS NOT excluded.is_primary \
                     OR calendars.description IS NOT excluded.description OR calendars.fg_color IS NOT excluded.fg_color",
                    params![
                        account_id,
                        e.id,
                        summary,
                        e.description.clone().unwrap_or_default(),
                        e.background_color.clone().unwrap_or_else(|| "#9fc6e7".into()),
                        e.foreground_color.clone().unwrap_or_else(|| "#000000".into()),
                        e.selected.unwrap_or(e.primary == Some(true)) as i64,
                        e.access_role,
                        (e.primary == Some(true)) as i64,
                        e.time_zone.clone().unwrap_or_else(|| "UTC".into()),
                        serde_json::to_string(&e.default_reminders)?,
                        pos as i64,
                    ],
                )?;
            }
            let existing: Vec<(i64, String)> = {
                let mut st = tx.prepare("SELECT id, gcal_id FROM calendars WHERE account_id=?1")?;
                st.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
            };
            for (id, gid) in existing {
                if !seen.contains(&gid) {
                    tx.execute("DELETE FROM calendars WHERE id=?1", [id])?;
                    changed += 1;
                }
            }
            tx.commit()?;
            Ok(changed)
        })
        .await?;
    if changed > 0 {
        ctx.events.emit("calendar.changed", json!({ "accountId": account_id }));
    }
    ctx.db
        .read(move |c| {
            let mut st = c.prepare("SELECT * FROM calendars WHERE account_id=?1 ORDER BY position")?;
            Ok(st.query_map([account_id], CalRow::from_row)?.collect::<Result<Vec<_>, _>>()?)
        })
        .await
}

/// Overlap for `updatedMin`, so a change Google makes visible a little late is still seen.
const UPDATED_MIN_MARGIN_MS: i64 = 5 * 60_000;

/// `sync_state` key holding "<start of the last pull, ms> <local date of the last full pull>".
fn pulled_key(cal_id: i64) -> String {
    format!("calendar.pulled:{cal_id}")
}

/// Calendars that had their full pull in this run of the app (database path + calendar id).
fn full_this_run() -> &'static parking_lot::Mutex<HashSet<String>> {
    static DONE: OnceLock<parking_lot::Mutex<HashSet<String>>> = OnceLock::new();
    DONE.get_or_init(Default::default)
}

/// Refresh one calendar's window. Once a day (and on the first pull of a run) the whole window is
/// listed and replaced; in between only what changed since the last pull (`updatedMin`, deleted
/// events included). Returns the number of rows changed.
pub async fn pull_calendar(ctx: &Arc<AppCtx>, api: &Api, cal: &CalRow) -> anyhow::Result<usize> {
    let started = crate::clock::now_ms();
    let today = crate::clock::today().format("%Y-%m-%d").to_string();
    let (account_id, key) = (cal.account_id, pulled_key(cal.id));
    let k2 = key.clone();
    let last = ctx.db.read(move |c| crate::db::util::get_sync_state(c, account_id, &k2)).await?;
    let last: Option<(i64, String)> =
        last.as_deref().and_then(|v| v.split_once(' ')).and_then(|(ms, d)| Some((ms.parse().ok()?, d.to_string())));
    let run_key = format!("{}|{}", ctx.db.path().display(), cal.id);
    let since = match &last {
        Some((ms, day)) if *day == today && full_this_run().lock().contains(&run_key) => Some(*ms - UPDATED_MIN_MARGIN_MS),
        _ => None,
    };
    let (wmin, wmax) = window();
    let p = ListEvents {
        time_min: Some(rfc(wmin)),
        time_max: Some(rfc(wmax)),
        single_events: true,
        show_deleted: since.is_some(),
        // to the millisecond, like Google's `updated`
        updated_min: since
            .map(|ms| chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        ..Default::default()
    };
    let instances = gcal::list_events(api, &cal.gcal_id, &p).await?;
    let masters: Vec<Event> = gcal::list_events(api, &cal.gcal_id, &ListEvents { single_events: false, ..p.clone() })
        .await?
        .into_iter()
        // a deleted series may come back as a bare tombstone (no recurrence)
        .filter(|e| e.recurrence.as_ref().is_some_and(|r| !r.is_empty()) || e.status.as_deref() == Some("cancelled"))
        .collect();
    let full = since.is_none();
    // a series that changed between full pulls (e.g. a new rule): re-read all its occurrences,
    // since the changes alone needn't say which old ones are gone
    let mut series = Vec::new();
    if !full {
        for m in masters.iter().filter(|m| m.status.as_deref() != Some("cancelled") && m.recurrence.is_some()) {
            let Some(mid) = m.id.clone() else { continue };
            match gcal::instances(api, &cal.gcal_id, &mid, &rfc(wmin), &rfc(wmax)).await {
                Ok(occ) => series.push((mid, occ)),
                // deleted meanwhile: the next pull brings its tombstone
                Err(e) if matches!(crate::google::client::api_status(&e), Some(404 | 410)) => {}
                Err(e) => return Err(e),
            }
        }
    }
    let cal = cal.clone();
    let email = api.email.clone();
    let full_day = if full { today } else { last.map(|l| l.1).unwrap_or(today) };
    let n = ctx
        .db
        .write(move |c| {
            let n = replace_window(c, &cal, &email, masters, instances, series, wmin, wmax, full)?;
            crate::db::util::set_sync_state(c, account_id, &key, Some(&format!("{started} {full_day}")))?;
            Ok(n)
        })
        .await?;
    if full {
        full_this_run().lock().insert(run_key);
    }
    Ok(n)
}

/// Store pulled events. `full`: they are the whole window, so synced rows in it that weren't
/// returned are deleted; otherwise they are only the changes, and deleted ones arrive `cancelled`.
/// `series`: (master id, all its occurrences in the window) for series that changed.
pub fn replace_window(
    c: &mut Connection,
    cal: &CalRow,
    email: &str,
    masters: Vec<Event>,
    instances: Vec<Event>,
    series: Vec<(String, Vec<Event>)>,
    wmin: i64,
    wmax: i64,
    full: bool,
) -> anyhow::Result<usize> {
    let tx = c.transaction()?;
    let mut changed = 0usize;
    let mut seen: HashSet<String> = HashSet::new();
    for ev in masters.iter().chain(instances.iter()).chain(series.iter().flat_map(|s| s.1.iter())) {
        let Some(gid) = ev.id.clone() else { continue };
        if ev.status.as_deref() == Some("cancelled") {
            // deleted on Google (a series takes its occurrences along); unpushed local edits stay
            changed += tx.execute(
                "DELETE FROM events WHERE calendar_id=?1 AND local_dirty=0 AND (gcal_id=?2 OR recurring_event_id=?2)",
                params![cal.id, gid],
            )?;
            continue;
        }
        seen.insert(gid.clone());
        // cached: one lookup per pulled event
        let existing = tx
            .prepare_cached("SELECT * FROM events WHERE calendar_id=?1 AND gcal_id=?2")?
            .query_row(params![cal.id, gid], EventRow::from_row)
            .optional()?;
        match existing {
            Some(row) if row.local_dirty => {}
            Some(mut row) => {
                // unchanged on Google: skip — unless what we keep of it changed (events synced before
                // the video-call details were kept get them now, without Google touching the event)
                if row.etag.is_some()
                    && row.etag == ev.etag
                    && row.reminder_min == super::model::effective_reminder(ev.reminders.as_ref(), cal)
                    && row.conference == super::conference::conference_text(ev.extra.get("conferenceData"), ev.hangout_link.as_deref())
                {
                    continue;
                }
                row.apply_google(ev, cal, email);
                row.save(&tx)?;
                changed += 1;
            }
            None => {
                let mut row = EventRow { account_id: cal.account_id, calendar_id: cal.id, ..Default::default() };
                row.apply_google(ev, cal, email);
                row.insert(&tx)?;
                changed += 1;
            }
        }
    }
    // a changed series: its occurrences in the window are exactly what Google listed for it
    for (master_gid, occ) in series {
        let keep: HashSet<&str> = occ.iter().filter_map(|e| e.id.as_deref()).collect();
        let old: Vec<(i64, String)> = {
            let mut st = tx.prepare_cached(
                "SELECT id, gcal_id FROM events WHERE calendar_id=?1 AND recurring_event_id=?2 AND local_dirty=0 \
                 AND gcal_id IS NOT NULL AND end_ms > ?3 AND start_ms < ?4",
            )?;
            st.query_map(params![cal.id, master_gid, wmin, wmax], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
        };
        for (id, gid) in old {
            if !keep.contains(gid.as_str()) {
                changed += tx.execute("DELETE FROM events WHERE id=?1", [id])?;
            }
        }
    }
    if !full {
        tx.commit()?;
        return Ok(changed);
    }
    // window replace: everything synced from the server in the window that the server no longer has
    let stale: Vec<(i64, String)> = {
        let mut st = tx.prepare(
            "SELECT id, gcal_id FROM events WHERE calendar_id=?1 AND gcal_id IS NOT NULL AND local_dirty=0 \
             AND (is_master=1 OR (end_ms > ?2 AND start_ms < ?3))",
        )?;
        st.query_map(params![cal.id, wmin, wmax], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
    };
    for (id, gid) in stale {
        if !seen.contains(&gid) {
            tx.execute("DELETE FROM events WHERE id=?1", [id])?;
            changed += 1;
        }
    }
    tx.commit()?;
    Ok(changed)
}

//! Calendar domain: calendars + events (Google Calendar), local-first edits, reminders.
//!
//! rpc: calendar.listCalendars, setCalendarSelected, listEvents, search, listSeries, getEvent, createEvent,
//! updateEvent, copyEvent, deleteEvent, respond, respondToInvite, resendInvite, findByIcalUid,
//! listCategories, saveCategories, setEventCategories, setOwnSummary, findDuplicates, deleteEvents, moveToCalendar,
//! dueReminders, dismissReminder, snoozeReminder. Events: `calendar.changed {accountId}`,
//! `reminder.due {items}`.

pub mod categories;
pub mod conference;
pub mod dupes;
pub mod invite;
pub mod model;
pub mod organize;
pub mod reminders;
pub mod rrule;
pub mod schema;
pub mod sync;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_organize;

use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use model::{CalEvent, CalRow, EventDetail, EventRow, attendee_dtos, get_cal, local_date_of, local_midnight_ms, to_cal_event};
use sync::{CreateOp, DeleteOp, PatchOp};

use crate::{
    ctx::AppCtx,
    db::util::now_ms,
    google::calendar::{Attendee, Reminder, Reminders},
    rpc::{NoParams, Registry},
    sync::pending,
};

pub use reminders::start_reminders;

pub fn register(r: &mut Registry) {
    r.add("calendar.listCalendars", list_calendars);
    r.add("calendar.setCalendarSelected", set_calendar_selected);
    r.add("calendar.listEvents", list_events);
    r.add("calendar.search", search);
    r.add("calendar.listSeries", list_series);
    r.add("calendar.getEvent", get_event);
    r.add("calendar.createEvent", create_event);
    r.add("calendar.updateEvent", update_event);
    r.add("calendar.copyEvent", copy_event);
    r.add("calendar.deleteEvent", delete_event);
    r.add("calendar.respond", respond);
    r.add("calendar.respondToInvite", respond_to_invite);
    r.add("calendar.resendInvite", invite::resend_invite);
    r.add("calendar.findByIcalUid", find_by_ical_uid);
    r.add("calendar.listCategories", organize::list_categories);
    r.add("calendar.saveCategories", organize::save_categories);
    r.add("calendar.setEventCategories", organize::set_event_categories);
    r.add("calendar.setOwnSummary", organize::set_own_summary);
    r.add("calendar.findDuplicates", organize::find_duplicates);
    r.add("calendar.deleteEvents", organize::delete_events);
    r.add("calendar.moveToCalendar", organize::move_to_calendar);
    r.add("calendar.dueReminders", reminders::due_reminders);
    r.add("calendar.dismissReminder", reminders::dismiss);
    r.add("calendar.snoozeReminder", reminders::snooze);
}

pub(crate) fn not_found(id: i64) -> anyhow::Error {
    crate::user_err!("not_found", "The item could not be found. It may have been moved or deleted (id {id}).")
}

pub(crate) fn read_only() -> anyhow::Error {
    crate::user_err!("read_only", "You do not have sufficient permission to perform this operation on this object.")
}

/// Push local changes soon (and refresh `pull` calendars afterwards). No-op in tests.
pub(crate) fn kick(ctx: &Arc<AppCtx>, account_id: i64, pull: Vec<i64>) {
    if ctx.app.is_none() {
        return;
    }
    let ctx = ctx.clone();
    tauri::async_runtime::spawn(async move { sync::push_soon(ctx, account_id, pull).await });
}

pub(crate) fn changed(ctx: &AppCtx, account_id: i64) {
    ctx.events.emit("calendar.changed", json!({ "accountId": account_id }));
}

// ---- calendars -------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarDto {
    pub id: i64,
    pub account_id: i64,
    pub account_email: String,
    pub account_name: String,
    pub name: String,
    pub color: String,
    pub selected: bool,
    pub access_role: String,
    pub is_primary: bool,
    pub can_write: bool,
    pub time_zone: String,
    /// "mine" | "shared" | "other" (My / Shared / Other Calendars)
    pub group: String,
}

/// How a calendar is named outside the navigation pane ("Calendar - you@example.com").
pub fn calendar_label(is_primary: bool, name: &str, account_email: &str) -> String {
    if is_primary { format!("Calendar - {account_email}") } else { format!("{name} - {account_email}") }
}

async fn list_calendars(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<Vec<CalendarDto>> {
    ctx.db
        .read(|c| {
            let mut st = c.prepare(
                "SELECT c.*, a.email AS a_email, a.display_name AS a_name FROM calendars c JOIN accounts a ON a.id=c.account_id \
                 WHERE a.enabled=1 ORDER BY a.position, a.id, c.is_primary DESC, c.position",
            )?;
            let rows = st
                .query_map([], |r| {
                    let cal = CalRow::from_row(r)?;
                    Ok(CalendarDto {
                        id: cal.id,
                        account_id: cal.account_id,
                        account_email: r.get("a_email")?,
                        account_name: r.get("a_name")?,
                        name: cal.summary.clone(),
                        color: cal.color.clone(),
                        selected: cal.selected,
                        access_role: cal.access_role.clone(),
                        is_primary: cal.is_primary,
                        can_write: cal.can_write(),
                        time_zone: cal.time_zone.clone(),
                        group: dupes::classify(&cal.gcal_id, &cal.access_role, cal.is_primary, &r.get::<_, String>("a_email")?).to_string(),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
}

#[derive(Deserialize)]
struct SelParams {
    id: i64,
    selected: bool,
}

async fn set_calendar_selected(ctx: Arc<AppCtx>, p: SelParams) -> anyhow::Result<()> {
    let acct = ctx
        .db
        .write(move |c| {
            c.execute("UPDATE calendars SET selected=?1 WHERE id=?2", params![p.selected as i64, p.id])?;
            Ok(c.query_row("SELECT account_id FROM calendars WHERE id=?1", [p.id], |r| r.get::<_, i64>(0)).optional()?)
        })
        .await?;
    if let Some(a) = acct {
        changed(&ctx, a);
    }
    Ok(())
}

// ---- reading ---------------------------------------------------------------------------------

const SELECT_EV: &str = "SELECT e.*, c.color AS cal_color, c.access_role AS cal_role FROM events e JOIN calendars c ON c.id=e.calendar_id JOIN accounts a ON a.id=e.account_id";

fn read_cal_event(r: &rusqlite::Row) -> rusqlite::Result<CalEvent> {
    let row = EventRow::from_row(r)?;
    let color: String = r.get("cal_color")?;
    let role: String = r.get("cal_role")?;
    Ok(to_cal_event(&row, &color, !matches!(role.as_str(), "owner" | "writer")))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListParams {
    start: i64,
    end: i64,
    calendar_ids: Option<Vec<i64>>,
    /// merge the same event shown from several calendars (default: setting calendar.mergeDuplicates, on)
    dedupe: Option<bool>,
}

async fn list_events(ctx: Arc<AppCtx>, p: ListParams) -> anyhow::Result<Vec<CalEvent>> {
    reminders::ensure_started(&ctx);
    ctx.db
        .read(move |c| {
            let cal_filter = match &p.calendar_ids {
                Some(ids) if !ids.is_empty() => format!("c.id IN ({})", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")),
                Some(_) => "0".to_string(),
                None => "c.selected=1".to_string(),
            };
            let sql = format!(
                "{SELECT_EV} WHERE a.enabled=1 AND {cal_filter} AND e.is_master=0 AND e.status!='cancelled' \
                 AND (e.my_response IS NULL OR e.my_response!='declined') \
                 AND ((e.end_ms > ?1 AND e.start_ms < ?2) OR (e.start_ms = e.end_ms AND e.start_ms >= ?1 AND e.start_ms < ?2)) \
                 ORDER BY e.all_day DESC, e.start_ms, e.end_ms DESC, e.id"
            );
            let mut st = c.prepare(&sql)?;
            let rows = st
                .query_map(params![p.start, p.end], |r| Ok((read_cal_event(r)?, r.get::<_, Option<String>>("ical_uid")?)))?
                .collect::<Result<Vec<_>, _>>()?;
            let merge = match p.dedupe {
                Some(d) => d,
                None => crate::db::util::get_setting::<bool>(c, "calendar.mergeDuplicates")?.unwrap_or(true),
            };
            if !merge {
                return Ok(rows.into_iter().map(|x| x.0).collect());
            }
            // calendars in display order: the first listed one keeps the item
            let mut st = c.prepare(
                "SELECT c.id, c.summary, c.is_primary, a.email FROM calendars c JOIN accounts a ON a.id=c.account_id                  ORDER BY a.position, a.id, c.is_primary DESC, c.position",
            )?;
            let cals: Vec<(i64, String, bool, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0, r.get(3)?)))?.collect::<Result<_, _>>()?;
            let rank: std::collections::HashMap<i64, usize> = cals.iter().enumerate().map(|(i, c)| (c.0, i)).collect();
            let ical: std::collections::HashMap<i64, String> = rows.iter().filter_map(|(e, u)| u.clone().map(|u| (e.id, u))).collect();
            let mut evs: Vec<CalEvent> = rows.into_iter().map(|x| x.0).collect();
            evs.sort_by_key(|e| rank.get(&e.calendar_id).copied().unwrap_or(usize::MAX));
            let label = |id: i64| cals.iter().find(|c| c.0 == id).map(|c| calendar_label(c.2, &c.1, &c.3)).unwrap_or_default();
            let mut out = dupes::merge_display(evs, &ical, &label);
            out.sort_by(|a, b| b.all_day.cmp(&a.all_day).then(a.start.cmp(&b.start)).then(b.end.cmp(&a.end)).then(a.id.cmp(&b.id)));
            Ok(out)
        })
        .await
}

#[derive(Deserialize)]
struct SearchParams {
    query: String,
    limit: Option<i64>,
}

/// "Search Calendar": subject / location / notes contain every word.
async fn search(ctx: Arc<AppCtx>, p: SearchParams) -> anyhow::Result<Vec<CalEvent>> {
    ctx.db
        .read(move |c| {
            // a typed % or _ is searched for literally
            let words: Vec<String> = p
                .query
                .split_whitespace()
                .map(|w| format!("%{}%", w.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")))
                .collect();
            if words.is_empty() {
                return Ok(vec![]);
            }
            let conds: Vec<String> = (0..words.len())
                .map(|i| {
                    format!(
                        "(e.summary LIKE ?{n} ESCAPE '\\' OR e.location LIKE ?{n} ESCAPE '\\' OR e.description LIKE ?{n} ESCAPE '\\' \
                         OR e.attendees LIKE ?{n} ESCAPE '\\')",
                        n = i + 1
                    )
                })
                .collect();
            let sql = format!(
                "{SELECT_EV} WHERE a.enabled=1 AND c.selected=1 AND e.is_master=0 AND e.status!='cancelled' AND {} ORDER BY e.start_ms DESC LIMIT {}",
                conds.join(" AND "),
                p.limit.unwrap_or(500).clamp(1, 5000)
            );
            let mut st = c.prepare(&sql)?;
            let rows = st
                .query_map(rusqlite::params_from_iter(words.iter()), |r| {
                    let row = EventRow::from_row(r)?;
                    let key = match (&row.recurring_event_id, row.parent_local_id) {
                        (Some(s), _) => format!("{}:{s}", row.calendar_id),
                        (None, Some(p)) => format!("p:{p}"),
                        _ => format!("e:{}", row.id),
                    };
                    Ok((key, read_cal_event(r)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            // a recurring series appears once, as its occurrence nearest to today
            let now = now_ms();
            let mut best: std::collections::HashMap<String, CalEvent> = std::collections::HashMap::new();
            let mut order: Vec<String> = Vec::new();
            for (k, ev) in rows {
                match best.get(&k) {
                    Some(cur) if (cur.start - now).abs() <= (ev.start - now).abs() => {}
                    Some(_) => {
                        best.insert(k, ev);
                    }
                    None => {
                        order.push(k.clone());
                        best.insert(k, ev);
                    }
                }
            }
            Ok(order.into_iter().filter_map(|k| best.remove(&k)).collect())
        })
        .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SeriesItem {
    #[serde(flatten)]
    ev: CalEvent,
    recurrence: Vec<String>,
}

/// "View Recurrences": every recurring series (masters) on the selected calendars.
async fn list_series(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<Vec<SeriesItem>> {
    ctx.db
        .read(|c| {
            let sql = format!("{SELECT_EV} WHERE a.enabled=1 AND c.selected=1 AND e.is_master=1 AND e.status!='cancelled' ORDER BY e.summary COLLATE NOCASE, e.start_ms");
            let mut st = c.prepare(&sql)?;
            let rows = st
                .query_map([], |r| {
                    let ev = read_cal_event(r)?;
                    let row = EventRow::from_row(r)?;
                    Ok(SeriesItem { ev, recurrence: row.recurrence.unwrap_or_default() })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
}

pub(crate) fn load_row(c: &Connection, id: i64) -> anyhow::Result<EventRow> {
    c.query_row("SELECT * FROM events WHERE id=?1", [id], EventRow::from_row).optional()?.ok_or_else(|| not_found(id))
}

/// The series master of an instance (or the row itself when it is a master).
pub(crate) fn find_master(c: &Connection, row: &EventRow) -> anyhow::Result<Option<EventRow>> {
    if row.is_master {
        return Ok(Some(row.clone()));
    }
    if let Some(pid) = row.parent_local_id {
        return Ok(c.query_row("SELECT * FROM events WHERE id=?1", [pid], EventRow::from_row).optional()?);
    }
    if let Some(rid) = &row.recurring_event_id {
        return Ok(c
            .query_row("SELECT * FROM events WHERE calendar_id=?1 AND gcal_id=?2", params![row.calendar_id, rid], EventRow::from_row)
            .optional()?);
    }
    Ok(None)
}

pub(crate) fn detail(c: &Connection, row: &EventRow) -> anyhow::Result<EventDetail> {
    let cal = get_cal(c, row.calendar_id)?;
    let email: String = c.query_row("SELECT email FROM accounts WHERE id=?1", [row.account_id], |r| r.get(0))?;
    let master = find_master(c, row)?;
    let pending: bool = row.local_dirty || row.gcal_id.is_none();
    Ok(EventDetail {
        ev: to_cal_event(row, &cal.color, !cal.can_write()),
        description: row.description.clone(),
        conference: row.conference.clone(),
        attendees: attendee_dtos(&row.attendees),
        organizer_email: row.organizer_email.clone(),
        organizer_name: row.organizer_name.clone(),
        reminder_minutes: row.reminder_min,
        visibility: row.visibility.clone(),
        html_link: row.html_link.clone(),
        time_zone: row.time_zone.clone(),
        calendar_name: cal.summary.clone(),
        account_email: email,
        series_id: master.as_ref().map(|m| m.id),
        recurrence: master.as_ref().and_then(|m| m.recurrence.clone()),
        series_start: master.as_ref().map(|m| m.start_ms),
        series_end: master.as_ref().map(|m| m.end_ms),
        original_start: row.original_start_ms,
        is_master: row.is_master,
        pending,
    })
}

#[derive(Deserialize)]
pub(crate) struct IdParams {
    pub(crate) id: i64,
}

pub(crate) async fn get_event(ctx: Arc<AppCtx>, p: IdParams) -> anyhow::Result<EventDetail> {
    ctx.db
        .read(move |c| {
            let row = load_row(c, p.id)?;
            detail(c, &row)
        })
        .await
}

// ---- writing ---------------------------------------------------------------------------------

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AttendeeIn {
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub optional: Option<bool>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EventInput {
    pub calendar_id: Option<i64>,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub all_day: Option<bool>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub attendees: Option<Vec<AttendeeIn>>,
    /// minutes before start; -1 = no reminder
    pub reminder_minutes: Option<i64>,
    pub busy_status: Option<String>,
    pub is_private: Option<bool>,
    pub importance: Option<String>,
    /// RRULE / EXDATE lines; empty = not recurring
    pub recurrence: Option<Vec<String>>,
    /// send invitations / updates to attendees
    pub send_updates: Option<bool>,
    /// colour categories (names); also sets the Google colour
    pub categories: Option<Vec<String>>,
    /// the time zone the start and end were set in (IANA, "America/New_York"); default the calendar's
    pub time_zone: Option<String>,
}

pub(crate) fn add_days(d: &str, n: i64) -> String {
    chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
        .map(|x| (x + chrono::Duration::days(n)).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| d.to_string())
}

/// Apply user input to a row (times, fields, attendees, reminder...). Does not touch recurrence.
fn apply_input(row: &mut EventRow, p: &EventInput, cal: &CalRow, my_email: &str, my_name: &str, defs: &[categories::Category]) {
    if let Some(cats) = &p.categories {
        let cats = categories::normalise(cats);
        row.color_id = categories::next_color_id(&row.categories, &cats, row.color_id.as_deref(), defs);
        row.categories = cats;
    }
    if let Some(s) = &p.summary {
        row.summary = s.clone();
    }
    if let Some(s) = &p.location {
        row.location = s.clone();
    }
    if let Some(s) = &p.description {
        row.description = s.clone();
    }
    let all_day = p.all_day.unwrap_or(row.all_day);
    if all_day {
        let sd = p
            .start_date
            .clone()
            .or_else(|| p.start.map(local_date_of))
            .or_else(|| row.start_date.clone())
            .unwrap_or_else(|| local_date_of(row.start_ms));
        let mut ed = p
            .end_date
            .clone()
            .or_else(|| {
                p.end.map(|e| {
                    // an end exactly at midnight is exclusive; otherwise the item runs through that day
                    let d = local_date_of(e);
                    if local_midnight_ms(&d) == Some(e) { d } else { add_days(&d, 1) }
                })
            })
            .or_else(|| if row.all_day { row.end_date.clone() } else { None })
            .unwrap_or_else(|| add_days(&sd, 1));
        if ed <= sd {
            ed = add_days(&sd, 1);
        }
        row.start_ms = local_midnight_ms(&sd).unwrap_or(row.start_ms);
        row.end_ms = local_midnight_ms(&ed).unwrap_or(row.start_ms + model::DAY_MS);
        row.start_date = Some(sd);
        row.end_date = Some(ed);
        row.all_day = true;
    } else {
        if let Some(s) = p.start {
            row.start_ms = s;
        }
        if let Some(e) = p.end {
            row.end_ms = e;
        }
        if row.end_ms < row.start_ms {
            row.end_ms = row.start_ms;
        }
        row.all_day = false;
        row.start_date = None;
        row.end_date = None;
        if let Some(tz) = p.time_zone.as_deref().map(str::trim).filter(|t| t.parse::<chrono_tz::Tz>().is_ok()) {
            row.time_zone = Some(tz.to_string());
        } else if row.time_zone.is_none() {
            row.time_zone = Some(cal.time_zone.clone());
        }
    }
    if let Some(list) = &p.attendees {
        let old = std::mem::take(&mut row.attendees);
        let mut out: Vec<Attendee> = Vec::new();
        for a in list {
            let email = a.email.trim().to_string();
            if email.is_empty() || out.iter().any(|x| x.email.eq_ignore_ascii_case(&email)) {
                continue;
            }
            let prev = old.iter().find(|x| x.email.eq_ignore_ascii_case(&email));
            out.push(Attendee {
                email: email.clone(),
                display_name: a.name.clone().filter(|n| !n.is_empty()).or_else(|| prev.and_then(|x| x.display_name.clone())),
                optional: a.optional.filter(|o| *o),
                response_status: prev.map(|x| x.response_status.clone()).unwrap_or_else(|| "needsAction".into()),
                organizer: prev.and_then(|x| x.organizer),
                is_self: email.eq_ignore_ascii_case(my_email).then_some(true),
                comment: prev.and_then(|x| x.comment.clone()),
                resource: None,
            });
        }
        // keep the organizer (and me) in the list, as Google does
        if !out.is_empty() {
            if let Some(org) = old.iter().find(|x| x.organizer == Some(true)) {
                if !out.iter().any(|x| x.email.eq_ignore_ascii_case(&org.email)) {
                    out.insert(0, org.clone());
                }
            } else if row.organizer_self && !out.iter().any(|x| x.email.eq_ignore_ascii_case(my_email)) {
                out.insert(
                    0,
                    Attendee {
                        email: my_email.into(),
                        display_name: Some(my_name.into()),
                        organizer: Some(true),
                        is_self: Some(true),
                        response_status: "accepted".into(),
                        ..Default::default()
                    },
                );
            }
            if !row.organizer_self
                && !out.iter().any(|x| x.is_self == Some(true))
                && let Some(me) = old.iter().find(|x| x.is_self == Some(true))
            {
                out.push(me.clone());
            }
        }
        row.attendees = out;
    }
    if let Some(m) = p.reminder_minutes {
        row.reminders = Some(if m < 0 {
            Reminders { use_default: false, overrides: Some(vec![]) }
        } else {
            Reminders { use_default: false, overrides: Some(vec![Reminder { method: "popup".into(), minutes: m }]) }
        });
    }
    row.reminder_min = model::effective_reminder(row.reminders.as_ref(), cal);
    if let Some(b) = &p.busy_status
        && matches!(b.as_str(), "busy" | "free" | "tentative" | "oof")
    {
        row.show_as = Some(b.clone());
        row.transparency = if b == "free" { "transparent".into() } else { "opaque".into() };
    }
    if let Some(pv) = p.is_private {
        row.visibility = if pv { "private".into() } else { "default".into() };
    }
    if let Some(i) = &p.importance {
        row.importance = Some(i.clone());
    }
    row.updated_ms = Some(now_ms());
    row.local_dirty = true;
}

/// Local placeholder instances of a series that isn't on the server yet (or whose rule changed),
/// so the calendar shows it immediately. Replaced by real instances after the next pull.
pub(crate) fn make_placeholders(c: &Connection, master: &EventRow, cal: &CalRow) -> anyhow::Result<()> {
    let Some(rec) = master.recurrence.as_ref().filter(|r| !r.is_empty()) else { return Ok(()) };
    let (wmin, wmax) = sync::window();
    let tz: chrono_tz::Tz = master.time_zone.as_deref().unwrap_or(&cal.time_zone).parse().unwrap_or(cal.tz());
    let dur = master.end_ms - master.start_ms;
    let (dtstart, all_day) = if master.all_day {
        let sd = master.start_date.clone().unwrap_or_else(|| local_date_of(master.start_ms));
        let ms = chrono::NaiveDate::parse_from_str(&sd, "%Y-%m-%d")?.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis();
        (ms, true)
    } else {
        (master.start_ms, false)
    };
    let occ = rrule::expand_recurrence_from(rec, dtstart, all_day, tz, Some(wmin), wmax, 1000);
    for o in occ {
        let mut inst = master.clone();
        inst.is_master = false;
        inst.recurrence = None;
        inst.gcal_id = None;
        inst.parent_local_id = Some(master.id);
        inst.local_dirty = false;
        if all_day {
            let sd = chrono::DateTime::from_timestamp_millis(o).unwrap().format("%Y-%m-%d").to_string();
            let days = ((master.end_ms - master.start_ms) as f64 / model::DAY_MS as f64).round().max(1.0) as i64;
            let ed = add_days(&sd, days);
            inst.start_ms = local_midnight_ms(&sd).unwrap_or(o);
            inst.end_ms = local_midnight_ms(&ed).unwrap_or(o + model::DAY_MS);
            inst.start_date = Some(sd);
            inst.end_date = Some(ed);
        } else {
            inst.start_ms = o;
            inst.end_ms = o + dur;
        }
        if inst.end_ms < wmin {
            continue;
        }
        inst.original_start_ms = Some(inst.start_ms);
        inst.insert(c)?;
    }
    Ok(())
}

/// Remove a series' local instances (synced, unmodified ones and placeholders).
pub(crate) fn drop_instances(c: &Connection, master: &EventRow) -> anyhow::Result<()> {
    c.execute("DELETE FROM events WHERE parent_local_id=?1", [master.id])?;
    if let Some(g) = &master.gcal_id {
        c.execute("DELETE FROM events WHERE calendar_id=?1 AND recurring_event_id=?2", params![master.calendar_id, g])?;
    }
    Ok(())
}

pub(crate) fn account_identity(c: &Connection, account_id: i64) -> anyhow::Result<(String, String)> {
    Ok(c.query_row("SELECT email, display_name FROM accounts WHERE id=?1", [account_id], |r| Ok((r.get(0)?, r.get(1)?)))?)
}

pub(crate) fn send_mode(send: Option<bool>, row: &EventRow) -> String {
    if send.unwrap_or(true) && row.attendees.iter().any(|a| a.is_self != Some(true)) { "all".into() } else { "none".into() }
}

fn default_calendar(c: &Connection) -> anyhow::Result<CalRow> {
    c.query_row(
        "SELECT c.* FROM calendars c JOIN accounts a ON a.id=c.account_id WHERE a.enabled=1 AND c.access_role IN ('owner','writer') \
         ORDER BY c.is_primary DESC, a.position, a.id, c.position LIMIT 1",
        [],
        CalRow::from_row,
    )
    .optional()?
    .ok_or_else(|| crate::user_err!("no_calendar", "There is no calendar to save this item in yet. Wait for Send/Receive to finish."))
}

pub(crate) async fn create_event(ctx: Arc<AppCtx>, p: EventInput) -> anyhow::Result<EventDetail> {
    let (d, account_id) = ctx
        .db
        .write(move |c| {
            let cal = match p.calendar_id {
                Some(id) => get_cal(c, id)?,
                None => default_calendar(c)?,
            };
            if !cal.can_write() {
                return Err(read_only());
            }
            let (email, name) = account_identity(c, cal.account_id)?;
            let mut row = EventRow {
                account_id: cal.account_id,
                calendar_id: cal.id,
                status: "confirmed".into(),
                organizer_self: true,
                organizer_email: Some(if cal.is_primary { email.clone() } else { cal.gcal_id.clone() }),
                organizer_name: Some(if cal.is_primary { name.clone() } else { cal.summary.clone() }),
                transparency: "opaque".into(),
                visibility: "default".into(),
                event_type: "default".into(),
                start_ms: now_ms(),
                end_ms: now_ms() + 30 * 60_000,
                reminders: Some(Reminders { use_default: true, overrides: None }),
                ..Default::default()
            };
            apply_input(&mut row, &p, &cal, &email, &name, &categories::load(c)?);
            if let Some(rec) = p.recurrence.as_ref().filter(|r| !r.is_empty()) {
                for line in rec.iter().filter(|l| l.starts_with("RRULE")) {
                    rrule::Rule::parse(line).map_err(|e| crate::user_err!("bad_recurrence", "{e}"))?;
                }
                row.recurrence = Some(rec.clone());
                row.is_master = true;
            }
            let tx = c.transaction()?;
            row.id = row.insert(&tx)?;
            if row.is_master {
                make_placeholders(&tx, &row, &cal)?;
            }
            pending::enqueue(
                &tx,
                cal.account_id,
                sync::DOMAIN,
                "create",
                &CreateOp { event_id: row.id, send_updates: Some(send_mode(p.send_updates, &row)) },
            )?;
            let out = detail(&tx, &row)?;
            tx.commit()?;
            Ok((out, cal.account_id))
        })
        .await?;
    changed(&ctx, account_id);
    let pull = if d.is_master { vec![d.ev.calendar_id] } else { vec![] };
    kick(&ctx, account_id, pull);
    // a new master isn't listed itself: hand back its first occurrence when there is one
    if d.is_master {
        let mid = d.ev.id;
        if let Some(first) = ctx
            .db
            .read(move |c| {
                Ok(c.query_row("SELECT id FROM events WHERE parent_local_id=?1 ORDER BY start_ms LIMIT 1", [mid], |r| r.get::<_, i64>(0))
                    .optional()?)
            })
            .await?
        {
            return get_event(ctx, IdParams { id: first }).await;
        }
    }
    Ok(d)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateParams {
    pub(crate) id: i64,
    /// "occurrence" (default) | "series"
    pub(crate) scope: Option<String>,
    #[serde(flatten)]
    pub(crate) input: EventInput,
}

pub(crate) async fn update_event(ctx: Arc<AppCtx>, p: UpdateParams) -> anyhow::Result<EventDetail> {
    let (d, account_id, pull) = ctx
        .db
        .write(move |c| {
            let row = load_row(c, p.id)?;
            let cal = get_cal(c, row.calendar_id)?;
            if !cal.can_write() || row.type_read_only() {
                return Err(read_only());
            }
            let (email, name) = account_identity(c, row.account_id)?;
            let defs = categories::load(c)?;
            let series = p.scope.as_deref() == Some("series") || row.is_master;
            let tx = c.transaction()?;
            let mut pull = vec![];

            let result_id = if series && (row.is_master || row.recurring_event_id.is_some() || row.parent_local_id.is_some()) {
                let mut master = find_master(&tx, &row)?.ok_or_else(|| {
                    crate::user_err!("not_found", "The series of this item could not be found. Try again after Send/Receive.")
                })?;
                let mut input = p.input.clone();
                if !row.is_master {
                    // shift the series by the change made to this occurrence
                    if let (Some(s), Some(e)) = (input.start, input.end) {
                        let ns = master.start_ms + (s - row.start_ms);
                        input.start = Some(ns);
                        input.end = Some(ns + (e - s));
                    }
                    if input.all_day == Some(true) || (input.all_day.is_none() && master.all_day) {
                        input.start_date = None;
                        input.end_date = None;
                        input.start = None;
                        input.end = None;
                    }
                }
                let before = (master.start_ms, master.end_ms, master.all_day, master.recurrence.clone());
                apply_input(&mut master, &input, &cal, &email, &name, &defs);
                if let Some(rec) = &input.recurrence {
                    for line in rec.iter().filter(|l| l.starts_with("RRULE")) {
                        rrule::Rule::parse(line).map_err(|e| crate::user_err!("bad_recurrence", "{e}"))?;
                    }
                    if rec.is_empty() {
                        master.recurrence = Some(vec![]);
                        master.is_master = false;
                    } else {
                        master.recurrence = Some(rec.clone());
                    }
                }
                let reshaped = before != (master.start_ms, master.end_ms, master.all_day, master.recurrence.clone());
                master.save(&tx)?;
                if reshaped {
                    drop_instances(&tx, &master)?;
                    // a series made single keeps `recurrence: []`, so the patch tells Google
                    if master.is_master {
                        make_placeholders(&tx, &master, &cal)?;
                    }
                    pull.push(cal.id);
                } else {
                    // carry the new details to every occurrence
                    let ids: Vec<i64> = {
                        let mut st =
                            tx.prepare("SELECT id FROM events WHERE (parent_local_id=?1) OR (calendar_id=?2 AND recurring_event_id=?3)")?;
                        st.query_map(params![master.id, master.calendar_id, master.gcal_id.clone().unwrap_or_default()], |r| r.get(0))?
                            .collect::<Result<Vec<i64>, _>>()?
                    };
                    for id in ids {
                        let mut inst = load_row(&tx, id)?;
                        let mut i2 = p.input.clone();
                        i2.start = None;
                        i2.end = None;
                        i2.all_day = Some(inst.all_day);
                        i2.start_date = None;
                        i2.end_date = None;
                        apply_input(&mut inst, &i2, &cal, &email, &name, &defs);
                        inst.local_dirty = inst.gcal_id.is_some();
                        inst.save(&tx)?;
                    }
                }
                if master.gcal_id.is_some() {
                    pending::enqueue(
                        &tx,
                        row.account_id,
                        sync::DOMAIN,
                        "patch",
                        &PatchOp { event_id: master.id, what: "all".into(), send_updates: Some(send_mode(p.input.send_updates, &master)) },
                    )?;
                }
                if row.is_master || !reshaped {
                    if row.is_master { master.id } else { row.id }
                } else {
                    // the occurrence was regenerated; return the series' first visible item
                    tx.query_row(
                        "SELECT id FROM events WHERE parent_local_id=?1 ORDER BY ABS(start_ms - ?2) LIMIT 1",
                        params![master.id, row.start_ms],
                        |r| r.get(0),
                    )
                    .optional()?
                    .unwrap_or(master.id)
                }
            } else {
                if row.gcal_id.is_none() && row.parent_local_id.is_some() {
                    return Err(crate::user_err!("pending", "This recurring series is still being saved. Try again in a moment."));
                }
                let mut r2 = row.clone();
                apply_input(&mut r2, &p.input, &cal, &email, &name, &defs);
                if let Some(rec) = p.input.recurrence.as_ref().filter(|r| !r.is_empty()) {
                    // an appointment becomes a series
                    if r2.recurring_event_id.is_none() {
                        for line in rec.iter().filter(|l| l.starts_with("RRULE")) {
                            rrule::Rule::parse(line).map_err(|e| crate::user_err!("bad_recurrence", "{e}"))?;
                        }
                        r2.recurrence = Some(rec.clone());
                        r2.is_master = true;
                        pull.push(cal.id);
                    }
                }
                r2.save(&tx)?;
                if r2.is_master && !row.is_master {
                    make_placeholders(&tx, &r2, &cal)?;
                }
                if r2.gcal_id.is_some() {
                    pending::enqueue(
                        &tx,
                        row.account_id,
                        sync::DOMAIN,
                        "patch",
                        &PatchOp { event_id: r2.id, what: "all".into(), send_updates: Some(send_mode(p.input.send_updates, &r2)) },
                    )?;
                }
                if r2.is_master {
                    tx.query_row("SELECT id FROM events WHERE parent_local_id=?1 ORDER BY start_ms LIMIT 1", [r2.id], |r| r.get(0))
                        .optional()?
                        .unwrap_or(r2.id)
                } else {
                    r2.id
                }
            };
            let out = detail(&tx, &load_row(&tx, result_id)?)?;
            tx.commit()?;
            Ok((out, row.account_id, pull))
        })
        .await?;
    changed(&ctx, account_id);
    kick(&ctx, account_id, pull);
    Ok(d)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CopyParams {
    pub(crate) id: i64,
    pub(crate) start: Option<i64>,
    pub(crate) end: Option<i64>,
    pub(crate) calendar_id: Option<i64>,
}

/// Ctrl+drag: a plain copy of an item (without attendees) at a new time.
pub(crate) async fn copy_event(ctx: Arc<AppCtx>, p: CopyParams) -> anyhow::Result<EventDetail> {
    let src = ctx.db.read(move |c| load_row(c, p.id)).await?;
    let input = EventInput {
        calendar_id: p.calendar_id.or(Some(src.calendar_id)),
        summary: Some(src.summary.clone()),
        location: Some(src.location.clone()),
        description: Some(src.description.clone()),
        start: Some(p.start.unwrap_or(src.start_ms)),
        end: Some(p.end.unwrap_or(src.end_ms)),
        all_day: Some(src.all_day),
        start_date: None,
        end_date: None,
        attendees: None,
        reminder_minutes: Some(src.reminder_min.unwrap_or(-1)),
        busy_status: Some(src.busy_status().to_string()),
        is_private: Some(src.is_private()),
        importance: src.importance.clone(),
        recurrence: None,
        send_updates: Some(false),
        categories: Some(src.categories.clone()),
        time_zone: src.time_zone.clone(),
    };
    create_event(ctx, input).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteParams {
    pub(crate) id: i64,
    pub(crate) scope: Option<String>,
    /// organiser cancelling a meeting: notify attendees (default true)
    pub(crate) send_updates: Option<bool>,
}

pub(crate) async fn delete_event(ctx: Arc<AppCtx>, p: DeleteParams) -> anyhow::Result<()> {
    let account_id = ctx.db.write(move |c| delete_tx(c, &p)).await?;
    changed(&ctx, account_id);
    kick(&ctx, account_id, vec![]);
    Ok(())
}

/// Delete an item locally and queue the delete for Google. Returns the item's account.
pub(crate) fn delete_tx(c: &mut Connection, p: &DeleteParams) -> anyhow::Result<i64> {
    let row = load_row(c, p.id)?;
    let cal = get_cal(c, row.calendar_id)?;
    // birthdays belong to the contact; Google refuses to delete them here
    if !cal.can_write() || row.event_type == "birthday" {
        return Err(read_only());
    }
    let tx = c.transaction()?;
    let series = p.scope.as_deref() == Some("series") || row.is_master;
    let target = if series { find_master(&tx, &row)?.unwrap_or_else(|| row.clone()) } else { row.clone() };
    if target.gcal_id.is_none() && target.parent_local_id.is_some() {
        return Err(crate::user_err!("pending", "This recurring series is still being saved. Try again in a moment."));
    }
    if target.is_master {
        drop_instances(&tx, &target)?;
    }
    tx.execute("DELETE FROM events WHERE id=?1", [target.id])?;
    match &target.gcal_id {
        Some(g) => {
            let send = if target.organizer_self { send_mode(p.send_updates, &target) } else { "none".into() };
            pending::enqueue(
                &tx,
                row.account_id,
                sync::DOMAIN,
                "delete",
                &DeleteOp { calendar_gcal_id: cal.gcal_id.clone(), gcal_id: g.clone(), send_updates: Some(send) },
            )?;
        }
        None => {
            tx.execute(
                "DELETE FROM pending_ops WHERE domain='calendar' AND account_id=?1 AND json_extract(payload,'$.eventId')=?2",
                params![row.account_id, target.id],
            )?;
        }
    }
    tx.commit()?;
    Ok(row.account_id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RespondParams {
    id: i64,
    response: String,
    scope: Option<String>,
    comment: Option<String>,
    send_response: Option<bool>,
}

fn set_my_response(row: &mut EventRow, email: &str, name: &str, response: &str, comment: Option<&str>) {
    match row.attendees.iter_mut().find(|a| a.is_self == Some(true) || a.email.eq_ignore_ascii_case(email)) {
        Some(a) => {
            a.response_status = response.into();
            a.is_self = Some(true);
            if let Some(cm) = comment {
                a.comment = (!cm.is_empty()).then(|| cm.to_string());
            }
        }
        None => row.attendees.push(Attendee {
            email: email.into(),
            display_name: Some(name.into()),
            response_status: response.into(),
            is_self: Some(true),
            comment: comment.filter(|c| !c.is_empty()).map(str::to_string),
            ..Default::default()
        }),
    }
    row.my_response = Some(response.into());
    row.updated_ms = Some(now_ms());
    row.local_dirty = true;
}

fn respond_tx(c: &mut Connection, id: i64, response: &str, series: bool, comment: Option<&str>, send: bool) -> anyhow::Result<(i64, i64)> {
    if !matches!(response, "accepted" | "tentative" | "declined") {
        return Err(crate::user_err!("bad_params", "response must be accepted, tentative or declined"));
    }
    let row = load_row(c, id)?;
    let (email, name) = account_identity(c, row.account_id)?;
    let tx = c.transaction()?;
    let target = if series || row.is_master { find_master(&tx, &row)?.unwrap_or_else(|| row.clone()) } else { row.clone() };
    let mut t = target.clone();
    set_my_response(&mut t, &email, &name, response, comment);
    t.save(&tx)?;
    if t.is_master {
        let ids: Vec<i64> = {
            let mut st = tx.prepare("SELECT id FROM events WHERE (parent_local_id=?1) OR (calendar_id=?2 AND recurring_event_id=?3)")?;
            st.query_map(params![t.id, t.calendar_id, t.gcal_id.clone().unwrap_or_default()], |r| r.get(0))?
                .collect::<Result<Vec<i64>, _>>()?
        };
        for i in ids {
            let mut inst = load_row(&tx, i)?;
            set_my_response(&mut inst, &email, &name, response, comment);
            inst.local_dirty = inst.gcal_id.is_some();
            inst.save(&tx)?;
        }
    }
    if t.gcal_id.is_some() {
        pending::enqueue(
            &tx,
            row.account_id,
            sync::DOMAIN,
            "patch",
            &PatchOp { event_id: t.id, what: "response".into(), send_updates: Some(if send { "all" } else { "none" }.into()) },
        )?;
    }
    tx.commit()?;
    Ok((row.account_id, t.id))
}

async fn respond(ctx: Arc<AppCtx>, p: RespondParams) -> anyhow::Result<()> {
    let (account_id, _) = ctx
        .db
        .write(move |c| {
            respond_tx(c, p.id, &p.response, p.scope.as_deref() == Some("series"), p.comment.as_deref(), p.send_response.unwrap_or(true))
        })
        .await?;
    changed(&ctx, account_id);
    kick(&ctx, account_id, vec![]);
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InviteParams {
    account_id: i64,
    ical_uid: String,
    response: String,
    comment: Option<String>,
    send_response: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InviteResult {
    event_id: Option<i64>,
}

/// Local row for an iCalUID: the series master first, else the next upcoming occurrence.
fn find_ical_row(c: &Connection, account_id: i64, uid: &str) -> anyhow::Result<Option<EventRow>> {
    Ok(c.query_row(
        "SELECT * FROM events WHERE account_id=?1 AND ical_uid=?2 AND status!='cancelled' \
         ORDER BY is_master DESC, (end_ms >= ?3) DESC, start_ms LIMIT 1",
        params![account_id, uid, now_ms()],
        EventRow::from_row,
    )
    .optional()?)
}

async fn respond_to_invite(ctx: Arc<AppCtx>, p: InviteParams) -> anyhow::Result<InviteResult> {
    let (aid, uid) = (p.account_id, p.ical_uid.clone());
    let mut found = ctx.db.read(move |c| find_ical_row(c, aid, &uid)).await?;
    if found.is_none() {
        // not synced yet: fetch it from the server (Google puts invitations on the calendar)
        if let Ok(api) = ctx.api(p.account_id).await {
            let cals = ctx
                .db
                .read(move |c| {
                    let mut st = c.prepare("SELECT * FROM calendars WHERE account_id=?1 ORDER BY is_primary DESC")?;
                    Ok(st.query_map([aid], CalRow::from_row)?.collect::<Result<Vec<_>, _>>()?)
                })
                .await?;
            if let Some(cal) = cals.first() {
                let _ = sync::pull_calendar(&ctx, &api, cal).await;
            }
            let uid = p.ical_uid.clone();
            found = ctx.db.read(move |c| find_ical_row(c, aid, &uid)).await?;
        }
    }
    let Some(row) = found else { return Ok(InviteResult { event_id: None }) };
    let id = row.id;
    let resp = p.response.clone();
    let (account_id, _) =
        ctx.db.write(move |c| respond_tx(c, id, &resp, true, p.comment.as_deref(), p.send_response.unwrap_or(true))).await?;
    changed(&ctx, account_id);
    kick(&ctx, account_id, vec![]);
    // report a visible item (an occurrence rather than the hidden master)
    let uid = p.ical_uid.clone();
    let shown = ctx
        .db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT id FROM events WHERE account_id=?1 AND ical_uid=?2 AND is_master=0 ORDER BY (end_ms >= ?3) DESC, start_ms LIMIT 1",
                params![aid, uid, now_ms()],
                |r| r.get::<_, i64>(0),
            )
            .optional()?)
        })
        .await?;
    Ok(InviteResult { event_id: shown.or(Some(id)) })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IcalParams {
    account_id: i64,
    ical_uid: String,
}

async fn find_by_ical_uid(ctx: Arc<AppCtx>, p: IcalParams) -> anyhow::Result<Option<CalEvent>> {
    ctx.db
        .read(move |c| {
            let sql = format!(
                "{SELECT_EV} WHERE e.account_id=?1 AND e.ical_uid=?2 AND e.status!='cancelled' \
                 ORDER BY e.is_master ASC, (e.end_ms >= ?3) DESC, e.start_ms LIMIT 1"
            );
            Ok(c.query_row(&sql, params![p.account_id, p.ical_uid, now_ms()], read_cal_event).optional()?)
        })
        .await
}

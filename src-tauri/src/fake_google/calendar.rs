//! Fake Google Calendar v3 API (see fake_google/mod.rs for conventions).
//!
//! Stores calendarList entries and, per calendar, the raw events Google itself stores: single
//! events, recurring masters (with `recurrence`) and exceptions (`recurringEventId` +
//! `originalStartTime`, possibly `status: cancelled`). `singleEvents=true` expands masters into
//! instances with Google-style ids (`<masterId>_20261012T070000Z` / `<masterId>_20261012`).

use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Shared, User, error, not_found, paginate, seed};
use crate::{
    calendar::rrule,
    google::calendar::{Attendee, CalendarListEntry, Event, EventDateTime, ExtendedProperties, Person, Reminder, Reminders},
};

pub const HOME_TZ: &str = "Asia/Jerusalem";
const DAY: i64 = 86_400_000;

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    pub calendars: Vec<FakeCalendar>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FakeCalendar {
    pub entry: CalendarListEntry,
    pub events: Vec<Event>,
}

impl Store {
    fn cal_mut(&mut self, id: &str) -> Option<&mut FakeCalendar> {
        if id == "primary" {
            return self.calendars.iter_mut().find(|c| c.entry.primary == Some(true));
        }
        self.calendars.iter_mut().find(|c| c.entry.id == id)
    }
}

// ---- time helpers ----------------------------------------------------------------------------

fn tz_named(name: Option<&str>, fallback: Tz) -> Tz {
    name.and_then(|n| n.parse::<Tz>().ok()).unwrap_or(fallback)
}

fn cal_tz(c: &FakeCalendar) -> Tz {
    tz_named(c.entry.time_zone.as_deref(), chrono_tz::UTC)
}

fn parse_rfc3339(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// Start/end of an EventDateTime as epoch ms; all-day dates are midnight in `tz`.
fn edt_ms(e: &EventDateTime, tz: Tz) -> Option<i64> {
    if let Some(dt) = &e.date_time {
        return parse_rfc3339(dt);
    }
    let d = parse_date(e.date.as_deref()?)?;
    tz.from_local_datetime(&d.and_hms_opt(0, 0, 0)?).earliest().map(|t| t.timestamp_millis())
}

fn fmt_dt(ms: i64, tz: Tz) -> String {
    DateTime::from_timestamp_millis(ms).unwrap_or_default().with_timezone(&tz).to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

fn rfc_now() -> String {
    crate::clock::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn event_span(e: &Event, tz: Tz) -> Option<(i64, i64)> {
    let s = edt_ms(e.start.as_ref()?, tz)?;
    let en = e.end.as_ref().and_then(|x| edt_ms(x, tz)).unwrap_or(s);
    Some((s, en.max(s)))
}

fn is_all_day(e: &Event) -> bool {
    e.start.as_ref().is_some_and(|s| s.date.is_some())
}

// ---- recurrence expansion --------------------------------------------------------------------

fn is_cancelled(e: &Event) -> bool {
    e.status.as_deref() == Some("cancelled")
}

/// Instances of `master` overlapping [min, max), with stored exceptions applied (cancelled ones
/// only with `show_deleted`).
fn expand_master(master: &Event, exceptions: &[&Event], ctz: Tz, min: i64, max: i64, show_deleted: bool) -> Vec<Event> {
    let Some(start) = &master.start else { return vec![] };
    let Some(mid) = &master.id else { return vec![] };
    let rec = master.recurrence.clone().unwrap_or_default();
    let all_day = start.date.is_some();
    let etz = tz_named(start.time_zone.as_deref(), ctz);
    let (dtstart, dur) = if all_day {
        let sd = parse_date(start.date.as_deref().unwrap_or("")).unwrap_or_default();
        let ed = master.end.as_ref().and_then(|e| e.date.as_deref()).and_then(parse_date).unwrap_or(sd + Duration::days(1));
        (sd.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis(), (ed - sd).num_days().max(1) * DAY)
    } else {
        let Some((s, e)) = event_span(master, etz) else { return vec![] };
        (s, e - s)
    };
    // all-day occurrences are expressed as UTC midnights; allow a day of slack either side
    let before = if all_day { max + 2 * DAY } else { max };
    let occ = rrule::expand_recurrence(&rec, dtstart, all_day, etz, before, 5000);
    let mut out = Vec::new();
    for o in occ {
        let (id, ostart, oend) = if all_day {
            let d = DateTime::from_timestamp_millis(o).unwrap().date_naive();
            let e = DateTime::from_timestamp_millis(o + dur).unwrap().date_naive();
            (
                format!("{}_{}", mid, d.format("%Y%m%d")),
                EventDateTime { date: Some(d.format("%Y-%m-%d").to_string()), ..Default::default() },
                EventDateTime { date: Some(e.format("%Y-%m-%d").to_string()), ..Default::default() },
            )
        } else {
            let tzname = start.time_zone.clone();
            (
                format!("{}_{}", mid, DateTime::from_timestamp_millis(o).unwrap().format("%Y%m%dT%H%M%SZ")),
                EventDateTime { date_time: Some(fmt_dt(o, etz)), time_zone: tzname.clone(), ..Default::default() },
                EventDateTime { date_time: Some(fmt_dt(o + dur, etz)), time_zone: tzname, ..Default::default() },
            )
        };
        let inst = if let Some(ex) = exceptions.iter().find(|x| x.id.as_deref() == Some(id.as_str())) {
            if is_cancelled(ex) && !show_deleted {
                continue;
            }
            (*ex).clone()
        } else {
            let mut i = master.clone();
            i.id = Some(id);
            i.recurrence = None;
            i.recurring_event_id = Some(mid.clone());
            i.original_start_time = Some(ostart.clone());
            i.start = Some(ostart);
            i.end = Some(oend);
            i
        };
        if let Some((s, e)) = event_span(&inst, ctz)
            && (e > min && s < max || (s == e && s >= min && s < max))
        {
            out.push(inst);
        }
    }
    out
}

fn overlaps(e: &Event, tz: Tz, min: i64, max: i64) -> bool {
    match event_span(e, tz) {
        Some((s, en)) => (en > min || (s == en && s >= min)) && s < max,
        None => false,
    }
}

/// Mark `self` on organizer / attendees for the requesting user (Google computes it per viewer).
fn present(mut e: Event, email: &str) -> Event {
    let me = |x: &Option<String>| x.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(email));
    if let Some(o) = &mut e.organizer {
        o.is_self = me(&o.email).then_some(true);
    }
    if let Some(c) = &mut e.creator {
        c.is_self = me(&c.email).then_some(true);
    }
    if let Some(a) = &mut e.attendees {
        for at in a.iter_mut() {
            at.is_self = at.email.eq_ignore_ascii_case(email).then_some(true);
        }
    }
    e
}

// ---- routes ----------------------------------------------------------------------------------

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/calendar/v3/users/me/calendarList", get(calendar_list))
        .route("/calendar/v3/calendars/{cal}/events", get(list_events).post(insert_event))
        .route("/calendar/v3/calendars/{cal}/events/{id}", get(get_event).patch(patch_event).put(put_event).delete(delete_event))
        .route("/calendar/v3/calendars/{cal}/events/{id}/instances", get(instances))
        .route("/calendar/v3/calendars/{cal}/events/{id}/move", axum::routing::post(move_event))
}

type Q = Query<HashMap<String, String>>;

async fn calendar_list(State(s): State<Shared>, User(email): User, Query(q): Q) -> Response {
    let items: Vec<CalendarListEntry> = s.read(|st| st.user(&email).calendar.calendars.iter().map(|c| c.entry.clone()).collect());
    let max = q.get("maxResults").and_then(|m| m.parse().ok()).unwrap_or(100usize).clamp(1, 250);
    let (page, next) = paginate(&items, &q.get("pageToken").cloned(), max);
    let items: Vec<Value> = page
        .into_iter()
        .map(|c| {
            let mut v = serde_json::to_value(c).unwrap();
            v["kind"] = json!("calendar#calendarListEntry");
            v["etag"] = json!("\"1\"");
            v
        })
        .collect();
    let mut body = json!({ "kind": "calendar#calendarList", "etag": "\"1\"", "items": items });
    if let Some(n) = next {
        body["nextPageToken"] = json!(n);
    }
    Json(body).into_response()
}

fn parse_bound(q: &HashMap<String, String>, k: &str) -> Result<Option<i64>, Response> {
    match q.get(k) {
        None => Ok(None),
        Some(v) => parse_rfc3339(v).map(Some).ok_or_else(|| error(400, &format!("Bad Request: invalid {k}"), "badRequest")),
    }
}

/// All events of a calendar as Google would list them.
fn listing(c: &FakeCalendar, single: bool, min: i64, max: i64, q: &HashMap<String, String>) -> Vec<Event> {
    let tz = cal_tz(c);
    let show_deleted = q.get("showDeleted").is_some_and(|v| v == "true");
    let ical = q.get("iCalUID");
    let text = q.get("q").map(|t| t.to_lowercase());
    let mut out = Vec::new();
    for e in &c.events {
        if let Some(u) = ical
            && e.ical_uid.as_ref() != Some(u)
        {
            continue;
        }
        let cancelled = is_cancelled(e);
        if e.recurrence.is_some() && cancelled {
            // a deleted series: its tombstone only
            if show_deleted {
                out.push(e.clone());
            }
        } else if e.recurrence.is_some() {
            let exc: Vec<&Event> = c.events.iter().filter(|x| x.recurring_event_id == e.id).collect();
            let inst = expand_master(e, &exc, tz, min, max, show_deleted);
            if single {
                out.extend(inst);
            } else if !inst.is_empty() || ical.is_some() {
                out.push(e.clone());
            }
        } else if e.recurring_event_id.is_some() {
            // exceptions: part of the expansion when singleEvents=true
            if !single && (show_deleted || !cancelled) && (ical.is_some() || overlaps(e, tz, min, max)) {
                out.push(e.clone());
            }
        } else if (show_deleted || !cancelled) && (ical.is_some() || overlaps(e, tz, min, max)) {
            out.push(e.clone());
        }
    }
    // updatedMin: changes only (deleted ones included, as Google does)
    if let Some(since) = q.get("updatedMin").and_then(|u| parse_rfc3339(u)) {
        out.retain(|e| e.updated.as_deref().and_then(parse_rfc3339).is_some_and(|u| u >= since));
    }
    if let Some(t) = text {
        out.retain(|e| {
            [&e.summary, &e.description, &e.location].iter().any(|f| f.as_deref().is_some_and(|s| s.to_lowercase().contains(&t)))
        });
    }
    if single {
        out.sort_by_key(|e| event_span(e, tz).map(|x| x.0).unwrap_or(0));
    }
    out
}

fn events_page(c: &FakeCalendar, items: Vec<Event>, email: &str, q: &HashMap<String, String>) -> Response {
    let max = q.get("maxResults").and_then(|m| m.parse().ok()).unwrap_or(250usize).clamp(1, 2500);
    let (page, next) = paginate(&items, &q.get("pageToken").cloned(), max);
    let items: Vec<Value> = page
        .into_iter()
        .map(|e| {
            let mut v = serde_json::to_value(present(e, email)).unwrap();
            v["kind"] = json!("calendar#event");
            v
        })
        .collect();
    let mut body = json!({
        "kind": "calendar#events",
        "etag": "\"p1\"",
        "summary": c.entry.summary,
        "updated": rfc_now(),
        "timeZone": c.entry.time_zone,
        "accessRole": c.entry.access_role,
        "defaultReminders": c.entry.default_reminders,
        "items": items,
    });
    if let Some(n) = next {
        body["nextPageToken"] = json!(n);
    }
    Json(body).into_response()
}

async fn list_events(State(s): State<Shared>, User(email): User, Path(cal): Path<String>, Query(q): Q) -> Response {
    let (min, max) = match (parse_bound(&q, "timeMin"), parse_bound(&q, "timeMax")) {
        (Ok(a), Ok(b)) => (a.unwrap_or(i64::MIN / 4), b.unwrap_or(seed::now_ms() + 3 * 365 * DAY)),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let single = q.get("singleEvents").is_some_and(|v| v == "true");
    if q.get("orderBy").is_some_and(|o| o == "startTime") && !single {
        return error(400, "The requested ordering is not available for the particular query.", "badRequest");
    }
    if q.get("maxResults").and_then(|m| m.parse::<i64>().ok()).is_some_and(|m| !(1..=2500).contains(&m)) {
        return error(400, "Invalid value for maxResults (1-2500).", "invalid");
    }
    s.read(|st| {
        let store = &mut st.user(&email).calendar;
        let Some(c) = store.cal_mut(&cal) else { return not_found() };
        let items = listing(c, single, min, max, &q);
        events_page(c, items, &email, &q)
    })
}

async fn instances(State(s): State<Shared>, User(email): User, Path((cal, id)): Path<(String, String)>, Query(q): Q) -> Response {
    let (min, max) = match (parse_bound(&q, "timeMin"), parse_bound(&q, "timeMax")) {
        (Ok(a), Ok(b)) => (a.unwrap_or(i64::MIN / 4), b.unwrap_or(seed::now_ms() + 3 * 365 * DAY)),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    s.read(|st| {
        let store = &mut st.user(&email).calendar;
        let Some(c) = store.cal_mut(&cal) else { return not_found() };
        let Some(m) = c.events.iter().find(|e| e.id.as_deref() == Some(id.as_str()) && e.recurrence.is_some() && !is_cancelled(e)) else {
            return not_found();
        };
        let exc: Vec<&Event> = c.events.iter().filter(|x| x.recurring_event_id == m.id).collect();
        let items = expand_master(m, &exc, cal_tz(c), min, max, false);
        events_page(c, items, &email, &q)
    })
}

/// Find a stored event, or generate the instance a `<master>_<suffix>` id refers to.
fn find_event(c: &FakeCalendar, id: &str) -> Option<Event> {
    if let Some(e) = c.events.iter().find(|e| e.id.as_deref() == Some(id)) {
        return Some(e.clone());
    }
    let (mid, suffix) = id.rsplit_once('_')?;
    let master = c.events.iter().find(|e| e.id.as_deref() == Some(mid) && e.recurrence.is_some() && !is_cancelled(e))?;
    let tz = cal_tz(c);
    let at = if suffix.len() == 8 {
        NaiveDate::parse_from_str(suffix, "%Y%m%d").ok()?.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis()
    } else {
        chrono::NaiveDateTime::parse_from_str(suffix, "%Y%m%dT%H%M%SZ").ok()?.and_utc().timestamp_millis()
    };
    let exc: Vec<&Event> = c.events.iter().filter(|x| x.recurring_event_id.as_deref() == Some(mid)).collect();
    expand_master(master, &exc, tz, at - 2 * DAY, at + 2 * DAY, false).into_iter().find(|e| e.id.as_deref() == Some(id))
}

async fn get_event(State(s): State<Shared>, User(email): User, Path((cal, id)): Path<(String, String)>) -> Response {
    s.read(|st| {
        let store = &mut st.user(&email).calendar;
        let Some(c) = store.cal_mut(&cal) else { return not_found() };
        match find_event(c, &id) {
            Some(e) => Json(present(e, &email)).into_response(),
            None => not_found(),
        }
    })
}

fn can_write(c: &FakeCalendar) -> bool {
    matches!(c.entry.access_role.as_str(), "owner" | "writer")
}

fn forbidden() -> Response {
    error(403, "You need to have writer access to this calendar.", "requiredAccessLevel")
}

fn new_id(n: u64) -> String {
    // Google event ids use base32hex characters (a-v, 0-9)
    let mut x = n.wrapping_mul(0x9E3779B97F4A7C15) ^ 0x5bd1e995;
    let mut s = String::from("cm");
    for _ in 0..14 {
        let d = (x % 32) as u32;
        s.push(std::char::from_digit(d, 32).unwrap());
        x /= 32;
    }
    s
}

async fn insert_event(State(s): State<Shared>, User(email): User, Path(cal): Path<String>, Json(body): Json<Value>) -> Response {
    let mut ev: Event = match serde_json::from_value(body) {
        Ok(e) => e,
        Err(e) => return error(400, &format!("Invalid event: {e}"), "invalid"),
    };
    if ev.start.is_none() || ev.end.is_none() {
        return error(400, "Missing end time.", "required");
    }
    if ev.recurrence.is_some() && !is_all_day(&ev) && ev.start.as_ref().and_then(|s| s.time_zone.as_ref()).is_none() {
        return error(400, "Missing time zone definition for start time.", "invalid");
    }
    s.with(|st| {
        let n = st.next();
        let u = st.user(&email);
        let name = u.name.clone();
        let Some(c) = u.calendar.cal_mut(&cal) else { return not_found() };
        if !can_write(c) {
            return forbidden();
        }
        let id = ev.id.clone().unwrap_or_else(|| new_id(n));
        if c.events.iter().any(|e| e.id.as_deref() == Some(id.as_str())) {
            return error(409, "The requested identifier already exists.", "duplicate");
        }
        ev.id = Some(id.clone());
        ev.etag = Some(format!("\"{n}\""));
        ev.status = Some(ev.status.clone().unwrap_or_else(|| "confirmed".into()));
        ev.created = Some(rfc_now());
        ev.updated = Some(rfc_now());
        ev.html_link = Some(format!("https://www.google.com/calendar/event?eid={id}"));
        ev.ical_uid = Some(ev.ical_uid.clone().unwrap_or_else(|| format!("{id}@google.com")));
        ev.sequence = Some(0);
        ev.event_type = Some(ev.event_type.clone().unwrap_or_else(|| "default".into()));
        let owner = if c.entry.primary == Some(true) {
            Person { email: Some(email.clone()), display_name: Some(name.clone()), is_self: None }
        } else {
            Person { email: Some(c.entry.id.clone()), display_name: Some(c.entry.summary.clone()), is_self: None }
        };
        ev.creator = Some(Person { email: Some(email.clone()), display_name: Some(name.clone()), is_self: None });
        if ev.organizer.is_none() {
            ev.organizer = Some(owner);
        }
        if let Some(att) = &mut ev.attendees
            && !att.is_empty()
            && !att.iter().any(|a| a.email.eq_ignore_ascii_case(&email))
        {
            att.insert(
                0,
                Attendee {
                    email: email.clone(),
                    display_name: Some(name),
                    organizer: Some(true),
                    response_status: "accepted".into(),
                    ..Default::default()
                },
            );
        }
        if ev.reminders.is_none() {
            ev.reminders = Some(Reminders { use_default: true, overrides: None });
        }
        c.events.push(ev.clone());
        Json(present(ev, &email)).into_response()
    })
}

async fn patch_event(
    State(s): State<Shared>,
    User(email): User,
    Path((cal, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    update_event(s, email, cal, id, body, false)
}

async fn put_event(
    State(s): State<Shared>,
    User(email): User,
    Path((cal, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    update_event(s, email, cal, id, body, true)
}

fn update_event(s: Shared, email: String, cal: String, id: String, body: Value, replace: bool) -> Response {
    let Value::Object(patch) = body else { return error(400, "Invalid JSON body", "badRequest") };
    s.with(|st| {
        let n = st.next();
        let u = st.user(&email);
        let Some(c) = u.calendar.cal_mut(&cal) else { return not_found() };
        let tz = cal_tz(c);
        // attendees may update their own response even on a calendar they can only read
        let only_attendees = patch.keys().all(|k| k == "attendees");
        if !can_write(c) && !only_attendees {
            return forbidden();
        }
        let Some(cur) = find_event(c, &id) else { return not_found() };
        if cur.status.as_deref() == Some("cancelled") {
            return error(410, "Resource has been deleted", "deleted");
        }
        // birthday / Gmail-created / working-location events: only cosmetic fields are writable
        let restricted = matches!(cur.event_type.as_deref(), Some("birthday" | "fromGmail" | "workingLocation"));
        let cosmetic = ["reminders", "colorId", "transparency", "extendedProperties", "visibility"];
        if restricted && patch.keys().any(|k| !cosmetic.contains(&k.as_str()) && patch[k] != serde_json::to_value(&cur).unwrap()[k]) {
            return error(
                400,
                "This event type cannot be changed through the API except for reminders, colour and visibility.",
                "eventTypeRestriction",
            );
        }
        let mut v = serde_json::to_value(&cur).unwrap();
        if replace {
            // keep server-owned fields
            let keep: Vec<(String, Value)> =
                ["id", "etag", "created", "htmlLink", "iCalUID", "organizer", "creator", "recurringEventId", "originalStartTime"]
                    .iter()
                    .filter_map(|k| v.get(*k).map(|x| (k.to_string(), x.clone())))
                    .collect();
            v = Value::Object(patch.clone());
            for (k, x) in keep {
                v[k] = x;
            }
        } else {
            for (k, x) in &patch {
                if x.is_null() {
                    v.as_object_mut().unwrap().remove(k);
                } else {
                    v[k] = x.clone();
                }
            }
        }
        let mut ev: Event = match serde_json::from_value(v) {
            Ok(e) => e,
            Err(e) => return error(400, &format!("Invalid event: {e}"), "invalid"),
        };
        for a in ev.attendees.iter_mut().flatten() {
            a.is_self = None;
        }
        if let Some(o) = &mut ev.organizer {
            o.is_self = None;
        }
        ev.id = Some(id.clone());
        ev.etag = Some(format!("\"{n}\""));
        ev.updated = Some(rfc_now());
        let time_changed = ev.start != cur.start || ev.end != cur.end;
        if time_changed || ev.recurrence != cur.recurrence {
            ev.sequence = Some(cur.sequence.unwrap_or(0) + 1);
        }
        // moving a series master moves its exceptions' original times with it
        if cur.recurrence.is_some() && time_changed {
            let old = cur.start.as_ref().and_then(|x| edt_ms(x, tz));
            let new = ev.start.as_ref().and_then(|x| edt_ms(x, tz));
            if let (Some(o), Some(nw)) = (old, new) {
                let delta = nw - o;
                let all_day = is_all_day(&ev);
                for x in c.events.iter_mut().filter(|x| x.recurring_event_id.as_deref() == Some(id.as_str())) {
                    if let Some(os) = x.original_start_time.as_ref().and_then(|t| edt_ms(t, tz)) {
                        let ns = os + delta;
                        let tzn = x.original_start_time.as_ref().and_then(|t| t.time_zone.clone());
                        let etz = tz_named(tzn.as_deref(), tz);
                        if all_day {
                            let d = DateTime::from_timestamp_millis(ns).unwrap().with_timezone(&etz).date_naive();
                            x.original_start_time =
                                Some(EventDateTime { date: Some(d.format("%Y-%m-%d").to_string()), ..Default::default() });
                            x.id = Some(format!("{}_{}", id, d.format("%Y%m%d")));
                        } else {
                            x.original_start_time =
                                Some(EventDateTime { date_time: Some(fmt_dt(ns, etz)), time_zone: tzn, ..Default::default() });
                            x.id = Some(format!("{}_{}", id, DateTime::from_timestamp_millis(ns).unwrap().format("%Y%m%dT%H%M%SZ")));
                        }
                        // an exception that only existed because of its time is now redundant
                        if x.status.as_deref() != Some("cancelled") {
                            x.start = x.original_start_time.clone();
                            let dur = event_span(&cur, tz).map(|(a, b)| b - a).unwrap_or(0);
                            x.end = Some(if all_day {
                                let d = DateTime::from_timestamp_millis(ns + dur).unwrap().with_timezone(&etz).date_naive();
                                EventDateTime { date: Some(d.format("%Y-%m-%d").to_string()), ..Default::default() }
                            } else {
                                EventDateTime {
                                    date_time: Some(fmt_dt(ns + dur, etz)),
                                    time_zone: x.original_start_time.as_ref().and_then(|t| t.time_zone.clone()),
                                    ..Default::default()
                                }
                            });
                        }
                    }
                }
            }
        }
        match c.events.iter_mut().find(|e| e.id.as_deref() == Some(id.as_str())) {
            Some(slot) => *slot = ev.clone(),
            None => c.events.push(ev.clone()), // first change to a generated instance → exception
        }
        Json(present(ev, &email)).into_response()
    })
}

/// `events.move?destination=`: only default events, not instances; the organizer becomes the
/// destination calendar.
async fn move_event(State(s): State<Shared>, User(email): User, Path((cal, id)): Path<(String, String)>, Query(q): Q) -> Response {
    let Some(dest) = q.get("destination").cloned() else { return error(400, "Required parameter: destination", "required") };
    s.with(|st| {
        let n = st.next();
        let u = st.user(&email);
        let name = u.name.clone();
        let store = &mut u.calendar;
        let Some(src) = store.cal_mut(&cal) else { return not_found() };
        if !can_write(src) {
            return forbidden();
        }
        let src_id = src.entry.id.clone();
        let Some(pos) = src.events.iter().position(|e| e.id.as_deref() == Some(id.as_str())) else { return not_found() };
        let cur = &src.events[pos];
        if cur.recurring_event_id.is_some() || id.contains('_') {
            return error(400, "Cannot change the organizer of an instance.", "cannotChangeOrganizerOfInstance");
        }
        if cur.event_type.as_deref().is_some_and(|t| t != "default") {
            return error(400, "This event type cannot be moved.", "eventTypeRestriction");
        }
        let Some(d) = store.cal_mut(&dest) else { return not_found() };
        if !can_write(d) {
            return forbidden();
        }
        let owner = if d.entry.primary == Some(true) {
            Person { email: Some(email.clone()), display_name: Some(name), is_self: None }
        } else {
            Person { email: Some(d.entry.id.clone()), display_name: Some(d.entry.summary.clone()), is_self: None }
        };
        let src = store.cal_mut(&src_id).unwrap();
        let mut moved: Vec<Event> = Vec::new();
        src.events.retain(|e| {
            let take = e.id.as_deref() == Some(id.as_str()) || e.recurring_event_id.as_deref() == Some(id.as_str());
            if take {
                moved.push(e.clone());
            }
            !take
        });
        for e in moved.iter_mut() {
            e.organizer = Some(owner.clone());
            e.etag = Some(format!("\"{n}\""));
            e.updated = Some(rfc_now());
        }
        let out = moved.iter().find(|e| e.id.as_deref() == Some(id.as_str())).cloned().unwrap();
        store.cal_mut(&dest).unwrap().events.extend(moved);
        Json(present(out, &email)).into_response()
    })
}

async fn delete_event(State(s): State<Shared>, User(email): User, Path((cal, id)): Path<(String, String)>) -> Response {
    s.with(|st| {
        let n = st.next();
        let u = st.user(&email);
        let Some(c) = u.calendar.cal_mut(&cal) else { return not_found() };
        if !can_write(c) {
            return forbidden();
        }
        let Some(cur) = find_event(c, &id) else { return not_found() };
        if cur.status.as_deref() == Some("cancelled") {
            return error(410, "Resource has been deleted", "deleted");
        }
        if cur.event_type.as_deref() == Some("birthday")
            || cur
                .recurring_event_id
                .as_deref()
                .is_some_and(|m| c.events.iter().any(|e| e.id.as_deref() == Some(m) && e.event_type.as_deref() == Some("birthday")))
        {
            return error(400, "Birthday events can only be removed from the contact.", "eventTypeRestriction");
        }
        // like Google, a deleted event stays as a `cancelled` tombstone (listed with showDeleted /
        // updatedMin); a series loses its exceptions
        if cur.recurrence.is_some() {
            c.events.retain(|e| e.recurring_event_id.as_deref() != Some(id.as_str()));
        }
        let mut ex = cur.clone();
        ex.status = Some("cancelled".into());
        ex.etag = Some(format!("\"{n}\""));
        ex.updated = Some(rfc_now());
        match c.events.iter_mut().find(|e| e.id.as_deref() == Some(id.as_str())) {
            Some(slot) => *slot = ex,
            None => c.events.push(ex),
        }
        StatusCode::NO_CONTENT.into_response()
    })
}

// ---- seed ------------------------------------------------------------------------------------

struct Seeder {
    tz: Tz,
    today: NaiveDate,
    email: String,
    name: String,
    n: u64,
    rng: seed::Rng,
}

impl Seeder {
    fn id(&mut self) -> String {
        self.n += 1;
        new_id(self.n ^ (self.rng.next() & 0xffff_ffff) << 8)
    }

    fn at(&self, day: i64, h: u32, m: u32) -> i64 {
        let d = self.today + Duration::days(day);
        self.tz.from_local_datetime(&d.and_hms_opt(h, m, 0).unwrap()).earliest().unwrap().timestamp_millis()
    }

    fn timed(&self, day: i64, h: u32, m: u32, minutes: i64) -> (EventDateTime, EventDateTime) {
        let s = self.at(day, h, m);
        let tzn = Some(HOME_TZ.to_string());
        (
            EventDateTime { date_time: Some(fmt_dt(s, self.tz)), time_zone: tzn.clone(), ..Default::default() },
            EventDateTime { date_time: Some(fmt_dt(s + minutes * 60_000, self.tz)), time_zone: tzn, ..Default::default() },
        )
    }

    fn all_day(&self, day: i64, days: i64) -> (EventDateTime, EventDateTime) {
        let d = self.today + Duration::days(day);
        (
            EventDateTime { date: Some(d.format("%Y-%m-%d").to_string()), ..Default::default() },
            EventDateTime { date: Some((d + Duration::days(days)).format("%Y-%m-%d").to_string()), ..Default::default() },
        )
    }

    fn event(&mut self, summary: &str, when: (EventDateTime, EventDateTime)) -> Event {
        let id = self.id();
        Event {
            ical_uid: Some(format!("{id}@google.com")),
            html_link: Some(format!("https://www.google.com/calendar/event?eid={id}")),
            etag: Some(format!("\"{}\"", self.n)),
            id: Some(id),
            status: Some("confirmed".into()),
            created: Some(rfc_now()),
            updated: Some(rfc_now()),
            summary: Some(summary.into()),
            creator: Some(Person { email: Some(self.email.clone()), display_name: Some(self.name.clone()), is_self: None }),
            organizer: Some(Person { email: Some(self.email.clone()), display_name: Some(self.name.clone()), is_self: None }),
            start: Some(when.0),
            end: Some(when.1),
            reminders: Some(Reminders { use_default: true, overrides: None }),
            sequence: Some(0),
            event_type: Some("default".into()),
            ..Default::default()
        }
    }

    fn me_attendee(&self, status: &str, organizer: bool) -> Attendee {
        Attendee {
            email: self.email.clone(),
            display_name: Some(self.name.clone()),
            organizer: organizer.then_some(true),
            response_status: status.into(),
            ..Default::default()
        }
    }
}

fn person_att(p: &seed::Person, status: &str) -> Attendee {
    Attendee { email: p.email(), display_name: Some(p.name()), response_status: status.into(), ..Default::default() }
}

fn by_name(first: &str) -> &'static seed::Person {
    seed::PEOPLE.iter().find(|p| p.first == first).unwrap_or(&seed::PEOPLE[0])
}

fn ext(show_as: Option<&str>, importance: Option<&str>) -> Option<ExtendedProperties> {
    let mut m = serde_json::Map::new();
    if let Some(s) = show_as {
        m.insert("cmShowAs".into(), json!(s));
    }
    if let Some(i) = importance {
        m.insert("cmImportance".into(), json!(i));
    }
    Some(ExtendedProperties { private: Some(m), shared: None })
}

fn entry(id: &str, summary: &str, color: &str, role: &str, primary: bool, reminders: Vec<Reminder>) -> CalendarListEntry {
    CalendarListEntry {
        id: id.into(),
        summary: summary.into(),
        time_zone: Some(HOME_TZ.into()),
        background_color: Some(color.into()),
        foreground_color: Some("#000000".into()),
        selected: Some(true),
        access_role: role.into(),
        primary: primary.then_some(true),
        default_reminders: reminders,
        ..Default::default()
    }
}

/// Israel holidays (fixed dates, Israel observance) around the demo period.
const HOLIDAYS: &[(&str, &str, i64)] = &[
    ("2026-09-12", "Rosh Hashana (ראש השנה)", 2),
    ("2026-09-21", "Yom Kippur (יום כיפור)", 1),
    ("2026-09-26", "Sukkot (סוכות)", 1),
    ("2026-10-03", "Simchat Torah (שמחת תורה)", 1),
    ("2026-12-05", "Hanukkah (חנוכה)", 8),
    ("2027-01-23", "Tu BiShvat (ט״ו בשבט)", 1),
    ("2027-03-23", "Purim (פורים)", 1),
    ("2027-04-22", "Pesach (פסח)", 1),
    ("2027-04-28", "Seventh day of Pesach", 1),
    ("2027-05-04", "Yom HaShoah", 1),
    ("2027-05-11", "Yom HaZikaron", 1),
    ("2027-05-12", "Yom HaAtzmaut (יום העצמאות)", 1),
    ("2027-06-11", "Shavuot (שבועות)", 1),
    ("2027-09-02", "Rosh Hashana (ראש השנה)", 2),
];

const CLIENTS: &[&str] = &["Orion Technologies", "Corvina Medical", "Sunfield Energy"];
const INVESTOR_FIRSTS: &[&str] =
    &["Sarah", "David", "Michael", "Yael", "James", "Emily", "Thomas", "Lior", "Anna", "Robert", "Sophie", "Avi"];

pub fn seed(email: &str, name: &str) -> Store {
    let tz: Tz = HOME_TZ.parse().unwrap();
    let today = DateTime::from_timestamp_millis(seed::now_ms()).unwrap().with_timezone(&tz).date_naive();
    let mut s = Seeder { tz, today, email: email.into(), name: name.into(), n: 0, rng: seed::Rng::for_key(&format!("cal:{email}")) };
    let is_info = email.starts_with("info@");
    let popup15 = vec![Reminder { method: "popup".into(), minutes: 15 }];

    let mut primary = FakeCalendar {
        entry: entry(email, email, if is_info { "#7bd148" } else { "#9fc6e7" }, "owner", true, popup15.clone()),
        events: vec![],
    };
    let mut conf = FakeCalendar {
        entry: entry(
            &format!("c_{}conf@group.calendar.google.com", if is_info { "info" } else { "kenny" }),
            if is_info { "Company events" } else { "Conferences" },
            if is_info { "#fad165" } else { "#b99aff" },
            "owner",
            false,
            vec![],
        ),
        events: vec![],
    };
    // days back to the most recent Sunday (Israeli week) / Monday
    let dow = today.weekday().num_days_from_sunday() as i64; // Sun=0
    let sunday = -dow;

    if !is_info {
        // --- recurring series -------------------------------------------------------------
        let mut team = s.event("Weekly team call", s.timed(sunday - 42, 10, 0, 45));
        team.location = Some("Zoom".into());
        team.description = Some("Pipeline, client updates, conference planning.\nhttps://zoom.example/j/5550142".into());
        team.recurrence = Some(vec!["RRULE:FREQ=WEEKLY;BYDAY=SU".into()]);
        team.attendees =
            Some(vec![s.me_attendee("accepted", true), person_att(by_name("Maya"), "accepted"), person_att(by_name("Tom"), "tentative")]);
        let tid = team.id.clone().unwrap();
        // exception: next week's call moved to 11:30; the one in 3 weeks cancelled
        let moved_orig = s.at(sunday + 7, 10, 0);
        let mut moved = team.clone();
        moved.id = Some(format!("{tid}_{}", DateTime::from_timestamp_millis(moved_orig).unwrap().format("%Y%m%dT%H%M%SZ")));
        moved.recurrence = None;
        moved.recurring_event_id = Some(tid.clone());
        moved.original_start_time = Some(s.timed(sunday + 7, 10, 0, 45).0);
        let (ms, me) = s.timed(sunday + 7, 11, 30, 45);
        moved.start = Some(ms);
        moved.end = Some(me);
        moved.summary = Some("Weekly team call (moved)".into());
        let canc_orig = s.at(sunday + 21, 10, 0);
        let mut canc = team.clone();
        canc.id = Some(format!("{tid}_{}", DateTime::from_timestamp_millis(canc_orig).unwrap().format("%Y%m%dT%H%M%SZ")));
        canc.recurrence = None;
        canc.recurring_event_id = Some(tid.clone());
        canc.original_start_time = Some(s.timed(sunday + 21, 10, 0, 45).0);
        canc.status = Some("cancelled".into());
        primary.events.extend([team, moved, canc]);

        let mut mkt = s.event("Market open review", s.timed(sunday - 27, 16, 15, 30));
        mkt.recurrence = Some(vec!["RRULE:FREQ=WEEKLY;BYDAY=MO,WE".into()]);
        mkt.reminders = Some(Reminders { use_default: false, overrides: Some(vec![]) });
        mkt.description = Some("US pre-market movers for client names; check overnight news.".into());
        primary.events.push(mkt);

        let mut board = s.event("Monthly board prep — Corvina Medical", s.timed(-40, 14, 0, 60));
        board.recurrence = Some(vec!["RRULE:FREQ=MONTHLY;BYDAY=1TU".into()]);
        // align the series start to the first Tuesday of that month
        {
            let d0 = today + Duration::days(-40);
            let first = NaiveDate::from_ymd_opt(d0.year(), d0.month(), 1).unwrap();
            let off = (7 + 1 - first.weekday().num_days_from_monday() as i64) % 7; // Tuesday
            let day = (first + Duration::days(off) - today).num_days();
            let w = s.timed(day, 14, 0, 60);
            board.start = Some(w.0);
            board.end = Some(w.1);
        }
        board.location = Some("Corvina Medical HQ, Haifa".into());
        board.attendees =
            Some(vec![s.me_attendee("accepted", true), person_att(by_name("Oren"), "accepted"), person_att(by_name("Rachel"), "accepted")]);
        primary.events.push(board);

        let mut gym = s.event("Gym", s.timed(sunday - 14, 7, 0, 60));
        gym.recurrence = Some(vec!["RRULE:FREQ=DAILY;INTERVAL=2;COUNT=30".into()]);
        gym.visibility = Some("private".into());
        gym.transparency = Some("transparent".into());
        gym.reminders = Some(Reminders { use_default: false, overrides: Some(vec![]) });
        primary.events.push(gym);

        let mut bday = s.event("יום הולדת לנועה", s.all_day(9, 1));
        bday.recurrence = Some(vec!["RRULE:FREQ=YEARLY".into()]);
        bday.transparency = Some("transparent".into());
        bday.reminders =
            Some(Reminders { use_default: false, overrides: Some(vec![Reminder { method: "popup".into(), minutes: 15 * 60 }]) });
        primary.events.push(bday);

        // a contact's birthday, as Google Calendar shows it on the primary calendar since 2024:
        // eventType "birthday", yearly, read-only apart from reminders / colour
        let mut cbd = s.event("Sarah Cohen's birthday", s.all_day(23, 1));
        cbd.event_type = Some("birthday".into());
        cbd.recurrence = Some(vec!["RRULE:FREQ=YEARLY".into()]);
        cbd.transparency = Some("transparent".into());
        cbd.visibility = Some("private".into());
        cbd.extra.insert("birthdayProperties".into(), json!({ "type": "birthday", "contact": "people/c7000000000000007919" }));
        primary.events.push(cbd);

        // --- conferences (all-day, multi-day) ---------------------------------------------
        for (day, len, title, loc) in [
            (-5i64, 2i64, "Tel Aviv Investor Week", "David InterContinental, Tel Aviv"),
            (10, 3, "Atlas TMT Conference — New York", "Atlas Conferences, 1 Park Ave, New York"),
            (45, 2, "Healthcare Investor Summit — London", "The Landmark, London"),
            (93, 3, "Israel Tech Week", "Expo Tel Aviv"),
            (150, 2, "Energy Transition Forum — Frankfurt", "Messe Frankfurt"),
        ] {
            let mut e = s.event(title, s.all_day(day, len));
            e.location = Some(loc.into());
            e.transparency = Some("transparent".into());
            e.description = Some(format!(
                "{title}\nRegistration: atlasconf.example/register\nContact: {} ({})",
                by_name("Chris").name(),
                by_name("Chris").email()
            ));
            e.organizer = Some(Person { email: Some(conf.entry.id.clone()), display_name: Some("Conferences".into()), is_self: None });
            conf.events.push(e);
        }
        // 1:1s during the New York conference
        for (i, first) in ["Sarah", "James", "Emily", "Robert"].iter().enumerate() {
            let p = by_name(first);
            let mut e = s.event(
                &format!("1:1 — {} ({}) / Orion Technologies", p.name(), p.company),
                s.timed(10 + (i as i64) / 2, 9 + (i as u32 % 2) * 2, 0, 45),
            );
            e.location = Some("Atlas TMT Conference, Room 4B".into());
            e.attendees = Some(vec![s.me_attendee("accepted", true), person_att(p, "accepted"), person_att(by_name("Noa"), "accepted")]);
            primary.events.push(e);
        }

        // --- vacation (out of office) ---------------------------------------------------------
        let mut vac = s.event("Vacation — Eilat", s.all_day(28, 3));
        vac.extended_properties = ext(Some("oof"), None);
        vac.reminders = Some(Reminders { use_default: false, overrides: Some(vec![]) });
        primary.events.push(vac);

        // --- earnings calls ---------------------------------------------------------------
        for (day, client, q) in [
            (-9i64, "Sunfield Energy", "Q2"),
            (3, "Orion Technologies", "Q3"),
            (17, "Corvina Medical", "Q3"),
            (38, "Sunfield Energy", "Q3"),
            (100, "Orion Technologies", "Q4"),
            (128, "Corvina Medical", "Q4"),
        ] {
            let mut e = s.event(&format!("{client} {q} 2026 earnings call"), s.timed(day, 15, 30, 60));
            e.location = Some("Dial-in +1 646 555 0100, PIN 4471#".into());
            e.description = Some("Speakers: CEO, CFO. Q&A moderated by Duvz Software.\nWebcast: https://webcast.example/earnings".into());
            let (a, b) = match client {
                "Orion Technologies" => ("Noa", "Daniel"),
                "Corvina Medical" => ("Oren", "Rachel"),
                _ => ("Tamar", "Maya"),
            };
            e.attendees =
                Some(vec![s.me_attendee("accepted", true), person_att(by_name(a), "accepted"), person_att(by_name(b), "accepted")]);
            e.reminders = Some(Reminders { use_default: false, overrides: Some(vec![Reminder { method: "popup".into(), minutes: 30 }]) });
            e.extended_properties = ext(None, Some("high"));
            primary.events.push(e);
        }

        // --- invitations from others (some still awaiting my response) --------------------------
        for (k, (day, h, first, status)) in [
            (1i64, 11u32, "Yael", "needsAction"),
            (2, 14, "Thomas", "needsAction"),
            (4, 17, "Lior", "tentative"),
            (6, 10, "Sophie", "accepted"),
            (-3, 13, "Michael", "accepted"),
            (8, 12, "Robert", "needsAction"),
            (12, 16, "Anna", "declined"),
        ]
        .iter()
        .enumerate()
        {
            let p = by_name(first);
            let mut e = s.event(&format!("{} / Duvz Software — portfolio review", p.company), s.timed(*day, *h, 0, 60));
            e.ical_uid = Some(format!("invite-{k}-{}@{}", email.split('@').next().unwrap_or("u"), p.domain));
            e.organizer = Some(Person { email: Some(p.email()), display_name: Some(p.name()), is_self: None });
            e.creator = e.organizer.clone();
            e.location = Some(if k % 2 == 0 { "Microsoft Teams".into() } else { format!("{} offices", p.company) });
            e.description = Some(format!(
                "Hi Kenny,\n\nLooking forward to catching up on your coverage list.\n\nBest,\n{}\n{}, {}",
                p.first, p.title, p.company
            ));
            e.attendees = Some(vec![
                Attendee {
                    email: p.email(),
                    display_name: Some(p.name()),
                    organizer: Some(true),
                    response_status: "accepted".into(),
                    ..Default::default()
                },
                s.me_attendee(status, false),
                person_att(by_name("Maya"), if k % 3 == 0 { "accepted" } else { "needsAction" }),
            ]);
            primary.events.push(e);
        }

        // --- investor meetings and day-to-day items across −2 / +6 weeks -----------------------
        for day in -14i64..=42 {
            let d = today + Duration::days(day);
            let wd = d.weekday().num_days_from_sunday();
            if wd == 6 {
                continue; // Saturday
            }
            if wd == 5 {
                if s.rng.chance(40) {
                    let mut e = s.event("ארוחת ערב משפחתית", s.timed(day, 19, 30, 150));
                    e.visibility = Some("private".into());
                    e.reminders = Some(Reminders { use_default: false, overrides: Some(vec![]) });
                    primary.events.push(e);
                }
                continue;
            }
            let n = 1 + s.rng.below(2);
            let mut used: Vec<u32> = vec![];
            for _ in 0..n {
                let hour = *s.rng.pick(&[9u32, 11, 13, 14, 16]);
                if used.contains(&hour) {
                    continue;
                }
                used.push(hour);
                let kind = s.rng.below(10);
                let client = *s.rng.pick(CLIENTS);
                let inv = by_name(s.rng.pick(INVESTOR_FIRSTS));
                let e = match kind {
                    0..=4 => {
                        let mut e = s.event(&format!("Meeting: {} / {}", inv.company, client), s.timed(day, hour, 0, 60));
                        e.location = Some(if s.rng.chance(50) { "Zoom".into() } else { format!("{} offices", inv.company) });
                        let cp = match client {
                            "Orion Technologies" => by_name("Daniel"),
                            "Corvina Medical" => by_name("Rachel"),
                            _ => by_name("Tamar"),
                        };
                        e.attendees = Some(vec![
                            s.me_attendee("accepted", true),
                            person_att(inv, if s.rng.chance(80) { "accepted" } else { "needsAction" }),
                            person_att(cp, "accepted"),
                        ]);
                        e.description = Some(format!(
                            "Investor: {} ({}, {})\nCompany: {}\nFocus: strategy, margins, capital allocation.",
                            inv.name(),
                            inv.title,
                            inv.company,
                            client
                        ));
                        e
                    }
                    5 => {
                        let mut e = s.event(&format!("Call with {}", inv.name()), s.timed(day, hour, 30, 30));
                        e.location = Some(inv.phone.into());
                        e
                    }
                    6 => {
                        let mut e = s.event(&format!("{client} — draft press release review"), s.timed(day, hour, 0, 90));
                        e.extended_properties = ext(Some("tentative"), None);
                        e
                    }
                    7 => {
                        let mut e = s.event(&format!("Lunch with {}", inv.name()), s.timed(day, 12, 30, 75));
                        e.location = Some(
                            s.rng
                                .pick(&[
                                    "Café Noir, Tel Aviv",
                                    "Shila, Ben Yehuda 182",
                                    "Herbert Samuel, Tel Aviv",
                                    "Social Club, Rothschild 45",
                                ])
                                .to_string(),
                        );
                        e
                    }
                    8 => {
                        let mut e = s.event("שיחה עם רואה החשבון", s.timed(day, hour, 0, 30));
                        e.location = Some("טלפון".into());
                        e.visibility = Some("private".into());
                        e
                    }
                    _ => {
                        let mut e = s.event(&format!("Prepare {client} investor deck"), s.timed(day, hour, 0, 120));
                        e.transparency = Some("transparent".into());
                        e
                    }
                };
                primary.events.push(e);
            }
        }

        // --- duplicates for "Clean Up Calendar" ----------------------------------------------
        // Gmail created the hotel reservation three times (different ids / iCalUIDs)
        for k in 0..3 {
            let mut e = s.event("Stay at Grand Hyatt San Francisco", s.all_day(1, 3));
            e.event_type = Some("fromGmail".into());
            e.location = Some("Grand Hyatt San Francisco, 345 Stockton St, San Francisco, CA 94108".into());
            e.transparency = Some("transparent".into());
            e.reminders = Some(Reminders { use_default: false, overrides: Some(vec![]) });
            if k == 0 {
                e.description = Some(
                    "Reservation 84311207 — check-in 15:00, check-out 11:00.
From your Gmail."
                        .into(),
                );
            }
            e.created = Some(fmt_dt(s.at(-30 + k, 9, 0), s.tz));
            primary.events.push(e);
        }
        // a series made in Outlook: Google keeps its notes as Outlook's HTML
        let mut ol = s.event("Orion weekly IR call update", s.timed(1, 17, 0, 30));
        ol.location = Some("https://zoom.us/j/8125550147?pwd=dEx4QmZh".into());
        ol.description = Some(
            "<!-- Converted from text/plain format --><P><FONT SIZE=2>&lt;<A HREF=\"https://zoom.us/j/8125550147?pwd=dEx4QmZh\">https://zoom.us/j/8125550147?pwd=dEx4QmZh</A>&gt;<BR><BR>Meeting ID: 812 555 0147<BR><BR>Passcode: 7319<BR><BR>One tap mobile<BR><BR>+15555550100,,8125550147# US<BR></FONT></P>"
                .into(),
        );
        ol.ical_uid = Some("040000008200E00074C5B7101A82E00800000000ORIONWEEKLY".into());
        primary.events.push(ol);

        // the same call entered twice, one copy with notes
        for k in 0..2 {
            let p = by_name("David");
            let mut e = s.event(&format!("Call with {}", p.name()), s.timed(3, 12, 0, 30));
            e.location = Some(p.phone.into());
            if k == 1 {
                e.description = Some("Follow up on the Orion Q3 numbers; he asked about gross margin guidance.".into());
            }
            e.created = Some(fmt_dt(s.at(-10 + k, 9, 0), s.tz));
            primary.events.push(e);
        }
        // a meeting organised by someone else that landed twice (an import ran twice)
        for k in 0..2 {
            let p = by_name("Emily");
            let mut e = s.event(&format!("{} / Duvz Software — intro", p.company), s.timed(4, 14, 0, 45));
            e.id = Some(format!("dupinvite{k}"));
            e.ical_uid = Some(format!("dupinvite{k}@{}", p.domain));
            e.organizer = Some(Person { email: Some(p.email()), display_name: Some(p.name()), is_self: None });
            e.attendees = Some(vec![
                Attendee {
                    email: p.email(),
                    display_name: Some(p.name()),
                    organizer: Some(true),
                    response_status: "accepted".into(),
                    ..Default::default()
                },
                s.me_attendee("accepted", false),
            ]);
            primary.events.push(e);
        }

        // --- further out ----------------------------------------------------------------------
        let mut agm = s.event("Annual general meeting — Sunfield Energy", s.timed(140, 11, 0, 180));
        agm.location = Some("Sunfield Energy, Beer Sheva".into());
        agm.attendees = Some(vec![s.me_attendee("accepted", true), person_att(by_name("Tamar"), "accepted")]);
        primary.events.push(agm);
        let mut nyx = s.event("Non-deal roadshow — Boston (Orion)", s.all_day(60, 2));
        nyx.extended_properties = ext(Some("oof"), None);
        primary.events.push(nyx);
    } else {
        // --- info@ (company mailbox): lighter calendar ---------------------------------------
        let mut nl = s.event("Newsletter deadline", s.timed(sunday - 21 + 4, 12, 0, 30));
        nl.recurrence = Some(vec!["RRULE:FREQ=WEEKLY;BYDAY=TH".into()]);
        primary.events.push(nl);
        let mut filings = s.event("Quarterly filings check", s.timed(-20, 9, 0, 60));
        filings.recurrence = Some(vec!["RRULE:FREQ=MONTHLY;BYMONTHDAY=15;COUNT=12".into()]);
        primary.events.push(filings);
        for day in -14i64..=42 {
            let wd = (today + Duration::days(day)).weekday().num_days_from_sunday();
            if wd >= 5 || !s.rng.chance(35) {
                continue;
            }
            let client = *s.rng.pick(CLIENTS);
            let inv = by_name(s.rng.pick(INVESTOR_FIRSTS));
            let hour = *s.rng.pick(&[10u32, 13, 15]);
            let mut e = s.event(&format!("Website update — {client}"), s.timed(day, hour, 0, 45));
            if s.rng.chance(50) {
                e.summary = Some(format!("Investor inquiry follow-up: {}", inv.company));
                e.attendees = Some(vec![s.me_attendee("accepted", true), person_att(inv, "needsAction")]);
            }
            primary.events.push(e);
        }
        for (day, len, title) in [(-6i64, 1i64, "Office closed"), (20, 1, "Team offsite"), (55, 2, "Client summit")] {
            let mut e = s.event(title, s.all_day(day, len));
            e.transparency = Some("transparent".into());
            conf.events.push(e);
        }
    }

    // an invitation sent to both mailboxes: the same iCalUID on both primary calendars
    {
        let p = by_name("Chris");
        let domain = email.split('@').nth(1).unwrap_or("duvz.demo");
        let mut e = s.event("Atlas TMT Conference — speaker briefing", s.timed(2, 17, 0, 60));
        e.id = Some("atlasbriefing2026".into());
        e.ical_uid = Some("atlas-briefing-2026@atlasconf.demo".into());
        e.organizer = Some(Person { email: Some(p.email()), display_name: Some(p.name()), is_self: None });
        e.creator = e.organizer.clone();
        e.location = Some("Zoom".into());
        // no description: the organiser's Zoom add-on keeps the call details in conferenceData
        e.extra.insert(
            "conferenceData".into(),
            json!({
                "conferenceSolution": { "name": "Zoom Meeting", "key": { "type": "addOn" } },
                "conferenceId": "81234567890",
                "entryPoints": [
                    { "entryPointType": "video", "uri": "https://atlasconf.zoom.us/j/81234567890?pwd=AbCdEf12345678", "label": "atlasconf.zoom.us/j/81234567890", "meetingCode": "81234567890", "passcode": "135790" },
                    { "entryPointType": "phone", "uri": "tel:+442039017895,,81234567890#", "label": "+44 20 3901 7895", "regionCode": "GB", "passcode": "135790" },
                    { "entryPointType": "phone", "uri": "tel:+13126266799,,81234567890#", "label": "+1 312-626-6799", "regionCode": "US", "passcode": "135790" },
                    { "entryPointType": "sip", "uri": "sip:81234567890@zoomcrc.com", "label": "81234567890@zoomcrc.com", "passcode": "135790" },
                    { "entryPointType": "more", "uri": "https://www.google.com/url?q=https://applications.zoom.us/addon/invitation/detail?meetingUuid%3DexampleUuid%26v%3D1&sa=D&source=calendar&usg=AOvVaw0example" }
                ],
                "notes": "Meeting host: <a href=\"mailto:chris.donovan@atlasconf.example\" target=\"_blank\">chris.donovan@atlasconf.example</a><br /><br />Join Zoom Meeting: <br /><a href=\"https://www.google.com/url?q=https://atlasconf.zoom.us/j/81234567890?pwd%3DAbCdEf12345678&amp;sa=D&amp;source=calendar\" target=\"_blank\">https://atlasconf.zoom.us/j/81234567890?pwd=AbCdEf12345678</a><br /><br />Meeting agenda: <br /><a href=\"https://www.google.com/url?q=https://docs.zoom.us/agenda/doc/0a1b2c3d?from%3Dgsuite&amp;sa=D\" target=\"_blank\">https://docs.zoom.us/agenda/doc/0a1b2c3d?from=gsuite</a>"
            }),
        );
        let me = |local: &str, nm: &str| Attendee {
            email: format!("{local}@{domain}"),
            display_name: Some(nm.into()),
            response_status: "accepted".into(),
            ..Default::default()
        };
        e.attendees = Some(vec![
            Attendee {
                email: p.email(),
                display_name: Some(p.name()),
                organizer: Some(true),
                response_status: "accepted".into(),
                ..Default::default()
            },
            me("kenny", "Kenny Green"),
            me("info", "Duvz Software"),
        ]);
        primary.events.push(e);
    }

    let mut cals = vec![primary, conf];
    if !is_info {
        let mut hol = FakeCalendar {
            entry: entry("en.jewish#holiday@group.v.calendar.google.com", "Israel holidays", "#ffad46", "reader", false, vec![]),
            events: vec![],
        };
        for (date, title, len) in HOLIDAYS {
            let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
            let mut e = s.event(title, s.all_day((d - today).num_days(), *len));
            e.transparency = Some("transparent".into());
            e.organizer = Some(Person { email: Some(hol.entry.id.clone()), display_name: Some("Israel holidays".into()), is_self: None });
            e.reminders = Some(Reminders { use_default: false, overrides: None });
            hol.events.push(e);
        }
        cals.push(hol);
        // a family member's personal Gmail calendar shared with Kenny (can edit)
        let shared_id = "robin.parker@gmail.demo";
        let mut fam = FakeCalendar { entry: entry(shared_id, shared_id, "#f691b2", "writer", false, vec![]), events: vec![] };
        let owner = Person { email: Some(shared_id.into()), display_name: Some("Robin Parker".into()), is_self: None };
        for (day, h, m, mins, title) in [
            (0i64, 16u32, 0u32, 60i64, "Noa — dentist"),
            (1, 18, 30, 90, "Parent-teacher evening"),
            (3, 7, 30, 30, "Car service (drop off)"),
            (8, 17, 0, 60, "Swimming lesson"),
            (-4, 20, 0, 120, "Book club"),
        ] {
            let mut e = s.event(title, s.timed(day, h, m, mins));
            e.organizer = Some(owner.clone());
            e.creator = Some(owner.clone());
            fam.events.push(e);
        }
        // a dinner she invited Kenny to: it is on his own calendar too (same iCalUID)
        let mut dinner = s.event("Dinner with the Parkers", s.timed(2, 19, 30, 150));
        dinner.organizer = Some(owner.clone());
        dinner.creator = Some(owner.clone());
        dinner.ical_uid = Some("family-dinner@gmail.demo".into());
        dinner.location = Some("Ra'anana".into());
        dinner.attendees = Some(vec![
            Attendee {
                email: shared_id.into(),
                display_name: Some("Robin Parker".into()),
                organizer: Some(true),
                response_status: "accepted".into(),
                ..Default::default()
            },
            s.me_attendee("accepted", false),
        ]);
        let mut mine = dinner.clone();
        mine.id = Some(s.id());
        cals[0].events.push(mine);
        fam.events.push(dinner);
        cals.push(fam);
        // a colleague's calendar shared as free/busy only: Outlook-style clients don't list it
        cals.push(FakeCalendar {
            entry: entry("dana.levi@duvz.demo", "Dana Levi", "#cd74e6", "freeBusyReader", false, vec![]),
            events: vec![],
        });
    }
    Store { calendars: cals }
}

/// Seeded invitations (meetings organised by someone else) for `email`, so a fake mail seed can
/// send matching invitation e-mails (same iCalUID). Deterministic for a given day.
pub fn seeded_invites(email: &str, name: &str) -> Vec<Event> {
    seed(email, name)
        .calendars
        .into_iter()
        .take(1)
        .flat_map(|c| c.events)
        // the duplicate / cross-account test items don't come with invitation mails
        .filter(|e| !e.id.as_deref().is_some_and(|i| i.starts_with("dupinvite") || i == "atlasbriefing2026"))
        .filter(|e| e.ical_uid.as_deref() != Some("family-dinner@gmail.demo"))
        .filter(|e| {
            e.organizer
                .as_ref()
                .and_then(|o| o.email.as_deref())
                .is_some_and(|o| !o.eq_ignore_ascii_case(email) && o.contains('@') && !o.contains("calendar.google.com"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_has_plausible_volume() {
        let st = seed("kenny@duvz.demo", "Kenny Green");
        assert_eq!(st.calendars.len(), 5, "primary, conferences, holidays, a shared personal calendar, a free/busy-only colleague");
        let total: usize = st.calendars.iter().map(|c| c.events.len()).sum();
        assert!(total >= 60, "only {total} events");
        let now = seed::now_ms();
        let c = &st.calendars[0];
        let inst = listing(c, true, now - 14 * DAY, now + 42 * DAY, &HashMap::new());
        assert!(inst.len() >= 60 && inst.len() <= 250, "{} instances", inst.len());
        // the moved exception replaces its generated instance; the cancelled one is gone
        assert!(inst.iter().any(|e| e.summary.as_deref() == Some("Weekly team call (moved)")));
        assert!(!seeded_invites("kenny@duvz.demo", "Kenny Green").is_empty());
    }
}

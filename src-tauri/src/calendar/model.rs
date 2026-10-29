//! Local rows ↔ Google JSON, and the DTOs the front end sees.

use chrono::{DateTime, NaiveDate, TimeZone};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::google::calendar::{Attendee, Event, EventDateTime, Reminder, Reminders};

pub const DAY_MS: i64 = 86_400_000;

/// A calendar row (subset needed by sync / handlers).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalRow {
    pub id: i64,
    pub account_id: i64,
    pub gcal_id: String,
    pub summary: String,
    pub color: String,
    pub selected: bool,
    pub access_role: String,
    pub is_primary: bool,
    pub time_zone: String,
    pub default_reminders: Vec<Reminder>,
}

impl CalRow {
    pub fn from_row(r: &Row) -> rusqlite::Result<CalRow> {
        Ok(CalRow {
            id: r.get("id")?,
            account_id: r.get("account_id")?,
            gcal_id: r.get("gcal_id")?,
            summary: r.get("summary")?,
            color: r.get("color")?,
            selected: r.get::<_, i64>("selected")? != 0,
            access_role: r.get("access_role")?,
            is_primary: r.get::<_, i64>("is_primary")? != 0,
            time_zone: r.get("time_zone")?,
            default_reminders: serde_json::from_str(&r.get::<_, String>("default_reminders")?).unwrap_or_default(),
        })
    }
    pub fn can_write(&self) -> bool {
        matches!(self.access_role.as_str(), "owner" | "writer")
    }
    pub fn tz(&self) -> chrono_tz::Tz {
        self.time_zone.parse().unwrap_or(chrono_tz::UTC)
    }
    pub fn default_reminder_min(&self) -> Option<i64> {
        pick_reminder(&self.default_reminders)
    }
}

pub fn get_cal(c: &Connection, id: i64) -> anyhow::Result<CalRow> {
    c.query_row("SELECT * FROM calendars WHERE id=?1", [id], CalRow::from_row)
        .optional()?
        .ok_or_else(|| crate::user_err!("not_found", "calendar {id} not found"))
}

fn pick_reminder(r: &[Reminder]) -> Option<i64> {
    r.iter().filter(|x| x.method == "popup").map(|x| x.minutes).min().or_else(|| r.iter().map(|x| x.minutes).min())
}

/// Effective reminder (minutes before start) for an event's `reminders` on a calendar.
pub fn effective_reminder(rem: Option<&Reminders>, cal: &CalRow) -> Option<i64> {
    match rem {
        None => cal.default_reminder_min(),
        Some(r) if r.use_default => cal.default_reminder_min(),
        Some(r) => pick_reminder(r.overrides.as_deref().unwrap_or(&[])),
    }
}

/// Midnight of a calendar date in the system's local zone (how all-day items are drawn).
pub fn local_midnight_ms(d: &str) -> Option<i64> {
    let d = NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()?;
    chrono::Local.from_local_datetime(&d.and_hms_opt(0, 0, 0)?).earliest().map(|t| t.timestamp_millis())
}

pub fn local_date_of(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms).unwrap_or_default().with_timezone(&chrono::Local).format("%Y-%m-%d").to_string()
}

pub use crate::google::rfc3339_ms as parse_rfc3339;

/// All columns of an `events` row.
#[derive(Debug, Clone, Default)]
pub struct EventRow {
    pub id: i64,
    pub account_id: i64,
    pub calendar_id: i64,
    pub gcal_id: Option<String>,
    pub recurring_event_id: Option<String>,
    pub is_master: bool,
    pub parent_local_id: Option<i64>,
    pub original_start_ms: Option<i64>,
    pub ical_uid: Option<String>,
    pub summary: String,
    pub location: String,
    pub description: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub all_day: bool,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub time_zone: Option<String>,
    pub status: String,
    pub organizer_email: Option<String>,
    pub organizer_name: Option<String>,
    pub organizer_self: bool,
    pub attendees: Vec<Attendee>,
    pub reminders: Option<Reminders>,
    pub reminder_min: Option<i64>,
    pub recurrence: Option<Vec<String>>,
    pub transparency: String,
    pub visibility: String,
    pub show_as: Option<String>,
    pub importance: Option<String>,
    pub event_type: String,
    pub html_link: Option<String>,
    pub etag: Option<String>,
    pub updated_ms: Option<i64>,
    pub my_response: Option<String>,
    pub color_id: Option<String>,
    pub local_dirty: bool,
    /// Outlook colour categories (names), synced as extendedProperties.private.cmCategories
    pub categories: Vec<String>,
    /// Google `created`
    pub created_ms: Option<i64>,
    /// Zoom / Meet link, dial-ins, notes from Google's conferenceData (read-only)
    pub conference: String,
    /// A meeting someone else organised that you renamed on your calendar (Outlook lets you): the
    /// organizer's subject, while `summary` holds yours. Yours is kept on your copy of the event as
    /// extendedProperties.private.cmSummary; Google and the other guests keep the organizer's.
    pub summary_orig: Option<String>,
}

/// extendedProperties.private key of your own subject for an invitation (`EventRow::summary_orig`).
pub const OWN_SUMMARY: &str = "cmSummary";

impl EventRow {
    pub fn from_row(r: &Row) -> rusqlite::Result<EventRow> {
        fn json_opt<T: serde::de::DeserializeOwned>(s: Option<String>) -> Option<T> {
            s.and_then(|s| serde_json::from_str(&s).ok())
        }
        Ok(EventRow {
            id: r.get("id")?,
            account_id: r.get("account_id")?,
            calendar_id: r.get("calendar_id")?,
            gcal_id: r.get("gcal_id")?,
            recurring_event_id: r.get("recurring_event_id")?,
            is_master: r.get::<_, i64>("is_master")? != 0,
            parent_local_id: r.get("parent_local_id")?,
            original_start_ms: r.get("original_start_ms")?,
            ical_uid: r.get("ical_uid")?,
            summary: r.get("summary")?,
            location: r.get("location")?,
            description: r.get("description")?,
            start_ms: r.get("start_ms")?,
            end_ms: r.get("end_ms")?,
            all_day: r.get::<_, i64>("all_day")? != 0,
            start_date: r.get("start_date")?,
            end_date: r.get("end_date")?,
            time_zone: r.get("time_zone")?,
            status: r.get("status")?,
            organizer_email: r.get("organizer_email")?,
            organizer_name: r.get("organizer_name")?,
            organizer_self: r.get::<_, i64>("organizer_self")? != 0,
            attendees: serde_json::from_str(&r.get::<_, String>("attendees")?).unwrap_or_default(),
            reminders: json_opt(r.get("reminders")?),
            reminder_min: r.get("reminder_min")?,
            recurrence: json_opt(r.get("recurrence")?),
            transparency: r.get("transparency")?,
            visibility: r.get("visibility")?,
            show_as: r.get("show_as")?,
            importance: r.get("importance")?,
            event_type: r.get("event_type")?,
            html_link: r.get("html_link")?,
            etag: r.get("etag")?,
            updated_ms: r.get("updated_ms")?,
            my_response: r.get("my_response")?,
            color_id: r.get("color_id")?,
            local_dirty: r.get::<_, i64>("local_dirty")? != 0,
            categories: super::categories::parse_list(&r.get::<_, String>("categories")?),
            created_ms: r.get("created_ms")?,
            conference: r.get("conference")?,
            summary_orig: r.get("summary_orig")?,
        })
    }

    /// Google event types whose time / details can't be edited through the API: birthdays
    /// (come from Contacts), events Gmail created (flights, reservations) and working locations.
    pub fn type_read_only(&self) -> bool {
        matches!(self.event_type.as_str(), "birthday" | "fromGmail" | "workingLocation")
    }

    pub fn is_meeting(&self) -> bool {
        self.attendees.iter().any(|a| a.is_self != Some(true) && a.organizer != Some(true))
            || (!self.organizer_self
                && self.organizer_email.as_deref().is_some_and(|o| o.contains('@') && !o.ends_with("calendar.google.com")))
    }

    pub fn busy_status(&self) -> &str {
        if let Some(s) = self.show_as.as_deref()
            && matches!(s, "busy" | "free" | "tentative" | "oof")
        {
            return s;
        }
        if self.event_type == "outOfOffice" {
            return "oof";
        }
        if self.transparency == "transparent" {
            return "free";
        }
        if self.my_response.as_deref() == Some("tentative") || self.my_response.as_deref() == Some("needsAction") {
            return "tentative";
        }
        "busy"
    }

    pub fn is_private(&self) -> bool {
        matches!(self.visibility.as_str(), "private" | "confidential")
    }

    /// Insert as a new row; returns the id.
    pub fn insert(&self, c: &Connection) -> anyhow::Result<i64> {
        c.execute(
            "INSERT INTO events(account_id, calendar_id, gcal_id, recurring_event_id, is_master, parent_local_id, original_start_ms, ical_uid, \
             summary, location, description, start_ms, end_ms, all_day, start_date, end_date, time_zone, status, organizer_email, organizer_name, \
             organizer_self, attendees, reminders, reminder_min, recurrence, transparency, visibility, show_as, importance, event_type, html_link, \
             etag, updated_ms, my_response, color_id, local_dirty, categories, created_ms, conference, summary_orig) VALUES \
             (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,?36,?37,?38,?39,?40)",
            params![
                self.account_id,
                self.calendar_id,
                self.gcal_id,
                self.recurring_event_id,
                self.is_master as i64,
                self.parent_local_id,
                self.original_start_ms,
                self.ical_uid,
                self.summary,
                self.location,
                self.description,
                self.start_ms,
                self.end_ms,
                self.all_day as i64,
                self.start_date,
                self.end_date,
                self.time_zone,
                self.status,
                self.organizer_email,
                self.organizer_name,
                self.organizer_self as i64,
                serde_json::to_string(&self.attendees)?,
                self.reminders.as_ref().map(serde_json::to_string).transpose()?,
                self.reminder_min,
                self.recurrence.as_ref().map(serde_json::to_string).transpose()?,
                self.transparency,
                self.visibility,
                self.show_as,
                self.importance,
                self.event_type,
                self.html_link,
                self.etag,
                self.updated_ms,
                self.my_response,
                self.color_id,
                self.local_dirty as i64,
                super::categories::join_list(&self.categories),
                self.created_ms,
                self.conference,
                self.summary_orig,
            ],
        )?;
        Ok(c.last_insert_rowid())
    }

    /// Overwrite every column of row `self.id`.
    pub fn save(&self, c: &Connection) -> anyhow::Result<()> {
        c.execute(
            "UPDATE events SET calendar_id=?2, gcal_id=?3, recurring_event_id=?4, is_master=?5, parent_local_id=?6, original_start_ms=?7, ical_uid=?8, \
             summary=?9, location=?10, description=?11, start_ms=?12, end_ms=?13, all_day=?14, start_date=?15, end_date=?16, time_zone=?17, status=?18, \
             organizer_email=?19, organizer_name=?20, organizer_self=?21, attendees=?22, reminders=?23, reminder_min=?24, recurrence=?25, \
             transparency=?26, visibility=?27, show_as=?28, importance=?29, event_type=?30, html_link=?31, etag=?32, updated_ms=?33, my_response=?34, \
             color_id=?35, local_dirty=?36, categories=?37, created_ms=?38, conference=?39, summary_orig=?40 WHERE id=?1",
            params![
                self.id,
                self.calendar_id,
                self.gcal_id,
                self.recurring_event_id,
                self.is_master as i64,
                self.parent_local_id,
                self.original_start_ms,
                self.ical_uid,
                self.summary,
                self.location,
                self.description,
                self.start_ms,
                self.end_ms,
                self.all_day as i64,
                self.start_date,
                self.end_date,
                self.time_zone,
                self.status,
                self.organizer_email,
                self.organizer_name,
                self.organizer_self as i64,
                serde_json::to_string(&self.attendees)?,
                self.reminders.as_ref().map(serde_json::to_string).transpose()?,
                self.reminder_min,
                self.recurrence.as_ref().map(serde_json::to_string).transpose()?,
                self.transparency,
                self.visibility,
                self.show_as,
                self.importance,
                self.event_type,
                self.html_link,
                self.etag,
                self.updated_ms,
                self.my_response,
                self.color_id,
                self.local_dirty as i64,
                super::categories::join_list(&self.categories),
                self.created_ms,
                self.conference,
                self.summary_orig,
            ],
        )?;
        Ok(())
    }

    /// Google request body for insert / full patch.
    pub fn to_google(&self, cal: &CalRow) -> Value {
        let tz = self.time_zone.clone().unwrap_or_else(|| cal.time_zone.clone());
        let (start, end) = if self.all_day {
            (
                json!({ "date": self.start_date.clone().unwrap_or_else(|| local_date_of(self.start_ms)) }),
                json!({ "date": self.end_date.clone().unwrap_or_else(|| local_date_of(self.end_ms)) }),
            )
        } else {
            let z: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
            let f = |ms: i64| {
                DateTime::from_timestamp_millis(ms)
                    .unwrap_or_default()
                    .with_timezone(&z)
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
            };
            (json!({ "dateTime": f(self.start_ms), "timeZone": tz }), json!({ "dateTime": f(self.end_ms), "timeZone": tz }))
        };
        let mut body = json!({
            // your own subject for an invitation stays on your copy (cmSummary), not in its subject
            "summary": self.summary_orig.as_ref().unwrap_or(&self.summary),
            "location": self.location,
            "description": self.description,
            "start": start,
            "end": end,
            "transparency": if self.busy_status() == "free" { "transparent" } else { "opaque" },
            "visibility": self.visibility,
            "reminders": self.reminders.clone().unwrap_or(Reminders { use_default: true, overrides: None }),
            "extendedProperties": { "private": {
                "cmShowAs": self.show_as.clone().unwrap_or_else(|| self.busy_status().to_string()),
                "cmImportance": self.importance.clone().unwrap_or_else(|| "normal".into()),
                "cmCategories": super::categories::join_list(&self.categories),
                OWN_SUMMARY: self.own_summary(),
            }},
            "colorId": self.color_id,
        });
        if !self.attendees.is_empty() {
            body["attendees"] = attendees_body(&self.attendees);
        }
        if self.is_master || self.recurrence.is_some() {
            body["recurrence"] = json!(self.recurrence.clone().unwrap_or_default());
        }
        body
    }

    /// Fill from a Google event (keeps id / calendar / account / dirty).
    pub fn apply_google(&mut self, ev: &Event, cal: &CalRow, my_email: &str) {
        self.gcal_id = ev.id.clone();
        self.recurring_event_id = ev.recurring_event_id.clone();
        self.is_master = ev.recurrence.as_ref().is_some_and(|r| !r.is_empty());
        self.ical_uid = ev.ical_uid.clone();
        self.summary = ev.summary.clone().unwrap_or_default();
        self.location = ev.location.clone().unwrap_or_default();
        self.description = ev.description.clone().unwrap_or_default();
        let (s, e, all_day, sd, ed, etz) = times(ev.start.as_ref(), ev.end.as_ref());
        self.start_ms = s;
        self.end_ms = e;
        self.all_day = all_day;
        self.start_date = sd;
        self.end_date = ed;
        self.time_zone = etz;
        self.original_start_ms = ev.original_start_time.as_ref().map(|o| times(Some(o), None).0);
        self.status = ev.status.clone().unwrap_or_else(|| "confirmed".into());
        self.organizer_email = ev.organizer.as_ref().and_then(|o| o.email.clone());
        self.organizer_name = ev.organizer.as_ref().and_then(|o| o.display_name.clone());
        self.organizer_self = ev
            .organizer
            .as_ref()
            .map(|o| o.is_self == Some(true) || o.email.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(my_email) || m == cal.gcal_id))
            .unwrap_or(true);
        self.attendees = ev.attendees.clone().unwrap_or_default();
        for a in self.attendees.iter_mut() {
            if a.is_self.is_none() && a.email.eq_ignore_ascii_case(my_email) {
                a.is_self = Some(true);
            }
        }
        self.my_response = if self.organizer_self {
            None
        } else {
            self.attendees.iter().find(|a| a.is_self == Some(true)).map(|a| a.response_status.clone())
        };
        self.reminders = ev.reminders.clone();
        self.reminder_min = effective_reminder(ev.reminders.as_ref(), cal);
        self.recurrence = ev.recurrence.clone();
        self.transparency = ev.transparency.clone().unwrap_or_else(|| "opaque".into());
        self.visibility = ev.visibility.clone().unwrap_or_else(|| "default".into());
        let private = ev.extended_properties.as_ref().and_then(|x| x.private.as_ref());
        self.show_as = private.and_then(|p| p.get("cmShowAs")).and_then(|v| v.as_str()).map(str::to_string);
        self.importance = private.and_then(|p| p.get("cmImportance")).and_then(|v| v.as_str()).map(str::to_string);
        self.event_type = ev.event_type.clone().unwrap_or_else(|| "default".into());
        self.html_link = ev.html_link.clone();
        self.etag = ev.etag.clone();
        self.updated_ms = ev.updated.as_deref().and_then(parse_rfc3339);
        self.color_id = ev.color_id.clone();
        self.categories = private
            .and_then(|p| p.get(super::categories::PROP))
            .and_then(|v| v.as_str())
            .map(super::categories::parse_list)
            .unwrap_or_default();
        self.created_ms = ev.created.as_deref().and_then(parse_rfc3339);
        self.conference = super::conference::conference_text(ev.extra.get("conferenceData"), ev.hangout_link.as_deref());
        // your own subject for someone else's meeting (an organizer's own copy never has one)
        let own = private.and_then(|p| p.get(OWN_SUMMARY)).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
        match own {
            Some(own) if !self.organizer_self => {
                self.summary_orig = Some(std::mem::replace(&mut self.summary, own.to_string()));
            }
            _ => self.summary_orig = None,
        }
    }

    /// Your own subject for an invitation ("" = none), as kept in cmSummary.
    pub fn own_summary(&self) -> &str {
        if self.summary_orig.is_some() { &self.summary } else { "" }
    }

    /// Body for a colour / categories-only patch (allowed on every event type).
    pub fn cosmetic_body(&self) -> Value {
        json!({
            "extendedProperties": { "private": {
                super::categories::PROP: super::categories::join_list(&self.categories),
                OWN_SUMMARY: self.own_summary(),
            } },
            "colorId": self.color_id,
        })
    }
}

pub fn attendees_body(a: &[Attendee]) -> Value {
    let list: Vec<Value> = a
        .iter()
        .map(|x| {
            let mut x = x.clone();
            x.is_self = None;
            serde_json::to_value(x).unwrap_or(Value::Null)
        })
        .collect();
    Value::Array(list)
}

/// (start ms, end ms, all day, start date, end date, time zone)
fn times(start: Option<&EventDateTime>, end: Option<&EventDateTime>) -> (i64, i64, bool, Option<String>, Option<String>, Option<String>) {
    let Some(st) = start else { return (0, 0, false, None, None, None) };
    if let Some(d) = &st.date {
        let ed = end.and_then(|e| e.date.clone()).unwrap_or_else(|| super::add_days(d, 1));
        let s = local_midnight_ms(d).unwrap_or(0);
        let e = local_midnight_ms(&ed).unwrap_or(s + DAY_MS);
        return (s, e.max(s), true, Some(d.clone()), Some(ed), None);
    }
    let s = st.date_time.as_deref().and_then(parse_rfc3339).unwrap_or(0);
    let e = end.and_then(|e| e.date_time.as_deref()).and_then(parse_rfc3339).unwrap_or(s);
    (s, e.max(s), false, None, None, st.time_zone.clone())
}

// ---- DTOs ------------------------------------------------------------------------------------

/// The cross-domain `CalEvent` (docs/contracts.md).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalEvent {
    pub id: i64,
    pub account_id: i64,
    pub calendar_id: i64,
    pub calendar_color: String,
    pub summary: String,
    /// you renamed this invitation on your calendar: the organizer's subject (`summary` is yours)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_summary: Option<String>,
    pub location: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    pub is_recurring: bool,
    pub is_meeting: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_response: Option<String>,
    pub busy_status: String,
    pub is_private: bool,
    pub has_reminder: bool,
    // --- extras beyond the contract (used by the calendar module) ---
    pub is_exception: bool,
    pub importance: String,
    pub is_organizer: bool,
    pub read_only: bool,
    pub categories: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_id: Option<String>,
    /// display de-duplication: the other calendars this item also appears in
    #[serde(skip_serializing_if = "Option::is_none")]
    pub also_in: Option<Vec<String>>,
}

pub fn to_cal_event(e: &EventRow, color: &str, read_only: bool) -> CalEvent {
    let read_only = read_only || e.type_read_only();
    CalEvent {
        id: e.id,
        account_id: e.account_id,
        calendar_id: e.calendar_id,
        calendar_color: color.to_string(),
        summary: e.summary.clone(),
        original_summary: e.summary_orig.clone(),
        location: e.location.clone(),
        start: e.start_ms,
        end: e.end_ms,
        all_day: e.all_day,
        start_date: e.start_date.clone(),
        end_date: e.end_date.clone(),
        is_recurring: e.recurring_event_id.is_some() || e.parent_local_id.is_some() || e.is_master,
        is_meeting: e.is_meeting(),
        my_response: e.my_response.clone(),
        busy_status: e.busy_status().to_string(),
        is_private: e.is_private(),
        has_reminder: e.reminder_min.is_some(),
        is_exception: e.recurring_event_id.is_some() && e.original_start_ms.is_some_and(|o| o != e.start_ms),
        importance: e.importance.clone().unwrap_or_else(|| "normal".into()),
        is_organizer: e.organizer_self,
        read_only,
        categories: e.categories.clone(),
        color_id: e.color_id.clone(),
        also_in: None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttendeeDto {
    pub email: String,
    pub name: String,
    /// accepted | tentative | declined | needsAction
    pub response: String,
    pub organizer: bool,
    pub optional: bool,
    pub is_self: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDetail {
    #[serde(flatten)]
    pub ev: CalEvent,
    pub description: String,
    /// video-call details (Zoom / Meet), shown apart from the notes
    pub conference: String,
    pub attendees: Vec<AttendeeDto>,
    pub organizer_email: Option<String>,
    pub organizer_name: Option<String>,
    pub reminder_minutes: Option<i64>,
    pub visibility: String,
    pub html_link: Option<String>,
    pub time_zone: Option<String>,
    pub calendar_name: String,
    pub account_email: String,
    /// local id of the series master (when this is an occurrence or the master itself)
    pub series_id: Option<i64>,
    /// recurrence lines of the series
    pub recurrence: Option<Vec<String>>,
    /// start / end of the series' first occurrence (for the recurrence dialog)
    pub series_start: Option<i64>,
    pub series_end: Option<i64>,
    pub original_start: Option<i64>,
    pub is_master: bool,
    pub pending: bool,
}

pub fn attendee_dtos(a: &[Attendee]) -> Vec<AttendeeDto> {
    a.iter()
        .map(|x| AttendeeDto {
            email: x.email.clone(),
            name: x.display_name.clone().unwrap_or_default(),
            response: x.response_status.clone(),
            organizer: x.organizer == Some(true),
            optional: x.optional == Some(true),
            is_self: x.is_self == Some(true),
            comment: x.comment.clone(),
        })
        .collect()
}

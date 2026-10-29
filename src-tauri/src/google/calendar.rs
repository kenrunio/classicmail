//! Typed wrappers for the Google Calendar v3 REST API (the subset ClassicMail uses).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{Api, seg};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarListEntry {
    pub id: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default)]
    pub access_role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<bool>,
    #[serde(default)]
    pub default_reminders: Vec<Reminder>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventDateTime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "self")]
    pub is_self: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Attendee {
    #[serde(default)]
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organizer: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "self")]
    pub is_self: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optional: Option<bool>,
    /// needsAction | declined | tentative | accepted
    #[serde(default = "needs_action")]
    pub response_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

fn needs_action() -> String {
    "needsAction".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub method: String,
    pub minutes: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Reminders {
    #[serde(default)]
    pub use_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<Vec<Reminder>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtendedProperties {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html_link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<Person>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organizer: Option<Person>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<EventDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<EventDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurring_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_start_time: Option<EventDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "iCalUID")]
    pub ical_uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attendees: Option<Vec<Attendee>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminders: Option<Reminders>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extended_properties: Option<ExtendedProperties>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hangout_link: Option<String>,
    /// anything else Google sends that we don't model
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub async fn list_calendars(api: &Api) -> anyhow::Result<Vec<CalendarListEntry>> {
    let url = format!("{}/users/me/calendarList", api.urls.calendar);
    api.get_all_items(&url, &[("maxResults", "250".to_string())]).await
}

#[derive(Debug, Default, Clone)]
pub struct ListEvents {
    pub time_min: Option<String>,
    pub time_max: Option<String>,
    pub single_events: bool,
    pub ical_uid: Option<String>,
    pub show_deleted: bool,
    pub q: Option<String>,
    /// RFC 3339: only events changed since then (deleted ones included)
    pub updated_min: Option<String>,
}

/// One calendar's events (all pages).
pub async fn list_events(api: &Api, calendar_id: &str, p: &ListEvents) -> anyhow::Result<Vec<Event>> {
    let url = format!("{}/calendars/{}/events", api.urls.calendar, seg(calendar_id));
    let mut q = vec![("maxResults", "2500".to_string())];
    if let Some(v) = &p.time_min {
        q.push(("timeMin", v.clone()));
    }
    if let Some(v) = &p.time_max {
        q.push(("timeMax", v.clone()));
    }
    if p.single_events {
        q.push(("singleEvents", "true".into()));
        q.push(("orderBy", "startTime".into()));
    }
    if let Some(v) = &p.ical_uid {
        q.push(("iCalUID", v.clone()));
    }
    if p.show_deleted {
        q.push(("showDeleted", "true".into()));
    }
    if let Some(v) = &p.updated_min {
        q.push(("updatedMin", v.clone()));
    }
    if let Some(v) = &p.q {
        q.push(("q", v.clone()));
    }
    api.get_all_items(&url, &q).await
}

pub async fn get_event(api: &Api, calendar_id: &str, event_id: &str) -> anyhow::Result<Event> {
    let url = format!("{}/calendars/{}/events/{}", api.urls.calendar, seg(calendar_id), seg(event_id));
    api.get(&url, &[]).await
}

pub async fn instances(api: &Api, calendar_id: &str, event_id: &str, time_min: &str, time_max: &str) -> anyhow::Result<Vec<Event>> {
    let url = format!("{}/calendars/{}/events/{}/instances", api.urls.calendar, seg(calendar_id), seg(event_id));
    let q = [("maxResults", "2500".to_string()), ("timeMin", time_min.to_string()), ("timeMax", time_max.to_string())];
    api.get_all_items(&url, &q).await
}

/// `send_updates`: "all" | "externalOnly" | "none".
pub async fn insert_event(api: &Api, calendar_id: &str, body: Value, send_updates: &str) -> anyhow::Result<Event> {
    let url = format!("{}/calendars/{}/events", api.urls.calendar, seg(calendar_id));
    api.post(&url, &[("sendUpdates", send_updates.to_string())], body).await
}

pub async fn patch_event(api: &Api, calendar_id: &str, event_id: &str, body: Value, send_updates: &str) -> anyhow::Result<Event> {
    let url = format!("{}/calendars/{}/events/{}", api.urls.calendar, seg(calendar_id), seg(event_id));
    api.patch(&url, &[("sendUpdates", send_updates.to_string())], body).await
}

/// `events.move`: change an event's organizer calendar (same account; not for instances).
pub async fn move_event(api: &Api, calendar_id: &str, event_id: &str, destination: &str) -> anyhow::Result<Event> {
    let url = format!("{}/calendars/{}/events/{}/move", api.urls.calendar, seg(calendar_id), seg(event_id));
    api.post(&url, &[("destination", destination.to_string()), ("sendUpdates", "none".to_string())], Value::Object(Map::new())).await
}

/// Delete; an event that is already gone (404 / 410) counts as deleted.
pub async fn delete_event(api: &Api, calendar_id: &str, event_id: &str, send_updates: &str) -> anyhow::Result<()> {
    let url = format!("{}/calendars/{}/events/{}", api.urls.calendar, seg(calendar_id), seg(event_id));
    match api.delete(&url, &[("sendUpdates", send_updates.to_string())]).await {
        Ok(()) => Ok(()),
        Err(e) if matches!(super::client::api_status(&e), Some(404 | 410)) => Ok(()),
        Err(e) => Err(e),
    }
}

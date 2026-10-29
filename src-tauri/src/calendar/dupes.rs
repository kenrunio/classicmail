//! Calendar list grouping (My / Shared / Other Calendars) and duplicate detection:
//! * display de-duplication across calendars (the same invitation in both accounts, a shared
//!   calendar that repeats an event of mine): shown once, from the calendar listed first;
//! * "Clean Up Calendar": true duplicates inside one calendar (Gmail-generated reservations
//!   repeated, imports run twice) offered for deletion.

use std::collections::HashMap;

use serde::Serialize;

use super::model::{CalEvent, EventRow};

// ---- calendar groups -------------------------------------------------------------------------

/// "mine" | "shared" | "other" for a calendarList entry, the way Outlook groups calendars.
pub fn classify(gcal_id: &str, access_role: &str, is_primary: bool, account_email: &str) -> &'static str {
    let id = gcal_id.to_ascii_lowercase();
    if is_primary || id.eq_ignore_ascii_case(account_email) {
        return "mine";
    }
    // holidays, contacts' birthdays, weather, imported iCal feeds: subscriptions
    if id.contains("#holiday@")
        || id.contains("#contacts@")
        || id.starts_with("addressbook#")
        || id.contains("#weather@")
        || id.contains("#weeknum@")
        || id.ends_with("group.v.calendar.google.com")
        || id.ends_with("import.calendar.google.com")
    {
        return "other";
    }
    if id.ends_with("@group.calendar.google.com") || id.ends_with("@resource.calendar.google.com") {
        return match access_role {
            "owner" => "mine",
            "writer" => "shared",
            _ => "other",
        };
    }
    // someone else's address: a person's calendar shared with me (any access role)
    if id.contains('@') && !id.contains('#') {
        return "shared";
    }
    match access_role {
        "owner" => "mine",
        "writer" => "shared",
        _ => "other",
    }
}

// ---- display de-duplication ------------------------------------------------------------------

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Collapse the same event shown from several calendars. `events` must be in calendar display
/// order (first listed calendar first); `ical` gives each event's iCalUID. The kept item gets
/// `also_in` = labels of the other calendars it was found in.
pub fn merge_display(events: Vec<CalEvent>, ical: &HashMap<i64, String>, cal_label: &dyn Fn(i64) -> String) -> Vec<CalEvent> {
    let mut out: Vec<CalEvent> = Vec::with_capacity(events.len());
    let mut by_uid: HashMap<(String, i64), usize> = HashMap::new();
    let mut by_text: HashMap<(String, i64, i64, bool), usize> = HashMap::new();
    for ev in events {
        let k1 = ical.get(&ev.id).filter(|u| !u.is_empty()).map(|u| (u.clone(), ev.start));
        let k2 = (!ev.summary.trim().is_empty()).then(|| (norm(&ev.summary), ev.start, ev.end, ev.all_day));
        let hit = k1.as_ref().and_then(|k| by_uid.get(k)).or_else(|| k2.as_ref().and_then(|k| by_text.get(k))).copied();
        // only across calendars: duplicates inside one calendar stay visible (Clean Up removes them)
        if let Some(i) = hit.filter(|&i| out[i].calendar_id != ev.calendar_id) {
            let label = cal_label(ev.calendar_id);
            let kept = &mut out[i];
            let list = kept.also_in.get_or_insert_with(Vec::new);
            if !list.contains(&label) {
                list.push(label);
            }
            continue;
        }
        let i = out.len();
        if let Some(k) = k1 {
            by_uid.entry(k).or_insert(i);
        }
        if let Some(k) = k2 {
            by_text.entry(k).or_insert(i);
        }
        out.push(ev);
    }
    out
}

// ---- Clean Up Calendar -----------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupItem {
    pub id: i64,
    pub calendar_id: i64,
    pub summary: String,
    pub location: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub is_series: bool,
    pub has_attendees: bool,
    pub has_notes: bool,
    pub organizer_self: bool,
    pub organizer: Option<String>,
    pub created: Option<i64>,
    /// pre-ticked for deletion in the dialog
    pub suggest_delete: bool,
    /// deleting it removes a meeting someone else organised (warn first)
    pub warn: bool,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupGroup {
    pub calendar_id: i64,
    pub summary: String,
    pub start: i64,
    pub end: i64,
    pub items: Vec<DupItem>,
}

/// Key of a "true duplicate" inside one calendar: same subject, start, end, all-day and location
/// (whitespace / case-insensitive). Series compare their rule too; occurrences are never offered.
pub fn dup_key(e: &EventRow) -> Option<String> {
    if e.recurring_event_id.is_some() || e.parent_local_id.is_some() || e.status == "cancelled" || e.gcal_id.is_none() {
        return None;
    }
    if e.summary.trim().is_empty() {
        return None;
    }
    let rec = if e.is_master { e.recurrence.clone().unwrap_or_default().join("|") } else { String::new() };
    Some(format!(
        "{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
        e.calendar_id,
        norm(&e.summary),
        e.start_ms,
        e.end_ms,
        e.all_day,
        norm(&e.location),
        rec
    ))
}

/// Which one of a group to keep: one with attendees, then with notes, then the oldest.
fn keep_rank(e: &EventRow) -> (i64, i64, i64, i64) {
    (
        -(e.attendees.iter().any(|a| a.is_self != Some(true)) as i64),
        -(!e.description.trim().is_empty() as i64),
        e.created_ms.unwrap_or(i64::MAX),
        e.id,
    )
}

pub fn find_groups(rows: &[EventRow], read_only_cals: &[i64]) -> Vec<DupGroup> {
    let mut map: HashMap<String, Vec<&EventRow>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for r in rows {
        if let Some(k) = dup_key(r) {
            let v = map.entry(k.clone()).or_default();
            if v.is_empty() {
                order.push(k);
            }
            v.push(r);
        }
    }
    let mut out = Vec::new();
    for k in order {
        let mut v = map.remove(&k).unwrap_or_default();
        if v.len() < 2 {
            continue;
        }
        v.sort_by_key(|e| keep_rank(e));
        let first = v[0];
        let items = v
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let others_meeting = !e.organizer_self && e.attendees.iter().any(|a| a.is_self != Some(true));
                let ro = read_only_cals.contains(&e.calendar_id) || e.event_type == "birthday";
                DupItem {
                    id: e.id,
                    calendar_id: e.calendar_id,
                    summary: e.summary.clone(),
                    location: e.location.clone(),
                    start: e.start_ms,
                    end: e.end_ms,
                    all_day: e.all_day,
                    is_series: e.is_master,
                    has_attendees: e.attendees.iter().any(|a| a.is_self != Some(true)),
                    has_notes: !e.description.trim().is_empty(),
                    organizer_self: e.organizer_self,
                    organizer: e.organizer_name.clone().or_else(|| e.organizer_email.clone()),
                    created: e.created_ms,
                    // never pre-tick the keeper, meetings organised by others, or what we can't delete
                    suggest_delete: i > 0 && !others_meeting && !ro,
                    warn: others_meeting,
                    read_only: ro,
                }
            })
            .collect();
        out.push(DupGroup {
            calendar_id: first.calendar_id,
            summary: first.summary.clone(),
            start: first.start_ms,
            end: first.end_ms,
            items,
        });
    }
    out.sort_by_key(|g| g.start);
    out
}

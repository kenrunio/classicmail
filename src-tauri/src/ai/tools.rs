//! What the AI may look up while writing (AI Review's `{…}` instructions), all read-only and all
//! answered from this computer — only the answer goes to the model, never the whole calendar:
//!
//! * `find_free_time` — free slots in the user's work hours, with the gap kept around meetings
//!   (Tools › Options › Calendar), optionally also within the recipient's hours in their zone;
//! * `whats_on` — events in a period; `find_meeting` — the next / last matching meeting;
//!   `meeting_details` — one event in full (dial-in / Zoom link, address, attendees);
//! * `convert_time` — exact time-zone conversion (daylight saving included);
//! * `find_tasks` — To-Do / tasks and their due dates.
//!
//! Busy = the user's primary calendar (the first mailbox's), unless a tool is asked for another
//! calendar by name; tentative and unanswered invitations count as busy, declined ones don't.

use std::{collections::HashSet, sync::Arc};

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use chrono_tz::Tz;
use rusqlite::{Connection, params_from_iter};
use serde_json::{Value, json};

use super::tooluse::ToolSpec;
use crate::{calendar::model::EventRow, ctx::AppCtx, db::util::get_setting};

pub const GAP_SETTING: &str = "calendar.meetingGapMinutes";
pub const LENGTH_SETTING: &str = "calendar.meetingMinutes";
const DEFAULT_GAP: i64 = 15;
const DEFAULT_LENGTH: i64 = 45;

pub fn specs() -> Vec<ToolSpec> {
    let date = |d: &str| json!({ "type": "string", "description": format!("{d} (YYYY-MM-DD, in the user's time zone)") });
    let calendar = json!({ "type": "string", "description": "Only when the user names someone else's / another calendar (e.g. \"Shira\"); otherwise leave out: the user's own primary calendar. \"all\" = every calendar of the user." });
    vec![
        ToolSpec {
            name: "find_free_time",
            description: "Free time slots in the user's calendar within their work hours, keeping the usual gap around existing meetings. Use for any request to suggest or offer meeting times.",
            parameters: json!({ "type": "object", "properties": {
                "from": date("First day"),
                "to": date("Last day (inclusive)"),
                "minutes": { "type": "integer", "description": "Meeting length; leave out for the user's usual length" },
                "part_of_day": { "type": "string", "enum": ["any", "morning", "afternoon", "evening"] },
                "earliest": { "type": "string", "description": "Not before this time of day, HH:MM (user's zone)" },
                "latest": { "type": "string", "description": "Ending by this time of day, HH:MM (user's zone)" },
                "their_time_zone": { "type": "string", "description": "IANA zone of the other side (e.g. America/New_York) when they are abroad: slots then also fit their work hours and show their time" },
                "their_hours": { "type": "string", "description": "Their acceptable hours, HH:MM-HH:MM in their zone (default 09:00-18:00)" },
                "calendar": calendar,
                "max": { "type": "integer", "description": "How many slots (default 4)" }
            }, "required": ["from", "to"] }),
        },
        ToolSpec {
            name: "whats_on",
            description: "The user's events in a period (time, title, place, attendees, the user's response). Use to check whether the user is free at a given time, or what they have on a day.",
            parameters: json!({ "type": "object", "properties": {
                "from": date("First day"), "to": date("Last day (inclusive; default = from)"),
                "query": { "type": "string", "description": "Only events whose title / place / attendees / notes contain these words" },
                "calendar": calendar
            }, "required": ["from"] }),
        },
        ToolSpec {
            name: "find_meeting",
            description: "Find meetings by company, person, e-mail domain or words (e.g. \"Acme\", \"Dan Levi\", \"results call\"): the next ones, the most recent ones, or all in a period. Searches all of the user's calendars.",
            parameters: json!({ "type": "object", "properties": {
                "query": { "type": "string" },
                "which": { "type": "string", "enum": ["next", "last", "all"], "description": "next (default) = upcoming; last = most recent past; all = in from–to" },
                "from": date("Start of the period for which=all"), "to": date("End of the period for which=all")
            }, "required": ["query"] }),
        },
        ToolSpec {
            name: "meeting_details",
            description: "One event in full by its id (from the other tools): notes, dial-in / Zoom / Teams links, address, organizer, attendees and their responses.",
            parameters: json!({ "type": "object", "properties": { "id": { "type": "integer" } }, "required": ["id"] }),
        },
        ToolSpec {
            name: "convert_time",
            description: "Convert a date and time between time zones exactly (daylight saving included). Use instead of converting yourself.",
            parameters: json!({ "type": "object", "properties": {
                "time": { "type": "string", "description": "YYYY-MM-DD HH:MM" },
                "from_zone": { "type": "string", "description": "IANA zone of `time` (default: the user's)" },
                "to_zones": { "type": "array", "items": { "type": "string" }, "description": "IANA zones, e.g. [\"America/New_York\", \"Europe/London\"]" }
            }, "required": ["time", "to_zones"] }),
        },
        ToolSpec {
            name: "find_tasks",
            description: "The user's tasks (To-Do) with due dates, by words and / or due-date range.",
            parameters: json!({ "type": "object", "properties": {
                "query": { "type": "string" },
                "due_from": date("Due on or after"), "due_to": date("Due on or before"),
                "include_completed": { "type": "boolean" }
            } }),
        },
    ]
}

/// Run a tool; a problem comes back as `{"error": …}` for the model to deal with.
pub async fn run(ctx: &Arc<AppCtx>, name: &str, args: &Value) -> Value {
    let (name, args) = (name.to_string(), args.clone());
    let now = crate::clock::now_ms();
    let r = ctx
        .db
        .read(move |c| {
            let env = Env::load(c, now)?;
            Ok(match name.as_str() {
                "find_free_time" => find_free_time(c, &env, &args),
                "whats_on" => whats_on(c, &env, &args),
                "find_meeting" => find_meeting(c, &env, &args),
                "meeting_details" => meeting_details(c, &env, &args),
                "convert_time" => convert_time(&env, &args),
                "find_tasks" => find_tasks(c, &args),
                other => Err(anyhow::anyhow!("no tool called {other}")),
            })
        })
        .await;
    match r {
        Ok(Ok(v)) => v,
        Ok(Err(e)) | Err(e) => json!({ "error": e.to_string() }),
    }
}

/// The facts every tool needs: the user's zone, work week, usual meeting length and gap.
pub struct Env {
    pub tz: Tz,
    pub now_ms: i64,
    /// 0 = Sunday
    work_days: Vec<u32>,
    work_start: i64,
    work_end: i64,
    pub gap: i64,
    pub length: i64,
}

impl Env {
    pub fn load(c: &Connection, now_ms: i64) -> anyhow::Result<Env> {
        Ok(Env {
            tz: user_tz(c),
            now_ms,
            work_days: get_setting(c, "calendar.workDays")?.unwrap_or_else(|| vec![0, 1, 2, 3, 4]),
            work_start: get_setting(c, "calendar.workDayStart")?.unwrap_or(480),
            work_end: get_setting(c, "calendar.workDayEnd")?.unwrap_or(1080),
            gap: get_setting(c, GAP_SETTING)?.unwrap_or(DEFAULT_GAP).clamp(0, 240),
            length: get_setting(c, LENGTH_SETTING)?.unwrap_or(DEFAULT_LENGTH).clamp(5, 600),
        })
    }
    pub fn today(&self) -> NaiveDate {
        self.tz.timestamp_millis_opt(self.now_ms).single().map(|d| d.date_naive()).unwrap_or_default()
    }
    fn at(&self, d: NaiveDate, minutes: i64) -> i64 {
        local_ms(&self.tz, d, minutes)
    }
}

fn local_ms(tz: &Tz, d: NaiveDate, minutes: i64) -> i64 {
    let dt = d.and_time(NaiveTime::MIN) + Duration::minutes(minutes);
    tz.from_local_datetime(&dt)
        .earliest()
        .or_else(|| tz.from_local_datetime(&(dt + Duration::hours(1))).earliest())
        .map(|t| t.timestamp_millis())
        .unwrap_or(0)
}

/// The user's time zone: their primary calendar's, else this computer's.
pub fn user_tz(c: &Connection) -> Tz {
    let cal: Option<String> = c
        .query_row(
            "SELECT c.time_zone FROM calendars c JOIN accounts a ON a.id=c.account_id WHERE c.is_primary=1 AND c.time_zone!='UTC' ORDER BY a.position, a.id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok();
    cal.or_else(|| iana_time_zone::get_timezone().ok()).and_then(|z| z.parse().ok()).unwrap_or(chrono_tz::UTC)
}

// ---- formatting ---------------------------------------------------------------------------

fn fmt_day(tz: &Tz, ms: i64) -> String {
    tz.timestamp_millis_opt(ms).single().map(|t| t.format("%a %-d %b %Y").to_string()).unwrap_or_default()
}
fn fmt_time(tz: &Tz, ms: i64) -> String {
    tz.timestamp_millis_opt(ms).single().map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}
fn fmt_range(tz: &Tz, s: i64, e: i64) -> String {
    format!("{}, {}–{}", fmt_day(tz, s), fmt_time(tz, s), fmt_time(tz, e))
}
fn fmt_event_when(tz: &Tz, ev: &EventRow) -> String {
    if ev.all_day {
        let days = (ev.end_ms - ev.start_ms) / 86_400_000;
        if days > 1 {
            format!("{} – {} (all day)", fmt_day(tz, ev.start_ms), fmt_day(tz, ev.end_ms - 1))
        } else {
            format!("{} (all day)", fmt_day(tz, ev.start_ms))
        }
    } else {
        fmt_range(tz, ev.start_ms, ev.end_ms)
    }
}

fn arg_date(args: &Value, key: &str) -> anyhow::Result<Option<NaiveDate>> {
    match args[key].as_str() {
        None | Some("") => Ok(None),
        Some(s) => {
            NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map(Some).map_err(|_| anyhow::anyhow!("{key} must be YYYY-MM-DD, not {s}"))
        }
    }
}
fn arg_hhmm(s: &str) -> Option<i64> {
    let t = NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()?;
    Some(t.hour() as i64 * 60 + t.minute() as i64)
}

// ---- calendars and events -------------------------------------------------------------------

struct Cal {
    id: i64,
    summary: String,
    email: String,
    primary: bool,
    owned: bool,
    holidays: bool,
}

fn calendars(c: &Connection) -> anyhow::Result<Vec<Cal>> {
    let mut st = c.prepare(
        "SELECT c.id, c.summary, a.email, c.is_primary, c.access_role, c.gcal_id FROM calendars c JOIN accounts a ON a.id=c.account_id \
         WHERE a.enabled=1 ORDER BY a.position, a.id, c.is_primary DESC, c.position",
    )?;
    let v = st
        .query_map([], |r| {
            let role: String = r.get(4)?;
            let gid: String = r.get(5)?;
            Ok(Cal {
                id: r.get(0)?,
                summary: r.get(1)?,
                email: r.get(2)?,
                primary: r.get::<_, i64>(3)? != 0,
                owned: matches!(role.as_str(), "owner" | "writer"),
                holidays: gid.contains("#holiday@"),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

/// The calendars a tool looks at: the named one(s), "all" of the user's, or the primary one.
fn pick(cals: &[Cal], name: Option<&str>, default_all: bool) -> anyhow::Result<(Vec<i64>, String)> {
    let name = name.map(str::trim).filter(|s| !s.is_empty());
    match name {
        Some(n) if n.eq_ignore_ascii_case("all") => {
            Ok((cals.iter().filter(|c| c.owned).map(|c| c.id).collect(), "all your calendars".into()))
        }
        Some(n) => {
            let l = n.to_lowercase();
            let hits: Vec<&Cal> =
                cals.iter().filter(|c| c.summary.to_lowercase().contains(&l) || c.email.to_lowercase().contains(&l)).collect();
            if hits.is_empty() {
                let names: Vec<&str> = cals.iter().map(|c| c.summary.as_str()).collect();
                anyhow::bail!("no calendar matches \"{n}\"; the calendars are: {}", names.join(", "));
            }
            Ok((hits.iter().map(|c| c.id).collect(), hits.iter().map(|c| c.summary.as_str()).collect::<Vec<_>>().join(", ")))
        }
        None if default_all => Ok((cals.iter().filter(|c| c.owned).map(|c| c.id).collect(), "all your calendars".into())),
        None => {
            let p = cals.iter().find(|c| c.primary).ok_or_else(|| anyhow::anyhow!("no calendar yet"))?;
            Ok((vec![p.id], format!("{} (primary)", p.summary)))
        }
    }
}

/// Events of these calendars overlapping [from, to), declined and cancelled ones left out.
fn events(c: &Connection, cal_ids: &[i64], from: i64, to: i64) -> anyhow::Result<Vec<EventRow>> {
    if cal_ids.is_empty() {
        return Ok(vec![]);
    }
    let ids = cal_ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT * FROM events WHERE calendar_id IN ({ids}) AND is_master=0 AND status!='cancelled' \
         AND (my_response IS NULL OR my_response!='declined') AND end_ms > ?1 AND start_ms < ?2 ORDER BY start_ms, id"
    );
    let mut st = c.prepare(&sql)?;
    let v = st.query_map([from, to], EventRow::from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

/// Busy time: timed events not shown as free (tentative / unanswered invitations included).
fn is_busy(e: &EventRow) -> bool {
    !e.all_day && e.transparency != "transparent" && e.show_as.as_deref() != Some("free")
}
/// A whole day off: an all-day event shown as busy, or out of office.
fn blocks_day(e: &EventRow) -> bool {
    e.event_type == "outOfOffice"
        || e.show_as.as_deref() == Some("oof")
        || (e.all_day && e.transparency == "opaque" && e.show_as.as_deref().is_none_or(|s| s == "busy"))
}

fn people(e: &EventRow) -> Vec<String> {
    e.attendees
        .iter()
        .filter(|a| a.is_self != Some(true) && a.resource != Some(true))
        .take(12)
        .map(|a| match &a.display_name {
            Some(n) if !n.is_empty() => format!("{n} <{}>", a.email),
            _ => a.email.clone(),
        })
        .collect()
}

fn brief(tz: &Tz, e: &EventRow, cal: &str) -> Value {
    json!({
        "id": e.id, "when": fmt_event_when(tz, e), "title": e.summary,
        "location": if e.location.is_empty() { Value::Null } else { json!(e.location) },
        "attendees": people(e), "my_response": e.my_response, "calendar": cal,
        "shown_as": if is_busy(e) || e.all_day { e.show_as.clone().unwrap_or_else(|| "busy".into()) } else { "free".into() },
    })
}

// ---- the tools ------------------------------------------------------------------------------

fn find_free_time(c: &Connection, env: &Env, args: &Value) -> anyhow::Result<Value> {
    let tz = env.tz;
    let today = env.today();
    let from = arg_date(args, "from")?.unwrap_or(today).max(today);
    let to = arg_date(args, "to")?.unwrap_or(from + Duration::days(7)).min(from + Duration::days(62));
    let len = args["minutes"].as_i64().unwrap_or(env.length).clamp(5, 600);
    let max = args["max"].as_i64().unwrap_or(4).clamp(1, 12) as usize;
    let cals = calendars(c)?;
    let (ids, cal_name) = pick(&cals, args["calendar"].as_str(), false)?;

    // the part of the day asked for, in the user's minutes after midnight
    let (mut lo, mut hi) = (env.work_start, env.work_end);
    match args["part_of_day"].as_str().unwrap_or("any") {
        "morning" => hi = hi.min(12 * 60),
        "afternoon" => {
            lo = lo.max(12 * 60);
            hi = hi.min(18 * 60);
        }
        "evening" => {
            lo = lo.max(17 * 60);
            hi = hi.clamp(21 * 60, 22 * 60); // evenings run past the work day, to 21:00–22:00
        }
        _ => {}
    }
    if let Some(m) = args["earliest"].as_str().and_then(arg_hhmm) {
        lo = lo.max(m);
    }
    if let Some(m) = args["latest"].as_str().and_then(arg_hhmm) {
        hi = hi.min(m);
    }
    let theirs: Option<(Tz, i64, i64)> = match args["their_time_zone"].as_str().filter(|s| !s.is_empty()) {
        Some(z) => {
            let tz2: Tz = z.parse().map_err(|_| anyhow::anyhow!("unknown time zone {z} (use an IANA name like America/New_York)"))?;
            let (a, b) = args["their_hours"]
                .as_str()
                .and_then(|h| h.split_once('-'))
                .and_then(|(a, b)| Some((arg_hhmm(a)?, arg_hhmm(b)?)))
                .unwrap_or((9 * 60, 18 * 60));
            Some((tz2, a, b))
        }
        None => None,
    };

    let start_all = env.at(from, 0);
    let end_all = env.at(to + Duration::days(1), 0);
    let evs = events(c, &ids, start_all, end_all)?;
    let holiday_ids: Vec<i64> = cals.iter().filter(|c| c.holidays).map(|c| c.id).collect();
    let hols = events(c, &holiday_ids, start_all, end_all)?;
    let gap = env.gap * 60_000;
    let busy: Vec<(i64, i64)> = evs.iter().filter(|e| is_busy(e)).map(|e| (e.start_ms - gap, e.end_ms + gap)).collect();
    let earliest_ok = {
        let q = 15 * 60_000;
        ((env.now_ms + 60 * 60_000) + q - 1) / q * q
    };

    let mut slots = Vec::new();
    let mut holidays = Vec::new();
    let mut d = from;
    while d <= to && slots.len() < max {
        let dow = d.weekday().num_days_from_sunday();
        let day_s = env.at(d, 0);
        let day_e = env.at(d + Duration::days(1), 0);
        let hol: Vec<String> = hols.iter().filter(|h| h.start_ms < day_e && h.end_ms > day_s).map(|h| h.summary.clone()).collect();
        if !hol.is_empty() {
            holidays.push(json!({ "day": fmt_day(&tz, day_s), "holiday": hol.join(", ") }));
        }
        let off = evs.iter().any(|e| blocks_day(e) && e.start_ms < day_e && e.end_ms > day_s);
        if !env.work_days.contains(&dow) || off {
            d += Duration::days(1);
            continue;
        }
        let mut w0 = env.at(d, lo).max(earliest_ok);
        let mut w1 = env.at(d, hi);
        if let Some((tz2, a, b)) = theirs {
            w0 = w0.max(local_ms(&tz2, d, a));
            w1 = w1.min(local_ms(&tz2, d, b));
        }
        let mut free: Vec<(i64, i64)> = vec![];
        let mut cur = w0;
        let mut day_busy: Vec<&(i64, i64)> = busy.iter().filter(|b| b.0 < w1 && b.1 > w0).collect();
        day_busy.sort();
        for b in day_busy {
            if b.0 > cur {
                free.push((cur, b.0.min(w1)));
            }
            cur = cur.max(b.1);
        }
        if cur < w1 {
            free.push((cur, w1));
        }
        let mut today_count = 0;
        for (s, e) in free {
            let q = 15 * 60_000;
            let s = (s + q - 1) / q * q;
            if s + len * 60_000 <= e && today_count < 2 && slots.len() < max {
                let end = s + len * 60_000;
                let mut slot = json!({ "when": fmt_range(&tz, s, end), "start": tz.timestamp_millis_opt(s).single().map(|t| t.format("%Y-%m-%dT%H:%M").to_string()) });
                if let Some((tz2, _, _)) = theirs {
                    slot["their_time"] = json!(format!("{} ({})", fmt_range(&tz2, s, end), tz2.name()));
                }
                if !hol.is_empty() {
                    slot["holiday"] = json!(hol.join(", "));
                }
                slots.push(slot);
                today_count += 1;
            }
        }
        d += Duration::days(1);
    }
    let mut out = json!({
        "time_zone": tz.name(), "calendar": cal_name, "meeting_minutes": len, "gap_minutes": env.gap,
        "work_hours": format!("{:02}:{:02}-{:02}:{:02}", env.work_start / 60, env.work_start % 60, env.work_end / 60, env.work_end % 60),
        "slots": slots,
    });
    if !holidays.is_empty() {
        out["holidays_in_period"] = json!(holidays);
    }
    if out["slots"].as_array().is_some_and(|s| s.is_empty()) {
        out["note"] = json!("No free time of that length in that period.");
    }
    Ok(out)
}

fn whats_on(c: &Connection, env: &Env, args: &Value) -> anyhow::Result<Value> {
    let from = arg_date(args, "from")?.unwrap_or(env.today());
    let to = arg_date(args, "to")?.unwrap_or(from).min(from + Duration::days(62));
    let cals = calendars(c)?;
    let (ids, cal_name) = pick(&cals, args["calendar"].as_str(), false)?;
    let words: Vec<String> = args["query"].as_str().unwrap_or("").split_whitespace().map(str::to_lowercase).collect();
    let evs = events(c, &ids, env.at(from, 0), env.at(to + Duration::days(1), 0))?;
    let name = |id: i64| cals.iter().find(|c| c.id == id).map(|c| c.summary.clone()).unwrap_or_default();
    let list: Vec<Value> = evs.iter().filter(|e| matches(e, &words)).take(60).map(|e| brief(&env.tz, e, &name(e.calendar_id))).collect();
    Ok(json!({ "time_zone": env.tz.name(), "calendar": cal_name, "events": list }))
}

fn matches(e: &EventRow, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {} {} {}",
        e.summary,
        e.location,
        e.description,
        e.organizer_email.as_deref().unwrap_or(""),
        e.attendees.iter().map(|a| format!("{} {}", a.display_name.as_deref().unwrap_or(""), a.email)).collect::<Vec<_>>().join(" ")
    )
    .to_lowercase();
    words.iter().all(|w| hay.contains(w.as_str()))
}

fn find_meeting(c: &Connection, env: &Env, args: &Value) -> anyhow::Result<Value> {
    let query = args["query"].as_str().unwrap_or("").trim().to_string();
    if query.is_empty() {
        anyhow::bail!("query is required");
    }
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let cals = calendars(c)?;
    let (ids, _) = pick(&cals, args["calendar"].as_str(), true)?;
    let which = args["which"].as_str().unwrap_or("next");
    let year = 365 * 86_400_000_i64;
    let (from, to) = match which {
        "last" => (env.now_ms - 2 * year, env.now_ms),
        "all" => {
            let f = arg_date(args, "from")?.unwrap_or(env.today() - Duration::days(30));
            let t = arg_date(args, "to")?.unwrap_or(f + Duration::days(60));
            (env.at(f, 0), env.at(t + Duration::days(1), 0))
        }
        _ => (env.now_ms, env.now_ms + year),
    };
    let mut evs: Vec<EventRow> = events(c, &ids, from, to)?.into_iter().filter(|e| matches(e, &words)).collect();
    if which == "last" {
        evs.retain(|e| e.end_ms <= env.now_ms);
        evs.reverse();
    }
    // the same meeting on two calendars once
    let mut seen = HashSet::new();
    evs.retain(|e| seen.insert((e.summary.to_lowercase(), e.start_ms)));
    let name = |id: i64| cals.iter().find(|c| c.id == id).map(|c| c.summary.clone()).unwrap_or_default();
    let list: Vec<Value> = evs.iter().take(if which == "all" { 30 } else { 5 }).map(|e| brief(&env.tz, e, &name(e.calendar_id))).collect();
    Ok(json!({ "time_zone": env.tz.name(), "meetings": list, "today": fmt_day(&env.tz, env.now_ms) }))
}

fn meeting_details(c: &Connection, env: &Env, args: &Value) -> anyhow::Result<Value> {
    let id = args["id"].as_i64().ok_or_else(|| anyhow::anyhow!("id is required"))?;
    let e = c.query_row("SELECT * FROM events WHERE id=?1", [id], EventRow::from_row).map_err(|_| anyhow::anyhow!("no event {id}"))?;
    static LINK: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r#"https?://[^\s<>"')\]]+"#).expect("link pattern"));
    let links: Vec<String> = LINK
        .find_iter(&format!("{} {} {}", e.location, e.conference, e.description))
        .map(|m| m.as_str().trim_end_matches(['.', ',', ';']).to_string())
        .filter(|u| ["zoom.", "teams.", "meet.google", "webex", "gotomeeting", "whereby", "chime"].iter().any(|k| u.contains(k)))
        .collect::<Vec<_>>();
    let notes: String = e.description.chars().take(3000).collect();
    let attendees: Vec<Value> = e
        .attendees
        .iter()
        .filter(|a| a.resource != Some(true))
        .map(|a| json!({ "name": a.display_name, "email": a.email, "response": a.response_status, "organizer": a.organizer == Some(true), "you": a.is_self == Some(true) }))
        .collect();
    Ok(json!({
        "id": e.id, "title": e.summary, "when": fmt_event_when(&env.tz, &e), "time_zone": env.tz.name(),
        "location": e.location, "meeting_links": links, "video_call": e.conference, "notes": notes,
        "organizer": e.organizer_name.clone().or(e.organizer_email.clone()), "attendees": attendees, "my_response": e.my_response,
    }))
}

fn convert_time(env: &Env, args: &Value) -> anyhow::Result<Value> {
    let t = args["time"].as_str().unwrap_or("").trim();
    let dt = NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M"))
        .map_err(|_| anyhow::anyhow!("time must be YYYY-MM-DD HH:MM"))?;
    let from: Tz = match args["from_zone"].as_str().filter(|s| !s.is_empty()) {
        Some(z) => z.parse().map_err(|_| anyhow::anyhow!("unknown time zone {z}"))?,
        None => env.tz,
    };
    let at =
        from.from_local_datetime(&dt).earliest().ok_or_else(|| anyhow::anyhow!("{t} doesn't exist in {} (clock change)", from.name()))?;
    let mut out = vec![];
    for z in args["to_zones"].as_array().into_iter().flatten().filter_map(|z| z.as_str()) {
        match z.parse::<Tz>() {
            Ok(tz) => {
                let x = at.with_timezone(&tz);
                out.push(json!({ "zone": tz.name(), "time": x.format("%a %-d %b %Y, %H:%M").to_string(), "utc_offset": x.format("%:z").to_string() }));
            }
            Err(_) => out.push(json!({ "zone": z, "error": "unknown time zone (use an IANA name like America/New_York)" })),
        }
    }
    Ok(json!({ "from": format!("{} ({})", at.format("%a %-d %b %Y, %H:%M"), from.name()), "converted": out }))
}

fn find_tasks(c: &Connection, args: &Value) -> anyhow::Result<Value> {
    let mut sql = String::from(
        "SELECT t.title, t.due, t.status, t.outlook_status, t.importance, t.notes, l.title FROM tasks t JOIN task_lists l ON l.id=t.list_id \
         JOIN accounts a ON a.id=t.account_id WHERE a.enabled=1 AND t.hidden=0",
    );
    let mut ps: Vec<String> = vec![];
    if !args["include_completed"].as_bool().unwrap_or(false) {
        sql.push_str(" AND t.status!='completed'");
    }
    if let Some(d) = arg_date(args, "due_from")? {
        ps.push(d.to_string());
        sql.push_str(&format!(" AND t.due >= ?{}", ps.len()));
    }
    if let Some(d) = arg_date(args, "due_to")? {
        ps.push(d.to_string());
        sql.push_str(&format!(" AND t.due <= ?{}", ps.len()));
    }
    for w in args["query"].as_str().unwrap_or("").split_whitespace() {
        ps.push(format!("%{}%", w.replace(['%', '_'], "")));
        sql.push_str(&format!(" AND (t.title LIKE ?{n} OR t.notes LIKE ?{n})", n = ps.len()));
    }
    sql.push_str(" ORDER BY t.due IS NULL, t.due, t.title LIMIT 25");
    let mut st = c.prepare(&sql)?;
    let rows = st
        .query_map(params_from_iter(ps.iter()), |r| {
            let notes: String = r.get(5)?;
            Ok(json!({
                "title": r.get::<_, String>(0)?, "due": r.get::<_, Option<String>>(1)?,
                "status": if r.get::<_, String>(2)? == "completed" { "completed".to_string() } else { r.get::<_, String>(3)? },
                "importance": match r.get::<_, i64>(4)? { 2 => "high", 0 => "low", _ => "normal" },
                "notes": notes.chars().take(300).collect::<String>(), "list": r.get::<_, String>(6)?,
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "tasks": rows }))
}

#[cfg(test)]
mod tests;

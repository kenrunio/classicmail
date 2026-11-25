//! The agreed time as the AI read it (wall time in the sender's zone) → a moment.

use chrono::{Datelike, NaiveDate, TimeZone};
use chrono_tz::Tz;
use serde::Deserialize;

use crate::ai::tools::{fmt_time, local_ms};

/// The time as the AI read it from the email.
#[derive(Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct AiTime {
    /// YYYY-MM-DD
    pub date: Option<String>,
    /// false when the email gave no year (the AI guessed one)
    pub year_stated: bool,
    /// HH:MM
    pub start: Option<String>,
    pub end: Option<String>,
    /// IANA zone the time was written in
    pub time_zone: Option<String>,
}

pub(crate) struct Slot {
    pub start: i64,
    pub end: i64,
    pub tz: Tz,
    pub zone_stated: bool,
}

fn minutes(hhmm: &str) -> Option<i64> {
    let (h, m) = hhmm.trim().split_once(':')?;
    let (h, m): (i64, i64) = (h.trim().parse().ok()?, m.trim().get(..2).unwrap_or(m.trim()).parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some(h * 60 + m)
}

/// The slot, or None when there's no usable date and start. An unknown zone → `user_tz`; a
/// year-less date already past at the email's date → next year; no end (or none after the start)
/// → start + `length_min`.
pub(crate) fn resolve(t: &AiTime, email_ms: i64, user_tz: Tz, length_min: i64) -> Option<Slot> {
    let mut date = NaiveDate::parse_from_str(t.date.as_deref()?.trim(), "%Y-%m-%d").ok()?;
    let start_min = minutes(t.start.as_deref()?)?;
    let parsed: Option<Tz> = t.time_zone.as_deref().map(str::trim).filter(|z| !z.is_empty()).and_then(|z| z.parse().ok());
    let (tz, zone_stated) = match parsed {
        Some(z) => (z, true),
        None => (user_tz, false),
    };
    if !t.year_stated {
        let sent = tz.timestamp_millis_opt(email_ms).single().map(|d| d.date_naive());
        if let Some(sent) = sent
            && date < sent
        {
            date = date.with_year(date.year() + 1).unwrap_or(date);
        }
    }
    let start = local_ms(&tz, date, start_min);
    let end = t.end.as_deref().and_then(minutes).map(|m| local_ms(&tz, date, m)).filter(|e| *e > start).unwrap_or(start + length_min * 60_000);
    Some(Slot { start, end, tz, zone_stated })
}

/// "New York" for America/New_York.
fn zone_name(tz: &Tz) -> String {
    tz.name().rsplit('/').next().unwrap_or(tz.name()).replace('_', " ")
}

/// What the info line says about the zone the time was read in ("" when it's the user's own).
pub(crate) fn zone_note(s: &Slot, user_tz: Tz) -> String {
    if !s.zone_stated {
        return format!("Time zone not stated in the email — read as your time ({}).", zone_name(&user_tz));
    }
    if s.tz == user_tz {
        return String::new();
    }
    format!("Time read as {} {} ({} your time).", fmt_time(&s.tz, s.start), zone_name(&s.tz), fmt_time(&user_tz, s.start))
}

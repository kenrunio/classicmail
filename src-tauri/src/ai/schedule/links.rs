//! Call links: which meetings use the same video room, and whether the main or backup link is free.

use chrono_tz::Tz;

use crate::{
    ai::tools::fmt_time,
    calendar::{
        conference::{LINK, notes_text},
        model::EventRow,
    },
};

/// The room a call link opens: a Zoom meeting by its number (`zoom:81234567890`, whatever the
/// subdomain or passcode), anything else by host and path (no `www.`, no trailing `/`, no query).
pub(crate) fn link_key(url: &str) -> Option<String> {
    let u = url::Url::parse(url.trim()).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let host = u.host_str()?.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let path = u.path().trim_end_matches('/');
    if (host == "zoom.us" || host.ends_with(".zoom.us"))
        && let Some(n) = path.strip_prefix("/j/")
        && !n.is_empty()
        && n.chars().all(|c| c.is_ascii_digit())
    {
        return Some(format!("zoom:{n}"));
    }
    Some(format!("{host}{path}"))
}

/// Whether the meeting's place, notes or video-call details hold a link to that room.
pub(crate) fn uses_link(ev: &EventRow, key: &str) -> bool {
    let notes = notes_text(&ev.description);
    [ev.location.as_str(), notes.as_str(), ev.conference.as_str()]
        .iter()
        .any(|t| LINK.find_iter(t).any(|m| link_key(m.as_str().trim_end_matches(['.', ',', ';', ')', '>'])).as_deref() == Some(key)))
}

pub(crate) struct LinkPick {
    pub link: Option<String>,
    pub note: Option<String>,
    pub warn: bool,
}

/// The first of `events` that uses `link` at the slot or within `gap_min` minutes of it.
fn taken_by<'a>(link: &str, events: &'a [EventRow], start: i64, end: i64, gap_min: i64) -> Option<&'a EventRow> {
    let key = link_key(link)?;
    let gap = gap_min.max(0) * 60_000;
    let mut hits: Vec<&EventRow> =
        events.iter().filter(|e| !e.all_day && e.end_ms >= start - gap && e.start_ms <= end + gap && uses_link(e, &key)).collect();
    // with no gap, a meeting ending exactly at the start (or starting at the end) touches; one
    // ending 10 minutes earlier doesn't
    hits.retain(|e| gap > 0 || (e.end_ms >= start && e.start_ms <= end));
    hits.sort_by_key(|e| e.start_ms);
    hits.first().copied()
}

/// The main call link unless a meeting at (or right next to) the slot uses it; then the backup if
/// that one is free; else the main one with a warning.
pub(crate) fn pick_link(main: &str, backup: &str, events: &[EventRow], start: i64, end: i64, gap_min: i64, tz: &Tz) -> LinkPick {
    let (main, backup) = (main.trim(), backup.trim());
    if main.is_empty() {
        return LinkPick { link: None, note: Some("No video link set (Tools › Options › Calendar).".into()), warn: false };
    }
    let Some(busy) = taken_by(main, events, start, end, gap_min) else {
        return LinkPick { link: Some(main.into()), note: None, warn: false };
    };
    let when = format!("“{}” ({}–{})", busy.summary, fmt_time(tz, busy.start_ms), fmt_time(tz, busy.end_ms));
    if !backup.is_empty() && taken_by(backup, events, start, end, gap_min).is_none() {
        return LinkPick {
            link: Some(backup.into()),
            note: Some(format!("Using the backup video link: the main one is in {when}.")),
            warn: false,
        };
    }
    LinkPick {
        link: Some(main.into()),
        note: Some(format!("The video link is also used by {when}. Change it before sending.")),
        warn: true,
    }
}

use chrono::{NaiveDate, TimeZone};
use chrono_tz::Asia::Jerusalem;

use super::links::*;
use crate::calendar::model::EventRow;

/// Israel time on 8 Dec 2026 → ms.
fn h(hour: u32, min: u32) -> i64 {
    Jerusalem.from_local_datetime(&NaiveDate::from_ymd_opt(2026, 12, 8).unwrap().and_hms_opt(hour, min, 0).unwrap()).unwrap().timestamp_millis()
}

fn ev(summary: &str, start_ms: i64, end_ms: i64, location: &str, description: &str, conference: &str) -> EventRow {
    EventRow {
        summary: summary.into(),
        start_ms,
        end_ms,
        location: location.into(),
        description: description.into(),
        conference: conference.into(),
        status: "confirmed".into(),
        transparency: "opaque".into(),
        ..Default::default()
    }
}

#[test]
fn link_key_matches_same_zoom_room() {
    let k = link_key("https://zoom.us/j/81234567890?pwd=abc").unwrap();
    assert_eq!(k, "zoom:81234567890");
    assert_eq!(link_key("https://us02web.zoom.us/j/81234567890?pwd=zzz#success").unwrap(), k);
    assert_eq!(link_key("https://www.example.com/room/").unwrap(), "example.com/room");
    assert_eq!(link_key("not a link"), None);
}

#[test]
fn uses_link_reads_location_notes_and_video_details() {
    let k = link_key("https://zoom.us/j/111").unwrap();
    assert!(uses_link(&ev("A", 0, 1, "https://zoom.us/j/111", "", ""), &k));
    assert!(uses_link(&ev("B", 0, 1, "", "<p>Join <a href=\"https://zoom.us/j/111?pwd=q\">here</a></p>", ""), &k));
    assert!(uses_link(&ev("C", 0, 1, "", "", "Join Zoom Meeting\nhttps://us06web.zoom.us/j/111"), &k));
    assert!(!uses_link(&ev("D", 0, 1, "https://zoom.us/j/222", "", ""), &k));
}

#[test]
fn pick_link_main_backup_warn_none() {
    let tz = Jerusalem;
    let (s, e) = (h(10, 0), h(10, 30));
    let main = "https://zoom.us/j/111";
    let backup = "https://zoom.us/j/222";
    let p = pick_link(main, backup, &[], s, e, 15, &tz);
    assert_eq!((p.link.as_deref(), p.warn, p.note.is_none()), (Some(main), false, true));
    // main ends 10 minutes before (within the 15-minute gap) → backup
    let busy = [ev("Weekly sync", h(9, 20), h(9, 50), main, "", "")];
    let p = pick_link(main, backup, &busy, s, e, 15, &tz);
    assert_eq!(p.link.as_deref(), Some(backup));
    assert_eq!(p.note.as_deref(), Some("Using the backup video link: the main one is in “Weekly sync” (09:20–09:50)."));
    assert!(!p.warn);
    // gap 0: not touching → main; touching → backup
    assert_eq!(pick_link(main, backup, &busy, s, e, 0, &tz).link.as_deref(), Some(main));
    let touching = [ev("Prep", h(9, 30), h(10, 0), main, "", "")];
    assert_eq!(pick_link(main, backup, &touching, s, e, 0, &tz).link.as_deref(), Some(backup));
    let after = [ev("Next", h(10, 30), h(11, 0), main, "", "")];
    assert_eq!(pick_link(main, backup, &after, s, e, 0, &tz).link.as_deref(), Some(backup));
    // both taken → main + warning
    let both = [ev("Weekly sync", h(9, 30), h(10, 0), main, "", ""), ev("Other", h(10, 30), h(11, 0), backup, "", "")];
    let p = pick_link(main, backup, &both, s, e, 15, &tz);
    assert_eq!((p.link.as_deref(), p.warn), (Some(main), true));
    assert_eq!(p.note.as_deref(), Some("The video link is also used by “Weekly sync” (09:30–10:00). Change it before sending."));
    // main taken, no backup → main + warning
    assert!(pick_link(main, "", &busy, s, e, 15, &tz).warn);
    // no main link
    let p = pick_link("", backup, &[], s, e, 15, &tz);
    assert_eq!(p.link, None);
    assert_eq!(p.note.as_deref(), Some("No video link set (Tools › Options › Calendar)."));
}

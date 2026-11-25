use chrono::{NaiveDate, TimeZone};
use chrono_tz::Asia::Jerusalem;

use super::{links::*, people::*, when::*};
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

#[test]
fn people_sorted_by_company_then_first_name() {
    let p = |n: &str, e: &str, c: Option<&str>| Person { name: n.into(), email: e.into(), title: None, company: c.map(Into::into), optional: false };
    let mut v = vec![
        p("Kimberly Reitano", "kimberly.reitano@tlcap.com", Some("Throughline Capital")),
        p("", "someone@gmail.com", None),
        p("Moshe Eisenberg", "moshee@camtek.com", Some("Camtek")),
        p("Earnestine Boles", "earnestine.boles@tlcap.com", Some("throughline capital")),
        p("Ehud Helft", "ehud@ekgir.com", Some("EK Global IR")),
    ];
    sort_people(&mut v);
    let order: Vec<&str> = v.iter().map(|x| x.email.as_str()).collect();
    assert_eq!(order, ["moshee@camtek.com", "ehud@ekgir.com", "earnestine.boles@tlcap.com", "kimberly.reitano@tlcap.com", "someone@gmail.com"]);
    assert_eq!(first_name(&v[4]), "someone");
    assert_eq!(first_name(&v[0]), "Moshe");
}

#[test]
fn call_link_placeholder_is_the_apps() {
    let t = mark_link("{Call Link}\n\nCall between {attendees}. { call  link }");
    assert!(!t.to_lowercase().contains("call link"));
    assert!(t.contains("{attendees}"));
    assert_eq!(put_link(&t, Some("https://zoom.us/j/1")), "https://zoom.us/j/1\n\nCall between {attendees}. https://zoom.us/j/1");
    assert_eq!(put_link(&mark_link("{call link}\n\nHi"), None), "Hi");
}

fn ms_utc(y: i32, mo: u32, d: u32, hh: u32, mm: u32) -> i64 {
    chrono::Utc.with_ymd_and_hms(y, mo, d, hh, mm, 0).unwrap().timestamp_millis()
}

#[test]
fn resolves_their_zone_to_ours() {
    let email = ms_utc(2026, 11, 25, 14, 45);
    let t = AiTime { date: Some("2026-12-08".into()), year_stated: false, start: Some("10:00".into()), end: Some("10:30".into()), time_zone: Some("America/New_York".into()) };
    let s = resolve(&t, email, Jerusalem, 45).unwrap();
    assert_eq!(s.start, ms_utc(2026, 12, 8, 15, 0));
    assert_eq!(s.end - s.start, 30 * 60_000);
    assert!(s.zone_stated);
    assert_eq!(zone_note(&s, Jerusalem), "Time read as 10:00 New York (17:00 your time).");
}

#[test]
fn missing_end_unknown_zone_and_yearless_date() {
    let email = ms_utc(2026, 12, 20, 9, 0);
    let t = AiTime { date: Some("2026-01-05".into()), year_stated: false, start: Some("09:00".into()), end: None, time_zone: Some("Mars/Base".into()) };
    let s = resolve(&t, email, Jerusalem, 45).unwrap();
    assert_eq!(s.start, ms_utc(2027, 1, 5, 7, 0));
    assert_eq!(s.end - s.start, 45 * 60_000);
    assert!(!s.zone_stated);
    assert_eq!(zone_note(&s, Jerusalem), "Time zone not stated in the email — read as your time (Jerusalem).");
    assert!(resolve(&AiTime { start: Some("09:00".into()), ..Default::default() }, email, Jerusalem, 45).is_none());
    // an end not after the start → usual length
    let t = AiTime { date: Some("2027-01-05".into()), year_stated: true, start: Some("09:00".into()), end: Some("08:00".into()), time_zone: None };
    let s = resolve(&t, email, Jerusalem, 45).unwrap();
    assert_eq!(s.end - s.start, 45 * 60_000);
    // same zone, stated → no note
    let t = AiTime { time_zone: Some("Asia/Jerusalem".into()), ..t };
    assert_eq!(zone_note(&resolve(&t, email, Jerusalem, 45).unwrap(), Jerusalem), "");
}

#[test]
fn dst_weeks_use_the_stated_zone() {
    let t = |d: &str| AiTime { date: Some(d.into()), year_stated: true, start: Some("10:00".into()), end: None, time_zone: Some("America/New_York".into()) };
    assert_eq!(resolve(&t("2026-03-16"), 0, Jerusalem, 30).unwrap().start, ms_utc(2026, 3, 16, 14, 0));
    assert_eq!(resolve(&t("2026-11-02"), 0, Jerusalem, 30).unwrap().start, ms_utc(2026, 11, 2, 15, 0));
}

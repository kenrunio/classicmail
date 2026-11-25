use chrono::{NaiveDate, TimeZone};
use chrono_tz::Asia::Jerusalem;

use super::{links::*, people::*, when::*};
use crate::calendar::model::EventRow;

/// Israel time on 8 Dec 2026 → ms.
fn h(hour: u32, min: u32) -> i64 {
    Jerusalem
        .from_local_datetime(&NaiveDate::from_ymd_opt(2026, 12, 8).unwrap().and_hms_opt(hour, min, 0).unwrap())
        .unwrap()
        .timestamp_millis()
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
    let p = |n: &str, e: &str, c: Option<&str>| Person {
        name: n.into(),
        email: e.into(),
        title: None,
        company: c.map(Into::into),
        optional: false,
    };
    let mut v = vec![
        p("Kimberly Reitano", "kimberly.reitano@tlcap.com", Some("Throughline Capital")),
        p("", "someone@gmail.com", None),
        p("Moshe Eisenberg", "moshee@camtek.com", Some("Camtek")),
        p("Earnestine Boles", "earnestine.boles@tlcap.com", Some("throughline capital")),
        p("Ehud Helft", "ehud@ekgir.com", Some("EK Global IR")),
    ];
    sort_people(&mut v);
    let order: Vec<&str> = v.iter().map(|x| x.email.as_str()).collect();
    assert_eq!(
        order,
        ["moshee@camtek.com", "ehud@ekgir.com", "earnestine.boles@tlcap.com", "kimberly.reitano@tlcap.com", "someone@gmail.com"]
    );
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
    let t = AiTime {
        date: Some("2026-12-08".into()),
        year_stated: Some(false),
        start: Some("10:00".into()),
        end: Some("10:30".into()),
        time_zone: Some("America/New_York".into()),
    };
    let s = resolve(&t, email, Jerusalem, 45).unwrap();
    assert_eq!(s.start, ms_utc(2026, 12, 8, 15, 0));
    assert_eq!(s.end - s.start, 30 * 60_000);
    assert!(s.zone_stated);
    assert_eq!(zone_note(&s, Jerusalem), "Time read as 10:00 New York (17:00 your time).");
}

#[test]
fn missing_end_unknown_zone_and_yearless_date() {
    let email = ms_utc(2026, 12, 20, 9, 0);
    let t = AiTime {
        date: Some("2026-01-05".into()),
        year_stated: Some(false),
        start: Some("09:00".into()),
        end: None,
        time_zone: Some("Mars/Base".into()),
    };
    let s = resolve(&t, email, Jerusalem, 45).unwrap();
    assert_eq!(s.start, ms_utc(2027, 1, 5, 7, 0));
    assert_eq!(s.end - s.start, 45 * 60_000);
    assert!(!s.zone_stated);
    assert_eq!(zone_note(&s, Jerusalem), "Time zone not stated in the email — read as your time (Jerusalem).");
    assert!(resolve(&AiTime { start: Some("09:00".into()), ..Default::default() }, email, Jerusalem, 45).is_none());
    // an end not after the start → usual length
    let t = AiTime {
        date: Some("2027-01-05".into()),
        year_stated: Some(true),
        start: Some("09:00".into()),
        end: Some("08:00".into()),
        time_zone: None,
    };
    let s = resolve(&t, email, Jerusalem, 45).unwrap();
    assert_eq!(s.end - s.start, 45 * 60_000);
    // same zone, stated → no note
    let t = AiTime { time_zone: Some("Asia/Jerusalem".into()), ..t };
    assert_eq!(zone_note(&resolve(&t, email, Jerusalem, 45).unwrap(), Jerusalem), "");
}

#[test]
fn dst_weeks_use_the_stated_zone() {
    let t = |d: &str| AiTime {
        date: Some(d.into()),
        year_stated: Some(true),
        start: Some("10:00".into()),
        end: None,
        time_zone: Some("America/New_York".into()),
    };
    assert_eq!(resolve(&t("2026-03-16"), 0, Jerusalem, 30).unwrap().start, ms_utc(2026, 3, 16, 14, 0));
    assert_eq!(resolve(&t("2026-11-02"), 0, Jerusalem, 30).unwrap().start, ms_utc(2026, 11, 2, 15, 0));
}

// ---- ai.schedule.fromMessage ------------------------------------------------------------------

use std::sync::Arc;

use serde_json::{Value, json};

use crate::{accounts, ctx::AppCtx, db::util::set_setting, rpc};

const THREAD: &str = "Thanks for the reply, Moshe. Our team can do 2026-12-08 10:00-10:30 America/New_York.\n\
Are you able to send us a calendar invite?\n\nFrom: Moshe Eisenberg\nHow about 2026-12-09 11:00-11:30 America/New_York?";

struct Fx {
    ctx: Arc<AppCtx>,
    _g: crate::db::tempdir::TempDirGuard,
    msg: i64,
    cal_a: i64,
    cal_b: i64,
    cal_other: i64,
}

/// Israel time on a day in December 2026 → ms.
fn il(day: u32, hour: u32, min: u32) -> i64 {
    Jerusalem
        .from_local_datetime(&NaiveDate::from_ymd_opt(2026, 12, day).unwrap().and_hms_opt(hour, min, 0).unwrap())
        .unwrap()
        .timestamp_millis()
}

async fn fixture(text: &str) -> Fx {
    let (ctx, g) = AppCtx::for_test().unwrap();
    let acct = accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny Green", "demo").await.unwrap().id;
    let other = accounts::upsert(&ctx.db, "kenny@other.demo", "Kenny Green", "demo").await.unwrap().id;
    let text = text.to_string();
    let (msg, cal_a, cal_b, cal_other) = ctx
        .db
        .write(move |c| {
            let cal = |a: i64, gid: &str, primary: i64| -> anyhow::Result<i64> {
                c.execute(
                    "INSERT INTO calendars(account_id, gcal_id, summary, is_primary, access_role, time_zone) VALUES (?1,?2,?2,?3,'owner','Asia/Jerusalem')",
                    rusqlite::params![a, gid, primary],
                )?;
                Ok(c.last_insert_rowid())
            };
            let a = cal(acct, "kenny@duvz.demo", 1)?;
            let b = cal(acct, "side@duvz.demo", 0)?;
            let o = cal(other, "kenny@other.demo", 1)?;
            set_setting(c, "calendar.callLink", &"https://zoom.us/j/111")?;
            set_setting(c, "calendar.backupCallLink", &"https://zoom.us/j/222")?;
            set_setting(c, "calendar.meetingGapMinutes", &15)?;
            c.execute(
                "INSERT INTO messages(account_id, gmail_id, thread_id, internal_date, sent_date, subject, from_name, from_email, to_json, cc_json, body_state) \
                 VALUES (?1,'g1','t1',?2,?2,'RE: [EXTERNAL] Throughline Capital Call Request - CAMT','Earnestine Boles','earnestine.boles@tlcap.com',?3,?4,1)",
                rusqlite::params![
                    acct,
                    ms_utc(2026, 11, 25, 14, 45),
                    r#"[{"name":"Moshe Eisenberg","email":"moshee@camtek.com"},{"name":"Kenny Green","email":"kenny@duvz.demo"}]"#,
                    r#"[{"name":"","email":"friend@gmail.com"},{"name":"Kenny Green","email":"KENNY@other.demo"}]"#
                ],
            )?;
            let id = c.last_insert_rowid();
            c.execute("INSERT INTO message_bodies(message_id, html, text, fetched_at) VALUES (?1, NULL, ?2, 0)", rusqlite::params![id, text])?;
            Ok((id, a, b, o))
        })
        .await
        .unwrap();
    Fx { ctx, _g: g, msg, cal_a, cal_b, cal_other }
}

impl Fx {
    /// An event: `attendee` (optional) is a guest; `recurring` makes it an occurrence of a series.
    #[allow(clippy::too_many_arguments)]
    async fn event(&self, cal: i64, title: &str, s: i64, e: i64, location: &str, attendee: Option<&str>, recurring: bool) {
        let (title, location) = (title.to_string(), location.to_string());
        let attendees = attendee
            .map(|a| format!(r#"[{{"email":"{a}","responseStatus":"accepted"}},{{"email":"kenny@duvz.demo","self":true}}]"#))
            .unwrap_or_else(|| "[]".into());
        let series = recurring.then(|| "series-1".to_string());
        self.ctx
            .db
            .write(move |c| {
                let acct: i64 = c.query_row("SELECT account_id FROM calendars WHERE id=?1", [cal], |r| r.get(0))?;
                c.execute(
                    "INSERT INTO events(account_id, calendar_id, summary, start_ms, end_ms, location, attendees, recurring_event_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    rusqlite::params![acct, cal, title, s, e, location, attendees, series],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }
    async fn prepare(&self) -> Value {
        rpc::build().call(self.ctx.clone(), "ai.schedule.fromMessage", json!({ "messageId": self.msg })).await.unwrap()
    }
}

fn subjects(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|c| c["subject"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn prepares_the_meeting_from_the_email() {
    let f = fixture(THREAD).await;
    let r = f.prepare().await;
    assert_eq!(r["found"], true);
    assert_eq!(r["calendarId"], f.cal_a);
    assert_eq!(r["start"], ms_utc(2026, 12, 8, 15, 0)); // the newest time wins
    assert_eq!(r["end"], ms_utc(2026, 12, 8, 15, 30));
    assert_eq!(r["timeZone"], "America/New_York");
    let emails: Vec<&str> = r["attendees"].as_array().unwrap().iter().map(|a| a["email"].as_str().unwrap()).collect();
    assert_eq!(emails, ["moshee@camtek.com", "earnestine.boles@tlcap.com", "friend@gmail.com"]); // both of Kenny's addresses left out
    assert_eq!(r["attendees"][0]["optional"], false);
    assert_eq!(r["attendees"][2]["optional"], true);
    assert_eq!(r["location"], "https://zoom.us/j/111");
    let notes = r["notes"].as_str().unwrap();
    assert!(
        notes.starts_with(
            "https://zoom.us/j/111\n\nCall between Moshe Eisenberg (Title of moshee, Camtek), Earnestine Boles (Title of earnestine.boles, Tlcap), "
        ),
        "{notes}"
    );
    assert!(!notes.contains("null") && !notes.contains("Gmail"), "{notes}"); // no company invented
    assert_eq!(r["info"][0]["text"], "Filled in by AI from Earnestine Boles's email.");
    assert_eq!(r["info"][1]["text"], "Time read as 10:00 New York (17:00 your time).");
    assert_eq!(r["clashes"], json!([]));
    assert_eq!(r["alreadyInvited"], json!([]));
}

#[tokio::test]
async fn only_the_organiser_calendar_is_checked() {
    let f = fixture(THREAD).await;
    f.event(f.cal_b, "Elsewhere", il(8, 17, 0), il(8, 17, 30), "https://zoom.us/j/111", None, false).await;
    f.event(f.cal_other, "Other account", il(8, 17, 0), il(8, 17, 30), "https://zoom.us/j/111", Some("earnestine.boles@tlcap.com"), false)
        .await;
    let r = f.prepare().await;
    assert_eq!(r["clashes"], json!([]));
    assert_eq!(r["alreadyInvited"], json!([]));
    assert_eq!(r["location"], "https://zoom.us/j/111");
}

#[tokio::test]
async fn already_invited_clashes_and_backup_link() {
    let f = fixture(THREAD).await;
    f.event(f.cal_a, "Throughline / Camtek", il(8, 17, 0), il(8, 17, 30), "", Some("Earnestine.Boles@tlcap.com"), false).await;
    f.event(f.cal_a, "Dentist", il(8, 17, 15), il(8, 18, 0), "", None, false).await;
    f.event(f.cal_a, "Weekly sync", il(8, 16, 20), il(8, 16, 50), "https://us02web.zoom.us/j/111?pwd=x", None, true).await;
    let r = f.prepare().await;
    assert_eq!(subjects(&r["alreadyInvited"]), ["Throughline / Camtek"]);
    assert_eq!(r["alreadyInvited"][0]["when"], "17:00–17:30");
    assert_eq!(subjects(&r["clashes"]), ["Dentist"]);
    assert_eq!(r["location"], "https://zoom.us/j/222");
    assert!(r["notes"].as_str().unwrap().starts_with("https://zoom.us/j/222"));
    assert!(r["info"].as_array().unwrap().iter().any(|l| l["text"].as_str().unwrap().contains("backup video link")));
}

#[tokio::test]
async fn no_time_found_still_prepares_people_and_notes() {
    let f = fixture("Let's find a time next week. NO-TIME").await;
    let r = f.prepare().await;
    assert_eq!(r["found"], false);
    assert_eq!(r["start"], Value::Null);
    assert!(r["notes"].as_str().unwrap().contains("Call between Moshe"), "{}", r["notes"]);
    assert!(r["info"].as_array().unwrap().iter().any(|l| l["icon"] == "warning"));
}

#[tokio::test]
async fn template_without_ai_parts_needs_no_second_call() {
    let f = fixture(THREAD).await;
    f.ctx.db.write(|c| set_setting(c, "calendar.meetingNotesTemplate", &"Join: {call link}")).await.unwrap();
    let r = f.prepare().await;
    assert_eq!(r["notes"], "Join: https://zoom.us/j/111");
}

// ---- final review fixes -----------------------------------------------------------------------

#[test]
fn twelve_hour_times_are_read_right_or_not_at_all() {
    let t = |start: &str| AiTime {
        date: Some("2026-12-08".into()),
        year_stated: Some(true),
        start: Some(start.into()),
        end: None,
        time_zone: Some("Asia/Jerusalem".into()),
    };
    let at = |start: &str| resolve(&t(start), 0, Jerusalem, 30).map(|s| s.start);
    assert_eq!(at("3:30 PM"), Some(il(8, 15, 30)));
    assert_eq!(at("12:15 am"), Some(il(8, 0, 15)));
    assert_eq!(at("12:00 pm"), Some(il(8, 12, 0)));
    assert_eq!(at("10am"), Some(il(8, 10, 0)));
    assert_eq!(at("15:30"), Some(il(8, 15, 30)));
    assert_eq!(at("10:00 xyz"), None);
    assert_eq!(at("13:00 pm"), None);
}

#[test]
fn a_missing_year_flag_keeps_the_year_given() {
    // the AI left yearStated out: a date before the email stays as given (not moved a year on)
    let t = AiTime { date: Some("2026-12-01".into()), year_stated: None, start: Some("10:00".into()), end: None, time_zone: None };
    assert_eq!(resolve(&t, il(8, 9, 0), Jerusalem, 30).unwrap().start, il(1, 10, 0));
}

#[test]
fn the_email_date_is_the_local_one() {
    // 22:30 UTC on 7 Dec is already 8 Dec in Israel
    assert_eq!(super::email_sent(ms_utc(2026, 12, 7, 22, 30), Jerusalem), "2026-12-08 00:30 (Asia/Jerusalem)");
}

#[tokio::test]
async fn nulls_from_the_ai_are_not_an_error() {
    let f = fixture("Some text FAKE-NULLS").await;
    let r = f.prepare().await;
    assert_eq!(r["found"], false);
    assert_eq!(r["attendees"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn a_failed_notes_call_keeps_the_meeting() {
    let f = fixture(THREAD).await;
    f.ctx
        .db
        .write(|c| set_setting(c, "calendar.meetingNotesTemplate", &"{call link}\n\nCall between {attendees}. FAKE-FAIL"))
        .await
        .unwrap();
    let r = f.prepare().await;
    assert_eq!(r["found"], true);
    assert_eq!(r["notes"], "https://zoom.us/j/111\n\nCall between {attendees}. FAKE-FAIL");
    assert!(
        r["info"].as_array().unwrap().iter().any(|l| l["icon"] == "warning" && l["text"].as_str().unwrap().contains("notes")),
        "{}",
        r["info"]
    );
}

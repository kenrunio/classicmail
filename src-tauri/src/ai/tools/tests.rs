//! The look-up tools on a known calendar. The test clock is Wed 14 Oct 2026, 21:00 in Israel.

use std::sync::Arc;

use chrono::{NaiveDate, TimeZone};
use chrono_tz::Asia::Jerusalem;
use serde_json::{Value, json};

use super::run;
use crate::{accounts, ctx::AppCtx, db::util::set_setting, rpc};

/// Israel time on a day in October 2026 → ms.
fn il(day: u32, h: u32, m: u32) -> i64 {
    Jerusalem
        .from_local_datetime(&NaiveDate::from_ymd_opt(2026, 10, day).unwrap().and_hms_opt(h, m, 0).unwrap())
        .unwrap()
        .timestamp_millis()
}

async fn setup() -> (Arc<AppCtx>, crate::db::tempdir::TempDirGuard) {
    let (ctx, g) = AppCtx::for_test().unwrap();
    let acct = accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny Green", "demo").await.unwrap().id;
    ctx.db
        .write(move |c| {
            let cal = |gid: &str, summary: &str, primary: i64, role: &str| -> anyhow::Result<i64> {
                c.execute(
                    "INSERT INTO calendars(account_id, gcal_id, summary, is_primary, access_role, time_zone) VALUES (?1,?2,?3,?4,?5,'Asia/Jerusalem')",
                    rusqlite::params![acct, gid, summary, primary, role],
                )?;
                Ok(c.last_insert_rowid())
            };
            let me = cal("kenny@duvz.demo", "Kenny Green", 1, "owner")?;
            let shira = cal("shira@duvz.demo", "Shira Levin", 0, "reader")?;
            let hol = cal("en.jewish#holiday@group.v.calendar.google.com", "Israel holidays", 0, "reader")?;
            let ev = |cal: i64, title: &str, s: i64, e: i64, extra: &str, vals: &[&dyn rusqlite::ToSql]| -> anyhow::Result<()> {
                let mut cols = String::from("account_id, calendar_id, summary, start_ms, end_ms");
                let mut q = String::from("?1, ?2, ?3, ?4, ?5");
                for (i, col) in extra.split(',').filter(|x| !x.is_empty()).enumerate() {
                    cols.push_str(&format!(", {col}"));
                    q.push_str(&format!(", ?{}", i + 6));
                }
                let mut ps: Vec<&dyn rusqlite::ToSql> = vec![&acct, &cal, &title, &s, &e];
                ps.extend_from_slice(vals);
                c.execute(&format!("INSERT INTO events({cols}) VALUES ({q})"), ps.as_slice())?;
                Ok(())
            };
            let orion = r#"[{"email":"noa@oriontech.example","displayName":"Noa Shapiro","responseStatus":"accepted"},{"email":"kenny@duvz.demo","self":true,"responseStatus":"accepted"}]"#;
            ev(me, "Orion results prep", il(15, 10, 0), il(15, 11, 0), "attendees,description", &[&orion, &"Join Zoom: https://zoom.us/j/123?pwd=x. Thanks"])?;
            ev(me, "Investor call (not answered yet)", il(15, 14, 0), il(15, 15, 0), "my_response", &[&"needsAction"])?;
            ev(me, "Declined lunch", il(15, 12, 0), il(15, 13, 0), "my_response", &[&"declined"])?;
            ev(me, "Gym (shown as free)", il(15, 16, 0), il(15, 17, 0), "transparency", &[&"transparent"])?;
            ev(me, "Orion board", il(12, 9, 0), il(12, 10, 0), "attendees", &[&orion])?;
            ev(me, "Out of office", il(18, 0, 0), il(19, 0, 0), "all_day,transparency", &[&1i64, &"opaque"])?;
            ev(shira, "Shira busy", il(15, 8, 0), il(15, 12, 0), "", &[])?;
            ev(hol, "Simchat Torah (example)", il(19, 0, 0), il(20, 0, 0), "all_day,transparency", &[&1i64, &"transparent"])?;
            c.execute("INSERT INTO task_lists(account_id, title) VALUES (?1, 'Tasks')", [acct])?;
            let list = c.last_insert_rowid();
            c.execute("INSERT INTO tasks(account_id, list_id, title, due) VALUES (?1, ?2, 'Corvina Q3 release draft', '2026-10-20')", rusqlite::params![acct, list])?;
            c.execute("INSERT INTO tasks(account_id, list_id, title, due, status) VALUES (?1, ?2, 'Old done thing', '2026-10-01', 'completed')", rusqlite::params![acct, list])?;
            Ok(())
        })
        .await
        .unwrap();
    (ctx, g)
}

fn whens(v: &Value) -> Vec<String> {
    v["slots"].as_array().unwrap().iter().map(|s| s["when"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn find_free_time_keeps_gaps_and_counts_unanswered_invites() {
    let (ctx, _g) = setup().await;
    let v = run(&ctx, "find_free_time", &json!({ "from": "2026-10-15", "to": "2026-10-15" })).await;
    // 45-minute slots, 15 minutes clear of the 10:00 meeting and of the unanswered 14:00 invite;
    // the declined lunch and the "free" gym don't count
    assert_eq!(whens(&v), ["Thu 15 Oct 2026, 08:00–08:45", "Thu 15 Oct 2026, 11:15–12:00"], "{v}");
    assert_eq!(v["time_zone"], "Asia/Jerusalem");
    assert_eq!(v["meeting_minutes"], 45);
    let v = run(&ctx, "find_free_time", &json!({ "from": "2026-10-15", "to": "2026-10-15", "part_of_day": "afternoon" })).await;
    assert_eq!(whens(&v), ["Thu 15 Oct 2026, 12:00–12:45", "Thu 15 Oct 2026, 15:15–16:00"], "{v}");
}

#[tokio::test]
async fn find_free_time_in_their_hours_too() {
    let (ctx, _g) = setup().await;
    let v = run(&ctx, "find_free_time", &json!({ "from": "2026-10-15", "to": "2026-10-15", "their_time_zone": "America/New_York" })).await;
    // 09:00 in New York is 16:00 in Israel that week
    assert_eq!(whens(&v), ["Thu 15 Oct 2026, 16:00–16:45"], "{v}");
    assert_eq!(v["slots"][0]["their_time"], "Thu 15 Oct 2026, 09:00–09:45 (America/New_York)");
}

#[tokio::test]
async fn find_free_time_on_a_named_calendar() {
    let (ctx, _g) = setup().await;
    let v = run(&ctx, "find_free_time", &json!({ "from": "2026-10-15", "to": "2026-10-15", "calendar": "Shira" })).await;
    assert_eq!(v["calendar"], "Shira Levin");
    assert_eq!(whens(&v)[0], "Thu 15 Oct 2026, 12:15–13:00", "{v}");
    let v = run(&ctx, "find_free_time", &json!({ "from": "2026-10-15", "to": "2026-10-15", "calendar": "Nobody" })).await;
    assert!(v["error"].as_str().unwrap().contains("Shira Levin"), "{v}");
}

#[tokio::test]
async fn find_free_time_skips_weekends_and_days_off_and_notes_holidays() {
    let (ctx, _g) = setup().await;
    let v = run(&ctx, "find_free_time", &json!({ "from": "2026-10-16", "to": "2026-10-19", "max": 10 })).await;
    let w = whens(&v);
    assert!(w.iter().all(|s| s.starts_with("Mon 19 Oct")), "Fri / Sat aren't work days, Sunday is out of office: {w:?}");
    assert_eq!(v["slots"][0]["holiday"], "Simchat Torah (example)");
    assert_eq!(v["holidays_in_period"][0]["day"], "Mon 19 Oct 2026");
}

#[tokio::test]
async fn usual_length_and_gap_are_settings() {
    let (ctx, _g) = setup().await;
    ctx.db
        .write(|c| {
            set_setting(c, "calendar.meetingMinutes", &30)?;
            set_setting(c, "calendar.meetingGapMinutes", &0)?;
            Ok(())
        })
        .await
        .unwrap();
    let v = run(&ctx, "find_free_time", &json!({ "from": "2026-10-15", "to": "2026-10-15" })).await;
    assert_eq!(whens(&v), ["Thu 15 Oct 2026, 08:00–08:30", "Thu 15 Oct 2026, 11:00–11:30"], "{v}");
}

#[tokio::test]
async fn whats_on_and_meetings() {
    let (ctx, _g) = setup().await;
    let v = run(&ctx, "whats_on", &json!({ "from": "2026-10-15" })).await;
    let titles: Vec<&str> = v["events"].as_array().unwrap().iter().map(|e| e["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Orion results prep", "Investor call (not answered yet)", "Gym (shown as free)"], "declined left out");
    assert_eq!(v["events"][2]["shown_as"], "free");
    // by company (the attendees' domain), next and last
    let next = run(&ctx, "find_meeting", &json!({ "query": "oriontech.example" })).await;
    assert_eq!(next["meetings"][0]["title"], "Orion results prep");
    assert_eq!(next["meetings"][0]["when"], "Thu 15 Oct 2026, 10:00–11:00");
    let last = run(&ctx, "find_meeting", &json!({ "query": "Orion", "which": "last" })).await;
    assert_eq!(last["meetings"][0]["title"], "Orion board");
    let id = next["meetings"][0]["id"].as_i64().unwrap();
    let d = run(&ctx, "meeting_details", &json!({ "id": id })).await;
    assert_eq!(d["meeting_links"], json!(["https://zoom.us/j/123?pwd=x"]));
    assert_eq!(d["attendees"][0]["name"], "Noa Shapiro");
}

#[tokio::test]
async fn converts_times_and_finds_tasks() {
    let (ctx, _g) = setup().await;
    // the week Israel has moved its clocks back and New York hasn't: 6 hours, not 7
    let v = run(&ctx, "convert_time", &json!({ "time": "2026-10-27 14:00", "to_zones": ["America/New_York", "Mars/Base"] })).await;
    assert_eq!(v["converted"][0]["time"], "Tue 27 Oct 2026, 08:00");
    assert!(v["converted"][1]["error"].is_string());
    let v = run(&ctx, "find_tasks", &json!({ "query": "corvina" })).await;
    assert_eq!(v["tasks"][0]["due"], "2026-10-20");
    assert_eq!(run(&ctx, "find_tasks", &json!({ "due_to": "2026-10-10" })).await["tasks"], json!([]), "completed ones only on request");
}

#[tokio::test]
async fn ai_review_looks_up_the_calendar() {
    let (ctx, _g) = setup().await;
    let reg = rpc::build();
    let r =
        reg.call(ctx.clone(), "ai.compose.review", json!({ "html": "<p>Happy to speak on {a free slot this week}.</p>" })).await.unwrap();
    // the (fake) model called find_free_time and wrote the first slot it got back
    assert_eq!(r["html"], "<p>Happy to speak on Thu 15 Oct 2026, 08:00–08:45.</p>");
    assert_eq!(r["applied"], 1);
}

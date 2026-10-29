//! Integration tests: calendar sync + rpc against the in-process fake Google server.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::{accounts, ctx::AppCtx, google::calendar as gcal, rpc};

const HOUR: i64 = 3_600_000;

async fn setup() -> (Arc<AppCtx>, crate::db::tempdir::TempDirGuard, i64) {
    let (ctx, guard) = AppCtx::for_test().unwrap();
    let acct = accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny Green", "demo").await.unwrap();
    (ctx, guard, acct.id)
}

async fn sync(ctx: &Arc<AppCtx>, account: i64) {
    let api = ctx.api(account).await.unwrap();
    super::sync::sync_account(ctx, &api).await.unwrap();
}

async fn call(ctx: &Arc<AppCtx>, m: &str, p: Value) -> Value {
    rpc::build().call(ctx.clone(), m, p).await.unwrap_or_else(|e| panic!("{m}: {e:#}"))
}

fn now() -> i64 {
    crate::clock::now_ms()
}

async fn count(ctx: &Arc<AppCtx>, sql: &'static str) -> i64 {
    ctx.db.read(move |c| Ok(c.query_row(sql, [], |r| r.get(0))?)).await.unwrap()
}

async fn primary_gcal(ctx: &Arc<AppCtx>) -> (i64, String) {
    ctx.db
        .read(|c| Ok(c.query_row("SELECT id, gcal_id FROM calendars WHERE is_primary=1", [], |r| Ok((r.get(0)?, r.get(1)?)))?))
        .await
        .unwrap()
}

#[tokio::test]
async fn full_sync_brings_calendars_events_and_series() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let cals = call(&ctx, "calendar.listCalendars", json!({})).await;
    let cals = cals.as_array().unwrap();
    assert_eq!(cals.len(), 4, "free/busy-only calendars are not listed: {cals:?}");
    assert!(cals.iter().any(|c| c["name"] == "Israel holidays" && c["canWrite"] == false));
    let evs = call(&ctx, "calendar.listEvents", json!({ "start": now() - 14 * 24 * HOUR, "end": now() + 42 * 24 * HOUR })).await;
    let evs = evs.as_array().unwrap();
    assert!(evs.len() >= 60, "only {} events", evs.len());
    // recurring weekly team call: instances with the moved exception, the cancelled one absent
    let team: Vec<&Value> = evs.iter().filter(|e| e["summary"].as_str().unwrap_or("").starts_with("Weekly team call")).collect();
    assert!(team.len() >= 5, "{}", team.len());
    assert!(team.iter().all(|e| e["isRecurring"] == true));
    assert!(team.iter().any(|e| e["summary"] == "Weekly team call (moved)" && e["isException"] == true));
    // masters stored but not listed
    assert!(count(&ctx, "SELECT COUNT(*) FROM events WHERE is_master=1 AND recurrence IS NOT NULL").await >= 5);
    // an invite awaiting my answer shows as tentative
    assert!(evs.iter().any(|e| e["myResponse"] == "needsAction" && e["busyStatus"] == "tentative" && e["isMeeting"] == true));
    // multi-day all-day conference
    assert!(evs.iter().any(|e| e["allDay"] == true && e["summary"].as_str().unwrap().contains("Atlas TMT")));
    // second sync changes nothing and is idempotent
    let n1 = count(&ctx, "SELECT COUNT(*) FROM events").await;
    sync(&ctx, a).await;
    assert_eq!(n1, count(&ctx, "SELECT COUNT(*) FROM events").await);
    // detail of an occurrence has the series' recurrence
    let id = team[0]["id"].as_i64().unwrap();
    let d = call(&ctx, "calendar.getEvent", json!({ "id": id })).await;
    assert!(d["recurrence"][0].as_str().unwrap().starts_with("RRULE:FREQ=WEEKLY"));
    assert!(d["attendees"].as_array().unwrap().len() >= 3);
}

/// Events synced before the Zoom / Meet details were kept get them at the next sync, though
/// Google reports them unchanged.
#[tokio::test]
async fn video_call_details_reach_events_synced_before() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let sql = "SELECT conference FROM events WHERE summary LIKE 'Atlas TMT Conference — speaker briefing%' AND conference<>''";
    let has = |ctx: &Arc<AppCtx>| {
        let ctx = ctx.clone();
        async move { ctx.db.read(move |c| Ok(c.query_row(sql, [], |r| r.get::<_, String>(0)).ok())).await.unwrap() }
    };
    assert!(has(&ctx).await.unwrap().starts_with("Join Zoom Meeting\nhttps://"));
    // as an older version stored it
    ctx.db.write(|c| Ok(c.execute("UPDATE events SET conference=''", [])?)).await.unwrap();
    sync(&ctx, a).await;
    assert!(has(&ctx).await.is_some(), "filled in again");
}

#[tokio::test]
async fn create_update_delete_round_trip() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let (cal_id, cal_gid) = primary_gcal(&ctx).await;
    let start = now() + 3 * 24 * HOUR;
    let start = start - start % 60_000;
    let d = call(
        &ctx,
        "calendar.createEvent",
        json!({ "calendarId": cal_id, "summary": "Roadshow prep", "location": "Office", "start": start, "end": start + HOUR, "reminderMinutes": 10, "busyStatus": "oof", "isPrivate": true, "importance": "high" }),
    )
    .await;
    let id = d["id"].as_i64().unwrap();
    assert_eq!(d["pending"], true);
    assert_eq!(crate::sync::pending::count(&ctx.db, a, "calendar").await.unwrap(), 1);
    sync(&ctx, a).await;
    assert_eq!(crate::sync::pending::count(&ctx.db, a, "calendar").await.unwrap(), 0);
    let d = call(&ctx, "calendar.getEvent", json!({ "id": id })).await;
    assert_eq!(d["pending"], false);
    assert_eq!(d["busyStatus"], "oof");
    assert_eq!(d["isPrivate"], true);
    assert_eq!(d["importance"], "high");
    assert_eq!(d["reminderMinutes"], 10);
    let gid: String = ctx.db.read(move |c| Ok(c.query_row("SELECT gcal_id FROM events WHERE id=?1", [id], |r| r.get(0))?)).await.unwrap();
    let api = ctx.api(a).await.unwrap();
    let remote = gcal::get_event(&api, &cal_gid, &gid).await.unwrap();
    assert_eq!(remote.summary.as_deref(), Some("Roadshow prep"));
    assert_eq!(remote.visibility.as_deref(), Some("private"));

    // move + resize (drag) — persisted locally at once, pushed on sync
    let ns = start + 2 * HOUR;
    let d = call(&ctx, "calendar.updateEvent", json!({ "id": id, "start": ns, "end": ns + 90 * 60_000 })).await;
    assert_eq!(d["start"], ns);
    sync(&ctx, a).await;
    let remote = gcal::get_event(&api, &cal_gid, &gid).await.unwrap();
    let rs = chrono::DateTime::parse_from_rfc3339(remote.start.unwrap().date_time.as_deref().unwrap()).unwrap().timestamp_millis();
    assert_eq!(rs, ns);
    let d = call(&ctx, "calendar.getEvent", json!({ "id": id })).await;
    assert_eq!(d["end"], ns + 90 * 60_000, "survives the pull");

    // all-day conversion
    call(&ctx, "calendar.updateEvent", json!({ "id": id, "allDay": true, "startDate": "2027-02-01", "endDate": "2027-02-03" })).await;
    sync(&ctx, a).await;
    let remote = gcal::get_event(&api, &cal_gid, &gid).await.unwrap();
    assert_eq!(remote.start.unwrap().date.as_deref(), Some("2027-02-01"));
    assert_eq!(remote.end.unwrap().date.as_deref(), Some("2027-02-03"));

    // delete
    call(&ctx, "calendar.deleteEvent", json!({ "id": id })).await;
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM events WHERE summary='Roadshow prep'").await, 0);
    sync(&ctx, a).await;
    // gone, or (like Google) kept as a cancelled tombstone
    assert!(gcal::get_event(&api, &cal_gid, &gid).await.ok().is_none_or(|e| e.status.as_deref() == Some("cancelled")));
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM events WHERE summary='Roadshow prep'").await, 0);
}

#[tokio::test]
async fn create_then_delete_before_push_never_reaches_server() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let start = now() + 24 * HOUR;
    let d = call(&ctx, "calendar.createEvent", json!({ "summary": "Oops", "start": start, "end": start + HOUR })).await;
    call(&ctx, "calendar.deleteEvent", json!({ "id": d["id"] })).await;
    assert_eq!(crate::sync::pending::count(&ctx.db, a, "calendar").await.unwrap(), 0);
    sync(&ctx, a).await;
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM events WHERE summary='Oops'").await, 0);
}

#[tokio::test]
async fn respond_round_trip_and_invite_contracts() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let (row_id, uid): (i64, String) = ctx
        .db
        .read(|c| {
            Ok(c.query_row("SELECT id, ical_uid FROM events WHERE my_response='needsAction' ORDER BY start_ms LIMIT 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?)
        })
        .await
        .unwrap();
    // findByIcalUid
    let f = call(&ctx, "calendar.findByIcalUid", json!({ "accountId": a, "icalUid": uid })).await;
    assert_eq!(f["id"], row_id);
    assert_eq!(f["myResponse"], "needsAction");
    // accept via the mail-pane contract
    let r = call(
        &ctx,
        "calendar.respondToInvite",
        json!({ "accountId": a, "icalUid": uid, "response": "accepted", "comment": "See you there" }),
    )
    .await;
    assert_eq!(r["eventId"], row_id);
    sync(&ctx, a).await;
    let f = call(&ctx, "calendar.findByIcalUid", json!({ "accountId": a, "icalUid": uid })).await;
    assert_eq!(f["myResponse"], "accepted");
    assert_eq!(f["busyStatus"], "busy");
    // server has my response
    let (cal_gid, gid): (String, String) = ctx
        .db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT c.gcal_id, e.gcal_id FROM events e JOIN calendars c ON c.id=e.calendar_id WHERE e.id=?1",
                [row_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap();
    let api = ctx.api(a).await.unwrap();
    let remote = gcal::get_event(&api, &cal_gid, &gid).await.unwrap();
    let me = remote.attendees.unwrap().into_iter().find(|x| x.email == "kenny@duvz.demo").unwrap();
    assert_eq!(me.response_status, "accepted");
    assert_eq!(me.comment.as_deref(), Some("See you there"));
    // decline via calendar.respond hides it from the calendar
    call(&ctx, "calendar.respond", json!({ "id": row_id, "response": "declined" })).await;
    let evs = call(&ctx, "calendar.listEvents", json!({ "start": 0, "end": i64::MAX / 2 })).await;
    assert!(!evs.as_array().unwrap().iter().any(|e| e["id"] == row_id));
    // unknown uid
    let r = call(&ctx, "calendar.respondToInvite", json!({ "accountId": a, "icalUid": "nope@x", "response": "accepted" })).await;
    assert!(r["eventId"].is_null());
    let f = call(&ctx, "calendar.findByIcalUid", json!({ "accountId": a, "icalUid": "nope@x" })).await;
    assert!(f.is_null());
}

#[tokio::test]
async fn recurring_series_create_occurrence_and_series_edits() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let start = now() + 2 * 24 * HOUR;
    let start = start - start % 60_000;
    let d = call(
        &ctx,
        "calendar.createEvent",
        json!({ "summary": "Investor update series", "start": start, "end": start + HOUR, "recurrence": ["RRULE:FREQ=DAILY;COUNT=4"] }),
    )
    .await;
    assert_eq!(d["isRecurring"], true);
    assert_eq!(d["pending"], true);
    // placeholders show at once
    let q = |ctx: Arc<AppCtx>| async move {
        call(&ctx, "calendar.listEvents", json!({ "start": now(), "end": now() + 30 * 24 * HOUR }))
            .await
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["summary"].as_str().unwrap_or("").contains("Investor update"))
            .cloned()
            .collect::<Vec<Value>>()
    };
    assert_eq!(q(ctx.clone()).await.len(), 4);
    sync(&ctx, a).await;
    let items = q(ctx.clone()).await;
    assert_eq!(items.len(), 4);
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM events WHERE parent_local_id IS NOT NULL").await, 0, "placeholders replaced");
    assert_eq!(
        count(&ctx, "SELECT COUNT(*) FROM events WHERE summary='Investor update series' AND recurring_event_id IS NOT NULL").await,
        4
    );

    // move just the second occurrence → exception on the server
    let second = items[1]["id"].as_i64().unwrap();
    let s2 = items[1]["start"].as_i64().unwrap();
    call(
        &ctx,
        "calendar.updateEvent",
        json!({ "id": second, "scope": "occurrence", "summary": "Investor update (moved)", "start": s2 + HOUR, "end": s2 + 2 * HOUR }),
    )
    .await;
    sync(&ctx, a).await;
    let items = q(ctx.clone()).await;
    assert_eq!(items.len(), 4);
    let moved = items.iter().find(|e| e["summary"] == "Investor update (moved)").expect("moved occurrence");
    assert_eq!(moved["start"], s2 + HOUR);
    assert_eq!(moved["isException"], true);

    // delete the third occurrence only
    let third = items.iter().filter(|e| e["summary"] == "Investor update series").nth(1).unwrap()["id"].as_i64().unwrap();
    call(&ctx, "calendar.deleteEvent", json!({ "id": third, "scope": "occurrence" })).await;
    sync(&ctx, a).await;
    assert_eq!(q(ctx.clone()).await.len(), 3);

    // rename the whole series from an occurrence
    let first = q(ctx.clone()).await[0]["id"].as_i64().unwrap();
    call(&ctx, "calendar.updateEvent", json!({ "id": first, "scope": "series", "location": "Board room" })).await;
    sync(&ctx, a).await;
    let items = q(ctx.clone()).await;
    assert_eq!(items.len(), 3);
    assert!(items.iter().filter(|e| e["summary"] == "Investor update series").all(|e| e["location"] == "Board room"));

    // change the rule: 4 daily → 2 weekly
    call(&ctx, "calendar.updateEvent", json!({ "id": first, "scope": "series", "recurrence": ["RRULE:FREQ=WEEKLY;COUNT=2"] })).await;
    sync(&ctx, a).await;
    assert_eq!(q(ctx.clone()).await.len(), 2);

    // delete the series
    let any = q(ctx.clone()).await[0]["id"].as_i64().unwrap();
    call(&ctx, "calendar.deleteEvent", json!({ "id": any, "scope": "series" })).await;
    assert_eq!(q(ctx.clone()).await.len(), 0);
    sync(&ctx, a).await;
    assert_eq!(q(ctx.clone()).await.len(), 0);
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM events WHERE summary LIKE 'Investor update%'").await, 0);
}

#[tokio::test]
async fn window_replace_drops_server_deletions_but_keeps_local_edits() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let (_cal, cal_gid) = primary_gcal(&ctx).await;
    let rows: Vec<(i64, String)> = ctx
        .db
        .read(|c| {
            let mut st = c.prepare("SELECT id, gcal_id FROM events WHERE recurring_event_id IS NULL AND is_master=0 AND my_response IS NULL AND calendar_id=(SELECT id FROM calendars WHERE is_primary=1) ORDER BY start_ms LIMIT 2")?;
            Ok(st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?)
        })
        .await
        .unwrap();
    let api = ctx.api(a).await.unwrap();
    // both deleted on the server; one of them was edited locally meanwhile
    gcal::delete_event(&api, &cal_gid, &rows[0].1, "none").await.unwrap();
    gcal::patch_event(&api, &cal_gid, &rows[1].1, json!({ "summary": "Changed on the web" }), "none").await.unwrap();
    let edited = rows[1].0;
    ctx.db.write(move |c| Ok(c.execute("UPDATE events SET summary='Local edit', local_dirty=1 WHERE id=?1", [edited])?)).await.unwrap();
    let (wmin, wmax) = super::sync::window();
    let cal = ctx.db.read(|c| super::model::get_cal(c, 1)).await.unwrap();
    super::sync::pull_calendar(&ctx, &api, &cal).await.unwrap();
    let _ = (wmin, wmax);
    let gone = rows[0].0;
    assert_eq!(
        ctx.db.read(move |c| Ok(c.query_row("SELECT COUNT(*) FROM events WHERE id=?1", [gone], |r| r.get::<_, i64>(0))?)).await.unwrap(),
        0
    );
    let s: String = ctx.db.read(move |c| Ok(c.query_row("SELECT summary FROM events WHERE id=?1", [edited], |r| r.get(0))?)).await.unwrap();
    assert_eq!(s, "Local edit", "dirty rows are not overwritten by a pull");
}

#[tokio::test]
async fn read_only_calendar_rejects_edits() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let hol: i64 =
        ctx.db.read(|c| Ok(c.query_row("SELECT id FROM calendars WHERE access_role='reader'", [], |r| r.get(0))?)).await.unwrap();
    let err = rpc::build()
        .call(ctx.clone(), "calendar.createEvent", json!({ "calendarId": hol, "summary": "x", "start": now(), "end": now() + HOUR }))
        .await;
    assert!(err.is_err());
    let ev: i64 =
        ctx.db.read(move |c| Ok(c.query_row("SELECT id FROM events WHERE calendar_id=?1 LIMIT 1", [hol], |r| r.get(0))?)).await.unwrap();
    assert!(rpc::build().call(ctx.clone(), "calendar.deleteEvent", json!({ "id": ev })).await.is_err());
}

#[tokio::test]
async fn reminders_due_dismiss_snooze() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    // clear seeded state that might be due right now
    let due = call(&ctx, "calendar.dueReminders", json!({})).await;
    let keys: Vec<Value> = due.as_array().unwrap().iter().map(|i| i["key"].clone()).collect();
    if !keys.is_empty() {
        call(&ctx, "calendar.dismissReminder", json!({ "keys": keys })).await;
    }
    let start = now() + 10 * 60_000;
    let d =
        call(&ctx, "calendar.createEvent", json!({ "summary": "Call Sarah", "start": start, "end": start + HOUR, "reminderMinutes": 15 }))
            .await;
    let due = call(&ctx, "calendar.dueReminders", json!({})).await;
    let item = due.as_array().unwrap().iter().find(|i| i["eventId"] == d["id"]).expect("due").clone();
    call(&ctx, "calendar.snoozeReminder", json!({ "keys": [item["key"]], "minutes": 5 })).await;
    let due = call(&ctx, "calendar.dueReminders", json!({})).await;
    assert!(!due.as_array().unwrap().iter().any(|i| i["eventId"] == d["id"]));
    // snooze over → due again; dismiss → gone
    ctx.db.write(|c| Ok(c.execute("UPDATE reminder_state SET snooze_until=1", [])?)).await.unwrap();
    let due = call(&ctx, "calendar.dueReminders", json!({})).await;
    assert!(due.as_array().unwrap().iter().any(|i| i["eventId"] == d["id"]));
    call(&ctx, "calendar.dismissReminder", json!({ "keys": [item["key"]] })).await;
    let due = call(&ctx, "calendar.dueReminders", json!({})).await;
    assert!(!due.as_array().unwrap().iter().any(|i| i["eventId"] == d["id"]));
    // moving the item re-arms its reminder
    call(&ctx, "calendar.updateEvent", json!({ "id": d["id"], "start": start + 60_000, "end": start + HOUR })).await;
    let due = call(&ctx, "calendar.dueReminders", json!({})).await;
    assert!(due.as_array().unwrap().iter().any(|i| i["eventId"] == d["id"]));
    let _ = a;
}

#[tokio::test]
async fn meeting_create_sends_and_copy_and_search() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let start = now() + 5 * 24 * HOUR;
    let d = call(
        &ctx,
        "calendar.createEvent",
        json!({ "summary": "Q4 strategy", "start": start, "end": start + HOUR, "attendees": [{ "email": "sarah.cohen@harborcap.example", "name": "Sarah Cohen" }], "sendUpdates": true }),
    )
    .await;
    assert_eq!(d["isMeeting"], true);
    assert_eq!(d["isOrganizer"], true);
    let att = d["attendees"].as_array().unwrap();
    assert_eq!(att.len(), 2, "organizer added: {att:?}");
    sync(&ctx, a).await;
    let c = call(&ctx, "calendar.copyEvent", json!({ "id": d["id"], "start": start + 24 * HOUR, "end": start + 25 * HOUR })).await;
    assert_eq!(c["isMeeting"], false);
    assert_eq!(c["summary"], "Q4 strategy");
    let s = call(&ctx, "calendar.search", json!({ "query": "q4 strat" })).await;
    assert_eq!(s.as_array().unwrap().len(), 2);
}

/// Responses › Resend Invitation: one meeting request e-mail, to just that attendee.
#[tokio::test]
async fn resend_invitation_mails_just_that_attendee() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let start = now() + 5 * 24 * HOUR;
    let guests = json!([{ "email": "sarah.cohen@harborcap.example", "name": "Sarah Cohen" }, { "email": "noa@oriontech.example", "name": "Noa Shapiro" }]);
    let d = call(
        &ctx,
        "calendar.createEvent",
        json!({ "summary": "Q4 strategy", "start": start, "end": start + HOUR, "attendees": guests, "sendUpdates": true }),
    )
    .await;
    sync(&ctx, a).await; // on Google now
    let api = ctx.api(a).await.unwrap();
    let before = api.gmail_list(Some("subject:Invitation"), &["SENT"], None, 50, false).await.unwrap().messages.len();
    let r = call(&ctx, "calendar.resendInvite", json!({ "id": d["id"], "emails": ["noa@oriontech.example"] })).await;
    assert_eq!(r, json!({ "sent": ["noa@oriontech.example"], "error": null }));
    let sent = api.gmail_list(Some("subject:Invitation"), &["SENT"], None, 50, false).await.unwrap().messages;
    assert_eq!(sent.len(), before + 1);
    let m = api.gmail_get(&sent[0].id, "raw", &[]).await.unwrap();
    let raw =
        base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, m.raw.unwrap_or_default().trim_end_matches('=')).unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("noa@oriontech.example") && !text.contains("To: \"Sarah Cohen\""), "only Noa");
    assert!(text.contains("METHOD:REQUEST") && text.contains("SUMMARY:Q4 strategy"));
    // someone who isn't invited gets nothing
    let r = call(&ctx, "calendar.resendInvite", json!({ "id": d["id"], "emails": ["stranger@x.example"] })).await;
    assert_eq!(r["sent"], json!([]));
}

#[tokio::test]
async fn task_reminders_are_included_and_dismissable() {
    let (ctx, _g, a) = setup().await;
    let at = now() - 60_000;
    ctx.db
        .write(move |c| {
            c.execute("INSERT INTO task_lists(id, account_id, title) VALUES (1, ?1, 'Tasks')", [a])?;
            c.execute(
                "INSERT INTO tasks(id, account_id, list_id, title, reminder_at) VALUES (7, ?1, 1, 'Send Q3 deck', ?2)",
                rusqlite::params![a, at],
            )?;
            c.execute(
                "INSERT INTO tasks(id, account_id, list_id, title, reminder_at, status) VALUES (8, ?1, 1, 'Done already', ?2, 'completed')",
                rusqlite::params![a, at],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let due = call(&ctx, "calendar.dueReminders", json!({})).await;
    let tasks: Vec<&Value> = due.as_array().unwrap().iter().filter(|i| i["kind"] == "task").collect();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["summary"], "Send Q3 deck");
    let key = tasks[0]["key"].clone();
    call(&ctx, "calendar.snoozeReminder", json!({ "keys": [key.clone()], "minutes": 10 })).await;
    assert!(!call(&ctx, "calendar.dueReminders", json!({})).await.as_array().unwrap().iter().any(|i| i["kind"] == "task"));
    ctx.db.write(|c| Ok(c.execute("UPDATE task_reminder_state SET snooze_until=1", [])?)).await.unwrap();
    assert!(call(&ctx, "calendar.dueReminders", json!({})).await.as_array().unwrap().iter().any(|i| i["kind"] == "task"));
    call(&ctx, "calendar.dismissReminder", json!({ "keys": [key] })).await;
    assert!(!call(&ctx, "calendar.dueReminders", json!({})).await.as_array().unwrap().iter().any(|i| i["kind"] == "task"));
}

/// Google's contact birthdays live on the primary calendar as eventType "birthday": shown, but
/// not editable (Google refuses time changes and deletion for them).
#[tokio::test]
async fn birthday_events_are_read_only() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let bd: i64 = ctx
        .db
        .read(|c| {
            Ok(c.query_row("SELECT id FROM events WHERE event_type='birthday' AND recurring_event_id IS NOT NULL LIMIT 1", [], |r| {
                r.get(0)
            })?)
        })
        .await
        .unwrap();
    let d = call(&ctx, "calendar.getEvent", json!({ "id": bd })).await;
    assert_eq!(d["readOnly"], true, "{d}");
    let err = rpc::build().call(ctx.clone(), "calendar.updateEvent", json!({ "id": bd, "summary": "x" })).await;
    assert!(err.is_err());
    assert!(rpc::build().call(ctx.clone(), "calendar.deleteEvent", json!({ "id": bd })).await.is_err());
    // the fake refuses it too, like Google
    let api = ctx.api(a).await.unwrap();
    let (_, gcal_id) = primary_gcal(&ctx).await;
    let gid: String = ctx.db.read(move |c| Ok(c.query_row("SELECT gcal_id FROM events WHERE id=?1", [bd], |r| r.get(0))?)).await.unwrap();
    let e = gcal::patch_event(&api, &gcal_id, &gid, json!({ "start": { "date": "2030-01-01" }, "end": { "date": "2030-01-02" } }), "none")
        .await
        .unwrap_err();
    assert_eq!(crate::google::client::api_status(&e), Some(400));
    gcal::patch_event(&api, &gcal_id, &gid, json!({ "colorId": "3" }), "none").await.unwrap();
    // orderBy=startTime without singleEvents is a 400 on Google
    let url = format!("{}/calendars/{}/events", api.urls.calendar, crate::google::seg(&gcal_id));
    let e = api.get::<Value>(&url, &[("orderBy", "startTime".into())]).await.unwrap_err();
    assert_eq!(crate::google::client::api_status(&e), Some(400));
}

/// Remove Recurrence: the series' patch tells Google `recurrence: []` (an omitted key keeps it).
#[tokio::test]
async fn removing_recurrence_is_sent_to_google() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let start = now() + 2 * 24 * HOUR;
    let start = start - start % 60_000;
    let d = call(
        &ctx,
        "calendar.createEvent",
        json!({ "summary": "Series to end", "start": start, "end": start + HOUR, "recurrence": ["RRULE:FREQ=DAILY;COUNT=3"] }),
    )
    .await;
    sync(&ctx, a).await;
    let master = d["seriesId"].as_i64().unwrap();
    call(&ctx, "calendar.updateEvent", json!({ "id": master, "scope": "series", "recurrence": [] })).await;
    let (row, cal) = ctx
        .db
        .read(move |c| {
            let row = super::load_row(c, master)?;
            let cal = super::model::get_cal(c, row.calendar_id)?;
            Ok((row, cal))
        })
        .await
        .unwrap();
    assert!(!row.is_master);
    assert_eq!(row.to_google(&cal)["recurrence"], json!([]));
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM events WHERE summary='Series to end'").await, 1, "instances dropped");
}

/// A calendar whose time zone changed on Google gets the new zone (no other field changed).
#[tokio::test]
async fn calendar_time_zone_change_is_stored() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let tz: String =
        ctx.db.read(|c| Ok(c.query_row("SELECT time_zone FROM calendars WHERE is_primary=1", [], |r| r.get(0))?)).await.unwrap();
    ctx.db.write(|c| Ok(c.execute("UPDATE calendars SET time_zone='Pacific/Fiji' WHERE is_primary=1", [])?)).await.unwrap();
    sync(&ctx, a).await;
    let now_tz: String =
        ctx.db.read(|c| Ok(c.query_row("SELECT time_zone FROM calendars WHERE is_primary=1", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(now_tz, tz);
    assert_ne!(tz, "Pacific/Fiji");
}

/// Between the daily full pulls only changes are fetched (an edit and a deletion arrive, an
/// unchanged event isn't fetched again, and nothing is swept); the full pull still sweeps.
#[tokio::test]
async fn incremental_pull_between_daily_full_pulls() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let (cal_id, cal_gid) = primary_gcal(&ctx).await;
    let rows: Vec<(i64, String)> = ctx
        .db
        .read(move |c| {
            let mut st = c.prepare(
                "SELECT id, gcal_id FROM events WHERE recurring_event_id IS NULL AND is_master=0 AND my_response IS NULL \
                 AND calendar_id=?1 ORDER BY start_ms LIMIT 3",
            )?;
            Ok(st.query_map([cal_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?)
        })
        .await
        .unwrap();
    let (edited, deleted, untouched) = (rows[0].0, rows[1].0, rows[2].0);
    // the seed was written just now: as if the last pull had been 5 minutes later (the overlap),
    // so the seed counts as older than it
    let key = format!("calendar.pulled:{cal_id}");
    let (k2, at) = (key.clone(), now() + 300_000);
    ctx.db
        .write(move |c| {
            Ok(c.execute("UPDATE sync_state SET value=?2 || substr(value, instr(value, ' ')) WHERE kind=?1", rusqlite::params![k2, at])?)
        })
        .await
        .unwrap();
    let api = ctx.api(a).await.unwrap();
    gcal::patch_event(&api, &cal_gid, &rows[0].1, json!({ "summary": "Edited on the web" }), "none").await.unwrap();
    gcal::delete_event(&api, &cal_gid, &rows[1].1, "none").await.unwrap();
    // markers: a stale etag (a fetch would replace it) and a row the server never had
    ctx.db.write(move |c| Ok(c.execute("UPDATE events SET etag='stale' WHERE id=?1", [untouched])?)).await.unwrap();
    let start = now() + 3 * 24 * HOUR;
    let ghost = ctx
        .db
        .write(move |c| {
            let row = super::model::EventRow {
                account_id: a,
                calendar_id: cal_id,
                gcal_id: Some("ghost-event".into()),
                summary: "Ghost".into(),
                start_ms: start,
                end_ms: start + HOUR,
                status: "confirmed".into(),
                ..Default::default()
            };
            row.insert(c)
        })
        .await
        .unwrap();
    let one = |sql: &'static str, id: i64| {
        let ctx = ctx.clone();
        async move { ctx.db.read(move |c| Ok(c.query_row(sql, [id], |r| r.get::<_, Option<String>>(0)).ok().flatten())).await.unwrap() }
    };
    sync(&ctx, a).await;
    assert_eq!(one("SELECT summary FROM events WHERE id=?1", edited).await.as_deref(), Some("Edited on the web"));
    assert_eq!(one("SELECT summary FROM events WHERE id=?1", deleted).await, None, "deletion arrives as cancelled");
    assert_eq!(one("SELECT etag FROM events WHERE id=?1", untouched).await.as_deref(), Some("stale"), "not fetched again");
    assert!(one("SELECT summary FROM events WHERE id=?1", ghost).await.is_some(), "no sweep on an incremental pull");
    // the next day's first pull is a full one: it sweeps and refreshes everything
    ctx.db
        .write(move |c| {
            Ok(c.execute("UPDATE sync_state SET value=substr(value, 1, instr(value, ' ')) || '2000-01-01' WHERE kind=?1", [key])?)
        })
        .await
        .unwrap();
    sync(&ctx, a).await;
    assert_eq!(one("SELECT summary FROM events WHERE id=?1", ghost).await, None, "the full pull sweeps");
    assert_ne!(one("SELECT etag FROM events WHERE id=?1", untouched).await.as_deref(), Some("stale"));
}

/// A series whose rule changed between full pulls gets all its occurrences re-read: none left on
/// the old day, and an exception (a moved occurrence) is kept.
#[tokio::test]
async fn incremental_pull_refreshes_a_changed_series() {
    use chrono::{Datelike, TimeZone};
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let (cal_id, cal_gid) = primary_gcal(&ctx).await;
    let tz = ctx.db.read(move |c| super::model::get_cal(c, cal_id)).await.unwrap().tz();
    // next Tuesday 10:00 in the calendar's zone
    let today = chrono::DateTime::from_timestamp_millis(now()).unwrap().with_timezone(&tz).date_naive();
    let tue = (1..=7).map(|d| today + chrono::Duration::days(d)).find(|d| d.weekday() == chrono::Weekday::Tue).unwrap();
    let start = tz.from_local_datetime(&tue.and_hms_opt(10, 0, 0).unwrap()).unwrap().timestamp_millis();
    call(
        &ctx,
        "calendar.createEvent",
        json!({ "summary": "Desk rota", "start": start, "end": start + HOUR, "recurrence": ["RRULE:FREQ=WEEKLY;BYDAY=TU,TH"] }),
    )
    .await;
    sync(&ctx, a).await;
    let occ = |ctx: Arc<AppCtx>| async move {
        call(&ctx, "calendar.listEvents", json!({ "start": now(), "end": now() + 60 * 24 * HOUR }))
            .await
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["summary"].as_str().unwrap_or("").starts_with("Desk rota"))
            .cloned()
            .collect::<Vec<Value>>()
    };
    let day = move |e: &Value| chrono::DateTime::from_timestamp_millis(e["start"].as_i64().unwrap()).unwrap().with_timezone(&tz).weekday();
    let thu = occ(ctx.clone()).await.into_iter().find(|e| day(e) == chrono::Weekday::Thu).unwrap();
    let moved_to = thu["start"].as_i64().unwrap() + 2 * HOUR;
    let thu_id = thu["id"].as_i64().unwrap();
    call(
        &ctx,
        "calendar.updateEvent",
        json!({ "id": thu_id, "scope": "occurrence", "summary": "Desk rota (moved)", "start": moved_to, "end": moved_to + HOUR }),
    )
    .await;
    sync(&ctx, a).await;
    // the rule changes on the web: Tuesdays become Wednesdays
    let master_gid: String = ctx
        .db
        .read(|c| Ok(c.query_row("SELECT gcal_id FROM events WHERE summary='Desk rota' AND is_master=1", [], |r| r.get(0))?))
        .await
        .unwrap();
    let api = ctx.api(a).await.unwrap();
    gcal::patch_event(&api, &cal_gid, &master_gid, json!({ "recurrence": ["RRULE:FREQ=WEEKLY;BYDAY=WE,TH"] }), "none").await.unwrap();
    sync(&ctx, a).await;
    let items = occ(ctx.clone()).await;
    assert!(!items.iter().any(|e| day(e) == chrono::Weekday::Tue), "no Tuesdays left");
    assert!(items.iter().any(|e| day(e) == chrono::Weekday::Wed));
    let moved = items.iter().find(|e| e["summary"] == "Desk rota (moved)").expect("the exception survives");
    assert_eq!(moved["start"], moved_to);
}

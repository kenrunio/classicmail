//! Integration tests: calendar groups, colour categories, duplicates and moving between calendars.

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
        .read(|c| {
            Ok(c.query_row("SELECT id, gcal_id FROM calendars WHERE is_primary=1 ORDER BY account_id", [], |r| Ok((r.get(0)?, r.get(1)?)))?)
        })
        .await
        .unwrap()
}

#[test]
fn calendar_groups_follow_outlook() {
    use super::dupes::classify;
    let me = "kenny@duvz.demo";
    assert_eq!(classify("kenny@duvz.demo", "owner", true, me), "mine");
    assert_eq!(classify("c_abc123@group.calendar.google.com", "owner", false, me), "mine");
    assert_eq!(classify("c_abc123@group.calendar.google.com", "writer", false, me), "shared");
    assert_eq!(classify("c_abc123@group.calendar.google.com", "reader", false, me), "other");
    // someone else's personal calendar: shared, whatever the access role
    assert_eq!(classify("robin.parker@mail.example", "writer", false, me), "shared");
    assert_eq!(classify("robin.parker@mail.example", "owner", false, me), "shared");
    assert_eq!(classify("dana.levi@duvz.demo", "reader", false, me), "shared");
    // subscriptions
    assert_eq!(classify("en.jewish#holiday@group.v.calendar.google.com", "reader", false, me), "other");
    assert_eq!(classify("addressbook#contacts@group.v.calendar.google.com", "reader", false, me), "other");
    assert_eq!(classify("abcdef@import.calendar.google.com", "reader", false, me), "other");
}

#[tokio::test]
async fn seeded_calendars_are_grouped() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let cals = call(&ctx, "calendar.listCalendars", json!({})).await;
    let group =
        |name: &str| cals.as_array().unwrap().iter().find(|c| c["name"] == name).map(|c| c["group"].as_str().unwrap().to_string()).unwrap();
    assert_eq!(group("kenny@duvz.demo"), "mine");
    assert_eq!(group("Conferences"), "mine");
    assert_eq!(group("robin.parker@gmail.demo"), "shared");
    assert_eq!(group("Israel holidays"), "other");
}

#[tokio::test]
async fn categories_round_trip_through_google() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let (cal_id, cal_gid) = primary_gcal(&ctx).await;
    let start = now() + 2 * 24 * HOUR;
    let start = start - start % 60_000;
    let d = call(&ctx, "calendar.createEvent", json!({ "calendarId": cal_id, "summary": "Categorised", "start": start, "end": start + HOUR, "categories": ["Green Category", "Blue Category"] })).await;
    let id = d["id"].as_i64().unwrap();
    assert_eq!(d["categories"], json!(["Green Category", "Blue Category"]));
    assert_eq!(d["colorId"], "10", "the first category's colour → Basil");
    sync(&ctx, a).await;
    let api = ctx.api(a).await.unwrap();
    let gid: String = ctx.db.read(move |c| Ok(c.query_row("SELECT gcal_id FROM events WHERE id=?1", [id], |r| r.get(0))?)).await.unwrap();
    let ev = gcal::get_event(&api, &cal_gid, &gid).await.unwrap();
    assert_eq!(ev.color_id.as_deref(), Some("10"));
    assert_eq!(ev.extended_properties.as_ref().unwrap().private.as_ref().unwrap()["cmCategories"], "Green Category, Blue Category");

    // categorize from the menu: Red first → Tomato; then clear → colour removed
    call(&ctx, "calendar.setEventCategories", json!({ "ids": [id], "categories": ["Red Category"] })).await;
    sync(&ctx, a).await;
    let ev = gcal::get_event(&api, &cal_gid, &gid).await.unwrap();
    assert_eq!(ev.color_id.as_deref(), Some("11"));
    call(&ctx, "calendar.setEventCategories", json!({ "ids": [id], "categories": [] })).await;
    sync(&ctx, a).await;
    let ev = gcal::get_event(&api, &cal_gid, &gid).await.unwrap();
    assert_eq!(ev.color_id, None);

    // categories / colour set in another client come back on pull
    gcal::patch_event(
        &api,
        &cal_gid,
        &gid,
        json!({ "colorId": "3", "extendedProperties": { "private": { "cmCategories": "Purple Category, Clients" } } }),
        "none",
    )
    .await
    .unwrap();
    sync(&ctx, a).await;
    let d = call(&ctx, "calendar.getEvent", json!({ "id": id })).await;
    assert_eq!(d["categories"], json!(["Purple Category", "Clients"]));
    assert_eq!(d["colorId"], "3");

    // rename + recolour in the master list: items follow, Google gets the new colour
    let mut list = call(&ctx, "calendar.listCategories", json!({})).await["categories"].clone();
    for c in list.as_array_mut().unwrap() {
        if c["name"] == "Purple Category" {
            c["name"] = json!("Investors");
            c["color"] = json!(5);
            c["shortcut"] = json!("F2");
        }
    }
    call(&ctx, "calendar.saveCategories", json!({ "categories": list, "renames": [["Purple Category", "Investors"]] })).await;
    sync(&ctx, a).await;
    let ev = gcal::get_event(&api, &cal_gid, &gid).await.unwrap();
    assert_eq!(ev.extended_properties.as_ref().unwrap().private.as_ref().unwrap()["cmCategories"], "Investors, Clients");
    assert_eq!(ev.color_id.as_deref(), Some("10"));
    let list = call(&ctx, "calendar.listCategories", json!({})).await;
    assert!(list["categories"].as_array().unwrap().iter().any(|c| c["name"] == "Investors" && c["shortcut"] == "F2"));
}

#[tokio::test]
async fn categorize_gmail_event_and_series() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    // a Gmail-created event (read-only otherwise) can still be categorised
    let hotel: i64 = count(&ctx, "SELECT MIN(id) FROM events WHERE summary='Stay at Grand Hyatt San Francisco'").await;
    call(&ctx, "calendar.setEventCategories", json!({ "ids": [hotel], "categories": ["Orange Category"] })).await;
    // an occurrence categorises its whole series
    let occ: i64 = count(&ctx, "SELECT MIN(id) FROM events WHERE summary='Market open review' AND is_master=0").await;
    call(&ctx, "calendar.setEventCategories", json!({ "ids": [occ], "categories": ["Blue Category"] })).await;
    sync(&ctx, a).await;
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM pending_ops WHERE domain='calendar'").await, 0, "all pushed");
    assert!(
        count(
            &ctx,
            "SELECT COUNT(*) FROM events WHERE summary='Market open review' AND is_master=0 AND categories='Blue Category' AND color_id='9'"
        )
        .await
            >= 5
    );
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM events WHERE id=(SELECT MIN(id) FROM events WHERE summary='Stay at Grand Hyatt San Francisco') AND color_id='6'").await, 1);
}

#[tokio::test]
async fn duplicates_are_merged_for_display_and_offered_for_clean_up() {
    let (ctx, _g, a) = setup().await;
    let b = accounts::upsert(&ctx.db, "info@duvz.demo", "Duvz Software", "demo").await.unwrap().id;
    sync(&ctx, a).await;
    sync(&ctx, b).await;
    let range = json!({ "start": now() - 2 * 24 * HOUR, "end": now() + 10 * 24 * HOUR });
    let merged = call(&ctx, "calendar.listEvents", range.clone()).await;
    let mut all = range.clone();
    all["dedupe"] = json!(false);
    let raw = call(&ctx, "calendar.listEvents", all).await;
    let n = |v: &Value, s: &str| v.as_array().unwrap().iter().filter(|e| e["summary"] == s).count();
    // same invitation in both mailboxes → once, from kenny's calendar (listed first)
    assert_eq!(n(&raw, "Atlas TMT Conference — speaker briefing"), 2);
    assert_eq!(n(&merged, "Atlas TMT Conference — speaker briefing"), 1);
    let kept = merged.as_array().unwrap().iter().find(|e| e["summary"] == "Atlas TMT Conference — speaker briefing").unwrap();
    assert_eq!(kept["accountId"], a);
    assert_eq!(kept["alsoIn"], json!(["Calendar - info@duvz.demo"]));
    // shared family calendar + my own copy of her invitation → once
    assert_eq!(n(&raw, "Dinner with the Parkers"), 2);
    assert_eq!(n(&merged, "Dinner with the Parkers"), 1);
    // duplicates inside one calendar stay visible (Clean Up handles them)
    assert_eq!(n(&merged, "Stay at Grand Hyatt San Francisco"), 3);
    // the setting turns merging off
    ctx.db.write(|c| crate::db::util::set_setting(c, "calendar.mergeDuplicates", &false)).await.unwrap();
    assert_eq!(n(&call(&ctx, "calendar.listEvents", range.clone()).await, "Dinner with the Parkers"), 2);

    // Clean Up Calendar on kenny's primary
    let (cal_id, cal_gid) = primary_gcal(&ctx).await;
    let r = call(&ctx, "calendar.findDuplicates", json!({ "calendarIds": [cal_id] })).await;
    let groups = r["groups"].as_array().unwrap();
    let g = |s: &str| {
        groups.iter().find(|g| g["summary"].as_str().unwrap().starts_with(s)).unwrap_or_else(|| panic!("no group {s}: {groups:?}"))
    };
    let hotel = g("Stay at Grand Hyatt");
    let items = hotel["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["hasNotes"], true, "the one with notes is kept");
    assert_eq!(items[0]["suggestDelete"], false);
    assert!(items[1..].iter().all(|i| i["suggestDelete"] == true));
    assert_eq!(g("Call with David")["items"][0]["hasNotes"], true);
    // a meeting someone else organised: never pre-ticked, flagged for a warning
    let intro = g("Summit Securities / Duvz Software — intro");
    assert!(intro["items"].as_array().unwrap().iter().all(|i| i["warn"] == true && i["suggestDelete"] == false));
    // occurrences of a series are never offered
    assert!(!groups.iter().any(|g| g["summary"].as_str().unwrap().starts_with("Weekly team call")));

    // delete only what is confirmed
    let ids: Vec<i64> = items[1..].iter().map(|i| i["id"].as_i64().unwrap()).collect();
    let res = call(&ctx, "calendar.deleteEvents", json!({ "ids": ids })).await;
    assert_eq!(res["deleted"], 2);
    sync(&ctx, a).await;
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM events WHERE summary='Stay at Grand Hyatt San Francisco'").await, 1);
    let api = ctx.api(a).await.unwrap();
    let left = gcal::list_events(&api, &cal_gid, &gcal::ListEvents { q: Some("Grand Hyatt".into()), ..Default::default() }).await.unwrap();
    assert_eq!(left.len(), 1);
    assert!(left[0].description.is_some(), "the copy with the notes survived");
    let r = call(&ctx, "calendar.findDuplicates", json!({ "calendarIds": [cal_id] })).await;
    assert!(!r["groups"].as_array().unwrap().iter().any(|g| g["summary"] == "Stay at Grand Hyatt San Francisco"));
}

#[tokio::test]
async fn move_between_calendars() {
    let (ctx, _g, a) = setup().await;
    let b = accounts::upsert(&ctx.db, "info@duvz.demo", "Duvz Software", "demo").await.unwrap().id;
    sync(&ctx, a).await;
    sync(&ctx, b).await;
    let (prim, prim_gid) = primary_gcal(&ctx).await;
    let conf: (i64, String) = ctx
        .db
        .read(|c| Ok(c.query_row("SELECT id, gcal_id FROM calendars WHERE summary='Conferences'", [], |r| Ok((r.get(0)?, r.get(1)?)))?))
        .await
        .unwrap();
    let info_cal: i64 = ctx
        .db
        .read(move |c| Ok(c.query_row("SELECT id FROM calendars WHERE account_id=?1 AND is_primary=1", [b], |r| r.get(0))?))
        .await
        .unwrap();
    let start = now() + 5 * 24 * HOUR;
    let start = start - start % 60_000;
    let d =
        call(&ctx, "calendar.createEvent", json!({ "calendarId": prim, "summary": "Move me", "start": start, "end": start + HOUR })).await;
    let id = d["id"].as_i64().unwrap();
    sync(&ctx, a).await;
    // same account: events.move, plus a new time in the same step
    let d =
        call(&ctx, "calendar.moveToCalendar", json!({ "id": id, "calendarId": conf.0, "start": start + HOUR, "end": start + 2 * HOUR }))
            .await;
    assert_eq!(d["calendarId"], conf.0);
    sync(&ctx, a).await;
    let api = ctx.api(a).await.unwrap();
    let q = || gcal::ListEvents { q: Some("Move me".into()), ..Default::default() };
    let in_conf = gcal::list_events(&api, &conf.1, &q()).await.unwrap();
    assert_eq!(in_conf.len(), 1);
    assert_eq!(in_conf[0].start.as_ref().and_then(|s| s.date_time.as_deref()).and_then(super::model::parse_rfc3339), Some(start + HOUR));
    assert!(gcal::list_events(&api, &prim_gid, &q()).await.unwrap().is_empty());
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM events WHERE summary='Move me'").await, 1);
    // another account: copy + delete
    let d = call(&ctx, "calendar.moveToCalendar", json!({ "id": id, "calendarId": info_cal })).await;
    assert_eq!(d["accountId"], b);
    sync(&ctx, a).await;
    sync(&ctx, b).await;
    assert!(gcal::list_events(&api, &conf.1, &q()).await.unwrap().is_empty());
    let api_b = ctx.api(b).await.unwrap();
    assert_eq!(gcal::list_events(&api_b, "primary", &q()).await.unwrap().len(), 1);
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM events WHERE summary='Move me'").await, 1);
    // a meeting organised by someone else can't be moved
    let inv: i64 = count(&ctx, "SELECT MIN(id) FROM events WHERE summary='Atlas TMT Conference — speaker briefing'").await;
    assert!(rpc::build().call(ctx.clone(), "calendar.moveToCalendar", json!({ "id": inv, "calendarId": conf.0 })).await.is_err());
    // an item not yet on the server just changes calendar before its create is pushed
    let d =
        call(&ctx, "calendar.createEvent", json!({ "calendarId": prim, "summary": "Unpushed", "start": start, "end": start + HOUR })).await;
    let id2 = d["id"].as_i64().unwrap();
    call(&ctx, "calendar.moveToCalendar", json!({ "id": id2, "calendarId": info_cal })).await;
    sync(&ctx, a).await;
    sync(&ctx, b).await;
    assert_eq!(
        gcal::list_events(&api_b, "primary", &gcal::ListEvents { q: Some("Unpushed".into()), ..Default::default() }).await.unwrap().len(),
        1
    );
    assert!(
        gcal::list_events(&api, &prim_gid, &gcal::ListEvents { q: Some("Unpushed".into()), ..Default::default() })
            .await
            .unwrap()
            .is_empty()
    );
}

/// An edit still waiting to be pushed when the item is moved to another calendar reaches Google
/// (after the move) instead of being sent to the new calendar too early and lost.
#[tokio::test]
async fn edit_waiting_before_a_move_is_not_lost() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let (prim, _) = primary_gcal(&ctx).await;
    let conf: (i64, String) = ctx
        .db
        .read(|c| Ok(c.query_row("SELECT id, gcal_id FROM calendars WHERE summary='Conferences'", [], |r| Ok((r.get(0)?, r.get(1)?)))?))
        .await
        .unwrap();
    let start = now() + 6 * 24 * HOUR;
    let start = start - start % 60_000;
    let d =
        call(&ctx, "calendar.createEvent", json!({ "calendarId": prim, "summary": "Edit then move", "start": start, "end": start + HOUR }))
            .await;
    let id = d["id"].as_i64().unwrap();
    sync(&ctx, a).await;
    // offline: the edit and the move both wait in the queue, the edit first
    call(&ctx, "calendar.updateEvent", json!({ "id": id, "location": "Room 4" })).await;
    call(&ctx, "calendar.moveToCalendar", json!({ "id": id, "calendarId": conf.0 })).await;
    sync(&ctx, a).await;
    let api = ctx.api(a).await.unwrap();
    let found =
        gcal::list_events(&api, &conf.1, &gcal::ListEvents { q: Some("Edit then move".into()), ..Default::default() }).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].location.as_deref(), Some("Room 4"));
    let d = call(&ctx, "calendar.getEvent", json!({ "id": id })).await;
    assert_eq!(d["location"], "Room 4");
}

/// Renaming a colour category renames it on tasks too.
#[tokio::test]
async fn category_rename_reaches_tasks() {
    let (ctx, _g, a) = setup().await;
    let list = call(&ctx, "tasks.createList", json!({ "accountId": a, "title": "Follow-ups" })).await;
    let t = call(&ctx, "tasks.create", json!({ "listId": list["id"], "title": "Call Eli", "categories": ["Purple Category", "Clients"] }))
        .await;
    let other = call(&ctx, "tasks.create", json!({ "listId": list["id"], "title": "Book room", "categories": ["Clients"] })).await;
    let mut cats = call(&ctx, "calendar.listCategories", json!({})).await["categories"].clone();
    for c in cats.as_array_mut().unwrap() {
        if c["name"] == "Purple Category" {
            c["name"] = json!("Investors");
        }
    }
    let n = call(&ctx, "calendar.saveCategories", json!({ "categories": cats, "renames": [["Purple Category", "Investors"]] })).await;
    assert_eq!(n, 1);
    assert_eq!(call(&ctx, "tasks.get", json!({ "id": t["id"] })).await["categories"], json!(["Investors", "Clients"]));
    assert_eq!(call(&ctx, "tasks.get", json!({ "id": other["id"] })).await["categories"], json!(["Clients"]));
}

#[tokio::test]
async fn rename_your_copy_of_someone_elses_meeting() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let (_, cal_gid) = primary_gcal(&ctx).await;
    let api = ctx.api(a).await.unwrap();
    let start = now() + 48 * HOUR;
    let body = json!({
        "summary": "Orion results call",
        "start": { "dateTime": chrono::DateTime::from_timestamp_millis(start).unwrap().to_rfc3339() },
        "end": { "dateTime": chrono::DateTime::from_timestamp_millis(start + HOUR).unwrap().to_rfc3339() },
        "organizer": { "email": "noa@oriontech.example", "displayName": "Noa Shapiro" },
        "attendees": [
            { "email": "noa@oriontech.example", "organizer": true, "responseStatus": "accepted" },
            { "email": "kenny@duvz.demo", "self": true, "responseStatus": "needsAction" }
        ]
    });
    let gid = gcal::insert_event(&api, &cal_gid, body, "none").await.unwrap().id.unwrap();
    sync(&ctx, a).await;
    let id: i64 = ctx
        .db
        .read({
            let g = gid.clone();
            move |c| Ok(c.query_row("SELECT id FROM events WHERE gcal_id=?1", [g], |r| r.get(0))?)
        })
        .await
        .unwrap();
    let d = call(&ctx, "calendar.getEvent", json!({ "id": id })).await;
    assert_eq!(d["isOrganizer"], false, "{d}");

    // rename it: yours everywhere here, the organizer's at Google (yours on your copy)
    let d = call(
        &ctx,
        "undo.run",
        json!({ "label": "Rename", "method": "calendar.setOwnSummary", "params": { "id": id, "summary": "Orion Q3 - prep notes" } }),
    )
    .await;
    assert_eq!(d["summary"], "Orion Q3 - prep notes");
    assert_eq!(d["originalSummary"], "Orion results call");
    sync(&ctx, a).await;
    let ev = gcal::get_event(&api, &cal_gid, &gid).await.unwrap();
    assert_eq!(ev.summary.as_deref(), Some("Orion results call"), "the organizer's subject is untouched");
    assert_eq!(ev.extended_properties.as_ref().unwrap().private.as_ref().unwrap()["cmSummary"], "Orion Q3 - prep notes");
    // it survives a pull of a change made elsewhere (the organizer moves the meeting room)
    gcal::patch_event(&api, &cal_gid, &gid, json!({ "location": "Room 4" }), "none").await.unwrap();
    sync(&ctx, a).await;
    let d = call(&ctx, "calendar.getEvent", json!({ "id": id })).await;
    assert_eq!(d["summary"], "Orion Q3 - prep notes");

    // Undo: back to the organizer's subject
    call(&ctx, "undo.undo", json!({})).await;
    let d = call(&ctx, "calendar.getEvent", json!({ "id": id })).await;
    assert_eq!(d["summary"], "Orion results call");
    assert!(d.get("originalSummary").is_none() || d["originalSummary"].is_null());
    sync(&ctx, a).await;
    let ev = gcal::get_event(&api, &cal_gid, &gid).await.unwrap();
    assert_eq!(ev.extended_properties.as_ref().unwrap().private.as_ref().unwrap()["cmSummary"], "");

    // your own appointments are renamed by editing them
    let mine: i64 = ctx
        .db
        .read(|c| Ok(c.query_row("SELECT id FROM events WHERE organizer_self=1 AND is_master=0 LIMIT 1", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert!(rpc::build().call(ctx.clone(), "calendar.setOwnSummary", json!({ "id": mine, "summary": "x" })).await.is_err());
}

#[tokio::test]
async fn an_appointment_keeps_the_time_zone_it_was_set_in() {
    let (ctx, _g, a) = setup().await;
    sync(&ctx, a).await;
    let (cal_id, cal_gid) = primary_gcal(&ctx).await;
    // 10:00 in New York on 7 December 2026 = 15:00 UTC
    let start = chrono::DateTime::parse_from_rfc3339("2026-12-07T15:00:00Z").unwrap().timestamp_millis();
    let d = call(
        &ctx,
        "calendar.createEvent",
        json!({ "calendarId": cal_id, "summary": "NYC investor lunch", "start": start, "end": start + HOUR, "timeZone": "America/New_York" }),
    )
    .await;
    assert_eq!(d["timeZone"], "America/New_York");
    assert_eq!(d["start"].as_i64(), Some(start));
    sync(&ctx, a).await;
    let api = ctx.api(a).await.unwrap();
    let gid: String = {
        let id = d["id"].as_i64().unwrap();
        ctx.db.read(move |c| Ok(c.query_row("SELECT gcal_id FROM events WHERE id=?1", [id], |r| r.get(0))?)).await.unwrap()
    };
    let ev = gcal::get_event(&api, &cal_gid, &gid).await.unwrap();
    let s = serde_json::to_value(ev.start.unwrap()).unwrap();
    assert_eq!(s["timeZone"], "America/New_York", "{s}");
    assert!(s["dateTime"].as_str().unwrap().starts_with("2026-12-07T10:00:00-05:00"), "{s}");
    // a nonsense zone is ignored (the calendar's stays)
    let id = d["id"].as_i64().unwrap();
    let d = call(
        &ctx,
        "calendar.updateEvent",
        json!({ "id": id, "scope": "occurrence", "timeZone": "Mars/Base", "start": start, "end": start + HOUR }),
    )
    .await;
    assert_eq!(d["timeZone"], "America/New_York");
}

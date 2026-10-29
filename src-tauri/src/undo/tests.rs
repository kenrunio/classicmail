//! Undo stack + inverses against the fake Google server.

use std::sync::Arc;

use rusqlite::params;
use serde_json::{Map, Value, json};

use super::{Op, UndoStack};
use crate::{accounts, ctx::AppCtx, rpc};

fn op(v: i64) -> Op {
    Op::new("settings.set", json!({ "key": "mail.pollSeconds", "value": v }))
}

#[test]
fn limit_trims_oldest_and_new_action_clears_redo() {
    let s = UndoStack::default();
    for i in 0..5 {
        s.push(format!("a{i}"), vec![op(i)], vec![], Map::new(), 3);
    }
    let (u, r) = s.entries();
    assert_eq!(u.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(), ["a2", "a3", "a4"]);
    assert!(r.is_empty());
    // something on the redo stack, then a new action
    {
        let mut st = s.st.lock();
        let e = st.undo.pop().unwrap();
        st.redo.push(e);
    }
    assert_eq!(s.peek().redo_label.as_deref(), Some("a4"));
    s.push("b".into(), vec![op(9)], vec![], Map::new(), 3);
    let p = s.peek();
    assert_eq!(p.undo_label.as_deref(), Some("b"));
    assert_eq!(p.redo_label, None, "a new action clears redo");
    assert_eq!(p.undo_count, 3);
    s.trim(1);
    assert_eq!(s.peek().undo_count, 1);
}

#[test]
fn substitution_follows_paths() {
    let mut vars = Map::new();
    vars.insert("new".into(), json!({ "id": 7, "list": [1, 2] }));
    let v = super::subst(&json!({ "id": "$new.id", "ids": ["$new.list.1", "$nope", "plain"], "n": 3 }), &vars);
    assert_eq!(v, json!({ "id": 7, "ids": [2, "$nope", "plain"], "n": 3 }));
    assert_eq!(super::subst(&json!("$new.list"), &vars), json!([1, 2]));
}

async fn ctx() -> (Arc<AppCtx>, crate::db::tempdir::TempDirGuard) {
    AppCtx::for_test().unwrap()
}

async fn call(ctx: &Arc<AppCtx>, m: &str, p: Value) -> Value {
    rpc::build().call(ctx.clone(), m, p).await.unwrap_or_else(|e| panic!("{m}: {e:#}"))
}

async fn setting(ctx: &Arc<AppCtx>, k: &str) -> Value {
    call(ctx, "settings.get", json!({ "key": k })).await
}

async fn run(ctx: &Arc<AppCtx>, label: &str, m: &str, p: Value) -> Value {
    call(ctx, "undo.run", json!({ "label": label, "method": m, "params": p })).await
}

#[tokio::test]
async fn replay_through_registry_undo_redo_and_limit_setting() {
    let (ctx, _g) = ctx().await;
    run(&ctx, "Poll 120", "settings.set", json!({ "key": "mail.pollSeconds", "value": 120 })).await;
    run(&ctx, "Poll 300", "settings.set", json!({ "key": "mail.pollSeconds", "value": 300 })).await;
    assert_eq!(call(&ctx, "undo.peek", json!({})).await["undoLabel"], "Poll 300");
    let r = call(&ctx, "undo.undo", json!({})).await;
    assert_eq!(r["label"], "Poll 300");
    assert_eq!(setting(&ctx, "mail.pollSeconds").await, 120);
    call(&ctx, "undo.undo", json!({})).await;
    assert_eq!(setting(&ctx, "mail.pollSeconds").await, Value::Null, "back to never set");
    assert_eq!(call(&ctx, "undo.undo", json!({})).await["label"], Value::Null, "nothing left");
    call(&ctx, "undo.redo", json!({})).await;
    call(&ctx, "undo.redo", json!({})).await;
    assert_eq!(setting(&ctx, "mail.pollSeconds").await, 300);
    let p = call(&ctx, "undo.peek", json!({})).await;
    assert_eq!((p["undoLabel"].as_str(), p["redoLabel"].as_str()), (Some("Poll 300"), None));
    // events carry the status-bar message
    let ev: Vec<Value> = ctx.events.captured().into_iter().filter(|e| e["kind"] == "undo.changed").collect();
    assert!(ev.iter().any(|e| e["message"] == "Undid: Poll 300"));
    assert!(ev.iter().any(|e| e["message"] == "Redid: Poll 300"));
    // app.undoSteps limits the history
    call(&ctx, "settings.set", json!({ "key": "app.undoSteps", "value": 2 })).await;
    for i in 0..5 {
        run(&ctx, &format!("s{i}"), "settings.set", json!({ "key": "mail.pollSeconds", "value": i })).await;
    }
    assert_eq!(ctx.undo.peek().undo_count, 2);
    // push: front-end supplied ops
    call(&ctx, "undo.push", json!({ "label": "Custom", "undo": [{ "method": "settings.set", "params": { "key": "mail.pollSeconds", "value": 42 } }], "redo": [] })).await;
    call(&ctx, "undo.undo", json!({})).await;
    assert_eq!(setting(&ctx, "mail.pollSeconds").await, 42);
    call(&ctx, "undo.clear", json!({})).await;
    assert_eq!(ctx.undo.peek(), super::Peek::default());
}

#[tokio::test]
async fn failing_inverse_reports_and_drops_the_entry() {
    let (ctx, _g) = ctx().await;
    run(&ctx, "Poll", "settings.set", json!({ "key": "mail.pollSeconds", "value": 1 })).await;
    call(&ctx, "undo.push", json!({ "label": "Broken", "undo": [{ "method": "tasks.delete", "params": { "nope": 1 } }], "redo": [] }))
        .await;
    let e = rpc::build().call(ctx.clone(), "undo.undo", json!({})).await.unwrap_err();
    let msg = format!("{e:#}");
    assert!(msg.starts_with("Couldn't undo \"Broken\""), "{msg}");
    // dropped (not on either stack); the older entry is next
    let p = ctx.undo.peek();
    assert_eq!((p.undo_label.as_deref(), p.redo_label.as_deref()), (Some("Poll"), None));
    assert!(
        ctx.events
            .captured()
            .iter()
            .any(|e| e["kind"] == "undo.changed" && e["message"].as_str().unwrap_or("").starts_with("Couldn't undo"))
    );
    // only known helpers may be pushed from the undo domain
    assert!(
        rpc::build()
            .call(ctx.clone(), "undo.push", json!({ "label": "x", "undo": [{ "method": "undo.undo" }], "redo": [] }))
            .await
            .is_err()
    );
    assert!(rpc::build().call(ctx.clone(), "undo.run", json!({ "label": "x", "method": "undo.clear" })).await.is_err());
}

// ---- mail ------------------------------------------------------------------------------------------

struct M {
    ctx: Arc<AppCtx>,
    _g: crate::db::tempdir::TempDirGuard,
    api: crate::google::Api,
    account: i64,
}

async fn mail_setup(email: &str) -> M {
    let (ctx, g) = AppCtx::for_test().unwrap();
    let a = accounts::upsert(&ctx.db, email, "Kenny Green", "demo").await.unwrap();
    let api = ctx.api(a.id).await.unwrap();
    crate::mail::sync::sync_account(&ctx, &api).await.unwrap();
    M { ctx, _g: g, api, account: a.id }
}

impl M {
    async fn folder(&self, role: &str, path: Option<&str>) -> i64 {
        let (a, role, path) = (self.account, role.to_string(), path.map(str::to_string));
        self.ctx
            .db
            .read(move |c| {
                Ok(match path {
                    Some(p) => c.query_row("SELECT id FROM folders WHERE account_id=?1 AND path=?2", params![a, p], |r| r.get(0))?,
                    None => c.query_row("SELECT id FROM folders WHERE account_id=?1 AND role=?2", params![a, role], |r| r.get(0))?,
                })
            })
            .await
            .unwrap()
    }
    async fn ids_in(&self, folder: i64, n: usize) -> Vec<i64> {
        let rows = call(&self.ctx, "mail.listMessages", json!({ "source": { "folderId": folder }, "limit": n })).await;
        rows["rows"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect()
    }
    async fn labels(&self, id: i64) -> Vec<String> {
        let mut v: Vec<String> = self
            .ctx
            .db
            .read(move |c| Ok(c.query_row("SELECT label_ids FROM messages WHERE id=?1", [id], |r| r.get::<_, String>(0))?))
            .await
            .unwrap()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        v.sort();
        v
    }
    async fn remote(&self, id: i64) -> Vec<String> {
        // wait for the background push
        for _ in 0..100 {
            if crate::sync::pending::count(&self.ctx.db, self.account, "mail").await.unwrap() == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let gid: String =
            self.ctx.db.read(move |c| Ok(c.query_row("SELECT gmail_id FROM messages WHERE id=?1", [id], |r| r.get(0))?)).await.unwrap();
        let mut v = self.api.gmail_get(&gid, "minimal", &[]).await.unwrap().label_ids;
        v.sort();
        v
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mail_move_archive_delete_undo_restore_labels() {
    let t = mail_setup("undo-mail+seed@duvz.demo").await;
    let inbox = t.folder("inbox", None).await;
    let investors = t.folder("", Some("Investors")).await;
    let ids = t.ids_in(inbox, 3).await;
    let before: Vec<Vec<String>> = futures::future::join_all(ids.iter().map(|i| t.labels(*i))).await;

    // move → undo → labels back (local and on Gmail) → redo
    run(&t.ctx, "Move 2 items to Investors", "mail.move", json!({ "ids": [ids[0], ids[1]], "folderId": investors, "fromFolderId": inbox }))
        .await;
    assert_ne!(t.labels(ids[0]).await, before[0]);
    call(&t.ctx, "undo.undo", json!({})).await;
    assert_eq!(t.labels(ids[0]).await, before[0]);
    assert_eq!(t.labels(ids[1]).await, before[1]);
    let r = t.remote(ids[0]).await;
    assert!(r.contains(&"INBOX".to_string()), "{r:?}");
    call(&t.ctx, "undo.redo", json!({})).await;
    assert!(!t.labels(ids[0]).await.contains(&"INBOX".to_string()));
    call(&t.ctx, "undo.undo", json!({})).await;

    // archive → undo
    run(&t.ctx, "Archive", "mail.archive", json!({ "ids": [ids[2]], "fromFolderId": inbox })).await;
    assert!(!t.labels(ids[2]).await.contains(&"INBOX".to_string()));
    call(&t.ctx, "undo.undo", json!({})).await;
    assert_eq!(t.labels(ids[2]).await, before[2]);
    assert!(t.remote(ids[2]).await.contains(&"INBOX".to_string()));

    // delete → undo: out of Deleted Items with every label it had
    run(&t.ctx, "Delete", "mail.delete", json!({ "ids": [ids[0]] })).await;
    assert!(t.labels(ids[0]).await.contains(&"TRASH".to_string()));
    assert!(t.remote(ids[0]).await.contains(&"TRASH".to_string()));
    call(&t.ctx, "undo.undo", json!({})).await;
    assert_eq!(t.labels(ids[0]).await, before[0]);
    let r = t.remote(ids[0]).await;
    assert!(!r.contains(&"TRASH".to_string()) && r.contains(&"INBOX".to_string()), "{r:?}");
    let loc: i64 =
        t.ctx.db.read(move |c| Ok(c.query_row("SELECT location FROM messages WHERE id=?1", [ids[0]], |r| r.get(0))?)).await.unwrap();
    assert_eq!(loc, 0);

    // deleting from Deleted Items is permanent: no entry
    let trash = t.folder("trash", None).await;
    let in_trash = t.ids_in(trash, 1).await;
    let n = t.ctx.undo.peek().undo_count;
    run(&t.ctx, "Delete", "mail.delete", json!({ "ids": in_trash })).await;
    assert_eq!(t.ctx.undo.peek().undo_count, n, "permanent delete is not undoable");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mail_mark_read_flag_and_mark_all_read_restore_each_message() {
    let t = mail_setup("undo-read+seed@duvz.demo").await;
    let inbox = t.folder("inbox", None).await;
    let ids = t.ids_in(inbox, 6).await;
    let unread = |l: &Vec<String>| l.contains(&"UNREAD".to_string());
    call(&t.ctx, "mail.markRead", json!({ "ids": [ids[0], ids[2]], "read": true })).await;
    call(&t.ctx, "mail.markRead", json!({ "ids": [ids[1], ids[3]], "read": false })).await;
    let before: Vec<bool> = futures::future::join_all(ids.iter().map(|i| t.labels(*i))).await.iter().map(unread).collect();
    assert!(before.iter().any(|x| *x) && before.iter().any(|x| !*x), "mixed read state in the seed: {before:?}");
    run(&t.ctx, "Mark as Read", "mail.markRead", json!({ "ids": ids, "read": true })).await;
    for i in &ids {
        assert!(!unread(&t.labels(*i).await));
    }
    call(&t.ctx, "undo.undo", json!({})).await;
    let now: Vec<bool> = futures::future::join_all(ids.iter().map(|i| t.labels(*i))).await.iter().map(unread).collect();
    assert_eq!(now, before, "each message gets its own previous state");

    // flag with a due date → undo
    let id0 = ids[0];
    let flag_of = move |c: &rusqlite::Connection| -> anyhow::Result<(Option<String>, Option<String>)> {
        Ok(c.query_row("SELECT flag_status, flag_due FROM messages WHERE id=?1", [id0], |r| Ok((r.get(0)?, r.get(1)?)))?)
    };
    let flag_before = t.ctx.db.read(flag_of).await.unwrap();
    run(&t.ctx, "Flag", "mail.setFlag", json!({ "ids": [ids[0]], "flag": "custom", "due": "2026-12-01" })).await;
    call(&t.ctx, "undo.undo", json!({})).await;
    assert_eq!(t.ctx.db.read(flag_of).await.unwrap(), flag_before);

    // mark all as read → undo restores the unread ones
    let unread_before: i64 =
        t.ctx.db.read(move |c| Ok(c.query_row("SELECT unread_count FROM folders WHERE id=?1", [inbox], |r| r.get(0))?)).await.unwrap();
    assert!(unread_before > 0);
    run(&t.ctx, "Mark All as Read", "mail.markAllRead", json!({ "folderId": inbox })).await;
    call(&t.ctx, "undo.undo", json!({})).await;
    let unread_after: i64 =
        t.ctx.db.read(move |c| Ok(c.query_row("SELECT unread_count FROM folders WHERE id=?1", [inbox], |r| r.get(0))?)).await.unwrap();
    assert_eq!(unread_after, unread_before);

    // junk → undo
    run(&t.ctx, "Junk", "mail.markJunk", json!({ "ids": [ids[1]] })).await;
    assert!(t.labels(ids[1]).await.contains(&"SPAM".to_string()));
    call(&t.ctx, "undo.undo", json!({})).await;
    assert!(!t.labels(ids[1]).await.contains(&"SPAM".to_string()));
    assert!(t.labels(ids[1]).await.contains(&"INBOX".to_string()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn folder_create_and_rename_undo() {
    let t = mail_setup("undo-folder+seed@duvz.demo").await;
    let id = run(&t.ctx, "New Folder", "mail.createFolder", json!({ "accountId": t.account, "name": "Undo Me" })).await.as_i64().unwrap();
    call(&t.ctx, "undo.undo", json!({})).await;
    let n: i64 = t.ctx.db.read(move |c| Ok(c.query_row("SELECT COUNT(*) FROM folders WHERE id=?1", [id], |r| r.get(0))?)).await.unwrap();
    assert_eq!(n, 0, "the empty folder is gone");
    call(&t.ctx, "undo.redo", json!({})).await;
    let fid = t.folder("", Some("Undo Me")).await;
    run(&t.ctx, "Rename", "mail.renameFolder", json!({ "folderId": fid, "name": "Renamed" })).await;
    let fid2 = t.folder("", Some("Renamed")).await;
    call(&t.ctx, "undo.undo", json!({})).await;
    let back = t.folder("", Some("Undo Me")).await;
    assert_eq!(back, fid2);
    // a folder dragged into the Inbox goes back to the top of the mailbox
    let inbox = t.folder("inbox", None).await;
    run(&t.ctx, "Move Folder", "mail.moveFolder", json!({ "folderId": back, "parentId": inbox })).await;
    let path = |id: i64| t.ctx.db.read(move |c| Ok(c.query_row("SELECT path FROM folders WHERE id=?1", [id], |r| r.get::<_, String>(0))?));
    assert_eq!(path(back).await.unwrap(), "Inbox/Undo Me");
    call(&t.ctx, "undo.undo", json!({})).await;
    assert_eq!(path(back).await.unwrap(), "Undo Me");
    call(&t.ctx, "undo.redo", json!({})).await;
    assert_eq!(path(back).await.unwrap(), "Inbox/Undo Me");
    // with mail in it, undoing the creation is refused (and the entry dropped)
    t.ctx.undo.clear();
    let id = run(&t.ctx, "New Folder", "mail.createFolder", json!({ "accountId": t.account, "name": "Keep Me" })).await.as_i64().unwrap();
    let inbox = t.folder("inbox", None).await;
    let m = t.ids_in(inbox, 1).await;
    call(&t.ctx, "mail.copy", json!({ "ids": m, "folderId": id })).await;
    let e = rpc::build().call(t.ctx.clone(), "undo.undo", json!({})).await.unwrap_err();
    assert!(format!("{e:#}").contains("isn't empty"), "{e:#}");
    assert_eq!(t.ctx.undo.peek().undo_count, 0);
}

// ---- calendar / contacts / tasks ---------------------------------------------------------------

async fn pim_setup(email: &str) -> (Arc<AppCtx>, crate::db::tempdir::TempDirGuard, i64) {
    let (ctx, g) = AppCtx::for_test().unwrap();
    let a = accounts::upsert(&ctx.db, email, "Kenny Green", "demo").await.unwrap();
    let api = ctx.api(a.id).await.unwrap();
    crate::calendar::sync::sync_account(&ctx, &api).await.unwrap();
    crate::contacts::sync::sync_account(&ctx, &api).await.unwrap();
    crate::tasks::sync::sync_account(&ctx, &api).await.unwrap();
    (ctx, g, a.id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn calendar_move_delete_categories_undo() {
    let (ctx, _g, _a) = pim_setup("undo-cal@duvz.demo").await;
    let now = crate::db::util::now_ms();
    let ev = call(
        &ctx,
        "calendar.createEvent",
        json!({ "summary": "Undo test", "start": now + 86_400_000, "end": now + 86_400_000 + 3_600_000, "categories": [] }),
    )
    .await;
    let id = ev["id"].as_i64().unwrap();
    // move (drag) → undo → back at the old time
    run(&ctx, "Move", "calendar.updateEvent", json!({ "id": id, "start": now + 2 * 86_400_000, "end": now + 2 * 86_400_000 + 3_600_000 }))
        .await;
    call(&ctx, "undo.undo", json!({})).await;
    let d = call(&ctx, "calendar.getEvent", json!({ "id": id })).await;
    assert_eq!(d["start"], ev["start"]);
    assert_eq!(d["end"], ev["end"]);
    call(&ctx, "undo.redo", json!({})).await;
    assert_eq!(call(&ctx, "calendar.getEvent", json!({ "id": id })).await["start"], now + 2 * 86_400_000);
    // edit of the subject only patches the subject back
    run(&ctx, "Edit", "calendar.updateEvent", json!({ "id": id, "summary": "Renamed", "location": "Tel Aviv" })).await;
    call(&ctx, "undo.undo", json!({})).await;
    let d = call(&ctx, "calendar.getEvent", json!({ "id": id })).await;
    assert_eq!((d["summary"].as_str(), d["location"].as_str()), (Some("Undo test"), Some("")));
    // categories → undo
    run(&ctx, "Categorize", "calendar.setEventCategories", json!({ "ids": [id], "categories": ["Red Category"] })).await;
    call(&ctx, "undo.undo", json!({})).await;
    assert_eq!(call(&ctx, "calendar.getEvent", json!({ "id": id })).await["categories"], json!([]));
    // delete → undo recreates it; redo deletes the new one
    run(&ctx, "Delete", "calendar.deleteEvent", json!({ "id": id })).await;
    call(&ctx, "undo.undo", json!({})).await;
    let n: i64 =
        ctx.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM events WHERE summary='Undo test'", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(n, 1, "re-created");
    call(&ctx, "undo.redo", json!({})).await;
    let n: i64 =
        ctx.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM events WHERE summary='Undo test'", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(n, 0, "redo deletes the re-created item");
    call(&ctx, "undo.undo", json!({})).await;
    // drag-copy → undo removes the copy
    let id: i64 = ctx.db.read(|c| Ok(c.query_row("SELECT id FROM events WHERE summary='Undo test'", [], |r| r.get(0))?)).await.unwrap();
    run(&ctx, "Copy", "calendar.copyEvent", json!({ "id": id, "start": now + 5 * 86_400_000, "end": now + 5 * 86_400_000 + 3_600_000 }))
        .await;
    call(&ctx, "undo.undo", json!({})).await;
    let n: i64 =
        ctx.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM events WHERE summary='Undo test'", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(n, 1);
    // a meeting with attendees: editing it sends updates → no entry
    let n0 = ctx.undo.peek().undo_count;
    let m: i64 = ctx
        .db
        .read(|c| Ok(c.query_row("SELECT id FROM events WHERE attendees LIKE '%@%' AND organizer_self=1 AND is_master=0 AND recurring_event_id IS NULL LIMIT 1", [], |r| r.get(0))?))
        .await
        .unwrap();
    run(&ctx, "Edit", "calendar.updateEvent", json!({ "id": m, "location": "Room 2", "sendUpdates": false })).await;
    assert_eq!(ctx.undo.peek().undo_count, n0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn contact_delete_and_edit_undo() {
    let (ctx, _g, _a) = pim_setup("undo-contacts@duvz.demo").await;
    let list = call(&ctx, "contacts.list", json!({})).await;
    let c = list.as_array().unwrap().iter().find(|c| !c["emails"].as_array().unwrap().is_empty()).unwrap().clone();
    let id = c["id"].as_i64().unwrap();
    // edit → undo
    let mut edited = c.clone();
    edited["jobTitle"] = json!("Chief Undo Officer");
    run(&ctx, "Edit contact", "contacts.update", json!({ "id": id, "contact": edited })).await;
    call(&ctx, "undo.undo", json!({})).await;
    assert_eq!(call(&ctx, "contacts.get", json!({ "id": id })).await["jobTitle"], c["jobTitle"]);
    // delete → undo re-creates it (new id), redo deletes that one
    run(&ctx, "Delete", "contacts.delete", json!({ "ids": [id] })).await;
    call(&ctx, "undo.undo", json!({})).await;
    let back = call(&ctx, "contacts.list", json!({})).await;
    let back = back.as_array().unwrap();
    let again = back.iter().find(|x| x["displayName"] == c["displayName"]).unwrap_or_else(|| panic!("re-created: {back:?}"));
    assert_ne!(again["id"], c["id"]);
    assert_eq!(again["company"], c["company"]);
    assert_eq!(again["phones"], c["phones"]);
    call(&ctx, "undo.redo", json!({})).await;
    assert!(rpc::build().call(ctx.clone(), "contacts.get", json!({ "id": again["id"] })).await.is_err(), "redo deleted it again");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_complete_edit_delete_create_undo() {
    let (ctx, _g, _a) = pim_setup("undo-tasks@duvz.demo").await;
    let list = call(&ctx, "tasks.list", json!({})).await;
    let t = list.as_array().unwrap().iter().find(|t| t["complete"] == false && t["parentId"].is_null()).unwrap().clone();
    let id = t["id"].as_i64().unwrap();
    run(&ctx, "Mark Complete", "tasks.setComplete", json!({ "ids": [id], "complete": true })).await;
    assert_eq!(call(&ctx, "tasks.get", json!({ "id": id })).await["complete"], true);
    call(&ctx, "undo.undo", json!({})).await;
    let now = call(&ctx, "tasks.get", json!({ "id": id })).await;
    assert_eq!(
        (now["complete"].clone(), now["status"].clone(), now["percent"].clone()),
        (t["complete"].clone(), t["status"].clone(), t["percent"].clone())
    );
    call(&ctx, "undo.redo", json!({})).await;
    assert_eq!(call(&ctx, "tasks.get", json!({ "id": id })).await["complete"], true);
    call(&ctx, "undo.undo", json!({})).await;
    // edit → undo
    run(&ctx, "Edit", "tasks.update", json!({ "id": id, "title": "Changed", "due": null })).await;
    call(&ctx, "undo.undo", json!({})).await;
    let now = call(&ctx, "tasks.get", json!({ "id": id })).await;
    assert_eq!((now["title"].clone(), now["due"].clone()), (t["title"].clone(), t["due"].clone()));
    // delete → undo re-creates in place; redo deletes it again
    let pos_before: Vec<String> = call(&ctx, "tasks.list", json!({ "listId": t["listId"] }))
        .await
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["parentId"].is_null())
        .map(|x| x["title"].as_str().unwrap().to_string())
        .collect();
    run(&ctx, "Delete", "tasks.delete", json!({ "ids": [id] })).await;
    call(&ctx, "undo.undo", json!({})).await;
    let pos_after: Vec<String> = call(&ctx, "tasks.list", json!({ "listId": t["listId"] }))
        .await
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["parentId"].is_null())
        .map(|x| x["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(pos_after, pos_before, "back in its old place");
    call(&ctx, "undo.redo", json!({})).await;
    assert!(!call(&ctx, "tasks.list", json!({ "listId": t["listId"] })).await.as_array().unwrap().iter().any(|x| x["title"] == t["title"]));
    // new task → undo deletes it
    let nt = run(&ctx, "New Task", "tasks.create", json!({ "title": "Fresh one" })).await;
    call(&ctx, "undo.undo", json!({})).await;
    assert!(rpc::build().call(ctx.clone(), "tasks.get", json!({ "id": nt["id"] })).await.is_err());
}

#[tokio::test]
async fn run_many_is_one_step_with_separate_variables() {
    let (ctx, _g) = ctx().await;
    let r = call(
        &ctx,
        "undo.runMany",
        json!({ "label": "Two settings", "calls": [
            { "method": "settings.set", "params": { "key": "mail.pollSeconds", "value": 5 } },
            { "method": "settings.set", "params": { "key": "mail.prefetchDays", "value": 9 } },
        ] }),
    )
    .await;
    assert_eq!(r.as_array().unwrap().len(), 2);
    assert_eq!(ctx.undo.peek().undo_count, 1);
    call(&ctx, "undo.undo", json!({})).await;
    assert_eq!(setting(&ctx, "mail.pollSeconds").await, Value::Null);
    assert_eq!(setting(&ctx, "mail.prefetchDays").await, Value::Null);
    call(&ctx, "undo.redo", json!({})).await;
    assert_eq!(setting(&ctx, "mail.prefetchDays").await, 9);
    // variables of each call are kept apart
    let mut rec = super::inverse::Recorded {
        undo: vec![Op::new("x.y", json!({ "id": "$new.id", "other": "$keep" }))],
        redo: vec![Op::new("x.z", json!({})).saving("new")],
        vars: [("new".to_string(), json!({ "id": 1 }))].into_iter().collect(),
    };
    super::prefix_vars(&mut rec, "k3_");
    assert_eq!(rec.undo[0].params, json!({ "id": "$k3_new.id", "other": "$keep" }));
    assert_eq!(rec.redo[0].save.as_deref(), Some("k3_new"));
    assert!(rec.vars.contains_key("k3_new"));
}

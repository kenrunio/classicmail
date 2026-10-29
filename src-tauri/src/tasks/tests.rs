//! Integration tests: sync round trips against the in-process fake Google Tasks server.

use std::sync::Arc;

use serde_json::{Value, json};

use super::store::{TaskDto, TaskListDto};
use crate::{
    accounts,
    ctx::AppCtx,
    db::tempdir::TempDirGuard,
    google::{
        Api,
        tasks::{self as gt, ListOpts},
    },
    rpc,
};

struct T {
    ctx: Arc<AppCtx>,
    _g: TempDirGuard,
    api: Api,
    reg: rpc::Registry,
}

async fn setup() -> T {
    let (ctx, g) = AppCtx::for_test().unwrap();
    let acct = accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny Green", "demo").await.unwrap();
    let api = ctx.api(acct.id).await.unwrap();
    T { ctx, _g: g, api, reg: rpc::build() }
}

impl T {
    async fn call(&self, m: &str, p: Value) -> Value {
        self.reg.call(self.ctx.clone(), m, p).await.unwrap_or_else(|e| panic!("{m}: {e:#}"))
    }
    async fn sync(&self) {
        super::sync::sync_account(&self.ctx, &self.api).await.unwrap();
    }
    async fn lists(&self) -> Vec<TaskListDto> {
        let v = self.call("tasks.lists", json!({})).await;
        v.as_array()
            .unwrap()
            .iter()
            .map(|l| TaskListDto {
                id: l["id"].as_i64().unwrap(),
                account_id: l["accountId"].as_i64().unwrap(),
                account_email: l["accountEmail"].as_str().unwrap().into(),
                account_name: String::new(),
                google_id: l["googleId"].as_str().map(str::to_string),
                title: l["title"].as_str().unwrap().into(),
                is_default: l["isDefault"].as_bool().unwrap(),
                position: 0,
                active_count: l["activeCount"].as_i64().unwrap(),
                total_count: l["totalCount"].as_i64().unwrap(),
            })
            .collect()
    }
    async fn tasks(&self, p: Value) -> Vec<Value> {
        self.call("tasks.list", p).await.as_array().unwrap().clone()
    }
    async fn task(&self, id: i64) -> Value {
        self.call("tasks.get", json!({ "id": id })).await
    }
    async fn remote(&self, list_gid: &str) -> Vec<gt::Task> {
        gt::list_tasks(&self.api, list_gid, &ListOpts { show_completed: true, show_hidden: true, ..Default::default() }).await.unwrap()
    }
    async fn pending(&self) -> i64 {
        crate::sync::pending::count(&self.ctx.db, self.api.account_id, "tasks").await.unwrap()
    }
}

fn title(v: &Value) -> &str {
    v["title"].as_str().unwrap()
}

#[tokio::test]
async fn full_sync_pulls_seeded_lists_and_tasks() {
    let t = setup().await;
    t.sync().await;
    let lists = t.lists().await;
    assert_eq!(lists.len(), 2);
    assert_eq!(lists[0].title, "My Tasks");
    assert!(lists[0].is_default);
    assert_eq!(lists[1].title, "Client follow-ups");
    let all = t.tasks(json!({ "view": "simple" })).await;
    assert!(all.len() >= 30, "{} tasks", all.len());
    // subtasks are linked to their parent and listed right after it
    let i = all.iter().position(|x| title(x) == "Prepare Q&A script for Orion earnings call").unwrap();
    let pid = all[i]["id"].as_i64().unwrap();
    assert_eq!(all[i + 1]["parentId"].as_i64(), Some(pid));
    assert_eq!(all[i + 2]["parentId"].as_i64(), Some(pid));
    assert!(all.iter().any(|x| x["complete"].as_bool().unwrap() && x["percent"] == 100));
    assert!(all.iter().any(|x| title(x).contains("רו\"ח")));
    // views
    let overdue = t.tasks(json!({ "view": "overdue" })).await;
    assert!(!overdue.is_empty());
    assert!(overdue.iter().all(|x| !x["complete"].as_bool().unwrap()));
    let completed = t.tasks(json!({ "view": "completed" })).await;
    assert!(completed.iter().all(|x| x["complete"].as_bool().unwrap()));
    let next7 = t.tasks(json!({ "view": "next7" })).await;
    assert!(next7.iter().any(|x| title(x).starts_with("Book roadshow flights")));
    assert!(!next7.iter().any(|x| title(x).starts_with("Plan 2027")));
    let found = t.tasks(json!({ "view": "simple", "search": "orion" })).await;
    assert!(found.len() >= 3);
    // sync state for incremental pulls was recorded per list
    let gid = lists[0].google_id.clone().unwrap();
    let key = super::sync::updated_min_key(&gid);
    let acct = t.api.account_id;
    let mark = t.ctx.db.read(move |c| crate::db::util::get_sync_state(c, acct, &key)).await.unwrap();
    assert!(mark.is_some());
    // a second sync with nothing new changes nothing
    let before: Vec<Value> = t.tasks(json!({ "view": "simple" })).await;
    t.sync().await;
    assert_eq!(before, t.tasks(json!({ "view": "simple" })).await);
}

#[tokio::test]
async fn create_update_complete_delete_round_trip() {
    let t = setup().await;
    t.sync().await;
    let lists = t.lists().await;
    let gid = lists[0].google_id.clone().unwrap();

    // create (default list, top of the list like Google)
    let created = t.call("tasks.create", json!({ "title": "Send Q4 deck to Orion", "due": "2026-11-02", "notes": "v2" })).await;
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["listId"].as_i64(), Some(lists[0].id));
    assert!(created["googleId"].is_null());
    assert_eq!(t.pending().await, 1);
    t.sync().await;
    assert_eq!(t.pending().await, 0);
    let local = t.task(id).await;
    let tgid = local["googleId"].as_str().unwrap().to_string();
    let remote = gt::get_task(&t.api, &gid, &tgid).await.unwrap();
    assert_eq!(remote.title.as_deref(), Some("Send Q4 deck to Orion"));
    assert_eq!(remote.notes.as_deref(), Some("v2"));
    assert_eq!(remote.due.as_deref(), Some("2026-11-02T00:00:00.000Z"));
    let order = t.remote(&gid).await;
    assert_eq!(order[0].id, tgid, "new tasks go to the top");

    // update Google + Outlook-only fields
    t.call(
        "tasks.update",
        json!({ "id": id, "title": "Send Q4 deck to Orion (final)", "importance": "high", "percent": 50,
                "categories": ["Red Category"], "start": "2026-10-20", "reminderAt": 1_800_000_000_000i64, "isPrivate": true }),
    )
    .await;
    let local = t.task(id).await;
    assert_eq!(local["status"], "inProgress");
    t.sync().await;
    let remote = gt::get_task(&t.api, &gid, &tgid).await.unwrap();
    assert_eq!(remote.title.as_deref(), Some("Send Q4 deck to Orion (final)"));

    // a local-only change queues nothing
    t.call("tasks.update", json!({ "id": id, "importance": "low" })).await;
    assert_eq!(t.pending().await, 0);
    t.call("tasks.update", json!({ "id": id, "importance": "high" })).await;

    // change it on Google (another client); the pull brings it in and keeps the local fields
    gt::patch_task(&t.api, &gid, &tgid, json!({ "title": "Edited on the web", "due": "2026-11-05T00:00:00.000Z" })).await.unwrap();
    t.sync().await;
    let local = t.task(id).await;
    assert_eq!(title(&local), "Edited on the web");
    assert_eq!(local["due"], "2026-11-05");
    assert_eq!(local["importance"], "high");
    assert_eq!(local["percent"], 50);
    assert_eq!(local["status"], "inProgress");
    assert_eq!(local["start"], "2026-10-20");
    assert_eq!(local["categories"], json!(["Red Category"]));
    assert_eq!(local["reminderAt"], 1_800_000_000_000i64);
    assert_eq!(local["isPrivate"], true);

    // complete
    t.call("tasks.setComplete", json!({ "ids": [id], "complete": true })).await;
    let local = t.task(id).await;
    assert_eq!(local["status"], "completed");
    assert_eq!(local["percent"], 100);
    t.sync().await;
    let remote = gt::get_task(&t.api, &gid, &tgid).await.unwrap();
    assert_eq!(remote.status.as_deref(), Some("completed"));
    assert!(remote.completed.is_some());
    // completed on Google → local; un-completed on Google → local back to Not Started / 0%
    gt::patch_task(&t.api, &gid, &tgid, json!({ "status": "needsAction" })).await.unwrap();
    t.sync().await;
    let local = t.task(id).await;
    assert_eq!(local["complete"], false);
    assert_eq!(local["status"], "notStarted");
    assert_eq!(local["percent"], 0);
    assert_eq!(local["importance"], "high");

    // delete locally → gone on Google
    t.call("tasks.delete", json!({ "ids": [id] })).await;
    t.sync().await;
    assert!(!t.remote(&gid).await.iter().any(|x| x.id == tgid));
    assert_eq!(t.pending().await, 0);

    // delete on Google → gone locally (incremental pull sees the tombstone)
    let victim = t.tasks(json!({ "listId": lists[0].id })).await.into_iter().find(|x| title(x) == "Expense report – September").unwrap();
    gt::delete_task(&t.api, &gid, victim["googleId"].as_str().unwrap()).await.unwrap();
    t.sync().await;
    assert!(t.reg.call(t.ctx.clone(), "tasks.get", json!({ "id": victim["id"] })).await.is_err());

    // created on Google → appears locally
    gt::insert_task(&t.api, &gid, json!({ "title": "Added from phone" }), None, None).await.unwrap();
    t.sync().await;
    assert!(t.tasks(json!({ "view": "active" })).await.iter().any(|x| title(x) == "Added from phone"));
}

#[tokio::test]
async fn incremental_pull_uses_updated_min() {
    let t = setup().await;
    t.sync().await;
    let lists = t.lists().await;
    let gid = lists[0].google_id.clone().unwrap();
    let all = t.tasks(json!({ "listId": lists[0].id })).await;
    let untouched = all.iter().find(|x| title(x) == "Renew Bloomberg terminal subscription").unwrap()["id"].as_i64().unwrap();
    let edited = all.iter().find(|x| title(x) == "Update CRM with roadshow feedback").unwrap().clone();
    // backdate the stored mark so the overlap window doesn't re-fetch the seeded tasks
    let acct = t.api.account_id;
    let key = super::sync::updated_min_key(&gid);
    t.ctx
        .db
        .write(move |c| {
            crate::db::util::set_sync_state(c, acct, &key, Some(&gt::fmt_ts(crate::db::util::now_ms() - 1000)))?;
            // simulate a stale local copy of a task nobody changed remotely
            c.execute("UPDATE tasks SET title = 'stale local copy', etag = 'x' WHERE id = ?1", [untouched])?;
            Ok(())
        })
        .await
        .unwrap();
    gt::patch_task(&t.api, &gid, edited["googleId"].as_str().unwrap(), json!({ "title": "Update CRM (done half)" })).await.unwrap();
    t.sync().await;
    // the changed task came in, the unchanged one wasn't asked for (so the stale copy survives)
    assert_eq!(title(&t.task(edited["id"].as_i64().unwrap()).await), "Update CRM (done half)");
    assert_eq!(title(&t.task(untouched).await), "stale local copy");
    // a full resync (no mark) repairs it
    let key = super::sync::updated_min_key(&gid);
    t.ctx.db.write(move |c| crate::db::util::set_sync_state(c, acct, &key, None)).await.unwrap();
    t.sync().await;
    assert_eq!(title(&t.task(untouched).await), "Renew Bloomberg terminal subscription");
}

#[tokio::test]
async fn local_changes_are_not_clobbered_before_push() {
    let t = setup().await;
    t.sync().await;
    let lists = t.lists().await;
    let gid = lists[0].google_id.clone().unwrap();
    let x = t
        .tasks(json!({ "listId": lists[0].id }))
        .await
        .into_iter()
        .find(|x| title(x) == "Read sell-side initiation report on Corvina Medical")
        .unwrap();
    let id = x["id"].as_i64().unwrap();
    t.call("tasks.update", json!({ "id": id, "title": "Local edit" })).await;
    // remote changes too, and we pull without pushing first
    gt::patch_task(&t.api, &gid, x["googleId"].as_str().unwrap(), json!({ "title": "Remote edit" })).await.unwrap();
    super::sync::pull(&t.ctx, &t.api).await.unwrap();
    assert_eq!(title(&t.task(id).await), "Local edit");
    // next full cycle: our edit wins (last writer)
    t.sync().await;
    let remote = gt::get_task(&t.api, &gid, x["googleId"].as_str().unwrap()).await.unwrap();
    assert_eq!(remote.title.as_deref(), Some("Local edit"));
}

#[tokio::test]
async fn move_reorders_and_changes_list() {
    let t = setup().await;
    t.sync().await;
    let lists = t.lists().await;
    let (g0, g1) = (lists[0].google_id.clone().unwrap(), lists[1].google_id.clone().unwrap());
    let tops: Vec<Value> = t.tasks(json!({ "listId": lists[0].id })).await.into_iter().filter(|x| x["parentId"].is_null()).collect();
    let (a, b, c) = (tops[0].clone(), tops[1].clone(), tops[2].clone());
    // move c between a and b
    t.call("tasks.move", json!({ "ids": [c["id"]], "previousId": a["id"] })).await;
    let local: Vec<i64> = t
        .tasks(json!({ "listId": lists[0].id }))
        .await
        .into_iter()
        .filter(|x| x["parentId"].is_null())
        .map(|x| x["id"].as_i64().unwrap())
        .take(3)
        .collect();
    assert_eq!(local, vec![a["id"].as_i64().unwrap(), c["id"].as_i64().unwrap(), b["id"].as_i64().unwrap()]);
    t.sync().await;
    let remote: Vec<String> = t.remote(&g0).await.into_iter().filter(|x| x.parent.is_none()).map(|x| x.id).take(3).collect();
    let gid = |v: &Value| v["googleId"].as_str().unwrap().to_string();
    assert_eq!(remote, vec![gid(&a), gid(&c), gid(&b)]);
    // the order survives a full resync
    let acct = t.api.account_id;
    t.ctx.db.write(move |c| Ok(c.execute("DELETE FROM sync_state WHERE account_id = ?1", [acct])?)).await.unwrap();
    t.sync().await;
    let again: Vec<i64> = t
        .tasks(json!({ "listId": lists[0].id }))
        .await
        .into_iter()
        .filter(|x| x["parentId"].is_null())
        .map(|x| x["id"].as_i64().unwrap())
        .take(3)
        .collect();
    assert_eq!(again, local);

    // make b a subtask of a
    t.call("tasks.move", json!({ "ids": [b["id"]], "parentId": a["id"] })).await;
    t.sync().await;
    let rb = gt::get_task(&t.api, &g0, &gid(&b)).await.unwrap();
    assert_eq!(rb.parent.as_deref(), Some(gid(&a).as_str()));

    // move a task (with its subtasks) to the other list
    t.call("tasks.move", json!({ "ids": [a["id"]], "listId": lists[1].id })).await;
    let moved = t.task(a["id"].as_i64().unwrap()).await;
    assert_eq!(moved["listId"].as_i64(), Some(lists[1].id));
    assert_eq!(t.task(b["id"].as_i64().unwrap()).await["listId"].as_i64(), Some(lists[1].id));
    t.sync().await;
    let r1 = t.remote(&g1).await;
    assert!(r1.iter().any(|x| x.id == gid(&a)));
    assert!(r1.iter().any(|x| x.id == gid(&b) && x.parent.as_deref() == Some(gid(&a).as_str())));
    assert!(!t.remote(&g0).await.iter().any(|x| x.id == gid(&a)));
    // still in the right list after a full pull
    t.ctx.db.write(move |c| Ok(c.execute("DELETE FROM sync_state WHERE account_id = ?1", [acct])?)).await.unwrap();
    t.sync().await;
    assert_eq!(t.task(a["id"].as_i64().unwrap()).await["listId"].as_i64(), Some(lists[1].id));
}

#[tokio::test]
async fn task_lists_round_trip() {
    let t = setup().await;
    t.sync().await;
    let l = t.call("tasks.createList", json!({ "title": "Roadshow – November" })).await;
    let lid = l["id"].as_i64().unwrap();
    let x = t.call("tasks.create", json!({ "listId": lid, "title": "Book Hotel in NYC" })).await;
    t.sync().await;
    let remote = gt::list_tasklists(&t.api).await.unwrap();
    let rl = remote.iter().find(|r| r.title == "Roadshow – November").expect("list pushed");
    assert_eq!(t.remote(&rl.id).await.len(), 1);
    assert!(t.task(x["id"].as_i64().unwrap()).await["googleId"].is_string());

    t.call("tasks.renameList", json!({ "id": lid, "title": "Roadshow – Dec" })).await;
    t.sync().await;
    assert!(gt::list_tasklists(&t.api).await.unwrap().iter().any(|r| r.title == "Roadshow – Dec"));

    // renamed on Google → local
    gt::patch_tasklist(&t.api, &rl.id, "Roadshow (web)").await.unwrap();
    t.sync().await;
    assert!(t.lists().await.iter().any(|l| l.title == "Roadshow (web)"));

    // the default list can't be deleted
    let def = t.lists().await[0].id;
    assert!(t.reg.call(t.ctx.clone(), "tasks.deleteList", json!({ "id": def })).await.is_err());

    t.call("tasks.deleteList", json!({ "id": lid })).await;
    t.sync().await;
    assert!(!gt::list_tasklists(&t.api).await.unwrap().iter().any(|r| r.id == rl.id));
    assert_eq!(t.lists().await.len(), 2);

    // a list created on Google appears; deleted on Google disappears
    let web = gt::insert_tasklist(&t.api, "From the web").await.unwrap();
    gt::insert_task(&t.api, &web.id, json!({ "title": "x" }), None, None).await.unwrap();
    t.sync().await;
    let lists = t.lists().await;
    let wl = lists.iter().find(|l| l.title == "From the web").unwrap();
    assert_eq!(wl.total_count, 1);
    gt::delete_tasklist(&t.api, &web.id).await.unwrap();
    t.sync().await;
    assert!(!t.lists().await.iter().any(|l| l.title == "From the web"));
}

#[tokio::test]
async fn status_and_percent_follow_outlook_rules() {
    let t = setup().await;
    t.sync().await;
    let x = t.call("tasks.create", json!({ "title": "t" })).await;
    let id = x["id"].as_i64().unwrap();
    assert_eq!(x["status"], "notStarted");
    let v = t.call("tasks.update", json!({ "id": id, "percent": 25 })).await;
    assert_eq!(v["status"], "inProgress");
    let v = t.call("tasks.update", json!({ "id": id, "status": "waiting" })).await;
    assert_eq!((v["status"].as_str(), v["percent"].as_i64()), (Some("waiting"), Some(25)));
    let v = t.call("tasks.update", json!({ "id": id, "percent": 100 })).await;
    assert_eq!((v["status"].as_str(), v["complete"].as_bool()), (Some("completed"), Some(true)));
    let v = t.call("tasks.update", json!({ "id": id, "status": "inProgress" })).await;
    assert_eq!((v["complete"].as_bool(), v["percent"].as_i64()), (Some(false), Some(0)));
    let v = t.call("tasks.update", json!({ "id": id, "due": "2026-12-01", "start": "2026-11-28" })).await;
    assert_eq!(v["due"], "2026-12-01");
    let v = t.call("tasks.update", json!({ "id": id, "due": null })).await;
    assert!(v["due"].is_null());
    assert_eq!(v["start"], "2026-11-28");
    assert!(t.reg.call(t.ctx.clone(), "tasks.update", json!({ "id": id, "due": "31/12/2026" })).await.is_err());
    let _: Vec<TaskDto> = Vec::new();
}

/// Tasks completed in Google's apps come back hidden; Google refuses a hidden task as `previous`,
/// so a local move next to one must not use it.
#[tokio::test]
async fn hidden_neighbours_are_not_used_as_previous() {
    let t = setup().await;
    t.sync().await;
    let lists = t.lists().await;
    let g0 = lists[0].google_id.clone().unwrap();
    let tops: Vec<Value> = t.tasks(json!({ "listId": lists[0].id })).await.into_iter().filter(|x| x["parentId"].is_null()).collect();
    let (a, c) = (tops[0].clone(), tops[2].clone());
    // a is completed and cleared on Google (as the Tasks app does)
    let ga = a["googleId"].as_str().unwrap().to_string();
    gt::patch_task(&t.api, &g0, &ga, json!({ "status": "completed" })).await.unwrap();
    let _: Value = t
        .api
        .call(reqwest::Method::POST, &format!("{}/lists/{}/clear", t.api.urls.tasks, g0), &[], crate::google::client::Body::None)
        .await
        .unwrap();
    t.sync().await;
    assert!(t.remote(&g0).await.iter().any(|x| x.id == ga && x.hidden == Some(true)), "showHidden brings it in");
    // move c right after a locally
    t.call("tasks.move", json!({ "ids": [c["id"]], "previousId": a["id"] })).await;
    t.sync().await;
    assert_eq!(t.pending().await, 0, "the move was accepted");
    let st = t.ctx.events.captured().into_iter().filter(|e| e["kind"] == "sync.status" && e["state"] == "error").count();
    assert_eq!(st, 0);
}

/// A task moved to another account takes its subtasks along (order, notes, due date and done
/// state kept); a subtask also in the selection isn't moved twice.
#[tokio::test]
async fn move_to_another_account_keeps_subtasks() {
    let t = setup().await;
    t.sync().await;
    let other = accounts::upsert(&t.ctx.db, "info@duvz.demo", "Duvz Software", "demo").await.unwrap();
    let other_api = t.ctx.api(other.id).await.unwrap();
    super::sync::sync_account(&t.ctx, &other_api).await.unwrap();
    let lists = t.lists().await;
    let src = lists.iter().find(|l| l.account_id == t.api.account_id && l.is_default).unwrap().id;
    let dest = lists.iter().find(|l| l.account_id == other.id && l.is_default).unwrap().id;
    let parent = t.call("tasks.create", json!({ "listId": src, "title": "Roadshow" })).await;
    let pid = parent["id"].as_i64().unwrap();
    let k1 = t
        .call("tasks.create", json!({ "listId": src, "title": "Book flights", "parentId": pid, "notes": "TLV-LHR", "due": "2026-11-02" }))
        .await;
    let k2 = t
        .call("tasks.create", json!({ "listId": src, "title": "Hotel", "parentId": pid, "previousId": k1["id"], "status": "completed" }))
        .await;
    let out = t.call("tasks.move", json!({ "ids": [pid, k2["id"]], "listId": dest })).await;
    let out: Vec<i64> = out.as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
    assert_eq!(out.len(), 2);
    let moved = t.tasks(json!({ "listId": dest })).await;
    let np = moved.iter().find(|x| title(x) == "Roadshow").unwrap();
    let kids: Vec<&Value> = moved.iter().filter(|x| x["parentId"] == np["id"]).collect();
    assert_eq!(kids.iter().map(|x| title(x)).collect::<Vec<_>>(), vec!["Book flights", "Hotel"]);
    assert_eq!(kids[0]["notes"], "TLV-LHR");
    assert_eq!(kids[0]["due"], "2026-11-02");
    assert_eq!(kids[1]["complete"], true);
    assert_eq!(out, vec![np["id"].as_i64().unwrap(), kids[1]["id"].as_i64().unwrap()]);
    let left = t.tasks(json!({ "listId": src })).await;
    assert!(!left.iter().any(|x| ["Roadshow", "Book flights", "Hotel"].contains(&title(x))), "originals deleted");
    // on Google the subtasks sit under the new parent
    super::sync::sync_account(&t.ctx, &other_api).await.unwrap();
    let dest_gid = t.lists().await.into_iter().find(|l| l.id == dest).unwrap().google_id.unwrap();
    let remote =
        gt::list_tasks(&other_api, &dest_gid, &ListOpts { show_completed: true, show_hidden: true, ..Default::default() }).await.unwrap();
    let rp = remote.iter().find(|x| x.title.as_deref() == Some("Roadshow")).unwrap();
    assert_eq!(remote.iter().filter(|x| x.parent.as_deref() == Some(rp.id.as_str())).count(), 2);
}

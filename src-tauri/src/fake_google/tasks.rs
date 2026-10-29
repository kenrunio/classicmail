//! Fake Google Tasks API v1 (see fake_google/mod.rs for conventions).
//!
//! Implements tasklists list/get/insert/patch/update/delete and tasks
//! list/get/insert/patch/update/delete/move/clear with Google's JSON shapes, `maxResults` +
//! `pageToken` paging, `updatedMin`, `showCompleted` / `showHidden` / `showDeleted`, tombstones for
//! deleted tasks, parent/previous ordering (20-digit `position` strings) and etags.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{Shared, User, error, not_found, paginate, seed};
use crate::google::tasks::{date_to_due, due_to_date, fmt_ts, parse_ts};

/// Spacing between sibling positions; new positions are midpoints, renumbered when exhausted.
const GAP: u64 = 1 << 32;

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    pub lists: Vec<FList>,
    /// per-user id / etag counter
    pub counter: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FList {
    pub id: String,
    pub etag: String,
    pub title: String,
    pub updated: i64,
    pub tasks: Vec<FTask>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FTask {
    pub id: String,
    pub etag: String,
    pub title: String,
    pub notes: Option<String>,
    pub status: String,
    /// "YYYY-MM-DD"
    pub due: Option<String>,
    pub completed: Option<i64>,
    pub updated: i64,
    pub parent: Option<String>,
    pub position: u64,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub hidden: bool,
}

impl Store {
    fn next(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }
    fn new_id(&mut self, prefix: &str) -> String {
        let n = self.next();
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{prefix}:{n:010}"))
    }
    fn etag(&mut self) -> String {
        format!("\"{}\"", self.next())
    }
    fn list_idx(&self, id: &str) -> Option<usize> {
        if id == "@default" {
            return if self.lists.is_empty() { None } else { Some(0) };
        }
        self.lists.iter().position(|l| l.id == id)
    }
}

fn now() -> i64 {
    seed::now_ms()
}

// ---- seed -------------------------------------------------------------------------------------

/// (list index, title, notes, due offset in days, completed N days ago, parent index into this table)
type Tpl = (usize, &'static str, &'static str, Option<i64>, Option<i64>, Option<usize>);

const KENNY: &[Tpl] = &[
    (
        0,
        "Send Q3 earnings deck to Orion Technologies",
        "Final version after Noa's comments; include the KPI appendix and the updated guidance slide.",
        Some(0),
        None,
        None,
    ),
    (
        0,
        "Book roadshow flights TLV–JFK for Corvina Medical NDR",
        "Oren + Rachel, business class, return via Boston if the Bluewater meeting is confirmed.",
        Some(1),
        None,
        None,
    ),
    (
        0,
        "Update investor targeting list – US healthcare funds",
        "Use Eastbridge's shareholder ID output; flag funds that hold peers but not Corvina.",
        Some(3),
        None,
        None,
    ),
    (
        0,
        "Draft press release: Sunfield Energy Q3 results",
        "Wait for audited numbers from Tamar. Embargo until 07:00 TLV.",
        Some(-2),
        None,
        None,
    ),
    (0, "Prepare Q&A script for Orion earnings call", "", Some(2), None, None),
    (0, "Collect analyst questions from last call", "", Some(1), None, Some(4)),
    (0, "Review guidance wording with CFO", "Noa wants to soften the H2 margin language.", Some(2), None, Some(4)),
    (0, "Confirm hotel for London investor conference", "Northgate block booking expires Friday.", Some(6), None, None),
    (0, "Expense report – September", "", Some(-5), None, None),
    (0, "Renew Bloomberg terminal subscription", "", Some(12), None, None),
    (0, "Review shareholder register analysis from Eastbridge", "", Some(8), None, None),
    (0, "לתאם פגישה עם רו\"ח לגבי הדוח הרבעוני", "לבדוק זמינות ביום ראשון או שני בבוקר.", Some(4), None, None),
    (0, "Call Tom re Birchwood Partners retainer proposal", "", Some(0), None, None),
    (0, "Write follow-up notes from Tel Aviv Tech Conference", "Meetings: Harbor Capital, Meridian, Rhein AM.", Some(-1), None, None),
    (0, "Update CRM with roadshow feedback", "", None, None, None),
    (0, "Read sell-side initiation report on Corvina Medical", "Summit Securities, Buy, TP $34.", None, None, None),
    (0, "Plan 2027 IR calendar for all clients", "Earnings dates, conferences, NDR windows.", Some(25), None, None),
    (0, "Send investor day invitations", "", Some(-3), Some(2), None),
    (0, "Book meeting room for Juniper Ventures visit", "", Some(-1), Some(1), None),
    (0, "Submit Q2 IR activity report to Orion board", "", Some(-12), Some(10), None),
    (0, "Distribute conference call dial-in details", "", Some(0), Some(0), None),
    (1, "Follow up with Sarah Cohen (Harbor Capital) on Orion model", "She asked for the segment breakdown.", Some(0), None, None),
    (1, "Send Corvina Medical fact sheet to James Whitaker", "", Some(1), None, None),
    (1, "Schedule call with Yael Friedman – Tamar Investments", "", Some(5), None, None),
    (1, "Thomas Meyer: send German translation of investor presentation", "", Some(-3), None, None),
    (1, "Check in with Michael Brennan re London NDR dates", "", Some(9), None, None),
    (1, "Anna Kowalski – arrange site visit to Sunfield Energy", "", Some(15), None, None),
    (1, "תזכורת: לשלוח לאבי רוזן את מצגת המשקיעים", "", Some(2), None, None),
    (1, "Thank-you notes after Boston meetings", "", None, None, None),
    (1, "Bluewater Fund", "", None, None, Some(28)),
    (1, "Summit Securities", "", None, Some(1), Some(28)),
    (1, "Send meeting summary to Lior Ben-David", "", Some(-4), Some(3), None),
    (1, "Follow up with Sophie Laurent re Paris roadshow", "", Some(-7), Some(6), None),
    (1, "Emily Carter – confirm sales desk breakfast", "", Some(4), None, None),
];

const INFO: &[Tpl] = &[
    (0, "Renew the company domain and SSL certificate", "", Some(6), None, None),
    (0, "Update website investor-relations page", "New client logos + Q3 webcast links.", Some(2), None, None),
    (0, "Pay Atlas Conferences invoice", "", Some(-1), None, None),
    (0, "Archive Q2 client reports", "", None, None, None),
    (0, "Send October newsletter", "", Some(0), None, None),
    (0, "לעדכן את רשימת התפוצה של הניוזלטר", "", Some(3), None, None),
    (0, "Back up shared drive", "", Some(-8), Some(7), None),
    (1, "Reply to Chris Donovan re conference sponsorship", "", Some(1), None, None),
    (1, "Send capabilities deck to Seine Gestion", "", Some(10), None, None),
    (1, "Confirm Corvina Medical webcast provider", "", Some(-2), Some(1), None),
];

pub fn seed(email: &str, _name: &str) -> Store {
    let mut st = Store::default();
    let mut rng = seed::Rng::for_key(&format!("tasks/{email}"));
    let now = now();
    let today = crate::clock::today(); // the app clock (fixed in tests), like everything else
    let tpl = if email.starts_with("info@") { INFO } else { KENNY };
    for title in ["My Tasks", "Client follow-ups"] {
        let id = st.new_id("list");
        let etag = st.etag();
        st.lists.push(FList { id, etag, title: title.into(), updated: now - 30 * seed::DAY_MS, tasks: Vec::new() });
    }
    let mut ids: Vec<String> = Vec::new();
    let mut counters = [0u64; 2];
    let mut sub_counters: std::collections::HashMap<usize, u64> = Default::default();
    for (li, title, notes, due, done, parent) in tpl.iter() {
        let id = st.new_id("task");
        let etag = st.etag();
        ids.push(id.clone());
        let position = match parent {
            Some(p) => {
                let c = sub_counters.entry(*p).or_default();
                *c += 1;
                *c * GAP
            }
            None => {
                counters[*li] += 1;
                counters[*li] * GAP
            }
        };
        let completed = done.map(|d| now - d * seed::DAY_MS - rng.below(6) as i64 * seed::HOUR_MS);
        let updated = completed.unwrap_or(now - (1 + rng.below(20 * 24)) as i64 * seed::HOUR_MS);
        st.lists[*li].tasks.push(FTask {
            id,
            etag,
            title: title.to_string(),
            notes: if notes.is_empty() { None } else { Some(notes.to_string()) },
            status: if completed.is_some() { "completed".into() } else { "needsAction".into() },
            due: due.map(|d| (today + chrono::Duration::days(d)).format("%Y-%m-%d").to_string()),
            completed,
            updated,
            parent: parent.map(|p| ids[p].clone()),
            position,
            deleted: false,
            hidden: false,
        });
    }
    st
}

// ---- JSON -------------------------------------------------------------------------------------

fn list_json(l: &FList) -> Value {
    json!({
        "kind": "tasks#taskList",
        "id": l.id,
        "etag": l.etag,
        "title": l.title,
        "updated": fmt_ts(l.updated),
        "selfLink": format!("https://www.googleapis.com/tasks/v1/users/@me/lists/{}", l.id),
    })
}

fn task_json(list: &str, t: &FTask) -> Value {
    let mut v = json!({
        "kind": "tasks#task",
        "id": t.id,
        "etag": t.etag,
        "title": t.title,
        "updated": fmt_ts(t.updated),
        "selfLink": format!("https://www.googleapis.com/tasks/v1/lists/{list}/tasks/{}", t.id),
        "position": format!("{:020}", t.position),
        "status": t.status,
        "links": [],
        "webViewLink": format!("https://tasks.google.com/task/{}", t.id),
    });
    if let Some(p) = &t.parent {
        v["parent"] = json!(p);
    }
    if let Some(n) = &t.notes {
        v["notes"] = json!(n);
    }
    if let Some(d) = &t.due {
        v["due"] = json!(date_to_due(d));
    }
    if let Some(c) = t.completed {
        v["completed"] = json!(fmt_ts(c));
    }
    if t.deleted {
        v["deleted"] = json!(true);
    }
    if t.hidden {
        v["hidden"] = json!(true);
    }
    v
}

fn bad(msg: &str) -> Response {
    error(400, msg, "invalid")
}

// ---- routes -----------------------------------------------------------------------------------

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/tasks/v1/users/@me/lists", get(lists_list).post(lists_insert))
        .route("/tasks/v1/users/@me/lists/{list}", get(lists_get).patch(lists_patch).put(lists_patch).delete(lists_delete))
        .route("/tasks/v1/lists/{list}/tasks", get(tasks_list).post(tasks_insert))
        .route("/tasks/v1/lists/{list}/tasks/{task}", get(tasks_get).patch(tasks_patch).put(tasks_put).delete(tasks_delete))
        .route("/tasks/v1/lists/{list}/tasks/{task}/move", post(tasks_move))
        .route("/tasks/v1/lists/{list}/clear", post(tasks_clear))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageQ {
    max_results: Option<usize>,
    page_token: Option<String>,
}

async fn lists_list(State(s): State<Shared>, User(email): User, Query(q): Query<PageQ>) -> Response {
    s.read(|st| {
        let u = st.user(&email);
        let items: Vec<Value> = u.tasks.lists.iter().map(list_json).collect();
        let (page, next) = paginate(&items, &q.page_token, q.max_results.unwrap_or(1000).clamp(1, 1000)); // tasklists: default and max 1000
        let mut v = json!({ "kind": "tasks#taskLists", "etag": format!("\"{}\"", u.tasks.counter), "items": page });
        if let Some(n) = next {
            v["nextPageToken"] = json!(n);
        }
        Json(v).into_response()
    })
}

async fn lists_insert(State(s): State<Shared>, User(email): User, Json(body): Json<Value>) -> Response {
    let title = body["title"].as_str().unwrap_or("").trim().to_string();
    if title.is_empty() {
        return bad("Missing title.");
    }
    s.with(|st| {
        let t = &mut st.user(&email).tasks;
        let l = FList { id: t.new_id("list"), etag: t.etag(), title, updated: now(), tasks: Vec::new() };
        let v = list_json(&l);
        t.lists.push(l);
        Json(v).into_response()
    })
}

async fn lists_get(State(s): State<Shared>, User(email): User, Path(list): Path<String>) -> Response {
    s.read(|st| {
        let t = &st.user(&email).tasks;
        match t.list_idx(&list) {
            Some(i) => Json(list_json(&t.lists[i])).into_response(),
            None => not_found(),
        }
    })
}

async fn lists_patch(State(s): State<Shared>, User(email): User, Path(list): Path<String>, Json(body): Json<Value>) -> Response {
    s.with(|st| {
        let t = &mut st.user(&email).tasks;
        let Some(i) = t.list_idx(&list) else { return not_found() };
        let etag = t.etag();
        let l = &mut t.lists[i];
        if let Some(title) = body["title"].as_str() {
            if title.trim().is_empty() {
                return bad("Missing title.");
            }
            l.title = title.trim().to_string();
        }
        l.etag = etag;
        l.updated = now();
        Json(list_json(l)).into_response()
    })
}

async fn lists_delete(State(s): State<Shared>, User(email): User, Path(list): Path<String>) -> Response {
    s.with(|st| {
        let t = &mut st.user(&email).tasks;
        match t.list_idx(&list) {
            Some(0) => bad("The default task list can't be deleted."),
            Some(i) => {
                t.lists.remove(i);
                axum::http::StatusCode::NO_CONTENT.into_response()
            }
            None => not_found(),
        }
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQ {
    max_results: Option<usize>,
    page_token: Option<String>,
    show_completed: Option<bool>,
    show_hidden: Option<bool>,
    show_deleted: Option<bool>,
    updated_min: Option<String>,
    due_min: Option<String>,
    due_max: Option<String>,
    completed_min: Option<String>,
    completed_max: Option<String>,
}

async fn tasks_list(State(s): State<Shared>, User(email): User, Path(list): Path<String>, Query(q): Query<ListQ>) -> Response {
    let updated_min = match q.updated_min.as_deref().map(parse_ts) {
        Some(None) => return bad("Invalid value for updatedMin."),
        Some(Some(v)) => Some(v),
        None => None,
    };
    let due_min = q.due_min.as_deref().and_then(due_to_date);
    let due_max = q.due_max.as_deref().and_then(due_to_date);
    let completed_min = q.completed_min.as_deref().and_then(parse_ts);
    let completed_max = q.completed_max.as_deref().and_then(parse_ts);
    s.read(|st| {
        let t = &st.user(&email).tasks;
        let Some(li) = t.list_idx(&list) else { return not_found() };
        let l = &t.lists[li];
        let show_completed = q.show_completed.unwrap_or(true);
        let mut items: Vec<&FTask> = l
            .tasks
            .iter()
            .filter(|x| q.show_deleted.unwrap_or(false) || !x.deleted)
            .filter(|x| q.show_hidden.unwrap_or(false) || !x.hidden)
            .filter(|x| show_completed || x.status != "completed")
            .filter(|x| updated_min.is_none_or(|m| x.updated >= m))
            .filter(|x| due_min.as_ref().is_none_or(|m| x.due.as_ref().is_some_and(|d| d >= m)))
            .filter(|x| due_max.as_ref().is_none_or(|m| x.due.as_ref().is_some_and(|d| d < m)))
            .filter(|x| completed_min.is_none_or(|m| x.completed.is_some_and(|c| c >= m)))
            .filter(|x| completed_max.is_none_or(|m| x.completed.is_some_and(|c| c < m)))
            .collect();
        // top-level tasks first, each followed by its subtasks, in position order
        items.sort_by(|a, b| {
            let ka = (
                a.parent.as_ref().and_then(|p| l.tasks.iter().find(|x| &x.id == p)).map(|p| p.position).unwrap_or(a.position),
                a.parent.is_some(),
                a.position,
            );
            let kb = (
                b.parent.as_ref().and_then(|p| l.tasks.iter().find(|x| &x.id == p)).map(|p| p.position).unwrap_or(b.position),
                b.parent.is_some(),
                b.position,
            );
            ka.cmp(&kb)
        });
        let json_items: Vec<Value> = items.iter().map(|x| task_json(&l.id, x)).collect();
        let (page, next) = paginate(&json_items, &q.page_token, q.max_results.unwrap_or(20).clamp(1, 100));
        let mut v = json!({ "kind": "tasks#tasks", "etag": l.etag, "items": page });
        if let Some(n) = next {
            v["nextPageToken"] = json!(n);
        }
        Json(v).into_response()
    })
}

async fn tasks_get(State(s): State<Shared>, User(email): User, Path((list, task)): Path<(String, String)>) -> Response {
    s.read(|st| {
        let t = &st.user(&email).tasks;
        let Some(li) = t.list_idx(&list) else { return not_found() };
        match t.lists[li].tasks.iter().find(|x| x.id == task) {
            Some(x) => Json(task_json(&t.lists[li].id, x)).into_response(),
            None => not_found(),
        }
    })
}

/// Position for a task placed under `parent` right after `previous` (or first). Renumbers the
/// siblings (bumping their etag/updated so incremental sync sees it) when no gap is left.
fn position_for(store: &mut Store, li: usize, parent: &Option<String>, previous: Option<&str>, exclude: &str) -> Result<u64, Response> {
    // Google: "previous" / "parent" must exist and must not be hidden
    for (what, id) in [("previous", previous), ("parent", parent.as_deref())] {
        if let Some(id) = id
            && store.lists[li].tasks.iter().any(|x| x.id == id && x.hidden)
        {
            return Err(bad(&format!("Invalid value for {what}: the task is hidden.")));
        }
    }
    for attempt in 0..2 {
        let l = &store.lists[li];
        let mut sibs: Vec<(u64, String)> =
            l.tasks.iter().filter(|x| !x.deleted && &x.parent == parent && x.id != exclude).map(|x| (x.position, x.id.clone())).collect();
        sibs.sort();
        let (lo, hi) = match previous {
            Some(p) => {
                let Some(i) = sibs.iter().position(|(_, id)| id == p) else {
                    return Err(bad("Invalid value for previous."));
                };
                (sibs[i].0, sibs.get(i + 1).map(|s| s.0))
            }
            None => (0, sibs.first().map(|s| s.0)),
        };
        let pos = match hi {
            None => Some(lo + GAP),
            Some(h) if h > lo + 1 => Some(lo + (h - lo) / 2),
            _ => None,
        };
        if let Some(p) = pos {
            return Ok(p);
        }
        if attempt == 1 {
            break;
        }
        // renumber
        let now = now();
        for (n, (_, id)) in sibs.iter().enumerate() {
            let etag = store.etag();
            if let Some(x) = store.lists[li].tasks.iter_mut().find(|x| &x.id == id) {
                x.position = (n as u64 + 1) * GAP;
                x.etag = etag;
                x.updated = now;
            }
        }
    }
    Err(error(500, "Backend Error", "backendError"))
}

/// Apply a JSON body to a task (PATCH semantics; `full` = PUT, absent fields are cleared).
fn apply_body(x: &mut FTask, body: &Map<String, Value>, full: bool) -> Result<(), Response> {
    let has = |k: &str| body.contains_key(k) || full;
    if has("title") {
        x.title = body.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    }
    if has("notes") {
        x.notes = body.get("notes").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    }
    if has("due") {
        x.due = match body.get("due") {
            Some(Value::String(s)) => Some(due_to_date(s).ok_or_else(|| bad("Invalid value for due."))?),
            _ => None,
        };
    }
    if has("status") {
        match body.get("status").and_then(Value::as_str).unwrap_or("needsAction") {
            "completed" => {
                x.status = "completed".into();
                if x.completed.is_none() {
                    x.completed = Some(now());
                }
            }
            "needsAction" => {
                x.status = "needsAction".into();
                x.completed = None;
                x.hidden = false;
            }
            _ => return Err(bad("Invalid value for status.")),
        }
    }
    if body.contains_key("completed")
        && x.status == "completed"
        && let Some(Value::String(s)) = body.get("completed")
    {
        x.completed = Some(parse_ts(s).ok_or_else(|| bad("Invalid value for completed."))?)
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaceQ {
    parent: Option<String>,
    previous: Option<String>,
    destination_tasklist: Option<String>,
}

async fn tasks_insert(
    State(s): State<Shared>,
    User(email): User,
    Path(list): Path<String>,
    Query(q): Query<PlaceQ>,
    Json(body): Json<Value>,
) -> Response {
    let Some(obj) = body.as_object().cloned() else { return bad("Invalid JSON payload.") };
    s.with(|st| {
        let t = &mut st.user(&email).tasks;
        let Some(li) = t.list_idx(&list) else { return not_found() };
        let parent = q.parent.filter(|p| !p.is_empty());
        if let Some(p) = &parent
            && !t.lists[li].tasks.iter().any(|x| &x.id == p && !x.deleted)
        {
            return bad("Invalid value for parent.");
        }
        let id = t.new_id("task");
        let position = match position_for(t, li, &parent, q.previous.as_deref().filter(|p| !p.is_empty()), &id) {
            Ok(p) => p,
            Err(r) => return r,
        };
        let mut x = FTask {
            id,
            etag: t.etag(),
            title: String::new(),
            notes: None,
            status: "needsAction".into(),
            due: None,
            completed: None,
            updated: now(),
            parent,
            position,
            deleted: false,
            hidden: false,
        };
        if let Err(r) = apply_body(&mut x, &obj, false) {
            return r;
        }
        let v = task_json(&t.lists[li].id, &x);
        t.lists[li].tasks.push(x);
        Json(v).into_response()
    })
}

fn update(s: &Shared, email: &str, list: &str, task: &str, body: Value, full: bool) -> Response {
    let Some(obj) = body.as_object().cloned() else { return bad("Invalid JSON payload.") };
    s.with(|st| {
        let t = &mut st.user(email).tasks;
        let Some(li) = t.list_idx(list) else { return not_found() };
        let etag = t.etag();
        let lid = t.lists[li].id.clone();
        let Some(x) = t.lists[li].tasks.iter_mut().find(|x| x.id == task && !x.deleted) else { return not_found() };
        if let Err(r) = apply_body(x, &obj, full) {
            return r;
        }
        x.etag = etag;
        x.updated = now();
        Json(task_json(&lid, x)).into_response()
    })
}

async fn tasks_patch(
    State(s): State<Shared>,
    User(email): User,
    Path((list, task)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    update(&s, &email, &list, &task, body, false)
}

async fn tasks_put(
    State(s): State<Shared>,
    User(email): User,
    Path((list, task)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    update(&s, &email, &list, &task, body, true)
}

async fn tasks_delete(State(s): State<Shared>, User(email): User, Path((list, task)): Path<(String, String)>) -> Response {
    s.with(|st| {
        let t = &mut st.user(&email).tasks;
        let Some(li) = t.list_idx(&list) else { return not_found() };
        if !t.lists[li].tasks.iter().any(|x| x.id == task && !x.deleted) {
            return not_found();
        }
        // the task and its subtasks become tombstones
        let now = now();
        let ids: Vec<String> =
            t.lists[li].tasks.iter().filter(|x| x.id == task || x.parent.as_deref() == Some(task.as_str())).map(|x| x.id.clone()).collect();
        for id in ids {
            let etag = t.etag();
            if let Some(x) = t.lists[li].tasks.iter_mut().find(|x| x.id == id) {
                x.deleted = true;
                x.updated = now;
                x.etag = etag;
            }
        }
        axum::http::StatusCode::NO_CONTENT.into_response()
    })
}

async fn tasks_move(
    State(s): State<Shared>,
    User(email): User,
    Path((list, task)): Path<(String, String)>,
    Query(q): Query<PlaceQ>,
) -> Response {
    s.with(|st| {
        let t = &mut st.user(&email).tasks;
        let Some(li) = t.list_idx(&list) else { return not_found() };
        if !t.lists[li].tasks.iter().any(|x| x.id == task && !x.deleted) {
            return not_found();
        }
        let di = match q.destination_tasklist.as_deref().filter(|d| !d.is_empty()) {
            Some(d) => match t.list_idx(d) {
                Some(i) => i,
                None => return not_found(),
            },
            None => li,
        };
        let parent = q.parent.filter(|p| !p.is_empty());
        if let Some(p) = &parent
            && (p == &task || !t.lists[di].tasks.iter().any(|x| &x.id == p && !x.deleted))
        {
            return bad("Invalid value for parent.");
        }
        if di != li {
            // move the task and its subtasks to the destination list
            let moving: Vec<FTask> = {
                let src = &mut t.lists[li].tasks;
                let (m, keep): (Vec<FTask>, Vec<FTask>) =
                    src.drain(..).partition(|x| x.id == task || (x.parent.as_deref() == Some(task.as_str()) && !x.deleted));
                *src = keep;
                m
            };
            t.lists[di].tasks.extend(moving);
        }
        let position = match position_for(t, di, &parent, q.previous.as_deref().filter(|p| !p.is_empty()), &task) {
            Ok(p) => p,
            Err(r) => return r,
        };
        let etag = t.etag();
        let lid = t.lists[di].id.clone();
        let now = now();
        let x = t.lists[di].tasks.iter_mut().find(|x| x.id == task).expect("moved task");
        x.parent = parent;
        x.position = position;
        x.etag = etag;
        x.updated = now;
        let v = task_json(&lid, x);
        if di != li {
            // subtasks that moved along get a new etag / updated too
            let kids: Vec<String> =
                t.lists[di].tasks.iter().filter(|x| x.parent.as_deref() == Some(task.as_str())).map(|x| x.id.clone()).collect();
            for k in kids {
                let e = t.etag();
                if let Some(x) = t.lists[di].tasks.iter_mut().find(|x| x.id == k) {
                    x.etag = e;
                    x.updated = now;
                }
            }
        }
        Json(v).into_response()
    })
}

async fn tasks_clear(State(s): State<Shared>, User(email): User, Path(list): Path<String>) -> Response {
    s.with(|st| {
        let t = &mut st.user(&email).tasks;
        let Some(li) = t.list_idx(&list) else { return not_found() };
        let now = now();
        let ids: Vec<String> =
            t.lists[li].tasks.iter().filter(|x| x.status == "completed" && !x.hidden && !x.deleted).map(|x| x.id.clone()).collect();
        for id in ids {
            let etag = t.etag();
            if let Some(x) = t.lists[li].tasks.iter_mut().find(|x| x.id == id) {
                x.hidden = true;
                x.updated = now;
                x.etag = etag;
            }
        }
        axum::http::StatusCode::NO_CONTENT.into_response()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_deterministic_and_plausible() {
        let a = seed("kenny@duvz.demo", "Kenny Green");
        let b = seed("kenny@duvz.demo", "Kenny Green");
        assert_eq!(a.lists.len(), 2);
        assert_eq!(a.lists[0].title, "My Tasks");
        let n: usize = a.lists.iter().map(|l| l.tasks.len()).sum();
        assert!(n >= 30, "{n} tasks");
        assert_eq!(a.lists[0].tasks[3].id, b.lists[0].tasks[3].id);
        assert!(a.lists.iter().flat_map(|l| &l.tasks).any(|t| t.parent.is_some()));
        assert!(a.lists.iter().flat_map(|l| &l.tasks).any(|t| t.status == "completed"));
        assert!(a.lists.iter().flat_map(|l| &l.tasks).any(|t| t.title.chars().any(|c| ('\u{5d0}'..='\u{5ea}').contains(&c))));
    }

    #[test]
    fn positions_fill_gaps_then_renumber() {
        let mut st = seed("x@y.demo", "X");
        let first = st.lists[0].tasks.iter().filter(|t| t.parent.is_none()).map(|t| t.id.clone()).next().unwrap();
        let p = position_for(&mut st, 0, &None, None, "new").unwrap();
        assert!(p > 0 && p < GAP);
        // squeeze until the gap after `first` is exhausted → renumber keeps order
        for i in 0..40 {
            let p = position_for(&mut st, 0, &None, Some(&first), "").unwrap();
            let id = format!("n{i}");
            st.lists[0].tasks.push(FTask {
                id,
                etag: String::new(),
                title: String::new(),
                notes: None,
                status: "needsAction".into(),
                due: None,
                completed: None,
                updated: 0,
                parent: None,
                position: p,
                deleted: false,
                hidden: false,
            });
        }
        let mut pos: Vec<u64> = st.lists[0].tasks.iter().filter(|t| t.parent.is_none()).map(|t| t.position).collect();
        let n = pos.len();
        pos.sort();
        pos.dedup();
        assert_eq!(pos.len(), n, "positions stay unique");
    }
}

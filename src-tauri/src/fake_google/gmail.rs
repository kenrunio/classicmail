//! Fake Gmail API. Messages are stored as raw RFC 822 text (exactly what Gmail keeps); the read
//! endpoints derive `format=full|metadata|raw` from it. Every change appends to `history` so the
//! client's incremental sync (`history.list`) works like the real thing.
//!
//! Ownership: the store + read/modify/history/labels endpoints + seed belong to the mail-core
//! builder; send/drafts/upload/sendAs endpoints live in `gmail_compose.rs` (mail-compose builder).

use std::collections::{BTreeMap, HashSet};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Shared, User, error, not_found};

#[path = "gmail_mime.rs"]
pub mod mime;
#[path = "gmail_search.rs"]
pub mod search;
#[path = "gmail_seed.rs"]
pub mod seed_data;

pub const SYSTEM_LABELS: &[&str] = &[
    "INBOX",
    "SENT",
    "DRAFT",
    "TRASH",
    "SPAM",
    "STARRED",
    "UNREAD",
    "IMPORTANT",
    "CHAT",
    "CATEGORY_PERSONAL",
    "CATEGORY_SOCIAL",
    "CATEGORY_PROMOTIONS",
    "CATEGORY_UPDATES",
    "CATEGORY_FORUMS",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FakeLabel {
    pub id: String,
    pub name: String,
    /// "system" | "user"
    pub kind: String,
    pub color_bg: Option<String>,
    pub color_text: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FakeMessage {
    pub id: String,
    pub thread_id: String,
    pub label_ids: Vec<String>,
    /// epoch ms
    pub internal_date: i64,
    pub history_id: u64,
    /// RFC 822 message text (7-bit / encoded-words, as built by mail-builder)
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HistoryKind {
    MessageAdded,
    MessageDeleted,
    LabelsAdded(Vec<String>),
    LabelsRemoved(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub id: u64,
    pub message_id: String,
    pub thread_id: String,
    pub kind: HistoryKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FakeDraft {
    pub id: String,
    pub message_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FakeSendAs {
    pub email: String,
    pub display_name: String,
    pub signature: String,
    pub is_primary: bool,
    pub is_default: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Store {
    pub email: String,
    pub history_id: u64,
    /// history ids before this are "expired" (history.list → 404)
    pub history_floor: u64,
    pub next_id: u64,
    pub labels: Vec<FakeLabel>,
    pub messages: BTreeMap<String, FakeMessage>,
    pub drafts: BTreeMap<String, FakeDraft>,
    pub history: Vec<HistoryRecord>,
    pub send_as: Vec<FakeSendAs>,
    /// settings.filters (stored as sent; not applied to incoming mail)
    #[serde(default)]
    pub filters: Vec<crate::google::gmail::Filter>,
    #[serde(default)]
    pub next_filter: u64,
}

impl Store {
    /// New store with the system labels.
    pub fn empty(email: &str, name: &str) -> Self {
        let mut s =
            Store { email: email.to_string(), history_id: 1000, history_floor: 1000, next_id: 0x18a3c0000000, ..Default::default() };
        for l in SYSTEM_LABELS {
            s.labels.push(FakeLabel { id: l.to_string(), name: l.to_string(), kind: "system".into(), ..Default::default() });
        }
        s.send_as.push(FakeSendAs {
            email: email.to_string(),
            display_name: name.to_string(),
            signature: format!("<div>{name}<br>Duvz Software</div>"),
            is_primary: true,
            is_default: true,
        });
        s
    }

    /// Gmail-looking 16-hex-digit id.
    pub fn new_id(&mut self) -> String {
        self.next_id += 1 + (self.next_id % 7);
        format!("{:016x}", self.next_id)
    }

    fn bump(&mut self) -> u64 {
        self.history_id += 1;
        self.history_id
    }

    pub fn label_by_name(&self, name: &str) -> Option<&FakeLabel> {
        self.labels.iter().find(|l| l.name == name)
    }

    /// Create a user label (or return the existing id).
    pub fn ensure_label(&mut self, name: &str) -> String {
        if let Some(l) = self.label_by_name(name) {
            return l.id.clone();
        }
        let id = format!("Label_{}", self.labels.len() + 1);
        self.labels.push(FakeLabel { id: id.clone(), name: name.to_string(), kind: "user".into(), ..Default::default() });
        id
    }

    /// Store a message. `thread_id: None` starts a new thread. Returns the message id.
    pub fn add_message(&mut self, raw: String, label_ids: Vec<String>, thread_id: Option<String>, internal_date: i64) -> String {
        let id = self.new_id();
        let thread_id = thread_id.unwrap_or_else(|| id.clone());
        let h = self.bump();
        self.messages
            .insert(id.clone(), FakeMessage { id: id.clone(), thread_id: thread_id.clone(), label_ids, internal_date, history_id: h, raw });
        self.history.push(HistoryRecord { id: h, message_id: id.clone(), thread_id, kind: HistoryKind::MessageAdded });
        id
    }

    /// Add/remove labels; records history only for actual changes. Returns false if no such message.
    pub fn modify(&mut self, id: &str, add: &[String], remove: &[String]) -> bool {
        let Some(m) = self.messages.get(id) else { return false };
        let thread_id = m.thread_id.clone();
        let added: Vec<String> = add.iter().filter(|l| !m.label_ids.contains(l)).cloned().collect();
        let removed: Vec<String> = remove.iter().filter(|l| m.label_ids.contains(l)).cloned().collect();
        if added.is_empty() && removed.is_empty() {
            return true;
        }
        let h = self.bump();
        let m = self.messages.get_mut(id).unwrap();
        m.label_ids.retain(|l| !removed.contains(l));
        m.label_ids.extend(added.iter().cloned());
        m.history_id = h;
        if !added.is_empty() {
            self.history.push(HistoryRecord {
                id: h,
                message_id: id.to_string(),
                thread_id: thread_id.clone(),
                kind: HistoryKind::LabelsAdded(added),
            });
        }
        if !removed.is_empty() {
            self.history.push(HistoryRecord { id: h, message_id: id.to_string(), thread_id, kind: HistoryKind::LabelsRemoved(removed) });
        }
        true
    }

    /// Permanently delete.
    pub fn delete(&mut self, id: &str) -> bool {
        let Some(m) = self.messages.remove(id) else { return false };
        let h = self.bump();
        self.history.push(HistoryRecord { id: h, message_id: id.to_string(), thread_id: m.thread_id, kind: HistoryKind::MessageDeleted });
        self.drafts.retain(|_, d| d.message_id != id);
        true
    }

    /// Thread id for a reply: the thread of the message whose Message-ID matches `in_reply_to`.
    pub fn thread_for_message_id_header(&self, in_reply_to: &str) -> Option<String> {
        let needle = in_reply_to.trim().trim_start_matches('<').trim_end_matches('>');
        self.messages.values().find_map(|m| {
            let hdr = m.raw.lines().take_while(|l| !l.is_empty()).find(|l| l.to_ascii_lowercase().starts_with("message-id:"))?;
            hdr.contains(needle).then(|| m.thread_id.clone())
        })
    }
}

/// Seeded mailbox for a demo user: ~400 messages over six months (see `gmail_seed.rs`).
pub fn seed(email: &str, name: &str) -> Store {
    seed_data::seed(email, name)
}

/// Demo helper: a plausible new Inbox message "arrives" (timer in demo mode, `demo.deliverMail`).
/// Returns the new message id.
pub fn deliver_random(store: &mut Store, now_ms: i64) -> String {
    seed_data::deliver_random(store, now_ms)
}

/// Keep the history log bounded; older start ids then get 404 like real Gmail.
const HISTORY_KEEP: usize = 20_000;

impl Store {
    pub fn trim_history(&mut self) {
        if self.history.len() > HISTORY_KEEP {
            let cut = self.history.len() - HISTORY_KEEP;
            self.history.drain(..cut);
            self.history_floor = self.history.first().map(|h| h.id - 1).unwrap_or(self.history_id);
        }
    }

    /// Forget all history (tests: forces clients into a full resync via 404).
    pub fn expire_history(&mut self) {
        self.history.clear();
        self.history_floor = self.history_id;
    }

    pub fn trash(&mut self, id: &str) -> bool {
        self.modify(id, &["TRASH".to_string()], &["SPAM".to_string()])
    }

    pub fn untrash(&mut self, id: &str) -> bool {
        self.modify(id, &[], &["TRASH".to_string()])
    }

    fn label_known(&self, id: &str) -> bool {
        self.labels.iter().any(|l| l.id == id)
    }

    /// Messages newest first (Gmail lists by internal date, descending).
    fn sorted(&self) -> Vec<&FakeMessage> {
        let mut v: Vec<&FakeMessage> = self.messages.values().collect();
        v.sort_by(|a, b| b.internal_date.cmp(&a.internal_date).then(b.id.cmp(&a.id)));
        v
    }
}

pub fn routes() -> Router<Shared> {
    let base = "/gmail/v1/users/{uid}";
    Router::new()
        .route(&format!("{base}/profile"), get(profile))
        .route(&format!("{base}/labels"), get(labels_list).post(labels_create))
        .route(&format!("{base}/labels/{{id}}"), get(labels_get).patch(labels_patch).put(labels_patch).delete(labels_delete))
        .route(&format!("{base}/messages"), get(messages_list))
        .route(&format!("{base}/messages/batchModify"), post(messages_batch_modify))
        .route(&format!("{base}/messages/batchDelete"), post(messages_batch_delete))
        .route(&format!("{base}/messages/{{id}}"), get(messages_get).delete(messages_delete))
        .route(&format!("{base}/messages/{{id}}/modify"), post(messages_modify))
        .route(&format!("{base}/messages/{{id}}/trash"), post(messages_trash))
        .route(&format!("{base}/messages/{{id}}/untrash"), post(messages_untrash))
        .route(&format!("{base}/messages/{{id}}/attachments/{{aid}}"), get(attachments_get))
        .route(&format!("{base}/history"), get(history_list))
        // not part of Gmail: demo / test helpers
        .route(&format!("{base}/x-demo/deliver"), post(demo_deliver))
        .route(&format!("{base}/x-demo/expireHistory"), post(demo_expire_history))
        .route(&format!("{base}/x-demo/bulk"), post(demo_bulk))
        .route(&format!("{base}/x-demo/deliverRaw"), post(demo_deliver_raw))
        .merge(super::gmail_compose::routes())
}

// ---- handlers -------------------------------------------------------------------------------

type Q = Query<Vec<(String, String)>>;

fn qget<'a>(q: &'a [(String, String)], k: &str) -> Option<&'a str> {
    q.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

fn qall(q: &[(String, String)], k: &str) -> Vec<String> {
    q.iter().filter(|(n, _)| n == k).flat_map(|(_, v)| v.split(',').map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect()
}

fn bad_request(msg: &str) -> Response {
    error(400, msg, "invalidArgument")
}

fn ids_of(body: &Value) -> Vec<String> {
    body["ids"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

async fn profile(State(s): State<Shared>, User(email): User) -> Json<Value> {
    s.read(|st| {
        let g = &st.user(&email).gmail;
        let threads: HashSet<&str> = g.messages.values().map(|m| m.thread_id.as_str()).collect();
        Json(json!({
            "emailAddress": email,
            "messagesTotal": g.messages.len(),
            "threadsTotal": threads.len(),
            "historyId": g.history_id.to_string(),
        }))
    })
}

fn label_json(g: &Store, l: &FakeLabel, counts: bool) -> Value {
    let mut v = json!({ "id": l.id, "name": l.name, "type": l.kind });
    if l.kind == "user" {
        v["messageListVisibility"] = json!("show");
        v["labelListVisibility"] = json!("labelShow");
    } else if matches!(l.id.as_str(), "INBOX" | "SPAM" | "TRASH") {
        v["messageListVisibility"] = json!("hide");
        v["labelListVisibility"] = json!("labelShow");
    }
    if let (Some(bg), Some(tx)) = (&l.color_bg, &l.color_text) {
        v["color"] = json!({ "backgroundColor": bg, "textColor": tx });
    }
    if counts {
        let msgs: Vec<&FakeMessage> = g.messages.values().filter(|m| m.label_ids.contains(&l.id)).collect();
        let unread_msgs: Vec<&&FakeMessage> = msgs.iter().filter(|m| m.label_ids.iter().any(|x| x == "UNREAD")).collect();
        let threads: HashSet<&str> = msgs.iter().map(|m| m.thread_id.as_str()).collect();
        let uthreads: HashSet<&str> = unread_msgs.iter().map(|m| m.thread_id.as_str()).collect();
        v["messagesTotal"] = json!(msgs.len());
        v["messagesUnread"] = json!(unread_msgs.len());
        v["threadsTotal"] = json!(threads.len());
        v["threadsUnread"] = json!(uthreads.len());
    }
    v
}

async fn labels_list(State(s): State<Shared>, User(email): User) -> Json<Value> {
    s.read(|st| {
        let g = &st.user(&email).gmail;
        Json(json!({ "labels": g.labels.iter().map(|l| label_json(g, l, false)).collect::<Vec<_>>() }))
    })
}

async fn labels_get(State(s): State<Shared>, User(email): User, Path((_u, id)): Path<(String, String)>) -> Response {
    s.read(|st| {
        let g = &st.user(&email).gmail;
        match g.labels.iter().find(|l| l.id == id) {
            Some(l) => Json(label_json(g, l, true)).into_response(),
            None => not_found(),
        }
    })
}

fn valid_label_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && !n.starts_with('/')
        && !n.ends_with('/')
        && !n.contains("//")
        && !SYSTEM_LABELS.iter().any(|s| s.eq_ignore_ascii_case(n))
}

async fn labels_create(State(s): State<Shared>, User(email): User, Json(body): Json<Value>) -> Response {
    let name = body["name"].as_str().unwrap_or("").trim().to_string();
    if !valid_label_name(&name) {
        return bad_request("Invalid label name");
    }
    s.with(|st| {
        let g = &mut st.user(&email).gmail;
        if g.labels.iter().any(|l| l.name.eq_ignore_ascii_case(&name)) {
            return error(409, "Label name exists or conflicts", "alreadyExists");
        }
        let id = g.ensure_label(&name);
        let l = g.labels.iter().find(|l| l.id == id).unwrap().clone();
        Json(label_json(g, &l, false)).into_response()
    })
}

async fn labels_patch(
    State(s): State<Shared>,
    User(email): User,
    Path((_u, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    s.with(|st| {
        let g = &mut st.user(&email).gmail;
        let Some(idx) = g.labels.iter().position(|l| l.id == id) else { return not_found() };
        if g.labels[idx].kind != "user" {
            return bad_request("Invalid label: system labels can't be changed");
        }
        if let Some(name) = body["name"].as_str() {
            let name = name.trim().to_string();
            if !valid_label_name(&name) {
                return bad_request("Invalid label name");
            }
            if g.labels.iter().any(|l| l.id != id && l.name.eq_ignore_ascii_case(&name)) {
                return error(409, "Label name exists or conflicts", "alreadyExists");
            }
            g.labels[idx].name = name;
        }
        if let Some(c) = body.get("color") {
            g.labels[idx].color_bg = c["backgroundColor"].as_str().map(str::to_string);
            g.labels[idx].color_text = c["textColor"].as_str().map(str::to_string);
        }
        let l = g.labels[idx].clone();
        Json(label_json(g, &l, false)).into_response()
    })
}

async fn labels_delete(State(s): State<Shared>, User(email): User, Path((_u, id)): Path<(String, String)>) -> Response {
    s.with(|st| {
        let g = &mut st.user(&email).gmail;
        let Some(idx) = g.labels.iter().position(|l| l.id == id) else { return not_found() };
        if g.labels[idx].kind != "user" {
            return bad_request("Invalid label: system labels can't be deleted");
        }
        let ids: Vec<String> = g.messages.values().filter(|m| m.label_ids.contains(&id)).map(|m| m.id.clone()).collect();
        for m in ids {
            g.modify(&m, &[], std::slice::from_ref(&id));
        }
        g.labels.remove(idx);
        StatusCode::NO_CONTENT.into_response()
    })
}

fn message_json(m: &FakeMessage, format: &str, wanted_headers: &[String]) -> Value {
    let root = mime::parse(m.raw.as_bytes());
    let mut v = json!({
        "id": m.id,
        "threadId": m.thread_id,
        "labelIds": m.label_ids,
        "snippet": mime::snippet(&root),
        "sizeEstimate": m.raw.len(),
        "historyId": m.history_id.to_string(),
        "internalDate": m.internal_date.to_string(),
    });
    match format {
        "minimal" => {}
        "raw" => v["raw"] = json!(mime::b64url(m.raw.as_bytes())),
        "metadata" => v["payload"] = mime::metadata_payload_json(&root, wanted_headers),
        _ => v["payload"] = mime::payload_json(&m.id, &root, "", true),
    }
    v
}

fn minimal_json(m: &FakeMessage) -> Value {
    json!({ "id": m.id, "threadId": m.thread_id, "labelIds": m.label_ids })
}

async fn messages_get(State(s): State<Shared>, User(email): User, Path((_u, id)): Path<(String, String)>, Query(q): Q) -> Response {
    let format = qget(&q, "format").unwrap_or("full").to_string();
    if !matches!(format.as_str(), "full" | "metadata" | "minimal" | "raw") {
        return bad_request("Invalid format");
    }
    let wanted = qall(&q, "metadataHeaders");
    let m = s.read(|st| st.user(&email).gmail.messages.get(&id).cloned());
    match m {
        Some(m) => Json(message_json(&m, &format, &wanted)).into_response(),
        None => not_found(),
    }
}

async fn messages_list(State(s): State<Shared>, User(email): User, Query(q): Q) -> Response {
    let max = qget(&q, "maxResults").and_then(|v| v.parse::<usize>().ok()).unwrap_or(100).clamp(1, 500);
    let token = qget(&q, "pageToken").map(str::to_string);
    // like Gmail, a page token we never issued (or one that went stale) is a 400
    if token.as_deref().is_some_and(|t| t.parse::<usize>().is_err()) {
        return bad_request("Invalid pageToken");
    }
    let label_ids = qall(&q, "labelIds");
    let include_spam_trash = qget(&q, "includeSpamTrash").map(|v| v == "true").unwrap_or(false);
    let parsed = search::parse(qget(&q, "q").unwrap_or(""));
    let ids: Vec<Value> = s.read(|st| {
        let g = &st.user(&email).gmail;
        let exclude = !include_spam_trash && !label_ids.iter().any(|l| l == "TRASH" || l == "SPAM") && !parsed.overrides_spam_trash();
        g.sorted()
            .into_iter()
            .filter(|m| label_ids.iter().all(|l| m.label_ids.contains(l)))
            .filter(|m| !exclude || !m.label_ids.iter().any(|l| l == "TRASH" || l == "SPAM"))
            .filter(|m| parsed.is_empty() || parsed.matches(g, m))
            .map(|m| json!({ "id": m.id, "threadId": m.thread_id }))
            .collect()
    });
    let total = ids.len();
    let (page, next) = super::paginate(&ids, &token, max);
    let mut out = json!({ "resultSizeEstimate": total });
    if !page.is_empty() {
        out["messages"] = Value::Array(page);
    }
    if let Some(n) = next {
        out["nextPageToken"] = json!(n);
    }
    Json(out).into_response()
}

fn label_lists(body: &Value) -> (Vec<String>, Vec<String>) {
    let list = |k: &str| -> Vec<String> {
        body[k].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    (list("addLabelIds"), list("removeLabelIds"))
}

fn check_labels(g: &Store, add: &[String], remove: &[String]) -> Result<(), Response> {
    for l in add.iter().chain(remove) {
        if !g.label_known(l) {
            return Err(bad_request(&format!("Invalid label: {l}")));
        }
    }
    if add.iter().any(|l| l == "DRAFT") {
        return Err(bad_request("Invalid label: DRAFT"));
    }
    Ok(())
}

async fn messages_modify(
    State(s): State<Shared>,
    User(email): User,
    Path((_u, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let (add, remove) = label_lists(&body);
    s.with(|st| {
        let g = &mut st.user(&email).gmail;
        if let Err(r) = check_labels(g, &add, &remove) {
            return r;
        }
        if !g.modify(&id, &add, &remove) {
            return not_found();
        }
        g.trim_history();
        Json(minimal_json(&g.messages[&id])).into_response()
    })
}

async fn messages_batch_modify(State(s): State<Shared>, User(email): User, Json(body): Json<Value>) -> Response {
    let (add, remove) = label_lists(&body);
    let ids = ids_of(&body);
    if ids.len() > 1000 {
        return bad_request("Too many ids (max 1000)");
    }
    s.with(|st| {
        let g = &mut st.user(&email).gmail;
        if let Err(r) = check_labels(g, &add, &remove) {
            return r;
        }
        for id in &ids {
            g.modify(id, &add, &remove);
        }
        g.trim_history();
        StatusCode::NO_CONTENT.into_response()
    })
}

async fn messages_batch_delete(State(s): State<Shared>, User(email): User, Json(body): Json<Value>) -> Response {
    let ids = ids_of(&body);
    if ids.len() > 1000 {
        return bad_request("Too many ids (max 1000)");
    }
    s.with(|st| {
        let g = &mut st.user(&email).gmail;
        for id in &ids {
            g.delete(id);
        }
        g.trim_history();
        StatusCode::NO_CONTENT.into_response()
    })
}

async fn messages_trash(State(s): State<Shared>, User(email): User, Path((_u, id)): Path<(String, String)>) -> Response {
    s.with(|st| {
        let g = &mut st.user(&email).gmail;
        if !g.trash(&id) {
            return not_found();
        }
        Json(minimal_json(&g.messages[&id])).into_response()
    })
}

async fn messages_untrash(State(s): State<Shared>, User(email): User, Path((_u, id)): Path<(String, String)>) -> Response {
    s.with(|st| {
        let g = &mut st.user(&email).gmail;
        if !g.untrash(&id) {
            return not_found();
        }
        Json(minimal_json(&g.messages[&id])).into_response()
    })
}

async fn messages_delete(State(s): State<Shared>, User(email): User, Path((_u, id)): Path<(String, String)>) -> Response {
    s.with(|st| {
        let g = &mut st.user(&email).gmail;
        if !g.delete(&id) {
            return not_found();
        }
        StatusCode::NO_CONTENT.into_response()
    })
}

async fn attachments_get(State(s): State<Shared>, User(email): User, Path((_u, id, aid)): Path<(String, String, String)>) -> Response {
    let Some((mid, part_id)) = mime::parse_attachment_id(&aid) else { return bad_request("Invalid attachment id") };
    if mid != id {
        return not_found();
    }
    let raw = s.read(|st| st.user(&email).gmail.messages.get(&id).map(|m| m.raw.clone()));
    let Some(raw) = raw else { return not_found() };
    let root = mime::parse(raw.as_bytes());
    match mime::part_by_id(&root, &part_id) {
        Some(p) if p.children.is_empty() => {
            let data = p.decoded();
            Json(json!({ "attachmentId": aid, "size": data.len(), "data": mime::b64url(&data) })).into_response()
        }
        _ => not_found(),
    }
}

async fn history_list(State(s): State<Shared>, User(email): User, Query(q): Q) -> Response {
    let Some(start) = qget(&q, "startHistoryId").and_then(|v| v.parse::<u64>().ok()) else {
        return bad_request("startHistoryId is required");
    };
    let max = qget(&q, "maxResults").and_then(|v| v.parse::<usize>().ok()).unwrap_or(100).clamp(1, 500);
    let token = qget(&q, "pageToken").map(str::to_string);
    let types = qall(&q, "historyTypes");
    let want = |t: &str| types.is_empty() || types.iter().any(|x| x == t);
    s.read(|st| {
        let g = &st.user(&email).gmail;
        if start < g.history_floor {
            return not_found();
        }
        // one History entry per history id
        let mut groups: Vec<Value> = Vec::new();
        let mut cur_id = 0u64;
        for r in g.history.iter().filter(|r| r.id > start) {
            let (key, ok) = match &r.kind {
                HistoryKind::MessageAdded => ("messagesAdded", want("messageAdded")),
                HistoryKind::MessageDeleted => ("messagesDeleted", want("messageDeleted")),
                HistoryKind::LabelsAdded(_) => ("labelsAdded", want("labelAdded")),
                HistoryKind::LabelsRemoved(_) => ("labelsRemoved", want("labelRemoved")),
            };
            if !ok {
                continue;
            }
            if groups.is_empty() || cur_id != r.id {
                groups.push(json!({ "id": r.id.to_string(), "messages": [] }));
                cur_id = r.id;
            }
            let h = groups.last_mut().unwrap();
            let mut msg = json!({ "id": r.message_id, "threadId": r.thread_id });
            if let Some(m) = g.messages.get(&r.message_id) {
                msg["labelIds"] = json!(m.label_ids);
            }
            h["messages"].as_array_mut().unwrap().push(json!({ "id": r.message_id, "threadId": r.thread_id }));
            let entry = match &r.kind {
                HistoryKind::LabelsAdded(l) | HistoryKind::LabelsRemoved(l) => json!({ "message": msg, "labelIds": l }),
                _ => json!({ "message": msg }),
            };
            if h.get(key).is_none() {
                h[key] = json!([]);
            }
            h[key].as_array_mut().unwrap().push(entry);
        }
        let (page, next) = super::paginate(&groups, &token, max);
        let mut out = json!({ "historyId": g.history_id.to_string() });
        if !page.is_empty() {
            out["history"] = Value::Array(page);
        }
        if let Some(n) = next {
            out["nextPageToken"] = json!(n);
        }
        Json(out).into_response()
    })
}

// ---- demo / test helpers (not Gmail API) -------------------------------------------------------

async fn demo_deliver(State(s): State<Shared>, User(email): User) -> Json<Value> {
    let id = s.with(|st| deliver_random(&mut st.user(&email).gmail, crate::fake_google::seed::now_ms()));
    Json(json!({ "id": id }))
}

/// Deliver an arbitrary RFC 822 message (`{"raw": base64url}`) to the Inbox, unread — for
/// hostile-mail tests (the sender can be anyone, unlike `messages.send`).
async fn demo_deliver_raw(State(s): State<Shared>, User(email): User, Json(body): Json<Value>) -> Response {
    let Some(raw) = body["raw"].as_str().and_then(|r| crate::google::gmail::b64url_decode(r).ok()) else {
        return error(400, "raw (base64url) required", "invalidArgument");
    };
    let text = String::from_utf8_lossy(&raw).into_owned();
    let id = s.with(|st| {
        st.user(&email).gmail.add_message(text, vec!["INBOX".into(), "UNREAD".into()], None, crate::fake_google::seed::now_ms())
    });
    Json(json!({ "id": id })).into_response()
}

async fn demo_expire_history(State(s): State<Shared>, User(email): User) -> StatusCode {
    s.with(|st| st.user(&email).gmail.expire_history());
    StatusCode::NO_CONTENT
}

/// Add `count` generated messages (large-mailbox performance tests).
async fn demo_bulk(State(s): State<Shared>, User(email): User, Json(body): Json<Value>) -> Json<Value> {
    let count = body["count"].as_u64().unwrap_or(1000) as usize;
    let thread_pct = body["threadPct"].as_u64().unwrap_or(0).min(100);
    s.with(|st| {
        let g = &mut st.user(&email).gmail;
        seed_data::bulk(g, count, crate::fake_google::seed::now_ms(), thread_pct);
        g.history.clear();
        g.history_floor = g.history_id;
        Json(json!({ "total": g.messages.len() }))
    })
}

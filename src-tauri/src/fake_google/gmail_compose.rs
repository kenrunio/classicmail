//! Fake Gmail send / drafts / media upload / settings.sendAs endpoints (mail-compose builder).
//!
//! Shapes follow https://developers.google.com/gmail/api/reference/rest:
//! * `POST /gmail/v1/users/me/messages/send` `{raw, threadId?}` → `{id, threadId, labelIds}`
//! * `POST /upload/gmail/v1/users/me/messages/send?uploadType=media` (message/rfc822 body, no
//!   metadata) or `?uploadType=multipart` (multipart/related: JSON metadata `{threadId}` + message).
//!   Like Google, an unknown query parameter such as `threadId` is a 400.
//! * `drafts`: create (`POST`, also via `/upload/...`), list, get (`format=raw|minimal|metadata|full`),
//!   update (`PUT`, also via `/upload/...`), delete, `drafts/send`
//! * `GET settings/sendAs`
//! * `POST messages/import` `{raw, labelIds}` (also via `/upload/...` with metadata `{labelIds}`):
//!   adds the message with those labels, joined to the conversation its In-Reply-To points at.
//!   `internalDateSource=dateHeader` takes the date from the Date header.
//!
//! A sent message gets `SENT` (+ `INBOX`/`UNREAD` when addressed to the sender). Threading follows
//! Gmail's documented rule: it joins `threadId` only if the message has In-Reply-To / References
//! and its Subject matches the thread's (ignoring Re:/Fwd:); otherwise — and always without a
//! `threadId` — it starts a new conversation. Demo nicety: it is also delivered to the Inbox of any
//! other mailbox on this fake server it is addressed to (threaded there by References, as
//! incoming mail is).

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use mail_parser::MessageParser;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{FakeState, Shared, User, error, gmail::FakeMessage, not_found, paginate};

const BASE: &str = "/gmail/v1/users/me";
const UPLOAD: &str = "/upload/gmail/v1/users/me";

pub fn routes() -> Router<Shared> {
    Router::new()
        .route(&format!("{BASE}/messages/send"), post(send_json))
        .route(&format!("{UPLOAD}/messages/send"), post(send_upload))
        .route(&format!("{BASE}/messages/import"), post(import_json))
        .route(&format!("{UPLOAD}/messages/import"), post(import_upload))
        .route(&format!("{BASE}/drafts"), post(draft_create).get(draft_list))
        .route(&format!("{UPLOAD}/drafts"), post(draft_create_upload))
        .route(&format!("{BASE}/drafts/send"), post(draft_send))
        .route(&format!("{BASE}/drafts/{{id}}"), get(draft_get).put(draft_update).delete(draft_delete))
        .route(&format!("{UPLOAD}/drafts/{{id}}"), axum::routing::put(draft_update_upload))
        .route(&format!("{BASE}/settings/sendAs"), get(send_as_list))
        .route(&format!("{BASE}/settings/filters"), get(filters_list).post(filter_create))
        .route(&format!("{BASE}/settings/filters/{{id}}"), axum::routing::delete(filter_delete))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
}

fn decode_raw(raw: &str) -> Result<Vec<u8>, Response> {
    crate::google::gmail::b64url_decode(raw).map_err(|_| error(400, "Invalid value for ByteString", "invalidArgument"))
}

/// Recipients (lower case) of an RFC 822 message.
fn recipients(raw: &[u8]) -> Vec<String> {
    let Some(m) = MessageParser::default().parse(raw) else { return vec![] };
    let mut v = Vec::new();
    for a in [m.to(), m.cc(), m.bcc()].into_iter().flatten() {
        for x in a.iter() {
            if let Some(e) = x.address() {
                v.push(e.trim().to_lowercase());
            }
        }
    }
    v
}

fn header(raw: &[u8], name: &str) -> Option<String> {
    let m = MessageParser::default().parse(raw)?;
    m.header_raw(name).map(|s| s.trim().to_string())
}

/// Remove a header (e.g. Bcc from copies delivered to recipients).
fn strip_header(raw: &[u8], name: &str) -> Vec<u8> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 2).unwrap_or(0);
    if end == 0 {
        return raw.to_vec();
    }
    let head = String::from_utf8_lossy(&raw[..end]);
    let prefix = format!("{}:", name.to_ascii_lowercase());
    let mut out = String::new();
    let mut skipping = false;
    for line in head.split_inclusive("\r\n") {
        if skipping && (line.starts_with(' ') || line.starts_with('\t')) {
            continue;
        }
        skipping = line.to_ascii_lowercase().starts_with(&prefix);
        if !skipping {
            out.push_str(line);
        }
    }
    let mut v = out.into_bytes();
    v.extend_from_slice(&raw[end..]);
    v
}

fn now_ms() -> i64 {
    crate::clock::now_ms()
}

fn minimal(m: &FakeMessage) -> Value {
    json!({ "id": m.id, "threadId": m.thread_id, "labelIds": m.label_ids })
}

/// Message resource for drafts.get (raw / minimal / metadata; `full` gives headers without parts).
fn resource(m: &FakeMessage, format: &str) -> Value {
    let raw = m.raw.as_bytes();
    let parsed = MessageParser::default().parse(raw);
    let snippet: String =
        parsed.as_ref().and_then(|p| p.body_preview(200)).map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
    let mut v = json!({
        "id": m.id,
        "threadId": m.thread_id,
        "labelIds": m.label_ids,
        "snippet": snippet,
        "historyId": m.history_id.to_string(),
        "internalDate": m.internal_date.to_string(),
        "sizeEstimate": raw.len(),
    });
    match format {
        "raw" => v["raw"] = json!(crate::google::gmail::b64url_encode(raw)),
        "minimal" => {}
        _ => {
            let headers: Vec<Value> = parsed
                .as_ref()
                .map(|p| p.headers_raw().map(|(n, val)| json!({ "name": n, "value": val.trim() })).collect())
                .unwrap_or_default();
            let ct = header(raw, "Content-Type").unwrap_or_else(|| "text/plain".into());
            let mime = ct.split(';').next().unwrap_or("text/plain").trim().to_lowercase();
            v["payload"] = json!({ "partId": "", "mimeType": mime, "filename": "", "headers": headers, "body": { "size": 0 } });
        }
    }
    v
}

/// Store a sent message in the sender's mailbox (and deliver to other fake mailboxes).
fn deliver(st: &mut FakeState, email: &str, raw: Vec<u8>, thread_id: Option<String>) -> Result<Value, Response> {
    let rcpts = recipients(&raw);
    if rcpts.is_empty() {
        return Err(error(400, "Invalid To header", "invalidArgument"));
    }
    let in_reply_to = header(&raw, "In-Reply-To");
    let now = now_ms();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (id, msg) = {
        let u = st.user(email);
        let own: Vec<String> = u.gmail.send_as.iter().map(|s| s.email.to_lowercase()).chain([email.to_lowercase()]).collect();
        let thread = match thread_id.filter(|t| !t.is_empty()) {
            Some(t) => {
                if !u.gmail.messages.values().any(|m| m.thread_id == t) {
                    return Err(not_found());
                }
                joins_thread(&u.gmail, &t, &raw).then_some(t)
            }
            // Gmail does not thread API sends by headers alone: no threadId, new conversation
            None => None,
        };
        let mut labels = vec!["SENT".to_string()];
        if rcpts.iter().any(|r| own.contains(r)) {
            labels.push("INBOX".into());
            labels.push("UNREAD".into());
        }
        let id = u.gmail.add_message(text, labels, thread, now);
        let msg = minimal(&u.gmail.messages[&id]);
        (id, msg)
    };
    // other mailboxes on this server receive a copy (without Bcc)
    let others: Vec<String> = st.users.keys().filter(|k| k.as_str() != email && rcpts.iter().any(|r| r == *k)).cloned().collect();
    if !others.is_empty() {
        let copy = String::from_utf8_lossy(&strip_header(&raw, "Bcc")).into_owned();
        for o in others {
            let u = st.user(&o);
            let thread = in_reply_to.as_deref().and_then(|h| u.gmail.thread_for_message_id_header(h));
            u.gmail.add_message(copy.clone(), vec!["INBOX".into(), "UNREAD".into()], thread, now);
        }
    }
    log::debug!("fake gmail: {email} sent {id}");
    Ok(msg)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMessage {
    raw: Option<String>,
    thread_id: Option<String>,
}

async fn send_json(State(s): State<Shared>, User(email): User, Json(body): Json<RawMessage>) -> Response {
    let Some(raw) = body.raw else {
        return error(400, "'raw' RFC822 payload message string or uploading message via /upload/* URL required", "invalidArgument");
    };
    let bytes = match decode_raw(&raw) {
        Ok(b) => b,
        Err(e) => return e,
    };
    match s.with(|st| deliver(st, &email, bytes, body.thread_id)) {
        Ok(v) => Json(v).into_response(),
        Err(e) => e,
    }
}

/// Subject without reply / forward prefixes, for Gmail's "Subject headers must match" rule.
fn thread_subject(s: &str) -> String {
    let mut t = s.trim().to_lowercase();
    loop {
        let before = t.len();
        for p in ["re:", "fw:", "fwd:", "aw:", "wg:"] {
            if let Some(rest) = t.strip_prefix(p) {
                t = rest.trim_start().to_string();
            }
        }
        if t.len() == before {
            return t;
        }
    }
}

/// Gmail's documented rule for `threadId`: the message must carry In-Reply-To or References and its
/// Subject must match the conversation's.
fn joins_thread(g: &super::gmail::Store, thread_id: &str, raw: &[u8]) -> bool {
    let Some(m) = MessageParser::default().parse(raw) else { return false };
    let has_refs = header(raw, "In-Reply-To").is_some() || header(raw, "References").is_some();
    let subj = thread_subject(m.subject().unwrap_or(""));
    has_refs
        && g.messages
            .values()
            .filter(|x| x.thread_id == thread_id)
            .any(|x| MessageParser::default().parse(x.raw.as_bytes()).is_some_and(|p| thread_subject(p.subject().unwrap_or("")) == subj))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadQuery {
    upload_type: Option<String>,
    thread_id: Option<String>,
}

fn find_bytes(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

/// An upload request (`uploadType=media` or `multipart`) → (metadata JSON, RFC 822 message).
fn parse_upload(q: &UploadQuery, headers: &HeaderMap, body: &[u8]) -> Result<(Value, Vec<u8>), Response> {
    if q.thread_id.is_some() {
        return Err(error(
            400,
            "Invalid JSON payload received. Unknown name \"threadId\": Cannot bind query parameter. Field 'threadId' could not be found in request message.",
            "invalid",
        ));
    }
    match q.upload_type.as_deref() {
        Some("media") => {
            if body.is_empty() {
                return Err(error(400, "Media upload has no content", "invalidArgument"));
            }
            Ok((json!({}), body.to_vec()))
        }
        Some("multipart") => {
            let ct = headers.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("");
            let boundary = ct
                .split(';')
                .find_map(|p| p.trim().strip_prefix("boundary="))
                .map(|b| b.trim_matches('"').to_string())
                .filter(|_| ct.to_ascii_lowercase().starts_with("multipart/related"))
                .ok_or_else(|| error(400, "Multipart upload needs Content-Type: multipart/related; boundary=...", "invalidArgument"))?;
            let delim = format!("--{boundary}");
            let mut cuts = Vec::new();
            let mut i = 0;
            while let Some(p) = find_bytes(&body[i..], delim.as_bytes()) {
                cuts.push(i + p);
                i += p + delim.len();
            }
            let mut parts: Vec<&[u8]> = Vec::new();
            for w in cuts.windows(2) {
                let mut seg = &body[w[0] + delim.len()..w[1]];
                seg = seg.strip_prefix(b"\r\n").unwrap_or(seg);
                seg = seg.strip_suffix(b"\r\n").unwrap_or(seg);
                let content = find_bytes(seg, b"\r\n\r\n").map(|p| &seg[p + 4..]).unwrap_or(&[]);
                parts.push(content);
            }
            let [meta, raw] = parts.as_slice() else {
                return Err(error(400, "Multipart upload must have a metadata part and a media part", "invalidArgument"));
            };
            let meta: Value =
                serde_json::from_slice(meta).map_err(|_| error(400, "Invalid JSON in the metadata part", "invalidArgument"))?;
            if raw.is_empty() {
                return Err(error(400, "Media upload has no content", "invalidArgument"));
            }
            Ok((meta, raw.to_vec()))
        }
        _ => Err(error(400, "Invalid uploadType (media, multipart or resumable)", "invalidArgument")),
    }
}

async fn send_upload(
    State(s): State<Shared>,
    User(email): User,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (meta, raw) = match parse_upload(&q, &headers, &body) {
        Ok(x) => x,
        Err(e) => return e,
    };
    let thread = meta["threadId"].as_str().map(str::to_string);
    match s.with(|st| deliver(st, &email, raw, thread)) {
        Ok(v) => Json(v).into_response(),
        Err(e) => e,
    }
}

// ---- import --------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportQuery {
    upload_type: Option<String>,
    internal_date_source: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportBody {
    raw: Option<String>,
    #[serde(default)]
    label_ids: Vec<String>,
}

fn import(st: &mut FakeState, email: &str, raw: Vec<u8>, labels: Vec<String>, date_header: bool) -> Result<Value, Response> {
    if labels.iter().any(|l| l == "DRAFT" || l == "SENT") {
        return Err(error(400, "Invalid label: DRAFT/SENT can't be set on import", "invalidArgument"));
    }
    let parsed = MessageParser::default().parse(&raw);
    let date = parsed.as_ref().and_then(|p| p.date()).map(|d| d.to_timestamp() * 1000).filter(|_| date_header).unwrap_or_else(now_ms);
    let in_reply_to = header(&raw, "In-Reply-To");
    let u = st.user(email);
    let thread = in_reply_to.as_deref().and_then(|h| u.gmail.thread_for_message_id_header(h));
    let id = u.gmail.add_message(String::from_utf8_lossy(&raw).into_owned(), labels, thread, date);
    Ok(minimal(&u.gmail.messages[&id]))
}

async fn import_json(State(s): State<Shared>, User(email): User, Query(q): Query<ImportQuery>, Json(body): Json<ImportBody>) -> Response {
    let Some(raw) = body.raw else { return error(400, "Missing message", "invalidArgument") };
    let bytes = match decode_raw(&raw) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let date_header = q.internal_date_source.as_deref() == Some("dateHeader");
    match s.with(|st| import(st, &email, bytes, body.label_ids, date_header)) {
        Ok(v) => Json(v).into_response(),
        Err(e) => e,
    }
}

async fn import_upload(
    State(s): State<Shared>,
    User(email): User,
    Query(q): Query<ImportQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let uq = UploadQuery { upload_type: q.upload_type.clone(), thread_id: None };
    let (meta, raw) = match parse_upload(&uq, &headers, &body) {
        Ok(x) => x,
        Err(e) => return e,
    };
    let labels: Vec<String> = serde_json::from_value(meta["labelIds"].clone()).unwrap_or_default();
    let date_header = q.internal_date_source.as_deref() == Some("dateHeader");
    match s.with(|st| import(st, &email, raw, labels, date_header)) {
        Ok(v) => Json(v).into_response(),
        Err(e) => e,
    }
}

// ---- drafts --------------------------------------------------------------------------------

#[derive(Deserialize)]
struct DraftBody {
    id: Option<String>,
    message: Option<RawMessage>,
}

fn new_draft(st: &mut FakeState, email: &str, raw: Vec<u8>, thread_id: Option<String>, draft_id: Option<String>) -> Value {
    let now = now_ms();
    let u = st.user(email);
    let thread = thread_id.filter(|t| u.gmail.messages.values().any(|m| &m.thread_id == t) && joins_thread(&u.gmail, t, &raw));
    let mid = u.gmail.add_message(String::from_utf8_lossy(&raw).into_owned(), vec!["DRAFT".into()], thread, now);
    let id = draft_id.unwrap_or_else(|| format!("r{}", u64::from_str_radix(&u.gmail.new_id(), 16).unwrap_or(0)));
    u.gmail.drafts.insert(id.clone(), super::gmail::FakeDraft { id: id.clone(), message_id: mid.clone() });
    json!({ "id": id, "message": minimal(&u.gmail.messages[&mid]) })
}

async fn draft_create(State(s): State<Shared>, User(email): User, Json(body): Json<DraftBody>) -> Response {
    let Some(msg) = body.message else { return error(400, "Missing draft message", "invalidArgument") };
    let Some(raw) = msg.raw else { return error(400, "Missing draft message", "invalidArgument") };
    let bytes = match decode_raw(&raw) {
        Ok(b) => b,
        Err(e) => return e,
    };
    Json(s.with(|st| new_draft(st, &email, bytes, msg.thread_id, None))).into_response()
}

async fn draft_create_upload(
    State(s): State<Shared>,
    User(email): User,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (meta, raw) = match parse_upload(&q, &headers, &body) {
        Ok(x) => x,
        Err(e) => return e,
    };
    let thread = meta["message"]["threadId"].as_str().map(str::to_string);
    Json(s.with(|st| new_draft(st, &email, raw, thread, None))).into_response()
}

async fn draft_update_upload(
    State(s): State<Shared>,
    User(email): User,
    Path(id): Path<String>,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (meta, raw) = match parse_upload(&q, &headers, &body) {
        Ok(x) => x,
        Err(e) => return e,
    };
    let thread = meta["message"]["threadId"].as_str().map(str::to_string);
    replace_draft(&s, &email, &id, raw, thread)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    max_results: Option<usize>,
    page_token: Option<String>,
}

async fn draft_list(State(s): State<Shared>, User(email): User, Query(q): Query<ListQuery>) -> Json<Value> {
    s.read(|st| {
        let u = st.user(&email);
        // newest first, like Gmail
        let mut items: Vec<(i64, Value)> = u
            .gmail
            .drafts
            .values()
            .filter_map(|d| {
                let m = u.gmail.messages.get(&d.message_id)?;
                Some((m.internal_date, json!({ "id": d.id, "message": { "id": m.id, "threadId": m.thread_id } })))
            })
            .collect();
        items.sort_by(|a, b| b.0.cmp(&a.0));
        let items: Vec<Value> = items.into_iter().map(|(_, v)| v).collect();
        let (page, next) = paginate(&items, &q.page_token, q.max_results.unwrap_or(100).clamp(1, 500));
        let mut v = json!({ "drafts": page, "resultSizeEstimate": items.len() });
        if let Some(n) = next {
            v["nextPageToken"] = json!(n);
        }
        Json(v)
    })
}

#[derive(Deserialize)]
struct FormatQuery {
    format: Option<String>,
}

async fn draft_get(State(s): State<Shared>, User(email): User, Path(id): Path<String>, Query(q): Query<FormatQuery>) -> Response {
    s.read(|st| {
        let u = st.user(&email);
        let Some(d) = u.gmail.drafts.get(&id) else { return not_found() };
        let Some(m) = u.gmail.messages.get(&d.message_id) else { return not_found() };
        Json(json!({ "id": d.id, "message": resource(m, q.format.as_deref().unwrap_or("full")) })).into_response()
    })
}

async fn draft_update(State(s): State<Shared>, User(email): User, Path(id): Path<String>, Json(body): Json<DraftBody>) -> Response {
    let Some(msg) = body.message else { return error(400, "Missing draft message", "invalidArgument") };
    let Some(raw) = msg.raw else { return error(400, "Missing draft message", "invalidArgument") };
    if body.id.as_deref().is_some_and(|b| b != id) {
        return error(400, "Draft id in the body does not match the URL", "invalidArgument");
    }
    let bytes = match decode_raw(&raw) {
        Ok(b) => b,
        Err(e) => return e,
    };
    replace_draft(&s, &email, &id, bytes, msg.thread_id)
}

fn replace_draft(s: &Shared, email: &str, id: &str, bytes: Vec<u8>, thread_id: Option<String>) -> Response {
    let email = email.to_string();
    let id = id.to_string();
    s.with(|st| {
        let old = {
            let u = st.user(&email);
            match u.gmail.drafts.get(&id) {
                Some(d) => d.message_id.clone(),
                None => return not_found(),
            }
        };
        // Gmail replaces the draft's message (new message id, same draft id, same thread)
        let thread = st.user(&email).gmail.messages.get(&old).map(|m| m.thread_id.clone());
        st.user(&email).gmail.delete(&old);
        let thread = thread_id.or(thread);
        Json(new_draft(st, &email, bytes, thread, Some(id.clone()))).into_response()
    })
}

async fn draft_delete(State(s): State<Shared>, User(email): User, Path(id): Path<String>) -> Response {
    s.with(|st| {
        let u = st.user(&email);
        let Some(d) = u.gmail.drafts.remove(&id) else { return not_found() };
        u.gmail.delete(&d.message_id);
        StatusCode::NO_CONTENT.into_response()
    })
}

async fn draft_send(State(s): State<Shared>, User(email): User, Json(body): Json<DraftBody>) -> Response {
    let Some(id) = body.id else { return error(400, "Missing draft id", "invalidArgument") };
    s.with(|st| {
        let (mid, raw) = {
            let u = st.user(&email);
            let Some(d) = u.gmail.drafts.get(&id).cloned() else { return not_found() };
            let Some(m) = u.gmail.messages.get(&d.message_id) else { return not_found() };
            (d.message_id.clone(), m.raw.clone())
        };
        let rcpts = recipients(raw.as_bytes());
        if rcpts.is_empty() {
            return error(400, "Invalid To header", "invalidArgument");
        }
        let u = st.user(&email);
        let own: Vec<String> = u.gmail.send_as.iter().map(|s| s.email.to_lowercase()).chain([email.to_lowercase()]).collect();
        u.gmail.drafts.remove(&id);
        let mut add = vec!["SENT".to_string()];
        if rcpts.iter().any(|r| own.contains(r)) {
            add.push("INBOX".into());
            add.push("UNREAD".into());
        }
        u.gmail.modify(&mid, &add, &["DRAFT".to_string()]);
        Json(minimal(&u.gmail.messages[&mid])).into_response()
    })
}

// ---- settings.sendAs -----------------------------------------------------------------------

async fn filters_list(State(s): State<Shared>, User(email): User) -> Json<Value> {
    s.read(|st| {
        let u = st.user(&email);
        Json(if u.gmail.filters.is_empty() { json!({}) } else { json!({ "filter": u.gmail.filters }) })
    })
}

async fn filter_create(State(s): State<Shared>, User(email): User, Json(mut f): Json<crate::google::gmail::Filter>) -> Response {
    if f.criteria.query.as_deref().unwrap_or("").trim().is_empty() {
        return error(400, "Filter doesn't have any criteria", "invalidArgument");
    }
    s.with(|st| {
        let g = &mut st.user(&email).gmail;
        g.next_filter += 1;
        f.id = Some(format!("ANe1Bm{:06}", g.next_filter));
        g.filters.push(f.clone());
        Json(f).into_response()
    })
}

async fn filter_delete(State(s): State<Shared>, User(email): User, Path(id): Path<String>) -> Response {
    s.with(|st| {
        let g = &mut st.user(&email).gmail;
        let before = g.filters.len();
        g.filters.retain(|f| f.id.as_deref() != Some(id.as_str()));
        if g.filters.len() == before { not_found() } else { StatusCode::NO_CONTENT.into_response() }
    })
}

async fn send_as_list(State(s): State<Shared>, User(email): User) -> Json<Value> {
    s.read(|st| {
        let u = st.user(&email);
        let list: Vec<Value> = u
            .gmail
            .send_as
            .iter()
            .map(|a| {
                let mut v = json!({
                    "sendAsEmail": a.email,
                    "displayName": a.display_name,
                    "replyToAddress": "",
                    "signature": a.signature,
                    "isDefault": a.is_default,
                    "treatAsAlias": !a.is_primary,
                });
                if a.is_primary {
                    v["isPrimary"] = json!(true);
                } else {
                    v["verificationStatus"] = json!("accepted");
                }
                v
            })
            .collect();
        Json(json!({ "sendAs": list }))
    })
}

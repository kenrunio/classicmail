//! Fake Google Drive v3 (see fake_google/mod.rs for conventions) — what large attachments sent as
//! links use (google/drive.rs): files list (the `q` clauses ClassicMail writes), get, create a
//! folder, patch (rename / trash), permissions create, about (storage) and resumable uploads
//! (start, parts with 308 Resume Incomplete + Range, status query, cancel).
//!
//! Only metadata is kept, not the uploaded bytes. An address whose domain contains "nogoogle" has
//! no Google account (permissions answer 400 invalidSharingRequest, as Google does).

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Shared, User, error, not_found, seed};
use crate::google::tasks::fmt_ts;

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    pub files: Vec<FFile>,
    pub sessions: BTreeMap<String, FSession>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: Option<u64>,
    pub parents: Vec<String>,
    pub app_properties: BTreeMap<String, String>,
    pub trashed: bool,
    pub created: i64,
    /// ("user", email) / ("anyone", "")
    pub permissions: Vec<(String, String)>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FSession {
    pub meta: Value,
    pub mime: String,
    pub total: u64,
    pub received: u64,
}

fn file_json(f: &FFile) -> Value {
    let mut v = json!({
        "id": f.id,
        "name": f.name,
        "mimeType": f.mime_type,
        "createdTime": fmt_ts(f.created),
        "trashed": f.trashed,
        "parents": f.parents,
        "appProperties": f.app_properties,
        "webViewLink": format!("https://drive.google.com/file/d/{}/view?usp=drivesdk", f.id),
    });
    if let Some(s) = f.size {
        v["size"] = json!(s.to_string());
    }
    v
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/drive/v3/files", get(files_list).post(files_create))
        .route("/drive/v3/files/{id}", get(files_get).patch(files_patch))
        .route("/drive/v3/files/{id}/permissions", post(permissions_create))
        .route("/drive/v3/about", get(about))
        .route("/upload/drive/v3/files", post(upload_start).put(upload_put).delete(upload_cancel))
        // upload parts are 8 MB (axum's default limit is 2 MB)
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
}

fn new_id(s: &Shared) -> String {
    s.with(|st| format!("drv{:08}", st.next()))
}

/// `'x'` → x (with \' and \\ undone)
fn lit(s: &str) -> String {
    s.trim().trim_matches('\'').replace("\\'", "'").replace("\\\\", "\\")
}

/// The clauses ClassicMail writes, joined by `and`.
fn matches(f: &FFile, q: &str) -> bool {
    let props = regex::Regex::new(r"appProperties has \{ key='([^']*)' and value='([^']*)' \}").unwrap();
    for c in props.captures_iter(q) {
        if f.app_properties.get(&c[1]) != Some(&c[2].to_string()) {
            return false;
        }
    }
    let rest = props.replace_all(q, "");
    for clause in rest.split(" and ").map(str::trim).filter(|c| !c.is_empty()) {
        let ok = if let Some(v) = clause.strip_prefix("mimeType!=") {
            f.mime_type != lit(v)
        } else if let Some(v) = clause.strip_prefix("mimeType=") {
            f.mime_type == lit(v)
        } else if let Some(v) = clause.strip_prefix("trashed=") {
            f.trashed == (v.trim() == "true")
        } else if let Some(id) = clause.strip_suffix(" in parents") {
            f.parents.contains(&lit(id))
        } else if let Some(v) = clause.strip_prefix("createdTime < ") {
            crate::google::rfc3339_ms(&lit(v)).is_some_and(|t| f.created < t)
        } else if let Some(v) = clause.strip_prefix("name=") {
            f.name == lit(v)
        } else {
            true
        };
        if !ok {
            return false;
        }
    }
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQ {
    q: Option<String>,
    page_token: Option<String>,
    page_size: Option<usize>,
}

async fn files_list(State(s): State<Shared>, User(email): User, Query(q): Query<ListQ>) -> Response {
    s.read(|st| {
        let mut files: Vec<FFile> =
            st.user(&email).drive.files.iter().filter(|f| matches(f, q.q.as_deref().unwrap_or(""))).cloned().collect();
        files.sort_by_key(|f| f.created);
        let (page, next) = super::paginate(&files, &q.page_token, q.page_size.unwrap_or(100).min(1000));
        let mut v = json!({ "files": page.iter().map(file_json).collect::<Vec<_>>() });
        if let Some(n) = next {
            v["nextPageToken"] = json!(n);
        }
        Json(v).into_response()
    })
}

async fn files_get(State(s): State<Shared>, User(email): User, Path(id): Path<String>) -> Response {
    s.read(|st| match st.user(&email).drive.files.iter().find(|f| f.id == id) {
        Some(f) => Json(file_json(f)).into_response(),
        None => not_found(),
    })
}

async fn files_create(State(s): State<Shared>, User(email): User, Json(body): Json<Value>) -> Response {
    let id = new_id(&s);
    s.with(|st| {
        let f = FFile {
            id,
            name: body["name"].as_str().unwrap_or("Untitled").to_string(),
            mime_type: body["mimeType"].as_str().unwrap_or("application/octet-stream").to_string(),
            size: None,
            parents: body["parents"]
                .as_array()
                .map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            app_properties: serde_json::from_value(body["appProperties"].clone()).unwrap_or_default(),
            trashed: false,
            created: seed::now_ms(),
            permissions: vec![],
        };
        let v = file_json(&f);
        st.user(&email).drive.files.push(f);
        Json(v).into_response()
    })
}

async fn files_patch(State(s): State<Shared>, User(email): User, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    s.with(|st| match st.user(&email).drive.files.iter_mut().find(|f| f.id == id) {
        Some(f) => {
            if let Some(n) = body["name"].as_str() {
                f.name = n.to_string();
            }
            if let Some(t) = body["trashed"].as_bool() {
                f.trashed = t;
            }
            Json(file_json(f)).into_response()
        }
        None => not_found(),
    })
}

async fn permissions_create(State(s): State<Shared>, User(email): User, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    let kind = body["type"].as_str().unwrap_or("").to_string();
    let who = body["emailAddress"].as_str().unwrap_or("").to_lowercase();
    if kind == "user" && who.split('@').nth(1).is_some_and(|d| d.contains("nogoogle")) {
        let msg = format!(
            "Bad Request. User message: \"You are trying to invite {who}. Since there is no Google account associated with this email address, you must check the 'Notify people' box to invite this recipient.\""
        );
        return error(400, &msg, "invalidSharingRequest");
    }
    s.with(|st| match st.user(&email).drive.files.iter_mut().find(|f| f.id == id) {
        Some(f) => {
            let p = (kind, who);
            if !f.permissions.contains(&p) {
                f.permissions.push(p);
            }
            Json(json!({ "id": format!("perm{}", f.permissions.len()) })).into_response()
        }
        None => not_found(),
    })
}

async fn about(State(s): State<Shared>, User(email): User) -> Response {
    s.read(|st| {
        let used: u64 = st.user(&email).drive.files.iter().filter_map(|f| f.size).sum();
        Json(json!({ "storageQuota": { "limit": (15u64 << 30).to_string(), "usage": (used + (2u64 << 30)).to_string() } })).into_response()
    })
}

#[derive(Deserialize)]
struct UploadQ {
    upload_id: Option<String>,
}

async fn upload_start(State(s): State<Shared>, User(email): User, headers: HeaderMap, Query(q): Query<UploadQ>, body: Bytes) -> Response {
    if q.upload_id.is_some() {
        return error(400, "Bad Request", "badRequest");
    }
    let meta: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let Ok(total) = h("x-upload-content-length").parse::<u64>() else {
        return error(400, "X-Upload-Content-Length is required", "badRequest");
    };
    let sid = new_id(&s);
    s.with(|st| {
        st.user(&email).drive.sessions.insert(sid.clone(), FSession { meta, mime: h("x-upload-content-type"), total, received: 0 });
    });
    let host = h("host");
    let loc = format!("http://{host}/upload/drive/v3/files?uploadType=resumable&upload_id={sid}");
    (StatusCode::OK, [("location", loc)]).into_response()
}

/// `Content-Range: bytes a-b/total` (a part) or `bytes */total` (where does it stand?).
async fn upload_put(State(s): State<Shared>, User(email): User, headers: HeaderMap, Query(q): Query<UploadQ>, body: Bytes) -> Response {
    let Some(sid) = q.upload_id else { return error(400, "upload_id is required", "badRequest") };
    let range = headers.get("content-range").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let id = new_id(&s);
    s.with(|st| {
        let u = st.user(&email);
        let Some(sess) = u.drive.sessions.get_mut(&sid) else { return not_found() };
        let spec = range.strip_prefix("bytes ").unwrap_or("");
        if let Some((part, _total)) = spec.split_once('/')
            && part != "*"
        {
            let start: u64 = part.split('-').next().and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
            if start != sess.received {
                return error(400, "Invalid Content-Range", "badRequest");
            }
            sess.received += body.len() as u64;
        }
        if sess.received >= sess.total {
            let sess = u.drive.sessions.remove(&sid).unwrap();
            let f = FFile {
                id,
                name: sess.meta["name"].as_str().unwrap_or("Untitled").to_string(),
                mime_type: sess.meta["mimeType"].as_str().map(str::to_string).unwrap_or(sess.mime),
                size: Some(sess.total),
                parents: sess.meta["parents"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_string)).collect())
                    .unwrap_or_default(),
                app_properties: serde_json::from_value(sess.meta["appProperties"].clone()).unwrap_or_default(),
                trashed: false,
                created: seed::now_ms(),
                permissions: vec![],
            };
            let v = file_json(&f);
            u.drive.files.push(f);
            return (StatusCode::OK, Json(v)).into_response();
        }
        let mut resp = StatusCode::PERMANENT_REDIRECT.into_response();
        if sess.received > 0
            && let Ok(v) = format!("bytes=0-{}", sess.received - 1).parse()
        {
            resp.headers_mut().insert("range", v);
        }
        resp
    })
}

async fn upload_cancel(State(s): State<Shared>, User(email): User, Query(q): Query<UploadQ>) -> Response {
    let Some(sid) = q.upload_id else { return not_found() };
    s.with(|st| st.user(&email).drive.sessions.remove(&sid));
    StatusCode::from_u16(499).unwrap_or(StatusCode::NO_CONTENT).into_response()
}

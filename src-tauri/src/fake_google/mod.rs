//! In-process fake of the Google APIs we use (Gmail, Calendar, People, Tasks + OAuth endpoints),
//! for demo accounts, development and tests. Speaks Google's JSON shapes, paging and error format.
//!
//! Auth: `Authorization: Bearer demo-<email>` selects the mailbox. A user's data is seeded on first
//! use (deterministic, see each API's `seed`) and the whole state is persisted to a JSON file.
//!
//! Each API lives in its own file and exposes `routes() -> Router<Shared>`, a `Store` type
//! (Serialize + Deserialize + Default) and `seed(email, name) -> Store`.

pub mod calendar;
pub mod gmail;
pub mod gmail_compose;
pub mod imap;
pub mod people;
pub mod seed;
pub mod tasks;

use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{FromRequestParts, Query, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Default, Serialize, Deserialize)]
pub struct FakeState {
    pub users: BTreeMap<String, UserData>,
    /// monotonically increasing counter used for ids / history ids / etags
    pub counter: u64,
}

#[derive(Serialize, Deserialize)]
pub struct UserData {
    pub name: String,
    pub gmail: gmail::Store,
    pub calendar: calendar::Store,
    pub people: people::Store,
    pub tasks: tasks::Store,
}

impl FakeState {
    pub fn next(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    /// The user's data, seeding it on first use.
    pub fn user(&mut self, email: &str) -> &mut UserData {
        if !self.users.contains_key(email) {
            let name = seed::display_name_for(email);
            let data = UserData {
                gmail: gmail::seed(email, &name),
                calendar: calendar::seed(email, &name),
                people: people::seed(email, &name),
                tasks: tasks::seed(email, &name),
                name,
            };
            self.users.insert(email.to_string(), data);
        }
        self.users.get_mut(email).unwrap()
    }
}

pub struct FakeServer {
    pub state: Mutex<FakeState>,
    path: Option<PathBuf>,
    dirty: Mutex<bool>,
    /// injected failures (tests): the next requests whose path contains `path_part` get these
    faults: Mutex<Vec<Fault>>,
}

/// A canned error the fake answers instead of the real handler (rate limits, 5xx).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fault {
    pub status: u16,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub message: String,
    /// substring of the request path this applies to ("" = any)
    #[serde(default)]
    pub path_part: String,
    #[serde(default)]
    pub retry_after: Option<String>,
    #[serde(default = "one")]
    pub count: u32,
}

fn one() -> u32 {
    1
}

pub type Shared = Arc<FakeServer>;

impl FakeServer {
    /// Mutate state and mark it for persisting.
    pub fn with<T>(&self, f: impl FnOnce(&mut FakeState) -> T) -> T {
        let out = f(&mut self.state.lock());
        *self.dirty.lock() = true;
        out
    }

    /// Read-only access.
    pub fn read<T>(&self, f: impl FnOnce(&mut FakeState) -> T) -> T {
        f(&mut self.state.lock())
    }

    fn flush(&self) {
        let Some(path) = &self.path else { return };
        if !std::mem::take(&mut *self.dirty.lock()) {
            return;
        }
        let text = match serde_json::to_string(&*self.state.lock()) {
            Ok(t) => t,
            Err(e) => {
                log::error!("fake google: serialise: {e}");
                return;
            }
        };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// The authenticated user (from `Bearer demo-<email>`).
pub struct User(pub String);

impl<S: Send + Sync> FromRequestParts<S> for User {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let h = parts.headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
        match h.strip_prefix("Bearer demo-") {
            Some(email) if !email.is_empty() => Ok(User(email.to_lowercase())),
            _ => Err(error(401, "Request had invalid authentication credentials.", "authError")),
        }
    }
}

/// Google-style JSON error.
pub fn error(status: u16, message: &str, reason: &str) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, Json(json!({ "error": { "code": status, "message": message, "errors": [{ "message": message, "reason": reason }] } })))
        .into_response()
}

pub fn not_found() -> Response {
    error(404, "Requested entity was not found.", "notFound")
}

/// Start the server on a random loopback port; returns its base URL.
pub async fn start(path: Option<PathBuf>) -> anyhow::Result<String> {
    let state = match &path {
        Some(p) if p.exists() => match std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()) {
            Some(s) => s,
            None => {
                log::warn!("fake google: could not read {}, starting fresh", p.display());
                FakeState::default()
            }
        },
        _ => FakeState::default(),
    };
    let server = Arc::new(FakeServer { state: Mutex::new(state), path, dirty: Mutex::new(false), faults: Mutex::new(Vec::new()) });
    let app = router(server.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    // Gmail IMAP (fast initial download) on its own loopback port, same state
    let imap_url = imap::start(server.clone(), &base).await?;
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log::error!("fake google server stopped: {e}");
        }
    });
    let s2 = server.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            s2.flush();
        }
    });
    SERVERS.lock().insert(base.clone(), server.clone());
    *SERVER.lock() = Some(server);
    log::info!("fake google server at {base}, IMAP at {imap_url}");
    Ok(base)
}

/// The running server (for demo helpers like "deliver a new message").
static SERVER: Mutex<Option<Shared>> = Mutex::new(None);

/// Every server started in this process, by base URL (tests run several in parallel).
static SERVERS: std::sync::LazyLock<Mutex<std::collections::HashMap<String, Shared>>> = std::sync::LazyLock::new(Default::default);

/// The server listening at `base` (what a test's own `AppCtx::fake_base()` returned).
pub fn server_at(base: &str) -> Option<Shared> {
    SERVERS.lock().get(base).cloned()
}

pub fn server() -> Option<Shared> {
    SERVER.lock().clone()
}

pub fn router(server: Shared) -> Router {
    Router::new()
        .merge(gmail::routes())
        .merge(calendar::routes())
        .merge(people::routes())
        .merge(tasks::routes())
        .route("/auth", get(oauth_auth))
        .route("/token", post(oauth_token))
        .route("/revoke", post(oauth_revoke))
        .route("/userinfo", get(userinfo))
        // not Google: queue failures for the next matching requests (tests)
        .route("/x-fake/faults", post(add_faults))
        .layer(axum::middleware::from_fn_with_state(server.clone(), inject_faults))
        .with_state(server)
}

async fn add_faults(State(s): State<Shared>, Json(f): Json<Vec<Fault>>) -> StatusCode {
    s.faults.lock().extend(f);
    StatusCode::NO_CONTENT
}

/// Answer with a queued fault (Google's error JSON + optional Retry-After) if one matches.
async fn inject_faults(State(s): State<Shared>, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let path = req.uri().path().to_string();
    let hit = {
        let mut faults = s.faults.lock();
        let idx = faults.iter().position(|f| path.contains(&f.path_part) && !path.starts_with("/x-fake/"));
        idx.map(|i| {
            let f = faults[i].clone();
            if faults[i].count <= 1 {
                faults.remove(i);
            } else {
                faults[i].count -= 1;
            }
            f
        })
    };
    let Some(f) = hit else { return next.run(req).await };
    let msg = if f.message.is_empty() { "Injected failure".to_string() } else { f.message.clone() };
    let mut resp = error(f.status, &msg, &f.reason);
    if let Some(ra) = &f.retry_after
        && let Ok(v) = axum::http::HeaderValue::from_str(ra)
    {
        resp.headers_mut().insert("retry-after", v);
    }
    resp
}

// ---- fake OAuth: lets the real sign-in code be exercised end to end -------------------------

#[derive(Deserialize)]
struct AuthQuery {
    redirect_uri: String,
    state: String,
    login_hint: Option<String>,
}

async fn oauth_auth(Query(q): Query<AuthQuery>) -> Response {
    let email = q.login_hint.unwrap_or_else(|| "kenny@duvz.demo".into());
    let code = format!("code-{}", email);
    let url = format!("{}?code={}&state={}", q.redirect_uri, urlencoding::encode(&code), urlencoding::encode(&q.state));
    Redirect::to(&url).into_response()
}

async fn oauth_token(body: String) -> Response {
    let form: BTreeMap<String, String> = url::form_urlencoded::parse(body.as_bytes()).into_owned().collect();
    let email = match form.get("grant_type").map(String::as_str) {
        Some("authorization_code") => form.get("code").and_then(|c| c.strip_prefix("code-")).map(str::to_string),
        Some("refresh_token") => form.get("refresh_token").and_then(|c| c.strip_prefix("refresh-")).map(str::to_string),
        _ => None,
    };
    match email {
        Some(e) => Json(json!({
            "access_token": format!("demo-{e}"),
            "expires_in": 3600,
            "refresh_token": format!("refresh-{e}"),
            "token_type": "Bearer",
            "scope": crate::google::auth::SCOPES.join(" "),
        }))
        .into_response(),
        None => (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_grant", "error_description": "Bad Request"}))).into_response(),
    }
}

/// Like Google's: 200 for a token we issued, 400 `invalid_token` for anything else.
async fn oauth_revoke(body: String) -> Response {
    let form: BTreeMap<String, String> = url::form_urlencoded::parse(body.as_bytes()).into_owned().collect();
    let ours = form.get("token").is_some_and(|t| t.starts_with("refresh-") || t.starts_with("demo-"));
    if ours {
        Json(json!({})).into_response()
    } else {
        (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_token", "error_description": "Token expired or revoked"}))).into_response()
    }
}

async fn userinfo(State(s): State<Shared>, User(email): User) -> Json<Value> {
    let name = s.read(|st| st.user(&email).name.clone());
    Json(json!({ "email": email, "name": name, "email_verified": true }))
}

/// Parse a `pageToken` we issued (an offset).
pub fn page_offset(token: &Option<String>) -> usize {
    token.as_deref().and_then(|t| t.parse().ok()).unwrap_or(0)
}

/// Slice `items` by offset/size; returns the page and the next token.
pub fn paginate<T: Clone>(items: &[T], token: &Option<String>, max: usize) -> (Vec<T>, Option<String>) {
    let start = page_offset(token).min(items.len());
    let end = (start + max).min(items.len());
    let next = if end < items.len() { Some(end.to_string()) } else { None };
    (items[start..end].to_vec(), next)
}

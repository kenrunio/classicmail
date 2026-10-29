//! Authenticated HTTP client for one account. Handles bearer tokens (refresh on 401), pacing to
//! Google's per-user quotas (`limits`), retry with exponential back-off on 429 / 403 rate limits /
//! 5xx, and Google's JSON error formats. Base URLs are overridable so demo accounts and tests talk
//! to the in-process fake server.

use std::{sync::Arc, time::Duration};

use reqwest::{Method, StatusCode};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::{
    auth::TokenSource,
    limits::{Family, Limiter, gmail_cost},
};

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Urls {
    pub gmail: String,
    pub gmail_upload: String,
    pub calendar: String,
    pub people: String,
    pub tasks: String,
    pub token: String,
    pub auth: String,
    pub userinfo: String,
    /// Gmail IMAP for the fast initial download: `imaps://host:port`, or `imap://127.0.0.1:port`
    /// (fake server). Empty = not available (e.g. a `dev.googleUrls` override without it).
    #[serde(default)]
    pub imap: String,
}

impl Urls {
    pub fn google() -> Self {
        Urls {
            gmail: "https://gmail.googleapis.com/gmail/v1".into(),
            gmail_upload: "https://gmail.googleapis.com/upload/gmail/v1".into(),
            calendar: "https://www.googleapis.com/calendar/v3".into(),
            people: "https://people.googleapis.com/v1".into(),
            tasks: "https://tasks.googleapis.com/tasks/v1".into(),
            token: "https://oauth2.googleapis.com/token".into(),
            auth: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            userinfo: "https://openidconnect.googleapis.com/v1/userinfo".into(),
            imap: "imaps://imap.gmail.com:993".into(),
        }
    }

    /// All APIs served by one base (the fake server).
    pub fn fake(base: &str) -> Self {
        Urls {
            gmail: format!("{base}/gmail/v1"),
            gmail_upload: format!("{base}/upload/gmail/v1"),
            calendar: format!("{base}/calendar/v3"),
            people: format!("{base}/v1"),
            tasks: format!("{base}/tasks/v1"),
            token: format!("{base}/token"),
            auth: format!("{base}/auth"),
            userinfo: format!("{base}/userinfo"),
            // the fake IMAP server started next to this fake HTTP server (none: IMAP unavailable)
            imap: crate::fake_google::imap::url_for_base(base).unwrap_or_default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// `reason` is Google's machine-readable cause: `errors[0].reason` (Gmail / Calendar / Tasks),
    /// else the `ErrorInfo.reason` in `details` (People, e.g. EXPIRED_SYNC_TOKEN), else `status`
    /// (e.g. FAILED_PRECONDITION).
    #[error("Google API error {status}: {message}")]
    Http { status: u16, message: String, reason: Option<String> },
    #[error("network error: {0}")]
    Network(String),
    #[error("not signed in: {0}")]
    Auth(String),
}

impl ApiError {
    pub fn status(&self) -> Option<u16> {
        match self {
            ApiError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }
    pub fn is_not_found(&self) -> bool {
        self.status() == Some(404)
    }
    pub fn reason(&self) -> Option<&str> {
        match self {
            ApiError::Http { reason, .. } => reason.as_deref(),
            _ => None,
        }
    }
    /// Quota / rate-limit answers: 429, or 403 with a rate-limit reason (Gmail, Calendar).
    pub fn is_rate_limited(&self) -> bool {
        match self {
            ApiError::Http { status: 429, .. } => true,
            ApiError::Http { status: 403, reason, .. } => is_rate_limit_reason(reason.as_deref()),
            _ => false,
        }
    }
    /// A client error that retrying won't fix (bad request, forbidden, gone). Rate limits aren't.
    pub fn is_permanent(&self) -> bool {
        match self {
            ApiError::Http { status: 400 | 404 | 410, .. } => true,
            ApiError::Http { status: 403, .. } => !self.is_rate_limited(),
            _ => false,
        }
    }
}

fn is_rate_limit_reason(reason: Option<&str>) -> bool {
    matches!(reason, Some("rateLimitExceeded" | "userRateLimitExceeded" | "RATE_LIMIT_EXCEEDED" | "RESOURCE_EXHAUSTED"))
}

/// Status of an `anyhow::Error` that wraps an `ApiError`, if any.
pub fn api_status(e: &anyhow::Error) -> Option<u16> {
    e.downcast_ref::<ApiError>().and_then(|a| a.status())
}

/// Google's error reason of an `anyhow::Error` that wraps an `ApiError`, if any.
pub fn api_reason(e: &anyhow::Error) -> Option<&str> {
    e.downcast_ref::<ApiError>().and_then(ApiError::reason)
}

/// `ApiError::is_permanent` of an `anyhow::Error` (false for anything that isn't an ApiError).
pub fn is_permanent(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ApiError>().is_some_and(ApiError::is_permanent)
}

#[derive(Clone)]
pub struct Api {
    pub account_id: i64,
    pub email: String,
    pub urls: Arc<Urls>,
    http: reqwest::Client,
    tokens: Arc<dyn TokenSource>,
    limiter: Arc<Limiter>,
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        // Google only gzips responses when the user agent also contains "gzip"
        .user_agent(concat!("ClassicMail/", env!("CARGO_PKG_VERSION"), " (gzip)"))
        // whole request incl. body: big attachments on a slow line need minutes
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(20))
        .gzip(true)
        // HTTP/1.1 only: h2 0.4.20+ drops the whole connection ("too_many_data_frames",
        // ENHANCE_YOUR_CALM) when Gmail streams a big attachment in many small padded DATA
        // frames, which surfaced as "error decoding response body" on every PDF over a few MB
        .http1_only()
        .build()
        .expect("http client")
}

/// "error decoding response body: <the real cause>" instead of just the outer message.
fn error_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        let m = s.to_string();
        if !out.contains(&m) {
            out.push_str(": ");
            out.push_str(&m);
        }
        src = s.source();
    }
    out
}

/// A request body.
pub enum Body {
    None,
    Json(Value),
    Raw { content_type: String, bytes: Vec<u8> },
}

impl Api {
    pub fn new(account_id: i64, email: String, urls: Urls, http: reqwest::Client, tokens: Arc<dyn TokenSource>) -> Self {
        Api { account_id, email, urls: Arc::new(urls), http, tokens, limiter: Arc::new(Limiter::unlimited()) }
    }

    /// Pace calls to Google's per-user quotas (real accounts; demo accounts stay unlimited).
    pub fn with_limiter(mut self, limiter: Limiter) -> Self {
        self.limiter = Arc::new(limiter);
        self
    }

    /// The current OAuth access token (IMAP XOAUTH2); `force_refresh` after the server refused it.
    pub async fn access_token(&self, force_refresh: bool) -> anyhow::Result<String> {
        self.tokens.access_token(&self.http, force_refresh).await
    }

    /// Which quota a URL counts against, and what it costs.
    fn quota_of(&self, method: &Method, url: &str) -> Option<(Family, f64)> {
        let u = &self.urls;
        if url.starts_with(&u.gmail) || url.starts_with(&u.gmail_upload) {
            let path = url.split_once("/users/me/").map(|(_, p)| p).unwrap_or("");
            Some((Family::Gmail, gmail_cost(method, path)))
        } else if url.starts_with(&u.calendar) {
            Some((Family::Calendar, 1.0))
        } else if url.starts_with(&u.tasks) {
            Some((Family::Tasks, 1.0))
        } else if url.starts_with(&u.people) {
            Some((Family::People, 1.0))
        } else {
            None
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, url: &str, query: &[(&str, String)]) -> anyhow::Result<T> {
        self.call(Method::GET, url, query, Body::None).await
    }

    pub async fn post<T: DeserializeOwned>(&self, url: &str, query: &[(&str, String)], body: Value) -> anyhow::Result<T> {
        self.call(Method::POST, url, query, Body::Json(body)).await
    }

    pub async fn patch<T: DeserializeOwned>(&self, url: &str, query: &[(&str, String)], body: Value) -> anyhow::Result<T> {
        self.call(Method::PATCH, url, query, Body::Json(body)).await
    }

    pub async fn put<T: DeserializeOwned>(&self, url: &str, query: &[(&str, String)], body: Value) -> anyhow::Result<T> {
        self.call(Method::PUT, url, query, Body::Json(body)).await
    }

    /// Every page of a list call that answers `{ items, nextPageToken }` (Calendar, Tasks).
    pub async fn get_all_items<T: DeserializeOwned>(&self, url: &str, query: &[(&str, String)]) -> anyhow::Result<Vec<T>> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page<T> {
            #[serde(default = "Vec::new")]
            items: Vec<T>,
            #[serde(default)]
            next_page_token: Option<String>,
        }
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut q = query.to_vec();
            if let Some(t) = &token {
                q.push(("pageToken", t.clone()));
            }
            let page: Page<T> = self.get(url, &q).await?;
            out.extend(page.items);
            match page.next_page_token {
                Some(t) if !t.is_empty() => token = Some(t),
                _ => break,
            }
        }
        Ok(out)
    }

    pub async fn delete(&self, url: &str, query: &[(&str, String)]) -> anyhow::Result<()> {
        let _: Value = self.call(Method::DELETE, url, query, Body::None).await?;
        Ok(())
    }

    /// Generic JSON call. Empty response bodies deserialize as `null`.
    pub async fn call<T: DeserializeOwned>(&self, method: Method, url: &str, query: &[(&str, String)], body: Body) -> anyhow::Result<T> {
        let bytes = self.call_bytes(method, url, query, body).await?;
        let v: Value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes)? };
        Ok(serde_json::from_value(v)?)
    }

    /// Like `call` but returns the raw response body.
    pub async fn call_bytes(&self, method: Method, url: &str, query: &[(&str, String)], body: Body) -> anyhow::Result<Vec<u8>> {
        let mut attempt = 0u32;
        let mut refreshed = false;
        let quota = self.quota_of(&method, url);
        // uploads: allow for ~50 KB/s on top of the normal timeout
        let upload_timeout = match &body {
            Body::Raw { bytes, .. } if bytes.len() > 1_000_000 => Some(Duration::from_secs(300 + bytes.len() as u64 / 50_000)),
            _ => None,
        };
        loop {
            attempt += 1;
            if let Some((fam, cost)) = quota {
                self.limiter.acquire(fam, cost).await;
            }
            let token = self.tokens.access_token(&self.http, false).await?;
            let mut req = self.http.request(method.clone(), url).bearer_auth(&token).query(query);
            if let Some(t) = upload_timeout {
                req = req.timeout(t);
            }
            req = match &body {
                Body::None => req,
                Body::Json(v) => req.json(v),
                Body::Raw { content_type, bytes } => req.header(reqwest::header::CONTENT_TYPE, content_type.as_str()).body(bytes.clone()),
            };
            let res = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    // a POST that may have reached Google (timeout, reset) is not repeated: it could
                    // send a message twice or create a duplicate event / contact / task. For a POST
                    // only a failed connection (nothing was sent) is tried again here.
                    let safe = e.is_connect() || is_idempotent(&method);
                    if safe && attempt < 4 {
                        tokio::time::sleep(backoff(attempt)).await;
                        continue;
                    }
                    // without the URL: its query can hold a search, an address
                    return Err(ApiError::Network(error_chain(&e.without_url())).into());
                }
            };
            let status = res.status();
            if status.is_success() {
                match res.bytes().await {
                    Ok(b) => return Ok(b.to_vec()),
                    // the connection broke while the body was arriving: a read can safely be repeated
                    Err(e) if is_idempotent(&method) && attempt < 4 => {
                        log::warn!(
                            "google {method} {}: body read failed ({}), retrying",
                            endpoint(&self.urls, url),
                            error_chain(&e.without_url())
                        );
                        tokio::time::sleep(backoff(attempt)).await;
                        continue;
                    }
                    Err(e) => return Err(ApiError::Network(error_chain(&e.without_url())).into()),
                }
            }
            let retry_after = res.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(parse_retry_after);
            let text = res.text().await.unwrap_or_default();
            if status == StatusCode::UNAUTHORIZED && !refreshed {
                refreshed = true;
                self.tokens.access_token(&self.http, true).await?;
                continue;
            }
            let (message, reason) = parse_google_error(&text);
            let err = ApiError::Http { status: status.as_u16(), message, reason };
            // a rate-limit answer means Google did nothing, so any call may be repeated. A 5xx can
            // come after the work was done: a POST (send, insert) is not repeated but reported,
            // for the caller's queue (or the user) to decide
            let retry = err.is_rate_limited() || (status.is_server_error() && is_idempotent(&method));
            if retry && attempt < 6 {
                let wait = retry_after.unwrap_or_else(|| backoff(attempt));
                // "retry in an hour" (e.g. Gmail's daily sending limit): report it now instead of
                // holding up the whole sync cycle
                if wait <= Duration::from_secs(90) {
                    log::debug!("google {method} {}: {status} {:?}, retrying in {wait:?}", endpoint(&self.urls, url), err.reason());
                    tokio::time::sleep(wait).await;
                    continue;
                }
            }
            return Err(err.into());
        }
    }
}

/// A URL for the log: the API and the words of its path (`gmail users/me/messages/attachments`),
/// without ids (a calendar id is often an address) or the query (a search).
fn endpoint(urls: &Urls, url: &str) -> String {
    let apis = [
        ("gmail upload", &urls.gmail_upload),
        ("gmail", &urls.gmail),
        ("calendar", &urls.calendar),
        ("tasks", &urls.tasks),
        ("people", &urls.people),
    ];
    let (api, rest) = apis.into_iter().find_map(|(name, base)| Some((name, url.strip_prefix(base.as_str())?))).unwrap_or(("other", ""));
    let path = rest.split(['?', '#']).next().unwrap_or("");
    let words: Vec<&str> =
        path.split('/').filter(|w| !w.is_empty() && w.len() <= 30 && w.chars().all(|c| c.is_ascii_alphabetic() || c == ':')).collect();
    if words.is_empty() { api.to_string() } else { format!("{api} {}", words.join("/")) }
}

fn is_idempotent(m: &Method) -> bool {
    matches!(*m, Method::GET | Method::HEAD | Method::PUT | Method::DELETE | Method::PATCH)
}

/// `Retry-After`: delay-seconds or an HTTP date.
fn parse_retry_after(v: &str) -> Option<Duration> {
    let v = v.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let when = chrono::DateTime::parse_from_rfc2822(v).ok()?;
    let ms = (when.timestamp_millis() - chrono::Utc::now().timestamp_millis()).max(0);
    Some(Duration::from_millis(ms as u64))
}

/// Google's advice: 1 s, 2 s, 4 s … plus up to 1 s of random jitter.
fn backoff(attempt: u32) -> Duration {
    let base = 500u64 * 2u64.pow(attempt.min(6));
    Duration::from_millis(base + rand::random::<u64>() % 1000)
}

/// Google's error JSON. Three shapes exist:
/// * Gmail / Calendar / Tasks: `{"error": {"code", "message", "errors": [{"reason", "domain"}]}}`
/// * People (google.rpc.Status): `{"error": {"code", "message", "status": "FAILED_PRECONDITION",
///   "details": [{"@type": ".../google.rpc.ErrorInfo", "reason": "EXPIRED_SYNC_TOKEN"}]}}`
/// * OAuth token endpoint: `{"error": "invalid_grant", "error_description": "..."}`
pub(crate) fn parse_google_error(text: &str) -> (String, Option<String>) {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        let err = &v["error"];
        if err.is_object() {
            let msg = err["message"].as_str().unwrap_or("").to_string();
            let reason = err["errors"][0]["reason"]
                .as_str()
                .or_else(|| err["details"].as_array()?.iter().find_map(|d| d["reason"].as_str()))
                .or_else(|| err["status"].as_str())
                .map(str::to_string);
            return (msg, reason);
        }
        if let Some(s) = err.as_str() {
            return (v["error_description"].as_str().unwrap_or(s).to_string(), Some(s.to_string()));
        }
    }
    (text.chars().take(300).collect(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_error_shapes() {
        let gmail = r#"{"error":{"code":403,"message":"User Rate Limit Exceeded","errors":[{"message":"User Rate Limit Exceeded","domain":"usageLimits","reason":"userRateLimitExceeded"}]}}"#;
        assert_eq!(parse_google_error(gmail), ("User Rate Limit Exceeded".into(), Some("userRateLimitExceeded".into())));
        let people = r#"{"error":{"code":400,"message":"Sync token is expired. Clear local cache and retry call without the sync token.","status":"FAILED_PRECONDITION","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"EXPIRED_SYNC_TOKEN","domain":"people.googleapis.com"}]}}"#;
        assert_eq!(parse_google_error(people).1.as_deref(), Some("EXPIRED_SYNC_TOKEN"));
        let etag = r#"{"error":{"code":400,"message":"Request person.etag is different than the current person.etag.","status":"FAILED_PRECONDITION"}}"#;
        assert_eq!(parse_google_error(etag).1.as_deref(), Some("FAILED_PRECONDITION"));
        let oauth = r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#;
        assert_eq!(parse_google_error(oauth), ("Token has been expired or revoked.".into(), Some("invalid_grant".into())));
    }

    #[test]
    fn endpoints_for_the_log_have_no_ids() {
        let u = Urls::google();
        assert_eq!(
            endpoint(&u, "https://gmail.googleapis.com/gmail/v1/users/me/messages/18c2f/attachments/ANGjdJ8"),
            "gmail users/me/messages/attachments"
        );
        assert_eq!(
            endpoint(&u, "https://www.googleapis.com/calendar/v3/calendars/dana%40harbor.example/events?q=lunch"),
            "calendar calendars/events"
        );
        assert_eq!(endpoint(&u, "https://people.googleapis.com/v1/people:batchUpdateContacts"), "people people:batchUpdateContacts");
        assert_eq!(endpoint(&u, "https://example.com/x"), "other");
    }

    #[test]
    fn rate_limits_are_not_permanent() {
        let e = |status, reason: Option<&str>| ApiError::Http { status, message: String::new(), reason: reason.map(str::to_string) };
        assert!(e(403, Some("userRateLimitExceeded")).is_rate_limited());
        assert!(e(403, Some("rateLimitExceeded")).is_rate_limited());
        assert!(e(429, None).is_rate_limited());
        assert!(!e(403, Some("userRateLimitExceeded")).is_permanent());
        assert!(e(403, Some("forbidden")).is_permanent());
        assert!(e(403, Some("insufficientPermissions")).is_permanent());
        assert!(e(400, None).is_permanent());
        assert!(!e(500, None).is_permanent());
        assert!(!e(429, None).is_permanent());
    }

    async fn demo_api(email: &str) -> (Arc<crate::ctx::AppCtx>, crate::db::tempdir::TempDirGuard, Api, String) {
        let (ctx, g) = crate::ctx::AppCtx::for_test().unwrap();
        let a = crate::accounts::upsert(&ctx.db, email, "RL", "demo").await.unwrap();
        let api = ctx.api(a.id).await.unwrap();
        let base = ctx.fake_base().await.unwrap();
        (ctx, g, api, base)
    }

    async fn inject(base: &str, faults: Value) {
        let r = reqwest::Client::new().post(format!("{base}/x-fake/faults")).json(&faults).send().await.unwrap();
        assert!(r.status().is_success());
    }

    /// Gmail answers 403 userRateLimitExceeded or 429 (sometimes with Retry-After) under load:
    /// both are retried; a plain 403 is not.
    #[tokio::test]
    async fn rate_limits_are_retried_with_retry_after() {
        let (_ctx, _g, api, base) = demo_api("rl@duvz.demo").await;
        inject(
            &base,
            serde_json::json!([
                { "status": 403, "reason": "userRateLimitExceeded", "message": "User Rate Limit Exceeded", "pathPart": "/labels" },
                { "status": 429, "reason": "rateLimitExceeded", "message": "Too many concurrent requests for user", "pathPart": "/labels", "retryAfter": "1" },
            ]),
        )
        .await;
        let t0 = std::time::Instant::now();
        let labels = api.gmail_labels().await.unwrap();
        assert!(!labels.is_empty());
        assert!(t0.elapsed() >= Duration::from_millis(1900), "backoff ≥1 s, then Retry-After 1 s: {:?}", t0.elapsed());
        inject(&base, serde_json::json!([{ "status": 403, "reason": "forbidden", "pathPart": "/labels" }])).await;
        let t0 = std::time::Instant::now();
        let e = api.gmail_labels().await.unwrap_err();
        assert_eq!(api_status(&e), Some(403));
        assert!(is_permanent(&e));
        assert!(t0.elapsed() < Duration::from_millis(900), "not retried");
        // an hour-long Retry-After (daily sending limit) is reported at once instead of waited out
        inject(&base, serde_json::json!([{ "status": 429, "reason": "rateLimitExceeded", "pathPart": "/labels", "retryAfter": "3600" }]))
            .await;
        let t0 = std::time::Instant::now();
        let e = api.gmail_labels().await.unwrap_err();
        assert_eq!(api_status(&e), Some(429));
        assert!(t0.elapsed() < Duration::from_millis(900));
    }

    /// A 5xx after a send may come after Google sent it: a POST is reported, not repeated (no
    /// duplicate e-mail); a read is still retried. A rate limit is retried for a POST too.
    #[tokio::test]
    async fn server_errors_are_not_retried_for_a_post() {
        let (_ctx, _g, api, base) = demo_api("s5@duvz.demo").await;
        let raw = "From: s5@duvz.demo\r\nTo: x@duvz.demo\r\nSubject: once\r\n\r\nhi\r\n";
        inject(&base, serde_json::json!([{ "status": 500, "reason": "backendError", "pathPart": "/messages/send" }])).await;
        let e = api.gmail_send(raw.as_bytes(), None).await.unwrap_err();
        assert_eq!(api_status(&e), Some(500));
        inject(&base, serde_json::json!([{ "status": 503, "pathPart": "/labels" }])).await;
        assert!(api.gmail_labels().await.is_ok(), "a GET is retried after a 5xx");
        inject(
            &base,
            serde_json::json!([{ "status": 429, "reason": "rateLimitExceeded", "pathPart": "/messages/send", "retryAfter": "0" }]),
        )
        .await;
        assert!(api.gmail_send(raw.as_bytes(), None).await.is_ok(), "a rate-limited POST is retried");
    }

    /// A local change that hit Google's rate limit is retried later, not thrown away.
    #[tokio::test]
    async fn pending_ops_survive_rate_limits() {
        let (ctx, _g, api, _base) = demo_api("rl2@duvz.demo").await;
        let acct = api.account_id;
        let id = ctx
            .db
            .write(move |c| crate::sync::pending::enqueue(c, acct, "mail", "modify", &serde_json::json!({ "ids": ["x"] })))
            .await
            .unwrap();
        let op = crate::sync::pending::due(&ctx.db, acct, "mail").await.unwrap().into_iter().find(|o| o.id == id).unwrap();
        let rl: anyhow::Error =
            ApiError::Http { status: 403, message: "User Rate Limit Exceeded".into(), reason: Some("userRateLimitExceeded".into()) }.into();
        assert!(!crate::sync::pending::failed(&ctx.db, &op, &rl).await.unwrap(), "kept for a retry");
        assert_eq!(crate::sync::pending::count(&ctx.db, acct, "mail").await.unwrap(), 1);
        let forbidden: anyhow::Error = ApiError::Http { status: 403, message: "Forbidden".into(), reason: Some("forbidden".into()) }.into();
        assert!(crate::sync::pending::failed(&ctx.db, &op, &forbidden).await.unwrap(), "given up");
    }

    #[test]
    fn retry_after_seconds_or_date() {
        assert_eq!(parse_retry_after("7"), Some(Duration::from_secs(7)));
        let later = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc2822();
        let d = parse_retry_after(&later).unwrap();
        assert!(d > Duration::from_secs(25) && d <= Duration::from_secs(31));
        assert_eq!(parse_retry_after("soon"), None);
    }
}

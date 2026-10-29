//! A fake model provider for tests and demo mode — never a real AI call. Speaks both wire formats:
//! Gemini `POST /v1beta/models/{model}:generateContent` (key in `x-goog-api-key`) and Claude
//! `POST /v1/messages` (key in `x-api-key`).
//!
//! Behaviour (deterministic):
//! * a translation prompt (`{"target": "English", "texts": [...]}`) → a JSON array where each
//!   text is prefixed with "[EN] " (first two letters of the target); a batch containing
//!   `DROP_MARKER` comes back one item short (exercises the length check / fallback);
//! * with tools offered (`ai::complete_json_tools`), an AI Review instruction about free time /
//!   a slot / the calendar first calls `find_free_time` (today to a week on), one about a task /
//!   due date `find_tasks`; with the result back, the instruction becomes the first slot's
//!   "when" / the first task's title and due date ("[?]" when there is none);
//! * anything else → "OK".
//!
//! Keys: `FAKE_KEY` works; `QUOTA_KEY` → HTTP 429; anything else → the provider's bad-key error.

use axum::{
    Json, Router,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};

pub const FAKE_KEY: &str = "demo-ai-key";
pub const QUOTA_KEY: &str = "demo-ai-quota";
pub const DROP_MARKER: &str = "<<fake-drop>>";

pub async fn start() -> anyhow::Result<String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let app = Router::new().route("/v1beta/models/{model}", post(gemini)).route("/v1/messages", post(claude));
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log::error!("fake AI provider stopped: {e}");
        }
    });
    log::info!("fake AI provider at {base}");
    Ok(base)
}

/// The fake "model": translation prompts → JSON array, compose tasks (`ai::compose`) → their
/// JSON, anything else → OK.
pub fn answer(prompt: &str) -> String {
    let parsed = prompt.find('{').and_then(|i| serde_json::from_str::<Value>(&prompt[i..]).ok());
    if let Some(v) = &parsed
        && let Some(task) = v["task"].as_str()
    {
        return compose_answer(task, v).to_string();
    }
    if let Some(v) = parsed
        && let Some(texts) = v["texts"].as_array()
    {
        let target = v["target"].as_str().unwrap_or("English");
        let tag: String = target.chars().take(2).collect::<String>().to_uppercase();
        let mut out: Vec<String> = texts.iter().map(|t| format!("[{tag}] {}", t.as_str().unwrap_or(""))).collect();
        if texts.iter().any(|t| t.as_str().is_some_and(|s| s.contains(DROP_MARKER))) {
            out.pop();
        }
        return Value::from(out).to_string();
    }
    "OK".into()
}

/// * `compose.translate` → the HTML with "[HE] " (first two letters of the language) at the start
///   of every paragraph;
/// * `compose.review` → every `{x}` replaced by `X` upper-cased, except merge `fields` and
///   instructions containing `FAKE-KEEP`, which survive (exercises the retry / partial result);
/// * `compose.draft` → two paragraphs, the first naming the newest sender's address.
fn compose_answer(task: &str, v: &Value) -> Value {
    let html = v["html"].as_str().unwrap_or("");
    match task {
        "compose.translate" => {
            let tag: String = v["language"].as_str().unwrap_or("English").chars().take(2).collect::<String>().to_uppercase();
            let out = if html.contains("<p") { regex_lite_paragraphs(html, &format!("[{tag}] ")) } else { format!("[{tag}] {html}") };
            json!({ "html": out })
        }
        "compose.review" => {
            let key = crate::mail::merge::fill::key_norm;
            let fields: Vec<String> =
                v["fields"].as_array().map(|a| a.iter().filter_map(|f| f.as_str()).map(key).collect()).unwrap_or_default();
            let mut out = String::new();
            let mut rest = html;
            while let Some(i) = rest.find('{') {
                match rest[i..].find('}') {
                    Some(e) if !rest[i + 1..i + e].contains("FAKE-KEEP") && !fields.contains(&key(&rest[i + 1..i + e])) => {
                        out.push_str(&rest[..i]);
                        out.push_str(&rest[i + 1..i + e].to_uppercase());
                        rest = &rest[i + e + 1..];
                    }
                    Some(e) => {
                        out.push_str(&rest[..i + e + 1]);
                        rest = &rest[i + e + 1..];
                    }
                    None => break,
                }
            }
            out.push_str(rest);
            json!({ "html": out })
        }
        "compose.draft" => {
            let thread = v["thread"].as_str().unwrap_or("");
            let from = thread.lines().next().unwrap_or("").trim_start_matches("From: ");
            json!({ "paragraphs": [format!("Thanks for your note ({from})."), "I'll get back to you by [date].\nBest,"] })
        }
        _ => json!({}),
    }
}

/// Insert `prefix` after every opening `<p…>` tag.
fn regex_lite_paragraphs(html: &str, prefix: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find("<p") {
        let after = &rest[i + 2..];
        if !(after.starts_with('>') || after.starts_with(' ')) {
            out.push_str(&rest[..i + 2]);
            rest = after;
            continue;
        }
        let Some(e) = rest[i..].find('>') else { break };
        out.push_str(&rest[..i + e + 1]);
        out.push_str(prefix);
        rest = &rest[i + e + 1..];
    }
    out.push_str(rest);
    out
}

/// With tools: the call to make first, or (results in hand) the final answer.
enum ToolReply {
    Call(&'static str, Value),
    Text(String),
}

fn tool_reply(prompt: &str, result: Option<Value>) -> ToolReply {
    let v: Value = prompt.find('{').and_then(|i| serde_json::from_str(&prompt[i..]).ok()).unwrap_or(Value::Null);
    let html = v["html"].as_str().unwrap_or("");
    let lower = html.to_lowercase();
    let wants_slot = ["free", "slot", "calendar"].iter().any(|w| lower.contains(w));
    let wants_task = ["task", "due"].iter().any(|w| lower.contains(w));
    if v["task"] != "compose.review" || !(wants_slot || wants_task) {
        return ToolReply::Text(answer(prompt));
    }
    let Some(r) = result else {
        if wants_slot {
            let today = chrono::NaiveDate::parse_from_str(v["today_date"].as_str().unwrap_or(""), "%Y-%m-%d").unwrap_or_default();
            let to = today + chrono::Duration::days(7);
            return ToolReply::Call("find_free_time", json!({ "from": today.to_string(), "to": to.to_string() }));
        }
        return ToolReply::Call("find_tasks", json!({}));
    };
    let fill = r["slots"][0]["when"]
        .as_str()
        .map(str::to_string)
        .or_else(|| r["tasks"][0]["title"].as_str().map(|t| format!("{t} (due {})", r["tasks"][0]["due"].as_str().unwrap_or("?"))))
        .unwrap_or_else(|| "[?]".into());
    // every {instruction} becomes what was looked up
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('{') {
        let Some(e) = rest[i..].find('}') else { break };
        out.push_str(&rest[..i]);
        out.push_str(&fill);
        rest = &rest[i + e + 1..];
    }
    out.push_str(rest);
    ToolReply::Text(json!({ "html": out }).to_string())
}

fn key(headers: &HeaderMap, name: &str) -> String {
    headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
}

async fn gemini(Path(model): Path<String>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if !model.ends_with(":generateContent") {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": { "code": 404, "message": "not found", "status": "NOT_FOUND" } })))
            .into_response();
    }
    match key(&headers, "x-goog-api-key").as_str() {
        FAKE_KEY => {}
        QUOTA_KEY => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({ "error": { "code": 429, "message": "Resource has been exhausted (e.g. check quota).", "status": "RESOURCE_EXHAUSTED" } })),
            )
                .into_response();
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": { "code": 400, "message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT",
                    "details": [{ "@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "API_KEY_INVALID", "domain": "googleapis.com" }] } })),
            )
                .into_response();
        }
    }
    let prompt = body["contents"][0]["parts"][0]["text"].as_str().unwrap_or("");
    if body.get("tools").is_some() {
        let last = body["contents"].as_array().and_then(|c| c.last()).cloned().unwrap_or(Value::Null);
        let result = last["parts"][0]["functionResponse"]["response"]["result"].clone();
        let parts = match tool_reply(prompt, (!result.is_null()).then_some(result)) {
            ToolReply::Call(name, args) => json!([{ "functionCall": { "name": name, "args": args } }]),
            ToolReply::Text(t) => json!([{ "text": t }]),
        };
        return Json(json!({ "candidates": [{ "content": { "role": "model", "parts": parts }, "finishReason": "STOP", "index": 0 }] }))
            .into_response();
    }
    Json(json!({
        "candidates": [{ "content": { "role": "model", "parts": [{ "text": answer(prompt) }] }, "finishReason": "STOP", "index": 0 }],
        "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 10, "totalTokenCount": 20 },
        "modelVersion": model.trim_end_matches(":generateContent"),
    }))
    .into_response()
}

async fn claude(headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let err = |status: StatusCode, kind: &str, msg: &str| {
        (status, Json(json!({ "type": "error", "error": { "type": kind, "message": msg } }))).into_response()
    };
    match key(&headers, "x-api-key").as_str() {
        FAKE_KEY => {}
        QUOTA_KEY => {
            return err(StatusCode::TOO_MANY_REQUESTS, "rate_limit_error", "Number of request tokens has exceeded your rate limit.");
        }
        _ => return err(StatusCode::UNAUTHORIZED, "authentication_error", "invalid x-api-key"),
    }
    if headers.get("anthropic-version").is_none() {
        return err(StatusCode::BAD_REQUEST, "invalid_request_error", "anthropic-version header is required");
    }
    let prompt = match &body["messages"][0]["content"] {
        Value::String(s) => s.clone(),
        other => other[0]["text"].as_str().unwrap_or("").to_string(),
    };
    if body.get("tools").is_some() {
        let last = body["messages"].as_array().and_then(|m| m.last()).cloned().unwrap_or(Value::Null);
        let result = last["content"][0]["content"].as_str().and_then(|s| serde_json::from_str::<Value>(s).ok());
        let (content, stop) = match tool_reply(&prompt, result) {
            ToolReply::Call(name, args) => (json!([{ "type": "tool_use", "id": "toolu_fake", "name": name, "input": args }]), "tool_use"),
            ToolReply::Text(t) => (json!([{ "type": "text", "text": t }]), "end_turn"),
        };
        return Json(json!({ "id": "msg_fake", "type": "message", "role": "assistant", "model": body["model"], "content": content, "stop_reason": stop })).into_response();
    }
    Json(json!({
        "id": "msg_fake", "type": "message", "role": "assistant", "model": body["model"],
        "content": [{ "type": "text", "text": answer(&prompt) }],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": { "input_tokens": 10, "output_tokens": 10 },
    }))
    .into_response()
}

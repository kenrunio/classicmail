//! Request building and response parsing for the two model providers (pure functions, so the
//! JSON shapes are unit-tested without any network).
//!
//! * Gemini: `POST {base}/v1beta/models/{model}:generateContent`, header `x-goog-api-key`.
//! * Claude: `POST {base}/v1/messages`, headers `x-api-key` + `anthropic-version: 2023-06-01`.
//!
//! The API key only ever goes into a request header; it is never part of a URL, an error
//! message or a log line.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Gemini,
    Claude,
}

impl Provider {
    pub fn parse(s: &str) -> Provider {
        if s.eq_ignore_ascii_case("claude") || s.eq_ignore_ascii_case("anthropic") { Provider::Claude } else { Provider::Gemini }
    }
    pub fn id(self) -> &'static str {
        match self {
            Provider::Gemini => "gemini",
            Provider::Claude => "claude",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Provider::Gemini => "Google Gemini",
            Provider::Claude => "Anthropic Claude",
        }
    }
    /// Fast, inexpensive default model per provider.
    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Gemini => "gemini-2.5-flash",
            Provider::Claude => "claude-haiku-5-5",
        }
    }
    pub fn default_base(self) -> &'static str {
        match self {
            Provider::Gemini => "https://generativelanguage.googleapis.com",
            Provider::Claude => "https://api.anthropic.com",
        }
    }
    /// Keyring entry (google::auth::store_secret) holding this provider's API key.
    pub fn secret_name(self) -> &'static str {
        match self {
            Provider::Gemini => "ai.geminiKey",
            Provider::Claude => "ai.anthropicKey",
        }
    }
}

pub const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Clone, Debug)]
pub struct CompleteOpts {
    /// ask for a JSON reply (Gemini responseMimeType; Claude: instruction in the system prompt)
    pub json: bool,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub timeout: std::time::Duration,
}

impl Default for CompleteOpts {
    fn default() -> Self {
        CompleteOpts { json: false, max_tokens: 8192, temperature: Some(0.2), timeout: std::time::Duration::from_secs(60) }
    }
}

/// A ready-to-send request: URL, headers (excluding the key) and JSON body.
#[derive(Debug)]
pub struct BuiltRequest {
    pub url: String,
    /// header carrying the key
    pub key_header: &'static str,
    pub headers: Vec<(&'static str, &'static str)>,
    pub body: Value,
}

pub const JSON_ONLY: &str = "Reply with valid JSON only — no prose, no Markdown code fences.";

pub fn build_request(provider: Provider, base: &str, model: &str, system: &str, prompt: &str, opts: &CompleteOpts) -> BuiltRequest {
    let base = base.trim_end_matches('/');
    match provider {
        Provider::Gemini => {
            let mut gcfg = json!({ "maxOutputTokens": opts.max_tokens });
            if let Some(t) = opts.temperature {
                gcfg["temperature"] = json!(t);
            }
            if opts.json {
                gcfg["responseMimeType"] = json!("application/json");
            }
            // 2.5 Flash thinks by default; simple text tasks don't need it (cheaper, faster)
            if model.contains("flash") {
                gcfg["thinkingConfig"] = json!({ "thinkingBudget": 0 });
            }
            let mut body = json!({
                "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
                "generationConfig": gcfg,
            });
            if !system.is_empty() {
                body["systemInstruction"] = json!({ "parts": [{ "text": system }] });
            }
            BuiltRequest {
                url: format!("{base}/v1beta/models/{}:generateContent", urlencoding::encode(model)),
                key_header: "x-goog-api-key",
                headers: vec![],
                body,
            }
        }
        Provider::Claude => {
            let sys = match (system.is_empty(), opts.json) {
                (true, false) => String::new(),
                (true, true) => JSON_ONLY.to_string(),
                (false, false) => system.to_string(),
                (false, true) => format!("{system}\n\n{JSON_ONLY}"),
            };
            let mut body = json!({
                "model": model,
                "max_tokens": opts.max_tokens,
                "messages": [{ "role": "user", "content": prompt }],
                // short utility tasks: keep thinking light
                "output_config": { "effort": "low" },
            });
            if !sys.is_empty() {
                body["system"] = json!(sys);
            }
            BuiltRequest {
                url: format!("{base}/v1/messages"),
                key_header: "x-api-key",
                headers: vec![("anthropic-version", ANTHROPIC_VERSION)],
                body,
            }
        }
    }
}

/// Why a call failed, in words the user can act on. `retryable` = 5xx / overloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub code: &'static str,
    pub message: String,
    pub status: u16,
    pub retryable: bool,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProviderError {}

/// The provider's own error text (`error.message`, the same field for both), cut to 300
/// characters. Neither provider puts the key in it; nothing is filtered out.
fn provider_message(body: &Value) -> String {
    body["error"]["message"].as_str().unwrap_or("").trim().chars().take(300).collect()
}

/// Map an HTTP error response to a clear error.
pub fn classify_error(provider: Provider, status: u16, body: &Value) -> ProviderError {
    let name = provider.label();
    let detail = provider_message(body);
    let gemini_reason = body["error"]["details"].as_array().and_then(|d| d.iter().find_map(|x| x["reason"].as_str())).unwrap_or("");
    let claude_type = body["error"]["type"].as_str().unwrap_or("");
    let bad_key = status == 401
        || status == 403
        || gemini_reason == "API_KEY_INVALID"
        || claude_type == "authentication_error"
        || claude_type == "permission_error"
        || (status == 400 && detail.to_ascii_lowercase().contains("api key"));
    let (code, message, retryable) = if bad_key {
        ("ai_bad_key", format!("{name} didn't accept the API key. Check it in Tools › Options › AI."), false)
    } else if status == 429 {
        (
            "ai_quota",
            format!(
                "{name}: quota or rate limit reached. Try again in a minute, or check the plan / billing for your API key.{}",
                if detail.is_empty() { String::new() } else { format!(" ({detail})") }
            ),
            false,
        )
    } else if status == 404 {
        (
            "ai_model",
            format!(
                "{name} doesn't know the model you chose. Pick another one in Tools › Options › AI.{}",
                if detail.is_empty() { String::new() } else { format!(" ({detail})") }
            ),
            false,
        )
    } else if status >= 500 || status == 529 {
        ("ai_unavailable", format!("{name} is unavailable right now (HTTP {status}). Try again shortly."), true)
    } else {
        (
            "ai_error",
            format!("{name} error (HTTP {status}){}", if detail.is_empty() { String::new() } else { format!(": {detail}") }),
            false,
        )
    };
    ProviderError { code, message, status, retryable }
}

/// The model's text from a successful (HTTP 200) response.
pub fn parse_response(provider: Provider, body: &Value) -> Result<String, ProviderError> {
    let name = provider.label();
    let fail = |code: &'static str, message: String| ProviderError { code, message, status: 200, retryable: false };
    match provider {
        Provider::Gemini => {
            if let Some(r) = body["promptFeedback"]["blockReason"].as_str() {
                return Err(fail("ai_blocked", format!("{name} declined this request ({r}).")));
            }
            let Some(c) = body["candidates"].as_array().and_then(|a| a.first()) else {
                return Err(fail("ai_empty", format!("{name} returned no answer.")));
            };
            let text: String = c["content"]["parts"]
                .as_array()
                .map(|parts| parts.iter().filter(|p| p["thought"].as_bool() != Some(true)).filter_map(|p| p["text"].as_str()).collect())
                .unwrap_or_default();
            let reason = c["finishReason"].as_str().unwrap_or("");
            if text.is_empty() {
                return Err(match reason {
                    "SAFETY" | "PROHIBITED_CONTENT" | "BLOCKLIST" | "RECITATION" | "SPII" => {
                        fail("ai_blocked", format!("{name} declined this request ({reason})."))
                    }
                    "MAX_TOKENS" => fail("ai_too_long", format!("{name}'s answer was cut off (too long).")),
                    _ => fail("ai_empty", format!("{name} returned an empty answer.")),
                });
            }
            if reason == "MAX_TOKENS" {
                return Err(fail("ai_too_long", format!("{name}'s answer was cut off (too long).")));
            }
            Ok(text)
        }
        Provider::Claude => {
            let stop = body["stop_reason"].as_str().unwrap_or("");
            if stop == "refusal" {
                return Err(fail("ai_blocked", format!("{name} declined this request.")));
            }
            let text: String = body["content"]
                .as_array()
                .map(|blocks| blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect())
                .unwrap_or_default();
            if stop == "max_tokens" {
                return Err(fail("ai_too_long", format!("{name}'s answer was cut off (too long).")));
            }
            if text.is_empty() {
                return Err(fail("ai_empty", format!("{name} returned an empty answer.")));
            }
            Ok(text)
        }
    }
}

/// Parse a JSON reply, tolerating Markdown fences and text around the JSON.
pub fn parse_json_reply(text: &str) -> anyhow::Result<Value> {
    let t = text.trim();
    let t = t.strip_prefix("```json").or_else(|| t.strip_prefix("```JSON")).or_else(|| t.strip_prefix("```")).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Ok(v);
    }
    // first [...] or {...} span
    let start = t.find(['[', '{']).ok_or_else(|| anyhow::anyhow!("the reply wasn't JSON"))?;
    let close = if t.as_bytes()[start] == b'[' { ']' } else { '}' };
    let end = t.rfind(close).ok_or_else(|| anyhow::anyhow!("the reply wasn't JSON"))?;
    if end <= start {
        anyhow::bail!("the reply wasn't JSON");
    }
    Ok(serde_json::from_str(&t[start..=end])?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_request_shape() {
        let o = CompleteOpts { json: true, max_tokens: 100, temperature: Some(0.1), ..Default::default() };
        let r = build_request(Provider::Gemini, "https://generativelanguage.googleapis.com/", "gemini-2.5-flash", "be brief", "hi", &o);
        assert_eq!(r.url, "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent");
        assert_eq!(r.key_header, "x-goog-api-key");
        assert_eq!(r.body["contents"][0]["role"], "user");
        assert_eq!(r.body["contents"][0]["parts"][0]["text"], "hi");
        assert_eq!(r.body["systemInstruction"]["parts"][0]["text"], "be brief");
        assert_eq!(r.body["generationConfig"]["responseMimeType"], "application/json");
        assert_eq!(r.body["generationConfig"]["maxOutputTokens"], 100);
        assert_eq!(r.body["generationConfig"]["thinkingConfig"]["thinkingBudget"], 0);
        assert!(!r.url.contains("key="), "the key never goes in the URL");
        // no JSON mode, no system → fields absent
        let r = build_request(Provider::Gemini, "http://x", "gemini-2.5-pro", "", "hi", &CompleteOpts::default());
        assert!(r.body.get("systemInstruction").is_none());
        assert!(r.body["generationConfig"].get("responseMimeType").is_none());
        assert!(r.body["generationConfig"].get("thinkingConfig").is_none());
    }

    #[test]
    fn claude_request_shape() {
        let o = CompleteOpts { json: true, max_tokens: 256, ..Default::default() };
        let r = build_request(Provider::Claude, "https://api.anthropic.com", "claude-haiku-5-5", "translate", "hello", &o);
        assert_eq!(r.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(r.key_header, "x-api-key");
        assert_eq!(r.headers, vec![("anthropic-version", "2023-06-01")]);
        assert_eq!(r.body["model"], "claude-haiku-5-5");
        assert_eq!(r.body["max_tokens"], 256);
        assert_eq!(r.body["messages"][0]["role"], "user");
        assert_eq!(r.body["messages"][0]["content"], "hello");
        assert!(r.body["system"].as_str().unwrap().starts_with("translate"));
        assert!(r.body["system"].as_str().unwrap().contains("JSON only"));
        // no sampling params / thinking budgets (400 on current models)
        assert!(r.body.get("temperature").is_none());
        assert!(r.body.get("thinking").is_none());
        assert_eq!(r.body["output_config"]["effort"], "low");
    }

    #[test]
    fn parse_gemini() {
        let ok = json!({
            "candidates": [{ "content": { "role": "model", "parts": [{ "text": "Hel" }, { "text": "lo" }] }, "finishReason": "STOP" }],
            "usageMetadata": { "promptTokenCount": 3 }
        });
        assert_eq!(parse_response(Provider::Gemini, &ok).unwrap(), "Hello");
        let thought = json!({ "candidates": [{ "content": { "parts": [{ "text": "hmm", "thought": true }, { "text": "OK" }] }, "finishReason": "STOP" }] });
        assert_eq!(parse_response(Provider::Gemini, &thought).unwrap(), "OK");
        let blocked = json!({ "promptFeedback": { "blockReason": "SAFETY" } });
        assert_eq!(parse_response(Provider::Gemini, &blocked).unwrap_err().code, "ai_blocked");
        let cut = json!({ "candidates": [{ "content": { "parts": [{ "text": "[\"a\"," }] }, "finishReason": "MAX_TOKENS" }] });
        assert_eq!(parse_response(Provider::Gemini, &cut).unwrap_err().code, "ai_too_long");
        let empty = json!({ "candidates": [] });
        assert_eq!(parse_response(Provider::Gemini, &empty).unwrap_err().code, "ai_empty");
    }

    #[test]
    fn parse_claude() {
        let ok = json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-haiku-5-5",
            "content": [{ "type": "thinking", "thinking": "" }, { "type": "text", "text": "Bonjour" }],
            "stop_reason": "end_turn", "usage": { "input_tokens": 5, "output_tokens": 2 }
        });
        assert_eq!(parse_response(Provider::Claude, &ok).unwrap(), "Bonjour");
        let refusal = json!({ "content": [], "stop_reason": "refusal", "stop_details": { "type": "refusal", "category": null } });
        assert_eq!(parse_response(Provider::Claude, &refusal).unwrap_err().code, "ai_blocked");
        let cut = json!({ "content": [{ "type": "text", "text": "[" }], "stop_reason": "max_tokens" });
        assert_eq!(parse_response(Provider::Claude, &cut).unwrap_err().code, "ai_too_long");
    }

    #[test]
    fn errors_are_clear() {
        // Gemini bad key: HTTP 400 INVALID_ARGUMENT with reason API_KEY_INVALID
        let g = json!({ "error": { "code": 400, "message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT",
            "details": [{ "@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "API_KEY_INVALID" }] } });
        let e = classify_error(Provider::Gemini, 400, &g);
        assert_eq!(e.code, "ai_bad_key");
        assert!(!e.retryable);
        let q = json!({ "error": { "code": 429, "message": "Resource has been exhausted (e.g. check quota).", "status": "RESOURCE_EXHAUSTED" } });
        let e = classify_error(Provider::Gemini, 429, &q);
        assert_eq!(e.code, "ai_quota");
        assert!(e.message.contains("quota"));
        assert!(!e.retryable, "no retries on 4xx");
        let c = json!({ "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" } });
        assert_eq!(classify_error(Provider::Claude, 401, &c).code, "ai_bad_key");
        let o = json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } });
        let e = classify_error(Provider::Claude, 529, &o);
        assert_eq!(e.code, "ai_unavailable");
        assert!(e.retryable);
        let m = json!({ "type": "error", "error": { "type": "not_found_error", "message": "model: nope" } });
        assert_eq!(classify_error(Provider::Claude, 404, &m).code, "ai_model");
    }

    #[test]
    fn json_reply_tolerant() {
        assert_eq!(parse_json_reply("[\"a\",\"b\"]").unwrap(), json!(["a", "b"]));
        assert_eq!(parse_json_reply("```json\n[\"a\"]\n```").unwrap(), json!(["a"]));
        assert_eq!(parse_json_reply("Here you go:\n[\"x\", \"y\"]\nDone.").unwrap(), json!(["x", "y"]));
        assert!(parse_json_reply("no json here").is_err());
    }
}

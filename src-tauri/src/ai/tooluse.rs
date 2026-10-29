//! Tool use ("function calling") for both providers, so a model can ask ClassicMail for facts
//! mid-answer (AI Review's `{a free slot Tuesday}` → the calendar). Pure request building and
//! response parsing, like `provider.rs`; the loop that runs the tools is `ai::complete_json_tools`.
//!
//! * Gemini: `tools: [{functionDeclarations}]`; the model answers with `functionCall` parts, we
//!   send `functionResponse` parts back (its own content, thought signatures included, goes back
//!   unchanged). JSON-mode isn't allowed together with tools, so JSON is asked for in the prompt.
//! * Claude: `tools: [{name, description, input_schema}]`; the model answers with `tool_use`
//!   blocks (stop_reason "tool_use"), we send `tool_result` blocks back.

use serde_json::{Value, json};

use super::provider::{ANTHROPIC_VERSION, BuiltRequest, CompleteOpts, JSON_ONLY, Provider, ProviderError};

/// A tool the model may call: name, what it does, and its parameters as a JSON schema (object
/// with properties / required; plain types only, which both providers accept).
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// Claude's tool_use id (Gemini has none: the name)
    pub id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Turn {
    /// the final answer
    Text(String),
    /// tools to run; `assistant` is the model's message, to go back into the history as it is
    Calls { calls: Vec<ToolCall>, assistant: Value },
}

/// The first message of a conversation.
pub fn user_message(provider: Provider, prompt: &str) -> Value {
    match provider {
        Provider::Gemini => json!({ "role": "user", "parts": [{ "text": prompt }] }),
        Provider::Claude => json!({ "role": "user", "content": prompt }),
    }
}

/// The tools' results, as the message that answers the model's calls.
pub fn results_message(provider: Provider, results: &[(ToolCall, Value)]) -> Value {
    match provider {
        Provider::Gemini => json!({
            "role": "user",
            "parts": results.iter().map(|(c, v)| json!({ "functionResponse": { "name": c.name, "response": { "result": v } } })).collect::<Vec<_>>(),
        }),
        Provider::Claude => json!({
            "role": "user",
            "content": results.iter().map(|(c, v)| json!({ "type": "tool_result", "tool_use_id": c.id, "content": v.to_string() })).collect::<Vec<_>>(),
        }),
    }
}

pub fn build_request(
    provider: Provider,
    base: &str,
    model: &str,
    system: &str,
    history: &[Value],
    tools: &[ToolSpec],
    opts: &CompleteOpts,
) -> BuiltRequest {
    let base = base.trim_end_matches('/');
    let sys = match (system.is_empty(), opts.json) {
        (true, false) => String::new(),
        (true, true) => JSON_ONLY.to_string(),
        (false, false) => system.to_string(),
        (false, true) => format!("{system}\n\n{JSON_ONLY}"),
    };
    match provider {
        Provider::Gemini => {
            let mut gcfg = json!({ "maxOutputTokens": opts.max_tokens });
            if let Some(t) = opts.temperature {
                gcfg["temperature"] = json!(t);
            }
            if model.contains("flash") {
                gcfg["thinkingConfig"] = json!({ "thinkingBudget": 0 });
            }
            let decls: Vec<Value> =
                tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "parameters": t.parameters })).collect();
            let mut body = json!({ "contents": history, "generationConfig": gcfg, "tools": [{ "functionDeclarations": decls }] });
            if !sys.is_empty() {
                body["systemInstruction"] = json!({ "parts": [{ "text": sys }] });
            }
            BuiltRequest {
                url: format!("{base}/v1beta/models/{}:generateContent", urlencoding::encode(model)),
                key_header: "x-goog-api-key",
                headers: vec![],
                body,
            }
        }
        Provider::Claude => {
            let decls: Vec<Value> =
                tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.parameters })).collect();
            let mut body = json!({ "model": model, "max_tokens": opts.max_tokens, "messages": history, "tools": decls, "output_config": { "effort": "low" } });
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

/// What the model did: asked for tools, or answered. Errors as `provider::parse_response`.
pub fn parse_turn(provider: Provider, body: &Value) -> Result<Turn, ProviderError> {
    match provider {
        Provider::Gemini => {
            let content = &body["candidates"][0]["content"];
            let calls: Vec<ToolCall> = content["parts"]
                .as_array()
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| p.get("functionCall"))
                        .map(|f| ToolCall {
                            id: f["name"].as_str().unwrap_or("").into(),
                            name: f["name"].as_str().unwrap_or("").into(),
                            args: f["args"].clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !calls.is_empty() {
                return Ok(Turn::Calls { calls, assistant: content.clone() });
            }
        }
        Provider::Claude => {
            let calls: Vec<ToolCall> = body["content"]
                .as_array()
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter(|b| b["type"] == "tool_use")
                        .map(|b| ToolCall {
                            id: b["id"].as_str().unwrap_or("").into(),
                            name: b["name"].as_str().unwrap_or("").into(),
                            args: b["input"].clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !calls.is_empty() {
                return Ok(Turn::Calls { calls, assistant: json!({ "role": "assistant", "content": body["content"] }) });
            }
        }
    }
    super::provider::parse_response(provider, body).map(Turn::Text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: "find_free_time",
            description: "free slots",
            parameters: json!({ "type": "object", "properties": { "from": { "type": "string" } }, "required": ["from"] }),
        }]
    }

    #[test]
    fn gemini_round_trip_shapes() {
        let o = CompleteOpts { json: true, ..Default::default() };
        let h = vec![user_message(Provider::Gemini, "hi")];
        let r = build_request(Provider::Gemini, "http://x/", "gemini-2.5-flash", "be brief", &h, &spec(), &o);
        assert_eq!(r.body["tools"][0]["functionDeclarations"][0]["name"], "find_free_time");
        assert!(r.body["generationConfig"].get("responseMimeType").is_none(), "no JSON mode with tools");
        assert!(r.body["systemInstruction"]["parts"][0]["text"].as_str().unwrap().contains("JSON only"));
        let reply = json!({ "candidates": [{ "content": { "role": "model", "parts": [{ "functionCall": { "name": "find_free_time", "args": { "from": "2026-10-27" } }, "thoughtSignature": "sig" }] } }] });
        let Turn::Calls { calls, assistant } = parse_turn(Provider::Gemini, &reply).unwrap() else { panic!() };
        assert_eq!(calls[0].name, "find_free_time");
        assert_eq!(calls[0].args["from"], "2026-10-27");
        assert_eq!(assistant["parts"][0]["thoughtSignature"], "sig", "the model's content goes back unchanged");
        let back = results_message(Provider::Gemini, &[(calls[0].clone(), json!({ "slots": [] }))]);
        assert_eq!(back["parts"][0]["functionResponse"]["name"], "find_free_time");
        assert_eq!(back["parts"][0]["functionResponse"]["response"]["result"]["slots"], json!([]));
        let text = json!({ "candidates": [{ "content": { "parts": [{ "text": "{\"html\":\"x\"}" }] }, "finishReason": "STOP" }] });
        assert_eq!(parse_turn(Provider::Gemini, &text).unwrap(), Turn::Text("{\"html\":\"x\"}".into()));
    }

    #[test]
    fn claude_round_trip_shapes() {
        let h = vec![user_message(Provider::Claude, "hi")];
        let r =
            build_request(Provider::Claude, "https://api.anthropic.com", "claude-haiku-5-5", "s", &h, &spec(), &CompleteOpts::default());
        assert_eq!(r.body["tools"][0]["input_schema"]["required"][0], "from");
        assert_eq!(r.body["messages"][0]["content"], "hi");
        let reply = json!({ "content": [{ "type": "text", "text": "Let me look." }, { "type": "tool_use", "id": "toolu_1", "name": "find_free_time", "input": { "from": "2026-10-27" } }], "stop_reason": "tool_use" });
        let Turn::Calls { calls, assistant } = parse_turn(Provider::Claude, &reply).unwrap() else { panic!() };
        assert_eq!(calls[0].id, "toolu_1");
        assert_eq!(assistant["role"], "assistant");
        let back = results_message(Provider::Claude, &[(calls[0].clone(), json!({ "slots": [] }))]);
        assert_eq!(back["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(back["content"][0]["type"], "tool_result");
    }
}

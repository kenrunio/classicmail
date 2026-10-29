//! Which models the user's key can use, asked from the provider itself (model names change: Google
//! retires models for new keys), ranked newest first, with a recommended default.
//!
//! * Gemini: `GET {base}/v1beta/models?pageSize=1000` (header `x-goog-api-key`) →
//!   `{models: [{name: "models/gemini-…", displayName, supportedGenerationMethods: [...]}]}`
//! * Claude: `GET {base}/v1/models?limit=1000` (headers `x-api-key`, `anthropic-version`) →
//!   `{data: [{id, display_name}]}`

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AiConfig, Provider, provider::ANTHROPIC_VERSION};
use crate::ctx::AppCtx;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelList {
    pub models: Vec<ModelInfo>,
    pub recommended: Option<String>,
    /// when the provider was last asked (epoch ms; 0 = never)
    pub checked_at: i64,
}

/// When the list was last fetched (epoch ms), for "Last checked" and the daily refresh.
pub const CHECKED_SETTING: &str = "ai.modelsCheckedAt";
/// How often the list is fetched again in the background (models come and go).
const REFRESH_EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// Gemini model ids that can't do plain text generation for us.
const GEMINI_SKIP: &[&str] =
    &["embedding", "embed", "tts", "image", "audio", "live", "native-audio", "computer-use", "robotics", "aqa", "veo", "imagen", "learnlm"];

/// The version number in an id as (major, minor): "gemini-3.8-flash" → (3, 8), "claude-haiku-5-5" →
/// (5, 5). A date after the number ("claude-sonnet-4-20250514") is not a minor version.
fn version(id: &str) -> (u32, u32) {
    let parts: Vec<&str> = id.split(['-', '_']).collect();
    let small = |s: &str| (!s.is_empty() && s.len() <= 2).then(|| s.parse::<u32>().ok()).flatten();
    for (i, p) in parts.iter().enumerate() {
        if let Some((a, b)) = p.split_once('.')
            && let (Ok(major), Ok(minor)) = (a.parse::<u32>(), b.parse::<u32>())
        {
            return (major, minor);
        }
        if let Ok(major) = p.parse::<u32>() {
            // "5" "5" (claude-haiku-5-5) → 5.5
            return (major, parts.get(i + 1).and_then(|m| small(m)).unwrap_or(0));
        }
    }
    (0, 0)
}

/// Lower is better: stable before preview/experimental, flash > flash-lite > pro for Gemini;
/// haiku > sonnet > opus for Claude (translation wants fast and cheap).
fn tier(id: &str) -> u8 {
    let l = id.to_ascii_lowercase();
    let unstable = l.contains("preview") || l.contains("exp") || l.contains("latest");
    let base = if l.contains("flash-lite") || l.contains("lite") {
        2
    } else if l.contains("flash") || l.contains("haiku") {
        1
    } else if l.contains("sonnet") {
        3
    } else {
        4
    };
    base + if unstable { 10 } else { 0 }
}

pub fn rank(mut models: Vec<ModelInfo>) -> ModelList {
    models.sort_by(|a, b| tier(&a.id).cmp(&tier(&b.id)).then(version(&b.id).cmp(&version(&a.id))).then(a.id.cmp(&b.id)));
    models.dedup_by(|a, b| a.id == b.id);
    let recommended = models.first().map(|m| m.id.clone());
    ModelList { models, recommended, checked_at: 0 }
}

pub fn parse_gemini(body: &Value) -> Vec<ModelInfo> {
    body["models"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|m| m["supportedGenerationMethods"].as_array().is_some_and(|ms| ms.iter().any(|x| x == "generateContent")))
                .filter_map(|m| {
                    let id = m["name"].as_str()?.trim_start_matches("models/").to_string();
                    let l = id.to_ascii_lowercase();
                    if !l.starts_with("gemini") || GEMINI_SKIP.iter().any(|s| l.contains(s)) {
                        return None;
                    }
                    let label = m["displayName"].as_str().unwrap_or(&id).to_string();
                    Some(ModelInfo { id, label })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_claude(body: &Value) -> Vec<ModelInfo> {
    body["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    let id = m["id"].as_str()?.to_string();
                    let label = m["display_name"].as_str().unwrap_or(&id).to_string();
                    Some(ModelInfo { id, label })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Ask the provider. The demo/fake provider has no model list: the static defaults are used.
pub async fn fetch(http: &reqwest::Client, cfg: &AiConfig) -> anyhow::Result<ModelList> {
    if cfg.demo {
        return Ok(rank(vec![ModelInfo {
            id: cfg.provider.default_model().into(),
            label: format!("{} (demo)", cfg.provider.default_model()),
        }]));
    }
    let base = cfg.base.trim_end_matches('/');
    let req = match cfg.provider {
        Provider::Gemini => http.get(format!("{base}/v1beta/models")).query(&[("pageSize", "1000")]).header("x-goog-api-key", cfg.key()),
        Provider::Claude => http
            .get(format!("{base}/v1/models"))
            .query(&[("limit", "1000")])
            .header("x-api-key", cfg.key())
            .header("anthropic-version", ANTHROPIC_VERSION),
    };
    let resp = req
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| crate::user_err!("ai_network", "Couldn't reach {}: {}", cfg.provider.label(), e.without_url()))?;
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if status >= 400 {
        let e = super::provider::classify_error(cfg.provider, status, &body);
        return Err(crate::user_err!(e.code, "{}", e.message));
    }
    let models = match cfg.provider {
        Provider::Gemini => parse_gemini(&body),
        Provider::Claude => parse_claude(&body),
    };
    Ok(rank(models))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ListParams {
    /// another provider than the saved one (Options page, before saving)
    provider: Option<String>,
    /// a key typed in the Options page but not saved yet
    key: Option<String>,
}

/// rpc `ai.listModels {provider?, key?}` → { models, recommended }: with a typed key (not yet saved)
/// or the saved one. The recommended model is remembered as the provider's default.
pub async fn list_rpc(ctx: Arc<AppCtx>, p: ListParams) -> anyhow::Result<ModelList> {
    let typed = p.key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    let cfg = match (typed, p.provider.as_deref().map(Provider::parse)) {
        (Some(key), prov) => {
            let prov = match prov {
                Some(x) => x,
                None => super::require_config(&ctx).await.map(|c| c.provider).unwrap_or(Provider::Gemini),
            };
            AiConfig::new(prov, prov.default_model(), prov.default_base(), key, false)
        }
        (None, _) => super::require_config(&ctx).await?,
    };
    let mut list = fetch(&ctx.http, &cfg).await?;
    if !cfg.demo {
        list.checked_at = remember(&ctx, cfg.provider, list.recommended.clone()).await?;
    }
    Ok(list)
}

/// The provider's recommended model as its default, and the time it was checked (returned).
async fn remember(ctx: &AppCtx, provider: Provider, recommended: Option<String>) -> anyhow::Result<i64> {
    let now = crate::clock::now_ms();
    let key = super::recommended_key(provider);
    ctx.db
        .write(move |c| {
            if let Some(rec) = recommended {
                crate::db::util::set_setting(c, &key, &rec)?;
            }
            crate::db::util::set_setting(c, CHECKED_SETTING, &now)
        })
        .await?;
    Ok(now)
}

/// The saved provider's list fetched again: its recommended model becomes the default, and a
/// chosen model the key can't use any more is dropped (the default takes over), as Options does.
pub async fn refresh(ctx: &AppCtx) -> anyhow::Result<()> {
    let Some(cfg) = super::config(ctx).await? else { return Ok(()) };
    if cfg.demo {
        return Ok(());
    }
    let list = fetch(&ctx.http, &cfg).await?;
    remember(ctx, cfg.provider, list.recommended.clone()).await?;
    let chosen: Option<String> = ctx.db.read(|c| crate::db::util::get_setting(c, super::MODEL_SETTING)).await?;
    if let Some(m) = chosen.filter(|m| !m.trim().is_empty())
        && !list.models.is_empty()
        && !list.models.iter().any(|x| x.id == m.trim())
    {
        log::info!("ai: model {m} isn't offered any more; using the default ({:?})", list.recommended);
        ctx.db.write(|c| crate::db::util::set_setting(c, super::MODEL_SETTING, &String::new())).await?;
    }
    log::info!("ai: model list checked ({} models, recommended {:?})", list.models.len(), list.recommended);
    Ok(())
}

/// In the background: the model list fetched again once a day (first a few minutes after start,
/// if the last check is older than a day).
pub fn start_refresh(ctx: Arc<AppCtx>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(180)).await;
        loop {
            let last: i64 = ctx.db.read(|c| crate::db::util::get_setting(c, CHECKED_SETTING)).await.ok().flatten().unwrap_or(0);
            if crate::clock::now_ms() - last >= REFRESH_EVERY.as_millis() as i64
                && let Err(e) = refresh(&ctx).await
            {
                log::warn!("ai: model list check failed: {}", crate::logging::err(&e));
            }
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gemini_list_is_filtered_and_ranked() {
        let body = json!({ "models": [
            { "name": "models/gemini-2.5-flash", "displayName": "Gemini 2.5 Flash", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-3.8-flash", "displayName": "Gemini 3.8 Flash", "supportedGenerationMethods": ["generateContent", "countTokens"] },
            { "name": "models/gemini-3.8-flash-lite", "displayName": "Gemini 3.8 Flash-Lite", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-3.8-pro", "displayName": "Gemini 3.8 Pro", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-4.0-flash-preview", "displayName": "Gemini 4 Flash Preview", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-embedding-001", "displayName": "Embedding", "supportedGenerationMethods": ["embedContent"] },
            { "name": "models/gemini-3.8-flash-image", "displayName": "Image", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-2.5-flash-preview-tts", "displayName": "TTS", "supportedGenerationMethods": ["generateContent"] }
        ]});
        let l = rank(parse_gemini(&body));
        let ids: Vec<&str> = l.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(l.recommended.as_deref(), Some("gemini-3.8-flash"));
        assert_eq!(
            ids,
            vec!["gemini-3.8-flash", "gemini-2.5-flash", "gemini-3.8-flash-lite", "gemini-3.8-pro", "gemini-4.0-flash-preview"]
        );
    }

    #[test]
    fn versions_ignore_dates() {
        assert_eq!(version("claude-haiku-5-5"), (5, 5));
        assert_eq!(version("claude-haiku-4-5-20251001"), (4, 5));
        assert_eq!(version("claude-sonnet-4-20250514"), (4, 0));
        assert_eq!(version("claude-3-5-haiku-20241022"), (3, 5));
        assert_eq!(version("gemini-3.10-flash"), (3, 10));
        assert!(version("gemini-3.10-flash") > version("gemini-3.8-flash"));
        let body = json!({ "data": [
            { "id": "claude-sonnet-4-20250514" }, { "id": "claude-sonnet-5-5" }, { "id": "claude-opus-4-20250514" }, { "id": "claude-opus-5-5" }
        ]});
        let ids: Vec<String> = rank(parse_claude(&body)).models.into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["claude-sonnet-5-5", "claude-sonnet-4-20250514", "claude-opus-5-5", "claude-opus-4-20250514"]);
    }

    #[test]
    fn claude_list_prefers_newest_haiku() {
        let body = json!({ "data": [
            { "id": "claude-opus-5-5", "display_name": "Claude Opus 5.5" },
            { "id": "claude-haiku-5-5", "display_name": "Claude Haiku 5.5" },
            { "id": "claude-sonnet-5-5", "display_name": "Claude Sonnet 5.5" },
            { "id": "claude-haiku-4-5", "display_name": "Claude Haiku 4.5" }
        ]});
        let l = rank(parse_claude(&body));
        assert_eq!(l.recommended.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(l.models[1].id, "claude-haiku-4-5");
    }
}

//! AI layer: one small, reusable "ask a model" client (`complete` / `complete_json`) over Google
//! Gemini or Anthropic Claude, plus features built on it (message translation, `translate.rs`).
//!
//! Settings (settings table): `ai.provider` ("gemini" default | "claude"), `ai.model` (empty =
//! the provider's default), `ai.targetLanguage` (default "English"), `ai.neverTranslate` (list of
//! language names). API keys live in Windows Credential Manager (`google::auth::store_secret`
//! "ai.geminiKey" / "ai.anthropicKey") — never in settings, never in logs or error messages.
//!
//! **Demo mode:** with no key for the chosen provider and a demo account present, the in-process
//! fake provider (`fake.rs`, deterministic "[EN] …" translations) is used.
//!
//! **AI notice:** nothing from the user's mail goes to a real model until they have accepted the
//! one-time AI notice (`ai.consentAt`, epoch ms; set by `ai.setConsent`). Until then every feature
//! fails with `ai_consent`, which the front end answers by showing the notice (src/api/aiConsent.ts).
//! The demo provider sends nothing anywhere, so it needs no notice.
//!
//! rpc: `ai.getSettings`, `ai.setSettings`, `ai.setConsent`, `ai.test`, `ai.translate`. Event: `ai.changed`.

pub mod compose;
pub mod fake;
pub mod models;
pub mod provider;
pub mod schema;
pub mod tools;
pub mod tooluse;
pub mod translate;

use std::{sync::Arc, time::Duration};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{
    ctx::AppCtx,
    db::util::{get_setting, set_setting},
    rpc::{NoParams, Registry},
};
pub use provider::{CompleteOpts, Provider, ProviderError};

pub const PROVIDER_SETTING: &str = "ai.provider";
pub const MODEL_SETTING: &str = "ai.model";
pub const TARGET_SETTING: &str = "ai.targetLanguage";
pub const NEVER_SETTING: &str = "ai.neverTranslate";
pub const DEFAULT_TARGET: &str = "English";
/// When the user accepted the AI notice (epoch ms); absent = not yet.
pub const CONSENT_SETTING: &str = "ai.consentAt";

pub fn register(r: &mut Registry) {
    r.add("ai.getSettings", get_settings);
    r.add("ai.setSettings", set_settings);
    r.add("ai.setConsent", set_consent);
    r.add("ai.test", test);
    r.add("ai.listModels", models::list_rpc);
    r.add("ai.translate", translate::translate_rpc);
    compose::register(r);
}

// ---- key storage (keyring; in-memory in tests so they never touch Credential Manager) ---------

pub(crate) mod secrets {
    #[cfg(not(test))]
    pub fn get(name: &str) -> anyhow::Result<Option<String>> {
        crate::google::auth::load_secret(name)
    }
    #[cfg(not(test))]
    pub fn set(name: &str, value: &str) -> anyhow::Result<()> {
        crate::google::auth::store_secret(name, value)
    }
    #[cfg(not(test))]
    pub fn delete(name: &str) -> anyhow::Result<()> {
        crate::google::auth::delete_secret(name)
    }

    #[cfg(test)]
    static MEM: parking_lot::Mutex<Option<std::collections::HashMap<String, String>>> = parking_lot::Mutex::new(None);
    #[cfg(test)]
    pub fn get(name: &str) -> anyhow::Result<Option<String>> {
        Ok(MEM.lock().as_ref().and_then(|m| m.get(name).cloned()))
    }
    #[cfg(test)]
    pub fn set(name: &str, value: &str) -> anyhow::Result<()> {
        MEM.lock().get_or_insert_with(Default::default).insert(name.into(), value.into());
        Ok(())
    }
    #[cfg(test)]
    pub fn delete(name: &str) -> anyhow::Result<()> {
        if let Some(m) = MEM.lock().as_mut() {
            m.remove(name);
        }
        Ok(())
    }
}

fn load_key(p: Provider) -> Option<String> {
    match secrets::get(p.secret_name()) {
        Ok(k) => k.filter(|k| !k.trim().is_empty()),
        Err(e) => {
            log::warn!("ai: reading the {} key from the keyring failed: {}", p.id(), crate::logging::err(&e));
            None
        }
    }
}

// ---- configuration --------------------------------------------------------------------------

/// What `complete` talks to. `Debug` never prints the key.
#[derive(Clone)]
pub struct AiConfig {
    pub provider: Provider,
    pub model: String,
    pub base: String,
    key: String,
    /// the built-in fake provider (demo accounts, no key)
    pub demo: bool,
}

impl std::fmt::Debug for AiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiConfig")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("base", &self.base)
            .field("key", &"<redacted>")
            .field("demo", &self.demo)
            .finish()
    }
}

impl AiConfig {
    pub fn new(provider: Provider, model: impl Into<String>, base: impl Into<String>, key: impl Into<String>, demo: bool) -> Self {
        AiConfig { provider, model: model.into(), base: base.into(), key: key.into(), demo }
    }

    /// The API key (for request headers only — never log it).
    pub(crate) fn key(&self) -> &str {
        &self.key
    }
}

static FAKES: tokio::sync::Mutex<Option<std::collections::HashMap<std::path::PathBuf, String>>> = tokio::sync::Mutex::const_new(None);

/// The fake provider for this data dir (one in the app, one per test context).
async fn fake_base(ctx: &AppCtx) -> anyhow::Result<String> {
    let mut g = FAKES.lock().await;
    let m = g.get_or_insert_with(Default::default);
    if let Some(u) = m.get(&ctx.data_dir) {
        return Ok(u.clone());
    }
    let u = fake::start().await?;
    m.insert(ctx.data_dir.clone(), u.clone());
    Ok(u)
}

/// Only demo accounts (a test build / demo profile): the fake provider is used even when a real
/// key is in the keyring — the keyring is shared with the real ClassicMail, and demo mail must
/// never go to a real model.
async fn demo_only(ctx: &AppCtx) -> anyhow::Result<bool> {
    ctx.db
        .read(|c| {
            Ok(c.query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE provider='demo') AND NOT EXISTS(SELECT 1 FROM accounts WHERE provider<>'demo')",
                [],
                |r| r.get::<_, bool>(0),
            )?)
        })
        .await
}

async fn has_demo_account(ctx: &AppCtx) -> anyhow::Result<bool> {
    ctx.db.read(|c| Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM accounts WHERE provider='demo')", [], |r| r.get::<_, bool>(0))?)).await
}

/// Setting holding the provider's recommended model, remembered each time the model list is fetched
/// (so a retired built-in default never bites: Google retires models for new keys).
pub fn recommended_key(p: Provider) -> String {
    format!("ai.recommendedModel.{}", p.id())
}

struct Stored {
    provider: Provider,
    model: Option<String>,
    target: String,
    never: Vec<String>,
    recommended: std::collections::HashMap<&'static str, String>,
}

impl Stored {
    /// The model to use for `p` when none is chosen.
    fn default_for(&self, p: Provider) -> String {
        self.recommended.get(p.id()).cloned().unwrap_or_else(|| p.default_model().to_string())
    }
}

async fn stored(ctx: &AppCtx) -> anyhow::Result<Stored> {
    ctx.db
        .read(|c| {
            let provider = get_setting::<String>(c, PROVIDER_SETTING)?.map(|s| Provider::parse(&s)).unwrap_or(Provider::Gemini);
            let model = get_setting::<String>(c, MODEL_SETTING)?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            let target =
                get_setting::<String>(c, TARGET_SETTING)?.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| DEFAULT_TARGET.into());
            let never = get_setting::<Vec<String>>(c, NEVER_SETTING)?.unwrap_or_default();
            let mut recommended = std::collections::HashMap::new();
            for p in [Provider::Gemini, Provider::Claude] {
                if let Some(m) = get_setting::<String>(c, &recommended_key(p))?.filter(|m| !m.trim().is_empty()) {
                    recommended.insert(p.id(), m);
                }
            }
            Ok(Stored { provider, model, target, never, recommended })
        })
        .await
}

/// The configured model, or None when no key is set (and no demo account to fall back on).
pub async fn config(ctx: &AppCtx) -> anyhow::Result<Option<AiConfig>> {
    let s = stored(ctx).await?;
    let model = s.model.clone().unwrap_or_else(|| s.default_for(s.provider));
    if demo_only(ctx).await? {
        return Ok(Some(AiConfig::new(s.provider, model, fake_base(ctx).await?, fake::FAKE_KEY, true)));
    }
    if let Some(key) = load_key(s.provider) {
        return Ok(Some(AiConfig::new(s.provider, model, s.provider.default_base(), key, false)));
    }
    if has_demo_account(ctx).await? {
        return Ok(Some(AiConfig::new(s.provider, model, fake_base(ctx).await?, fake::FAKE_KEY, true)));
    }
    Ok(None)
}

pub async fn require_config(ctx: &AppCtx) -> anyhow::Result<AiConfig> {
    config(ctx)
        .await?
        .ok_or_else(|| crate::user_err!("ai_not_configured", "AI features aren't set up yet: add an API key in Tools › Options › AI."))
}

/// `require_config` for a feature that sends the user's content: also requires the AI notice to
/// have been accepted (not needed for the demo provider, which sends nothing anywhere).
pub async fn require_use(ctx: &AppCtx) -> anyhow::Result<AiConfig> {
    let cfg = require_config(ctx).await?;
    check_consent(&cfg, consent_at(ctx).await?)?;
    Ok(cfg)
}

fn check_consent(cfg: &AiConfig, consent_at: Option<i64>) -> anyhow::Result<()> {
    if cfg.demo || consent_at.is_some() {
        return Ok(());
    }
    Err(crate::user_err!("ai_consent", "Turn on the AI features first: sending text to {} needs your OK.", cfg.provider.label()))
}

async fn consent_at(ctx: &AppCtx) -> anyhow::Result<Option<i64>> {
    ctx.db.read(|c| get_setting::<i64>(c, CONSENT_SETTING)).await
}

// ---- the reusable client ------------------------------------------------------------------

fn user_error(e: ProviderError) -> anyhow::Error {
    crate::user_err!(e.code, "{}", e.message)
}

/// One request, no retries (callers decide). Errors are `UserError`s with an `ai_*` code.
pub async fn complete_with(
    http: &reqwest::Client,
    cfg: &AiConfig,
    system: &str,
    prompt: &str,
    opts: &CompleteOpts,
) -> anyhow::Result<String> {
    let req = provider::build_request(cfg.provider, &cfg.base, &cfg.model, system, prompt, opts);
    let body = send(http, cfg, req, opts).await?;
    provider::parse_response(cfg.provider, &body).map_err(user_error)
}

/// Send a built request; the response body of an HTTP 200, else a clear error.
async fn send(http: &reqwest::Client, cfg: &AiConfig, req: provider::BuiltRequest, opts: &CompleteOpts) -> anyhow::Result<Value> {
    let mut rb = http.post(&req.url).timeout(opts.timeout).header(req.key_header, &cfg.key).json(&req.body);
    for (k, v) in &req.headers {
        rb = rb.header(*k, *v);
    }
    let started = std::time::Instant::now();
    let resp = rb.send().await.map_err(|e| {
        // reqwest errors carry the URL only (the key is a header), but keep it short anyway
        if e.is_timeout() {
            crate::user_err!("ai_timeout", "{} didn't answer within {} seconds.", cfg.provider.label(), opts.timeout.as_secs())
        } else {
            crate::user_err!("ai_network", "Couldn't reach {}: {}", cfg.provider.label(), e.without_url())
        }
    })?;
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    log::debug!("ai: {} {} → HTTP {status} in {} ms", cfg.provider.id(), cfg.model, started.elapsed().as_millis());
    if status != 200 {
        return Err(user_error(provider::classify_error(cfg.provider, status, &body)));
    }
    Ok(body)
}

/// `complete_json` with the look-up tools of `tools.rs` (calendar, tasks, time zones): the model
/// asks for what it needs, ClassicMail answers from this computer, and the model writes its JSON
/// answer — usually two calls, a few more when several things are looked up.
pub async fn complete_json_tools<T: DeserializeOwned>(
    ctx: &Arc<AppCtx>,
    system: &str,
    prompt: &str,
    opts: CompleteOpts,
) -> anyhow::Result<T> {
    let cfg = require_use(ctx).await?;
    let opts = CompleteOpts { json: true, ..opts };
    let specs = tools::specs();
    let mut history = vec![tooluse::user_message(cfg.provider, prompt)];
    for _ in 0..6 {
        let body = {
            let req = tooluse::build_request(cfg.provider, &cfg.base, &cfg.model, system, &history, &specs, &opts);
            match send(&ctx.http, &cfg, req, &opts).await {
                Err(e) if is_retryable(&e) => {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    let req = tooluse::build_request(cfg.provider, &cfg.base, &cfg.model, system, &history, &specs, &opts);
                    send(&ctx.http, &cfg, req, &opts).await?
                }
                r => r?,
            }
        };
        match tooluse::parse_turn(cfg.provider, &body).map_err(user_error)? {
            tooluse::Turn::Text(text) => {
                let v = provider::parse_json_reply(&text)
                    .map_err(|e| crate::user_err!("ai_bad_reply", "The model's reply wasn't the expected JSON ({e})."))?;
                return serde_json::from_value(v)
                    .map_err(|e| crate::user_err!("ai_bad_reply", "The model's reply had an unexpected shape ({e})."));
            }
            tooluse::Turn::Calls { calls, assistant } => {
                history.push(assistant);
                let mut results = Vec::new();
                for call in calls {
                    let v = tools::run(ctx, &call.name, &call.args).await;
                    // sizes only: the arguments and the answer are the user's (a search, an event)
                    log::debug!("ai tool {} ({} bytes) → {} bytes", call.name, call.args.to_string().len(), v.to_string().len());
                    results.push((call, v));
                }
                history.push(tooluse::results_message(cfg.provider, &results));
            }
        }
    }
    Err(crate::user_err!("ai_bad_reply", "The AI kept looking things up without answering. Try a simpler instruction."))
}

/// Ask the configured model. Retries once on 5xx/overload/timeouts, never on 4xx.
pub async fn complete(ctx: &AppCtx, system: &str, prompt: &str, opts: CompleteOpts) -> anyhow::Result<String> {
    let cfg = require_use(ctx).await?;
    complete_cfg(&ctx.http, &cfg, system, prompt, &opts).await
}

pub async fn complete_cfg(
    http: &reqwest::Client,
    cfg: &AiConfig,
    system: &str,
    prompt: &str,
    opts: &CompleteOpts,
) -> anyhow::Result<String> {
    match complete_with(http, cfg, system, prompt, opts).await {
        Err(e) if is_retryable(&e) => {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            complete_with(http, cfg, system, prompt, opts).await
        }
        r => r,
    }
}

fn is_retryable(e: &anyhow::Error) -> bool {
    e.downcast_ref::<crate::error::UserError>().is_some_and(|u| matches!(u.code, "ai_unavailable" | "ai_timeout" | "ai_network"))
}

/// `complete` in JSON mode, parsed into `T` (tolerates code fences around the JSON).
pub async fn complete_json<T: DeserializeOwned>(ctx: &AppCtx, system: &str, prompt: &str, opts: CompleteOpts) -> anyhow::Result<T> {
    let cfg = require_use(ctx).await?;
    complete_json_cfg(&ctx.http, &cfg, system, prompt, opts).await
}

pub async fn complete_json_cfg<T: DeserializeOwned>(
    http: &reqwest::Client,
    cfg: &AiConfig,
    system: &str,
    prompt: &str,
    opts: CompleteOpts,
) -> anyhow::Result<T> {
    let opts = CompleteOpts { json: true, ..opts };
    let text = complete_cfg(http, cfg, system, prompt, &opts).await?;
    let v = provider::parse_json_reply(&text)
        .map_err(|e| crate::user_err!("ai_bad_reply", "The model's reply wasn't the expected JSON ({e})."))?;
    serde_json::from_value(v).map_err(|e| crate::user_err!("ai_bad_reply", "The model's reply had an unexpected shape ({e})."))
}

// ---- settings rpc ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsDto {
    provider: Provider,
    /// saved model ("" = the provider's default)
    model: String,
    default_models: Value,
    effective_model: String,
    has_gemini_key: bool,
    has_anthropic_key: bool,
    /// a key exists for the selected provider (or demo mode)
    configured: bool,
    demo: bool,
    target_language: String,
    never_translate: Vec<String>,
    /// when the AI notice was accepted (epoch ms), None = not yet
    consent_at: Option<i64>,
}

async fn get_settings(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<SettingsDto> {
    let s = stored(&ctx).await?;
    let cfg = config(&ctx).await?;
    Ok(SettingsDto {
        provider: s.provider,
        model: s.model.clone().unwrap_or_default(),
        default_models: json!({ "gemini": s.default_for(Provider::Gemini), "claude": s.default_for(Provider::Claude) }),
        effective_model: s.model.clone().unwrap_or_else(|| s.default_for(s.provider)),
        has_gemini_key: load_key(Provider::Gemini).is_some(),
        has_anthropic_key: load_key(Provider::Claude).is_some(),
        configured: cfg.is_some(),
        demo: cfg.as_ref().is_some_and(|c| c.demo),
        target_language: s.target,
        never_translate: s.never,
        consent_at: consent_at(&ctx).await?,
    })
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SetSettings {
    provider: Option<String>,
    /// "" = the provider's default
    model: Option<String>,
    /// key for `provider` (or the saved provider); "" deletes it
    key: Option<String>,
    target_language: Option<String>,
    never_translate: Option<Vec<String>>,
}

async fn set_settings(ctx: Arc<AppCtx>, p: SetSettings) -> anyhow::Result<SettingsDto> {
    let current = stored(&ctx).await?;
    let provider = p.provider.as_deref().map(Provider::parse).unwrap_or(current.provider);
    let mut keys_changed: Vec<&str> = Vec::new();
    if p.provider.is_some() {
        let id = provider.id().to_string();
        ctx.db.write(move |c| set_setting(c, PROVIDER_SETTING, &id)).await?;
        keys_changed.push(PROVIDER_SETTING);
    }
    if let Some(m) = p.model {
        let m = m.trim().to_string();
        if m.contains(['/', '?', '#', ' ']) {
            return Err(crate::user_err!("bad_model", "\"{m}\" isn't a valid model name."));
        }
        ctx.db.write(move |c| set_setting(c, MODEL_SETTING, &m)).await?;
        keys_changed.push(MODEL_SETTING);
    }
    if let Some(k) = p.key {
        let k = k.trim();
        if k.is_empty() {
            secrets::delete(provider.secret_name())?;
        } else {
            secrets::set(provider.secret_name(), k)?;
        }
    }
    if let Some(t) = p.target_language {
        let t = t.trim().to_string();
        let t = if t.is_empty() { DEFAULT_TARGET.to_string() } else { t };
        ctx.db.write(move |c| set_setting(c, TARGET_SETTING, &t)).await?;
        keys_changed.push(TARGET_SETTING);
    }
    if let Some(n) = p.never_translate {
        let mut n: Vec<String> = n.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        n.sort();
        n.dedup();
        ctx.db.write(move |c| set_setting(c, NEVER_SETTING, &n)).await?;
        keys_changed.push(NEVER_SETTING);
    }
    ctx.events.emit("ai.changed", json!({}));
    for k in keys_changed {
        ctx.events.emit("settings.changed", json!({ "key": k }));
    }
    get_settings(ctx, NoParams {}).await
}

#[derive(Deserialize)]
struct ConsentParams {
    /// true: the user accepted the AI notice; false: withdrawn (the notice shows again on next use)
    consent: bool,
}

async fn set_consent(ctx: Arc<AppCtx>, p: ConsentParams) -> anyhow::Result<SettingsDto> {
    ctx.db
        .write(move |c| {
            if p.consent {
                set_setting(c, CONSENT_SETTING, &chrono::Utc::now().timestamp_millis())
            } else {
                c.execute("DELETE FROM settings WHERE key=?1", [CONSENT_SETTING])?;
                Ok(())
            }
        })
        .await?;
    ctx.events.emit("ai.changed", json!({}));
    get_settings(ctx, NoParams {}).await
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TestParams {
    provider: Option<String>,
    model: Option<String>,
    /// an unsaved key typed in the Options page
    key: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TestResult {
    ok: bool,
    message: String,
}

/// "Test": a tiny round trip with the given (or saved) provider, model and key.
async fn test(ctx: Arc<AppCtx>, p: TestParams) -> anyhow::Result<TestResult> {
    let s = stored(&ctx).await?;
    let provider = p.provider.as_deref().map(Provider::parse).unwrap_or(s.provider);
    let model = p
        .model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .or(if provider == s.provider { s.model.clone() } else { None })
        .unwrap_or_else(|| s.default_for(provider));
    let typed = p.key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    let cfg = match typed.or_else(|| load_key(provider)) {
        Some(key) => AiConfig::new(provider, model, provider.default_base(), key, false),
        None if has_demo_account(&ctx).await? => AiConfig::new(provider, model, fake_base(&ctx).await?, fake::FAKE_KEY, true),
        None => return Ok(TestResult { ok: false, message: format!("Enter your {} API key first.", provider.label()) }),
    };
    let opts = CompleteOpts { max_tokens: 64, timeout: Duration::from_secs(30), ..Default::default() };
    let started = std::time::Instant::now();
    Ok(match complete_with(&ctx.http, &cfg, "", "Reply with the single word OK.", &opts).await {
        Ok(reply) => TestResult {
            ok: true,
            message: format!(
                "{} ({}) answered \"{}\" in {:.1} s.{}",
                provider.label(),
                cfg.model,
                reply.trim().chars().take(40).collect::<String>(),
                started.elapsed().as_secs_f32(),
                if cfg.demo { " (built-in demo provider — no key is saved)" } else { "" }
            ),
        },
        Err(e) => TestResult { ok: false, message: e.to_string() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{accounts, rpc};

    #[test]
    fn debug_redacts_key() {
        let c = AiConfig::new(Provider::Gemini, "m", "http://x", "AIzaSECRET-KEY-123", false);
        let s = format!("{c:?}");
        assert!(!s.contains("SECRET"), "{s}");
        assert!(s.contains("redacted"));
    }

    #[tokio::test]
    async fn bad_key_is_clear_and_never_echoed() {
        let base = fake::start().await.unwrap();
        let http = reqwest::Client::new();
        for p in [Provider::Gemini, Provider::Claude] {
            let key = "sk-test-very-secret-0123456789";
            let cfg = AiConfig::new(p, p.default_model(), &base, key, false);
            let err = complete_with(&http, &cfg, "", "hi", &CompleteOpts::default()).await.unwrap_err();
            let u = err.downcast_ref::<crate::error::UserError>().unwrap();
            assert_eq!(u.code, "ai_bad_key", "{p:?}: {}", u.message);
            assert!(!format!("{err:#} {err:?}").contains(key));
        }
        // quota: 429, no retry
        let cfg = AiConfig::new(Provider::Gemini, "gemini-2.5-flash", &base, fake::QUOTA_KEY, false);
        let t = std::time::Instant::now();
        let err = complete_cfg(&http, &cfg, "", "hi", &CompleteOpts::default()).await.unwrap_err();
        assert_eq!(err.downcast_ref::<crate::error::UserError>().unwrap().code, "ai_quota");
        assert!(t.elapsed() < Duration::from_millis(1000), "4xx must not be retried");
    }

    #[tokio::test]
    async fn complete_round_trip_both_providers() {
        let base = fake::start().await.unwrap();
        let http = reqwest::Client::new();
        for p in [Provider::Gemini, Provider::Claude] {
            let cfg = AiConfig::new(p, p.default_model(), &base, fake::FAKE_KEY, true);
            let r = complete_cfg(&http, &cfg, "", "Reply with the single word OK.", &CompleteOpts::default()).await.unwrap();
            assert_eq!(r, "OK");
            let v: Vec<String> = complete_json_cfg(
                &http,
                &cfg,
                translate::SYSTEM,
                &translate::prompt("English", &["שלום".into(), "עולם".into()]),
                CompleteOpts::default(),
            )
            .await
            .unwrap();
            assert_eq!(v, vec!["[EN] שלום", "[EN] עולם"]);
        }
    }

    #[test]
    fn real_providers_need_the_ai_notice_accepted() {
        let real = AiConfig::new(Provider::Claude, "m", "http://x", "k", false);
        let e = check_consent(&real, None).unwrap_err();
        assert_eq!(e.downcast_ref::<crate::error::UserError>().unwrap().code, "ai_consent");
        assert!(check_consent(&real, Some(1)).is_ok());
        // the demo provider sends nothing anywhere
        assert!(check_consent(&AiConfig::new(Provider::Gemini, "m", "http://x", "k", true), None).is_ok());
    }

    #[tokio::test]
    async fn consent_is_remembered_and_can_be_withdrawn() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let r = rpc::build();
        assert_eq!(r.call(ctx.clone(), "ai.getSettings", json!({})).await.unwrap()["consentAt"], Value::Null);
        let s = r.call(ctx.clone(), "ai.setConsent", json!({ "consent": true })).await.unwrap();
        assert!(s["consentAt"].as_i64().unwrap() > 0, "{s}");
        assert!(consent_at(&ctx).await.unwrap().is_some());
        let s = r.call(ctx.clone(), "ai.setConsent", json!({ "consent": false })).await.unwrap();
        assert_eq!(s["consentAt"], Value::Null);
    }

    #[tokio::test]
    async fn settings_rpc_and_demo_mode() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let r = rpc::build();
        let s = r.call(ctx.clone(), "ai.getSettings", json!({})).await.unwrap();
        assert_eq!(s["configured"], false);
        assert_eq!(s["provider"], "gemini");
        assert_eq!(s["effectiveModel"], "gemini-2.5-flash");
        assert_eq!(s["targetLanguage"], "English");
        accounts::upsert(&ctx.db, "demo@duvz.demo", "Demo", "demo").await.unwrap();
        let s = r.call(ctx.clone(), "ai.getSettings", json!({})).await.unwrap();
        assert_eq!(s["configured"], true);
        assert_eq!(s["demo"], true);
        let t = r.call(ctx.clone(), "ai.test", json!({})).await.unwrap();
        assert_eq!(t["ok"], true, "{t}");
        let s = r
            .call(ctx.clone(), "ai.setSettings", json!({ "provider": "claude", "neverTranslate": ["French", " French", ""] }))
            .await
            .unwrap();
        assert_eq!(s["provider"], "claude");
        assert_eq!(s["effectiveModel"], "claude-haiku-5-5");
        assert_eq!(s["neverTranslate"], json!(["French"]));
        // the settings payload never carries a key, only flags
        assert!(s.get("key").is_none());
        // a saved key: flags only, never echoed in results or events
        let secret = "sk-ant-test-SECRET-4242";
        let s = r.call(ctx.clone(), "ai.setSettings", json!({ "key": secret })).await.unwrap();
        assert_eq!(s["hasAnthropicKey"], true);
        // only demo accounts: the fake model stays in use (the keyring is shared with the real app)
        assert_eq!(s["demo"], true);
        assert!(!s.to_string().contains("SECRET"));
        let stored: Option<String> =
            ctx.db.read(|c| Ok(c.query_row("SELECT group_concat(value) FROM settings", [], |r| r.get(0))?)).await.unwrap();
        assert!(!stored.unwrap_or_default().contains("SECRET"), "keys never go in the settings table");
        assert!(!serde_json::to_string(&ctx.events.captured()).unwrap().contains("SECRET"));
        let s = r.call(ctx.clone(), "ai.setSettings", json!({ "key": "" })).await.unwrap();
        assert_eq!(s["hasAnthropicKey"], false);
    }
}

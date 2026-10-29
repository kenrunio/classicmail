//! Message translation (the new-Outlook "Translate message" bar).
//!
//! The front end collects the visible text nodes of the rendered body (plus the subject) and sends
//! them here as a list of strings. We translate only what isn't cached yet, in batches: each batch
//! goes to the model as a JSON array and must come back as a JSON array of the same length; a bad
//! batch is split in halves and retried down to single strings (a string that still fails keeps its
//! original text). Results are cached per (target language, text) in `ai_translations`, so
//! reopening a message — or any text seen before, like a signature — costs nothing.
//!
//! Privacy: only the texts of the message the user clicked Translate on are ever sent.

use std::{collections::HashMap, ops::Range, sync::Arc};

use futures::StreamExt;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{AiConfig, CompleteOpts};
use crate::{ctx::AppCtx, db::util::now_ms};

pub const SYSTEM: &str = "You are a professional translator for business e-mail (investor relations, finance). \
You receive a JSON object {\"target\": language, \"texts\": [strings]} — the text fragments of one e-mail, in reading order. \
Translate every fragment into the target language, naturally and accurately, keeping the meaning of fragments that continue each other. \
Keep names of people and companies, e-mail addresses, URLs, numbers, dates and ticker symbols as they are. \
A fragment already in the target language is returned unchanged. \
Return ONLY a JSON array of strings with exactly one translation per input fragment, in the same order.";

/// Batch limits: small enough for a fast, reliable reply; large enough to keep context.
pub const MAX_BATCH_CHARS: usize = 3500;
pub const MAX_BATCH_ITEMS: usize = 60;
const MAX_TOTAL_CHARS: usize = 250_000;
const MAX_TOTAL_ITEMS: usize = 8000;
const PARALLEL_BATCHES: usize = 3;

pub fn prompt(target: &str, texts: &[String]) -> String {
    json!({ "target": target, "texts": texts }).to_string()
}

/// Split consecutive items into batches of at most `max_chars` characters / `max_items` items
/// (a single over-long item gets a batch of its own).
pub fn batches(lens: &[usize], max_chars: usize, max_items: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = 0;
    for (i, &l) in lens.iter().enumerate() {
        if i > start && (chars + l > max_chars || i - start >= max_items) {
            out.push(start..i);
            start = i;
            chars = 0;
        }
        chars += l;
    }
    if start < lens.len() {
        out.push(start..lens.len());
    }
    out
}

/// The model's reply for a batch, if it is a JSON array of exactly `n` strings.
pub fn validate_reply(v: &serde_json::Value, n: usize) -> Option<Vec<String>> {
    let a = v.as_array().or_else(|| {
        // some models wrap the array: {"translations": [...]}
        v.as_object().and_then(|o| if o.len() == 1 { o.values().next().and_then(|x| x.as_array()) } else { None })
    })?;
    if a.len() != n {
        return None;
    }
    a.iter().map(|x| x.as_str().map(str::to_string)).collect()
}

/// Leading whitespace, core text, trailing whitespace (only the core is sent / cached).
pub fn split_ws(s: &str) -> (&str, &str, &str) {
    let core = s.trim();
    if core.is_empty() {
        return (s, "", "");
    }
    let lead = &s[..s.len() - s.trim_start().len()];
    let trail = &s[s.trim_end().len()..];
    (lead, core, trail)
}

fn hash(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    d.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Translate `texts` (cores, no surrounding whitespace) with the model: batched, validated,
/// halved on a bad reply. None = this text couldn't be translated (keep the original).
/// Errors that a smaller batch can't fix (bad key, quota, network…) are returned.
pub async fn translate_uncached(
    http: &reqwest::Client,
    cfg: &AiConfig,
    target: &str,
    texts: &[String],
) -> anyhow::Result<Vec<Option<String>>> {
    let lens: Vec<usize> = texts.iter().map(|t| t.chars().count()).collect();
    let mut out: Vec<Option<String>> = vec![None; texts.len()];
    let mut queue: Vec<Range<usize>> = batches(&lens, MAX_BATCH_CHARS, MAX_BATCH_ITEMS);
    while !queue.is_empty() {
        let round: Vec<Range<usize>> = std::mem::take(&mut queue);
        let results: Vec<(Range<usize>, anyhow::Result<Option<Vec<String>>>)> = futures::stream::iter(round)
            .map(|r| async move {
                let slice = &texts[r.clone()];
                let max_tokens = (slice.iter().map(|s| s.len()).sum::<usize>() as u32 * 2 + 256).clamp(512, 16_000);
                let opts =
                    CompleteOpts { max_tokens, temperature: Some(0.1), timeout: std::time::Duration::from_secs(90), ..Default::default() };
                let res = super::complete_json_cfg::<serde_json::Value>(http, cfg, SYSTEM, &prompt(target, slice), opts).await;
                let res = match res {
                    Ok(v) => Ok(validate_reply(&v, slice.len())),
                    Err(e) if is_batch_fault(&e) => Ok(None),
                    Err(e) => Err(e),
                };
                (r, res)
            })
            .buffer_unordered(PARALLEL_BATCHES)
            .collect()
            .await;
        for (r, res) in results {
            match res? {
                Some(tr) => {
                    for (i, t) in r.clone().zip(tr) {
                        out[i] = Some(t);
                    }
                }
                None if r.len() > 1 => {
                    let mid = r.start + r.len() / 2;
                    log::info!("ai: translation batch of {} came back malformed; retrying in halves", r.len());
                    queue.push(r.start..mid);
                    queue.push(mid..r.end);
                }
                None => log::info!("ai: one fragment couldn't be translated; keeping the original"),
            }
        }
    }
    Ok(out)
}

/// A reply problem that splitting the batch may fix.
fn is_batch_fault(e: &anyhow::Error) -> bool {
    e.downcast_ref::<crate::error::UserError>().is_some_and(|u| matches!(u.code, "ai_bad_reply" | "ai_too_long" | "ai_empty"))
}

// ---- cache ------------------------------------------------------------------------------------

async fn cache_get(ctx: &AppCtx, target: &str, cores: &[String]) -> anyhow::Result<HashMap<String, String>> {
    let target = target.to_string();
    let keys: Vec<(String, String)> = cores.iter().map(|c| (hash(c), c.clone())).collect();
    ctx.db
        .read(move |c| {
            let mut st = c.prepare_cached("SELECT translation FROM ai_translations WHERE target=?1 AND src_hash=?2")?;
            let mut m = HashMap::new();
            for (h, core) in keys {
                if let Some(t) = st.query_row(params![target, h], |r| r.get::<_, String>(0)).optional()? {
                    m.insert(core, t);
                }
            }
            Ok(m)
        })
        .await
}

async fn cache_put(ctx: &AppCtx, target: &str, message_id: i64, model: &str, pairs: Vec<(String, String)>) -> anyhow::Result<()> {
    if pairs.is_empty() {
        return Ok(());
    }
    let target = target.to_string();
    let model = model.to_string();
    ctx.db
        .write(move |c| {
            let tx = c.transaction()?;
            {
                let mut st = tx.prepare_cached(
                    "INSERT INTO ai_translations(target, src_hash, translation, message_id, model, created_at) VALUES (?1,?2,?3,?4,?5,?6) \
                     ON CONFLICT(target, src_hash) DO UPDATE SET translation=excluded.translation, message_id=excluded.message_id, model=excluded.model, created_at=excluded.created_at",
                )?;
                let now = now_ms();
                for (src, tr) in pairs {
                    st.execute(params![target, hash(&src), tr, message_id, model, now])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await
}

// ---- rpc --------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateParams {
    pub message_id: i64,
    pub texts: Vec<String>,
    /// default: the `ai.targetLanguage` setting
    pub target: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TranslateResult {
    /// same length and order as `texts` (untranslatable items keep their original text)
    pub translations: Vec<String>,
    pub target: String,
    /// fragments answered from the cache / by the model / left as they were
    pub cached: usize,
    pub translated: usize,
    pub failed: usize,
    pub demo: bool,
}

pub async fn translate_rpc(ctx: Arc<AppCtx>, p: TranslateParams) -> anyhow::Result<TranslateResult> {
    let total: usize = p.texts.iter().map(|t| t.len()).sum();
    if p.texts.len() > MAX_TOTAL_ITEMS || total > MAX_TOTAL_CHARS {
        return Err(crate::user_err!("ai_too_long", "This message is too long to translate."));
    }
    let target = match p.target.filter(|t| !t.trim().is_empty()) {
        Some(t) => t.trim().to_string(),
        None => super::stored(&ctx).await?.target,
    };
    let split: Vec<(&str, &str, &str)> = p.texts.iter().map(|t| split_ws(t)).collect();
    // unique non-empty cores, in order of first appearance
    let mut cores: Vec<String> = Vec::new();
    {
        let mut seen = std::collections::HashSet::new();
        for (_, core, _) in &split {
            if !core.is_empty() && seen.insert(*core) {
                cores.push(core.to_string());
            }
        }
    }
    let mut known = cache_get(&ctx, &target, &cores).await?;
    let cached = cores.iter().filter(|c| known.contains_key(*c)).count();
    let missing: Vec<String> = cores.iter().filter(|c| !known.contains_key(*c)).cloned().collect();
    let mut translated = 0;
    let mut demo = false;
    if !missing.is_empty() {
        let cfg = super::require_use(&ctx).await?;
        demo = cfg.demo;
        let started = std::time::Instant::now();
        let res = translate_uncached(&ctx.http, &cfg, &target, &missing).await?;
        let pairs: Vec<(String, String)> = missing.iter().zip(res).filter_map(|(src, tr)| tr.map(|t| (src.clone(), t))).collect();
        translated = pairs.len();
        log::info!(
            "ai: translated {translated}/{} fragments of message {} into {target} ({} cached) in {} ms",
            missing.len(),
            p.message_id,
            cached,
            started.elapsed().as_millis()
        );
        for (s, t) in &pairs {
            known.insert(s.clone(), t.clone());
        }
        cache_put(&ctx, &target, p.message_id, &cfg.model, pairs).await?;
    }
    let failed = cores.len() - cached - translated;
    let translations = split
        .iter()
        .zip(&p.texts)
        .map(|((lead, core, trail), orig)| match known.get(*core) {
            Some(t) if !core.is_empty() => format!("{lead}{t}{trail}"),
            _ => orig.clone(),
        })
        .collect();
    Ok(TranslateResult { translations, target, cached, translated, failed, demo })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{accounts, ai::Provider, rpc};

    #[test]
    fn batching() {
        assert_eq!(batches(&[], 10, 5), Vec::<Range<usize>>::new());
        assert_eq!(batches(&[3, 3, 3, 3], 7, 10), vec![0..2, 2..4]);
        assert_eq!(batches(&[3, 3, 3, 3], 100, 3), vec![0..3, 3..4]);
        // an over-long item gets its own batch
        assert_eq!(batches(&[2, 50, 2], 10, 10), vec![0..1, 1..2, 2..3]);
        // every item is covered exactly once
        let lens: Vec<usize> = (0..500).map(|i| (i * 37) % 200).collect();
        let b = batches(&lens, MAX_BATCH_CHARS, MAX_BATCH_ITEMS);
        assert_eq!(b.first().unwrap().start, 0);
        assert_eq!(b.last().unwrap().end, 500);
        for w in b.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
        for r in &b {
            assert!(r.len() <= MAX_BATCH_ITEMS);
            assert!(r.len() == 1 || lens[r.clone()].iter().sum::<usize>() <= MAX_BATCH_CHARS);
        }
    }

    #[test]
    fn reply_validation() {
        assert_eq!(validate_reply(&json!(["a", "b"]), 2), Some(vec!["a".into(), "b".into()]));
        assert_eq!(validate_reply(&json!(["a"]), 2), None, "length mismatch");
        assert_eq!(validate_reply(&json!(["a", 1]), 2), None, "non-string item");
        assert_eq!(validate_reply(&json!({ "translations": ["a", "b"] }), 2), Some(vec!["a".into(), "b".into()]));
        assert_eq!(validate_reply(&json!({ "a": 1 }), 1), None);
    }

    #[test]
    fn whitespace_kept_outside() {
        assert_eq!(split_ws("  שלום \n"), ("  ", "שלום", " \n"));
        assert_eq!(split_ws("x"), ("", "x", ""));
        assert_eq!(split_ws("   "), ("   ", "", ""));
    }

    #[tokio::test]
    async fn malformed_batches_fall_back_to_halves() {
        let base = crate::ai::fake::start().await.unwrap();
        let cfg = AiConfig::new(Provider::Gemini, "gemini-2.5-flash", &base, crate::ai::fake::FAKE_KEY, true);
        // the fake drops one item from any batch containing the marker → halves, then singles
        let texts: Vec<String> = vec!["a".into(), "b".into(), format!("c {}", crate::ai::fake::DROP_MARKER), "d".into()];
        let out = translate_uncached(&reqwest::Client::new(), &cfg, "English", &texts).await.unwrap();
        assert_eq!(out[0].as_deref(), Some("[EN] a"));
        assert_eq!(out[1].as_deref(), Some("[EN] b"));
        assert_eq!(out[2], None, "the bad one keeps its original");
        assert_eq!(out[3].as_deref(), Some("[EN] d"));
    }

    #[tokio::test]
    async fn rpc_translates_caches_and_keeps_whitespace() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        accounts::upsert(&ctx.db, "demo@duvz.demo", "Demo", "demo").await.unwrap();
        let r = rpc::build();
        let texts = json!(["סיכום פגישה", "  קני שלום,\n", "\n", "קני שלום,", "https://example.com"]);
        let a = r.call(ctx.clone(), "ai.translate", json!({ "messageId": 7, "texts": texts })).await.unwrap();
        assert_eq!(
            a["translations"],
            json!(["[EN] סיכום פגישה", "  [EN] קני שלום,\n", "\n", "[EN] קני שלום,", "[EN] https://example.com"])
        );
        assert_eq!(a["target"], "English");
        assert_eq!(a["translated"], 3, "duplicates are sent once: {a}");
        assert_eq!(a["cached"], 0);
        assert_eq!(a["demo"], true);
        // second time: all from the cache, no model call
        let b = r.call(ctx.clone(), "ai.translate", json!({ "messageId": 7, "texts": texts })).await.unwrap();
        assert_eq!(b["translations"], a["translations"]);
        assert_eq!(b["cached"], 3);
        assert_eq!(b["translated"], 0);
        let n: i64 =
            ctx.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM ai_translations WHERE message_id=7", [], |r| r.get(0))?)).await.unwrap();
        assert_eq!(n, 3);
        // another target is a separate cache entry
        let c = r.call(ctx.clone(), "ai.translate", json!({ "messageId": 7, "texts": ["שלום"], "target": "French" })).await.unwrap();
        assert_eq!(c["translations"], json!(["[FR] שלום"]));
    }

    #[tokio::test]
    async fn no_key_no_demo_is_a_clear_error() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let e = rpc::build().call(ctx.clone(), "ai.translate", json!({ "messageId": 1, "texts": ["שלום"] })).await.unwrap_err();
        assert_eq!(e.downcast_ref::<crate::error::UserError>().unwrap().code, "ai_not_configured");
    }
}

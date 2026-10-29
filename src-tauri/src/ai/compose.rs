//! AI help while writing (docs/superpowers/specs/2026-10-13-reply-ai-design.md):
//! * `ai.compose.translate { html, language }` → `{ html }` — the user's reply in informal e-mail
//!   register of `language` (Translate to Hebrew / Back to English in the compose window);
//! * `ai.compose.draftReply { messageId }` → `{ html }` — a short English reply to a message
//!   (Reply with AI);
//! * `ai.compose.review { html, context, fields }` → `{ html, applied }` — carries out the
//!   `{instructions}` in the user's text (AI Review); `fields` are mail-merge column names, whose
//!   `{Field}` placeholders are not instructions. Square brackets are ordinary text.
//!
//! The model gets JSON in the prompt and answers JSON; HTML coming back is sanitised like quoted
//! mail before it reaches the editor.

use std::{collections::HashSet, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{CompleteOpts, complete_json};
use crate::{
    ctx::AppCtx,
    mail::{
        compose::{
            prepare::{esc, sanitize},
            store,
        },
        merge::fill::key_norm,
        mime::{html_to_text, strip_tags},
    },
    rpc::Registry,
};

pub fn register(r: &mut Registry) {
    r.add("ai.compose.translate", translate);
    r.add("ai.compose.draftReply", draft_reply);
    r.add("ai.compose.review", review);
}

/// How much of a conversation the model reads for a draft / review (characters).
const MAX_CONTEXT: usize = 30_000;

const TRANSLATE_SYSTEM: &str = "You translate the user's own e-mail text into the language given as \"language\". \
Write it the way a native speaker writes everyday work e-mail: natural, friendly and direct, short sentences — informal-but-professional, \
never formal, literary or stiff. For Hebrew: modern Israeli work e-mail style (phrases like \"היי\", \"תודה רבה\", \"מצ\\\"ב\", \"אשמח\" are fine), \
not formal or biblical phrasing. Translate everything, including greetings and sign-offs. \
Write people's names the way they are normally written in that language (in Hebrew: Kenny → קני, Dana → דנה, Moshe → משה). \
Keep exactly: the HTML structure (the same tags, attributes, links and line breaks, the same number of paragraphs), company and product names, \
tickers, numbers, dates, phone numbers, URLs and e-mail addresses. Do not add or drop content. \
Answer with JSON only: {\"html\": \"<the translated HTML>\"}.";

const DRAFT_SYSTEM: &str = "You draft a short reply e-mail for the user (\"me\"). The conversation is given newest message first; \
reply to the newest message. Write in English, 2 to 6 sentences, matching the tone of the thread (usually friendly-professional). \
Start with a first-name greeting when the thread uses them (e.g. \"Hi Dan,\"). Answer what was asked, or acknowledge and give the next step. \
You may end with a short closing such as \"Thanks,\" or \"Best,\" but no name and no signature (it is added separately), no subject line, \
no quoted text. Never invent facts, dates, numbers, prices or commitments: where the reply needs something you don't know, \
write a short placeholder in square brackets, e.g. [date] or [confirm the amount]. \
Answer with JSON only: {\"paragraphs\": [\"<paragraph 1>\", \"<paragraph 2>\"]} — plain text, one string per paragraph.";

const REVIEW_SYSTEM: &str = "You edit the user's e-mail draft (HTML). Text inside curly braces {like this} is an instruction \
from the user to you, not part of the e-mail. Square brackets [like this] are ordinary e-mail text, not instructions: leave them exactly as they are. \
The names listed in \"fields\" are mail-merge fields: {First Name} and the like, written with one of those names, are not instructions — leave them exactly as they are. \
Carry out every instruction at exactly that spot: either replace the instruction with the text it asks for \
(\"Dear {first names of everyone on the thread}\" → \"Dear Dan, Ruth and Moshe\"; \"{thank John for the great work}\" → a sentence doing that), \
or change the surrounding sentence or paragraph as it asks (\"{make this paragraph more polite}\"), then remove the instruction. \
Write what you add in the language of the surrounding text, in the same voice. Use the \"context\" (the conversation being answered, \
recipients, subject) for names and facts; never invent facts — if something needed is unknown, write [?] there. \
Everything outside the instructions stays exactly as written: the same words, language, HTML tags, formatting and links. \
You can look things up with the tools — the user's calendar (free time, what's on, meetings and their details), their tasks, and \
exact time-zone conversions. When an instruction needs such facts, call the tools; never guess dates, times or availability. \
Dates like \"next Tuesday\" are relative to \"today\" in the user's \"time_zone\". Unless the instruction names another person's calendar, \
only the user's own calendar counts. When you offer meeting times, give the weekday, date and time with the time zone \
(\"Tuesday 27 October at 14:00 Israel time\"), usually two or three options on different days; when the recipients are clearly in \
another country, ask find_free_time for their time zone too and give both times. If the tools give no answer (no free time, no such \
meeting), write [?] with a short note instead of inventing one. \
Answer with JSON only: {\"html\": \"<the edited HTML>\"}.";

#[derive(Deserialize)]
struct HtmlReply {
    html: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HtmlOut {
    pub html: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied: Option<usize>,
}

fn opts() -> CompleteOpts {
    CompleteOpts { temperature: Some(0.3), timeout: std::time::Duration::from_secs(90), ..Default::default() }
}

/// AI-made HTML is treated like someone else's: allow-list sanitised.
fn clean(html: &str) -> String {
    sanitize(html.trim())
}

#[derive(Deserialize)]
struct TranslateParams {
    html: String,
    language: String,
}

async fn translate(ctx: Arc<AppCtx>, p: TranslateParams) -> anyhow::Result<HtmlOut> {
    if strip_tags(&p.html).trim().is_empty() {
        return Err(crate::user_err!("ai_nothing", "There is no text to translate yet."));
    }
    let prompt = json!({ "task": "compose.translate", "language": p.language, "html": p.html }).to_string();
    let r: HtmlReply = complete_json(&ctx, TRANSLATE_SYSTEM, &prompt, opts()).await?;
    let html = clean(&r.html);
    if strip_tags(&html).trim().is_empty() {
        return Err(crate::user_err!("ai_bad_reply", "The AI returned an empty translation."));
    }
    Ok(HtmlOut { html, applied: None })
}

// ---- instructions ---------------------------------------------------------------------------

/// The `{…}` instructions in a text, in order, leaving out mail-merge placeholders (`fields`,
/// compared the way the merge does: `First Name` = `first_name`). `{?}` is not an instruction.
pub fn instructions(text: &str, fields: &[String]) -> Vec<String> {
    let fields: HashSet<String> = fields.iter().map(|f| key_norm(f)).collect();
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch != '{' {
            continue;
        }
        if let Some(end) = text[i + 1..].find(['}', '\n']).map(|e| i + 1 + e)
            && text[end..].starts_with('}')
        {
            let inner = text[i + 1..end].trim();
            if !inner.is_empty() && inner != "?" && !inner.contains('{') && !fields.contains(&key_norm(inner)) {
                out.push(inner.to_string());
            }
            while chars.peek().is_some_and(|(j, _)| *j <= end) {
                chars.next();
            }
        }
    }
    out
}

/** Instructions in an HTML text, counted on its visible text (html_to_text would add "[link][1]"
 *  footnotes that look like instructions). */
pub fn count_instructions(html: &str, fields: &[String]) -> usize {
    instructions(&strip_tags(html), fields).len()
}

#[derive(Deserialize)]
struct ReviewParams {
    html: String,
    #[serde(default)]
    context: String,
    /// mail-merge column names
    #[serde(default)]
    fields: Vec<String>,
}

async fn review(ctx: Arc<AppCtx>, p: ReviewParams) -> anyhow::Result<HtmlOut> {
    let before = count_instructions(&p.html, &p.fields);
    if before == 0 {
        return Ok(HtmlOut { html: p.html, applied: Some(0) });
    }
    let context: String = p.context.chars().take(MAX_CONTEXT).collect();
    // when it is, for "next Tuesday" and the calendar look-ups (ai/tools.rs)
    let now = crate::clock::now_ms();
    let (today, today_date, time, zone, me) = ctx
        .db
        .read(move |c| {
            let env = super::tools::Env::load(c, now)?;
            let t = chrono::TimeZone::timestamp_millis_opt(&env.tz, now).single().ok_or_else(|| anyhow::anyhow!("no local time"))?;
            let me: Option<String> = c
                .query_row(
                    "SELECT display_name || ' <' || email || '>' FROM accounts WHERE enabled=1 ORDER BY position, id LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .ok();
            Ok((
                t.format("%A %-d %B %Y").to_string(),
                t.format("%Y-%m-%d").to_string(),
                t.format("%H:%M").to_string(),
                env.tz.name().to_string(),
                me,
            ))
        })
        .await?;
    let prompt = json!({
        "task": "compose.review", "fields": p.fields, "html": p.html, "context": context,
        "today": today, "today_date": today_date, "time_now": time, "time_zone": zone, "me": me,
    })
    .to_string();
    let mut html = String::new();
    let mut left = before;
    for _ in 0..2 {
        let r: HtmlReply = super::complete_json_tools(&ctx, REVIEW_SYSTEM, &prompt, opts()).await?;
        html = clean(&r.html);
        left = count_instructions(&html, &p.fields);
        if left == 0 && !strip_tags(&html).trim().is_empty() {
            break;
        }
    }
    if strip_tags(&html).trim().is_empty() {
        return Err(crate::user_err!("ai_bad_reply", "The AI returned an empty text."));
    }
    Ok(HtmlOut { html, applied: Some(before.saturating_sub(left)) })
}

// ---- Reply with AI ----------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DraftParams {
    message_id: i64,
}

#[derive(Deserialize)]
struct DraftReply {
    paragraphs: Vec<String>,
}

fn names(v: &[crate::mail::compose::model::Addr]) -> String {
    v.iter().map(|a| a.display()).collect::<Vec<_>>().join("; ")
}

/// What the model reads: the message's headers and its full text (which carries the quoted
/// history below it), capped.
pub fn thread_context(src: &store::SourceMsg) -> String {
    let body = match (&src.text, &src.html) {
        (Some(t), _) if !t.trim().is_empty() => t.clone(),
        (_, Some(h)) => html_to_text(h),
        _ => String::new(),
    };
    let date = chrono::DateTime::from_timestamp_millis(src.date).map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default();
    let mut s = format!(
        "From: {}\nTo: {}\nCc: {}\nDate: {date}\nSubject: {}\n\n{body}",
        src.from.display(),
        names(&src.to),
        names(&src.cc),
        src.subject
    );
    if s.chars().count() > MAX_CONTEXT {
        s = s.chars().take(MAX_CONTEXT).collect();
    }
    s
}

async fn draft_reply(ctx: Arc<AppCtx>, p: DraftParams) -> anyhow::Result<HtmlOut> {
    let id = p.message_id;
    if let Err(e) = crate::mail::ensure_body(&ctx, id).await {
        log::debug!("ai draft: ensure_body({id}): {}", crate::logging::err(&e));
    }
    let (src, me) = ctx
        .db
        .read(move |c| {
            let src = store::load_source(c, id)?;
            let me = match &src {
                Some(s) => store::identity(c, s.account_id, None).ok(),
                None => None,
            };
            Ok((src, me))
        })
        .await?;
    let src = src.ok_or_else(|| crate::user_err!("not_found", "The original message could not be found."))?;
    let me = me.map(|a| a.display()).unwrap_or_default();
    let prompt = json!({ "task": "compose.draft", "me": me, "thread": thread_context(&src) }).to_string();
    let r: DraftReply = complete_json(&ctx, DRAFT_SYSTEM, &prompt, opts()).await?;
    let paras: Vec<&str> = r.paragraphs.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if paras.is_empty() {
        return Err(crate::user_err!("ai_bad_reply", "The AI returned an empty draft."));
    }
    let html = paras.iter().map(|s| format!("<p>{}</p>", esc(s).replace('\n', "<br>"))).collect::<String>();
    Ok(HtmlOut { html, applied: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{accounts, rpc};
    use serde_json::Value;

    /// A demo profile: the fake model answers (never a real AI).
    async fn demo() -> (Arc<AppCtx>, crate::db::tempdir::TempDirGuard, i64) {
        let (ctx, g) = AppCtx::for_test().unwrap();
        let a = accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny Green", "demo").await.unwrap();
        (ctx, g, a.id)
    }

    async fn call(ctx: &Arc<AppCtx>, method: &str, params: Value) -> anyhow::Result<Value> {
        rpc::build().call(ctx.clone(), method, params).await
    }

    #[tokio::test]
    async fn translate_keeps_structure_and_sanitises() {
        let (ctx, _g, _) = demo().await;
        let r = call(
            &ctx,
            "ai.compose.translate",
            json!({ "html": "<p>Hi Dan,</p><p>See <b>attached</b>.<script>x()</script></p>", "language": "Hebrew" }),
        )
        .await
        .unwrap();
        assert_eq!(r["html"], "<p>[HE] Hi Dan,</p><p>[HE] See <b>attached</b>.</p>");
        let e = call(&ctx, "ai.compose.translate", json!({ "html": "<p> </p>", "language": "Hebrew" })).await.unwrap_err();
        assert!(e.to_string().contains("no text"), "{e}");
    }

    #[tokio::test]
    async fn review_carries_out_instructions() {
        let (ctx, _g, _) = demo().await;
        let r =
            call(&ctx, "ai.compose.review", json!({ "html": "<p>Dear {first names}, {thank him} [ref 12].</p>", "context": "From: Dan" }))
                .await
                .unwrap();
        assert_eq!(r["html"], "<p>Dear FIRST NAMES, THANK HIM [ref 12].</p>");
        assert_eq!(r["applied"], 2);
        // mail merge: {fields} stay
        let r = call(&ctx, "ai.compose.review", json!({ "html": "<p>Dear {First Name}, {thank him}</p>", "fields": ["first_name"] }))
            .await
            .unwrap();
        assert_eq!(r["html"], "<p>Dear {First Name}, THANK HIM</p>");
        assert_eq!(r["applied"], 1);
        // an instruction the model leaves: retried once, then the partial result
        let r = call(&ctx, "ai.compose.review", json!({ "html": "<p>{a} {FAKE-KEEP b}</p>" })).await.unwrap();
        assert_eq!(r["html"], "<p>A {FAKE-KEEP b}</p>");
        assert_eq!(r["applied"], 1);
        // square brackets are ordinary text
        let r = call(&ctx, "ai.compose.review", json!({ "html": "<p>[EXTERNAL] see [1]</p>" })).await.unwrap();
        assert_eq!(r["applied"], 0);
        // links are not instructions
        let r =
            call(&ctx, "ai.compose.review", json!({ "html": "<p>See <a href=\"https://x.example/deck\">the deck</a>. {thank Dan}</p>" }))
                .await
                .unwrap();
        assert_eq!(r["applied"], 1);
        assert!(r["html"].as_str().unwrap().contains("THANK DAN"), "{r}");
        // nothing to do: no model call, text unchanged
        let r = call(&ctx, "ai.compose.review", json!({ "html": "<p>Plain text</p>" })).await.unwrap();
        assert_eq!(r["applied"], 0);
        assert_eq!(r["html"], "<p>Plain text</p>");
    }

    #[tokio::test]
    async fn draft_reply_reads_the_message() {
        let (ctx, _g, acct) = demo().await;
        let id = ctx
            .db
            .write(move |c| {
                c.execute(
                    "INSERT INTO messages(account_id, gmail_id, thread_id, internal_date, sent_date, subject, from_name, from_email, to_json, cc_json, body_state) \
                     VALUES (?1,'g1','t1',0,0,'Q3 call','Dan Levi','dan@harbor.example','[{\"name\":\"Kenny Green\",\"email\":\"kenny@duvz.demo\"}]','[]',1)",
                    [acct],
                )?;
                let id = c.last_insert_rowid();
                c.execute("INSERT INTO message_bodies(message_id, html, text, fetched_at) VALUES (?1, NULL, 'Can we meet on Tuesday?', 0)", [id])?;
                Ok(id)
            })
            .await
            .unwrap();
        let r = call(&ctx, "ai.compose.draftReply", json!({ "messageId": id })).await.unwrap();
        assert_eq!(
            r["html"],
            "<p>Thanks for your note (Dan Levi &lt;dan@harbor.example&gt;).</p><p>I'll get back to you by [date].<br>Best,</p>"
        );
        let e = call(&ctx, "ai.compose.draftReply", json!({ "messageId": 9999 })).await.unwrap_err();
        assert!(e.to_string().contains("could not be found"), "{e}");
    }

    #[test]
    fn context_is_capped_and_has_headers() {
        let src = store::SourceMsg {
            subject: "Q3".into(),
            from: crate::mail::compose::model::Addr::new("Dan", "dan@x.com"),
            text: Some("x".repeat(MAX_CONTEXT * 2)),
            ..Default::default()
        };
        let c = thread_context(&src);
        assert!(c.starts_with("From: Dan <dan@x.com>\nTo: \nCc: "), "{}", &c[..60]);
        assert!(c.contains("Subject: Q3"));
        assert_eq!(c.chars().count(), MAX_CONTEXT);
    }

    #[test]
    fn links_are_not_instructions() {
        let html = r#"<p>See <a href="https://x.example/deck">the deck</a> and <a href="mailto:d@x.example">Dan</a>. {thank Dan}</p>"#;
        assert_eq!(count_instructions(html, &[]), 1);
    }

    #[test]
    fn finds_instructions() {
        assert_eq!(instructions("Dear {first names}, see [the deck] {?} {} x", &[]), vec!["first names"]);
        assert_eq!(instructions("Dear {First Name}, {thank him}", &["first_name".into()]), vec!["thank him"]);
        assert!(instructions("no {unclosed\n} here", &[]).is_empty());
        assert_eq!(instructions("{a} and {b}", &[]).len(), 2);
    }
}

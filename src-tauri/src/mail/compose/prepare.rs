//! `mail.compose.prepare`: the initial contents of a compose window (new / reply / reply all /
//! forward), Outlook 2007 style.

use std::{collections::HashSet, sync::Arc};

use base64::Engine;
use chrono::TimeZone;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{
    attachments::attachment_data,
    model::{Addr, AttachmentSpec, Compose, Mode},
    store::{self, FromChoice, SourceMsg},
};
use crate::ctx::AppCtx;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareParams {
    pub mode: Mode,
    pub id: Option<i64>,
    pub account_id: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub compose: Compose,
    /// editable part (TipTap): blank lines + signature (+ the draft's own text)
    pub body_html: String,
    /// quoted original ("-----Original Message-----" block), kept as raw HTML
    pub quoted_html: String,
    pub from_choices: Vec<FromChoice>,
    /// local id of the draft message when opened from Drafts
    pub draft_message_id: Option<i64>,
    /// replies: the recipients of a plain Reply and of a Reply All (the Reply to One toggle)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_targets: Option<ReplyTargets>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Recipients {
    pub to: Vec<Addr>,
    pub cc: Vec<Addr>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ReplyTargets {
    pub one: Recipients,
    pub all: Recipients,
}

/// Marker element wrapping the quoted original, so a reopened draft can split it off again.
pub const ORIGINAL_MARKER: &str = "<div id=\"cm-original\"";

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// "RE: " / "FW: " without doubling (Outlook: RE: RE: x stays RE: x; FW: x replied → RE: FW: x).
pub fn prefixed_subject(subject: &str, mode: Mode) -> String {
    let s = subject.trim();
    let (want, known): (&str, &[&str]) = match mode {
        Mode::Reply | Mode::ReplyAll => ("RE", &["re", "aw", "sv", "השב"]),
        Mode::Forward => ("FW", &["fw", "fwd", "wg", "tr", "הועבר"]),
        _ => return s.to_string(),
    };
    if let Some((head, rest)) = s.split_once(':') {
        let h = head.trim().to_lowercase();
        if known.contains(&h.as_str()) {
            return format!("{want}: {}", rest.trim_start());
        }
    }
    format!("{want}: {s}")
}

/// "Sunday, October 11, 2026 10:15 AM" in local time (Outlook's Sent: line).
pub fn outlook_date(ms: i64) -> String {
    match chrono::Local.timestamp_millis_opt(ms).single() {
        Some(d) => d.format("%A, %B %-d, %Y %-I:%M %p").to_string(),
        None => String::new(),
    }
}

fn names(v: &[Addr]) -> String {
    v.iter().map(|a| if a.name.trim().is_empty() { a.email.clone() } else { a.name.clone() }).collect::<Vec<_>>().join("; ")
}

/// Allow-list sanitiser for quoting someone else's HTML in our editor (no scripts / handlers).
pub fn sanitize(html: &str) -> String {
    if crate::mail::mime::html_too_deep(html) {
        return text_to_html(&crate::mail::mime::strip_tags(html));
    }
    let mut b = ammonia::Builder::default();
    b.add_generic_attributes(&[
        "style",
        "class",
        "dir",
        "align",
        "valign",
        "bgcolor",
        "width",
        "height",
        "border",
        "cellpadding",
        "cellspacing",
        "color",
        "face",
        "size",
        "lang",
    ])
    .add_tags(&["font", "center", "span", "div", "u", "s", "strike", "hr"])
    .add_url_schemes(&["cid", "data", "mailto"])
    .add_generic_attributes(&["id"])
    .link_rel(Some("noopener noreferrer"))
    .strip_comments(true)
    // the only id kept is our quote marker (a reopened draft is split on it)
    .attribute_filter(|_, attr, value| if attr == "id" && value != "cm-original" { None } else { Some(value.into()) });
    b.clean(html).to_string()
}

/// `type/subtype` safe to put into a `data:` URL inside an attribute (it comes from the message).
fn safe_image_mime(mime: &str) -> Option<&str> {
    let m = mime.trim();
    let ok = m.len() <= 100
        && m.split_once('/').is_some_and(|(t, st)| {
            t.eq_ignore_ascii_case("image") && !st.is_empty() && st.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        });
    ok.then_some(m)
}

fn text_to_html(t: &str) -> String {
    let mut out = String::new();
    for line in t.replace("\r\n", "\n").split('\n') {
        out.push_str(&esc(line));
        out.push_str("<br>");
    }
    format!("<div style=\"font-family:Calibri,sans-serif;font-size:11pt\" dir=\"auto\">{out}</div>")
}

/// The "-----Original Message-----" header + original body, wrapped in the marker div.
pub fn quoted_block(src: &SourceMsg, body_html: &str) -> String {
    let mut rows = format!("<b>From:</b> {}<br>", esc(&src.from.outlook_display()));
    rows.push_str(&format!("<b>Sent:</b> {}<br>", esc(&outlook_date(src.date))));
    if !src.to.is_empty() {
        rows.push_str(&format!("<b>To:</b> {}<br>", esc(&names(&src.to))));
    }
    if !src.cc.is_empty() {
        rows.push_str(&format!("<b>Cc:</b> {}<br>", esc(&names(&src.cc))));
    }
    rows.push_str(&format!("<b>Subject:</b> {}", esc(&src.subject)));
    format!(
        "{ORIGINAL_MARKER}><p style=\"margin:0\">&nbsp;</p><div style=\"font-family:Tahoma,sans-serif;font-size:10pt\"><p style=\"margin:0\">\
         -----Original Message-----<br>{rows}</p></div><p style=\"margin:0\">&nbsp;</p>{body_html}</div>"
    )
}

/// Reply / reply-all recipients (own addresses removed, duplicates dropped).
pub fn reply_recipients(src: &SourceMsg, all: bool, own: &HashSet<String>) -> (Vec<Addr>, Vec<Addr>) {
    let is_own = |a: &Addr| own.contains(&a.email.trim().to_lowercase());
    let from_me = is_own(&src.from);
    let sender: Vec<Addr> = if !src.reply_to.is_empty() { src.reply_to.clone() } else { vec![src.from.clone()] };
    let mut to: Vec<Addr> = if from_me {
        // replying to something I sent goes back to its recipients
        src.to.clone()
    } else if all {
        sender.iter().cloned().chain(src.to.iter().cloned()).collect()
    } else {
        sender.clone()
    };
    let mut cc: Vec<Addr> = if all { src.cc.clone() } else { vec![] };
    let mut seen = HashSet::new();
    let keep_self_if_only = to.iter().all(&is_own);
    to.retain(|a| (keep_self_if_only || !is_own(a)) && seen.insert(a.email.trim().to_lowercase()));
    cc.retain(|a| !is_own(a) && seen.insert(a.email.trim().to_lowercase()));
    if to.is_empty() {
        to = sender;
    }
    (to, cc)
}

/// Which of my addresses the source was sent to (reply from the same alias).
fn reply_from(src: &SourceMsg, choices: &[FromChoice]) -> Option<String> {
    let mine: Vec<&FromChoice> = choices.iter().filter(|c| c.account_id == src.account_id).collect();
    for a in src.to.iter().chain(src.cc.iter()) {
        if let Some(c) = mine.iter().find(|c| c.email.eq_ignore_ascii_case(a.email.trim())) {
            return Some(c.email.clone());
        }
    }
    mine.iter().find(|c| c.primary).map(|c| c.email.clone())
}

pub fn signature_for(c: &rusqlite::Connection, account_id: i64, reply: bool) -> anyhow::Result<Option<String>> {
    let col = if reply { "reply_signature_id" } else { "new_signature_id" };
    Ok(c.query_row(
        &format!("SELECT s.html FROM signature_defaults d JOIN signatures s ON s.id = d.{col} WHERE d.account_id=?1"),
        [account_id],
        |r| r.get::<_, String>(0),
    )
    .optional()?)
}

/// Editable part: two blank lines then the signature (Outlook puts the cursor at the top).
pub fn body_with_signature(sig: Option<&str>) -> String {
    match sig {
        Some(s) if !s.trim().is_empty() => format!("<p></p><p></p>{s}"),
        _ => "<p></p>".to_string(),
    }
}

pub async fn prepare(ctx: Arc<AppCtx>, p: PrepareParams) -> anyhow::Result<Prepared> {
    if p.mode == Mode::Draft {
        let id = p.id.ok_or_else(|| crate::user_err!("bad_params", "draft id missing"))?;
        return super::drafts::open_draft(ctx, id).await;
    }
    let choices = ctx.db.read(store::from_choices).await?;
    if p.mode == Mode::New || p.id.is_none() {
        let account_id = match p.account_id {
            Some(a) => a,
            None => default_account(&ctx, &choices).await?,
        };
        let sig = ctx.db.read(move |c| signature_for(c, account_id, false)).await?;
        let from = choices.iter().find(|c| c.account_id == account_id && c.primary).map(|c| c.email.clone());
        return Ok(Prepared {
            compose: Compose { account_id, from, mode: Mode::New, ..Default::default() },
            body_html: body_with_signature(sig.as_deref()),
            quoted_html: String::new(),
            from_choices: choices,
            draft_message_id: None,
            reply_targets: None,
        });
    }
    let id = p.id.unwrap();
    // make sure the body is local (mail-core); fall back to whatever is stored
    if let Err(e) = crate::mail::ensure_body(&ctx, id).await {
        log::debug!("compose prepare: ensure_body({id}): {}", crate::logging::err(&e));
    }
    let (src, own) = ctx.db.read(move |c| Ok((store::load_source(c, id)?, store::own_addresses(c)?))).await?;
    let src = src.ok_or_else(|| crate::user_err!("not_found", "The original message could not be found."))?;
    let account_id = p.account_id.unwrap_or(src.account_id);
    let reply = matches!(p.mode, Mode::Reply | Mode::ReplyAll);
    let sig = ctx.db.read(move |c| signature_for(c, account_id, true)).await?;

    // original body, sanitised, with inline images made self-contained
    let original = match &src.html {
        Some(h) => inline_cid_images(&ctx, &src, &sanitize(h)).await,
        None => text_to_html(src.text.as_deref().unwrap_or("")),
    };
    let quoted = quoted_block(&src, &original);

    let mut compose = Compose {
        account_id,
        from: reply_from(&src, &choices).filter(|_| account_id == src.account_id),
        subject: prefixed_subject(&src.subject, p.mode),
        mode: p.mode,
        source_message_id: Some(src.id),
        ..Default::default()
    };
    if compose.from.is_none() {
        compose.from = choices.iter().find(|c| c.account_id == account_id && c.primary).map(|c| c.email.clone());
    }
    let mut reply_targets = None;
    if reply {
        let (one_to, one_cc) = reply_recipients(&src, false, &own);
        let (all_to, all_cc) = reply_recipients(&src, true, &own);
        let t = ReplyTargets { one: Recipients { to: one_to, cc: one_cc }, all: Recipients { to: all_to, cc: all_cc } };
        let pick = if p.mode == Mode::ReplyAll { &t.all } else { &t.one };
        compose.to = pick.to.clone();
        compose.cc = pick.cc.clone();
        reply_targets = Some(t);
    } else {
        compose.attachments = src
            .attachments
            .iter()
            .filter(|a| !a.is_inline || a.content_id.is_none())
            .map(|a| AttachmentSpec {
                attachment_id: Some(a.id),
                name: Some(a.filename.clone()),
                size: Some(a.size),
                mime_type: Some(a.mime_type.clone()),
                ..Default::default()
            })
            .collect();
    }
    Ok(Prepared {
        compose,
        body_html: body_with_signature(sig.as_deref()),
        quoted_html: quoted,
        from_choices: choices,
        draft_message_id: None,
        reply_targets,
    })
}

async fn default_account(ctx: &Arc<AppCtx>, choices: &[FromChoice]) -> anyhow::Result<i64> {
    let configured: Option<i64> = ctx.db.read(|c| crate::db::util::get_setting::<i64>(c, "mail.defaultAccountId")).await.ok().flatten();
    if let Some(id) = configured.filter(|id| choices.iter().any(|c| c.account_id == *id)) {
        return Ok(id);
    }
    choices
        .first()
        .map(|c| c.account_id)
        .ok_or_else(|| crate::user_err!("no_account", "Add an e-mail account first (Tools › Account Settings)."))
}

/// Replace `cid:` references by data: URLs (so the quote shows and re-sends its pictures).
async fn inline_cid_images(ctx: &Arc<AppCtx>, src: &SourceMsg, html: &str) -> String {
    let mut out = html.to_string();
    let mut budget: i64 = 8 * 1024 * 1024;
    for a in src.attachments.iter().filter(|a| a.content_id.is_some()) {
        let cid = a.content_id.clone().unwrap_or_default();
        let cid = cid.trim_start_matches('<').trim_end_matches('>').to_string();
        let needle = format!("cid:{cid}");
        if !out.contains(&needle) || a.size > budget {
            continue;
        }
        if let Ok((_, mime, bytes)) = attachment_data(ctx, a.id).await {
            // the type goes into an HTML attribute unescaped: only plain image types
            let Some(mime) = safe_image_mime(&mime) else { continue };
            budget -= bytes.len() as i64;
            let url = format!("data:{};base64,{}", mime, base64::engine::general_purpose::STANDARD.encode(&bytes));
            out = out.replace(&needle, &url);
        }
    }
    out
}

/// Remember the verb on the source message ("You replied on ...").
pub fn set_last_verb(c: &rusqlite::Connection, message_id: i64, verb: &str, at: i64) -> anyhow::Result<Option<i64>> {
    let n = c.execute("UPDATE messages SET last_verb=?1, last_verb_at=?2 WHERE id=?3", params![verb, at, message_id])?;
    if n == 0 {
        return Ok(None);
    }
    Ok(c.query_row("SELECT account_id FROM messages WHERE id=?1", [message_id], |r| r.get(0)).optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_prefixes() {
        assert_eq!(prefixed_subject("Q3 results", Mode::Reply), "RE: Q3 results");
        assert_eq!(prefixed_subject("RE: Q3 results", Mode::Reply), "RE: Q3 results");
        assert_eq!(prefixed_subject("Re: Q3 results", Mode::ReplyAll), "RE: Q3 results");
        assert_eq!(prefixed_subject("FW: Q3 results", Mode::Reply), "RE: FW: Q3 results");
        assert_eq!(prefixed_subject("Fwd: Q3", Mode::Forward), "FW: Q3");
        assert_eq!(prefixed_subject("RE: Q3", Mode::Forward), "FW: RE: Q3");
        assert_eq!(prefixed_subject("דוח רבעוני", Mode::Forward), "FW: דוח רבעוני");
        assert_eq!(prefixed_subject("Time: 10:00", Mode::Reply), "RE: Time: 10:00");
    }

    fn src() -> SourceMsg {
        SourceMsg {
            from: Addr::new("Sarah Cohen", "sarah@harborcap.example"),
            to: vec![Addr::new("Kenny Green", "kenny@duvz.demo"), Addr::new("David Levi", "david@meridian.example")],
            cc: vec![Addr::new("", "info@duvz.demo"), Addr::new("Noa", "noa@oriontech.example")],
            subject: "Q3".into(),
            date: 1_760_000_000_000,
            ..Default::default()
        }
    }

    #[test]
    fn reply_all_excludes_me() {
        let own: HashSet<String> = ["kenny@duvz.demo".to_string(), "info@duvz.demo".to_string()].into();
        let (to, cc) = reply_recipients(&src(), false, &own);
        assert_eq!(to, vec![Addr::new("Sarah Cohen", "sarah@harborcap.example")]);
        assert!(cc.is_empty());
        let (to, cc) = reply_recipients(&src(), true, &own);
        assert_eq!(to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["sarah@harborcap.example", "david@meridian.example"]);
        assert_eq!(cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["noa@oriontech.example"]);
        // reply-to wins
        let mut s = src();
        s.reply_to = vec![Addr::new("IR desk", "ir@harborcap.example")];
        let (to, _) = reply_recipients(&s, false, &own);
        assert_eq!(to[0].email, "ir@harborcap.example");
        // replying to my own sent message goes to its recipients
        let mut s = src();
        s.from = Addr::new("Kenny", "kenny@duvz.demo");
        s.to = vec![Addr::new("Sarah", "sarah@harborcap.example")];
        let (to, _) = reply_recipients(&s, false, &own);
        assert_eq!(to[0].email, "sarah@harborcap.example");
    }

    #[test]
    fn quote_block_has_outlook_header() {
        let q = quoted_block(&src(), "<p>hello</p>");
        assert!(q.starts_with(ORIGINAL_MARKER));
        assert!(q.contains("-----Original Message-----"));
        assert!(q.contains("<b>From:</b> Sarah Cohen [mailto:sarah@harborcap.example]"));
        assert!(q.contains("<b>To:</b> Kenny Green; David Levi"));
        assert!(q.contains("<b>Cc:</b> info@duvz.demo; Noa"));
        assert!(q.contains("<b>Subject:</b> Q3"));
    }

    #[test]
    fn inline_image_types_are_validated() {
        assert_eq!(safe_image_mime("image/png"), Some("image/png"));
        assert_eq!(safe_image_mime("image/svg+xml"), Some("image/svg+xml"));
        assert_eq!(safe_image_mime("image/png\" onerror=\"alert(1)"), None);
        assert_eq!(safe_image_mime("text/html"), None);
        assert_eq!(safe_image_mime("image/"), None);
    }

    #[test]
    fn sanitizer_keeps_only_the_quote_marker_id() {
        let s = sanitize("<div id=\"cm-original\"><p id=\"x\">a</p></div>");
        assert!(s.contains("id=\"cm-original\""), "{s}");
        assert!(!s.contains("id=\"x\""), "{s}");
        let deep = format!("{}deep text{}", "<span>".repeat(5000), "</span>".repeat(5000));
        let t = std::time::Instant::now();
        let s = sanitize(&deep);
        assert!(s.contains("deep text") && !s.contains("<span>"));
        assert!(t.elapsed().as_secs() < 2);
    }

    #[test]
    fn sanitizer_removes_script() {
        let s = sanitize("<div style=\"color:red\" onclick=\"x()\">hi<script>alert(1)</script><img src=\"cid:a@b\" onerror=\"y()\"></div>");
        assert!(s.contains("style=\"color:red\""));
        assert!(!s.contains("onclick") && !s.contains("script") && !s.contains("onerror"));
        assert!(s.contains("cid:a@b"));
    }
}

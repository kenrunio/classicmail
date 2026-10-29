//! Message bodies: fetch `format=full`, walk Gmail's part tree, decode text with its charset, collect
//! attachments / inline images / calendar invitations, store them and index the text.

use std::{path::PathBuf, sync::Arc};

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use super::{
    attach_policy::safe_file_name,
    mime::{self, decode_charset, header_params},
};
use crate::{
    ctx::AppCtx,
    google::{
        Api,
        client::api_status,
        gmail::{MessagePart, b64url_decode},
    },
};

#[derive(Debug, Clone, Default)]
pub struct AttPart {
    pub part_id: String,
    pub attachment_id: Option<String>,
    pub filename: String,
    pub mime: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub disposition_inline: bool,
    pub in_related: bool,
    /// small parts Gmail returned inline
    pub data: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct TextPart {
    pub mime: String,
    pub charset: Option<String>,
    pub data: Option<Vec<u8>>,
    pub attachment_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Walked {
    pub html: Vec<TextPart>,
    pub text: Vec<TextPart>,
    pub calendar: Option<TextPart>,
    pub attachments: Vec<AttPart>,
}

/// Walk Gmail's payload tree: which parts are the body (html / text), which are attachments.
pub fn walk(p: &MessagePart) -> Walked {
    let mut w = Walked::default();
    walk_into(p, &mut w, false, false);
    w
}

fn walk_into(p: &MessagePart, w: &mut Walked, in_related: bool, in_alternative: bool) {
    let mime = p.mime_type.to_ascii_lowercase();
    if mime.starts_with("multipart/") {
        let related = mime == "multipart/related";
        let alt = mime == "multipart/alternative";
        for c in &p.parts {
            walk_into(c, w, in_related || related, in_alternative || alt);
        }
        return;
    }
    let ct = p.header("Content-Type").map(header_params);
    let charset = ct.as_ref().and_then(|(_, params)| params.get("charset").cloned());
    let cd = p.header("Content-Disposition").map(header_params);
    let is_attachment_disp = cd.as_ref().map(|(d, _)| d == "attachment").unwrap_or(false);
    let mut filename = p.filename.clone();
    if filename.is_empty() {
        filename = cd
            .as_ref()
            .and_then(|(_, pr)| pr.get("filename").cloned())
            .or_else(|| ct.as_ref().and_then(|(_, pr)| pr.get("name").cloned()))
            .unwrap_or_default();
    }
    let data = p.body.data.as_deref().and_then(|d| b64url_decode(d).ok());
    let text_part =
        || TextPart { mime: mime.clone(), charset: charset.clone(), data: data.clone(), attachment_id: p.body.attachment_id.clone() };
    if (mime == "text/plain" || mime == "text/html") && filename.is_empty() && !is_attachment_disp {
        if mime == "text/html" {
            w.html.push(text_part());
        } else {
            w.text.push(text_part());
        }
        return;
    }
    if mime == "text/calendar" && w.calendar.is_none() && (filename.is_empty() || in_alternative) {
        w.calendar = Some(text_part());
        return;
    }
    // no bidi controls in names we show ("invoice<RLO>fdp.exe" would read "invoiceexe.pdf")
    let filename = super::attach_policy::display_name(&filename);
    let content_id = p
        .header("Content-ID")
        .or_else(|| p.header("X-Attachment-Id"))
        .map(|s| s.trim().trim_start_matches('<').trim_end_matches('>').to_string())
        .filter(|s| !s.is_empty());
    w.attachments.push(AttPart {
        part_id: p.part_id.clone(),
        attachment_id: p.body.attachment_id.clone(),
        filename,
        mime,
        size: p.body.size,
        content_id,
        disposition_inline: cd.as_ref().map(|(d, _)| d == "inline").unwrap_or(false),
        in_related,
        data,
    });
}

fn ext_for(mime: &str) -> String {
    mime_guess::get_mime_extensions_str(mime).and_then(|e| e.first().map(|s| s.to_string())).unwrap_or_else(|| "dat".into())
}

/// Minimal iCalendar invitation details for the meeting-request info bar.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub method: String,
    pub uid: String,
    pub summary: String,
    pub location: String,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub all_day: bool,
    pub organizer: String,
    pub organizer_email: String,
    /// METHOD:REPLY — the attendee answering and how (ACCEPTED / TENTATIVE / DECLINED)
    pub reply_from: String,
    pub reply_status: String,
}

fn ics_time(prop_params: &str, value: &str) -> (Option<i64>, bool) {
    let v = value.trim();
    if prop_params.to_ascii_uppercase().contains("VALUE=DATE") || (v.len() == 8 && !v.contains('T')) {
        let d = chrono::NaiveDate::parse_from_str(v, "%Y%m%d").ok();
        return (d.and_then(|d| d.and_hms_opt(0, 0, 0)).map(|d| d.and_utc().timestamp_millis()), true);
    }
    let utc = v.ends_with('Z');
    let naive = chrono::NaiveDateTime::parse_from_str(v.trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok();
    let Some(naive) = naive else { return (None, false) };
    if utc {
        return (Some(naive.and_utc().timestamp_millis()), false);
    }
    let tzid = prop_params.split(';').find_map(|p| p.strip_prefix("TZID=")).map(|t| t.trim_matches('"').to_string());
    if let Some(tz) = tzid.and_then(|t| t.parse::<chrono_tz::Tz>().ok()) {
        use chrono::TimeZone;
        if let Some(dt) = tz.from_local_datetime(&naive).earliest() {
            return (Some(dt.timestamp_millis()), false);
        }
    }
    use chrono::TimeZone;
    (chrono::Local.from_local_datetime(&naive).earliest().map(|d| d.timestamp_millis()), false)
}

/// Undo iCalendar text escapes (`\n`, `\,`, `\;`, `\\`) in one pass, so an escaped backslash
/// before an `n` stays a backslash.
fn ics_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(e @ (',' | ';' | '\\')) => out.push(e),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

pub fn parse_invite(ics: &str) -> Option<Invite> {
    // unfold
    let unfolded = ics.replace("\r\n ", "").replace("\r\n\t", "").replace("\n ", "").replace("\n\t", "");
    let mut inv = Invite::default();
    let mut in_event = false;
    let mut seen_event = false;
    for line in unfolded.lines() {
        let line = line.trim_end_matches('\r');
        if line.eq_ignore_ascii_case("BEGIN:VEVENT") {
            if seen_event {
                break; // first event only
            }
            in_event = true;
            seen_event = true;
            continue;
        }
        if line.eq_ignore_ascii_case("END:VEVENT") {
            in_event = false;
            continue;
        }
        let Some((left, value)) = line.split_once(':') else { continue };
        let (name, prop_params) = left.split_once(';').unwrap_or((left, ""));
        let name = name.to_ascii_uppercase();
        if name == "METHOD" && !in_event {
            inv.method = value.trim().to_ascii_uppercase();
        }
        if !in_event {
            continue;
        }
        match name.as_str() {
            "UID" => inv.uid = value.trim().to_string(),
            "SUMMARY" => inv.summary = ics_unescape(value),
            "LOCATION" => inv.location = ics_unescape(value),
            "DTSTART" => {
                let (t, all_day) = ics_time(prop_params, value);
                inv.start = t;
                inv.all_day = all_day;
            }
            "DTEND" => inv.end = ics_time(prop_params, value).0,
            // a reply carries just the attendee who answered
            "ATTENDEE" if inv.method == "REPLY" && inv.reply_status.is_empty() => {
                let param = |k: &str| prop_params.split(';').find_map(|p| p.strip_prefix(k)).map(|s| s.trim_matches('"').to_string());
                inv.reply_status = param("PARTSTAT=").unwrap_or_default().to_ascii_uppercase();
                let email = value.trim().trim_start_matches("mailto:").trim_start_matches("MAILTO:").to_string();
                inv.reply_from = param("CN=").filter(|n| !n.is_empty()).unwrap_or(email);
            }
            "ORGANIZER" => {
                inv.organizer_email = value.trim().trim_start_matches("mailto:").trim_start_matches("MAILTO:").to_string();
                inv.organizer = prop_params
                    .split(';')
                    .find_map(|p| p.strip_prefix("CN="))
                    .map(|s| s.trim_matches('"').to_string())
                    .unwrap_or_else(|| inv.organizer_email.clone());
            }
            _ => {}
        }
    }
    (seen_event && !inv.uid.is_empty()).then_some(inv)
}

async fn part_bytes(api: &Api, gmail_id: &str, p: &TextPart) -> anyhow::Result<Vec<u8>> {
    if let Some(d) = &p.data {
        return Ok(d.clone());
    }
    if let Some(aid) = &p.attachment_id {
        return api.gmail_attachment(gmail_id, aid).await;
    }
    Ok(Vec::new())
}

async fn join_parts(api: &Api, gmail_id: &str, parts: &[TextPart]) -> anyhow::Result<Option<String>> {
    if parts.is_empty() {
        return Ok(None);
    }
    let mut out = Vec::new();
    for p in parts {
        let b = part_bytes(api, gmail_id, p).await?;
        out.push(decode_charset(&b, p.charset.as_deref()));
    }
    Ok(Some(out.join(if parts[0].mime == "text/html" { "<hr>" } else { "\n\n" })))
}

/// Does the HTML reference `cid:<id>`? `lower_html` is the body already lower-cased.
fn html_uses_cid(lower_html: &str, cid: &str) -> bool {
    lower_html.contains(&format!("cid:{}", cid.to_ascii_lowercase()))
}

/// A body fetched from Gmail, ready to store (`store_body`).
pub(crate) struct FetchedBody {
    message_id: i64,
    html: Option<String>,
    plain: String,
    headers_json: String,
    ics: Option<String>,
    invite_uid: Option<String>,
    is_invite: bool,
    size_estimate: i64,
    /// (attachment, is inline, file already written for a part Gmail sent inline)
    atts: Vec<(AttPart, bool, Option<PathBuf>)>,
    real_attachments: usize,
    /// subject, from, recipients (full-text index)
    index: (String, String, String),
}

/// Fetch and store the body of local message `message_id` unless already present.
pub async fn ensure_body(ctx: &Arc<AppCtx>, message_id: i64) -> anyhow::Result<()> {
    let Some(b) = fetch_body(ctx, message_id).await? else { return Ok(()) };
    ctx.db.write(move |c| store_body(c, &b)).await
}

/// Download the body of `message_id` (None when it is already here).
pub(crate) async fn fetch_body(ctx: &Arc<AppCtx>, message_id: i64) -> anyhow::Result<Option<FetchedBody>> {
    let row: Option<(i64, String, i64)> = ctx
        .db
        .read(move |c| {
            Ok(c.query_row("SELECT account_id, gmail_id, body_state FROM messages WHERE id=?1", [message_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?)
        })
        .await?;
    let Some((account_id, gmail_id, state)) = row else {
        return Err(crate::user_err!("not_found", "The message no longer exists."));
    };
    if state == 1 {
        return Ok(None);
    }
    let api = ctx.api(account_id).await?;
    let msg = match api.gmail_get(&gmail_id, "full", &[]).await {
        Ok(m) => m,
        Err(e) if api_status(&e) == Some(404) => {
            return Err(crate::user_err!("gone", "This message was deleted on the server."));
        }
        Err(e) => return Err(e),
    };
    let Some(payload) = msg.payload.clone() else { anyhow::bail!("message has no payload") };
    let w = walk(&payload);
    let html = join_parts(&api, &gmail_id, &w.html).await?;
    let text = join_parts(&api, &gmail_id, &w.text).await?;
    let ics = match &w.calendar {
        Some(p) => Some(decode_charset(&part_bytes(&api, &gmail_id, p).await?, p.charset.as_deref())),
        None => None,
    };
    let invite = ics.as_deref().and_then(parse_invite);
    let is_invite = invite.as_ref().map(|i| i.method == "REQUEST" || i.method.is_empty()).unwrap_or(false);
    let plain = match (&text, &html) {
        (Some(t), _) if !t.trim().is_empty() => t.clone(),
        (_, Some(h)) => mime::html_to_text(h),
        _ => String::new(),
    };
    let headers: Vec<serde_json::Value> = payload.headers.iter().map(|h| serde_json::json!({ "name": h.name, "value": h.value })).collect();
    let mut atts = w.attachments;
    // nameless parts get Outlook-style names; decide which are inline images
    let mut n = 0;
    for a in atts.iter_mut() {
        if a.filename.is_empty() {
            n += 1;
            a.filename = if a.mime.starts_with("image/") {
                format!("image{n:03}.{}", ext_for(&a.mime))
            } else if a.mime == "message/rfc822" {
                "Attached Message.eml".into()
            } else {
                format!("ATT{n:05}.{}", ext_for(&a.mime))
            };
        }
    }
    // invitation attachments (invite.ics) are shown as the meeting bar instead
    atts.retain(|a| !(is_invite && (a.mime == "application/ics" || a.mime == "text/calendar")));
    let html_lower = html.as_deref().unwrap_or_default().to_ascii_lowercase();
    let flagged: Vec<(AttPart, bool)> = atts
        .into_iter()
        .map(|a| {
            let inline = match &a.content_id {
                Some(cid) => {
                    html_uses_cid(&html_lower, cid)
                        || (a.in_related && a.mime.starts_with("image/") && !html_lower.is_empty() && a.disposition_inline)
                }
                None => false,
            };
            (a, inline)
        })
        .collect();
    let real_attachments = flagged.iter().filter(|(_, inline)| !inline).count();
    // parts Gmail sent inline go to disk now, outside the database transaction
    let dir = ctx.data_dir.join("attachments").join(message_id.to_string());
    let atts = tokio::task::spawn_blocking(move || {
        flagged
            .into_iter()
            .enumerate()
            .map(|(i, (a, inline))| {
                let path = a.data.as_ref().and_then(|data| {
                    let path = dir.join(format!("part{i}_{}", safe_file_name(&a.filename)));
                    let _ = std::fs::create_dir_all(&dir);
                    std::fs::write(&path, data).is_ok().then_some(path)
                });
                (a, inline, path)
            })
            .collect::<Vec<_>>()
    })
    .await?;
    let index: (String, String, String) = ctx
        .db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT subject, from_name || ' ' || from_email, to_json || cc_json FROM messages WHERE id=?1",
                [message_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await?;
    Ok(Some(FetchedBody {
        message_id,
        html,
        plain,
        headers_json: serde_json::to_string(&headers)?,
        ics,
        invite_uid: invite.map(|i| i.uid),
        is_invite,
        size_estimate: msg.size_estimate,
        atts,
        real_attachments,
        index,
    }))
}

/// Store a fetched body, its attachments and its index row. Does nothing when another fetch of the
/// same message got there first, so attachment ids already handed out stay valid.
pub(crate) fn store_body(c: &mut rusqlite::Connection, b: &FetchedBody) -> anyhow::Result<()> {
    let message_id = b.message_id;
    let tx = c.transaction()?;
    let state: Option<i64> = tx.query_row("SELECT body_state FROM messages WHERE id=?1", [message_id], |r| r.get(0)).optional()?;
    if state != Some(0) {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO message_bodies(message_id, html, text, headers_json, calendar_ics, fetched_at) VALUES (?1,?2,?3,?4,?5,?6) \
         ON CONFLICT(message_id) DO UPDATE SET html=excluded.html, text=excluded.text, headers_json=excluded.headers_json, \
         calendar_ics=excluded.calendar_ics, fetched_at=excluded.fetched_at",
        params![message_id, b.html, b.plain, b.headers_json, b.ics, crate::db::util::now_ms()],
    )?;
    // keep cached files of attachments that still exist (same part id)
    let old: Vec<(String, Option<String>)> = {
        let mut st = tx.prepare("SELECT part_id, cached_path FROM attachments WHERE message_id=?1")?;
        st.query_map([message_id], |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get(1)?)))?.collect::<Result<_, _>>()?
    };
    tx.execute("DELETE FROM attachments WHERE message_id=?1", [message_id])?;
    for (a, inline, written) in &b.atts {
        let cached = old
            .iter()
            .find(|(p, _)| *p == a.part_id)
            .and_then(|(_, c)| c.clone())
            .or_else(|| written.as_ref().map(|p| p.to_string_lossy().into_owned()));
        tx.execute(
            "INSERT INTO attachments(message_id, part_id, gmail_attachment_id, filename, mime_type, size, content_id, is_inline, cached_path) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![message_id, a.part_id, a.attachment_id, a.filename, a.mime, a.size, a.content_id, inline, cached],
        )?;
    }
    tx.execute(
        // headers from the fast IMAP download come without a snippet: fill it in now
        "UPDATE messages SET body_state=1, has_attachments=?1, is_invite=?2, ical_uid=?3, size=MAX(size, ?4), \
         snippet = CASE WHEN snippet='' THEN ?6 ELSE snippet END WHERE id=?5",
        params![b.real_attachments > 0, b.is_invite, b.invite_uid, b.size_estimate, message_id, mime::make_snippet(&b.plain, 200)],
    )?;
    super::store::fts_put(&tx, message_id, &b.index.0, &b.index.1, &b.index.2, &b.plain)?;
    tx.commit()?;
    Ok(())
}

pub fn cache_path(data_dir: &std::path::Path, message_id: i64, att_id: i64, filename: &str) -> PathBuf {
    data_dir.join("attachments").join(message_id.to_string()).join(format!("{att_id}_{}", safe_file_name(filename)))
}

/// Attachment content by local `attachments.id` → (filename, mime type, bytes). Cached on disk.
pub async fn attachment_bytes(ctx: &Arc<AppCtx>, attachment_id: i64) -> anyhow::Result<(String, String, Vec<u8>)> {
    type Row = (i64, Option<String>, Option<String>, String, String, Option<String>, String, i64);
    let row: Option<Row> = ctx
        .db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT a.message_id, a.part_id, a.gmail_attachment_id, a.filename, a.mime_type, a.cached_path, m.gmail_id, m.account_id \
                 FROM attachments a JOIN messages m ON m.id=a.message_id WHERE a.id=?1",
                [attachment_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
            )
            .optional()?)
        })
        .await?;
    let Some((message_id, part_id, gmail_att, filename, mime_type, cached, gmail_id, account_id)) = row else {
        return Err(crate::user_err!("not_found", "The attachment no longer exists."));
    };
    if let Some(p) = &cached
        && let Ok(b) = tokio::fs::read(p).await
    {
        return Ok((filename, mime_type, b));
    }
    let api = ctx.api(account_id).await?;
    let bytes = match &gmail_att {
        Some(aid) => match api.gmail_attachment(&gmail_id, aid).await {
            Ok(b) => b,
            Err(e) if matches!(api_status(&e), Some(400 | 404)) => refetch_part(&api, &gmail_id, part_id.as_deref().unwrap_or("")).await?,
            Err(e) => return Err(e),
        },
        None => refetch_part(&api, &gmail_id, part_id.as_deref().unwrap_or("")).await?,
    };
    let path = cache_path(&ctx.data_dir, message_id, attachment_id, &filename);
    // file I/O off the async workers (attachments run up to 25 MB)
    let (bytes, written) = tokio::task::spawn_blocking(move || {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let ok = std::fs::write(&path, &bytes).is_ok();
        (bytes, ok.then_some(path))
    })
    .await?;
    if let Some(path) = written {
        let p = path.to_string_lossy().into_owned();
        ctx.db
            .write(move |c| {
                c.execute("UPDATE attachments SET cached_path=?1 WHERE id=?2", params![p, attachment_id])?;
                Ok(())
            })
            .await?;
    }
    Ok((filename, mime_type, bytes))
}

/// Attachment ids can go stale: look the part up again by part id.
async fn refetch_part(api: &Api, gmail_id: &str, part_id: &str) -> anyhow::Result<Vec<u8>> {
    let msg = api.gmail_get(gmail_id, "full", &[]).await?;
    fn find<'a>(p: &'a MessagePart, id: &str) -> Option<&'a MessagePart> {
        if p.part_id == id {
            return Some(p);
        }
        p.parts.iter().find_map(|c| find(c, id))
    }
    let p = msg
        .payload
        .as_ref()
        .and_then(|p| find(p, part_id))
        .ok_or_else(|| crate::user_err!("not_found", "The attachment is no longer on the server."))?;
    if let Some(d) = &p.body.data {
        return b64url_decode(d);
    }
    match &p.body.attachment_id {
        Some(aid) => api.gmail_attachment(gmail_id, aid).await,
        None => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::gmail::{Header, PartBody};

    fn part(
        id: &str,
        mime: &str,
        filename: &str,
        headers: &[(&str, &str)],
        data: Option<&[u8]>,
        aid: Option<&str>,
        parts: Vec<MessagePart>,
    ) -> MessagePart {
        MessagePart {
            part_id: id.into(),
            mime_type: mime.into(),
            filename: filename.into(),
            headers: headers.iter().map(|(n, v)| Header { name: n.to_string(), value: v.to_string() }).collect(),
            body: PartBody {
                attachment_id: aid.map(str::to_string),
                size: data.map(|d| d.len() as i64).unwrap_or(10),
                data: data.map(crate::google::gmail::b64url_encode),
            },
            parts,
        }
    }

    #[test]
    fn walks_related_alternative_mixed() {
        let heb = encoding_rs::WINDOWS_1255.encode("שלום").0.into_owned();
        let tree = part(
            "",
            "multipart/mixed",
            "",
            &[],
            None,
            None,
            vec![
                part(
                    "0",
                    "multipart/related",
                    "",
                    &[],
                    None,
                    None,
                    vec![
                        part(
                            "0.0",
                            "multipart/alternative",
                            "",
                            &[],
                            None,
                            None,
                            vec![
                                part(
                                    "0.0.0",
                                    "text/plain",
                                    "",
                                    &[("Content-Type", "text/plain; charset=windows-1255")],
                                    Some(&heb),
                                    None,
                                    vec![],
                                ),
                                part("0.0.1", "text/html", "", &[("Content-Type", "text/html; charset=utf-8")], None, Some("BIG"), vec![]),
                                part(
                                    "0.0.2",
                                    "text/calendar",
                                    "",
                                    &[("Content-Type", "text/calendar; method=REQUEST")],
                                    Some(b"BEGIN:VCALENDAR"),
                                    None,
                                    vec![],
                                ),
                            ],
                        ),
                        part(
                            "0.1",
                            "image/png",
                            "",
                            &[("Content-ID", "<logo1>"), ("Content-Disposition", "inline")],
                            None,
                            Some("IMG"),
                            vec![],
                        ),
                    ],
                ),
                part(
                    "1",
                    "application/pdf",
                    "r.pdf",
                    &[("Content-Disposition", "attachment; filename=\"r.pdf\"")],
                    None,
                    Some("PDF"),
                    vec![],
                ),
            ],
        );
        let w = walk(&tree);
        assert_eq!(w.text.len(), 1);
        assert_eq!(decode_charset(w.text[0].data.as_ref().unwrap(), w.text[0].charset.as_deref()), "שלום");
        assert_eq!(w.html.len(), 1);
        assert_eq!(w.html[0].attachment_id.as_deref(), Some("BIG"));
        assert!(w.calendar.is_some());
        assert_eq!(w.attachments.len(), 2);
        assert_eq!(w.attachments[0].content_id.as_deref(), Some("logo1"));
        assert!(w.attachments[0].in_related);
        assert_eq!(w.attachments[1].filename, "r.pdf");
        assert!(html_uses_cid(&"<img src=\"CID:logo1\">".to_ascii_lowercase(), "LOGO1"));
    }

    #[test]
    fn ics_text_escapes() {
        assert_eq!(ics_unescape(r"C:\\new folder"), r"C:\new folder");
        assert_eq!(ics_unescape(r"a\nb\Nc\, d\; e"), "a\nb\nc, d; e");
        assert_eq!(ics_unescape(r"odd \x and trailing \"), r"odd \x and trailing \");
    }

    #[test]
    fn parses_invites() {
        let ics = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nDTSTART;TZID=Asia/Jerusalem:20261014T100000\r\nDTEND:20261014T080000Z\r\nUID:abc@x\r\nSUMMARY:Q3 call\\, prep\r\nLOCATION:Room\r\n 12\r\nORGANIZER;CN=\"Sarah Cohen\":mailto:sarah@x.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let i = parse_invite(ics).unwrap();
        assert_eq!(i.method, "REQUEST");
        assert_eq!(i.uid, "abc@x");
        assert_eq!(i.summary, "Q3 call, prep");
        assert_eq!(i.location, "Room12");
        assert_eq!(i.organizer, "Sarah Cohen");
        assert_eq!(i.start, Some(1_791_961_200_000));
        assert_eq!(i.end, Some(1_791_964_800_000));
        assert_eq!(i.reply_status, "", "a request isn't a reply");

        // Noa's "Accepted: …" (Outlook)
        let reply = "BEGIN:VCALENDAR
METHOD:REPLY
BEGIN:VEVENT
ATTENDEE;PARTSTAT=ACCEPTED;CN=Noa Shapiro:mailto:Noa.S@oriontech.example
UID:abc@x
SUMMARY;LANGUAGE=en-US:Accepted: Orion Weekly IR Call Update
DTSTART:20261020T140000Z
END:VEVENT
END:VCALENDAR
";
        let r = parse_invite(reply).unwrap();
        assert_eq!((r.method.as_str(), r.reply_status.as_str(), r.reply_from.as_str()), ("REPLY", "ACCEPTED", "Noa Shapiro"));
    }
}

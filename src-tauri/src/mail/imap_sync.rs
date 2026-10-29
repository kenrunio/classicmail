//! Fast initial download: message headers of the whole mailbox over Gmail IMAP.
//!
//! The Gmail API meters per user (6,000 units/min, 20 per `messages.get`), so headers for a big
//! mailbox take hours. IMAP isn't metered that way (only ~2.5 GB/day of download), and Gmail's
//! IMAP extensions give the API's own ids (X-GM-MSGID / X-GM-THRID are the decimal form of the
//! REST hex ids) and labels (X-GM-LABELS), so the same rows can be written in minutes.
//!
//! It runs as the first phase of the API full sync (`sync::full_sync_sliced`), inside the same
//! bookkeeping: the profile historyId recorded when that full sync started stays the point the
//! incremental history sync continues from (it covers what changed during the download), rows
//! get the full sync's generation, and the API listing that follows only *checks* every id
//! (5 units per 500) and fetches just what IMAP didn't cover (chats, folders hidden from IMAP,
//! mail that arrived meanwhile); a cheap per-label resync (`resync_labels`) then makes labels and
//! categories exactly what the API reports.
//!
//! Read-only: EXAMINE, UID SEARCH, UID FETCH with BODY.PEEK — flags never change. Resumable per
//! mailbox (UIDVALIDITY + lowest UID done, in sync_state). Any failure falls back to the API path
//! with no data loss (rows are upserted by gmail_id); permanent failures (IMAP disabled by the
//! Workspace admin, sign-in refused, no All Mail folder) switch it off for this full sync.
//! Setting `mail.fastInitialSync` (default true) turns it off.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{
    mime,
    store::{self, LabelMap},
    sync::Throttle,
};
use crate::{
    archive::index::fmt_n,
    ctx::AppCtx,
    db::util::{get_setting, get_sync_state, set_sync_state},
    google::{
        Api,
        gmail::{Header, Label, Message, MessagePart},
        imap::{GmLabel, HeaderRecord, ImapError, Mailbox, Session, quote},
    },
};

pub(crate) const K_IMAP: &str = "mail.imap.state";
const CHUNK: usize = 2000;
const TIMEOUT: Duration = Duration::from_secs(180);
/// transient failures (network, timeouts) before giving up on IMAP for this full sync
const MAX_ATTEMPTS: u32 = 3;
pub const UNAVAILABLE: &str = "Fast download unavailable \u{2014} using the standard method";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ImapState {
    /// "" = to do / running, "done", "failed"
    pub phase: String,
    pub attempts: u32,
    pub error: Option<String>,
    /// per mailbox role ("all", "trash", "spam")
    pub boxes: BTreeMap<String, BoxState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct BoxState {
    pub uid_validity: u32,
    /// every UID >= this one is stored
    pub below: Option<u32>,
    pub complete: bool,
    pub stored: i64,
}

/// What the full sync should do next.
pub(crate) enum Fast {
    /// not used (switched off, no IMAP address, failed now or earlier): API listing as usual
    NotUsed,
    /// time slice used up; continue next cycle
    Paused,
    /// all headers are here; `now` = finished in this call (the API listing must start over)
    Done { now: bool },
}

pub(crate) async fn load_state(ctx: &Arc<AppCtx>, account_id: i64) -> anyhow::Result<ImapState> {
    let raw = ctx.db.read(move |c| get_sync_state(c, account_id, K_IMAP)).await?;
    Ok(raw.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default())
}

async fn save_state(ctx: &Arc<AppCtx>, account_id: i64, st: &ImapState) -> anyhow::Result<()> {
    let s = serde_json::to_string(st)?;
    ctx.db.write(move |c| set_sync_state(c, account_id, K_IMAP, Some(&s))).await
}

/// Run (or continue) the fast download if it applies. Never fails because of IMAP: problems are
/// logged, shown on the status bar and answered with `NotUsed` (the API path takes over).
pub(crate) async fn run(ctx: &Arc<AppCtx>, api: &Api, map: &LabelMap, generation: i64, slice: Duration) -> anyhow::Result<Fast> {
    let account_id = api.account_id;
    let mut st = load_state(ctx, account_id).await?;
    match st.phase.as_str() {
        "done" => return Ok(Fast::Done { now: false }),
        "failed" => return Ok(Fast::NotUsed),
        _ => {}
    }
    let enabled: bool = ctx.db.read(|c| get_setting(c, "mail.fastInitialSync")).await?.unwrap_or(true);
    if !enabled || api.urls.imap.is_empty() {
        return Ok(Fast::NotUsed);
    }
    match bootstrap(ctx, api, map, generation, slice, &mut st).await {
        Ok(true) => {
            st.phase = "done".into();
            st.error = None;
            save_state(ctx, account_id, &st).await?;
            log::info!("mail: fast IMAP download complete for account {account_id}");
            Ok(Fast::Done { now: true })
        }
        Ok(false) => Ok(Fast::Paused),
        Err(e) => {
            let permanent = e.downcast_ref::<ImapError>().is_some_and(ImapError::is_permanent);
            // progress saved by finished chunks stays; reload so it isn't overwritten
            let mut st = load_state(ctx, account_id).await?;
            st.attempts += 1;
            st.error = Some(format!("{e:#}").chars().take(300).collect());
            let give_up = permanent || st.attempts >= MAX_ATTEMPTS;
            if give_up {
                st.phase = "failed".into();
            }
            save_state(ctx, account_id, &st).await?;
            if give_up {
                log::warn!(
                    "mail: fast IMAP download unavailable for account {account_id} ({}); using the Gmail API",
                    crate::logging::err(&e)
                );
                ctx.events.status(account_id, "sync", UNAVAILABLE, None);
            } else {
                log::warn!(
                    "mail: fast IMAP download for account {account_id} interrupted ({}); using the Gmail API this cycle, retrying IMAP next cycle",
                    crate::logging::err(&e)
                );
            }
            Ok(Fast::NotUsed)
        }
    }
}

/// Connect + XOAUTH2; a refused token is refreshed once (it may just have expired).
async fn open_session(api: &Api) -> anyhow::Result<Session> {
    let mut last: Option<ImapError> = None;
    for force in [false, true] {
        let token = api.access_token(force).await?;
        let mut s = Session::connect(&api.urls.imap, TIMEOUT).await?;
        match s.authenticate_xoauth2(&api.email, &token).await {
            Ok(()) => return Ok(s),
            Err(e @ ImapError::Auth(_)) => {
                log::info!(
                    "mail: IMAP sign-in for account {} refused ({}){}",
                    api.account_id,
                    crate::logging::redact(&e.to_string()),
                    if force { "" } else { "; retrying with a fresh token" }
                );
                last = Some(e);
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(last.unwrap_or_else(|| ImapError::Auth("refused".into())).into())
}

/// Download headers until done (true) or the slice is used up (false).
async fn bootstrap(
    ctx: &Arc<AppCtx>,
    api: &Api,
    map: &LabelMap,
    generation: i64,
    slice: Duration,
    st: &mut ImapState,
) -> anyhow::Result<bool> {
    let started = Instant::now();
    let account_id = api.account_id;
    let names = LabelNames::new(&api.gmail_labels().await?);
    let mut sess = open_session(api).await?;
    let boxes = sess.list().await?;
    let find = |attrs: &[&str]| boxes.iter().find(|b| attrs.iter().any(|a| b.has(a)) && !b.has("\\Noselect")).cloned();
    let all =
        find(&["\\All", "\\AllMail"]).ok_or_else(|| ImapError::Unsupported("Gmail's All Mail folder is not visible over IMAP".into()))?;
    let plan: Vec<(&str, Option<Mailbox>)> = vec![("all", Some(all)), ("trash", find(&["\\Trash"])), ("spam", find(&["\\Junk", "\\Spam"]))];
    // progress: messages in all three vs. what earlier slices stored
    let mut total = 0i64;
    for (_, mb) in &plan {
        if let Some(mb) = mb {
            total += sess.status_messages(&mb.raw).await.unwrap_or(0) as i64;
        }
    }
    let mut done: i64 = st.boxes.values().map(|b| b.stored).sum();
    let mut status = Throttle::new(300);
    let mut ui = Throttle::new(2500);
    let mut pending: Option<tokio::task::JoinHandle<anyhow::Result<()>>> = None;
    for (role, mb) in plan {
        let Some(mb) = mb else {
            log::info!("mail: the {role} folder of account {account_id} is not visible over IMAP; the API check fetches its messages");
            continue;
        };
        if st.boxes.get(role).is_some_and(|b| b.complete) {
            continue;
        }
        let ex = sess.examine(&mb.raw).await?;
        if !ex.read_only {
            log::warn!("mail: IMAP EXAMINE of the {role} folder did not report READ-ONLY");
        }
        let bs = st.boxes.entry(role.to_string()).or_default();
        if bs.uid_validity != ex.uid_validity {
            if bs.uid_validity != 0 {
                log::info!("mail: UIDVALIDITY of the {role} folder changed; downloading that folder again");
            }
            done -= bs.stored;
            *bs = BoxState { uid_validity: ex.uid_validity, ..Default::default() };
        }
        let mut uids = sess.uid_search("ALL").await?;
        if let Some(b) = bs.below {
            uids.retain(|u| *u < b);
        }
        // newest first
        uids.sort_unstable_by(|a, b| b.cmp(a));
        uids.dedup();
        let cats = if uids.is_empty() { HashMap::new() } else { categories(&mut sess, &names).await? };
        log::info!("mail: fast download of the {role} folder for account {account_id}: {} messages to go", uids.len());
        for chunk in uids.chunks(CHUNK) {
            let (hi, lo) = (chunk[0], chunk[chunk.len() - 1]);
            let recs = sess.fetch_headers(lo, hi).await?;
            let msgs: Vec<Message> = recs.iter().map(|r| to_message(r, role, &names, cats.get(&r.uid).map(String::as_str))).collect();
            let n = msgs.len() as i64;
            {
                let bs = st.boxes.get_mut(role).expect("box state");
                bs.below = Some(lo);
                bs.stored += n;
            }
            done += n;
            // store this chunk while the next one downloads (one write in flight)
            if let Some(h) = pending.take() {
                h.await??;
            }
            let snapshot = serde_json::to_string(&*st)?;
            let map2 = map.clone();
            let recount = ui.ready();
            let ctx2 = ctx.clone();
            pending = Some(tokio::spawn(async move {
                ctx2.db
                    .write(move |c| {
                        let tx = c.transaction()?;
                        for m in &msgs {
                            store::upsert_meta(&tx, account_id, &store::meta_from(m), &map2, generation)?;
                        }
                        set_sync_state(&tx, account_id, K_IMAP, Some(&snapshot))?;
                        if recount {
                            store::recount(&tx, account_id)?;
                        }
                        tx.commit()?;
                        Ok(())
                    })
                    .await?;
                if recount {
                    ctx2.events.mail_changed(account_id, None);
                }
                Ok(())
            }));
            if status.ready() {
                let t = total.max(done).max(1);
                ctx.events.status(
                    account_id,
                    "sync",
                    &format!("Downloading message headers (fast): {} of {}", fmt_n(done), fmt_n(t)),
                    Some((done as f64 / t as f64).min(1.0)),
                );
            }
            if started.elapsed() >= slice {
                if let Some(h) = pending.take() {
                    h.await??;
                }
                sess.logout().await;
                log::info!("mail: fast download for account {account_id} paused at {done} of {total}; continuing next cycle");
                return Ok(false);
            }
        }
        if let Some(h) = pending.take() {
            h.await??;
        }
        st.boxes.get_mut(role).expect("box state").complete = true;
        save_state(ctx, account_id, st).await?;
    }
    sess.logout().await;
    ctx.db.write(move |c| store::recount(c, account_id)).await?;
    ctx.events.mail_changed(account_id, None);
    Ok(true)
}

/// Gmail categories aren't IMAP labels; X-GM-RAW searches give them (UID → CATEGORY_* label).
/// A server that refuses the search just leaves categories to the label check afterwards.
async fn categories(sess: &mut Session, names: &LabelNames) -> anyhow::Result<HashMap<u32, String>> {
    let mut out = HashMap::new();
    for (id, q) in [
        ("CATEGORY_PERSONAL", "category:primary"),
        ("CATEGORY_SOCIAL", "category:social"),
        ("CATEGORY_PROMOTIONS", "category:promotions"),
        ("CATEGORY_UPDATES", "category:updates"),
        ("CATEGORY_FORUMS", "category:forums"),
    ] {
        if !names.ids.contains_key(id) {
            continue;
        }
        match sess.uid_search(&format!("X-GM-RAW {}", quote(q))).await {
            Ok(uids) => {
                for u in uids {
                    out.entry(u).or_insert_with(|| id.to_string());
                }
            }
            Err(e @ (ImapError::No(_) | ImapError::Protocol(_))) => {
                log::info!(
                    "mail: IMAP category search refused ({}); categories come from the label check",
                    crate::logging::redact(&e.to_string())
                );
                return Ok(HashMap::new());
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}

/// Gmail label names (as in X-GM-LABELS) → REST label ids.
pub(crate) struct LabelNames {
    exact: HashMap<String, String>,
    folded: HashMap<String, String>,
    ids: HashMap<String, ()>,
}

impl LabelNames {
    pub fn new(labels: &[Label]) -> Self {
        let mut n = LabelNames { exact: HashMap::new(), folded: HashMap::new(), ids: HashMap::new() };
        for l in labels {
            n.ids.insert(l.id.clone(), ());
            if l.kind == "user" {
                n.exact.insert(l.name.clone(), l.id.clone());
                n.folded.entry(l.name.to_lowercase()).or_insert_with(|| l.id.clone());
            }
        }
        n
    }
    fn user(&self, name: &str) -> Option<&String> {
        self.exact.get(name).or_else(|| self.folded.get(&name.to_lowercase()))
    }
}

/// `\Inbox` & co. → REST system label id.
pub(crate) fn system_label(s: &str) -> Option<&'static str> {
    Some(match s.to_ascii_lowercase().as_str() {
        "\\inbox" => "INBOX",
        "\\sent" => "SENT",
        "\\draft" | "\\drafts" => "DRAFT",
        "\\starred" | "\\flagged" => "STARRED",
        "\\important" => "IMPORTANT",
        "\\trash" => "TRASH",
        "\\spam" | "\\junk" => "SPAM",
        "\\chat" | "\\chats" => "CHAT",
        _ => return None,
    })
}

/// An IMAP header record as the `Message` the API's `format=metadata` would give, so the very
/// same `store::meta_from` / `upsert_meta` path writes it.
pub(crate) fn to_message(r: &HeaderRecord, role: &str, names: &LabelNames, category: Option<&str>) -> Message {
    let headers = parse_header_block(&r.header);
    let ct =
        headers.iter().find(|h| h.name.eq_ignore_ascii_case("Content-Type")).map(|h| mime::header_params(&h.value).0).unwrap_or_default();
    let ct = if ct.is_empty() { "text/plain".to_string() } else { ct.to_ascii_lowercase() };
    let mut labels: Vec<String> = Vec::new();
    let mut add = |l: &str| {
        if !labels.iter().any(|x| x == l) {
            labels.push(l.to_string());
        }
    };
    for l in &r.labels {
        match l {
            GmLabel::System(s) => {
                if let Some(id) = system_label(s) {
                    add(id);
                }
            }
            GmLabel::User(name) => match names.user(name) {
                Some(id) => add(id),
                None => log::debug!("mail: an IMAP label has no Gmail API label"),
            },
        }
    }
    let flag = |f: &str| r.flags.iter().any(|x| x.eq_ignore_ascii_case(f));
    if !flag("\\Seen") {
        add("UNREAD");
    }
    if flag("\\Flagged") {
        add("STARRED");
    }
    if flag("\\Draft") {
        add("DRAFT");
    }
    match role {
        "trash" => add("TRASH"),
        "spam" => add("SPAM"),
        _ => {}
    }
    if let Some(c) = category {
        add(c);
    }
    let id = format!("{:016x}", r.msgid);
    let thread_id = if r.thrid == 0 { id.clone() } else { format!("{:016x}", r.thrid) };
    let date = r
        .internal_date
        .or_else(|| headers.iter().find(|h| h.name.eq_ignore_ascii_case("Date")).and_then(|h| mime::parse_date(&h.value)))
        .unwrap_or(0);
    Message {
        id,
        thread_id,
        label_ids: labels,
        snippet: String::new(),
        history_id: "0".into(),
        internal_date: date.to_string(),
        size_estimate: r.size,
        payload: Some(MessagePart { mime_type: ct, headers, ..Default::default() }),
        raw: None,
    }
}

/// Raw header bytes (encoded words, raw UTF-8, or raw 8-bit legacy charsets) → unfolded headers.
pub(crate) fn parse_header_block(b: &[u8]) -> Vec<Header> {
    let text = decode_header_bytes(b);
    let mut out: Vec<Header> = Vec::new();
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = out.last_mut() {
                last.value.push(' ');
                last.value.push_str(line.trim());
            }
            continue;
        }
        if let Some((n, v)) = line.split_once(':') {
            out.push(Header { name: n.trim().to_string(), value: v.trim().to_string() });
        }
    }
    out
}

/// UTF-8 if valid; else Hebrew Windows-1255 when the high bytes look like Hebrew words (runs of
/// letters 0xE0–0xFA), else Windows-1252 (accented letters stand alone between ASCII ones).
fn decode_header_bytes(b: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(b) {
        return s.to_string();
    }
    let heb = |c: u8| (0xE0..=0xFA).contains(&c);
    let (mut high, mut in_runs) = (0usize, 0usize);
    for (i, c) in b.iter().enumerate() {
        if *c >= 0x80 {
            high += 1;
            let prev = i > 0 && heb(b[i - 1]);
            let next = b.get(i + 1).is_some_and(|n| heb(*n));
            if heb(*c) && (prev || next) {
                in_runs += 1;
            }
        }
    }
    let enc = if in_runs * 10 >= high * 6 { encoding_rs::WINDOWS_1255 } else { encoding_rs::WINDOWS_1252 };
    enc.decode_without_bom_handling(b).0.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> LabelNames {
        let l = |id: &str, name: &str, kind: &str| Label { id: id.into(), name: name.into(), kind: kind.into(), ..Default::default() };
        LabelNames::new(&[
            l("INBOX", "INBOX", "system"),
            l("CATEGORY_UPDATES", "CATEGORY_UPDATES", "system"),
            l("Label_1", "Clients/Orion Technologies", "user"),
            l("Label_9", "לקוחות", "user"),
            l("Label_10", "Conferences/\"Tel Aviv\" 2026", "user"),
        ])
    }

    #[test]
    fn maps_a_record_like_the_api() {
        let hdr = "From: =?UTF-8?B?15PXoNeUINeb15TXnw==?= <dana@x.co.il>\r\nTo: Kenny <k@duvz.demo>, \"Cohen, Dana\" <d@y.com>\r\nSubject: =?UTF-8?B?16LXk9eb15XXnyDXqNeR16LXldeg15k=?=\r\n =?UTF-8?B?IFEz?=\r\nDate: Tue, 7 Jul 2026 09:00:00 +0300\r\nContent-Type: multipart/mixed;\r\n\tboundary=\"x\"\r\nX-Priority: 1\r\n\r\n";
        let r = HeaderRecord {
            uid: 5,
            msgid: 0x18a3c0000123,
            thrid: 0x18a3c0000100,
            labels: vec![
                GmLabel::System("\\Inbox".into()),
                GmLabel::System("\\Important".into()),
                GmLabel::User("לקוחות".into()),
                GmLabel::User("clients/orion technologies".into()),
                GmLabel::User("Conferences/\"Tel Aviv\" 2026".into()),
                GmLabel::User("Gone".into()),
            ],
            flags: vec!["\\Flagged".into()],
            internal_date: Some(1_783_400_000_000),
            size: 1234,
            header: hdr.as_bytes().to_vec(),
        };
        let m = to_message(&r, "all", &names(), Some("CATEGORY_UPDATES"));
        assert_eq!(m.id, "000018a3c0000123");
        assert_eq!(m.thread_id, "000018a3c0000100");
        assert_eq!(m.label_ids, vec!["INBOX", "IMPORTANT", "Label_9", "Label_1", "Label_10", "UNREAD", "STARRED", "CATEGORY_UPDATES"]);
        let meta = store::meta_from(&m);
        assert_eq!(meta.subject, "עדכון רבעוני Q3");
        assert_eq!(meta.from.name, "דנה כהן");
        assert_eq!(meta.from.email, "dana@x.co.il");
        assert_eq!(meta.to.len(), 2);
        assert_eq!(meta.to[1].name, "Cohen, Dana");
        assert!(meta.has_attachments);
        assert_eq!(meta.importance, 2);
        assert_eq!(meta.internal_date, 1_783_400_000_000);
        assert_eq!(meta.size, 1234);
        assert!(meta.sent_date.is_some());
        // trash / spam folders add their label; \Seen = read
        let r2 = HeaderRecord { flags: vec!["\\Seen".into()], labels: vec![], ..r.clone() };
        assert_eq!(to_message(&r2, "trash", &names(), None).label_ids, vec!["TRASH"]);
        assert_eq!(to_message(&r2, "spam", &names(), None).label_ids, vec!["SPAM"]);
    }

    #[test]
    fn header_bytes_in_legacy_charsets() {
        // raw windows-1255 Hebrew (no encoded words) — seen in old Israeli mail
        let (b, _, _) = encoding_rs::WINDOWS_1255.encode("Subject: שלום רב\r\n");
        let h = parse_header_block(&b);
        assert_eq!(h[0].value, "שלום רב");
        let h = parse_header_block(b"Subject: caf\xe9 cr\xe8me\r\n");
        assert_eq!(h[0].value, "café crème");
        let h = parse_header_block("Subject: שלום UTF-8\r\n\r\n".as_bytes());
        assert_eq!(h[0].value, "שלום UTF-8");
        assert!(parse_header_block(b"").is_empty());
    }

    #[test]
    fn system_labels() {
        assert_eq!(system_label("\\Inbox"), Some("INBOX"));
        assert_eq!(system_label("\\IMPORTANT"), Some("IMPORTANT"));
        assert_eq!(system_label("\\Drafts"), Some("DRAFT"));
        assert_eq!(system_label("\\Junk"), Some("SPAM"));
        assert_eq!(system_label("\\Muted"), None);
    }
}

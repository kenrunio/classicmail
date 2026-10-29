//! Fast initial download over IMAP, against the fake Gmail IMAP server (same mailbox as the fake
//! REST API): same rows as the API full sync, resumable, read-only, falls back to the API, takes
//! over a half-done API sync, and history picks up what changed during the download.

use std::{collections::HashSet, sync::Arc, time::Duration};

use rusqlite::types::Value as SqlValue;
use serde_json::{Value, json};

use super::{imap_sync, sync};
use crate::{
    accounts,
    ctx::AppCtx,
    db::util::{get_sync_state, set_setting, set_sync_state},
    fake_google::imap::{self as fimap, Faults},
    google::Api,
};

struct T {
    ctx: Arc<AppCtx>,
    _g: crate::db::tempdir::TempDirGuard,
    api: Api,
    id: i64,
    email: String,
}

async fn setup(email: &str) -> T {
    let (ctx, g) = AppCtx::for_test().unwrap();
    let a = accounts::upsert(&ctx.db, email, "Kenny Green", "demo").await.unwrap();
    let api = ctx.api(a.id).await.unwrap();
    assert!(api.urls.imap.starts_with("imap://127.0.0.1:"), "demo accounts use the fake IMAP: {}", api.urls.imap);
    T { ctx, _g: g, api, id: a.id, email: email.to_string() }
}

const SNAPSHOT_SQL: &str = "SELECT m.gmail_id, m.thread_id, m.label_ids, m.is_read, m.is_flagged, m.flag_status, m.is_draft, m.location, \
    m.internal_date / 1000, m.sent_date, m.subject, m.from_name, m.from_email, m.to_json, m.cc_json, m.bcc_json, m.reply_to_json, m.size, \
    m.message_id_header, m.in_reply_to, m.references_header, m.importance, m.has_attachments, m.subject_norm, m.to_display, \
    (SELECT group_concat(p, '|') FROM (SELECT f.path AS p FROM message_folders mf JOIN folders f ON f.id = mf.folder_id WHERE mf.message_id = m.id ORDER BY f.path)) \
    FROM messages m WHERE m.account_id = ?1 ORDER BY m.gmail_id";

const COLUMNS: &[&str] = &[
    "gmail_id",
    "thread_id",
    "label_ids",
    "is_read",
    "is_flagged",
    "flag_status",
    "is_draft",
    "location",
    "internal_date(s)",
    "sent_date",
    "subject",
    "from_name",
    "from_email",
    "to_json",
    "cc_json",
    "bcc_json",
    "reply_to_json",
    "size",
    "message_id_header",
    "in_reply_to",
    "references_header",
    "importance",
    "has_attachments",
    "subject_norm",
    "to_display",
    "folders",
];

impl T {
    async fn sync(&self) {
        sync::sync_account(&self.ctx, &self.api).await.unwrap();
    }
    async fn slice(&self) -> bool {
        sync::full_sync_slice_for_test(&self.ctx, &self.api, Duration::ZERO).await.unwrap()
    }
    async fn state(&self, k: &'static str) -> Option<String> {
        let id = self.id;
        self.ctx.db.read(move |c| get_sync_state(c, id, k)).await.unwrap()
    }
    async fn one(&self, sql: &'static str) -> i64 {
        let id = self.id;
        self.ctx.db.read(move |c| Ok(c.query_row(sql, [id], |r| r.get(0))?)).await.unwrap()
    }
    async fn set_fast(&self, on: bool) {
        self.ctx.db.write(move |c| set_setting(c, "mail.fastInitialSync", &on)).await.unwrap();
    }
    async fn x_demo(&self, what: &str, body: Value) -> Value {
        let url = format!("{}/users/me/x-demo/{what}", self.api.urls.gmail);
        self.api.post(&url, &[], body).await.unwrap()
    }
    fn statuses(&self) -> Vec<String> {
        self.ctx
            .events
            .captured()
            .into_iter()
            .filter(|e| e["kind"] == "sync.status")
            .map(|e| e["text"].as_str().unwrap_or("").to_string())
            .collect()
    }
    async fn total(&self) -> i64 {
        self.api.gmail_profile().await.unwrap().messages_total
    }
    /// Every message's columns as text (labels sorted), keyed by gmail id order.
    async fn snapshot(&self) -> Vec<Vec<String>> {
        let id = self.id;
        self.ctx
            .db
            .read(move |c| {
                let mut st = c.prepare(SNAPSHOT_SQL)?;
                let rows = st
                    .query_map([id], |r| {
                        let mut v = Vec::new();
                        for i in 0..COLUMNS.len() {
                            let x: SqlValue = r.get(i)?;
                            let s = match x {
                                SqlValue::Null => "NULL".to_string(),
                                SqlValue::Integer(n) => n.to_string(),
                                SqlValue::Real(f) => f.to_string(),
                                SqlValue::Text(t) => t,
                                SqlValue::Blob(b) => format!("{b:?}"),
                            };
                            v.push(if i == 2 {
                                let mut l: Vec<&str> = s.split_whitespace().collect();
                                l.sort();
                                l.join(" ")
                            } else {
                                s
                            });
                        }
                        Ok(v)
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap()
    }
    /// Local labels of every message equal the server's (API `minimal`), message set equal too.
    async fn assert_matches_server(&self) {
        let mut remote: Vec<(String, Vec<String>)> = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self.api.gmail_list(None, &[], token.as_deref(), 500, true).await.unwrap();
            for m in &page.messages {
                let mut l = self.api.gmail_get(&m.id, "minimal", &[]).await.unwrap().label_ids;
                l.sort();
                remote.push((m.id.clone(), l));
            }
            token = page.next_page_token;
            if token.is_none() {
                break;
            }
        }
        remote.sort();
        let local: Vec<(String, Vec<String>)> =
            self.snapshot().await.into_iter().map(|r| (r[0].clone(), r[2].split_whitespace().map(str::to_string).collect())).collect();
        assert_eq!(local.len(), remote.len(), "message count");
        for (l, r) in local.iter().zip(&remote) {
            assert_eq!(l, r, "labels of {}", l.0);
        }
    }
}

fn diff(a: &[Vec<String>], b: &[Vec<String>]) -> Vec<String> {
    let mut out = Vec::new();
    if a.len() != b.len() {
        out.push(format!("row count {} vs {}", a.len(), b.len()));
    }
    for (ra, rb) in a.iter().zip(b) {
        for (i, (x, y)) in ra.iter().zip(rb).enumerate() {
            if x != y {
                out.push(format!("{} {}: imap {x:?} vs api {y:?}", ra[0], COLUMNS[i]));
            }
        }
    }
    out
}

/// The IMAP bootstrap alone writes exactly the rows the API full sync writes (bar the snippet,
/// which comes with the body, and sub-second internal dates, which IMAP doesn't carry).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn imap_bootstrap_matches_api_full_sync() {
    let t = setup("imap-eq+seed@duvz.demo").await;
    let map = sync::labels_for_test(&t.ctx, &t.api).await.unwrap();
    let r = imap_sync::run(&t.ctx, &t.api, &map, 1, Duration::from_secs(3600)).await.unwrap();
    assert!(matches!(r, imap_sync::Fast::Done { now: true }));
    let via_imap = t.snapshot().await;
    assert_eq!(via_imap.len() as i64, t.total().await);
    // what the seed makes sure of
    assert!(via_imap.iter().any(|r| r[10].chars().any(|c| ('\u{5d0}'..='\u{5ea}').contains(&c))), "Hebrew subjects");
    assert!(via_imap.iter().any(|r| r[25].contains("לקוחות/ישראל")), "Hebrew label (modified UTF-7)");
    assert!(via_imap.iter().any(|r| r[25].contains("Conferences/\"Tel Aviv\" 2026")), "label with quotes");
    assert!(via_imap.iter().any(|r| r[2].contains("CATEGORY_UPDATES")), "categories via X-GM-RAW");
    assert!(via_imap.iter().any(|r| r[25] == "Deleted Items") && via_imap.iter().any(|r| r[25] == "Junk E-mail"));
    let s = fimap::stats(&t.email);
    assert_eq!((s.writes, s.non_readonly), (0, 0), "read-only: EXAMINE + BODY.PEEK only");
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1 AND snippet<>''").await, 0, "headers only");

    // the same mailbox through the API path
    let id = t.id;
    t.ctx
        .db
        .write(move |c| {
            c.execute("DELETE FROM messages_fts WHERE rowid IN (SELECT id FROM messages WHERE account_id=?1)", [id])?;
            c.execute("DELETE FROM messages WHERE account_id=?1", [id])?;
            set_sync_state(c, id, imap_sync::K_IMAP, None)?;
            Ok(())
        })
        .await
        .unwrap();
    t.set_fast(false).await;
    t.sync().await;
    let via_api = t.snapshot().await;
    let d = diff(&via_imap, &via_api);
    assert!(d.is_empty(), "{} differences:\n{}", d.len(), d.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
}

/// Default first sync of a seeded demo account: IMAP download, API check, labels resync, then
/// incremental history; folder counts match Gmail's label counts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_sync_uses_imap_and_matches_server() {
    let t = setup("imap-flow+seed@duvz.demo").await;
    t.sync().await;
    let total = t.total().await;
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await, total);
    let s = fimap::stats(&t.email);
    assert_eq!(s.fetched as i64, total, "every message came over IMAP, once");
    assert_eq!((s.writes, s.non_readonly), (0, 0));
    assert_eq!(
        t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1 AND snippet<>'' AND body_state=0").await,
        0,
        "no API metadata fetches (bodies prefetched in the background may already have set a snippet)"
    );
    assert!(t.statuses().iter().any(|s| s.starts_with("Downloading message headers (fast):")), "{:?}", t.statuses());
    assert!(t.statuses().iter().any(|s| s.starts_with("Checking the message list:")));
    assert!(t.state("mail.historyId").await.is_some());
    assert!(t.state("mail.full.historyId").await.is_none());
    assert!(t.state(imap_sync::K_IMAP).await.is_none());
    t.assert_matches_server().await;
    for (label, role) in [("INBOX", "inbox"), ("SENT", "sent"), ("TRASH", "trash"), ("SPAM", "spam"), ("DRAFT", "drafts")] {
        let l = t.api.gmail_label(label).await.unwrap();
        let role = role.to_string();
        let id = t.id;
        let (total, unread): (i64, i64) = t
            .ctx
            .db
            .read(move |c| {
                Ok(c.query_row(
                    "SELECT total_count, unread_count FROM folders WHERE account_id=?1 AND role=?2",
                    rusqlite::params![id, role],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!((total, unread), (l.messages_total.unwrap(), l.messages_unread.unwrap()), "{label}");
    }
    // opening a message fills its snippet
    let mid = t.one("SELECT id FROM messages WHERE account_id=?1 ORDER BY internal_date DESC LIMIT 1").await;
    super::body::ensure_body(&t.ctx, mid).await.unwrap();
    let snip: String =
        t.ctx.db.read(move |c| Ok(c.query_row("SELECT snippet FROM messages WHERE id=?1", [mid], |r| r.get(0))?)).await.unwrap();
    assert!(!snip.is_empty());
    // incremental from here; IMAP not used again
    t.x_demo("deliver", json!({})).await;
    t.sync().await;
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await, total + 1);
    assert_eq!(fimap::stats(&t.email).logins, s.logins);
}

/// Interrupted downloads continue where they stopped: time slices and a dropped connection.
/// Nothing is downloaded twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn imap_download_resumes() {
    let t = setup("imap-resume+seed@duvz.demo").await;
    t.x_demo("bulk", json!({ "count": 5000 })).await;
    let total = t.total().await;
    // first slice: one chunk of 2,000, then the slice is used up
    assert!(!t.slice().await);
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await, 2000);
    assert!(t.statuses().iter().any(|s| s.starts_with("Downloading message headers (fast): 2,000 of ")), "{:?}", t.statuses());
    // the connection drops before the next chunk: this cycle falls back to the API for a page,
    // the next one goes on over IMAP from where it stopped
    fimap::set_faults(&t.email, Faults { drop_after_fetches: Some(0), ..Default::default() });
    assert!(!t.slice().await);
    let st = imap_sync::load_state(&t.ctx, t.id).await.unwrap();
    assert_eq!((st.phase.as_str(), st.attempts), ("", 1), "{st:?}");
    assert!(st.error.as_deref().unwrap_or("").contains("network"), "{st:?}");
    fimap::set_faults(&t.email, Faults::default());
    let mut rounds = 0;
    while !t.slice().await {
        rounds += 1;
        assert!(rounds < 20);
    }
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await, total);
    let s = fimap::stats(&t.email);
    assert_eq!(s.fetched as i64, total, "no message fetched twice over IMAP");
    assert!(s.logins >= 3);
    assert!(t.state("mail.historyId").await.is_some());
    t.assert_matches_server().await;
}

/// IMAP refused (Workspace admin disabled it): the API full sync does the job, a one-line status
/// says so, and IMAP isn't tried again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn falls_back_to_the_api_when_imap_is_refused() {
    let t = setup("imap-off+seed@duvz.demo").await;
    fimap::set_faults(
        &t.email,
        Faults { refuse_login: Some("IMAP access is disabled for your domain. Please contact your domain administrator for questions about this feature. (Failure)".into()), ..Default::default() },
    );
    t.sync().await;
    let total = t.total().await;
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await, total);
    assert!(t.statuses().iter().any(|s| s == imap_sync::UNAVAILABLE), "{:?}", t.statuses());
    assert!(t.statuses().iter().any(|s| s.starts_with("Receiving message")));
    assert_eq!(fimap::stats(&t.email).fetched, 0);
    assert!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1 AND snippet<>''").await > 0, "API metadata");
    assert!(t.state("mail.historyId").await.is_some());
    t.assert_matches_server().await;

    // a server without special-use flags (no way to find All Mail): same fallback
    let t = setup("imap-nospecial+seed@duvz.demo").await;
    fimap::set_faults(&t.email, Faults { hide_special_use: true, ..Default::default() });
    t.sync().await;
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await, t.total().await);
    assert!(t.statuses().iter().any(|s| s == imap_sync::UNAVAILABLE));
    assert_eq!(fimap::stats(&t.email).fetched, 0);

    // switched off in settings: never connects
    let t = setup("imap-setting+seed@duvz.demo").await;
    t.set_fast(false).await;
    t.sync().await;
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await, t.total().await);
    assert_eq!(fimap::stats(&t.email).logins, 0);
}

/// An account half way through an API full sync (page token stored) switches to IMAP on the next
/// run and keeps what it already has.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn half_done_api_sync_switches_to_imap() {
    let t = setup("imap-switch+seed@duvz.demo").await;
    t.x_demo("bulk", json!({ "count": 1500 })).await;
    t.set_fast(false).await;
    assert!(!t.slice().await);
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await, 500);
    assert!(t.state("mail.full.pageToken").await.is_some());
    let full_history = t.state("mail.full.historyId").await.unwrap();
    let before: HashSet<String> = t.snapshot().await.into_iter().map(|r| r[0].clone()).collect();
    t.set_fast(true).await;
    t.sync().await;
    let total = t.total().await;
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await, total);
    assert_eq!(t.one("SELECT COUNT(DISTINCT gmail_id) FROM messages WHERE account_id=?1").await, total);
    // the 500 from the API kept their rows (and snippets); the rest came over IMAP
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1 AND snippet<>''").await, 500);
    let after: HashSet<String> = t.snapshot().await.into_iter().map(|r| r[0].clone()).collect();
    assert!(before.is_subset(&after));
    assert!(fimap::stats(&t.email).fetched as i64 >= total - 500);
    assert!(t.state("mail.full.pageToken").await.is_none());
    // history continues from the point the API sync recorded at its start
    let h: u64 = t.state("mail.historyId").await.unwrap().parse().unwrap();
    assert!(h >= full_history.parse::<u64>().unwrap());
    t.assert_matches_server().await;
}

/// Changes made in Gmail while the download runs (between slices) end up locally: read / starred,
/// trashed, deleted, relabelled, new mail.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changes_during_the_download_are_picked_up() {
    let t = setup("imap-changes+seed@duvz.demo").await;
    t.x_demo("bulk", json!({ "count": 3000 })).await;
    assert!(!t.slice().await, "paused after the first chunk");
    let local: Vec<Vec<String>> = t.snapshot().await;
    let local_ids: HashSet<String> = local.iter().map(|r| r[0].clone()).collect();
    let unread = local.iter().find(|r| r[3] == "0").map(|r| r[0].clone()).expect("an unread message");
    let mut others = local.iter().filter(|r| r[0] != unread && r[7] == "0").map(|r| r[0].clone());
    let (to_trash, to_delete, to_label) = (others.next().unwrap(), others.next().unwrap(), others.next().unwrap());
    // one the download hasn't reached yet
    let mut later = None;
    let mut token: Option<String> = None;
    while later.is_none() {
        let page = t.api.gmail_list(None, &[], token.as_deref(), 500, true).await.unwrap();
        later = page.messages.iter().map(|m| m.id.clone()).find(|id| !local_ids.contains(id));
        token = page.next_page_token;
        if token.is_none() {
            break;
        }
    }
    let later = later.expect("a message not downloaded yet");
    let receipts = t.api.gmail_labels().await.unwrap().into_iter().find(|l| l.name == "Bulk").unwrap().id;
    t.api.gmail_modify(&unread, &["STARRED".to_string()], &["UNREAD".to_string()]).await.unwrap();
    t.api.gmail_trash(&to_trash).await.unwrap();
    t.api.gmail_delete(&to_delete).await.unwrap();
    t.api.gmail_modify(&to_label, std::slice::from_ref(&receipts), &[]).await.unwrap();
    t.api.gmail_modify(&later, &["STARRED".to_string()], &[]).await.unwrap();
    let new_id = t.x_demo("deliver", json!({})).await["id"].as_str().unwrap().to_string();
    t.sync().await;
    assert!(t.state("mail.historyId").await.is_some());
    let snap = t.snapshot().await;
    let row = |id: &str| snap.iter().find(|r| r[0] == id).cloned();
    assert!(row(&to_delete).is_none(), "deleted on the server");
    assert!(row(&new_id).is_some(), "new mail");
    let u = row(&unread).unwrap();
    assert_eq!((u[3].as_str(), u[4].as_str()), ("1", "1"), "read + flagged");
    assert_eq!(row(&to_trash).unwrap()[7], "1", "in Deleted Items");
    assert!(row(&to_label).unwrap()[2].contains(&receipts));
    assert_eq!(row(&later).unwrap()[4], "1");
    t.assert_matches_server().await;
}

/// Large-mailbox timing: `CM_BIG=50000 cargo test --lib imap_big -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore]
async fn imap_big_mailbox_performance() {
    let t = setup("imap-perf+seed@duvz.demo").await;
    let n: u64 = std::env::var("CM_BIG").ok().and_then(|v| v.parse().ok()).unwrap_or(50_000);
    t.x_demo("bulk", json!({ "count": n })).await;
    let total = t.total().await;
    let map = sync::labels_for_test(&t.ctx, &t.api).await.unwrap();
    let started = std::time::Instant::now();
    imap_sync::run(&t.ctx, &t.api, &map, 1, Duration::from_secs(3600)).await.unwrap();
    let secs = started.elapsed().as_secs_f64();
    let local = t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await;
    println!("IMAP header download: {local} of {total} messages in {secs:.1}s ({:.0} msg/s)", local as f64 / secs);
    let id = t.id;
    t.ctx.db.write(move |c| set_sync_state(c, id, imap_sync::K_IMAP, None)).await.unwrap();
    let started = std::time::Instant::now();
    t.sync().await;
    println!("whole first sync (IMAP + API check + label resync + history): {:.1}s", started.elapsed().as_secs_f64());
}

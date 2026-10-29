//! Mail sync for one account: Outbox → pending local changes → labels → full sync (resumable,
//! newest first, metadata 8 in flight) or incremental via history (404 → full resync keeping local
//! flags) → counts → background body prefetch.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use futures::{StreamExt, stream};
use parking_lot::Mutex;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::store::{self, LabelMap, Meta};
use crate::{
    archive::index::fmt_n,
    ctx::AppCtx,
    db::util::{get_setting, get_sync_state, now_ms, set_sync_state},
    google::{Api, client::api_status, gmail::Message},
};

const K_HISTORY: &str = "mail.historyId";
const K_FULL_HISTORY: &str = "mail.full.historyId";
const K_FULL_TOKEN: &str = "mail.full.pageToken";
const K_FULL_GEN: &str = "mail.full.generation";
const K_FULL_DONE: &str = "mail.full.received";
const K_FULL_TOTAL: &str = "mail.full.total";
const K_FULL_RESYNC: &str = "mail.full.resync";
/// history id up to which new mail was already caught up while a sliced full sync runs
const K_FULL_CATCHUP: &str = "mail.full.catchup";

/// Gmail metadata requests in flight at once.
pub(super) const CONCURRENCY: usize = 8;
const PAGE: u32 = 500;
/// A full sync of a big mailbox takes hours under Gmail's quota (6,000 units/min, 20 per message):
/// work in slices so calendar, contacts and tasks get their turn in between. Progress is kept in
/// sync_state; the next mail cycle continues where this one stopped.
const FULL_SYNC_SLICE: Duration = Duration::from_secs(120);

/// One sync at a time per account (sync cycle, immediate pushes after local actions).
fn account_lock(account_id: i64) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(Default::default).lock().entry(account_id).or_default().clone()
}

pub async fn sync_account(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    // send first; a failed send stays in the Outbox and is reported after we've pulled
    let outbox = super::compose::process_outbox(ctx, api).await;
    let pulled = pull(ctx, api).await;
    pulled?;
    outbox
}

async fn pull(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    start_demo_delivery(ctx, api.account_id).await;
    let lock = account_lock(api.account_id);
    let _guard = lock.lock().await;
    push_pending_locked(ctx, api).await?;
    let map = sync_labels(ctx, api).await?;
    let account_id = api.account_id;
    let state =
        ctx.db.read(move |c| Ok((get_sync_state(c, account_id, K_HISTORY)?, get_sync_state(c, account_id, K_FULL_HISTORY)?))).await?;
    let complete = match state {
        (Some(h), None) => match incremental(ctx, api, &h, &map).await {
            Ok(()) => true,
            Err(e) if api_status(&e) == Some(404) => {
                log::info!("mail: history {h} expired for account {account_id}; full resync");
                ctx.db
                    .write(move |c| {
                        set_sync_state(c, account_id, K_HISTORY, None)?;
                        set_sync_state(c, account_id, K_FULL_RESYNC, Some("1"))?;
                        Ok(())
                    })
                    .await?;
                full_sync(ctx, api, &map).await?
            }
            Err(e) => return Err(e),
        },
        _ => full_sync(ctx, api, &map).await?,
    };
    ctx.db.write(move |c| store::recount(c, account_id)).await?;
    ctx.events.mail_changed(account_id, None);
    // bodies only once all headers are here: both compete for the same Gmail quota
    if complete {
        spawn_prefetch(ctx.clone(), account_id);
    }
    Ok(())
}

/// Labels → folders. Returns the label map for this account.
async fn sync_labels(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<LabelMap> {
    let labels = api.gmail_labels().await?;
    let account_id = api.account_id;
    let changed = ctx
        .db
        .write(move |c| {
            let show_all: bool = get_setting(c, "mail.showAllMail")?.unwrap_or(false);
            let changed = store::sync_labels(c, account_id, &labels, show_all)?;
            if changed {
                // re-derive membership for folders that were just created (e.g. a new label)
                let map = LabelMap::load(c, account_id)?;
                let tx = c.transaction()?;
                store::rederive_membership(&tx, account_id, &map)?;
                store::recount(&tx, account_id)?;
                tx.commit()?;
            }
            Ok(changed)
        })
        .await?;
    if changed {
        ctx.events.mail_changed(account_id, None);
    }
    ctx.db.read(move |c| LabelMap::load(c, account_id)).await
}

/// One message's metadata (`format=metadata`), tagged with its id.
pub(super) async fn fetch_meta(api: &Api, id: String) -> (String, anyhow::Result<Message>) {
    let r = api.gmail_get(&id, "metadata", store::METADATA_HEADERS).await;
    (id, r)
}

/// Throttle for UI events during long operations: the first call is always ready.
pub(super) struct Throttle {
    last: Option<Instant>,
    every: Duration,
}

impl Throttle {
    pub(super) fn new(ms: u64) -> Self {
        Throttle { last: None, every: Duration::from_millis(ms) }
    }
    pub(super) fn ready(&mut self) -> bool {
        if self.last.is_none_or(|t| t.elapsed() >= self.every) {
            self.last = Some(Instant::now());
            true
        } else {
            false
        }
    }
}

/// Full (re)sync: list every message newest first and store metadata. Resumable via sync_state;
/// returns false when it stopped after `FULL_SYNC_SLICE` and will continue next cycle.
async fn full_sync(ctx: &Arc<AppCtx>, api: &Api, map: &LabelMap) -> anyhow::Result<bool> {
    full_sync_sliced(ctx, api, map, FULL_SYNC_SLICE).await
}

pub(crate) async fn full_sync_sliced(ctx: &Arc<AppCtx>, api: &Api, map: &LabelMap, slice: Duration) -> anyhow::Result<bool> {
    let started = Instant::now();
    let account_id = api.account_id;
    type St = (Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>);
    let st: St = ctx
        .db
        .read(move |c| {
            Ok((
                get_sync_state(c, account_id, K_FULL_HISTORY)?,
                get_sync_state(c, account_id, K_FULL_TOKEN)?,
                get_sync_state(c, account_id, K_FULL_GEN)?,
                get_sync_state(c, account_id, K_FULL_DONE)?,
                get_sync_state(c, account_id, K_FULL_TOTAL)?,
                get_sync_state(c, account_id, K_FULL_RESYNC)?,
            ))
        })
        .await?;
    let resumed = matches!(st, (Some(_), _, Some(_), _, _, _));
    let (start_history, mut token, generation, mut received, total, mut resync) = match st {
        (Some(h), tok, Some(g), done, total, resync) => (
            h,
            tok,
            g.parse::<i64>().unwrap_or(1),
            done.and_then(|d| d.parse().ok()).unwrap_or(0i64),
            total.and_then(|t| t.parse().ok()).unwrap_or(0i64),
            resync.is_some(),
        ),
        (_, _, _, _, _, resync) => {
            let prof = api.gmail_profile().await?;
            let generation: i64 = ctx
                .db
                .read(move |c| {
                    Ok(c.query_row("SELECT COALESCE(MAX(sync_gen), 0) + 1 FROM messages WHERE account_id=?1", [account_id], |r| r.get(0))?)
                })
                .await?;
            let h = prof.history_id.clone();
            let total = prof.messages_total;
            ctx.db
                .write(move |c| {
                    set_sync_state(c, account_id, K_FULL_HISTORY, Some(&h))?;
                    set_sync_state(c, account_id, K_FULL_GEN, Some(&generation.to_string()))?;
                    set_sync_state(c, account_id, K_FULL_TOKEN, None)?;
                    set_sync_state(c, account_id, K_FULL_DONE, Some("0"))?;
                    set_sync_state(c, account_id, K_FULL_TOTAL, Some(&total.to_string()))?;
                    set_sync_state(c, account_id, super::imap_sync::K_IMAP, None)?;
                    Ok(())
                })
                .await?;
            (prof.history_id, None, generation, 0, total, resync.is_some())
        }
    };
    let total = total.max(1);
    // A first sync of a big mailbox spans many cycles: meanwhile, pick up mail that arrived since
    // it started (with alerts), so new mail isn't held back for hours.
    if resumed {
        let from = ctx.db.read(move |c| get_sync_state(c, account_id, K_FULL_CATCHUP)).await?.unwrap_or_else(|| start_history.clone());
        match incremental_inner(ctx, api, &from, map, true, K_FULL_CATCHUP, generation).await {
            Ok(()) => {}
            Err(e) if api_status(&e) == Some(404) => log::info!("mail: catch-up history expired for account {account_id}"),
            Err(e) => return Err(e),
        }
    }
    // Fast path: headers of the whole mailbox over IMAP (not metered like the API). The listing
    // below then only checks every id (5 units per 500) and fetches what IMAP didn't cover, and
    // labels are re-checked per label (`resync_labels`) so they match the API exactly.
    let fast = super::imap_sync::run(ctx, api, map, generation, slice.saturating_sub(started.elapsed())).await?;
    let verify = match fast {
        super::imap_sync::Fast::Paused => return Ok(false),
        super::imap_sync::Fast::NotUsed => false,
        super::imap_sync::Fast::Done { now } => {
            if now {
                // list from the top again (an API sync may have been half way), then fix labels
                token = None;
                received = 0;
                resync = true;
                ctx.db
                    .write(move |c| {
                        set_sync_state(c, account_id, K_FULL_TOKEN, None)?;
                        set_sync_state(c, account_id, K_FULL_DONE, Some("0"))?;
                        set_sync_state(c, account_id, K_FULL_RESYNC, Some("1"))?;
                        Ok(())
                    })
                    .await?;
            }
            true
        }
    };
    let progress_text = |n: i64| {
        if verify {
            format!("Checking the message list: {} of {}", fmt_n(n.min(total)), fmt_n(total))
        } else {
            format!("Receiving message {} of {}", fmt_n(n.min(total)), fmt_n(total))
        }
    };
    let mut ui = Throttle::new(1200);
    let mut status = Throttle::new(250);
    ctx.events.status(account_id, "sync", &progress_text(received), Some((received as f64 / total as f64).min(1.0)));
    loop {
        let page = match api.gmail_list(None, &[], token.as_deref(), PAGE, true).await {
            Ok(p) => p,
            // a stored page token from an interrupted sync can go stale: list again from the top
            // (messages already here are only marked as seen, not fetched again)
            Err(e) if token.is_some() && api_status(&e) == Some(400) => {
                log::info!("mail: page token rejected for account {account_id} ({}); restarting the listing", crate::logging::err(&e));
                token = None;
                continue;
            }
            Err(e) => return Err(e),
        };
        let ids: Vec<String> = page.messages.iter().map(|m| m.id.clone()).collect();
        // which are already here (resumed / resync)? mark them as seen in this generation
        let ids2 = ids.clone();
        let existing: HashSet<String> = ctx
            .db
            .write(move |c| {
                let tx = c.transaction()?;
                let mut have = HashSet::new();
                {
                    let mut st = tx.prepare_cached("UPDATE messages SET sync_gen=?1 WHERE account_id=?2 AND gmail_id=?3")?;
                    for id in &ids2 {
                        if st.execute(params![generation, account_id, id])? > 0 {
                            have.insert(id.clone());
                        }
                    }
                }
                tx.commit()?;
                Ok(have)
            })
            .await?;
        // new messages: metadata (20 quota units each). Messages we already have keep their row;
        // on a resync their labels are re-read in bulk afterwards (`resync_labels`), a few hundred
        // units in all instead of 20 per message.
        let wanted: Vec<String> = ids.iter().filter(|i| !existing.contains(*i)).cloned().collect();
        if verify {
            // already here (fast download): counts as progress
            received += existing.len() as i64;
            if !wanted.is_empty() {
                log::info!("mail: {} messages of this page were not in the fast download; fetching them from the API", wanted.len());
            }
        }
        let mut results = stream::iter(wanted.into_iter().map(|id| fetch_meta(api, id))).buffer_unordered(CONCURRENCY);
        let mut batch: Vec<Message> = Vec::new();
        loop {
            let next = results.next().await;
            let done = next.is_none();
            if let Some((id, r)) = next {
                match r {
                    Ok(m) => batch.push(m),
                    Err(e) if api_status(&e) == Some(404) => log::debug!("mail: {id} vanished during sync"),
                    Err(e) => return Err(e),
                }
            }
            if batch.len() >= 50 || (done && !batch.is_empty()) {
                let n = batch.len() as i64;
                let b = std::mem::take(&mut batch);
                let map2 = map.clone();
                ctx.db
                    .write(move |c| {
                        let tx = c.transaction()?;
                        for m in &b {
                            store::upsert_meta(&tx, account_id, &store::meta_from(m), &map2, generation)?;
                        }
                        tx.commit()?;
                        Ok(())
                    })
                    .await?;
                received += n;
                if status.ready() {
                    ctx.events.status(account_id, "sync", &progress_text(received), Some((received as f64 / total as f64).min(1.0)));
                }
                if ui.ready() {
                    ctx.db.write(move |c| store::recount(c, account_id)).await?;
                    ctx.events.mail_changed(account_id, None);
                }
            }
            if done {
                break;
            }
        }
        drop(results);
        token = page.next_page_token.clone();
        let t2 = token.clone();
        ctx.db
            .write(move |c| {
                set_sync_state(c, account_id, K_FULL_TOKEN, t2.as_deref())?;
                set_sync_state(c, account_id, K_FULL_DONE, Some(&received.to_string()))?;
                Ok(())
            })
            .await?;
        if token.is_none() {
            break;
        }
        if started.elapsed() >= slice {
            log::info!("mail: full sync for account {account_id} paused at {received} of {total}; continuing next cycle");
            return Ok(false);
        }
    }
    if resync {
        resync_labels(ctx, api, map, generation).await?;
    }
    // anything not seen in this generation is gone from the server
    let caught_up = ctx.db.read(move |c| get_sync_state(c, account_id, K_FULL_CATCHUP)).await?;
    let h = caught_up.unwrap_or_else(|| start_history.clone());
    let h2 = h.clone();
    let removed = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let gone: Vec<i64> = {
                let mut st = tx.prepare("SELECT id FROM messages WHERE account_id=?1 AND sync_gen < ?2")?;
                st.query_map(params![account_id, generation], |r| r.get(0))?.collect::<Result<_, _>>()?
            };
            store::delete_messages(&tx, &gone)?;
            for k in [
                K_FULL_HISTORY,
                K_FULL_TOKEN,
                K_FULL_GEN,
                K_FULL_DONE,
                K_FULL_TOTAL,
                K_FULL_RESYNC,
                K_FULL_CATCHUP,
                super::imap_sync::K_IMAP,
            ] {
                set_sync_state(&tx, account_id, k, None)?;
            }
            set_sync_state(&tx, account_id, K_HISTORY, Some(&h2))?;
            store::recount(&tx, account_id)?;
            tx.commit()?;
            Ok(gone.len())
        })
        .await?;
    if removed > 0 {
        log::info!("mail: full sync removed {removed} stale messages for account {account_id}");
    }
    ctx.events.mail_changed(account_id, None);
    // catch up on what changed while we were listing (no new-mail alerts for this first pass)
    incremental_inner(ctx, api, &h, map, false, K_HISTORY, 0).await?;
    Ok(true)
}

/// After a resync (history expired): bring the labels of messages we already had up to date by
/// listing each label's message ids (`messages.list?labelIds=X`, 5 units per 500 ids) instead of
/// fetching every message again (20 units each).
async fn resync_labels(ctx: &Arc<AppCtx>, api: &Api, map: &LabelMap, generation: i64) -> anyhow::Result<()> {
    let account_id = api.account_id;
    ctx.events.status(account_id, "sync", "Checking folders\u{2026}", None);
    let labels = api.gmail_labels().await?;
    let mut by_msg: HashMap<String, Vec<String>> = HashMap::new();
    // a few labels at a time (big mailboxes: hundreds of pages in all)
    let mut lists = stream::iter(labels.into_iter().map(|l| label_members(api, l.id))).buffer_unordered(4);
    while let Some(r) = lists.next().await {
        let (label, ids) = r?;
        for id in ids {
            by_msg.entry(id).or_default().push(label.clone());
        }
    }
    drop(lists);
    let map2 = map.clone();
    let n = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let rows: Vec<(i64, String, String)> = {
                let mut st = tx.prepare("SELECT id, gmail_id, label_ids FROM messages WHERE account_id=?1 AND sync_gen=?2")?;
                st.query_map(params![account_id, generation], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?
            };
            let mut n = 0;
            for (id, gid, current) in rows {
                let mut want = by_msg.get(&gid).cloned().unwrap_or_default();
                let mut have: Vec<String> = current.split_whitespace().map(str::to_string).collect();
                want.sort();
                have.sort();
                if want != have {
                    store::apply_labels(&tx, id, &want, &map2)?;
                    n += 1;
                }
            }
            tx.commit()?;
            Ok(n)
        })
        .await?;
    log::info!("mail: resync updated the labels of {n} messages for account {account_id}");
    Ok(())
}

/// All message ids carrying a label.
async fn label_members(api: &Api, label: String) -> anyhow::Result<(String, Vec<String>)> {
    let mut ids: Vec<String> = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let page = api.gmail_list(None, &[label.as_str()], token.as_deref(), PAGE, true).await?;
        ids.extend(page.messages.into_iter().map(|m| m.id));
        token = page.next_page_token;
        if token.is_none() {
            return Ok((label, ids));
        }
    }
}

async fn incremental(ctx: &Arc<AppCtx>, api: &Api, start: &str, map: &LabelMap) -> anyhow::Result<()> {
    incremental_inner(ctx, api, start, map, true, K_HISTORY, 0).await
}

#[derive(Debug, Clone)]
enum LabelOp {
    Add(Vec<String>),
    Remove(Vec<String>),
}

/// Apply history since `start`; the new position is saved under `history_key` and new messages
/// get sync generation `generation` (a running full sync must not count them as stale).
async fn incremental_inner(
    ctx: &Arc<AppCtx>,
    api: &Api,
    start: &str,
    map: &LabelMap,
    notify: bool,
    history_key: &'static str,
    generation: i64,
) -> anyhow::Result<()> {
    let account_id = api.account_id;
    let mut token: Option<String> = None;
    let mut latest = start.to_string();
    let mut added: Vec<String> = Vec::new();
    let mut added_set: HashSet<String> = HashSet::new();
    let mut deleted: HashSet<String> = HashSet::new();
    let mut ops: Vec<(String, LabelOp)> = Vec::new();
    loop {
        let page = api.gmail_history(start, token.as_deref()).await?;
        if !page.history_id.is_empty() {
            latest = page.history_id.clone();
        }
        for h in &page.history {
            for a in &h.messages_added {
                if added_set.insert(a.message.id.clone()) {
                    added.push(a.message.id.clone());
                }
                deleted.remove(&a.message.id);
            }
            for d in &h.messages_deleted {
                deleted.insert(d.message.id.clone());
            }
            for l in &h.labels_added {
                ops.push((l.message.id.clone(), LabelOp::Add(l.label_ids.clone())));
            }
            for l in &h.labels_removed {
                ops.push((l.message.id.clone(), LabelOp::Remove(l.label_ids.clone())));
            }
        }
        token = page.next_page_token.clone();
        if token.is_none() {
            break;
        }
    }
    added.retain(|id| !deleted.contains(id));
    // which "added" ids we don't have yet
    let added2 = added.clone();
    let have: HashSet<String> = ctx
        .db
        .read(move |c| {
            let mut st = c.prepare_cached("SELECT 1 FROM messages WHERE account_id=?1 AND gmail_id=?2")?;
            let mut have = HashSet::new();
            for id in &added2 {
                if st.exists(params![account_id, id])? {
                    have.insert(id.clone());
                }
            }
            Ok(have)
        })
        .await?;
    let to_fetch: Vec<String> = added.iter().filter(|id| !have.contains(*id)).cloned().collect();
    let fetched: Vec<Message> = {
        let mut out = Vec::new();
        let mut s = stream::iter(to_fetch.into_iter().map(|id| fetch_meta(api, id))).buffer_unordered(CONCURRENCY);
        while let Some((id, r)) = s.next().await {
            match r {
                Ok(m) => out.push(m),
                Err(e) if api_status(&e) == Some(404) => log::debug!("mail: added message {id} already gone"),
                Err(e) => return Err(e),
            }
        }
        out
    };
    let fetched_ids: HashSet<String> = fetched.iter().map(|m| m.id.clone()).collect();
    let map2 = map.clone();

    let latest2 = latest.clone();
    let (new_inbox, changed) = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let mut changed = false;
            // deletions
            for gid in &deleted {
                let id: Option<i64> = tx
                    .query_row("SELECT id FROM messages WHERE account_id=?1 AND gmail_id=?2", params![account_id, gid], |r| r.get(0))
                    .optional()?;
                if let Some(id) = id {
                    store::delete_messages(&tx, &[id])?;
                    changed = true;
                }
            }
            // label changes, applied in order, for messages we already have
            let mut current: HashMap<String, (i64, Vec<String>)> = HashMap::new();
            for (gid, op) in &ops {
                if fetched_ids.contains(gid) || deleted.contains(gid) {
                    continue;
                }
                if !current.contains_key(gid) {
                    let row: Option<(i64, String)> = tx
                        .query_row(
                            "SELECT id, label_ids FROM messages WHERE account_id=?1 AND gmail_id=?2",
                            params![account_id, gid],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    let Some((id, l)) = row else { continue };
                    current.insert(gid.clone(), (id, l.split_whitespace().map(str::to_string).collect()));
                }
                let entry = current.get_mut(gid).unwrap();
                match op {
                    LabelOp::Add(ls) => {
                        for l in ls {
                            if !entry.1.contains(l) {
                                entry.1.push(l.clone());
                            }
                        }
                    }
                    LabelOp::Remove(ls) => entry.1.retain(|x| !ls.contains(x)),
                }
            }
            for (id, labels) in current.values() {
                store::apply_labels(&tx, *id, labels, &map2)?;
                changed = true;
            }
            // new messages
            let mut new_inbox: Vec<(i64, String, String, String)> = Vec::new();
            for m in &fetched {
                let meta: Meta = store::meta_from(m);
                let (id, inserted) = store::upsert_meta(&tx, account_id, &meta, &map2, generation)?;
                changed = true;
                let l = &meta.labels;
                if inserted
                    && l.iter().any(|x| x == "INBOX")
                    && l.iter().any(|x| x == "UNREAD")
                    && !l.iter().any(|x| x == "SENT" || x == "DRAFT" || x == "SPAM" || x == "TRASH")
                {
                    new_inbox.push((id, meta.from.display().to_string(), meta.subject.clone(), meta.snippet.chars().take(120).collect()));
                }
            }
            set_sync_state(&tx, account_id, history_key, Some(&latest2))?;
            if changed {
                store::recount(&tx, account_id)?;
            }
            tx.commit()?;
            Ok((new_inbox, changed))
        })
        .await?;
    if changed {
        ctx.events.mail_changed(account_id, None);
    }
    if notify && !new_inbox.is_empty() {
        let ids: Vec<i64> = new_inbox.iter().map(|n| n.0).collect();
        if let Err(e) = crate::rules::on_new_messages(ctx, account_id, &ids).await {
            log::warn!("rules on new mail: {}", crate::logging::err(&e));
        }
        // only what is still unread in the Inbox after rules ran gets an alert
        let ids2 = ids.clone();
        let still: HashSet<i64> = ctx
            .db
            .read(move |c| {
                let mut st = c.prepare(
                    "SELECT id FROM messages WHERE id=?1 AND is_read=0 AND location=0 AND (' ' || label_ids || ' ') LIKE '% INBOX %'",
                )?;
                let mut out = HashSet::new();
                for id in &ids2 {
                    if st.exists([id])? {
                        out.insert(*id);
                    }
                }
                Ok(out)
            })
            .await?;
        let msgs: Vec<serde_json::Value> = new_inbox
            .into_iter()
            .filter(|n| still.contains(&n.0))
            .map(|(id, from, subject, preview)| json!({ "id": id, "from": from, "subject": subject, "preview": preview }))
            .collect();
        if !msgs.is_empty() {
            ctx.events.emit("mail.newMail", json!({ "accountId": account_id, "messages": msgs }));
        }
    }
    Ok(())
}

// ---- pending local changes ------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifyOp {
    pub ids: Vec<String>,
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdsOp {
    pub ids: Vec<String>,
}

/// Push queued changes right away and wait for it (e.g. before deleting a label).
pub async fn push_now(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let lock = account_lock(api.account_id);
    let _g = lock.lock().await;
    push_pending_locked(ctx, api).await
}

/// Push queued local changes now (after a local action), outside the sync cycle.
pub fn push_soon(ctx: &Arc<AppCtx>, account_id: i64) {
    let ctx = ctx.clone();
    tokio::spawn(async move {
        let lock = account_lock(account_id);
        let _g = lock.lock().await;
        let api = match ctx.api(account_id).await {
            Ok(a) => a,
            Err(e) => {
                log::warn!("mail push: {}", crate::logging::err(&e));
                return;
            }
        };
        if let Err(e) = push_pending_locked(&ctx, &api).await {
            log::warn!("mail push for account {account_id}: {}", crate::logging::err(&e));
        }
    });
}

async fn push_pending_locked(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let account_id = api.account_id;
    let dropped = crate::sync::pending::drain(&ctx.db, account_id, "mail", |op| {
        let api = api.clone();
        let ctx = ctx.clone();
        async move {
            let r = push_one(&api, &op).await;
            if let Err(e) = &r {
                let permanent = crate::google::client::is_permanent(e) || op.attempts + 1 >= crate::sync::pending::MAX_ATTEMPTS;
                if permanent {
                    // give up: bring these messages back to the server's state
                    let ids: Vec<String> = op.payload["ids"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                        .unwrap_or_default();
                    if let Err(e2) = refresh_messages(&ctx, &api, &ids).await {
                        log::warn!("mail: refresh after failed op: {}", crate::logging::err(&e2));
                    }
                }
            }
            r
        }
    })
    .await?;
    if dropped > 0 {
        ctx.events.status(account_id, "error", "Some changes could not be saved to the server and were undone.", None);
    }
    Ok(())
}

async fn push_one(api: &Api, op: &crate::sync::pending::PendingOp) -> anyhow::Result<()> {
    match op.kind.as_str() {
        "modify" => {
            let p: ModifyOp = op.payload()?;
            if p.ids.len() == 1 {
                match api.gmail_modify(&p.ids[0], &p.add, &p.remove).await {
                    Err(e) if api_status(&e) == Some(404) => Ok(()),
                    r => r.map(|_| ()),
                }
            } else {
                api.gmail_batch_modify(&p.ids, &p.add, &p.remove).await
            }
        }
        "trash" | "untrash" | "delete" => {
            let p: IdsOp = op.payload()?;
            for id in &p.ids {
                let r = match op.kind.as_str() {
                    "trash" => api.gmail_trash(id).await.map(|_| ()),
                    "untrash" => api.gmail_untrash(id).await.map(|_| ()),
                    _ => api.gmail_delete(id).await,
                };
                match r {
                    Ok(()) => {}
                    Err(e) if api_status(&e) == Some(404) => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        }
        other => {
            log::warn!("mail: unknown pending op {other}");
            Ok(())
        }
    }
}

/// Re-read labels of some messages from the server (after a failed push).
async fn refresh_messages(ctx: &Arc<AppCtx>, api: &Api, gmail_ids: &[String]) -> anyhow::Result<()> {
    let account_id = api.account_id;
    let map = ctx.db.read(move |c| LabelMap::load(c, account_id)).await?;
    let mut got = Vec::new();
    for id in gmail_ids {
        match api.gmail_get(id, "metadata", store::METADATA_HEADERS).await {
            Ok(m) => got.push((id.clone(), Some(m))),
            Err(e) if api_status(&e) == Some(404) => got.push((id.clone(), None)),
            Err(e) => return Err(e),
        }
    }
    ctx.db
        .write(move |c| {
            let tx = c.transaction()?;
            for (gid, m) in &got {
                match m {
                    Some(m) => {
                        store::upsert_meta(&tx, account_id, &store::meta_from(m), &map, 0)?;
                    }
                    None => {
                        if let Ok(id) =
                            tx.query_row("SELECT id FROM messages WHERE account_id=?1 AND gmail_id=?2", params![account_id, gid], |r| {
                                r.get::<_, i64>(0)
                            })
                        {
                            store::delete_messages(&tx, &[id])?;
                        }
                    }
                }
            }
            store::recount(&tx, account_id)?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    ctx.events.mail_changed(account_id, None);
    Ok(())
}

// ---- background body prefetch ---------------------------------------------------------------

fn spawn_prefetch(ctx: Arc<AppCtx>, account_id: i64) {
    static RUNNING: OnceLock<Mutex<HashSet<i64>>> = OnceLock::new();
    let running = RUNNING.get_or_init(Default::default);
    if !running.lock().insert(account_id) {
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = crate::google::limits::background(prefetch(&ctx, account_id)).await {
            log::debug!("mail prefetch for {account_id}: {}", crate::logging::err(&e));
        }
        RUNNING.get().unwrap().lock().remove(&account_id);
    });
}

/// What the background download covers: bodies from the last `mail.prefetchDays` days, or of all
/// mail with `mail.downloadAllBodies`; with `mail.downloadAttachments` also the attachments of
/// those messages. Newest first, up to `BATCH` bodies and `ATT_BATCH` attachments per sync cycle,
/// at background priority (anything the user does goes first); it resumes after a restart since
/// each message / attachment records whether it is already here.
const BATCH: i64 = 400;
const ATT_BATCH: i64 = 60;

pub(super) struct Scope {
    pub since: i64,
    pub attachments: bool,
}

pub(super) fn download_scope(c: &rusqlite::Connection) -> anyhow::Result<Option<Scope>> {
    let all: bool = get_setting(c, "mail.downloadAllBodies")?.unwrap_or(false);
    let days: i64 = get_setting(c, "mail.prefetchDays")?.unwrap_or(30);
    if !all && days <= 0 {
        return Ok(None);
    }
    let since = if all { i64::MIN } else { now_ms() - days * 86_400_000 };
    Ok(Some(Scope { since, attachments: get_setting(c, "mail.downloadAttachments")?.unwrap_or(false) }))
}

/// Bodies and attachments still to download in the current scope (status bar progress).
fn left_to_download(c: &rusqlite::Connection, account_id: i64, s: &Scope) -> anyhow::Result<(i64, i64)> {
    let bodies: i64 = c.query_row(
        "SELECT COUNT(*) FROM messages WHERE account_id=?1 AND body_state=0 AND location=0 AND internal_date>=?2",
        params![account_id, s.since],
        |r| r.get(0),
    )?;
    let atts: i64 = if s.attachments {
        c.query_row(
            "SELECT COUNT(*) FROM attachments a JOIN messages m ON m.id=a.message_id              WHERE m.account_id=?1 AND m.location=0 AND m.internal_date>=?2 AND a.cached_path IS NULL",
            params![account_id, s.since],
            |r| r.get(0),
        )?
    } else {
        0
    };
    Ok((bodies, atts))
}

pub(super) async fn prefetch(ctx: &Arc<AppCtx>, account_id: i64) -> anyhow::Result<()> {
    let plan = ctx
        .db
        .read(move |c| {
            let Some(s) = download_scope(c)? else { return Ok(None) };
            let mut st = c.prepare(
                "SELECT id FROM messages WHERE account_id=?1 AND body_state=0 AND location=0 AND internal_date>=?2                  ORDER BY ((' ' || label_ids || ' ') LIKE '% INBOX %') DESC, internal_date DESC LIMIT ?3",
            )?;
            let ids = st.query_map(params![account_id, s.since, BATCH], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?;
            let atts: Vec<i64> = if s.attachments {
                let mut st = c.prepare(
                    "SELECT a.id FROM attachments a JOIN messages m ON m.id=a.message_id                      WHERE m.account_id=?1 AND m.location=0 AND m.internal_date>=?2 AND a.cached_path IS NULL                      ORDER BY m.internal_date DESC LIMIT ?3",
                )?;
                st.query_map(params![account_id, s.since, ATT_BATCH], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?
            } else {
                vec![]
            };
            Ok(Some((s, ids, atts)))
        })
        .await?;
    let Some((scope, ids, atts)) = plan else { return Ok(()) };
    let mut n = 0usize;
    let mut offline = false;
    let mut s = stream::iter(ids.into_iter().map(|id| {
        let ctx = ctx.clone();
        async move { (id, super::body::ensure_body(&ctx, id).await) }
    }))
    .buffer_unordered(3);
    while let Some((id, r)) = s.next().await {
        match r {
            Ok(()) => n += 1,
            Err(e) => {
                if matches!(e.downcast_ref::<crate::google::client::ApiError>(), Some(crate::google::client::ApiError::Network(_))) {
                    offline = true;
                    break;
                }
                log::debug!("prefetch {id}: {}", crate::logging::err(&e));
            }
        }
    }
    drop(s);
    // attachments (Options › Mail › download attachments): cached on disk like an opened one
    if !offline {
        let mut s = stream::iter(atts.into_iter().map(|id| {
            let ctx = ctx.clone();
            async move { (id, super::body::attachment_bytes(&ctx, id).await.map(|_| ())) }
        }))
        .buffer_unordered(2);
        while let Some((id, r)) = s.next().await {
            if let Err(e) = r {
                if matches!(e.downcast_ref::<crate::google::client::ApiError>(), Some(crate::google::client::ApiError::Network(_))) {
                    break;
                }
                log::debug!("prefetch attachment {id}: {}", crate::logging::err(&e));
            }
        }
    }
    if n > 0 {
        ctx.events.mail_changed(account_id, None);
    }
    let (bodies, attachments) = ctx.db.read(move |c| left_to_download(c, account_id, &scope)).await?;
    ctx.events.emit("mail.download", json!({ "accountId": account_id, "bodies": bodies, "attachments": attachments }));
    Ok(())
}

// ---- demo: new mail arrives every few minutes ------------------------------------------------

async fn start_demo_delivery(ctx: &Arc<AppCtx>, account_id: i64) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.get().is_some() || ctx.app.is_none() {
        return; // never in tests
    }
    let is_demo = crate::accounts::get(&ctx.db, account_id).await.map(|a| a.provider == "demo").unwrap_or(false);
    if !is_demo || STARTED.set(()).is_err() {
        return;
    }
    let ctx = ctx.clone();
    tokio::spawn(async move {
        loop {
            let wait = 180 + rand::random::<u64>() % 120;
            tokio::time::sleep(Duration::from_secs(wait)).await;
            let enabled: bool = ctx.db.read(|c| get_setting(c, "demo.autoDeliver")).await.ok().flatten().unwrap_or(true);
            if !enabled {
                continue;
            }
            if let Ok(accts) = crate::accounts::list(&ctx.db).await
                && let Some(a) = accts.iter().find(|a| a.provider == "demo" && a.enabled)
                && deliver_demo(&a.email).is_some()
            {
                ctx.sync_now(Some(a.id));
            }
        }
    });
}

/// Put a random new message into a demo mailbox on the fake server.
pub fn deliver_demo(email: &str) -> Option<String> {
    let server = crate::fake_google::server()?;
    let email = email.to_lowercase();
    Some(server.with(|st| crate::fake_google::gmail::deliver_random(&mut st.user(&email).gmail, crate::fake_google::seed::now_ms())))
}

/// Tests: labels → folders, returning the label map.
#[cfg(test)]
pub(crate) async fn labels_for_test(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<LabelMap> {
    sync_labels(ctx, api).await
}

/// Tests: labels + one slice of the full sync.
#[cfg(test)]
pub(crate) async fn full_sync_slice_for_test(ctx: &Arc<AppCtx>, api: &Api, slice: Duration) -> anyhow::Result<bool> {
    let map = sync_labels(ctx, api).await?;
    full_sync_sliced(ctx, api, &map, slice).await
}

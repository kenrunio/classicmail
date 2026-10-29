//! Background sync. One worker per enabled account; each cycle pushes pending local changes and
//! pulls remote changes for every domain that is due. F9 / `sync.now` triggers an immediate cycle.

pub mod pending;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::{accounts, ctx::AppCtx};

/// Domains and how often they pull (seconds). Mail's interval comes from settings.
const DOMAINS: &[(&str, u64)] = &[("mail", 60), ("calendar", 300), ("contacts", 900), ("tasks", 300)];

#[derive(Clone)]
pub struct SyncHandle {
    inner: Arc<Inner>,
}

struct Inner {
    /// account id → wake-up signal
    workers: Mutex<HashMap<i64, Arc<Worker>>>,
}

struct Worker {
    wake: Notify,
    /// set by `trigger` so the next cycle runs every domain regardless of interval
    force: Mutex<bool>,
    stop: Mutex<bool>,
}

impl SyncHandle {
    pub fn trigger(&self, account_id: Option<i64>) {
        for (id, w) in self.inner.workers.lock().iter() {
            if account_id.is_none() || account_id == Some(*id) {
                *w.force.lock() = true;
                w.wake.notify_one();
            }
        }
    }

    /// Stop an account's worker now (it is being removed; `refresh_workers` won't start it again).
    pub fn stop(&self, account_id: i64) {
        if let Some(w) = self.inner.workers.lock().remove(&account_id) {
            *w.stop.lock() = true;
            w.wake.notify_one();
        }
    }

    /// Re-check timers without forcing a sync (e.g. a later Outbox send time was queued).
    pub fn wake(&self, account_id: i64) {
        if let Some(w) = self.inner.workers.lock().get(&account_id) {
            w.wake.notify_one();
        }
    }
}

/// Start workers for every enabled account; call `refresh_workers` after accounts change.
pub fn start(ctx: Arc<AppCtx>) -> SyncHandle {
    let h = SyncHandle { inner: Arc::new(Inner { workers: Mutex::new(HashMap::new()) }) };
    let _ = ctx.sync.set(h.clone());
    let c2 = ctx.clone();
    tokio::spawn(async move {
        if let Err(e) = refresh_workers(&c2).await {
            log::error!("sync start: {}", crate::logging::err(&e));
        }
    });
    h
}

pub async fn refresh_workers(ctx: &Arc<AppCtx>) -> anyhow::Result<()> {
    let Some(handle) = ctx.sync.get() else { return Ok(()) };
    let accts = accounts::list(&ctx.db).await?;
    let mut workers = handle.inner.workers.lock();
    // stop removed / disabled
    let wanted: Vec<i64> = accts.iter().filter(|a| a.enabled).map(|a| a.id).collect();
    workers.retain(|id, w| {
        let keep = wanted.contains(id);
        if !keep {
            *w.stop.lock() = true;
            w.wake.notify_one();
        }
        keep
    });
    for id in wanted {
        if workers.contains_key(&id) {
            continue;
        }
        let w = Arc::new(Worker { wake: Notify::new(), force: Mutex::new(true), stop: Mutex::new(false) });
        workers.insert(id, w.clone());
        let ctx = ctx.clone();
        tokio::spawn(async move { run_worker(ctx, id, w).await });
    }
    Ok(())
}

async fn mail_interval(ctx: &AppCtx) -> u64 {
    ctx.db.read(|c| crate::db::util::get_setting::<u64>(c, "mail.pollSeconds")).await.ok().flatten().unwrap_or(60).max(15)
}

/// When the earliest waiting Outbox message is due (epoch ms).
async fn next_outbox_due(ctx: &AppCtx, account_id: i64) -> Option<i64> {
    ctx.db
        .read(move |c| {
            Ok(c.query_row("SELECT MIN(send_at) FROM outbox WHERE account_id=?1 AND status IN ('queued','failed')", [account_id], |r| {
                r.get::<_, Option<i64>>(0)
            })?)
        })
        .await
        .ok()
        .flatten()
}

/// Send what is due in the Outbox without a full mail sync (paced mail merges, Delay Delivery).
async fn run_outbox(ctx: &Arc<AppCtx>, account_id: i64) -> anyhow::Result<()> {
    let api = ctx.api(account_id).await?;
    crate::mail::compose::process_outbox(ctx, &api).await
}

async fn run_worker(ctx: Arc<AppCtx>, account_id: i64, w: Arc<Worker>) {
    let mut last: HashMap<&'static str, Instant> = HashMap::new();
    // no Outbox-only run before this (after a failure, so an offline Outbox doesn't spin)
    let mut outbox_hold = Instant::now();
    loop {
        if *w.stop.lock() {
            return;
        }
        let force = std::mem::take(&mut *w.force.lock());
        let mail_secs = mail_interval(&ctx).await;
        let mut ran_any = false;
        // the mail sync starts by sending the Outbox: no second Outbox pass in the same cycle
        let mut ran_mail = false;
        let mut errors = Vec::new();
        for (domain, secs) in DOMAINS {
            let secs = if *domain == "mail" { mail_secs } else { *secs };
            let due = force || last.get(domain).map(|t| t.elapsed() >= Duration::from_secs(secs)).unwrap_or(true);
            if !due {
                continue;
            }
            ran_any = true;
            ran_mail |= *domain == "mail";
            last.insert(domain, Instant::now());
            if let Err(e) = run_domain(&ctx, account_id, domain).await {
                log::warn!("sync {domain} for account {account_id}: {}", crate::logging::err(&e));
                errors.push(format!("{domain}: {}", short(&e)));
            }
        }
        // rules kept as Gmail filters: catch up on what failed or changed while offline
        if ran_mail && let Err(e) = crate::rules::gmail_filters::reconcile(&ctx, account_id, false).await {
            log::warn!("rules: Gmail filters for account {account_id}: {}", crate::logging::err(&e));
        }
        // between mail syncs: send Outbox messages whose time has come
        if !ran_mail
            && Instant::now() >= outbox_hold
            && next_outbox_due(&ctx, account_id).await.is_some_and(|t| t <= crate::db::util::now_ms())
        {
            ran_any = true;
            if let Err(e) = run_outbox(&ctx, account_id).await {
                log::warn!("outbox for account {account_id}: {}", crate::logging::err(&e));
                errors.push(format!("mail: {}", short(&e)));
                outbox_hold = Instant::now() + Duration::from_secs(60);
            }
        }
        if ran_any {
            if errors.is_empty() {
                ctx.events.status(account_id, "idle", "All folders are up to date.", None);
            } else {
                ctx.events.status(account_id, "error", &format!("Send/Receive error — {}", errors.join("; ")), None);
            }
        }
        // sleep until the next domain is due, or until woken
        let next = DOMAINS
            .iter()
            .map(|(d, s)| {
                let s = if *d == "mail" { mail_secs } else { *s };
                let el = last.get(d).map(|t| t.elapsed().as_secs()).unwrap_or(s);
                s.saturating_sub(el)
            })
            .min()
            .unwrap_or(30);
        let next = match next_outbox_due(&ctx, account_id).await {
            Some(t) => {
                let wait_ms = (t - crate::db::util::now_ms()).max(0) as u64;
                let hold = outbox_hold.saturating_duration_since(Instant::now()).as_millis() as u64;
                next.min(wait_ms.max(hold).div_ceil(1000))
            }
            None => next,
        }
        .max(1);
        tokio::select! {
            _ = w.wake.notified() => {},
            _ = tokio::time::sleep(Duration::from_secs(next)) => {},
        }
    }
}

fn short(e: &anyhow::Error) -> String {
    let s = e.to_string();
    if s.chars().count() > 140 { format!("{}…", s.chars().take(140).collect::<String>()) } else { s }
}

async fn run_domain(ctx: &Arc<AppCtx>, account_id: i64, domain: &str) -> anyhow::Result<()> {
    let api = ctx.api(account_id).await?;
    // sync is background work: it waits for Google quota, user actions don't (google::limits)
    crate::google::limits::background(async {
        match domain {
            "mail" => crate::mail::sync::sync_account(ctx, &api).await,
            "calendar" => crate::calendar::sync::sync_account(ctx, &api).await,
            "contacts" => crate::contacts::sync::sync_account(ctx, &api).await,
            "tasks" => crate::tasks::sync::sync_account(ctx, &api).await,
            _ => Ok(()),
        }
    })
    .await
}

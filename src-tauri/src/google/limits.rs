//! Client-side pacing so one account stays inside Google's per-user quotas instead of living on
//! 429 / 403 rateLimitExceeded answers.
//!
//! Google meters per user and per minute (sliding window). Projects created after 1 May 2026 get:
//! * Gmail: 6,000 quota units / minute / user; `messages.get` costs 20 units, `messages.list` 5,
//!   `history.list` 2, `messages.send` 100, `batchModify` 50 … (see `gmail_cost`).
//! * Calendar: 600 requests / minute / user.
//! * People and Tasks: not published per user; we assume conservative numbers.
//!
//! Each API family gets a token bucket. Background work (sync, prefetch) waits for tokens;
//! interactive calls (the user opened a message, saved a contact) may borrow up to one bucket's
//! worth of debt so they are never stuck behind a long initial sync.
//! Run background work inside [`background`].

use std::{future::Future, time::Duration};

use parking_lot::Mutex;
use reqwest::Method;
use tokio::time::Instant;

tokio::task_local! {
    static BACKGROUND: bool;
}

/// Run `f` as background work: its Google calls wait for quota instead of borrowing.
pub async fn background<F: Future>(f: F) -> F::Output {
    BACKGROUND.scope(true, f).await
}

pub fn is_background() -> bool {
    BACKGROUND.try_with(|b| *b).unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Gmail,
    Calendar,
    People,
    Tasks,
}

impl Family {
    fn idx(self) -> usize {
        self as usize
    }
}

/// Per-minute budgets for one user.
#[derive(Debug, Clone, Copy)]
pub struct Quotas {
    pub gmail_units_per_minute: f64,
    pub calendar_per_minute: f64,
    pub people_per_minute: f64,
    pub tasks_per_minute: f64,
}

impl Default for Quotas {
    fn default() -> Self {
        Quotas { gmail_units_per_minute: 6000.0, calendar_per_minute: 600.0, people_per_minute: 90.0, tasks_per_minute: 300.0 }
    }
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    cap: f64,
    rate: f64, // per second
    last: Instant,
}

impl Bucket {
    /// Worst case inside any minute: full bucket + one more bucket of interactive debt + a minute
    /// of refill = 0.1 + 0.1 + 0.75 = 95 % of the quota.
    fn for_minute_quota(per_minute: f64) -> Bucket {
        let cap = (per_minute * 0.1).max(1.0);
        Bucket { tokens: cap, cap, rate: per_minute * 0.75 / 60.0, last: Instant::now() }
    }

    fn refill(&mut self, now: Instant) {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + dt * self.rate).min(self.cap);
        self.last = now;
    }

    /// Take `cost` now, or say how long to wait.
    fn try_take(&mut self, cost: f64, background: bool, now: Instant) -> Option<Duration> {
        self.refill(now);
        let floor = if background { 0.0 } else { -self.cap };
        // a single call dearer than the whole bucket goes through once the bucket is full
        let cost_eff = cost.min(self.cap);
        if self.tokens - cost_eff >= floor {
            self.tokens -= cost;
            return None;
        }
        let need = floor + cost_eff - self.tokens;
        Some(Duration::from_secs_f64((need / self.rate).clamp(0.01, 60.0)))
    }
}

pub struct Limiter {
    buckets: Option<Mutex<[Bucket; 4]>>,
}

impl Limiter {
    /// No pacing (demo accounts on the fake server, tests).
    pub fn unlimited() -> Self {
        Limiter { buckets: None }
    }

    pub fn new(q: Quotas) -> Self {
        Limiter {
            buckets: Some(Mutex::new([
                Bucket::for_minute_quota(q.gmail_units_per_minute),
                Bucket::for_minute_quota(q.calendar_per_minute),
                Bucket::for_minute_quota(q.people_per_minute),
                Bucket::for_minute_quota(q.tasks_per_minute),
            ])),
        }
    }

    pub async fn acquire(&self, fam: Family, cost: f64) {
        let Some(b) = &self.buckets else { return };
        let bg = is_background();
        loop {
            let wait = b.lock()[fam.idx()].try_take(cost, bg, Instant::now());
            match wait {
                None => return,
                Some(d) => tokio::time::sleep(d).await,
            }
        }
    }
}

/// Gmail quota units of one call. `path` is the part after `/users/me/`.
/// https://developers.google.com/workspace/gmail/api/reference/quota
pub fn gmail_cost(method: &Method, path: &str) -> f64 {
    let path = path.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let m = method.as_str();
    let c = match (m, segs.as_slice()) {
        (_, ["profile"]) => 1,
        ("GET", ["labels"]) | ("GET", ["labels", _]) => 1,
        (_, ["labels", ..]) => 5,
        ("GET", ["messages"]) => 5,
        ("POST", ["messages", "send"]) => 100,
        ("POST", ["messages", "batchModify"]) | ("POST", ["messages", "batchDelete"]) => 50,
        ("POST", ["messages", "import"]) => 25,
        ("GET", ["messages", _, "attachments", _]) => 20,
        ("POST", ["messages", _, "modify"]) => 5,
        ("POST", ["messages", _, "trash"]) => 20,
        ("POST", ["messages", _, "untrash"]) => 5,
        ("DELETE", ["messages", _]) => 10,
        ("GET", ["messages", _]) => 20,
        ("GET", ["history"]) => 2,
        ("GET", ["drafts"]) => 5,
        ("GET", ["drafts", _]) => 20,
        ("POST", ["drafts", "send"]) => 100,
        ("POST", ["drafts"]) => 10,
        ("PUT", ["drafts", _]) => 15,
        ("DELETE", ["drafts", _]) => 10,
        ("GET", ["threads", _]) => 40,
        ("GET", ["threads"]) => 10,
        (_, ["settings", ..]) => 1,
        _ => 10,
    };
    c as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_costs_follow_the_quota_table() {
        assert_eq!(gmail_cost(&Method::GET, "messages/18a3c?format=metadata"), 20.0);
        assert_eq!(gmail_cost(&Method::GET, "messages"), 5.0);
        assert_eq!(gmail_cost(&Method::GET, "history"), 2.0);
        assert_eq!(gmail_cost(&Method::POST, "messages/send"), 100.0);
        assert_eq!(gmail_cost(&Method::POST, "messages/batchModify"), 50.0);
        assert_eq!(gmail_cost(&Method::GET, "messages/1/attachments/ANGj"), 20.0);
        assert_eq!(gmail_cost(&Method::GET, "labels"), 1.0);
        assert_eq!(gmail_cost(&Method::GET, "profile"), 1.0);
        assert_eq!(gmail_cost(&Method::PUT, "drafts/r123"), 15.0);
    }

    #[tokio::test(start_paused = true)]
    async fn background_work_is_paced_to_the_minute_quota() {
        let l = Limiter::new(Quotas { gmail_units_per_minute: 6000.0, ..Default::default() });
        let t0 = Instant::now();
        // 60 metadata gets = 1200 units: 600 from the bucket, 600 at 75 units/s = 8 s
        background(async {
            for _ in 0..60 {
                l.acquire(Family::Gmail, 20.0).await;
            }
        })
        .await;
        let took = t0.elapsed().as_secs_f64();
        assert!((7.5..9.5).contains(&took), "took {took}");
        // a minute of background calls never exceeds 95 % of the quota
        let t1 = Instant::now();
        let mut units = 0.0;
        background(async {
            while t1.elapsed() < Duration::from_secs(60) {
                l.acquire(Family::Gmail, 20.0).await;
                units += 20.0;
            }
        })
        .await;
        assert!(units <= 6000.0 * 0.95, "{units} units in a minute");
    }

    #[tokio::test(start_paused = true)]
    async fn interactive_calls_borrow_instead_of_waiting() {
        let l = Limiter::new(Quotas::default());
        background(async {
            for _ in 0..30 {
                l.acquire(Family::Gmail, 20.0).await; // drain the bucket
            }
        })
        .await;
        let t0 = Instant::now();
        l.acquire(Family::Gmail, 20.0).await; // user opens a message
        assert!(t0.elapsed() < Duration::from_millis(5));
        // … but background work now waits until the debt is repaid
        let t1 = Instant::now();
        background(l.acquire(Family::Gmail, 20.0)).await;
        assert!(t1.elapsed() >= Duration::from_millis(500));
    }

    #[tokio::test]
    async fn unlimited_never_waits() {
        let l = Limiter::unlimited();
        background(async {
            for _ in 0..10_000 {
                l.acquire(Family::Gmail, 100.0).await;
            }
        })
        .await;
    }
}

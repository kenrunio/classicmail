//! Queue of local changes waiting to be pushed to Google (see design §3 "Local-first writes").
//!
//! Domains enqueue inside the same transaction that applies the local change, then each sync cycle
//! calls `drain(domain, |op| push(op))`.

use rusqlite::{Connection, params};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::db::{Db, util::now_ms};

pub const MAX_ATTEMPTS: i64 = 5;

#[derive(Debug, Clone)]
pub struct PendingOp {
    pub id: i64,
    pub account_id: i64,
    pub domain: String,
    pub kind: String,
    pub payload: Value,
    pub attempts: i64,
}

impl PendingOp {
    pub fn payload<T: DeserializeOwned>(&self) -> anyhow::Result<T> {
        Ok(serde_json::from_value(self.payload.clone())?)
    }
}

/// Enqueue from inside a write transaction.
pub fn enqueue<T: Serialize>(c: &Connection, account_id: i64, domain: &str, kind: &str, payload: &T) -> anyhow::Result<i64> {
    c.execute(
        "INSERT INTO pending_ops(account_id, domain, kind, payload, created_at) VALUES (?1,?2,?3,?4,?5)",
        params![account_id, domain, kind, serde_json::to_string(payload)?, now_ms()],
    )?;
    Ok(c.last_insert_rowid())
}

pub async fn due(db: &Db, account_id: i64, domain: &str) -> anyhow::Result<Vec<PendingOp>> {
    let domain = domain.to_string();
    db.read(move |c| {
        let mut st = c.prepare(
            "SELECT id, account_id, domain, kind, payload, attempts FROM pending_ops \
             WHERE account_id=?1 AND domain=?2 AND next_try_at<=?3 ORDER BY id LIMIT 200",
        )?;
        let rows = st
            .query_map(params![account_id, domain, now_ms()], |r| {
                Ok(PendingOp {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    domain: r.get(2)?,
                    kind: r.get(3)?,
                    payload: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(Value::Null),
                    attempts: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

pub async fn count(db: &Db, account_id: i64, domain: &str) -> anyhow::Result<i64> {
    let domain = domain.to_string();
    db.read(move |c| {
        Ok(c.query_row("SELECT COUNT(*) FROM pending_ops WHERE account_id=?1 AND domain=?2", params![account_id, domain], |r| r.get(0))?)
    })
    .await
}

pub async fn done(db: &Db, id: i64) -> anyhow::Result<()> {
    db.write(move |c| {
        c.execute("DELETE FROM pending_ops WHERE id=?1", [id])?;
        Ok(())
    })
    .await
}

/// Record a failure; returns true if the op was given up on (and deleted).
pub async fn failed(db: &Db, op: &PendingOp, err: &anyhow::Error) -> anyhow::Result<bool> {
    let id = op.id;
    let attempts = op.attempts + 1;
    let msg = format!("{err:#}");
    // permanent client errors (400 / 403 other than rate limits / 404 / 410) aren't worth retrying
    let permanent = crate::google::client::is_permanent(err);
    let give_up = permanent || attempts >= MAX_ATTEMPTS;
    db.write(move |c| {
        if give_up {
            c.execute("DELETE FROM pending_ops WHERE id=?1", [id])?;
        } else {
            let delay = 30_000i64 * (1 << attempts.min(6));
            c.execute(
                "UPDATE pending_ops SET attempts=?1, last_error=?2, next_try_at=?3 WHERE id=?4",
                params![attempts, msg, now_ms() + delay, id],
            )?;
        }
        Ok(())
    })
    .await?;
    Ok(give_up)
}

/// Push every due op of `domain` with `push`; stops at the first network error (offline).
/// Being offline doesn't count as a failed attempt: the op is simply tried again next cycle.
/// Returns how many ops were given up on, so the caller can resync the affected data.
pub async fn drain<F, Fut>(db: &Db, account_id: i64, domain: &str, mut push: F) -> anyhow::Result<usize>
where
    F: FnMut(PendingOp) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let mut dropped = 0;
    for op in due(db, account_id, domain).await? {
        let id = op.id;
        let op2 = op.clone();
        match push(op).await {
            Ok(()) => done(db, id).await?,
            Err(e) => {
                let offline =
                    matches!(e.downcast_ref::<crate::google::client::ApiError>(), Some(crate::google::client::ApiError::Network(_)));
                log::warn!("pending op {} {}/{} failed: {}", id, op2.domain, op2.kind, crate::logging::err(&e));
                if offline {
                    break;
                }
                if failed(db, &op2, &e).await? {
                    dropped += 1;
                }
            }
        }
    }
    Ok(dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::client::ApiError;

    async fn attempts(db: &Db, id: i64) -> Option<i64> {
        db.read(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT attempts FROM pending_ops WHERE id=?1", [id], |r| r.get(0)).optional()?)
        })
        .await
        .unwrap()
    }

    /// Hours offline never use up an op's attempts; a real error still does.
    #[tokio::test]
    async fn offline_is_not_an_attempt() {
        let (db, _g) = Db::open_temp().unwrap();
        let acct = crate::accounts::upsert(&db, "off@duvz.demo", "Off", "demo").await.unwrap().id;
        let id = db.write(move |c| enqueue(c, acct, "calendar", "create", &serde_json::json!({ "eventId": 1 }))).await.unwrap();
        for _ in 0..(MAX_ATTEMPTS * 2) {
            let dropped =
                drain(&db, acct, "calendar", |_| async { Err::<(), _>(ApiError::Network("offline".into()).into()) }).await.unwrap();
            assert_eq!(dropped, 0);
        }
        assert_eq!(attempts(&db, id).await, Some(0), "kept, no attempt used");
        let server: fn() -> anyhow::Error = || ApiError::Http { status: 500, message: "backend".into(), reason: None }.into();
        drain(&db, acct, "calendar", |_| async move { Err::<(), _>(server()) }).await.unwrap();
        assert_eq!(attempts(&db, id).await, Some(1), "a server error counts");
    }
}

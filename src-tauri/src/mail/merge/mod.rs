//! Mail merge: one personalised message per row of an Excel / CSV list, written in the normal
//! compose window with `{Column}` placeholders (docs/superpowers/specs/2026-10-12-mail-merge-design.md).
//!
//! rpc methods:
//! * `mail.merge.load { path }` → `{ fileName, headers, emailColumn, rowCount }`
//! * `mail.merge.check { path, compose }` → `Preflight`
//! * `mail.merge.run { path, compose, mode: "test" | "drafts" | "send" }` → `{ created, testRow?, testEmail? }`
//!
//! The list is re-read from `path` on every call (the file is the source of truth). Sends go
//! through the Outbox, spaced `SEND_GAP_MS` apart; drafts through `compose::drafts::save`.

pub mod fill;
pub mod list;

use std::{path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::{
    ctx::AppCtx,
    db::util::now_ms,
    mail::compose::{drafts, model::Compose, outbox, store},
    rpc::Registry,
};

/// Time between two merged messages leaving the Outbox (≈10 a minute).
pub const SEND_GAP_MS: i64 = 6_000;

pub fn register(r: &mut Registry) {
    r.add("mail.merge.load", load);
    r.add("mail.merge.check", check);
    r.add("mail.merge.run", run);
}

#[derive(Deserialize)]
struct PathParams {
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    file_name: String,
    headers: Vec<String>,
    email_column: usize,
    row_count: usize,
}

async fn read_list(path: String) -> anyhow::Result<list::MergeList> {
    tokio::task::spawn_blocking(move || list::load(&PathBuf::from(path))).await?
}

async fn load(_ctx: Arc<AppCtx>, p: PathParams) -> anyhow::Result<Summary> {
    let l = read_list(p.path).await?;
    Ok(Summary { file_name: l.file_name, row_count: l.rows.len(), email_column: l.email_column, headers: l.headers })
}

#[derive(Deserialize)]
struct CheckParams {
    path: String,
    compose: Compose,
}

async fn check(_ctx: Arc<AppCtx>, p: CheckParams) -> anyhow::Result<fill::Preflight> {
    let l = read_list(p.path).await?;
    Ok(fill::plan(&l, &p.compose).1)
}

#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum RunMode {
    Test,
    Drafts,
    Send,
}

#[derive(Deserialize)]
struct RunParams {
    path: String,
    compose: Compose,
    mode: RunMode,
}

#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    pub created: usize,
    /// test: the list row used and its address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_row: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_email: Option<String>,
}

async fn run(ctx: Arc<AppCtx>, p: RunParams) -> anyhow::Result<RunResult> {
    let l = read_list(p.path).await?;
    run_list(&ctx, &l, p.compose, p.mode).await
}

pub async fn run_list(ctx: &Arc<AppCtx>, l: &list::MergeList, template: Compose, mode: RunMode) -> anyhow::Result<RunResult> {
    let (targets, pre) = fill::plan(l, &template);
    if !pre.unknown_fields.is_empty() {
        return Err(crate::user_err!(
            "merge_fields",
            "These fields don't match any column in {}: {}",
            l.file_name,
            pre.unknown_fields.join(", ")
        ));
    }
    if targets.is_empty() {
        return Err(crate::user_err!("merge_empty", "{} has no rows with a valid e-mail address.", l.file_name));
    }
    let cols = fill::columns(l);
    match mode {
        RunMode::Test => {
            let (row, addr) = &targets[rand::random_range(0..targets.len())];
            let account_id = template.account_id;
            let from = template.from.clone();
            let me = ctx.db.read(move |c| store::identity(c, account_id, from.as_deref())).await?;
            let mut m = fill::message_for(&template, l, &cols, *row, me);
            m.subject = format!("[TEST] {}", m.subject);
            m.defer_until = None;
            outbox::queue_many(ctx, vec![(m, 0)]).await?;
            Ok(RunResult { created: 1, test_row: Some(l.row_numbers[*row]), test_email: Some(addr.email.clone()) })
        }
        RunMode::Send => {
            let start = template.defer_until.filter(|t| *t > now_ms()).unwrap_or_else(now_ms);
            let items: Vec<Compose> =
                targets.iter().map(|(row, addr)| fill::message_for(&template, l, &cols, *row, addr.clone())).collect();
            // paced, and built one at a time when sent (not 300 copies of the attachments up front)
            let n = outbox::queue_batch(ctx, items, start, SEND_GAP_MS).await?;
            Ok(RunResult { created: n, ..Default::default() })
        }
        RunMode::Drafts => {
            let total = targets.len();
            let account_id = template.account_id;
            for (i, (row, addr)) in targets.iter().enumerate() {
                ctx.events.status(
                    account_id,
                    "syncing",
                    &format!("Mail merge: saving draft {} of {total}…", i + 1),
                    Some(i as f64 / total as f64),
                );
                let m = fill::message_for(&template, l, &cols, *row, addr.clone());
                if let Err(e) = drafts::save(ctx, m).await {
                    ctx.events.status(account_id, "error", &format!("Mail merge stopped after {i} of {total} drafts."), None);
                    return Err(crate::user_err!("merge_drafts", "Saved {i} of {total} drafts, then: {e}"));
                }
            }
            ctx.events.status(account_id, "idle", &format!("Mail merge: {total} drafts saved."), None);
            Ok(RunResult { created: total, ..Default::default() })
        }
    }
}

#[cfg(test)]
mod tests;

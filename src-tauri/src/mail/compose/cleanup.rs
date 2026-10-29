//! Attachment copies under `compose/` in the data dir: a reopened draft's attachments
//! (`compose/<uuid>`, drafts.rs `parse_raw`, so they can be sent again) and attachments opened
//! from a compose window (`compose/open/<uuid>`). A folder goes once it's a week old and nothing
//! refers to it any more:
//! * an Outbox message lists its files in `compose_json` (a paced batch reads them when it goes
//!   out; Cancel / editing it builds the draft from them again);
//! * an open compose window holds them only in the window: while one is open, no `compose/<uuid>`
//!   folder goes;
//! * a Drafts row never does: a saved draft keeps its own copy in Gmail, and reopening it writes a
//!   new folder.
//!
//! `compose/open/<uuid>` is only ever handed to another program (the default one for the file), so
//! age alone decides.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

use crate::ctx::AppCtx;

const KEEP: Duration = Duration::from_secs(7 * 24 * 3600);

/// At start-up, then every few hours.
pub fn start(ctx: Arc<AppCtx>) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = clean_old(&ctx).await {
                log::warn!("compose files clean-up: {}", crate::logging::err(&e));
            }
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}

pub async fn clean_old(ctx: &Arc<AppCtx>) -> anyhow::Result<()> {
    clean(ctx, KEEP).await
}

pub(super) async fn clean(ctx: &Arc<AppCtx>, keep_for: Duration) -> anyhow::Result<()> {
    let root = ctx.data_dir.join("compose");
    let open = root.join("open");
    tokio::task::spawn_blocking(move || remove_old(&open, keep_for, |_| false)).await?;
    if compose_window_open(ctx) {
        return Ok(());
    }
    // the folder names are uuids: one in an Outbox message's compose is one of its files' folders
    let outbox: Vec<String> = ctx
        .db
        .read(|c| {
            let mut st = c.prepare("SELECT lower(compose_json) FROM outbox")?;
            let rows = st.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    tokio::task::spawn_blocking(move || remove_old(&root, keep_for, |name| name == "open" || outbox.iter().any(|j| j.contains(name))))
        .await?;
    Ok(())
}

fn compose_window_open(ctx: &AppCtx) -> bool {
    use tauri::Manager;
    let Some(app) = &ctx.app else { return false };
    app.webview_windows().keys().any(|label| crate::shell::window_state_key(label) == "compose" && !crate::spare::is_unclaimed(label))
}

fn remove_old(root: &Path, keep_for: Duration, in_use: impl Fn(&str) -> bool) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        if in_use(&e.file_name().to_string_lossy().to_lowercase()) {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > keep_for);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_old_unreferenced_folders_go() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let root = ctx.data_dir.join("compose");
        let folder = |name: &str| {
            let d = root.join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("Q3 deck.pptx"), b"x").unwrap();
            d
        };
        let queued = folder("0f9a3c51d2b84e6fa1c27d5e8b903a11");
        let orphan = folder("7c2e91b04a5d4f38b6e0d1a9c4f27e55");
        let opened = folder("open/a41d9e7f3b2c4c0e8f56a7b1d2e3c4f5");
        // an Outbox message (e.g. deferred delivery) attaching a file from the first folder
        let acct = crate::accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny", "demo").await.unwrap().id;
        let compose = serde_json::json!({ "accountId": acct, "attachments": [{ "path": queued.join("Q3 deck.pptx").to_string_lossy() }] });
        ctx.db
            .write(move |c| {
                c.execute(
                    "INSERT INTO outbox(account_id, compose_json, status, send_at, created_at) VALUES (?1,?2,'queued',0,0)",
                    rusqlite::params![acct, compose.to_string()],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        // younger than a week: everything stays
        clean_old(&ctx).await.unwrap();
        assert!(queued.exists() && orphan.exists() && opened.exists());

        std::thread::sleep(Duration::from_millis(50));
        clean(&ctx, Duration::ZERO).await.unwrap();
        assert!(queued.join("Q3 deck.pptx").exists(), "the Outbox still needs it");
        assert!(!orphan.exists(), "nothing refers to it");
        assert!(!opened.exists());
        assert!(root.join("open").exists());
    }
}

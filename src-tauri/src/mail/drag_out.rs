//! Dragging an attachment out of ClassicMail onto an Explorer folder, the desktop, Word, Teams…:
//! a page can't hand Windows a file, so the attachment is written to a temporary file (marked as
//! downloaded from the internet, like Save As) and the window starts an ordinary Windows file drag
//! of it (the `drag` crate: OLE DoDragDrop on the window's thread, while the button is still down).
//!
//! rpc:
//! * `mail.attachments.prepareDrag { attachmentId | archiveId + index }` → path — on mouse down,
//!   so the file is ready (downloaded from Gmail if need be) by the time the drag starts;
//! * `mail.attachments.drag { …the same, window }` → `bool` — start the drag from that window.
//!
//! The temporary copies live in `<data>/dragout/` and are removed after a day.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use serde::Deserialize;
use tauri::Manager;

use crate::{ctx::AppCtx, rpc::Registry};

pub fn register(r: &mut Registry) {
    r.add("mail.attachments.prepareDrag", |ctx: Arc<AppCtx>, w: Which| async move {
        Ok(materialize(&ctx, &w).await?.to_string_lossy().into_owned())
    });
    r.add("mail.attachments.drag", drag_rpc);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Which {
    attachment_id: Option<i64>,
    archive_id: Option<i64>,
    index: Option<usize>,
    /// the window the drag starts from (its label)
    window: Option<String>,
}

/// A file name Windows accepts.
pub fn safe_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let s = s.trim().trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() { "attachment".into() } else { s }
}

/// Copies older than a day go (whatever they were dropped on has its own copy by then).
fn clean_old(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let day = Duration::from_secs(24 * 3600);
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > day);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// The attachment as a file on disk (written once; the same file for a second drag).
async fn materialize(ctx: &Arc<AppCtx>, w: &Which) -> anyhow::Result<PathBuf> {
    let (key, name, bytes) = match (w.attachment_id, w.archive_id, w.index) {
        (Some(id), _, _) => {
            let (name, _, bytes) = crate::mail::attachment_bytes(ctx, id).await?;
            (format!("m{id}"), name, bytes)
        }
        (None, Some(a), Some(i)) => {
            let (name, bytes) = crate::archive::api::attachment_bytes(ctx, a, i).await?;
            (format!("a{a}-{i}"), name, bytes)
        }
        _ => return Err(crate::user_err!("bad_params", "Which attachment?")),
    };
    let root = ctx.data_dir.join("dragout");
    clean_old(&root);
    let dir = root.join(key);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(safe_name(&name));
    if std::fs::metadata(&path).map(|m| m.len() as usize != bytes.len()).unwrap_or(true) {
        crate::mail::attach_policy::write_marked(&path, &bytes).map_err(|e| crate::user_err!("io", "Can't prepare {name}: {e}"))?;
    }
    Ok(path)
}

async fn drag_rpc(ctx: Arc<AppCtx>, w: Which) -> anyhow::Result<bool> {
    let path = materialize(&ctx, &w).await?;
    let Some(app) = ctx.app.clone() else { return Ok(false) };
    let label = w.window.clone().unwrap_or_else(|| "main".into());
    let win = app.get_webview_window(&label).ok_or_else(|| crate::user_err!("not_found", "No window {label}."))?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    // OLE drag and drop runs on the window's (main) thread and returns when the file is dropped
    app.run_on_main_thread(move || {
        let r = drag::start_drag(
            &win,
            drag::DragItem::Files(vec![path]),
            drag::Image::Raw(Vec::new()), // no picture: Windows shows its own drag cursor
            |_, _| {},
            drag::Options { skip_animatation_on_cancel_or_failure: false, mode: drag::DragMode::Copy },
        );
        let _ = tx.send(r.map_err(|e| e.to_string()));
    })?;
    rx.await?.map_err(|e| crate::user_err!("drag", "The attachment couldn't be dragged: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_windows_accepts() {
        assert_eq!(safe_name("Q3 results: draft?.pdf"), "Q3 results_ draft_.pdf");
        assert_eq!(safe_name("  "), "attachment");
        assert_eq!(safe_name("notes. "), "notes");
    }
}

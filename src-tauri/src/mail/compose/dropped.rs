//! Files dropped on a message being written (from Explorer, the desktop, another program): the
//! page gets their contents but no path, so it hands them over here; each is saved under
//! `<data>/dropin/` and attached by that path like a file picked in Attach File.
//!
//! rpc: `mail.compose.stageDropped { name, data (base64) }` → path.
//!
//! (A native drop handler would give the paths, but on Windows it switches off the page's own
//! drag and drop — names between To / Cc / Bcc, messages onto folders — so it stays off.)

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

use base64::Engine;
use serde::Deserialize;

use crate::ctx::AppCtx;

/// Gmail takes up to 25 MB per message; a little room for the encoding.
const MAX_BYTES: usize = 30 * 1024 * 1024;
/// Kept long enough for a message written over a few days (a saved draft carries its own copy).
const KEEP: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Deserialize)]
pub struct Dropped {
    name: String,
    data: String,
}

fn clean_old(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > KEEP);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

pub async fn stage(ctx: Arc<AppCtx>, p: Dropped) -> anyhow::Result<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(p.data.trim())
        .map_err(|_| crate::user_err!("bad_params", "{} couldn't be read.", p.name))?;
    if bytes.len() > MAX_BYTES {
        return Err(crate::user_err!("too_big", "{} is too big to attach (Gmail takes up to 25 MB per message).", p.name));
    }
    let root = ctx.data_dir.join("dropin");
    clean_old(&root);
    // a folder per file: two dropped files may have the same name
    let dir = root.join(format!("{}-{}", crate::clock::now_ms(), rand_suffix()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(crate::mail::drag_out::safe_name(&p.name));
    std::fs::write(&path, &bytes).map_err(|e| crate::user_err!("io", "Can't keep {}: {e}", p.name))?;
    Ok(path.to_string_lossy().into_owned())
}

fn rand_suffix() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn a_dropped_file_becomes_a_file_to_attach() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let reg = crate::rpc::build();
        let data = base64::engine::general_purpose::STANDARD.encode(b"hello");
        let a = reg.call(ctx.clone(), "mail.compose.stageDropped", json!({ "name": "Q3 deck.pdf", "data": data })).await.unwrap();
        let b = reg.call(ctx.clone(), "mail.compose.stageDropped", json!({ "name": "Q3 deck.pdf", "data": data })).await.unwrap();
        let (a, b) = (a.as_str().unwrap(), b.as_str().unwrap());
        assert!(a.ends_with("Q3 deck.pdf") && a != b, "same name, two files: {a} / {b}");
        assert_eq!(std::fs::read(a).unwrap(), b"hello");
        let e = reg.call(ctx.clone(), "mail.compose.stageDropped", json!({ "name": "x", "data": "%%%" })).await.unwrap_err();
        assert!(e.to_string().contains("couldn't be read"), "{e}");
    }
}

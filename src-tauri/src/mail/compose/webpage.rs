//! Include › Web Page: a web page (a file on this computer or a web address) as the message's
//! content. The page opens in a hidden, private window of its own, where its scripts run as in a
//! browser (a page that builds itself with a script is otherwise empty); once it has settled, the
//! compose window's script (`script`, src/modules/mail/compose/webPage.ts) turns what is on screen
//! into e-mail HTML, which comes back here and goes into the message.
//!
//! The page gets no access to ClassicMail: it is a remote origin (no IPC), and the window is
//! private (`incognito`: nothing it stores outlives it).
//!
//! A Markdown file or link (.md) is read as text instead (`mail.compose.readText`) and turned
//! into formatting by the compose window (compose/markdown.ts).
//!
//! rpc: `mail.compose.webPage { source, script }` → the e-mail HTML;
//! `mail.compose.readText { source }` → a file's or a web address's text.

use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use serde::Deserialize;

use crate::ctx::AppCtx;

/// The window label's kind (shell.rs: never remembered in the window-state file).
pub const WINDOW_KIND: &str = "webpage";
/// How long the page may take to load, settle and convert.
const TIMEOUT: Duration = Duration::from_secs(45);
/// Time for the page's own scripts to build it after it has loaded.
const SETTLE: Duration = Duration::from_millis(1500);

#[derive(Deserialize)]
pub struct WebPageParams {
    /// a web address ("https://…", "www.…", "example.com/x") or a file path
    source: String,
    /// the conversion, run in the page; returns the HTML as a string
    script: String,
}

/// What `source` means: a file that exists, or a web address (https:// added when it has none).
pub fn page_url(source: &str) -> anyhow::Result<url::Url> {
    let s = source.trim().trim_matches('"');
    if s.is_empty() {
        return Err(crate::user_err!("no_page", "Type a web address or choose a file."));
    }
    if let Some(rest) = s.strip_prefix("file:///") {
        return Ok(url::Url::parse(&format!("file:///{rest}"))?);
    }
    let p = Path::new(s);
    if p.is_absolute() {
        if !p.is_file() {
            return Err(crate::user_err!("no_page", "{s} wasn't found."));
        }
        return url::Url::from_file_path(p).map_err(|_| crate::user_err!("no_page", "{s} can't be opened."));
    }
    let with_scheme = if s.contains("://") { s.to_string() } else { format!("https://{s}") };
    let u = url::Url::parse(&with_scheme).map_err(|_| crate::user_err!("no_page", "\"{s}\" isn't a web address or a file."))?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none_or(|h| !h.contains('.') && h != "localhost") {
        return Err(crate::user_err!("no_page", "\"{s}\" isn't a web address or a file."));
    }
    Ok(u)
}

pub async fn web_page(ctx: Arc<AppCtx>, p: WebPageParams) -> anyhow::Result<String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder, webview::PageLoadEvent};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let url = page_url(&p.source)?;
    let app = ctx.app.clone().ok_or_else(|| anyhow::anyhow!("no app"))?;
    let label = format!("{WINDOW_KIND}-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let script = p.script;
    // the host only: the rest of a link can carry the reader's identity (a newsletter's tracking)
    log::info!("web page for a message: {}", url.host_str().unwrap_or("a file"));
    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url.clone()))
        .title("Web page")
        .visible(false)
        .focused(false)
        .skip_taskbar(true)
        // the page lays itself out at a desktop width (columns, tables), as it would be read
        .inner_size(1000.0, 900.0)
        .incognito(true)
        .on_page_load(move |w, payload| {
            if payload.event() != PageLoadEvent::Finished {
                return;
            }
            let (tx, script) = (tx.clone(), script.clone());
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(SETTLE).await;
                // the first load that finishes (a redirect loads again: the later one is the page)
                if tx.lock().map(|t| t.is_none()).unwrap_or(true) {
                    return;
                }
                let _ = w.eval_with_callback(script, move |json| {
                    if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
                        let _ = tx.send(json);
                    }
                });
            });
        })
        .build()?;
    let out = tokio::time::timeout(TIMEOUT, rx).await;
    let _ = window.destroy();
    let json = match out {
        Ok(Ok(json)) => json,
        _ => return Err(crate::user_err!("page_timeout", "The page didn't load in time. Check the address and try again.")),
    };
    // the script's return value, as JSON: the HTML, or {"error": "..."}
    let v: serde_json::Value = serde_json::from_str(&json).unwrap_or(serde_json::Value::Null);
    match v {
        serde_json::Value::String(html) if !html.trim().is_empty() => Ok(html),
        serde_json::Value::Object(o) if o.get("error").is_some() => {
            Err(crate::user_err!("page_failed", "The page couldn't be turned into a message: {}", o["error"].as_str().unwrap_or("")))
        }
        _ => Err(crate::user_err!("page_empty", "The page has nothing to put in a message.")),
    }
}

#[derive(Deserialize)]
pub struct ReadTextParams {
    source: String,
}

/// The most text `read_text` takes (a Markdown document, not a book).
const MAX_TEXT: usize = 2 * 1024 * 1024;

/// A GitHub page of a file shows it inside the site: its raw text is at raw.githubusercontent.com.
fn raw_github(u: &url::Url) -> url::Url {
    if u.host_str() == Some("github.com") {
        let parts: Vec<&str> = u.path().trim_start_matches('/').splitn(4, '/').collect();
        if parts.len() == 4
            && parts[2] == "blob"
            && let Ok(raw) = url::Url::parse(&format!("https://raw.githubusercontent.com/{}/{}/{}", parts[0], parts[1], parts[3]))
        {
            return raw;
        }
    }
    u.clone()
}

/// The text of a file on this computer or at a web address (a Markdown document to insert).
pub async fn read_text(ctx: Arc<AppCtx>, p: ReadTextParams) -> anyhow::Result<String> {
    let url = page_url(&p.source)?;
    let bytes = if url.scheme() == "file" {
        let path = url.to_file_path().map_err(|_| crate::user_err!("no_page", "{} can't be opened.", p.source))?;
        tokio::fs::read(&path).await?
    } else {
        let r = ctx.http.get(raw_github(&url)).timeout(Duration::from_secs(30)).send().await?.error_for_status()?;
        r.bytes().await?.to_vec()
    };
    if bytes.len() > MAX_TEXT {
        return Err(crate::user_err!("too_big", "That document is too long to put in a message."));
    }
    Ok(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string())
}

#[cfg(test)]
mod tests {
    use super::page_url;

    #[test]
    fn a_github_file_page_reads_its_raw_text() {
        let u = url::Url::parse("https://github.com/owner/repo/blob/main/docs/README.md").unwrap();
        assert_eq!(super::raw_github(&u).as_str(), "https://raw.githubusercontent.com/owner/repo/main/docs/README.md");
        let other = url::Url::parse("https://example.com/notes.md").unwrap();
        assert_eq!(super::raw_github(&other), other);
    }

    #[test]
    fn addresses_and_files() {
        assert_eq!(page_url("https://www.first.org.il/news").unwrap().as_str(), "https://www.first.org.il/news");
        assert_eq!(page_url("www.example.com").unwrap().as_str(), "https://www.example.com/");
        assert_eq!(page_url("example.com/about").unwrap().as_str(), "https://example.com/about");
        assert!(page_url("hello").is_err(), "a word isn't an address");
        assert!(page_url("ftp://x.example").is_err());
        assert!(page_url("").is_err());
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sheet one.html");
        std::fs::write(&f, "<p>x</p>").unwrap();
        let u = page_url(&format!("\"{}\"", f.display())).unwrap();
        assert_eq!(u.scheme(), "file");
        assert!(u.as_str().ends_with("sheet%20one.html"));
        assert!(page_url(&dir.path().join("missing.html").to_string_lossy()).is_err());
    }
}

//! Spare windows: a hidden, already-loaded compose window and message window kept in the
//! background, so New / Reply / Forward and opening an e-mail show a finished window at once instead of a blank one while WebView2 starts
//! and the page loads (~1 s). The opener claims the spare (`app.claimSpare`), which gets the
//! message's parameters, renders it while still hidden and shows itself; a new spare is then
//! loaded quietly for next time.
//!
//! rpc methods:
//! * `app.prepareSpare { kind }` — load a spare of that kind if there is none (main window, after start-up)
//! * `app.spareReady { label }` — from the spare: loaded and waiting
//! * `app.claimSpare { kind, query, title }` → `label | null` — hand the spare its window's query string
//! * `app.revealSpare { label }` — from the claimed spare: its content is drawn, make it opaque
//!
//! A hidden WebView doesn't draw (React then waits ~300 ms for a frame that doesn't come), so a
//! claimed spare is shown at once but fully transparent, draws at full speed, and is made opaque
//! when its content is on screen: the user sees the finished window, never a blank one.

use std::{collections::HashMap, sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::{ctx::AppCtx, rpc::Registry};

/// Kinds that keep a spare (src/api/windows.ts asks only for these).
const KINDS: &[(&str, f64, f64)] = &[("compose", 1040.0, 700.0), ("message", 900.0, 700.0)];

#[derive(Default)]
struct Spare {
    label: String,
    ready: bool,
}

static SPARES: Mutex<Option<HashMap<String, Spare>>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut HashMap<String, Spare>) -> R) -> R {
    f(SPARES.lock().get_or_insert_with(HashMap::new))
}

/// Close the waiting spares before the app exits: the window-state plugin saves every window's
/// place on exit, and a spare's would be where compose windows were when it was made.
pub fn close_unclaimed(app: &AppHandle) {
    let labels: Vec<String> = with(|m| m.drain().map(|(_, s)| s.label).collect());
    for l in labels {
        if let Some(w) = app.get_webview_window(&l) {
            let _ = w.destroy();
        }
    }
}

/// Unclaimed spares: not real item windows (quitting doesn't wait for them, nothing to save).
pub fn is_unclaimed(label: &str) -> bool {
    with(|m| m.values().any(|s| s.label == label))
}

pub fn register(r: &mut Registry) {
    r.add("app.prepareSpare", prepare);
    r.add("app.spareReady", ready);
    r.add("app.claimSpare", claim);
    r.add("app.revealSpare", reveal);
}

#[derive(Deserialize)]
struct KindParams {
    kind: String,
}

async fn prepare(ctx: Arc<AppCtx>, p: KindParams) -> anyhow::Result<()> {
    if let Some(app) = &ctx.app {
        create(app, &p.kind);
    }
    Ok(())
}

/// Load a hidden spare of `kind` unless one exists (or is loading).
fn create(app: &AppHandle, kind: &str) {
    let Some(&(_, w, h)) = KINDS.iter().find(|(k, _, _)| *k == kind) else { return };
    let label = format!("{kind}-spare-{}", crate::db::util::now_ms());
    let fresh = with(|m| {
        if m.get(kind).is_some_and(|s| app.get_webview_window(&s.label).is_some()) {
            return false;
        }
        m.insert(kind.to_string(), Spare { label: label.clone(), ready: false });
        true
    });
    if !fresh {
        return;
    }
    let url = WebviewUrl::App(format!("index.html?window={kind}&spare=1").into());
    if let Err(e) = WebviewWindowBuilder::new(app, &label, url)
        .title("ClassicMail")
        .inner_size(w, h)
        .center()
        .visible(false)
        .focused(false)
        // the page's own drag and drop (names between To / Cc, files onto a message) — a
        // native drop handler would switch it off on Windows (as in the main window's config)
        .disable_drag_drop_handler()
        .build()
    {
        log::warn!("spare {kind} window: {e}");
        with(|m| m.remove(kind));
    }
}

#[derive(Deserialize)]
struct LabelParams {
    label: String,
}

async fn ready(_ctx: Arc<AppCtx>, p: LabelParams) -> anyhow::Result<()> {
    with(|m| {
        if let Some(s) = m.values_mut().find(|s| s.label == p.label) {
            s.ready = true;
        }
    });
    Ok(())
}

#[derive(Deserialize)]
struct ClaimParams {
    kind: String,
    query: String,
    title: String,
}

async fn claim(ctx: Arc<AppCtx>, p: ClaimParams) -> anyhow::Result<Option<String>> {
    let Some(app) = ctx.app.clone() else { return Ok(None) };
    let label = with(|m| match m.get(&p.kind) {
        Some(s) if s.ready && app.get_webview_window(&s.label).is_some() => m.remove(&p.kind).map(|s| s.label),
        _ => None,
    });
    let Some(label) = label else {
        // none ready (still loading, or never made): open normally this time
        create(&app, &p.kind);
        return Ok(None);
    };
    if let Some(w) = app.get_webview_window(&label) {
        // where the last compose window was (it may have moved since the spare was made)
        use tauri_plugin_window_state::{StateFlags, WindowExt};
        let _ = w.restore_state(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED);
        set_transparent(&w, true);
        let _ = w.show();
    }
    app.emit_to(label.as_str(), "cm-spare-claim", serde_json::json!({ "query": p.query, "title": p.title }))?;
    // the next one, once this window is on screen
    let kind = p.kind.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(2500)).await;
        create(&app, &kind);
    });
    Ok(Some(label))
}

async fn reveal(ctx: Arc<AppCtx>, p: LabelParams) -> anyhow::Result<()> {
    if let Some(w) = ctx.app.as_ref().and_then(|a| a.get_webview_window(&p.label)) {
        set_transparent(&w, false);
        let _ = w.show();
        let _ = w.set_focus();
    }
    Ok(())
}

/// Fully transparent (still "shown", so WebView2 draws) or back to a normal opaque window.
#[cfg(windows)]
fn set_transparent(w: &tauri::WebviewWindow, on: bool) {
    use windows::Win32::{
        Foundation::{COLORREF, HWND},
        UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW, WS_EX_LAYERED,
        },
    };
    let Ok(hwnd) = w.hwnd() else { return };
    let hwnd = HWND(hwnd.0);
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if on {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
            // 1 of 255, not 0: a window at 0 counts as hidden and its WebView stops drawing
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA);
        } else {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_LAYERED.0 as isize));
        }
    }
}

#[cfg(not(windows))]
fn set_transparent(_w: &tauri::WebviewWindow, _on: bool) {}

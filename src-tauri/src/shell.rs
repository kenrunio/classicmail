//! Window / tray plumbing: main window show/hide, tray icon (with a new-mail badge), close-to-tray,
//! start minimised (autostart), and background services started after launch.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tauri::{
    App, AppHandle, Emitter, Listener, Manager, WindowEvent,
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::{ctx::AppCtx, db::util::get_setting};

const TRAY_ID: &str = "main";

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    set_tray_state(app, Some(false), None);
}

fn emit_ui(app: &AppHandle, action: &str) {
    use tauri::Emitter;
    let _ = app.emit("cm-event", serde_json::json!({ "kind": "ui.action", "action": action }));
}

pub fn setup(app: &mut App, ctx: Arc<AppCtx>) -> anyhow::Result<()> {
    let handle = app.handle().clone();

    let open = MenuItem::with_id(app, "open", "Open ClassicMail", true, None::<&str>)?;
    let new_msg = MenuItem::with_id(app, "new-mail", "New E-mail Message", true, None::<&str>)?;
    let sendrecv = MenuItem::with_id(app, "send-receive", "Send/Receive", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let exit = MenuItem::with_id(app, "exit", "Exit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &new_msg, &sendrecv, &sep, &exit])?;

    let c2 = ctx.clone();
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .tooltip("ClassicMail")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, e| match e.id.as_ref() {
            "open" => show_main(app),
            "new-mail" => {
                show_main(app);
                emit_ui(app, "new-mail");
            }
            "send-receive" => c2.sync_now(None),
            "exit" => request_quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;

    // a "Do you want to save changes?" prompt answered with Cancel aborts a pending quit
    app.listen_any("cm-quit-cancelled", |_| {
        QUITTING.store(false, Ordering::SeqCst);
    });

    if let Some(main) = app.get_webview_window("main") {
        // started by Windows at logon → stay in the tray
        if std::env::args().any(|a| a == "--minimized") {
            let _ = main.hide();
        }
        let ctx2 = ctx.clone();
        let h2 = handle.clone();
        main.on_window_event(move |ev| match ev {
            WindowEvent::CloseRequested { api, .. } => {
                let to_tray = ctx2.db.read_blocking(|c| get_setting::<bool>(c, "app.closeToTray")).ok().flatten().unwrap_or(false);
                if to_tray && !QUITTING.load(Ordering::SeqCst) {
                    api.prevent_close();
                    if let Some(w) = h2.get_webview_window("main") {
                        let _ = w.hide();
                    }
                } else if !open_item_windows(&h2).is_empty() {
                    // unsaved compose/appointment/contact/task windows get to ask first
                    api.prevent_close();
                    request_quit(&h2);
                }
            }
            WindowEvent::Destroyed => {
                // closing the main window quits the app (item windows included), like Outlook
                crate::spare::close_unclaimed(&h2);
                h2.exit(0);
            }
            WindowEvent::Focused(true) => set_tray_state(&h2, Some(false), None),
            _ => {}
        });
    }
    Ok(())
}

/// Tray badge for new mail (None = leave as is) + tooltip with the unread count.
pub fn set_tray_state(app: &AppHandle, new_mail: Option<bool>, unread: Option<i64>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    if let Some(new_mail) = new_mail {
        let bytes: &[u8] = if new_mail { include_bytes!("../icons/tray-new.png") } else { include_bytes!("../icons/tray.png") };
        if let Ok(img) = Image::from_bytes(bytes) {
            let _ = tray.set_icon(Some(img));
        }
    }
    if let Some(n) = unread {
        let tip = if n > 0 { format!("ClassicMail — {n} unread") } else { "ClassicMail".to_string() };
        let _ = tray.set_tooltip(Some(tip));
    }
}

/// Item windows that may hold unsaved work.
const ITEM_KINDS: &[&str] = &["compose", "appointment", "contact", "task"];

static QUITTING: AtomicBool = AtomicBool::new(false);

fn open_item_windows(app: &AppHandle) -> Vec<tauri::WebviewWindow> {
    app.webview_windows()
        .into_iter()
        .filter(|(label, _)| ITEM_KINDS.contains(&window_state_key(label)) && !crate::spare::is_unclaimed(label))
        .map(|(_, w)| w)
        .collect()
}

/// Quit like Outlook: every open item window is asked to close (each prompts "Do you want to save
/// changes?" if needed); the app exits once they are all gone. Cancel in any prompt aborts.
pub fn request_quit(app: &AppHandle) {
    let items = open_item_windows(app);
    if items.is_empty() {
        crate::spare::close_unclaimed(app);
        app.exit(0);
        return;
    }
    if QUITTING.swap(true, Ordering::SeqCst) {
        return; // already in progress
    }
    let _ = app.emit("cm-quit-start", ());
    for w in &items {
        let _ = w.set_focus();
        let _ = w.close();
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            if !QUITTING.load(Ordering::SeqCst) {
                return; // cancelled
            }
            if open_item_windows(&app).is_empty() {
                crate::spare::close_unclaimed(&app);
                app.exit(0);
                return;
            }
        }
    });
}

/// Window kinds (see src/api/windows.ts). Item windows get labels like `appointment-12-occurrence`
/// or `compose-1760000000-123`; their size and position are remembered per kind, like Outlook
/// opens every message window where the last one was (and the state file doesn't grow per item).
const WINDOW_KINDS: &[&str] = &[
    "main",
    // Open in New Window: extra main windows (`explorer-<n>`), all placed where the last one was
    "explorer",
    "compose",
    "message",
    "appointment",
    "contact",
    "task",
    "reminders",
    "desktop-alert",
    "options",
    "rules",
    "accounts",
    "address-book",
    "recurrence",
    // Include › Web Page's hidden page window (mail/compose/webpage.rs)
    "webpage",
];

pub fn window_state_key(label: &str) -> &str {
    WINDOW_KINDS
        .iter()
        .filter(|k| label == **k || (label.starts_with(**k) && label[k.len()..].starts_with('-')))
        .max_by_key(|k| k.len())
        .copied()
        .unwrap_or(label)
}

#[cfg(test)]
mod tests {
    use super::window_state_key;

    #[test]
    fn window_state_keys_are_per_kind() {
        assert_eq!(window_state_key("main"), "main");
        assert_eq!(window_state_key("explorer-1761300000-42"), "explorer");
        assert_eq!(window_state_key("appointment-634-occurrence"), "appointment");
        assert_eq!(window_state_key("compose-1760000000000-12345"), "compose");
        assert_eq!(window_state_key("address-book"), "address-book");
        assert_eq!(window_state_key("recurrence-appointment-1-2"), "recurrence");
        assert_eq!(window_state_key("desktop-alert-3"), "desktop-alert");
        assert_eq!(window_state_key("something-else"), "something-else");
        assert_eq!(window_state_key("mainx"), "mainx");
    }
}

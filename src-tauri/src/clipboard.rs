//! Files copied in Explorer (Ctrl+C on a file): `app.clipboard.files` → their paths, so Ctrl+V
//! in a message attaches them as in Outlook. A web page only gets nameless File objects for a
//! pasted file, never its path, so the compose window asks here.

use std::sync::Arc;

use crate::{
    ctx::AppCtx,
    rpc::{NoParams, Registry},
};

pub fn register(r: &mut Registry) {
    r.add("app.clipboard.files", |_: Arc<AppCtx>, _: NoParams| async move { Ok(copied_files()) });
}

#[cfg(windows)]
fn copied_files() -> Vec<String> {
    use windows::Win32::{
        System::DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard},
        UI::Shell::{DragQueryFileW, HDROP},
    };
    const CF_HDROP: u32 = 15;
    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP).is_err() {
            return vec![];
        }
        // another program may have the clipboard open for a moment
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if !opened {
            return vec![];
        }
        let mut out = Vec::new();
        if let Ok(h) = GetClipboardData(CF_HDROP) {
            let drop = HDROP(h.0);
            let n = DragQueryFileW(drop, u32::MAX, None);
            for i in 0..n {
                let len = DragQueryFileW(drop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                let got = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
                out.push(String::from_utf16_lossy(&buf[..got]));
            }
        }
        let _ = CloseClipboard();
        out
    }
}

#[cfg(not(windows))]
fn copied_files() -> Vec<String> {
    vec![]
}

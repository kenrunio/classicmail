//! The keyboard, for Word-style typing fixes in the compose window
//! (src/modules/mail/compose/typingFixes.ts):
//! * `app.keyboard.layout` → `{ lang, rtl }` — the input language of the focused window
//!   ("he", "en", …; `rtl` for Hebrew, Arabic, Persian, Urdu), or `null` when unknown;
//! * `app.keyboard.capsLockOff` — turn Caps Lock off (it was left on: "eLI" → "Eli");
//! * `app.keyboard.switchToEnglish` → `bool` — switch the focused window to an installed English
//!   layout (Hebrew was left on: "Eךן" → "Eli"); false when none is installed.
//!
//! WebView2 draws in its own process, so the focused window is found through the foreground
//! window's GUI thread info rather than in this process.

use std::sync::Arc;

use serde::Serialize;

use crate::{
    ctx::AppCtx,
    rpc::{NoParams, Registry},
};

pub fn register(r: &mut Registry) {
    r.add("app.keyboard.layout", |_: Arc<AppCtx>, _: NoParams| async move { Ok(layout()) });
    r.add("app.keyboard.capsLockOff", |_: Arc<AppCtx>, _: NoParams| async move {
        caps_lock_off();
        Ok(())
    });
    r.add("app.keyboard.switchToEnglish", |_: Arc<AppCtx>, _: NoParams| async move { Ok(switch_to_english()) });
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Layout {
    pub lang: &'static str,
    pub rtl: bool,
}

/// A keyboard layout's language (the low word of its HKL).
pub fn lang_of(langid: u16) -> Layout {
    let (lang, rtl) = match langid & 0x3ff {
        0x09 => ("en", false),
        0x0d => ("he", true),
        0x01 => ("ar", true),
        0x29 => ("fa", true),
        0x20 => ("ur", true),
        0x19 => ("ru", false),
        0x0c => ("fr", false),
        0x07 => ("de", false),
        0x0a => ("es", false),
        _ => ("other", false),
    };
    Layout { lang, rtl }
}

#[cfg(windows)]
mod win {
    use windows::Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::{
            Input::KeyboardAndMouse::{
                GetKeyState, GetKeyboardLayout, GetKeyboardLayoutList, HKL, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
                KEYEVENTF_KEYUP, SendInput, VK_CAPITAL,
            },
            WindowsAndMessaging::{
                GUITHREADINFO, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, PostMessageW, WM_INPUTLANGCHANGEREQUEST,
            },
        },
    };

    /// The window with the keyboard focus (in WebView2's process when typing in a page).
    pub fn focused() -> Option<HWND> {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.0.is_null() {
                return None;
            }
            let tid = GetWindowThreadProcessId(fg, None);
            let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
            let f = if GetGUIThreadInfo(tid, &mut info).is_ok() && !info.hwndFocus.0.is_null() { info.hwndFocus } else { fg };
            Some(f)
        }
    }

    pub fn langid() -> Option<u16> {
        let w = focused()?;
        unsafe {
            let tid = GetWindowThreadProcessId(w, None);
            Some((GetKeyboardLayout(tid).0 as usize & 0xffff) as u16)
        }
    }

    pub fn caps_lock_off() {
        unsafe {
            if GetKeyState(VK_CAPITAL.0 as i32) & 1 == 0 {
                return;
            }
            let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_CAPITAL, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
            };
            let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    pub fn switch_to_english() -> bool {
        unsafe {
            let n = GetKeyboardLayoutList(None);
            let mut list = vec![HKL::default(); n.max(0) as usize];
            let n = GetKeyboardLayoutList(Some(&mut list));
            list.truncate(n.max(0) as usize);
            let Some(en) = list.into_iter().find(|h| (h.0 as usize & 0x3ff) == 0x09) else { return false };
            let Some(w) = focused() else { return false };
            PostMessageW(Some(w), WM_INPUTLANGCHANGEREQUEST, WPARAM(0), LPARAM(en.0 as isize)).is_ok()
        }
    }
}

#[cfg(windows)]
fn layout() -> Option<Layout> {
    win::langid().map(lang_of)
}
#[cfg(windows)]
fn caps_lock_off() {
    win::caps_lock_off()
}
#[cfg(windows)]
fn switch_to_english() -> bool {
    win::switch_to_english()
}

#[cfg(not(windows))]
fn layout() -> Option<Layout> {
    None
}
#[cfg(not(windows))]
fn caps_lock_off() {}
#[cfg(not(windows))]
fn switch_to_english() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_and_direction() {
        assert_eq!(lang_of(0x040d), Layout { lang: "he", rtl: true });
        assert_eq!(lang_of(0x0409), Layout { lang: "en", rtl: false });
        assert_eq!(lang_of(0x0809), Layout { lang: "en", rtl: false }); // English (UK)
        assert_eq!(lang_of(0x0401), Layout { lang: "ar", rtl: true });
    }
}

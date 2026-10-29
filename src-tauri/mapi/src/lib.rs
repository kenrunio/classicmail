//! ClassicMail's Simple MAPI DLL: what Word / Excel / PowerPoint's "Email" and "Send as attachment"
//! (and any program's "Send to › Mail recipient") call on the user's mail client. It hands the
//! message to ClassicMail, which opens it in a new message window:
//!
//! 1. The settings of the Windows user calling (the DLL is shared by every user of the computer):
//!    `%LOCALAPPDATA%\ClassicMail\mapi.ini`, written by that user's ClassicMail: `InboxDir`,
//!    `ExePath`, `RunningMutex`. (Not the registry: Office Click-to-Run shows its programs its own
//!    HKCU\Software\Clients, without ours.) None: a message says to open ClassicMail first.
//! 2. The files are copied at once (the calling program deletes its temporary copy as soon as
//!    MAPISendMail returns) into `<InboxDir>\<id>.part\`, with `request.json` (recipients, subject,
//!    text, files); renaming the folder to `<id>` makes it visible as a whole.
//! 3. ClassicMail running (its mutex exists) picks the request up within a second; otherwise the
//!    DLL starts it, and it reads its inbox on start-up.
//!
//! Only the entry points programs use to *send* are real; the rest of Simple MAPI answers
//! "not supported". Built for x64 and x86 (32-bit Office) by the app's build.rs.

#![allow(clippy::missing_safety_doc)]

use std::{
    ffi::c_void,
    path::{Path, PathBuf},
    ptr,
};

use windows_sys::Win32::{
    Foundation::CloseHandle,
    Globalization::{CP_ACP, MultiByteToWideChar},
    System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE},
    UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow},
};

const SUCCESS_SUCCESS: u32 = 0;
const MAPI_E_USER_ABORT: u32 = 1;
const MAPI_E_FAILURE: u32 = 2;
const MAPI_E_ATTACHMENT_NOT_FOUND: u32 = 11;
const MAPI_E_NOT_SUPPORTED: u32 = 26;

// ---- Simple MAPI structures (MAPI.h) ----------------------------------------------------------

#[repr(C)]
pub struct MapiRecipDescW {
    ul_reserved: u32,
    ul_recip_class: u32,
    lpsz_name: *const u16,
    lpsz_address: *const u16,
    ul_eid_size: u32,
    lp_entry_id: *mut c_void,
}
#[repr(C)]
pub struct MapiFileDescW {
    ul_reserved: u32,
    fl_flags: u32,
    n_position: u32,
    lpsz_path_name: *const u16,
    lpsz_file_name: *const u16,
    lp_file_type: *mut c_void,
}
#[repr(C)]
pub struct MapiMessageW {
    ul_reserved: u32,
    lpsz_subject: *const u16,
    lpsz_note_text: *const u16,
    lpsz_message_type: *const u16,
    lpsz_date_received: *const u16,
    lpsz_conversation_id: *const u16,
    fl_flags: u32,
    lp_originator: *const MapiRecipDescW,
    n_recip_count: u32,
    lp_recips: *const MapiRecipDescW,
    n_file_count: u32,
    lp_files: *const MapiFileDescW,
}
#[repr(C)]
pub struct MapiRecipDesc {
    ul_reserved: u32,
    ul_recip_class: u32,
    lpsz_name: *const u8,
    lpsz_address: *const u8,
    ul_eid_size: u32,
    lp_entry_id: *mut c_void,
}
#[repr(C)]
pub struct MapiFileDesc {
    ul_reserved: u32,
    fl_flags: u32,
    n_position: u32,
    lpsz_path_name: *const u8,
    lpsz_file_name: *const u8,
    lp_file_type: *mut c_void,
}
#[repr(C)]
pub struct MapiMessage {
    ul_reserved: u32,
    lpsz_subject: *const u8,
    lpsz_note_text: *const u8,
    lpsz_message_type: *const u8,
    lpsz_date_received: *const u8,
    lpsz_conversation_id: *const u8,
    fl_flags: u32,
    lp_originator: *const MapiRecipDesc,
    n_recip_count: u32,
    lp_recips: *const MapiRecipDesc,
    n_file_count: u32,
    lp_files: *const MapiFileDesc,
}

// ---- strings ----------------------------------------------------------------------------------

unsafe fn wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) })
}

/// A string in the system's ANSI code page.
unsafe fn ansi(p: *const u8) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    if n == 0 {
        return String::new();
    }
    let len = unsafe { MultiByteToWideChar(CP_ACP, 0, p, n as i32, ptr::null_mut(), 0) };
    let mut buf = vec![0u16; len.max(0) as usize];
    unsafe { MultiByteToWideChar(CP_ACP, 0, p, n as i32, buf.as_mut_ptr(), len) };
    String::from_utf16_lossy(&buf)
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

// ---- the request --------------------------------------------------------------------------------

pub struct Recip {
    /// 1 To, 2 Cc, 3 Bcc
    pub class: u32,
    pub name: String,
    pub address: String,
}
pub struct File {
    pub path: String,
    pub name: String,
}
pub struct Request {
    pub subject: String,
    pub body: String,
    pub recips: Vec<Recip>,
    pub files: Vec<File>,
}

/// This Windows user's settings, written by their ClassicMail.
fn user_ini() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData\\Local")))?;
    Some(local.join("ClassicMail").join("mapi.ini"))
}

/// `Key=Value` lines (UTF-8).
pub fn ini_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        k.trim().eq_ignore_ascii_case(key).then(|| v.trim().to_string()).filter(|v| !v.is_empty())
    })
}

/// A setting from this user's `mapi.ini`.
fn setting(key: &str) -> Option<String> {
    let ini = user_ini().and_then(|p| std::fs::read_to_string(p).ok());
    ini.and_then(|t| ini_value(t.trim_start_matches('\u{feff}'), key))
}

fn message(owner: usize, text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW};
    let (text, caption) = (to_wide(text), to_wide("ClassicMail"));
    unsafe { MessageBoxW(owner as _, text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONINFORMATION) };
}

fn running(mutex: &str) -> bool {
    let name = to_wide(mutex);
    let h = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
    if h.is_null() {
        return false;
    }
    unsafe { CloseHandle(h) };
    true
}

/// A file name Windows accepts.
fn safe_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let s = s.trim().trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() { "attachment".into() } else { s }
}

fn unique(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    (2..).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|p| !p.exists()).unwrap()
}

/// Write the request into `inbox` (files copied now); the folder that holds it.
pub fn deliver(inbox: &Path, req: &Request, id: &str) -> Result<PathBuf, u32> {
    let part = inbox.join(format!("{id}.part"));
    std::fs::create_dir_all(&part).map_err(|_| MAPI_E_FAILURE)?;
    let mut files = Vec::new();
    for f in &req.files {
        let src = Path::new(&f.path);
        let name = if f.name.trim().is_empty() {
            src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        } else {
            f.name.clone()
        };
        let dest = unique(&part, &safe_name(&name));
        if std::fs::copy(src, &dest).is_err() {
            let _ = std::fs::remove_dir_all(&part);
            return Err(MAPI_E_ATTACHMENT_NOT_FOUND);
        }
        files.push(dest.file_name().unwrap().to_string_lossy().into_owned());
    }
    let recips = |class: u32| {
        req.recips
            .iter()
            .filter(|r| r.class == class)
            .map(|r| {
                let addr = r.address.trim();
                let addr = addr.strip_prefix("SMTP:").or_else(|| addr.strip_prefix("smtp:")).unwrap_or(addr);
                format!("{{\"name\":{},\"email\":{}}}", json_str(r.name.trim()), json_str(addr))
            })
            .collect::<Vec<_>>()
            .join(",")
    };
    let json = format!(
        "{{\"subject\":{},\"body\":{},\"to\":[{}],\"cc\":[{}],\"bcc\":[{}],\"files\":[{}]}}",
        json_str(&req.subject),
        json_str(&req.body),
        recips(1),
        recips(2),
        recips(3),
        files.iter().map(|f| json_str(f)).collect::<Vec<_>>().join(",")
    );
    std::fs::write(part.join("request.json"), json).map_err(|_| MAPI_E_FAILURE)?;
    let done = inbox.join(id);
    std::fs::rename(&part, &done).map_err(|_| MAPI_E_FAILURE)?;
    Ok(done)
}

/// One line per call in %TEMP%\ClassicMailMapi.log: what a program asked for and how it went
/// (a program only shows "MAPI failure"; this says why).
fn log(line: &str) {
    use std::io::Write;
    let path = std::env::temp_dir().join("ClassicMailMapi.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 256 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    let exe = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{secs} {exe}[{}] {line}", std::process::id());
    }
}

/// `owner`: the calling program's window, for a message.
fn send(req: Request, owner: usize) -> u32 {
    log(&format!(
        "send: {} recipient(s), {} file(s) {:?}",
        req.recips.len(),
        req.files.len(),
        req.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    ));
    let Some(inbox) = setting("InboxDir") else {
        log(&format!("send stopped: no InboxDir in {:?}", user_ini()));
        message(
            owner,
            "E-mail on this computer goes through ClassicMail, but it isn't set up for your Windows account yet.\n\n\
             Open ClassicMail once, then try again.",
        );
        return MAPI_E_USER_ABORT;
    };
    let id = format!(
        "{}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0),
        std::process::id()
    );
    if let Err(code) = deliver(Path::new(&inbox), &req, &id) {
        log(&format!("send failed: couldn't write to {inbox} (MAPI error {code})"));
        return code;
    }
    // the calling program is in front: let ClassicMail bring its message window up over it
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    let up = setting("RunningMutex").is_some_and(|m| running(&m));
    if !up {
        let Some(exe) = setting("ExePath") else {
            log("send failed: no ExePath");
            return MAPI_E_FAILURE;
        };
        if let Err(e) = std::process::Command::new(&exe).spawn() {
            log(&format!("send failed: couldn't start {exe}: {e}"));
            return MAPI_E_FAILURE;
        }
    }
    log(&format!("send: delivered {id} (ClassicMail {})", if up { "running" } else { "started" }));
    SUCCESS_SUCCESS
}

// ---- entry points -------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPISendMailW(_session: usize, ui: usize, msg: *const MapiMessageW, flags: u32, _reserved: u32) -> u32 {
    log(&format!("MAPISendMailW flags {flags:#x}"));
    if msg.is_null() {
        return MAPI_E_FAILURE;
    }
    let m = unsafe { &*msg };
    let recips = (0..m.n_recip_count as usize)
        .filter_map(|i| {
            let r = unsafe { &*m.lp_recips.add(i) };
            (r.ul_recip_class != 0).then(|| Recip {
                class: r.ul_recip_class,
                name: unsafe { wide(r.lpsz_name) },
                address: unsafe { wide(r.lpsz_address) },
            })
        })
        .collect();
    let files = (0..m.n_file_count as usize)
        .map(|i| {
            let f = unsafe { &*m.lp_files.add(i) };
            File { path: unsafe { wide(f.lpsz_path_name) }, name: unsafe { wide(f.lpsz_file_name) } }
        })
        .collect();
    send(Request { subject: unsafe { wide(m.lpsz_subject) }, body: unsafe { wide(m.lpsz_note_text) }, recips, files }, ui)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPISendMail(_session: usize, ui: usize, msg: *const MapiMessage, flags: u32, _reserved: u32) -> u32 {
    log(&format!("MAPISendMail flags {flags:#x}"));
    if msg.is_null() {
        return MAPI_E_FAILURE;
    }
    let m = unsafe { &*msg };
    let recips = (0..m.n_recip_count as usize)
        .filter_map(|i| {
            let r = unsafe { &*m.lp_recips.add(i) };
            (r.ul_recip_class != 0).then(|| Recip {
                class: r.ul_recip_class,
                name: unsafe { ansi(r.lpsz_name) },
                address: unsafe { ansi(r.lpsz_address) },
            })
        })
        .collect();
    let files = (0..m.n_file_count as usize)
        .map(|i| {
            let f = unsafe { &*m.lp_files.add(i) };
            File { path: unsafe { ansi(f.lpsz_path_name) }, name: unsafe { ansi(f.lpsz_file_name) } }
        })
        .collect();
    send(Request { subject: unsafe { ansi(m.lpsz_subject) }, body: unsafe { ansi(m.lpsz_note_text) }, recips, files }, ui)
}

/// Older programs: "Send To › Mail Recipient" with a list of files.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPISendDocuments(ui: usize, delim: *const u8, paths: *const u8, names: *const u8, _reserved: u32) -> u32 {
    log("MAPISendDocuments");
    let d = unsafe { ansi(delim) }.chars().next().unwrap_or(';');
    let paths = unsafe { ansi(paths) };
    let names = unsafe { ansi(names) };
    let names: Vec<&str> = names.split(d).collect();
    let files = paths
        .split(d)
        .filter(|p| !p.is_empty())
        .enumerate()
        .map(|(i, p)| File { path: p.to_string(), name: names.get(i).unwrap_or(&"").to_string() })
        .collect();
    send(Request { subject: String::new(), body: String::new(), recips: vec![], files }, ui)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPILogon(
    _ui: usize,
    _profile: *const u8,
    _password: *const u8,
    flags: u32,
    _reserved: u32,
    session: *mut usize,
) -> u32 {
    log(&format!("MAPILogon flags {flags:#x}"));
    if !session.is_null() {
        unsafe { *session = 1 };
    }
    SUCCESS_SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPILogoff(_session: usize, _ui: usize, _flags: u32, _reserved: u32) -> u32 {
    SUCCESS_SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPIFreeBuffer(_pv: *mut c_void) -> u32 {
    SUCCESS_SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPIResolveName(
    _session: usize,
    _ui: usize,
    _name: *const u8,
    _flags: u32,
    _reserved: u32,
    _recip: *mut *mut c_void,
) -> u32 {
    log("MAPIResolveName (not supported)");
    MAPI_E_NOT_SUPPORTED
}

// The rest of Simple MAPI (reading the mailbox, the address book): not offered, but exported so a
// program that looks them all up finds them.
macro_rules! not_supported {
    ($($name:ident($($arg:ident),*);)*) => {$(
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn $name($($arg: usize),*) -> u32 {
            let _ = ($($arg),*);
            log(concat!(stringify!($name), " (not supported)"));
            MAPI_E_NOT_SUPPORTED
        }
    )*};
}
not_supported! {
    MAPIFindNext(a, b, c, d, e, f, g);
    MAPIReadMail(a, b, c, d, e, f);
    MAPISaveMail(a, b, c, d, e, f);
    MAPIDeleteMail(a, b, c, d, e);
    MAPIAddress(a, b, c, d, e, f, g, h, i, j, k);
    MAPIDetails(a, b, c, d, e);
}

/// Thunderbird's MAPI DLL answers this too; some callers check it.
#[unsafe(no_mangle)]
pub extern "system" fn GetMapiDllVersion() -> u32 {
    log("GetMapiDllVersion");
    94
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_its_settings_file() {
        let t = "InboxDir=C:\\data\\mapi-inbox\r\nexepath = C:\\Program Files\\ClassicMail\\classicmail.exe\r\nRunningMutex=\r\n";
        assert_eq!(ini_value(t, "InboxDir").as_deref(), Some("C:\\data\\mapi-inbox"));
        assert_eq!(ini_value(t, "ExePath").as_deref(), Some("C:\\Program Files\\ClassicMail\\classicmail.exe"));
        assert_eq!(ini_value(t, "RunningMutex"), None);
    }

    #[test]
    fn delivers_a_request_with_its_files() {
        let tmp = std::env::temp_dir().join(format!("cm-mapi-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let src = tmp.join("Globex Q3.docx");
        std::fs::write(&src, b"doc").unwrap();
        let inbox = tmp.join("inbox");
        let req = Request {
            subject: "Globex \"Q3\"".into(),
            body: "See attached.\r\n".into(),
            recips: vec![
                Recip { class: 1, name: "Eli".into(), address: "SMTP:eli@globex.example".into() },
                Recip { class: 2, name: "".into(), address: "tom@duvz.demo".into() },
            ],
            files: vec![
                File { path: src.to_string_lossy().into(), name: "".into() },
                File { path: src.to_string_lossy().into(), name: "Globex Q3.docx".into() },
            ],
        };
        let dir = deliver(&inbox, &req, "1-2").unwrap();
        assert!(dir.ends_with("1-2"));
        assert!(dir.join("Globex Q3.docx").exists() && dir.join("Globex Q3 (2).docx").exists());
        let json = std::fs::read_to_string(dir.join("request.json")).unwrap();
        assert_eq!(
            json,
            r#"{"subject":"Globex \"Q3\"","body":"See attached.\r\n","to":[{"name":"Eli","email":"eli@globex.example"}],"cc":[{"name":"","email":"tom@duvz.demo"}],"bcc":[],"files":["Globex Q3.docx","Globex Q3 (2).docx"]}"#
        );
        // a file that is gone: nothing half-delivered
        let bad = Request {
            subject: String::new(),
            body: String::new(),
            recips: vec![],
            files: vec![File { path: "C:\\nope\\x.pdf".into(), name: String::new() }],
        };
        assert_eq!(deliver(&inbox, &bad, "3-4").unwrap_err(), MAPI_E_ATTACHMENT_NOT_FOUND);
        assert!(!inbox.join("3-4.part").exists() && !inbox.join("3-4").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

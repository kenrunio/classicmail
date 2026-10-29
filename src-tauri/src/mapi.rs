//! ClassicMail as Windows' mail client: Word / Excel / PowerPoint's "Email" › "Send as Attachment"
//! (and any program's Simple MAPI "send mail") open a new ClassicMail message with the document
//! attached.
//!
//! * Tools › Options › Other turns it on: our Simple MAPI DLL (the crate `src-tauri/mapi`, built
//!   by build.rs for 64- and 32-bit Office and embedded here) is registered as the mail client:
//!   - for this user (HKCU\Software\Clients\Mail; a copy of the DLL in the data dir): what most
//!     programs follow; no admin needed.
//!   - for the computer (HKLM\Software\Clients\Mail, and Office Click-to-Run's own copy of it):
//!     what Word / Excel follow — they don't see the per-user one. Every program, 64- or 32-bit:
//!     the 64-bit DLL goes to System32 and the 32-bit one to SysWOW64 under one name, and the keys
//!     name `System32\ClassicMailMapi.dll` (Windows sends a 32-bit program asking for System32 to
//!     SysWOW64, so each loads the DLL it can). The per-user key names it too once it's there.
//!     One administrator prompt: ClassicMail starts itself elevated with `--mapi-machine on|off`
//!     (see `elevated_command`), which writes the DLLs and those keys and exits. A new version
//!     asks again only if a DLL itself changed.
//! * Options shows whether every piece is in place (`ok`: keys, DLL files, this user's mapi.ini
//!   naming this ClassicMail); turning it on, or Fix It, puts back whatever isn't.
//! * The DLL serves every Windows user, so it reads the settings of whoever is calling:
//!   `%LOCALAPPDATA%\ClassicMail\mapi.ini` (inbox, exe, mutex), written by that user's ClassicMail
//!   when it's turned on or (computer-wide registration) whenever it starts without one.
//! * Turning it off puts back the clients that were the defaults before, if still installed
//!   (else, for the computer, Outlook if installed; for this user, the computer's choice).
//! * The DLL drops each message into `<data>/mapi-inbox/<id>/` (request.json + copies of the
//!   files) and starts ClassicMail if it isn't running (`RunningMutex`).
//! * Here: a watcher sees new requests and tells the main window (`mail.mapiPending`), which takes
//!   them (`mail.mapi.take`, renamed `<id>.open` so each opens once) and opens a message window
//!   for each; that window reads its request with `mail.compose.readMapi`.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ctx::AppCtx,
    mail::compose::attachments::{FileInfo, file_info},
    rpc::{NoParams, Registry},
};

const CLIENT: &str = "ClassicMail";
const POLL: Duration = Duration::from_millis(700);
/// files of a message being written stay this long (a saved draft carries its own copy)
const KEEP: Duration = Duration::from_secs(7 * 24 * 3600);

pub fn register(r: &mut Registry) {
    r.add("app.getMailClient", get_mail_client);
    r.add("app.setMailClient", set_mail_client);
    r.add("mail.mapi.take", |ctx: Arc<AppCtx>, _: NoParams| async move { Ok(take(&inbox(&ctx))) });
    r.add("mail.compose.readMapi", read_request);
}

pub fn inbox(ctx: &AppCtx) -> PathBuf {
    ctx.data_dir.join("mapi-inbox")
}

/// Named per data dir, so a dev slot never answers for the installed app.
fn mutex_name(ctx: &AppCtx) -> String {
    use sha2::Digest;
    let h = sha2::Sha256::digest(ctx.data_dir.to_string_lossy().to_lowercase().as_bytes());
    format!("Local\\ClassicMail-MAPI-{}", h.iter().take(6).map(|b| format!("{b:02x}")).collect::<String>())
}

/// A test build (a debug exe from `scripts/dev.mjs`) never touches the mail client registration or
/// this user's `mapi.ini`: one run would point Word's "Send as Attachment" at the test build and
/// its throw-away data, away from the installed ClassicMail.
const TEST_BUILD: bool = cfg!(debug_assertions);

/// Start-up: say we're running, keep the registration current, watch the inbox.
pub fn start(ctx: Arc<AppCtx>) {
    #[cfg(windows)]
    {
        reg::hold_mutex(&mutex_name(&ctx));
        let r = if TEST_BUILD {
            Ok(())
        } else if reg::is_default() {
            // a new version / install folder: point the registration at this exe and DLL
            reg::enable(&ctx)
        } else if reg::machine_default_is_us() && !reg::user_settings_ok() {
            // another user made ClassicMail the computer's mail client: Word should reach this one
            reg::write_user_settings(&ctx)
        } else {
            Ok(())
        };
        if let Err(e) = r {
            log::warn!("mail client registration: {}", crate::logging::err(&e));
        }
    }
    tauri::async_runtime::spawn(async move {
        let dir = inbox(&ctx);
        let mut last_clean = None::<std::time::Instant>;
        loop {
            if last_clean.is_none_or(|t| t.elapsed() > Duration::from_secs(3600)) {
                clean_old(&dir);
                last_clean = Some(std::time::Instant::now());
            }
            if !pending(&dir).is_empty() {
                ctx.events.emit("mail.mapiPending", Value::Null);
            }
            tokio::time::sleep(POLL).await;
        }
    });
}

/// Requests the DLL finished writing that no window has taken yet.
fn pending(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| !n.to_string_lossy().contains('.')))
        .collect();
    out.sort();
    out
}

/// Each waiting request, renamed `<id>.open` so it's opened only once; the folders to open.
pub fn take(dir: &Path) -> Vec<String> {
    pending(dir)
        .into_iter()
        .filter_map(|p| {
            let open = p.with_extension("open");
            std::fs::rename(&p, &open).ok().map(|_| open.to_string_lossy().into_owned())
        })
        .collect()
}

fn clean_old(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let age = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).unwrap_or_default();
        let name = e.file_name().to_string_lossy().into_owned();
        // `.part`: a send that died half-way
        let limit = if name.ends_with(".part") { Duration::from_secs(24 * 3600) } else { KEEP };
        if name.contains('.') && age > limit {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

#[derive(Deserialize, Serialize, Default)]
struct Who {
    name: String,
    email: String,
}

#[derive(Deserialize, Default)]
struct RequestFile {
    #[serde(default)]
    subject: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    to: Vec<Who>,
    #[serde(default)]
    cc: Vec<Who>,
    #[serde(default)]
    bcc: Vec<Who>,
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Serialize)]
pub struct MapiMessage {
    subject: String,
    /// plain text
    body: String,
    to: Vec<Who>,
    cc: Vec<Who>,
    bcc: Vec<Who>,
    attachments: Vec<FileInfo>,
}

#[derive(Deserialize)]
pub struct DirParam {
    dir: String,
}

async fn read_request(ctx: Arc<AppCtx>, p: DirParam) -> anyhow::Result<MapiMessage> {
    let gone = || crate::user_err!("not_found", "The message from the other program is no longer there.");
    // only folders of our own inbox
    let root = std::fs::canonicalize(inbox(&ctx)).map_err(|_| gone())?;
    let dir = std::fs::canonicalize(&p.dir).map_err(|_| gone())?;
    if dir.parent() != Some(root.as_path()) {
        return Err(crate::user_err!("bad_params", "not a request folder"));
    }
    let text = std::fs::read_to_string(dir.join("request.json")).map_err(|_| gone())?;
    let r: RequestFile = serde_json::from_str(&text)?;
    // names only, kept inside the folder
    let paths = r
        .files
        .iter()
        .filter(|f| !f.contains(['/', '\\']) && *f != ".." && !f.is_empty())
        .map(|f| dir.join(f).to_string_lossy().into_owned())
        .collect();
    let named = |v: Vec<Who>| v.into_iter().filter(|w| !w.email.trim().is_empty() || !w.name.trim().is_empty()).collect();
    Ok(MapiMessage {
        subject: r.subject,
        body: r.body,
        to: named(r.to),
        cc: named(r.cc),
        bcc: named(r.bcc),
        attachments: file_info(paths).await.into_iter().filter(|f| f.exists).collect(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientState {
    /// the mail client for this user
    enabled: bool,
    /// ... and for the computer, which Word / Excel go by
    office: bool,
    /// every piece in place: both registrations, the DLL files they name, and this user's
    /// mapi.ini naming this ClassicMail (when false, turning it on again repairs it)
    ok: bool,
}

async fn get_mail_client(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<ClientState> {
    #[cfg(windows)]
    {
        let enabled = reg::is_default();
        let office = reg::machine_is_ours();
        Ok(ClientState { enabled, office, ok: enabled && office && reg::user_is_ours(&ctx) })
    }
    #[cfg(not(windows))]
    {
        let _ = ctx;
        Ok(ClientState { enabled: false, office: false, ok: false })
    }
}

#[derive(Deserialize)]
struct Enabled {
    enabled: bool,
}

/// On (also: repair): for the computer if anything there is missing or old (one administrator
/// prompt), then for this user. Off: both back.
async fn set_mail_client(ctx: Arc<AppCtx>, p: Enabled) -> anyhow::Result<ClientState> {
    #[cfg(windows)]
    {
        if TEST_BUILD {
            return Err(crate::user_err!("test_build", "A test build doesn't change the mail client: use the installed ClassicMail."));
        }
        if p.enabled {
            reg::enable(&ctx)?;
            if !reg::machine_is_ours() {
                tokio::task::spawn_blocking(|| reg::run_elevated(&["on"])).await??;
                // the per-user key can name the System32 DLL now (it serves 32-bit programs too)
                reg::enable(&ctx)?;
            }
        } else {
            reg::disable(&ctx)?;
            if reg::machine_mentions_us() {
                tokio::task::spawn_blocking(|| reg::run_elevated(&["off"])).await??;
            }
        }
        get_mail_client(ctx, NoParams {}).await
    }
    #[cfg(not(windows))]
    {
        let _ = (ctx, p);
        Err(crate::user_err!("unsupported", "Only on Windows."))
    }
}

/// `classicmail.exe --mapi-machine on|off`, started elevated by `set_mail_client`: the
/// computer-wide registration (with this exe's own copy of the DLL). Returns the process exit code; None for a normal start.
pub fn elevated_command() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("--mapi-machine") {
        return None;
    }
    #[cfg(windows)]
    {
        let r = match args.get(2).map(String::as_str) {
            Some("on") => reg::machine_on(),
            Some("off") => reg::machine_off(),
            _ => Err(anyhow::anyhow!("usage: --mapi-machine on|off")),
        };
        Some(if r.is_ok() { 0 } else { 2 })
    }
    #[cfg(not(windows))]
    Some(2)
}

#[cfg(windows)]
mod reg {
    use super::*;
    use anyhow::Context;
    use windows::{
        Win32::{
            Foundation::{CloseHandle, ERROR_CANCELLED, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
            System::{
                Registry::{
                    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
                    RRF_SUBKEY_WOW6432KEY, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW,
                    RegSetValueExW,
                },
                Threading::{CreateMutexW, GetExitCodeProcess, INFINITE, WaitForSingleObject},
            },
            UI::{
                Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
                WindowsAndMessaging::SW_HIDE,
            },
        },
        core::{HSTRING, PCWSTR},
    };

    const MAIL: &str = "Software\\Clients\\Mail";
    /// Office Click-to-Run's own HKLM\Software\Clients\Mail, which Word / Excel see instead
    const C2R_REGISTRY: &str = "SOFTWARE\\Microsoft\\Office\\ClickToRun\\REGISTRY\\MACHINE";

    pub fn hold_mutex(name: &str) {
        // held (never closed) for as long as the app runs
        let _ = unsafe { CreateMutexW(None, false, &HSTRING::from(name)) };
    }

    fn get(root: HKEY, key: &str, value: &str, wow32: bool) -> Option<String> {
        let mut buf = vec![0u16; 2048];
        let mut size = (buf.len() * 2) as u32;
        let flags = if wow32 { RRF_RT_REG_SZ | RRF_SUBKEY_WOW6432KEY } else { RRF_RT_REG_SZ };
        let value = HSTRING::from(value);
        let name = if value.is_empty() { PCWSTR::null() } else { PCWSTR(value.as_ptr()) };
        let r = unsafe { RegGetValueW(root, &HSTRING::from(key), name, flags, None, Some(buf.as_mut_ptr().cast()), Some(&mut size)) };
        (r == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    }

    fn key_exists(root: HKEY, key: &str) -> bool {
        let mut h = HKEY::default();
        let ok = unsafe { RegOpenKeyExW(root, &HSTRING::from(key), None, KEY_READ, &mut h) } == ERROR_SUCCESS;
        if ok {
            unsafe {
                let _ = RegCloseKey(h);
            }
        }
        ok
    }

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    fn open_write(key: &str) -> anyhow::Result<Key> {
        open_write_in(HKEY_CURRENT_USER, key)
    }

    fn open_write_in(root: HKEY, key: &str) -> anyhow::Result<Key> {
        let mut h = HKEY::default();
        unsafe { RegCreateKeyExW(root, &HSTRING::from(key), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut h, None) }
            .ok()
            .with_context(|| format!("can't write {key}"))?;
        Ok(Key(h))
    }

    fn delete_default(k: &Key) -> anyhow::Result<()> {
        let r = unsafe { RegDeleteValueW(k.0, PCWSTR::null()) };
        if r != ERROR_SUCCESS && r != ERROR_FILE_NOT_FOUND {
            r.ok().context("can't reset the default mail client")?;
        }
        Ok(())
    }

    fn set(k: &Key, value: &str, data: &str) -> anyhow::Result<()> {
        let w: Vec<u16> = data.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = unsafe { std::slice::from_raw_parts(w.as_ptr().cast::<u8>(), w.len() * 2) };
        let name = HSTRING::from(value);
        let name = if value.is_empty() { PCWSTR::null() } else { PCWSTR(name.as_ptr()) };
        unsafe { RegSetValueExW(k.0, name, None, REG_SZ, Some(bytes)) }.ok().with_context(|| format!("can't set {value}"))?;
        Ok(())
    }

    pub fn is_default() -> bool {
        get(HKEY_CURRENT_USER, MAIL, "", false).is_some_and(|v| v == CLIENT)
    }

    /// 32-bit Office loads only a 32-bit DLL (and 64-bit only a 64-bit one).
    fn office_is_32bit() -> bool {
        let x86 = |v: Option<String>| v.is_some_and(|v| v.eq_ignore_ascii_case("x86"));
        if let Some(p) = get(HKEY_LOCAL_MACHINE, "SOFTWARE\\Microsoft\\Office\\ClickToRun\\Configuration", "Platform", false) {
            return x86(Some(p));
        }
        for ver in ["16.0", "15.0", "14.0"] {
            let key = format!("SOFTWARE\\Microsoft\\Office\\{ver}\\Outlook");
            for wow32 in [false, true] {
                if let Some(b) = get(HKEY_LOCAL_MACHINE, &key, "Bitness", wow32) {
                    return x86(Some(b));
                }
            }
        }
        false
    }

    const X64: &[u8] = include_bytes!(env!("CLASSICMAIL_MAPI_X64"));
    const X86: &[u8] = include_bytes!(env!("CLASSICMAIL_MAPI_X86"));
    /// The name of the computer-wide copies (System32: 64-bit, SysWOW64: 32-bit).
    const SYSTEM_DLL: &str = "ClassicMailMapi.dll";

    /// The DLL for this computer's Office, and its file name: the per-user copy, used until the
    /// computer-wide copies are in place.
    fn embedded_dll() -> (&'static [u8], &'static str) {
        if office_is_32bit() { (X86, "ClassicMailMapi32.dll") } else { (X64, "ClassicMailMapi64.dll") }
    }

    /// Windows' folders for 64-bit programs (System32) and, on 64-bit Windows, 32-bit ones (SysWOW64).
    fn system_dirs() -> (PathBuf, Option<PathBuf>) {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        let wow = root.join("SysWOW64");
        (root.join("System32"), wow.is_dir().then_some(wow))
    }

    /// What every registration names once the computer-wide copies are there.
    fn system_dll() -> PathBuf {
        system_dirs().0.join(SYSTEM_DLL)
    }

    /// Both computer-wide copies are there and are this version's.
    fn system_dlls_current() -> bool {
        let (sys, wow) = system_dirs();
        std::fs::read(sys.join(SYSTEM_DLL)).ok().as_deref() == Some(X64)
            && wow.is_none_or(|w| std::fs::read(w.join(SYSTEM_DLL)).ok().as_deref() == Some(X86))
    }

    /// This Office's DLL into `dir` (the per-user copy).
    fn put_dll(dir: &Path) -> anyhow::Result<PathBuf> {
        let (bytes, name) = embedded_dll();
        put_file(dir, name, bytes)
    }

    /// `bytes` as `dir\name`. A copy a running program has loaded can't be overwritten but can be
    /// renamed, so it's moved aside and deleted once nothing has it loaded.
    fn put_file(dir: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        // only our own leftovers (this may be System32)
        let old = format!("{}.old-", name.to_lowercase());
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().to_lowercase().starts_with(&old) {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let path = dir.join(name);
        if std::fs::read(&path).ok().as_deref() != Some(bytes) && std::fs::write(&path, bytes).is_err() {
            let aside = dir.join(format!("{name}.old-{}", crate::clock::now_ms()));
            std::fs::rename(&path, &aside).context("can't replace the mail DLL")?;
            std::fs::write(&path, bytes).context("can't write the mail DLL")?;
        }
        Ok(path)
    }

    /// Where earlier versions kept the computer-wide copy (removed when it's set up again).
    fn shared_dir() -> PathBuf {
        std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\ProgramData")).join(CLIENT)
    }

    /// Where the DLL finds this Windows user's settings (the same path in mapi/src/lib.rs).
    fn user_settings_path() -> Option<PathBuf> {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
        Some(local.join(CLIENT).join("mapi.ini"))
    }

    /// There, and naming a ClassicMail that still exists.
    pub fn user_settings_ok() -> bool {
        user_settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("ExePath=").map(str::to_string)))
            .is_some_and(|exe| Path::new(&exe).is_file())
    }

    /// This user's mapi.ini names this ClassicMail: its exe, its inbox and its running signal.
    fn user_settings_current(ctx: &AppCtx) -> bool {
        let Some(text) = user_settings_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return false };
        let value = |k: &str| text.lines().find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string()));
        let exe = std::env::current_exe().ok();
        value("ExePath=").is_some_and(|v| exe.is_some_and(|e| Path::new(&v) == e))
            && value("InboxDir=").is_some_and(|v| Path::new(&v) == super::inbox(ctx))
            && value("RunningMutex=").is_some_and(|v| v == super::mutex_name(ctx))
    }

    /// For this user, everything in place: ClassicMail is the client, its key names a DLL that is
    /// there and current, and mapi.ini names this ClassicMail.
    pub fn user_is_ours(ctx: &AppCtx) -> bool {
        let dll = get(HKEY_CURRENT_USER, &format!("{MAIL}\\{CLIENT}"), "DLLPathEx", false);
        let dll_ok = dll.is_some_and(|p| {
            let p = Path::new(&p);
            if p == system_dll() {
                system_dlls_current()
            } else {
                let (bytes, _) = embedded_dll();
                std::fs::read(p).ok().as_deref() == Some(bytes)
            }
        });
        is_default() && dll_ok && user_settings_current(ctx)
    }

    /// What the DLL reads when this user's programs send mail. Not the registry: Office
    /// Click-to-Run hides our HKCU\Software\Clients key from Word and Excel.
    pub fn write_user_settings(ctx: &AppCtx) -> anyhow::Result<()> {
        let path = user_settings_path().context("no LOCALAPPDATA")?;
        let inbox = super::inbox(ctx);
        std::fs::create_dir_all(&inbox)?;
        let exe = std::env::current_exe()?;
        let ini = format!("InboxDir={}\r\nExePath={}\r\nRunningMutex={}\r\n", inbox.display(), exe.display(), super::mutex_name(ctx));
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, ini).context("can't write the mail settings")?;
        Ok(())
    }

    /// For this user: the computer-wide DLL once it's there (it serves 32-bit programs too), else
    /// a copy for this Office in the data dir.
    pub fn enable(ctx: &AppCtx) -> anyhow::Result<()> {
        let dll = if system_dlls_current() { system_dll() } else { put_dll(&ctx.data_dir.join("mapi"))? };
        write_user_settings(ctx)?;
        let exe = std::env::current_exe()?;
        let client_key = format!("{MAIL}\\{CLIENT}");
        let k = open_write(&client_key)?;
        set(&k, "", CLIENT)?;
        set(&k, "DLLPath", &dll.to_string_lossy())?;
        set(&k, "DLLPathEx", &dll.to_string_lossy())?;
        // 1.16.0 kept the DLL's settings here; they're in the user's mapi.ini now
        for old in ["ExePath", "InboxDir", "RunningMutex"] {
            unsafe {
                let _ = RegDeleteValueW(k.0, &HSTRING::from(old));
            }
        }
        // how "Set as default" lists it, and a mailto: handler isn't claimed (Windows' Default apps decides that)
        set(&open_write(&format!("{client_key}\\DefaultIcon"))?, "", &format!("{},0", exe.to_string_lossy()))?;
        let mail = open_write(MAIL)?;
        let before = get(HKEY_CURRENT_USER, MAIL, "", false);
        if before.as_deref() != Some(CLIENT) {
            set(&k, "PreviousDefault", before.as_deref().unwrap_or(""))?;
        }
        set(&mail, "", CLIENT)?;
        log::info!("ClassicMail is now the mail client (was {before:?}), DLL {}", dll.display());
        Ok(())
    }

    pub fn disable(_ctx: &AppCtx) -> anyhow::Result<()> {
        let client_key = format!("{MAIL}\\{CLIENT}");
        let prev = get(HKEY_CURRENT_USER, &client_key, "PreviousDefault", false).filter(|p| !p.is_empty() && p != CLIENT);
        // only a client that's still installed; otherwise the machine's default applies
        let prev = prev
            .filter(|p| key_exists(HKEY_CURRENT_USER, &format!("{MAIL}\\{p}")) || key_exists(HKEY_LOCAL_MACHINE, &format!("{MAIL}\\{p}")));
        if is_default() {
            let mail = open_write(MAIL)?;
            match &prev {
                Some(p) => set(&mail, "", p)?,
                None => delete_default(&mail)?,
            }
        }
        log::info!("ClassicMail is no longer the mail client (now {:?})", prev.as_deref().unwrap_or("the machine default"));
        Ok(())
    }

    // ---- for the computer (HKLM; run elevated) ----

    /// Where the computer's mail client is set: Windows' key, plus Office Click-to-Run's copy.
    fn machine_roots() -> Vec<String> {
        let mut roots = vec![MAIL.to_string()];
        if key_exists(HKEY_LOCAL_MACHINE, C2R_REGISTRY) {
            roots.push(format!("{C2R_REGISTRY}\\{MAIL}"));
        }
        roots
    }

    /// The computer's mail client is ClassicMail (whoever set it up).
    pub fn machine_default_is_us() -> bool {
        get(HKEY_LOCAL_MACHINE, MAIL, "", false).is_some_and(|v| v == CLIENT)
    }

    /// Every program (Word / Excel included) reaches this version's DLL: both computer-wide
    /// copies are current and every computer-wide key names them.
    pub fn machine_is_ours() -> bool {
        let dll = system_dll();
        system_dlls_current()
            && machine_roots().iter().all(|root| {
                get(HKEY_LOCAL_MACHINE, root, "", false).is_some_and(|v| v == CLIENT)
                    && ["DLLPath", "DLLPathEx"]
                        .iter()
                        .all(|v| get(HKEY_LOCAL_MACHINE, &format!("{root}\\{CLIENT}"), v, false).is_some_and(|p| Path::new(&p) == dll))
            })
    }

    pub fn machine_mentions_us() -> bool {
        machine_roots().iter().any(|root| {
            get(HKEY_LOCAL_MACHINE, root, "", false).is_some_and(|v| v == CLIENT)
                || key_exists(HKEY_LOCAL_MACHINE, &format!("{root}\\{CLIENT}"))
        })
    }

    pub fn machine_on() -> anyhow::Result<()> {
        let (sys, wow) = system_dirs();
        put_file(&sys, SYSTEM_DLL, X64)?;
        if let Some(w) = &wow {
            put_file(w, SYSTEM_DLL, X86)?;
        }
        // the copy earlier versions kept in %ProgramData%
        let _ = std::fs::remove_dir_all(shared_dir());
        let dll = system_dll();
        let dll = &*dll.to_string_lossy();
        for root in machine_roots() {
            let ours = open_write_in(HKEY_LOCAL_MACHINE, &format!("{root}\\{CLIENT}"))?;
            set(&ours, "", CLIENT)?;
            set(&ours, "DLLPath", dll)?;
            set(&ours, "DLLPathEx", dll)?;
            let before = get(HKEY_LOCAL_MACHINE, &root, "", false);
            if before.as_deref() != Some(CLIENT) {
                set(&ours, "PreviousDefault", before.as_deref().unwrap_or(""))?;
            }
            set(&open_write_in(HKEY_LOCAL_MACHINE, &root)?, "", CLIENT)?;
        }
        Ok(())
    }

    pub fn machine_off() -> anyhow::Result<()> {
        for root in machine_roots() {
            let ours = format!("{root}\\{CLIENT}");
            if get(HKEY_LOCAL_MACHINE, &root, "", false).is_some_and(|v| v == CLIENT) {
                // the one before us if it's still installed, else Outlook if it is: never no client at all
                let installed = |p: &String| !p.is_empty() && p != CLIENT && key_exists(HKEY_LOCAL_MACHINE, &format!("{root}\\{p}"));
                let prev = get(HKEY_LOCAL_MACHINE, &ours, "PreviousDefault", false)
                    .filter(installed)
                    .or_else(|| Some("Microsoft Outlook".to_string()).filter(installed));
                let k = open_write_in(HKEY_LOCAL_MACHINE, &root)?;
                match prev {
                    Some(p) => set(&k, "", &p)?,
                    None => delete_default(&k)?,
                }
            }
            let r = unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, &HSTRING::from(ours.as_str())) };
            if r != ERROR_SUCCESS && r != ERROR_FILE_NOT_FOUND {
                r.ok().context("can't remove ClassicMail's computer-wide entry")?;
            }
        }
        // the DLLs, unless a running program still has one loaded
        let _ = std::fs::remove_dir_all(shared_dir());
        let (sys, wow) = system_dirs();
        for d in std::iter::once(sys).chain(wow) {
            let _ = std::fs::remove_file(d.join(SYSTEM_DLL));
        }
        Ok(())
    }

    /// This exe with `--mapi-machine <args>`, as administrator (Windows asks the user); waits.
    pub fn run_elevated(args: &[&str]) -> anyhow::Result<()> {
        let exe = HSTRING::from(std::env::current_exe()?.as_os_str());
        let params =
            std::iter::once("--mapi-machine".to_string()).chain(args.iter().map(|a| format!("\"{a}\""))).collect::<Vec<_>>().join(" ");
        let params = HSTRING::from(params);
        let verb = HSTRING::from("runas");
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(exe.as_ptr()),
            lpParameters: PCWSTR(params.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
            if e.code() == ERROR_CANCELLED.to_hresult() {
                return Err(crate::user_err!(
                    "cancelled",
                    "Word, Excel and other Office programs need administrator permission to use ClassicMail, so they'll keep using their current mail program."
                ));
            }
            return Err(e).context("couldn't ask for administrator permission");
        }
        let mut code = 1u32;
        unsafe {
            WaitForSingleObject(info.hProcess, INFINITE);
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
        }
        anyhow::ensure!(code == 0, "the computer-wide mail client setting couldn't be changed (code {code})");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn requests_are_taken_once_and_read_from_the_inbox_only() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let dir = inbox(&ctx);
        let req = dir.join("1700000000000-42");
        std::fs::create_dir_all(&req).unwrap();
        std::fs::create_dir_all(dir.join("1700000000001-42.part")).unwrap();
        std::fs::write(req.join("Q3 results.docx"), b"doc").unwrap();
        std::fs::write(
            req.join("request.json"),
            r#"{"subject":"Q3 results","body":"See attached.","to":[{"name":"Eli","email":"eli@globex.example"},{"name":"","email":""}],"cc":[],"bcc":[],"files":["Q3 results.docx","..\\..\\secret.txt"]}"#,
        )
        .unwrap();
        assert_eq!(pending(&dir).len(), 1, "a half-written request isn't offered");
        let taken = take(&dir);
        assert_eq!(taken.len(), 1);
        assert!(take(&dir).is_empty(), "each request opens once");

        let reg = crate::rpc::build();
        let m = reg.call(ctx.clone(), "mail.compose.readMapi", serde_json::json!({ "dir": taken[0] })).await.unwrap();
        assert_eq!(m["subject"], "Q3 results");
        assert_eq!(m["to"].as_array().unwrap().len(), 1);
        let att = m["attachments"].as_array().unwrap();
        assert_eq!(att.len(), 1, "{att:?}");
        assert_eq!(att[0]["name"], "Q3 results.docx");

        // a folder outside the inbox is refused
        let e =
            reg.call(ctx.clone(), "mail.compose.readMapi", serde_json::json!({ "dir": ctx.data_dir.to_string_lossy() })).await.unwrap_err();
        assert!(e.to_string().contains("not a request folder"), "{e}");
    }
}

//! What ClassicMail lets you do with an attachment from the internet, Outlook-style:
//! * file types Outlook blocks ("Level 1": programs, scripts, shortcuts, disk images...) can't be
//!   opened from the app at all (Save As still works, like Outlook with the registry override);
//! * other types that aren't plainly documents/pictures ask first ("Opening Mail Attachment");
//! * every file written to disk gets the Mark of the Web (`Zone.Identifier`, zone 3 = Internet), so
//!   Office opens it in Protected View / blocks macros and SmartScreen checks programs.
//!
//! File names come from the sender: `safe_file_name` makes them safe to use as a path (no
//! separators / device names / trailing dots or spaces that Windows silently drops, which would
//! turn `x.exe.` into `x.exe` after the extension check) and removes bidi controls (the
//! `invoice<U+202E>fdp.exe` → "invoiceexe.pdf" trick).

use std::path::Path;

/// Outlook's Level 1 list (+ disk images / newer script hosts that are used to bypass MOTW).
const BLOCKED: &[&str] = &[
    "ade",
    "adp",
    "app",
    "application",
    "appref-ms",
    "appx",
    "appxbundle",
    "asp",
    "aspx",
    "asx",
    "bas",
    "bat",
    "bgi",
    "cab",
    "cer",
    "chm",
    "cmd",
    "cnt",
    "com",
    "cpl",
    "crt",
    "csh",
    "der",
    "diagcab",
    "exe",
    "fxp",
    "gadget",
    "grp",
    "hlp",
    "hpj",
    "hta",
    "htc",
    "img",
    "inf",
    "ins",
    "iso",
    "isp",
    "its",
    "jar",
    "jnlp",
    "js",
    "jse",
    "ksh",
    "library-ms",
    "lnk",
    "mad",
    "maf",
    "mag",
    "mam",
    "maq",
    "mar",
    "mas",
    "mat",
    "mau",
    "mav",
    "maw",
    "mcf",
    "mda",
    "mdb",
    "mde",
    "mdt",
    "mdw",
    "mdz",
    "msc",
    "msh",
    "msh1",
    "msh2",
    "mshxml",
    "msh1xml",
    "msh2xml",
    "msi",
    "msix",
    "msixbundle",
    "msp",
    "mst",
    "msu",
    "ops",
    "osd",
    "pcd",
    "pif",
    "pl",
    "plg",
    "prf",
    "prg",
    "printerexport",
    "ps1",
    "ps1xml",
    "ps2",
    "ps2xml",
    "psc1",
    "psc2",
    "psd1",
    "psdm1",
    "psm1",
    "pst",
    "py",
    "pyc",
    "pyo",
    "pyw",
    "pyz",
    "pyzw",
    "reg",
    "scf",
    "scr",
    "sct",
    "search-ms",
    "settingcontent-ms",
    "shb",
    "shs",
    "theme",
    "tmp",
    "udl",
    "url",
    "vb",
    "vbe",
    "vbp",
    "vbs",
    "vhd",
    "vhdx",
    "vsmacros",
    "vsw",
    "webpnp",
    "website",
    "ws",
    "wsb",
    "wsc",
    "wsf",
    "wsh",
    "xbap",
    "xll",
    "xnk",
    "appinstaller",
];

/// Opened without asking: documents and media that their usual programs open safely (Office
/// formats without macros open in Protected View thanks to the Mark of the Web).
const SAFE: &[&str] = &[
    "pdf", "txt", "csv", "rtf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "pps", "ppsx", "odt", "ods", "odp", "png", "jpg", "jpeg",
    "gif", "bmp", "tif", "tiff", "webp", "heic", "heif", "ico", "mp3", "mp4", "m4a", "wav", "wma", "wmv", "mov", "avi", "ics", "vcf",
    "eml", "msg",
];

/// Lower-case extension of a (sanitised) file name, "" when there is none.
pub fn extension(name: &str) -> String {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn is_blocked(name: &str) -> bool {
    let ext = extension(&safe_file_name(name));
    // no extension at all: Windows can't open it anyway; a name of only dots/spaces → "attachment"
    BLOCKED.contains(&ext.as_str())
}

pub fn needs_confirmation(name: &str) -> bool {
    let ext = extension(&safe_file_name(name));
    !SAFE.contains(&ext.as_str())
}

/// Ok when `name` may be opened now; `unsafe` (blocked type) or `confirm_open` (ask the user,
/// then call again with `confirmed`) otherwise.
pub fn check_open(name: &str, confirmed: bool) -> anyhow::Result<()> {
    if is_blocked(name) {
        return Err(crate::user_err!(
            "unsafe",
            "ClassicMail blocked access to the following potentially unsafe attachment: {}. Use Save As to save it to disk if you trust the sender.",
            safe_file_name(name)
        ));
    }
    if needs_confirmation(name) && !confirmed {
        return Err(crate::user_err!(
            "confirm_open",
            "You should only open attachments from a trustworthy source.\n\nAttachment: {}\n\nWould you like to open the file?",
            safe_file_name(name)
        ));
    }
    Ok(())
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// A sender-supplied name without bidi controls (for display and storage).
pub fn display_name(name: &str) -> String {
    name.chars().filter(|c| !is_bidi_control(*c)).collect()
}

/// A sender-supplied file name made safe to create on Windows (see the module docs).
pub fn safe_file_name(name: &str) -> String {
    let s: String =
        name.chars().filter(|c| !is_bidi_control(*c)).map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    // Windows drops trailing dots and spaces; leading ones hide the name
    let s = s.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let mut s: String = s.chars().take(120).collect();
    // truncation can leave a trailing dot / space again
    while s.ends_with(['.', ' ']) {
        s.pop();
    }
    if s.is_empty() {
        return "attachment".into();
    }
    let stem = s.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if reserved { format!("_{s}") } else { s }
}

/// Mark a file we wrote as downloaded from the internet (Zone.Identifier stream, zone 3). Best effort.
pub fn mark_from_internet(path: &Path) {
    #[cfg(windows)]
    {
        let mut ads = path.as_os_str().to_owned();
        ads.push(":Zone.Identifier");
        if let Err(e) = std::fs::write(&ads, "[ZoneTransfer]\r\nZoneId=3\r\n") {
            log::warn!("can't mark a saved file as from the internet: {}", crate::logging::redact(&e.to_string()));
        }
    }
    #[cfg(not(windows))]
    let _ = path;
}

/// Write an attachment and mark it as from the internet.
pub fn write_marked(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)?;
    mark_from_internet(path);
    Ok(())
}

/// Hand a saved attachment to Windows: its default program, or (`choose`) the "Open with" chooser.
pub fn launch(app: Option<&tauri::AppHandle>, path: &str, name: &str, choose: bool) -> anyhow::Result<()> {
    let Some(app) = app else { return Ok(()) };
    if choose {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("rundll32.exe")
                .arg("shell32.dll,OpenAs_RunDLL")
                .raw_arg(path)
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                .spawn()
                .map_err(|e| crate::user_err!("open", "Can't open {name}: {e}"))?;
            return Ok(());
        }
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(path.to_string(), None::<&str>)
        .map_err(|e| crate::user_err!("open", "Can't open {name}: {e}. Use Open With… to pick a program, or Save As."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_on_windows() {
        assert_eq!(safe_file_name("report.pdf"), "report.pdf");
        assert_eq!(safe_file_name("..\\..\\Windows\\evil.exe"), "_.._Windows_evil.exe");
        assert_eq!(safe_file_name("x/../../y.txt"), "x_.._.._y.txt");
        assert_eq!(safe_file_name("C:evil.txt:stream.exe"), "C_evil.txt_stream.exe");
        // trailing dots / spaces would be dropped by Windows after our extension check
        assert_eq!(safe_file_name("evil.exe. ."), "evil.exe");
        assert_eq!(safe_file_name("evil.exe   "), "evil.exe");
        assert_eq!(safe_file_name(" . "), "attachment");
        // bidi override: "invoice<RLO>fdp.exe" shows as "invoiceexe.pdf"
        assert_eq!(safe_file_name("invoice\u{202E}fdp.exe"), "invoicefdp.exe");
        assert_eq!(display_name("invoice\u{202E}fdp.exe"), "invoicefdp.exe");
        assert_eq!(safe_file_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_file_name("com1"), "_com1");
        assert_eq!(safe_file_name("console.txt"), "console.txt");
        assert_eq!(safe_file_name("דוח רבעוני.pdf"), "דוח רבעוני.pdf");
        let long = format!("{}.{}", "a".repeat(119), "exe");
        assert!(!safe_file_name(&long).ends_with('.'));
    }

    #[test]
    fn open_policy() {
        for blocked in [
            "setup.exe",
            "x.JS",
            "a.hta",
            "b.lnk",
            "c.iso",
            "d.vhdx",
            "e.url",
            "f.scr",
            "g.ps1",
            "h.exe.",
            "i.exe ",
            "j\u{202E}fdp.exe",
            "k.appref-ms",
            "l.library-ms",
        ] {
            assert!(is_blocked(blocked), "{blocked}");
            let e = check_open(blocked, true).unwrap_err();
            assert_eq!(e.downcast_ref::<crate::error::UserError>().unwrap().code, "unsafe");
        }
        for ask in ["page.html", "a.htm", "x.svg", "m.docm", "m.xlsm", "z.zip", "noext", "x.7z", "y.one"] {
            assert!(!is_blocked(ask) && needs_confirmation(ask), "{ask}");
            let e = check_open(ask, false).unwrap_err();
            assert_eq!(e.downcast_ref::<crate::error::UserError>().unwrap().code, "confirm_open");
            assert!(check_open(ask, true).is_ok());
        }
        for ok in ["r.pdf", "R.PDF", "p.jpg", "d.docx", "s.xlsx", "דוח.pdf"] {
            assert!(check_open(ok, false).is_ok(), "{ok}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn written_files_carry_the_mark_of_the_web() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.docx");
        write_marked(&p, b"x").unwrap();
        let mut ads = p.as_os_str().to_owned();
        ads.push(":Zone.Identifier");
        let z = std::fs::read_to_string(&ads).unwrap();
        assert!(z.contains("ZoneId=3"), "{z}");
    }
}

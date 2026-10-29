//! What may go into the log file (%LOCALAPPDATA%\com.ekgir.classicmail\logs). The log never holds
//! the user's content — subjects, bodies, addresses, names, search words, file names, links from
//! mail, AI text, tokens — only ids, counts, method names, error codes, HTTP statuses and Google's
//! reason codes. `err` is the log form of an error (the UI keeps the full message); `redact` masks
//! any free text that still has to be logged.

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::{error::UserError, google::client::ApiError};

/// An error for the log: a user error's code, a Google error's status and reason, else the kind of
/// the root cause and the message with addresses, links, paths and quoted text masked.
pub fn err(e: &anyhow::Error) -> String {
    if let Some(u) = e.downcast_ref::<UserError>() {
        return format!("user error {}", u.code);
    }
    if let Some(a) = e.downcast_ref::<ApiError>() {
        return api_err(a);
    }
    format!("{} error: {}", kind(e), redact(&format!("{e:#}")))
}

/// An `ApiError` for the log. Google's message can echo the request (an address, a label name).
pub fn api_err(a: &ApiError) -> String {
    match a {
        ApiError::Http { status, reason, .. } => format!("Google API error {status} ({})", reason.as_deref().unwrap_or("no reason")),
        ApiError::Network(m) => format!("network error: {}", redact(m)),
        // "<address> needs to sign in again"
        ApiError::Auth(_) => "not signed in".into(),
    }
}

/// What kind of failure the root cause is (the message alone may be masked beyond recognition).
fn kind(e: &anyhow::Error) -> String {
    let root = e.root_cause();
    if let Some(io) = root.downcast_ref::<std::io::Error>() {
        format!("io ({:?})", io.kind())
    } else if root.is::<rusqlite::Error>() {
        "database".into()
    } else if root.is::<serde_json::Error>() {
        "json".into()
    } else if root.is::<reqwest::Error>() {
        "http".into()
    } else if root.is::<keyring::Error>() {
        "keyring".into()
    } else if root.is::<crate::google::imap::ImapError>() {
        "imap".into()
    } else {
        "internal".into()
    }
}

static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\b[A-Za-z][A-Za-z0-9+.-]*://[^\s"'`<>()\[\]{}]*"#).expect("url pattern"));
static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}").expect("email pattern"));
/// `C:\…`, `\\server\share\…` (to the next character a file name can't hold: names have spaces)
/// and `/a/b…` after a space, `=` or `(`.
static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\b[A-Za-z]:[\\/][^\n"<>|:*?]*|\\\\[^\n"<>|:*?]+|(?:^|[\s=(])/[^\s/"<>]+/[^\s"<>]*"#).expect("path pattern")
});
/// "…", `…`, “…”, and '…' that opens after a non-letter (not the apostrophe in "can't").
static QUOTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""(?:[^"\\\n]|\\.)*"|`[^`\n]*`|“[^”\n]*”|(^|[^\w])'[^'\n]*'|(^|[^\w])‘[^’\n]*’"#).expect("quote pattern")
});

/// `s` with e-mail addresses, links (down to scheme and host), file paths and quoted text masked:
/// for free-form text that must still be logged (error messages from libraries, the front end).
pub fn redact(s: &str) -> String {
    let s = URL.replace_all(s, |c: &Captures| match url::Url::parse(&c[0]) {
        Ok(u) => format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")),
        Err(_) => "<url>".into(),
    });
    let s = EMAIL.replace_all(&s, "<email>");
    let s = PATH.replace_all(&s, |c: &Captures| {
        // the space / `=` / `(` before a Unix path stays
        let lead: String = c[0].chars().take_while(|ch| !matches!(ch, '/' | '\\') && !ch.is_ascii_alphabetic()).collect();
        format!("{lead}<path>")
    });
    QUOTED
        .replace_all(&s, |c: &Captures| match (c.get(1), c.get(2)) {
            // the character before a single quote stays
            (Some(lead), _) => format!("{}'…'", lead.as_str()),
            (_, Some(lead)) => format!("{}‘…’", lead.as_str()),
            _ => match c[0].chars().next() {
                Some('“') => "“…”".into(),
                Some(q) => format!("{q}…{q}"),
                None => String::new(),
            },
        })
        .into_owned()
}

/// A script location in a stack frame (`http://tauri.localhost/assets/index-x.js:12:34`): the app's
/// own file name and position, which `redact` would cut down to the host.
static SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[A-Za-z][A-Za-z0-9+.-]*://[^\s()]*?/([^/\s()?#]+\.(?:m?jsx?|tsx?)\b)(?:\?[^\s():]*)?((?::\d+){0,2})")
        .expect("script pattern")
});

/// A front-end log line (`app.log`): stack frames keep their file:line, the rest goes through `redact`.
pub fn redact_ui(s: &str) -> String {
    redact(&SCRIPT.replace_all(s, "$1$2"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_addresses_links_paths_and_quotes() {
        assert_eq!(redact("can't send to Dana <dana.levi+x@harbor.example>"), "can't send to Dana <<email>>");
        assert_eq!(
            redact("error sending request for url (https://gmail.googleapis.com/gmail/v1/users/me/messages?q=invoice)"),
            "error sending request for url (https://gmail.googleapis.com)"
        );
        assert_eq!(redact("reading C:\\Users\\alex\\Documents\\Alex's offer.pdf: access denied"), "reading <path>: access denied");
        assert_eq!(redact("open \\\\server\\share\\a b.pst failed"), "open <path>");
        assert_eq!(redact("no such file (/home/k/mail/notes.txt)"), "no such file (<path>");
        assert_eq!(redact(r#"invalid type: string "Quarterly \"results\"", expected i64"#), r#"invalid type: string "…", expected i64"#);
        assert_eq!(redact("unknown variant `Board meeting`, expected `a`"), "unknown variant `…`, expected `…`");
        assert_eq!(redact("label 'Clients/ACME' isn't there"), "label '…' isn't there");
        assert_eq!(redact("subject “Hello” and ‘there’"), "subject “…” and ‘…’");
        assert_eq!(redact("database is locked (code 5)"), "database is locked (code 5)");
        assert_eq!(redact("HTTP/1.1 at line 1 column 7"), "HTTP/1.1 at line 1 column 7");
    }

    #[test]
    fn errors_log_codes_not_messages() {
        let e = crate::user_err!("not_found", "No message from dana@harbor.example about \"Budget\"");
        assert_eq!(err(&e), "user error not_found");
        let e: anyhow::Error =
            ApiError::Http { status: 400, message: "Invalid label name Clients".into(), reason: Some("invalidArgument".into()) }.into();
        assert_eq!(err(&e.context("creating the label")), "Google API error 400 (invalidArgument)");
        let e: anyhow::Error = ApiError::Auth("dana@harbor.example needs to sign in again".into()).into();
        assert_eq!(err(&e), "not signed in");
        let e = anyhow::anyhow!("no attachment \"contract.pdf\" in message for dana@harbor.example");
        assert_eq!(err(&e), "internal error: no attachment \"…\" in message for <email>");
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let e = anyhow::Error::new(io).context("saving C:\\Users\\k\\a.pdf");
        assert_eq!(err(&e), "io (NotFound) error: saving <path>: gone");
    }

    #[test]
    fn ui_lines_keep_stack_positions() {
        let stack = "TypeError: no 'subject' for dana@harbor.example\n    at List (http://tauri.localhost/assets/index-Bx2.js:123:45)\n    at http://localhost:1420/src/modules/mail/List.tsx?t=17:9:3";
        assert_eq!(redact_ui(stack), "TypeError: no '…' for <email>\n    at List (index-Bx2.js:123:45)\n    at List.tsx:9:3");
        assert_eq!(redact_ui("[main] boom at http://tauri.localhost/assets/index.js:12"), "[main] boom at index.js:12");
        assert_eq!(redact_ui("Find All: see https://example.com/a?b=c"), "Find All: see https://example.com");
    }
}

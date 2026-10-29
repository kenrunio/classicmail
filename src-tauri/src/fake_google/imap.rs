//! Fake Gmail IMAP (plain TCP on 127.0.0.1) serving the same fake Gmail `Store` as the REST
//! fake, so ids, threads and labels match exactly: X-GM-MSGID / X-GM-THRID are the decimal values
//! of the REST hex ids, X-GM-LABELS carries the labels Gmail exposes over IMAP (system labels as
//! `\Inbox` etc., user labels by name in modified UTF-7; categories and UNREAD are not labels
//! there), FLAGS `\Seen` / `\Flagged` / `\Draft`.
//!
//! Implements just what the client uses — CAPABILITY, AUTHENTICATE XOAUTH2 (token
//! `demo-<email>`), LIST / XLIST with special-use flags, STATUS, EXAMINE, UID SEARCH (ALL, UID
//! ranges, X-GM-RAW "category:x"), UID FETCH, NOOP, LOGOUT — and refuses (and records) anything
//! that could change the mailbox, so tests can assert the client stayed read-only.
//! Mailbox names are localised on purpose (Hebrew "All Mail", British "Bin") so nothing can
//! depend on English folder names.
//!
//! Test hooks (per e-mail address): `set_faults` (refuse sign-in, drop the connection after N
//! fetches) and `stats` (fetch / write / select counters).

use std::{collections::HashMap, sync::OnceLock, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD};
use parking_lot::Mutex;
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, tcp::OwnedWriteHalf},
};

use super::{Shared, gmail::FakeMessage};
use crate::google::imap::{Val, mutf7_encode, parse_values, quote, read_line};

const UID_BASE: u64 = 0x18a3c0000000;

/// Behaviour switches for tests (keyed by lower-case e-mail).
#[derive(Debug, Clone, Default)]
pub struct Faults {
    /// answer AUTHENTICATE with this NO text (e.g. Workspace "IMAP access is disabled")
    pub refuse_login: Option<String>,
    /// close the connection when this many more UID FETCH commands have been answered
    pub drop_after_fetches: Option<u32>,
    /// no special-use attributes in LIST / XLIST (an old or odd server)
    pub hide_special_use: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub logins: u32,
    pub fetch_commands: u32,
    /// messages returned by UID FETCH
    pub fetched: u64,
    /// STORE / COPY / MOVE / EXPUNGE / APPEND / CREATE / DELETE / RENAME... attempts
    pub writes: u32,
    /// SELECT (we only ever want EXAMINE) or a FETCH that would set \Seen (BODY[ without PEEK)
    pub non_readonly: u32,
}

fn faults() -> &'static Mutex<HashMap<String, Faults>> {
    static F: OnceLock<Mutex<HashMap<String, Faults>>> = OnceLock::new();
    F.get_or_init(Default::default)
}

fn stats_map() -> &'static Mutex<HashMap<String, Stats>> {
    static S: OnceLock<Mutex<HashMap<String, Stats>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

pub fn set_faults(email: &str, f: Faults) {
    faults().lock().insert(email.to_lowercase(), f);
}

pub fn stats(email: &str) -> Stats {
    stats_map().lock().get(&email.to_lowercase()).cloned().unwrap_or_default()
}

fn stat(email: &str, f: impl FnOnce(&mut Stats)) {
    f(stats_map().lock().entry(email.to_string()).or_default());
}

fn bases() -> &'static Mutex<HashMap<String, String>> {
    static B: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    B.get_or_init(Default::default)
}

/// The IMAP URL of the fake server whose HTTP base is `base`.
pub fn url_for_base(base: &str) -> Option<String> {
    bases().lock().get(base.trim_end_matches('/')).cloned()
}

/// Start the IMAP listener for `server`; registers it under the HTTP `base`.
pub async fn start(server: Shared, base: &str) -> anyhow::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("imap://{}", listener.local_addr()?);
    bases().lock().insert(base.trim_end_matches('/').to_string(), url.clone());
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { continue };
            let s = server.clone();
            tokio::spawn(async move {
                if let Err(e) = session(s, sock).await {
                    log::debug!("fake imap session: {e:#}");
                }
            });
        }
    });
    Ok(url)
}

// ---- mailboxes ------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Box_ {
    All,
    Trash,
    Spam,
    Inbox,
    Label(String),
}

const ALL_NAME: &str = "[Gmail]/כל הדואר";
const TRASH_NAME: &str = "[Gmail]/Bin";
const SPAM_NAME: &str = "[Gmail]/Spam";

impl Box_ {
    fn uid_validity(&self) -> u32 {
        match self {
            Box_::All => 11,
            Box_::Trash => 12,
            Box_::Spam => 13,
            Box_::Inbox => 14,
            Box_::Label(id) => 100 + id.bytes().map(u32::from).sum::<u32>(),
        }
    }

    fn contains(&self, m: &FakeMessage) -> bool {
        let has = |l: &str| m.label_ids.iter().any(|x| x == l);
        let hidden = has("TRASH") || has("SPAM");
        match self {
            Box_::All => !hidden && !has("CHAT"),
            Box_::Trash => has("TRASH"),
            Box_::Spam => has("SPAM"),
            Box_::Inbox => has("INBOX") && !hidden,
            Box_::Label(id) => has(id) && !hidden,
        }
    }
}

pub fn uid_of(id: &str) -> u32 {
    let n = u64::from_str_radix(id, 16).unwrap_or(0);
    (n.wrapping_sub(UID_BASE) & 0x7fff_ffff) as u32
}

struct Conn {
    server: Shared,
    w: OwnedWriteHalf,
    user: Option<String>,
    selected: Option<Box_>,
}

/// `(attrs, name)` for LIST; XLIST uses Gmail's older attribute names.
fn mailboxes(server: &Shared, email: &str, xlist: bool, special: bool) -> Vec<(String, String)> {
    let sp = |a: &str, x: &str| {
        if !special {
            String::new()
        } else if xlist {
            format!(" {x}")
        } else {
            format!(" {a}")
        }
    };
    let mut v = vec![
        (format!("\\HasNoChildren{}", if xlist && special { " \\Inbox" } else { "" }), "INBOX".to_string()),
        ("\\HasChildren \\Noselect".into(), "[Gmail]".into()),
        (format!("\\HasNoChildren{}", sp("\\All", "\\AllMail")), ALL_NAME.into()),
        (format!("\\HasNoChildren{}", sp("\\Drafts", "\\Drafts")), "[Gmail]/Drafts".into()),
        (format!("\\HasNoChildren{}", sp("\\Important", "\\Important")), "[Gmail]/Important".into()),
        (format!("\\HasNoChildren{}", sp("\\Sent", "\\Sent")), "[Gmail]/Sent Mail".into()),
        (format!("\\HasNoChildren{}", sp("\\Junk", "\\Spam")), SPAM_NAME.into()),
        (format!("\\HasNoChildren{}", sp("\\Flagged", "\\Starred")), "[Gmail]/Starred".into()),
        (format!("\\HasNoChildren{}", sp("\\Trash", "\\Trash")), TRASH_NAME.into()),
    ];
    let users: Vec<String> =
        server.read(|st| st.user(email).gmail.labels.iter().filter(|l| l.kind == "user").map(|l| l.name.clone()).collect());
    for name in users {
        v.push(("\\HasNoChildren".into(), name));
    }
    v
}

fn resolve(server: &Shared, email: &str, name: &str) -> Option<Box_> {
    match name {
        n if n.eq_ignore_ascii_case("INBOX") => Some(Box_::Inbox),
        ALL_NAME => Some(Box_::All),
        TRASH_NAME => Some(Box_::Trash),
        SPAM_NAME => Some(Box_::Spam),
        _ => server
            .read(|st| st.user(email).gmail.labels.iter().find(|l| l.kind == "user" && l.name == name).map(|l| Box_::Label(l.id.clone()))),
    }
}

/// Messages of a mailbox, ascending by UID.
fn box_messages(server: &Shared, email: &str, b: &Box_, uid_range: Option<(u32, u32)>) -> (Vec<u32>, Vec<(u32, FakeMessage)>) {
    server.read(|st| {
        let g = &st.user(email).gmail;
        let mut uids: Vec<u32> = Vec::new();
        let mut msgs: Vec<(u32, FakeMessage)> = Vec::new();
        for m in g.messages.values().filter(|m| b.contains(m)) {
            let u = uid_of(&m.id);
            uids.push(u);
            if uid_range.is_some_and(|(lo, hi)| u >= lo && u <= hi) {
                msgs.push((u, m.clone()));
            }
        }
        uids.sort_unstable();
        msgs.sort_by_key(|(u, _)| *u);
        (uids, msgs)
    })
}

// ---- protocol -------------------------------------------------------------------------------

async fn session(server: Shared, sock: TcpStream) -> anyhow::Result<()> {
    let _ = sock.set_nodelay(true);
    let (r, w) = sock.into_split();
    let mut r = BufReader::new(r);
    let mut c = Conn { server, w, user: None, selected: None };
    c.send(b"* OK Gimap ready for requests from 127.0.0.1 fake\r\n").await?;
    let timeout = Duration::from_secs(600);
    loop {
        let mut line = Vec::new();
        if read_line(&mut r, &mut line, timeout).await.is_err() {
            return Ok(());
        }
        let vals = match parse_values(&line) {
            Ok(v) if v.len() >= 2 => v,
            _ => {
                c.send(b"* BAD Could not parse command\r\n").await?;
                continue;
            }
        };
        let tag = vals[0].text().unwrap_or_default();
        let cmd = vals[1].text().unwrap_or_default().to_ascii_uppercase();
        let args = &vals[2..];
        match cmd.as_str() {
            "CAPABILITY" => {
                c.send(b"* CAPABILITY IMAP4rev1 UNSELECT IDLE NAMESPACE QUOTA ID XLIST CHILDREN X-GM-EXT-1 UIDPLUS COMPRESS=DEFLATE ENABLE MOVE CONDSTORE ESEARCH UTF8=ACCEPT LIST-EXTENDED LIST-STATUS LITERAL- SPECIAL-USE AUTH=XOAUTH2 AUTH=PLAIN AUTH=OAUTHBEARER\r\n").await?;
                c.ok(&tag, "Thats all she wrote! (Success)").await?;
            }
            "NOOP" => c.ok(&tag, "Success").await?,
            "LOGOUT" => {
                c.send(b"* BYE LOGOUT Requested\r\n").await?;
                c.ok(&tag, "73 good day (Success)").await?;
                return Ok(());
            }
            "AUTHENTICATE" => {
                if !c.authenticate(&mut r, &tag, args).await? {
                    continue;
                }
            }
            "LOGIN" => c.tagged(&tag, "NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)").await?,
            _ if c.user.is_none() => c.tagged(&tag, "BAD Command not allowed now. (Failure)").await?,
            "LIST" | "XLIST" => c.list(&tag, cmd == "XLIST").await?,
            "STATUS" => c.status(&tag, args).await?,
            "EXAMINE" | "SELECT" => c.examine(&tag, args, cmd == "SELECT").await?,
            "UID" => {
                let sub = args.first().and_then(Val::text).unwrap_or_default().to_ascii_uppercase();
                match sub.as_str() {
                    "SEARCH" => c.search(&tag, &args[1..]).await?,
                    "FETCH" => {
                        if !c.fetch(&tag, &args[1..]).await? {
                            return Ok(()); // dropped (fault injection)
                        }
                    }
                    _ => c.refuse_write(&tag).await?,
                }
            }
            "CLOSE" | "UNSELECT" => {
                c.selected = None;
                c.ok(&tag, "Returned to authenticated state. (Success)").await?;
            }
            "STORE" | "COPY" | "MOVE" | "EXPUNGE" | "APPEND" | "CREATE" | "DELETE" | "RENAME" | "SUBSCRIBE" | "UNSUBSCRIBE" | "FETCH"
            | "SEARCH" => c.refuse_write(&tag).await?,
            _ => c.tagged(&tag, "BAD Unknown command (Failure)").await?,
        }
    }
}

impl Conn {
    async fn send(&mut self, b: &[u8]) -> anyhow::Result<()> {
        self.w.write_all(b).await?;
        Ok(())
    }
    async fn tagged(&mut self, tag: &str, text: &str) -> anyhow::Result<()> {
        self.send(format!("{tag} {text}\r\n").as_bytes()).await
    }
    async fn ok(&mut self, tag: &str, text: &str) -> anyhow::Result<()> {
        self.tagged(tag, &format!("OK {text}")).await
    }
    fn email(&self) -> String {
        self.user.clone().unwrap_or_default()
    }

    async fn refuse_write(&mut self, tag: &str) -> anyhow::Result<()> {
        if let Some(u) = &self.user {
            stat(u, |s| s.writes += 1);
        }
        self.tagged(tag, "NO [CANNOT] This fake mailbox is read-only for ClassicMail (Failure)").await
    }

    /// Returns false when the connection should just continue (the reply was sent).
    async fn authenticate(&mut self, r: &mut BufReader<tokio::net::tcp::OwnedReadHalf>, tag: &str, args: &[Val]) -> anyhow::Result<bool> {
        if !args.first().is_some_and(|a| a.is("XOAUTH2")) {
            self.tagged(tag, "NO Unsupported mechanism (Failure)").await?;
            return Ok(false);
        }
        let initial = match args.get(1).and_then(Val::text) {
            Some(t) => t,
            None => {
                self.send(b"+ \r\n").await?;
                let mut line = Vec::new();
                read_line(r, &mut line, Duration::from_secs(60)).await?;
                String::from_utf8_lossy(&line).trim().to_string()
            }
        };
        let decoded = STANDARD.decode(initial.trim()).ok().map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let mut user = String::new();
        let mut token = String::new();
        for part in decoded.split('\x01') {
            if let Some(u) = part.strip_prefix("user=") {
                user = u.to_lowercase();
            } else if let Some(t) = part.strip_prefix("auth=Bearer ") {
                token = t.to_string();
            }
        }
        let refuse = faults().lock().get(&user).and_then(|f| f.refuse_login.clone());
        if let Some(text) = refuse {
            self.tagged(tag, &format!("NO [ALERT] {text}")).await?;
            return Ok(false);
        }
        if user.is_empty() || token != format!("demo-{user}") {
            // like Gmail: an error challenge, then NO after the client's empty answer
            let challenge = STANDARD.encode(r#"{"status":"400","schemes":"Bearer","scope":"https://mail.google.com/"}"#);
            self.send(format!("+ {challenge}\r\n").as_bytes()).await?;
            let mut line = Vec::new();
            read_line(r, &mut line, Duration::from_secs(60)).await?;
            self.tagged(tag, "NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)").await?;
            return Ok(false);
        }
        stat(&user, |s| s.logins += 1);
        self.send(b"* CAPABILITY IMAP4rev1 UNSELECT IDLE NAMESPACE QUOTA ID XLIST CHILDREN X-GM-EXT-1 UIDPLUS COMPRESS=DEFLATE ENABLE MOVE CONDSTORE ESEARCH UTF8=ACCEPT LIST-EXTENDED LIST-STATUS LITERAL- SPECIAL-USE APPENDLIMIT=35651584\r\n").await?;
        self.ok(tag, &format!("{user} authenticated (Success)")).await?;
        self.user = Some(user);
        Ok(true)
    }

    async fn list(&mut self, tag: &str, xlist: bool) -> anyhow::Result<()> {
        let email = self.email();
        let special = !faults().lock().get(&email).is_some_and(|f| f.hide_special_use);
        let word = if xlist { "XLIST" } else { "LIST" };
        let mut out = Vec::new();
        for (attrs, name) in mailboxes(&self.server, &email, xlist, special) {
            out.extend_from_slice(format!("* {word} ({attrs}) \"/\" {}\r\n", quote(&mutf7_encode(&name))).as_bytes());
        }
        self.send(&out).await?;
        self.ok(tag, "Success").await
    }

    fn mailbox_arg(&self, v: Option<&Val>) -> Option<Box_> {
        let raw = v?.text()?;
        let name = crate::google::imap::mutf7_decode(&raw);
        resolve(&self.server, &self.email(), &name)
    }

    async fn status(&mut self, tag: &str, args: &[Val]) -> anyhow::Result<()> {
        let Some(b) = self.mailbox_arg(args.first()) else { return self.tagged(tag, "NO [NONEXISTENT] Unknown Mailbox (Failure)").await };
        let (uids, _) = box_messages(&self.server, &self.email(), &b, None);
        let raw = args[0].text().unwrap_or_default();
        let next = uids.last().copied().unwrap_or(0) + 1;
        self.send(
            format!("* STATUS {} (MESSAGES {} UIDNEXT {next} UIDVALIDITY {})\r\n", quote(&raw), uids.len(), b.uid_validity()).as_bytes(),
        )
        .await?;
        self.ok(tag, "Success").await
    }

    async fn examine(&mut self, tag: &str, args: &[Val], select: bool) -> anyhow::Result<()> {
        let email = self.email();
        if select {
            stat(&email, |s| s.non_readonly += 1);
        }
        let Some(b) = self.mailbox_arg(args.first()) else { return self.tagged(tag, "NO [NONEXISTENT] Unknown Mailbox (Failure)").await };
        let (uids, _) = box_messages(&self.server, &email, &b, None);
        let next = uids.last().copied().unwrap_or(0) + 1;
        let text = format!(
            "* FLAGS (\\Answered \\Flagged \\Draft \\Deleted \\Seen $NotPhishing $Phishing)\r\n* OK [PERMANENTFLAGS ()] Flags permitted.\r\n* OK [UIDVALIDITY {}] UIDs valid.\r\n* {} EXISTS\r\n* 0 RECENT\r\n* OK [UIDNEXT {next}] Predicted next UID.\r\n* OK [HIGHESTMODSEQ 917162500]\r\n",
            b.uid_validity(),
            uids.len()
        );
        self.send(text.as_bytes()).await?;
        self.selected = Some(b);
        self.tagged(tag, if select { "OK [READ-WRITE] Success" } else { "OK [READ-ONLY] Success" }).await
    }

    async fn search(&mut self, tag: &str, args: &[Val]) -> anyhow::Result<()> {
        let Some(b) = self.selected.clone() else { return self.tagged(tag, "BAD No mailbox selected (Failure)").await };
        let email = self.email();
        let words: Vec<String> = args.iter().filter_map(Val::text).collect();
        let (uids, _) = box_messages(&self.server, &email, &b, None);
        let hits: Vec<u32> = match words.first().map(|w| w.to_ascii_uppercase()).as_deref() {
            None | Some("ALL") => uids,
            Some("UID") => {
                let set = parse_set(words.get(1).map(String::as_str).unwrap_or(""), uids.last().copied().unwrap_or(0));
                uids.into_iter().filter(|u| set.iter().any(|(lo, hi)| u >= lo && u <= hi)).collect()
            }
            Some("X-GM-RAW") => {
                let q = words.get(1).cloned().unwrap_or_default().to_lowercase();
                let label = match q.trim().strip_prefix("category:") {
                    Some("primary") | Some("personal") => "CATEGORY_PERSONAL",
                    Some("social") => "CATEGORY_SOCIAL",
                    Some("promotions") => "CATEGORY_PROMOTIONS",
                    Some("updates") => "CATEGORY_UPDATES",
                    Some("forums") => "CATEGORY_FORUMS",
                    _ => return self.tagged(tag, "BAD Could not parse command (Failure)").await,
                };
                let (_, msgs) = box_messages(&self.server, &email, &b, Some((0, u32::MAX)));
                msgs.into_iter().filter(|(_, m)| m.label_ids.iter().any(|l| l == label)).map(|(u, _)| u).collect()
            }
            _ => return self.tagged(tag, "BAD Could not parse command (Failure)").await,
        };
        let mut line = String::from("* SEARCH");
        for u in hits {
            line.push(' ');
            line.push_str(&u.to_string());
        }
        line.push_str("\r\n");
        self.send(line.as_bytes()).await?;
        self.ok(tag, "SEARCH completed (Success)").await
    }

    /// Returns false if the connection was dropped on purpose.
    async fn fetch(&mut self, tag: &str, args: &[Val]) -> anyhow::Result<bool> {
        let Some(b) = self.selected.clone() else {
            self.tagged(tag, "BAD No mailbox selected (Failure)").await?;
            return Ok(true);
        };
        let email = self.email();
        // fault: drop the connection instead of answering
        {
            let mut f = faults().lock();
            if let Some(n) = f.get_mut(&email).and_then(|x| x.drop_after_fetches.as_mut()) {
                if *n == 0 {
                    return Ok(false);
                }
                *n -= 1;
            }
        }
        let set_text = args.first().and_then(Val::text).unwrap_or_default();
        let items: Vec<String> = match args.get(1) {
            Some(Val::List(l)) => l.iter().filter_map(Val::text).collect(),
            Some(v) => v.text().into_iter().collect(),
            None => vec![],
        };
        let items_upper: Vec<String> = items.iter().map(|i| i.to_ascii_uppercase()).collect();
        if items_upper.iter().any(|i| i.starts_with("BODY[") || i == "RFC822" || i == "RFC822.TEXT") {
            stat(&email, |s| s.non_readonly += 1);
        }
        let (all_uids, _) = box_messages(&self.server, &email, &b, None);
        let max = all_uids.last().copied().unwrap_or(0);
        let set = parse_set(&set_text, max);
        let lo = set.iter().map(|(l, _)| *l).min().unwrap_or(1);
        let hi = set.iter().map(|(_, h)| *h).max().unwrap_or(0);
        let (_, msgs) = box_messages(&self.server, &email, &b, Some((lo, hi)));
        let header_item = items_upper.iter().position(|i| i.starts_with("BODY.PEEK[HEADER.FIELDS") || i.starts_with("BODY[HEADER.FIELDS"));
        let fields: Vec<String> = header_item.map(|i| section_fields(&items[i])).unwrap_or_default();
        let names: HashMap<String, String> = self
            .server
            .read(|st| st.user(&email).gmail.labels.iter().filter(|l| l.kind == "user").map(|l| (l.id.clone(), l.name.clone())).collect());
        let mut n = 0u64;
        let mut out: Vec<u8> = Vec::new();
        for (uid, m) in msgs.iter().filter(|(u, _)| set.iter().any(|(lo, hi)| u >= lo && u <= hi)) {
            let seq = all_uids.binary_search(uid).map(|i| i + 1).unwrap_or(0);
            out.extend_from_slice(fetch_line(seq, *uid, m, &names, header_item.map(|i| (items[i].as_str(), fields.as_slice()))).as_slice());
            n += 1;
            if out.len() > 256 * 1024 {
                self.send(&out).await?;
                out.clear();
            }
        }
        self.send(&out).await?;
        stat(&email, |s| {
            s.fetch_commands += 1;
            s.fetched += n;
        });
        self.ok(tag, "Success").await?;
        Ok(true)
    }
}

/// `1:5,7,9:*` → ranges (`*` = `max`).
fn parse_set(s: &str, max: u32) -> Vec<(u32, u32)> {
    let num = |t: &str| if t == "*" { Some(max) } else { t.parse::<u32>().ok() };
    s.split(',')
        .filter_map(|part| match part.split_once(':') {
            Some((a, b)) => {
                let (a, b) = (num(a)?, num(b)?);
                Some((a.min(b), a.max(b)))
            }
            None => num(part).map(|n| (n, n)),
        })
        .collect()
}

/// `BODY.PEEK[HEADER.FIELDS (FROM TO)]` → ["FROM", "TO"].
fn section_fields(item: &str) -> Vec<String> {
    let inner = item.split_once('(').and_then(|(_, r)| r.split_once(')')).map(|(f, _)| f).unwrap_or("");
    inner.split_whitespace().map(|s| s.to_ascii_uppercase()).collect()
}

/// The header lines (with their continuation lines) whose names are in `fields`, plus the blank
/// line that ends a header block — what Gmail returns for BODY[HEADER.FIELDS (...)].
pub fn header_fields(raw: &[u8], fields: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut keep = false;
    let mut pos = 0;
    while pos < raw.len() {
        let end = raw[pos..].iter().position(|c| *c == b'\n').map(|i| pos + i + 1).unwrap_or(raw.len());
        let line = &raw[pos..end];
        pos = end;
        let content = line.strip_suffix(b"\n").map(|l| l.strip_suffix(b"\r").unwrap_or(l)).unwrap_or(line);
        if content.is_empty() {
            break; // end of the header block
        }
        if content[0] == b' ' || content[0] == b'\t' {
            if keep {
                out.extend_from_slice(content);
                out.extend_from_slice(b"\r\n");
            }
            continue;
        }
        let name = content
            .iter()
            .position(|c| *c == b':')
            .map(|i| String::from_utf8_lossy(&content[..i]).trim().to_ascii_uppercase())
            .unwrap_or_default();
        keep = !name.is_empty() && fields.contains(&name);
        if keep {
            out.extend_from_slice(content);
            out.extend_from_slice(b"\r\n");
        }
    }
    out.extend_from_slice(b"\r\n");
    out
}

fn imap_label(id: &str, user_names: &HashMap<String, String>) -> Option<String> {
    Some(match id {
        "INBOX" => "\\Inbox".into(),
        "SENT" => "\\Sent".into(),
        "DRAFT" => "\\Draft".into(),
        "STARRED" => "\\Starred".into(),
        // real Gmail quotes this one
        "IMPORTANT" => "\"\\\\Important\"".into(),
        "UNREAD" | "TRASH" | "SPAM" | "CHAT" => return None,
        x if x.starts_with("CATEGORY_") => return None,
        x => {
            let name = mutf7_encode(user_names.get(x)?);
            let atom_safe = !name.is_empty() && name.bytes().all(|c| c.is_ascii_graphic() && !b"(){%*\"\\]".contains(&c));
            if atom_safe { name } else { quote(&name) }
        }
    })
}

fn internal_date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().format("%d-%b-%Y %H:%M:%S +0000").to_string()
}

fn fetch_line(seq: usize, uid: u32, m: &FakeMessage, user_names: &HashMap<String, String>, header: Option<(&str, &[String])>) -> Vec<u8> {
    let labels: Vec<String> = m.label_ids.iter().filter_map(|l| imap_label(l, user_names)).collect();
    let mut flags = Vec::new();
    if !m.label_ids.iter().any(|l| l == "UNREAD") {
        flags.push("\\Seen");
    }
    if m.label_ids.iter().any(|l| l == "STARRED") {
        flags.push("\\Flagged");
    }
    if m.label_ids.iter().any(|l| l == "DRAFT") {
        flags.push("\\Draft");
    }
    let msgid = u64::from_str_radix(&m.id, 16).unwrap_or(0);
    let thrid = u64::from_str_radix(&m.thread_id, 16).unwrap_or(0);
    // Gmail's own item order
    let mut out = format!(
        "* {seq} FETCH (X-GM-THRID {thrid} X-GM-MSGID {msgid} X-GM-LABELS ({}) UID {uid} FLAGS ({}) INTERNALDATE \"{}\" RFC822.SIZE {}",
        labels.join(" "),
        flags.join(" "),
        internal_date(m.internal_date),
        m.raw.len()
    )
    .into_bytes();
    if let Some((item, fields)) = header {
        let block = header_fields(m.raw.as_bytes(), fields);
        let key = item.replacen("BODY.PEEK[", "BODY[", 1).replacen("body.peek[", "BODY[", 1);
        out.extend_from_slice(format!(" {key} {{{}}}\r\n", block.len()).as_bytes());
        out.extend_from_slice(&block);
    }
    out.extend_from_slice(b")\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_fields_keep_folding() {
        let raw = b"Received: x\r\nSubject: =?UTF-8?B?abc?=\r\n =?UTF-8?B?def?=\r\nFrom: a@b\r\nX-Other: y\r\n\r\nbody\r\nSubject: not a header\r\n";
        let f = vec!["SUBJECT".to_string(), "FROM".to_string()];
        assert_eq!(header_fields(raw, &f), b"Subject: =?UTF-8?B?abc?=\r\n =?UTF-8?B?def?=\r\nFrom: a@b\r\n\r\n");
        assert_eq!(parse_set("1:5,7,9:*", 20), vec![(1, 5), (7, 7), (9, 20)]);
        assert_eq!(section_fields("BODY.PEEK[HEADER.FIELDS (from To)]"), vec!["FROM", "TO"]);
    }
}

//! Minimal IMAP4rev1 client for Gmail's fast initial download (headers of the whole mailbox in
//! bulk, outside the Gmail API's per-user quota). Read-only by construction: the only commands it
//! can send are CAPABILITY, AUTHENTICATE XOAUTH2, LIST/XLIST, STATUS, EXAMINE, UID SEARCH,
//! UID FETCH (BODY.PEEK only) and LOGOUT — never SELECT, STORE, COPY, MOVE, EXPUNGE or APPEND.
//!
//! The response parser handles what real Gmail sends: literals (`{n}` + bytes, also several per
//! response), quoted strings with `\"` / `\\` escapes, NIL, nested lists, `BODY[...]` section
//! atoms with spaces inside the brackets, raw UTF-8 in strings, very long lines. Mailbox and label
//! names are modified UTF-7 (RFC 3501 §5.1.3); `mutf7_decode` / `mutf7_encode` convert them.
//!
//! Gmail extensions (https://developers.google.com/gmail/imap/imap-extensions): X-GM-MSGID and
//! X-GM-THRID are "the decimal equivalent for the ID hex string used in the web interface and the
//! Gmail API"; X-GM-LABELS is a list of astrings (system labels as `\Inbox`, `\Sent`, `\Important`,
//! `\Starred`, `\Draft`…, user labels by name, modified UTF-7).
//!
//! Plain (non-TLS) connections are allowed for loopback hosts only (the fake server in tests and
//! demo mode); anything else must be `imaps://` with a verified certificate (webpki roots).

use std::{sync::Arc, time::Duration};

use base64::{
    Engine,
    alphabet::Alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose::STANDARD},
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
};

/// One response line without its literals (`UID SEARCH ALL` of 1M messages is ~8 MB).
pub const MAX_LINE: usize = 16 << 20;
/// One literal (a header block is a few KB; a body could be big).
pub const MAX_LITERAL: usize = 64 << 20;
/// One whole response (line + literals).
pub const MAX_RESPONSE: usize = 96 << 20;

#[derive(Debug, thiserror::Error)]
pub enum ImapError {
    /// Sign-in refused (bad token, IMAP disabled for the account / domain).
    #[error("IMAP sign-in refused: {0}")]
    Auth(String),
    /// The server can't do what we need (no All Mail folder, plain IMAP to a remote host...).
    #[error("IMAP: {0}")]
    Unsupported(String),
    /// A command answered NO.
    #[error("IMAP command refused: {0}")]
    No(String),
    /// Connection problems, timeouts, the server closing the connection.
    #[error("IMAP network error: {0}")]
    Network(String),
    /// Something we could not parse, or a BAD answer.
    #[error("IMAP protocol error: {0}")]
    Protocol(String),
}

impl ImapError {
    /// Retrying later won't help (fall back to the API at once).
    pub fn is_permanent(&self) -> bool {
        matches!(self, ImapError::Auth(_) | ImapError::Unsupported(_))
    }
}

type Result<T> = std::result::Result<T, ImapError>;

fn net(e: impl std::fmt::Display) -> ImapError {
    ImapError::Network(e.to_string())
}

// ---- values ---------------------------------------------------------------------------------

/// A parsed IMAP value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Val {
    /// atoms, numbers, flags (`\Seen`), section specs (`BODY[HEADER.FIELDS (FROM)]`)
    Atom(String),
    /// quoted string or literal (raw bytes)
    Str(Vec<u8>),
    Nil,
    List(Vec<Val>),
}

impl Val {
    pub fn atom(&self) -> Option<&str> {
        match self {
            Val::Atom(a) => Some(a),
            _ => None,
        }
    }
    pub fn is(&self, word: &str) -> bool {
        self.atom().is_some_and(|a| a.eq_ignore_ascii_case(word))
    }
    /// Atom or string as text (lossy UTF-8); None for NIL / lists.
    pub fn text(&self) -> Option<String> {
        match self {
            Val::Atom(a) => Some(a.clone()),
            Val::Str(b) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        }
    }
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Val::Atom(a) => Some(a.as_bytes()),
            Val::Str(b) => Some(b),
            _ => None,
        }
    }
    pub fn num(&self) -> Option<u64> {
        let t = match self {
            Val::Atom(a) => a.as_str(),
            Val::Str(b) => std::str::from_utf8(b).ok()?,
            _ => return None,
        };
        t.trim().parse().ok()
    }
    pub fn list(&self) -> Option<&[Val]> {
        match self {
            Val::List(v) => Some(v),
            _ => None,
        }
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

const MAX_DEPTH: usize = 64;

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\r' | b'\n') {
            self.i += 1;
        }
    }

    fn err(&self, what: &str) -> ImapError {
        ImapError::Protocol(format!("{what} at byte {} of {}", self.i, self.b.len()))
    }

    fn value(&mut self, depth: usize) -> Result<Val> {
        if depth > MAX_DEPTH {
            return Err(self.err("lists nested too deeply"));
        }
        let Some(&c) = self.b.get(self.i) else { return Err(self.err("value expected")) };
        match c {
            b'(' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.skip_ws();
                    match self.b.get(self.i) {
                        None => return Err(self.err("unterminated list")),
                        Some(b')') => {
                            self.i += 1;
                            return Ok(Val::List(v));
                        }
                        Some(_) => v.push(self.value(depth + 1)?),
                    }
                }
            }
            b'"' => self.quoted(),
            b'{' => self.literal(),
            b'~' if self.b.get(self.i + 1) == Some(&b'{') => {
                self.i += 1;
                self.literal()
            }
            b')' => Err(self.err("unexpected ')'")),
            _ => self.atom(),
        }
    }

    fn quoted(&mut self) -> Result<Val> {
        self.i += 1;
        let mut s = Vec::new();
        while self.i < self.b.len() {
            let c = self.b[self.i];
            self.i += 1;
            match c {
                b'\\' if self.i < self.b.len() => {
                    s.push(self.b[self.i]);
                    self.i += 1;
                }
                b'"' => return Ok(Val::Str(s)),
                _ => s.push(c),
            }
        }
        Err(self.err("unterminated quoted string"))
    }

    fn literal(&mut self) -> Result<Val> {
        self.i += 1; // '{'
        let start = self.i;
        while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
            self.i += 1;
        }
        let n: usize =
            std::str::from_utf8(&self.b[start..self.i]).ok().and_then(|s| s.parse().ok()).ok_or_else(|| self.err("bad literal length"))?;
        if self.b.get(self.i) == Some(&b'+') || self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        if self.b.get(self.i) != Some(&b'}') {
            return Err(self.err("bad literal"));
        }
        self.i += 1;
        if self.b.get(self.i) == Some(&b'\r') {
            self.i += 1;
        }
        if self.b.get(self.i) == Some(&b'\n') {
            self.i += 1;
        } else {
            return Err(self.err("literal without CRLF"));
        }
        let end = self.i.checked_add(n).filter(|e| *e <= self.b.len()).ok_or_else(|| self.err("literal longer than the response"))?;
        let v = self.b[self.i..end].to_vec();
        self.i = end;
        Ok(Val::Str(v))
    }

    fn atom(&mut self) -> Result<Val> {
        let start = self.i;
        let mut brackets = 0usize;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if brackets > 0 {
                match c {
                    b'[' => brackets += 1,
                    b']' => brackets -= 1,
                    b'\r' | b'\n' => break,
                    _ => {}
                }
                self.i += 1;
                continue;
            }
            match c {
                b' ' | b'\t' | b'(' | b')' | b'"' | b'\r' | b'\n' => break,
                b'[' => brackets += 1,
                _ => {}
            }
            self.i += 1;
        }
        if self.i == start {
            return Err(self.err("atom expected"));
        }
        let s = String::from_utf8_lossy(&self.b[start..self.i]).into_owned();
        Ok(if s.eq_ignore_ascii_case("NIL") { Val::Nil } else { Val::Atom(s) })
    }
}

/// Parse a sequence of values (the part of a response after `* `).
pub fn parse_values(b: &[u8]) -> Result<Vec<Val>> {
    let mut p = Parser { b, i: 0 };
    let mut out = Vec::new();
    loop {
        p.skip_ws();
        if p.i >= b.len() {
            return Ok(out);
        }
        out.push(p.value(0)?);
    }
}

// ---- responses ------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resp {
    /// OK / NO / BAD / BYE / PREAUTH; `tag` is "*" when untagged
    Status { tag: String, kind: String, code: Option<String>, text: String },
    /// `+ ...`
    Cont(String),
    /// other untagged data (`* 3 EXISTS`, `* LIST ...`, `* 12 FETCH (...)`)
    Data(Vec<Val>),
}

fn trim_crlf(mut b: &[u8]) -> &[u8] {
    while let Some((&last, rest)) = b.split_last() {
        if last == b'\n' || last == b'\r' {
            b = rest;
        } else {
            break;
        }
    }
    b
}

fn split_word(b: &[u8]) -> (&[u8], &[u8]) {
    match b.iter().position(|c| *c == b' ') {
        Some(i) => (&b[..i], &b[i + 1..]),
        None => (b, &b[b.len()..]),
    }
}

pub fn parse_resp(raw: &[u8]) -> Result<Resp> {
    let line = trim_crlf(raw);
    let (tag, rest) = split_word(line);
    if tag == b"+" {
        return Ok(Resp::Cont(String::from_utf8_lossy(rest).into_owned()));
    }
    if tag.is_empty() {
        return Err(ImapError::Protocol("empty response".into()));
    }
    let tag = String::from_utf8_lossy(tag).into_owned();
    let (word, after) = split_word(rest);
    let kind = String::from_utf8_lossy(word).to_ascii_uppercase();
    if matches!(kind.as_str(), "OK" | "NO" | "BAD" | "BYE" | "PREAUTH") {
        let (code, text) = split_code(after);
        return Ok(Resp::Status { tag, kind, code, text });
    }
    if tag != "*" {
        return Err(ImapError::Protocol(format!("unexpected tagged response {:?}", String::from_utf8_lossy(&line[..line.len().min(80)]))));
    }
    Ok(Resp::Data(parse_values(rest)?))
}

/// `[CODE args] text` → (Some("CODE args"), "text").
fn split_code(b: &[u8]) -> (Option<String>, String) {
    if b.first() != Some(&b'[') {
        return (None, String::from_utf8_lossy(b).trim().to_string());
    }
    let mut depth = 0usize;
    for (i, c) in b.iter().enumerate() {
        match c {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    let code = String::from_utf8_lossy(&b[1..i]).into_owned();
                    let text = String::from_utf8_lossy(&b[i + 1..]).trim().to_string();
                    return (Some(code), text);
                }
            }
            _ => {}
        }
    }
    (None, String::from_utf8_lossy(b).trim().to_string())
}

/// `{123}` / `{123+}` at the end of a line → 123.
pub fn literal_len_at_end(line: &[u8]) -> Option<usize> {
    let t = trim_crlf(line);
    let t = t.strip_suffix(b"}")?;
    let open = t.iter().rposition(|c| *c == b'{')?;
    let inner = &t[open + 1..];
    let inner = inner.strip_suffix(b"+").or_else(|| inner.strip_suffix(b"-")).unwrap_or(inner);
    if inner.is_empty() || inner.len() > 12 || !inner.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(inner).ok()?.parse().ok()
}

/// Read one line (through `\n`) into `buf`, at most `MAX_LINE` bytes.
pub async fn read_line<R: AsyncBufRead + Unpin>(r: &mut R, buf: &mut Vec<u8>, timeout: Duration) -> Result<()> {
    let start = buf.len();
    loop {
        let (n, done) = {
            let avail = tokio::time::timeout(timeout, r.fill_buf())
                .await
                .map_err(|_| ImapError::Network("timed out waiting for the server".into()))?
                .map_err(net)?;
            if avail.is_empty() {
                return Err(ImapError::Network("connection closed by the server".into()));
            }
            match avail.iter().position(|c| *c == b'\n') {
                Some(p) => {
                    buf.extend_from_slice(&avail[..=p]);
                    (p + 1, true)
                }
                None => {
                    buf.extend_from_slice(avail);
                    (avail.len(), false)
                }
            }
        };
        r.consume(n);
        if done {
            return Ok(());
        }
        if buf.len() - start > MAX_LINE {
            return Err(ImapError::Protocol("response line too long".into()));
        }
    }
}

/// Read one complete response: a line plus any literals it announces (and the lines after them).
pub async fn read_response<R: AsyncBufRead + Unpin>(r: &mut R, timeout: Duration) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    loop {
        let line_start = buf.len();
        read_line(r, &mut buf, timeout).await?;
        let Some(n) = literal_len_at_end(&buf[line_start..]) else { return Ok(buf) };
        if n > MAX_LITERAL || buf.len() + n > MAX_RESPONSE {
            return Err(ImapError::Protocol(format!("literal of {n} bytes is too big")));
        }
        let at = buf.len();
        buf.resize(at + n, 0);
        tokio::time::timeout(timeout, r.read_exact(&mut buf[at..]))
            .await
            .map_err(|_| ImapError::Network("timed out reading from the server".into()))?
            .map_err(net)?;
    }
}

// ---- modified UTF-7 (mailbox and label names) -----------------------------------------------

fn mutf7_engine() -> GeneralPurpose {
    // RFC 3501: base64 with "," instead of "/", no padding
    let alpha = Alphabet::new("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,").expect("alphabet");
    GeneralPurpose::new(
        &alpha,
        GeneralPurposeConfig::new()
            .with_encode_padding(false)
            .with_decode_padding_mode(DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    )
}

/// Modified UTF-7 → text. Malformed input is returned unchanged (lossy, never fails).
pub fn mutf7_decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let eng = mutf7_engine();
    let mut out = String::new();
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let after = &rest[p + 1..];
        let Some(end) = after.find('-') else {
            out.push_str(&rest[p..]);
            return out;
        };
        let b64 = &after[..end];
        if b64.is_empty() {
            out.push('&');
        } else {
            match eng.decode(b64) {
                Ok(bytes) if bytes.len() % 2 == 0 => {
                    let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    out.push_str(&String::from_utf16_lossy(&units));
                }
                _ => {
                    out.push('&');
                    out.push_str(b64);
                    out.push('-');
                }
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Text → modified UTF-7.
pub fn mutf7_encode(s: &str) -> String {
    let eng = mutf7_engine();
    let mut out = String::new();
    let mut pending: Vec<u16> = Vec::new();
    let flush = |pending: &mut Vec<u16>, out: &mut String| {
        if !pending.is_empty() {
            let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
            out.push('&');
            out.push_str(&eng.encode(bytes));
            out.push('-');
            pending.clear();
        }
    };
    for ch in s.chars() {
        if ('\u{20}'..='\u{7e}').contains(&ch) {
            flush(&mut pending, &mut out);
            if ch == '&' {
                out.push_str("&-");
            } else {
                out.push(ch);
            }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(ch.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// A quoted string for a command argument.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// SASL XOAUTH2 initial response (base64).
pub fn xoauth2(user: &str, token: &str) -> String {
    STANDARD.encode(format!("user={user}\x01auth=Bearer {token}\x01\x01"))
}

/// `INTERNALDATE` ("17-Jul-1996 02:44:25 -0700") → epoch ms.
pub fn parse_internal_date(s: &str) -> Option<i64> {
    let t = s.trim();
    chrono::DateTime::parse_from_str(t, "%d-%b-%Y %H:%M:%S %z").ok().map(|d| d.timestamp_millis())
}

// ---- client ---------------------------------------------------------------------------------

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// A mailbox from LIST, with its (special-use) attributes.
#[derive(Debug, Clone)]
pub struct Mailbox {
    /// name as sent by the server (modified UTF-7) — use this in commands
    pub raw: String,
    /// decoded for display / logs
    pub name: String,
    pub attrs: Vec<String>,
}

impl Mailbox {
    pub fn has(&self, attr: &str) -> bool {
        self.attrs.iter().any(|a| a.eq_ignore_ascii_case(attr))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Examined {
    pub exists: u32,
    pub uid_validity: u32,
    pub uid_next: Option<u32>,
    pub read_only: bool,
}

/// One X-GM-LABELS entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GmLabel {
    /// `\Inbox`, `\Sent`, `\Important`... (with the backslash)
    System(String),
    /// user label name, decoded from modified UTF-7
    User(String),
}

/// What the bootstrap fetches per message.
#[derive(Debug, Clone, Default)]
pub struct HeaderRecord {
    pub uid: u32,
    pub msgid: u64,
    pub thrid: u64,
    pub labels: Vec<GmLabel>,
    pub flags: Vec<String>,
    pub internal_date: Option<i64>,
    pub size: i64,
    pub header: Vec<u8>,
}

/// Header fields fetched for every message (what the API path asks for with format=metadata).
pub const HEADER_FIELDS: &str =
    "FROM TO CC BCC REPLY-TO SUBJECT DATE MESSAGE-ID IN-REPLY-TO REFERENCES CONTENT-TYPE IMPORTANCE X-PRIORITY PRIORITY";

pub struct Session {
    r: BufReader<Box<dyn Io>>,
    tag: u32,
    timeout: Duration,
    pub greeting: String,
}

fn is_loopback(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost") || h.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

/// `imaps://host:port` (TLS) or `imap://127.0.0.1:port` (plain, loopback only).
pub fn parse_url(url: &str) -> Result<(bool, String, u16)> {
    let u = url::Url::parse(url).map_err(|e| ImapError::Unsupported(format!("bad IMAP address {url:?}: {e}")))?;
    let tls = match u.scheme() {
        "imaps" => true,
        "imap" => false,
        s => return Err(ImapError::Unsupported(format!("unknown IMAP scheme {s}"))),
    };
    let host = u.host_str().ok_or_else(|| ImapError::Unsupported("IMAP address without a host".into()))?.to_string();
    if !tls && !is_loopback(&host) {
        return Err(ImapError::Unsupported("plain IMAP is only allowed to this computer".into()));
    }
    let port = u.port().unwrap_or(if tls { 993 } else { 143 });
    Ok((tls, host, port))
}

fn tls_connector() -> Result<tokio_rustls::TlsConnector> {
    use tokio_rustls::rustls;
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| ImapError::Protocol(format!("TLS setup: {e}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(tokio_rustls::TlsConnector::from(Arc::new(cfg)))
}

impl Session {
    /// Connect and read the greeting. `timeout` applies to every network wait.
    pub async fn connect(url: &str, timeout: Duration) -> Result<Session> {
        let (tls, host, port) = parse_url(url)?;
        let tcp = tokio::time::timeout(Duration::from_secs(20).min(timeout), TcpStream::connect((host.as_str(), port)))
            .await
            .map_err(|_| ImapError::Network(format!("timed out connecting to {host}:{port}")))?
            .map_err(net)?;
        let _ = tcp.set_nodelay(true);
        let io: Box<dyn Io> = if tls {
            let name = tokio_rustls::rustls::pki_types::ServerName::try_from(host.clone())
                .map_err(|e| ImapError::Unsupported(format!("bad host name: {e}")))?;
            let s = tokio::time::timeout(Duration::from_secs(20).min(timeout), tls_connector()?.connect(name, tcp))
                .await
                .map_err(|_| ImapError::Network("timed out in the TLS handshake".into()))?
                .map_err(|e| ImapError::Network(format!("TLS: {e}")))?;
            Box::new(s)
        } else {
            Box::new(tcp)
        };
        let mut s = Session { r: BufReader::with_capacity(64 * 1024, io), tag: 0, timeout, greeting: String::new() };
        match s.read().await? {
            Resp::Status { kind, text, .. } if kind == "OK" || kind == "PREAUTH" => s.greeting = text,
            Resp::Status { kind, text, .. } if kind == "BYE" => {
                return Err(ImapError::Network(format!("server refused the connection: {text}")));
            }
            other => return Err(ImapError::Protocol(format!("unexpected greeting {other:?}"))),
        }
        Ok(s)
    }

    async fn read(&mut self) -> Result<Resp> {
        let raw = read_response(&mut self.r, self.timeout).await?;
        parse_resp(&raw)
    }

    async fn write(&mut self, line: &str) -> Result<()> {
        let w = self.r.get_mut();
        tokio::time::timeout(self.timeout, async {
            w.write_all(line.as_bytes()).await?;
            w.flush().await
        })
        .await
        .map_err(|_| ImapError::Network("timed out sending to the server".into()))?
        .map_err(net)
    }

    fn next_tag(&mut self) -> String {
        self.tag += 1;
        format!("c{}", self.tag)
    }

    /// Run a command; returns the untagged responses and the tagged OK's code.
    pub async fn command(&mut self, cmd: &str) -> Result<(Vec<Resp>, Option<String>)> {
        let tag = self.next_tag();
        self.write(&format!("{tag} {cmd}\r\n")).await?;
        let mut out = Vec::new();
        loop {
            match self.read().await? {
                Resp::Status { tag: t, kind, code, text } if t == tag => {
                    return match kind.as_str() {
                        "OK" => Ok((out, code)),
                        "NO" => Err(ImapError::No(format!("{}{text}", code.map(|c| format!("[{c}] ")).unwrap_or_default()))),
                        _ => Err(ImapError::Protocol(format!("{kind} {text}"))),
                    };
                }
                Resp::Status { tag: t, kind, text, .. } if t == "*" && kind == "BYE" => {
                    return Err(ImapError::Network(format!("server closed the connection: {text}")));
                }
                Resp::Cont(_) => return Err(ImapError::Protocol("unexpected continuation request".into())),
                r => out.push(r),
            }
        }
    }

    /// SASL XOAUTH2 with an OAuth access token (scope https://mail.google.com/).
    pub async fn authenticate_xoauth2(&mut self, user: &str, token: &str) -> Result<()> {
        let tag = self.next_tag();
        self.write(&format!("{tag} AUTHENTICATE XOAUTH2\r\n")).await?;
        let mut sent = false;
        let mut detail = String::new();
        loop {
            match self.read().await? {
                Resp::Cont(text) => {
                    if !sent {
                        sent = true;
                        self.write(&format!("{}\r\n", xoauth2(user, token))).await?;
                    } else {
                        // error details (base64 JSON); answer with an empty line to get the NO
                        detail = STANDARD.decode(text.trim()).ok().map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or(text);
                        self.write("\r\n").await?;
                    }
                }
                Resp::Status { tag: t, kind, code, text } if t == tag => {
                    if kind == "OK" {
                        return Ok(());
                    }
                    let code = code.map(|c| format!("[{c}] ")).unwrap_or_default();
                    let detail = if detail.is_empty() { String::new() } else { format!(" {detail}") };
                    return Err(ImapError::Auth(format!("{code}{text}{detail}")));
                }
                Resp::Status { tag: t, kind, text, .. } if t == "*" && kind == "BYE" => {
                    return Err(ImapError::Auth(format!("server closed the connection: {text}")));
                }
                _ => {}
            }
        }
    }

    /// All mailboxes with their attributes. Special-use attributes (RFC 6154) come with a plain
    /// LIST on Gmail; XLIST (Gmail's older variant: `\AllMail`, `\Spam`) is tried if none do.
    pub async fn list(&mut self) -> Result<Vec<Mailbox>> {
        let mut boxes = self.list_with("LIST").await?;
        let special = |b: &Mailbox| ["\\All", "\\AllMail", "\\Trash", "\\Junk", "\\Spam"].iter().any(|a| b.has(a));
        if !boxes.iter().any(special)
            && let Ok(x) = self.list_with("XLIST").await
            && x.iter().any(special)
        {
            boxes = x;
        }
        Ok(boxes)
    }

    async fn list_with(&mut self, cmd: &str) -> Result<Vec<Mailbox>> {
        let (untagged, _) = self.command(&format!("{cmd} \"\" \"*\"")).await?;
        let mut out = Vec::new();
        for r in untagged {
            let Resp::Data(v) = r else { continue };
            if v.len() < 4 || !(v[0].is("LIST") || v[0].is("XLIST")) {
                continue;
            }
            let attrs: Vec<String> = v[1].list().unwrap_or(&[]).iter().filter_map(Val::text).collect();
            let Some(raw) = v[3].text() else { continue };
            out.push(Mailbox { name: mutf7_decode(&raw), raw, attrs });
        }
        Ok(out)
    }

    /// Message count without selecting the mailbox.
    pub async fn status_messages(&mut self, mailbox_raw: &str) -> Result<u32> {
        let (untagged, _) = self.command(&format!("STATUS {} (MESSAGES)", quote(mailbox_raw))).await?;
        for r in untagged {
            let Resp::Data(v) = r else { continue };
            if v.first().is_some_and(|x| x.is("STATUS"))
                && let Some(items) = v.last().and_then(Val::list)
            {
                for pair in items.chunks(2) {
                    if pair.len() == 2 && pair[0].is("MESSAGES") {
                        return pair[1].num().map(|n| n as u32).ok_or_else(|| ImapError::Protocol("bad STATUS".into()));
                    }
                }
            }
        }
        Err(ImapError::Protocol("STATUS without MESSAGES".into()))
    }

    /// Open a mailbox read-only.
    pub async fn examine(&mut self, mailbox_raw: &str) -> Result<Examined> {
        let (untagged, code) = self.command(&format!("EXAMINE {}", quote(mailbox_raw))).await?;
        let mut e = Examined { read_only: code.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("READ-ONLY")), ..Default::default() };
        for r in untagged {
            match r {
                Resp::Data(v) if v.len() >= 2 && v[1].is("EXISTS") => e.exists = v[0].num().unwrap_or(0) as u32,
                Resp::Status { code: Some(c), .. } => {
                    let mut it = c.split_whitespace();
                    match (it.next().map(str::to_ascii_uppercase).as_deref(), it.next().and_then(|n| n.parse::<u32>().ok())) {
                        (Some("UIDVALIDITY"), Some(n)) => e.uid_validity = n,
                        (Some("UIDNEXT"), Some(n)) => e.uid_next = Some(n),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        Ok(e)
    }

    /// `UID SEARCH <criteria>` → UIDs (unsorted).
    pub async fn uid_search(&mut self, criteria: &str) -> Result<Vec<u32>> {
        let (untagged, _) = self.command(&format!("UID SEARCH {criteria}")).await?;
        let mut out = Vec::new();
        for r in untagged {
            let Resp::Data(v) = r else { continue };
            if v.first().is_some_and(|x| x.is("SEARCH")) {
                // numbers; ignore a trailing "(MODSEQ n)" (CONDSTORE)
                out.extend(v[1..].iter().filter_map(|x| x.atom().and_then(|a| a.parse::<u32>().ok())));
            }
        }
        Ok(out)
    }

    /// Headers + Gmail attributes of the messages with UIDs in `lo..=hi` (read-only: BODY.PEEK).
    pub async fn fetch_headers(&mut self, lo: u32, hi: u32) -> Result<Vec<HeaderRecord>> {
        let cmd = format!(
            "UID FETCH {lo}:{hi} (UID X-GM-MSGID X-GM-THRID X-GM-LABELS FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER.FIELDS ({HEADER_FIELDS})])"
        );
        let (untagged, _) = self.command(&cmd).await?;
        let mut out = Vec::with_capacity(untagged.len());
        for r in untagged {
            let Resp::Data(v) = r else { continue };
            if v.len() < 3 || !v[1].is("FETCH") {
                continue;
            }
            let Some(items) = v[2].list() else { continue };
            if let Some(rec) = header_record(items)
                && rec.uid >= lo
                && rec.uid <= hi
            {
                out.push(rec);
            }
        }
        Ok(out)
    }

    pub async fn logout(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(5), self.command("LOGOUT")).await;
    }
}

/// FETCH item list → record; None if it lacks what we need (e.g. an unsolicited flags update).
pub fn header_record(items: &[Val]) -> Option<HeaderRecord> {
    let mut rec = HeaderRecord::default();
    let (mut has_uid, mut has_id) = (false, false);
    for pair in items.chunks(2) {
        let [k, v] = pair else { continue };
        let Some(key) = k.atom() else { continue };
        let key = key.to_ascii_uppercase();
        match key.as_str() {
            "UID" => {
                rec.uid = v.num()? as u32;
                has_uid = true;
            }
            "X-GM-MSGID" => {
                rec.msgid = v.num()?;
                has_id = true;
            }
            "X-GM-THRID" => rec.thrid = v.num().unwrap_or(0),
            "X-GM-LABELS" => rec.labels = v.list().unwrap_or(&[]).iter().filter_map(gm_label).collect(),
            "FLAGS" => rec.flags = v.list().unwrap_or(&[]).iter().filter_map(Val::text).collect(),
            "INTERNALDATE" => rec.internal_date = v.text().and_then(|t| parse_internal_date(&t)),
            "RFC822.SIZE" => rec.size = v.num().unwrap_or(0) as i64,
            k if k.starts_with("BODY[HEADER") || k.starts_with("BODY.PEEK[HEADER") || k.starts_with("RFC822.HEADER") => {
                rec.header = v.bytes().map(<[u8]>::to_vec).unwrap_or_default();
            }
            _ => {}
        }
    }
    (has_uid && has_id).then_some(rec)
}

fn gm_label(v: &Val) -> Option<GmLabel> {
    let t = v.text()?;
    if t.is_empty() {
        return None;
    }
    // system labels come as atoms (\Inbox) — and sometimes quoted ("\\Important")
    if t.starts_with('\\') {
        return Some(GmLabel::System(t));
    }
    Some(GmLabel::User(mutf7_decode(&t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(raw: &str) -> Vec<Val> {
        match parse_resp(raw.as_bytes()).unwrap() {
            Resp::Data(v) => v,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parses_values() {
        let v = parse_values(b"a \"q \\\"x\\\" \\\\ y\" NIL (1 (2 3) \"\") {5}\r\nhe(l) z").unwrap();
        assert_eq!(v[0], Val::Atom("a".into()));
        assert_eq!(v[1], Val::Str(b"q \"x\" \\ y".to_vec()));
        assert_eq!(v[2], Val::Nil);
        assert_eq!(
            v[3],
            Val::List(vec![Val::Atom("1".into()), Val::List(vec![Val::Atom("2".into()), Val::Atom("3".into())]), Val::Str(vec![])])
        );
        assert_eq!(v[4], Val::Str(b"he(l)".to_vec()));
        assert_eq!(v[5], Val::Atom("z".into()));
        // section atoms keep their bracketed spaces and parens
        let v = parse_values(b"(BODY[HEADER.FIELDS (FROM TO)] {3}\r\nabc FLAGS (\\Seen \\*))").unwrap();
        let l = v[0].list().unwrap();
        assert_eq!(l[0], Val::Atom("BODY[HEADER.FIELDS (FROM TO)]".into()));
        assert_eq!(l[1], Val::Str(b"abc".to_vec()));
        assert_eq!(l[3], Val::List(vec![Val::Atom("\\Seen".into()), Val::Atom("\\*".into())]));
        // errors, not panics
        for bad in [&b"(a b"[..], b"\"abc", b"{10}\r\nabc", b"{x}\r\n", b")", b"{3}abc"] {
            assert!(parse_values(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
        let deep = "(".repeat(100) + &")".repeat(100);
        assert!(parse_values(deep.as_bytes()).is_err());
    }

    #[test]
    fn parses_gmail_fetch_response() {
        let hdr = "From: =?UTF-8?B?15PXoNeUINeb15TXnw==?= <dana@x.co.il>\r\nSubject: =?UTF-8?B?16LXk9eb15XXnyDXqNeR16LXldef15k=?=\r\n\r\n";
        let raw = format!(
            "* 7 FETCH (X-GM-THRID 1266894439832287888 X-GM-MSGID 1278455344230334865 X-GM-LABELS (\\Inbox \"\\\\Important\" \\Starred \"Muy Importante\" &BdwF5wXVBdcF1QXq- \"Clients/Orion\" {{12}}\r\nLong \"label\") UID 4242 FLAGS (\\Seen \\Flagged) INTERNALDATE \" 7-Jul-2026 02:44:25 +0300\" RFC822.SIZE 4286 BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{hdr})\r\n",
            hdr.len()
        );
        let v = data(&raw);
        assert!(v[1].is("FETCH"));
        let r = header_record(v[2].list().unwrap()).unwrap();
        assert_eq!(r.uid, 4242);
        assert_eq!(format!("{:016x}", r.msgid), "11bdfc5cae0c8191");
        assert_eq!(r.thrid, 1266894439832287888);
        assert_eq!(
            r.labels,
            vec![
                GmLabel::System("\\Inbox".into()),
                GmLabel::System("\\Important".into()),
                GmLabel::System("\\Starred".into()),
                GmLabel::User("Muy Importante".into()),
                GmLabel::User("לקוחות".into()),
                GmLabel::User("Clients/Orion".into()),
                GmLabel::User("Long \"label\"".into()),
            ]
        );
        assert_eq!(r.flags, vec!["\\Seen", "\\Flagged"]);
        assert_eq!(r.size, 4286);
        assert_eq!(r.internal_date, Some(chrono::DateTime::parse_from_rfc3339("2026-07-07T02:44:25+03:00").unwrap().timestamp_millis()));
        assert_eq!(r.header, hdr.as_bytes());
        // NIL header, unsolicited flag update
        let v = data("* 8 FETCH (UID 9 X-GM-MSGID 1 X-GM-LABELS () FLAGS () BODY[HEADER.FIELDS (FROM)] NIL)");
        assert!(header_record(v[2].list().unwrap()).unwrap().header.is_empty());
        let v = data("* 9 FETCH (FLAGS (\\Seen) UID 10)");
        assert!(header_record(v[2].list().unwrap()).is_none());
    }

    #[test]
    fn parses_status_and_list() {
        match parse_resp(b"c1 NO [ALERT] IMAP access is disabled for your domain. (Failure)\r\n").unwrap() {
            Resp::Status { tag, kind, code, text } => {
                assert_eq!((tag.as_str(), kind.as_str(), code.as_deref()), ("c1", "NO", Some("ALERT")));
                assert!(text.contains("disabled"));
            }
            r => panic!("{r:?}"),
        }
        match parse_resp(b"* OK [PERMANENTFLAGS (\\Seen \\*)] Flags permitted. (unbalanced :)\r\n").unwrap() {
            Resp::Status { code, .. } => assert_eq!(code.as_deref(), Some("PERMANENTFLAGS (\\Seen \\*)")),
            r => panic!("{r:?}"),
        }
        assert_eq!(parse_resp(b"+ \r\n").unwrap(), Resp::Cont("".into()));
        let v = data("* LIST (\\All \\HasNoChildren) \"/\" \"[Gmail]/&BdsF3A- &BdQF0wXVBdAF6A-\"\r\n");
        assert_eq!(v[1].list().unwrap()[0], Val::Atom("\\All".into()));
        assert_eq!(mutf7_decode(&v[3].text().unwrap()), "[Gmail]/כל הדואר");
        let v = data("* LIST (\\HasNoChildren) NIL INBOX");
        assert_eq!(v[2], Val::Nil);
        let v = data("* SEARCH 1 2 30 (MODSEQ 917162500)");
        assert_eq!(v.len(), 5);
    }

    #[test]
    fn literal_detection() {
        assert_eq!(literal_len_at_end(b"* 1 FETCH (BODY[] {123}\r\n"), Some(123));
        assert_eq!(literal_len_at_end(b"a LOGIN {5+}\r\n"), Some(5));
        assert_eq!(literal_len_at_end(b"* OK done\r\n"), None);
        assert_eq!(literal_len_at_end(b"* OK {}\r\n"), None);
        assert_eq!(literal_len_at_end(b"* OK {99999999999999999}\r\n"), None);
    }

    #[tokio::test]
    async fn reads_responses_with_literals() {
        let input = b"* 1 FETCH (BODY[] {5}\r\nab\r\nc X {2}\r\nzz)\r\n* OK next\r\n".to_vec();
        let mut r = BufReader::new(&input[..]);
        let one = read_response(&mut r, Duration::from_secs(1)).await.unwrap();
        assert_eq!(one, b"* 1 FETCH (BODY[] {5}\r\nab\r\nc X {2}\r\nzz)\r\n");
        let v = data(std::str::from_utf8(&one).unwrap());
        assert_eq!(v[2].list().unwrap()[1], Val::Str(b"ab\r\nc".to_vec()));
        let two = read_response(&mut r, Duration::from_secs(1)).await.unwrap();
        assert_eq!(two, b"* OK next\r\n");
        assert!(matches!(read_response(&mut r, Duration::from_secs(1)).await, Err(ImapError::Network(_))));
        // truncated literal
        let input = b"* 1 FETCH (BODY[] {50}\r\nshort".to_vec();
        let mut r = BufReader::new(&input[..]);
        assert!(read_response(&mut r, Duration::from_secs(1)).await.is_err());
    }

    #[test]
    fn modified_utf7() {
        for s in [
            "INBOX",
            "Clients/Orion Technologies",
            "לקוחות",
            "[Gmail]/כל הדואר",
            "R&D",
            "a&b ✓ 日本語",
            "Conferences/\"Tel Aviv\" 2026",
            "😀",
        ] {
            let e = mutf7_encode(s);
            assert!(e.is_ascii(), "{e}");
            assert_eq!(mutf7_decode(&e), s, "{e}");
        }
        assert_eq!(mutf7_encode("R&D"), "R&-D");
        assert_eq!(mutf7_encode("לקוחות"), "&BdwF5wXVBdcF1QXq-");
        assert_eq!(mutf7_decode("&Jjo-!"), "☺!");
        assert_eq!(mutf7_decode("~peter/mail/&U,BTFw-/&ZeVnLIqe-"), "~peter/mail/台北/日本語");
        // malformed input comes back unchanged
        assert_eq!(mutf7_decode("a&b"), "a&b");
        assert_eq!(mutf7_decode("&@@@-x"), "&@@@-x");
    }

    #[test]
    fn misc_helpers() {
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(STANDARD.decode(xoauth2("k@x.com", "tok")).unwrap(), b"user=k@x.com\x01auth=Bearer tok\x01\x01");
        assert_eq!(parse_internal_date("17-Jul-1996 02:44:25 -0700"), Some(837596665000));
        assert_eq!(parse_internal_date(" 1-Jan-2026 00:00:00 +0000"), Some(1767225600000));
        assert_eq!(parse_internal_date("garbage"), None);
        assert!(parse_url("imaps://imap.gmail.com:993").unwrap().0);
        assert_eq!(parse_url("imap://127.0.0.1:1430").unwrap(), (false, "127.0.0.1".into(), 1430));
        assert!(parse_url("imap://imap.gmail.com:143").unwrap_err().is_permanent());
        assert!(parse_url("http://x").is_err());
    }
}

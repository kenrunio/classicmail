//! MAPI properties of archived items (independent of the PST reader, so the rules are unit
//! tested): list headers from a contents-table row / message, and the body to show
//! (PR_HTML → compressed RTF → PR_BODY).

use std::collections::HashMap;

use super::{codepage, lzfu, rtf};

pub mod tag {
    pub const IMPORTANCE: u16 = 0x0017;
    pub const MESSAGE_CLASS: u16 = 0x001A;
    pub const SUBJECT: u16 = 0x0037;
    pub const CLIENT_SUBMIT_TIME: u16 = 0x0039;
    pub const SENT_REPR_NAME: u16 = 0x0042;
    pub const SENT_REPR_ADDRTYPE: u16 = 0x0064;
    pub const SENT_REPR_EMAIL: u16 = 0x0065;
    pub const CONVERSATION_TOPIC: u16 = 0x0070;
    pub const TRANSPORT_HEADERS: u16 = 0x007D;
    pub const SENDER_NAME: u16 = 0x0C1A;
    pub const SENDER_ADDRTYPE: u16 = 0x0C1E;
    pub const SENDER_EMAIL: u16 = 0x0C1F;
    pub const RECIPIENT_TYPE: u16 = 0x0C15;
    pub const DISPLAY_BCC: u16 = 0x0E02;
    pub const DISPLAY_CC: u16 = 0x0E03;
    pub const DISPLAY_TO: u16 = 0x0E04;
    pub const DELIVERY_TIME: u16 = 0x0E06;
    pub const MESSAGE_FLAGS: u16 = 0x0E07;
    pub const MESSAGE_SIZE: u16 = 0x0E08;
    pub const ATTACH_SIZE: u16 = 0x0E20;
    pub const BODY: u16 = 0x1000;
    pub const RTF_COMPRESSED: u16 = 0x1009;
    pub const HTML: u16 = 0x1013;
    pub const INTERNET_MESSAGE_ID: u16 = 0x1035;
    pub const ICON_INDEX: u16 = 0x1080;
    pub const LAST_VERB: u16 = 0x1081;
    pub const FLAG_STATUS: u16 = 0x1090;
    pub const DISPLAY_NAME: u16 = 0x3001;
    pub const ADDRTYPE: u16 = 0x3002;
    pub const EMAIL: u16 = 0x3003;
    pub const CREATION_TIME: u16 = 0x3007;
    pub const CONTAINER_CLASS: u16 = 0x3613;
    pub const ATTACH_DATA: u16 = 0x3701;
    pub const ATTACH_EXTENSION: u16 = 0x3703;
    pub const ATTACH_FILENAME: u16 = 0x3704;
    pub const ATTACH_METHOD: u16 = 0x3705;
    pub const ATTACH_LONG_FILENAME: u16 = 0x3707;
    pub const ATTACH_MIME: u16 = 0x370E;
    pub const ATTACH_CONTENT_ID: u16 = 0x3712;
    pub const ATTACH_FLAGS: u16 = 0x3714;
    pub const SMTP_ADDRESS: u16 = 0x39FE;
    pub const INTERNET_CPID: u16 = 0x3FDE;
    pub const MESSAGE_CODEPAGE: u16 = 0x3FFD;
    pub const SENDER_SMTP: u16 = 0x5D01;
    pub const SENT_REPR_SMTP: u16 = 0x5D02;
    pub const ATTACH_HIDDEN: u16 = 0x7FFE;
}

/// The contents-table / message properties the index uses.
pub const HEADER_TAGS: &[u16] = &[
    tag::IMPORTANCE,
    tag::MESSAGE_CLASS,
    tag::SUBJECT,
    tag::CLIENT_SUBMIT_TIME,
    tag::SENT_REPR_NAME,
    tag::SENT_REPR_ADDRTYPE,
    tag::SENT_REPR_EMAIL,
    tag::CONVERSATION_TOPIC,
    tag::SENDER_NAME,
    tag::SENDER_ADDRTYPE,
    tag::SENDER_EMAIL,
    tag::DISPLAY_CC,
    tag::DISPLAY_TO,
    tag::DELIVERY_TIME,
    tag::MESSAGE_FLAGS,
    tag::MESSAGE_SIZE,
    tag::ICON_INDEX,
    tag::LAST_VERB,
    tag::FLAG_STATUS,
    tag::CREATION_TIME,
    tag::SENDER_SMTP,
    tag::SENT_REPR_SMTP,
    tag::MESSAGE_CODEPAGE,
];

#[derive(Debug, Clone, PartialEq)]
pub enum PVal {
    Int(i64),
    Bool(bool),
    /// FILETIME (100 ns since 1601)
    Time(i64),
    /// 8-bit string in an external code page
    Str8(Vec<u8>),
    Str(String),
    Bin(Vec<u8>),
    Other,
}

#[derive(Debug, Clone, Default)]
pub struct Props(pub HashMap<u16, PVal>);

/// FILETIME → epoch ms (None for 0 / out of range).
pub fn filetime_ms(ft: i64) -> Option<i64> {
    if ft <= 0 {
        return None;
    }
    let ms = ft / 10_000 - 11_644_473_600_000;
    // before 1980 or after 2100: not a real message date
    (315_532_800_000..4_102_444_800_000).contains(&ms).then_some(ms)
}

impl Props {
    pub fn set(&mut self, id: u16, v: PVal) {
        self.0.insert(id, v);
    }

    pub fn has(&self, id: u16) -> bool {
        self.0.contains_key(&id)
    }

    pub fn int(&self, id: u16) -> Option<i64> {
        match self.0.get(&id)? {
            PVal::Int(v) => Some(*v),
            PVal::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    pub fn bool(&self, id: u16) -> Option<bool> {
        match self.0.get(&id)? {
            PVal::Bool(b) => Some(*b),
            PVal::Int(v) => Some(*v != 0),
            _ => None,
        }
    }

    pub fn time_ms(&self, id: u16) -> Option<i64> {
        match self.0.get(&id)? {
            PVal::Time(t) => filetime_ms(*t),
            _ => None,
        }
    }

    pub fn bin(&self, id: u16) -> Option<&[u8]> {
        match self.0.get(&id)? {
            PVal::Bin(b) | PVal::Str8(b) => Some(b),
            _ => None,
        }
    }

    /// The code page of this item's 8-bit strings, when it says.
    pub fn codepage(&self) -> Option<u32> {
        [tag::MESSAGE_CODEPAGE, tag::INTERNET_CPID]
            .iter()
            .filter_map(|t| self.int(*t))
            .find(|cp| *cp > 0 && codepage::encoding(*cp as u32).is_some())
            .map(|cp| cp as u32)
    }

    /// A string property (8-bit strings decoded with the item's code page, else guessed).
    /// Mis-labelled Hebrew (see `codepage::repair_hebrew`) is repaired.
    pub fn str(&self, id: u16) -> Option<String> {
        let s = match self.0.get(&id)? {
            PVal::Str(s) => s.clone(),
            PVal::Str8(b) => {
                let cp = match self.codepage() {
                    Some(cp) if !codepage::is_weak_latin(cp) => cp,
                    _ => codepage::guess(b),
                };
                codepage::decode(b, cp)
            }
            _ => return None,
        };
        let s = s.trim_end_matches('\0');
        Some(codepage::repair_hebrew(s).unwrap_or_else(|| s.to_string()))
    }

    /// Non-empty trimmed string.
    pub fn text(&self, id: u16) -> Option<String> {
        self.str(id).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }
}

/// `PR_SUBJECT` as stored in a PST may start with `\x01` + a prefix-length character.
pub fn clean_subject(s: &str) -> String {
    let mut it = s.chars();
    if it.next() == Some('\u{1}') {
        it.next();
        return it.as_str().trim().to_string();
    }
    s.trim().to_string()
}

/// An SMTP address from (address type, address, SMTP address) — EX / X.500 addresses don't count.
pub fn smtp_of(addrtype: Option<String>, email: Option<String>, smtp: Option<String>) -> Option<String> {
    if let Some(s) = smtp.filter(|s| s.contains('@')) {
        return Some(s);
    }
    let ex = addrtype.as_deref().map(|t| t.eq_ignore_ascii_case("EX")).unwrap_or(false);
    email.filter(|e| !ex && e.contains('@') && !e.starts_with('/'))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Header {
    pub message_class: String,
    pub subject: String,
    pub from_name: String,
    pub from_email: String,
    pub to_display: String,
    pub cc_display: String,
    pub date: i64,
    pub sent: Option<i64>,
    pub size: i64,
    pub is_read: bool,
    pub flag_status: Option<&'static str>,
    pub has_attachments: bool,
    /// 0 low, 1 normal, 2 high
    pub importance: i64,
    pub conversation_topic: String,
    pub last_verb: Option<&'static str>,
}

pub fn header(p: &Props) -> Header {
    let from_email = smtp_of(p.text(tag::SENDER_ADDRTYPE), p.text(tag::SENDER_EMAIL), p.text(tag::SENDER_SMTP))
        .or_else(|| smtp_of(p.text(tag::SENT_REPR_ADDRTYPE), p.text(tag::SENT_REPR_EMAIL), p.text(tag::SENT_REPR_SMTP)))
        .unwrap_or_default();
    let from_name = p.text(tag::SENT_REPR_NAME).or_else(|| p.text(tag::SENDER_NAME)).unwrap_or_else(|| from_email.clone());
    let flags = p.int(tag::MESSAGE_FLAGS).unwrap_or(1);
    let sent = p.time_ms(tag::CLIENT_SUBMIT_TIME);
    let date = p.time_ms(tag::DELIVERY_TIME).or(sent).or_else(|| p.time_ms(tag::CREATION_TIME)).unwrap_or(0);
    let last_verb = match p.int(tag::LAST_VERB) {
        Some(102) => Some("replied"),
        Some(103) => Some("repliedAll"),
        Some(104) => Some("forwarded"),
        _ => match p.int(tag::ICON_INDEX) {
            Some(0x105) => Some("replied"),
            Some(0x106) => Some("forwarded"),
            _ => None,
        },
    };
    let subject = p.str(tag::SUBJECT).map(|s| clean_subject(&s)).unwrap_or_default();
    Header {
        message_class: p.text(tag::MESSAGE_CLASS).unwrap_or_else(|| "IPM.Note".into()),
        conversation_topic: p.text(tag::CONVERSATION_TOPIC).unwrap_or_else(|| crate::mail::mime::normalize_subject(&subject)),
        subject,
        from_name,
        from_email,
        to_display: p.text(tag::DISPLAY_TO).unwrap_or_default(),
        cc_display: p.text(tag::DISPLAY_CC).unwrap_or_default(),
        date,
        sent,
        size: p.int(tag::MESSAGE_SIZE).unwrap_or(0).max(0),
        is_read: flags & 0x1 != 0,
        flag_status: match p.int(tag::FLAG_STATUS) {
            Some(2) => Some("flagged"),
            Some(1) => Some("complete"),
            _ => None,
        },
        has_attachments: flags & 0x10 != 0,
        importance: p.int(tag::IMPORTANCE).unwrap_or(1).clamp(0, 2),
        last_verb,
    }
}

/// What to show: sanitisable HTML (if any) and the plain text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Body {
    pub html: Option<String>,
    pub text: String,
    /// "html" | "rtf-html" | "rtf" | "text" | "none" (diagnostics / tests)
    pub source: &'static str,
}

fn meta_charset(bytes: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    let i = head.find("charset=")?;
    let rest = &head[i + 8..];
    let rest = rest.trim_start_matches(['"', '\'']);
    let cs: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (!cs.is_empty()).then_some(cs)
}

/// Decode PR_HTML bytes: PR_INTERNET_CPID, else the document's `<meta charset>`, else UTF-8/1252.
pub fn decode_html(bytes: &[u8], cpid: Option<u32>) -> String {
    let meta = meta_charset(bytes);
    // a Latin-1 PR_INTERNET_CPID loses against the document's own meta charset
    let s = match cpid.filter(|cp| !codepage::is_weak_latin(*cp) || meta.is_none()).and_then(codepage::encoding) {
        Some(enc) => enc.decode(bytes).0.into_owned(),
        None => crate::mail::mime::decode_charset(bytes, meta.as_deref()),
    };
    codepage::repair_hebrew(&s).unwrap_or(s)
}

fn wrap_rich(fragment: &str) -> String {
    format!("<div style=\"font-family:Calibri,Arial,sans-serif;font-size:11pt\">{fragment}</div>")
}

pub fn body(p: &Props) -> Body {
    let plain = p.str(tag::BODY).filter(|s| !s.trim().is_empty());
    let cpid = p.int(tag::INTERNET_CPID).filter(|c| *c > 0).map(|c| c as u32).or_else(|| p.codepage());
    let html_prop = match p.0.get(&tag::HTML) {
        Some(PVal::Bin(b)) | Some(PVal::Str8(b)) if !b.is_empty() => Some(decode_html(b, cpid)),
        Some(PVal::Str(s)) if !s.trim().is_empty() => Some(s.clone()),
        _ => None,
    };
    let text_of = |html: &str| plain.clone().unwrap_or_else(|| crate::mail::mime::html_to_text(html));
    if let Some(h) = html_prop.filter(|h| !h.trim().is_empty()) {
        return Body { text: text_of(&h), html: Some(h), source: "html" };
    }
    if let Some(raw) = p.bin(tag::RTF_COMPRESSED).filter(|b| b.len() >= 16) {
        match lzfu::decompress(raw) {
            Ok(rtf_bytes) => match rtf::convert(&rtf_bytes) {
                rtf::Rtf::Html(h) if !h.trim().is_empty() => return Body { text: text_of(&h), html: Some(h), source: "rtf-html" },
                rtf::Rtf::Text(t) => {
                    let t = plain.clone().unwrap_or(t);
                    return Body { html: None, text: t, source: "text" };
                }
                rtf::Rtf::Rich(h) if !h.trim().is_empty() => {
                    let html = wrap_rich(&h);
                    return Body { text: text_of(&html), html: Some(html), source: "rtf" };
                }
                _ => {}
            },
            Err(e) => log::debug!("archive: compressed RTF: {}", crate::logging::redact(&e.to_string())),
        }
    }
    match plain {
        Some(t) => Body { html: None, text: t, source: "text" },
        None => Body { html: None, text: String::new(), source: "none" },
    }
}

/// A header value from PR_TRANSPORT_MESSAGE_HEADERS (unfolded, RFC 2047 decoded).
pub fn transport_header(headers: &str, name: &str) -> Option<String> {
    let mut lines = headers.split("\r\n").flat_map(|l| l.split('\n'));
    let mut cur: Option<String> = None;
    let want = format!("{}:", name.to_ascii_lowercase());
    for l in lines.by_ref() {
        if let Some(c) = cur.as_mut() {
            if l.starts_with(' ') || l.starts_with('\t') {
                c.push(' ');
                c.push_str(l.trim());
                continue;
            }
            break;
        }
        if l.to_ascii_lowercase().starts_with(&want) {
            cur = Some(l[want.len()..].trim().to_string());
        }
    }
    cur.map(|v| crate::mail::mime::decode_rfc2047(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ft(ms: i64) -> PVal {
        PVal::Time((ms + 11_644_473_600_000) * 10_000)
    }

    #[test]
    fn header_from_contents_row() {
        let mut p = Props::default();
        p.set(tag::SUBJECT, PVal::Str("\u{1}\u{4}RE: Q3 results".into()));
        p.set(tag::SENDER_NAME, PVal::Str("Sarah Cohen".into()));
        p.set(tag::SENDER_ADDRTYPE, PVal::Str("EX".into()));
        p.set(tag::SENDER_EMAIL, PVal::Str("/O=HARBOR/OU=FIRST/CN=RECIPIENTS/CN=SARAH".into()));
        p.set(tag::SENDER_SMTP, PVal::Str("sarah@harborcap.example".into()));
        p.set(tag::DISPLAY_TO, PVal::Str("Kenny Green; David Levi".into()));
        p.set(tag::DELIVERY_TIME, ft(1_200_000_000_000));
        p.set(tag::CLIENT_SUBMIT_TIME, ft(1_199_999_990_000));
        p.set(tag::MESSAGE_FLAGS, PVal::Int(0x10));
        p.set(tag::IMPORTANCE, PVal::Int(2));
        p.set(tag::FLAG_STATUS, PVal::Int(2));
        p.set(tag::LAST_VERB, PVal::Int(104));
        p.set(tag::MESSAGE_SIZE, PVal::Int(12345));
        let h = header(&p);
        assert_eq!(h.subject, "RE: Q3 results");
        assert_eq!(h.conversation_topic, "Q3 results");
        assert_eq!(h.from_name, "Sarah Cohen");
        assert_eq!(h.from_email, "sarah@harborcap.example");
        assert_eq!(h.date, 1_200_000_000_000);
        assert_eq!(h.sent, Some(1_199_999_990_000));
        assert!(!h.is_read && h.has_attachments);
        assert_eq!((h.importance, h.flag_status, h.last_verb, h.size), (2, Some("flagged"), Some("forwarded"), 12345));
        assert_eq!(h.message_class, "IPM.Note");
    }

    #[test]
    fn ex_sender_without_smtp_falls_back_to_sent_representing() {
        let mut p = Props::default();
        p.set(tag::SENDER_ADDRTYPE, PVal::Str("EX".into()));
        p.set(tag::SENDER_EMAIL, PVal::Str("/o=ExchangeLabs/ou=x/cn=Recipients/cn=abc".into()));
        p.set(tag::SENT_REPR_ADDRTYPE, PVal::Str("SMTP".into()));
        p.set(tag::SENT_REPR_EMAIL, PVal::Str("noa@oriontech.example".into()));
        p.set(tag::MESSAGE_FLAGS, PVal::Int(1));
        let h = header(&p);
        assert_eq!(h.from_email, "noa@oriontech.example");
        assert_eq!(h.from_name, "noa@oriontech.example");
        assert!(h.is_read);
        assert_eq!(h.date, 0);
    }

    #[test]
    fn ansi_strings_use_the_message_code_page() {
        let mut p = Props::default();
        p.set(tag::SUBJECT, PVal::Str8(vec![0xf9, 0xec, 0xe5, 0xed]));
        p.set(tag::SENDER_NAME, PVal::Str8(b"Ren\xe9".to_vec()));
        // no code page: Hebrew guessed for the subject, Latin-1 for the name
        let h = header(&p);
        assert_eq!(h.subject, "שלום");
        assert_eq!(h.from_name, "René");
        p.set(tag::MESSAGE_CODEPAGE, PVal::Int(1255));
        assert_eq!(header(&p).from_name, "Renי");
        // Hebrew mislabelled as ISO-8859-1 (common in the 2000s)
        let mut q = Props::default();
        q.set(tag::INTERNET_CPID, PVal::Int(28591));
        q.set(tag::SUBJECT, PVal::Str8(vec![0xe4, 0xe5, 0xe3, 0xf2, 0xe4]));
        assert_eq!(header(&q).subject, "הודעה");
        q.set(tag::HTML, PVal::Bin(b"<meta charset=\"windows-1255\"><p>\xf9\xec\xe5\xed</p>".to_vec()));
        assert!(body(&q).html.unwrap().contains("<p>שלום</p>"));
        // UTF-8 HTML of text that was mis-decoded upstream
        q.set(tag::INTERNET_CPID, PVal::Int(65001));
        q.set(tag::HTML, PVal::Bin("<p>ùìåí</p>".as_bytes().to_vec()));
        assert_eq!(body(&q).html.as_deref(), Some("<p>שלום</p>"));
    }

    #[test]
    fn body_prefers_html_then_rtf_then_text() {
        let mut p = Props::default();
        p.set(tag::BODY, PVal::Str("plain body".into()));
        assert_eq!(body(&p), Body { html: None, text: "plain body".into(), source: "text" });

        // RTF (plain RTF) beats PR_BODY
        let rtf = b"{\\rtf1\\ansi\\ansicpg1255 \\b \\'f9\\'ec\\'e5\\'ed\\b0\\par}";
        p.set(tag::RTF_COMPRESSED, PVal::Bin(lzfu::compress_literal(rtf)));
        let b = body(&p);
        assert_eq!(b.source, "rtf");
        assert!(b.html.as_deref().unwrap().contains("<span style=\"font-weight:bold;\">שלום</span><br>"), "{b:?}");
        assert_eq!(b.text, "plain body");

        // encapsulated HTML in RTF
        let enc = b"{\\rtf1\\ansi\\fromhtml1 {\\*\\htmltag <p>}hi{\\*\\htmltag </p>}}";
        p.set(tag::RTF_COMPRESSED, PVal::Bin(lzfu::compress_literal(enc)));
        assert_eq!(body(&p).html.as_deref(), Some("<p>hi</p>"));
        assert_eq!(body(&p).source, "rtf-html");

        // \fromtext RTF: PR_BODY is the original
        let ft = b"{\\rtf1\\ansi\\fromtext line\\par}";
        p.set(tag::RTF_COMPRESSED, PVal::Bin(lzfu::compress_literal(ft)));
        assert_eq!(body(&p), Body { html: None, text: "plain body".into(), source: "text" });

        // PR_HTML wins, decoded with PR_INTERNET_CPID
        p.set(tag::HTML, PVal::Bin(b"<p>\xf9\xec\xe5\xed</p>".to_vec()));
        p.set(tag::INTERNET_CPID, PVal::Int(1255));
        assert_eq!(body(&p).html.as_deref(), Some("<p>שלום</p>"));
        assert_eq!(body(&p).source, "html");
        // ... or with the document's meta charset
        p.0.remove(&tag::INTERNET_CPID);
        p.set(tag::HTML, PVal::Bin(b"<meta charset=\"windows-1255\"><p>\xf9\xec\xe5\xed</p>".to_vec()));
        assert!(body(&p).html.unwrap().contains("שלום"));

        assert_eq!(body(&Props::default()).source, "none");
    }

    #[test]
    fn transport_headers() {
        let h = "Received: from x\r\n\tby y\r\nFrom: =?utf-8?B?16nXnNeV150=?=\r\n <a@b.example>\r\nSubject: hi\r\n";
        assert_eq!(transport_header(h, "from").as_deref(), Some("שלום <a@b.example>"));
        assert_eq!(transport_header(h, "Subject").as_deref(), Some("hi"));
        assert_eq!(transport_header(h, "To"), None);
    }
}

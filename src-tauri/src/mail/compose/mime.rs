//! RFC 822 building with mail-builder:
//!
//! ```text
//! multipart/mixed                      (only with attachments)
//!   multipart/alternative
//!     text/plain
//!     multipart/related                (only with inline images)
//!       text/html
//!       image/* (Content-ID)
//!   application/... (attachments)
//! ```

use base64::Engine;
use chrono::{DateTime, FixedOffset};
use mail_builder::{
    MessageBuilder,
    headers::{address::Address, content_type::ContentType, message_id::MessageId, raw::Raw, text::Text},
    mime::MimePart,
};
use regex::Regex;

use super::model::{Addr, Compose, Importance};

/// A resolved attachment (bytes in hand).
#[derive(Debug, Clone)]
pub struct Part {
    pub filename: String,
    pub mime: String,
    pub bytes: Vec<u8>,
    /// inline image referenced as cid:<cid>
    pub cid: Option<String>,
}

/// Headers decided by the caller (not part of the compose itself).
#[derive(Debug, Clone)]
pub struct Envelope {
    pub from: Addr,
    /// without angle brackets
    pub message_id: String,
    pub date: DateTime<FixedOffset>,
    /// without angle brackets
    pub in_reply_to: Option<String>,
    /// without angle brackets, oldest first
    pub references: Vec<String>,
}

pub fn new_message_id(from_email: &str) -> String {
    let domain = from_email.rsplit_once('@').map(|(_, d)| d).filter(|d| !d.is_empty()).unwrap_or("classicmail.local");
    format!("{}@{}", uuid::Uuid::new_v4().simple(), domain)
}

/// Strip `<>` and whitespace; split a References header into ids.
pub fn split_ids(s: &str) -> Vec<String> {
    s.split(|c: char| c.is_whitespace() || c == ',')
        .map(|t| t.trim().trim_start_matches('<').trim_end_matches('>').to_string())
        .filter(|t| !t.is_empty() && t.contains('@'))
        .collect()
}

fn data_url_re() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)src\s*=\s*(["'])data:([a-z0-9.+/-]+);base64,([A-Za-z0-9+/=\s]+)(["'])"#).unwrap())
}

/// Replace `src="data:image/...;base64,..."` images by `cid:` references and return the parts.
pub fn extract_data_images(html: &str) -> (String, Vec<Part>) {
    let mut parts = Vec::new();
    let out = data_url_re().replace_all(html, |caps: &regex::Captures| {
        let mime = caps[2].to_lowercase();
        let b64: String = caps[3].chars().filter(|c| !c.is_whitespace()).collect();
        match base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()) {
            Ok(bytes) => {
                let n = parts.len() + 1;
                let cid = format!("image{:03}.{}@classicmail", n, &uuid::Uuid::new_v4().simple().to_string()[..12]);
                let ext = mime_guess::get_mime_extensions_str(&mime).and_then(|e| e.first().copied()).unwrap_or("png");
                parts.push(Part { filename: format!("image{n:03}.{ext}"), mime, bytes, cid: Some(cid.clone()) });
                format!("src=\"cid:{cid}\"")
            }
            Err(_) => caps[0].to_string(),
        }
    });
    (out.into_owned(), parts)
}

/// Turn an HTML fragment into a full document (Word-mail style defaults).
pub fn html_document(html: &str) -> String {
    let lower = html.trim_start().get(..200.min(html.trim_start().len())).unwrap_or("").to_ascii_lowercase();
    if lower.starts_with("<!doctype") || lower.starts_with("<html") {
        return html.to_string();
    }
    format!(
        "<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\">\
         <meta name=\"Generator\" content=\"ClassicMail\">\
         <style>p{{margin:0}} p.MsoNormal{{margin:0}}</style></head><body>{html}</body></html>"
    )
}

/// Plain-text rendering of an HTML body (the text/plain alternative).
pub fn html_to_text(html: &str) -> String {
    if crate::mail::mime::html_too_deep(html) {
        return crate::mail::mime::strip_tags(html);
    }
    let t = html2text::config::plain_no_decorate().link_footnotes(false).string_from_read(html.as_bytes(), 78).unwrap_or_default();
    // html2text keeps trailing spaces on wrapped lines; trim them
    t.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\r\n")
}

fn addr_list(v: &[Addr]) -> Address<'static> {
    Address::new_list(
        v.iter()
            .map(|a| {
                let name = a.name.trim();
                Address::new_address(
                    if name.is_empty() || name.eq_ignore_ascii_case(&a.email) { None } else { Some(name.to_string()) },
                    a.email.trim().to_string(),
                )
            })
            .collect(),
    )
}

fn rfc2822(d: &DateTime<FixedOffset>) -> String {
    d.format("%a, %d %b %Y %H:%M:%S %z").to_string()
}

/// Build the RFC 822 message.
pub fn build(c: &Compose, env: &Envelope, attachments: Vec<Part>) -> anyhow::Result<Vec<u8>> {
    let mut b = MessageBuilder::new()
        .from(addr_list(std::slice::from_ref(&env.from)))
        .subject(Text::new(c.subject.clone()))
        .header("Date", Raw::new(rfc2822(&env.date)))
        .message_id(MessageId::new(env.message_id.clone()));
    if !c.to.is_empty() {
        b = b.to(addr_list(&c.to));
    }
    if !c.cc.is_empty() {
        b = b.cc(addr_list(&c.cc));
    }
    if !c.bcc.is_empty() {
        b = b.bcc(addr_list(&c.bcc));
    }
    if !c.reply_to.is_empty() {
        b = b.reply_to(addr_list(&c.reply_to));
    }
    if let Some(irt) = &env.in_reply_to {
        b = b.in_reply_to(MessageId::new(irt.clone()));
    }
    if !env.references.is_empty() {
        b = b.references(MessageId::new_list(env.references.clone().into_iter()));
    }
    match c.importance {
        Importance::High => {
            b = b.header("X-Priority", Raw::new("1 (Highest)")).header("Importance", Raw::new("High"));
        }
        Importance::Low => {
            b = b.header("X-Priority", Raw::new("5 (Lowest)")).header("Importance", Raw::new("Low"));
        }
        Importance::Normal => {}
    }
    if c.request_read_receipt {
        b = b.header("Disposition-Notification-To", addr_list(std::slice::from_ref(&env.from)));
    }
    if c.request_delivery_receipt {
        b = b.header("Return-Receipt-To", addr_list(std::slice::from_ref(&env.from)));
    }
    if let Some(f) = c.flag_for_recipients.as_deref().filter(|f| !f.trim().is_empty()) {
        b = b.header("X-Message-Flag", Text::new(f.trim().to_string()));
    }
    b = b.header("X-Mailer", Raw::new(concat!("ClassicMail ", env!("CARGO_PKG_VERSION"))));

    let (mut inline, files): (Vec<Part>, Vec<Part>) = attachments.into_iter().partition(|p| p.cid.is_some());

    let text_body = c.text.clone().filter(|t| !t.trim().is_empty() || c.html.trim().is_empty());
    let body = if c.plain_text {
        let text = text_body.unwrap_or_else(|| html_to_text(&c.html));
        MimePart::new("text/plain", crlf(&text))
    } else {
        let (html, data_parts) = extract_data_images(&c.html);
        // only keep inline parts that the HTML still references
        inline.extend(data_parts);
        inline.retain(|p| p.cid.as_ref().map(|cid| html.contains(&format!("cid:{cid}"))).unwrap_or(false));
        let text = text_body.unwrap_or_else(|| html_to_text(&html));
        let html_part = MimePart::new("text/html", html_document(&html));
        let html_or_related = if inline.is_empty() {
            html_part
        } else {
            let mut v = vec![html_part];
            for p in inline {
                let cid = p.cid.clone().unwrap_or_default();
                v.push(MimePart::new(ContentType::new(p.mime).attribute("name", p.filename.clone()), p.bytes).inline().cid(cid));
            }
            MimePart::new(ContentType::new("multipart/related").attribute("type", "text/html"), v)
        };
        MimePart::new("multipart/alternative", vec![MimePart::new("text/plain", crlf(&text)), html_or_related])
    };
    let root = if files.is_empty() {
        body
    } else {
        let mut v = vec![body];
        for p in files {
            v.push(MimePart::new(ContentType::new(p.mime).attribute("name", p.filename.clone()), p.bytes).attachment(p.filename));
        }
        MimePart::new("multipart/mixed", v)
    };
    Ok(b.body(root).write_to_vec()?)
}

fn crlf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Replace the Date header of a built message (deferred delivery is stamped when it is sent).
pub fn restamp_date(raw: &[u8], date: &DateTime<FixedOffset>) -> Vec<u8> {
    let Some(end) = find_header_end(raw) else { return raw.to_vec() };
    let head = String::from_utf8_lossy(&raw[..end]);
    let mut out = String::with_capacity(head.len());
    let mut replaced = false;
    let mut skipping = false;
    for line in head.split_inclusive("\r\n") {
        if skipping && (line.starts_with(' ') || line.starts_with('\t')) {
            continue;
        }
        skipping = false;
        if !replaced && line.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("date:")) {
            out.push_str(&format!("Date: {}\r\n", rfc2822(date)));
            replaced = true;
            skipping = true;
            continue;
        }
        out.push_str(line);
    }
    let mut v = out.into_bytes();
    v.extend_from_slice(&raw[end..]);
    v
}

/// Byte offset just after the header block's last header line (points at the blank line).
fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 2)
}

/// Mime type from a file name.
pub fn guess_mime(filename: &str) -> String {
    mime_guess::from_path(filename).first_raw().unwrap_or("application/octet-stream").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::compose::model::{Addr, Compose};
    use mail_parser::{MessageParser, MimeHeaders};

    fn env() -> Envelope {
        Envelope {
            from: Addr::new("קני גרין", "kenny@duvz.demo"),
            message_id: "abc123@duvz.demo".into(),
            date: chrono::DateTime::parse_from_rfc3339("2026-10-11T10:15:00+03:00").unwrap(),
            in_reply_to: Some("orig@example.com".into()),
            references: vec!["root@example.com".into(), "orig@example.com".into()],
        }
    }

    #[test]
    fn hebrew_round_trip_with_inline_and_attachment() {
        let png = base64::engine::general_purpose::STANDARD.encode([0x89u8, b'P', b'N', b'G', 1, 2, 3, 4]);
        let c = Compose {
            account_id: 1,
            to: vec![Addr::new("שרה כהן", "sarah@harborcap.example")],
            cc: vec![Addr::new("David Levi", "david@meridian.example")],
            bcc: vec![Addr::new("", "secret@example.com")],
            subject: "RE: דוח רבעוני – Q3 results".into(),
            html: format!("<p dir=\"auto\">שלום <b>עולם</b></p><p><img src=\"data:image/png;base64,{png}\"></p>"),
            importance: Importance::High,
            request_read_receipt: true,
            ..Default::default()
        };
        let att = Part { filename: "סיכום.pdf".into(), mime: "application/pdf".into(), bytes: b"%PDF-1.4 test".to_vec(), cid: None };
        let raw = build(&c, &env(), vec![att]).unwrap();
        assert!(raw.is_ascii(), "message must be 7-bit");
        let m = MessageParser::default().parse(&raw).unwrap();
        assert_eq!(m.subject().unwrap(), "RE: דוח רבעוני – Q3 results");
        let from = m.from().unwrap().first().unwrap();
        assert_eq!(from.name().unwrap(), "קני גרין");
        assert_eq!(from.address().unwrap(), "kenny@duvz.demo");
        assert_eq!(m.to().unwrap().first().unwrap().name().unwrap(), "שרה כהן");
        assert_eq!(m.cc().unwrap().first().unwrap().address().unwrap(), "david@meridian.example");
        assert_eq!(m.bcc().unwrap().first().unwrap().address().unwrap(), "secret@example.com");
        assert_eq!(m.message_id().unwrap(), "abc123@duvz.demo");
        assert_eq!(m.in_reply_to().as_text().unwrap(), "orig@example.com");
        assert_eq!(m.references().as_text_list().unwrap().len(), 2);
        assert_eq!(m.header_raw("X-Priority").unwrap().trim(), "1 (Highest)");
        assert!(m.header_raw("Disposition-Notification-To").unwrap().contains("kenny@duvz.demo"));
        let html = m.body_html(0).unwrap();
        assert!(html.contains("שלום <b>עולם</b>"));
        assert!(html.contains("src=\"cid:image001."));
        let text = m.body_text(0).unwrap();
        assert!(text.contains("שלום"));
        let atts: Vec<_> = m.attachments().collect();
        assert_eq!(atts.len(), 2, "inline image + pdf");
        let pdf = atts.iter().find(|a| a.attachment_name() == Some("סיכום.pdf")).expect("pdf");
        assert_eq!(pdf.contents(), b"%PDF-1.4 test");
        let img = atts.iter().find(|a| a.content_id().is_some()).expect("inline");
        assert_eq!(&img.contents()[..4], &[0x89, b'P', b'N', b'G']);
        // structure
        let s = String::from_utf8_lossy(&raw);
        assert!(s.contains("multipart/mixed"));
        assert!(s.contains("multipart/alternative"));
        assert!(s.contains("multipart/related"));
    }

    #[test]
    fn plain_text_only() {
        let c = Compose {
            to: vec![Addr::new("", "a@b.example")],
            subject: "Plain".into(),
            html: "<p>Hello <b>there</b></p>".into(),
            plain_text: true,
            ..Default::default()
        };
        let raw = build(&c, &env(), vec![]).unwrap();
        let m = MessageParser::default().parse(&raw).unwrap();
        assert!(m.body_html(0).map(|h| !h.contains("<b>")).unwrap_or(true));
        assert!(m.body_text(0).unwrap().contains("Hello there"));
        assert!(!String::from_utf8_lossy(&raw).contains("text/html"));
    }

    #[test]
    fn restamps_date() {
        let c = Compose { to: vec![Addr::new("", "a@b.example")], subject: "x".into(), html: "<p>x</p>".into(), ..Default::default() };
        let raw = build(&c, &env(), vec![]).unwrap();
        let later = chrono::DateTime::parse_from_rfc3339("2026-12-01T08:00:00+02:00").unwrap();
        let raw2 = restamp_date(&raw, &later);
        let m = MessageParser::default().parse(&raw2).unwrap();
        assert_eq!(m.date().unwrap().to_timestamp(), later.timestamp());
        assert_eq!(m.subject().unwrap(), "x");
    }

    #[test]
    fn splits_reference_ids() {
        assert_eq!(split_ids("<a@x> <b@y>\r\n <c@z>"), vec!["a@x", "b@y", "c@z"]);
    }
}

//! Fake Gmail: derive Gmail's JSON message representation (payload part tree, snippet, size) from
//! the stored RFC 822 text, and the message search used by `messages.list?q=`.
//!
//! Like real Gmail, `body.data` carries the part content with the *transfer* encoding removed but
//! in its *original charset*; header values of the top-level part come back RFC 2047-decoded.

use std::collections::HashMap;

use base64::{Engine, engine::general_purpose::URL_SAFE, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};

use crate::mail::mime::{decode_base64_lenient, decode_charset, decode_qp, decode_rfc2047, escape_html, header_params, strip_tags};

/// Text parts larger than this are served through `attachments.get` (like Gmail does for big bodies).
pub const INLINE_DATA_MAX: usize = 32 * 1024;

#[derive(Debug, Clone, Default)]
pub struct RawPart {
    /// (name, unfolded raw value)
    pub headers: Vec<(String, String)>,
    pub mime: String,
    pub params: HashMap<String, String>,
    /// raw (still transfer-encoded) body for leaves
    pub body: Vec<u8>,
    pub children: Vec<RawPart>,
}

impl RawPart {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Content with the transfer encoding removed (original charset).
    pub fn decoded(&self) -> Vec<u8> {
        match self.header("content-transfer-encoding").map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("base64") => decode_base64_lenient(&self.body),
            Some("quoted-printable") => decode_qp(&self.body),
            _ => self.body.clone(),
        }
    }

    pub fn filename(&self) -> String {
        if let Some(cd) = self.header("content-disposition") {
            let (_, p) = header_params(cd);
            if let Some(f) = p.get("filename") {
                return f.clone();
            }
        }
        self.params.get("name").cloned().unwrap_or_default()
    }

    pub fn is_attachment_disposition(&self) -> bool {
        self.header("content-disposition").map(|d| d.trim().to_ascii_lowercase().starts_with("attachment")).unwrap_or(false)
    }

    pub fn text(&self) -> String {
        decode_charset(&self.decoded(), self.params.get("charset").map(String::as_str))
    }
}

fn split_head_body(b: &[u8]) -> (&[u8], &[u8]) {
    let crlf = find(b, b"\r\n\r\n");
    let lf = find(b, b"\n\n");
    match (crlf, lf) {
        (Some(c), Some(l)) if l < c => (&b[..l], &b[l + 2..]),
        (Some(c), _) => (&b[..c], &b[c + 4..]),
        (None, Some(l)) => (&b[..l], &b[l + 2..]),
        (None, None) => {
            // header-only message
            if b.starts_with(b"\r\n") || b.starts_with(b"\n") { (&b[..0], b) } else { (b, &b[b.len()..]) }
        }
    }
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn parse_headers(head: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(head);
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = out.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
            continue;
        }
        if let Some((n, v)) = line.split_once(':') {
            out.push((n.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

/// Parse a MIME entity (recursively for multiparts).
pub fn parse(b: &[u8]) -> RawPart {
    parse_depth(b, 0)
}

fn parse_depth(b: &[u8], depth: usize) -> RawPart {
    let (head, body) = split_head_body(b);
    let headers = parse_headers(head);
    let ct = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "text/plain; charset=us-ascii".into());
    let (mime, params) = header_params(&ct);
    let mut part = RawPart { headers, mime: if mime.is_empty() { "text/plain".into() } else { mime }, params, ..Default::default() };
    if part.mime.starts_with("multipart/")
        && depth < 20
        && let Some(boundary) = part.params.get("boundary").cloned()
    {
        part.children = split_multipart(body, &boundary).into_iter().map(|p| parse_depth(p, depth + 1)).collect();
        return part;
    }
    part.body = body.to_vec();
    part
}

fn split_multipart<'a>(body: &'a [u8], boundary: &str) -> Vec<&'a [u8]> {
    let delim = format!("--{boundary}");
    let mut parts = Vec::new();
    let mut start: Option<usize> = None;
    let mut pos = 0;
    while pos <= body.len() {
        let end = body[pos..].iter().position(|c| *c == b'\n').map(|i| pos + i).unwrap_or(body.len());
        let line = &body[pos..end];
        let line_t = trim_ascii_end(line);
        if line_t.starts_with(delim.as_bytes()) {
            let rest = &line_t[delim.len()..];
            if rest.is_empty() || rest == b"--" {
                if let Some(s) = start {
                    // content ends before the CRLF preceding the delimiter line
                    let mut e = pos;
                    if e > s && body[e - 1] == b'\n' {
                        e -= 1;
                    }
                    if e > s && body[e - 1] == b'\r' {
                        e -= 1;
                    }
                    parts.push(&body[s..e.max(s)]);
                }
                if rest == b"--" {
                    return parts;
                }
                start = Some((end + 1).min(body.len()));
            }
        }
        pos = end + 1;
    }
    if let Some(s) = start
        && s < body.len()
    {
        parts.push(&body[s..]);
    }
    parts
}

fn trim_ascii_end(b: &[u8]) -> &[u8] {
    let mut e = b.len();
    while e > 0 && (b[e - 1] == b'\r' || b[e - 1] == b' ' || b[e - 1] == b'\t') {
        e -= 1;
    }
    &b[..e]
}

pub fn b64url(b: &[u8]) -> String {
    URL_SAFE.encode(b)
}

pub fn attachment_id(message_id: &str, part_id: &str) -> String {
    format!("ANGjdJ{}", URL_SAFE_NO_PAD.encode(format!("{message_id}|{part_id}|cm-fake-attachment")))
}

/// (message id, part id) from an attachment id we issued.
pub fn parse_attachment_id(aid: &str) -> Option<(String, String)> {
    let raw = URL_SAFE_NO_PAD.decode(aid.strip_prefix("ANGjdJ")?.trim_end_matches('=')).ok()?;
    let s = String::from_utf8(raw).ok()?;
    let mut it = s.split('|');
    Some((it.next()?.to_string(), it.next()?.to_string()))
}

/// Find a part by Gmail part id ("", "0", "1.0"...).
pub fn part_by_id<'a>(root: &'a RawPart, part_id: &str) -> Option<&'a RawPart> {
    if part_id.is_empty() {
        return Some(root);
    }
    let mut cur = root;
    for seg in part_id.split('.') {
        let i: usize = seg.parse().ok()?;
        cur = cur.children.get(i)?;
    }
    Some(cur)
}

/// Gmail `payload` JSON for a part tree. `headers_filter`: None = all headers.
pub fn payload_json(message_id: &str, p: &RawPart, part_id: &str, top: bool) -> Value {
    let headers: Vec<Value> =
        p.headers.iter().map(|(n, v)| json!({ "name": n, "value": if top { decode_rfc2047(v) } else { v.clone() } })).collect();
    let filename = p.filename();
    let mut obj = json!({
        "partId": part_id,
        "mimeType": p.mime,
        "filename": filename,
        "headers": headers,
    });
    if p.mime.starts_with("multipart/") {
        obj["body"] = json!({ "size": 0 });
        let parts: Vec<Value> = p
            .children
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let id = if part_id.is_empty() { i.to_string() } else { format!("{part_id}.{i}") };
                payload_json(message_id, c, &id, false)
            })
            .collect();
        obj["parts"] = Value::Array(parts);
    } else {
        let data = p.decoded();
        let inline = filename.is_empty() && p.mime.starts_with("text/") && data.len() <= INLINE_DATA_MAX && !p.is_attachment_disposition();
        obj["body"] = if inline {
            json!({ "size": data.len(), "data": b64url(&data) })
        } else {
            json!({ "size": data.len(), "attachmentId": attachment_id(message_id, part_id) })
        };
    }
    obj
}

/// Metadata payload: top-level headers only (optionally filtered), no parts.
pub fn metadata_payload_json(p: &RawPart, wanted: &[String]) -> Value {
    let headers: Vec<Value> = p
        .headers
        .iter()
        .filter(|(n, _)| wanted.is_empty() || wanted.iter().any(|w| w.eq_ignore_ascii_case(n)))
        .map(|(n, v)| json!({ "name": n, "value": decode_rfc2047(v) }))
        .collect();
    json!({ "partId": "", "mimeType": p.mime, "filename": "", "headers": headers, "body": { "size": 0 } })
}

/// Readable body text (first text/plain, else stripped first text/html).
pub fn body_text(p: &RawPart) -> String {
    fn walk<'a>(p: &'a RawPart, mime: &str) -> Option<&'a RawPart> {
        if p.children.is_empty() {
            return (p.mime == mime && p.filename().is_empty() && !p.is_attachment_disposition()).then_some(p);
        }
        p.children.iter().find_map(|c| walk(c, mime))
    }
    if let Some(t) = walk(p, "text/plain") {
        return t.text();
    }
    if let Some(h) = walk(p, "text/html") {
        return strip_tags(&h.text());
    }
    String::new()
}

/// Gmail-style snippet: ~200 chars, HTML-escaped.
pub fn snippet(p: &RawPart) -> String {
    let t = body_text(p);
    let flat: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
    escape_html(&flat.chars().take(200).collect::<String>())
}

/// Any real attachment (a part with a filename that isn't a text body)?
pub fn has_attachment(p: &RawPart) -> bool {
    if p.children.is_empty() {
        return !p.filename().is_empty();
    }
    p.children.iter().any(has_attachment)
}

pub fn attachment_names(p: &RawPart, out: &mut Vec<String>) {
    if p.children.is_empty() {
        let f = p.filename();
        if !f.is_empty() {
            out.push(f.to_lowercase());
        }
    }
    for c in &p.children {
        attachment_names(c, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_tree_numbering_and_bodies() {
        let raw = mail_builder::MessageBuilder::new()
            .from(("Sarah", "sarah@x.example"))
            .to("kenny@duvz.demo")
            .subject("שלום")
            .text_body("hello שלום")
            .html_body("<p>hello <b>שלום</b></p>")
            .attachment("application/pdf", "report.pdf", vec![1u8, 2, 3])
            .write_to_string()
            .unwrap();
        let root = parse(raw.as_bytes());
        assert_eq!(root.mime, "multipart/mixed");
        let j = payload_json("m1", &root, "", true);
        assert_eq!(j["partId"], "");
        assert_eq!(j["parts"][0]["mimeType"], "multipart/alternative");
        assert_eq!(j["parts"][0]["parts"][0]["partId"], "0.0");
        assert_eq!(j["parts"][0]["parts"][1]["partId"], "0.1");
        assert_eq!(j["parts"][1]["partId"], "1");
        assert_eq!(j["parts"][1]["filename"], "report.pdf");
        let aid = j["parts"][1]["body"]["attachmentId"].as_str().unwrap();
        assert_eq!(parse_attachment_id(aid), Some(("m1".into(), "1".into())));
        let subj = j["headers"].as_array().unwrap().iter().find(|h| h["name"] == "Subject").unwrap();
        assert_eq!(subj["value"], "שלום");
        let data = j["parts"][0]["parts"][0]["body"]["data"].as_str().unwrap();
        let bytes = crate::google::gmail::b64url_decode(data).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap().trim(), "hello שלום");
        assert!(has_attachment(&root));
        assert_eq!(snippet(&root), "hello שלום");
    }
}

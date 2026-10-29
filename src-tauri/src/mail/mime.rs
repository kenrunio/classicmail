//! Small MIME helpers shared by the mail client code and the fake Gmail server: RFC 2047 header
//! decoding, charsets, quoted-printable, address lists, header parameters (RFC 2231), dates,
//! subject normalisation and text ⇄ HTML conversion.

use std::collections::HashMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Addr {
    pub name: String,
    pub email: String,
}

impl Addr {
    /// Name if any, else the address.
    pub fn display(&self) -> &str {
        if self.name.trim().is_empty() { &self.email } else { &self.name }
    }
}

/// Decode bytes in `charset` (any WHATWG label; unknown/missing → UTF-8 with a Windows-1252 fallback).
pub fn decode_charset(bytes: &[u8], charset: Option<&str>) -> String {
    let label = charset.map(|c| c.trim().trim_matches('"').to_ascii_lowercase()).unwrap_or_default();
    // "iso-8859-8" is visual-order Hebrew in theory, but mailers use it for logical text: decode the same.
    let label = match label.as_str() {
        "" | "us-ascii" | "ascii" | "unknown-8bit" | "x-unknown" => "",
        "cp1255" => "windows-1255",
        "cp1252" => "windows-1252",
        "iso-8859-8-i" | "iso-8859-8-e" | "hebrew" => "iso-8859-8",
        other => other,
    };
    if label.is_empty() {
        return match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
        };
    }
    match encoding_rs::Encoding::for_label(label.as_bytes()) {
        // Labelled with a legacy charset but valid UTF-8 with non-ASCII in it: it is UTF-8 (Gmail
        // hands over Outlook's "windows-1255" HTML as UTF-8, meta tag and all). Genuine
        // single-byte text practically never forms valid multi-byte UTF-8 sequences.
        Some(enc)
            if enc != encoding_rs::UTF_8
                && enc != encoding_rs::UTF_16LE
                && enc != encoding_rs::UTF_16BE
                && !bytes.is_ascii()
                && std::str::from_utf8(bytes).is_ok() =>
        {
            String::from_utf8_lossy(bytes).into_owned()
        }
        Some(enc) => {
            let (text, _, had_errors) = enc.decode(bytes);
            if had_errors && enc == encoding_rs::UTF_8 {
                // mislabelled legacy text
                if let Some(s) = try_legacy(bytes) {
                    return s;
                }
            }
            text.into_owned()
        }
        None => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn try_legacy(bytes: &[u8]) -> Option<String> {
    // Hebrew letters live in 0xE0..=0xFA in both windows-1255 and iso-8859-8
    let hebrew = bytes.iter().filter(|b| (0xE0..=0xFA).contains(*b)).count();
    let high = bytes.iter().filter(|b| **b >= 0x80).count();
    if high > 0 && hebrew * 10 >= high * 7 {
        return Some(encoding_rs::WINDOWS_1255.decode(bytes).0.into_owned());
    }
    None
}

/// Quoted-printable body decoding (soft line breaks, =XX).
pub fn decode_qp(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        if b == b'=' {
            // soft line break
            if input.get(i + 1) == Some(&b'\r') && input.get(i + 2) == Some(&b'\n') {
                i += 3;
                continue;
            }
            if input.get(i + 1) == Some(&b'\n') {
                i += 2;
                continue;
            }
            if let (Some(h), Some(l)) = (input.get(i + 1).and_then(|c| hex(*c)), input.get(i + 2).and_then(|c| hex(*c))) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
            out.push(b);
            i += 1;
        } else {
            out.push(b);
            i += 1;
        }
    }
    out
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Lenient base64 (ignores whitespace and junk; tolerates missing padding).
pub fn decode_base64_lenient(input: &[u8]) -> Vec<u8> {
    let clean: Vec<u8> = input.iter().copied().filter(|c| c.is_ascii_alphanumeric() || *c == b'+' || *c == b'/').collect();
    let mut padded = clean;
    while !padded.len().is_multiple_of(4) {
        padded.push(b'=');
    }
    STANDARD.decode(&padded).unwrap_or_default()
}

/// Decode RFC 2047 encoded words (`=?charset?B|Q?...?=`) in a header value. Whitespace between
/// adjacent encoded words is dropped, as the RFC says.
pub fn decode_rfc2047(s: &str) -> String {
    if !s.contains("=?") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut last_was_word = false;
    while !rest.is_empty() {
        if let Some(start) = rest.find("=?") {
            let (before, after) = rest.split_at(start);
            if let Some((decoded, consumed)) = parse_encoded_word(after) {
                let only_ws = before.chars().all(char::is_whitespace);
                if !(last_was_word && only_ws) {
                    out.push_str(before);
                }
                out.push_str(&decoded);
                rest = &after[consumed..];
                last_was_word = true;
                continue;
            }
            out.push_str(before);
            out.push_str("=?");
            rest = &after[2..];
            last_was_word = false;
        } else {
            out.push_str(rest);
            break;
        }
    }
    out
}

/// Parses one encoded word at the start of `s`; returns (decoded, bytes consumed).
fn parse_encoded_word(s: &str) -> Option<(String, usize)> {
    let body = s.strip_prefix("=?")?;
    let q1 = body.find('?')?;
    let charset = &body[..q1];
    let after_cs = &body[q1 + 1..];
    let q2 = after_cs.find('?')?;
    let enc = &after_cs[..q2];
    let after_enc = &after_cs[q2 + 1..];
    let end = after_enc.find("?=")?;
    let text = &after_enc[..end];
    if text.contains(' ') || charset.is_empty() {
        return None;
    }
    // RFC 2231 language suffix: charset*lang
    let charset = charset.split('*').next().unwrap_or(charset);
    let bytes = match enc.to_ascii_uppercase().as_str() {
        "B" => decode_base64_lenient(text.as_bytes()),
        "Q" => decode_qp(text.replace('_', " ").as_bytes()),
        _ => return None,
    };
    let consumed = 2 + q1 + 1 + q2 + 1 + end + 2;
    Some((decode_charset(&bytes, Some(charset)), consumed))
}

/// Split a structured header value into its main value and lower-cased parameters, handling quoted
/// strings and RFC 2231 (`filename*=utf-8''...`, continuations `name*0*=`).
pub fn header_params(value: &str) -> (String, HashMap<String, String>) {
    let parts = split_unquoted(value, ';');
    let main = parts.first().map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
    let mut params: HashMap<String, String> = HashMap::new();
    let mut continuations: HashMap<String, Vec<(u32, bool, String)>> = HashMap::new();
    for p in parts.iter().skip(1) {
        let Some((k, v)) = p.split_once('=') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim();
        let v = if v.starts_with('"') && v.ends_with('"') && v.len() >= 2 { unquote(&v[1..v.len() - 1]) } else { v.to_string() };
        if let Some((base, idx)) = k.split_once('*') {
            // name* or name*N or name*N*
            let extended = idx.ends_with('*') || idx.is_empty();
            let n: u32 = idx.trim_end_matches('*').parse().unwrap_or(0);
            continuations.entry(base.to_string()).or_default().push((n, extended, v));
        } else {
            params.insert(k, v);
        }
    }
    for (base, mut segs) in continuations {
        segs.sort_by_key(|s| s.0);
        let mut charset: Option<String> = None;
        let mut bytes = Vec::new();
        for (i, (_, extended, v)) in segs.iter().enumerate() {
            if *extended {
                let mut val = v.as_str();
                if i == 0 {
                    let mut it = v.splitn(3, '\'');
                    if let (Some(cs), Some(_lang), Some(rest)) = (it.next(), it.next(), it.next()) {
                        charset = Some(cs.to_string());
                        val = rest;
                    }
                }
                bytes.extend(percent_decode(val));
            } else {
                bytes.extend_from_slice(v.as_bytes());
            }
        }
        params.insert(base, decode_charset(&bytes, charset.as_deref().filter(|c| !c.is_empty())));
    }
    // encoded words inside quoted params (common for filenames from Outlook / Gmail)
    for v in params.values_mut() {
        if v.contains("=?") {
            *v = decode_rfc2047(v);
        }
    }
    (main, params)
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (b.get(i + 1).and_then(|c| hex(*c)), b.get(i + 2).and_then(|c| hex(*c)))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn unquote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut esc = false;
    for c in s.chars() {
        if esc {
            out.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// Split on `sep` outside double quotes / angle brackets / parentheses.
fn split_unquoted(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut esc = false;
    let mut angle = 0i32;
    let mut paren = 0i32;
    for c in s.chars() {
        if esc {
            cur.push(c);
            esc = false;
            continue;
        }
        match c {
            '\\' if in_q => {
                cur.push(c);
                esc = true;
            }
            '"' => {
                in_q = !in_q;
                cur.push(c);
            }
            '<' if !in_q => {
                angle += 1;
                cur.push(c);
            }
            '>' if !in_q => {
                angle = (angle - 1).max(0);
                cur.push(c);
            }
            '(' if !in_q => {
                paren += 1;
                cur.push(c);
            }
            ')' if !in_q => {
                paren = (paren - 1).max(0);
                cur.push(c);
            }
            c if c == sep && !in_q && angle == 0 && paren == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Parse an address-list header ("A <a@x>, \"B, C\" <b@y>, c@z, group: d@w;").
pub fn parse_addresses(value: &str) -> Vec<Addr> {
    let value = value.replace(['\r', '\n'], " ");
    let mut out = Vec::new();
    // Gmail hands out header values with RFC 2047 words already decoded, so a display name like
    // "Cohen, Dana" can arrive unquoted: `Cohen, Dana <dana@x.com>`. A piece with neither '@' nor
    // '<' is not an address on its own; glue it to the next piece.
    let mut items: Vec<String> = Vec::new();
    let mut carry = String::new();
    for item in split_unquoted(&value, ',') {
        let t = item.trim();
        if t.is_empty() {
            continue;
        }
        if !t.contains('@') && !t.contains('<') && !t.contains(':') && !t.contains(';') {
            if !carry.is_empty() {
                carry.push_str(", ");
            }
            carry.push_str(t);
            continue;
        }
        if carry.is_empty() {
            items.push(t.to_string());
        } else {
            items.push(format!("{}, {t}", std::mem::take(&mut carry)));
        }
    }
    if !carry.is_empty() {
        items.push(carry);
    }
    for item in items {
        let mut item = item.trim().to_string();
        if item.is_empty() {
            continue;
        }
        // group syntax "name: a, b;" — split_unquoted already split on commas; strip "name:" and ";"
        if !item.contains('<')
            && let Some(i) = item.find(':')
            && !item[..i].contains('@')
            && !item[..i].contains('"')
        {
            item = item[i + 1..].to_string();
        }
        let item = item.trim().trim_end_matches(';').trim();
        if item.is_empty() {
            continue;
        }
        let (name, email) = if let (Some(lt), Some(gt)) = (item.rfind('<'), item.rfind('>')) {
            if lt < gt { (item[..lt].trim().to_string(), item[lt + 1..gt].trim().to_string()) } else { (String::new(), item.to_string()) }
        } else {
            // "a@x (Name)" old style
            match (item.find('('), item.rfind(')')) {
                (Some(o), Some(c)) if o < c => (item[o + 1..c].trim().to_string(), item[..o].trim().to_string()),
                _ => (String::new(), item.to_string()),
            }
        };
        let name = name.trim();
        let name = if name.starts_with('"') && name.ends_with('"') && name.len() >= 2 {
            unquote(&name[1..name.len() - 1])
        } else {
            name.to_string()
        };
        let name = decode_rfc2047(&name).trim().to_string();
        let email = email.trim_matches(|c: char| c == '"' || c.is_whitespace()).to_string();
        if email.is_empty() && name.is_empty() {
            continue;
        }
        let name = if name.eq_ignore_ascii_case(&email) { String::new() } else { name.trim_matches('\'').to_string() };
        out.push(Addr { name: strip_bidi_overrides(&name), email: strip_bidi_overrides(&email) });
    }
    out
}

/// Remove bidi embedding / override / isolate controls (U+202A..U+202E, U+2066..U+2069) from text
/// the sender controls: `"Investor Relations <U+202E>moc.kab"` would display as "...bak.com".
/// (LRM/RLM marks stay: Hebrew names use them legitimately.)
pub fn strip_bidi_overrides(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')).collect()
}

/// RFC 2822 date header → epoch ms (lenient: trailing comments, missing weekday, 2-digit years).
pub fn parse_date(value: &str) -> Option<i64> {
    let mut v = value.trim().to_string();
    if let Some(i) = v.find('(') {
        v.truncate(i);
    }
    let v = v.trim();
    if let Ok(d) = chrono::DateTime::parse_from_rfc2822(v) {
        return Some(d.timestamp_millis());
    }
    for fmt in ["%a, %d %b %Y %H:%M:%S %z", "%d %b %Y %H:%M:%S %z", "%a, %d %b %Y %H:%M %z", "%a, %d %b %Y %H:%M:%S %Z"] {
        if let Ok(d) = chrono::DateTime::parse_from_str(v, fmt) {
            return Some(d.timestamp_millis());
        }
    }
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(v) {
        return Some(d.timestamp_millis());
    }
    None
}

/// 0 low, 1 normal, 2 high — from Importance / X-Priority / Priority / X-MSMail-Priority.
pub fn importance(importance: Option<&str>, x_priority: Option<&str>, priority: Option<&str>) -> i64 {
    if let Some(i) = importance {
        match i.trim().to_ascii_lowercase().as_str() {
            "high" => return 2,
            "low" => return 0,
            _ => {}
        }
    }
    if let Some(p) = x_priority {
        match p.trim().chars().next() {
            Some('1') | Some('2') => return 2,
            Some('4') | Some('5') => return 0,
            _ => {}
        }
        let l = p.to_ascii_lowercase();
        if l.contains("high") {
            return 2;
        }
        if l.contains("low") {
            return 0;
        }
    }
    if let Some(p) = priority {
        match p.trim().to_ascii_lowercase().as_str() {
            "urgent" => return 2,
            "non-urgent" => return 0,
            _ => {}
        }
    }
    1
}

/// Subject without reply/forward prefixes ("RE: FW: Re[2]: תשובה: x" → "x").
pub fn normalize_subject(s: &str) -> String {
    let mut t = s.trim();
    loop {
        // ASCII-only lowercasing keeps byte offsets identical to `t` (full Unicode lowercasing may not)
        let lower = t.to_ascii_lowercase();
        let mut stripped = false;
        for p in ["re:", "fw:", "fwd:", "aw:", "wg:", "sv:", "vs:", "tr:", "rif:", "השב:", "תשובה:", "הועבר:", "re :", "fw :"]
        {
            if lower.starts_with(p) {
                t = t[p.len()..].trim_start();
                stripped = true;
                break;
            }
        }
        if !stripped {
            // Re[2]: / RE(2):
            if (lower.starts_with("re[") || lower.starts_with("re("))
                && let Some(i) = t.find(':')
                && i < 8
            {
                t = t[i + 1..].trim_start();
                continue;
            }
            break;
        }
    }
    t.to_string()
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Gmail snippets are HTML-escaped ("&#39;", "&amp;"); undo the common entities.
pub fn unescape_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        // look for ';' within ~10 bytes, cut on a char boundary (Hebrew letters are 2 bytes)
        let lim = (0..=tail.len().min(10)).rev().find(|&i| tail.is_char_boundary(i)).unwrap_or(0);
        if let Some(semi) = tail[..lim].find(';') {
            let ent = &tail[1..semi];
            let rep = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
                _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            if let Some(c) = rep {
                out.push(c);
                rest = &tail[semi + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

/// Plain text → HTML (escaped, links clickable, line breaks kept, each paragraph `dir=auto` so
/// Hebrew paragraphs flow right-to-left).
pub fn text_to_html(text: &str) -> String {
    let url_re = url_regex();
    let mut out = String::with_capacity(text.len() * 2);
    out.push_str("<div class=\"cm-plain\">");
    for para in text.replace("\r\n", "\n").split("\n\n") {
        out.push_str("<p dir=\"auto\">");
        let mut first = true;
        for line in para.split('\n') {
            if !first {
                out.push_str("<br>");
            }
            first = false;
            let mut last = 0;
            for m in url_re.find_iter(line) {
                out.push_str(&escape_html(&line[last..m.start()]));
                let url = m.as_str().trim_end_matches(['.', ',', ')', ';', ':', '!', '?']);
                let href = if url.starts_with("www.") { format!("http://{url}") } else { url.to_string() };
                out.push_str(&format!("<a href=\"{}\">{}</a>", escape_html(&href), escape_html(url)));
                out.push_str(&escape_html(&m.as_str()[url.len()..]));
                last = m.end();
            }
            out.push_str(&escape_html(&line[last..]));
        }
        out.push_str("</p>");
    }
    out.push_str("</div>");
    out
}

fn url_regex() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r#"(?i)\b(?:https?://|mailto:|www\.)[^\s<>"]+"#).unwrap())
}

/// HTML → readable plain text (for search, quoting and the plain-text fallback).
pub fn html_to_text(html: &str) -> String {
    if html_too_deep(html) {
        return strip_tags(html);
    }
    let text = html2text::config::plain().string_from_read(html.as_bytes(), 100).unwrap_or_else(|_| strip_tags(html));
    // html2text marks links as [text][n] with footnotes; keep it simple and readable
    text.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Crude tag stripper (fallback; also used for snippets).
pub fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let lower = html.to_ascii_lowercase();
    let b = html.as_bytes();
    let mut i = 0;
    // once a closing tag isn't found it never will be: don't search again (keeps this linear)
    let mut unclosed = [false; 4];
    while i < b.len() {
        if b[i] == b'<' {
            // skip <style>/<script>/<head> blocks entirely
            for (k, tag) in ["style", "script", "head", "title"].into_iter().enumerate() {
                if !unclosed[k] && lower[i + 1..].starts_with(tag) {
                    match lower[i..].find(&format!("</{tag}")) {
                        Some(end) => {
                            i += end;
                            break;
                        }
                        None => unclosed[k] = true,
                    }
                }
            }
            match html[i..].find('>') {
                Some(e) => {
                    let tag = &lower[i..i + e];
                    if tag.starts_with("<br")
                        || tag.starts_with("<p")
                        || tag.starts_with("</p")
                        || tag.starts_with("<div")
                        || tag.starts_with("</tr")
                        || tag.starts_with("</h")
                    {
                        out.push(' ');
                    }
                    i += e + 1;
                }
                None => break,
            }
        } else {
            let next = html[i..].find('<').map(|n| i + n).unwrap_or(b.len());
            out.push_str(&html[i..next]);
            i = next;
        }
    }
    let out = unescape_entities(&out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Elements nested deeper than this make an HTML body "hostile": html5ever / ammonia / html2text
/// get quadratic (50,000 nested `<div>`s take ~5 s to sanitise and ~25 s to convert to text), so
/// such bodies are shown as plain text instead. Real mail stays far below (browsers flatten at 512).
pub const MAX_HTML_DEPTH: usize = 1000;

/// Cheap estimate of the element nesting depth of `html` (a stack of open elements; void elements,
/// self-closing tags and elements whose end tag is optional don't count) — true when it exceeds
/// `MAX_HTML_DEPTH`.
pub fn html_too_deep(html: &str) -> bool {
    const VOID_OR_OPTIONAL: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr", "p", "li", "dt",
        "dd", "tr", "td", "th", "option", "optgroup", "thead", "tbody", "tfoot", "colgroup", "caption", "rp", "rt", "o:p", "html", "head",
        "body",
    ];
    let b = html.as_bytes();
    // open elements (a stray end tag closes nothing, like in the HTML parser)
    let mut stack: Vec<String> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let closing = b.get(i + 1) == Some(&b'/');
        let start = if closing { i + 2 } else { i + 1 };
        let mut j = start;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b':' || b[j] == b'-') {
            j += 1;
        }
        if j == start {
            i += 1;
            continue;
        }
        let name = html[start..j].to_ascii_lowercase();
        let end = html[j..].find('>').map(|e| j + e).unwrap_or(b.len());
        if closing {
            if let Some(pos) = stack.iter().rposition(|n| *n == name) {
                stack.truncate(pos);
            }
        } else if !VOID_OR_OPTIONAL.contains(&name.as_str()) && !html[j..end].ends_with('/') {
            stack.push(name);
            if stack.len() > MAX_HTML_DEPTH {
                return true;
            }
        }
        i = end;
    }
    false
}

/// First ~200 characters of readable text (Gmail-style snippet, not escaped).
pub fn make_snippet(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(max).collect()
}

/// Bytes → human size like Outlook ("12 KB", "1.2 MB").
pub fn human_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", (bytes + 1023) / 1024)
    } else {
        format!("{:.1} MB", bytes as f64 / 1048576.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc2047_b_and_q_and_adjacent_words() {
        assert_eq!(decode_rfc2047("=?UTF-8?B?16nXnNeV150=?="), "שלום");
        assert_eq!(decode_rfc2047("=?iso-8859-1?Q?Caf=E9_cr=E8me?="), "Café crème");
        assert_eq!(decode_rfc2047("=?UTF-8?Q?a?= =?UTF-8?Q?b?="), "ab");
        assert_eq!(decode_rfc2047("Re: =?UTF-8?Q?x?= tail"), "Re: x tail");
        assert_eq!(decode_rfc2047("plain"), "plain");
        assert_eq!(decode_rfc2047("broken =?x"), "broken =?x");
    }

    #[test]
    fn hebrew_legacy_charsets() {
        let (bytes, _, _) = encoding_rs::WINDOWS_1255.encode("דוח רבעוני");
        assert_eq!(decode_charset(&bytes, Some("windows-1255")), "דוח רבעוני");
        assert_eq!(decode_charset(&bytes, Some("iso-8859-8")), "דוח רבעוני");
        assert_eq!(decode_charset(&bytes, Some("ISO-8859-8-I")), "דוח רבעוני");
        // mislabelled as utf-8
        assert_eq!(decode_charset(&bytes, Some("utf-8")), "דוח רבעוני");
        let enc = format!("=?windows-1255?B?{}?=", STANDARD.encode(&bytes));
        assert_eq!(decode_rfc2047(&enc), "דוח רבעוני");
    }

    #[test]
    fn utf8_labelled_as_a_legacy_charset() {
        // Outlook's HTML says windows-1255 but the bytes Gmail hands over are UTF-8
        let s = "היי קני ודנה,\u{200e}Harbor tower\u{200e} – 2,500 ש\"ח";
        for label in ["windows-1255", "iso-8859-8", "windows-1252", "iso-8859-1"] {
            assert_eq!(decode_charset(s.as_bytes(), Some(label)), s, "{label}");
        }
        // genuine single-byte text is still decoded with its label
        let (b, _, _) = encoding_rs::WINDOWS_1252.encode("café – naïve");
        assert_eq!(decode_charset(&b, Some("windows-1252")), "café – naïve");
        let (b, _, _) = encoding_rs::WINDOWS_1255.encode("שלום ש\"ח");
        assert_eq!(decode_charset(&b, Some("windows-1255")), "שלום ש\"ח");
    }

    #[test]
    fn qp_decoding() {
        assert_eq!(decode_qp(b"a=3Db=\r\nc"), b"a=bc");
        assert_eq!(decode_charset(&decode_qp(b"=D7=A9=D7=9C"), Some("utf-8")), "של");
    }

    #[test]
    fn params_with_rfc2231() {
        let (m, p) = header_params("attachment; filename*=UTF-8''%D7%93%D7%95%D7%97.pdf");
        assert_eq!(m, "attachment");
        assert_eq!(p["filename"], "דוח.pdf");
        let (m, p) = header_params("text/plain; charset=\"windows-1255\"; format=flowed");
        assert_eq!(m, "text/plain");
        assert_eq!(p["charset"], "windows-1255");
        let (_, p) = header_params("application/pdf; name*0=\"long\"; name*1=\"name.pdf\"");
        assert_eq!(p["name"], "longname.pdf");
    }

    #[test]
    fn address_lists() {
        let a = parse_addresses("\"Cohen, Sarah\" <sarah@x.com>, bob@y.com, =?UTF-8?B?16nXnNeV150=?= <s@z.com>, team: a@b.c, d@e.f;");
        assert_eq!(a.len(), 5);
        assert_eq!(a[0], Addr { name: "Cohen, Sarah".into(), email: "sarah@x.com".into() });
        assert_eq!(a[1].email, "bob@y.com");
        assert_eq!(a[2].name, "שלום");
        assert_eq!(a[3].email, "a@b.c");
        assert_eq!(a[4].email, "d@e.f");
        // Gmail decodes encoded words itself, so a comma in a display name arrives unquoted
        let a = parse_addresses("Cohen, Dana <dana@x.com>, כהן, דנה <d2@x.com>, bob@y.com");
        assert_eq!(a.len(), 3, "{a:?}");
        assert_eq!(a[0], Addr { name: "Cohen, Dana".into(), email: "dana@x.com".into() });
        assert_eq!(a[1], Addr { name: "כהן, דנה".into(), email: "d2@x.com".into() });
        assert_eq!(a[2].email, "bob@y.com");
    }

    #[test]
    fn subjects_and_dates() {
        assert_eq!(normalize_subject("RE: Fw: Re[2]: Q3 results"), "Q3 results");
        assert_eq!(normalize_subject("תשובה: דוח"), "דוח");
        assert_eq!(parse_date("Tue, 1 Jul 2003 10:52:37 +0200 (IST)"), Some(1057049557000));
        assert_eq!(importance(None, Some("1 (Highest)"), None), 2);
        assert_eq!(importance(Some("low"), None, None), 0);
    }

    #[test]
    fn text_html_round_trip() {
        let h = text_to_html("Hi <b>\nsee https://example.com/x.\n\nבוקר טוב");
        assert!(h.contains("&lt;b&gt;"));
        assert!(h.contains("<a href=\"https://example.com/x\">https://example.com/x</a>."));
        assert!(h.contains("dir=\"auto\""));
        assert_eq!(unescape_entities("it&#39;s &amp; &lt;b&gt;"), "it's & <b>");
        assert_eq!(strip_tags("<style>x{}</style><p>Hello&nbsp;<b>you</b></p>"), "Hello you");
    }
}

#[cfg(test)]
mod hebrew_slicing_tests {
    use super::*;

    /// Real-account crash (2026-10-11): an '&' followed by Hebrew within 10 bytes panicked.
    #[test]
    fn unescape_entities_with_hebrew_after_ampersand() {
        assert_eq!(unescape_entities("שלום & שלום"), "שלום & שלום");
        assert_eq!(unescape_entities("a&ששששש;b"), "a&ששששש;b");
        assert_eq!(unescape_entities("x &amp; שלום &#39;ok&#39;"), "x & שלום 'ok'");
        assert_eq!(unescape_entities("&"), "&");
    }

    #[test]
    fn normalize_subject_hebrew_and_unicode() {
        assert_eq!(normalize_subject("RE: תשובה: שלום"), "שלום");
        assert_eq!(normalize_subject("İstanbul trip"), "İstanbul trip");
        assert_eq!(normalize_subject("השב: Re[2]: x"), "x");
    }
}

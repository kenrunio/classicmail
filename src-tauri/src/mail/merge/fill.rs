//! `{keyword}` placeholders: finding them, matching them to list columns, filling them in, and
//! the pre-flight check run before a merge is sent or saved as drafts.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::Serialize;

use super::list::MergeList;
use crate::mail::compose::model::{Addr, Compose, parse_address_list, valid_email};

/// Google Workspace's daily sending limit (recipients per account per day).
pub const DAILY_LIMIT: usize = 2_000;
const MAX_KEY: usize = 60;

/// `First Name`, `first_name`, `FIRSTNAME` → `firstname`.
pub fn key_norm(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, ' ' | '_' | '-' | '\u{a0}')).flat_map(char::to_lowercase).collect()
}

/// A placeholder found in a text: byte range of `{…}` and the key between the braces (tags
/// removed, a few entities decoded). `tags` are the HTML tags that sat inside the braces; they
/// are put back after the value so the markup stays balanced.
#[derive(Debug, PartialEq)]
struct Found {
    start: usize,
    end: usize,
    key: String,
    tags: String,
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Find placeholders. With `html`, `<…>` tags are skipped (outside: a `{` inside a tag is not a
/// placeholder; inside braces: kept aside in `tags`).
fn find(text: &str, html: bool) -> Vec<Found> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'<' if html => {
                i = text[i..].find('>').map(|p| i + p + 1).unwrap_or(b.len());
            }
            b'{' => {
                let mut j = i + 1;
                let mut key = String::new();
                let mut tags = String::new();
                let mut ok = false;
                while j < b.len() {
                    match b[j] {
                        b'}' => {
                            ok = true;
                            break;
                        }
                        b'{' | b'\n' | b'\r' => break,
                        b'<' if html => {
                            let e = text[j..].find('>').map(|p| j + p + 1).unwrap_or(b.len());
                            tags.push_str(&text[j..e]);
                            j = e;
                            continue;
                        }
                        _ => {
                            let ch = text[j..].chars().next().unwrap();
                            key.push(ch);
                            j += ch.len_utf8();
                        }
                    }
                    if key.chars().count() > MAX_KEY * 2 {
                        break;
                    }
                }
                let key = if html { decode_entities(&key) } else { key };
                let key = key.trim().to_string();
                if ok && !key.is_empty() && key.chars().count() <= MAX_KEY {
                    out.push(Found { start: i, end: j + 1, key, tags });
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    out
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Column index by normalised title.
pub fn columns(list: &MergeList) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for (i, h) in list.headers.iter().enumerate() {
        let k = key_norm(h);
        if !k.is_empty() {
            m.entry(k).or_insert(i);
        }
    }
    m
}

/// Replace every known placeholder with the row's value; unknown ones are left as they are.
fn fill(text: &str, html: bool, cols: &HashMap<String, usize>, row: &[String]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for f in find(text, html) {
        let Some(&c) = cols.get(&key_norm(&f.key)) else { continue };
        out.push_str(&text[at..f.start]);
        let v = row.get(c).map(String::as_str).unwrap_or("");
        if html {
            out.push_str(&escape_html(v));
            out.push_str(&f.tags);
        } else {
            out.push_str(v);
        }
        at = f.end;
    }
    out.push_str(&text[at..]);
    out
}

/// The keys used in the subject and body (as written, first spelling wins), in order of appearance.
fn used_keys(c: &Compose) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut add = |k: String| {
        if seen.insert(key_norm(&k)) {
            out.push(k);
        }
    };
    find(&c.subject, false).into_iter().for_each(|f| add(f.key));
    find(&c.html, true).into_iter().for_each(|f| add(f.key));
    if let Some(t) = &c.text {
        find(t, false).into_iter().for_each(|f| add(f.key));
    }
    out
}

/// The address in a row's e-mail cell, if it holds exactly one valid one.
pub fn row_address(cell: &str) -> Option<Addr> {
    let v = parse_address_list(cell);
    match v.as_slice() {
        [a] if valid_email(&a.email) => Some(a.clone()),
        _ => None,
    }
}

/// One personalised message for `row`.
pub fn message_for(template: &Compose, list: &MergeList, cols: &HashMap<String, usize>, row: usize, to: Addr) -> Compose {
    let r = &list.rows[row];
    let mut c = template.clone();
    c.to = vec![to];
    c.cc.clear();
    c.bcc.clear();
    c.draft_id = None;
    c.subject = fill(&template.subject, false, cols, r);
    c.html = fill(&template.html, true, cols, r);
    c.text = template.text.as_ref().map(|t| fill(t, false, cols, r));
    c
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RowIssue {
    /// row number in the file
    pub row: usize,
    pub email: String,
    /// the blank fields (blank-field issues only)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    pub messages: usize,
    /// blank or invalid e-mail
    pub skipped: Vec<RowIssue>,
    /// same address as an earlier row
    pub duplicates: Vec<RowIssue>,
    /// `{x}` matching no column (blocking)
    pub unknown_fields: Vec<String>,
    /// rows where a used field is empty
    pub blank_fields: Vec<RowIssue>,
    pub over_limit: bool,
}

/// The rows that get a message (first occurrence of each valid address) and the check result.
pub fn plan(list: &MergeList, template: &Compose) -> (Vec<(usize, Addr)>, Preflight) {
    let cols = columns(list);
    let mut p = Preflight::default();
    let keys = used_keys(template);
    let mut unknown = BTreeSet::new();
    let mut used_cols = Vec::new();
    for k in &keys {
        match cols.get(&key_norm(k)) {
            Some(&c) => used_cols.push(c),
            None => {
                unknown.insert(format!("{{{k}}}"));
            }
        }
    }
    p.unknown_fields = unknown.into_iter().collect();
    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    for (i, r) in list.rows.iter().enumerate() {
        let row = list.row_numbers[i];
        let cell = r[list.email_column].trim();
        let Some(addr) = row_address(cell) else {
            p.skipped.push(RowIssue { row, email: cell.to_string(), fields: vec![] });
            continue;
        };
        if !seen.insert(addr.email.to_lowercase()) {
            p.duplicates.push(RowIssue { row, email: addr.email, fields: vec![] });
            continue;
        }
        let blank: Vec<String> = used_cols.iter().filter(|&&c| r[c].trim().is_empty()).map(|&c| list.headers[c].clone()).collect();
        if !blank.is_empty() {
            p.blank_fields.push(RowIssue { row, email: addr.email.clone(), fields: blank });
        }
        targets.push((i, addr));
    }
    p.messages = targets.len();
    p.over_limit = targets.len() > DAILY_LIMIT;
    (targets, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(headers: &[&str], rows: &[&[&str]]) -> MergeList {
        let headers: Vec<String> = headers.iter().map(|s| s.to_string()).collect();
        let email_column = headers.iter().position(|h| super::super::list::is_email_header(h)).unwrap();
        MergeList {
            file_name: "t.csv".into(),
            email_column,
            rows: rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect(),
            row_numbers: (2..rows.len() + 2).collect(),
            headers,
        }
    }

    fn compose(subject: &str, html: &str) -> Compose {
        Compose { subject: subject.into(), html: html.into(), ..Default::default() }
    }

    #[test]
    fn key_normalisation() {
        assert_eq!(key_norm("First Name"), "firstname");
        assert_eq!(key_norm("first_name"), "firstname");
        assert_eq!(key_norm("FIRST-NAME"), "firstname");
        assert_eq!(key_norm("שם פרטי"), "שםפרטי");
    }

    #[test]
    fn finds_placeholders_in_text_and_html() {
        let f = find("Hi {First Name}, {} {a\nb} { x }", false);
        assert_eq!(f.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(), vec!["First Name", "x"]);
        // braces inside a tag are not placeholders; tags inside braces are kept aside
        let f = find(r#"<p style="a{b}">Dear <b>{first</b>name}</p>"#, true);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].key, "firstname");
        assert_eq!(f[0].tags, "</b>");
        let f = find("<p>{First&nbsp;Name}</p>", true);
        assert_eq!(f[0].key, "First Name");
    }

    #[test]
    fn fills_subject_and_body() {
        let l = list(&["First Name", "Email", "Company"], &[&["Dan & Co", "dan@x.com", "<Acme>"]]);
        let cols = columns(&l);
        let t = compose("Meeting with {company}", "<p>Dear <b>{first</b>name}, re {COMPANY} {unknown}</p>");
        let m = message_for(&t, &l, &cols, 0, Addr::new("", "dan@x.com"));
        assert_eq!(m.subject, "Meeting with <Acme>");
        assert_eq!(m.html, "<p>Dear <b>Dan &amp; Co</b>, re &lt;Acme&gt; {unknown}</p>");
        assert_eq!(m.to, vec![Addr::new("", "dan@x.com")]);
    }

    #[test]
    fn plain_text_body_is_not_escaped() {
        let l = list(&["Email", "Name"], &[&["a@b.com", "A & B"]]);
        let mut t = compose("x", "<div>Hi {name}</div>");
        t.text = Some("Hi {name}".into());
        t.plain_text = true;
        let m = message_for(&t, &l, &columns(&l), 0, Addr::new("", "a@b.com"));
        assert_eq!(m.text.as_deref(), Some("Hi A & B"));
        assert_eq!(m.html, "<div>Hi A &amp; B</div>");
    }

    #[test]
    fn email_cell_forms() {
        assert_eq!(row_address(" dan@x.com ").unwrap().email, "dan@x.com");
        let a = row_address("Dan Cohen <dan@x.com>").unwrap();
        assert_eq!((a.name.as_str(), a.email.as_str()), ("Dan Cohen", "dan@x.com"));
        assert!(row_address("").is_none());
        assert!(row_address("not an address").is_none());
        assert!(row_address("a@b.com; c@d.com").is_none());
    }

    #[test]
    fn preflight_reports_problems() {
        let l = list(
            &["Email Address", "First Name", "Company"],
            &[
                &["dan@x.com", "Dan", "X"],
                &["", "Nobody", ""],
                &["bad", "Bad", ""],
                &["DAN@x.com", "Dan again", "X"],
                &["ruth@y.com", "", "Y"],
            ],
        );
        let t = compose("Hello {First Name} from {Firm}", "<p>{company} {firm}</p>");
        let (targets, p) = plan(&l, &t);
        assert_eq!(targets.iter().map(|(i, a)| (*i, a.email.as_str())).collect::<Vec<_>>(), vec![(0, "dan@x.com"), (4, "ruth@y.com")]);
        assert_eq!(p.messages, 2);
        assert_eq!(p.skipped.iter().map(|r| r.row).collect::<Vec<_>>(), vec![3, 4]);
        assert_eq!(p.duplicates, vec![RowIssue { row: 5, email: "DAN@x.com".into(), fields: vec![] }]);
        assert_eq!(p.unknown_fields, vec!["{Firm}"]);
        assert_eq!(p.blank_fields, vec![RowIssue { row: 6, email: "ruth@y.com".into(), fields: vec!["First Name".into()] }]);
        assert!(!p.over_limit);
    }

    #[test]
    fn email_column_can_be_used_as_a_field() {
        let l = list(&["Email"], &[&["a@b.com"]]);
        let m = message_for(&compose("For {email}", ""), &l, &columns(&l), 0, Addr::new("", "a@b.com"));
        assert_eq!(m.subject, "For a@b.com");
    }
}

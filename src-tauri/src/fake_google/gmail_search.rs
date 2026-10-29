//! Fake Gmail `q` search: is:, in:, label:, from:, to:, cc:, subject:, has:attachment, filename:,
//! newer_than:/older_than:, after:/before:, larger:/smaller:, "phrases", words and -negation.

use super::{FakeMessage, Store, mime};

#[derive(Debug, Clone)]
enum Term {
    Is(String),
    In(String),
    Label(String),
    From(String),
    To(String),
    Cc(String),
    Subject(String),
    HasAttachment,
    Filename(String),
    After(i64),
    Before(i64),
    Larger(i64),
    Smaller(i64),
    Text(String),
}

#[derive(Debug, Clone, Default)]
pub struct Query {
    terms: Vec<(bool, Term)>,
}

fn tokens(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in q.chars() {
        match c {
            '"' => {
                in_q = !in_q;
                cur.push(c);
            }
            c if c.is_whitespace() && !in_q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn date_arg(v: &str) -> Option<i64> {
    if let Ok(secs) = v.parse::<i64>() {
        return Some(secs * 1000);
    }
    let v = v.replace('-', "/");
    let d = chrono::NaiveDate::parse_from_str(&v, "%Y/%m/%d").ok()?;
    Some(d.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis())
}

fn size_arg(v: &str) -> Option<i64> {
    let l = v.to_ascii_lowercase();
    let (n, mult) = if let Some(n) = l.strip_suffix('m') {
        (n, 1_048_576)
    } else if let Some(n) = l.strip_suffix('k') {
        (n, 1024)
    } else {
        (l.as_str(), 1)
    };
    n.parse::<f64>().ok().map(|x| (x * mult as f64) as i64)
}

pub fn parse(q: &str) -> Query {
    let now = crate::fake_google::seed::now_ms();
    let mut terms = Vec::new();
    for t in tokens(q) {
        if t == "OR" || t == "AND" {
            continue;
        }
        let (neg, t) = match t.strip_prefix('-') {
            Some(r) if !r.is_empty() => (true, r.to_string()),
            _ => (false, t),
        };
        let unq = |s: &str| s.trim_matches('"').to_lowercase();
        let term = match t.split_once(':') {
            Some((k, v)) if !v.is_empty() && !k.contains('"') => {
                let v = unq(v);
                match k.to_ascii_lowercase().as_str() {
                    "is" => Term::Is(v),
                    "in" => Term::In(v),
                    "label" => Term::Label(v),
                    "from" => Term::From(v),
                    "to" => Term::To(v),
                    "cc" | "bcc" => Term::Cc(v),
                    "subject" => Term::Subject(v),
                    "has" if v == "attachment" => Term::HasAttachment,
                    "filename" => Term::Filename(v),
                    "after" | "newer" => date_arg(&v).map(Term::After).unwrap_or(Term::Text(v)),
                    "before" | "older" => date_arg(&v).map(Term::Before).unwrap_or(Term::Text(v)),
                    "newer_than" | "older_than" => {
                        let (n, unit) = v.split_at(v.len().saturating_sub(1));
                        let n: i64 = n.parse().unwrap_or(0);
                        let ms = match unit {
                            "d" => n * 86_400_000,
                            "m" => n * 30 * 86_400_000,
                            "y" => n * 365 * 86_400_000,
                            "h" => n * 3_600_000,
                            _ => 0,
                        };
                        if k.eq_ignore_ascii_case("newer_than") { Term::After(now - ms) } else { Term::Before(now - ms) }
                    }
                    "larger" => size_arg(&v).map(Term::Larger).unwrap_or(Term::Text(v)),
                    "smaller" => size_arg(&v).map(Term::Smaller).unwrap_or(Term::Text(v)),
                    _ => Term::Text(unq(&t)),
                }
            }
            _ => Term::Text(unq(&t)),
        };
        terms.push((neg, term));
    }
    Query { terms }
}

fn label_slug(name: &str) -> String {
    name.to_lowercase().chars().map(|c| if c == ' ' || c == '/' || c == '&' { '-' } else { c }).collect()
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn overrides_spam_trash(&self) -> bool {
        self.terms.iter().any(|(_, t)| matches!(t, Term::In(v) if v == "trash" || v == "spam" || v == "anywhere"))
    }

    pub fn matches(&self, g: &Store, m: &FakeMessage) -> bool {
        let root = mime::parse(m.raw.as_bytes());
        let hdr = |n: &str| root.header(n).map(crate::mail::mime::decode_rfc2047).unwrap_or_default().to_lowercase();
        let has = |l: &str| m.label_ids.iter().any(|x| x == l);
        let mut body: Option<String> = None;
        for (neg, t) in &self.terms {
            let ok = match t {
                Term::Is(v) => match v.as_str() {
                    "unread" => has("UNREAD"),
                    "read" => !has("UNREAD"),
                    "starred" => has("STARRED"),
                    "important" => has("IMPORTANT"),
                    _ => true,
                },
                Term::In(v) => match v.as_str() {
                    "anywhere" => true,
                    "inbox" => has("INBOX"),
                    "sent" => has("SENT"),
                    "trash" => has("TRASH"),
                    "spam" => has("SPAM"),
                    "drafts" | "draft" => has("DRAFT"),
                    "starred" => has("STARRED"),
                    other => g.labels.iter().any(|l| label_slug(&l.name) == label_slug(other) && has(&l.id)),
                },
                Term::Label(v) => {
                    has(&v.to_uppercase())
                        || g.labels.iter().any(|l| (label_slug(&l.name) == label_slug(v) || l.name.to_lowercase() == *v) && has(&l.id))
                }
                Term::From(v) => hdr("from").contains(v.as_str()),
                Term::To(v) => hdr("to").contains(v.as_str()) || hdr("cc").contains(v.as_str()) || hdr("bcc").contains(v.as_str()),
                Term::Cc(v) => hdr("cc").contains(v.as_str()),
                Term::Subject(v) => hdr("subject").contains(v.as_str()),
                Term::HasAttachment => mime::has_attachment(&root),
                Term::Filename(v) => {
                    let mut names = Vec::new();
                    mime::attachment_names(&root, &mut names);
                    names.iter().any(|n| n.contains(v.as_str()))
                }
                Term::After(t) => m.internal_date >= *t,
                Term::Before(t) => m.internal_date < *t,
                Term::Larger(n) => m.raw.len() as i64 > *n,
                Term::Smaller(n) => (m.raw.len() as i64) < *n,
                Term::Text(v) => {
                    let b = body.get_or_insert_with(|| {
                        format!("{} {} {} {} {}", hdr("subject"), hdr("from"), hdr("to"), hdr("cc"), mime::body_text(&root).to_lowercase())
                    });
                    b.contains(v.as_str())
                }
            };
            if ok == *neg {
                return false;
            }
        }
        true
    }
}

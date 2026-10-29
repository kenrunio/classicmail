//! Instant Search syntax (Outlook-ish) → FTS5 MATCH + SQL conditions, and → Gmail `q` for
//! "search the server".
//!
//! Supported: words, "phrases", from:, to:, cc:, subject:, body:, hasattachments:yes|no,
//! received:today|yesterday|"this week"|"last week"|"this month"|"last month"|YYYY-MM-DD,
//! read:yes|no (unread), isflagged:yes|no / flag:, importance:high|normal|low.

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};

#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Any,
    From,
    To,
    Subject,
    Body,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    /// (field, text, is_phrase)
    pub terms: Vec<(FieldW, String, bool)>,
    pub has_attachments: Option<bool>,
    pub received: Option<(i64, i64)>,
    pub unread: Option<bool>,
    pub flagged: Option<bool>,
    pub importance: Option<i64>,
}

/// Wrapper so `Parsed` can derive Default.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldW(pub Field);

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

fn yes(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" | "y" => Some(true),
        "no" | "false" | "0" | "n" => Some(false),
        _ => None,
    }
}

fn day_start(d: NaiveDate) -> i64 {
    Local.from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap()).earliest().map(|t| t.timestamp_millis()).unwrap_or(0)
}

/// Date range for a `received:` keyword, in local time. Weeks start on Sunday.
pub fn received_range(v: &str, today: NaiveDate) -> Option<(i64, i64)> {
    let week_start = today - Duration::days(today.weekday().num_days_from_sunday() as i64);
    let month_start = today.with_day(1)?;
    let prev_month_start = (month_start - Duration::days(1)).with_day(1)?;
    let r = |a: NaiveDate, b: NaiveDate| Some((day_start(a), day_start(b)));
    match v.to_ascii_lowercase().replace(['_', '-'], " ").as_str() {
        "today" => r(today, today + Duration::days(1)),
        "yesterday" => r(today - Duration::days(1), today),
        "this week" => r(week_start, week_start + Duration::days(7)),
        "last week" => r(week_start - Duration::days(7), week_start),
        "this month" => r(month_start, (month_start + Duration::days(32)).with_day(1)?),
        "last month" => r(prev_month_start, month_start),
        "this year" => r(today.with_ordinal(1)?, today.with_ordinal(1)?.with_year(today.year() + 1)?),
        other => {
            let d = NaiveDate::parse_from_str(other, "%Y-%m-%d")
                .or_else(|_| NaiveDate::parse_from_str(other, "%d/%m/%Y"))
                .or_else(|_| NaiveDate::parse_from_str(other, "%d.%m.%Y"))
                .ok()?;
            r(d, d + Duration::days(1))
        }
    }
}

pub fn parse(q: &str) -> Parsed {
    parse_at(q, crate::clock::today())
}

pub fn parse_at(q: &str, today: NaiveDate) -> Parsed {
    let mut p = Parsed::default();
    let toks = tokens(q);
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        i += 1;
        let (key, val) = match t.split_once(':') {
            Some((k, v)) if !k.is_empty() && !k.contains('"') => (k.to_ascii_lowercase(), v.to_string()),
            _ => (String::new(), t.clone()),
        };
        let phrase = val.starts_with('"') && val.ends_with('"') && val.len() >= 2;
        let text = val.trim_matches('"').to_string();
        let field = match key.as_str() {
            "" => Some(Field::Any),
            "from" => Some(Field::From),
            "to" | "cc" | "bcc" => Some(Field::To),
            "subject" => Some(Field::Subject),
            "body" => Some(Field::Body),
            _ => None,
        };
        if let Some(f) = field {
            if !text.is_empty() {
                p.terms.push((FieldW(f), text, phrase));
            }
            continue;
        }
        match key.as_str() {
            "hasattachments" | "hasattachment" | "attachments" | "has" => p.has_attachments = yes(&text).or(Some(true)),
            "received" | "sent" | "date" => {
                let mut v = text.clone();
                // received:this week / received:last month (unquoted, two words)
                if !phrase
                    && matches!(v.to_ascii_lowercase().as_str(), "this" | "last")
                    && let Some(next) = toks.get(i)
                {
                    v = format!("{v} {next}");
                    i += 1;
                }
                p.received = received_range(&v, today);
            }
            "read" | "isread" => p.unread = yes(&text).map(|b| !b),
            "unread" => p.unread = yes(&text).or(Some(true)),
            "is" if text.eq_ignore_ascii_case("unread") => p.unread = Some(true),
            "is" if text.eq_ignore_ascii_case("read") => p.unread = Some(false),
            "is" if text.eq_ignore_ascii_case("flagged") || text.eq_ignore_ascii_case("starred") => p.flagged = Some(true),
            "isflagged" | "flag" | "flagged" | "followupflag" => p.flagged = yes(&text).or(Some(true)),
            "importance" => {
                p.importance = match text.to_ascii_lowercase().as_str() {
                    "high" => Some(2),
                    "low" => Some(0),
                    "normal" => Some(1),
                    _ => None,
                }
            }
            _ => p.terms.push((FieldW(Field::Any), t.trim_matches('"').to_string(), phrase)),
        }
    }
    p
}

fn fts_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

impl Parsed {
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.has_attachments.is_none()
            && self.received.is_none()
            && self.unread.is_none()
            && self.flagged.is_none()
            && self.importance.is_none()
    }

    /// FTS5 MATCH expression for the text terms (None if there are none).
    pub fn fts(&self) -> Option<String> {
        let parts: Vec<String> = self
            .terms
            .iter()
            .flat_map(|(f, text, phrase)| {
                let col = match f.0 {
                    Field::Any => "",
                    Field::From => "from_text : ",
                    Field::To => "to_text : ",
                    Field::Subject => "subject : ",
                    Field::Body => "body : ",
                };
                let words: Vec<String> = if *phrase {
                    vec![fts_quote(text)]
                } else {
                    text.split(char::is_whitespace).filter(|w| !w.is_empty()).map(|w| format!("{}*", fts_quote(w))).collect()
                };
                words.into_iter().map(move |w| format!("{col}{w}"))
            })
            .collect();
        if parts.is_empty() { None } else { Some(parts.join(" AND ")) }
    }

    /// Extra SQL conditions on `m` (messages alias) with their parameters.
    pub fn sql(&self) -> (Vec<String>, Vec<rusqlite::types::Value>) {
        use rusqlite::types::Value;
        let mut w = Vec::new();
        let mut a: Vec<Value> = Vec::new();
        if let Some(h) = self.has_attachments {
            w.push(format!("m.has_attachments = {}", h as i64));
        }
        if let Some((from, to)) = self.received {
            w.push("m.internal_date >= ? AND m.internal_date < ?".to_string());
            a.push(Value::Integer(from));
            a.push(Value::Integer(to));
        }
        if let Some(u) = self.unread {
            w.push(format!("m.is_read = {}", (!u) as i64));
        }
        if let Some(f) = self.flagged {
            w.push(if f {
                "(m.is_flagged = 1 OR m.flag_status IS NOT NULL)".into()
            } else {
                "(m.is_flagged = 0 AND m.flag_status IS NULL)".into()
            });
        }
        if let Some(i) = self.importance {
            w.push(format!("m.importance = {i}"));
        }
        (w, a)
    }

    /// Gmail `q` for searching the server.
    pub fn gmail_q(&self) -> String {
        let mut q: Vec<String> = Vec::new();
        for (f, text, phrase) in &self.terms {
            let v = if *phrase || text.contains(' ') { format!("\"{text}\"") } else { text.clone() };
            q.push(match f.0 {
                Field::Any | Field::Body => v,
                Field::From => format!("from:{v}"),
                Field::To => format!("to:{v}"),
                Field::Subject => format!("subject:{v}"),
            });
        }
        if self.has_attachments == Some(true) {
            q.push("has:attachment".into());
        }
        if let Some((from, to)) = self.received {
            q.push(format!("after:{} before:{}", from / 1000, to / 1000));
        }
        match self.unread {
            Some(true) => q.push("is:unread".into()),
            Some(false) => q.push("is:read".into()),
            None => {}
        }
        if self.flagged == Some(true) {
            q.push("is:starred".into());
        }
        if self.importance == Some(2) {
            q.push("is:important".into());
        }
        q.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_outlook_syntax() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(); // Wednesday
        let p = parse_at("from:sarah subject:\"Q3 results\" orion hasattachments:yes received:this week", today);
        assert_eq!(p.terms.len(), 3);
        assert_eq!(p.has_attachments, Some(true));
        let (a, b) = p.received.unwrap();
        assert_eq!(b - a, 7 * 86_400_000);
        let fts = p.fts().unwrap();
        assert_eq!(fts, "from_text : \"sarah\"* AND subject : \"Q3 results\" AND \"orion\"*");
        assert!(
            p.gmail_q().contains("from:sarah") && p.gmail_q().contains("subject:\"Q3 results\"") && p.gmail_q().contains("has:attachment")
        );
        let p = parse_at("read:no importance:high דוח", today);
        assert_eq!(p.unread, Some(true));
        assert_eq!(p.importance, Some(2));
        assert_eq!(p.fts().unwrap(), "\"דוח\"*");
        let (w, args) = p.sql();
        assert_eq!(w.len(), 2);
        assert!(args.is_empty());
        let p = parse_at("received:yesterday", today);
        assert!(p.fts().is_none() && p.received.is_some() && !p.is_empty());
        assert!(parse_at("  ", today).is_empty());
    }
}

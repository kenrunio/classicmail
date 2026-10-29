//! `archive.listMessages`: one page of an archive folder or search, with the same shape as
//! `mail.listMessages` (rows, Outlook arrangements, group headers) so the mail MessageList can show it.

use rusqlite::{Connection, params_from_iter, types::Value};
use serde::Deserialize;

use crate::mail::list::{Group, ListResult, Row, SIZE_BUCKETS, case_buckets, date_buckets};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Source {
    /// archive folder id
    pub folder_id: Option<i64>,
    pub archive_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListParams {
    pub source: Source,
    pub arrange: String,
    pub desc: bool,
    pub search: Option<String>,
    /// "folder" (default) | "archive" (the whole archive) | "all" (Search All Archives)
    pub scope: Option<String>,
    pub offset: i64,
    pub limit: i64,
    pub with_groups: bool,
}

impl Default for ListParams {
    fn default() -> Self {
        ListParams {
            source: Source::default(),
            arrange: "date".into(),
            desc: true,
            search: None,
            scope: None,
            offset: 0,
            limit: 200,
            with_groups: false,
        }
    }
}

/// Instant Search text → FTS5 query over the archive index: `from:`, `to:`, `subject:` (also
/// `body:` when message text was indexed) restrict a word to a column; other words match anywhere.
pub fn fts_query(input: &str) -> Option<String> {
    let mut terms = Vec::new();
    let mut words: Vec<String> = Vec::new();
    // split on whitespace, keeping "quoted phrases"
    let mut cur = String::new();
    let mut quoted = false;
    for ch in input.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    for w in words {
        let (col, val) = match w.split_once(':') {
            Some((k, v)) if !v.is_empty() => match k.to_ascii_lowercase().as_str() {
                "from" => (Some("from_text"), v.to_string()),
                "to" | "cc" => (Some("to_text"), v.to_string()),
                "subject" => (Some("subject"), v.to_string()),
                "body" => (Some("body"), v.to_string()),
                _ => (None, w.clone()),
            },
            _ => (None, w.clone()),
        };
        let val = val.trim_matches('*').replace('"', "\"\"");
        if val.trim().is_empty() {
            continue;
        }
        let term = format!("\"{val}\"*");
        terms.push(match col {
            Some(c) => format!("{c} : {term}"),
            None => term,
        });
    }
    if terms.is_empty() { None } else { Some(terms.join(" AND ")) }
}

pub const ROW_COLS: &str = "m.id, m.archive_id, m.conversation_topic, m.subject, m.from_name, m.from_email, m.to_display, m.date, m.size, m.is_read, \
    m.flag_status, m.has_attachments, m.importance, m.last_verb, m.message_class";

fn row_from(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    let flag: Option<String> = r.get(10)?;
    let class: String = r.get(14)?;
    Ok(Row {
        id: r.get(0)?,
        account_id: r.get(1)?,
        thread_id: r.get(2)?,
        subject: r.get(3)?,
        from_name: r.get(4)?,
        from_email: r.get(5)?,
        to_display: r.get(6)?,
        date: r.get(7)?,
        size: r.get(8)?,
        is_read: r.get::<_, i64>(9)? != 0,
        is_flagged: flag.as_deref() == Some("flagged"),
        flag_status: flag,
        flag_due: None,
        has_attachments: r.get::<_, i64>(11)? != 0,
        importance: r.get(12)?,
        last_verb: r.get(13)?,
        is_draft: false,
        is_invite: class.starts_with("IPM.Schedule.Meeting.Request"),
        preview: String::new(),
        folder: r.get(15)?,
        outbox_id: None,
        outbox_status: None,
        conv: None,
        in_view: None,
    })
}

pub fn list(c: &Connection, p: &ListParams) -> anyhow::Result<ListResult> {
    let mut wh: Vec<String> = Vec::new();
    let mut args: Vec<Value> = Vec::new();
    let search = p.search.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let scope = if search.is_some() { p.scope.as_deref().unwrap_or("folder") } else { "folder" };
    let cross;
    match scope {
        "all" => {
            cross = true;
        }
        "archive" => {
            cross = true;
            let aid = match p.source.archive_id {
                Some(a) => Some(a),
                None => match p.source.folder_id {
                    Some(f) => c.query_row("SELECT archive_id FROM archive_folders WHERE id=?1", [f], |r| r.get(0)).ok(),
                    None => None,
                },
            };
            wh.push("m.archive_id = ?".into());
            args.push(Value::Integer(aid.unwrap_or(-1)));
        }
        _ => {
            cross = false;
            wh.push("m.folder_id = ?".into());
            args.push(Value::Integer(p.source.folder_id.unwrap_or(-1)));
        }
    }
    if let Some(q) = search {
        match fts_query(q) {
            Some(f) => {
                wh.push("m.id IN (SELECT rowid FROM archive_fts WHERE archive_fts MATCH ?)".into());
                args.push(Value::Text(f));
            }
            None => wh.push("0".into()),
        }
    }
    let wh = if wh.is_empty() { "1".to_string() } else { wh.join(" AND ") };
    let (total, unread): (i64, i64) = c.query_row(
        &format!("SELECT COUNT(*), COALESCE(SUM(m.is_read = 0), 0) FROM archive_messages m WHERE {wh}"),
        params_from_iter(args.iter()),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    let from_disp = "CASE WHEN m.from_name <> '' THEN m.from_name ELSE m.from_email END";
    let (sort, group, label, labels): (String, String, String, Option<Vec<String>>) = match p.arrange.as_str() {
        "from" => (format!("lower({from_disp})"), format!("lower({from_disp})"), format!("MIN({from_disp})"), None),
        "to" => ("lower(m.to_display)".into(), "lower(m.to_display)".into(), "MIN(m.to_display)".into(), None),
        "subject" | "conversation" => ("lower(m.subject_norm)".into(), "lower(m.subject_norm)".into(), "MIN(m.subject_norm)".into(), None),
        "size" => {
            let starts: Vec<i64> = SIZE_BUCKETS.iter().map(|b| b.1).collect();
            ("m.size".into(), case_buckets("m.size", &starts), "''".into(), Some(SIZE_BUCKETS.iter().map(|b| b.0.to_string()).collect()))
        }
        "flag" => {
            let e = "(CASE WHEN m.flag_status = 'flagged' THEN 2 WHEN m.flag_status = 'complete' THEN 1 ELSE 0 END)".to_string();
            (e.clone(), e, "''".into(), Some(vec!["Flag: None".into(), "Flag: Completed".into(), "Flag: Flagged".into()]))
        }
        "attachments" => (
            "m.has_attachments".into(),
            "m.has_attachments".into(),
            "''".into(),
            Some(vec!["Attachment: Without Attachments".into(), "Attachment: With Attachments".into()]),
        ),
        "importance" => (
            "m.importance".into(),
            "m.importance".into(),
            "''".into(),
            Some(vec!["Importance: Low".into(), "Importance: Normal".into(), "Importance: High".into()]),
        ),
        _ => {
            let buckets = date_buckets(crate::clock::today());
            let starts: Vec<i64> = buckets.iter().map(|b| b.1).collect();
            ("m.date".into(), case_buckets("m.date", &starts), "''".into(), Some(buckets.into_iter().map(|b| b.0).collect()))
        }
    };
    let dir = if p.desc { "DESC" } else { "ASC" };
    let groups = if p.with_groups {
        let agg = if p.desc { "MAX" } else { "MIN" };
        let sql = format!(
            "SELECT {group} AS g, COUNT(*), {agg}({sort}) AS s, {label} FROM archive_messages m WHERE {wh} GROUP BY g ORDER BY s {dir}, g"
        );
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(params_from_iter(args.iter()), |r| {
            Ok((r.get::<_, Value>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<String>>(3).ok().flatten()))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (key, count, lab) = row?;
            let key_s = match &key {
                Value::Integer(i) => i.to_string(),
                Value::Text(t) => t.clone(),
                Value::Real(f) => f.to_string(),
                _ => String::new(),
            };
            let lab = match (&labels, &key) {
                (Some(ls), Value::Integer(i)) => ls.get(*i as usize).cloned().unwrap_or_default(),
                _ => lab.filter(|l| !l.trim().is_empty()).unwrap_or_else(|| {
                    if matches!(p.arrange.as_str(), "subject" | "conversation") { "(no subject)".into() } else { "(none)".into() }
                }),
            };
            let lab = match p.arrange.as_str() {
                "from" => format!("From: {lab}"),
                "to" => format!("To: {lab}"),
                "subject" => format!("Subject: {lab}"),
                "conversation" => format!("Conversation: {lab}"),
                _ => lab,
            };
            out.push(Group { key: key_s, label: lab, count });
        }
        Some(out)
    } else {
        None
    };
    let folder_col = match scope {
        "all" => {
            ", (SELECT a.display_name || ' › ' || f.name FROM archive_folders f JOIN archives a ON a.id = f.archive_id WHERE f.id = m.folder_id)"
        }
        "archive" => ", (SELECT f.name FROM archive_folders f WHERE f.id = m.folder_id)",
        _ => ", NULL",
    };
    let sql = format!(
        "SELECT {ROW_COLS}{folder_col} FROM archive_messages m WHERE {wh} ORDER BY {sort} {dir}, m.date DESC, m.id DESC LIMIT ? OFFSET ?"
    );
    let mut a2 = args.clone();
    a2.push(Value::Integer(p.limit.clamp(0, 2000)));
    a2.push(Value::Integer(p.offset.max(0)));
    let rows = if p.limit > 0 {
        let mut st = c.prepare(&sql)?;
        st.query_map(params_from_iter(a2.iter()), row_from)?.collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    Ok(ListResult { total, unread, groups, rows, cross_folder: cross, messages: total })
}

/// Next / previous message of the same folder in date order (message window).
pub fn adjacent(c: &Connection, id: i64, next: bool) -> anyhow::Result<Option<i64>> {
    let (folder, date): (i64, i64) =
        c.query_row("SELECT folder_id, date FROM archive_messages WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    // the list is newest first: "next" is older
    let sql = if next {
        "SELECT id FROM archive_messages WHERE folder_id=?1 AND (date < ?2 OR (date = ?2 AND id < ?3)) ORDER BY date DESC, id DESC LIMIT 1"
    } else {
        "SELECT id FROM archive_messages WHERE folder_id=?1 AND (date > ?2 OR (date = ?2 AND id > ?3)) ORDER BY date ASC, id ASC LIMIT 1"
    };
    use rusqlite::OptionalExtension;
    Ok(c.query_row(sql, rusqlite::params![folder, date, id], |r| r.get(0)).optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_syntax() {
        assert_eq!(fts_query("hello").unwrap(), "\"hello\"*");
        assert_eq!(fts_query("from:sarah q3").unwrap(), "from_text : \"sarah\"* AND \"q3\"*");
        assert_eq!(fts_query("subject:\"board meeting\"").unwrap(), "subject : \"board meeting\"*");
        assert_eq!(fts_query("to:\"a\"\"b\"").unwrap(), "to_text : \"ab\"*");
        assert_eq!(fts_query("http://x").unwrap(), "\"http://x\"*");
        assert!(fts_query("  ").is_none());
        assert!(fts_query("from:").is_some());
    }
}

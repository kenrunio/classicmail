//! `mail.listMessages`: one page of a folder / search folder / search result in an Outlook
//! arrangement (Date, From, To, Subject, Size, Flag, Attachments, Importance, Conversation), with
//! the group headers ("Today", "Yesterday", "Last Week"...) and their sizes.

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use rusqlite::{Connection, params_from_iter, types::Value};
use serde::{Deserialize, Serialize};

use super::{search, store};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Source {
    pub folder_id: Option<i64>,
    /// "unread" | "followUp" | "large" | "archive"
    pub search_folder: Option<String>,
    pub account_id: Option<i64>,
    pub ids: Option<Vec<i64>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListParams {
    pub source: Source,
    pub arrange: String,
    pub desc: bool,
    pub search: Option<String>,
    /// "folder" (default) | "all" (Search All Mail Items)
    pub scope: Option<String>,
    pub offset: i64,
    pub limit: i64,
    /// include group headers (Show in Groups)
    pub with_groups: bool,
    /// Show as Conversations: one row per (account, thread) with at least one message in the view
    pub conversations: bool,
    /// with `search`: messages Gmail's own search found, shown too (if they are in the view's scope)
    pub also_ids: Option<Vec<i64>>,
    /// only these messages, within the view (Gmail's extra search results in this folder)
    pub only_ids: Option<Vec<i64>>,
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
            conversations: false,
            also_ids: None,
            only_ids: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub key: String,
    pub label: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: i64,
    pub account_id: i64,
    pub thread_id: String,
    pub subject: String,
    pub from_name: String,
    pub from_email: String,
    pub to_display: String,
    pub date: i64,
    pub size: i64,
    pub is_read: bool,
    pub is_flagged: bool,
    pub flag_status: Option<String>,
    pub flag_due: Option<String>,
    pub has_attachments: bool,
    pub importance: i64,
    pub last_verb: Option<String>,
    pub is_draft: bool,
    pub is_invite: bool,
    pub preview: String,
    /// folder name, for lists spanning folders (search results, search folders)
    pub folder: Option<String>,
    /// queued Outbox item (rows of the Outbox folder)
    pub outbox_id: Option<i64>,
    pub outbox_status: Option<String>,
    /// conversation header (Show as Conversations): the other fields are the newest message's
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conv: Option<super::threads::Conv>,
    /// `mail.listThread`: the message is part of the current view (else it's in another folder)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_view: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResult {
    pub total: i64,
    pub unread: i64,
    pub groups: Option<Vec<Group>>,
    pub rows: Vec<Row>,
    /// the list spans several folders (show "In Folder")
    pub cross_folder: bool,
    /// number of messages in the view (= `total` unless conversations are shown, where `total`
    /// counts conversations)
    pub messages: i64,
}

/// Outlook 2007 date groups, newest first: (label, start ms). A row belongs to the first group whose
/// start ≤ its date. Weeks start on Sunday.
pub fn date_buckets(today: NaiveDate) -> Vec<(String, i64)> {
    let start =
        |d: NaiveDate| Local.from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap()).earliest().map(|t| t.timestamp_millis()).unwrap_or(0);
    let week_start = today - Duration::days(today.weekday().num_days_from_sunday() as i64);
    let month_start = today.with_day(1).unwrap();
    let prev_month = (month_start - Duration::days(1)).with_day(1).unwrap();
    let mut v: Vec<(String, i64)> = vec![("Today".into(), start(today)), ("Yesterday".into(), start(today - Duration::days(1)))];
    for d in 2..7 {
        let day = today - Duration::days(d);
        if day >= week_start {
            v.push((day.format("%A").to_string(), start(day)));
        }
    }
    v.push(("Last Week".into(), start(week_start - Duration::days(7))));
    v.push(("Two Weeks Ago".into(), start(week_start - Duration::days(14))));
    v.push(("Three Weeks Ago".into(), start(week_start - Duration::days(21))));
    v.push(("Earlier this Month".into(), start(month_start)));
    v.push(("Last Month".into(), start(prev_month)));
    // keep strictly decreasing starts
    let mut out: Vec<(String, i64)> = Vec::new();
    for (l, s) in v {
        if out.last().map(|(_, p)| s < *p).unwrap_or(true) {
            out.push((l, s));
        }
    }
    out.push(("Older".into(), i64::MIN));
    out
}

pub const SIZE_BUCKETS: &[(&str, i64)] = &[
    ("Enormous (> 5 MB)", 5 * 1024 * 1024),
    ("Huge (1 - 5 MB)", 1024 * 1024),
    ("Very Large (500 KB - 1 MB)", 500 * 1024),
    ("Large (100 - 500 KB)", 100 * 1024),
    ("Medium (25 - 100 KB)", 25 * 1024),
    ("Small (10 - 25 KB)", 10 * 1024),
    ("Tiny (< 10 KB)", i64::MIN),
];

pub(super) struct Base {
    pub from: String,
    pub wh: Vec<String>,
    pub args: Vec<Value>,
    pub date_col: &'static str,
    pub id_col: &'static str,
    pub cross: bool,
}

pub(super) fn base(c: &Connection, p: &ListParams) -> anyhow::Result<Option<Base>> {
    let mut b = Base { from: "messages m".into(), wh: vec![], args: vec![], date_col: "m.internal_date", id_col: "m.id", cross: false };
    let search = p.search.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let all_scope = search.is_some() && p.scope.as_deref() == Some("all");
    if all_scope {
        b.cross = true;
        b.wh.push("m.location <> 2".into());
    } else if let Some(ids) = &p.source.ids {
        b.cross = true;
        let list: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
        b.wh.push(format!("m.id IN ({})", if list.is_empty() { "NULL".into() } else { list.join(",") }));
    } else if let Some(sf) = &p.source.search_folder {
        // "archive" is a folder of its own (Gmail-archived mail), the others are cross-folder views
        b.cross = sf != "archive";
        if let Some(a) = p.source.account_id {
            b.wh.push("m.account_id = ?".into());
            b.args.push(Value::Integer(a));
        }
        b.wh.push("m.location = 0".into());
        b.wh.push(
            match sf.as_str() {
                "unread" => "m.is_read = 0 AND m.is_draft = 0",
                "followUp" => "(m.is_flagged = 1 OR m.flag_status IS NOT NULL)",
                "large" => "m.size > 102400",
                // archived (Gmail): in no visible folder at all — only in All Mail
                "archive" => "m.is_draft = 0 AND NOT EXISTS (SELECT 1 FROM message_folders x JOIN folders f ON f.id = x.folder_id WHERE x.message_id = m.id AND f.hidden = 0 AND f.role <> 'all')",
                _ => "0",
            }
            .into(),
        );
    } else if let Some(fid) = p.source.folder_id {
        let f = store::folder(c, fid)?;
        match f.role.as_str() {
            store::ROLE_OUTBOX => return Ok(None),
            store::ROLE_ALL => {
                b.wh.push("m.account_id = ? AND m.location = 0 AND m.is_draft = 0".into());
                b.args.push(Value::Integer(f.account_id));
            }
            _ => {
                b.from = "message_folders mf JOIN messages m ON m.id = mf.message_id".into();
                b.wh.push("mf.folder_id = ?".into());
                b.args.push(Value::Integer(fid));
                b.date_col = "mf.msg_date";
                b.id_col = "mf.message_id";
            }
        }
    } else {
        b.wh.push("0".into());
    }
    if let Some(ids) = &p.only_ids {
        let list: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
        b.wh.push(format!("m.id IN ({})", if list.is_empty() { "NULL".into() } else { list.join(",") }));
    }
    if let Some(q) = search {
        let parsed = search::parse(q);
        let mut sw = Vec::new();
        if let Some(fts) = parsed.fts() {
            sw.push("m.id IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)".to_string());
            b.args.push(Value::Text(fts));
        }
        let (w, a) = parsed.sql();
        sw.extend(w);
        b.args.extend(a);
        match p.also_ids.as_deref().filter(|ids| !ids.is_empty()) {
            // Gmail's matches join the local ones (the view's folder / scope still applies)
            Some(ids) if !sw.is_empty() => {
                let ids: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
                b.wh.push(format!("(({}) OR m.id IN ({}))", sw.join(" AND "), ids.join(",")));
            }
            _ => b.wh.extend(sw),
        }
    }
    Ok(Some(b))
}

struct Arr {
    sort: String,
    group: String,
    label: String,
    /// extra ORDER BY key after `sort` (conversation: keep threads together)
    tiebreak: Option<&'static str>,
    labels: Option<Vec<String>>,
    /// extra join needed by the sort key
    join: &'static str,
}

pub fn case_buckets(col: &str, starts: &[i64]) -> String {
    let mut s = String::from("CASE");
    for (i, st) in starts.iter().enumerate() {
        if *st == i64::MIN {
            s.push_str(&format!(" ELSE {i}"));
            break;
        }
        s.push_str(&format!(" WHEN {col} >= {st} THEN {i}"));
    }
    s.push_str(" END");
    s
}

fn arrangement(arrange: &str, date_col: &str) -> Arr {
    let from_disp = "CASE WHEN m.from_name <> '' THEN m.from_name ELSE m.from_email END";
    match arrange {
        "from" => Arr {
            sort: format!("lower({from_disp})"),
            group: format!("lower({from_disp})"),
            label: format!("MIN({from_disp})"),
            tiebreak: None,
            labels: None,
            join: "",
        },
        "to" => Arr {
            sort: "lower(m.to_display)".into(),
            group: "lower(m.to_display)".into(),
            label: "MIN(m.to_display)".into(),
            tiebreak: None,
            labels: None,
            join: "",
        },
        "subject" => Arr {
            sort: "lower(m.subject_norm)".into(),
            group: "lower(m.subject_norm)".into(),
            label: "MIN(m.subject_norm)".into(),
            tiebreak: None,
            labels: None,
            join: "",
        },
        "size" => {
            let starts: Vec<i64> = SIZE_BUCKETS.iter().map(|b| b.1).collect();
            Arr {
                sort: "m.size".into(),
                group: case_buckets("m.size", &starts),
                label: "''".into(),
                tiebreak: None,
                labels: Some(SIZE_BUCKETS.iter().map(|b| b.0.to_string()).collect()),
                join: "",
            }
        }
        "flag" => {
            let e = "(CASE WHEN m.is_flagged = 1 OR m.flag_status = 'flagged' THEN 2 WHEN m.flag_status = 'complete' THEN 1 ELSE 0 END)";
            Arr {
                sort: e.into(),
                group: e.into(),
                label: "''".into(),
                tiebreak: None,
                labels: Some(vec!["Flag: None".into(), "Flag: Completed".into(), "Flag: Flagged".into()]),
                join: "",
            }
        }
        "attachments" => Arr {
            sort: "m.has_attachments".into(),
            group: "m.has_attachments".into(),
            label: "''".into(),
            tiebreak: None,
            labels: Some(vec!["Attachment: Without Attachments".into(), "Attachment: With Attachments".into()]),
            join: "",
        },
        "importance" => Arr {
            sort: "m.importance".into(),
            group: "m.importance".into(),
            label: "''".into(),
            tiebreak: None,
            labels: Some(vec!["Importance: Low".into(), "Importance: Normal".into(), "Importance: High".into()]),
            join: "",
        },
        "conversation" => Arr {
            sort: "th.td".into(),
            join: " JOIN (SELECT account_id AS ta, thread_id AS tt, MAX(internal_date) AS td FROM messages GROUP BY account_id, thread_id) th ON th.ta = m.account_id AND th.tt = m.thread_id",
            group: "(m.account_id || ':' || m.thread_id)".into(),
            label: "MIN(m.subject_norm)".into(),
            tiebreak: Some("m.account_id, m.thread_id"),
            labels: None,
        },
        _ => {
            let buckets = date_buckets(crate::clock::today());
            let starts: Vec<i64> = buckets.iter().map(|b| b.1).collect();
            Arr {
                sort: date_col.into(),
                group: case_buckets(date_col, &starts),
                label: "''".into(),
                tiebreak: None,
                labels: Some(buckets.into_iter().map(|b| b.0).collect()),
                join: "",
            }
        }
    }
}

/// The message's first visible folder by name ("Archive" when it is in none), for cross-folder lists.
pub(super) const FOLDER_NAME_COL: &str = "COALESCE((SELECT f.name FROM message_folders x JOIN folders f ON f.id = x.folder_id WHERE x.message_id = m.id AND f.hidden = 0 ORDER BY f.position, f.id LIMIT 1), 'Archive')";

pub(super) const ROW_COLS: &str = "m.id, m.account_id, m.thread_id, m.subject, m.from_name, m.from_email, m.to_display, m.internal_date, m.size, m.is_read, \
    m.is_flagged, m.flag_status, m.flag_due, m.has_attachments, m.importance, m.last_verb, m.is_draft, m.is_invite, substr(m.snippet, 1, 160)";

pub fn list(c: &Connection, p: &ListParams) -> anyhow::Result<ListResult> {
    let Some(b) = base(c, p)? else {
        return outbox(c, p.source.folder_id.unwrap_or(0));
    };
    if p.conversations {
        return super::threads::list(c, p, b);
    }
    let wh = if b.wh.is_empty() { "1".to_string() } else { b.wh.join(" AND ") };
    let (total, unread): (i64, i64) = c.query_row(
        &format!("SELECT COUNT(*), COALESCE(SUM(m.is_read = 0), 0) FROM {} WHERE {wh}", b.from),
        params_from_iter(b.args.iter()),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let arr = arrangement(&p.arrange, b.date_col);
    let dir = if p.desc { "DESC" } else { "ASC" };
    let from = format!("{}{}", b.from, arr.join);
    let groups = if p.with_groups {
        let agg = if p.desc { "MAX" } else { "MIN" };
        let tb = arr.tiebreak.map(|_| ", g").unwrap_or("");
        let sql = format!(
            "SELECT {g} AS g, COUNT(*), {agg}({s}) AS s, {l} FROM {from} WHERE {wh} GROUP BY g ORDER BY s {dir}{tb}, g",
            g = arr.group,
            s = arr.sort,
            l = arr.label,
        );
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(params_from_iter(b.args.iter()), |r| {
            let key: Value = r.get(0)?;
            let label: Option<String> = r.get(3).ok();
            Ok((key, r.get::<_, i64>(1)?, label))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (key, count, label) = row?;
            let (key, label) = group_label(&p.arrange, arr.labels.as_deref(), &key, label);
            out.push(Group { key, label, count });
        }
        Some(out)
    } else {
        None
    };
    let folder_col = if b.cross { format!(", {FOLDER_NAME_COL}") } else { ", NULL".into() };
    let tb = arr.tiebreak.map(|t| format!(", {t}")).unwrap_or_default();
    let sql = format!(
        "SELECT {ROW_COLS}{folder_col} FROM {from} WHERE {wh} ORDER BY {s} {dir}{tb}, {d} DESC, {i} DESC LIMIT ? OFFSET ?",
        s = arr.sort,
        d = b.date_col,
        i = b.id_col
    );
    let mut args = b.args.clone();
    args.push(Value::Integer(p.limit.clamp(0, 2000)));
    args.push(Value::Integer(p.offset.max(0)));
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map(params_from_iter(args.iter()), row_from)?.collect::<Result<Vec<_>, _>>()?;
    Ok(ListResult { total, unread, groups, rows, cross_folder: b.cross, messages: total })
}

/// A group header's key and label: a fixed label by index (date, size...) or the group's own text
/// with its arrangement's prefix ("From: Sarah Cohen").
pub(super) fn group_label(arrange: &str, labels: Option<&[String]>, key: &Value, label: Option<String>) -> (String, String) {
    let key_s = match key {
        Value::Integer(i) => i.to_string(),
        Value::Text(t) => t.clone(),
        Value::Real(f) => f.to_string(),
        _ => String::new(),
    };
    let label = match (labels, key) {
        (Some(ls), Value::Integer(i)) => ls.get(*i as usize).cloned().unwrap_or_default(),
        _ => label
            .filter(|l| !l.trim().is_empty())
            .unwrap_or_else(|| if arrange == "subject" || arrange == "conversation" { "(no subject)".into() } else { "(none)".into() }),
    };
    let label = match arrange {
        "from" => format!("From: {label}"),
        "to" => format!("To: {label}"),
        "subject" => format!("Subject: {label}"),
        "conversation" => format!("Conversation: {label}"),
        _ => label,
    };
    (key_s, label)
}

/// A list row from `ROW_COLS` + the folder name column (index 19).
pub(super) fn row_from(r: &rusqlite::Row) -> rusqlite::Result<Row> {
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
        is_flagged: r.get::<_, i64>(10)? != 0,
        flag_status: r.get(11)?,
        flag_due: r.get(12)?,
        has_attachments: r.get::<_, i64>(13)? != 0,
        importance: r.get(14)?,
        last_verb: r.get(15)?,
        is_draft: r.get::<_, i64>(16)? != 0,
        is_invite: r.get::<_, i64>(17)? != 0,
        preview: r.get::<_, Option<String>>(18)?.unwrap_or_default(),
        folder: r.get(19)?,
        outbox_id: None,
        outbox_status: None,
        conv: None,
        in_view: None,
    })
}

/// The Outbox folder lists queued messages from the compose builder's `outbox` table.
fn outbox(c: &Connection, folder_id: i64) -> anyhow::Result<ListResult> {
    let account_id: i64 = c.query_row("SELECT account_id FROM folders WHERE id=?1", [folder_id], |r| r.get(0))?;
    let mut st = c.prepare("SELECT id, compose_json, send_at, status, created_at, COALESCE(length(raw), size) FROM outbox WHERE account_id=?1 ORDER BY created_at DESC")?;
    let rows = st
        .query_map([account_id], |r| {
            let id: i64 = r.get(0)?;
            let json: String = r.get(1)?;
            let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
            let to = match &v["to"] {
                serde_json::Value::Array(a) => a
                    .iter()
                    .map(|x| match x {
                        serde_json::Value::String(s) => s.clone(),
                        o => o["name"].as_str().filter(|s| !s.is_empty()).or(o["email"].as_str()).unwrap_or("").to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("; "),
                serde_json::Value::String(s) => s.clone(),
                _ => String::new(),
            };
            Ok(Row {
                id: -id,
                account_id,
                thread_id: String::new(),
                subject: v["subject"].as_str().unwrap_or("").to_string(),
                from_name: String::new(),
                from_email: String::new(),
                to_display: to,
                date: r.get::<_, i64>(2)?.max(r.get::<_, i64>(4)?),
                size: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                is_read: true,
                is_flagged: false,
                flag_status: None,
                flag_due: None,
                has_attachments: v["attachments"].as_array().map(|a| !a.is_empty()).unwrap_or(false),
                importance: match v["importance"].as_str() {
                    Some("high") => 2,
                    Some("low") => 0,
                    _ => 1,
                },
                last_verb: None,
                is_draft: false,
                is_invite: false,
                preview: String::new(),
                folder: None,
                outbox_id: Some(id),
                outbox_status: r.get(3)?,
                conv: None,
                in_view: None,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let n = rows.len() as i64;
    Ok(ListResult { total: n, unread: 0, groups: None, rows, cross_folder: false, messages: n })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_buckets_are_ordered() {
        // Wednesday 14 Oct 2026: Today, Yesterday, Monday, Sunday, Last Week, ...
        let b = date_buckets(NaiveDate::from_ymd_opt(2026, 10, 14).unwrap());
        let labels: Vec<&str> = b.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(
            labels,
            ["Today", "Yesterday", "Monday", "Sunday", "Last Week", "Two Weeks Ago", "Three Weeks Ago", "Last Month", "Older"]
        );
        for w in b.windows(2) {
            assert!(w[0].1 > w[1].1);
        }
        // Sunday: no weekday groups, yesterday is in last week
        let b = date_buckets(NaiveDate::from_ymd_opt(2026, 10, 25).unwrap());
        let labels: Vec<&str> = b.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(
            labels,
            ["Today", "Yesterday", "Last Week", "Two Weeks Ago", "Three Weeks Ago", "Earlier this Month", "Last Month", "Older"]
        );
    }
}

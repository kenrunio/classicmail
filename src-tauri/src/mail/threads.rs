//! Show as Conversations: `mail.listMessages { conversations: true }` lists one row per
//! (account, Gmail thread) that has at least one message in the view, with aggregates over those
//! messages (latest date, count, unread, size, attachments, importance, flag) plus thread-wide
//! numbers (messages in every folder, participants). `mail.listThread` returns every message of a
//! thread across folders (the expanded conversation's child rows).
//!
//! Sorting, grouping and paging work on conversations: a conversation sorts and groups by its
//! newest message in the view (Date), its total size (Size), etc.

use std::collections::HashSet;

use rusqlite::{Connection, params, params_from_iter, types::Value};
use serde::Serialize;

use super::list::{self, Base, FOLDER_NAME_COL, Group, ListParams, ListResult, ROW_COLS, Row};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conv {
    /// "<accountId>:<threadId>" — stable key of the conversation
    pub key: String,
    /// messages of the conversation in this view (folder / search)
    pub count: i64,
    /// messages of the conversation in all folders (what expanding shows)
    pub total: i64,
    /// unread messages in this view
    pub unread: i64,
    /// total size of the messages in this view
    pub size: i64,
    pub has_attachments: bool,
    /// highest importance in the view (0 low, 1 normal, 2 high)
    pub importance: i64,
    /// 2 = some message flagged, 1 = completed, 0 = none
    pub flag: i64,
    /// who wrote in it, whoever wrote last first, then the others back in time; not you, even if
    /// you replied (unless only you wrote): "Eran Gilad; Liat Nahum"
    pub participants: String,
    /// ids of the conversation's messages in this view (what actions on the header apply to)
    pub ids: Vec<i64>,
}

/// Conversation key column: Gmail always has thread ids; a local message without one is its own
/// conversation.
const TKEY: &str = "CASE WHEN m.thread_id = '' THEN 'id:' || m.id ELSE m.thread_id END";
const FROM_DISP: &str = "CASE WHEN m.from_name <> '' THEN m.from_name ELSE m.from_email END";

/// One row per conversation of the view. `MAX(date)` is the only min/max aggregate, so SQLite
/// takes the bare columns (newest message id, sender, subject) from the newest message.
/// `extra`: bare columns the sort needs (`cfrom` / `cto` / `csn`); `with_ids` adds the id list.
fn conv_sql(b: &Base, wh: &str, extra: &str, with_ids: bool) -> String {
    let ids = if with_ids { ", group_concat(m.id) AS cids" } else { "" };
    format!(
        "SELECT m.account_id AS ca, {TKEY} AS ct, MAX({d}) AS cd, m.id AS cid, COUNT(*) AS cn, SUM(m.is_read = 0) AS cu, \
         SUM(m.size) AS csz, (SUM(m.has_attachments) > 0) AS catt, \
         CASE WHEN SUM(m.importance = 2) > 0 THEN 2 WHEN SUM(m.importance = 1) > 0 THEN 1 ELSE 0 END AS cimp, \
         CASE WHEN SUM(m.is_flagged = 1 OR m.flag_status = 'flagged') > 0 THEN 2 WHEN SUM(m.flag_status = 'complete') > 0 THEN 1 ELSE 0 END AS cflag, \
         SUM(m.location <> 0) AS coth{extra}{ids} \
         FROM {from} WHERE {wh} GROUP BY m.account_id, ct",
        d = b.date_col,
        from = b.from,
    )
}

struct Arr {
    sort: String,
    group: String,
    label: String,
    labels: Option<Vec<String>>,
    /// bare columns of the newest message the sort needs
    extra: String,
}

fn arrangement(arrange: &str) -> Arr {
    let fixed = |sort: &str, labels: Vec<String>| Arr {
        sort: sort.into(),
        group: sort.into(),
        label: "''".into(),
        labels: Some(labels),
        extra: String::new(),
    };
    let text = |col: &str, expr: &str| Arr {
        sort: format!("lower(cv.{col})"),
        group: format!("lower(cv.{col})"),
        label: format!("MIN(cv.{col})"),
        labels: None,
        extra: format!(", {expr} AS {col}"),
    };
    match arrange {
        "from" => text("cfrom", FROM_DISP),
        "to" => text("cto", "m.to_display"),
        "subject" => text("csn", "m.subject_norm"),
        "size" => {
            let starts: Vec<i64> = list::SIZE_BUCKETS.iter().map(|b| b.1).collect();
            Arr {
                sort: "cv.csz".into(),
                group: list::case_buckets("cv.csz", &starts),
                label: "''".into(),
                labels: Some(list::SIZE_BUCKETS.iter().map(|b| b.0.to_string()).collect()),
                extra: String::new(),
            }
        }
        "flag" => fixed("cv.cflag", vec!["Flag: None".into(), "Flag: Completed".into(), "Flag: Flagged".into()]),
        "attachments" => fixed("cv.catt", vec!["Attachment: Without Attachments".into(), "Attachment: With Attachments".into()]),
        "importance" => fixed("cv.cimp", vec!["Importance: Low".into(), "Importance: Normal".into(), "Importance: High".into()]),
        // date, and "conversation" (the conversations are already together)
        _ => {
            let buckets = list::date_buckets(crate::clock::today());
            let starts: Vec<i64> = buckets.iter().map(|b| b.1).collect();
            Arr {
                sort: "cv.cd".into(),
                group: list::case_buckets("cv.cd", &starts),
                label: "''".into(),
                labels: Some(buckets.into_iter().map(|b| b.0).collect()),
                extra: String::new(),
            }
        }
    }
}

struct ConvRow {
    account_id: i64,
    ct: String,
    date: i64,
    rep: i64,
    conv: Conv,
    other: i64,
}

pub(super) fn list(c: &Connection, p: &ListParams, b: Base) -> anyhow::Result<ListResult> {
    let wh = if b.wh.is_empty() { "1".to_string() } else { b.wh.join(" AND ") };
    let arr = arrangement(&p.arrange);
    let dir = if p.desc { "DESC" } else { "ASC" };
    let (mut total, mut messages, mut unread) = (0i64, 0i64, 0i64);
    let groups = if p.with_groups {
        // one pass: the groups also give the totals
        let cv = conv_sql(&b, &wh, &arr.extra, false);
        let agg = if p.desc { "MAX" } else { "MIN" };
        let sql = format!(
            "SELECT {g} AS g, COUNT(*), {agg}({s}) AS s, {l}, SUM(cv.cn), SUM(cv.cu) FROM ({cv}) cv GROUP BY g ORDER BY s {dir}, g",
            g = arr.group,
            s = arr.sort,
            l = arr.label
        );
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(params_from_iter(b.args.iter()), |r| {
            Ok((
                r.get::<_, Value>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(3).ok().flatten(),
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (key, count, label, n, u) = row?;
            total += count;
            messages += n;
            unread += u;
            // arranged by conversation, conversations group by date (no "Conversation:" prefix)
            let arrange = if p.arrange == "conversation" { "date" } else { p.arrange.as_str() };
            let (key, label) = list::group_label(arrange, arr.labels.as_deref(), &key, label);
            out.push(Group { key, label, count });
        }
        Some(out)
    } else {
        let cv = conv_sql(&b, &wh, "", false);
        (total, messages, unread) = c.query_row(
            &format!("SELECT COUNT(*), COALESCE(SUM(cv.cn), 0), COALESCE(SUM(cv.cu), 0) FROM ({cv}) cv"),
            params_from_iter(b.args.iter()),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        None
    };
    let limit = p.limit.clamp(0, 2000);
    let mut rows = Vec::new();
    if limit > 0 {
        // the page of conversations (narrow rows), then their newest messages by id
        let cv = conv_sql(&b, &wh, &arr.extra, true);
        let sql = format!(
            "SELECT cv.ca, cv.ct, cv.cd, cv.cid, cv.cn, cv.cu, cv.csz, cv.catt, cv.cimp, cv.cflag, cv.coth, cv.cids \
             FROM ({cv}) cv ORDER BY {s} {dir}, cv.cd DESC, cv.cid DESC LIMIT ? OFFSET ?",
            s = arr.sort
        );
        let mut args = b.args.clone();
        args.push(Value::Integer(limit));
        args.push(Value::Integer(p.offset.max(0)));
        let mut st = c.prepare(&sql)?;
        let page = st
            .query_map(params_from_iter(args.iter()), |r| {
                let account_id: i64 = r.get(0)?;
                let ct: String = r.get(1)?;
                let ids: String = r.get::<_, Option<String>>(11)?.unwrap_or_default();
                let mut ids: Vec<i64> = ids.split(',').filter_map(|x| x.parse().ok()).collect();
                ids.sort_unstable_by(|a, b| b.cmp(a));
                Ok(ConvRow {
                    conv: Conv {
                        key: format!("{account_id}:{ct}"),
                        count: r.get(4)?,
                        total: 0,
                        unread: r.get(5)?,
                        size: r.get(6)?,
                        has_attachments: r.get::<_, i64>(7)? != 0,
                        importance: r.get(8)?,
                        flag: r.get(9)?,
                        participants: String::new(),
                        ids,
                    },
                    account_id,
                    ct,
                    date: r.get(2)?,
                    rep: r.get(3)?,
                    other: r.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let folder_col = if b.cross { format!(", {FOLDER_NAME_COL}") } else { ", NULL".into() };
        let mut rep_st = c.prepare_cached(&format!("SELECT {ROW_COLS}{folder_col} FROM messages m WHERE m.id = ?1"))?;
        // thread-wide numbers (index on (account_id, thread_id, internal_date)), newest first
        let mut thr_st = c.prepare_cached(
            "SELECT from_name, from_email, location FROM messages WHERE account_id = ?1 AND thread_id = ?2 ORDER BY internal_date DESC, id DESC",
        )?;
        let mine = my_addresses(c)?;
        for cr in page {
            let ConvRow { account_id, ct, date, rep, mut conv, other } = cr;
            let mut row = rep_st.query_row([rep], list::row_from)?;
            row.date = date;
            if ct.starts_with("id:") {
                conv.total = conv.count;
                conv.participants = if row.from_name.is_empty() { row.from_email.clone() } else { row.from_name.clone() };
            } else {
                let msgs = thr_st
                    .query_map(params![account_id, ct], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                // what listThread shows: messages outside Deleted Items / Junk, plus the view's own
                conv.total = (msgs.iter().filter(|m| m.2 == 0).count() as i64 + other).max(conv.count);
                conv.participants =
                    participants(msgs.iter().filter(|m| m.2 == 0 || other > 0).map(|m| (m.0.as_str(), m.1.as_str())), &mine);
            }
            row.conv = Some(conv);
            rows.push(row);
        }
    }
    Ok(ListResult { total, unread, groups, rows, cross_folder: b.cross, messages })
}

/// Your own addresses (every account, and their Send As addresses), lower case.
fn my_addresses(c: &Connection) -> anyhow::Result<HashSet<String>> {
    let mut st = c.prepare_cached("SELECT lower(email) FROM accounts UNION SELECT lower(email) FROM send_as")?;
    Ok(st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<_, _>>()?)
}

/// A conversation's From column: the distinct senders of `senders` (newest first, so each sits
/// where they last wrote), "; " separated, leaving out `mine` (your own addresses), unless you
/// are the only one who wrote.
pub fn participants<'a>(senders: impl Iterator<Item = (&'a str, &'a str)>, mine: &HashSet<String>) -> String {
    let mut seen = HashSet::new();
    let mut others: Vec<&str> = Vec::new();
    let mut me: Option<&str> = None;
    for (name, email) in senders {
        let key = if email.is_empty() { name.to_lowercase() } else { email.to_lowercase() };
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        let shown = if name.trim().is_empty() { email } else { name };
        if mine.contains(&key) {
            me.get_or_insert(shown);
        } else {
            others.push(shown);
        }
    }
    if others.is_empty() { me.unwrap_or_default().to_string() } else { others.join("; ") }
}

/// Every message of a thread, newest first: those in the view (`view`, the list's params) plus
/// those in other folders except Deleted Items / Junk E-mail. `folder` is always filled and
/// `in_view` tells which ones are part of the view.
pub fn list_thread(c: &Connection, account_id: i64, thread_id: &str, view: Option<&ListParams>) -> anyhow::Result<Vec<Row>> {
    let (cond, key): (&str, Value) = match thread_id.strip_prefix("id:").and_then(|x| x.parse::<i64>().ok()) {
        Some(id) => ("m.id = ?", Value::Integer(id)),
        None => ("m.thread_id = ?", Value::Text(thread_id.to_string())),
    };
    let mut in_view: HashSet<i64> = HashSet::new();
    if let Some(v) = view {
        let mut v = v.clone();
        v.conversations = false;
        if let Some(mut b) = list::base(c, &v)? {
            b.wh.push(format!("m.account_id = ? AND {cond}"));
            b.args.push(Value::Integer(account_id));
            b.args.push(key.clone());
            let mut st = c.prepare(&format!("SELECT m.id FROM {} WHERE {}", b.from, b.wh.join(" AND ")))?;
            for id in st.query_map(params_from_iter(b.args.iter()), |r| r.get::<_, i64>(0))? {
                in_view.insert(id?);
            }
        }
    }
    let sql = format!(
        "SELECT {ROW_COLS}, {FOLDER_NAME_COL}, m.location \
         FROM messages m WHERE m.account_id = ? AND {cond} ORDER BY m.internal_date DESC, m.id DESC"
    );
    let mut st = c.prepare(&sql)?;
    let rows = st
        .query_map(params_from_iter([Value::Integer(account_id), key].iter()), |r| Ok((list::row_from(r)?, r.get::<_, i64>(20)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter(|(row, loc)| *loc == 0 || in_view.contains(&row.id))
        .map(|(mut row, _)| {
            row.in_view = Some(in_view.contains(&row.id));
            row
        })
        .collect())
}

#[cfg(test)]
mod participant_tests {
    use super::*;

    #[test]
    fn latest_sender_first_then_back_in_time_without_you() {
        let mine: HashSet<String> = ["kenny@duvz.demo".to_string()].into();
        // newest first: you replied last, before that Eran, Liat, Bo, and Liat started it
        let thread = [
            ("Kenny Green", "Kenny@DUVZ.demo"),
            ("Eran Gilad", "eran@x.example"),
            ("Liat Nahum", "LIAT@x.example"),
            ("Bo Ek", "bo@x.example"),
            ("Kenny Green", "kenny@duvz.demo"),
            ("Liat Nahum", "liat@x.example"),
        ];
        assert_eq!(participants(thread.into_iter(), &mine), "Eran Gilad; Liat Nahum; Bo Ek");
        // only you wrote (a thread you started that nobody answered yet): you
        assert_eq!(participants([("Kenny Green", "kenny@duvz.demo")].into_iter(), &mine), "Kenny Green");
        // a sender without a name shows the address
        assert_eq!(participants([("", "a@x.example"), ("Bo", "b@x.example")].into_iter(), &mine), "a@x.example; Bo");
    }
}

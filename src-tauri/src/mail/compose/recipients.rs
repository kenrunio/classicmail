//! Outlook's recipient auto-complete ("nickname cache") + contacts, if that domain's table exists.

use std::{collections::HashSet, sync::Arc};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::store::{columns, table_exists};
use crate::ctx::AppCtx;

#[derive(Debug, Deserialize)]
pub struct SuggestParams {
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub name: String,
    pub email: String,
    /// "recent" (auto-complete list) | "contact"
    pub source: &'static str,
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

pub fn suggest_sync(c: &Connection, query: &str, limit: usize) -> anyhow::Result<Vec<Suggestion>> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let prefix = format!("{}%", like_escape(&q));
    let word = format!("% {}%", like_escape(&q));
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut st = c.prepare(
        "SELECT name, email FROM recipient_cache \
         WHERE email LIKE ?1 ESCAPE '\\' OR lower(name) LIKE ?1 ESCAPE '\\' OR lower(name) LIKE ?2 ESCAPE '\\' \
         ORDER BY use_count DESC, last_used DESC LIMIT ?3",
    )?;
    for r in st.query_map(params![prefix, word, limit as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (name, email) = r?;
        if seen.insert(email.to_lowercase()) {
            out.push(Suggestion { name, email, source: "recent" });
        }
    }
    if out.len() < limit {
        for s in contact_matches(c, &prefix, &word, limit)? {
            if out.len() >= limit {
                break;
            }
            if seen.insert(s.email.to_lowercase()) {
                out.push(s);
            }
        }
    }
    Ok(out)
}

/// The contacts table belongs to another domain; adapt to whatever columns it has.
fn contact_matches(c: &Connection, prefix: &str, word: &str, limit: usize) -> anyhow::Result<Vec<Suggestion>> {
    if !table_exists(c, "contacts") {
        return Ok(vec![]);
    }
    let cols = columns(c, "contacts");
    let has = |n: &str| cols.iter().any(|c| c == n);
    let name_col = ["display_name", "name", "full_name", "file_as"].into_iter().find(|n| has(n));
    let Some(name_col) = name_col else { return Ok(vec![]) };
    let sql = if let Some(ej) = ["emails_json", "emails"].into_iter().find(|n| has(n)) {
        // emails as a JSON array of strings or of {value|email|address}
        format!(
            "SELECT c.{name_col}, COALESCE(json_extract(e.value,'$.value'), json_extract(e.value,'$.email'), \
                    json_extract(e.value,'$.address'), e.value) AS em \
             FROM contacts c, json_each(CASE WHEN json_valid(c.{ej}) THEN c.{ej} ELSE '[]' END) e \
             WHERE em LIKE ?1 ESCAPE '\\' OR lower(c.{name_col}) LIKE ?1 ESCAPE '\\' OR lower(c.{name_col}) LIKE ?2 ESCAPE '\\' \
             LIMIT ?3"
        )
    } else if let Some(ec) = ["email", "primary_email", "email1"].into_iter().find(|n| has(n)) {
        format!(
            "SELECT {name_col}, {ec} FROM contacts WHERE {ec} IS NOT NULL AND {ec}<>'' AND \
             ({ec} LIKE ?1 ESCAPE '\\' OR lower({name_col}) LIKE ?1 ESCAPE '\\' OR lower({name_col}) LIKE ?2 ESCAPE '\\') LIMIT ?3"
        )
    } else {
        return Ok(vec![]);
    };
    let Ok(mut st) = c.prepare(&sql) else { return Ok(vec![]) };
    let rows = st.query_map(params![prefix, word, limit as i64], |r| {
        Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<String>>(1)?.unwrap_or_default()))
    });
    let Ok(rows) = rows else { return Ok(vec![]) };
    Ok(rows
        .filter_map(Result::ok)
        .filter(|(_, e)| e.contains('@'))
        .map(|(name, email)| Suggestion { name, email, source: "contact" })
        .collect())
}

pub async fn suggest(ctx: Arc<AppCtx>, p: SuggestParams) -> anyhow::Result<Vec<Suggestion>> {
    let limit = p.limit.unwrap_or(10).clamp(1, 50);
    ctx.db.read(move |c| suggest_sync(c, &p.query, limit)).await
}

#[derive(Debug, Deserialize)]
pub struct ForgetParams {
    pub email: String,
}

/// Delete from the auto-complete list (Del on a highlighted suggestion, like Outlook).
pub async fn forget(ctx: Arc<AppCtx>, p: ForgetParams) -> anyhow::Result<()> {
    ctx.db
        .write(move |c| {
            c.execute("DELETE FROM recipient_cache WHERE email=lower(?1)", [p.email.trim()])?;
            Ok(())
        })
        .await
}

// ---- usual groups ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GroupParams {
    /// the people already addressed (To, or a meeting's attendees)
    pub emails: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Person {
    pub name: String,
    pub email: String,
}

/// Who usually goes with the people addressed so far: `to` (a meeting's required attendees) and
/// `cc` (optional attendees), each where they usually were.
#[derive(Debug, Default, Serialize, PartialEq)]
pub struct UsualGroup {
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
}

/// Messages (or meetings) with the same people it takes before a group is suggested.
const MIN_TIMES: usize = 3;
/// Half-life of a message's weight: a group that changed stops being suggested within months.
const HALF_LIFE_DAYS: f64 = 180.0;

/// One message you sent / meeting you organised: its To (required) and Cc (optional) addresses,
/// lower-case, with names.
struct Sent {
    at: i64,
    to: Vec<(String, String)>,
    cc: Vec<(String, String)>,
}

/// The people who were on most (half or more, by weight: recent mail counts more) of the messages
/// and meetings you sent to all of `emails`, and on at least `MIN_TIMES` of them; your own
/// addresses never. Each goes to `to` or `cc` by where they usually were.
pub fn usual_group_sync(c: &Connection, emails: &[String], now_ms: i64) -> anyhow::Result<UsualGroup> {
    let want: Vec<String> = emails.iter().map(|e| e.trim().to_lowercase()).filter(|e| e.contains('@')).collect();
    let Some(first) = want.first().cloned() else { return Ok(UsualGroup::default()) };
    let mut own: HashSet<String> = HashSet::new();
    for sql in ["SELECT lower(email) FROM accounts", "SELECT lower(email) FROM send_as"] {
        if let Ok(mut st) = c.prepare(sql) {
            for e in st.query_map([], |r| r.get::<_, String>(0))?.flatten() {
                own.insert(e);
            }
        }
    }
    let lower = |list: Vec<(String, String)>| list.into_iter().map(|(n, e)| (n, e.to_lowercase())).collect::<Vec<_>>();
    let pat = format!("%{}%", like_escape(&first));
    let mut items: Vec<Sent> = Vec::new();
    if table_exists(c, "messages") {
        let mut st = c.prepare(
            "SELECT internal_date, to_json, cc_json FROM messages WHERE (' ' || label_ids || ' ') LIKE '% SENT %' \
             AND lower(to_json || cc_json) LIKE ?1 ESCAPE '\\'",
        )?;
        for r in st.query_map([&pat], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
            let (at, to, cc) = r?;
            items.push(Sent { at, to: lower(crate::contacts::parse_addr_list(&to)), cc: lower(crate::contacts::parse_addr_list(&cc)) });
        }
    }
    if table_exists(c, "events") {
        // meetings you organised (a series once, not each occurrence)
        let mut st = c.prepare(
            "SELECT start_ms, attendees FROM events WHERE organizer_self=1 AND recurring_event_id IS NULL AND status!='cancelled' \
             AND lower(attendees) LIKE ?1 ESCAPE '\\'",
        )?;
        for r in st.query_map([&pat], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (at, raw) = r?;
            let list: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap_or_default();
            let (mut to, mut cc) = (Vec::new(), Vec::new());
            for a in list {
                let email = a["email"].as_str().unwrap_or("").to_lowercase();
                if !email.contains('@') || a["organizer"] == true || a["self"] == true || a["resource"] == true {
                    continue;
                }
                let name = a["displayName"].as_str().unwrap_or("").to_string();
                if a["optional"] == true { cc.push((name, email)) } else { to.push((name, email)) }
            }
            items.push(Sent { at: at.min(now_ms), to, cc });
        }
    }
    // only those with every one of the people addressed
    items.retain(|s| want.iter().all(|w| s.to.iter().chain(&s.cc).any(|(_, e)| e == w)));
    if items.len() < MIN_TIMES {
        return Ok(UsualGroup::default());
    }
    let weight = |at: i64| 0.5f64.powf(((now_ms - at).max(0) as f64 / 86_400_000.0) / HALF_LIFE_DAYS);
    let total: f64 = items.iter().map(|s| weight(s.at)).sum();
    // email → (name, latest, times, weight in To, weight in Cc)
    let mut seen: std::collections::HashMap<String, (String, i64, usize, f64, f64)> = std::collections::HashMap::new();
    for s in &items {
        let w = weight(s.at);
        let mut once: HashSet<&str> = HashSet::new();
        for (field, list) in [(0, &s.to), (1, &s.cc)] {
            for (name, email) in list {
                if want.contains(email) || own.contains(email) || !once.insert(email) {
                    continue;
                }
                let e = seen.entry(email.clone()).or_insert((String::new(), i64::MIN, 0, 0.0, 0.0));
                if !name.is_empty() && s.at >= e.1 {
                    e.0 = name.clone();
                    e.1 = s.at;
                }
                e.2 += 1;
                if field == 0 { e.3 += w } else { e.4 += w }
            }
        }
    }
    let mut picked: Vec<(String, (String, i64, usize, f64, f64))> =
        seen.into_iter().filter(|(_, v)| v.2 >= MIN_TIMES && v.3 + v.4 >= total * 0.5).collect();
    picked.sort_by(|a, b| (b.1.3 + b.1.4).total_cmp(&(a.1.3 + a.1.4)).then(a.0.cmp(&b.0)));
    let mut out = UsualGroup::default();
    for (email, (name, _, _, to_w, cc_w)) in picked.into_iter().take(15) {
        let p = Person { name, email };
        if cc_w > to_w { out.cc.push(p) } else { out.to.push(p) }
    }
    Ok(out)
}

pub async fn usual_group(ctx: Arc<AppCtx>, p: GroupParams) -> anyhow::Result<UsualGroup> {
    let now = crate::clock::now_ms();
    ctx.db.read(move |c| usual_group_sync(c, &p.emails, now)).await
}

#[cfg(test)]
mod group_tests {
    use super::*;
    use crate::accounts;

    const DAY: i64 = 86_400_000;

    fn addrs(list: &[&str]) -> String {
        serde_json::to_string(
            &list.iter().map(|e| serde_json::json!({ "name": e.split('@').next().unwrap(), "email": e })).collect::<Vec<_>>(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn suggests_the_people_usually_with_them_in_to_or_cc() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let acct = accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny Green", "demo").await.unwrap().id;
        let now = 1_800_000_000_000i64;
        ctx.db
            .write(move |c| {
                let mut n = 0;
                let mut sent = |to: &[&str], cc: &[&str], days_ago: i64| -> anyhow::Result<()> {
                    n += 1;
                    c.execute(
                        "INSERT INTO messages(account_id, gmail_id, thread_id, internal_date, sent_date, subject, from_name, from_email, to_json, cc_json, label_ids, body_state) \
                         VALUES (?1, ?2, ?2, ?3, ?3, 's', 'Kenny', 'kenny@duvz.demo', ?4, ?5, 'SENT', 1)",
                        params![acct, format!("g{n}"), now - days_ago * DAY, addrs(to), addrs(cc)],
                    )?;
                    Ok(())
                };
                // Yoav at Orbit: always with Dana (To) and the colleague Ron in Cc; Moshe once
                sent(&["yoav@orbit.example", "dana@orbit.example"], &["ron@duvz.demo"], 3)?;
                sent(&["yoav@orbit.example", "dana@orbit.example"], &["ron@duvz.demo", "kenny@duvz.demo"], 20)?;
                sent(&["yoav@orbit.example", "dana@orbit.example", "moshe@orbit.example"], &["ron@duvz.demo"], 40)?;
                sent(&["yoav@orbit.example"], &["ron@duvz.demo"], 60)?;
                // a different pair: Yoav with the CFO, twice only
                sent(&["yoav@orbit.example", "cfo@orbit.example"], &[], 5)?;
                sent(&["yoav@orbit.example", "cfo@orbit.example"], &[], 9)?;
                // mail received (not sent) doesn't count
                c.execute(
                    "INSERT INTO messages(account_id, gmail_id, thread_id, internal_date, sent_date, subject, from_name, from_email, to_json, cc_json, label_ids, body_state) \
                     VALUES (?1, 'r1', 'r1', ?2, ?2, 's', 'Yoav', 'yoav@orbit.example', ?3, '[]', 'INBOX', 1)",
                    params![acct, now - DAY, addrs(&["kenny@duvz.demo", "spam@x.example"])],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let g = ctx.db.read(move |c| usual_group_sync(c, &["Yoav@Orbit.example".into()], now)).await.unwrap();
        assert_eq!(g.to.iter().map(|p| p.email.as_str()).collect::<Vec<_>>(), ["dana@orbit.example"]);
        assert_eq!(g.cc.iter().map(|p| p.email.as_str()).collect::<Vec<_>>(), ["ron@duvz.demo"]);
        assert_eq!(g.to[0].name, "dana");
        // Yoav + the CFO: only twice together, not enough
        let g = ctx.db.read(move |c| usual_group_sync(c, &["yoav@orbit.example".into(), "cfo@orbit.example".into()], now)).await.unwrap();
        assert_eq!(g, UsualGroup::default());
        // nobody known
        let g = ctx.db.read(move |c| usual_group_sync(c, &["nobody@x.example".into()], now)).await.unwrap();
        assert_eq!(g, UsualGroup::default());
    }
}

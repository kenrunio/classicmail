//! Contacts domain: local contact store (Google People API), Outlook-style views, address
//! autocomplete for compose.

pub mod collate;
pub mod model;
pub mod schema;
pub mod sync;

#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter, types::Value as SqlValue};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{ctx::AppCtx, rpc::Registry, sync::pending, user_err};
use model::{COLUMNS, Contact};
use sync::{DOMAIN, DeleteOp, LocalOp};

pub fn register(r: &mut Registry) {
    r.add("contacts.list", list);
    r.add("contacts.get", get);
    r.add("contacts.create", create);
    r.add("contacts.update", update);
    r.add("contacts.delete", delete);
    r.add("contacts.move", move_to);
    r.add("contacts.setPhoto", set_photo);
    r.add("contacts.groups", groups);
    r.add("contacts.folders", folders);
    r.add("contacts.search", search);
    r.add("contacts.autocomplete", autocomplete);
    r.add("contacts.findByEmail", find_by_email);
}

/// After a local change: tell every window, and push to Google soon (not in tests, where sync
/// is driven explicitly).
fn changed(ctx: &Arc<AppCtx>, account_ids: impl IntoIterator<Item = i64>) {
    let ids: HashSet<i64> = account_ids.into_iter().collect();
    for id in ids {
        ctx.events.emit("contacts.changed", json!({ "accountId": id }));
        if ctx.sync.get().is_some() {
            tokio::spawn(sync::push_now(ctx.clone(), id));
        }
    }
}

/// Index-tab bucket of a "File as" value: "a".."z", "א" for Hebrew, "123" for the rest.
pub fn index_key(file_as: &str) -> String {
    match file_as.trim().chars().next() {
        Some(c) if c.is_ascii_alphabetic() => c.to_ascii_lowercase().to_string(),
        Some(c) if ('\u{0590}'..='\u{05FF}').contains(&c) => "א".into(),
        Some(c) if c.is_alphabetic() => {
            // accented Latin letters file under their base letter (É → e), as they sort
            let base = collate::fold_latin(c.to_lowercase().next().unwrap_or(c));
            if base.is_ascii_alphabetic() { base.to_string() } else { "123".into() }
        }
        _ => "123".into(),
    }
}

// ---- list / get ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ListParams {
    account_id: Option<i64>,
    group_id: Option<i64>,
    query: Option<String>,
    letter: Option<String>,
    /// fileAs (default) | firstName | lastName | company | added | modified
    sort_by: Option<String>,
    descending: bool,
}

fn group_resource(c: &Connection, group_id: i64) -> anyhow::Result<(i64, String)> {
    c.query_row("SELECT account_id, resource_name FROM contact_groups WHERE id=?1", [group_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?
        .ok_or_else(|| user_err!("not_found", "That contact group no longer exists."))
}

fn query_contacts(c: &Connection, p: &ListParams, limit: Option<usize>) -> anyhow::Result<Vec<Contact>> {
    let mut sql = format!("SELECT {COLUMNS} FROM contacts c WHERE 1=1");
    let mut args: Vec<SqlValue> = Vec::new();
    let mut account = p.account_id;
    if let Some(g) = p.group_id {
        let (acct, rn) = group_resource(c, g)?;
        account = Some(acct);
        sql.push_str(" AND EXISTS (SELECT 1 FROM json_each(c.groups) WHERE value = ?)");
        args.push(SqlValue::Text(rn));
    }
    if let Some(a) = account {
        sql.push_str(" AND c.account_id = ?");
        args.push(SqlValue::Integer(a));
    }
    if let Some(q) = p.query.as_deref().and_then(crate::db::util::fts_query) {
        sql.push_str(" AND c.id IN (SELECT rowid FROM contacts_fts WHERE contacts_fts MATCH ?)");
        args.push(SqlValue::Text(q));
    }
    sql.push_str(" ORDER BY c.file_as COLLATE NOCASE, c.display_name COLLATE NOCASE, c.id");
    if let Some(l) = limit {
        sql.push_str(&format!(" LIMIT {l}"));
    }
    let mut st = c.prepare(&sql)?;
    let mut rows = st.query_map(params_from_iter(args), model::from_row)?.collect::<Result<Vec<_>, _>>()?;
    if let Some(letter) = p.letter.as_deref().map(str::to_lowercase).filter(|l| !l.is_empty()) {
        rows.retain(|ct| index_key(&ct.file_as) == letter);
    }
    collate::sort_contacts(&mut rows, collate::SortKey::parse(p.sort_by.as_deref().unwrap_or("")), p.descending);
    Ok(rows)
}

async fn list(ctx: Arc<AppCtx>, p: ListParams) -> anyhow::Result<Vec<Contact>> {
    ctx.db.read(move |c| query_contacts(c, &p, None)).await
}

#[derive(Deserialize)]
struct IdParams {
    id: i64,
}

async fn get(ctx: Arc<AppCtx>, p: IdParams) -> anyhow::Result<Contact> {
    ctx.db
        .read(move |c| {
            model::get(c, p.id)?.ok_or_else(|| user_err!("not_found", "The contact no longer exists. It may have been deleted."))
        })
        .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchParams {
    query: String,
    limit: Option<usize>,
    account_id: Option<i64>,
}

async fn search(ctx: Arc<AppCtx>, p: SearchParams) -> anyhow::Result<Vec<Contact>> {
    let lp = ListParams { account_id: p.account_id, query: Some(p.query), ..Default::default() };
    if lp.query.as_deref().and_then(crate::db::util::fts_query).is_none() {
        return Ok(Vec::new());
    }
    let limit = p.limit.unwrap_or(200);
    ctx.db.read(move |c| query_contacts(c, &lp, Some(limit))).await
}

// ---- create / update / delete -------------------------------------------------------------------

fn validate(ct: &Contact) -> anyhow::Result<()> {
    for e in &ct.emails {
        let v = e.value.trim();
        let ok =
            v.split_once('@').map(|(a, b)| !a.is_empty() && b.contains('.') && !b.starts_with('.') && !b.ends_with('.')).unwrap_or(false)
                && !v.contains(char::is_whitespace)
                && v.matches('@').count() == 1;
        if !ok {
            return Err(user_err!("invalid_email", "The e-mail address \"{v}\" is not valid. Check the address and try again."));
        }
    }
    if ct.display_name.trim().is_empty() && ct.phones.is_empty() {
        return Err(user_err!("empty_contact", "Type a name, company or e-mail address for this contact before saving it."));
    }
    if let Some(b) = &ct.birthday {
        validate_date(b, "Birthday")?;
    }
    if let Some(b) = &ct.anniversary {
        validate_date(b, "Anniversary")?;
    }
    Ok(())
}

fn validate_date(s: &str, what: &str) -> anyhow::Result<()> {
    let ok = if let Some(md) = s.strip_prefix("--") {
        chrono::NaiveDate::parse_from_str(&format!("2000-{md}"), "%Y-%m-%d").is_ok()
    } else {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
    };
    if ok { Ok(()) } else { Err(user_err!("invalid_date", "{what} is not a valid date.")) }
}

async fn first_account(ctx: &AppCtx) -> anyhow::Result<i64> {
    crate::accounts::list(&ctx.db)
        .await?
        .into_iter()
        .find(|a| a.enabled)
        .map(|a| a.id)
        .ok_or_else(|| user_err!("no_account", "Add an e-mail account first (Tools › Account Settings)."))
}

/// Keep only groups that belong to the account.
fn filter_groups(c: &Connection, account_id: i64, groups: &mut Vec<String>) -> anyhow::Result<()> {
    let mut st = c.prepare("SELECT resource_name FROM contact_groups WHERE account_id=?1")?;
    let valid: HashSet<String> = st.query_map([account_id], |r| r.get(0))?.collect::<Result<_, _>>()?;
    groups.retain(|g| valid.contains(g));
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateParams {
    account_id: Option<i64>,
    contact: Contact,
}

async fn create(ctx: Arc<AppCtx>, p: CreateParams) -> anyhow::Result<Contact> {
    let account_id = match p.account_id {
        Some(a) => crate::accounts::get(&ctx.db, a).await?.id,
        None => first_account(&ctx).await?,
    };
    let mut ct = p.contact;
    ct.normalize();
    validate(&ct)?;
    let out = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            filter_groups(&tx, account_id, &mut ct.groups)?;
            let id = model::insert_local(&tx, account_id, &ct)?;
            pending::enqueue(&tx, account_id, DOMAIN, "create", &LocalOp { local_id: id })?;
            if ct.photo_url.as_deref().is_some_and(|u| u.starts_with("data:")) {
                pending::enqueue(&tx, account_id, DOMAIN, "photo", &LocalOp { local_id: id })?;
            }
            tx.commit()?;
            Ok(model::get(c, id)?.expect("just inserted"))
        })
        .await?;
    changed(&ctx, [account_id]);
    Ok(out)
}

#[derive(Deserialize)]
struct UpdateParams {
    id: i64,
    contact: Contact,
}

async fn update(ctx: Arc<AppCtx>, p: UpdateParams) -> anyhow::Result<Contact> {
    let mut ct = p.contact;
    ct.normalize();
    validate(&ct)?;
    let id = p.id;
    let out = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let cur =
                model::get(&tx, id)?.ok_or_else(|| user_err!("not_found", "The contact no longer exists. It may have been deleted."))?;
            filter_groups(&tx, cur.account_id, &mut ct.groups)?;
            model::update_local(&tx, id, &ct)?;
            pending::enqueue(&tx, cur.account_id, DOMAIN, "update", &LocalOp { local_id: id })?;
            tx.commit()?;
            Ok(model::get(c, id)?.expect("exists"))
        })
        .await?;
    changed(&ctx, [out.account_id]);
    Ok(out)
}

#[derive(Deserialize)]
struct IdsParams {
    ids: Vec<i64>,
}

fn delete_rows(c: &Connection, ids: &[i64]) -> anyhow::Result<Vec<i64>> {
    let mut accounts = Vec::new();
    for &id in ids {
        let row: Option<(i64, Option<String>)> =
            c.query_row("SELECT account_id, resource_name FROM contacts WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let Some((account_id, rn)) = row else { continue };
        c.execute("DELETE FROM contacts WHERE id=?1", [id])?;
        if let Some(rn) = rn {
            pending::enqueue(c, account_id, DOMAIN, "delete", &DeleteOp { resource_name: rn })?;
        }
        accounts.push(account_id);
    }
    Ok(accounts)
}

async fn delete(ctx: Arc<AppCtx>, p: IdsParams) -> anyhow::Result<usize> {
    let n = p.ids.len();
    let accounts = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let a = delete_rows(&tx, &p.ids)?;
            tx.commit()?;
            Ok(a)
        })
        .await?;
    changed(&ctx, accounts.clone());
    Ok(n.min(accounts.len()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveParams {
    ids: Vec<i64>,
    account_id: i64,
}

/// "Move to Folder" between accounts' Contacts folders: a copy in the target, then a delete.
async fn move_to(ctx: Arc<AppCtx>, p: MoveParams) -> anyhow::Result<Vec<i64>> {
    let target = crate::accounts::get(&ctx.db, p.account_id).await?.id;
    let (new_ids, touched) = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let mut new_ids = Vec::new();
            let mut touched = vec![target];
            for id in p.ids {
                let Some(mut ct) = model::get(&tx, id)? else { continue };
                if ct.account_id == target {
                    new_ids.push(id);
                    continue;
                }
                ct.groups.clear();
                let nid = model::insert_local(&tx, target, &ct)?;
                // a moved contact keeps its "Date Added"
                tx.execute("UPDATE contacts SET first_seen_at=?2 WHERE id=?1 AND ?2 > 0", params![nid, ct.first_seen_at])?;
                pending::enqueue(&tx, target, DOMAIN, "create", &LocalOp { local_id: nid })?;
                if ct.photo_url.as_deref().is_some_and(|u| u.starts_with("data:")) {
                    pending::enqueue(&tx, target, DOMAIN, "photo", &LocalOp { local_id: nid })?;
                }
                touched.extend(delete_rows(&tx, &[id])?);
                new_ids.push(nid);
            }
            tx.commit()?;
            Ok((new_ids, touched))
        })
        .await?;
    changed(&ctx, touched);
    Ok(new_ids)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PhotoParams {
    id: i64,
    /// `data:image/png;base64,...` (PNG, JPEG or GIF)
    data_url: String,
}

async fn set_photo(ctx: Arc<AppCtx>, p: PhotoParams) -> anyhow::Result<Contact> {
    let ok_type = ["data:image/png;base64,", "data:image/jpeg;base64,", "data:image/gif;base64,"].iter().any(|t| p.data_url.starts_with(t));
    if !ok_type {
        return Err(user_err!("bad_picture", "The picture must be a PNG, JPEG or GIF file."));
    }
    if p.data_url.len() > 4_000_000 {
        return Err(user_err!("bad_picture", "The picture is too large. Choose a picture smaller than 3 MB."));
    }
    let out = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let cur = model::get(&tx, p.id)?.ok_or_else(|| user_err!("not_found", "The contact no longer exists."))?;
            tx.execute("UPDATE contacts SET photo_url=?2 WHERE id=?1", params![p.id, p.data_url])?;
            pending::enqueue(&tx, cur.account_id, DOMAIN, "photo", &LocalOp { local_id: p.id })?;
            tx.commit()?;
            Ok(model::get(c, p.id)?.expect("exists"))
        })
        .await?;
    changed(&ctx, [out.account_id]);
    Ok(out)
}

// ---- folders & groups -----------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    id: i64,
    account_id: i64,
    resource_name: String,
    name: String,
    count: i64,
}

/// Contact groups with their member counts (all members counted in one pass over the contacts).
fn load_groups(c: &Connection, account_id: Option<i64>) -> anyhow::Result<Vec<Group>> {
    let mut st = c.prepare(
        "SELECT g.id, g.account_id, g.resource_name, g.name, COALESCE(n.members, 0) FROM contact_groups g \
         LEFT JOIN (SELECT c.account_id, j.value AS rn, COUNT(*) AS members FROM contacts c, json_each(c.groups) j \
                    WHERE (?1 IS NULL OR c.account_id = ?1) GROUP BY c.account_id, j.value) n \
           ON n.account_id = g.account_id AND n.rn = g.resource_name \
         WHERE (?1 IS NULL OR g.account_id = ?1) ORDER BY g.name COLLATE NOCASE",
    )?;
    let rows = st
        .query_map([account_id], |r| {
            Ok(Group { id: r.get(0)?, account_id: r.get(1)?, resource_name: r.get(2)?, name: r.get(3)?, count: r.get(4)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct GroupsParams {
    account_id: Option<i64>,
}

async fn groups(ctx: Arc<AppCtx>, p: GroupsParams) -> anyhow::Result<Vec<Group>> {
    ctx.db.read(move |c| load_groups(c, p.account_id)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    account_id: i64,
    email: String,
    display_name: String,
    color: String,
    count: i64,
    groups: Vec<Group>,
}

/// The "My Contacts" list: one Contacts folder per account, with its groups.
async fn folders(ctx: Arc<AppCtx>, _: crate::rpc::NoParams) -> anyhow::Result<Vec<Folder>> {
    let accounts = crate::accounts::list(&ctx.db).await?;
    ctx.db
        .read(move |c| {
            let mut out = Vec::new();
            for a in accounts.into_iter().filter(|a| a.enabled) {
                let count: i64 = c.query_row("SELECT COUNT(*) FROM contacts WHERE account_id=?1", [a.id], |r| r.get(0))?;
                out.push(Folder {
                    account_id: a.id,
                    email: a.email,
                    display_name: a.display_name,
                    color: a.color,
                    count,
                    groups: load_groups(c, Some(a.id))?,
                });
            }
            Ok(out)
        })
        .await
}

// ---- autocomplete / lookup ----------------------------------------------------------------------

#[derive(Deserialize)]
struct AutoParams {
    query: String,
    limit: Option<usize>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub name: String,
    pub email: String,
    pub source: &'static str,
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Does `name` have a word starting with `q` (case-insensitive)?
fn word_prefix(name: &str, q: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with(q) || n.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == '-' || c == '.').any(|w| w.starts_with(q))
}

pub fn autocomplete_sync(c: &Connection, query: &str, limit: usize) -> anyhow::Result<Vec<Suggestion>> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let mut out: Vec<Suggestion> = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |out: &mut Vec<Suggestion>, name: String, email: String, source: &'static str| {
        if out.len() < limit && seen.insert(email.to_lowercase()) {
            out.push(Suggestion { name, email, source });
        }
    };
    let pre = format!("{}%", like_escape(&q));
    let mid = format!("% {}%", like_escape(&q));
    // contacts: name / nickname / company word or e-mail prefix
    {
        let mut st = c.prepare(
            "SELECT c.display_name, j.value ->> 'value' FROM contacts c, json_each(c.emails) j \
             WHERE c.display_name LIKE ?1 ESCAPE '\\' OR c.display_name LIKE ?2 ESCAPE '\\' OR c.file_as LIKE ?1 ESCAPE '\\' \
                OR c.nickname LIKE ?1 ESCAPE '\\' OR c.company LIKE ?1 ESCAPE '\\' OR (j.value ->> 'value') LIKE ?1 ESCAPE '\\' \
             ORDER BY (c.display_name LIKE ?1 ESCAPE '\\') DESC, c.display_name COLLATE NOCASE, j.key LIMIT 200",
        )?;
        let rows = st.query_map(params![pre, mid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
        for r in rows {
            let (name, email) = r?;
            if let Some(email) = email.filter(|e| e.contains('@')) {
                push(&mut out, name, email, "contact");
            }
        }
    }
    // Google "other contacts"
    {
        let mut st = c.prepare(
            "SELECT name, email FROM other_contacts WHERE name LIKE ?1 ESCAPE '\\' OR name LIKE ?2 ESCAPE '\\' OR email LIKE ?1 ESCAPE '\\' \
             ORDER BY (name = '') , name COLLATE NOCASE LIMIT 200",
        )?;
        let rows = st.query_map(params![pre, mid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for r in rows {
            let (name, email) = r?;
            push(&mut out, name, email, "other");
        }
    }
    // recipients of recently sent mail (only when the mail tables exist)
    if out.len() < limit {
        match recent_recipients_cached(c) {
            Ok(list) => {
                for (name, email) in list.iter() {
                    if word_prefix(name, &q) || email.to_lowercase().starts_with(&q) {
                        push(&mut out, name.clone(), email.clone(), "recent");
                    }
                }
            }
            Err(e) => log::debug!("recent recipients unavailable: {}", crate::logging::err(&e)),
        }
    }
    Ok(out)
}

async fn autocomplete(ctx: Arc<AppCtx>, p: AutoParams) -> anyhow::Result<Vec<Suggestion>> {
    let limit = p.limit.unwrap_or(10).clamp(1, 100);
    ctx.db.read(move |c| autocomplete_sync(c, &p.query, limit)).await
}

fn table_exists(c: &Connection, table: &str) -> anyhow::Result<bool> {
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)", [table], |r| r.get(0))?)
}

/// The longest a computed `recent_recipients` list is reused (it is also recomputed as soon as
/// `recent_stamp` changes).
const RECENT_TTL: Duration = Duration::from_secs(60);

type RecentList = Arc<Vec<(String, String)>>;

/// A cheap fingerprint of what `recent_recipients` reads: the auto-complete list's size, total use
/// count and last use (sending mail bumps them, Delete on a suggestion removes one) and the newest
/// message id (sent mail arriving by sync). Other changes are picked up after `RECENT_TTL`.
fn recent_stamp(c: &Connection) -> anyhow::Result<(i64, i64, i64, i64)> {
    let (n, uses, last) = if table_exists(c, "recipient_cache")? {
        c.query_row("SELECT COUNT(*), COALESCE(SUM(use_count), 0), COALESCE(MAX(last_used), 0) FROM recipient_cache", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
    } else {
        (0, 0, 0)
    };
    let newest = if table_exists(c, "messages")? { c.query_row("SELECT COALESCE(MAX(id), 0) FROM messages", [], |r| r.get(0))? } else { 0 };
    Ok((n, uses, last, newest))
}

/// `recent_recipients`, reused (per database) while `recent_stamp` is unchanged, so typing in To /
/// Cc doesn't rescan sent mail on every key.
pub fn recent_recipients_cached(c: &Connection) -> anyhow::Result<RecentList> {
    struct Entry {
        stamp: (i64, i64, i64, i64),
        at: Instant,
        list: RecentList,
    }
    static CACHE: Mutex<Option<HashMap<String, Entry>>> = Mutex::new(None);
    let key = c.path().unwrap_or_default().to_string();
    let stamp = recent_stamp(c)?;
    if let Some(e) = CACHE.lock().as_ref().and_then(|m| m.get(&key))
        && e.stamp == stamp
        && e.at.elapsed() < RECENT_TTL
    {
        return Ok(e.list.clone());
    }
    let list = Arc::new(recent_recipients(c)?);
    CACHE.lock().get_or_insert_with(HashMap::new).insert(key, Entry { stamp, at: Instant::now(), list: list.clone() });
    Ok(list)
}

/// (name, e-mail) of people the user recently wrote to, best first: Outlook's auto-complete cache
/// (`recipient_cache`, kept by compose) and the recipients of recently sent mail (Gmail SENT label).
/// Mail owns those tables; if they don't exist yet this returns nothing.
pub fn recent_recipients(c: &Connection) -> anyhow::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if table_exists(c, "recipient_cache")? {
        let mut st = c.prepare("SELECT name, email FROM recipient_cache ORDER BY use_count DESC, last_used DESC LIMIT 2000")?;
        for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (name, email) = r?;
            if email.contains('@') && seen.insert(email.to_lowercase()) {
                out.push((name, email));
            }
        }
    }
    if table_exists(c, "messages")? {
        let mut st = c.prepare(
            "SELECT to_json, cc_json, bcc_json FROM messages WHERE (' ' || label_ids || ' ') LIKE '% SENT %'              ORDER BY internal_date DESC LIMIT 1000",
        )?;
        let rows = st.query_map([], |r| Ok([r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?]))?;
        for r in rows {
            for col in r? {
                for (name, email) in parse_addr_list(&col) {
                    if seen.insert(email.to_lowercase()) {
                        out.push((name, email));
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Parse a recipients value: a JSON array of `{name, email|address}` / strings, or an RFC 822
/// address list (`"Name" <a@b>, c@d`).
pub fn parse_addr_list(raw: &str) -> Vec<(String, String)> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Vec::new();
    }
    if raw.starts_with('[')
        && let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(raw)
    {
        return items
            .iter()
            .flat_map(|it| match it {
                serde_json::Value::String(s) => parse_addr_list(s),
                serde_json::Value::Object(o) => {
                    let email = o.get("email").or(o.get("address")).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                    let name = o.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                    if email.contains('@') { vec![(name, email)] } else { vec![] }
                }
                _ => vec![],
            })
            .collect();
    }
    // Outlook separates with ';': make those commas (outside quotes / angle brackets) for mail's parser
    let (mut commas, mut quoted, mut angle) = (String::with_capacity(raw.len()), false, false);
    for ch in raw.chars() {
        match ch {
            '"' => quoted = !quoted,
            '<' => angle = true,
            '>' => angle = false,
            ';' if !quoted && !angle => {
                commas.push(',');
                continue;
            }
            _ => {}
        }
        commas.push(ch);
    }
    crate::mail::mime::parse_addresses(&commas).into_iter().filter(|a| a.email.contains('@')).map(|a| (a.name, a.email)).collect()
}

#[derive(Deserialize)]
struct EmailParams {
    email: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactCard {
    id: i64,
    account_id: i64,
    display_name: String,
    company: String,
    job_title: String,
    phones: Vec<model::Typed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    photo_url: Option<String>,
}

/// Outlook's label for a Google phone type.
pub fn phone_label(kind: &str) -> &'static str {
    match kind {
        "work" => "Business",
        "home" => "Home",
        "mobile" => "Mobile",
        "workFax" => "Business Fax",
        "homeFax" => "Home Fax",
        "otherFax" => "Other Fax",
        "pager" | "workPager" => "Pager",
        "main" => "Company Main",
        "workMobile" => "Mobile",
        "assistant" => "Assistant",
        "car" => "Car",
        "callback" => "Callback",
        "isdn" => "ISDN",
        "radio" => "Radio",
        "telex" => "Telex",
        "ttyTdd" => "TTY/TDD",
        "googleVoice" => "Google Voice",
        _ => "Other",
    }
}

async fn find_by_email(ctx: Arc<AppCtx>, p: EmailParams) -> anyhow::Result<Option<ContactCard>> {
    let email = p.email.trim().to_lowercase();
    if email.is_empty() {
        return Ok(None);
    }
    ctx.db
        .read(move |c| {
            let id: Option<i64> = c
                .query_row(
                    "SELECT c.id FROM contacts c WHERE EXISTS (SELECT 1 FROM json_each(c.emails) j WHERE lower(j.value ->> 'value') = ?1) \
                     ORDER BY c.account_id, c.id LIMIT 1",
                    [&email],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(ct) = id.map(|id| model::get(c, id)).transpose()?.flatten() else { return Ok(None) };
            Ok(Some(ContactCard {
                id: ct.id,
                account_id: ct.account_id,
                display_name: ct.display_name,
                company: ct.company,
                job_title: ct.job_title,
                phones: ct.phones.into_iter().map(|p| model::Typed { kind: phone_label(&p.kind).to_string(), value: p.value }).collect(),
                photo_url: ct.photo_url,
            }))
        })
        .await
}

#[cfg(test)]
mod unit {
    use super::*;

    fn sent(c: &Connection, acct: i64, gid: &str, name: &str, email: &str) -> anyhow::Result<()> {
        let to = json!([{ "name": name, "email": email }]).to_string();
        c.execute(
            "INSERT INTO messages(account_id, gmail_id, internal_date, to_json, label_ids) VALUES (?1, ?2, 1, ?3, 'SENT')",
            params![acct, gid, to],
        )?;
        Ok(())
    }

    #[tokio::test]
    async fn recent_recipients_are_reused_until_mail_changes() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let acct = crate::accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny", "demo").await.unwrap().id;
        ctx.db
            .write(move |c| {
                c.execute(
                    "INSERT INTO recipient_cache(email, name, use_count, last_used) VALUES ('dana@x.example', 'Dana Cohen', 3, 10)",
                    [],
                )?;
                sent(c, acct, "s1", "Ruth Levi", "ruth@y.example")
            })
            .await
            .unwrap();
        let (first, again, direct, ruth) = ctx
            .db
            .read(|c| {
                Ok((recent_recipients_cached(c)?, recent_recipients_cached(c)?, recent_recipients(c)?, autocomplete_sync(c, "ru", 10)?))
            })
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first, &again), "nothing changed: the list is reused");
        assert_eq!(*first, direct, "the same list as computing it");
        assert_eq!(ruth, vec![Suggestion { name: "Ruth Levi".into(), email: "ruth@y.example".into(), source: "recent" }]);
        let suggest = |q: &'static str| {
            let ctx = ctx.clone();
            async move { ctx.db.read(move |c| autocomplete_sync(c, q, 10)).await.unwrap().into_iter().map(|s| s.email).collect::<Vec<_>>() }
        };
        // mail sent elsewhere arrives by sync
        ctx.db.write(move |c| sent(c, acct, "s2", "Noa Shapiro", "noa@z.example")).await.unwrap();
        assert_eq!(suggest("noa").await, ["noa@z.example"]);
        // sent from ClassicMail: the auto-complete list is bumped
        ctx.db
            .write(|c| {
                Ok(c.execute("INSERT INTO recipient_cache(email, name, use_count, last_used) VALUES ('avi@w.example', 'Avi', 1, 20)", [])?)
            })
            .await
            .unwrap();
        assert_eq!(suggest("avi").await, ["avi@w.example"]);
        // Delete on a suggestion
        assert_eq!(suggest("dana").await, ["dana@x.example"]);
        ctx.db.write(|c| Ok(c.execute("DELETE FROM recipient_cache WHERE email='dana@x.example'", [])?)).await.unwrap();
        assert!(suggest("dana").await.is_empty());
    }

    #[tokio::test]
    async fn group_counts_match_counting_each_group() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let a = crate::accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny", "demo").await.unwrap().id;
        let b = crate::accounts::upsert(&ctx.db, "info@duvz.demo", "Info", "demo").await.unwrap().id;
        ctx.db
            .write(move |c| {
                for (acct, rn, name) in
                    [(a, "contactGroups/1", "Clients"), (a, "contactGroups/2", "Press"), (b, "contactGroups/1", "Family")]
                {
                    c.execute("INSERT INTO contact_groups(account_id, resource_name, name) VALUES (?1, ?2, ?3)", params![acct, rn, name])?;
                }
                for (acct, groups) in [
                    (a, r#"["contactGroups/1"]"#),
                    (a, r#"["contactGroups/1","contactGroups/2"]"#),
                    (b, r#"["contactGroups/1"]"#),
                    (a, "[]"),
                ] {
                    c.execute("INSERT INTO contacts(account_id, display_name, groups) VALUES (?1, 'x', ?2)", params![acct, groups])?;
                }
                Ok(())
            })
            .await
            .unwrap();
        let (all, one, per_group) = ctx
            .db
            .read(move |c| {
                // the previous query: one count per group
                let mut st = c.prepare(
                    "SELECT g.id, (SELECT COUNT(*) FROM contacts c, json_each(c.groups) j WHERE c.account_id = g.account_id AND j.value = g.resource_name) \
                     FROM contact_groups g ORDER BY g.name COLLATE NOCASE",
                )?;
                let per_group = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok((load_groups(c, None)?, load_groups(c, Some(a))?, per_group))
            })
            .await
            .unwrap();
        assert_eq!(all.iter().map(|g| (g.id, g.count)).collect::<Vec<_>>(), per_group);
        assert_eq!(all.iter().map(|g| (g.name.as_str(), g.count)).collect::<Vec<_>>(), [("Clients", 2), ("Family", 1), ("Press", 1)]);
        assert_eq!(one.iter().map(|g| (g.name.as_str(), g.count)).collect::<Vec<_>>(), [("Clients", 2), ("Press", 1)]);
    }

    #[test]
    fn index_keys() {
        assert_eq!(index_key("Cohen, Sarah"), "c");
        assert_eq!(index_key("Élodie"), "e");
        assert_eq!(index_key("Łukasz"), "l", "files where it sorts");
        assert_eq!(index_key("כהן, משה"), "א");
        assert_eq!(index_key("3M Company"), "123");
        assert_eq!(index_key(""), "123");
    }

    #[test]
    fn parses_address_lists() {
        assert_eq!(
            parse_addr_list("\"Cohen, Sarah\" <sarah@x.example>, bob@y.example"),
            vec![("Cohen, Sarah".to_string(), "sarah@x.example".to_string()), (String::new(), "bob@y.example".to_string())]
        );
        assert_eq!(
            parse_addr_list("Dana <dana@x.example>; \"Levi; Dan\" <dan@y.example>"),
            vec![("Dana".to_string(), "dana@x.example".to_string()), ("Levi; Dan".to_string(), "dan@y.example".to_string())]
        );
        assert_eq!(
            parse_addr_list(r#"[{"name":"A","email":"a@x.example"},{"address":"b@x.example"}]"#),
            vec![("A".to_string(), "a@x.example".to_string()), (String::new(), "b@x.example".to_string())]
        );
    }
}

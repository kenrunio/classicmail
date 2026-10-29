//! Contacts sync: push pending local changes (create / update / delete / photo), then pull contact
//! groups, contacts (incremental with People API sync tokens; full resync when the token expired)
//! and "other contacts" (for address autocomplete).

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::model::{self, Contact};
use crate::{
    ctx::AppCtx,
    db::util::{get_sync_state, set_sync_state},
    google::{
        Api,
        client::api_status,
        people::{self, Person},
    },
    sync::pending::{self, PendingOp},
};

pub const DOMAIN: &str = "contacts";
const TOKEN_KEY: &str = "contacts.syncToken";
const OTHER_TOKEN_KEY: &str = "contacts.otherSyncToken";

/// Page size for list calls (small in tests so paging is exercised).
pub const PAGE_SIZE: usize = if cfg!(test) { 25 } else { 500 };

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOp {
    pub local_id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOp {
    pub resource_name: String,
}

/// One push/pull at a time per account (the sync worker and the "push right after an edit" task).
fn lock_for(account_id: i64) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: parking_lot::Mutex<Option<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>> = parking_lot::Mutex::new(None);
    let mut g = LOCKS.lock();
    g.get_or_insert_with(HashMap::new).entry(account_id).or_default().clone()
}

pub async fn sync_account(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<()> {
    let lock = lock_for(api.account_id);
    let _guard = lock.lock().await;
    let (pushed, dropped) = push_pending(ctx, api, true).await?;
    ctx.events.status(api.account_id, "sync", "Receiving contacts…", None);
    let mut changed = pushed;
    if dropped > 0 {
        reset_orphans(ctx, api.account_id).await?;
    }
    changed |= sync_groups(ctx, api).await?;
    changed |= sync_connections(ctx, api, dropped > 0).await?;
    match sync_other(ctx, api).await {
        Ok(c) => changed |= c,
        // other contacts need an extra scope; never fail the whole sync over them
        Err(e) => log::warn!("other contacts for account {}: {}", api.account_id, crate::logging::err(&e)),
    }
    if changed {
        ctx.events.emit("contacts.changed", json!({ "accountId": api.account_id }));
    }
    Ok(())
}

/// Push local changes now (called in the background right after an edit).
pub async fn push_now(ctx: Arc<AppCtx>, account_id: i64) {
    let res: anyhow::Result<()> = async {
        let api = ctx.api(account_id).await?;
        let lock = lock_for(account_id);
        let _guard = lock.lock().await;
        let (pushed, dropped) = push_pending(&ctx, &api, false).await?;
        if dropped > 0 {
            reset_orphans(&ctx, account_id).await?;
            sync_connections(&ctx, &api, true).await?;
        }
        if pushed {
            ctx.events.emit("contacts.changed", json!({ "accountId": account_id }));
        }
        Ok(())
    }
    .await;
    if let Err(e) = res {
        log::warn!("contacts push for account {account_id}: {}", crate::logging::err(&e));
    }
}

/// Returns (anything pushed, ops given up on).
/// `show_status`: only from the sync worker, which resets the status bar when its cycle ends.
async fn push_pending(ctx: &Arc<AppCtx>, api: &Api, show_status: bool) -> anyhow::Result<(bool, usize)> {
    let before = pending::count(&ctx.db, api.account_id, DOMAIN).await?;
    if before == 0 {
        return Ok((false, 0));
    }
    if show_status {
        ctx.events.status(api.account_id, "sync", "Sending contact changes…", None);
    }
    let dropped = pending::drain(&ctx.db, api.account_id, DOMAIN, |op| {
        let ctx = ctx.clone();
        let api = api.clone();
        async move { push_op(&ctx, &api, &op).await }
    })
    .await?;
    let after = pending::count(&ctx.db, api.account_id, DOMAIN).await?;
    Ok((after < before, dropped))
}

async fn push_op(ctx: &Arc<AppCtx>, api: &Api, op: &PendingOp) -> anyhow::Result<()> {
    match op.kind.as_str() {
        "create" | "update" => {
            let LocalOp { local_id } = op.payload()?;
            push_contact(ctx, api, local_id).await
        }
        "delete" => {
            let DeleteOp { resource_name } = op.payload()?;
            match people::delete_contact(api, &resource_name).await {
                Err(e) if api_status(&e) == Some(404) => Ok(()),
                other => other,
            }
        }
        "photo" => {
            let LocalOp { local_id } = op.payload()?;
            push_photo(ctx, api, local_id).await
        }
        other => {
            log::warn!("unknown contacts op {other}");
            Ok(())
        }
    }
}

/// Create or update one contact on Google from its current local state.
async fn push_contact(ctx: &Arc<AppCtx>, api: &Api, id: i64) -> anyhow::Result<()> {
    let Some((row, rev)) = ctx.db.read(move |c| Ok(model::get(c, id)?.zip(model::local_rev(c, id)?))).await? else {
        return Ok(()); // deleted locally before it was pushed
    };
    if !row.dirty && row.resource_name.is_some() {
        return Ok(()); // an earlier op already pushed this revision
    }
    let remote = match &row.resource_name {
        None => people::create_contact(api, &row.to_person(None)).await?,
        Some(rn) => {
            // Read Google's current copy first: only the fields the user changed are sent
            // (updatePersonFields replaces each listed field wholesale, and a real contact has
            // data ClassicMail doesn't model — other events, relations, other apps' client data).
            let mut attempt = 0;
            loop {
                attempt += 1;
                let current = match people::get_person(api, rn).await {
                    Ok(p) => p,
                    // deleted elsewhere: keep the user's edit by creating it again
                    Err(e) if api_status(&e) == Some(404) => break people::create_contact(api, &row.to_person(None)).await?,
                    Err(e) => return Err(e),
                };
                let (fields, body) = model::update_for(&row, &current);
                if fields.is_empty() {
                    break current;
                }
                match people::update_contact_fields(api, rn, &body, &fields).await {
                    Ok(p) => break p,
                    // changed again between our read and write: read once more (local edit wins)
                    Err(e) if people::is_stale_etag(&e) && attempt == 1 => continue,
                    Err(e) if api_status(&e) == Some(404) => break people::create_contact(api, &row.to_person(None)).await?,
                    Err(e) => return Err(e),
                }
            }
        }
    };
    let pushed = Contact::from_person(&remote);
    ctx.db.write(move |c| model::apply_pushed(c, id, rev, &pushed)).await?;
    Ok(())
}

async fn push_photo(ctx: &Arc<AppCtx>, api: &Api, id: i64) -> anyhow::Result<()> {
    use base64::Engine;
    let Some(row) = ctx.db.read(move |c| model::get(c, id)).await? else { return Ok(()) };
    let Some(rn) = row.resource_name.clone() else {
        anyhow::bail!("contact {id} is not on the server yet");
    };
    let Some(data) =
        row.photo_url.as_deref().and_then(|u| u.strip_prefix("data:")).and_then(|u| u.split_once(";base64,")).map(|(_, b)| b.to_string())
    else {
        return Ok(()); // photo was replaced by a server URL meanwhile
    };
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim())?;
    let p = people::update_contact_photo(api, &rn, &bytes).await?;
    let etag = p.etag.clone();
    let url = Contact::from_person(&p).photo_url;
    ctx.db
        .write(move |c| {
            c.execute(
                "UPDATE contacts SET etag=coalesce(?2, etag), photo_url=coalesce(?3, photo_url) WHERE id=?1",
                params![id, etag, url],
            )?;
            Ok(())
        })
        .await
}

/// After Google rejected a change for good: local edits with no op left are reverted to the
/// server's version on the next full sync, and contacts that never reached Google are removed.
async fn reset_orphans(ctx: &Arc<AppCtx>, account_id: i64) -> anyhow::Result<()> {
    ctx.db
        .write(move |c| {
            let live = "SELECT json_extract(payload, '$.localId') FROM pending_ops WHERE account_id=?1 AND domain='contacts'                         AND kind IN ('create','update','photo')";
            c.execute(
                &format!("UPDATE contacts SET dirty=0, etag=NULL WHERE account_id=?1 AND dirty=1 AND resource_name IS NOT NULL AND id NOT IN ({live})"),
                [account_id],
            )?;
            c.execute(&format!("DELETE FROM contacts WHERE account_id=?1 AND resource_name IS NULL AND id NOT IN ({live})"), [account_id])?;
            Ok(())
        })
        .await
}

// ---- pull -----------------------------------------------------------------------------------

async fn sync_groups(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<bool> {
    let mut groups = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let res = people::list_contact_groups(api, page.as_deref()).await?;
        groups.extend(res.contact_groups);
        match res.next_page_token {
            Some(t) => page = Some(t),
            None => break,
        }
    }
    let account_id = api.account_id;
    ctx.db
        .write(move |c| {
            let tx = c.transaction()?;
            let mut changed = false;
            let mut keep = HashSet::new();
            for g in groups.iter().filter(|g| g.group_type.as_deref() == Some("USER_CONTACT_GROUP")) {
                keep.insert(g.resource_name.clone());
                let name = g.formatted_name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| g.name.clone());
                let n = tx.execute(
                    "INSERT INTO contact_groups(account_id, resource_name, name, group_type) VALUES (?1, ?2, ?3, 'USER_CONTACT_GROUP') \
                     ON CONFLICT(account_id, resource_name) DO UPDATE SET name=excluded.name WHERE name<>excluded.name",
                    params![account_id, g.resource_name, name],
                )?;
                changed |= n > 0;
            }
            let existing: Vec<(i64, String)> = {
                let mut st = tx.prepare("SELECT id, resource_name FROM contact_groups WHERE account_id=?1")?;
                st.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
            };
            for (id, rn) in existing {
                if !keep.contains(&rn) {
                    tx.execute("DELETE FROM contact_groups WHERE id=?1", [id])?;
                    changed = true;
                }
            }
            tx.commit()?;
            Ok(changed)
        })
        .await
}

/// Resource names with a delete waiting to be pushed (don't resurrect them on pull).
async fn pending_deletes(ctx: &Arc<AppCtx>, account_id: i64) -> anyhow::Result<HashSet<String>> {
    ctx.db
        .read(move |c| {
            let mut st = c.prepare("SELECT payload FROM pending_ops WHERE account_id=?1 AND domain=?2 AND kind='delete'")?;
            let rows = st.query_map(params![account_id, DOMAIN], |r| r.get::<_, String>(0))?;
            let mut out = HashSet::new();
            for p in rows {
                if let Ok(op) = serde_json::from_str::<DeleteOp>(&p?) {
                    out.insert(op.resource_name);
                }
            }
            Ok(out)
        })
        .await
}

/// Pull contacts. Incremental with the stored sync token; a full listing when there is none, when
/// it expired (410 / EXPIRED_SYNC_TOKEN) or when `force_full`.
pub async fn sync_connections(ctx: &Arc<AppCtx>, api: &Api, force_full: bool) -> anyhow::Result<bool> {
    let account_id = api.account_id;
    let token = if force_full { None } else { ctx.db.read(move |c| get_sync_state(c, account_id, TOKEN_KEY)).await? };
    if let Some(token) = token {
        match pull_connections(ctx, api, Some(&token)).await {
            Ok(changed) => return Ok(changed),
            Err(e) if people::is_expired_sync_token(&e) || people::is_mismatched_sync_token(&e) => {
                log::info!("contacts sync token unusable for account {account_id} ({}); doing a full sync", crate::logging::err(&e));
                ctx.db.write(move |c| set_sync_state(c, account_id, TOKEN_KEY, None)).await?;
            }
            Err(e) => return Err(e),
        }
    }
    pull_connections(ctx, api, None).await
}

async fn pull_connections(ctx: &Arc<AppCtx>, api: &Api, sync_token: Option<&str>) -> anyhow::Result<bool> {
    let account_id = api.account_id;
    let full = sync_token.is_none();
    let mut persons: Vec<Person> = Vec::new();
    let mut page: Option<String> = None;
    let mut next_sync: Option<String> = None;
    loop {
        let res = people::list_connections(api, page.as_deref(), sync_token, PAGE_SIZE).await?;
        persons.extend(res.connections);
        if full {
            let total = res.total_people.or(res.total_items).unwrap_or(0).max(persons.len() as i64);
            if total > PAGE_SIZE as i64 {
                let p = persons.len() as f64 / total.max(1) as f64;
                ctx.events.status(account_id, "sync", &format!("Receiving contacts ({} of {})", persons.len(), total), Some(p));
            }
        }
        if res.next_sync_token.is_some() {
            next_sync = res.next_sync_token;
        }
        match res.next_page_token {
            Some(t) => page = Some(t),
            None => break,
        }
    }
    let skip = pending_deletes(ctx, account_id).await?;
    ctx.db
        .write(move |c| {
            let tx = c.transaction()?;
            let mut changed = false;
            let mut seen = HashSet::new();
            for p in &persons {
                let Some(rn) = p.resource_name.clone() else { continue };
                if p.is_deleted() {
                    changed |= tx.execute("DELETE FROM contacts WHERE account_id=?1 AND resource_name=?2", params![account_id, rn])? > 0;
                    continue;
                }
                seen.insert(rn.clone());
                if skip.contains(&rn) {
                    continue;
                }
                changed |= model::upsert_remote(&tx, account_id, &Contact::from_person(p))?;
            }
            if full {
                // anything we have that Google no longer lists was deleted (unless edited locally)
                let local: Vec<(i64, String)> = {
                    let mut st =
                        tx.prepare("SELECT id, resource_name FROM contacts WHERE account_id=?1 AND resource_name IS NOT NULL AND dirty=0")?;
                    st.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
                };
                for (id, rn) in local {
                    if !seen.contains(&rn) {
                        tx.execute("DELETE FROM contacts WHERE id=?1", [id])?;
                        changed = true;
                    }
                }
            }
            set_sync_state(&tx, account_id, TOKEN_KEY, next_sync.as_deref())?;
            tx.commit()?;
            Ok(changed)
        })
        .await
}

async fn sync_other(ctx: &Arc<AppCtx>, api: &Api) -> anyhow::Result<bool> {
    let account_id = api.account_id;
    let token = ctx.db.read(move |c| get_sync_state(c, account_id, OTHER_TOKEN_KEY)).await?;
    if let Some(t) = token {
        match pull_other(ctx, api, Some(&t)).await {
            Ok(c) => return Ok(c),
            Err(e) if people::is_expired_sync_token(&e) || people::is_mismatched_sync_token(&e) => {
                ctx.db.write(move |c| set_sync_state(c, account_id, OTHER_TOKEN_KEY, None)).await?;
            }
            Err(e) => return Err(e),
        }
    }
    pull_other(ctx, api, None).await
}

async fn pull_other(ctx: &Arc<AppCtx>, api: &Api, sync_token: Option<&str>) -> anyhow::Result<bool> {
    let account_id = api.account_id;
    let full = sync_token.is_none();
    let mut persons = Vec::new();
    let mut page: Option<String> = None;
    let mut next_sync = None;
    loop {
        let res = people::list_other_contacts(api, page.as_deref(), sync_token, PAGE_SIZE.max(100)).await?;
        persons.extend(res.other_contacts);
        if res.next_sync_token.is_some() {
            next_sync = res.next_sync_token;
        }
        match res.next_page_token {
            Some(t) => page = Some(t),
            None => break,
        }
    }
    ctx.db
        .write(move |c| {
            let tx = c.transaction()?;
            let before: i64 = tx.query_row("SELECT COUNT(*) FROM other_contacts WHERE account_id=?1", [account_id], |r| r.get(0))?;
            if full {
                tx.execute("DELETE FROM other_contacts WHERE account_id=?1", [account_id])?;
            }
            let mut changed = full && before > 0 && persons.is_empty();
            for p in &persons {
                let Some(rn) = p.resource_name.clone() else { continue };
                if !full {
                    tx.execute("DELETE FROM other_contacts WHERE account_id=?1 AND resource_name=?2", params![account_id, rn])?;
                    changed = true;
                }
                if p.is_deleted() {
                    continue;
                }
                let name = p.names.first().and_then(|n| n.display_name.clone().or(n.unstructured_name.clone())).unwrap_or_default();
                for e in &p.email_addresses {
                    if let Some(email) = e.value.as_deref().filter(|v| v.contains('@')) {
                        tx.execute(
                            "INSERT OR IGNORE INTO other_contacts(account_id, resource_name, name, email) VALUES (?1, ?2, ?3, ?4)",
                            params![account_id, rn, name, email],
                        )?;
                    }
                }
            }
            if full {
                let after: i64 = tx.query_row("SELECT COUNT(*) FROM other_contacts WHERE account_id=?1", [account_id], |r| r.get(0))?;
                changed |= after != before;
            }
            set_sync_state(&tx, account_id, OTHER_TOKEN_KEY, next_sync.as_deref())?;
            tx.commit()?;
            Ok(changed)
        })
        .await
}

//! The accounts table.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};

use rusqlite::{Connection, Row, params};
use serde::{Deserialize, Serialize};

use crate::{
    ctx::AppCtx,
    db::{
        Db,
        util::{get_setting, now_ms, set_setting},
    },
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: i64,
    pub email: String,
    pub display_name: String,
    pub provider: String,
    pub color: String,
    pub position: i64,
    pub enabled: bool,
}

fn from_row(r: &Row) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get("id")?,
        email: r.get("email")?,
        display_name: r.get("display_name")?,
        provider: r.get("provider")?,
        color: r.get("color")?,
        position: r.get("position")?,
        enabled: r.get::<_, i64>("enabled")? != 0,
    })
}

pub const COLORS: &[&str] = &["#4A7EBB", "#C0504D", "#9BBB59", "#8064A2", "#F79646", "#4BACC6"];

pub async fn list(db: &Db) -> anyhow::Result<Vec<Account>> {
    db.read(|c| {
        let mut st = c.prepare("SELECT * FROM accounts ORDER BY position, id")?;
        let rows = st.query_map([], from_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

pub async fn get(db: &Db, id: i64) -> anyhow::Result<Account> {
    db.read(move |c| {
        c.query_row("SELECT * FROM accounts WHERE id=?1", [id], from_row)
            .map_err(|_| crate::user_err!("not_found", "account {id} not found"))
    })
    .await
}

/// Insert (or return the existing) account with this e-mail.
pub async fn upsert(db: &Db, email: &str, display_name: &str, provider: &str) -> anyhow::Result<Account> {
    let email = email.to_lowercase();
    let display_name = display_name.to_string();
    let provider = provider.to_string();
    db.write(move |c| {
        let n: i64 = c.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
        c.execute(
            "INSERT INTO accounts(email, display_name, provider, color, position, created_at) VALUES (?1,?2,?3,?4,?5,?6) \
             ON CONFLICT(email) DO UPDATE SET display_name=excluded.display_name, provider=excluded.provider, enabled=1",
            params![email, display_name, provider, COLORS[(n as usize) % COLORS.len()], n, now_ms()],
        )?;
        Ok(c.query_row("SELECT * FROM accounts WHERE email=?1", [&email], from_row)?)
    })
    .await
}

/// Remove an account and everything kept for it on this computer: Google's access is revoked
/// (best effort), the sign-in leaves the keyring, its rows go (`remove`), then its cached
/// attachment files and the undo history (which can hold its messages' details).
pub async fn remove_everywhere(ctx: &Arc<AppCtx>, acct: &Account) -> anyhow::Result<()> {
    // its sync worker stops first, so nothing new is fetched for it while it goes
    if let Some(sync) = ctx.sync.get() {
        sync.stop(acct.id);
    }
    ctx.forget_api(acct.id);
    if acct.provider == "google" {
        revoke_google(ctx, acct).await;
        if let Err(e) = crate::google::auth::delete_refresh_token(&acct.email) {
            log::warn!("account {}: the saved sign-in couldn't be deleted: {}", acct.id, crate::logging::err(&e));
        }
    }
    let purged = remove(&ctx.db, acct.id).await?;
    let root = ctx.data_dir.join("attachments");
    let ids = purged.message_ids;
    let _ = tokio::task::spawn_blocking(move || {
        for id in ids {
            let _ = std::fs::remove_dir_all(root.join(id.to_string()));
        }
    })
    .await;
    for key in purged.settings {
        ctx.events.emit("settings.changed", serde_json::json!({ "key": key }));
    }
    crate::undo::clear(ctx);
    Ok(())
}

/// Ask Google to drop the app's access (the refresh token and the access tokens made from it).
/// Best effort: removing an account must work offline too.
async fn revoke_google(ctx: &AppCtx, acct: &Account) {
    let token = match crate::google::auth::load_refresh_token(&acct.email) {
        Ok(Some(t)) => t,
        Ok(None) => return,
        Err(e) => {
            log::warn!("account {}: the saved sign-in couldn't be read to revoke it: {}", acct.id, crate::logging::err(&e));
            return;
        }
    };
    let res = match ctx.google_urls().await {
        Ok(urls) => revoke(&ctx.http, &urls.token, &token).await,
        Err(e) => Err(e),
    };
    match res {
        Ok(()) => log::info!("account {}: Google access revoked", acct.id),
        Err(e) => log::warn!("account {}: Google access not revoked: {}", acct.id, crate::logging::err(&e)),
    }
}

/// POST the token to the revoke endpoint beside `token_url` (Google's
/// https://oauth2.googleapis.com/revoke, or the fake server's), so it only ever goes where it came
/// from. The error never includes the token.
pub async fn revoke(http: &reqwest::Client, token_url: &str, token: &str) -> anyhow::Result<()> {
    let Some(base) = token_url.strip_suffix("/token") else {
        anyhow::bail!("no revoke endpoint beside {token_url}");
    };
    let res = http
        .post(format!("{base}/revoke"))
        .form(&[("token", token)])
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("network error: {}", e.without_url()))?;
    if !res.status().is_success() {
        anyhow::bail!("HTTP {}", res.status().as_u16());
    }
    Ok(())
}

/// What `remove` took out: the account's message ids (for their cached attachment folders) and
/// the settings it changed.
pub struct Purged {
    pub message_ids: Vec<i64>,
    pub settings: Vec<&'static str>,
}

/// Delete the account's rows. Most go by ON DELETE CASCADE; the full-text index, the translation
/// cache, task reminder state and settings naming it don't, so they go first in the same
/// transaction. With `secure_delete` the freed pages are zeroed, and the checkpoint writes that
/// into the database file (no VACUUM: the database can be many GB).
pub async fn remove(db: &Db, id: i64) -> anyhow::Result<Purged> {
    db.write(move |c| {
        let was: i64 = c.pragma_query_value(None, "secure_delete", |r| r.get(0))?;
        c.pragma_update(None, "secure_delete", true)?;
        let out = purge(c, id);
        if let Err(e) = c.pragma_update(None, "secure_delete", was) {
            log::warn!("secure_delete not restored: {e}");
        }
        let out = out?;
        if let Err(e) = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())) {
            log::warn!("account {id} removed, but the checkpoint failed: {e}");
        }
        Ok(out)
    })
    .await
}

fn purge(c: &mut Connection, id: i64) -> anyhow::Result<Purged> {
    let tx = c.transaction()?;
    let message_ids = ids(&tx, "SELECT id FROM messages WHERE account_id=?1", id)?;
    let folders: HashSet<i64> = ids(&tx, "SELECT id FROM folders WHERE account_id=?1", id)?.into_iter().collect();
    {
        // contentless: no delete trigger (see store::delete_messages)
        let mut st = tx.prepare("DELETE FROM messages_fts WHERE rowid=?1")?;
        for m in &message_ids {
            st.execute([m])?;
        }
    }
    tx.execute("DELETE FROM ai_translations WHERE message_id IN (SELECT id FROM messages WHERE account_id=?1)", [id])?;
    tx.execute("DELETE FROM task_reminder_state WHERE task_id IN (SELECT id FROM tasks WHERE account_id=?1)", [id])?;
    let settings = forget_in_settings(&tx, id, &folders)?;
    tx.execute("DELETE FROM accounts WHERE id=?1", [id])?;
    tx.commit()?;
    Ok(Purged { message_ids, settings })
}

fn ids(c: &Connection, sql: &str, id: i64) -> anyhow::Result<Vec<i64>> {
    let mut st = c.prepare(sql)?;
    let rows = st.query_map([id], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Settings that name the account: the default account for new mail, and the mail tree's
/// favorites and folder order (node ids, src/modules/mail/folders.ts). Returns the keys changed.
fn forget_in_settings(c: &Connection, id: i64, folders: &HashSet<i64>) -> anyhow::Result<Vec<&'static str>> {
    let mut changed = Vec::new();
    if get_setting::<i64>(c, "mail.defaultAccountId").ok().flatten() == Some(id) {
        c.execute("DELETE FROM settings WHERE key='mail.defaultAccountId'", [])?;
        changed.push("mail.defaultAccountId");
    }
    let mine = |node: &String| is_accounts_node(node, id, folders);
    if let Some(favs) = get_setting::<Vec<String>>(c, "mail.favorites").ok().flatten()
        && favs.iter().any(mine)
    {
        let keep: Vec<String> = favs.into_iter().filter(|n| !mine(n)).collect();
        set_setting(c, "mail.favorites", &keep)?;
        changed.push("mail.favorites");
    }
    if let Some(order) = get_setting::<BTreeMap<String, Vec<String>>>(c, "mail.folderOrder").ok().flatten()
        && order.iter().any(|(parent, kids)| mine(parent) || kids.iter().any(mine))
    {
        let keep: BTreeMap<String, Vec<String>> = order
            .into_iter()
            .filter(|(parent, _)| !mine(parent))
            .map(|(parent, kids)| (parent, kids.into_iter().filter(|n| !mine(n)).collect()))
            .collect();
        set_setting(c, "mail.folderOrder", &keep)?;
        changed.push("mail.folderOrder");
    }
    Ok(changed)
}

/// Is this mail tree node the account's? `mb<account>`, `f<folder>`, `s<search folder>:<account>`.
fn is_accounts_node(node: &str, id: i64, folders: &HashSet<i64>) -> bool {
    if let Some(a) = node.strip_prefix("mb") {
        return a.parse() == Ok(id);
    }
    if let Some(f) = node.strip_prefix('f') {
        return f.parse().is_ok_and(|f: i64| folders.contains(&f));
    }
    node.starts_with('s') && node.rsplit_once(':').is_some_and(|(_, a)| a.parse() == Ok(id))
}

pub async fn update(db: &Db, id: i64, display_name: Option<String>, color: Option<String>, position: Option<i64>) -> anyhow::Result<()> {
    db.write(move |c| {
        if let Some(n) = display_name {
            c.execute("UPDATE accounts SET display_name=?1 WHERE id=?2", params![n, id])?;
        }
        if let Some(col) = color {
            c.execute("UPDATE accounts SET color=?1 WHERE id=?2", params![col, id])?;
        }
        if let Some(p) = position {
            c.execute("UPDATE accounts SET position=?1 WHERE id=?2", params![p, id])?;
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    /// Two accounts' worth of rows of every kind `remove` cleans up by hand; returns (folder, message, task) ids.
    fn seed(c: &Connection, a: i64, word: &str) -> anyhow::Result<(i64, i64, i64)> {
        c.execute("INSERT INTO folders(account_id, gmail_label_id, name, path, role) VALUES (?1,'INBOX','Inbox','INBOX','inbox')", [a])?;
        let f = c.last_insert_rowid();
        c.execute("INSERT INTO messages(account_id, gmail_id, internal_date, subject) VALUES (?1,'g1',1,?2)", params![a, word])?;
        let m = c.last_insert_rowid();
        crate::mail::store::fts_put(c, m, word, "sender", "me", &format!("{word} body"))?;
        c.execute(
            "INSERT INTO ai_translations(target, src_hash, translation, message_id, created_at) VALUES ('Hebrew',?1,'x',?2,1)",
            params![word, m],
        )?;
        c.execute("INSERT INTO task_lists(account_id, title) VALUES (?1,'Tasks')", [a])?;
        c.execute("INSERT INTO tasks(account_id, list_id, title) VALUES (?1,?2,?3)", params![a, c.last_insert_rowid(), word])?;
        let t = c.last_insert_rowid();
        c.execute("INSERT INTO task_reminder_state(task_id, reminder_at, dismissed) VALUES (?1,5,1)", [t])?;
        Ok((f, m, t))
    }

    #[tokio::test]
    async fn removing_an_account_leaves_nothing_of_it() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let gone = upsert(&ctx.db, "gone@duvz.demo", "Gone", "demo").await.unwrap().id;
        let kept = upsert(&ctx.db, "kept@duvz.demo", "Kept", "demo").await.unwrap().id;
        let ((_, gm, gt), (kf, km, kt)) = ctx
            .db
            .write(move |c| {
                let out = (seed(c, gone, "alphaword")?, seed(c, kept, "betaword")?);
                let (gf, kf) = (out.0.0, out.1.0);
                set_setting(c, "mail.defaultAccountId", &gone)?;
                set_setting(c, "mail.favorites", &json!([format!("f{gf}"), format!("sfollowup:{gone}"), format!("f{kf}"), format!("mb{kept}")]))?;
                set_setting(
                    c,
                    "mail.folderOrder",
                    &json!({ "root": [format!("mb{gone}"), format!("mb{kept}")], format!("mb{gone}"): [format!("f{gf}")], format!("mb{kept}"): [format!("f{kf}")] }),
                )?;
                Ok(out)
            })
            .await
            .unwrap();
        for m in [gm, km] {
            let dir = ctx.data_dir.join("attachments").join(m.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("1_report.pdf"), b"%PDF").unwrap();
        }
        crate::undo::push(&ctx, "Move 1 item to Investors".into(), vec![], vec![], Default::default()).await;
        let secure_before: i64 = ctx.db.write(|c| Ok(c.pragma_query_value(None, "secure_delete", |r| r.get(0))?)).await.unwrap();

        crate::rpc::build().call(ctx.clone(), "accounts.remove", json!({ "id": gone })).await.unwrap();

        let (fts, translations, reminders, secure_after) = ctx
            .db
            .write(move |c| {
                let fts = |w: &str| -> rusqlite::Result<Vec<i64>> {
                    c.prepare("SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?1")?.query_map([w], |r| r.get(0))?.collect()
                };
                let count = |sql: &str, id: i64| -> rusqlite::Result<i64> { c.query_row(sql, [id], |r| r.get(0)) };
                let tr = "SELECT COUNT(*) FROM ai_translations WHERE message_id=?1";
                let rs = "SELECT COUNT(*) FROM task_reminder_state WHERE task_id=?1";
                Ok((
                    (fts("alphaword")?, fts("betaword")?),
                    (count(tr, gm)?, count(tr, km)?),
                    (count(rs, gt)?, count(rs, kt)?),
                    c.pragma_query_value(None, "secure_delete", |r| r.get::<_, i64>(0))?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(fts, (vec![], vec![km]), "full-text rows");
        assert_eq!(translations, (0, 1), "translation cache");
        assert_eq!(reminders, (0, 1), "task reminder state");
        assert_eq!(secure_after, secure_before, "secure_delete is put back");
        assert!(!ctx.data_dir.join("attachments").join(gm.to_string()).exists(), "its cached attachments");
        assert!(ctx.data_dir.join("attachments").join(km.to_string()).join("1_report.pdf").exists());

        let (default, favs, order) = ctx
            .db
            .read(|c| {
                Ok((
                    get_setting::<i64>(c, "mail.defaultAccountId")?,
                    get_setting::<Value>(c, "mail.favorites")?,
                    get_setting::<Value>(c, "mail.folderOrder")?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(default, None);
        assert_eq!(favs, Some(json!([format!("f{kf}"), format!("mb{kept}")])));
        assert_eq!(order, Some(json!({ "root": [format!("mb{kept}")], format!("mb{kept}"): [format!("f{kf}")] })));
        assert_eq!(ctx.undo.peek().undo_count, 0, "undo history");
        let left: Vec<i64> = list(&ctx.db).await.unwrap().into_iter().map(|a| a.id).collect();
        assert_eq!(left, [kept]);
    }

    #[tokio::test]
    async fn a_google_account_goes_offline_too() {
        // no saved sign-in for it (nothing to revoke) and no OAuth client: removal still works
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let email = format!("removal-test-{}@example.invalid", uuid::Uuid::new_v4().simple());
        let id = upsert(&ctx.db, &email, "Test", "google").await.unwrap().id;
        crate::rpc::build().call(ctx.clone(), "accounts.remove", json!({ "id": id })).await.unwrap();
        assert!(list(&ctx.db).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn revoke_goes_beside_the_token_endpoint() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let token_url = crate::google::client::Urls::fake(&ctx.fake_base().await.unwrap()).token;
        revoke(&ctx.http, &token_url, "refresh-kenny@duvz.demo").await.unwrap();
        let e = revoke(&ctx.http, &token_url, "secret-token-value").await.unwrap_err().to_string();
        assert!(e.contains("HTTP 400") && !e.contains("secret-token-value"), "{e}");
        // no endpoint beside it: nothing is sent
        assert!(revoke(&ctx.http, "http://127.0.0.1:9/oauth", "refresh-x").await.is_err());
        assert_eq!(crate::google::client::Urls::google().token.replace("/token", "/revoke"), "https://oauth2.googleapis.com/revoke");
    }
}

//! Rules domain — Outlook 2007 "Rules and Alerts", run locally (design §7).
//!
//! * `model`  — rule JSON types + the pure matcher/planner (unit-tested).
//! * `store`  — the `rules` table.
//! * `engine` — loads message facts from the mail tables and executes actions via mail's rpc
//!   methods (`mail.move`, `mail.copy`, `mail.markRead`, `mail.setFlag`, `mail.delete`,
//!   `mail.compose.send`). "Mark as <importance>" is local only (Gmail has no importance).
//! * `gmail_filters` — the option to keep rules as Gmail filters too (so they work while
//!   ClassicMail is closed).
//! * Events: `rules.changed {}` after edits; `rules.alert { messages }` for "display a Desktop
//!   Alert" (also shown as a native notification); `rules.playSound { sound, messageId }`.

pub mod engine;
pub mod gmail_filters;
pub mod model;
pub mod sample;
pub mod schema;
pub mod store;

use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    ctx::AppCtx,
    rpc::{NoParams, Registry},
};
use engine::{RegistryPort, RunStats};
use model::{Action, Person, Rule};

pub fn register(r: &mut Registry) {
    r.add("rules.list", list);
    r.add("rules.gmailStatus", gmail_filters::status);
    r.add("rules.setGmailFilters", set_gmail_filters);
    r.add("rules.get", get);
    r.add("rules.save", save);
    r.add("rules.delete", delete);
    r.add("rules.reorder", reorder);
    r.add("rules.setEnabled", set_enabled);
    r.add("rules.runNow", run_now);
    r.add("rules.test", test_rule);
    r.add("rules.messageInfo", message_info);
    r.add("rules.folders", folders);
    r.add("rules.export", export);
    r.add("rules.import", import);
    if cfg!(debug_assertions) {
        r.add("rules.devSeedSample", dev_seed_sample);
    }
}

/// Called by mail sync after new Inbox messages are stored (see docs/contracts.md).
pub async fn on_new_messages(ctx: &Arc<AppCtx>, account_id: i64, message_ids: &[i64]) -> anyhow::Result<()> {
    if message_ids.is_empty() {
        return Ok(());
    }
    let rules: Vec<Rule> = ctx.db.read(store::list).await?.into_iter().filter(|r| r.enabled && r.applies_to_account(account_id)).collect();
    if rules.is_empty() {
        return Ok(());
    }
    let port = RegistryPort(ctx.clone());
    let st = engine::apply(ctx, &port, &rules, message_ids, Some(account_id)).await?;
    if st.matched > 0 || st.failed > 0 {
        log::info!("rules: {} new message(s): {} matched, {} action(s), {} failed", st.processed, st.matched, st.actions, st.failed);
    }
    Ok(())
}

fn changed(ctx: &Arc<AppCtx>) {
    ctx.events.emit("rules.changed", json!({}));
    // rules kept as Gmail filters follow the edit
    gmail_filters::reconcile_all_soon(ctx);
}

#[derive(Deserialize)]
struct OnParams {
    on: bool,
}

/// Rules and Alerts › "Also keep these rules in Gmail": on creates the filters, off removes the
/// ones ClassicMail made.
async fn set_gmail_filters(ctx: Arc<AppCtx>, p: OnParams) -> anyhow::Result<()> {
    ctx.db.write(move |c| crate::db::util::set_setting(c, gmail_filters::SETTING, &p.on)).await?;
    changed(&ctx);
    Ok(())
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ListParams {
    /// Only rules for this mailbox (plus rules for every mailbox).
    account_id: Option<i64>,
}

async fn list(ctx: Arc<AppCtx>, p: ListParams) -> anyhow::Result<Vec<Rule>> {
    let all = ctx.db.read(store::list).await?;
    Ok(match p.account_id {
        Some(a) => all.into_iter().filter(|r| r.applies_to_account(a)).collect(),
        None => all,
    })
}

#[derive(Deserialize)]
struct IdParam {
    id: i64,
}

async fn get(ctx: Arc<AppCtx>, p: IdParam) -> anyhow::Result<Rule> {
    ctx.db.read(move |c| store::get(c, p.id)).await
}

#[derive(Deserialize)]
struct SaveParams {
    rule: Rule,
}

#[derive(Serialize)]
struct IdResult {
    id: i64,
}

async fn save(ctx: Arc<AppCtx>, p: SaveParams) -> anyhow::Result<IdResult> {
    p.rule.validate().map_err(|m| crate::user_err!("invalid_rule", "{m}"))?;
    let id = ctx.db.write(move |c| store::save(c, &p.rule)).await?;
    changed(&ctx);
    Ok(IdResult { id })
}

#[derive(Deserialize)]
struct DeleteParams {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    ids: Vec<i64>,
}

async fn delete(ctx: Arc<AppCtx>, p: DeleteParams) -> anyhow::Result<usize> {
    let mut ids = p.ids;
    ids.extend(p.id);
    let n = ctx.db.write(move |c| store::delete(c, &ids)).await?;
    changed(&ctx);
    Ok(n)
}

#[derive(Deserialize)]
struct ReorderParams {
    ids: Vec<i64>,
}

async fn reorder(ctx: Arc<AppCtx>, p: ReorderParams) -> anyhow::Result<()> {
    ctx.db.write(move |c| store::reorder(c, &p.ids)).await?;
    changed(&ctx);
    Ok(())
}

#[derive(Deserialize)]
struct EnabledParams {
    id: i64,
    enabled: bool,
}

async fn set_enabled(ctx: Arc<AppCtx>, p: EnabledParams) -> anyhow::Result<()> {
    ctx.db.write(move |c| store::set_enabled(c, p.id, p.enabled)).await?;
    changed(&ctx);
    Ok(())
}

// ---- Run Rules Now --------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunNowParams {
    rule_ids: Vec<i64>,
    folder_id: i64,
    #[serde(default)]
    include_subfolders: bool,
    #[serde(default)]
    only_unread: bool,
}

/// Messages in a folder (optionally with subfolders / unread only), oldest first.
fn folder_messages(c: &Connection, folder_id: i64, subfolders: bool, only_unread: bool) -> anyhow::Result<(i64, Vec<i64>)> {
    let account: i64 = c
        .query_row("SELECT account_id FROM folders WHERE id=?1", [folder_id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| crate::user_err!("not_found", "folder {folder_id} not found"))?;
    let mut folders = vec![folder_id];
    if subfolders {
        let mut i = 0;
        while i < folders.len() {
            let mut st = c.prepare_cached("SELECT id FROM folders WHERE parent_id=?1")?;
            let kids = st.query_map([folders[i]], |r| r.get::<_, i64>(0))?.collect::<Result<Vec<_>, _>>()?;
            for k in kids {
                if !folders.contains(&k) {
                    folders.push(k);
                }
            }
            i += 1;
        }
    }
    let mut ids = Vec::new();
    // a message in several of the folders runs once (a set: big folders have many thousands)
    let mut seen = std::collections::HashSet::new();
    let sql = format!(
        "SELECT m.id FROM message_folders mf JOIN messages m ON m.id = mf.message_id WHERE mf.folder_id=?1{} ORDER BY m.internal_date",
        if only_unread { " AND m.is_read = 0" } else { "" }
    );
    let mut st = c.prepare(&sql)?;
    for f in folders {
        for id in st.query_map([f], |r| r.get::<_, i64>(0))? {
            let id = id?;
            if seen.insert(id) {
                ids.push(id);
            }
        }
    }
    Ok((account, ids))
}

/// "Run Rules Now": the selected rules (even if turned off), in list order, over a folder.
async fn run_now(ctx: Arc<AppCtx>, p: RunNowParams) -> anyhow::Result<RunStats> {
    let wanted = p.rule_ids.clone();
    let rules: Vec<Rule> = ctx
        .db
        .read(store::list)
        .await?
        .into_iter()
        .filter(|r| wanted.contains(&r.id))
        .map(|mut r| {
            r.enabled = true;
            r
        })
        .collect();
    if rules.is_empty() {
        return Err(crate::user_err!("no_rules", "Select at least one rule to run."));
    }
    let (account, ids) = ctx.db.read(move |c| folder_messages(c, p.folder_id, p.include_subfolders, p.only_unread)).await?;
    let port = RegistryPort(ctx.clone());
    engine::apply(&ctx, &port, &rules, &ids, Some(account)).await
}

// ---- test a rule / prefill from a message ---------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestParams {
    rule: Rule,
    message_id: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TestResult {
    matches: bool,
    conditions: Vec<bool>,
    exceptions: Vec<bool>,
}

async fn test_rule(ctx: Arc<AppCtx>, p: TestParams) -> anyhow::Result<TestResult> {
    let id = p.message_id;
    let mut f = ctx
        .db
        .read(move |c| engine::load_facts(c, &[id]))
        .await?
        .pop()
        .ok_or_else(|| crate::user_err!("not_found", "message {id} not found"))?;
    if p.rule.needs_body() {
        engine::fill_body(&ctx, &RegistryPort(ctx.clone()), &mut f).await;
    }
    let mut rule = p.rule;
    rule.enabled = true;
    Ok(TestResult {
        matches: rule.matches(&f),
        conditions: rule.conditions.iter().map(|c| c.matches(&f)).collect(),
        exceptions: rule.exceptions.iter().map(|c| c.matches(&f)).collect(),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageParam {
    message_id: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MessageInfo {
    id: i64,
    account_id: i64,
    from: Person,
    to: Vec<Person>,
    cc: Vec<Person>,
    subject: String,
    /// the folder the message is in (Inbox when it is there) — "Run this rule now on messages already in …"
    folder_id: Option<i64>,
    folder_name: Option<String>,
}

/// What "Create Rule…" needs from a message.
async fn message_info(ctx: Arc<AppCtx>, p: MessageParam) -> anyhow::Result<MessageInfo> {
    let id = p.message_id;
    ctx.db
        .read(move |c| {
            let f = engine::load_facts(c, &[id])?.pop().ok_or_else(|| crate::user_err!("not_found", "message {id} not found"))?;
            let folder: Option<(i64, String)> = c
                .query_row(
                    "SELECT f.id, f.name FROM message_folders mf JOIN folders f ON f.id = mf.folder_id \
                     WHERE mf.message_id=?1 ORDER BY (f.role = 'inbox') DESC, (f.role = 'user') DESC LIMIT 1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            Ok(MessageInfo {
                id,
                account_id: f.account_id,
                from: Person { name: f.from_name, email: f.from_email },
                to: f.to,
                cc: f.cc,
                subject: f.subject,
                folder_id: folder.as_ref().map(|x| x.0),
                folder_name: folder.map(|x| x.1),
            })
        })
        .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderDto {
    id: i64,
    account_id: i64,
    name: String,
    path: String,
    role: String,
    parent_id: Option<i64>,
}

/// Fallback folder list straight from the mail tables (the UI prefers `mail.listFolders`).
async fn folders(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<Vec<FolderDto>> {
    ctx.db
        .read(|c| {
            let mut st = c.prepare(
                "SELECT id, account_id, name, path, role, parent_id FROM folders WHERE hidden = 0 ORDER BY account_id, position, name",
            )?;
            let rows = st
                .query_map([], |r| {
                    Ok(FolderDto {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        name: r.get(2)?,
                        path: r.get(3)?,
                        role: r.get(4)?,
                        parent_id: r.get(5)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
}

// ---- export / import ------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportedRule {
    #[serde(flatten)]
    rule: Rule,
    /// mailbox the rule belongs to (ids differ between machines)
    #[serde(default)]
    account_email: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile {
    format: String,
    version: i64,
    #[serde(default)]
    exported_at: i64,
    rules: Vec<ExportedRule>,
}

const EXPORT_FORMAT: &str = "classicmail-rules";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportParams {
    path: String,
    #[serde(default)]
    rule_ids: Option<Vec<i64>>,
}

async fn export(ctx: Arc<AppCtx>, p: ExportParams) -> anyhow::Result<usize> {
    let file = ctx
        .db
        .read(move |c| {
            let mut out = Vec::new();
            for r in store::list(c)? {
                if p.rule_ids.as_ref().is_some_and(|ids| !ids.contains(&r.id)) {
                    continue;
                }
                let account_email: Option<String> = match r.account_id {
                    Some(a) => c.query_row("SELECT email FROM accounts WHERE id=?1", [a], |x| x.get(0)).optional()?,
                    None => None,
                };
                out.push(ExportedRule { rule: r, account_email });
            }
            Ok(ExportFile { format: EXPORT_FORMAT.into(), version: 1, exported_at: crate::db::util::now_ms(), rules: out })
        })
        .await?;
    let n = file.rules.len();
    let text = serde_json::to_string_pretty(&file)?;
    tokio::fs::write(&p.path, text).await.map_err(|e| crate::user_err!("io", "Couldn't write {}: {e}", p.path))?;
    Ok(n)
}

#[derive(Deserialize)]
struct ImportParams {
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportResult {
    imported: usize,
    /// rules imported turned off because a folder or mailbox couldn't be matched
    disabled: Vec<String>,
    /// rules that forward mail, imported turned off so the user reviews where mail would go:
    /// "Rule name → a@x.example, b@y.example"
    forwards: Vec<String>,
    /// rules not imported because they are invalid ("Rule name: why")
    skipped: Vec<String>,
}

/// Map an exported rule onto this machine: mailbox by e-mail, folders by path.
fn adapt_import(c: &Connection, er: ExportedRule) -> anyhow::Result<(Rule, bool)> {
    let mut r = er.rule;
    let mut ok = true;
    r.id = 0;
    r.account_id = match &er.account_email {
        Some(e) => {
            let a: Option<i64> = c.query_row("SELECT id FROM accounts WHERE email=?1", [e.to_lowercase()], |x| x.get(0)).optional()?;
            if a.is_none() {
                ok = false;
            }
            a
        }
        None => None,
    };
    for a in r.actions.iter_mut() {
        if let Action::MoveToFolder { folder_id, folder_path } | Action::CopyToFolder { folder_id, folder_path } = a {
            let by_path: Option<i64> = if folder_path.is_empty() {
                None
            } else {
                match r.account_id {
                    Some(acct) => c
                        .query_row("SELECT id FROM folders WHERE account_id=?1 AND path=?2", params![acct, &*folder_path], |x| x.get(0))
                        .optional()?,
                    None => c
                        .query_row("SELECT id FROM folders WHERE path=?1 ORDER BY account_id LIMIT 1", [&*folder_path], |x| x.get(0))
                        .optional()?,
                }
            };
            match by_path {
                Some(id) => *folder_id = id,
                None => {
                    // an id from another machine would point at some unrelated local folder
                    *folder_id = 0;
                    ok = false;
                }
            }
        }
    }
    if !ok {
        r.enabled = false;
    }
    Ok((r, ok))
}

async fn import(ctx: Arc<AppCtx>, p: ImportParams) -> anyhow::Result<ImportResult> {
    let text = tokio::fs::read_to_string(&p.path).await.map_err(|e| crate::user_err!("io", "Couldn't read {}: {e}", p.path))?;
    let file: ExportFile =
        serde_json::from_str(&text).map_err(|e| crate::user_err!("bad_file", "This isn't a ClassicMail rules file ({e})."))?;
    if file.format != EXPORT_FORMAT {
        return Err(crate::user_err!("bad_file", "This isn't a ClassicMail rules file."));
    }
    let res = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let mut out = ImportResult { imported: 0, disabled: vec![], forwards: vec![], skipped: vec![] };
            for er in file.rules {
                let (mut r, ok) = adapt_import(&tx, er)?;
                // the same checks as saving in the wizard (an unmatched folder only disables it)
                let mut probe = r.clone();
                for a in probe.actions.iter_mut() {
                    if let Action::MoveToFolder { folder_id, .. } | Action::CopyToFolder { folder_id, .. } = a
                        && *folder_id <= 0
                    {
                        *folder_id = 1;
                    }
                }
                if let Err(why) = probe.validate() {
                    out.skipped.push(format!("{}: {why}", r.name.trim()));
                    continue;
                }
                if !ok {
                    out.disabled.push(r.name.clone());
                }
                // a rules file from someone else must not start forwarding mail silently
                let targets: Vec<String> = r
                    .actions
                    .iter()
                    .filter_map(|a| match a {
                        Action::ForwardTo { people } => {
                            Some(people.iter().map(|p| p.email.trim().to_string()).collect::<Vec<_>>().join(", "))
                        }
                        _ => None,
                    })
                    .collect();
                if !targets.is_empty() {
                    r.enabled = false;
                    out.forwards.push(format!("{} → {}", r.name.trim(), targets.join(", ")));
                }
                store::save(&tx, &r)?;
                out.imported += 1;
            }
            tx.commit()?;
            Ok(out)
        })
        .await?;
    changed(&ctx);
    Ok(res)
}

/// Debug builds only: put a few sample messages into the first (demo) account so the Rules and
/// Rules window can be tried before mail sync exists.
async fn dev_seed_sample(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<Value> {
    let accts = crate::accounts::list(&ctx.db).await?;
    let mut out = Vec::new();
    for a in accts.into_iter().filter(|a| a.provider == "demo") {
        let id = a.id;
        out.push(ctx.db.write(move |c| sample::insert(c, id)).await?);
        ctx.events.mail_changed(id, None);
    }
    Ok(json!(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::model::Condition;

    async fn setup() -> (Arc<AppCtx>, crate::db::tempdir::TempDirGuard, sample::Sample) {
        let (ctx, g) = AppCtx::for_test().unwrap();
        let acct = crate::accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny Green", "demo").await.unwrap();
        let s = ctx.db.write(move |c| sample::insert(c, acct.id)).await.unwrap();
        (ctx, g, s)
    }

    #[tokio::test]
    async fn rpc_crud_reorder_export_import() {
        let (ctx, g, s) = setup().await;
        let r = crate::rpc::build();
        let mk = |name: &str| {
            json!({ "rule": { "name": name, "accountId": 1,
                "conditions": [{ "type": "from", "people": [{ "name": "Sarah Cohen", "email": "sarah.cohen@harborcap.example" }] }],
                "actions": [{ "type": "moveToFolder", "folderId": s.orion_folder, "folderPath": "Clients/Orion" }] } })
        };
        let a = r.call(ctx.clone(), "rules.save", mk("A")).await.unwrap()["id"].as_i64().unwrap();
        let b = r.call(ctx.clone(), "rules.save", mk("B")).await.unwrap()["id"].as_i64().unwrap();
        // invalid: no actions
        let bad = r.call(ctx.clone(), "rules.save", json!({ "rule": { "name": "x" } })).await.unwrap_err();
        assert!(bad.to_string().contains("action"));
        r.call(ctx.clone(), "rules.reorder", json!({ "ids": [b, a] })).await.unwrap();
        let l = r.call(ctx.clone(), "rules.list", json!({})).await.unwrap();
        assert_eq!(l[0]["name"], "B");
        assert_eq!(l[0]["actions"][0]["folderPath"], "Clients/Orion");
        r.call(ctx.clone(), "rules.setEnabled", json!({ "id": a, "enabled": false })).await.unwrap();
        assert_eq!(r.call(ctx.clone(), "rules.get", json!({ "id": a })).await.unwrap()["enabled"], false);
        assert!(ctx.events.captured().iter().any(|e| e["kind"] == "rules.changed"));

        // test against messages
        let rule = &l[0];
        let t = r.call(ctx.clone(), "rules.test", json!({ "rule": rule, "messageId": s.messages[0] })).await.unwrap();
        assert_eq!(t["matches"], true);
        let t = r.call(ctx.clone(), "rules.test", json!({ "rule": rule, "messageId": s.messages[1] })).await.unwrap();
        assert_eq!(t["matches"], false);

        // message info for "Create Rule"
        let mi = r.call(ctx.clone(), "rules.messageInfo", json!({ "messageId": s.messages[0] })).await.unwrap();
        assert_eq!(mi["from"]["email"], "sarah.cohen@harborcap.example");
        assert_eq!(mi["folderName"], "Inbox");

        // export → delete → import
        let path = g.path().join("rules.json");
        let n = r.call(ctx.clone(), "rules.export", json!({ "path": path.to_string_lossy() })).await.unwrap();
        assert_eq!(n, 2);
        r.call(ctx.clone(), "rules.delete", json!({ "ids": [a, b] })).await.unwrap();
        assert_eq!(r.call(ctx.clone(), "rules.list", json!({})).await.unwrap().as_array().unwrap().len(), 0);
        let res = r.call(ctx.clone(), "rules.import", json!({ "path": path.to_string_lossy() })).await.unwrap();
        assert_eq!(res["imported"], 2);
        assert_eq!(res["disabled"], json!([]));
        let l = r.call(ctx.clone(), "rules.list", json!({ "accountId": 1 })).await.unwrap();
        assert_eq!(l.as_array().unwrap().len(), 2);
        assert_eq!(l[0]["name"], "B");
        assert_eq!(l[0]["actions"][0]["folderId"], s.orion_folder);
        assert_eq!(l[1]["enabled"], false);
        // filtered list for another account excludes them
        assert_eq!(r.call(ctx.clone(), "rules.list", json!({ "accountId": 99 })).await.unwrap().as_array().unwrap().len(), 0);

        // a hostile rules file: forwarding is imported turned off and reported; invalid rules are
        // skipped; folder ids from another machine are never trusted
        let hostile = json!({ "format": "classicmail-rules", "version": 1, "rules": [
            { "name": "Copy everything out", "enabled": true, "conditions": [],
              "actions": [{ "type": "forwardTo", "people": [{ "name": "", "email": "drop@evil.example" }] }] },
            { "name": "Bad forward", "enabled": true, "actions": [{ "type": "forwardTo", "people": [{ "email": "not an address" }] }] },
            { "name": "No actions", "enabled": true, "actions": [] },
            { "name": "Move by id", "enabled": true, "accountEmail": "kenny@duvz.demo",
              "actions": [{ "type": "moveToFolder", "folderId": s.orion_folder, "folderPath": "No/Such/Folder" }] },
        ]});
        let path = g.path().join("hostile.json");
        std::fs::write(&path, hostile.to_string()).unwrap();
        let res = r.call(ctx.clone(), "rules.import", json!({ "path": path.to_string_lossy() })).await.unwrap();
        assert_eq!(res["imported"], 2, "{res}");
        assert_eq!(res["forwards"], json!(["Copy everything out → drop@evil.example"]));
        assert_eq!(res["skipped"].as_array().unwrap().len(), 2, "{res}");
        assert_eq!(res["disabled"], json!(["Move by id"]));
        let all = r.call(ctx.clone(), "rules.list", json!({})).await.unwrap();
        let fwd = all.as_array().unwrap().iter().find(|x| x["name"] == "Copy everything out").unwrap();
        assert_eq!(fwd["enabled"], false);
        let mv = all.as_array().unwrap().iter().find(|x| x["name"] == "Move by id").unwrap();
        assert_eq!(mv["enabled"], false);
        assert_eq!(mv["actions"][0]["folderId"], 0);
    }

    #[tokio::test]
    async fn run_now_reports_counts_without_mail() {
        let (ctx, _g, s) = setup().await;
        let r = crate::rpc::build();
        let id = r
            .call(
                ctx.clone(),
                "rules.save",
                json!({ "rule": { "name": "Harbor", "enabled": false,
                "conditions": [{ "type": "fromDomain", "domains": ["harborcap.example"] }],
                "actions": [{ "type": "markRead" }] } }),
            )
            .await
            .unwrap()["id"]
            .as_i64()
            .unwrap();
        let st = r.call(ctx.clone(), "rules.runNow", json!({ "ruleIds": [id], "folderId": s.inbox })).await.unwrap();
        // two Harbor messages in the Inbox; in this branch mail.markRead doesn't exist yet → failed
        assert_eq!(st["processed"], 4);
        assert_eq!(st["matched"], 2);
        assert_eq!(st["actions"].as_i64().unwrap() + st["failed"].as_i64().unwrap(), 2);
        let st = r
            .call(ctx.clone(), "rules.runNow", json!({ "ruleIds": [id], "folderId": s.clients_folder, "includeSubfolders": true }))
            .await
            .unwrap();
        assert_eq!(st["processed"], 0);
    }

    #[tokio::test]
    async fn folder_messages_lists_each_message_once() {
        let (ctx, _g, s) = setup().await;
        let m = s.messages.clone();
        let (acct, ids) = ctx
            .db
            .write(move |c| {
                for (id, f) in [
                    (m[0], s.clients_folder),
                    (m[1], s.clients_folder),
                    (m[0], s.orion_folder),
                    (m[1], s.orion_folder),
                    (m[2], s.orion_folder),
                ] {
                    c.execute("INSERT OR IGNORE INTO message_folders(message_id, folder_id, msg_date) VALUES (?1, ?2, 0)", params![id, f])?;
                }
                folder_messages(c, s.clients_folder, true, false)
            })
            .await
            .unwrap();
        assert_eq!(acct, 1);
        // Clients oldest first, then what only Orion has
        assert_eq!(ids, vec![s.messages[1], s.messages[0], s.messages[2]]);
    }

    #[tokio::test]
    async fn on_new_messages_honours_enabled_and_account() {
        let (ctx, _g, s) = setup().await;
        let other = crate::accounts::upsert(&ctx.db, "info@duvz.demo", "Duvz Software", "demo").await.unwrap().id;
        ctx.db
            .write(move |c| {
                store::save(
                    c,
                    &Rule {
                        id: 0,
                        name: "off".into(),
                        enabled: false,
                        position: 0,
                        account_id: None,
                        conditions: vec![Condition::HasAttachment],
                        exceptions: vec![],
                        actions: vec![Action::SetImportance { level: model::Importance::Low }],
                        created_at: 0,
                        updated_at: 0,
                    },
                )?;
                store::save(
                    c,
                    &Rule {
                        id: 0,
                        name: "other mailbox".into(),
                        enabled: true,
                        position: 0,
                        account_id: Some(other),
                        conditions: vec![],
                        exceptions: vec![],
                        actions: vec![Action::SetImportance { level: model::Importance::Low }],
                        created_at: 0,
                        updated_at: 0,
                    },
                )?;
                Ok(())
            })
            .await
            .unwrap();
        on_new_messages(&ctx, 1, &s.messages).await.unwrap();
        let imp: Vec<i64> = ctx
            .db
            .read(|c| {
                let mut st = c.prepare("SELECT importance FROM messages ORDER BY id")?;
                Ok(st.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?)
            })
            .await
            .unwrap();
        assert_eq!(imp, vec![2, 1, 1, 1, 1]);
        // turn the first on: only the message with an attachment changes
        ctx.db.write(|c| store::set_enabled(c, 1, true)).await.unwrap();
        on_new_messages(&ctx, 1, &s.messages).await.unwrap();
        let first: i64 =
            ctx.db.read(|c| Ok(c.query_row("SELECT importance FROM messages ORDER BY id LIMIT 1", [], |r| r.get(0))?)).await.unwrap();
        assert_eq!(first, 0);
    }
}

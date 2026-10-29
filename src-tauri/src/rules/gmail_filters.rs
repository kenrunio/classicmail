//! Rules on the Gmail server: with the option on (setting `rules.gmailFilters`), each rule Gmail
//! can express is also kept as a Gmail filter in every mailbox it applies to, so it works while
//! ClassicMail is closed (and for mail read on the phone). ClassicMail still runs every rule itself
//! when it's open: doing a filter's actions again is harmless, and what Gmail can't do (desktop
//! alerts, sounds, importance, forwarding, follow-up dates) only happens then.
//!
//! A rule becomes a filter when all its conditions and exceptions translate into Gmail search terms
//! and at least one action is a label change (move / copy to a folder, delete, mark read, flag =
//! star). ClassicMail only ever deletes filters it created itself (table `rule_filters`); filters
//! made in Gmail are left alone. Changing a rule replaces its filter (Gmail filters can't be
//! edited). Writing filters needs the `gmail.settings.basic` permission: mailboxes signed in
//! before it was asked for must sign in again (`needs_sign_in`).
//!
//! rpc: `rules.gmailStatus` → `{ enabled, rules: [{ id, gmail, reason? }], accounts: [{ id, email,
//! state: "ok" | "off" | "signIn" | "error", message?, filters }] }`

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};

use super::model::{Action, Condition, Person, Rule};
use crate::{
    ctx::AppCtx,
    db::util::get_setting,
    google::gmail::{Filter, FilterAction, FilterCriteria},
};

pub const SETTING: &str = "rules.gmailFilters";

// ---- rule → filter (pure) ---------------------------------------------------------------------

/// A search term for each address / name / domain ("" when there's nothing usable).
fn people_terms(people: &[Person]) -> Vec<String> {
    people
        .iter()
        .map(|p| if !p.email.trim().is_empty() { p.email.trim().to_string() } else { quoted(p.name.trim()) })
        .filter(|t| !t.is_empty() && t != "\"\"")
        .collect()
}

fn quoted(w: &str) -> String {
    let w = w.replace('"', "");
    if w.contains(char::is_whitespace) { format!("\"{w}\"") } else { w }
}

/// `field:(a OR b)`
fn any_of(field: &str, terms: &[String]) -> Option<String> {
    match terms {
        [] => None,
        [one] => Some(format!("{field}{one}")),
        many => Some(format!("{field}({})", many.join(" OR "))),
    }
}

/// One condition as Gmail search terms, or why Gmail can't check it.
fn condition_query(c: &Condition) -> Result<String, &'static str> {
    let words = |ws: &[String]| ws.iter().map(|w| quoted(w.trim())).filter(|w| !w.is_empty() && w != "\"\"").collect::<Vec<_>>();
    let none = "a condition has nothing to look for";
    match c {
        Condition::From { people } => any_of("from:", &people_terms(people)).ok_or(none),
        Condition::SentTo { people } => {
            let t = people_terms(people);
            match (any_of("to:", &t), any_of("cc:", &t)) {
                (Some(to), Some(cc)) => Ok(format!("{{{to} {cc}}}")),
                _ => Err(none),
            }
        }
        Condition::SubjectContains { words: ws } => any_of("subject:", &words(ws)).ok_or(none),
        Condition::SubjectOrBodyContains { words: ws } => any_of("", &words(ws)).ok_or(none),
        Condition::FromDomain { domains } => {
            let t: Vec<String> =
                domains.iter().map(|d| d.trim().trim_start_matches('@')).filter(|d| !d.is_empty()).map(|d| format!("@{d}")).collect();
            any_of("from:", &t).ok_or(none)
        }
        Condition::HasAttachment => Ok("has:attachment".into()),
        Condition::MyNameInTo => Ok("to:me".into()),
        Condition::MyNameNotInTo => Ok("-to:me".into()),
        Condition::MyNameInToOrCc => Ok("{to:me cc:me}".into()),
        Condition::BodyContains { .. } => Err("Gmail can't search the message body on its own (only subject and body together)"),
        Condition::HeaderContains { .. } => Err("Gmail can't search message headers"),
        Condition::Importance { .. } => Err("Gmail doesn't know a message's importance"),
        Condition::SentOnlyToMe => Err("Gmail can't check \"sent only to me\""),
        // handled by choosing the mailbox
        Condition::ThroughAccount { .. } => Ok(String::new()),
    }
}

/// Where a folder action points in this mailbox: the Gmail label id (None = no such folder here).
pub trait Labels {
    fn label_for(&self, folder_id: i64, folder_path: &str) -> Option<String>;
}

/// The Gmail filter for `rule` in `account_id`, or why it has none there.
pub fn to_filter(rule: &Rule, account_id: i64, labels: &impl Labels) -> Result<Filter, String> {
    if !rule.applies_to_account(account_id)
        || rule.conditions.iter().any(|c| matches!(c, Condition::ThroughAccount { account_id: a } if *a != account_id))
    {
        return Err("it isn't for this mailbox".into());
    }
    let mut terms = Vec::new();
    for c in &rule.conditions {
        let q = condition_query(c)?;
        if !q.is_empty() {
            terms.push(q);
        }
    }
    if terms.is_empty() {
        return Err("it applies to every message (Gmail filters need something to look for)".into());
    }
    let mut not = Vec::new();
    for c in &rule.exceptions {
        if matches!(c, Condition::ThroughAccount { .. }) {
            return Err("its exception depends on the mailbox".into());
        }
        let q = condition_query(c)?;
        if !q.is_empty() {
            not.push(q);
        }
    }
    let mut add: Vec<String> = Vec::new();
    let mut remove: Vec<String> = Vec::new();
    let push = |v: &mut Vec<String>, l: &str| {
        if !v.iter().any(|x| x == l) {
            v.push(l.to_string());
        }
    };
    for a in &rule.actions {
        match a {
            Action::MoveToFolder { folder_id, folder_path } => {
                let l = labels
                    .label_for(*folder_id, folder_path)
                    .ok_or_else(|| format!("the folder \"{folder_path}\" isn't in this mailbox"))?;
                push(&mut add, &l);
                push(&mut remove, "INBOX");
            }
            Action::CopyToFolder { folder_id, folder_path } => {
                let l = labels
                    .label_for(*folder_id, folder_path)
                    .ok_or_else(|| format!("the folder \"{folder_path}\" isn't in this mailbox"))?;
                push(&mut add, &l);
            }
            Action::Delete => push(&mut add, "TRASH"),
            Action::MarkRead => push(&mut remove, "UNREAD"),
            // a follow-up flag is a Gmail star (its date only when ClassicMail runs the rule)
            Action::Flag { flag } if flag != "none" && flag != "complete" => push(&mut add, "STARRED"),
            _ => {}
        }
    }
    if add.is_empty() && remove.is_empty() {
        return Err("Gmail can't do any of its actions (alerts, sounds, importance and forwarding only work in ClassicMail)".into());
    }
    let negated = match not.len() {
        0 => None,
        1 => Some(not.remove(0)),
        _ => Some(format!("{{{}}}", not.iter().map(|q| format!("({q})")).collect::<Vec<_>>().join(" "))),
    };
    Ok(Filter {
        id: None,
        criteria: FilterCriteria { query: Some(terms.join(" ")), negated_query: negated },
        action: FilterAction { add_label_ids: add, remove_label_ids: remove },
    })
}

/// Folder → label in one mailbox, as the rules engine resolves folders: the folder itself when it
/// belongs to this mailbox, else the folder with the same path here.
struct DbLabels {
    account_id: i64,
    by_id: HashMap<i64, (i64, String, Option<String>)>,
    by_path: HashMap<String, Option<String>>,
}

impl DbLabels {
    fn load(c: &Connection, account_id: i64) -> anyhow::Result<Self> {
        let mut st = c.prepare("SELECT id, account_id, path, gmail_label_id FROM folders")?;
        let rows =
            st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?)))?;
        let mut me = DbLabels { account_id, by_id: HashMap::new(), by_path: HashMap::new() };
        for row in rows {
            let (id, acct, path, label) = row?;
            if acct == account_id {
                me.by_path.insert(path.clone(), label.clone());
            }
            me.by_id.insert(id, (acct, path, label));
        }
        Ok(me)
    }
}

impl Labels for DbLabels {
    fn label_for(&self, folder_id: i64, folder_path: &str) -> Option<String> {
        match self.by_id.get(&folder_id) {
            Some((a, _, label)) if *a == self.account_id => label.clone(),
            Some((_, path, _)) => self.by_path.get(path).cloned().flatten(),
            None => self.by_path.get(folder_path).cloned().flatten(),
        }
    }
}

// ---- keeping Gmail in step ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct AccountState {
    /// "ok" | "signIn" | "error"
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    /// what was wanted when it failed (a failed state isn't retried for the same thing for a while)
    #[serde(skip)]
    wanted: String,
    #[serde(skip)]
    at: Option<Instant>,
}

static STATES: Mutex<Option<HashMap<i64, AccountState>>> = Mutex::new(None);

fn set_state(account_id: i64, s: AccountState) {
    STATES.lock().get_or_insert_with(HashMap::new).insert(account_id, s);
}

fn state_of(account_id: i64) -> Option<AccountState> {
    STATES.lock().as_ref().and_then(|m| m.get(&account_id).cloned())
}

const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);

/// The filters this mailbox should have: rule id → filter JSON.
fn wanted(c: &Connection, account_id: i64) -> anyhow::Result<Vec<(i64, String)>> {
    if !get_setting::<bool>(c, SETTING)?.unwrap_or(false) {
        return Ok(vec![]);
    }
    let labels = DbLabels::load(c, account_id)?;
    let mut out = Vec::new();
    for r in super::store::list(c)?.into_iter().filter(|r| r.enabled) {
        if let Ok(f) = to_filter(&r, account_id, &labels) {
            out.push((r.id, serde_json::to_string(&f)?));
        }
    }
    Ok(out)
}

/// Bring this mailbox's Gmail filters in line with the rules (no Gmail call when nothing changed).
/// `force` retries even if the same change failed recently.
pub async fn reconcile(ctx: &Arc<AppCtx>, account_id: i64, force: bool) -> anyhow::Result<()> {
    // one at a time: two quick edits must not both create the same filter
    static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = RUNNING.lock().await;
    let (want, have): (Vec<(i64, String)>, Vec<(i64, String, String)>) = ctx
        .db
        .read(move |c| {
            let mut st = c.prepare("SELECT rule_id, filter_id, spec FROM rule_filters WHERE account_id=?1")?;
            let have = st.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok((wanted(c, account_id)?, have))
        })
        .await?;
    let stale: Vec<(i64, String)> =
        have.iter().filter(|(rid, _, spec)| !want.iter().any(|(w, s)| w == rid && s == spec)).map(|(r, f, _)| (*r, f.clone())).collect();
    let missing: Vec<(i64, String)> =
        want.iter().filter(|(rid, spec)| !have.iter().any(|(h, _, s)| h == rid && s == spec)).cloned().collect();
    if stale.is_empty() && missing.is_empty() {
        if state_of(account_id).is_none_or(|s| s.state != "ok") {
            set_state(account_id, AccountState { state: "ok".into(), ..Default::default() });
        }
        return Ok(());
    }
    let signature = format!("{stale:?}{missing:?}");
    if !force
        && let Some(s) = state_of(account_id)
        && s.state != "ok"
        && s.wanted == signature
        && s.at.is_some_and(|t| t.elapsed() < RETRY_AFTER)
    {
        return Ok(());
    }
    let result = push(ctx, account_id, stale, missing).await;
    match &result {
        Ok(()) => set_state(account_id, AccountState { state: "ok".into(), ..Default::default() }),
        Err(e) => {
            let scope = e.downcast_ref::<crate::google::client::ApiError>().is_some_and(|a| {
                a.status() == Some(403)
                    && matches!(a.reason(), Some("insufficientPermissions" | "ACCESS_TOKEN_SCOPE_INSUFFICIENT" | "forbidden"))
            }) || e.to_string().contains("insufficient authentication scopes");
            set_state(
                account_id,
                AccountState {
                    state: if scope { "signIn" } else { "error" }.into(),
                    message: Some(if scope {
                        "Sign in to this mailbox again to let ClassicMail keep rules in Gmail (Tools › Account Settings).".into()
                    } else {
                        e.to_string()
                    }),
                    wanted: signature,
                    at: Some(Instant::now()),
                },
            );
        }
    }
    ctx.events.emit("rules.changed", json!({}));
    result
}

async fn push(ctx: &Arc<AppCtx>, account_id: i64, stale: Vec<(i64, String)>, missing: Vec<(i64, String)>) -> anyhow::Result<()> {
    let api = ctx.api(account_id).await?;
    for (rule_id, filter_id) in stale {
        match api.gmail_delete_filter(&filter_id).await {
            Ok(()) => {}
            // already gone (deleted in Gmail)
            Err(e) if e.downcast_ref::<crate::google::client::ApiError>().is_some_and(|a| a.is_not_found()) => {}
            Err(e) => return Err(e),
        }
        let fid = filter_id.clone();
        ctx.db
            .write(move |c| Ok(c.execute("DELETE FROM rule_filters WHERE account_id=?1 AND filter_id=?2", params![account_id, fid])?))
            .await?;
        log::info!("rules: removed Gmail filter for rule {rule_id} in account {account_id}");
    }
    for (rule_id, spec) in missing {
        let f: Filter = serde_json::from_str(&spec)?;
        let id = api.gmail_create_filter(&f).await?;
        ctx.db
            .write(move |c| {
                Ok(c.execute(
                    "INSERT INTO rule_filters(account_id, rule_id, filter_id, spec) VALUES (?1, ?2, ?3, ?4)",
                    params![account_id, rule_id, id, spec],
                )?)
            })
            .await?;
        log::info!("rules: created Gmail filter for rule {rule_id} in account {account_id}");
    }
    Ok(())
}

/// Every Google mailbox, in the background (after a rule or the option changed).
pub fn reconcile_all_soon(ctx: &Arc<AppCtx>) {
    let ctx = ctx.clone();
    tauri::async_runtime::spawn(async move {
        let Ok(accts) = crate::accounts::list(&ctx.db).await else { return };
        for a in accts.into_iter().filter(|a| a.enabled) {
            if let Err(e) = reconcile(&ctx, a.id, true).await {
                log::warn!("rules: Gmail filters for account {}: {}", a.id, crate::logging::err(&e));
            }
        }
    });
}

pub async fn status(ctx: Arc<AppCtx>, _: crate::rpc::NoParams) -> anyhow::Result<Value> {
    let accts: Vec<(i64, String)> =
        crate::accounts::list(&ctx.db).await?.into_iter().filter(|a| a.enabled).map(|a| (a.id, a.email)).collect();
    let ids: Vec<i64> = accts.iter().map(|a| a.0).collect();
    ctx.db
        .read(move |c| {
            let enabled = get_setting::<bool>(c, SETTING)?.unwrap_or(false);
            let labels: Vec<(i64, DbLabels)> = ids.iter().map(|a| Ok((*a, DbLabels::load(c, *a)?))).collect::<anyhow::Result<_>>()?;
            let rules: Vec<Value> = super::store::list(c)?
                .iter()
                .map(|r| {
                    let tries: Vec<Result<Filter, String>> =
                        labels.iter().filter(|(a, _)| r.applies_to_account(*a)).map(|(a, l)| to_filter(r, *a, l)).collect();
                    let gmail = tries.iter().any(Result::is_ok);
                    let reason = if gmail { None } else { tries.into_iter().find_map(Result::err) };
                    json!({ "id": r.id, "gmail": gmail, "reason": reason })
                })
                .collect();
            let accounts: Vec<Value> = accts
                .iter()
                .map(|(id, email)| {
                    let n: i64 = c
                        .query_row("SELECT COUNT(*) FROM rule_filters WHERE account_id=?1", [id], |r| r.get(0))
                        .optional()
                        .ok()
                        .flatten()
                        .unwrap_or(0);
                    let st = if enabled {
                        state_of(*id).unwrap_or(AccountState { state: "ok".into(), ..Default::default() })
                    } else {
                        AccountState { state: "off".into(), ..Default::default() }
                    };
                    json!({ "id": id, "email": email, "state": st.state, "message": st.message, "filters": n })
                })
                .collect();
            Ok(json!({ "enabled": enabled, "rules": rules, "accounts": accounts }))
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::model::{Action, Condition, Importance, Person};
    use serde_json::json;

    struct L;
    impl Labels for L {
        fn label_for(&self, folder_id: i64, _: &str) -> Option<String> {
            (folder_id == 7).then(|| "Label_7".to_string())
        }
    }

    fn rule(conditions: Vec<Condition>, exceptions: Vec<Condition>, actions: Vec<Action>) -> Rule {
        Rule {
            id: 1,
            name: "r".into(),
            enabled: true,
            position: 0,
            account_id: None,
            conditions,
            exceptions,
            actions,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn a_typical_rule_becomes_a_filter() {
        let r = rule(
            vec![
                Condition::From { people: vec![Person::email("sarah@x.com"), Person { name: "Noa Shapiro".into(), email: String::new() }] },
                Condition::SubjectContains { words: vec!["Q3 results".into(), "roadshow".into()] },
                Condition::HasAttachment,
            ],
            vec![Condition::FromDomain { domains: vec!["@spam.example".into()] }],
            vec![Action::MoveToFolder { folder_id: 7, folder_path: "Clients/Orion".into() }, Action::MarkRead, Action::DisplayDesktopAlert],
        );
        let f = to_filter(&r, 1, &L).unwrap();
        assert_eq!(
            f.criteria.query.as_deref(),
            Some("from:(sarah@x.com OR \"Noa Shapiro\") subject:(\"Q3 results\" OR roadshow) has:attachment")
        );
        assert_eq!(f.criteria.negated_query.as_deref(), Some("from:@spam.example"));
        assert_eq!(f.action.add_label_ids, vec!["Label_7"]);
        assert_eq!(f.action.remove_label_ids, vec!["INBOX", "UNREAD"]);
    }

    #[test]
    fn rules_gmail_cant_express_stay_local() {
        let mv = || vec![Action::MoveToFolder { folder_id: 7, folder_path: "X".into() }];
        assert!(
            to_filter(&rule(vec![Condition::BodyContains { words: vec!["x".into()] }], vec![], mv()), 1, &L).unwrap_err().contains("body")
        );
        assert!(to_filter(&rule(vec![Condition::Importance { level: Importance::High }], vec![], mv()), 1, &L).is_err());
        assert!(to_filter(&rule(vec![], vec![], mv()), 1, &L).unwrap_err().contains("every message"));
        // only local actions
        assert!(to_filter(&rule(vec![Condition::HasAttachment], vec![], vec![Action::DisplayDesktopAlert]), 1, &L).is_err());
        // a folder this mailbox doesn't have
        assert!(
            to_filter(
                &rule(vec![Condition::HasAttachment], vec![], vec![Action::MoveToFolder { folder_id: 9, folder_path: "Gone".into() }]),
                1,
                &L
            )
            .is_err()
        );
        // another mailbox's rule
        let mut r = rule(vec![Condition::HasAttachment], vec![], mv());
        r.account_id = Some(2);
        assert!(to_filter(&r, 1, &L).is_err());
        let r = rule(vec![Condition::HasAttachment, Condition::ThroughAccount { account_id: 2 }], vec![], mv());
        assert!(to_filter(&r, 1, &L).is_err() && to_filter(&r, 2, &L).is_ok());
    }

    /// The option on: Gmail gets the rule's filter; an edit replaces it; a filter made in Gmail is
    /// left alone; disabling the rule or the option removes ClassicMail's.
    #[tokio::test]
    async fn filters_follow_the_rules_in_gmail() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let acct = crate::accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny Green", "demo").await.unwrap().id;
        let s = ctx.db.write(move |c| crate::rules::sample::insert(c, acct)).await.unwrap();
        let api = ctx.api(acct).await.unwrap();
        let mine = crate::google::gmail::Filter {
            criteria: FilterCriteria { query: Some("from:boss@x.com".into()), negated_query: None },
            action: FilterAction { add_label_ids: vec!["STARRED".into()], remove_label_ids: vec![] },
            ..Default::default()
        };
        api.gmail_create_filter(&mine).await.unwrap();
        let r = crate::rpc::build();
        let save = |name: &str, word: &str| {
            json!({ "rule": { "name": name, "conditions": [{ "type": "subjectContains", "words": [word] }],
                "actions": [{ "type": "moveToFolder", "folderId": s.orion_folder, "folderPath": "Clients/Orion" }, { "type": "playSound" }] } })
        };
        let id = r.call(ctx.clone(), "rules.save", save("Orion", "Orion")).await.unwrap()["id"].as_i64().unwrap();
        // a rule Gmail can't express
        r.call(
            ctx.clone(),
            "rules.save",
            json!({ "rule": { "name": "Body", "conditions": [{ "type": "bodyContains", "words": ["x"] }],
            "actions": [{ "type": "markRead" }] } }),
        )
        .await
        .unwrap();
        let queries =
            || async { api.gmail_filters().await.unwrap().into_iter().map(|f| f.criteria.query.unwrap_or_default()).collect::<Vec<_>>() };
        reconcile(&ctx, acct, true).await.unwrap();
        assert_eq!(queries().await, vec!["from:boss@x.com"], "option off: nothing created");

        r.call(ctx.clone(), "rules.setGmailFilters", json!({ "on": true })).await.unwrap();
        reconcile(&ctx, acct, true).await.unwrap();
        assert_eq!(queries().await, vec!["from:boss@x.com", "subject:Orion"]);
        let st = r.call(ctx.clone(), "rules.gmailStatus", json!({})).await.unwrap();
        assert_eq!(st["enabled"], true);
        assert_eq!(st["rules"].as_array().unwrap().iter().filter(|x| x["gmail"] == true).count(), 1);
        assert!(st["rules"].as_array().unwrap().iter().any(|x| x["reason"].as_str().is_some_and(|r| r.contains("body"))));
        assert_eq!(st["accounts"][0]["filters"], 1);
        // nothing changed: no new filter
        reconcile(&ctx, acct, true).await.unwrap();
        assert_eq!(queries().await.len(), 2);

        r.call(
            ctx.clone(),
            "rules.save",
            json!({ "rule": { "id": id, "name": "Orion", "conditions": [{ "type": "subjectContains", "words": ["Orion Tech"] }],
            "actions": [{ "type": "moveToFolder", "folderId": s.orion_folder, "folderPath": "Clients/Orion" }] } }),
        )
        .await
        .unwrap();
        reconcile(&ctx, acct, true).await.unwrap();
        assert_eq!(queries().await, vec!["from:boss@x.com", "subject:\"Orion Tech\""]);

        r.call(ctx.clone(), "rules.setEnabled", json!({ "id": id, "enabled": false })).await.unwrap();
        reconcile(&ctx, acct, true).await.unwrap();
        assert_eq!(queries().await, vec!["from:boss@x.com"], "only Gmail's own filter is left");
        r.call(ctx.clone(), "rules.setEnabled", json!({ "id": id, "enabled": true })).await.unwrap();
        reconcile(&ctx, acct, true).await.unwrap();
        r.call(ctx.clone(), "rules.setGmailFilters", json!({ "on": false })).await.unwrap();
        reconcile(&ctx, acct, true).await.unwrap();
        assert_eq!(queries().await, vec!["from:boss@x.com"]);
    }

    #[test]
    fn sent_to_covers_cc_and_several_exceptions_are_any_of() {
        let r = rule(
            vec![Condition::SentTo { people: vec![Person::email("ir@x.com")] }],
            vec![Condition::HasAttachment, Condition::SubjectContains { words: vec!["fyi".into()] }],
            vec![Action::Delete, Action::Flag { flag: "today".into() }],
        );
        let f = to_filter(&r, 1, &L).unwrap();
        assert_eq!(f.criteria.query.as_deref(), Some("{to:ir@x.com cc:ir@x.com}"));
        assert_eq!(f.criteria.negated_query.as_deref(), Some("{(has:attachment) (subject:fyi)}"));
        assert_eq!(f.action.add_label_ids, vec!["TRASH", "STARRED"]);
    }
}

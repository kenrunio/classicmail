//! How to undo each reversible action, computed from the state BEFORE it (`prepare`) and its result
//! (`finish`). Not undoable by design (no entry): permanent deletes, Empty folder, sending mail,
//! meeting responses, deleting / editing meetings that notify attendees, a single deleted
//! occurrence of a series, account changes.
//!
//! Helper rpcs used as inverse ops: `undo.mailRestore` (put messages' labels + flag back),
//! `undo.deleteEmptyFolder` (remove a folder the user created, only while it is still empty).
//! A move / copy into another mailbox is undone with `mail.deleteImported` (the copies it made go)
//! plus `undo.mailRestore` for the originals; its redo runs the move again, and the new copies'
//! ids replace the old ones (`$moved`).

use std::{collections::HashMap, sync::Arc};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::Op;
use crate::{
    ctx::AppCtx,
    mail::{actions, store as mstore},
    rpc::Registry,
};

pub fn register(r: &mut Registry) {
    r.add("undo.mailRestore", mail_restore);
    r.add("undo.deleteEmptyFolder", delete_empty_folder);
}

pub fn is_helper(m: &str) -> bool {
    matches!(m, "undo.mailRestore" | "undo.deleteEmptyFolder")
}

/// What `finish` needs, captured before the action ran.
pub enum Prep {
    Mail {
        ids: Vec<i64>,
        before: HashMap<i64, MailState>,
    },
    /// mail.move / mail.copy: like `Mail`, plus the copies made in another mailbox
    MailMove {
        method: String,
        ids: Vec<i64>,
        before: HashMap<i64, MailState>,
    },
    /// inverse fully known up front; `result_var` = save the action's result under this name
    Fixed {
        undo: Vec<Op>,
        redo: Vec<Op>,
        result_var: Option<String>,
    },
    CalUpdate {
        target: i64,
        series: bool,
        with_rec: bool,
        before: Value,
    },
    CalMove {
        before: Value,
        dest: i64,
    },
    ContactUpdate {
        id: i64,
        before: Value,
    },
    ContactMove {
        target: i64,
        before_accounts: Vec<(i64, i64)>,
    },
    TaskUpdate {
        ids: Vec<i64>,
        before: Vec<Value>,
    },
    TaskMove {
        ids: Vec<i64>,
        before: Vec<TaskPlace>,
    },
}

pub struct Recorded {
    pub undo: Vec<Op>,
    pub redo: Vec<Op>,
    pub vars: Map<String, Value>,
}

async fn call(ctx: &Arc<AppCtx>, method: &str, params: Value) -> anyhow::Result<Value> {
    super::registry().call(ctx.clone(), method, params).await
}

fn ids_of(p: &Value) -> Vec<i64> {
    p.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default()
}

fn i64_of(p: &Value, k: &str) -> Option<i64> {
    p.get(k).and_then(Value::as_i64)
}

pub async fn prepare(ctx: &Arc<AppCtx>, method: &str, p: &Value) -> anyhow::Result<Option<Prep>> {
    Ok(match method {
        // ---- mail ----
        "mail.move" | "mail.copy" => {
            let ids = ids_of(p);
            Some(Prep::MailMove { method: method.to_string(), before: mail_states(ctx, ids.clone()).await?, ids })
        }
        "mail.archive" | "mail.markRead" | "mail.setFlag" | "mail.markJunk" | "mail.notJunk" | "mail.restore" => {
            let ids = ids_of(p);
            Some(Prep::Mail { before: mail_states(ctx, ids.clone()).await?, ids })
        }
        "mail.delete" => {
            let ids = ids_of(p);
            let before = mail_states(ctx, ids.clone()).await?;
            // anything already in Deleted Items goes for good: not undoable
            if before.values().any(|s| s.labels.iter().any(|l| l == "TRASH")) {
                return Ok(None);
            }
            Some(Prep::Mail { ids, before })
        }
        "mail.markAllRead" => {
            let folder = i64_of(p, "folderId").unwrap_or_default();
            let ids = actions::folder_message_ids(ctx, folder, true).await?;
            Some(Prep::Mail { before: mail_states(ctx, ids.clone()).await?, ids })
        }
        "mail.createFolder" => Some(Prep::Fixed {
            undo: vec![Op::new("undo.deleteEmptyFolder", json!({ "folderId": "$folder" }))],
            redo: vec![Op::new("mail.createFolder", p.clone()).saving("folder")],
            result_var: Some("folder".into()),
        }),
        "mail.renameFolder" => {
            let fid = i64_of(p, "folderId").unwrap_or_default();
            let f = ctx.db.read(move |c| mstore::folder(c, fid)).await?;
            Some(Prep::Fixed {
                undo: vec![Op::new("mail.renameFolder", json!({ "folderId": fid, "name": f.name }))],
                redo: vec![Op::new("mail.renameFolder", p.clone())],
                result_var: None,
            })
        }
        "mail.moveFolder" => {
            let fid = i64_of(p, "folderId").unwrap_or_default();
            let f = ctx.db.read(move |c| mstore::folder(c, fid)).await?;
            let back = f.path.rsplit_once('/').map(|(parent, _)| parent.to_string()).unwrap_or_default();
            Some(Prep::Fixed {
                undo: vec![Op::new("mail.moveFolder", json!({ "folderId": fid, "parentPath": back }))],
                redo: vec![Op::new("mail.moveFolder", p.clone())],
                result_var: None,
            })
        }
        "settings.set" => {
            let key = p.get("key").and_then(Value::as_str).unwrap_or_default().to_string();
            let k2 = key.clone();
            let old: Value = ctx.db.read(move |c| crate::db::util::get_setting::<Value>(c, &k2)).await?.unwrap_or(Value::Null);
            Some(Prep::Fixed {
                undo: vec![Op::new("settings.set", json!({ "key": key, "value": old }))],
                redo: vec![Op::new("settings.set", p.clone())],
                result_var: None,
            })
        }
        // ---- calendar ----
        "calendar.updateEvent" => {
            let id = i64_of(p, "id").unwrap_or_default();
            let d = call(ctx, "calendar.getEvent", json!({ "id": id })).await?;
            if notifies(&d) || d["readOnly"].as_bool() == Some(true) || adds_attendees(p) {
                return Ok(None);
            }
            let series = (p["scope"].as_str() == Some("series") || d["isMaster"].as_bool() == Some(true)) && d["seriesId"].is_i64();
            let target = if series { d["seriesId"].as_i64().unwrap_or(id) } else { id };
            let before = if target == id { d } else { call(ctx, "calendar.getEvent", json!({ "id": target })).await? };
            let with_rec = series || p["recurrence"].is_array();
            Some(Prep::CalUpdate { target, series, with_rec, before })
        }
        "calendar.copyEvent" => Some(Prep::Fixed {
            undo: vec![Op::new("calendar.deleteEvent", json!({ "id": "$new.id", "scope": "occurrence", "sendUpdates": false }))],
            redo: vec![Op::new("calendar.copyEvent", p.clone()).saving("new")],
            result_var: Some("new".into()),
        }),
        "calendar.createEvent" => {
            if adds_attendees(p) {
                return Ok(None); // invitations went out
            }
            let series = p["recurrence"].as_array().is_some_and(|r| !r.is_empty());
            Some(Prep::Fixed {
                undo: vec![Op::new(
                    "calendar.deleteEvent",
                    json!({ "id": "$new.id", "scope": if series { "series" } else { "occurrence" }, "sendUpdates": false }),
                )],
                redo: vec![Op::new("calendar.createEvent", p.clone()).saving("new")],
                result_var: Some("new".into()),
            })
        }
        "calendar.deleteEvent" => {
            let id = i64_of(p, "id").unwrap_or_default();
            let d = call(ctx, "calendar.getEvent", json!({ "id": id })).await?;
            let series = p["scope"].as_str() == Some("series") || d["isMaster"].as_bool() == Some(true);
            let master = if series && d["seriesId"].as_i64().is_some_and(|s| s != id) {
                call(ctx, "calendar.getEvent", json!({ "id": d["seriesId"] })).await?
            } else {
                d.clone()
            };
            if notifies(&master) || has_others(&master) || d["readOnly"].as_bool() == Some(true) {
                return Ok(None);
            }
            if !series && d["isRecurring"].as_bool() == Some(true) {
                return Ok(None); // one occurrence of a series: Google keeps it as a cancelled exception
            }
            let mut input = cal_input(&master, true);
            input.insert("calendarId".into(), master["calendarId"].clone());
            input.insert("sendUpdates".into(), json!(false));
            if series {
                input.insert("recurrence".into(), master["recurrence"].clone());
            }
            Some(Prep::Fixed {
                undo: vec![Op::new("calendar.createEvent", Value::Object(input)).saving("new")],
                redo: vec![Op::new(
                    "calendar.deleteEvent",
                    json!({ "id": "$new.id", "scope": if series { "series" } else { "occurrence" }, "sendUpdates": false }),
                )],
                result_var: None,
            })
        }
        "calendar.setEventCategories" => {
            let ids = ids_of(p);
            let mut undo = Vec::new();
            for id in &ids {
                let d = call(ctx, "calendar.getEvent", json!({ "id": id })).await?;
                undo.push(Op::new("calendar.setEventCategories", json!({ "ids": [id], "categories": d["categories"] })));
            }
            Some(Prep::Fixed { undo, redo: vec![Op::new("calendar.setEventCategories", p.clone())], result_var: None })
        }
        "calendar.setOwnSummary" => {
            let id = i64_of(p, "id").unwrap_or_default();
            let d = call(ctx, "calendar.getEvent", json!({ "id": id })).await?;
            Some(Prep::Fixed {
                undo: vec![Op::new("calendar.setOwnSummary", json!({ "id": id, "summary": d["summary"] }))],
                redo: vec![Op::new("calendar.setOwnSummary", p.clone())],
                result_var: None,
            })
        }
        "calendar.moveToCalendar" => {
            let id = i64_of(p, "id").unwrap_or_default();
            let before = call(ctx, "calendar.getEvent", json!({ "id": id })).await?;
            Some(Prep::CalMove { before, dest: i64_of(p, "calendarId").unwrap_or_default() })
        }
        // ---- contacts ----
        "contacts.update" => {
            let id = i64_of(p, "id").unwrap_or_default();
            Some(Prep::ContactUpdate { id, before: call(ctx, "contacts.get", json!({ "id": id })).await? })
        }
        "contacts.create" => Some(Prep::Fixed {
            undo: vec![Op::new("contacts.delete", json!({ "ids": ["$new.id"] }))],
            redo: vec![Op::new("contacts.create", p.clone()).saving("new")],
            result_var: Some("new".into()),
        }),
        "contacts.delete" => {
            let mut undo = Vec::new();
            let mut again = Vec::new();
            for (i, id) in ids_of(p).into_iter().enumerate() {
                let Ok(c) = call(ctx, "contacts.get", json!({ "id": id })).await else { continue };
                let name = format!("c{i}");
                undo.push(Op::new("contacts.create", json!({ "accountId": c["accountId"], "contact": fresh_contact(&c) })).saving(&name));
                again.push(json!(format!("${name}.id")));
            }
            if undo.is_empty() {
                return Ok(None);
            }
            Some(Prep::Fixed { undo, redo: vec![Op::new("contacts.delete", json!({ "ids": again }))], result_var: None })
        }
        "contacts.move" => {
            let target = i64_of(p, "accountId").unwrap_or_default();
            let mut before = Vec::new();
            for id in ids_of(p) {
                let acct = ctx
                    .db
                    .read(move |c| {
                        Ok(c.query_row("SELECT account_id FROM contacts WHERE id=?1", [id], |r| r.get::<_, i64>(0)).optional()?)
                    })
                    .await?;
                if let Some(a) = acct {
                    before.push((id, a));
                }
            }
            Some(Prep::ContactMove { target, before_accounts: before })
        }
        // ---- tasks ----
        "tasks.create" => Some(Prep::Fixed {
            undo: vec![Op::new("tasks.delete", json!({ "ids": ["$new.id"] }))],
            redo: vec![Op::new("tasks.create", p.clone()).saving("new")],
            result_var: Some("new".into()),
        }),
        "tasks.update" => {
            let id = i64_of(p, "id").unwrap_or_default();
            Some(Prep::TaskUpdate { ids: vec![id], before: vec![call(ctx, "tasks.get", json!({ "id": id })).await?] })
        }
        "tasks.setComplete" => {
            let ids = ids_of(p);
            let mut before = Vec::new();
            for id in &ids {
                before.push(call(ctx, "tasks.get", json!({ "id": id })).await?);
            }
            Some(Prep::TaskUpdate { ids, before })
        }
        "tasks.delete" => task_delete(ctx, ids_of(p)).await?,
        "tasks.move" => {
            let ids = ids_of(p);
            let ids2 = ids.clone();
            let before = ctx.db.read(move |c| ids2.iter().map(|id| task_place(c, *id)).collect::<anyhow::Result<Vec<_>>>()).await?;
            Some(Prep::TaskMove { ids, before })
        }
        _ => None,
    })
}

pub async fn finish(ctx: &Arc<AppCtx>, prep: Prep, params: &Value, out: &Value) -> anyhow::Result<Option<Recorded>> {
    let mut vars = Map::new();
    let (undo, redo) = match prep {
        Prep::Mail { ids, before } => {
            let (back, fwd) = mail_changes(ctx, ids, &before).await?;
            if back.is_empty() {
                return Ok(None);
            }
            (vec![Op::new("undo.mailRestore", json!({ "items": back }))], vec![Op::new("undo.mailRestore", json!({ "items": fwd }))])
        }
        Prep::MailMove { method, ids, before } => {
            let (back, fwd) = mail_changes(ctx, ids, &before).await?;
            if out["imported"].as_array().is_some_and(|a| !a.is_empty()) {
                vars.insert("moved".into(), out.clone());
                let mut undo = vec![Op::new("mail.deleteImported", json!({ "copies": "$moved.imported" }))];
                if !back.is_empty() {
                    undo.push(Op::new("undo.mailRestore", json!({ "items": back })));
                }
                (undo, vec![Op::new(&method, params.clone()).saving("moved")])
            } else if back.is_empty() {
                return Ok(None);
            } else {
                (vec![Op::new("undo.mailRestore", json!({ "items": back }))], vec![Op::new("undo.mailRestore", json!({ "items": fwd }))])
            }
        }
        Prep::Fixed { undo, redo, result_var } => {
            if let Some(v) = result_var {
                vars.insert(v, out.clone());
            }
            (undo, redo)
        }
        Prep::CalUpdate { target, series, with_rec, before } => {
            let after = call(ctx, "calendar.getEvent", json!({ "id": target })).await?;
            let (b, a) = (cal_input(&before, with_rec), cal_input(&after, with_rec));
            let keys: Vec<&String> = b.keys().filter(|k| b.get(*k) != a.get(*k)).collect();
            if keys.is_empty() {
                return Ok(None);
            }
            let time_changed = keys.iter().any(|k| TIME_KEYS.contains(&k.as_str()));
            let pick = |src: &Map<String, Value>| {
                let mut m = Map::new();
                m.insert("id".into(), json!(target));
                m.insert("scope".into(), json!(if series { "series" } else { "occurrence" }));
                m.insert("sendUpdates".into(), json!(false));
                for (k, v) in src {
                    if keys.contains(&k) || (time_changed && TIME_KEYS.contains(&k.as_str())) {
                        m.insert(k.clone(), v.clone());
                    }
                }
                Value::Object(m)
            };
            (vec![Op::new("calendar.updateEvent", pick(&b))], vec![Op::new("calendar.updateEvent", pick(&a))])
        }
        Prep::CalMove { before, dest } => {
            vars.insert("new".into(), out.clone());
            let after = out;
            let times = |d: &Value| {
                let mut m = Map::new();
                for k in TIME_KEYS {
                    if !d[*k].is_null() {
                        m.insert(k.to_string(), d[*k].clone());
                    }
                }
                m
            };
            let mut back = times(&before);
            back.insert("sendUpdates".into(), json!(false));
            back.insert("id".into(), json!("$new.id"));
            back.insert("calendarId".into(), before["calendarId"].clone());
            let mut fwd = times(after);
            fwd.insert("sendUpdates".into(), json!(false));
            fwd.insert("id".into(), json!("$back.id"));
            fwd.insert("calendarId".into(), json!(dest));
            (
                vec![Op::new("calendar.moveToCalendar", Value::Object(back)).saving("back")],
                vec![Op::new("calendar.moveToCalendar", Value::Object(fwd)).saving("new")],
            )
        }
        Prep::ContactUpdate { id, before } => (
            vec![Op::new("contacts.update", json!({ "id": id, "contact": before }))],
            vec![Op::new("contacts.update", json!({ "id": id, "contact": out }))],
        ),
        Prep::ContactMove { target, before_accounts } => {
            let new_ids: Vec<i64> = out.as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
            if new_ids.len() != before_accounts.len() {
                return Ok(None);
            }
            let mut groups: Vec<(i64, Vec<i64>)> = Vec::new();
            for ((_, acct), nid) in before_accounts.iter().zip(new_ids) {
                if *acct == target {
                    continue;
                }
                match groups.iter_mut().find(|g| g.0 == *acct) {
                    Some(g) => g.1.push(nid),
                    None => groups.push((*acct, vec![nid])),
                }
            }
            if groups.is_empty() {
                return Ok(None);
            }
            let (mut undo, mut redo) = (Vec::new(), Vec::new());
            for (acct, ids) in groups {
                vars.insert(format!("new{acct}"), json!(ids));
                undo.push(
                    Op::new("contacts.move", json!({ "ids": format!("$new{acct}"), "accountId": acct })).saving(&format!("back{acct}")),
                );
                redo.push(
                    Op::new("contacts.move", json!({ "ids": format!("$back{acct}"), "accountId": target })).saving(&format!("new{acct}")),
                );
            }
            (undo, redo)
        }
        Prep::TaskUpdate { ids, before } => {
            let (mut undo, mut redo) = (Vec::new(), Vec::new());
            for (id, b) in ids.iter().zip(before) {
                let a = call(ctx, "tasks.get", json!({ "id": id })).await?;
                let (bf, af) = (task_fields(&b), task_fields(&a));
                let mut keys: Vec<&str> = TASK_KEYS.iter().copied().filter(|k| bf.get(*k) != af.get(*k)).collect();
                if keys.is_empty() {
                    continue;
                }
                if keys.contains(&"status") || keys.contains(&"percent") {
                    keys.extend(["status", "percent"]);
                }
                let pick = |f: &Map<String, Value>| {
                    let mut m = Map::new();
                    m.insert("id".into(), json!(id));
                    for k in &keys {
                        m.insert(k.to_string(), f.get(*k).cloned().unwrap_or(Value::Null));
                    }
                    Value::Object(m)
                };
                undo.push(Op::new("tasks.update", pick(&bf)));
                redo.push(Op::new("tasks.update", pick(&af)));
            }
            if undo.is_empty() {
                return Ok(None);
            }
            (undo, redo)
        }
        Prep::TaskMove { ids, before } => {
            let new_ids: Vec<i64> = out.as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
            if new_ids.len() != ids.len() {
                return Ok(None);
            }
            vars.insert("new".into(), json!(new_ids));
            // put them back in their old order: the earliest first, so a "previous" sibling that
            // was moved too is already back when its follower is placed after it
            let mut order: Vec<usize> = (0..ids.len()).collect();
            order.sort_by(|a, b| (before[*a].list_id, &before[*a].position).cmp(&(before[*b].list_id, &before[*b].position)));
            let mut undo = Vec::new();
            for i in &order {
                let b = &before[*i];
                let prev = match b.previous.and_then(|p| ids.iter().position(|x| *x == p)) {
                    Some(j) => json!(format!("$back{j}.0")),
                    None => json!(b.previous),
                };
                undo.push(
                    Op::new(
                        "tasks.move",
                        json!({ "ids": [format!("$new.{i}")], "listId": b.list_id, "parentId": b.parent_id, "previousId": prev }),
                    )
                    .saving(&format!("back{i}")),
                );
            }
            let mut again = params.clone();
            again["ids"] = json!((0..ids.len()).map(|i| format!("$back{i}.0")).collect::<Vec<_>>());
            (undo, vec![Op::new("tasks.move", again).saving("new")])
        }
    };
    Ok(Some(Recorded { undo, redo, vars }))
}

// ---- mail -----------------------------------------------------------------------------------------

/// The messages whose state the action changed: (`undo.mailRestore` items back, items forward).
async fn mail_changes(ctx: &Arc<AppCtx>, ids: Vec<i64>, before: &HashMap<i64, MailState>) -> anyhow::Result<(Vec<Value>, Vec<Value>)> {
    let after = mail_states(ctx, ids.clone()).await?;
    let mut back = Vec::new();
    let mut fwd = Vec::new();
    for id in ids {
        let (Some(b), Some(a)) = (before.get(&id), after.get(&id)) else { continue };
        if b == a {
            continue;
        }
        back.push(json!({ "id": id, "from": a, "to": b }));
        fwd.push(json!({ "id": id, "from": b, "to": a }));
    }
    Ok((back, fwd))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MailState {
    pub labels: Vec<String>,
    pub flag_status: Option<String>,
    pub flag_due: Option<String>,
    pub flag_completed_at: Option<i64>,
}

async fn mail_states(ctx: &Arc<AppCtx>, ids: Vec<i64>) -> anyhow::Result<HashMap<i64, MailState>> {
    ctx.db
        .read(move |c| {
            let mut st = c.prepare_cached("SELECT label_ids, flag_status, flag_due, flag_completed_at FROM messages WHERE id=?1")?;
            let mut out = HashMap::new();
            for id in ids {
                let s = st
                    .query_row([id], |r| {
                        let mut labels: Vec<String> = r.get::<_, String>(0)?.split_whitespace().map(str::to_string).collect();
                        labels.sort();
                        Ok(MailState { labels, flag_status: r.get(1)?, flag_due: r.get(2)?, flag_completed_at: r.get(3)? })
                    })
                    .optional()?;
                if let Some(s) = s {
                    out.insert(id, s);
                }
            }
            Ok(out)
        })
        .await
}

#[derive(Deserialize)]
struct RestoreItem {
    id: i64,
    /// the state the action left the message in
    from: MailState,
    /// the state to go back to
    to: MailState,
}

#[derive(Deserialize)]
struct RestoreParams {
    items: Vec<RestoreItem>,
}

/// Put messages back: labels the target state had are re-added, labels the action added are
/// removed (others added since are left alone); Deleted Items via trash / untrash; flag fields.
async fn mail_restore(ctx: Arc<AppCtx>, p: RestoreParams) -> anyhow::Result<()> {
    let items: HashMap<i64, RestoreItem> = p.items.into_iter().map(|i| (i.id, i)).collect();
    let ids: Vec<i64> = items.keys().copied().collect();
    let accounts = ctx
        .db
        .write(move |c| {
            let infos = actions::load_infos(c, &ids)?;
            if infos.is_empty() {
                return Err(crate::user_err!("not_found", "the items no longer exist."));
            }
            let mut accounts = actions::execute(c, &infos, &|i, _map| {
                let Some(it) = items.get(&i.id) else { return Ok(vec![]) };
                let has = |l: &str| i.labels.iter().any(|x| x == l);
                let add: Vec<String> = it.to.labels.iter().filter(|l| !has(l)).cloned().collect();
                let remove: Vec<String> = it.from.labels.iter().filter(|l| !it.to.labels.contains(l) && has(l)).cloned().collect();
                let mut steps = Vec::new();
                if remove.iter().any(|l| l == "TRASH") {
                    steps.push(actions::Step::Untrash);
                }
                let (a, r): (Vec<String>, Vec<String>) =
                    (add.iter().filter(|l| *l != "TRASH").cloned().collect(), remove.iter().filter(|l| *l != "TRASH").cloned().collect());
                if !a.is_empty() || !r.is_empty() {
                    steps.push(actions::Step::Modify { add: a, remove: r });
                }
                if add.iter().any(|l| l == "TRASH") {
                    steps.push(actions::Step::Trash);
                }
                Ok(steps)
            })?;
            for i in &infos {
                let it = &items[&i.id];
                let (f, t) = (&it.from, &it.to);
                if (&f.flag_status, &f.flag_due, f.flag_completed_at) != (&t.flag_status, &t.flag_due, t.flag_completed_at) {
                    c.execute(
                        "UPDATE messages SET flag_status=?1, flag_due=?2, flag_completed_at=?3 WHERE id=?4",
                        params![t.flag_status, t.flag_due, t.flag_completed_at, i.id],
                    )?;
                    if !accounts.contains(&i.account_id) {
                        accounts.push(i.account_id);
                    }
                }
            }
            Ok(accounts)
        })
        .await?;
    actions::after(&ctx, &accounts);
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FolderParam {
    folder_id: i64,
}

async fn delete_empty_folder(ctx: Arc<AppCtx>, p: FolderParam) -> anyhow::Result<()> {
    let fid = p.folder_id;
    let n: i64 = ctx
        .db
        .read(move |c| {
            let f = mstore::folder(c, fid)?;
            let prefix = format!("{}/%", f.path);
            Ok(c.query_row(
                "SELECT COUNT(*) FROM message_folders mf JOIN folders f ON f.id=mf.folder_id WHERE f.account_id=?1 AND (f.id=?2 OR f.path LIKE ?3)",
                params![f.account_id, fid, prefix],
                |r| r.get(0),
            )?)
        })
        .await
        .map_err(|_| crate::user_err!("not_found", "the folder no longer exists."))?;
    if n > 0 {
        return Err(crate::user_err!("not_empty", "the folder isn't empty any more."));
    }
    call(&ctx, "mail.deleteFolder", json!({ "folderId": fid })).await?;
    Ok(())
}

// ---- calendar -------------------------------------------------------------------------------------

const TIME_KEYS: &[&str] = &["start", "end", "allDay", "startDate", "endDate"];

/// Attendees besides me: a change to the item would send them updates.
fn has_others(d: &Value) -> bool {
    d["attendees"].as_array().is_some_and(|a| a.iter().any(|x| x["isSelf"].as_bool() != Some(true)))
}

/// The change invites people (a new meeting).
fn adds_attendees(p: &Value) -> bool {
    p["attendees"].as_array().is_some_and(|a| !a.is_empty())
}

/// A meeting I organize (changes notify attendees) or one I was invited to.
fn notifies(d: &Value) -> bool {
    has_others(d) && d["isMeeting"].as_bool().unwrap_or(true)
}

/// The fields of `calendar.updateEvent` / `createEvent` that describe an item, from its detail.
fn cal_input(d: &Value, with_recurrence: bool) -> Map<String, Value> {
    let mut m = Map::new();
    for k in ["summary", "location", "description", "busyStatus", "isPrivate", "importance", "categories", "allDay"] {
        m.insert(k.into(), d[k].clone());
    }
    if d["allDay"].as_bool() == Some(true) {
        m.insert("startDate".into(), d["startDate"].clone());
        m.insert("endDate".into(), d["endDate"].clone());
    } else {
        m.insert("start".into(), d["start"].clone());
        m.insert("end".into(), d["end"].clone());
    }
    m.insert("reminderMinutes".into(), if d["hasReminder"].as_bool() == Some(false) { json!(-1) } else { d["reminderMinutes"].clone() });
    if with_recurrence {
        m.insert("recurrence".into(), if d["recurrence"].is_array() { d["recurrence"].clone() } else { json!([]) });
    }
    m
}

// ---- contacts -------------------------------------------------------------------------------------

fn fresh_contact(c: &Value) -> Value {
    let mut c = c.clone();
    if let Some(o) = c.as_object_mut() {
        o.remove("id");
        o.remove("resourceName");
        o.remove("etag");
        o.insert("dirty".into(), json!(true));
        // a Google-hosted photo URL can't be re-uploaded as is
        if o.get("photoUrl").and_then(Value::as_str).is_some_and(|u| !u.starts_with("data:")) {
            o.insert("photoUrl".into(), Value::Null);
        }
    }
    c
}

// ---- tasks ----------------------------------------------------------------------------------------

const TASK_KEYS: &[&str] = &["title", "notes", "due", "start", "status", "importance", "percent", "reminderAt", "categories", "isPrivate"];

fn task_fields(t: &Value) -> Map<String, Value> {
    let mut m = Map::new();
    for k in TASK_KEYS {
        m.insert(k.to_string(), t[*k].clone());
    }
    if t["complete"].as_bool() == Some(true) {
        m.insert("status".into(), json!("completed"));
        m.insert("percent".into(), json!(100));
    }
    m
}

#[derive(Debug, Clone)]
pub struct TaskPlace {
    list_id: i64,
    parent_id: Option<i64>,
    position: String,
    previous: Option<i64>,
}

fn task_place(c: &rusqlite::Connection, id: i64) -> anyhow::Result<TaskPlace> {
    let (list_id, parent_id, position) = c.query_row("SELECT list_id, parent_id, position FROM tasks WHERE id=?1", [id], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, String>(2)?))
    })?;
    Ok(TaskPlace { list_id, parent_id, position, previous: crate::tasks::store::previous_sibling(c, id)? })
}

fn task_create_params(t: &Value) -> Map<String, Value> {
    let mut m = task_fields(t);
    m.insert("listId".into(), t["listId"].clone());
    m
}

/// Deleted tasks (and their subtasks) are re-created in place; the new ids feed redo.
async fn task_delete(ctx: &Arc<AppCtx>, ids: Vec<i64>) -> anyhow::Result<Option<Prep>> {
    let ids2 = ids.clone();
    // (task, place, subtask ids) in list order
    let rows = ctx
        .db
        .read(move |c| {
            let mut v = Vec::new();
            for id in ids2 {
                let Ok(place) = task_place(c, id) else { continue };
                let subs: Vec<i64> = {
                    let mut st = c.prepare("SELECT id FROM tasks WHERE parent_id=?1 ORDER BY position")?;
                    st.query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?
                };
                v.push((id, place, subs));
            }
            v.sort_by(|a, b| (a.1.list_id, &a.1.position).cmp(&(b.1.list_id, &b.1.position)));
            Ok(v)
        })
        .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    // a subtask whose parent goes too is re-created with the parent
    let all: Vec<i64> = rows.iter().map(|r| r.0).collect();
    let mut undo = Vec::new();
    let mut again = Vec::new();
    let mut made: HashMap<i64, String> = HashMap::new();
    for (id, place, subs) in &rows {
        if place.parent_id.is_some_and(|p| all.contains(&p)) {
            continue;
        }
        let t = call(ctx, "tasks.get", json!({ "id": id })).await?;
        let name = format!("t{id}");
        let mut m = task_create_params(&t);
        m.insert("parentId".into(), json!(place.parent_id));
        m.insert(
            "previousId".into(),
            match place.previous {
                Some(p) => made.get(&p).map(|n| json!(format!("${n}.id"))).unwrap_or(json!(p)),
                None => Value::Null,
            },
        );
        undo.push(Op::new("tasks.create", Value::Object(m)).saving(&name));
        again.push(json!(format!("${name}.id")));
        let mut prev_sub: Option<String> = None;
        for s in subs {
            let st = call(ctx, "tasks.get", json!({ "id": s })).await?;
            let sname = format!("t{s}");
            let mut sm = task_create_params(&st);
            sm.insert("parentId".into(), json!(format!("${name}.id")));
            sm.insert("previousId".into(), prev_sub.as_ref().map(|n| json!(format!("${n}.id"))).unwrap_or(Value::Null));
            undo.push(Op::new("tasks.create", Value::Object(sm)).saving(&sname));
            prev_sub = Some(sname);
        }
        made.insert(*id, name);
    }
    Ok(Some(Prep::Fixed { undo, redo: vec![Op::new("tasks.delete", json!({ "ids": again }))], result_var: None }))
}

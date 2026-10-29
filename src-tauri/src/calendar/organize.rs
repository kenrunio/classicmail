//! Organising items: colour categories, Clean Up Calendar (duplicates) and moving items to
//! another calendar (side-by-side drag).

use std::{collections::HashMap, sync::Arc};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::{
    DeleteParams, EventInput, UpdateParams, account_identity,
    categories::{self, Category},
    changed, delete_event, delete_tx, detail, dupes, find_master, kick, load_row,
    model::{CalRow, EventDetail, EventRow, get_cal},
    read_only,
    sync::{self, MoveOp, PatchOp},
    update_event,
};
use crate::{ctx::AppCtx, db::util::now_ms, rpc::NoParams, sync::pending};

// ---- categories ------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryList {
    pub categories: Vec<Category>,
    /// Outlook's 25 colours: [preset, name, hex]
    pub palette: Vec<(u8, String, String)>,
    /// Google event colours: [colorId, name, hex]
    pub google_colors: Vec<(String, String, String)>,
}

pub async fn list_categories(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<CategoryList> {
    let list = ctx.db.read(categories::load).await?;
    Ok(CategoryList {
        categories: list,
        palette: categories::PALETTE.iter().map(|p| (p.0, p.1.to_string(), p.2.to_string())).collect(),
        google_colors: categories::GOOGLE_EVENT_COLORS.iter().map(|g| (g.0.to_string(), g.1.to_string(), g.2.to_string())).collect(),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveCategories {
    categories: Vec<Category>,
    /// [old name, new name] pairs: items carrying the old name are renamed
    #[serde(default)]
    renames: Vec<(String, String)>,
}

/// Save the master list. Renamed categories are renamed on items (appointments and tasks), and
/// items whose first category changed colour get the new Google colour. Returns how many items
/// changed.
pub async fn save_categories(ctx: Arc<AppCtx>, p: SaveCategories) -> anyhow::Result<usize> {
    let mut list: Vec<Category> = Vec::new();
    for mut cat in p.categories {
        cat.name = categories::clean_name(&cat.name);
        if cat.name.is_empty() || list.iter().any(|x| x.name.eq_ignore_ascii_case(&cat.name)) {
            continue;
        }
        if !cat.shortcut.is_empty() {
            // a shortcut key belongs to one category only
            for x in list.iter_mut().filter(|x| x.shortcut == cat.shortcut) {
                x.shortcut.clear();
            }
        }
        list.push(cat);
    }
    let renames: Vec<(String, String)> =
        p.renames.into_iter().map(|(a, b)| (a, categories::clean_name(&b))).filter(|(a, b)| !b.is_empty() && a != b).collect();
    let (n, accounts, task_accounts) = ctx
        .db
        .write(move |c| {
            let old = categories::load(c)?;
            let tx = c.transaction()?;
            categories::store(&tx, &list)?;
            let rows: Vec<EventRow> = {
                let mut st = tx.prepare("SELECT * FROM events WHERE categories != ''")?;
                st.query_map([], EventRow::from_row)?.collect::<Result<_, _>>()?
            };
            let mut n = 0;
            let mut accounts = Vec::new();
            for mut r in rows {
                let cats: Vec<String> = categories::normalise(
                    &r.categories
                        .iter()
                        .map(|x| {
                            renames.iter().find(|(a, _)| a.eq_ignore_ascii_case(x)).map(|(_, b)| b.clone()).unwrap_or_else(|| x.clone())
                        })
                        .collect::<Vec<_>>(),
                );
                let color = match categories::color_id_for(&cats, &list) {
                    Some(c) => Some(c),
                    None => categories::next_color_id(&r.categories, &cats, r.color_id.as_deref(), &old),
                };
                if cats == r.categories && color == r.color_id {
                    continue;
                }
                r.categories = cats;
                r.color_id = color;
                // occurrences follow their series locally; only singles / masters are patched
                let push = r.gcal_id.is_some() && r.recurring_event_id.is_none() && r.parent_local_id.is_none();
                r.local_dirty = push || r.local_dirty;
                r.save(&tx)?;
                if push {
                    pending::enqueue(
                        &tx,
                        r.account_id,
                        sync::DOMAIN,
                        "patch",
                        &PatchOp { event_id: r.id, what: "cosmetic".into(), send_updates: Some("none".into()) },
                    )?;
                    if !accounts.contains(&r.account_id) {
                        accounts.push(r.account_id);
                    }
                }
                n += 1;
            }
            // tasks keep the same names (locally only: Google Tasks has no categories)
            let mut task_accounts: Vec<i64> = Vec::new();
            if !renames.is_empty() {
                let tasks: Vec<(i64, i64, String)> = {
                    let mut st = tx.prepare("SELECT id, account_id, categories FROM tasks WHERE categories != '[]'")?;
                    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?
                };
                for (id, account, json) in tasks {
                    let cur: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
                    let cats = categories::normalise(
                        &cur.iter()
                            .map(|x| renames.iter().find(|(a, _)| a.eq_ignore_ascii_case(x)).map_or_else(|| x.clone(), |(_, b)| b.clone()))
                            .collect::<Vec<_>>(),
                    );
                    if cats == cur {
                        continue;
                    }
                    tx.execute(
                        "UPDATE tasks SET categories=?1, updated=?2 WHERE id=?3",
                        params![serde_json::to_string(&cats)?, now_ms(), id],
                    )?;
                    n += 1;
                    if !task_accounts.contains(&account) {
                        task_accounts.push(account);
                    }
                }
            }
            tx.commit()?;
            Ok((n, accounts, task_accounts))
        })
        .await?;
    ctx.events.emit("calendar.changed", serde_json::json!({}));
    for a in task_accounts {
        ctx.events.emit("tasks.changed", serde_json::json!({ "accountId": a }));
    }
    for a in accounts {
        kick(&ctx, a, vec![]);
    }
    Ok(n)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCategories {
    ids: Vec<i64>,
    categories: Vec<String>,
}

/// Categorize: set the categories of items (an occurrence categorizes its whole series, as in
/// Outlook). Allowed on every item of a calendar I can write, also invitations and Gmail events.
pub async fn set_event_categories(ctx: Arc<AppCtx>, p: SetCategories) -> anyhow::Result<()> {
    let accounts = ctx
        .db
        .write(move |c| {
            let defs = categories::load(c)?;
            let cats = categories::normalise(&p.categories);
            let tx = c.transaction()?;
            let mut accounts: Vec<i64> = Vec::new();
            for id in p.ids {
                let row = load_row(&tx, id)?;
                let cal = get_cal(&tx, row.calendar_id)?;
                if !cal.can_write() {
                    return Err(read_only());
                }
                let mut target = find_master(&tx, &row)?.unwrap_or_else(|| row.clone());
                let color = categories::next_color_id(&target.categories, &cats, target.color_id.as_deref(), &defs);
                target.categories = cats.clone();
                target.color_id = color.clone();
                target.updated_ms = Some(now_ms());
                target.local_dirty = true;
                target.save(&tx)?;
                if target.id != row.id || target.is_master {
                    tx.execute(
                        "UPDATE events SET categories=?1, color_id=?2, local_dirty=(gcal_id IS NOT NULL) WHERE id!=?3 AND (parent_local_id=?3 OR (calendar_id=?4 AND recurring_event_id=?5))",
                        params![categories::join_list(&cats), color, target.id, target.calendar_id, target.gcal_id.clone().unwrap_or_default()],
                    )?;
                }
                if target.gcal_id.is_some() {
                    pending::enqueue(&tx, row.account_id, sync::DOMAIN, "patch", &PatchOp { event_id: target.id, what: "cosmetic".into(), send_updates: Some("none".into()) })?;
                }
                if !accounts.contains(&row.account_id) {
                    accounts.push(row.account_id);
                }
            }
            tx.commit()?;
            Ok(accounts)
        })
        .await?;
    for a in accounts {
        changed(&ctx, a);
        kick(&ctx, a, vec![]);
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct SetOwnSummary {
    id: i64,
    /// your subject; "" or the organizer's subject = back to the organizer's
    summary: String,
}

/// Rename a meeting someone else organised, on your calendar only (as Outlook lets you): your
/// subject is shown everywhere in ClassicMail and kept on your copy at Google (cmSummary); the
/// organizer and the other guests keep theirs. An occurrence renames its whole series.
pub async fn set_own_summary(ctx: Arc<AppCtx>, p: SetOwnSummary) -> anyhow::Result<EventDetail> {
    let (account, id) = ctx
        .db
        .write(move |c| {
            let tx = c.transaction()?;
            let row = load_row(&tx, p.id)?;
            let cal = get_cal(&tx, row.calendar_id)?;
            if !cal.can_write() {
                return Err(read_only());
            }
            let mut target = find_master(&tx, &row)?.unwrap_or_else(|| row.clone());
            if target.organizer_self {
                return Err(crate::user_err!("not_invitation", "Your own items are renamed by editing them."));
            }
            let orig = target.summary_orig.clone().unwrap_or_else(|| target.summary.clone());
            let mine = p.summary.trim().to_string();
            (target.summary, target.summary_orig) = if mine.is_empty() || mine == orig { (orig, None) } else { (mine, Some(orig)) };
            target.updated_ms = Some(now_ms());
            target.local_dirty = target.gcal_id.is_some();
            target.save(&tx)?;
            if target.id != row.id || target.is_master {
                tx.execute(
                    "UPDATE events SET summary=COALESCE(?1, COALESCE(summary_orig, summary)), summary_orig=CASE WHEN ?1 IS NULL THEN NULL \
                     ELSE COALESCE(summary_orig, summary) END WHERE id!=?2 AND (parent_local_id=?2 OR (calendar_id=?3 AND recurring_event_id=?4))",
                    params![
                        target.summary_orig.as_ref().map(|_| target.summary.clone()),
                        target.id,
                        target.calendar_id,
                        target.gcal_id.clone().unwrap_or_default()
                    ],
                )?;
            }
            if target.gcal_id.is_some() {
                pending::enqueue(&tx, row.account_id, sync::DOMAIN, "patch", &PatchOp { event_id: target.id, what: "cosmetic".into(), send_updates: Some("none".into()) })?;
            }
            tx.commit()?;
            Ok((row.account_id, row.id))
        })
        .await?;
    changed(&ctx, account);
    kick(&ctx, account, vec![]);
    ctx.db.read(move |c| detail(c, &load_row(c, id)?)).await
}

// ---- Clean Up Calendar -----------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindDupParams {
    calendar_ids: Option<Vec<i64>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupResult {
    groups: Vec<dupes::DupGroup>,
    /// calendar id → "Calendar - you@example.com"
    calendars: HashMap<i64, String>,
    scanned: usize,
}

pub async fn find_duplicates(ctx: Arc<AppCtx>, p: FindDupParams) -> anyhow::Result<DupResult> {
    ctx.db
        .read(move |c| {
            let cals: Vec<(CalRow, String)> = {
                let mut st = c.prepare("SELECT c.*, a.email AS a_email FROM calendars c JOIN accounts a ON a.id=c.account_id WHERE a.enabled=1")?;
                st.query_map([], |r| Ok((CalRow::from_row(r)?, r.get("a_email")?)))?.collect::<Result<_, _>>()?
            };
            let ids: Vec<i64> = match &p.calendar_ids {
                Some(v) => v.clone(),
                None => cals.iter().filter(|c| c.0.selected).map(|c| c.0.id).collect(),
            };
            let mut rows: Vec<EventRow> = Vec::new();
            for id in &ids {
                let mut st = c.prepare(
                    "SELECT * FROM events WHERE calendar_id=?1 AND recurring_event_id IS NULL AND parent_local_id IS NULL AND status!='cancelled' ORDER BY start_ms, id",
                )?;
                rows.extend(st.query_map([id], EventRow::from_row)?.collect::<Result<Vec<_>, _>>()?);
            }
            let ro: Vec<i64> = cals.iter().filter(|c| !c.0.can_write()).map(|c| c.0.id).collect();
            let groups = dupes::find_groups(&rows, &ro);
            let calendars = cals.iter().filter(|c| ids.contains(&c.0.id)).map(|(c, e)| (c.id, super::calendar_label(c.is_primary, &c.summary, e))).collect();
            Ok(DupResult { groups, calendars, scanned: rows.len() })
        })
        .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteMany {
    ids: Vec<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteManyResult {
    deleted: usize,
    failed: Vec<String>,
}

/// Delete items the user confirmed (Clean Up Calendar): local-first, without notifying anyone.
/// One write and one push per account for the whole selection.
pub async fn delete_events(ctx: Arc<AppCtx>, p: DeleteMany) -> anyhow::Result<DeleteManyResult> {
    let (deleted, failed, accounts) = ctx
        .db
        .write(move |c| {
            let (mut deleted, mut failed, mut accounts) = (0, Vec::new(), Vec::new());
            for id in p.ids {
                let is_master = load_row(c, id).map(|r| r.is_master).unwrap_or(false);
                let scope = Some(if is_master { "series" } else { "occurrence" }.to_string());
                match delete_tx(c, &DeleteParams { id, scope, send_updates: Some(false) }) {
                    Ok(a) => {
                        deleted += 1;
                        if !accounts.contains(&a) {
                            accounts.push(a);
                        }
                    }
                    Err(e) => failed.push(format!("{id}: {e:#}")),
                }
            }
            Ok((deleted, failed, accounts))
        })
        .await?;
    for a in accounts {
        changed(&ctx, a);
        kick(&ctx, a, vec![]);
    }
    Ok(DeleteManyResult { deleted, failed })
}

// ---- move to another calendar ----------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveParams {
    id: i64,
    calendar_id: i64,
    #[serde(flatten)]
    input: EventInput,
}

enum MovePlan {
    Done(i64),
    CopyAcrossAccounts(EventRow),
}

fn other_attendees(r: &EventRow) -> bool {
    r.attendees.iter().any(|a| a.is_self != Some(true) && a.organizer != Some(true))
}

fn plan_move(c: &mut Connection, id: i64, dest_id: i64) -> anyhow::Result<(MovePlan, i64)> {
    let row = load_row(c, id)?;
    let src = get_cal(c, row.calendar_id)?;
    let dest = get_cal(c, dest_id)?;
    if row.calendar_id == dest.id {
        return Ok((MovePlan::Done(row.id), row.account_id));
    }
    if !src.can_write() || !dest.can_write() || row.type_read_only() || row.event_type == "outOfOffice" {
        return Err(read_only());
    }
    if row.recurring_event_id.is_some() || row.parent_local_id.is_some() {
        return Err(crate::user_err!(
            "series",
            "An occurrence can't be moved to another calendar on its own. Open the series to move the whole series."
        ));
    }
    if !row.organizer_self && other_attendees(&row) {
        return Err(crate::user_err!(
            "not_organizer",
            "This meeting was organized by someone else, so it can't be moved to another calendar."
        ));
    }
    let (email, name) = account_identity(c, dest.account_id)?;
    let tx = c.transaction()?;
    let mut r = row.clone();
    if dest.account_id != row.account_id && r.gcal_id.is_some() {
        if other_attendees(&r) || r.is_master {
            return Err(crate::user_err!(
                "other_account",
                "Meetings and recurring series can't be moved to a calendar of another account. Copy the item instead (hold Ctrl while dragging)."
            ));
        }
        return Ok((MovePlan::CopyAcrossAccounts(r), row.account_id));
    }
    r.calendar_id = dest.id;
    r.account_id = dest.account_id;
    r.organizer_email = Some(if dest.is_primary { email.clone() } else { dest.gcal_id.clone() });
    r.organizer_name = Some(if dest.is_primary { name } else { dest.summary.clone() });
    r.updated_ms = Some(now_ms());
    r.local_dirty = true;
    r.save(&tx)?;
    // `save` keeps the account: set it here (an unpushed item may change account)
    tx.execute("UPDATE events SET account_id=?1 WHERE id=?2", params![dest.account_id, r.id])?;
    match r.gcal_id.clone() {
        Some(g) => {
            // series occurrences live with their master
            tx.execute("UPDATE events SET calendar_id=?1 WHERE calendar_id=?2 AND recurring_event_id=?3", params![dest.id, src.id, g])?;
            // edits not pushed yet are sent after the move: before it they would go to the new
            // calendar, where the event isn't yet (404, and the edit would be lost)
            let waiting: Vec<(i64, String)> = {
                let mut st = tx.prepare(
                    "SELECT id, payload FROM pending_ops WHERE domain='calendar' AND account_id=?1 AND kind='patch' \
                     AND json_extract(payload,'$.eventId') IN (SELECT id FROM events WHERE id=?2 OR (calendar_id=?3 AND recurring_event_id=?4)) \
                     ORDER BY id",
                )?;
                st.query_map(params![row.account_id, r.id, dest.id, g], |x| Ok((x.get(0)?, x.get(1)?)))?.collect::<Result<_, _>>()?
            };
            pending::enqueue(
                &tx,
                r.account_id,
                sync::DOMAIN,
                "move",
                &MoveOp { event_id: r.id, gcal_id: g, from_calendar_gcal_id: src.gcal_id.clone() },
            )?;
            for (op_id, payload) in waiting {
                tx.execute("DELETE FROM pending_ops WHERE id=?1", [op_id])?;
                let payload: serde_json::Value = serde_json::from_str(&payload)?;
                pending::enqueue(&tx, r.account_id, sync::DOMAIN, "patch", &payload)?;
            }
        }
        None => {
            // not on the server yet: the pending create goes to the new calendar (and account)
            tx.execute(
                "UPDATE events SET calendar_id=?1, account_id=?2 WHERE parent_local_id=?3",
                params![dest.id, dest.account_id, r.id],
            )?;
            tx.execute(
                "UPDATE pending_ops SET account_id=?1 WHERE domain='calendar' AND account_id=?2 AND json_extract(payload,'$.eventId')=?3",
                params![dest.account_id, row.account_id, r.id],
            )?;
        }
    }
    tx.commit()?;
    Ok((MovePlan::Done(r.id), row.account_id))
}

/// Move an item to another calendar (drag between side-by-side calendars), optionally changing
/// its time in the same step. Same account: Google `events.move`; another account: copy + delete.
pub async fn move_to_calendar(ctx: Arc<AppCtx>, p: MoveParams) -> anyhow::Result<EventDetail> {
    let (id, dest) = (p.id, p.calendar_id);
    let (plan, old_account) = ctx.db.write(move |c| plan_move(c, id, dest)).await?;
    let new_id = match plan {
        MovePlan::Done(i) => i,
        MovePlan::CopyAcrossAccounts(src) => {
            let input = EventInput {
                calendar_id: Some(dest),
                summary: Some(src.summary.clone()),
                location: Some(src.location.clone()),
                description: Some(src.description.clone()),
                start: Some(src.start_ms),
                end: Some(src.end_ms),
                all_day: Some(src.all_day),
                start_date: src.start_date.clone(),
                end_date: src.end_date.clone(),
                attendees: None,
                reminder_minutes: Some(src.reminder_min.unwrap_or(-1)),
                busy_status: Some(src.busy_status().to_string()),
                is_private: Some(src.is_private()),
                importance: src.importance.clone(),
                recurrence: None,
                send_updates: Some(false),
                categories: Some(src.categories.clone()),
                time_zone: src.time_zone.clone(),
            };
            let d = super::create_event(ctx.clone(), input).await?;
            delete_event(ctx.clone(), DeleteParams { id: src.id, scope: Some("occurrence".into()), send_updates: Some(false) }).await?;
            d.ev.id
        }
    };
    changed(&ctx, old_account);
    let has_change = p.input.start.is_some() || p.input.end.is_some() || p.input.start_date.is_some() || p.input.all_day.is_some();
    if has_change {
        let mut input = p.input;
        input.calendar_id = None;
        return update_event(ctx, UpdateParams { id: new_id, scope: Some("occurrence".into()), input }).await;
    }
    let account = ctx.db.read(move |c| Ok(load_row(c, new_id)?.account_id)).await?;
    changed(&ctx, account);
    kick(&ctx, account, vec![]);
    ctx.db.read(move |c| detail(c, &load_row(c, new_id)?)).await
}

//! `rules` table access (synchronous — call inside `ctx.db.read/write`).

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::model::{Action, Rule};
use crate::db::util::now_ms;

const COLS: &str = "id, name, enabled, position, account_id, conditions, exceptions, actions, created_at, updated_at";

fn from_row(r: &Row) -> rusqlite::Result<Result<Rule, String>> {
    let id: i64 = r.get(0)?;
    let name: String = r.get(1)?;
    let conditions: String = r.get(5)?;
    let exceptions: String = r.get(6)?;
    let actions: String = r.get(7)?;
    let parse = || -> Result<Rule, serde_json::Error> {
        Ok(Rule {
            id,
            name: name.clone(),
            enabled: r.get::<_, i64>(2).unwrap_or(0) != 0,
            position: r.get(3).unwrap_or(0),
            account_id: r.get(4).unwrap_or(None),
            conditions: serde_json::from_str(&conditions)?,
            exceptions: serde_json::from_str(&exceptions)?,
            actions: serde_json::from_str(&actions)?,
            created_at: r.get(8).unwrap_or(0),
            updated_at: r.get(9).unwrap_or(0),
        })
    };
    Ok(parse().map_err(|e| format!("rule {id} \"{name}\": {e}")))
}

/// Every rule in order. Rules whose JSON can't be read (e.g. written by a newer version) are
/// skipped with a warning rather than half-applied.
pub fn list(c: &Connection) -> anyhow::Result<Vec<Rule>> {
    let mut st = c.prepare(&format!("SELECT {COLS} FROM rules ORDER BY position, id"))?;
    let rows = st.query_map([], from_row)?;
    let mut out = Vec::new();
    for r in rows {
        match r? {
            Ok(rule) => out.push(rule),
            Err(e) => log::warn!("rules: skipping unreadable {}", crate::logging::redact(&e)),
        }
    }
    Ok(out)
}

pub fn get(c: &Connection, id: i64) -> anyhow::Result<Rule> {
    let r = c
        .query_row(&format!("SELECT {COLS} FROM rules WHERE id=?1"), [id], from_row)
        .optional()?
        .ok_or_else(|| crate::user_err!("not_found", "rule {id} not found"))?;
    r.map_err(|e| crate::user_err!("bad_rule", "{e}"))
}

fn json<T: serde::Serialize>(v: &T) -> anyhow::Result<String> {
    Ok(serde_json::to_string(v)?)
}

/// Insert (id 0) or update a rule; returns the id. New rules go to the end of the list.
pub fn save(c: &Connection, r: &Rule) -> anyhow::Result<i64> {
    let now = now_ms();
    let stop = r.actions.iter().any(|a| matches!(a, Action::StopProcessing)) as i64;
    if r.id > 0 {
        let n = c.execute(
            "UPDATE rules SET name=?1, enabled=?2, account_id=?3, conditions=?4, exceptions=?5, actions=?6, \
             stop_processing=?7, updated_at=?8 WHERE id=?9",
            params![
                r.name.trim(),
                r.enabled as i64,
                r.account_id,
                json(&r.conditions)?,
                json(&r.exceptions)?,
                json(&r.actions)?,
                stop,
                now,
                r.id
            ],
        )?;
        if n == 0 {
            return Err(crate::user_err!("not_found", "rule {} not found", r.id));
        }
        Ok(r.id)
    } else {
        let pos: i64 = c.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM rules", [], |x| x.get(0))?;
        c.execute(
            "INSERT INTO rules(name, enabled, position, account_id, conditions, exceptions, actions, stop_processing, \
             created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
            params![
                r.name.trim(),
                r.enabled as i64,
                pos,
                r.account_id,
                json(&r.conditions)?,
                json(&r.exceptions)?,
                json(&r.actions)?,
                stop,
                now
            ],
        )?;
        Ok(c.last_insert_rowid())
    }
}

pub fn delete(c: &Connection, ids: &[i64]) -> anyhow::Result<usize> {
    let mut n = 0;
    for id in ids {
        n += c.execute("DELETE FROM rules WHERE id=?1", [id])?;
    }
    Ok(n)
}

pub fn set_enabled(c: &Connection, id: i64, enabled: bool) -> anyhow::Result<()> {
    c.execute("UPDATE rules SET enabled=?1, updated_at=?2 WHERE id=?3", params![enabled as i64, now_ms(), id])?;
    Ok(())
}

/// Put the given rules in this order. Rules not listed keep their slots: the listed rules are
/// re-dealt over the positions they already occupy (so a filtered view can be reordered).
pub fn reorder(c: &mut Connection, ids: &[i64]) -> anyhow::Result<()> {
    let tx = c.transaction()?;
    let all = list(&tx)?;
    let mut slots: Vec<i64> = all.iter().filter(|r| ids.contains(&r.id)).map(|r| r.position).collect();
    if slots.len() != ids.len() {
        return Err(crate::user_err!("not_found", "some rules no longer exist"));
    }
    slots.sort();
    // positions may collide (old data); make them unique first
    let mut dedup = slots.clone();
    dedup.dedup();
    if dedup.len() != slots.len() {
        for (i, r) in all.iter().enumerate() {
            tx.execute("UPDATE rules SET position=?1 WHERE id=?2", params![i as i64 * 10, r.id])?;
        }
        tx.commit()?;
        return reorder(c, ids);
    }
    for (id, pos) in ids.iter().zip(slots) {
        tx.execute("UPDATE rules SET position=?1 WHERE id=?2", params![pos, id])?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::model::Condition;

    fn r(name: &str) -> Rule {
        Rule {
            id: 0,
            name: name.into(),
            enabled: true,
            position: 0,
            account_id: None,
            conditions: vec![Condition::HasAttachment],
            exceptions: vec![],
            actions: vec![Action::MarkRead],
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn crud_and_reorder() {
        let (db, _g) = crate::db::Db::open_temp().unwrap();
        db.write_blocking(|c| {
            let a = save(c, &r("A"))?;
            let b = save(c, &r("B"))?;
            let cc = save(c, &r("C"))?;
            let names = |c: &Connection| list(c).unwrap().into_iter().map(|r| r.name).collect::<Vec<_>>();
            assert_eq!(names(c), ["A", "B", "C"]);
            reorder(c, &[cc, a, b])?;
            assert_eq!(names(c), ["C", "A", "B"]);
            // partial reorder keeps the others in place
            reorder(c, &[b, a])?;
            assert_eq!(names(c), ["C", "B", "A"]);
            let mut rb = get(c, b)?;
            rb.name = "B2".into();
            rb.actions.push(Action::StopProcessing);
            save(c, &rb)?;
            let stop: i64 = c.query_row("SELECT stop_processing FROM rules WHERE id=?1", [b], |x| x.get(0))?;
            assert_eq!(stop, 1);
            set_enabled(c, a, false)?;
            assert!(!get(c, a)?.enabled);
            delete(c, &[cc])?;
            assert_eq!(names(c), ["B2", "A"]);
            // unreadable rows are skipped
            c.execute("UPDATE rules SET actions='[{\"type\":\"teleport\"}]' WHERE id=?1", [a])?;
            assert_eq!(names(c), ["B2"]);
            Ok(())
        })
        .unwrap();
    }
}

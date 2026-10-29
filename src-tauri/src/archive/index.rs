//! Background indexing of an archive: the folder tree and one header row per message (from the
//! folders' contents tables — no message is opened), plus an FTS index over subject / from / to.
//! Optional second pass: message text ("Index message text for search").
//!
//! Resumable: each folder remembers how many contents-table rows were indexed; a restarted job
//! continues there (rows are `INSERT OR IGNORE`d on (archive, node id) anyway). When the file's
//! size or modification time changed since it was indexed, the index is rebuilt.
//!
//! Runs on the indexer thread (`worker.rs`) with its own PST handle and short write transactions,
//! so the UI and mail sync are never blocked for long.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use super::{
    props::{self, Props, tag},
    pst,
    worker::file_stamp,
};
use crate::ctx::AppCtx;

/// Status-bar slot for archive work (sync.status is keyed by account id; accounts are > 0).
pub const STATUS_ID: i64 = -1;

const BATCH: usize = 1000;

pub fn folder_kind(class: &str) -> &'static str {
    let c = class.to_ascii_lowercase();
    if c.starts_with("ipf.appointment") {
        "calendar"
    } else if c.starts_with("ipf.contact") {
        "contacts"
    } else if c.starts_with("ipf.task") {
        "tasks"
    } else if c.starts_with("ipf.stickynote") {
        "notes"
    } else if c.starts_with("ipf.journal") {
        "journal"
    } else if c.starts_with("ipf.configuration") || c.starts_with("ipf.files") {
        "other"
    } else {
        "mail"
    }
}

struct ArchiveRow {
    path: String,
    name: String,
    size: i64,
    mtime: i64,
}

fn load(c: &Connection, id: i64) -> anyhow::Result<Option<ArchiveRow>> {
    Ok(c.query_row("SELECT path, display_name, file_size, file_mtime FROM archives WHERE id=?1", [id], |r| {
        Ok(ArchiveRow { path: r.get(0)?, name: r.get(1)?, size: r.get(2)?, mtime: r.get(3)? })
    })
    .optional()?)
}

fn set_status(ctx: &AppCtx, id: i64, status: &str, error: Option<&str>) -> anyhow::Result<()> {
    ctx.db.write_blocking(|c| {
        c.execute("UPDATE archives SET status=?2, error=?3 WHERE id=?1", params![id, status, error])?;
        Ok(())
    })?;
    ctx.events.emit("archive.changed", json!({ "archiveId": id }));
    Ok(())
}

/// Forget everything indexed for an archive (file changed / closed).
pub fn clear_index(c: &Connection, id: i64) -> anyhow::Result<()> {
    c.execute("DELETE FROM archive_fts WHERE rowid IN (SELECT id FROM archive_messages WHERE archive_id=?1)", [id])?;
    c.execute("DELETE FROM archive_messages WHERE archive_id=?1", [id])?;
    c.execute("DELETE FROM archive_folders WHERE archive_id=?1", [id])?;
    c.execute("UPDATE archives SET item_count=0, indexed_at=NULL, bodies_indexed=0, index_bodies=CASE WHEN index_bodies=2 THEN 1 ELSE index_bodies END WHERE id=?1", [id])?;
    Ok(())
}

pub fn mark_error(ctx: &AppCtx, id: i64, e: &anyhow::Error) {
    let (status, msg) = match e.downcast_ref::<crate::error::UserError>() {
        Some(u) if u.code == "locked" => ("locked", u.message.clone()),
        Some(u) if u.code == "not_found" => ("missing", u.message.clone()),
        Some(u) => ("error", u.message.clone()),
        None => ("error", format!("{e:#}")),
    };
    let _ = set_status(ctx, id, status, Some(&msg));
    ctx.events.status(STATUS_ID, "idle", "", None);
}

struct Progress<'a> {
    ctx: &'a AppCtx,
    label: String,
    total: i64,
    done: i64,
    last: Instant,
}

impl Progress<'_> {
    fn tick(&mut self, n: i64, force: bool) {
        self.done += n;
        if !force && self.last.elapsed() < Duration::from_millis(400) {
            return;
        }
        self.last = Instant::now();
        let total = self.total.max(self.done);
        let text = format!("{}: {} of {}", self.label, fmt_n(self.done), fmt_n(total));
        let p = if total > 0 { Some(self.done as f64 / total as f64) } else { None };
        self.ctx.events.status(STATUS_ID, "sync", &text, p);
    }
}

pub fn fmt_n(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Header pass. Returns early (Ok) when cancelled.
pub fn index_headers(ctx: &Arc<AppCtx>, id: i64, cancel: &AtomicBool) -> anyhow::Result<()> {
    let started = Instant::now();
    let Some(a) = ctx.db.write_blocking(|c| load(c, id))? else { return Ok(()) };
    let path = std::path::PathBuf::from(&a.path);
    let stamp = match file_stamp(&path) {
        Ok(s) => s,
        Err(e) => return Err(pst::open_error(&path, e)),
    };
    let handle = pst::open(&path)?;
    let store_name = handle.display_name();
    let format = handle.format().as_str();
    let have_folders: bool = ctx.db.write_blocking(|c| {
        let tx = c.transaction()?;
        let changed = (a.size, a.mtime) != stamp;
        if changed {
            clear_index(&tx, id)?;
        }
        tx.execute(
            "UPDATE archives SET status='indexing', error=NULL, store_name=?2, format=?3, file_size=?4, file_mtime=?5 WHERE id=?1",
            params![id, store_name, format, stamp.0, stamp.1],
        )?;
        let n: i64 = tx.query_row("SELECT COUNT(*) FROM archive_folders WHERE archive_id=?1", [id], |r| r.get(0))?;
        tx.commit()?;
        Ok(n > 0)
    })?;
    ctx.events.emit("archive.changed", json!({ "archiveId": id }));

    if !have_folders {
        let folders = handle.folders()?;
        ctx.db.write_blocking(|c| {
            let tx = c.transaction()?;
            for (pos, f) in folders.iter().enumerate() {
                let parent: Option<i64> = match f.parent {
                    Some(p) => tx
                        .query_row("SELECT id FROM archive_folders WHERE archive_id=?1 AND node_id=?2", params![id, p as i64], |r| r.get(0))
                        .optional()?,
                    None => None,
                };
                tx.execute(
                    "INSERT OR IGNORE INTO archive_folders(archive_id, node_id, parent_id, name, container_class, kind, content_count, position) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![id, f.node_id as i64, parent, f.name, f.container_class, folder_kind(&f.container_class), f.content_count, pos as i64],
                )?;
            }
            tx.commit()?;
            Ok(())
        })?;
        ctx.events.emit("archive.changed", json!({ "archiveId": id }));
    }

    // folders still to do (mail folders only in v1)
    let todo: Vec<(i64, u32, i64, i64)> = ctx.db.write_blocking(|c| {
        let mut st = c.prepare("SELECT id, node_id, indexed, content_count FROM archive_folders WHERE archive_id=?1 AND kind='mail' AND done=0 ORDER BY position")?;
        let v = st.query_map([id], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u32, r.get(2)?, r.get(3)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    })?;
    let (total, already): (i64, i64) = ctx.db.write_blocking(|c| {
        Ok(c.query_row(
            "SELECT COALESCE(SUM(content_count), 0), (SELECT COUNT(*) FROM archive_messages WHERE archive_id=?1) FROM archive_folders WHERE archive_id=?1 AND kind='mail'",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    })?;
    let mut prog =
        Progress { ctx, label: format!("Indexing {}", a.name), total, done: already, last: Instant::now() - Duration::from_secs(1) };
    prog.tick(0, true);

    let mut last_changed = Instant::now();
    for (fid, nid, indexed, _count) in todo {
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut batch: Vec<(u32, Props)> = Vec::with_capacity(BATCH);
        let mut failed: Option<anyhow::Error> = None;
        let rows = handle.folder_rows(nid, Some(props::HEADER_TAGS), indexed.max(0) as usize, |i, row_nid, p| {
            batch.push((row_nid, p));
            if batch.len() >= BATCH {
                if let Err(e) = write_batch(ctx, id, fid, &batch, i as i64 + 1) {
                    failed = Some(e);
                    return false;
                }
                prog.tick(batch.len() as i64, false);
                batch.clear();
                // be a good neighbour: let other writers in
                std::thread::sleep(Duration::from_millis(2));
            }
            !cancel.load(Ordering::SeqCst)
        })?;
        if let Some(e) = failed {
            return Err(e);
        }
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        write_batch(ctx, id, fid, &batch, rows as i64)?;
        prog.tick(batch.len() as i64, false);
        ctx.db.write_blocking(|c| {
            c.execute("UPDATE archive_folders SET done=1, indexed=?2 WHERE id=?1", params![fid, rows as i64])?;
            Ok(())
        })?;
        // the folder tree / list refresh as folders complete (not more than every 1.5 s)
        if last_changed.elapsed() > Duration::from_millis(1500) {
            last_changed = Instant::now();
            ctx.events.emit("archive.changed", json!({ "archiveId": id, "folderId": fid }));
        }
    }

    let ms = started.elapsed().as_millis() as i64;
    let count: i64 = ctx.db.write_blocking(|c| {
        let n: i64 = c.query_row("SELECT COUNT(*) FROM archive_messages WHERE archive_id=?1", [id], |r| r.get(0))?;
        c.execute(
            "UPDATE archives SET status='ready', error=NULL, item_count=?2, indexed_at=?3, index_ms=COALESCE(index_ms, ?4) WHERE id=?1",
            params![id, n, crate::db::util::now_ms(), ms],
        )?;
        Ok(n)
    })?;
    log::info!("archive {id}: {count} items indexed in {ms} ms");
    ctx.events.emit("archive.changed", json!({ "archiveId": id }));
    ctx.events.status(STATUS_ID, "idle", &format!("{} is ready ({} items).", a.name, fmt_n(count)), None);
    // queued text indexing follows
    let want_bodies: i64 =
        ctx.db.write_blocking(|c| Ok(c.query_row("SELECT index_bodies FROM archives WHERE id=?1", [id], |r| r.get(0)).unwrap_or(0)))?;
    if want_bodies == 1 {
        super::worker::enqueue(ctx, id, super::worker::JobKind::Bodies);
    }
    Ok(())
}

fn write_batch(ctx: &AppCtx, archive_id: i64, folder_id: i64, rows: &[(u32, Props)], indexed: i64) -> anyhow::Result<()> {
    if rows.is_empty() && indexed == 0 {
        return Ok(());
    }
    let heads: Vec<(u32, props::Header)> = rows.iter().map(|(nid, p)| (*nid, props::header(p))).collect();
    ctx.db.write_blocking(|c| {
        let tx = c.transaction()?;
        {
            let mut ins = tx.prepare_cached(
                "INSERT OR IGNORE INTO archive_messages(archive_id, folder_id, node_id, message_class, subject, subject_norm, from_name, from_email, \
                 to_display, cc_display, date, sent_date, size, is_read, flag_status, has_attachments, importance, conversation_topic, last_verb) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            )?;
            let mut fts = tx.prepare_cached("INSERT INTO archive_fts(rowid, subject, from_text, to_text, body) VALUES (?1, ?2, ?3, ?4, '')")?;
            for (nid, h) in &heads {
                let n = ins.execute(params![
                    archive_id,
                    folder_id,
                    *nid as i64,
                    h.message_class,
                    h.subject,
                    crate::mail::mime::normalize_subject(&h.subject),
                    h.from_name,
                    h.from_email,
                    h.to_display,
                    h.cc_display,
                    h.date,
                    h.sent,
                    h.size,
                    h.is_read as i64,
                    h.flag_status,
                    h.has_attachments as i64,
                    h.importance,
                    h.conversation_topic,
                    h.last_verb,
                ])?;
                if n == 1 {
                    let rowid = tx.last_insert_rowid();
                    fts.execute(params![rowid, h.subject, format!("{} {}", h.from_name, h.from_email), format!("{} {}", h.to_display, h.cc_display)])?;
                }
            }
        }
        tx.execute("UPDATE archive_folders SET indexed=MAX(indexed, ?2) WHERE id=?1", params![folder_id, indexed])?;
        tx.commit()?;
        Ok(())
    })
}

/// A message waiting for text indexing: (id, node id, subject, from, to).
type Todo = (i64, i64, String, String, String);

/// One batch of message text into the FTS index. Messages that are gone (the archive was closed
/// meanwhile) are left out, so no orphan text can match a later message that reuses the id.
/// Returns how many were written.
fn write_bodies(c: &mut Connection, archive_id: i64, rows: &[(Todo, String)]) -> anyhow::Result<i64> {
    let tx = c.transaction()?;
    let mut n = 0;
    {
        let mut mark = tx.prepare_cached("UPDATE archive_messages SET body_indexed=1 WHERE id=?1")?;
        let mut del = tx.prepare_cached("DELETE FROM archive_fts WHERE rowid=?1")?;
        let mut ins = tx.prepare_cached("INSERT INTO archive_fts(rowid, subject, from_text, to_text, body) VALUES (?1, ?2, ?3, ?4, ?5)")?;
        for ((mid, _, subj, from, to), text) in rows {
            if mark.execute([mid])? == 0 {
                continue;
            }
            del.execute([mid])?;
            ins.execute(params![mid, subj, from, to, text])?;
            n += 1;
        }
    }
    tx.execute("UPDATE archives SET bodies_indexed=bodies_indexed+?2 WHERE id=?1", params![archive_id, n])?;
    tx.commit()?;
    Ok(n)
}

/// Text pass ("Index message text for search"): message bodies into the FTS index.
pub fn index_bodies(ctx: &Arc<AppCtx>, id: i64, cancel: &AtomicBool) -> anyhow::Result<()> {
    let Some(a) = ctx.db.write_blocking(|c| load(c, id))? else { return Ok(()) };
    let want: i64 = ctx.db.write_blocking(|c| Ok(c.query_row("SELECT index_bodies FROM archives WHERE id=?1", [id], |r| r.get(0))?))?;
    if want == 0 {
        return Ok(());
    }
    let path = std::path::PathBuf::from(&a.path);
    let handle = pst::open(&path)?;
    let (total, done): (i64, i64) = ctx.db.write_blocking(|c| {
        Ok(c.query_row("SELECT COUNT(*), COALESCE(SUM(body_indexed), 0) FROM archive_messages WHERE archive_id=?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?)
    })?;
    let mut prog =
        Progress { ctx, label: format!("Indexing message text in {}", a.name), total, done, last: Instant::now() - Duration::from_secs(1) };
    let want_tags = [tag::BODY, tag::HTML, tag::RTF_COMPRESSED, tag::INTERNET_CPID, tag::MESSAGE_CODEPAGE];
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        let todo: Vec<Todo> = ctx.db.write_blocking(|c| {
            let mut st = c.prepare(
                "SELECT id, node_id, subject, from_name || ' ' || from_email, to_display || ' ' || cc_display FROM archive_messages \
                 WHERE archive_id=?1 AND body_indexed=0 LIMIT 200",
            )?;
            let v = st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })?;
        if todo.is_empty() {
            break;
        }
        let mut out: Vec<(Todo, String)> = Vec::with_capacity(todo.len());
        for t in todo {
            let text = match handle.message_props(t.1 as u32, &want_tags) {
                Ok(p) => {
                    let mut s = props::body(&p).text;
                    if s.len() > 64 * 1024 {
                        let mut cut = 64 * 1024;
                        while !s.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        s.truncate(cut);
                    }
                    s
                }
                Err(e) => {
                    log::debug!("archive {id}: text of {:#x}: {}", t.1, crate::logging::err(&e));
                    String::new()
                }
            };
            out.push((t, text));
            if cancel.load(Ordering::SeqCst) {
                break;
            }
        }
        let n = ctx.db.write_blocking(|c| write_bodies(c, id, &out))?;
        prog.tick(n, false);
        std::thread::sleep(Duration::from_millis(2));
    }
    ctx.db.write_blocking(|c| {
        c.execute("UPDATE archives SET index_bodies=2 WHERE id=?1 AND index_bodies=1", [id])?;
        Ok(())
    })?;
    ctx.events.emit("archive.changed", json!({ "archiveId": id }));
    ctx.events.status(STATUS_ID, "idle", &format!("Message text in {} is indexed.", a.name), None);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_numbers() {
        assert_eq!(folder_kind("IPF.Note"), "mail");
        assert_eq!(folder_kind(""), "mail");
        assert_eq!(folder_kind("IPF.Appointment"), "calendar");
        assert_eq!(folder_kind("IPF.Contact"), "contacts");
        assert_eq!(fmt_n(1234567), "1,234,567");
        assert_eq!(fmt_n(12), "12");
    }

    #[test]
    fn text_of_a_closed_archive_is_not_indexed() {
        let (db, _g) = crate::db::Db::open_temp().unwrap();
        db.write_blocking(|c| {
            c.execute("INSERT INTO archives(id, path, display_name, added_at) VALUES (1, 'a.pst', 'a', 0)", [])?;
            c.execute("INSERT INTO archive_folders(id, archive_id, node_id, name) VALUES (1, 1, 1, 'Inbox')", [])?;
            c.execute("INSERT INTO archive_messages(id, archive_id, folder_id, node_id) VALUES (10, 1, 1, 100)", [])?;
            let row = |id: i64, text: &str| ((id, 0, String::new(), String::new(), String::new()), text.to_string());
            // message 11 was removed (archive closed) while its text was being read
            let n = write_bodies(c, 1, &[row(10, "budget"), row(11, "stale")])?;
            assert_eq!(n, 1);
            let hits = |q: &str| -> rusqlite::Result<i64> {
                c.query_row("SELECT COUNT(*) FROM archive_fts WHERE archive_fts MATCH ?1", [q], |r| r.get(0))
            };
            assert_eq!(hits("budget")?, 1);
            assert_eq!(hits("stale")?, 0, "no orphan text");
            let done: i64 = c.query_row("SELECT bodies_indexed FROM archives WHERE id=1", [], |r| r.get(0))?;
            assert_eq!(done, 1);
            Ok(())
        })
        .unwrap();
    }
}

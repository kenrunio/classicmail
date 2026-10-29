//! `archive.*` rpc methods.

use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    index,
    list::{self, ListParams},
    props::{self, tag},
    pst::{AttachmentInfo, FullMessage},
    worker::{self, JobKind},
};
use crate::{
    ctx::AppCtx,
    db::util::{get_setting, set_setting},
    mail::{attach_policy, compose, list::ListResult, mime, render},
    rpc::{NoParams, Registry},
};

pub fn register(r: &mut Registry) {
    r.add("archive.open", open);
    r.add("archive.close", close);
    r.add("archive.list", list_archives);
    r.add("archive.folders", folders);
    r.add("archive.listMessages", list_messages);
    r.add("archive.search", search);
    r.add("archive.getMessage", get_message);
    r.add("archive.getHeaders", get_headers);
    r.add("archive.adjacent", adjacent);
    r.add("archive.saveAttachment", save_attachment);
    r.add("archive.saveAllAttachments", save_all_attachments);
    r.add("archive.openAttachment", open_attachment);
    r.add("archive.setIndexText", set_index_text);
    r.add("archive.reindex", reindex);
    r.add("archive.properties", properties);
    r.add("archive.prepareCompose", prepare_compose);
}

// ---- archives -----------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveFolder {
    pub id: i64,
    pub archive_id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub kind: String,
    /// items in the PST folder (PR_CONTENT_COUNT)
    pub content_count: i64,
    /// items indexed
    pub total: i64,
    pub unread: i64,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveDto {
    pub id: i64,
    pub path: String,
    pub display_name: String,
    pub store_name: String,
    pub format: String,
    pub file_size: i64,
    /// pending | indexing | ready | error | missing | locked
    pub status: String,
    pub error: Option<String>,
    pub item_count: i64,
    /// messages indexed so far
    pub indexed: i64,
    /// total items in the archive's mail folders (from the PST)
    pub expected: i64,
    pub index_text: bool,
    pub text_done: bool,
    pub text_indexed: i64,
    pub indexed_at: Option<i64>,
    pub index_ms: Option<i64>,
    pub folders: Vec<ArchiveFolder>,
}

fn folders_of(c: &Connection, archive_id: i64) -> anyhow::Result<Vec<ArchiveFolder>> {
    let mut st = c.prepare(
        "SELECT f.id, f.archive_id, f.parent_id, f.name, f.kind, f.content_count, f.done, \
         COALESCE(x.n, 0), COALESCE(x.u, 0) FROM archive_folders f \
         LEFT JOIN (SELECT folder_id, COUNT(*) AS n, SUM(is_read = 0) AS u FROM archive_messages WHERE archive_id=?1 GROUP BY folder_id) x ON x.folder_id = f.id \
         WHERE f.archive_id=?1 ORDER BY f.position",
    )?;
    let v = st
        .query_map([archive_id], |r| {
            Ok(ArchiveFolder {
                id: r.get(0)?,
                archive_id: r.get(1)?,
                parent_id: r.get(2)?,
                name: r.get(3)?,
                kind: r.get(4)?,
                content_count: r.get(5)?,
                done: r.get::<_, i64>(6)? != 0,
                total: r.get(7)?,
                unread: r.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

fn archives(c: &Connection, with_folders: bool) -> anyhow::Result<Vec<ArchiveDto>> {
    let mut st = c.prepare(
        "SELECT a.id, a.path, a.display_name, a.store_name, a.format, a.file_size, a.status, a.error, a.item_count, \
         (SELECT COUNT(*) FROM archive_messages m WHERE m.archive_id = a.id), \
         (SELECT COALESCE(SUM(content_count), 0) FROM archive_folders f WHERE f.archive_id = a.id AND f.kind = 'mail'), \
         a.index_bodies, a.bodies_indexed, a.indexed_at, a.index_ms FROM archives a ORDER BY a.position, lower(a.display_name), a.id",
    )?;
    let mut v = st
        .query_map([], |r| {
            let ib: i64 = r.get(11)?;
            Ok(ArchiveDto {
                id: r.get(0)?,
                path: r.get(1)?,
                display_name: r.get(2)?,
                store_name: r.get(3)?,
                format: r.get(4)?,
                file_size: r.get(5)?,
                status: r.get(6)?,
                error: r.get(7)?,
                item_count: r.get(8)?,
                indexed: r.get(9)?,
                expected: r.get(10)?,
                index_text: ib != 0,
                text_done: ib == 2,
                text_indexed: r.get(12)?,
                indexed_at: r.get(13)?,
                index_ms: r.get(14)?,
                folders: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if with_folders {
        for a in v.iter_mut() {
            a.folders = folders_of(c, a.id)?;
        }
    }
    Ok(v)
}

/// Display name from the file name ("Archive 2016.pst" → "Archive 2016").
pub fn name_from_path(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "Archive".into())
}

/// Resume / refresh indexing of every archive (once per app run, on the first `archive.list`).
pub async fn resume(ctx: &Arc<AppCtx>) -> anyhow::Result<()> {
    let rows: Vec<(i64, String, String, i64, i64, i64)> = ctx
        .db
        .read(|c| {
            let mut st = c.prepare("SELECT id, path, status, file_size, file_mtime, index_bodies FROM archives ORDER BY position, id")?;
            let v = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await?;
    for (id, path, status, size, mtime, bodies) in rows {
        let changed = match worker::file_stamp(Path::new(&path)) {
            Ok(s) => s != (size, mtime),
            Err(_) => {
                if status != "missing" {
                    ctx.db
                        .write(move |c| {
                            c.execute(
                                "UPDATE archives SET status='missing', error=?2 WHERE id=?1",
                                params![id, format!("The file {path} could not be found.")],
                            )?;
                            Ok(())
                        })
                        .await?;
                }
                continue;
            }
        };
        if changed || status != "ready" {
            worker::enqueue(ctx, id, JobKind::Headers);
        } else if bodies == 1 {
            worker::enqueue(ctx, id, JobKind::Bodies);
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct OpenParams {
    paths: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenError {
    path: String,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenResult {
    ids: Vec<i64>,
    errors: Vec<OpenError>,
}

async fn open(ctx: Arc<AppCtx>, p: OpenParams) -> anyhow::Result<OpenResult> {
    let mut ids = Vec::new();
    let mut errors = Vec::new();
    for raw in p.paths {
        let path = PathBuf::from(raw.trim());
        let check = {
            let path = path.clone();
            tokio::task::spawn_blocking(move || super::pst::sniff(&path)).await?
        };
        if let Err(e) = check {
            let message = e.downcast_ref::<crate::error::UserError>().map(|u| u.message.clone()).unwrap_or_else(|| format!("{e:#}"));
            errors.push(OpenError { path: path.display().to_string(), message });
            continue;
        }
        let path_s = path.display().to_string();
        let name = name_from_path(&path);
        let (id, is_new): (i64, bool) = ctx
            .db
            .write(move |c| {
                if let Some(id) = c.query_row("SELECT id FROM archives WHERE lower(path)=lower(?1)", [&path_s], |r| r.get(0)).optional()? {
                    return Ok((id, false));
                }
                let pos: i64 = c.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM archives", [], |r| r.get(0))?;
                c.execute(
                    "INSERT INTO archives(path, display_name, status, position, added_at) VALUES (?1, ?2, 'pending', ?3, ?4)",
                    params![path_s, name, pos, crate::db::util::now_ms()],
                )?;
                Ok((c.last_insert_rowid(), true))
            })
            .await?;
        if let Some(dir) = path.parent() {
            let dir = dir.display().to_string();
            ctx.db.write(move |c| set_setting(c, "archive.lastFolder", &dir)).await?;
        }
        if is_new || !worker::busy(&ctx, id) {
            worker::enqueue(&ctx, id, JobKind::Headers);
        }
        ids.push(id);
    }
    ctx.events.emit("archive.changed", json!({}));
    Ok(OpenResult { ids, errors })
}

#[derive(Deserialize)]
struct IdParam {
    id: i64,
}

async fn close(ctx: Arc<AppCtx>, p: IdParam) -> anyhow::Result<()> {
    worker::cancel(&ctx, p.id);
    let id = p.id;
    let path: Option<String> = ctx
        .db
        .write(move |c| {
            let path: Option<String> = c.query_row("SELECT path FROM archives WHERE id=?1", [id], |r| r.get(0)).optional()?;
            let tx = c.transaction()?;
            index::clear_index(&tx, id)?;
            tx.execute("DELETE FROM archives WHERE id=?1", [id])?;
            tx.commit()?;
            Ok(path)
        })
        .await?;
    if let Some(path) = path {
        worker::forget(PathBuf::from(path));
    }
    ctx.events.emit("archive.changed", json!({ "archiveId": id }));
    Ok(())
}

async fn list_archives(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<Vec<ArchiveDto>> {
    // first call of this run: pick up unfinished / changed archives
    static RESUMED: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();
    let key = Arc::as_ptr(&ctx) as usize;
    let first = {
        let mut seen = RESUMED.get_or_init(Default::default).lock();
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    };
    if first && let Err(e) = resume(&ctx).await {
        log::warn!("archive resume: {}", crate::logging::err(&e));
    }
    ctx.db.read(|c| archives(c, true)).await
}

async fn folders(ctx: Arc<AppCtx>, p: IdParam) -> anyhow::Result<Vec<ArchiveFolder>> {
    ctx.db.read(move |c| folders_of(c, p.id)).await
}

async fn list_messages(ctx: Arc<AppCtx>, p: ListParams) -> anyhow::Result<ListResult> {
    ctx.db.read(move |c| list::list(c, &p)).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchParams {
    query: String,
    archive_id: Option<i64>,
    #[serde(default)]
    offset: i64,
    #[serde(default)]
    limit: Option<i64>,
}

/// Search All Archives (or one archive): newest first.
async fn search(ctx: Arc<AppCtx>, p: SearchParams) -> anyhow::Result<ListResult> {
    let lp = ListParams {
        source: list::Source { folder_id: None, archive_id: p.archive_id },
        search: Some(p.query),
        scope: Some(if p.archive_id.is_some() { "archive".into() } else { "all".into() }),
        offset: p.offset,
        limit: p.limit.unwrap_or(200),
        ..Default::default()
    };
    ctx.db.read(move |c| list::list(c, &lp)).await
}

// ---- messages -----------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct MsgRow {
    archive_id: i64,
    archive_name: String,
    path: String,
    node_id: u32,
    folder_id: i64,
    folder_name: String,
    subject: String,
    from_name: String,
    from_email: String,
    date: i64,
    size: i64,
    importance: i64,
    flag_status: Option<String>,
    is_read: bool,
    message_class: String,
    last_verb: Option<String>,
}

fn msg_row(c: &Connection, id: i64) -> anyhow::Result<MsgRow> {
    c.query_row(
        "SELECT m.archive_id, a.display_name, a.path, m.node_id, m.folder_id, f.name, m.subject, m.from_name, m.from_email, m.date, m.size, \
         m.importance, m.flag_status, m.is_read, m.message_class, m.last_verb \
         FROM archive_messages m JOIN archives a ON a.id = m.archive_id JOIN archive_folders f ON f.id = m.folder_id WHERE m.id=?1",
        [id],
        |r| {
            Ok(MsgRow {
                archive_id: r.get(0)?,
                archive_name: r.get(1)?,
                path: r.get(2)?,
                node_id: r.get::<_, i64>(3)? as u32,
                folder_id: r.get(4)?,
                folder_name: r.get(5)?,
                subject: r.get(6)?,
                from_name: r.get(7)?,
                from_email: r.get(8)?,
                date: r.get(9)?,
                size: r.get(10)?,
                importance: r.get(11)?,
                flag_status: r.get(12)?,
                is_read: r.get::<_, i64>(13)? != 0,
                message_class: r.get(14)?,
                last_verb: r.get(15)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| crate::user_err!("not_found", "This item is no longer in the archive index."))
}

/// The last few messages read (attachments are opened / saved right after showing a message).
fn cache() -> &'static Mutex<VecDeque<((String, u32), Arc<FullMessage>)>> {
    static C: OnceLock<Mutex<VecDeque<((String, u32), Arc<FullMessage>)>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

async fn read_full(path: &str, nid: u32) -> anyhow::Result<Arc<FullMessage>> {
    let key = (path.to_string(), nid);
    if let Some((_, m)) = cache().lock().iter().find(|(k, _)| *k == key) {
        return Ok(m.clone());
    }
    let m = worker::with_pst(PathBuf::from(path), move |p| p.message(nid, true)).await.map_err(|e| {
        if e.downcast_ref::<crate::error::UserError>().is_some() {
            e
        } else {
            crate::user_err!("read", "The item could not be read from the archive: {e:#}")
        }
    })?;
    let m = Arc::new(m);
    let mut c = cache().lock();
    c.retain(|(k, _)| *k != key);
    c.push_back((key, m.clone()));
    while c.len() > 3 {
        c.pop_front();
    }
    Ok(m)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Addr {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDto {
    /// index in the message's attachment table
    pub index: usize,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub is_inline: bool,
    pub content_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveMessageView {
    pub id: i64,
    pub archive_id: i64,
    pub archive_name: String,
    pub folder_id: i64,
    pub folder_name: String,
    pub message_class: String,
    pub subject: String,
    pub from_name: String,
    pub from_email: String,
    pub to: Vec<Addr>,
    pub cc: Vec<Addr>,
    pub bcc: Vec<Addr>,
    pub sent_at: Option<i64>,
    pub received_at: i64,
    pub importance: i64,
    pub flag_status: Option<String>,
    pub is_read: bool,
    pub last_verb: Option<String>,
    pub size: i64,
    pub html: String,
    pub is_plain: bool,
    pub text: String,
    pub remote_blocked: bool,
    pub has_remote: bool,
    pub attachments: Vec<AttachmentDto>,
    /// "html" | "rtf-html" | "rtf" | "text" | "none"
    pub body_source: String,
    pub message_id_header: Option<String>,
}

fn cid_key(cid: &str) -> String {
    cid.trim().trim_start_matches('<').trim_end_matches('>').to_ascii_lowercase()
}

fn recipients(m: &FullMessage) -> (Vec<Addr>, Vec<Addr>, Vec<Addr>) {
    let (mut to, mut cc, mut bcc) = (Vec::new(), Vec::new(), Vec::new());
    for r in &m.recipients {
        let a = Addr { name: r.name.clone(), email: r.email.clone() };
        match r.kind {
            2 => cc.push(a),
            3 => bcc.push(a),
            _ => to.push(a),
        }
    }
    (to, cc, bcc)
}

/// Sender of a message (SMTP address; X.500 senders via the transport headers).
fn sender(m: &FullMessage, row_name: &str, row_email: &str) -> (String, String) {
    let h = props::header(&m.props);
    let mut name = if h.from_name.is_empty() { row_name.to_string() } else { h.from_name.clone() };
    let mut email = if h.from_email.is_empty() { row_email.to_string() } else { h.from_email.clone() };
    if email.is_empty()
        && let Some(from) = m.props.str(tag::TRANSPORT_HEADERS).and_then(|hs| props::transport_header(&hs, "From"))
        && let Some(a) = mime::parse_addresses(&from).into_iter().next()
    {
        email = a.email.clone();
        if name.is_empty() {
            name = a.name.clone();
        }
    }
    (name, email)
}

fn image_mime(a: &AttachmentInfo) -> Option<String> {
    let m = a.mime_type.trim().to_ascii_lowercase();
    let m = if m.starts_with("image/") { m } else { mime_guess::from_path(&a.filename).first()?.essence_str().to_string() };
    let ok = m.starts_with("image/")
        && m != "image/svg+xml"
        && m.len() < 60
        && m.chars().all(|c| c.is_ascii_alphanumeric() || "/+.-".contains(c));
    ok.then_some(m)
}

/// cid → data: URL of inline pictures (bounded), and the set of referenced cids.
fn inline_images(m: &FullMessage, html: &str) -> HashMap<String, String> {
    use base64::Engine;
    let lower = html.to_ascii_lowercase();
    let mut out = HashMap::new();
    let mut budget: usize = 12 * 1024 * 1024;
    for a in &m.attachments {
        let Some(cid) = a.content_id.as_deref().map(cid_key).filter(|c| !c.is_empty()) else { continue };
        if !lower.contains(&format!("cid:{cid}")) {
            continue;
        }
        let (Some(data), Some(mime)) = (a.data.as_ref(), image_mime(a)) else { continue };
        if data.len() > budget {
            continue;
        }
        budget -= data.len();
        out.insert(cid, format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(data)));
    }
    out
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetParams {
    id: i64,
    #[serde(default)]
    allow_remote: bool,
}

async fn get_message(ctx: Arc<AppCtx>, p: GetParams) -> anyhow::Result<ArchiveMessageView> {
    let id = p.id;
    let (row, block_setting) =
        ctx.db.read(move |c| Ok((msg_row(c, id)?, get_setting::<bool>(c, "mail.blockRemoteImages")?.unwrap_or(true)))).await?;
    let m = match read_full(&row.path, row.node_id).await {
        Ok(m) => m,
        Err(e) if e.downcast_ref::<crate::error::UserError>().is_some_and(|u| u.code == "read") => {
            // an item the reader can't open (e.g. no sub-node tree): show what the index knows
            log::info!("archive message {id}: {}", crate::logging::err(&e));
            let (to_display, cc_display): (String, String) = ctx
                .db
                .read(move |c| {
                    Ok(c.query_row("SELECT to_display, cc_display FROM archive_messages WHERE id=?1", [id], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })?)
                })
                .await?;
            let names = |s: &str| {
                s.split(';')
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(|n| Addr { name: n.to_string(), email: String::new() })
                    .collect::<Vec<_>>()
            };
            return Ok(ArchiveMessageView {
                id,
                archive_id: row.archive_id,
                archive_name: row.archive_name.clone(),
                folder_id: row.folder_id,
                folder_name: row.folder_name.clone(),
                message_class: row.message_class.clone(),
                subject: row.subject.clone(),
                from_name: row.from_name.clone(),
                from_email: row.from_email.clone(),
                to: names(&to_display),
                cc: names(&cc_display),
                bcc: vec![],
                sent_at: None,
                received_at: row.date,
                importance: row.importance,
                flag_status: row.flag_status.clone(),
                is_read: row.is_read,
                last_verb: row.last_verb.clone(),
                size: row.size,
                html: mime::text_to_html("The contents of this item could not be read from the archive."),
                is_plain: true,
                text: String::new(),
                remote_blocked: false,
                has_remote: false,
                attachments: vec![],
                body_source: "unreadable".into(),
                message_id_header: None,
            });
        }
        Err(e) => return Err(e),
    };
    let view = tokio::task::spawn_blocking(move || build_view(id, &row, &m, p.allow_remote || !block_setting)).await?;
    Ok(view)
}

fn build_view(id: i64, row: &MsgRow, m: &FullMessage, allow: bool) -> ArchiveMessageView {
    build_view_parts(id, row, m, allow).0
}

/// `build_view`, plus the original HTML body and its inline pictures (cid → data: URL) for a quote.
fn build_view_parts(id: i64, row: &MsgRow, m: &FullMessage, allow: bool) -> (ArchiveMessageView, Option<String>, HashMap<String, String>) {
    let h = props::header(&m.props);
    let body = props::body(&m.props);
    let (from_name, from_email) = sender(m, &row.from_name, &row.from_email);
    let (to, cc, bcc) = recipients(m);
    let (html, plain, blocked, has_remote, referenced, cids) = match &body.html {
        Some(src) => {
            let cids = inline_images(m, src);
            let r = render::render_html(src, &cids, allow);
            let lower = src.to_ascii_lowercase();
            (r.html, false, r.blocked, r.has_remote, lower, cids)
        }
        None => (mime::text_to_html(&body.text), true, false, false, String::new(), HashMap::new()),
    };
    let attachments = m
        .attachments
        .iter()
        .filter(|a| !a.hidden || a.content_id.is_none())
        .map(|a| {
            let cid = a.content_id.as_deref().map(cid_key);
            let inline = cid.as_ref().is_some_and(|c| !c.is_empty() && referenced.contains(&format!("cid:{c}"))) || a.hidden;
            AttachmentDto {
                index: a.index,
                filename: attach_policy::display_name(&a.filename),
                mime_type: a.mime_type.clone(),
                size: a.size,
                is_inline: inline,
                content_id: a.content_id.clone(),
            }
        })
        .collect();
    let subject = if h.subject.is_empty() { row.subject.clone() } else { h.subject.clone() };
    let view = ArchiveMessageView {
        id,
        archive_id: row.archive_id,
        archive_name: row.archive_name.clone(),
        folder_id: row.folder_id,
        folder_name: row.folder_name.clone(),
        message_class: row.message_class.clone(),
        subject,
        from_name,
        from_email,
        to,
        cc,
        bcc,
        sent_at: h.sent,
        received_at: if h.date > 0 { h.date } else { row.date },
        importance: row.importance,
        flag_status: row.flag_status.clone(),
        is_read: row.is_read,
        last_verb: row.last_verb.clone(),
        size: if h.size > 0 { h.size } else { row.size },
        html,
        is_plain: plain,
        text: body.text.clone(),
        remote_blocked: blocked,
        has_remote,
        attachments,
        body_source: body.source.to_string(),
        message_id_header: m.props.text(tag::INTERNET_MESSAGE_ID),
    };
    (view, body.html, cids)
}

async fn get_headers(ctx: Arc<AppCtx>, p: IdParam) -> anyhow::Result<String> {
    let id = p.id;
    let row = ctx.db.read(move |c| msg_row(c, id)).await?;
    let m = read_full(&row.path, row.node_id).await?;
    Ok(m.props.str(tag::TRANSPORT_HEADERS).unwrap_or_default())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdjacentParams {
    id: i64,
    direction: String,
}

async fn adjacent(ctx: Arc<AppCtx>, p: AdjacentParams) -> anyhow::Result<Option<i64>> {
    ctx.db.read(move |c| list::adjacent(c, p.id, p.direction != "previous")).await
}

// ---- attachments --------------------------------------------------------------------------------

/// (file name, bytes) of an attachment; embedded messages become .eml files.
pub(crate) async fn attachment_bytes(ctx: &Arc<AppCtx>, id: i64, index: usize) -> anyhow::Result<(String, Vec<u8>)> {
    let row = ctx.db.read(move |c| msg_row(c, id)).await?;
    let m = read_full(&row.path, row.node_id).await?;
    let a = m
        .attachments
        .iter()
        .find(|a| a.index == index)
        .ok_or_else(|| crate::user_err!("not_found", "The attachment could not be found."))?;
    let (name, bytes) = bytes_of(a)?;
    Ok((name, bytes.into_owned()))
}

/// (file name, bytes) of an attachment of a message already read; embedded messages become .eml files.
fn bytes_of(a: &AttachmentInfo) -> anyhow::Result<(String, Cow<'_, [u8]>)> {
    let name = attach_policy::display_name(&a.filename);
    if let Some(inner) = &a.embedded {
        return Ok((name, Cow::Owned(embedded_eml(inner)?)));
    }
    let data = a
        .data
        .as_deref()
        .ok_or_else(|| crate::user_err!("unsupported", "\"{name}\" is not stored in the archive (it was a link to a file)."))?;
    Ok((name, Cow::Borrowed(data)))
}

/// An attached Outlook item as an RFC 822 message (headers + body; its own attachments are left out).
fn embedded_eml(m: &FullMessage) -> anyhow::Result<Vec<u8>> {
    use mail_builder::{
        MessageBuilder,
        headers::{address::Address, raw::Raw, text::Text},
    };
    let h = props::header(&m.props);
    let body = props::body(&m.props);
    let (to, cc, _) = recipients(m);
    let (from_name, from_email) = sender(m, "", "");
    let addr = |v: &[Addr]| {
        Address::new_list(
            v.iter()
                .filter(|a| !a.email.is_empty() || !a.name.is_empty())
                .map(|a| {
                    Address::new_address(
                        if a.name.is_empty() { None } else { Some(a.name.clone()) },
                        if a.email.is_empty() { a.name.clone() } else { a.email.clone() },
                    )
                })
                .collect(),
        )
    };
    let mut b = MessageBuilder::new().subject(Text::new(h.subject.clone()));
    if !from_email.is_empty() || !from_name.is_empty() {
        b = b.from(addr(&[Addr { name: from_name, email: from_email }]));
    }
    if !to.is_empty() {
        b = b.to(addr(&to));
    }
    if !cc.is_empty() {
        b = b.cc(addr(&cc));
    }
    if let Some(d) = h.sent.or(Some(h.date)).filter(|d| *d > 0).and_then(chrono::DateTime::from_timestamp_millis) {
        b = b.header("Date", Raw::new(d.format("%a, %d %b %Y %H:%M:%S +0000").to_string()));
    }
    b = match &body.html {
        Some(html) => b.html_body(html.clone()).text_body(body.text.clone()),
        None => b.text_body(body.text.clone()),
    };
    Ok(b.write_to_vec()?)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveParams {
    id: i64,
    index: usize,
    path: String,
}

async fn save_attachment(ctx: Arc<AppCtx>, p: SaveParams) -> anyhow::Result<String> {
    let (_, bytes) = attachment_bytes(&ctx, p.id, p.index).await?;
    attach_policy::write_marked(Path::new(&p.path), &bytes).map_err(|e| crate::user_err!("io", "Can't save {}: {e}", p.path))?;
    Ok(p.path)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveAllParams {
    id: i64,
    dir: String,
}

async fn save_all_attachments(ctx: Arc<AppCtx>, p: SaveAllParams) -> anyhow::Result<Vec<String>> {
    let id = p.id;
    let row = ctx.db.read(move |c| msg_row(c, id)).await?;
    let m = read_full(&row.path, row.node_id).await?;
    let dir = PathBuf::from(&p.dir);
    // body decoding and file writes are blocking work
    tokio::task::spawn_blocking(move || {
        let html = props::body(&m.props).html.unwrap_or_default().to_ascii_lowercase();
        let mut out = Vec::new();
        for a in m.attachments.iter() {
            let inline =
                a.hidden || a.content_id.as_deref().map(cid_key).is_some_and(|c| !c.is_empty() && html.contains(&format!("cid:{c}")));
            if inline {
                continue;
            }
            let Ok((name, bytes)) = bytes_of(a) else { continue };
            let safe = attach_policy::safe_file_name(&name);
            let mut path = dir.join(&safe);
            let mut n = 1;
            while path.exists() {
                let stem = Path::new(&safe).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let ext = Path::new(&safe).extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
                path = dir.join(format!("{stem} ({n}){ext}"));
                n += 1;
            }
            attach_policy::write_marked(&path, &bytes).map_err(|e| crate::user_err!("io", "Can't save {}: {e}", path.display()))?;
            out.push(path.display().to_string());
        }
        Ok(out)
    })
    .await?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenAttParams {
    id: i64,
    index: usize,
    #[serde(default)]
    confirmed: bool,
    #[serde(default)]
    choose: bool,
}

async fn open_attachment(ctx: Arc<AppCtx>, p: OpenAttParams) -> anyhow::Result<String> {
    let (name, bytes) = attachment_bytes(&ctx, p.id, p.index).await?;
    attach_policy::check_open(&name, p.confirmed)?;
    let dir = std::env::temp_dir().join("ClassicMail").join(format!("archive-{}-{}", p.id, p.index));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(attach_policy::safe_file_name(&name));
    attach_policy::write_marked(&path, &bytes)?;
    let s = path.to_string_lossy().into_owned();
    attach_policy::launch(ctx.app.as_ref(), &s, &name, p.choose)?;
    Ok(s)
}

// ---- archive commands ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct IndexTextParams {
    id: i64,
    on: bool,
}

async fn set_index_text(ctx: Arc<AppCtx>, p: IndexTextParams) -> anyhow::Result<()> {
    let id = p.id;
    let on = p.on;
    ctx.db
        .write(move |c| {
            if on {
                c.execute("UPDATE archives SET index_bodies=CASE WHEN index_bodies=2 THEN 2 ELSE 1 END WHERE id=?1", [id])?;
            } else {
                c.execute("UPDATE archives SET index_bodies=0 WHERE id=?1", [id])?;
            }
            Ok(())
        })
        .await?;
    if on {
        worker::enqueue(&ctx, id, JobKind::Bodies);
    }
    ctx.events.emit("archive.changed", json!({ "archiveId": id }));
    Ok(())
}

/// Try again (after "open in Outlook" / missing). Finished folders are kept; the index is only
/// rebuilt when the file changed.
async fn reindex(ctx: Arc<AppCtx>, p: IdParam) -> anyhow::Result<()> {
    let id = p.id;
    ctx.db
        .write(move |c| {
            c.execute("UPDATE archives SET status='pending', error=NULL WHERE id=?1", [id])?;
            Ok(())
        })
        .await?;
    worker::enqueue(&ctx, id, JobKind::Headers);
    ctx.events.emit("archive.changed", json!({ "archiveId": id }));
    Ok(())
}

async fn properties(ctx: Arc<AppCtx>, p: IdParam) -> anyhow::Result<ArchiveDto> {
    let id = p.id;
    ctx.db
        .read(move |c| {
            archives(c, true)?.into_iter().find(|a| a.id == id).ok_or_else(|| crate::user_err!("not_found", "The archive is not open."))
        })
        .await
}

// ---- reply / forward ----------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareParams {
    id: i64,
    mode: compose::model::Mode,
    account_id: Option<i64>,
}

async fn prepare_compose(ctx: Arc<AppCtx>, p: PrepareParams) -> anyhow::Result<compose::prepare::Prepared> {
    use compose::{
        model::{Addr as CAddr, AttachmentSpec, Compose, Mode},
        prepare::{body_with_signature, prefixed_subject, quoted_block, reply_recipients, sanitize, signature_for},
        store::{SourceMsg, from_choices, own_addresses},
    };
    if !matches!(p.mode, Mode::Reply | Mode::ReplyAll | Mode::Forward) {
        return Err(crate::user_err!("bad_params", "Only Reply, Reply All and Forward work with archived items."));
    }
    let id = p.id;
    let (row, choices, own, default_acct) = ctx
        .db
        .read(move |c| Ok((msg_row(c, id)?, from_choices(c)?, own_addresses(c)?, get_setting::<i64>(c, "mail.defaultAccountId")?)))
        .await?;
    let m = read_full(&row.path, row.node_id).await?;
    let account_id = p
        .account_id
        .or(default_acct.filter(|a| choices.iter().any(|c| c.account_id == *a)))
        .or_else(|| choices.first().map(|c| c.account_id))
        .ok_or_else(|| crate::user_err!("no_account", "Add an e-mail account first (Tools › Account Settings)."))?;
    let (view, html, cids) = {
        let (row, m) = (row.clone(), m.clone());
        tokio::task::spawn_blocking(move || build_view_parts(id, &row, &m, false)).await?
    };
    let ca = |a: &Addr| CAddr::new(a.name.clone(), a.email.clone());
    let src = SourceMsg {
        id: 0,
        account_id,
        subject: view.subject.clone(),
        from: CAddr::new(view.from_name.clone(), view.from_email.clone()),
        to: view.to.iter().map(ca).collect(),
        cc: view.cc.iter().map(ca).collect(),
        date: view.sent_at.unwrap_or(view.received_at),
        message_id_header: view.message_id_header.clone(),
        html,
        text: Some(view.text.clone()),
        ..Default::default()
    };
    // the quote: the original HTML with its pictures inlined, else the text
    let original = match &src.html {
        Some(h) => {
            let mut s = sanitize(h);
            for (cid, url) in &cids {
                s = replace_ci(&s, &format!("cid:{cid}"), url);
            }
            s
        }
        None => mime::text_to_html(src.text.as_deref().unwrap_or("")),
    };
    let quoted = quoted_block(&src, &original);
    let reply = matches!(p.mode, Mode::Reply | Mode::ReplyAll);
    let sig = ctx.db.read(move |c| signature_for(c, account_id, reply)).await?;
    let mut c = Compose {
        account_id,
        from: choices.iter().find(|c| c.account_id == account_id && c.primary).map(|c| c.email.clone()),
        subject: prefixed_subject(&src.subject, p.mode),
        mode: p.mode,
        ..Default::default()
    };
    if reply {
        let (to, cc) = reply_recipients(&src, p.mode == Mode::ReplyAll, &own);
        c.to = to.into_iter().filter(|a| !a.email.is_empty()).collect();
        c.cc = cc.into_iter().filter(|a| !a.email.is_empty()).collect();
    } else {
        // forward: the attachments, extracted to a temp folder
        let dir = ctx.data_dir.join("archive-forward").join(uuid::Uuid::new_v4().to_string());
        let wanted: Vec<(usize, String)> =
            view.attachments.iter().filter(|a| !a.is_inline).map(|a| (a.index, a.mime_type.clone())).collect();
        let files = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<AttachmentSpec>> {
            let mut out = Vec::new();
            for (index, mime_type) in wanted {
                let Some(a) = m.attachments.iter().find(|a| a.index == index) else { continue };
                let Ok((name, bytes)) = bytes_of(a) else { continue };
                std::fs::create_dir_all(&dir)?;
                let path = dir.join(attach_policy::safe_file_name(&name));
                std::fs::write(&path, &bytes)?;
                out.push(AttachmentSpec {
                    path: Some(path.display().to_string()),
                    name: Some(name),
                    size: Some(bytes.len() as i64),
                    mime_type: Some(mime_type),
                    ..Default::default()
                });
            }
            Ok(out)
        })
        .await??;
        c.attachments.extend(files);
    }
    Ok(compose::prepare::Prepared {
        compose: c,
        body_html: body_with_signature(sig.as_deref()),
        quoted_html: quoted,
        from_choices: choices,
        draft_message_id: None,
        reply_targets: None,
    })
}

/// Case-insensitive replace of an ASCII needle.
fn replace_ci(hay: &str, needle: &str, with: &str) -> String {
    let lower = hay.to_ascii_lowercase();
    let n = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(hay.len());
    let mut last = 0;
    for (i, _) in lower.match_indices(&n) {
        out.push_str(&hay[last..i]);
        out.push_str(with);
        last = i + n.len();
    }
    out.push_str(&hay[last..]);
    out
}

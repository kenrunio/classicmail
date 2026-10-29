//! SQLite side of mail: Gmail labels → folders, message rows from Gmail metadata, label changes →
//! read/flag state and folder membership, folder counts, full-text index rows.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::mime::{self, Addr};
use crate::google::gmail::{Label, Message};

pub const ROLE_INBOX: &str = "inbox";
pub const ROLE_SENT: &str = "sent";
pub const ROLE_DRAFTS: &str = "drafts";
pub const ROLE_TRASH: &str = "trash";
pub const ROLE_SPAM: &str = "spam";
pub const ROLE_OUTBOX: &str = "outbox";
pub const ROLE_ALL: &str = "all";
pub const ROLE_USER: &str = "user";
/// intermediate folder for a nested label whose parent label doesn't exist
pub const ROLE_PLACEHOLDER: &str = "placeholder";

/// System label → (role, display name, position).
pub fn system_folder(label_id: &str) -> Option<(&'static str, &'static str, i64)> {
    Some(match label_id {
        "TRASH" => (ROLE_TRASH, "Deleted Items", 1),
        "DRAFT" => (ROLE_DRAFTS, "Drafts", 2),
        "INBOX" => (ROLE_INBOX, "Inbox", 3),
        "SPAM" => (ROLE_SPAM, "Junk E-mail", 4),
        "SENT" => (ROLE_SENT, "Sent Items", 6),
        _ => return None,
    })
}

/// Labels that never become folders (state flags / Gmail categories).
pub fn hidden_label(id: &str) -> bool {
    matches!(id, "UNREAD" | "STARRED" | "IMPORTANT" | "CHAT") || id.starts_with("CATEGORY_")
}

#[derive(Debug, Clone, Default)]
pub struct LabelMap {
    /// gmail label id → folder id
    pub by_label: HashMap<String, i64>,
    pub folder_label: HashMap<i64, String>,
    pub roles: HashMap<i64, String>,
}

impl LabelMap {
    pub fn load(c: &Connection, account_id: i64) -> anyhow::Result<LabelMap> {
        let mut st = c.prepare("SELECT id, gmail_label_id, role FROM folders WHERE account_id=?1")?;
        let mut m = LabelMap::default();
        for r in st.query_map([account_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?)))? {
            let (id, label, role) = r?;
            if let Some(l) = label {
                m.by_label.insert(l.clone(), id);
                m.folder_label.insert(id, l);
            }
            m.roles.insert(id, role);
        }
        Ok(m)
    }

    pub fn folder_of_role(&self, role: &str) -> Option<i64> {
        self.roles.iter().find(|(_, r)| r.as_str() == role).map(|(id, _)| *id)
    }

    /// Folder membership implied by a label set (Trash and Junk are exclusive, like Gmail views).
    pub fn folders_for(&self, labels: &[String]) -> Vec<i64> {
        let has = |l: &str| labels.iter().any(|x| x == l);
        if has("TRASH") {
            return self.by_label.get("TRASH").copied().into_iter().collect();
        }
        if has("SPAM") {
            return self.by_label.get("SPAM").copied().into_iter().collect();
        }
        labels.iter().filter_map(|l| self.by_label.get(l).copied()).collect()
    }
}

pub fn location_of(labels: &[String]) -> i64 {
    if labels.iter().any(|l| l == "TRASH") {
        1
    } else if labels.iter().any(|l| l == "SPAM") {
        2
    } else {
        0
    }
}

/// Bring `folders` in line with Gmail's labels (create, rename/re-parent, delete). Also makes sure
/// the local Outbox and the virtual All Mail folder exist. Returns true if anything changed.
pub fn sync_labels(c: &mut Connection, account_id: i64, labels: &[Label], show_all_mail: bool) -> anyhow::Result<bool> {
    let tx = c.transaction()?;
    let mut changed = false;
    let mut existing: HashMap<String, (i64, String, Option<i64>)> = HashMap::new();
    {
        let mut st =
            tx.prepare("SELECT id, gmail_label_id, path, parent_id FROM folders WHERE account_id=?1 AND gmail_label_id IS NOT NULL")?;
        for r in st.query_map([account_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<i64>>(3)?))
        })? {
            let (id, l, p, parent) = r?;
            existing.insert(l, (id, p, parent));
        }
    }
    let wanted: Vec<&Label> =
        labels.iter().filter(|l| system_folder(&l.id).is_some() || (l.kind == "user" && !hidden_label(&l.id))).collect();
    let wanted_ids: HashSet<&str> = wanted.iter().map(|l| l.id.as_str()).collect();
    // delete folders whose label is gone (cascade removes membership)
    for (label, (id, _, _)) in &existing {
        if !wanted_ids.contains(label.as_str()) {
            tx.execute("DELETE FROM folders WHERE id=?1", [id])?;
            changed = true;
        }
    }
    // system folders
    for l in &wanted {
        if let Some((role, name, pos)) = system_folder(&l.id)
            && !existing.contains_key(&l.id)
        {
            tx.execute(
                "INSERT INTO folders(account_id, gmail_label_id, name, path, role, position) VALUES (?1,?2,?3,?3,?4,?5)",
                params![account_id, l.id, name, role, pos],
            )?;
            changed = true;
        }
    }
    // local Outbox + virtual All Mail
    for (role, name, pos) in [(ROLE_OUTBOX, "Outbox", 5i64), (ROLE_ALL, "All Mail", 7)] {
        let id: Option<i64> =
            tx.query_row("SELECT id FROM folders WHERE account_id=?1 AND role=?2", params![account_id, role], |r| r.get(0)).optional()?;
        let hidden = role == ROLE_ALL && !show_all_mail;
        match id {
            None => {
                tx.execute(
                    "INSERT INTO folders(account_id, gmail_label_id, name, path, role, position, hidden) VALUES (?1, NULL, ?2, ?2, ?3, ?4, ?5)",
                    params![account_id, name, role, pos, hidden],
                )?;
                changed = true;
            }
            Some(id) => {
                tx.execute("UPDATE folders SET hidden=?1 WHERE id=?2", params![hidden, id])?;
            }
        }
    }
    // user labels sorted by path so parents come first
    let mut users: Vec<&&Label> = wanted.iter().filter(|l| system_folder(&l.id).is_none()).collect();
    users.sort_by_key(|a| a.name.to_lowercase());
    let mut path_ids: HashMap<String, i64> = HashMap::new();
    {
        let mut st = tx.prepare("SELECT id, path FROM folders WHERE account_id=?1 AND role IN ('user','placeholder')")?;
        for r in st.query_map([account_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (id, p) = r?;
            path_ids.insert(p.to_lowercase(), id);
        }
    }
    let mut used_placeholders: HashSet<i64> = HashSet::new();
    for l in users {
        let path = l.name.trim_matches('/').to_string();
        let segs: Vec<&str> = path.split('/').collect();
        let name = segs.last().copied().unwrap_or(&path).to_string();
        // parent: nearest existing ancestor path; create placeholders for missing intermediate levels
        let mut parent: Option<i64> = None;
        for i in 1..segs.len() {
            let ppath = segs[..i].join("/");
            let key = ppath.to_lowercase();
            let pid = match path_ids.get(&key) {
                Some(id) => *id,
                None => {
                    tx.execute(
                        "INSERT INTO folders(account_id, gmail_label_id, name, path, parent_id, role, position) VALUES (?1, NULL, ?2, ?3, ?4, 'placeholder', 100)",
                        params![account_id, segs[i - 1], ppath, parent],
                    )?;
                    let id = tx.last_insert_rowid();
                    path_ids.insert(key, id);
                    changed = true;
                    id
                }
            };
            used_placeholders.insert(pid);
            parent = Some(pid);
        }
        match existing.get(&l.id) {
            Some((id, old_path, old_parent)) => {
                if *old_path != path || *old_parent != parent {
                    tx.execute("UPDATE folders SET name=?1, path=?2, parent_id=?3 WHERE id=?4", params![name, path, parent, id])?;
                    changed = true;
                }
                path_ids.insert(path.to_lowercase(), *id);
            }
            None => {
                tx.execute(
                    "INSERT INTO folders(account_id, gmail_label_id, name, path, parent_id, role, position) VALUES (?1,?2,?3,?4,?5,'user',100)",
                    params![account_id, l.id, name, path, parent],
                )?;
                path_ids.insert(path.to_lowercase(), tx.last_insert_rowid());
                changed = true;
            }
        }
    }
    // placeholder folders that are no longer needed (or that a real label now covers)
    let placeholders: Vec<(i64, String)> = {
        let mut st = tx.prepare("SELECT id, path FROM folders WHERE account_id=?1 AND role='placeholder'")?;
        st.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
    };
    for (id, path) in placeholders {
        let real = path_ids.get(&path.to_lowercase()).copied().filter(|x| *x != id);
        if let Some(real) = real {
            tx.execute("UPDATE folders SET parent_id=?1 WHERE parent_id=?2", params![real, id])?;
        }
        if real.is_some() || !used_placeholders.contains(&id) {
            tx.execute("DELETE FROM folders WHERE id=?1", [id])?;
            changed = true;
        }
    }
    tx.commit()?;
    Ok(changed)
}

/// Fields of a message as stored from Gmail `format=metadata` (or `full`).
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub gmail_id: String,
    pub thread_id: String,
    pub history_id: i64,
    pub internal_date: i64,
    pub sent_date: Option<i64>,
    pub subject: String,
    pub from: Addr,
    pub to: Vec<Addr>,
    pub cc: Vec<Addr>,
    pub bcc: Vec<Addr>,
    pub reply_to: Vec<Addr>,
    pub snippet: String,
    pub labels: Vec<String>,
    pub size: i64,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub importance: i64,
    pub has_attachments: bool,
}

/// Headers asked for with `format=metadata`.
pub const METADATA_HEADERS: &[&str] = &[
    "From",
    "To",
    "Cc",
    "Bcc",
    "Reply-To",
    "Subject",
    "Date",
    "Message-ID",
    "In-Reply-To",
    "References",
    "Importance",
    "X-Priority",
    "Priority",
    "Content-Type",
];

pub fn meta_from(m: &Message) -> Meta {
    let h = |n: &str| m.header(n).map(|v| mime::decode_rfc2047(v.trim()));
    // addresses are split first and each display name decoded after: a decoded name may contain
    // a comma ("=?UTF-8?B?...?=" -> "Cohen, Dana")
    let addrs = |n: &str| m.header(n).map(mime::parse_addresses).unwrap_or_default();
    let from = addrs("From").into_iter().next().unwrap_or_default();
    let ct = m.payload.as_ref().map(|p| p.mime_type.to_ascii_lowercase()).unwrap_or_default();
    let ct_hdr = h("Content-Type").unwrap_or_default().to_ascii_lowercase();
    let has_att = ct == "multipart/mixed" || ct_hdr.starts_with("multipart/mixed") || payload_has_attachment(m.payload.as_ref());
    Meta {
        gmail_id: m.id.clone(),
        thread_id: m.thread_id.clone(),
        history_id: m.history_id.parse().unwrap_or(0),
        internal_date: m.internal_date_ms(),
        sent_date: h("Date").and_then(|d| mime::parse_date(&d)),
        subject: mime::strip_bidi_overrides(&h("Subject").unwrap_or_default().replace(['\r', '\n'], " ")),
        from,
        to: addrs("To"),
        cc: addrs("Cc"),
        bcc: addrs("Bcc"),
        reply_to: addrs("Reply-To"),
        snippet: mime::unescape_entities(&m.snippet),
        labels: m.label_ids.clone(),
        size: m.size_estimate,
        message_id: h("Message-ID").map(|s| s.trim().to_string()),
        in_reply_to: h("In-Reply-To").map(|s| s.trim().to_string()),
        references: h("References").map(|s| s.trim().to_string()),
        importance: mime::importance(h("Importance").as_deref(), h("X-Priority").as_deref(), h("Priority").as_deref()),
        has_attachments: has_att,
    }
}

fn payload_has_attachment(p: Option<&crate::google::gmail::MessagePart>) -> bool {
    let Some(p) = p else { return false };
    if !p.filename.is_empty() && p.body.attachment_id.is_some() {
        return true;
    }
    p.parts.iter().any(|c| payload_has_attachment(Some(c)))
}

fn addr_json(v: &[Addr]) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

fn to_display(to: &[Addr]) -> String {
    to.iter().map(|a| a.display().to_string()).collect::<Vec<_>>().join("; ")
}

fn has(labels: &[String], l: &str) -> bool {
    labels.iter().any(|x| x == l)
}

/// Insert or refresh a message from Gmail metadata. Local-only state (flag due date / completion,
/// replied/forwarded marks, fetched body) is preserved. Returns (local id, inserted?).
pub fn upsert_meta(c: &Connection, account_id: i64, m: &Meta, map: &LabelMap, generation: i64) -> anyhow::Result<(i64, bool)> {
    let existing: Option<i64> = c
        .query_row("SELECT id FROM messages WHERE account_id=?1 AND gmail_id=?2", params![account_id, m.gmail_id], |r| r.get(0))
        .optional()?;
    let labels = m.labels.join(" ");
    let is_read = !has(&m.labels, "UNREAD");
    let starred = has(&m.labels, "STARRED");
    let is_draft = has(&m.labels, "DRAFT");
    let loc = location_of(&m.labels);
    let id = match existing {
        Some(id) => {
            c.execute(
                "UPDATE messages SET thread_id=?1, history_id=?2, label_ids=?3, is_read=?4, is_flagged=?5, is_draft=?6, location=?7, sync_gen=MAX(sync_gen, ?8), \
                 flag_status = CASE WHEN ?5 THEN 'flagged' WHEN flag_status='complete' THEN 'complete' ELSE NULL END, \
                 flag_due = CASE WHEN ?5 OR flag_status='complete' THEN flag_due ELSE NULL END \
                 WHERE id=?9",
                params![m.thread_id, m.history_id, labels, is_read, starred, is_draft, loc, generation, id],
            )?;
            set_membership(c, id, &map.folders_for(&m.labels), m.internal_date)?;
            return Ok((id, false));
        }
        None => {
            c.execute(
                "INSERT INTO messages(account_id, gmail_id, thread_id, history_id, internal_date, sent_date, subject, from_name, from_email, \
                 to_json, cc_json, bcc_json, reply_to_json, snippet, label_ids, is_read, is_flagged, flag_status, is_draft, has_attachments, importance, size, \
                 message_id_header, in_reply_to, references_header, subject_norm, to_display, sync_gen, location) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29)",
                params![
                    account_id,
                    m.gmail_id,
                    m.thread_id,
                    m.history_id,
                    m.internal_date,
                    m.sent_date,
                    m.subject,
                    m.from.name,
                    m.from.email,
                    addr_json(&m.to),
                    addr_json(&m.cc),
                    addr_json(&m.bcc),
                    addr_json(&m.reply_to),
                    m.snippet,
                    labels,
                    is_read,
                    starred,
                    starred.then_some("flagged"),
                    is_draft,
                    m.has_attachments,
                    m.importance,
                    m.size,
                    m.message_id,
                    m.in_reply_to,
                    m.references,
                    mime::normalize_subject(&m.subject),
                    to_display(&m.to),
                    generation,
                    loc
                ],
            )?;
            c.last_insert_rowid()
        }
    };
    set_membership(c, id, &map.folders_for(&m.labels), m.internal_date)?;
    fts_put(c, id, &m.subject, &format!("{} {}", m.from.name, m.from.email), &addrs_text(&m.to, &m.cc), &m.snippet)?;
    Ok((id, true))
}

pub fn addrs_text(to: &[Addr], cc: &[Addr]) -> String {
    to.iter().chain(cc).map(|a| format!("{} {}", a.name, a.email)).collect::<Vec<_>>().join(" ")
}

pub fn fts_put(c: &Connection, id: i64, subject: &str, from: &str, to: &str, body: &str) -> anyhow::Result<()> {
    c.execute("DELETE FROM messages_fts WHERE rowid=?1", [id])?;
    let body: String = body.chars().take(200_000).collect();
    c.execute(
        "INSERT INTO messages_fts(rowid, subject, from_text, to_text, body) VALUES (?1,?2,?3,?4,?5)",
        params![id, subject, from, to, body],
    )?;
    Ok(())
}

pub fn set_membership(c: &Connection, id: i64, folders: &[i64], date: i64) -> anyhow::Result<()> {
    let current: Vec<i64> = {
        let mut st = c.prepare_cached("SELECT folder_id FROM message_folders WHERE message_id=?1")?;
        st.query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?
    };
    for f in &current {
        if !folders.contains(f) {
            c.prepare_cached("DELETE FROM message_folders WHERE message_id=?1 AND folder_id=?2")?.execute(params![id, f])?;
        }
    }
    for f in folders {
        if !current.contains(f) {
            c.prepare_cached("INSERT OR IGNORE INTO message_folders(message_id, folder_id, msg_date) VALUES (?1,?2,?3)")?
                .execute(params![id, f, date])?;
        }
    }
    Ok(())
}

/// Folder membership of every message from its labels, after the folder list changed.
pub fn rederive_membership(c: &Connection, account_id: i64, map: &LabelMap) -> anyhow::Result<()> {
    let rows: Vec<(i64, String, i64)> = {
        let mut st = c.prepare("SELECT id, label_ids, internal_date FROM messages WHERE account_id=?1")?;
        st.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?
    };
    for (id, labels, date) in rows {
        let l: Vec<String> = labels.split_whitespace().map(str::to_string).collect();
        set_membership(c, id, &map.folders_for(&l), date)?;
    }
    Ok(())
}

/// Replace a message's label set (from history or a local action) and derive read/flag/folders.
pub fn apply_labels(c: &Connection, id: i64, labels: &[String], map: &LabelMap) -> anyhow::Result<()> {
    let date: i64 = c.query_row("SELECT internal_date FROM messages WHERE id=?1", [id], |r| r.get(0))?;
    let starred = has(labels, "STARRED");
    c.execute(
        "UPDATE messages SET label_ids=?1, is_read=?2, is_flagged=?3, is_draft=?4, location=?5, \
         flag_status = CASE WHEN ?3 THEN 'flagged' WHEN flag_status='complete' THEN 'complete' ELSE NULL END, \
         flag_due = CASE WHEN ?3 OR flag_status='complete' THEN flag_due ELSE NULL END WHERE id=?6",
        params![labels.join(" "), !has(labels, "UNREAD"), starred, has(labels, "DRAFT"), location_of(labels), id],
    )?;
    set_membership(c, id, &map.folders_for(labels), date)?;
    Ok(())
}

pub fn labels_of(c: &Connection, id: i64) -> anyhow::Result<Vec<String>> {
    let s: String = c.query_row("SELECT label_ids FROM messages WHERE id=?1", [id], |r| r.get(0))?;
    Ok(s.split_whitespace().map(str::to_string).collect())
}

/// Delete local messages (and their FTS rows / cached attachments' records).
pub fn delete_messages(c: &Connection, ids: &[i64]) -> anyhow::Result<()> {
    for id in ids {
        c.execute("DELETE FROM messages_fts WHERE rowid=?1", [id])?;
        c.execute("DELETE FROM messages WHERE id=?1", [id])?;
    }
    Ok(())
}

/// Recompute unread/total counts for an account's folders.
pub fn recount(c: &Connection, account_id: i64) -> anyhow::Result<()> {
    c.execute(
        "UPDATE folders SET \
           total_count = (SELECT COUNT(*) FROM message_folders mf WHERE mf.folder_id=folders.id), \
           unread_count = (SELECT COUNT(*) FROM message_folders mf JOIN messages m ON m.id=mf.message_id WHERE mf.folder_id=folders.id AND m.is_read=0) \
         WHERE account_id=?1 AND gmail_label_id IS NOT NULL",
        [account_id],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderRow {
    pub id: i64,
    pub account_id: i64,
    pub gmail_label_id: Option<String>,
    pub name: String,
    pub path: String,
    pub parent_id: Option<i64>,
    pub role: String,
    pub hidden: bool,
    pub unread: i64,
    pub total: i64,
}

pub fn folders(c: &Connection, account_id: i64) -> anyhow::Result<Vec<FolderRow>> {
    let mut st = c.prepare(
        "SELECT id, account_id, gmail_label_id, name, path, parent_id, role, hidden, unread_count, total_count FROM folders \
         WHERE account_id=?1 ORDER BY position, lower(path)",
    )?;
    let rows = st
        .query_map([account_id], |r| {
            Ok(FolderRow {
                id: r.get(0)?,
                account_id: r.get(1)?,
                gmail_label_id: r.get(2)?,
                name: r.get(3)?,
                path: r.get(4)?,
                parent_id: r.get(5)?,
                role: r.get(6)?,
                hidden: r.get::<_, i64>(7)? != 0,
                unread: r.get(8)?,
                total: r.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn folder(c: &Connection, id: i64) -> anyhow::Result<FolderRow> {
    let mut st = c.prepare(
        "SELECT id, account_id, gmail_label_id, name, path, parent_id, role, hidden, unread_count, total_count FROM folders WHERE id=?1",
    )?;
    st.query_row([id], |r| {
        Ok(FolderRow {
            id: r.get(0)?,
            account_id: r.get(1)?,
            gmail_label_id: r.get(2)?,
            name: r.get(3)?,
            path: r.get(4)?,
            parent_id: r.get(5)?,
            role: r.get(6)?,
            hidden: r.get::<_, i64>(7)? != 0,
            unread: r.get(8)?,
            total: r.get(9)?,
        })
    })
    .optional()?
    .ok_or_else(|| crate::user_err!("not_found", "That folder no longer exists."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(id: &str, name: &str, kind: &str) -> Label {
        Label { id: id.into(), name: name.into(), kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn folder_tree_from_labels() {
        let (db, _g) = crate::db::Db::open_temp().unwrap();
        db.write_blocking(|c| {
            c.execute("INSERT INTO accounts(email, provider, created_at) VALUES ('a@x','demo',0)", [])?;
            let labels = vec![
                label("INBOX", "INBOX", "system"),
                label("SENT", "SENT", "system"),
                label("TRASH", "TRASH", "system"),
                label("UNREAD", "UNREAD", "system"),
                label("CATEGORY_UPDATES", "CATEGORY_UPDATES", "system"),
                label("Label_1", "Clients", "user"),
                label("Label_2", "Clients/Orion", "user"),
                label("Label_3", "Deep/Nested/Leaf", "user"),
            ];
            assert!(sync_labels(c, 1, &labels, false)?);
            let f = folders(c, 1)?;
            let by = |p: &str| f.iter().find(|x| x.path == p).unwrap().clone();
            assert_eq!(by("Inbox").role, "inbox");
            assert_eq!(by("Deleted Items").role, "trash");
            assert_eq!(by("Outbox").role, "outbox");
            assert!(by("All Mail").hidden);
            assert_eq!(by("Clients/Orion").parent_id, Some(by("Clients").id));
            assert_eq!(by("Clients/Orion").name, "Orion");
            assert_eq!(by("Deep").role, "placeholder");
            assert_eq!(by("Deep/Nested/Leaf").parent_id, Some(by("Deep/Nested").id));
            assert!(!f.iter().any(|x| x.path == "UNREAD" || x.path.starts_with("CATEGORY")));
            // rename + delete
            let labels2 =
                vec![label("INBOX", "INBOX", "system"), label("Label_1", "Customers", "user"), label("Label_2", "Customers/Orion", "user")];
            sync_labels(c, 1, &labels2, true)?;
            let f = folders(c, 1)?;
            assert!(f.iter().any(|x| x.path == "Customers/Orion" && x.name == "Orion"));
            assert!(!f.iter().any(|x| x.path.starts_with("Deep")));
            assert!(!f.iter().any(|x| x.role == "trash"));
            assert!(!f.iter().find(|x| x.role == "all").unwrap().hidden);
            // idempotent
            assert!(!sync_labels(c, 1, &labels2, true)?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn trash_and_spam_are_exclusive() {
        let mut m = LabelMap::default();
        m.by_label.insert("INBOX".into(), 1);
        m.by_label.insert("TRASH".into(), 2);
        m.by_label.insert("Label_1".into(), 3);
        m.by_label.insert("SPAM".into(), 4);
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(m.folders_for(&v(&["INBOX", "Label_1", "UNREAD"])), vec![1, 3]);
        assert_eq!(m.folders_for(&v(&["INBOX", "TRASH", "Label_1"])), vec![2]);
        assert_eq!(m.folders_for(&v(&["SPAM"])), vec![4]);
    }
}

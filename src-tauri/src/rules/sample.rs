//! A handful of plausible mail rows (folders + messages) for one account. Used by the rules
//! tests and — in debug builds only — by `rules.devSeedSample`, so the Rules UI can be
//! exercised before the mail module is merged. Idempotent per account.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::json;

use crate::db::util::now_ms;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub inbox: i64,
    pub sent: i64,
    pub clients_folder: i64,
    pub orion_folder: i64,
    /// [Sarah high-importance w/ attachment+body, David (to me + other), Sarah only-to-me, sent by me, Yael (Hebrew)]
    pub messages: Vec<i64>,
    pub attachment: i64,
}

fn folder(c: &Connection, account: i64, label: &str, name: &str, path: &str, role: &str, parent: Option<i64>) -> anyhow::Result<i64> {
    if let Some(id) =
        c.query_row("SELECT id FROM folders WHERE account_id=?1 AND gmail_label_id=?2", params![account, label], |r| r.get(0)).optional()?
    {
        return Ok(id);
    }
    c.execute(
        "INSERT INTO folders(account_id, gmail_label_id, name, path, parent_id, role) VALUES (?1,?2,?3,?4,?5,?6)",
        params![account, label, name, path, parent, role],
    )?;
    Ok(c.last_insert_rowid())
}

pub fn insert(c: &Connection, account: i64) -> anyhow::Result<Sample> {
    let me: String = c.query_row("SELECT email FROM accounts WHERE id=?1", [account], |r| r.get(0))?;
    let my_name: String = c.query_row("SELECT display_name FROM accounts WHERE id=?1", [account], |r| r.get(0))?;
    let inbox = folder(c, account, "INBOX", "Inbox", "INBOX", "inbox", None)?;
    let sent = folder(c, account, "SENT", "Sent Items", "SENT", "sent", None)?;
    let clients = folder(c, account, "Label_901", "Clients", "Clients", "user", None)?;
    let orion = folder(c, account, "Label_902", "Orion", "Clients/Orion", "user", Some(clients))?;
    let now = now_ms();
    let h = 3_600_000i64;
    let p = |n: &str, e: &str| json!({ "name": n, "email": e });
    let rows = [
        (
            "Sarah Cohen",
            "sarah.cohen@harborcap.example",
            json!([p(&my_name, &me)]),
            json!([p("Maya Goldberg", "maya.goldberg@duvz.demo")]),
            "Orion Q3 earnings call — follow-up questions",
            2,
            1,
            inbox,
            now - 2 * h,
        ),
        (
            "David Levi",
            "david.levi@meridian.example",
            json!([p(&my_name, &me), p("Analysts", "research@meridian.example")]),
            json!([]),
            "Meeting request: Tel Aviv roadshow",
            1,
            0,
            inbox,
            now - 5 * h,
        ),
        (
            "Sarah Cohen",
            "sarah.cohen@harborcap.example",
            json!([p(&my_name, &me)]),
            json!([]),
            "Quick question on Corvina Medical",
            1,
            0,
            inbox,
            now - 26 * h,
        ),
        (
            my_name.as_str(),
            me.as_str(),
            json!([p("Noa Shapiro", "noa.shapiro@oriontech.example")]),
            json!([]),
            "Re: Orion NDR schedule",
            1,
            0,
            sent,
            now - 30 * h,
        ),
        (
            "Yael Friedman",
            "yael.friedman@tamarinv.example",
            json!([p(&my_name, &me)]),
            json!([]),
            "פגישה עם המשקיעים — יום שלישי",
            1,
            0,
            inbox,
            now - 50 * h,
        ),
    ];
    let mut ids = Vec::new();
    for (i, (fname, femail, to, cc, subject, importance, atts, fid, date)) in rows.iter().enumerate() {
        let gid = format!("sample-{i}");
        c.execute(
            "INSERT INTO messages(account_id, gmail_id, thread_id, internal_date, sent_date, subject, from_name, from_email, \
             to_json, cc_json, snippet, label_ids, is_read, has_attachments, importance) \
             VALUES (?1,?2,?2,?3,?3,?4,?5,?6,?7,?8,?9,?10,0,?11,?12) ON CONFLICT(account_id, gmail_id) DO NOTHING",
            params![
                account,
                gid,
                date,
                subject,
                fname,
                femail,
                to.to_string(),
                cc.to_string(),
                format!("{subject} …"),
                if *fid == sent { "SENT" } else { "INBOX" },
                atts,
                importance
            ],
        )?;
        let id: i64 = c.query_row("SELECT id FROM messages WHERE account_id=?1 AND gmail_id=?2", params![account, gid], |r| r.get(0))?;
        c.execute("INSERT OR IGNORE INTO message_folders(message_id, folder_id, msg_date) VALUES (?1, ?2, ?3)", params![id, fid, date])?;
        ids.push(id);
    }
    c.execute(
        "INSERT OR IGNORE INTO message_bodies(message_id, html, text, headers_json, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            ids[0],
            "<p>Hi Kenny,</p><p>Thanks for the call. Please find the updated deck attached — could Orion confirm the Q3 guidance?</p><p>Regards,<br>Sarah</p>",
            "Hi Kenny,\n\nThanks for the call. Please find the updated deck attached — could Orion confirm the Q3 guidance?\n\nRegards,\nSarah",
            json!([{ "name": "X-Mailer", "value": "Harbor Mail 2.1" }, { "name": "Subject", "value": "Orion Q3 earnings call — follow-up questions" }]).to_string(),
            now
        ],
    )?;
    let att = match c.query_row("SELECT id FROM attachments WHERE message_id=?1", [ids[0]], |r| r.get(0)).optional()? {
        Some(id) => id,
        None => {
            c.execute(
                "INSERT INTO attachments(message_id, part_id, filename, mime_type, size) VALUES (?1, '2', 'Orion_Q3_deck.pdf', 'application/pdf', 482113)",
                [ids[0]],
            )?;
            c.last_insert_rowid()
        }
    };
    Ok(Sample { inbox, sent, clients_folder: clients, orion_folder: orion, messages: ids, attachment: att })
}

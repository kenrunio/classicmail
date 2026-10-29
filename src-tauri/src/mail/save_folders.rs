//! Where attachments get saved, learnt per correspondent's company (e-mail domain): Save As and
//! Save All Attachments open in the folder used before for that domain, and so does Attach File ›
//! Browse in a message to people there (writing to @acme.example opens the Acme folder).
//!
//! * The domain is the other party's: the sender's on mail received, the recipients' on mail
//!   sent (To, else Cc). Own addresses and own domains (not public webmail) are left out; with
//!   several addresses the most common domain wins, and a tie means no domain (nothing learnt).
//! * Every save adds 1 to (domain, folder) in `attachment_folders`, and so does every file
//!   attached to a message to them (the file's folder; not a general one such as Downloads, the
//!   desktop or a temporary folder, which says nothing about the company).
//! * Suggesting: a folder used for over 75% of that domain's saves (Tools › Options, setting
//!   `mail.saveFolderPercent`); else the folder all of them
//!   share (Clients\Acme for Clients\Acme and Clients\Acme\Results\Q326), unless that
//!   is only a drive, the user folder, My Drive and the like; else the most used folder. A folder
//!   that no longer exists gives way to its nearest existing parent.
//!
//! rpc: `mail.saveFolder.suggest { attachmentId | messageId | archiveMessageId | to, cc }` →
//! `string | null` (`to` / `cc`: a message being written), `mail.saveFolder.remember { …, folder }`.

use std::{collections::HashMap, collections::HashSet, path::Path, sync::Arc};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;

use crate::{ctx::AppCtx, db::util::get_setting, mail::compose::model::Addr, rpc::Registry};

pub fn register(r: &mut Registry) {
    r.add("mail.saveFolder.suggest", suggest_rpc);
    r.add("mail.saveFolder.remember", remember_rpc);
}

/// Webmail domains: someone's own address there doesn't make everyone else there "us".
const PUBLIC_MAIL: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "yahoo.com",
    "icloud.com",
    "me.com",
    "aol.com",
    "walla.co.il",
];

fn domain(email: &str) -> Option<String> {
    let d = email.trim().rsplit_once('@')?.1.trim().trim_end_matches('>').to_lowercase();
    (!d.is_empty() && d.contains('.')).then_some(d)
}

/// The other party's domain (see the module comment), or None when it can't be told.
pub fn other_party_domain(from: &str, to: &[String], cc: &[String], own: &HashSet<String>) -> Option<String> {
    let own_domains: HashSet<String> = own.iter().filter_map(|e| domain(e)).filter(|d| !PUBLIC_MAIL.contains(&d.as_str())).collect();
    let mine = |e: &str| own.contains(&e.trim().to_lowercase()) || domain(e).is_some_and(|d| own_domains.contains(&d));
    let others = |list: &[String]| list.iter().filter(|e| !e.trim().is_empty() && !mine(e)).cloned().collect::<Vec<_>>();
    let candidates = if !from.trim().is_empty() && !mine(from) {
        vec![from.to_string()]
    } else {
        let t = others(to);
        if t.is_empty() { others(cc) } else { t }
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    for d in candidates.iter().filter_map(|e| domain(e)) {
        *counts.entry(d).or_default() += 1;
    }
    let top = counts.values().copied().max()?;
    let mut best = counts.into_iter().filter(|(_, n)| *n == top);
    let (d, _) = best.next()?;
    best.next().is_none().then_some(d)
}

fn parts(p: &str) -> Vec<&str> {
    p.split(['\\', '/']).filter(|s| !s.is_empty()).collect()
}

/// The deepest folder every one of `folders` is in (or is), case-insensitively.
pub fn common_parent(folders: &[&str]) -> Option<String> {
    let first = parts(folders.first()?);
    let mut n = first.len();
    for f in &folders[1..] {
        let p = parts(f);
        n = n.min(first.iter().zip(&p).take_while(|(a, b)| a.eq_ignore_ascii_case(b)).count());
    }
    if n == 0 {
        return None;
    }
    let mut s = first[..n].join("\\");
    if n == 1 && s.ends_with(':') {
        s.push('\\');
    }
    Some(s)
}

/// Too general to be "the" folder: a drive, the user folder or above it, or a top folder such as
/// My Drive, Documents, Desktop.
pub fn too_shallow(folder: &str, home: &str) -> bool {
    let p: Vec<String> = parts(folder).iter().map(|s| s.to_lowercase()).collect();
    let h: Vec<String> = parts(home).iter().map(|s| s.to_lowercase()).collect();
    if p.len() <= 1 || (p.len() <= h.len() && h.starts_with(&p)) {
        return true;
    }
    let top = ["my drive", "shared drives", "documents", "desktop", "downloads", "onedrive"];
    let last = p.last().map(String::as_str).unwrap_or("");
    (p.len() == h.len() + 1 && h.iter().zip(&p).all(|(a, b)| a == b) && (top.contains(&last) || last.starts_with("onedrive")))
        || (p.len() == 2 && (last == "my drive" || last == "shared drives"))
}

/// A folder that can be a company's: not a general one (a drive, the user folder, Downloads,
/// Desktop, My Drive...: `too_shallow`) and not a program's temporary or cache folder.
fn company_folder(folder: &str, home: &str) -> bool {
    let lower = format!("{}\\", folder.to_lowercase().replace('/', "\\"));
    !too_shallow(folder, home) && !lower.contains("\\appdata\\") && !lower.contains("\\temp\\")
}

/// The setting: the share of a domain's saves (%) that makes a folder its usual one.
pub const PERCENT_SETTING: &str = "mail.saveFolderPercent";
pub const DEFAULT_PERCENT: i64 = 75;

/// The folder to offer from a domain's (folder, saves, last used) rows (see the module comment):
/// one with over `percent` % of the saves, else the folder they share, else the most used.
pub fn pick(rows: &[(String, i64, i64)], home: &str, percent: i64) -> Option<String> {
    let total: i64 = rows.iter().map(|r| r.1).sum();
    let best = rows.iter().max_by_key(|r| (r.1, r.2))?;
    if total <= 0 {
        return None;
    }
    if best.1 * 100 > total * percent {
        return Some(best.0.clone());
    }
    let all: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    match common_parent(&all) {
        Some(p) if !too_shallow(&p, home) => Some(p),
        _ => Some(best.0.clone()),
    }
}

fn home() -> String {
    std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default()
}

/// The folder itself if it exists, else its nearest existing parent.
fn existing(folder: &str) -> Option<String> {
    Path::new(folder).ancestors().find(|p| p.is_dir()).map(|p| p.to_string_lossy().into_owned())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Which {
    attachment_id: Option<i64>,
    message_id: Option<i64>,
    archive_message_id: Option<i64>,
    /// the recipients of a message being written
    to: Option<Vec<String>>,
    #[serde(default)]
    cc: Vec<String>,
    folder: Option<String>,
}

fn emails(json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<Addr>>(json).map(|v| v.into_iter().map(|a| a.email).collect()).unwrap_or_default()
}

/// Addresses in an archive's display text ("Dan <dan@x.com>; Ruth"): the ones with an @.
fn display_emails(s: &str) -> Vec<String> {
    s.split([';', ',', ' ', '<', '>', '"', '\'', '(', ')']).filter(|t| t.contains('@')).map(str::to_string).collect()
}

fn domain_for(c: &Connection, w: &Which) -> anyhow::Result<Option<String>> {
    if let Some(to) = &w.to {
        let own = crate::mail::compose::store::own_addresses(c)?;
        return Ok(other_party_domain("", to, &w.cc, &own));
    }
    let row: Option<(String, Vec<String>, Vec<String>)> = if let Some(id) = w.archive_message_id {
        c.query_row("SELECT from_email, to_display, cc_display FROM archive_messages WHERE id=?1", [id], |r| {
            Ok((r.get(0)?, display_emails(&r.get::<_, String>(1)?), display_emails(&r.get::<_, String>(2)?)))
        })
        .optional()?
    } else {
        let (sql, id) = match (w.attachment_id, w.message_id) {
            (Some(a), _) => {
                ("SELECT m.from_email, m.to_json, m.cc_json FROM attachments a JOIN messages m ON m.id=a.message_id WHERE a.id=?1", a)
            }
            (None, Some(m)) => ("SELECT from_email, to_json, cc_json FROM messages WHERE id=?1", m),
            _ => return Ok(None),
        };
        c.query_row(sql, [id], |r| Ok((r.get(0)?, emails(&r.get::<_, String>(1)?), emails(&r.get::<_, String>(2)?)))).optional()?
    };
    let Some((from, to, cc)) = row else { return Ok(None) };
    let own = crate::mail::compose::store::own_addresses(c)?;
    Ok(other_party_domain(&from, &to, &cc, &own))
}

async fn suggest_rpc(ctx: Arc<AppCtx>, w: Which) -> anyhow::Result<Option<String>> {
    let rows = ctx
        .db
        .read(move |c| {
            let percent = get_setting::<i64>(c, PERCENT_SETTING)?.unwrap_or(DEFAULT_PERCENT).clamp(1, 100);
            let Some(d) = domain_for(c, &w)? else { return Ok((vec![], percent)) };
            let mut st = c.prepare("SELECT folder, count, last_used FROM attachment_folders WHERE domain=?1")?;
            let rows = st.query_map([d], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<(String, i64, i64)>, _>>()?;
            Ok((rows, percent))
        })
        .await?;
    Ok(pick(&rows.0, &home(), rows.1).and_then(|f| existing(&f)))
}

async fn remember_rpc(ctx: Arc<AppCtx>, w: Which) -> anyhow::Result<bool> {
    let Some(folder) = w.folder.as_deref().map(|f| f.trim().trim_end_matches(['\\', '/']).to_string()).filter(|f| !f.is_empty()) else {
        return Ok(false);
    };
    // a file attached from Downloads or a temporary copy says nothing about the company
    if w.to.is_some() && !company_folder(&folder, &home()) {
        return Ok(false);
    }
    let now = crate::clock::now_ms();
    ctx.db
        .write(move |c| {
            let Some(d) = domain_for(c, &w)? else { return Ok(false) };
            c.execute(
                "INSERT INTO attachment_folders(domain, folder, count, last_used) VALUES (?1, ?2, 1, ?3) \
                 ON CONFLICT(domain, folder) DO UPDATE SET count = count + 1, last_used = excluded.last_used",
                params![d, folder, now],
            )?;
            Ok(true)
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{accounts, rpc};
    use serde_json::json;

    const HOME: &str = r"C:\Users\alex";
    fn own() -> HashSet<String> {
        ["alex@acme-ir.example", "info@acme-ir.example", "alex.demo@gmail.com"].iter().map(|s| s.to_string()).collect()
    }
    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn only_a_company_folder_is_learnt_from_attaching() {
        assert!(company_folder(r"C:\Users\alex\Documents\Clients\Globex\Press Releases", HOME));
        assert!(!company_folder(r"C:\Users\alex\Downloads", HOME));
        assert!(!company_folder(r"C:\Users\alex\Desktop", HOME));
        assert!(!company_folder(r"C:\Users\alex\My Drive", HOME));
        assert!(!company_folder(r"C:\Users\alex\AppData\Local\Microsoft\Windows\INetCache\Content.Word", HOME));
        assert!(!company_folder(r"C:\Temp", HOME));
    }

    #[test]
    fn the_other_partys_domain() {
        // received: the sender
        assert_eq!(other_party_domain("Dana@Acme.example", &v(&["alex@acme-ir.example"]), &[], &own()).as_deref(), Some("acme.example"));
        // sent: the To recipients' most common domain, own people left out
        let to = v(&["a@acme.example", "b@acme.example", "c@harbor.example", "sam@acme-ir.example"]);
        assert_eq!(other_party_domain("alex@acme-ir.example", &to, &[], &own()).as_deref(), Some("acme.example"));
        // a tie can't be told
        assert_eq!(other_party_domain("alex@acme-ir.example", &v(&["a@acme.example", "c@harbor.example"]), &[], &own()), None);
        // from a colleague to only us: the Cc
        assert_eq!(
            other_party_domain("sam@acme-ir.example", &v(&["alex@acme-ir.example"]), &v(&["x@globex.example"]), &own()).as_deref(),
            Some("globex.example")
        );
        // own gmail address doesn't make every gmail.com sender "us"
        assert_eq!(other_party_domain("dan@gmail.com", &[], &[], &own()).as_deref(), Some("gmail.com"));
        assert_eq!(other_party_domain("alex@acme-ir.example", &v(&["info@acme-ir.example"]), &[], &own()), None);
    }

    #[test]
    fn picks_the_folder() {
        let rows = |rows: &[(&str, i64)]| rows.iter().enumerate().map(|(i, (f, n))| (f.to_string(), *n, i as i64)).collect::<Vec<_>>();
        let r = |r: &[(&str, i64)]| pick(&rows(r), HOME, 75);
        // over 75%: that folder
        assert_eq!(
            r(&[(r"C:\Users\alex\Documents\Clients\Acme\Results\Q326", 4), (r"C:\Users\alex\Documents\Clients\Acme", 1)]).as_deref(),
            Some(r"C:\Users\alex\Documents\Clients\Acme\Results\Q326")
        );
        // 75% or less: the folder they share
        assert_eq!(
            r(&[(r"C:\Users\alex\Documents\Clients\Acme\Results\Q326", 3), (r"C:\Users\alex\Documents\Clients\Acme", 1)]).as_deref(),
            Some(r"C:\Users\alex\Documents\Clients\Acme")
        );
        assert_eq!(
            r(&[(r"C:\Users\alex\Documents\Clients\Acme\Results\Q2", 1), (r"c:\users\alex\documents\clients\acme\IR", 1)]).as_deref(),
            Some(r"C:\Users\alex\Documents\Clients\Acme")
        );
        // sharing only My Drive / the user folder / a drive: the most used one
        assert_eq!(
            r(&[(r"C:\Users\alex\Documents\Acme", 2), (r"C:\Users\alex\Documents\Globex", 1)]).as_deref(),
            Some(r"C:\Users\alex\Documents\Acme")
        );
        assert_eq!(r(&[(r"C:\Users\alex\Desktop", 1), (r"C:\Users\alex\Downloads", 2)]).as_deref(), Some(r"C:\Users\alex\Downloads"));
        assert_eq!(r(&[(r"D:\Clients", 1), (r"C:\Temp", 1)]).as_deref(), Some(r"C:\Temp")); // most recent of a tie
        assert_eq!(r(&[]), None);
        // the share is a setting: at 50%, 3 of 4 is enough
        assert_eq!(pick(&rows(&[(r"D:\Clients\Acme\Q3", 3), (r"D:\Clients\Acme", 1)]), HOME, 50).as_deref(), Some(r"D:\Clients\Acme\Q3"));
    }

    #[test]
    fn shallow_folders() {
        for f in [
            r"C:\",
            r"C:\Users",
            HOME,
            r"C:\Users\alex\My Drive",
            r"C:\Users\alex\Documents",
            r"G:\My Drive",
            r"C:\Users\alex\OneDrive - Contoso",
        ] {
            assert!(too_shallow(f, HOME), "{f}");
        }
        for f in [r"C:\Users\alex\Documents\Clients", r"D:\Clients", r"C:\Users\alex\Documents\IR"] {
            assert!(!too_shallow(f, HOME), "{f}");
        }
        assert_eq!(common_parent(&[r"C:\a\b", r"C:\a\c"]).as_deref(), Some(r"C:\a"));
        assert_eq!(common_parent(&[r"C:\a", r"D:\a"]), None);
    }

    #[tokio::test]
    async fn learns_and_suggests() {
        let (ctx, _g) = AppCtx::for_test().unwrap();
        let reg = rpc::build();
        let acct = accounts::upsert(&ctx.db, "alex@acme-ir.example", "Alex Morgan", "demo").await.unwrap().id;
        let dir = tempfile::tempdir().unwrap();
        let acme = dir.path().join("Clients").join("Acme");
        let q326 = acme.join("Results").join("Q326");
        std::fs::create_dir_all(&q326).unwrap();
        let id = ctx
            .db
            .write(move |c| {
                c.execute(
                    "INSERT INTO messages(account_id, gmail_id, thread_id, internal_date, sent_date, subject, from_name, from_email, to_json, cc_json, body_state) \
                     VALUES (?1,'g1','t1',0,0,'Q3','Moshe','moshe@acme.example','[{\"name\":\"Alex\",\"email\":\"alex@acme-ir.example\"}]','[]',1)",
                    [acct],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap();
        let call = |m: &'static str, p: serde_json::Value| reg.call(ctx.clone(), m, p);
        assert_eq!(call("mail.saveFolder.suggest", json!({ "messageId": id })).await.unwrap(), json!(null));
        for _ in 0..3 {
            call("mail.saveFolder.remember", json!({ "messageId": id, "folder": q326.to_string_lossy() })).await.unwrap();
        }
        assert_eq!(call("mail.saveFolder.suggest", json!({ "messageId": id })).await.unwrap(), json!(q326.to_string_lossy()));
        // 3 of 4 is not over 75%: the shared parent
        call("mail.saveFolder.remember", json!({ "messageId": id, "folder": format!("{}\\", acme.to_string_lossy()) })).await.unwrap();
        assert_eq!(call("mail.saveFolder.suggest", json!({ "messageId": id })).await.unwrap(), json!(acme.to_string_lossy()));
        // writing to someone there: Attach File › Browse opens the same folder
        assert_eq!(
            call("mail.saveFolder.suggest", json!({ "to": ["dana@acme.example", "sam@acme-ir.example"], "cc": [] })).await.unwrap(),
            json!(acme.to_string_lossy())
        );
        assert_eq!(call("mail.saveFolder.suggest", json!({ "to": ["x@other.example"] })).await.unwrap(), json!(null));
        // a folder that is gone: its nearest existing parent
        std::fs::remove_dir_all(&acme).unwrap();
        assert_eq!(
            call("mail.saveFolder.suggest", json!({ "messageId": id })).await.unwrap(),
            json!(dir.path().join("Clients").to_string_lossy())
        );
    }
}

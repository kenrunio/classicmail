//! Integration tests: mail sync + rpc against the in-process fake Gmail server.

use std::sync::Arc;

use rusqlite::params;
use serde_json::{Value, json};

use crate::{accounts, ctx::AppCtx, db::util::get_sync_state, google::Api, rpc};

struct T {
    ctx: Arc<AppCtx>,
    _g: crate::db::tempdir::TempDirGuard,
    api: Api,
    account_id: i64,
    reg: rpc::Registry,
}

async fn setup(email: &str) -> T {
    let (ctx, g) = AppCtx::for_test().unwrap();
    let a = accounts::upsert(&ctx.db, email, "Kenny Green", "demo").await.unwrap();
    let api = ctx.api(a.id).await.unwrap();
    T { ctx, _g: g, api, account_id: a.id, reg: rpc::build() }
}

impl T {
    async fn sync(&self) {
        super::sync::sync_account(&self.ctx, &self.api).await.unwrap();
    }
    async fn call(&self, m: &str, p: Value) -> Value {
        match self.reg.call(self.ctx.clone(), m, p).await {
            Ok(v) => v,
            Err(e) => panic!("{m}: {e:#}"),
        }
    }
    async fn q<T2: Send + 'static + rusqlite::types::FromSql>(&self, sql: &'static str) -> T2 {
        let id = self.account_id;
        self.ctx.db.read(move |c| Ok(c.query_row(sql, [id], |r| r.get(0))?)).await.unwrap()
    }
    async fn folder(&self, role: &str) -> i64 {
        let role = role.to_string();
        let id = self.account_id;
        self.ctx
            .db
            .read(move |c| Ok(c.query_row("SELECT id FROM folders WHERE account_id=?1 AND role=?2", params![id, role], |r| r.get(0))?))
            .await
            .unwrap()
    }
    async fn folder_by_path(&self, path: &str) -> i64 {
        let path = path.to_string();
        let id = self.account_id;
        self.ctx
            .db
            .read(move |c| Ok(c.query_row("SELECT id FROM folders WHERE account_id=?1 AND path=?2", params![id, path], |r| r.get(0))?))
            .await
            .unwrap()
    }
    async fn local_id(&self, gmail_id: &str) -> i64 {
        let g = gmail_id.to_string();
        let id = self.account_id;
        self.ctx
            .db
            .read(move |c| Ok(c.query_row("SELECT id FROM messages WHERE account_id=?1 AND gmail_id=?2", params![id, g], |r| r.get(0))?))
            .await
            .unwrap()
    }
    async fn gmail_id(&self, local: i64) -> String {
        self.ctx.db.read(move |c| Ok(c.query_row("SELECT gmail_id FROM messages WHERE id=?1", [local], |r| r.get(0))?)).await.unwrap()
    }
    async fn x_demo(&self, what: &str, body: Value) -> Value {
        let url = format!("{}/users/me/x-demo/{what}", self.api.urls.gmail);
        self.api.post(&url, &[], body).await.unwrap()
    }
    async fn one(&self, sql: &'static str) -> i64 {
        self.q::<i64>(sql).await
    }
    fn events(&self, kind: &str) -> Vec<Value> {
        self.ctx.events.captured().into_iter().filter(|e| e["kind"] == kind).collect()
    }
    async fn remote_labels(&self, gid: &str) -> Vec<String> {
        self.api.gmail_get(gid, "minimal", &[]).await.unwrap().label_ids
    }
    /// wait for background pushes of pending ops
    async fn settle(&self) {
        for _ in 0..100 {
            if crate::sync::pending::count(&self.ctx.db, self.account_id, "mail").await.unwrap() == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_sync_matches_server() {
    let t = setup("kenny+seed@duvz.demo").await;
    t.sync().await;
    let prof = t.api.gmail_profile().await.unwrap();
    let local = t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await;
    assert_eq!(local, prof.messages_total, "every message synced");
    for (label, role) in [("INBOX", "inbox"), ("SENT", "sent"), ("TRASH", "trash"), ("SPAM", "spam"), ("DRAFT", "drafts")] {
        let l = t.api.gmail_label(label).await.unwrap();
        let f = t.folder(role).await;
        let (total, unread): (i64, i64) = t
            .ctx
            .db
            .read(move |c| {
                Ok(c.query_row("SELECT total_count, unread_count FROM folders WHERE id=?1", [f], |r| Ok((r.get(0)?, r.get(1)?)))?)
            })
            .await
            .unwrap();
        assert_eq!(total, l.messages_total.unwrap(), "{label} total");
        assert_eq!(unread, l.messages_unread.unwrap(), "{label} unread");
    }
    let orion = t.folder_by_path("Clients/Orion Technologies").await;
    let clients = t.folder_by_path("Clients").await;
    let parent: i64 =
        t.ctx.db.read(move |c| Ok(c.query_row("SELECT parent_id FROM folders WHERE id=?1", [orion], |r| r.get(0))?)).await.unwrap();
    assert_eq!(parent, clients);
    assert!(get_sync_state_of(&t, "mail.historyId").await.is_some());
    assert!(get_sync_state_of(&t, "mail.full.historyId").await.is_none());
    assert!(!t.events("mail.changed").is_empty());
    assert!(t.events("mail.newMail").is_empty(), "no alerts on the first full sync");
    // headers over IMAP (fast initial download), then the API check of the message list
    assert!(t.events("sync.status").iter().any(|e| e["text"].as_str().unwrap_or("").starts_with("Downloading message headers (fast)")));
    assert!(t.events("sync.status").iter().any(|e| e["text"].as_str().unwrap_or("").starts_with("Checking the message list")));
    // folder tree rpc
    let f = t.call("mail.listFolders", json!({})).await;
    let names: Vec<String> = f[0]["folders"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect();
    for n in [
        "Deleted Items",
        "Drafts",
        "Inbox",
        "Junk E-mail",
        "Outbox",
        "Sent Items",
        "Clients",
        "Orion Technologies",
        "Investors",
        "Receipts",
    ] {
        assert!(names.contains(&n.to_string()), "{n} in {names:?}");
    }
    assert!(f[0]["unreadMail"].as_i64().unwrap() > 0);
    // second sync is incremental and quiet
    t.sync().await;
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await, local);
}

async fn get_sync_state_of(t: &T, k: &'static str) -> Option<String> {
    let id = t.account_id;
    t.ctx.db.read(move |c| get_sync_state(c, id, k)).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn incremental_sync_applies_remote_changes() {
    let t = setup("kenny+seed@duvz.demo").await;
    t.sync().await;
    let inbox = t.folder("inbox").await;
    let rows = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "limit": 10 })).await;
    let ids: Vec<i64> = rows["rows"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
    let g_read = t.gmail_id(ids[0]).await;
    let g_trash = t.gmail_id(ids[1]).await;
    let g_del = t.gmail_id(ids[2]).await;
    let g_star = t.gmail_id(ids[3]).await;
    // remote changes (as if made in Gmail on the web)
    t.api.gmail_modify(&g_read, &[], &["UNREAD".into()]).await.unwrap();
    t.api.gmail_modify(&g_star, &["STARRED".into(), "UNREAD".into()], &[]).await.unwrap();
    t.api.gmail_trash(&g_trash).await.unwrap();
    t.api.gmail_delete(&g_del).await.unwrap();
    let new_id = t.x_demo("deliver", json!({})).await["id"].as_str().unwrap().to_string();
    t.sync().await;
    let read: i64 = t
        .ctx
        .db
        .read({
            let i = ids[0];
            move |c| Ok(c.query_row("SELECT is_read FROM messages WHERE id=?1", [i], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(read, 1);
    let (flag, status, unread): (i64, Option<String>, i64) = t
        .ctx
        .db
        .read({
            let i = ids[3];
            move |c| {
                Ok(c.query_row("SELECT is_flagged, flag_status, 1-is_read FROM messages WHERE id=?1", [i], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?)
            }
        })
        .await
        .unwrap();
    assert_eq!((flag, status.as_deref(), unread), (1, Some("flagged"), 1));
    let trash = t.folder("trash").await;
    let in_trash: i64 = t
        .ctx
        .db
        .read({
            let i = ids[1];
            move |c| {
                Ok(c.query_row("SELECT COUNT(*) FROM message_folders WHERE message_id=?1 AND folder_id=?2", params![i, trash], |r| {
                    r.get(0)
                })?)
            }
        })
        .await
        .unwrap();
    assert_eq!(in_trash, 1);
    let in_inbox: i64 = t
        .ctx
        .db
        .read({
            let i = ids[1];
            move |c| {
                Ok(c.query_row("SELECT COUNT(*) FROM message_folders WHERE message_id=?1 AND folder_id=?2", params![i, inbox], |r| {
                    r.get(0)
                })?)
            }
        })
        .await
        .unwrap();
    assert_eq!(in_inbox, 0, "trashed messages leave the Inbox");
    let gone: i64 = t
        .ctx
        .db
        .read({
            let i = ids[2];
            move |c| Ok(c.query_row("SELECT COUNT(*) FROM messages WHERE id=?1", [i], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(gone, 0);
    let nid = t.local_id(&new_id).await;
    let alerts = t.events("mail.newMail");
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0]["messages"][0]["id"].as_i64().unwrap(), nid);
    assert!(!alerts[0]["messages"][0]["subject"].as_str().unwrap().is_empty());
    // counts match Gmail's again
    let l = t.api.gmail_label("INBOX").await.unwrap();
    let (total, unread): (i64, i64) = t
        .ctx
        .db
        .read(move |c| {
            Ok(c.query_row("SELECT total_count, unread_count FROM folders WHERE id=?1", [inbox], |r| Ok((r.get(0)?, r.get(1)?)))?)
        })
        .await
        .unwrap();
    // the trashed message still carries INBOX on the server but is shown only in Deleted Items
    assert_eq!(total, l.messages_total.unwrap() - 1);
    assert!(unread <= l.messages_unread.unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_history_triggers_full_resync_keeping_flags() {
    let t = setup("kenny+seed@duvz.demo").await;
    t.sync().await;
    let inbox = t.folder("inbox").await;
    let rows = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "limit": 5 })).await;
    let a = rows["rows"][0]["id"].as_i64().unwrap();
    let b = rows["rows"][1]["id"].as_i64().unwrap();
    t.call("mail.setFlag", json!({ "ids": [a], "flag": "custom", "due": "2026-12-24" })).await;
    t.settle().await;
    t.sync().await; // push the star
    let gb = t.gmail_id(b).await;
    t.api.gmail_delete(&gb).await.unwrap();
    // label changes made elsewhere while history was unavailable
    let c = rows["rows"][2]["id"].as_i64().unwrap();
    let gc = t.gmail_id(c).await;
    t.api.gmail_modify(&gc, &["STARRED".into()], &["INBOX".into()]).await.unwrap();
    let before = t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await;
    t.x_demo("expireHistory", json!({})).await;
    t.sync().await;
    let after = t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await;
    assert_eq!(after, before - 1, "message deleted while history was unavailable is gone");
    let (flagged, in_inbox): (i64, i64) = t
        .ctx
        .db
        .read(move |db| {
            Ok(db.query_row(
                "SELECT is_flagged, (SELECT COUNT(*) FROM message_folders WHERE message_id=?1 AND folder_id=?2) FROM messages WHERE id=?1",
                params![c, inbox],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!((flagged, in_inbox), (1, 0), "labels changed during the gap are picked up by the bulk label resync");
    let (status, due): (Option<String>, Option<String>) = t
        .ctx
        .db
        .read(move |c| Ok(c.query_row("SELECT flag_status, flag_due FROM messages WHERE id=?1", [a], |r| Ok((r.get(0)?, r.get(1)?)))?))
        .await
        .unwrap();
    assert_eq!(status.as_deref(), Some("flagged"));
    assert_eq!(due.as_deref(), Some("2026-12-24"), "local follow-up date survives a resync");
    assert!(get_sync_state_of(&t, "mail.full.resync").await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn move_and_copy_between_mailboxes() {
    let t = setup("kenny+seed@duvz.demo").await;
    let b = accounts::upsert(&t.ctx.db, "info@duvz.demo", "Info", "demo").await.unwrap();
    let api_b = t.ctx.api(b.id).await.unwrap();
    t.sync().await;
    super::sync::sync_account(&t.ctx, &api_b).await.unwrap();
    let inbox = t.folder("inbox").await;
    let b_inbox: i64 = t
        .ctx
        .db
        .read(move |c| Ok(c.query_row("SELECT id FROM folders WHERE account_id=?1 AND role='inbox'", [b.id], |r| r.get(0))?))
        .await
        .unwrap();
    let rows = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "limit": 2 })).await;
    let ids: Vec<i64> = rows["rows"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
    let (moved, copied) = (ids[0], ids[1]);
    let g_moved = t.gmail_id(moved).await;
    t.call("mail.markRead", json!({ "ids": [moved], "read": false })).await;
    t.settle().await;
    let subject: String =
        t.ctx.db.read(move |c| Ok(c.query_row("SELECT subject FROM messages WHERE id=?1", [moved], |r| r.get(0))?)).await.unwrap();
    let in_b = |subject: String| {
        let ctx = t.ctx.clone();
        async move {
            ctx.db
                .read(move |c| {
                    Ok(c.query_row(
                        "SELECT COUNT(*) FROM messages m JOIN message_folders f ON f.message_id=m.id WHERE m.account_id=?1 AND f.folder_id=?2 AND m.subject=?3",
                        params![b.id, b_inbox, subject],
                        |r| r.get::<_, i64>(0),
                    )?)
                })
                .await
                .unwrap()
        }
    };
    let before_b = in_b(subject.clone()).await;

    // move into the other mailbox's Inbox, as Undo records it
    let out = t
        .call(
            "undo.run",
            json!({ "label": "Move", "method": "mail.move", "params": { "ids": [moved], "folderId": b_inbox, "fromFolderId": inbox } }),
        )
        .await;
    let _ = out;
    t.settle().await;
    t.sync().await;
    super::sync::sync_account(&t.ctx, &api_b).await.unwrap();
    assert!(t.remote_labels(&g_moved).await.contains(&"TRASH".to_string()), "the original went to Deleted Items");
    assert_eq!(in_b(subject.clone()).await, before_b + 1, "the copy is in the other Inbox");
    let subj = subject.clone();
    let labels: String = t
        .ctx
        .db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT label_ids FROM messages WHERE account_id=?1 AND subject=?2 ORDER BY id DESC",
                params![b.id, subj],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert!(labels.split_whitespace().any(|l| l == "UNREAD"), "the copy keeps the unread state: {labels}");

    // Undo: the copy goes, the original is back in the Inbox
    t.call("undo.undo", json!({})).await;
    t.settle().await;
    t.sync().await;
    super::sync::sync_account(&t.ctx, &api_b).await.unwrap();
    let l = t.remote_labels(&g_moved).await;
    assert!(l.contains(&"INBOX".to_string()) && !l.contains(&"TRASH".to_string()), "{l:?}");
    assert_eq!(in_b(subject.clone()).await, before_b, "Undo removed the copy");

    // Redo moves it again
    t.call("undo.redo", json!({})).await;
    t.settle().await;
    super::sync::sync_account(&t.ctx, &api_b).await.unwrap();
    assert_eq!(in_b(subject.clone()).await, before_b + 1);

    // copy: the original stays where it was
    let g_copied = t.gmail_id(copied).await;
    let r = t.call("mail.copy", json!({ "ids": [copied], "folderId": b_inbox })).await;
    assert_eq!(r["imported"].as_array().unwrap().len(), 1, "{r}");
    assert!(r["failed"].as_array().unwrap().is_empty());
    t.settle().await;
    assert!(t.remote_labels(&g_copied).await.contains(&"INBOX".to_string()));
    assert!(!t.events("mail.transfer").is_empty(), "progress is reported");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_actions_are_pushed() {
    let t = setup("kenny+seed@duvz.demo").await;
    t.sync().await;
    let inbox = t.folder("inbox").await;
    let investors = t.folder_by_path("Investors").await;
    let trash = t.folder("trash").await;
    let rows = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "limit": 6 })).await;
    let ids: Vec<i64> = rows["rows"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
    let g: Vec<String> = {
        let mut v = Vec::new();
        for i in &ids {
            v.push(t.gmail_id(*i).await);
        }
        v
    };
    // read / unread
    t.call("mail.markRead", json!({ "ids": [ids[0], ids[1]], "read": false })).await;
    t.call("mail.markRead", json!({ "ids": [ids[0]], "read": true })).await;
    // move Inbox → Investors
    t.call("mail.move", json!({ "ids": [ids[2]], "folderId": investors, "fromFolderId": inbox })).await;
    // copy
    t.call("mail.copy", json!({ "ids": [ids[3]], "folderId": investors })).await;
    // delete → Deleted Items, then permanently; the warning before it counts only Deleted Items
    let info = t.call("mail.deleteInfo", json!({ "ids": [ids[4]] })).await;
    assert_eq!(info["permanent"], 0);
    t.call("mail.delete", json!({ "ids": [ids[4]] })).await;
    let r = t.call("mail.delete", json!({ "ids": [ids[4]] })).await;
    assert_eq!(r["permanent"], 1);
    // flag complete
    t.call("mail.setFlag", json!({ "ids": [ids[5]], "flag": "today" })).await;
    t.call("mail.setFlag", json!({ "ids": [ids[5]], "flag": "complete" })).await;
    // local state is immediate
    let in_inv: i64 = t
        .ctx
        .db
        .read({
            let i = ids[2];
            move |c| {
                Ok(c.query_row("SELECT COUNT(*) FROM message_folders WHERE message_id=?1 AND folder_id=?2", params![i, investors], |r| {
                    r.get(0)
                })?)
            }
        })
        .await
        .unwrap();
    assert_eq!(in_inv, 1);
    t.settle().await;
    t.sync().await;
    assert!(!t.remote_labels(&g[0]).await.contains(&"UNREAD".to_string()));
    assert!(t.remote_labels(&g[1]).await.contains(&"UNREAD".to_string()));
    let l2 = t.remote_labels(&g[2]).await;
    let inv_label: String = t
        .ctx
        .db
        .read(move |c| Ok(c.query_row("SELECT gmail_label_id FROM folders WHERE id=?1", [investors], |r| r.get(0))?))
        .await
        .unwrap();
    assert!(l2.contains(&inv_label) && !l2.contains(&"INBOX".to_string()), "{l2:?}");
    let l3 = t.remote_labels(&g[3]).await;
    assert!(l3.contains(&inv_label) && l3.contains(&"INBOX".to_string()));
    assert!(t.api.gmail_get(&g[4], "minimal", &[]).await.is_err(), "permanently deleted on the server");
    assert!(!t.remote_labels(&g[5]).await.contains(&"STARRED".to_string()));
    let status: Option<String> = t
        .ctx
        .db
        .read({
            let i = ids[5];
            move |c| Ok(c.query_row("SELECT flag_status FROM messages WHERE id=?1", [i], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(status.as_deref(), Some("complete"));
    // restore from Deleted Items, junk / not junk
    t.call("mail.delete", json!({ "ids": [ids[1]] })).await;
    let tr: i64 =
        t.ctx.db.read(move |c| Ok(c.query_row("SELECT total_count FROM folders WHERE id=?1", [trash], |r| r.get(0))?)).await.unwrap();
    assert!(tr >= 5);
    t.call("mail.restore", json!({ "ids": [ids[1]] })).await;
    t.call("mail.markJunk", json!({ "ids": [ids[0]] })).await;
    t.settle().await;
    t.sync().await;
    let l1 = t.remote_labels(&g[1]).await;
    assert!(l1.contains(&"INBOX".to_string()) && !l1.contains(&"TRASH".to_string()));
    assert!(t.remote_labels(&g[0]).await.contains(&"SPAM".to_string()));
    t.call("mail.notJunk", json!({ "ids": [ids[0]] })).await;
    // flagged list contract
    let fl = t.call("mail.listFlagged", json!({})).await;
    assert!(fl.as_array().unwrap().iter().any(|x| x["id"].as_i64() == Some(ids[5]) && x["flagStatus"] == "complete"));
    // empty Deleted Items
    let n = t.call("mail.emptyFolder", json!({ "folderId": trash })).await;
    assert!(n.as_i64().unwrap() >= 4);
    t.settle().await;
    t.sync().await;
    assert_eq!(t.api.gmail_label("TRASH").await.unwrap().messages_total, Some(0));
    assert_eq!(crate::sync::pending::count(&t.ctx.db, t.account_id, "mail").await.unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bodies_attachments_charsets_invites() {
    let t = setup("kenny+seed@duvz.demo").await;
    t.sync().await;

    // windows-1255 plain text
    let heb = t.one("SELECT id FROM messages WHERE account_id=?1 AND subject='הצעת מחיר לשירותי קשרי משקיעים'").await;
    let m = t.call("mail.getMessage", json!({ "id": heb })).await;
    assert_eq!(m["fromName"], "אבי רוזן");
    assert!(m["text"].as_str().unwrap().contains("מצורפת הצעת המחיר"), "{}", m["text"]);
    assert_eq!(m["isPlain"], true);
    // iso-8859-8 quoted-printable HTML
    let heb2 = t.one("SELECT id FROM messages WHERE account_id=?1 AND subject='הזמנה לאירוע השקה'").await;
    let m = t.call("mail.getMessage", json!({ "id": heb2 })).await;
    assert!(m["html"].as_str().unwrap().contains("מלון דן תל אביב"), "{}", m["html"]);
    // invite
    let inv =
        t.one("SELECT id FROM messages WHERE account_id=?1 AND subject LIKE 'Invitation:%' ORDER BY internal_date DESC LIMIT 1").await;
    let m = t.call("mail.getMessage", json!({ "id": inv })).await;
    assert!(m["invite"]["uid"].as_str().unwrap().contains("@"));
    assert_eq!(m["invite"]["method"], "REQUEST");
    assert!(m["invite"]["start"].as_i64().is_some());
    assert!(m["attachments"].as_array().unwrap().is_empty(), "invite.ics hidden for meeting requests");
    let (is_inv, uid): (i64, String) = t
        .ctx
        .db
        .read(move |c| Ok(c.query_row("SELECT is_invite, ical_uid FROM messages WHERE id=?1", [inv], |r| Ok((r.get(0)?, r.get(1)?)))?))
        .await
        .unwrap();
    assert_eq!(is_inv, 1);
    assert_eq!(uid, m["invite"]["uid"].as_str().unwrap());
    // the invitation matches a calendar event (cross-domain contract)
    crate::calendar::sync::sync_account(&t.ctx, &t.api).await.unwrap();
    let ev = t.call("calendar.findByIcalUid", json!({ "accountId": t.account_id, "icalUid": uid })).await;
    assert!(ev["id"].is_number(), "calendar event for {uid}: {ev}");
    let r = t.call("calendar.respondToInvite", json!({ "accountId": t.account_id, "icalUid": uid, "response": "tentative" })).await;
    assert_eq!(r["eventId"], ev["id"]);
    // HTML newsletter: remote pictures blocked, then allowed
    let nl = t.one("SELECT id FROM messages WHERE account_id=?1 AND from_email LIKE 'news@capmarkets%' ORDER BY size DESC LIMIT 1").await;
    let m = t.call("mail.getMessage", json!({ "id": nl })).await;
    assert_eq!(m["remoteBlocked"], true);
    assert!(!m["html"].as_str().unwrap().contains("https://media.capmarkets-weekly.example/img"));
    let m = t.call("mail.getMessage", json!({ "id": nl, "allowRemote": true })).await;
    assert_eq!(m["remoteBlocked"], false);
    assert!(m["html"].as_str().unwrap().contains("https://media.capmarkets-weekly.example/img"));
    // ...and remembered: reopening without allowRemote still shows the pictures
    let m = t.call("mail.getMessage", json!({ "id": nl })).await;
    assert_eq!(m["remoteBlocked"], false, "Download Pictures should be remembered per message");
    assert!(m["html"].as_str().unwrap().contains("https://media.capmarkets-weekly.example/img"));
    // the biggest newsletter body comes through attachments.get (> 32 KB)
    let big = t.one("SELECT id FROM messages WHERE account_id=?1 AND size > 60000 ORDER BY size DESC LIMIT 1").await;
    let m = t.call("mail.getMessage", json!({ "id": big })).await;
    assert!(m["html"].as_str().unwrap().len() > 30_000);
    // safe sender
    t.call("mail.addSafeSender", json!({ "address": "news@capmarkets-weekly.example" })).await;
    let m = t.call("mail.getMessage", json!({ "id": nl })).await;
    assert_eq!(m["safeSender"], true);
    assert_eq!(m["remoteBlocked"], false);
    // attachments + inline cid image
    let mut found_inline = false;
    let ids: Vec<i64> = t
        .ctx
        .db
        .read({ let a = t.account_id; move |c| { let mut st = c.prepare("SELECT id FROM messages WHERE account_id=?1 AND from_email LIKE '%.example' AND location=0 ORDER BY internal_date DESC LIMIT 120")?; Ok(st.query_map([a], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?) } })
        .await
        .unwrap();
    let mut pdf_checked = false;
    for id in ids {
        let m = t.call("mail.getMessage", json!({ "id": id })).await;
        for a in m["attachments"].as_array().unwrap() {
            if a["isInline"] == true {
                found_inline = true;
                assert!(m["html"].as_str().unwrap().contains(&super::render::inline_url(a["id"].as_i64().unwrap())));
                let resp = super::protocol_response(&t.ctx, &format!("/inline/{}", a["id"])).await;
                assert_eq!(resp.status(), 200);
                assert!(resp.body().starts_with(&[0x89, b'P', b'N', b'G']));
            }
            if a["mimeType"] == "application/pdf" && !pdf_checked {
                let (_, _, bytes) = super::attachment_bytes(&t.ctx, a["id"].as_i64().unwrap()).await.unwrap();
                assert!(bytes.starts_with(b"%PDF"));
                // cached: second read works offline from disk
                let p = t.ctx.data_dir.join("x.pdf");
                t.call("mail.saveAttachment", json!({ "attachmentId": a["id"], "path": p.to_string_lossy() })).await;
                assert!(std::fs::read(&p).unwrap().starts_with(b"%PDF"));
                // non-images are never served as something the WebView would render
                let resp = super::protocol_response(&t.ctx, &format!("/att/{}/r.pdf", a["id"])).await;
                assert_eq!(resp.headers()["content-type"], "application/octet-stream");
                assert!(resp.headers().get("access-control-allow-origin").is_none());
                assert!(resp.headers()["content-security-policy"].to_str().unwrap().contains("sandbox"));
                // opening writes a temp copy marked as from the internet
                let opened = t.call("mail.openAttachment", json!({ "attachmentId": a["id"] })).await;
                let mut ads = std::ffi::OsString::from(opened.as_str().unwrap());
                ads.push(":Zone.Identifier");
                #[cfg(windows)]
                assert!(std::fs::read_to_string(&ads).unwrap().contains("ZoneId=3"));
                let _ = ads;
                pdf_checked = true;
            }
        }
        if found_inline && pdf_checked {
            break;
        }
    }
    assert!(found_inline && pdf_checked);
    // internet headers
    let h = t.call("mail.getHeaders", json!({ "id": heb })).await;
    assert!(h.as_str().unwrap().contains("Content-Type: text/plain; charset=\"windows-1255\""));
    // FTS on fetched body text
    let r = t.call("mail.search", json!({ "query": "מצורפת", "scope": "all" })).await;
    assert!(r["rows"].as_array().unwrap().iter().any(|x| x["id"].as_i64() == Some(heb)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn listing_groups_search_and_folders() {
    let t = setup("kenny+seed@duvz.demo").await;
    t.sync().await;
    let inbox = t.folder("inbox").await;
    let r = t
        .call(
            "mail.listMessages",
            json!({ "source": { "folderId": inbox }, "arrange": "date", "desc": true, "withGroups": true, "offset": 0, "limit": 50 }),
        )
        .await;
    let groups = r["groups"].as_array().unwrap();
    assert_eq!(groups[0]["label"], "Today");
    let sum: i64 = groups.iter().map(|g| g["count"].as_i64().unwrap()).sum();
    assert_eq!(sum, r["total"].as_i64().unwrap());
    let dates: Vec<i64> = r["rows"].as_array().unwrap().iter().map(|x| x["date"].as_i64().unwrap()).collect();
    assert!(dates.windows(2).all(|w| w[0] >= w[1]));
    for arr in ["from", "to", "subject", "size", "flag", "attachments", "importance", "conversation"] {
        for desc in [true, false] {
            let r = t
                .call(
                    "mail.listMessages",
                    json!({ "source": { "folderId": inbox }, "arrange": arr, "desc": desc, "withGroups": true, "limit": 1000 }),
                )
                .await;
            let groups = r["groups"].as_array().unwrap();
            let sum: i64 = groups.iter().map(|g| g["count"].as_i64().unwrap()).sum();
            assert_eq!(sum, r["total"].as_i64().unwrap(), "{arr}");
            assert_eq!(r["rows"].as_array().unwrap().len() as i64, r["total"].as_i64().unwrap().min(1000));
        }
    }
    // search folders
    let r = t.call("mail.listMessages", json!({ "source": { "searchFolder": "unread", "accountId": t.account_id }, "limit": 500 })).await;
    assert!(r["rows"].as_array().unwrap().iter().all(|x| x["isRead"] == false));
    assert!(r["crossFolder"] == true && r["rows"][0]["folder"].is_string());
    // Instant Search
    let r = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "search": "from:sarah", "limit": 100 })).await;
    assert!(r["total"].as_i64().unwrap() > 0);
    assert!(r["rows"].as_array().unwrap().iter().all(|x| x["fromEmail"].as_str().unwrap().contains("sarah")));
    let r = t.call("mail.search", json!({ "query": "hasattachments:yes received:this month", "scope": "all" })).await;
    assert!(r["rows"].as_array().unwrap().iter().all(|x| x["hasAttachments"] == true));
    let r = t.call("mail.searchServer", json!({ "query": "subject:Receipt" })).await;
    assert!(!r["ids"].as_array().unwrap().is_empty());
    assert!(r["extra"].as_i64().is_some(), "how many Gmail found that the local search did not: {r}");
    // ...and which (the list marks them): Gmail's matches the local search doesn't have
    let extra_ids: Vec<i64> = r["extraIds"].as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect();
    assert_eq!(extra_ids.len() as i64, r["extra"].as_i64().unwrap());
    let all_ids: Vec<i64> = r["ids"].as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect();
    let locally = t.call("mail.search", json!({ "query": "subject:Receipt", "scope": "all" })).await;
    let local_found: Vec<i64> = locally["rows"].as_array().unwrap().iter().map(|x| x["id"].as_i64().unwrap()).collect();
    assert!(extra_ids.iter().all(|id| all_ids.contains(id) && !local_found.contains(id)), "{extra_ids:?}");
    assert!(all_ids.iter().all(|id| extra_ids.contains(id) || local_found.contains(id)));
    // Gmail's extra matches join a search (alsoIds), within the view's folder
    let local = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "search": "from:sarah", "limit": 500 })).await;
    let row_ids = |v: &Value| v["rows"].as_array().unwrap().iter().map(|x| x["id"].as_i64().unwrap()).collect::<Vec<_>>();
    let local_ids = row_ids(&local);
    let other = t.one("SELECT mf.message_id FROM message_folders mf JOIN folders f ON f.id=mf.folder_id JOIN messages m ON m.id=mf.message_id                        WHERE m.account_id=?1 AND f.role='inbox' AND m.from_email NOT LIKE '%sarah%' LIMIT 1").await;
    assert!(!local_ids.contains(&other));
    let merged = t
        .call("mail.listMessages", json!({ "source": { "folderId": inbox }, "search": "from:sarah", "alsoIds": [other], "limit": 500 }))
        .await;
    assert_eq!(merged["total"].as_i64().unwrap(), local["total"].as_i64().unwrap() + 1);
    assert!(row_ids(&merged).contains(&other));
    // ...and "Show only these": just those ids, still within the view's folder
    let outside = t.one("SELECT id FROM messages m WHERE account_id=?1 AND NOT EXISTS (SELECT 1 FROM message_folders mf JOIN folders f ON f.id=mf.folder_id WHERE mf.message_id=m.id AND f.role='inbox') LIMIT 1").await;
    let only = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "onlyIds": [other, outside], "limit": 50 })).await;
    assert_eq!(row_ids(&only), vec![other]);
    // folder management
    let fid = t.call("mail.createFolder", json!({ "accountId": t.account_id, "name": "Projects" })).await.as_i64().unwrap();
    let sub = t.call("mail.createFolder", json!({ "parentId": fid, "name": "Alpha" })).await.as_i64().unwrap();
    let mid = t.one("SELECT m.id FROM messages m JOIN message_folders mf ON mf.message_id=m.id JOIN folders f ON f.id=mf.folder_id          WHERE m.account_id=?1 AND f.role='inbox' AND (SELECT COUNT(*) FROM message_folders x WHERE x.message_id=m.id)=1 LIMIT 1").await;
    t.call("mail.move", json!({ "ids": [mid], "folderId": sub, "fromFolderId": inbox })).await;
    t.call("mail.renameFolder", json!({ "folderId": fid, "name": "Projects 2027" })).await;
    let f = t.call("mail.listFolders", json!({})).await;
    let paths: Vec<String> = f[0]["folders"].as_array().unwrap().iter().map(|x| x["path"].as_str().unwrap().to_string()).collect();
    assert!(paths.contains(&"Projects 2027/Alpha".to_string()), "{paths:?}");
    let err =
        t.reg.call(t.ctx.clone(), "mail.createFolder", json!({ "accountId": t.account_id, "name": "Projects 2027" })).await.unwrap_err();
    assert!(err.to_string().contains("already exists"));
    // Gmail keeps "Important" for itself at the top level
    let err = t.reg.call(t.ctx.clone(), "mail.createFolder", json!({ "accountId": t.account_id, "name": "Important" })).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "Gmail uses the name \"Important\" for one of its own labels, so a folder here can't have it. Choose another name, or put the folder inside another one (Inbox, for example)."
    );
    let fid2 = t.folder_by_path("Projects 2027").await;
    t.call("mail.deleteFolder", json!({ "folderId": fid2 })).await;
    let trash = t.folder("trash").await;
    let in_trash: i64 = t
        .ctx
        .db
        .read(move |c| {
            Ok(c.query_row("SELECT COUNT(*) FROM message_folders WHERE message_id=?1 AND folder_id=?2", params![mid, trash], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(in_trash, 1, "messages of a deleted folder go to Deleted Items");
    let f = t.call("mail.listFolders", json!({})).await;
    assert!(!f[0]["folders"].as_array().unwrap().iter().any(|x| x["path"].as_str().unwrap().starts_with("Projects")));
    // adjacent
    let rows = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "limit": 3 })).await;
    let a0 = rows["rows"][0]["id"].as_i64().unwrap();
    let a1 = rows["rows"][1]["id"].as_i64().unwrap();
    assert_eq!(t.call("mail.adjacent", json!({ "id": a0, "folderId": inbox, "direction": "next" })).await.as_i64(), Some(a1));
    assert_eq!(t.call("mail.adjacent", json!({ "id": a1, "folderId": inbox, "direction": "previous" })).await.as_i64(), Some(a0));
}

/// Large-mailbox timing: `cargo test --lib big_mailbox -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore]
async fn big_mailbox_performance() {
    let t = setup("perf+seed@duvz.demo").await;
    let n: u64 = std::env::var("CM_BIG").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000);
    let total = t.x_demo("bulk", json!({ "count": n })).await;
    println!("server mailbox: {}", total["total"]);
    let started = std::time::Instant::now();
    t.sync().await;
    let secs = started.elapsed().as_secs_f64();
    let local = t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await;
    println!("full sync: {local} messages in {secs:.1}s ({:.0} msg/s)", local as f64 / secs);
    let inbox = t.folder("inbox").await;
    for (arr, groups) in [("date", true), ("date", false), ("from", true), ("subject", true), ("size", true), ("conversation", true)] {
        let started = std::time::Instant::now();
        let r = t
            .call(
                "mail.listMessages",
                json!({ "source": { "folderId": inbox }, "arrange": arr, "withGroups": groups, "offset": 5000, "limit": 200 }),
            )
            .await;
        println!("listMessages {arr} groups={groups}: total {} in {:.1} ms", r["total"], started.elapsed().as_secs_f64() * 1000.0);
    }
    let started = std::time::Instant::now();
    let r = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "search": "meridian", "limit": 200 })).await;
    println!("search: {} hits in {:.1} ms", r["total"], started.elapsed().as_secs_f64() * 1000.0);
    let started = std::time::Instant::now();
    t.call("mail.listFolders", json!({})).await;
    println!("listFolders: {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
    let started = std::time::Instant::now();
    let ids: Vec<i64> = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "limit": 1000 })).await["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_i64().unwrap())
        .collect();
    t.call("mail.markRead", json!({ "ids": ids, "read": true })).await;
    println!("markRead 1000: {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
    let started = std::time::Instant::now();
    t.settle().await;
    t.sync().await;
    println!("push + incremental sync: {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
}

/// A big mailbox is synced in slices (other domains get their turn), resumes from the stored page
/// token, and survives a page token Gmail no longer accepts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_sync_is_sliced_and_resumable() {
    let t = setup("slices+seed@duvz.demo").await;
    // the API path (the fast IMAP download has its own tests in imap_tests.rs)
    t.ctx.db.write(|c| crate::db::util::set_setting(c, "mail.fastInitialSync", &false)).await.unwrap();
    t.x_demo("bulk", json!({ "count": 1200 })).await;
    let total = t.api.gmail_profile().await.unwrap().messages_total;
    let zero = std::time::Duration::ZERO;
    let done = super::sync::full_sync_slice_for_test(&t.ctx, &t.api, zero).await.unwrap();
    assert!(!done, "stops after the first page when its time slice is used up");
    let first = t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await;
    assert_eq!(first, 500);
    assert!(get_sync_state_of(&t, "mail.full.pageToken").await.is_some());
    assert!(get_sync_state_of(&t, "mail.historyId").await.is_none(), "not done yet");
    // the stored page token went stale (e.g. resumed days later): Gmail answers 400
    let id = t.account_id;
    t.ctx.db.write(move |c| crate::db::util::set_sync_state(c, id, "mail.full.pageToken", Some("stale-token"))).await.unwrap();
    // new mail arriving mid-way is picked up by the next slice, with an alert
    let new_id = t.x_demo("deliver", json!({})).await["id"].as_str().unwrap().to_string();
    let done = super::sync::full_sync_slice_for_test(&t.ctx, &t.api, zero).await.unwrap();
    assert!(!done);
    let nid = t.local_id(&new_id).await;
    assert!(t.events("mail.newMail").iter().any(|e| e["messages"][0]["id"].as_i64() == Some(nid)));
    // a normal cycle finishes the job (and keeps the caught-up message)
    t.sync().await;
    assert_eq!(t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await, total + 1);
    t.local_id(&new_id).await;
    assert_eq!(t.events("mail.newMail").len(), 1, "no second alert for the same message");
    assert!(get_sync_state_of(&t, "mail.historyId").await.is_some());
    assert!(get_sync_state_of(&t, "mail.full.pageToken").await.is_none());
}

// ---- Show as Conversations ---------------------------------------------------------------------

fn ids_of(v: &Value) -> Vec<i64> {
    v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn conversations_aggregate_and_page() {
    let t = setup("conv+seed@duvz.demo").await;
    t.sync().await;
    let inbox = t.folder("inbox").await;
    let sent = t.folder("sent").await;
    let flat = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "limit": 2000 })).await;
    let conv = t
        .call("mail.listMessages", json!({ "source": { "folderId": inbox }, "conversations": true, "withGroups": true, "limit": 2000 }))
        .await;
    let n_msgs = flat["total"].as_i64().unwrap();
    let n_conv = conv["total"].as_i64().unwrap();
    assert_eq!(conv["messages"].as_i64().unwrap(), n_msgs, "message count of the view");
    assert_eq!(conv["unread"], flat["unread"]);
    assert!(n_conv < n_msgs, "seed threads are grouped ({n_conv} conversations, {n_msgs} messages)");
    let rows = conv["rows"].as_array().unwrap();
    assert_eq!(rows.len() as i64, n_conv);
    // every inbox message is in exactly one conversation
    let mut all: Vec<i64> = rows.iter().flat_map(|r| ids_of(&r["conv"]["ids"])).collect();
    all.sort();
    let mut flat_ids: Vec<i64> = flat["rows"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
    flat_ids.sort();
    assert_eq!(all, flat_ids);
    // newest first by the conversation's newest message; the row shows that message
    let dates: Vec<i64> = rows.iter().map(|r| r["date"].as_i64().unwrap()).collect();
    assert!(dates.windows(2).all(|w| w[0] >= w[1]));
    let by_id: std::collections::HashMap<i64, &Value> =
        flat["rows"].as_array().unwrap().iter().map(|r| (r["id"].as_i64().unwrap(), r)).collect();
    for r in rows {
        let c = &r["conv"];
        let ids = ids_of(&c["ids"]);
        assert_eq!(c["count"].as_i64().unwrap(), ids.len() as i64);
        assert!(c["total"].as_i64().unwrap() >= c["count"].as_i64().unwrap());
        let newest = ids.iter().map(|i| by_id[i]["date"].as_i64().unwrap()).max().unwrap();
        assert_eq!(r["date"].as_i64().unwrap(), newest);
        assert_eq!(by_id[&r["id"].as_i64().unwrap()]["date"].as_i64().unwrap(), newest, "header = newest message");
        let unread = ids.iter().filter(|i| !by_id[i]["isRead"].as_bool().unwrap()).count() as i64;
        assert_eq!(c["unread"].as_i64().unwrap(), unread);
        let size: i64 = ids.iter().map(|i| by_id[i]["size"].as_i64().unwrap()).sum();
        assert_eq!(c["size"].as_i64().unwrap(), size);
        assert_eq!(c["hasAttachments"].as_bool().unwrap(), ids.iter().any(|i| by_id[i]["hasAttachments"].as_bool().unwrap()));
        assert!(!c["participants"].as_str().unwrap().is_empty());
    }
    // groups (Today / Yesterday ...) count conversations
    let gsum: i64 = conv["groups"].as_array().unwrap().iter().map(|g| g["count"].as_i64().unwrap()).sum();
    assert_eq!(gsum, n_conv);
    // paging gives the same rows in the same order
    let mut paged = Vec::new();
    for off in (0..n_conv).step_by(7) {
        let p =
            t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "conversations": true, "offset": off, "limit": 7 })).await;
        paged.extend(p["rows"].as_array().unwrap().iter().map(|r| r["conv"]["key"].as_str().unwrap().to_string()));
    }
    let keys: Vec<String> = rows.iter().map(|r| r["conv"]["key"].as_str().unwrap().to_string()).collect();
    assert_eq!(paged, keys);
    // sort by size (largest conversation first)
    let by_size = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "conversations": true, "arrange": "size", "desc": true, "withGroups": true, "limit": 2000 })).await;
    let sizes: Vec<i64> = by_size["rows"].as_array().unwrap().iter().map(|r| r["conv"]["size"].as_i64().unwrap()).collect();
    assert!(sizes.windows(2).all(|w| w[0] >= w[1]));
    // a thread with my reply in Sent Items: expanding shows it, newest first, with its folder
    let r = rows
        .iter()
        .find(|r| r["conv"]["total"].as_i64().unwrap() > r["conv"]["count"].as_i64().unwrap())
        .expect("an inbox thread with a reply elsewhere");
    let view = json!({ "source": { "folderId": inbox } });
    let kids = t.call("mail.listThread", json!({ "accountId": t.account_id, "threadId": r["threadId"], "view": view })).await;
    let kids = kids.as_array().unwrap();
    assert_eq!(kids.len() as i64, r["conv"]["total"].as_i64().unwrap());
    let kd: Vec<i64> = kids.iter().map(|k| k["date"].as_i64().unwrap()).collect();
    assert!(kd.windows(2).all(|w| w[0] >= w[1]), "newest first");
    let mut in_view: Vec<i64> = kids.iter().filter(|k| k["inView"] == true).map(|k| k["id"].as_i64().unwrap()).collect();
    in_view.sort();
    let mut cids = ids_of(&r["conv"]["ids"]);
    cids.sort();
    assert_eq!(in_view, cids);
    assert!(kids.iter().any(|k| k["inView"] == false && k["folder"] == "Sent Items"), "{kids:?}");
    // the same conversation in Sent Items shows the inbox messages as "elsewhere"
    let sent_list = t.call("mail.listMessages", json!({ "source": { "folderId": sent }, "conversations": true, "limit": 2000 })).await;
    let key = r["conv"]["key"].as_str().unwrap();
    let in_sent = sent_list["rows"].as_array().unwrap().iter().find(|x| x["conv"]["key"] == key).expect("conversation also in Sent Items");
    assert_eq!(in_sent["conv"]["total"], r["conv"]["total"]);
    // search results are grouped too
    let s = t
        .call(
            "mail.listMessages",
            json!({ "source": { "folderId": inbox }, "search": "RE:", "scope": "all", "conversations": true, "limit": 2000 }),
        )
        .await;
    assert!(s["crossFolder"].as_bool().unwrap());
    assert!(s["total"].as_i64().unwrap() <= s["messages"].as_i64().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn conversation_actions_apply_to_the_view() {
    let t = setup("convact+seed@duvz.demo").await;
    t.sync().await;
    let inbox = t.folder("inbox").await;
    let sent = t.folder("sent").await;
    let receipts = t.folder_by_path("Receipts").await;
    let conv = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "conversations": true, "limit": 2000 })).await;
    let r = conv["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["conv"]["total"].as_i64().unwrap() > r["conv"]["count"].as_i64().unwrap())
        .unwrap()
        .clone();
    let key = r["conv"]["key"].as_str().unwrap().to_string();
    let ids = ids_of(&r["conv"]["ids"]);
    let thread = json!({ "accountId": t.account_id, "threadId": r["threadId"] });
    let elsewhere: Vec<i64> = t
        .call("mail.listThread", thread.clone())
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["id"].as_i64().unwrap())
        .filter(|i| !ids.contains(i))
        .collect();
    assert!(!elsewhere.is_empty());
    // mark the conversation unread: every message of it in the Inbox, nothing elsewhere
    t.call("mail.markRead", json!({ "ids": ids, "read": false })).await;
    let find = |list: &Value| list["rows"].as_array().unwrap().iter().find(|x| x["conv"]["key"] == key.as_str()).cloned();
    let after = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "conversations": true, "limit": 2000 })).await;
    assert_eq!(find(&after).unwrap()["conv"]["unread"].as_i64().unwrap(), ids.len() as i64);
    // move the conversation: gone from the Inbox, the reply stays in Sent Items
    t.call("mail.move", json!({ "ids": ids, "folderId": receipts, "fromFolderId": inbox })).await;
    let after = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "conversations": true, "limit": 2000 })).await;
    assert!(find(&after).is_none());
    assert_eq!(after["total"].as_i64().unwrap(), conv["total"].as_i64().unwrap() - 1);
    let moved = t.call("mail.listMessages", json!({ "source": { "folderId": receipts }, "conversations": true, "limit": 2000 })).await;
    assert_eq!(ids_of(&find(&moved).unwrap()["conv"]["ids"]).len(), ids.len());
    let sent_ids: Vec<i64> = t.call("mail.listMessages", json!({ "source": { "folderId": sent }, "limit": 2000 })).await["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_i64().unwrap())
        .collect();
    assert!(elsewhere.iter().any(|e| sent_ids.contains(e)), "the reply is still in Sent Items");
    // delete the conversation: Deleted Items lists it; outside Deleted Items they're hidden
    t.call("mail.delete", json!({ "ids": ids })).await;
    let trash = t.folder("trash").await;
    let tl = t.call("mail.listMessages", json!({ "source": { "folderId": trash }, "conversations": true, "limit": 2000 })).await;
    let tr = find(&tl).expect("conversation in Deleted Items");
    assert_eq!(ids_of(&tr["conv"]["ids"]).len(), ids.len());
    let kids: Vec<i64> =
        t.call("mail.listThread", thread.clone()).await.as_array().unwrap().iter().map(|k| k["id"].as_i64().unwrap()).collect();
    assert!(ids.iter().all(|i| !kids.contains(i)), "deleted messages aren't shown outside Deleted Items");
    let mut with_view = thread.clone();
    with_view["view"] = json!({ "source": { "folderId": trash } });
    let kids = t.call("mail.listThread", with_view).await;
    assert_eq!(kids.as_array().unwrap().iter().filter(|k| k["inView"] == true).count(), ids.len());
    assert_eq!(tr["conv"]["total"].as_i64().unwrap(), kids.as_array().unwrap().len() as i64);
}

/// Conversation listing on a big mailbox: `cargo test --lib big_conversations -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore]
async fn big_conversations_performance() {
    let t = setup("perfconv+seed@duvz.demo").await;
    let n: u64 = std::env::var("CM_BIG").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000);
    t.x_demo("bulk", json!({ "count": n, "threadPct": 40 })).await;
    t.sync().await;
    let inbox = t.folder("inbox").await;
    let local = t.one("SELECT COUNT(*) FROM messages WHERE account_id=?1").await;
    println!("mailbox: {local} messages");
    for (arr, groups, conv, off) in [
        ("date", true, false, 0),
        ("date", true, true, 0),
        ("date", false, true, 0),
        ("date", true, true, 5000),
        ("from", true, true, 5000),
        ("size", true, true, 0),
        ("subject", true, true, 0),
    ] {
        // meta (counts + groups) and one page, like the list does
        let started = std::time::Instant::now();
        let m = t
            .call(
                "mail.listMessages",
                json!({ "source": { "folderId": inbox }, "arrange": arr, "withGroups": groups, "conversations": conv, "limit": 0 }),
            )
            .await;
        let meta_ms = started.elapsed().as_secs_f64() * 1000.0;
        let started = std::time::Instant::now();
        let r = t
            .call(
                "mail.listMessages",
                json!({ "source": { "folderId": inbox }, "arrange": arr, "conversations": conv, "offset": off, "limit": 200 }),
            )
            .await;
        let page_ms = started.elapsed().as_secs_f64() * 1000.0;
        println!(
            "{arr} groups={groups} conv={conv} offset={off}: {} rows / {} messages; meta {meta_ms:.1} ms, page {page_ms:.1} ms ({} rows)",
            m["total"],
            m["messages"],
            r["rows"].as_array().unwrap().len()
        );
    }
    let started = std::time::Instant::now();
    let r = t
        .call(
            "mail.listMessages",
            json!({ "source": { "folderId": inbox }, "search": "meridian", "conversations": true, "withGroups": true, "limit": 200 }),
        )
        .await;
    println!("search conv: {} conversations in {:.1} ms", r["total"], started.elapsed().as_secs_f64() * 1000.0);
    let row = r["rows"][0].clone();
    let started = std::time::Instant::now();
    let kids = t
        .call(
            "mail.listThread",
            json!({ "accountId": t.account_id, "threadId": row["threadId"], "view": { "source": { "folderId": inbox } } }),
        )
        .await;
    println!("listThread: {} messages in {:.1} ms", kids.as_array().unwrap().len(), started.elapsed().as_secs_f64() * 1000.0);
}

/// Backspace = Archive: Gmail semantics — out of the folder being viewed, kept in All Mail.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn archive_removes_only_the_viewed_folder() {
    let t = setup("archive+seed@duvz.demo").await;
    t.sync().await;
    let inbox = t.folder("inbox").await;
    let investors = t.folder_by_path("Investors").await;
    // two messages only in the Inbox (the seed is dated relative to now, so the newest few can
    // also be in Sent Items or a folder, and those don't count as archived)
    let ids: Vec<i64> = t
        .ctx
        .db
        .read(move |c| {
            let mut st = c.prepare(
                "SELECT m.id FROM messages m JOIN message_folders mf ON mf.message_id=m.id WHERE mf.folder_id=?1 \
                 AND (SELECT COUNT(*) FROM message_folders x WHERE x.message_id=m.id)=1 ORDER BY m.internal_date DESC LIMIT 2",
            )?;
            Ok(st.query_map([inbox], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?)
        })
        .await
        .unwrap();
    assert_eq!(ids.len(), 2);
    // put one of them in Investors too, then archive both from the Inbox
    t.call("mail.copy", json!({ "ids": [ids[1]], "folderId": investors })).await;
    t.call("mail.archive", json!({ "ids": [ids[0], ids[1]], "fromFolderId": inbox })).await;
    let after = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "limit": 50 })).await;
    let left: Vec<i64> = after["rows"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
    assert!(!left.contains(&ids[0]) && !left.contains(&ids[1]), "archived messages leave the Inbox");
    // the Archive folder shows archived mail: ids[0] (no other folder); ids[1] is still in Investors
    let arch =
        t.call("mail.listMessages", json!({ "source": { "searchFolder": "archive", "accountId": t.account_id }, "limit": 500 })).await;
    let in_arch: Vec<i64> = arch["rows"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
    assert!(in_arch.contains(&ids[0]), "archived message is in Archive");
    assert!(!in_arch.contains(&ids[1]), "a message still in Investors is not in Archive");
    t.settle().await;
    let g1 = t.gmail_id(ids[1]).await;
    let labels = t.remote_labels(&g1).await;
    assert!(!labels.contains(&"INBOX".to_string()), "{labels:?}");
    let inv0: String = t.q("SELECT gmail_label_id FROM folders WHERE account_id=?1 AND path='Investors'").await;
    assert!(labels.contains(&inv0), "other labels are kept: {labels:?}");
    // archiving from Investors removes that label only
    t.call("mail.archive", json!({ "ids": [ids[1]], "fromFolderId": investors })).await;
    t.settle().await;
    let labels = t.remote_labels(&g1).await;
    let inv: String = t.q("SELECT gmail_label_id FROM folders WHERE account_id=?1 AND path='Investors'").await;
    assert!(!labels.contains(&inv), "Investors label removed: {labels:?}");
    // still exists (All Mail) — not trashed
    assert!(!labels.contains(&"TRASH".to_string()));
}

/// Pictures load automatically from senders in your contacts and from senders whose pictures you
/// downloaded before (setting mail.autoPicturesKnownSenders, on by default).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pictures_load_for_known_senders() {
    let t = setup("pics+seed@duvz.demo").await;
    t.sync().await;
    let nl = t.one("SELECT id FROM messages WHERE account_id=?1 AND from_email LIKE 'news@capmarkets%' ORDER BY size DESC LIMIT 1").await;
    let sender: String = t.q("SELECT from_email FROM messages WHERE account_id=?1 AND from_email LIKE 'news@capmarkets%' LIMIT 1").await;
    let m = t.call("mail.getMessage", json!({ "id": nl })).await;
    assert_eq!(m["remoteBlocked"], true, "unknown sender: blocked");
    // the sender becomes a contact → pictures load
    let (acct, s2) = (t.account_id, sender.clone());
    t.ctx
        .db
        .write(move |c| {
            c.execute(
                "INSERT INTO contacts(account_id, display_name, emails) VALUES (?1, 'Cap Markets', json_array(json_object('value', ?2, 'type', 'work')))",
                rusqlite::params![acct, s2.to_uppercase()],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let m = t.call("mail.getMessage", json!({ "id": nl })).await;
    assert_eq!(m["remoteBlocked"], false, "contact: pictures load");
    // turned off → blocked again
    t.call("settings.set", json!({ "key": "mail.autoPicturesKnownSenders", "value": false })).await;
    let m = t.call("mail.getMessage", json!({ "id": nl })).await;
    assert_eq!(m["remoteBlocked"], true, "setting off");
    t.call("settings.set", json!({ "key": "mail.autoPicturesKnownSenders", "value": true })).await;
    // a sender you downloaded pictures from before (not a contact)
    t.ctx
        .db
        .write(|c| {
            c.execute("DELETE FROM contacts", [])?;
            Ok(())
        })
        .await
        .unwrap();
    t.call("mail.getMessage", json!({ "id": nl, "allowRemote": true })).await;
    let other = t.one("SELECT COALESCE((SELECT id FROM messages WHERE account_id=?1 AND from_email LIKE 'news@capmarkets%' ORDER BY size DESC LIMIT 1 OFFSET 1), 0)").await;
    if other > 0 {
        let m = t.call("mail.getMessage", json!({ "id": other })).await;
        assert_eq!(m["remoteBlocked"], false, "earlier download from this sender: pictures load");
    }
}

#[tokio::test]
async fn background_download_all_bodies_and_attachments() {
    let t = setup("kenny+seed@duvz.demo").await;
    t.sync().await;
    const LEFT: &str = "SELECT COUNT(*) FROM messages WHERE account_id=?1 AND body_state=0 AND location=0";
    // default: only the last 30 days
    let scope = t.ctx.db.read(super::sync::download_scope).await.unwrap().unwrap();
    assert!(scope.since > 0 && !scope.attachments);
    super::sync::prefetch(&t.ctx, t.account_id).await.unwrap();
    let before: i64 = t.q(LEFT).await;
    assert!(before > 0, "older mail has no body yet");
    // Options › Mail estimates: the 30 days are done, all mail still has `before` to go
    let est = t.call("mail.downloadEstimate", json!({ "days": 30 })).await;
    assert_eq!(est["recent"]["bodiesLeft"], 0);
    assert_eq!(est["all"]["bodiesLeft"], before);
    assert!(est["all"]["bodiesBytes"].as_i64().unwrap() > 0);
    // Options › Mail › download all mail (+ attachments)
    t.ctx
        .db
        .write(|c| {
            crate::db::util::set_setting(c, "mail.downloadAllBodies", &true)?;
            crate::db::util::set_setting(c, "mail.downloadAttachments", &true)
        })
        .await
        .unwrap();
    for _ in 0..40 {
        super::sync::prefetch(&t.ctx, t.account_id).await.unwrap();
        let atts_left: i64 = t
            .ctx
            .db
            .read(|c| Ok(c.query_row("SELECT COUNT(*) FROM attachments WHERE cached_path IS NULL", [], |r| r.get(0))?))
            .await
            .unwrap();
        if t.q::<i64>(LEFT).await == 0 && atts_left == 0 {
            break;
        }
    }
    assert_eq!(t.q::<i64>(LEFT).await, 0, "every body downloaded");
    let (atts, cached): (i64, i64) = t
        .ctx
        .db
        .read(|c| Ok(c.query_row("SELECT COUNT(*), COUNT(cached_path) FROM attachments", [], |r| Ok((r.get(0)?, r.get(1)?)))?))
        .await
        .unwrap();
    assert!(atts > 0);
    assert_eq!(cached, atts, "attachments downloaded too");
    // progress for the status bar
    let ev = t.ctx.events.captured();
    let last = ev.iter().rev().find(|e| e["kind"] == "mail.download").expect("mail.download event");
    assert_eq!(last["bodies"], 0);
    assert_eq!(last["attachments"], 0);
}

/// Two fetches of one body racing (prefetch + opening the message): the second must not replace the
/// attachment rows the first one stored, so ids already handed to the reading pane stay valid.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_body_fetches_keep_attachment_ids() {
    let t = setup("kenny+seed@duvz.demo").await;
    t.sync().await;
    let id = t
        .one(
            "SELECT id FROM messages WHERE account_id=?1 AND has_attachments=1 AND body_state=0 AND location=0 \
             ORDER BY internal_date DESC LIMIT 1",
        )
        .await;
    let a = super::body::fetch_body(&t.ctx, id).await.unwrap().expect("not fetched yet");
    let b = super::body::fetch_body(&t.ctx, id).await.unwrap().expect("not fetched yet");
    let ids = |t: &T| {
        let ctx = t.ctx.clone();
        async move {
            ctx.db
                .read(move |c| {
                    let mut st = c.prepare("SELECT id FROM attachments WHERE message_id=?1 ORDER BY id")?;
                    Ok(st.query_map([id], |r| r.get::<_, i64>(0))?.collect::<Result<Vec<_>, _>>()?)
                })
                .await
                .unwrap()
        }
    };
    t.ctx.db.write(move |c| super::body::store_body(c, &a)).await.unwrap();
    let first = ids(&t).await;
    assert!(!first.is_empty(), "message has attachments");
    t.ctx.db.write(move |c| super::body::store_body(c, &b)).await.unwrap();
    assert_eq!(ids(&t).await, first);
}

/// Emptying Deleted Items removes the cached attachment files of the messages deleted for good.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn empty_folder_drops_cached_files() {
    let t = setup("kenny+seed@duvz.demo").await;
    t.sync().await;
    let trash = t.folder("trash").await;
    let id = t.one("SELECT id FROM messages WHERE account_id=?1 AND location=1 LIMIT 1").await;
    let dir = t.ctx.data_dir.join("attachments").join(id.to_string());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("1_report.pdf"), b"%PDF").unwrap();
    t.call("mail.emptyFolder", json!({ "folderId": trash })).await;
    assert!(!dir.exists(), "cached files of {id} removed");
}

/// The warning before Delete counts what it erases for good: Deleted Items only (Junk goes to
/// Deleted Items like the Inbox does) — the same rule as the delete itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_info_counts_deleted_items() {
    let t = setup("kenny+seed@duvz.demo").await;
    t.sync().await;
    let inbox = t.folder("inbox").await;
    let rows = t.call("mail.listMessages", json!({ "source": { "folderId": inbox }, "limit": 3 })).await;
    let ids: Vec<i64> = rows["rows"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
    t.call("mail.delete", json!({ "ids": [ids[1]] })).await;
    t.call("mail.markJunk", json!({ "ids": [ids[2]] })).await;
    let info = t.call("mail.deleteInfo", json!({ "ids": ids })).await;
    assert_eq!(info["permanent"], 1);
    let r = t.call("mail.delete", json!({ "ids": ids })).await;
    assert_eq!(r["permanent"], info["permanent"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn folders_inside_inbox_and_dragged_around() {
    let t = setup("kenny+seed@duvz.demo").await;
    t.sync().await;
    let inbox = t.folder("inbox").await;
    let path = |id: i64| {
        let ctx = t.ctx.clone();
        async move { ctx.db.read(move |c| Ok(super::store::folder(c, id)?.path)).await.unwrap() }
    };
    // New Folder in the Inbox: a label "Inbox/<name>" (so "Important" is fine there)
    let imp = t.call("mail.createFolder", json!({ "parentId": inbox, "name": "Important" })).await.as_i64().unwrap();
    assert_eq!(path(imp).await, "Inbox/Important");
    let clients = t.call("mail.createFolder", json!({ "accountId": t.account_id, "name": "Ventures 2030" })).await.as_i64().unwrap();
    let acme = t.call("mail.createFolder", json!({ "parentId": clients, "name": "Kestrel" })).await.as_i64().unwrap();
    let mid = t.one("SELECT m.id FROM messages m JOIN message_folders mf ON mf.message_id=m.id JOIN folders f ON f.id=mf.folder_id WHERE m.account_id=?1 AND f.role='inbox' LIMIT 1").await;
    t.call("mail.move", json!({ "ids": [mid], "folderId": acme, "fromFolderId": inbox })).await;

    // drag Clients (with Kestrel and its mail) into the Inbox
    t.call("mail.moveFolder", json!({ "folderId": clients, "parentId": inbox })).await;
    assert_eq!(path(clients).await, "Inbox/Ventures 2030");
    assert_eq!(path(acme).await, "Inbox/Ventures 2030/Kestrel");
    let in_acme: i64 = t
        .ctx
        .db
        .read(move |c| {
            Ok(c.query_row("SELECT COUNT(*) FROM message_folders WHERE message_id=?1 AND folder_id=?2", params![mid, acme], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(in_acme, 1);
    // into another folder, then back to the top of the mailbox
    t.call("mail.moveFolder", json!({ "folderId": acme, "parentId": imp })).await;
    assert_eq!(path(acme).await, "Inbox/Important/Kestrel");
    t.call("mail.moveFolder", json!({ "folderId": acme, "accountId": t.account_id })).await;
    assert_eq!(path(acme).await, "Kestrel");
    // not into itself or its own subfolder
    t.call("mail.moveFolder", json!({ "folderId": acme, "parentId": clients })).await;
    let err = t.reg.call(t.ctx.clone(), "mail.moveFolder", json!({ "folderId": clients, "parentId": acme })).await.unwrap_err();
    assert!(err.to_string().contains("into itself"), "{err}");
    // Inbox itself doesn't move
    let err = t.reg.call(t.ctx.clone(), "mail.moveFolder", json!({ "folderId": inbox, "parentId": clients })).await.unwrap_err();
    assert!(err.to_string().contains("can't be moved"), "{err}");
}

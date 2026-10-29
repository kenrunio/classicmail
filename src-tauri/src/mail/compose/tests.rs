//! Integration tests: compose → Outbox → fake Gmail, drafts, replies, failures.

use std::sync::Arc;

use mail_parser::{MessageParser, MimeHeaders};
use rusqlite::params;
use serde_json::{Value, json};

use crate::{
    accounts,
    ctx::AppCtx,
    db::tempdir::TempDirGuard,
    fake_google::{self, Shared},
    google::Api,
    rpc,
};

const ME: &str = "kenny@duvz.demo";

/// The fake server is a process-wide singleton (`fake_google::server()`): run these one at a time.
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct T {
    ctx: Arc<AppCtx>,
    _g: TempDirGuard,
    acct: i64,
    api: Api,
    srv: Shared,
    drafts_folder: i64,
    outbox_folder: i64,
}

async fn setup() -> T {
    let (ctx, g) = AppCtx::for_test().unwrap();
    let acct = accounts::upsert(&ctx.db, ME, "Kenny Green", "demo").await.unwrap();
    let base = ctx.fake_base().await.unwrap();
    // this test's own server (the global `server()` is whichever test started one last)
    let srv = fake_google::server_at(&base).expect("fake server");
    let api = ctx.api(acct.id).await.unwrap();
    // make sure we hold the server our ctx talks to
    api.gmail_send_as().await.unwrap();
    assert!(srv.read(|st| st.users.contains_key(ME)), "fake server singleton raced with another test");
    let a = acct.id;
    let (drafts_folder, outbox_folder) = ctx
        .db
        .write(move |c| {
            c.execute(
                "INSERT INTO folders(account_id, gmail_label_id, name, path, role) VALUES (?1,'DRAFT','Drafts','DRAFT','drafts')",
                [a],
            )?;
            let d = c.last_insert_rowid();
            c.execute("INSERT INTO folders(account_id, name, path, role) VALUES (?1,'Outbox','Outbox','outbox')", [a])?;
            Ok((d, c.last_insert_rowid()))
        })
        .await
        .unwrap();
    T { ctx, _g: g, acct: acct.id, api, srv, drafts_folder, outbox_folder }
}

async fn call(t: &T, method: &str, params: Value) -> anyhow::Result<Value> {
    rpc::build().call(t.ctx.clone(), method, params).await
}

fn sent_messages(t: &T) -> Vec<(String, String, Vec<String>, String)> {
    t.srv.read(|st| {
        st.user(ME)
            .gmail
            .messages
            .values()
            .filter(|m| m.label_ids.iter().any(|l| l == "SENT"))
            .map(|m| (m.id.clone(), m.thread_id.clone(), m.label_ids.clone(), m.raw.clone()))
            .collect()
    })
}

async fn outbox_count(t: &T) -> (i64, i64) {
    let f = t.outbox_folder;
    t.ctx
        .db
        .read(move |c| {
            Ok((
                c.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))?,
                c.query_row("SELECT total_count FROM folders WHERE id=?1", [f], |r| r.get(0))?,
            ))
        })
        .await
        .unwrap()
}

fn tiny_png_data_url() -> String {
    use base64::Engine;
    let bytes: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[tokio::test]
async fn send_round_trip_json() {
    let _l = LOCK.lock().await;
    let t = setup().await;
    let file = t.ctx.data_dir.join("תקציר Q3.txt");
    std::fs::write(&file, "Revenue up 12%").unwrap();
    let compose = json!({
        "accountId": t.acct,
        "to": [{ "name": "שרה כהן", "email": "sarah.cohen@harborcap.example" }],
        "cc": [{ "name": "", "email": "david.levi@meridian.example" }],
        "subject": "דוח רבעוני – Q3",
        "html": format!("<p dir=\"auto\">שלום <b>שרה</b>, מצורף הדוח.</p><p><img src=\"{}\"></p>", tiny_png_data_url()),
        "attachments": [{ "path": file.to_string_lossy() }],
        "importance": "high",
        "requestReadReceipt": true,
    });
    let q = call(&t, "mail.compose.send", json!({ "compose": compose })).await.unwrap();
    assert!(q["outboxId"].as_i64().unwrap() > 0);
    assert_eq!(outbox_count(&t).await, (1, 1));
    let list = call(&t, "mail.outbox.list", json!({})).await.unwrap();
    assert_eq!(list[0]["subject"], "דוח רבעוני – Q3");
    assert_eq!(list[0]["status"], "queued");

    crate::mail::compose::process_outbox(&t.ctx, &t.api).await.unwrap();
    assert_eq!(outbox_count(&t).await, (0, 0));
    let sent = sent_messages(&t);
    assert_eq!(sent.len(), 1);
    let m = MessageParser::default().parse(sent[0].3.as_bytes()).unwrap();
    assert_eq!(m.subject().unwrap(), "דוח רבעוני – Q3");
    assert_eq!(m.to().unwrap().first().unwrap().name().unwrap(), "שרה כהן");
    assert_eq!(m.from().unwrap().first().unwrap().address().unwrap(), ME);
    assert!(m.body_html(0).unwrap().contains("שלום <b>שרה</b>"));
    let names: Vec<String> = m.attachments().filter_map(|a| a.attachment_name().map(str::to_string)).collect();
    assert!(names.contains(&"תקציר Q3.txt".to_string()), "{names:?}");
    assert!(m.attachments().any(|a| a.content_id().is_some()), "inline image");
    assert!(!sent[0].2.contains(&"INBOX".to_string()));
    // auto-complete learned the recipients
    let s = call(&t, "mail.recipients.suggest", json!({ "query": "sar" })).await.unwrap();
    assert_eq!(s[0]["email"], "sarah.cohen@harborcap.example");
    assert_eq!(s[0]["name"], "שרה כהן");
    let s = call(&t, "mail.recipients.suggest", json!({ "query": "כהן" })).await.unwrap();
    assert_eq!(s.as_array().unwrap().len(), 1);
    // events
    let ev = t.ctx.events.captured();
    assert!(ev.iter().any(|e| e["kind"] == "mail.changed"));
    // send-as were synced on the way
    let sa = call(&t, "mail.sendAs.list", json!({ "accountId": t.acct })).await.unwrap();
    assert_eq!(sa[0]["email"], ME);
}

#[tokio::test]
async fn send_large_message_uses_upload() {
    let _l = LOCK.lock().await;
    let t = setup().await;
    let file = t.ctx.data_dir.join("deck.pdf");
    let mut big = vec![0u8; 5 * 1024 * 1024];
    for (i, b) in big.iter_mut().enumerate() {
        *b = (i * 31 % 251) as u8;
    }
    std::fs::write(&file, &big).unwrap();
    let compose = json!({
        "accountId": t.acct,
        "to": [{ "email": "info@duvz.demo" }, { "email": ME }],
        "subject": "Investor deck",
        "html": "<p>Deck attached</p>",
        "attachments": [{ "path": file.to_string_lossy() }],
    });
    call(&t, "mail.compose.send", json!({ "compose": compose })).await.unwrap();
    crate::mail::compose::process_outbox(&t.ctx, &t.api).await.unwrap();
    let sent = sent_messages(&t);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].3.len() > 4 * 1024 * 1024);
    // sent to myself too → also in the Inbox
    assert!(sent[0].2.contains(&"INBOX".to_string()));
    let m = MessageParser::default().parse(sent[0].3.as_bytes()).unwrap();
    let a = m.attachments().find(|a| a.attachment_name() == Some("deck.pdf")).unwrap();
    assert_eq!(a.contents(), &big[..]);
}

/// A received message in the fake mailbox + local DB, for reply / forward tests.
async fn insert_source(t: &T) -> (i64, String) {
    let raw = "From: Sarah Cohen <sarah.cohen@harborcap.example>\r\nTo: Kenny Green <kenny@duvz.demo>, David Levi <david.levi@meridian.example>\r\n\
               Cc: info@duvz.demo\r\nSubject: Q3 call questions\r\nMessage-ID: <orig-123@harborcap.example>\r\n\
               References: <root-1@harborcap.example>\r\nDate: Thu, 08 Oct 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\n\
               Content-Type: text/html; charset=utf-8\r\n\r\n<p>Hi Kenny, <script>x()</script>questions attached.</p>\r\n"
        .to_string();
    let now = crate::db::util::now_ms();
    let gid = t.srv.with(|st| st.user(ME).gmail.add_message(raw, vec!["INBOX".into()], None, now));
    let a = t.acct;
    let att_path = t.ctx.data_dir.join("questions.xlsx");
    std::fs::write(&att_path, b"PK fake xlsx").unwrap();
    let ap = att_path.to_string_lossy().into_owned();
    let g2 = gid.clone();
    let id = t
        .ctx
        .db
        .write(move |c| {
            c.execute(
                "INSERT INTO messages(account_id, gmail_id, thread_id, internal_date, sent_date, subject, from_name, from_email, \
                   to_json, cc_json, message_id_header, references_header, has_attachments, body_state) \
                 VALUES (?1,?2,?2,?3,?3,'Q3 call questions','Sarah Cohen','sarah.cohen@harborcap.example',?4,?5, \
                   '<orig-123@harborcap.example>','<root-1@harborcap.example>',1,1)",
                params![
                    a,
                    g2,
                    now,
                    r#"[{"name":"Kenny Green","email":"kenny@duvz.demo"},{"name":"David Levi","email":"david.levi@meridian.example"}]"#,
                    r#"[{"name":"","email":"info@duvz.demo"}]"#
                ],
            )?;
            let id = c.last_insert_rowid();
            c.execute(
                "INSERT INTO message_bodies(message_id, html, text, fetched_at) VALUES (?1, '<p>Hi Kenny, <script>x()</script>questions attached.</p>', 'Hi Kenny', 0)",
                [id],
            )?;
            c.execute(
                "INSERT INTO attachments(message_id, filename, mime_type, size, cached_path) VALUES (?1,'questions.xlsx', \
                 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', 12, ?2)",
                params![id, ap],
            )?;
            Ok(id)
        })
        .await
        .unwrap();
    (id, gid)
}

#[tokio::test]
async fn reply_threading_and_prepare() {
    let _l = LOCK.lock().await;
    let t = setup().await;
    // a signature for replies
    let sig = call(&t, "mail.signatures.save", json!({ "name": "Short", "html": "<p>Kenny</p>" })).await.unwrap();
    call(&t, "mail.signatures.setDefaults", json!({ "accountId": t.acct, "newSignatureId": null, "replySignatureId": sig["id"] }))
        .await
        .unwrap();
    let (src, gid) = insert_source(&t).await;

    let p = call(&t, "mail.compose.prepare", json!({ "mode": "replyAll", "id": src })).await.unwrap();
    let c = &p["compose"];
    assert_eq!(c["subject"], "RE: Q3 call questions");
    let to: Vec<&str> = c["to"].as_array().unwrap().iter().map(|a| a["email"].as_str().unwrap()).collect();
    assert_eq!(to, ["sarah.cohen@harborcap.example", "david.levi@meridian.example"]);
    // info@ is not an account in this test, so it stays on Cc
    assert_eq!(c["cc"][0]["email"], "info@duvz.demo");
    assert!(p["bodyHtml"].as_str().unwrap().contains("<p>Kenny</p>"));
    let q = p["quotedHtml"].as_str().unwrap();
    assert!(q.contains("-----Original Message-----"));
    assert!(q.contains("Sarah Cohen [mailto:sarah.cohen@harborcap.example]"));
    assert!(!q.contains("script"));
    assert_eq!(c["attachments"].as_array().unwrap().len(), 0);
    // Reply to One / Reply All toggle targets
    let emails = |v: &Value| v.as_array().unwrap().iter().map(|a| a["email"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let rt = &p["replyTargets"];
    assert_eq!(emails(&rt["one"]["to"]), ["sarah.cohen@harborcap.example"]);
    assert!(emails(&rt["one"]["cc"]).is_empty());
    assert_eq!(emails(&rt["all"]["to"]), ["sarah.cohen@harborcap.example", "david.levi@meridian.example"]);
    assert_eq!(emails(&rt["all"]["cc"]), ["info@duvz.demo"]);
    let one = call(&t, "mail.compose.prepare", json!({ "mode": "reply", "id": src })).await.unwrap();
    assert_eq!(emails(&one["compose"]["to"]), ["sarah.cohen@harborcap.example"]);
    assert_eq!(one["replyTargets"], p["replyTargets"]);

    let fw = call(&t, "mail.compose.prepare", json!({ "mode": "forward", "id": src })).await.unwrap();
    assert_eq!(fw["compose"]["subject"], "FW: Q3 call questions");
    assert_eq!(fw["compose"]["attachments"][0]["name"], "questions.xlsx");
    assert!(fw["compose"]["to"].as_array().unwrap().is_empty());

    // send the reply
    let mut compose = p["compose"].clone();
    compose["html"] = json!(format!("<p>Thanks Sarah</p>{q}"));
    call(&t, "mail.compose.send", json!({ "compose": compose })).await.unwrap();
    crate::mail::compose::process_outbox(&t.ctx, &t.api).await.unwrap();
    let sent = sent_messages(&t);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1, gid, "reply joins the original's Gmail thread");
    let m = MessageParser::default().parse(sent[0].3.as_bytes()).unwrap();
    assert_eq!(m.in_reply_to().as_text().unwrap(), "orig-123@harborcap.example");
    let refs = m.references().as_text_list().unwrap();
    assert_eq!(refs, ["root-1@harborcap.example", "orig-123@harborcap.example"]);
    let (verb, at): (Option<String>, Option<i64>) = t
        .ctx
        .db
        .read(move |c| Ok(c.query_row("SELECT last_verb, last_verb_at FROM messages WHERE id=?1", [src], |r| Ok((r.get(0)?, r.get(1)?)))?))
        .await
        .unwrap();
    assert_eq!(verb.as_deref(), Some("repliedAll"));
    assert!(at.is_some());

    // forward with the original attachment (served from the cached copy)
    let mut fc = fw["compose"].clone();
    fc["to"] = json!([{ "name": "Yael", "email": "yael.friedman@tamarinv.example" }]);
    fc["html"] = json!(format!("<p>FYI</p>{}", fw["quotedHtml"].as_str().unwrap()));
    call(&t, "mail.compose.send", json!({ "compose": fc })).await.unwrap();
    crate::mail::compose::process_outbox(&t.ctx, &t.api).await.unwrap();
    let sent = sent_messages(&t);
    let f = sent.iter().find(|s| s.3.contains("FW: Q3")).unwrap();
    let m = MessageParser::default().parse(f.3.as_bytes()).unwrap();
    assert_eq!(m.attachments().find(|a| a.attachment_name() == Some("questions.xlsx")).unwrap().contents(), b"PK fake xlsx");

    // Gmail's rule: an API send joins a conversation only with threadId + In-Reply-To/References +
    // a matching Subject. In-Reply-To alone starts a new conversation.
    let raw = "From: kenny@duvz.demo\r\nTo: sarah.cohen@harborcap.example\r\nSubject: RE: Q3 call questions\r\n\
               Message-ID: <x-1@duvz.demo>\r\nIn-Reply-To: <orig-123@harborcap.example>\r\n\r\nhello\r\n";
    let r = t.api.gmail_send(raw.as_bytes(), None).await.unwrap();
    assert_ne!(r.thread_id, gid, "no threadId → new thread");
    let r = t.api.gmail_send(raw.as_bytes(), Some(&gid)).await.unwrap();
    assert_eq!(r.thread_id, gid);
    let other = raw.replace("RE: Q3 call questions", "Something else");
    let r = t.api.gmail_send(other.as_bytes(), Some(&gid)).await.unwrap();
    assert_ne!(r.thread_id, gid, "subject changed → Gmail starts a new thread");
    // a big reply goes through the multipart upload and still carries its threadId
    let big = format!("{raw}{}\r\n", "x".repeat(3 * 1024 * 1024));
    let r = t.api.gmail_send(big.as_bytes(), Some(&gid)).await.unwrap();
    assert_eq!(r.thread_id, gid, "multipart upload keeps the thread");
    // threadId as a query parameter is not part of the API (Google: 400 Cannot bind query parameter)
    let url = format!("{}/users/me/messages/send", t.api.urls.gmail_upload);
    let e = t
        .api
        .call_bytes(
            reqwest::Method::POST,
            &url,
            &[("uploadType", "media".into()), ("threadId", gid.clone())],
            crate::google::client::Body::Raw { content_type: "message/rfc822".into(), bytes: raw.as_bytes().to_vec() },
        )
        .await
        .unwrap_err();
    assert_eq!(crate::google::client::api_status(&e), Some(400));
}

#[tokio::test]
async fn drafts_create_update_open_send_delete() {
    let _l = LOCK.lock().await;
    let t = setup().await;
    let file = t.ctx.data_dir.join("agenda.docx");
    std::fs::write(&file, b"agenda bytes").unwrap();
    let mut compose = json!({
        "accountId": t.acct,
        "to": [{ "name": "Noa Shapiro", "email": "noa.shapiro@oriontech.example" }],
        "subject": "Roadshow agenda",
        "html": format!("<p>Draft one</p><p><img src=\"{}\"></p>", tiny_png_data_url()),
        "attachments": [{ "path": file.to_string_lossy() }],
        "importance": "low",
    });
    let s1 = call(&t, "mail.compose.saveDraft", json!({ "compose": compose })).await.unwrap();
    let draft_id = s1["draftId"].as_str().unwrap().to_string();
    let local1 = s1["messageId"].as_i64().unwrap();
    let (n, gm1) = t.srv.read(|st| {
        let g = &st.user(ME).gmail;
        (g.drafts.len(), g.drafts[&draft_id].message_id.clone())
    });
    assert_eq!(n, 1);
    assert!(t.srv.read(|st| st.user(ME).gmail.messages[&gm1].label_ids == vec!["DRAFT".to_string()]));
    let df = t.drafts_folder;
    let in_drafts: i64 = t
        .ctx
        .db
        .read(move |c| Ok(c.query_row("SELECT COUNT(*) FROM message_folders WHERE folder_id=?1", [df], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(in_drafts, 1);
    // the Drafts list shows the recipient, sorts it by the save time and Instant Search finds it
    let (to_display, msg_date, found): (String, i64, i64) = t
        .ctx
        .db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT m.to_display, mf.msg_date, (SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'roadshow' AND rowid=m.id) \
                 FROM messages m JOIN message_folders mf ON mf.message_id=m.id WHERE m.id=?1",
                [local1],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(to_display, "Noa Shapiro");
    assert!(msg_date > 0);
    assert_eq!(found, 1);

    // update: same draft id, new message, local row replaced
    compose["draftId"] = json!(draft_id);
    compose["html"] = json!(format!("<p>Draft two – עדכון</p><p><img src=\"{}\"></p>", tiny_png_data_url()));
    let s2 = call(&t, "mail.compose.saveDraft", json!({ "compose": compose })).await.unwrap();
    assert_eq!(s2["draftId"], json!(draft_id));
    let local2 = s2["messageId"].as_i64().unwrap();
    let _ = local1; // the row is replaced (SQLite may reuse the id)
    let gm2 = t.srv.read(|st| st.user(ME).gmail.drafts[&draft_id].message_id.clone());
    assert_ne!(gm1, gm2);
    assert!(t.srv.read(|st| !st.user(ME).gmail.messages.contains_key(&gm1)));
    let rows: i64 = t.ctx.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM messages WHERE is_draft=1", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(rows, 1);

    // reopen from Drafts
    let o = call(&t, "mail.compose.openDraft", json!({ "id": local2 })).await.unwrap();
    assert_eq!(o["compose"]["subject"], "Roadshow agenda");
    assert_eq!(o["compose"]["draftId"], json!(draft_id));
    assert_eq!(o["compose"]["importance"], "low");
    assert_eq!(o["compose"]["to"][0]["email"], "noa.shapiro@oriontech.example");
    assert!(o["bodyHtml"].as_str().unwrap().contains("Draft two – עדכון"));
    assert!(o["bodyHtml"].as_str().unwrap().contains("data:image/png;base64,"), "inline image back as data URL");
    let att = &o["compose"]["attachments"][0];
    assert_eq!(att["name"], "agenda.docx");
    assert_eq!(std::fs::read(att["path"].as_str().unwrap()).unwrap(), b"agenda bytes");
    assert_eq!(o["draftMessageId"], json!(local2));

    // send it: Gmail draft and local row go away, message is in Sent
    let mut c2 = o["compose"].clone();
    c2["html"] = json!(o["bodyHtml"]);
    call(&t, "mail.compose.send", json!({ "compose": c2 })).await.unwrap();
    let rows: i64 = t.ctx.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM messages WHERE is_draft=1", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(rows, 0, "draft leaves Drafts when queued");
    crate::mail::compose::process_outbox(&t.ctx, &t.api).await.unwrap();
    assert!(t.srv.read(|st| st.user(ME).gmail.drafts.is_empty()));
    assert_eq!(sent_messages(&t).len(), 1);

    // discard
    let s3 = call(&t, "mail.compose.saveDraft", json!({ "compose": { "accountId": t.acct, "subject": "tmp", "html": "<p>x</p>" } }))
        .await
        .unwrap();
    let local = s3["messageId"].as_i64().unwrap();
    let indexed = |local: i64| {
        t.ctx.db.read(move |c| {
            Ok(c.query_row("SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'tmp' AND rowid=?1", [local], |r| {
                r.get::<_, i64>(0)
            })?)
        })
    };
    assert_eq!(indexed(local).await.unwrap(), 1, "the draft is searchable");
    call(&t, "mail.compose.discardDraft", json!({ "accountId": t.acct, "draftId": s3["draftId"] })).await.unwrap();
    // its full-text row goes with it (a new message reusing the id must not match the old text)
    assert_eq!(indexed(local).await.unwrap(), 0, "index row removed with the draft");
    assert!(t.srv.read(|st| st.user(ME).gmail.drafts.is_empty()));
    let rows: i64 = t.ctx.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM messages WHERE is_draft=1", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(rows, 0);

    // drafts.send / list through the wrappers
    let s4 = call(
        &t,
        "mail.compose.saveDraft",
        json!({ "compose": {
        "accountId": t.acct, "to": [{ "email": ME }], "subject": "note to self", "html": "<p>x</p>" } }),
    )
    .await
    .unwrap();
    let l = t.api.gmail_list_drafts(None).await.unwrap();
    assert_eq!(l.drafts.len(), 1);
    let m = t.api.gmail_send_draft(s4["draftId"].as_str().unwrap()).await.unwrap();
    assert!(m.label_ids.contains(&"SENT".to_string()) && m.label_ids.contains(&"INBOX".to_string()));
    assert!(!m.label_ids.contains(&"DRAFT".to_string()));
}

#[tokio::test]
async fn outbox_failure_retry_and_deferred() {
    let _l = LOCK.lock().await;
    let t = setup().await;
    // a message without recipients gets past nothing but the API (inserted directly)
    let bad = crate::mail::compose::model::Compose {
        account_id: t.acct,
        subject: "Broken".into(),
        html: "<p>x</p>".into(),
        ..Default::default()
    };
    let built = crate::mail::compose::outbox::build(&t.ctx, &bad).await.unwrap();
    let a = t.acct;
    let cj = serde_json::to_string(&bad).unwrap();
    let raw = built.raw.clone();
    let id: i64 = t
        .ctx
        .db
        .write(move |c| {
            c.execute(
                "INSERT INTO outbox(account_id, compose_json, raw, status, send_at, created_at) VALUES (?1,?2,?3,'queued',0,?4)",
                params![a, cj, raw, crate::db::util::now_ms()],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await
        .unwrap();
    let err = crate::mail::compose::process_outbox(&t.ctx, &t.api).await.unwrap_err();
    assert!(err.to_string().contains("remain in the Outbox"), "{err}");
    let list = call(&t, "mail.outbox.list", json!({ "accountId": t.acct })).await.unwrap();
    assert_eq!(list[0]["status"], "failed");
    assert_eq!(list[0]["attempts"], 1);
    assert!(list[0]["lastError"].as_str().unwrap().contains("Invalid To header"));
    // not retried before its back-off...
    crate::mail::compose::process_outbox(&t.ctx, &t.api).await.unwrap();
    assert_eq!(outbox_count(&t).await.0, 1);
    // ...fix it and retry now
    let good = crate::mail::compose::model::Compose {
        to: vec![crate::mail::compose::model::Addr::new("Sarah", "sarah.cohen@harborcap.example")],
        ..bad.clone()
    };
    let raw2 = crate::mail::compose::outbox::build(&t.ctx, &good).await.unwrap().raw;
    t.ctx
        .db
        .write(move |c| {
            c.execute("UPDATE outbox SET raw=?1 WHERE id=?2", params![raw2, id])?;
            Ok(())
        })
        .await
        .unwrap();
    call(&t, "mail.outbox.retry", json!({ "id": id })).await.unwrap();
    crate::mail::compose::process_outbox(&t.ctx, &t.api).await.unwrap();
    assert_eq!(outbox_count(&t).await, (0, 0));
    assert_eq!(sent_messages(&t).len(), 1);

    // deferred delivery
    let later = crate::db::util::now_ms() + 3_600_000;
    let q = call(
        &t,
        "mail.compose.send",
        json!({ "compose": {
        "accountId": t.acct, "to": [{ "email": "yael.friedman@tamarinv.example" }], "subject": "Later", "html": "<p>later</p>",
        "deferUntil": later } }),
    )
    .await
    .unwrap();
    assert_eq!(q["sendAt"], json!(later));
    crate::mail::compose::process_outbox(&t.ctx, &t.api).await.unwrap();
    assert_eq!(outbox_count(&t).await, (1, 1), "deferred message waits in the Outbox");
    let oid = q["outboxId"].as_i64().unwrap();
    t.ctx
        .db
        .write(move |c| {
            c.execute("UPDATE outbox SET send_at=0, created_at=created_at-7200000 WHERE id=?1", [oid])?;
            Ok(())
        })
        .await
        .unwrap();
    let before = crate::clock::now().timestamp();
    crate::mail::compose::process_outbox(&t.ctx, &t.api).await.unwrap();
    assert_eq!(outbox_count(&t).await.0, 0);
    let sent = sent_messages(&t);
    let later_msg = sent.iter().find(|s| s.3.contains("Subject: Later")).unwrap();
    let m = MessageParser::default().parse(later_msg.3.as_bytes()).unwrap();
    assert!(m.date().unwrap().to_timestamp() >= before - 1, "Date restamped at send time");

    // cancel → back to Drafts
    let q = call(
        &t,
        "mail.compose.send",
        json!({ "compose": {
        "accountId": t.acct, "to": [{ "email": "yael.friedman@tamarinv.example" }], "subject": "Pulled back", "html": "<p>x</p>",
        "deferUntil": later } }),
    )
    .await
    .unwrap();
    let c = call(&t, "mail.outbox.cancel", json!({ "id": q["outboxId"] })).await.unwrap();
    assert!(c["draftId"].is_string());
    assert_eq!(outbox_count(&t).await.0, 0);
    assert_eq!(t.srv.read(|st| st.user(ME).gmail.drafts.len()), 1);
}

#[tokio::test]
async fn validation_and_identity() {
    let _l = LOCK.lock().await;
    let t = setup().await;
    let e = call(&t, "mail.compose.send", json!({ "compose": { "accountId": t.acct, "subject": "x" } })).await.unwrap_err();
    assert!(e.to_string().contains("at least one name"));
    let e = call(&t, "mail.compose.send", json!({ "compose": { "accountId": t.acct, "to": [{ "email": "not an address" }] } }))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("does not recognize"));
    let e = call(
        &t,
        "mail.compose.send",
        json!({ "compose": {
        "accountId": t.acct, "from": "ceo@elsewhere.example", "to": [{ "email": "a@b.example" }] } }),
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("can't send as"));
    // new message: From choices + Gmail signature seeded after the first send-as sync
    crate::mail::compose::process_outbox(&t.ctx, &t.api).await.unwrap();
    let p = call(&t, "mail.compose.prepare", json!({ "mode": "new" })).await.unwrap();
    assert_eq!(p["compose"]["accountId"], json!(t.acct));
    assert_eq!(p["fromChoices"][0]["email"], ME);
    assert!(p["bodyHtml"].as_str().unwrap().contains("Duvz Software"), "{}", p["bodyHtml"]);
    let sigs = call(&t, "mail.signatures.list", json!({})).await.unwrap();
    assert_eq!(sigs["signatures"].as_array().unwrap().len(), 1);
    assert!(sigs["defaults"][0]["newSignatureId"].is_i64());
    // desktop alert test hook emits mail.newMail
    call(&t, "mail.desktopAlert.test", json!({ "count": 2 })).await.unwrap();
    assert!(t.ctx.events.captured().iter().any(|e| e["kind"] == "mail.newMail" && e["messages"].as_array().unwrap().len() == 2));
}

#[tokio::test]
async fn offline_send_stays_queued_and_goes_later() {
    let _l = LOCK.lock().await;
    let t = setup().await;
    // "offline": a server that hangs up on every connection
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((s, _)) = listener.accept().await {
            drop(s);
        }
    });
    let mut dead = t.api.clone();
    let mut urls = (*dead.urls).clone();
    urls.gmail = format!("http://127.0.0.1:{port}/gmail/v1");
    urls.gmail_upload = format!("http://127.0.0.1:{port}/upload/gmail/v1");
    dead.urls = Arc::new(urls);

    for subject in ["Offline one", "Offline two"] {
        call(
            &t,
            "mail.compose.send",
            json!({ "compose": {
            "accountId": t.acct, "to": [{ "email": "sarah.cohen@harborcap.example" }], "subject": subject, "html": "<p>x</p>" } }),
        )
        .await
        .unwrap();
    }
    let err = crate::mail::compose::process_outbox(&t.ctx, &dead).await.unwrap_err();
    assert!(err.to_string().contains("Waiting for a network connection"), "{err}");
    let list = call(&t, "mail.outbox.list", json!({ "accountId": t.acct })).await.unwrap();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 2);
    for item in list {
        assert_eq!(item["status"], "queued", "offline is not a failure: {item}");
        assert_eq!(item["attempts"], 0);
    }
    // back online: the next Send/Receive sends both right away (no back-off)
    crate::mail::compose::process_outbox(&t.ctx, &t.api).await.unwrap();
    assert_eq!(outbox_count(&t).await.0, 0);
    assert_eq!(sent_messages(&t).len(), 2);
}

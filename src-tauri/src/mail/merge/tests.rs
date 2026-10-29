//! Mail merge against the fake Gmail server: paced Outbox queueing, drafts, test message.

use std::sync::Arc;

use mail_parser::MessageParser;
use serde_json::{Value, json};

use crate::{accounts, ctx::AppCtx, db::tempdir::TempDirGuard, fake_google, rpc};

const ME: &str = "kenny@duvz.demo";

struct T {
    ctx: Arc<AppCtx>,
    _g: TempDirGuard,
    acct: i64,
    srv: fake_google::Shared,
    list: String,
}

async fn setup() -> T {
    let (ctx, g) = AppCtx::for_test().unwrap();
    let acct = accounts::upsert(&ctx.db, ME, "Kenny Green", "demo").await.unwrap();
    let base = ctx.fake_base().await.unwrap();
    let srv = fake_google::server_at(&base).expect("fake server");
    let a = acct.id;
    ctx.db
        .write(move |c| {
            c.execute(
                "INSERT INTO folders(account_id, gmail_label_id, name, path, role) VALUES (?1,'DRAFT','Drafts','DRAFT','drafts')",
                [a],
            )?;
            c.execute("INSERT INTO folders(account_id, name, path, role) VALUES (?1,'Outbox','Outbox','outbox')", [a])?;
            Ok(())
        })
        .await
        .unwrap();
    let list = ctx.data_dir.join("investors.csv");
    std::fs::write(
        &list,
        "\u{feff}First Name,Email Address,Company\n\
         Dan,dan@harbor.example,Harbor & Co\n\
         ,,\n\
         Ruth,ruth@meridian.example,Meridian\n\
         Dup,DAN@harbor.example,Harbor\n\
         Bad,not-an-address,X\n\
         Moshe,moshe@acme.example,\n",
    )
    .unwrap();
    T { ctx, _g: g, acct: acct.id, srv, list: list.to_string_lossy().into_owned() }
}

async fn call(t: &T, method: &str, params: Value) -> anyhow::Result<Value> {
    rpc::build().call(t.ctx.clone(), method, params).await
}

fn template(t: &T, subject: &str, html: &str) -> Value {
    json!({ "accountId": t.acct, "subject": subject, "html": html })
}

async fn outbox(t: &T) -> Vec<(String, i64, String)> {
    t.ctx
        .db
        .read(|c| {
            let mut st = c.prepare("SELECT compose_json, send_at, raw FROM outbox ORDER BY send_at")?;
            let v = st
                .query_map([], |r| {
                    let raw = r.get::<_, Option<Vec<u8>>>(2)?.unwrap_or_default();
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, String::from_utf8_lossy(&raw).into_owned()))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn load_and_check() {
    let t = setup().await;
    let s = call(&t, "mail.merge.load", json!({ "path": t.list })).await.unwrap();
    assert_eq!(s["fileName"], "investors.csv");
    assert_eq!(s["emailColumn"], 1);
    assert_eq!(s["rowCount"], 5);
    let p = call(&t, "mail.merge.check", json!({ "path": t.list, "compose": template(&t, "Hi {first name}", "<p>{Company} {Fund}</p>") }))
        .await
        .unwrap();
    assert_eq!(p["messages"], 3);
    assert_eq!(p["unknownFields"], json!(["{Fund}"]));
    assert_eq!(p["skipped"][0]["row"], 6);
    assert_eq!(p["duplicates"][0]["email"], "DAN@harbor.example");
    assert_eq!(p["blankFields"][0]["email"], "moshe@acme.example");
    assert_eq!(p["blankFields"][0]["fields"], json!(["Company"]));
}

#[tokio::test]
async fn unknown_fields_block_sending() {
    let t = setup().await;
    let e = call(&t, "mail.merge.run", json!({ "path": t.list, "mode": "send", "compose": template(&t, "{Fund}", "") })).await.unwrap_err();
    assert!(e.to_string().contains("{Fund}"), "{e}");
    assert!(outbox(&t).await.is_empty());
}

/// The raw messages Gmail received, oldest first.
fn sent(t: &T) -> Vec<String> {
    let mut v: Vec<(i64, String)> = t.srv.read(|st| {
        st.user(ME)
            .gmail
            .messages
            .values()
            .filter(|m| m.label_ids.iter().any(|l| l == "SENT"))
            .map(|m| (m.internal_date, m.raw.clone()))
            .collect()
    });
    v.sort();
    v.into_iter().map(|x| x.1).collect()
}

#[tokio::test]
async fn send_queues_paced_personalised_messages() {
    let t = setup().await;
    let before = crate::db::util::now_ms();
    let r = call(
        &t,
        "mail.merge.run",
        json!({ "path": t.list, "mode": "send", "compose": template(&t, "Meeting – {Company}", "<p>Dear <b>{First Name}</b>,</p>") }),
    )
    .await
    .unwrap();
    assert_eq!(r["created"], 3);
    let rows = outbox(&t).await;
    assert_eq!(rows.len(), 3);
    let gaps: Vec<i64> = rows.windows(2).map(|w| w[1].1 - w[0].1).collect();
    assert_eq!(gaps, vec![super::SEND_GAP_MS, super::SEND_GAP_MS]);
    assert!(rows[0].1 >= before && rows[0].1 <= crate::db::util::now_ms());
    // queued rows hold only the filled-in compose: the message is built when it is sent
    assert!(rows.iter().all(|r| r.2.is_empty()), "no built copies kept");
    let c0: Value = serde_json::from_str(&rows[0].0).unwrap();
    assert_eq!(c0["to"][0]["email"], "dan@harbor.example");
    assert_eq!(c0["subject"], "Meeting – Harbor & Co");
    let c2: Value = serde_json::from_str(&rows[2].0).unwrap();
    assert_eq!(c2["to"][0]["email"], "moshe@acme.example");
    assert_eq!(c2["subject"], "Meeting – ");
    let (batch, pace, size): (Option<String>, i64, Option<i64>) = t
        .ctx
        .db
        .read(|c| Ok(c.query_row("SELECT batch, pace_ms, size FROM outbox LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?))
        .await
        .unwrap();
    assert!(batch.is_some());
    assert_eq!(pace, super::SEND_GAP_MS);
    assert!(size.unwrap_or(0) > 100, "size estimate for the Outbox list");
    // the first one is due now and goes out with the next Outbox run, built at that moment
    let api = t.ctx.api(t.acct).await.unwrap();
    crate::mail::compose::process_outbox(&t.ctx, &api).await.unwrap();
    assert_eq!(outbox(&t).await.len(), 2);
    let s = sent(&t);
    assert_eq!(s.len(), 1);
    let m = MessageParser::default().parse(s[0].as_bytes()).unwrap();
    assert_eq!(m.subject().unwrap(), "Meeting – Harbor & Co");
    assert_eq!(m.to().unwrap().first().unwrap().address().unwrap(), "dan@harbor.example");
    assert!(m.body_html(0).unwrap().contains("Dear <b>Dan</b>,"));
}

#[tokio::test]
async fn overdue_batch_is_spread_out_again() {
    let t = setup().await;
    call(&t, "mail.merge.run", json!({ "path": t.list, "mode": "send", "compose": template(&t, "Hi {First Name}", "<p>x</p>") }))
        .await
        .unwrap();
    // the app was closed for a while: every row is overdue
    t.ctx.db.write(|c| Ok(c.execute("UPDATE outbox SET send_at = 1000", [])?)).await.unwrap();
    let api = t.ctx.api(t.acct).await.unwrap();
    let now = crate::db::util::now_ms();
    crate::mail::compose::process_outbox(&t.ctx, &api).await.unwrap();
    // only one goes now; the others are re-spaced at the merge's pace from now
    assert_eq!(sent(&t).len(), 1);
    let rows = outbox(&t).await;
    assert_eq!(rows.len(), 2);
    assert!(rows[0].1 >= now + super::SEND_GAP_MS && rows[0].1 <= crate::db::util::now_ms() + super::SEND_GAP_MS, "{}", rows[0].1 - now);
    assert_eq!(rows[1].1 - rows[0].1, super::SEND_GAP_MS);
}

#[tokio::test]
async fn deleted_attachment_fails_that_row_not_the_app() {
    let t = setup().await;
    let file = t.ctx.data_dir.join("deck.pdf");
    std::fs::write(&file, b"%PDF fake").unwrap();
    let mut c = template(&t, "Hi {First Name}", "<p>x</p>");
    c["attachments"] = json!([{ "path": file.to_string_lossy() }]);
    call(&t, "mail.merge.run", json!({ "path": t.list, "mode": "send", "compose": c })).await.unwrap();
    std::fs::remove_file(&file).unwrap();
    let api = t.ctx.api(t.acct).await.unwrap();
    let e = crate::mail::compose::process_outbox(&t.ctx, &api).await.unwrap_err();
    assert!(e.to_string().contains("could not be sent"), "{e}");
    assert_eq!(sent(&t).len(), 0);
    assert_eq!(outbox(&t).await.len(), 3, "it stays in the Outbox to retry or fix");
}

#[tokio::test]
async fn test_message_goes_to_me() {
    let t = setup().await;
    let r = call(
        &t,
        "mail.merge.run",
        json!({ "path": t.list, "mode": "test", "compose": template(&t, "Hi {First Name}", "<p>{Company}</p>") }),
    )
    .await
    .unwrap();
    assert_eq!(r["created"], 1);
    let row = r["testRow"].as_u64().unwrap();
    assert!([2, 4, 7].contains(&row), "{row}");
    let rows = outbox(&t).await;
    assert_eq!(rows.len(), 1);
    let m = MessageParser::default().parse(rows[0].2.as_bytes()).unwrap();
    assert_eq!(m.to().unwrap().first().unwrap().address().unwrap(), ME);
    assert!(m.subject().unwrap().starts_with("[TEST] Hi "), "{:?}", m.subject());
}

#[tokio::test]
async fn drafts_one_per_row() {
    let t = setup().await;
    let r = call(&t, "mail.merge.run", json!({ "path": t.list, "mode": "drafts", "compose": template(&t, "Hi {First Name}", "<p>x</p>") }))
        .await
        .unwrap();
    assert_eq!(r["created"], 3);
    let n = t.srv.read(|st| st.user(ME).gmail.drafts.len());
    assert_eq!(n, 3);
    assert!(outbox(&t).await.is_empty());
}

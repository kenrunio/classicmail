//! Archive integration tests on small Unicode PSTs written by `outlook-pst-rw` (no real data).

use std::{path::Path, sync::atomic::AtomicBool};

use pst_rw::{UnicodePstAttachment, UnicodePstFile as RwPst, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType as Rt};
use serde_json::{Value, json};

use super::{index, worker};
use crate::{ctx::AppCtx, rpc};

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5, 0x1c, 0x0c, 0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63,
    0x64, 0xf8, 0x0f, 0x00, 0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42,
    0x60, 0x82,
];

/// FILETIME of an epoch-ms date.
fn ft(ms: i64) -> i64 {
    (ms + 11_644_473_600_000) * 10_000
}

const T0: i64 = 1_300_000_000_000; // March 2011

fn recips() -> [UnicodePstRecipient<'static>; 3] {
    [
        UnicodePstRecipient { name: "Kenny Green", email: "kenny@duvz.demo", recipient_type: Rt::To },
        UnicodePstRecipient { name: "David Levi", email: "david@meridian.example", recipient_type: Rt::Cc },
        UnicodePstRecipient { name: "Archive Bcc", email: "bcc@example.com", recipient_type: Rt::Bcc },
    ]
}

/// Inbox: 3 messages (one Hebrew, one with an inline picture + a PDF); Projects/2011: 2 messages.
pub fn make_fixture(path: &Path) {
    let r = recips();
    let base = UnicodePstMessage {
        subject: "Q3 results call",
        sender_name: "Sarah Cohen",
        sender_email: "sarah@harborcap.example",
        recipients: &r,
        body: "Plain fallback text",
        html_body: Some("<p>Hello <b>Kenny</b>, see the chart:</p><img src=\"cid:chart@x\">"),
        message_id: "<q3@harborcap.example>",
        delivery_time: ft(T0),
    };
    drop(
        RwPst::create_with_attachments(
            path,
            &base,
            &[
                UnicodePstAttachment { filename: "chart.png", mime_type: "image/png", content_id: Some("<chart@x>"), data: PNG },
                UnicodePstAttachment { filename: "Q3 deck.pdf", mime_type: "application/pdf", content_id: None, data: b"%PDF-1.4 fake" },
            ],
        )
        .unwrap(),
    );
    drop(
        RwPst::append_many(
            path,
            &[
                UnicodePstMessage {
                    subject: "דוח רבעוני",
                    body: "שלום וברכה",
                    html_body: None,
                    message_id: "<heb@x>",
                    delivery_time: ft(T0 + 86_400_000),
                    ..base
                },
                UnicodePstMessage {
                    subject: "Roadshow in London",
                    sender_name: "Noa Bar",
                    sender_email: "noa@oriontech.example",
                    html_body: None,
                    body: "Flights booked.",
                    message_id: "<rs@x>",
                    delivery_time: ft(T0 - 86_400_000),
                    ..base
                },
            ],
        )
        .unwrap(),
    );
    drop(
        RwPst::append_many_in_folder(
            path,
            &["Projects", "2011"],
            &[
                UnicodePstMessage {
                    subject: "Healthcare conference",
                    html_body: None,
                    body: "Agenda attached",
                    message_id: "<p1@x>",
                    delivery_time: ft(T0 + 5 * 86_400_000),
                    ..base
                },
                UnicodePstMessage {
                    subject: "Investor day",
                    html_body: None,
                    body: "Save the date",
                    message_id: "<p2@x>",
                    delivery_time: ft(T0 + 6 * 86_400_000),
                    ..base
                },
            ],
        )
        .unwrap(),
    );
}

async fn call(ctx: &std::sync::Arc<AppCtx>, m: &str, p: Value) -> Value {
    rpc::build().call(ctx.clone(), m, p).await.unwrap_or_else(|e| panic!("{m}: {e:#}"))
}

async fn open_fixture(ctx: &std::sync::Arc<AppCtx>, path: &Path) -> i64 {
    let r = call(ctx, "archive.open", json!({ "paths": [path.display().to_string()] })).await;
    assert_eq!(r["errors"], json!([]), "{r}");
    let id = r["ids"][0].as_i64().unwrap();
    worker::wait_idle().await;
    id
}

fn folder<'a>(list: &'a Value, name: &str) -> &'a Value {
    list[0]["folders"].as_array().unwrap().iter().find(|f| f["name"] == name).unwrap_or_else(|| panic!("no folder {name} in {list}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn open_index_list_read_search() {
    let (ctx, dir) = AppCtx::for_test().unwrap();
    let path = dir.path().join("Archive 2011.pst");
    make_fixture(&path);
    let id = open_fixture(&ctx, &path).await;

    let list = call(&ctx, "archive.list", json!({})).await;
    let a = &list[0];
    assert_eq!(a["id"], id);
    assert_eq!(a["displayName"], "Archive 2011");
    assert_eq!(a["status"], "ready", "{a}");
    assert_eq!(a["format"], "unicode");
    assert_eq!(a["itemCount"], 5);
    let inbox = folder(&list, "Inbox");
    assert_eq!(inbox["total"], 3);
    let projects = folder(&list, "Projects");
    let y2011 = folder(&list, "2011");
    assert_eq!(y2011["parentId"], projects["id"]);
    assert_eq!(y2011["total"], 2);

    // the message list: newest first, mail-compatible rows
    let rows = call(
        &ctx,
        "archive.listMessages",
        json!({ "source": { "folderId": inbox["id"] }, "arrange": "date", "desc": true, "offset": 0, "limit": 50, "withGroups": true }),
    )
    .await;
    assert_eq!(rows["total"], 3);
    let subjects: Vec<&str> = rows["rows"].as_array().unwrap().iter().map(|r| r["subject"].as_str().unwrap()).collect();
    assert_eq!(subjects, ["דוח רבעוני", "Q3 results call", "Roadshow in London"]);
    assert_eq!(rows["groups"][0]["label"], "Older");
    let q3 = &rows["rows"][1];
    assert_eq!(q3["fromName"], "Sarah Cohen");
    assert_eq!(q3["date"], T0);
    assert_eq!(q3["hasAttachments"], true);
    // arrange by From
    let by_from = call(
        &ctx,
        "archive.listMessages",
        json!({ "source": { "folderId": inbox["id"] }, "arrange": "from", "desc": false, "limit": 50, "withGroups": true }),
    )
    .await;
    assert_eq!(by_from["groups"][0]["label"], "From: Noa Bar");

    // read a message: HTML with the inline picture as data:, the PDF as an attachment, recipients
    let m = call(&ctx, "archive.getMessage", json!({ "id": q3["id"] })).await;
    assert_eq!(m["subject"], "Q3 results call");
    assert_eq!(m["fromEmail"], "sarah@harborcap.example");
    assert_eq!(m["to"][0]["email"], "kenny@duvz.demo");
    assert_eq!(m["cc"][0]["name"], "David Levi");
    assert_eq!(m["bcc"][0]["email"], "bcc@example.com");
    assert_eq!(m["bodySource"], "html");
    assert_eq!(m["isPlain"], false);
    let html = m["html"].as_str().unwrap();
    assert!(html.contains("<b>Kenny</b>") && html.contains("data:image/png;base64,"), "{html}");
    let atts = m["attachments"].as_array().unwrap();
    let pdf = atts.iter().find(|a| a["filename"] == "Q3 deck.pdf").expect("pdf");
    assert_eq!(pdf["isInline"], false);
    assert!(atts.iter().any(|a| a["filename"] == "chart.png" && a["isInline"] == true));
    // plain-text message
    let heb = call(&ctx, "archive.getMessage", json!({ "id": rows["rows"][0]["id"] })).await;
    assert_eq!(heb["isPlain"], true);
    assert!(heb["html"].as_str().unwrap().contains("שלום וברכה"));

    // save an attachment (marked as from the internet)
    let out = dir.path().join("deck.pdf");
    call(&ctx, "archive.saveAttachment", json!({ "id": q3["id"], "index": pdf["index"], "path": out.display().to_string() })).await;
    assert_eq!(std::fs::read(&out).unwrap(), b"%PDF-1.4 fake");
    // opening a blocked type is refused
    let e = rpc::build().call(ctx.clone(), "archive.openAttachment", json!({ "id": q3["id"], "index": 99 })).await.unwrap_err();
    assert!(format!("{e:#}").contains("could not be found"));

    // search: this archive, all archives, column syntax
    let s = call(&ctx, "archive.listMessages", json!({ "source": { "folderId": inbox["id"] }, "search": "roadshow", "limit": 50 })).await;
    assert_eq!(s["total"], 1);
    let s = call(&ctx, "archive.search", json!({ "query": "from:sarah" })).await;
    assert_eq!(s["total"], 4, "{s}");
    assert_eq!(s["crossFolder"], true);
    assert!(s["rows"][0]["folder"].as_str().unwrap().starts_with("Archive 2011 › "));
    let s = call(&ctx, "archive.search", json!({ "query": "רבעוני" })).await;
    assert_eq!(s["total"], 1);

    // message text search after "Index message text for search"
    let s = call(&ctx, "archive.search", json!({ "query": "flights" })).await;
    assert_eq!(s["total"], 0);
    call(&ctx, "archive.setIndexText", json!({ "id": id, "on": true })).await;
    worker::wait_idle().await;
    let s = call(&ctx, "archive.search", json!({ "query": "flights" })).await;
    assert_eq!(s["total"], 1);
    let p = call(&ctx, "archive.properties", json!({ "id": id })).await;
    assert_eq!(p["textDone"], true);
    assert_eq!(p["textIndexed"], 5);

    // reply from the archive: a normal compose, prefilled
    crate::accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny Green", "demo").await.unwrap();
    let prep = call(&ctx, "archive.prepareCompose", json!({ "id": q3["id"], "mode": "replyAll" })).await;
    assert_eq!(prep["compose"]["subject"], "RE: Q3 results call");
    assert_eq!(prep["compose"]["to"][0]["email"], "sarah@harborcap.example");
    assert_eq!(prep["compose"]["cc"][0]["email"], "david@meridian.example");
    assert!(prep["quotedHtml"].as_str().unwrap().contains("-----Original Message-----"));
    assert!(prep["quotedHtml"].as_str().unwrap().contains("data:image/png;base64,"));
    let fw = call(&ctx, "archive.prepareCompose", json!({ "id": q3["id"], "mode": "forward" })).await;
    assert_eq!(fw["compose"]["subject"], "FW: Q3 results call");
    assert_eq!(fw["compose"]["attachments"].as_array().unwrap().len(), 1);

    // adjacent (message window ▲▼)
    let next = call(&ctx, "archive.adjacent", json!({ "id": rows["rows"][0]["id"], "direction": "next" })).await;
    assert_eq!(next, rows["rows"][1]["id"]);

    // re-opening the same file doesn't add it twice
    let again = call(&ctx, "archive.open", json!({ "paths": [path.display().to_string()] })).await;
    assert_eq!(again["ids"][0], id);
    assert_eq!(call(&ctx, "archive.list", json!({})).await.as_array().unwrap().len(), 1);

    // close: gone with its index
    call(&ctx, "archive.close", json!({ "id": id })).await;
    assert_eq!(call(&ctx, "archive.list", json!({})).await, json!([]));
    let n: i64 = ctx.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM archive_messages", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(n, 0);
    let s = call(&ctx, "archive.search", json!({ "query": "roadshow" })).await;
    assert_eq!(s["total"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn indexing_resumes_and_notices_changes() {
    let (ctx, dir) = AppCtx::for_test().unwrap();
    let path = dir.path().join("Archive Resume.pst");
    make_fixture(&path);
    let id = open_fixture(&ctx, &path).await;
    let count = |ctx: std::sync::Arc<AppCtx>| async move {
        ctx.db
            .read(move |c| Ok(c.query_row("SELECT COUNT(*) FROM archive_messages WHERE archive_id=?1", [id], |r| r.get::<_, i64>(0))?))
            .await
            .unwrap()
    };
    assert_eq!(count(ctx.clone()).await, 5);

    // a job stopped after the first row of the Inbox: the next run continues from there
    ctx.db
        .write(move |c| {
            let inbox: i64 = c.query_row("SELECT id FROM archive_folders WHERE archive_id=?1 AND name='Inbox'", [id], |r| r.get(0))?;
            c.execute("DELETE FROM archive_fts WHERE rowid IN (SELECT id FROM archive_messages WHERE folder_id=?1)", [inbox])?;
            c.execute("DELETE FROM archive_messages WHERE folder_id=?1", [inbox])?;
            c.execute("UPDATE archive_folders SET done=0, indexed=1 WHERE id=?1", [inbox])?;
            c.execute("UPDATE archives SET status='indexing' WHERE id=?1", [id])?;
            Ok(())
        })
        .await
        .unwrap();
    let c2 = ctx.clone();
    tokio::task::spawn_blocking(move || index::index_headers(&c2, id, &AtomicBool::new(false))).await.unwrap().unwrap();
    // rows before the resume point are not read again
    assert_eq!(count(ctx.clone()).await, 4);

    // a cancelled job stops without finishing
    ctx.db.write(move |c| Ok(c.execute("UPDATE archive_folders SET done=0, indexed=0 WHERE archive_id=?1", [id])?)).await.unwrap();
    let c2 = ctx.clone();
    tokio::task::spawn_blocking(move || index::index_headers(&c2, id, &AtomicBool::new(true))).await.unwrap().unwrap();
    let status: String =
        ctx.db.read(move |c| Ok(c.query_row("SELECT status FROM archives WHERE id=?1", [id], |r| r.get(0))?)).await.unwrap();
    assert_eq!(status, "indexing");

    // unchanged file + ready: nothing to do on restart; changed file: rebuilt
    let c2 = ctx.clone();
    tokio::task::spawn_blocking(move || index::index_headers(&c2, id, &AtomicBool::new(false))).await.unwrap().unwrap();
    assert_eq!(count(ctx.clone()).await, 5);
    std::thread::sleep(std::time::Duration::from_millis(20));
    let r = recips();
    drop(
        RwPst::append(
            &path,
            &UnicodePstMessage {
                subject: "Late arrival",
                sender_name: "X",
                sender_email: "x@example.com",
                recipients: &r,
                body: "b",
                html_body: None,
                message_id: "<late@x>",
                delivery_time: ft(T0 + 10 * 86_400_000),
            },
        )
        .unwrap(),
    );
    super::api::resume(&ctx).await.unwrap();
    worker::wait_idle().await;
    assert_eq!(count(ctx.clone()).await, 6);
    let s = call(&ctx, "archive.search", json!({ "query": "late" })).await;
    assert_eq!(s["total"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_and_locked_files() {
    let (ctx, dir) = AppCtx::for_test().unwrap();
    let junk = dir.path().join("notes.pst");
    std::fs::write(&junk, b"hello, not a pst at all").unwrap();
    let r =
        call(&ctx, "archive.open", json!({ "paths": [junk.display().to_string(), dir.path().join("missing.pst").display().to_string()] }))
            .await;
    assert_eq!(r["ids"], json!([]));
    assert!(r["errors"][0]["message"].as_str().unwrap().contains("not an Outlook data file"), "{r}");
    assert!(r["errors"][1]["message"].as_str().unwrap().contains("could not be found"), "{r}");

    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let path = dir.path().join("Locked.pst");
        make_fixture(&path);
        // what Outlook does with a PST it has open: no sharing
        let lock = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&path).unwrap();
        let r = call(&ctx, "archive.open", json!({ "paths": [path.display().to_string()] })).await;
        assert_eq!(r["errors"][0]["message"], "This archive is open in Outlook. Close Outlook (or remove it there) and try again.");
        drop(lock);
        // our own reading never locks the file: a writer can still open it while we read
        let id = open_fixture(&ctx, &path).await;
        let rows = call(&ctx, "archive.search", json!({ "query": "healthcare", "archiveId": id })).await;
        let mid = rows["rows"][0]["id"].clone();
        call(&ctx, "archive.getMessage", json!({ "id": mid })).await;
        std::fs::OpenOptions::new().read(true).write(true).open(&path).expect("file is not locked by us");
    }
}

/// Timing on a real archive (read-only): `CLASSICMAIL_PST=<path> cargo test --lib archive::tests::real_archive -- --ignored --nocapture`.
/// Prints counts and timings only.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn real_archive() {
    let Ok(p) = std::env::var("CLASSICMAIL_PST") else { return };
    let sample: usize = std::env::var("CLASSICMAIL_PST_SAMPLE").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    let (ctx, _dir) = AppCtx::for_test().unwrap();
    let t = std::time::Instant::now();
    let id = open_fixture(&ctx, Path::new(&p)).await;
    let secs = t.elapsed().as_secs_f64();
    let list = call(&ctx, "archive.list", json!({})).await;
    let a = &list[0];
    let n = a["itemCount"].as_i64().unwrap_or(0);
    println!(
        "status={} format={} items={} folders={} index={:.2}s => {:.0} items/s",
        a["status"],
        a["format"],
        n,
        a["folders"].as_array().map(|f| f.len()).unwrap_or(0),
        secs,
        n as f64 / secs
    );
    // list the biggest folder
    let big = a["folders"].as_array().unwrap().iter().max_by_key(|f| f["total"].as_i64().unwrap_or(0)).unwrap().clone();
    let t = std::time::Instant::now();
    let rows = call(
        &ctx,
        "archive.listMessages",
        json!({ "source": { "folderId": big["id"] }, "arrange": "date", "desc": true, "offset": 0, "limit": 0, "withGroups": true }),
    )
    .await;
    let page = call(
        &ctx,
        "archive.listMessages",
        json!({ "source": { "folderId": big["id"] }, "arrange": "date", "desc": true, "offset": 0, "limit": 200 }),
    )
    .await;
    println!(
        "largest folder: {} items; meta+first page {:?} ({} rows)",
        rows["total"],
        t.elapsed(),
        page["rows"].as_array().unwrap().len()
    );
    let t = std::time::Instant::now();
    let s = call(&ctx, "archive.search", json!({ "query": "the" })).await;
    println!("search 'the': {} hits in {:?}", s["total"], t.elapsed());
    // read a sample of messages through the full getMessage path
    let ids: Vec<i64> = ctx
        .db
        .read(move |c| {
            let mut st = c.prepare("SELECT id FROM archive_messages WHERE archive_id=?1 ORDER BY id")?;
            let v = st.query_map([id], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?;
            Ok(v)
        })
        .await
        .unwrap();
    let step = (ids.len() / sample.max(1)).max(1);
    let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
    let mut errs = 0;
    let t = std::time::Instant::now();
    let mut read = 0;
    for mid in ids.iter().step_by(step).take(sample) {
        match rpc::build().call(ctx.clone(), "archive.getMessage", json!({ "id": mid })).await {
            Ok(m) => {
                let src = m["bodySource"].as_str().unwrap_or("").to_string();
                *kinds.entry(src.clone()).or_default() += 1;
                // encoding sanity (counts only): Hebrew text vs. runs of Latin-1 letters that look like mis-decoded Hebrew
                let body = format!("{} {}", m["subject"].as_str().unwrap_or(""), m["html"].as_str().unwrap_or(""));
                let heb = body.chars().any(|c| ('\u{05d0}'..='\u{05ea}').contains(&c));
                let mut run = 0;
                let mut moji = false;
                for c in body.chars() {
                    if ('\u{e0}'..='\u{fa}').contains(&c) {
                        run += 1;
                        moji |= run >= 4;
                    } else {
                        run = 0;
                    }
                }
                if heb {
                    *kinds.entry(format!("{src}+hebrew")).or_default() += 1;
                }
                if moji {
                    *kinds.entry(format!("{src}+latin-run")).or_default() += 1;
                    if std::env::var("CLASSICMAIL_PST_DIAG").is_ok() {
                        let subj = m["subject"].as_str().unwrap_or("");
                        let in_subj = subj.chars().filter(|c| ('\u{e0}'..='\u{fa}').contains(c)).count();
                        let codes: Vec<String> = body
                            .chars()
                            .filter(|c| ('\u{e0}'..='\u{fa}').contains(c))
                            .take(8)
                            .map(|c| format!("{:02x}", c as u32))
                            .collect();
                        let mid2 = *mid;
                        let nid: i64 = ctx
                            .db
                            .read(move |c| Ok(c.query_row("SELECT node_id FROM archive_messages WHERE id=?1", [mid2], |r| r.get(0))?))
                            .await
                            .unwrap();
                        let cps = worker::with_pst(std::path::PathBuf::from(&p), move |pst| {
                            let pr = pst.message_props(nid as u32, &[0x3FFD, 0x3FDE, 0x3FF1, 0x0037, 0x1013])?;
                            let html_meta = pr.bin(0x1013).map(|b| {
                                let h = String::from_utf8_lossy(&b[..b.len().min(3000)]).to_ascii_lowercase();
                                h.find("charset=").map(|i| h[i..].chars().take(22).collect::<String>()).unwrap_or_default()
                            });
                            let subj_type = match pr.0.get(&0x0037) {
                                Some(super::props::PVal::Str8(_)) => "str8",
                                Some(super::props::PVal::Str(_)) => "unicode",
                                _ => "-",
                            };
                            Ok(format!(
                                "cp3FFD={:?} cpid={:?} locale={:?} subj={subj_type} meta={html_meta:?}",
                                pr.int(0x3FFD),
                                pr.int(0x3FDE),
                                pr.int(0x3FF1)
                            ))
                        })
                        .await
                        .unwrap_or_default();
                        let letters: Vec<char> = body.chars().filter(|c| *c as u32 >= 0x80 && c.is_alphabetic()).collect();
                        let inr = letters.iter().filter(|c| ('\u{e0}'..='\u{fa}').contains(*c)).count();
                        let others: std::collections::BTreeSet<String> = letters
                            .iter()
                            .filter(|c| !('\u{e0}'..='\u{fa}').contains(*c))
                            .take(12)
                            .map(|c| format!("{:x}", *c as u32))
                            .collect();
                        let unmappable = encoding_rs::WINDOWS_1252.encode(&body).2;
                        println!(
                            "  latin-run: src={src} in_subject={in_subj} codes={codes:?} {cps} letters={} in_range={inr} others={others:?} unmappable={unmappable}",
                            letters.len()
                        );
                    }
                }
                if body.contains('\u{fffd}') {
                    *kinds.entry(format!("{src}+replacement-char")).or_default() += 1;
                }
            }
            Err(e) => {
                errs += 1;
                if errs < 5 {
                    println!("  read error: {}", format!("{e:#}").chars().take(120).collect::<String>());
                }
            }
        }
        read += 1;
    }
    println!(
        "getMessage x{read}: {:?} avg {:.1} ms; body sources {kinds:?}; errors {errs}",
        t.elapsed(),
        t.elapsed().as_secs_f64() * 1000.0 / read.max(1) as f64
    );
}

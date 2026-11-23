//! Large attachments sent as Google Drive links, against the fake Drive (fake_google/drive.rs).

use std::{sync::Arc, time::Duration};

use serde_json::{Value, json};

use super::{
    drive::{self, Linked},
    drive_folder,
    model::{Addr, AttachmentSpec, Compose, DriveLink},
    outbox,
};
use crate::{ctx::AppCtx, db::tempdir::TempDirGuard, google::Api};

async fn setup(email: &str) -> (Arc<AppCtx>, TempDirGuard, Api, String) {
    let (ctx, g) = AppCtx::for_test().unwrap();
    let a = crate::accounts::upsert(&ctx.db, email, "Drive", "demo").await.unwrap();
    let api = ctx.api(a.id).await.unwrap();
    let base = ctx.fake_base().await.unwrap();
    (ctx, g, api, base)
}

/// A file of `mb` MiB (+ a few bytes, so the last part is short).
fn big_file(g: &TempDirGuard, name: &str, mb: usize) -> String {
    let p = g.path().join(name);
    std::fs::write(&p, vec![7u8; mb * 1024 * 1024 + 123]).unwrap();
    p.to_string_lossy().into_owned()
}

async fn wait_done(ctx: &AppCtx, id: i64) -> drive::Upload {
    for _ in 0..300 {
        let u = drive::load(ctx, id).await.unwrap().unwrap();
        if u.status != "uploading" {
            return u;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let u = drive::load(ctx, id).await.unwrap().unwrap();
    panic!("upload {id} didn't finish: {:?}", (u.status, u.sent, u.size, u.error, u.session_url));
}

fn drive_files(base: &str, email: &str) -> Vec<crate::fake_google::drive::FFile> {
    crate::fake_google::server_at(base).unwrap().read(|st| st.user(email).drive.files.clone())
}

async fn upload(ctx: &Arc<AppCtx>, account_id: i64, path: &str) -> drive::Upload {
    let reg = crate::rpc::build();
    let v = reg.call(ctx.clone(), "mail.drive.upload", json!({ "accountId": account_id, "path": path })).await.unwrap();
    wait_done(ctx, v["id"].as_i64().unwrap()).await
}

/// In parts into a ClassicMail folder it creates once (marked, so it is found again); the file
/// gets its link; progress goes out as events.
#[tokio::test]
async fn uploads_in_parts_into_one_folder() {
    let (ctx, g, api, base) = setup("up@duvz.demo").await;
    let u = upload(&ctx, api.account_id, &big_file(&g, "Board deck.pptx", 17)).await;
    assert_eq!(u.status, "done", "{:?}", u.error);
    assert_eq!(u.sent, u.size);
    assert!(u.link.as_deref().is_some_and(|l| l.contains(u.file_id.as_deref().unwrap())));
    let u2 = upload(&ctx, api.account_id, &big_file(&g, "Model.xlsx", 1)).await;
    assert_eq!(u2.status, "done");
    let files = drive_files(&base, "up@duvz.demo");
    let folders: Vec<_> = files.iter().filter(|f| f.mime_type == crate::google::drive::FOLDER_MIME).collect();
    assert_eq!(folders.len(), 1, "one folder for both");
    assert_eq!(folders[0].name, "ClassicMail");
    assert_eq!(folders[0].app_properties.get("classicmail").map(String::as_str), Some("uploads"));
    let deck = files.iter().find(|f| f.name == "Board deck.pptx").unwrap();
    assert_eq!(deck.parents, vec![folders[0].id.clone()]);
    assert_eq!(deck.size, Some(17 * 1024 * 1024 + 123));
    // three parts: two progress events before the end, then done
    let events = ctx.events.captured();
    let progress: Vec<&Value> = events.iter().filter(|v| v["kind"] == "drive.upload" && v["id"] == json!(u.id)).collect();
    assert!(progress.len() >= 3, "{progress:?}");
    assert_eq!(progress.last().unwrap()["status"], "done");
}

/// ClassicMail installed afresh (its database gone): the same folder is found by its mark, not a
/// second one made; a folder in the trash isn't used.
#[tokio::test]
async fn the_folder_is_found_again_after_a_reinstall() {
    let (ctx, _g, api, base) = setup("again@duvz.demo").await;
    let first = drive_folder::ensure(&ctx, &api).await.unwrap();
    ctx.db.write(|c| Ok(c.execute("DELETE FROM drive_folders", [])?)).await.unwrap();
    assert_eq!(drive_folder::ensure(&ctx, &api).await.unwrap(), first, "found by its mark");
    // renamed in Options: the existing folder is renamed, still the same one
    let reg = crate::rpc::build();
    reg.call(ctx.clone(), "mail.drive.renameFolder", json!({ "name": "Large files" })).await.unwrap();
    let f = drive_files(&base, "again@duvz.demo").into_iter().find(|f| f.id == first).unwrap();
    assert_eq!(f.name, "Large files");
    // put in the trash by the user: a new one
    api.drive_trash(&first).await.unwrap();
    let second = drive_folder::ensure(&ctx, &api).await.unwrap();
    assert_ne!(second, first);
    assert_eq!(drive_files(&base, "again@duvz.demo").into_iter().find(|f| f.id == second).unwrap().name, "Large files");
}

fn compose_with(account_id: i64, spec: AttachmentSpec) -> Compose {
    Compose {
        account_id,
        to: vec![Addr::new("Sam", "sam@harbor.example")],
        cc: vec![Addr::new("", "pat@nogoogle.example")],
        subject: "The deck".into(),
        html: "<p>Here it is.</p>".into(),
        attachments: vec![spec],
        ..Default::default()
    }
}

/// Send while the file is still uploading: the Outbox holds the message, then, once the upload is
/// done, shares the file with the recipients and sends it with the link.
#[tokio::test]
async fn sending_waits_for_the_upload_then_shares() {
    let (ctx, g, api, base) = setup("send@duvz.demo").await;
    let path = big_file(&g, "Deck.pdf", 2);
    // an upload that hasn't started yet (as after a restart, before resume)
    let acct = api.account_id;
    let p2 = path.clone();
    let id = ctx
        .db
        .write(move |c| {
            c.execute(
                "INSERT INTO drive_uploads(account_id, path, name, mime, size, created_at) VALUES (?1,?2,'Deck.pdf','application/pdf',?3,0)",
                rusqlite::params![acct, p2, (2 * 1024 * 1024 + 123) as i64],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await
        .unwrap();
    let spec = AttachmentSpec {
        path: Some(path),
        name: Some("Deck.pdf".into()),
        size: Some(2 * 1024 * 1024 + 123),
        drive: Some(DriveLink { upload_id: Some(id), ..Default::default() }),
        ..Default::default()
    };
    let q = outbox::queue_many(&ctx, vec![(compose_with(acct, spec), 0)]).await.unwrap();
    let raw_null: bool =
        ctx.db.read(move |c| Ok(c.query_row("SELECT raw IS NULL FROM outbox WHERE id=?1", [q[0].outbox_id], |r| r.get(0))?)).await.unwrap();
    assert!(raw_null, "built again when it goes");
    outbox::process(&ctx, &api).await.unwrap();
    let (n, why): (i64, Option<String>) = ctx
        .db
        .read(|c| Ok(c.query_row("SELECT COUNT(*), max(last_error) FROM outbox", [], |r| Ok((r.get(0)?, r.get(1)?)))?))
        .await
        .unwrap();
    assert_eq!(n, 1, "still waiting");
    assert!(why.unwrap().contains("Waiting for the upload of \"Deck.pdf\""));

    drive::resume_all(ctx.clone());
    let u = wait_done(&ctx, id).await;
    assert_eq!(u.status, "done");
    outbox::process(&ctx, &api).await.unwrap();
    let n: i64 = ctx.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(n, 0, "sent");
    let file = drive_files(&base, "send@duvz.demo").into_iter().find(|f| f.name == "Deck.pdf").unwrap();
    assert!(file.permissions.contains(&("user".into(), "sam@harbor.example".into())), "{:?}", file.permissions);
    // pat has no Google account: anyone with the link (the default)
    assert!(file.permissions.contains(&("anyone".into(), String::new())), "{:?}", file.permissions);
    // the message as sent carries the card with the link
    let sent = crate::fake_google::server_at(&base).unwrap().read(|st| {
        let gm = serde_json::to_string(&st.user("send@duvz.demo").gmail).unwrap();
        gm.contains(&file.id)
    });
    assert!(sent, "the sent message links to the file");
}

/// With "keep it private", an address with no Google account doesn't open the file to anyone.
#[tokio::test]
async fn private_files_stay_private() {
    let (ctx, g, api, base) = setup("priv@duvz.demo").await;
    ctx.db.write(|c| crate::db::util::set_setting(c, drive::NO_ACCOUNT_SETTING, &"private")).await.unwrap();
    let u = upload(&ctx, api.account_id, &big_file(&g, "Q3.zip", 1)).await;
    let spec = AttachmentSpec {
        name: Some("Q3.zip".into()),
        drive: Some(DriveLink { upload_id: Some(u.id), ..Default::default() }),
        ..Default::default()
    };
    let c = compose_with(api.account_id, spec);
    let links = drive::linked(&ctx, &c.attachments).await.unwrap();
    drive::share(&ctx, &c, &links).await.unwrap();
    let file = drive_files(&base, "priv@duvz.demo").into_iter().find(|f| f.name == "Q3.zip").unwrap();
    assert_eq!(file.permissions, vec![("user".to_string(), "sam@harbor.example".to_string())]);
}

/// The cards go above the quoted original; the text version has the links written out; a draft
/// opened again gets them back as Drive attachments and loses them from the text.
#[test]
fn cards_round_trip() {
    let links = vec![
        Linked {
            name: "Deck \"final\".pptx".into(),
            size: 734_003_200,
            link: Some("https://drive.google.com/file/d/abc/view".into()),
            upload_id: Some(4),
            file_id: Some("abc".into()),
            account_id: Some(1),
            status: "done".into(),
            error: None,
        },
        Linked {
            name: "Raw.zip".into(),
            size: 30 * 1024 * 1024,
            link: None,
            upload_id: Some(5),
            file_id: None,
            account_id: Some(1),
            status: "uploading".into(),
            error: None,
        },
    ];
    let c = Compose { html: format!("<p>See below.</p>{}><p>original</p></div>", super::prepare::ORIGINAL_MARKER), ..Default::default() };
    let out = drive::with_cards(&c, &links);
    let card = out.html.find("data-cm-drives").unwrap();
    assert!(card < out.html.find("cm-original").unwrap(), "above the original");
    assert!(out.html.contains("href=\"https://drive.google.com/file/d/abc/view\""));
    assert!(out.html.contains("700 MB"));
    let text = out.text.clone().unwrap();
    assert!(text.contains("Deck \"final\".pptx (700 MB, Google Drive): https://drive.google.com/file/d/abc/view"), "{text}");
    assert!(text.find("Google Drive").unwrap() < text.find("original").unwrap());

    let (html, specs) = drive::take_cards(&out.html);
    assert_eq!(html, c.html);
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].name.as_deref(), Some("Deck \"final\".pptx"));
    assert_eq!(specs[0].size, Some(734_003_200));
    let d = specs[0].drive.clone().unwrap();
    assert_eq!(
        (d.file_id.as_deref(), d.upload_id, d.link.as_deref()),
        (Some("abc"), None, Some("https://drive.google.com/file/d/abc/view"))
    );
    assert_eq!(specs[1].drive.clone().unwrap().upload_id, Some(5));

    // plain text: a line per file
    let plain = Compose { plain_text: true, text: Some("Hi".into()), ..Default::default() };
    assert!(drive::with_cards(&plain, &links[..1]).text.unwrap().ends_with("https://drive.google.com/file/d/abc/view\r\n"));
}

/// A failed upload stops the message going (the user retries or removes it).
#[tokio::test]
async fn no_permission_for_drive_asks_to_sign_in_again() {
    let (ctx, g, api, base) = setup("scope@duvz.demo").await;
    let r = reqwest::Client::new()
        .post(format!("{base}/x-fake/faults"))
        .json(&json!([{ "status": 403, "reason": "insufficientPermissions", "message": "Request had insufficient authentication scopes.", "pathPart": "/drive/v3/files" }]))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let u = upload(&ctx, api.account_id, &big_file(&g, "a.bin", 1)).await;
    assert_eq!(u.status, "failed");
    assert!(u.error.as_deref().unwrap().contains("Sign in to it again"), "{:?}", u.error);
    let spec = AttachmentSpec {
        name: Some("a.bin".into()),
        drive: Some(DriveLink { upload_id: Some(u.id), ..Default::default() }),
        ..Default::default()
    };
    let e = outbox::queue_many(&ctx, vec![(compose_with(api.account_id, spec), 0)]).await.unwrap_err();
    assert!(e.to_string().contains("Try the upload again"), "{e}");
    // retried (the permission given meanwhile): it goes through
    let reg = crate::rpc::build();
    reg.call(ctx.clone(), "mail.drive.retry", json!({ "id": u.id })).await.unwrap();
    assert_eq!(wait_done(&ctx, u.id).await.status, "done");
}

/// Options › Large Files: the folder's size; Clean Up moves only old files to the trash.
#[tokio::test]
async fn status_and_clean_up() {
    let (ctx, g, api, base) = setup("clean@duvz.demo").await;
    let reg = crate::rpc::build();
    let s = reg.call(ctx.clone(), "mail.drive.status", json!({})).await.unwrap();
    assert_eq!(s["accounts"][0]["state"], "none", "no folder yet: {s}");
    upload(&ctx, api.account_id, &big_file(&g, "old.bin", 1)).await;
    upload(&ctx, api.account_id, &big_file(&g, "new.bin", 2)).await;
    let s = reg.call(ctx.clone(), "mail.drive.status", json!({})).await.unwrap();
    let a = &s["accounts"][0];
    assert_eq!((a["state"].as_str(), a["files"].as_i64()), (Some("ok"), Some(2)), "{s}");
    assert_eq!(a["bytes"].as_i64(), Some((3 * 1024 * 1024 + 246) as i64));
    assert!(a["limit"].as_i64().is_some());
    // one of them 40 days old
    crate::fake_google::server_at(&base).unwrap().with(|st| {
        let f = st.user("clean@duvz.demo").drive.files.iter_mut().find(|f| f.name == "old.bin").unwrap();
        f.created -= 40 * 86_400_000;
    });
    let r = reg.call(ctx.clone(), "mail.drive.cleanNow", json!({ "days": 30 })).await.unwrap();
    assert_eq!(r["trashed"], 1, "{r}");
    let files = drive_files(&base, "clean@duvz.demo");
    assert!(files.iter().find(|f| f.name == "old.bin").unwrap().trashed);
    assert!(!files.iter().find(|f| f.name == "new.bin").unwrap().trashed);
}

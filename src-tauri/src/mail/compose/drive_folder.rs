//! Each account's ClassicMail folder in its Google Drive (where large attachments go, drive.rs),
//! and Tools › Options › Large Files: the folder's name, size and clean-up.
//!
//! The folder is created on the first upload. It carries a mark (`appProperties`
//! `classicmail=uploads`), so after ClassicMail is installed afresh, or the database lost, the
//! same folder is found and used again instead of a new one. Its id is kept in `drive_folders`;
//! the user may move or rename it in Drive and it is still found. With the `drive.file`
//! permission ClassicMail sees only that folder and the files it put there — nothing else.
//!
//! Settings: `drive.folderName` (default "ClassicMail"), `drive.cleanup` (move files older than
//! `drive.cleanupDays` days to Drive's trash, once a day), `drive.noGoogleAccount` (drive.rs).
//!
//! rpc: `mail.drive.status` → { folderName, accounts: [{ accountId, email, state, message?,
//! folderId?, folderLink?, files, bytes, usage?, limit? }] }; `mail.drive.renameFolder { name }`;
//! `mail.drive.cleanNow` → { trashed, failed: [message] }.

use std::{sync::Arc, time::Duration};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{
    ctx::AppCtx,
    db::util::{get_setting, set_setting},
    google::{
        Api,
        drive::{self, DriveFile},
    },
};

pub const NAME_SETTING: &str = "drive.folderName";
pub const DEFAULT_NAME: &str = "ClassicMail";
pub const CLEANUP_SETTING: &str = "drive.cleanup";
pub const DAYS_SETTING: &str = "drive.cleanupDays";
const LAST_CLEANUP: &str = "drive.lastCleanup";

/// One folder lookup / creation at a time (two uploads starting together make one folder).
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn folder_name(ctx: &AppCtx) -> anyhow::Result<String> {
    let n: Option<String> = ctx.db.read(|c| get_setting(c, NAME_SETTING)).await?;
    Ok(n.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| DEFAULT_NAME.into()))
}

async fn cached(ctx: &AppCtx, account_id: i64) -> anyhow::Result<Option<String>> {
    ctx.db
        .read(move |c| Ok(c.query_row("SELECT folder_id FROM drive_folders WHERE account_id=?1", [account_id], |r| r.get(0)).optional()?))
        .await
}

async fn remember(ctx: &AppCtx, account_id: i64, folder_id: Option<String>) -> anyhow::Result<()> {
    ctx.db
        .write(move |c| {
            match folder_id {
                Some(f) => c.execute(
                    "INSERT INTO drive_folders(account_id, folder_id) VALUES (?1,?2) \
                     ON CONFLICT(account_id) DO UPDATE SET folder_id=excluded.folder_id",
                    params![account_id, f],
                )?,
                None => c.execute("DELETE FROM drive_folders WHERE account_id=?1", [account_id])?,
            };
            Ok(())
        })
        .await
}

/// The account's folder if it exists (kept id, else found by its mark); never creates one.
pub async fn existing(ctx: &AppCtx, api: &Api) -> anyhow::Result<Option<DriveFile>> {
    if let Some(id) = cached(ctx, api.account_id).await? {
        match api.drive_get(&id).await {
            Ok(f) if !f.trashed => return Ok(Some(f)),
            Ok(_) => {}
            Err(e) if crate::google::client::api_status(&e) == Some(404) => {}
            Err(e) => return Err(e),
        }
    }
    let found = api.drive_find_folder().await?;
    remember(ctx, api.account_id, found.as_ref().map(|f| f.id.clone())).await?;
    Ok(found)
}

/// The account's folder id, found or created, named as the setting says.
pub async fn ensure(ctx: &AppCtx, api: &Api) -> anyhow::Result<String> {
    let _g = LOCK.lock().await;
    let name = folder_name(ctx).await?;
    if let Some(f) = existing(ctx, api).await? {
        if f.name != name
            && let Err(e) = api.drive_rename(&f.id, &name).await
        {
            log::warn!("drive folder rename: {}", crate::logging::err(&e));
        }
        return Ok(f.id);
    }
    let f = api.drive_create_folder(&name).await?;
    log::info!("drive: created the uploads folder for account {}", api.account_id);
    remember(ctx, api.account_id, Some(f.id.clone())).await?;
    Ok(f.id)
}

/// The accounts that have a Google Drive.
async fn drive_accounts(ctx: &AppCtx) -> anyhow::Result<Vec<crate::accounts::Account>> {
    Ok(crate::accounts::list(&ctx.db).await?.into_iter().filter(|a| matches!(a.provider.as_str(), "google" | "demo")).collect())
}

// ---- Options › Large Files ----------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    account_id: i64,
    email: String,
    /// "ok" | "none" (no folder yet) | "signIn" | "error"
    state: &'static str,
    message: Option<String>,
    folder_id: Option<String>,
    folder_name: Option<String>,
    folder_link: Option<String>,
    files: usize,
    bytes: i64,
    /// the whole Drive's storage (bytes); limit is None when unlimited / not known
    usage: Option<i64>,
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    folder_name: String,
    accounts: Vec<AccountStatus>,
}

#[derive(Deserialize)]
pub struct NoParams {}

pub async fn status(ctx: Arc<AppCtx>, _p: NoParams) -> anyhow::Result<Status> {
    let folder_name = folder_name(&ctx).await?;
    let mut accounts = Vec::new();
    for a in drive_accounts(&ctx).await? {
        let mut s = AccountStatus {
            account_id: a.id,
            email: a.email.clone(),
            state: "ok",
            message: None,
            folder_id: None,
            folder_name: None,
            folder_link: None,
            files: 0,
            bytes: 0,
            usage: None,
            limit: None,
        };
        let r: anyhow::Result<()> = async {
            let api = ctx.api(a.id).await?;
            match existing(&ctx, &api).await? {
                Some(f) => {
                    let files = api.drive_files_in(&f.id, None).await?;
                    s.files = files.len();
                    s.bytes = files.iter().map(DriveFile::bytes).sum();
                    s.folder_link = Some(format!("https://drive.google.com/drive/folders/{}", f.id));
                    s.folder_name = Some(f.name);
                    s.folder_id = Some(f.id);
                }
                None => s.state = "none",
            }
            // the whole Drive (not essential: the permission may not reach it)
            if let Ok(q) = api.drive_storage().await {
                s.usage = q.usage.and_then(|v| v.parse().ok());
                s.limit = q.limit.and_then(|v| v.parse().ok());
            }
            Ok(())
        }
        .await;
        if let Err(e) = r {
            s.state = if drive::needs_sign_in(&e) { "signIn" } else { "error" };
            s.message = Some(super::drive::explain(&e, &a.email));
        }
        accounts.push(s);
    }
    Ok(Status { folder_name, accounts })
}

#[derive(Deserialize)]
pub struct RenameParams {
    name: String,
}

/// A new folder name: saved, and every account's existing folder renamed (best effort; one that
/// can't be reached now is renamed on its next upload).
pub async fn rename(ctx: Arc<AppCtx>, p: RenameParams) -> anyhow::Result<()> {
    let name = p.name.trim().to_string();
    if name.is_empty() {
        return Err(crate::user_err!("bad_params", "Type a name for the folder."));
    }
    if name.chars().count() > 100 {
        return Err(crate::user_err!("bad_params", "The folder name is too long (at most 100 characters)."));
    }
    let n2 = name.clone();
    ctx.db.write(move |c| set_setting(c, NAME_SETTING, &n2)).await?;
    ctx.events.emit("settings.changed", serde_json::json!({ "key": NAME_SETTING }));
    for a in drive_accounts(&ctx).await? {
        let Some(id) = cached(&ctx, a.id).await? else { continue };
        let r = async { ctx.api(a.id).await?.drive_rename(&id, &name).await }.await;
        if let Err(e) = r {
            log::warn!("drive folder rename for account {}: {}", a.id, crate::logging::err(&e));
        }
    }
    Ok(())
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleaned {
    trashed: usize,
    failed: Vec<String>,
}

#[derive(Deserialize)]
pub struct CleanParams {
    /// default: the setting (30)
    days: Option<i64>,
}

pub async fn clean_now(ctx: Arc<AppCtx>, p: CleanParams) -> anyhow::Result<Cleaned> {
    let days = match p.days {
        Some(d) => d,
        None => ctx.db.read(|c| get_setting::<i64>(c, DAYS_SETTING)).await?.unwrap_or(30),
    };
    clean_up(&ctx, days.max(1)).await
}

/// Move the files ClassicMail uploaded more than `days` days ago to Drive's trash, in every
/// account. Only its own folder's files are looked at (and the permission shows it no others).
pub async fn clean_up(ctx: &Arc<AppCtx>, days: i64) -> anyhow::Result<Cleaned> {
    let cutoff = crate::clock::now_ms() - days * 86_400_000;
    let before = chrono::DateTime::from_timestamp_millis(cutoff).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut out = Cleaned::default();
    for a in drive_accounts(ctx).await? {
        let r: anyhow::Result<usize> = async {
            let api = ctx.api(a.id).await?;
            let Some(folder) = existing(ctx, &api).await? else { return Ok(0) };
            let old = api.drive_files_in(&folder.id, Some(&before)).await?;
            let mut n = 0;
            for f in old {
                api.drive_trash(&f.id).await?;
                n += 1;
            }
            Ok(n)
        }
        .await;
        match r {
            Ok(n) => out.trashed += n,
            Err(e) => out.failed.push(format!("{}: {}", a.email, super::drive::explain(&e, &a.email))),
        }
    }
    if out.trashed > 0 {
        log::info!("drive clean-up: {} file(s) older than {days} days moved to the trash", out.trashed);
    }
    Ok(out)
}

/// With the clean-up on: once a day, a few minutes after start-up and then every few hours.
pub fn start_daily(ctx: Arc<AppCtx>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5 * 60)).await;
        loop {
            if let Err(e) = daily(&ctx).await {
                log::warn!("drive clean-up: {}", crate::logging::err(&e));
            }
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}

async fn daily(ctx: &Arc<AppCtx>) -> anyhow::Result<()> {
    let (on, days, last) = ctx
        .db
        .read(|c| {
            Ok((
                get_setting::<bool>(c, CLEANUP_SETTING)?.unwrap_or(false),
                get_setting::<i64>(c, DAYS_SETTING)?.unwrap_or(30),
                get_setting::<i64>(c, LAST_CLEANUP)?.unwrap_or(0),
            ))
        })
        .await?;
    let now = crate::clock::now_ms();
    if !on || now - last < 20 * 3600 * 1000 {
        return Ok(());
    }
    clean_up(ctx, days.max(1)).await?;
    ctx.db.write(move |c| set_setting(c, LAST_CLEANUP, &now)).await
}

pub fn register(r: &mut crate::rpc::Registry) {
    r.add("mail.drive.status", status);
    r.add("mail.drive.renameFolder", rename);
    r.add("mail.drive.cleanNow", clean_now);
}

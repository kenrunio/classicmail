//! Core rpc methods: app info, accounts, Google sign-in, settings, Send/Receive.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    accounts::{self, Account},
    ctx::AppCtx,
    db::util::{get_setting, set_setting},
    google::auth::{self, ClientConfig},
    rpc::{NoParams, Registry},
};

pub fn register(r: &mut Registry) {
    r.add("app.info", app_info);
    r.add("app.openUrl", open_url);
    r.add("app.log", app_log);
    r.add("accounts.list", |ctx: Arc<AppCtx>, _: NoParams| async move { accounts::list(&ctx.db).await });
    r.add("accounts.addDemo", add_demo);
    r.add("accounts.signInGoogle", sign_in_google);
    r.add("accounts.remove", remove_account);
    r.add("accounts.update", update_account);
    r.add("google.setClientJson", set_client_json);
    r.add("settings.get", settings_get);
    r.add("settings.getMany", settings_get_many);
    r.add("settings.set", settings_set);
    r.add("sync.now", sync_now);
    r.add("sync.status", |ctx: Arc<AppCtx>, _: NoParams| async move { Ok(ctx.events.statuses()) });
    r.add("app.setTrayState", set_tray_state);
    r.add("app.getAutostart", get_autostart);
    r.add("app.setAutostart", set_autostart);
    // File › Exit: quit even when "close to tray" is on (closing the main window would only hide it)
    r.add("app.exit", |ctx: Arc<AppCtx>, _: NoParams| async move {
        if let Some(app) = &ctx.app {
            crate::shell::request_quit(app);
        }
        Ok(())
    });
    r.add("app.showMain", |ctx: Arc<AppCtx>, _: NoParams| async move {
        if let Some(app) = &ctx.app {
            crate::shell::show_main(app);
        }
        Ok(())
    });
    r.add("app.restoreNormalSize", restore_normal_size);
}

#[derive(Deserialize)]
struct LabelParam {
    label: String,
}

/// A window the plugin doesn't restore by itself (meeting windows): its last size and position,
/// but never maximized — the next meeting opens like the last one you sized, not full screen.
async fn restore_normal_size(ctx: Arc<AppCtx>, p: LabelParam) -> anyhow::Result<()> {
    use tauri::Manager;
    use tauri_plugin_window_state::{StateFlags, WindowExt};
    if let Some(w) = ctx.app.as_ref().and_then(|a| a.get_webview_window(&p.label)) {
        w.restore_state(StateFlags::SIZE | StateFlags::POSITION)?;
    }
    Ok(())
}

async fn app_info(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<Value> {
    let client = ctx.google_client().await?;
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "dataDir": ctx.data_dir.to_string_lossy(),
        "googleClientConfigured": client.is_some(),
        "googleClientEmbedded": ClientConfig::embedded().is_some(),
    }))
}

#[derive(Deserialize)]
struct UrlParams {
    url: String,
}

async fn open_url(ctx: Arc<AppCtx>, p: UrlParams) -> anyhow::Result<()> {
    let lower = p.url.to_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")) {
        return Err(crate::user_err!("bad_url", "only web and mailto links can be opened"));
    }
    if let Some(app) = &ctx.app {
        use tauri_plugin_opener::OpenerExt;
        app.opener().open_url(p.url, None::<&str>)?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct LogParams {
    level: String,
    message: String,
}

async fn app_log(_ctx: Arc<AppCtx>, p: LogParams) -> anyhow::Result<()> {
    // a front-end error can quote what failed (an address, a file name): masked, stack frames kept
    let message = crate::logging::redact_ui(&p.message);
    match p.level.as_str() {
        "error" => log::error!("[ui] {message}"),
        "warn" => log::warn!("[ui] {message}"),
        _ => log::info!("[ui] {message}"),
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddDemo {
    email: Option<String>,
    display_name: Option<String>,
}

/// Adds a demo account served by the fake Google server. With no e-mail, adds the standard pair.
async fn add_demo(ctx: Arc<AppCtx>, p: AddDemo) -> anyhow::Result<Vec<Account>> {
    let wanted: Vec<(String, String)> = match p.email {
        Some(e) => {
            let name = p.display_name.unwrap_or_else(|| crate::fake_google::seed::display_name_for(&e));
            vec![(e, name)]
        }
        None => vec![("kenny@duvz.demo".into(), "Kenny Green".into()), ("info@duvz.demo".into(), "Duvz Software".into())],
    };
    let mut out = Vec::new();
    for (email, name) in wanted {
        out.push(accounts::upsert(&ctx.db, &email, &name, "demo").await?);
    }
    ctx.events.emit("accounts.changed", json!({}));
    crate::sync::refresh_workers(&ctx).await?;
    Ok(out)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignIn {
    login_hint: Option<String>,
}

/// Interactive Google sign-in in the system browser. Resolves when the user finishes (or 5 min).
async fn sign_in_google(ctx: Arc<AppCtx>, p: SignIn) -> anyhow::Result<Account> {
    let client = ctx.google_client().await?.ok_or_else(|| crate::user_err!("no_client", "Import the Google OAuth client file first"))?;
    let urls = ctx.google_urls().await?;
    let app = ctx.app.clone();
    let res = auth::sign_in(&ctx.http, &client, &urls, p.login_hint.as_deref(), move |url| {
        if let Some(app) = app {
            use tauri_plugin_opener::OpenerExt;
            app.opener().open_url(url, None::<&str>)?;
        } else {
            // not the URL itself: it carries the address (login_hint) and the sign-in's state
            log::info!("sign-in URL ready (no window to open it in)");
        }
        Ok(())
    })
    .await?;
    auth::store_refresh_token(&res.email, &res.refresh_token)?;
    let acct = accounts::upsert(&ctx.db, &res.email, &res.name, "google").await?;
    ctx.forget_api(acct.id);
    ctx.events.emit("accounts.changed", json!({}));
    crate::sync::refresh_workers(&ctx).await?;
    ctx.sync_now(Some(acct.id));
    Ok(acct)
}

#[derive(Deserialize)]
struct IdParams {
    id: i64,
}

async fn remove_account(ctx: Arc<AppCtx>, p: IdParams) -> anyhow::Result<()> {
    let acct = accounts::get(&ctx.db, p.id).await?;
    // nothing of it stays on this computer, and Google's access is revoked
    accounts::remove_everywhere(&ctx, &acct).await?;
    ctx.events.emit("accounts.changed", json!({}));
    crate::sync::refresh_workers(&ctx).await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateAccount {
    id: i64,
    display_name: Option<String>,
    color: Option<String>,
    position: Option<i64>,
}

async fn update_account(ctx: Arc<AppCtx>, p: UpdateAccount) -> anyhow::Result<()> {
    accounts::update(&ctx.db, p.id, p.display_name, p.color, p.position).await?;
    ctx.events.emit("accounts.changed", json!({}));
    Ok(())
}

#[derive(Deserialize)]
struct ClientJson {
    json: String,
}

async fn set_client_json(ctx: Arc<AppCtx>, p: ClientJson) -> anyhow::Result<()> {
    let cfg = ClientConfig::from_google_json(&p.json).map_err(|e| crate::user_err!("bad_client", "{e}"))?;
    ctx.db.write(move |c| set_setting(c, "google.client", &cfg)).await?;
    ctx.forget_all_apis();
    Ok(())
}

#[derive(Deserialize)]
struct KeyParams {
    key: String,
}

async fn settings_get(ctx: Arc<AppCtx>, p: KeyParams) -> anyhow::Result<Value> {
    Ok(ctx.db.read(move |c| get_setting::<Value>(c, &p.key)).await?.unwrap_or(Value::Null))
}

#[derive(Deserialize)]
struct KeysParams {
    keys: Vec<String>,
}

async fn settings_get_many(ctx: Arc<AppCtx>, p: KeysParams) -> anyhow::Result<serde_json::Map<String, Value>> {
    ctx.db
        .read(move |c| {
            let mut m = serde_json::Map::new();
            for k in p.keys {
                let v = get_setting::<Value>(c, &k)?.unwrap_or(Value::Null);
                m.insert(k, v);
            }
            Ok(m)
        })
        .await
}

#[derive(Deserialize)]
struct SetParams {
    key: String,
    value: Value,
}

/// `dev.*` settings (e.g. `dev.googleUrls`, which redirects Google sign-in and token refresh) only
/// exist in debug builds.
pub const DEV_SETTINGS: bool = cfg!(debug_assertions);

/// Settings the UI may write. Secrets (google.client, tokens) have dedicated methods; `dev.*` only
/// in debug builds.
fn writable(key: &str, dev: bool) -> bool {
    !(key == "google.client" || key.starts_with("secret.") || (key.starts_with("dev.") && !dev))
}

async fn settings_set(ctx: Arc<AppCtx>, p: SetParams) -> anyhow::Result<()> {
    if !writable(&p.key, DEV_SETTINGS) {
        return Err(crate::user_err!("forbidden", "setting {} can't be written here", p.key));
    }
    let key = p.key.clone();
    ctx.db.write(move |c| set_setting(c, &p.key, &p.value)).await?;
    ctx.events.emit("settings.changed", json!({ "key": key }));
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncNow {
    account_id: Option<i64>,
}

async fn sync_now(ctx: Arc<AppCtx>, p: SyncNow) -> anyhow::Result<()> {
    ctx.sync_now(p.account_id);
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrayState {
    new_mail: Option<bool>,
    unread: Option<i64>,
}

async fn set_tray_state(ctx: Arc<AppCtx>, p: TrayState) -> anyhow::Result<()> {
    if let Some(app) = &ctx.app {
        crate::shell::set_tray_state(app, p.new_mail, p.unread);
    }
    Ok(())
}

async fn get_autostart(ctx: Arc<AppCtx>, _: NoParams) -> anyhow::Result<bool> {
    use tauri_plugin_autostart::ManagerExt;
    Ok(match &ctx.app {
        Some(app) => app.autolaunch().is_enabled().unwrap_or(false),
        None => false,
    })
}

#[derive(Deserialize)]
struct Enabled {
    enabled: bool,
}

async fn set_autostart(ctx: Arc<AppCtx>, p: Enabled) -> anyhow::Result<()> {
    use tauri_plugin_autostart::ManagerExt;
    if let Some(app) = &ctx.app {
        let al = app.autolaunch();
        if p.enabled { al.enable()? } else { al.disable()? }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_settings() {
        for k in ["mail.blockRemoteImages", "app.closeToTray", "mail.defaultAccountId"] {
            assert!(writable(k, false) && writable(k, true), "{k}");
        }
        for k in ["google.client", "secret.x"] {
            assert!(!writable(k, false) && !writable(k, true), "{k}");
        }
        // dev.googleUrls would send sign-ins / refresh tokens to another server: release builds refuse it
        assert!(!writable("dev.googleUrls", false));
        assert!(writable("dev.googleUrls", true));
    }
}

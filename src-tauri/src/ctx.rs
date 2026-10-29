//! Process-wide state handed to every rpc handler and background task.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use parking_lot::RwLock;

use crate::{
    accounts::{self, Account},
    db::Db,
    events::Events,
    google::{
        Api,
        auth::{ClientConfig, OAuthTokens, StaticToken},
        client::{Urls, http_client},
        limits::{Limiter, Quotas},
    },
};

pub struct AppCtx {
    /// None in tests.
    pub app: Option<tauri::AppHandle>,
    pub data_dir: PathBuf,
    pub db: Db,
    pub events: Events,
    pub http: reqwest::Client,
    apis: RwLock<HashMap<i64, Api>>,
    /// Base URL of the in-process fake Google server, once started.
    fake_base: tokio::sync::OnceCell<String>,
    pub sync: OnceLock<crate::sync::SyncHandle>,
    /// app-wide Undo / Redo history (in memory only)
    pub undo: crate::undo::UndoStack,
}

impl AppCtx {
    pub fn new(app: Option<tauri::AppHandle>, data_dir: PathBuf, db: Db, events: Events) -> Arc<Self> {
        Arc::new(AppCtx {
            app,
            data_dir,
            db,
            events,
            http: http_client(),
            apis: RwLock::new(HashMap::new()),
            fake_base: tokio::sync::OnceCell::new(),
            sync: OnceLock::new(),
            undo: Default::default(),
        })
    }

    /// Test helper: temp db + captured events. Keep the guard alive for the test's duration.
    pub fn for_test() -> anyhow::Result<(Arc<Self>, crate::db::tempdir::TempDirGuard)> {
        let (db, guard) = Db::open_temp()?;
        let dir = guard.path().to_path_buf();
        Ok((AppCtx::new(None, dir, db, Events::capture()), guard))
    }

    /// Start (once) and return the fake Google server's base URL.
    pub async fn fake_base(&self) -> anyhow::Result<String> {
        let path = self.data_dir.join("fake_google.json");
        self.fake_base.get_or_try_init(|| async move { crate::fake_google::start(Some(path)).await }).await.cloned()
    }

    /// The configured OAuth client (settings, else embedded at build time).
    pub async fn google_client(&self) -> anyhow::Result<Option<ClientConfig>> {
        let from_settings: Option<ClientConfig> = self.db.read(|c| crate::db::util::get_setting(c, "google.client")).await?;
        Ok(from_settings.or_else(ClientConfig::embedded))
    }

    /// Developer override: point the 'google' provider at another server (e.g. the fake one).
    /// Debug builds only — in a release build a changed setting must never send Google sign-ins
    /// or refresh tokens anywhere but Google.
    pub async fn google_urls(&self) -> anyhow::Result<Urls> {
        if !crate::core_api::DEV_SETTINGS {
            return Ok(Urls::google());
        }
        let o: Option<Urls> = self.db.read(|c| crate::db::util::get_setting(c, "dev.googleUrls")).await?;
        Ok(o.unwrap_or_else(Urls::google))
    }

    /// Authenticated API client for an account (cached).
    pub async fn api(&self, account_id: i64) -> anyhow::Result<Api> {
        if let Some(a) = self.apis.read().get(&account_id) {
            return Ok(a.clone());
        }
        let acct = accounts::get(&self.db, account_id).await?;
        let api = self.build_api(&acct).await?;
        self.apis.write().insert(account_id, api.clone());
        Ok(api)
    }

    async fn build_api(&self, acct: &Account) -> anyhow::Result<Api> {
        match acct.provider.as_str() {
            "demo" => {
                let base = self.fake_base().await?;
                Ok(Api::new(
                    acct.id,
                    acct.email.clone(),
                    Urls::fake(&base),
                    self.http.clone(),
                    Arc::new(StaticToken(format!("demo-{}", acct.email))),
                ))
            }
            _ => {
                let client = self
                    .google_client()
                    .await?
                    .ok_or_else(|| crate::user_err!("no_client", "Google sign-in is not set up yet (Tools › Account Settings)"))?;
                let urls = self.google_urls().await?;
                let tokens = OAuthTokens::new(acct.email.clone(), client, urls.token.clone());
                // Gmail quota per user: 6,000 units/min for projects created after 1 May 2026;
                // older projects keep 15,000 — set `google.gmailUnitsPerMinute` to use more.
                let gmail_units: Option<f64> = self.db.read(|c| crate::db::util::get_setting(c, "google.gmailUnitsPerMinute")).await?;
                let mut quotas = Quotas::default();
                if let Some(u) = gmail_units.filter(|u| *u >= 600.0) {
                    quotas.gmail_units_per_minute = u;
                }
                Ok(Api::new(acct.id, acct.email.clone(), urls, self.http.clone(), Arc::new(tokens)).with_limiter(Limiter::new(quotas)))
            }
        }
    }

    pub fn forget_api(&self, account_id: i64) {
        self.apis.write().remove(&account_id);
    }

    /// Drop every cached client (e.g. a new OAuth client file: refresh with the new one).
    pub fn forget_all_apis(&self) {
        self.apis.write().clear();
    }

    /// Ask the sync workers to run now (F9). No-op before sync has started (tests).
    pub fn sync_now(&self, account_id: Option<i64>) {
        if let Some(s) = self.sync.get() {
            s.trigger(account_id);
        }
    }

    /// Let the account's sync worker re-check its timers (no forced sync).
    pub fn wake_sync(&self, account_id: i64) {
        if let Some(s) = self.sync.get() {
            s.wake(account_id);
        }
    }
}

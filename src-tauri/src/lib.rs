// Clippy lints that don't fit this code base:
// * result_large_err / large_enum_variant: errors and messages are built once per call, not in loops
// * type_complexity / too_many_arguments: a few rusqlite row tuples and internal helpers
// * unnecessary_sort_by: `b.x.cmp(&a.x)` (newest first) reads clearer than `Reverse(x.clone())`
// * should_implement_trait / inherent_to_string: parser `next()` and RRULE `to_string()` by design
#![allow(
    clippy::result_large_err,
    clippy::large_enum_variant,
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::unnecessary_sort_by,
    clippy::should_implement_trait,
    clippy::inherent_to_string
)]

pub mod accounts;
pub mod ai;
pub mod archive;
pub mod calendar;
pub mod clipboard;
pub mod clock;
pub mod contacts;
pub mod core_api;
pub mod ctx;
pub mod db;
pub mod error;
pub mod events;
pub mod fake_google;
pub mod google;
pub mod keyboard;
pub mod logging;
pub mod mail;
pub mod mapi;
pub mod rpc;
pub mod rules;
pub mod shell;
pub mod spare;
pub mod sync;
pub mod tasks;
pub mod undo;

use std::{path::PathBuf, sync::Arc};

use serde_json::Value;
use tauri::{Manager, State};

use crate::{ctx::AppCtx, error::RpcError, rpc::Registry};

pub struct AppState {
    pub ctx: Arc<AppCtx>,
    pub registry: Arc<Registry>,
}

#[tauri::command]
async fn rpc(state: State<'_, AppState>, method: String, params: Value) -> Result<Value, RpcError> {
    let ctx = state.ctx.clone();
    let reg = state.registry.clone();
    let started = std::time::Instant::now();
    let out = reg.call(ctx, &method, params).await;
    let ms = started.elapsed().as_millis();
    match &out {
        // the UI gets the whole message; the log only its code (it can quote names, addresses, files)
        Err(e) => log::warn!("rpc {method} failed after {ms} ms: {}", logging::err(e)),
        Ok(_) if ms > 500 => log::info!("rpc {method} took {ms} ms"),
        _ => {}
    }
    out.map_err(RpcError::from)
}

/// `CLASSICMAIL_DATA_DIR` overrides the app data dir (dev slots, tests, portable use).
fn data_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("CLASSICMAIL_DATA_DIR") {
        return PathBuf::from(d);
    }
    app.path().app_data_dir().expect("app data dir")
}

/// Panics go to the log file (release builds have no console) and no longer take the app down:
/// a panic inside a background task ends only that task. Release builds log only where: a panic
/// message can quote content (a string slice panic shows the text).
fn install_panic_logger() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        if cfg!(debug_assertions) {
            log::error!("PANIC at {loc}: {msg}");
        } else {
            log::error!("PANIC at {loc}");
        }
        default(info);
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_logger();
    let log_level = if cfg!(debug_assertions) { log::LevelFilter::Debug } else { log::LevelFilter::Info };
    let mut builder = tauri::Builder::default();
    // Dev slots (CLASSICMAIL_DATA_DIR set) run side by side, so only a normal install is single-instance.
    if std::env::var_os("CLASSICMAIL_DATA_DIR").is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            shell::show_main(app);
        }));
    }
    let mut log_builder = tauri_plugin_log::Builder::new();
    // dev slots / portable data dir: keep the log next to the data instead of the shared app log
    if let Ok(d) = std::env::var("CLASSICMAIL_DATA_DIR") {
        use tauri_plugin_log::{Target, TargetKind};
        log_builder = log_builder.targets([
            Target::new(TargetKind::Stdout),
            Target::new(TargetKind::Folder { path: PathBuf::from(d).join("logs"), file_name: Some("ClassicMail".into()) }),
        ]);
    }
    builder
        .plugin(
            log_builder
                .level(log_level)
                .level_for("hyper", log::LevelFilter::Warn)
                .level_for("hyper_util", log::LevelFilter::Warn)
                .level_for("reqwest", log::LevelFilter::Warn)
                .level_for("tao", log::LevelFilter::Warn)
                .level_for("html5ever", log::LevelFilter::Warn)
                // 2 MB files, the current one and the two before it (the default drops the old file)
                .max_file_size(2_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(2))
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_denylist(&["desktop-alert", "reminders", "recurrence", "webpage"])
                // one saved size/position per window kind, not per item window
                .map_label(shell::window_state_key)
                // meeting windows restore their own size, never maximized (app.restoreNormalSize)
                .skip_initial_state("appointment")
                // never save "hidden" (closed to the tray) — the main window must come back on start
                .with_state_flags(tauri_plugin_window_state::StateFlags::all() - tauri_plugin_window_state::StateFlags::VISIBLE)
                .build(),
        )
        .register_asynchronous_uri_scheme_protocol("cmattach", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = app.state::<AppState>();
                responder.respond(crate::mail::protocol_response(&state.ctx, request.uri().path()).await);
            });
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let dir = data_dir(&handle);
            std::fs::create_dir_all(&dir)?;
            log::info!("data dir: {}", dir.display());
            let db = db::Db::open(&dir.join("classicmail.db"))?;
            let ctx = AppCtx::new(Some(handle.clone()), dir, db, events::Events::tauri(handle.clone()));
            let registry = Arc::new(rpc::build());
            app.manage(AppState { ctx: ctx.clone(), registry });
            let c2 = ctx.clone();
            // on the async runtime: the sync workers are tokio tasks
            tauri::async_runtime::spawn(async move {
                sync::start(c2);
            });
            mapi::start(ctx.clone());
            // reopened drafts' attachment copies that nothing refers to any more
            mail::compose::cleanup::start(ctx.clone());
            // the AI model list, refreshed once a day (models are added and retired)
            ai::models::start_refresh(ctx.clone());
            shell::setup(app, ctx)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![rpc])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

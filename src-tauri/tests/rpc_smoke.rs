//! Links the whole app (every rpc handler, plugins included) into a test exe and calls a few core
//! methods — guards against the Windows manifest regression (0xc0000139) and registry panics.

use classicmail_lib::{ctx::AppCtx, rpc};
use serde_json::json;

#[tokio::test]
async fn registry_builds_and_core_methods_work() {
    let reg = rpc::build();
    assert!(reg.names().contains(&"app.info"));
    let (ctx, _guard) = AppCtx::for_test().unwrap();
    let info = reg.call(ctx.clone(), "app.info", json!({})).await.unwrap();
    // true when google-client.json is embedded at build time, false otherwise
    assert!(info["googleClientConfigured"].is_boolean());
    reg.call(ctx.clone(), "settings.set", json!({"key": "mail.pollSeconds", "value": 120})).await.unwrap();
    let v = reg.call(ctx.clone(), "settings.get", json!({"key": "mail.pollSeconds"})).await.unwrap();
    assert_eq!(v, json!(120));
    assert!(reg.call(ctx.clone(), "settings.set", json!({"key": "google.client", "value": 1})).await.is_err());
    let accts = reg.call(ctx.clone(), "accounts.addDemo", json!({})).await.unwrap();
    assert_eq!(accts.as_array().unwrap().len(), 2);
    let list = reg.call(ctx, "accounts.list", json!({})).await.unwrap();
    assert_eq!(list[0]["email"], json!("kenny@duvz.demo"));
}

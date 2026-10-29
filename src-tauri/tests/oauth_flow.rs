//! The real OAuth code path (PKCE + loopback redirect + token exchange + refresh via the keyring)
//! against the fake server's /auth, /token and /userinfo endpoints. The "browser" is simulated by
//! following the authorisation URL with an HTTP client.

use std::sync::Arc;

use classicmail_lib::google::{
    auth::{self, ClientConfig, OAuthTokens, TokenSource},
    client::{Urls, http_client},
};

#[tokio::test]
async fn sign_in_and_refresh_round_trip() {
    let base = classicmail_lib::fake_google::start(None).await.unwrap();
    let urls = Urls::fake(&base);
    let http = http_client();
    let client = ClientConfig { client_id: "test-client".into(), client_secret: "s".into() };
    let email = "oauth-test@duvz.demo";

    let res = auth::sign_in(&http, &client, &urls, Some(email), |url| {
        // the "browser": follow /auth → redirect to our loopback listener
        tokio::spawn(async move {
            let _ = reqwest::get(url).await;
        });
        Ok(())
    })
    .await
    .expect("sign in");
    assert_eq!(res.email, email);
    assert!(res.refresh_token.starts_with("refresh-"));

    auth::store_refresh_token(email, &res.refresh_token).unwrap();
    let tokens: Arc<dyn TokenSource> = Arc::new(OAuthTokens::new(email.into(), client, urls.token.clone()));
    let access = tokens.access_token(&http, false).await.unwrap();
    assert_eq!(access, format!("demo-{email}"));
    // cached second time, forced refresh works too
    assert_eq!(tokens.access_token(&http, false).await.unwrap(), access);
    assert_eq!(tokens.access_token(&http, true).await.unwrap(), access);

    auth::delete_refresh_token(email).unwrap();
    assert!(auth::load_refresh_token(email).unwrap().is_none());
    // no token → a clear "sign in again" error
    let fresh = OAuthTokens::new(email.into(), ClientConfig { client_id: "x".into(), client_secret: "y".into() }, urls.token);
    let err = fresh.access_token(&http, false).await.unwrap_err();
    assert!(err.to_string().contains("sign in again"), "{err}");
}

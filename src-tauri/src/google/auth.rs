//! OAuth 2.0 for installed apps: PKCE + loopback redirect, tokens refreshed from a refresh token
//! kept in Windows Credential Manager.

use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::client::{ApiError, Urls};

pub const SCOPES: &[&str] = &[
    "openid",
    "email",
    "profile",
    "https://mail.google.com/",
    "https://www.googleapis.com/auth/calendar",
    "https://www.googleapis.com/auth/contacts",
    "https://www.googleapis.com/auth/contacts.other.readonly",
    "https://www.googleapis.com/auth/tasks",
    // Rules kept as Gmail filters (rules::gmail_filters); optional, nothing else needs it
    "https://www.googleapis.com/auth/gmail.settings.basic",
];

const KEYRING_SERVICE: &str = "ClassicMail";

/// The OAuth client (from the "Desktop app" credentials JSON downloaded from Google Cloud).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientConfig {
    pub client_id: String,
    pub client_secret: String,
}

impl ClientConfig {
    /// Accepts the JSON exactly as Google downloads it (`{"installed": {...}}`) or a flat object.
    pub fn from_google_json(text: &str) -> anyhow::Result<Self> {
        let v: serde_json::Value = serde_json::from_str(text).context("not valid JSON")?;
        let inner = v.get("installed").or_else(|| v.get("web")).unwrap_or(&v);
        let id = inner["client_id"].as_str().or_else(|| inner["clientId"].as_str());
        let secret = inner["client_secret"].as_str().or_else(|| inner["clientSecret"].as_str());
        match (id, secret) {
            (Some(id), Some(secret)) => Ok(ClientConfig { client_id: id.into(), client_secret: secret.into() }),
            _ => bail!("the file has no client_id / client_secret — download the JSON of a \"Desktop app\" OAuth client"),
        }
    }

    /// Embedded at build time from `google-client.json` in the project root, if it existed.
    pub fn embedded() -> Option<Self> {
        let text = option_env!("CLASSICMAIL_GOOGLE_CLIENT_JSON")?;
        Self::from_google_json(text).ok()
    }
}

#[async_trait::async_trait]
pub trait TokenSource: Send + Sync {
    /// A valid access token; `force_refresh` after a 401.
    async fn access_token(&self, http: &reqwest::Client, force_refresh: bool) -> anyhow::Result<String>;
}

/// Fixed token (demo accounts / tests against the fake server).
pub struct StaticToken(pub String);

#[async_trait::async_trait]
impl TokenSource for StaticToken {
    async fn access_token(&self, _http: &reqwest::Client, _force: bool) -> anyhow::Result<String> {
        Ok(self.0.clone())
    }
}

/// Real Google account: refresh token in the keyring, access token cached in memory.
pub struct OAuthTokens {
    pub email: String,
    pub client: ClientConfig,
    pub token_url: String,
    cached: Mutex<Option<(String, Instant)>>,
    refresh_lock: tokio::sync::Mutex<()>,
}

impl OAuthTokens {
    pub fn new(email: String, client: ClientConfig, token_url: String) -> Self {
        OAuthTokens { email, client, token_url, cached: Mutex::new(None), refresh_lock: tokio::sync::Mutex::new(()) }
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    /// space-separated scopes actually granted (users can untick some on Google's consent screen)
    scope: Option<String>,
}

/// API scopes the user may have left unticked on Google's (granular) consent screen, with the
/// name shown to the user. `contacts.other.readonly` only feeds address autocomplete: optional.
fn missing_scopes(granted: Option<&str>) -> Vec<&'static str> {
    let Some(granted) = granted else { return vec![] }; // not reported: assume all
    let have: Vec<&str> = granted.split_whitespace().collect();
    [
        ("https://mail.google.com/", "Gmail"),
        ("https://www.googleapis.com/auth/calendar", "Calendar"),
        ("https://www.googleapis.com/auth/contacts", "Contacts"),
        ("https://www.googleapis.com/auth/tasks", "Tasks"),
    ]
    .into_iter()
    .filter(|(scope, _)| !have.contains(scope))
    .map(|(_, name)| name)
    .collect()
}

#[async_trait::async_trait]
impl TokenSource for OAuthTokens {
    async fn access_token(&self, http: &reqwest::Client, force_refresh: bool) -> anyhow::Result<String> {
        if !force_refresh
            && let Some((tok, exp)) = self.cached.lock().clone()
            && exp > Instant::now() + Duration::from_secs(60)
        {
            return Ok(tok);
        }
        let _g = self.refresh_lock.lock().await;
        // someone else may have refreshed while we waited
        if !force_refresh
            && let Some((tok, exp)) = self.cached.lock().clone()
            && exp > Instant::now() + Duration::from_secs(60)
        {
            return Ok(tok);
        }
        let refresh = load_refresh_token(&self.email)?.ok_or_else(|| ApiError::Auth(format!("{} needs to sign in again", self.email)))?;
        let res = http
            .post(&self.token_url)
            .form(&[
                ("client_id", self.client.client_id.as_str()),
                ("client_secret", self.client.client_secret.as_str()),
                ("refresh_token", refresh.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|e| ApiError::Network(e.without_url().to_string()))?;
        if !res.status().is_success() {
            let status = res.status().as_u16();
            let body = res.text().await.unwrap_or_default();
            // {"error": "invalid_grant", "error_description": "Token has been expired or revoked."}
            let (message, reason) = super::client::parse_google_error(&body);
            match reason.as_deref() {
                Some("invalid_grant") => {
                    return Err(ApiError::Auth(format!("{} needs to sign in again (Google: {message})", self.email)).into());
                }
                Some("invalid_client" | "unauthorized_client") => {
                    return Err(ApiError::Auth(format!(
                        "Google rejected ClassicMail's OAuth client ({message}); import the client file again in Account Settings"
                    ))
                    .into());
                }
                _ => {}
            }
            // 5xx / network trouble at the token endpoint: not a sign-in problem, retried next cycle
            return Err(ApiError::Http { status, message, reason }.into());
        }
        let t: TokenResponse = res.json().await?;
        let exp = Instant::now() + Duration::from_secs(t.expires_in.unwrap_or(3600));
        *self.cached.lock() = Some((t.access_token.clone(), exp));
        Ok(t.access_token)
    }
}

// ---- keyring --------------------------------------------------------------------------------

fn entry(email: &str) -> anyhow::Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, email)?)
}

pub fn store_refresh_token(email: &str, token: &str) -> anyhow::Result<()> {
    entry(email)?.set_password(token).context("saving the sign-in to Windows Credential Manager")?;
    Ok(())
}

pub fn load_refresh_token(email: &str) -> anyhow::Result<Option<String>> {
    match entry(email)?.get_password() {
        Ok(t) => Ok(Some(t)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn delete_refresh_token(email: &str) -> anyhow::Result<()> {
    match entry(email)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Generic secret storage (API keys etc.) under the same keyring service.
pub fn store_secret(name: &str, value: &str) -> anyhow::Result<()> {
    store_refresh_token(&format!("secret:{name}"), value)
}
pub fn load_secret(name: &str) -> anyhow::Result<Option<String>> {
    load_refresh_token(&format!("secret:{name}"))
}
pub fn delete_secret(name: &str) -> anyhow::Result<()> {
    delete_refresh_token(&format!("secret:{name}"))
}

// ---- interactive sign-in --------------------------------------------------------------------

pub struct SignInResult {
    pub email: String,
    pub name: String,
    pub refresh_token: String,
}

fn random_token(len: usize) -> String {
    let bytes: Vec<u8> = (0..len).map(|_| rand::random::<u8>()).collect();
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Full interactive flow: listen on a loopback port, open the browser (via `open_url`), wait for
/// Google to redirect back with the code (5 min timeout), exchange it, read the user's e-mail.
pub async fn sign_in(
    http: &reqwest::Client,
    client: &ClientConfig,
    urls: &Urls,
    login_hint: Option<&str>,
    open_url: impl FnOnce(String) -> anyhow::Result<()>,
) -> anyhow::Result<SignInResult> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let redirect = format!("http://127.0.0.1:{port}/");
    let verifier = random_token(48);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token(16);

    let mut auth = url::Url::parse(&urls.auth)?;
    {
        let mut q = auth.query_pairs_mut();
        q.append_pair("client_id", &client.client_id)
            .append_pair("redirect_uri", &redirect)
            .append_pair("response_type", "code")
            .append_pair("scope", &SCOPES.join(" "))
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent");
        if let Some(h) = login_hint {
            q.append_pair("login_hint", h);
        }
    }
    open_url(auth.to_string())?;

    let code = tokio::time::timeout(Duration::from_secs(300), wait_for_code(&listener, &state))
        .await
        .map_err(|_| anyhow!("timed out waiting for the Google sign-in to finish"))??;

    let res = http
        .post(&urls.token)
        .form(&[
            ("client_id", client.client_id.as_str()),
            ("client_secret", client.client_secret.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect.as_str()),
        ])
        .send()
        .await?;
    if !res.status().is_success() {
        bail!("Google refused the sign-in: {}", res.text().await.unwrap_or_default());
    }
    let t: TokenResponse = res.json().await?;
    let missing = missing_scopes(t.scope.as_deref());
    if !missing.is_empty() {
        bail!(
            "Google did not give ClassicMail access to {}. Sign in again and tick every box on Google's permission screen.",
            missing.join(", ")
        );
    }
    let refresh_token = t.refresh_token.ok_or_else(|| {
        anyhow!("Google did not return a refresh token; remove ClassicMail's access in your Google account and try again")
    })?;
    let _ = t.id_token;
    let info: serde_json::Value = http.get(&urls.userinfo).bearer_auth(&t.access_token).send().await?.error_for_status()?.json().await?;
    let email = info["email"].as_str().ok_or_else(|| anyhow!("no e-mail in the Google profile"))?.to_lowercase();
    let name = info["name"].as_str().unwrap_or(&email).to_string();
    Ok(SignInResult { email, name, refresh_token })
}

/// Accept connections on the loopback port until one carries Google's redirect. Browsers open
/// speculative connections that send nothing (or only later), so every connection is served on
/// its own task instead of blocking the accept loop.
async fn wait_for_code(listener: &tokio::net::TcpListener, state: &str) -> anyhow::Result<String> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<anyhow::Result<String>>(4);
    loop {
        tokio::select! {
            acc = listener.accept() => {
                let (sock, _) = acc?;
                let tx = tx.clone();
                let state = state.to_string();
                tokio::spawn(async move {
                    if let Some(r) = serve_redirect(sock, &state).await {
                        let _ = tx.send(r).await;
                    }
                });
            }
            Some(r) = rx.recv() => return r,
        }
    }
}

/// One loopback connection: Some(result) if it was the OAuth redirect.
async fn serve_redirect(mut sock: tokio::net::TcpStream, state: &str) -> Option<anyhow::Result<String>> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    // read the request head (a preconnected socket may stay silent: give up after a while)
    loop {
        let n = match tokio::time::timeout(Duration::from_secs(120), sock.read(&mut chunk)).await {
            Ok(Ok(n)) => n,
            _ => return None,
        };
        if n == 0 {
            if buf.is_empty() {
                return None;
            }
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 16 * 1024 {
            break;
        }
    }
    let req = String::from_utf8_lossy(&buf);
    let path = req.split_whitespace().nth(1).unwrap_or("/");
    let params: std::collections::HashMap<String, String> =
        url::Url::parse(&format!("http://localhost{path}")).map(|u| u.query_pairs().into_owned().collect()).unwrap_or_default();
    if params.is_empty() {
        // favicon or similar
        let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return None;
    }
    if params.get("state").map(String::as_str) != Some(state) {
        // not our browser tab (another local program, or a stale tab): answer, but keep waiting
        // for the real redirect instead of letting it end (or cancel) the sign-in
        let html = page("Sign-in failed", "Please try again from ClassicMail.");
        let resp = format!(
            "HTTP/1.1 400 Bad Request
Content-Type: text/html; charset=utf-8
Content-Length: {}
Connection: close

{html}",
            html.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        return None;
    }
    let (result, html) = if let Some(err) = params.get("error") {
        (
            Err(anyhow!("Google sign-in was cancelled ({err})")),
            page("Sign-in cancelled", "You can close this tab and try again from ClassicMail."),
        )
    } else if let Some(code) = params.get("code") {
        (Ok(code.clone()), page("Signed in", "ClassicMail is now connected. You can close this tab."))
    } else {
        let _ = sock.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return None;
    };
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = sock.write_all(resp.as_bytes()).await;
    let _ = sock.shutdown().await;
    Some(result)
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>{title}</title>\
         <body style=\"font:14px Segoe UI,Tahoma,sans-serif;background:#BFDBFF;margin:0\">\
         <div style=\"max-width:420px;margin:80px auto;background:#fff;border:1px solid #6593CF;padding:24px\">\
         <h2 style=\"color:#15428B;margin-top:0\">{title}</h2><p>{body}</p></div>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loopback_ignores_strangers_and_wrong_state() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(addr.ip().is_loopback());
        let waiter = tokio::spawn(async move { wait_for_code(&listener, "good-state").await });
        async fn get(addr: std::net::SocketAddr, path: &str) -> String {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out).await;
            out
        }
        // a silent connection doesn't block others
        let _silent = tokio::net::TcpStream::connect(addr).await.unwrap();
        // forged callbacks: wrong state (code or error) are refused and don't end the wait
        assert!(get(addr, "/?code=evil&state=bad").await.starts_with("HTTP/1.1 400"));
        assert!(get(addr, "/?error=access_denied&state=bad").await.starts_with("HTTP/1.1 400"));
        assert!(!waiter.is_finished());
        assert!(get(addr, "/?code=real&state=good-state").await.starts_with("HTTP/1.1 200"));
        assert_eq!(waiter.await.unwrap().unwrap(), "real");
    }

    #[test]
    fn unticked_scopes_are_reported() {
        let all = "openid https://www.googleapis.com/auth/userinfo.email https://mail.google.com/ https://www.googleapis.com/auth/calendar https://www.googleapis.com/auth/contacts https://www.googleapis.com/auth/tasks";
        assert!(missing_scopes(Some(all)).is_empty());
        assert!(missing_scopes(None).is_empty());
        let partial = "openid https://mail.google.com/ https://www.googleapis.com/auth/tasks";
        assert_eq!(missing_scopes(Some(partial)), vec!["Calendar", "Contacts"]);
    }

    /// A browser's speculative connection that never sends anything must not block the redirect.
    #[tokio::test]
    async fn loopback_survives_a_silent_preconnect() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = tokio::spawn(async move { wait_for_code(&listener, "st4te").await });
        let _silent = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut real = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        real.write_all(b"GET /?state=st4te&code=4%2F0Abc&scope=email HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").await.unwrap();
        let mut resp = String::new();
        real.read_to_string(&mut resp).await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 200"));
        let code = tokio::time::timeout(Duration::from_secs(5), waiter).await.unwrap().unwrap().unwrap();
        assert_eq!(code, "4/0Abc");
    }

    #[test]
    fn parses_google_client_json() {
        let j = r#"{"installed":{"client_id":"abc.apps.googleusercontent.com","client_secret":"s3","redirect_uris":["http://localhost"]}}"#;
        let c = ClientConfig::from_google_json(j).unwrap();
        assert_eq!(c.client_id, "abc.apps.googleusercontent.com");
        assert_eq!(c.client_secret, "s3");
        assert!(ClientConfig::from_google_json("{}").is_err());
    }
}

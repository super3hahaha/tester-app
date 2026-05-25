use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Deserialize)]
struct OAuthFile {
    installed: OAuthConfig,
}

#[derive(Deserialize, Clone)]
struct OAuthConfig {
    client_id: String,
    client_secret: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct UserInfo {
    pub email: String,
    pub name: String,
    pub picture: Option<String>,
}

pub struct AuthState {
    tokens: Mutex<Option<AuthTokens>>,
    user: Mutex<Option<UserInfo>>,
}

impl AuthState {
    pub fn new() -> Self {
        Self {
            tokens: Mutex::new(load_tokens_from_disk()),
            user: Mutex::new(load_user_from_disk()),
        }
    }
}

/// Returns a non-expired access_token, refreshing if necessary.
/// On refresh failure due to `invalid_grant` (refresh_token revoked / expired),
/// clears persisted auth and returns an error starting with "NEED_RELOGIN:"
/// so the frontend can route the user back to the login page.
pub async fn get_valid_access_token(state: &State<'_, AuthState>) -> Result<String, String> {
    // Snapshot current tokens; do NOT hold the std::sync::Mutex across .await.
    let current = state.tokens.lock().unwrap().clone();
    let Some(tokens) = current else {
        return Err("NEED_RELOGIN: not logged in".into());
    };

    if tokens.expires_at > now_ts() + 60 {
        return Ok(tokens.access_token);
    }

    match refresh_access_token(&tokens).await {
        Ok(new_tokens) => {
            save_tokens_to_disk(&new_tokens);
            let access = new_tokens.access_token.clone();
            *state.tokens.lock().unwrap() = Some(new_tokens);
            Ok(access)
        }
        Err(e) if e.starts_with("invalid_grant") => {
            *state.tokens.lock().unwrap() = None;
            *state.user.lock().unwrap() = None;
            std::fs::remove_file(data_dir().join("auth-tokens.json")).ok();
            std::fs::remove_file(data_dir().join("auth-user.json")).ok();
            Err(format!("NEED_RELOGIN: {}", e))
        }
        Err(e) => Err(format!("Token refresh failed: {}", e)),
    }
}

// ---- File helpers ----

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

fn load_tokens_from_disk() -> Option<AuthTokens> {
    std::fs::read_to_string(data_dir().join("auth-tokens.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn load_user_from_disk() -> Option<UserInfo> {
    std::fs::read_to_string(data_dir().join("auth-user.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn save_tokens_to_disk(tokens: &AuthTokens) {
    std::fs::create_dir_all(data_dir()).ok();
    let json = serde_json::to_string_pretty(tokens).unwrap();
    std::fs::write(data_dir().join("auth-tokens.json"), json).ok();
}

fn save_user_to_disk(user: &UserInfo) {
    std::fs::create_dir_all(data_dir()).ok();
    let json = serde_json::to_string_pretty(user).unwrap();
    std::fs::write(data_dir().join("auth-user.json"), json).ok();
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

// ---- PKCE ----

fn generate_code_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

// ---- OAuth config ----

fn load_oauth_config() -> Result<OAuthConfig, String> {
    let candidates = [
        PathBuf::from("credentials/oauth.json"),
        PathBuf::from("src-tauri/credentials/oauth.json"),
        data_dir().join("oauth.json"),
    ];
    for path in &candidates {
        if path.exists() {
            let content = std::fs::read_to_string(path)
                .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
            let file: OAuthFile = serde_json::from_str(&content)
                .map_err(|e| format!("Cannot parse oauth.json: {}", e))?;
            return Ok(file.installed);
        }
    }
    Err("oauth.json not found. Place it in src-tauri/credentials/oauth.json".into())
}

// ---- Commands ----

#[tauri::command]
pub async fn check_auth(state: State<'_, AuthState>) -> Result<Option<UserInfo>, String> {
    let tokens = state.tokens.lock().unwrap().clone();
    let user = state.user.lock().unwrap().clone();

    match (tokens, user) {
        (Some(tokens), Some(user)) => {
            if tokens.expires_at > now_ts() + 60 {
                Ok(Some(user))
            } else {
                match refresh_access_token(&tokens).await {
                    Ok(new_tokens) => {
                        save_tokens_to_disk(&new_tokens);
                        *state.tokens.lock().unwrap() = Some(new_tokens);
                        Ok(Some(user))
                    }
                    Err(_) => {
                        *state.tokens.lock().unwrap() = None;
                        *state.user.lock().unwrap() = None;
                        Ok(None)
                    }
                }
            }
        }
        _ => Ok(None),
    }
}

#[tauri::command]
pub async fn start_login(state: State<'_, AuthState>) -> Result<UserInfo, String> {
    let config = load_oauth_config()?;

    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Cannot bind local server: {}", e))?;
    let port = listener.local_addr().unwrap().port();
    let redirect_uri = format!("http://127.0.0.1:{}", port);

    let scopes = "openid email profile \
        https://www.googleapis.com/auth/spreadsheets \
        https://www.googleapis.com/auth/drive.readonly \
        https://www.googleapis.com/auth/drive.file \
        https://www.googleapis.com/auth/presentations.readonly";

    let mut auth_url =
        url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").unwrap();
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", scopes)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");

    open::that(auth_url.as_str()).map_err(|e| format!("Cannot open browser: {}", e))?;

    let auth_code = wait_for_callback(&listener).await?;

    let client = reqwest::Client::new();
    let token_resp: TokenResponse = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", auth_code.as_str()),
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", code_verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("Token exchange failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Token parse failed: {}", e))?;

    let refresh_token = token_resp.refresh_token.ok_or_else(|| {
        "Google did not return a refresh_token. Fully revoke access at \
         https://myaccount.google.com/permissions and log in again."
            .to_string()
    })?;
    let tokens = AuthTokens {
        access_token: token_resp.access_token.clone(),
        refresh_token,
        expires_at: now_ts() + token_resp.expires_in,
    };

    let user_info: UserInfo = client
        .get("https://www.googleapis.com/oauth2/v3/userinfo")
        .bearer_auth(&tokens.access_token)
        .send()
        .await
        .map_err(|e| format!("Userinfo failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Userinfo parse failed: {}", e))?;

    save_tokens_to_disk(&tokens);
    save_user_to_disk(&user_info);
    *state.tokens.lock().unwrap() = Some(tokens);
    *state.user.lock().unwrap() = Some(user_info.clone());

    Ok(user_info)
}

#[tauri::command]
pub async fn logout(state: State<'_, AuthState>) -> Result<(), String> {
    *state.tokens.lock().unwrap() = None;
    *state.user.lock().unwrap() = None;
    std::fs::remove_file(data_dir().join("auth-tokens.json")).ok();
    std::fs::remove_file(data_dir().join("auth-user.json")).ok();
    Ok(())
}

// ---- Internal ----

async fn wait_for_callback(listener: &TcpListener) -> Result<String, String> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| format!("Accept failed: {}", e))?;

        let mut buf = vec![0u8; 4096];
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| format!("Read failed: {}", e))?;

        let request = String::from_utf8_lossy(&buf[..n]);
        let path = request
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .nth(1)
            .unwrap_or("/");

        let parsed = url::Url::parse(&format!("http://localhost{}", path))
            .map_err(|e| format!("URL parse: {}", e))?;

        if let Some((_, err)) = parsed.query_pairs().find(|(k, _)| k == "error") {
            let html = format!(
                "<html><body><h2>Login failed: {}</h2><p>You can close this tab.</p></body></html>",
                err
            );
            send_html(&mut stream, &html).await;
            return Err(format!("OAuth error: {}", err));
        }

        if let Some((_, code)) = parsed.query_pairs().find(|(k, _)| k == "code") {
            let html = "<html><body style='font-family:sans-serif;display:flex;justify-content:center;align-items:center;height:100vh'>\
                <div><h2>Login successful!</h2><p>You can close this tab and return to the app.</p></div>\
                </body></html>";
            send_html(&mut stream, html).await;
            return Ok(code.to_string());
        }

        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\n\r\n")
            .await
            .ok();
    }
}

async fn send_html(stream: &mut tokio::net::TcpStream, html: &str) {
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    stream.write_all(resp.as_bytes()).await.ok();
}

async fn refresh_access_token(tokens: &AuthTokens) -> Result<AuthTokens, String> {
    if tokens.refresh_token.is_empty() {
        return Err("invalid_grant: no refresh_token stored locally".into());
    }
    let config = load_oauth_config()?;
    let client = reqwest::Client::new();

    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("refresh_token", tokens.refresh_token.as_str()),
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| format!("Refresh request failed: {}", e))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Refresh body read failed: {}", e))?;

    if !status.is_success() {
        // Google returns 400 with { "error": "invalid_grant", ... } when the
        // refresh_token is revoked / expired / from another client.
        if body.contains("invalid_grant") {
            return Err(format!("invalid_grant: {}", body));
        }
        return Err(format!("Refresh {}: {}", status, body));
    }

    let parsed: TokenResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Refresh parse failed: {}: body={}", e, body))?;

    Ok(AuthTokens {
        access_token: parsed.access_token,
        refresh_token: tokens.refresh_token.clone(),
        expires_at: now_ts() + parsed.expires_in,
    })
}

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
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
    /// Google OpenID 唯一 ID。作为账号存储 key 的首选；迁移的旧账号可能缺失。
    #[serde(default)]
    pub sub: Option<String>,
    pub email: String,
    pub name: String,
    pub picture: Option<String>,
    /// 账号稳定唯一标识（= account_key，sub 优先回退 email）。前端把它当 opaque id，
    /// 用作「按账号隔离本地存储」的维度；由后端在 account 进入 AuthState 时统一填充
    /// （load/登录处），前端不自行推算，换 provider 只改后端 account_key 一处。
    /// 落盘会带此字段但无害：读回时 account_key 重算、id 重填。
    #[serde(default)]
    pub id: String,
}

/// 暴露给前端的账号条目（含稳定 id 与 active 标记）。
#[derive(Serialize, Clone)]
pub struct AccountInfo {
    pub id: String,
    pub email: String,
    pub name: String,
    pub picture: Option<String>,
    pub active: bool,
}

#[derive(Clone)]
struct Account {
    tokens: AuthTokens,
    user: UserInfo,
}

pub struct AuthState {
    /// key = account_key(user)（sub 优先，回退 email）
    accounts: Mutex<HashMap<String, Account>>,
    /// 当前活跃账号的 key
    active: Mutex<Option<String>>,
}

impl AuthState {
    pub fn new() -> Self {
        migrate_legacy_if_needed();
        let mut accounts = load_accounts_from_disk();
        // 去重：迁移产生的 email-key 账号可能与正常登录的 sub-key 账号撞同一邮箱，
        // 同邮箱优先保留带 sub 的，删掉 email-key 残留，并记下重映射修正 active。
        let renames = dedup_accounts(&mut accounts);
        let active = load_active_from_disk()
            .map(|k| renames.get(&k).cloned().unwrap_or(k))
            .filter(|k| accounts.contains_key(k))
            .or_else(|| accounts.keys().next().cloned());
        save_active_to_disk(active.as_deref());
        Self {
            accounts: Mutex::new(accounts),
            active: Mutex::new(active),
        }
    }
}

/// 同一 email 若同时存在「带 sub」与「无 sub（迁移残留）」条目，删除无 sub 的那条，
/// 返回 被删 email-key -> 同邮箱 sub-key 的映射（供修正 active 指针）。
fn dedup_accounts(accounts: &mut HashMap<String, Account>) -> HashMap<String, String> {
    let mut sub_key_by_email: HashMap<String, String> = HashMap::new();
    for (k, a) in accounts.iter() {
        if a.user.sub.is_some() {
            sub_key_by_email.insert(a.user.email.clone(), k.clone());
        }
    }
    let to_remove: Vec<(String, String)> = accounts
        .iter()
        .filter_map(|(k, a)| {
            if a.user.sub.is_none() {
                sub_key_by_email
                    .get(&a.user.email)
                    .map(|sub_key| (k.clone(), sub_key.clone()))
            } else {
                None
            }
        })
        .collect();
    let mut renames = HashMap::new();
    for (old, new) in to_remove {
        remove_account_dir(&old);
        accounts.remove(&old);
        renames.insert(old, new);
    }
    renames
}

/// 账号存储 key：优先 Google sub（OpenID 唯一 ID），迁移的旧账号无 sub 时回退 email。
fn account_key(user: &UserInfo) -> String {
    user.sub.clone().unwrap_or_else(|| user.email.clone())
}

/// Returns a non-expired access_token, refreshing if necessary.
/// On refresh failure due to `invalid_grant` (refresh_token revoked / expired),
/// clears persisted auth and returns an error starting with "NEED_RELOGIN:"
/// so the frontend can route the user back to the login page.
pub async fn get_valid_access_token(state: &State<'_, AuthState>) -> Result<String, String> {
    // Snapshot active account's tokens; do NOT hold the std::sync::Mutex across .await.
    let Some(key) = state.active.lock().unwrap().clone() else {
        return Err("NEED_RELOGIN: not logged in".into());
    };
    let current = state
        .accounts
        .lock()
        .unwrap()
        .get(&key)
        .map(|a| a.tokens.clone());
    let Some(tokens) = current else {
        return Err("NEED_RELOGIN: not logged in".into());
    };

    if tokens.expires_at > now_ts() + 60 {
        return Ok(tokens.access_token);
    }

    match refresh_access_token(&tokens).await {
        Ok(new_tokens) => {
            save_account_tokens_to_disk(&key, &new_tokens);
            let access = new_tokens.access_token.clone();
            if let Some(acc) = state.accounts.lock().unwrap().get_mut(&key) {
                acc.tokens = new_tokens;
            }
            Ok(access)
        }
        Err(e) if e.starts_with("invalid_grant") => {
            // 该账号 refresh_token 失效：移除它，active 自动切到下一个（没有则回登录页）
            remove_account_dir(&key);
            let next = {
                let mut accounts = state.accounts.lock().unwrap();
                accounts.remove(&key);
                accounts.keys().next().cloned()
            };
            *state.active.lock().unwrap() = next.clone();
            save_active_to_disk(next.as_deref());
            Err(format!("NEED_RELOGIN: {}", e))
        }
        Err(e) => Err(format!("Token refresh failed: {}", e)),
    }
}

// ---- File helpers ----

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

fn accounts_dir() -> PathBuf {
    data_dir().join("accounts")
}

/// 把 key 变成安全的目录名（sub 是纯数字串；email 含 @/. 也合法，其余字符兜底替换）。
fn sanitize_key(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn account_dir(key: &str) -> PathBuf {
    accounts_dir().join(sanitize_key(key))
}

#[derive(Serialize, Deserialize)]
struct ActiveAccount {
    active: Option<String>,
}

/// 扫描 accounts/ 目录，按文件内容（user）重算 key 作为 HashMap key —— 与目录名解耦。
fn load_accounts_from_disk() -> HashMap<String, Account> {
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir(accounts_dir()) else {
        return map;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let tokens = std::fs::read_to_string(p.join("auth-tokens.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<AuthTokens>(&s).ok());
        let user = std::fs::read_to_string(p.join("auth-user.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<UserInfo>(&s).ok());
        if let (Some(tokens), Some(mut user)) = (tokens, user) {
            let key = account_key(&user);
            user.id = key.clone(); // 下发给前端的 opaque 隔离维度
            map.insert(key, Account { tokens, user });
        }
    }
    map
}

fn load_active_from_disk() -> Option<String> {
    std::fs::read_to_string(data_dir().join("active-account.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<ActiveAccount>(&s).ok())
        .and_then(|a| a.active)
}

fn save_active_to_disk(key: Option<&str>) {
    std::fs::create_dir_all(data_dir()).ok();
    let json = serde_json::to_string_pretty(&ActiveAccount {
        active: key.map(|s| s.to_string()),
    })
    .unwrap();
    std::fs::write(data_dir().join("active-account.json"), json).ok();
}

fn save_account_to_disk(key: &str, account: &Account) {
    let dir = account_dir(key);
    std::fs::create_dir_all(&dir).ok();
    std::fs::write(
        dir.join("auth-tokens.json"),
        serde_json::to_string_pretty(&account.tokens).unwrap(),
    )
    .ok();
    std::fs::write(
        dir.join("auth-user.json"),
        serde_json::to_string_pretty(&account.user).unwrap(),
    )
    .ok();
}

fn save_account_tokens_to_disk(key: &str, tokens: &AuthTokens) {
    let dir = account_dir(key);
    std::fs::create_dir_all(&dir).ok();
    std::fs::write(
        dir.join("auth-tokens.json"),
        serde_json::to_string_pretty(tokens).unwrap(),
    )
    .ok();
}

fn remove_account_dir(key: &str) {
    std::fs::remove_dir_all(account_dir(key)).ok();
}

/// 首启迁移：旧单账号明文 auth-tokens.json/auth-user.json → accounts/<key>/，并设为 active。
fn migrate_legacy_if_needed() {
    let legacy_tokens = data_dir().join("auth-tokens.json");
    let legacy_user = data_dir().join("auth-user.json");
    if !legacy_tokens.exists() || !legacy_user.exists() {
        return;
    }
    let tokens = std::fs::read_to_string(&legacy_tokens)
        .ok()
        .and_then(|s| serde_json::from_str::<AuthTokens>(&s).ok());
    let user = std::fs::read_to_string(&legacy_user)
        .ok()
        .and_then(|s| serde_json::from_str::<UserInfo>(&s).ok());
    if let (Some(tokens), Some(user)) = (tokens, user) {
        let key = account_key(&user);
        save_account_to_disk(&key, &Account { tokens, user });
        save_active_to_disk(Some(&key));
    }
    // 无论解析成功与否都删旧文件，避免重复迁移
    std::fs::remove_file(&legacy_tokens).ok();
    std::fs::remove_file(&legacy_user).ok();
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
    // 优先从编译时嵌入的内容读取（打包后可用）
    const EMBEDDED: &str = include_str!("../credentials/oauth.json");
    if !EMBEDDED.trim().is_empty() {
        let file: OAuthFile = serde_json::from_str(EMBEDDED)
            .map_err(|e| format!("Cannot parse embedded oauth.json: {}", e))?;
        return Ok(file.installed);
    }
    // 开发环境回退
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
    let Some(key) = state.active.lock().unwrap().clone() else {
        return Ok(None);
    };
    let Some(account) = state.accounts.lock().unwrap().get(&key).cloned() else {
        return Ok(None);
    };

    if account.tokens.expires_at > now_ts() + 60 {
        return Ok(Some(account.user));
    }
    match refresh_access_token(&account.tokens).await {
        Ok(new_tokens) => {
            save_account_tokens_to_disk(&key, &new_tokens);
            if let Some(acc) = state.accounts.lock().unwrap().get_mut(&key) {
                acc.tokens = new_tokens;
            }
            Ok(Some(account.user))
        }
        Err(_) => {
            // 当前 active 账号刷新失败：移除它，切到下一个并返回其用户（不强制验证）
            remove_account_dir(&key);
            let next = {
                let mut accounts = state.accounts.lock().unwrap();
                accounts.remove(&key);
                accounts.keys().next().cloned()
            };
            *state.active.lock().unwrap() = next.clone();
            save_active_to_disk(next.as_deref());
            let user = next.and_then(|k| {
                state.accounts.lock().unwrap().get(&k).map(|a| a.user.clone())
            });
            Ok(user)
        }
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
        https://www.googleapis.com/auth/presentations.readonly \
        https://www.googleapis.com/auth/androidpublisher \
        https://www.googleapis.com/auth/playdeveloperreporting";

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

    let mut user_info: UserInfo = client
        .get("https://www.googleapis.com/oauth2/v3/userinfo")
        .bearer_auth(&tokens.access_token)
        .send()
        .await
        .map_err(|e| format!("Userinfo failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Userinfo parse failed: {}", e))?;

    let key = account_key(&user_info);
    user_info.id = key.clone(); // 下发给前端的 opaque 隔离维度
    let account = Account {
        tokens,
        user: user_info.clone(),
    };
    {
        let mut accounts = state.accounts.lock().unwrap();
        // 去重：移除 email 相同但 key 不同的旧条目（迁移的 email-key 账号升级为 sub-key）
        let stale: Vec<String> = accounts
            .iter()
            .filter(|(k, a)| *k != &key && a.user.email == user_info.email)
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale {
            remove_account_dir(&k);
            accounts.remove(&k);
        }
        accounts.insert(key.clone(), account.clone());
    }
    save_account_to_disk(&key, &account);
    *state.active.lock().unwrap() = Some(key.clone());
    save_active_to_disk(Some(&key));

    Ok(user_info)
}

/// 登出指定账号（缺省登出当前 active）。返回登出后新的 active 用户（无则 None）。
#[tauri::command]
pub fn logout(
    state: State<'_, AuthState>,
    account_id: Option<String>,
) -> Result<Option<UserInfo>, String> {
    let key = account_id.or_else(|| state.active.lock().unwrap().clone());
    let Some(key) = key else {
        return Ok(None);
    };

    let was_active = state.active.lock().unwrap().as_deref() == Some(key.as_str());
    remove_account_dir(&key);
    let next = {
        let mut accounts = state.accounts.lock().unwrap();
        accounts.remove(&key);
        if was_active {
            accounts.keys().next().cloned()
        } else {
            None
        }
    };
    if was_active {
        *state.active.lock().unwrap() = next.clone();
        save_active_to_disk(next.as_deref());
    }

    let active_key = state.active.lock().unwrap().clone();
    let user = active_key
        .and_then(|k| state.accounts.lock().unwrap().get(&k).map(|a| a.user.clone()));
    Ok(user)
}

/// 列出全部已登录账号，供前端下拉展示（带 active 标记）。
#[tauri::command]
pub fn list_accounts(state: State<'_, AuthState>) -> Vec<AccountInfo> {
    let active = state.active.lock().unwrap().clone();
    let accounts = state.accounts.lock().unwrap();
    let mut list: Vec<AccountInfo> = accounts
        .iter()
        .map(|(k, a)| AccountInfo {
            id: k.clone(),
            email: a.user.email.clone(),
            name: a.user.name.clone(),
            picture: a.user.picture.clone(),
            active: active.as_deref() == Some(k.as_str()),
        })
        .collect();
    list.sort_by(|a, b| a.email.cmp(&b.email));
    list
}

/// 切换 active 账号（仅换内存指针 + 落盘，不重新走 OAuth）。返回新的当前用户。
#[tauri::command]
pub fn switch_account(
    state: State<'_, AuthState>,
    account_id: String,
) -> Result<UserInfo, String> {
    let user = state
        .accounts
        .lock()
        .unwrap()
        .get(&account_id)
        .map(|a| a.user.clone());
    let Some(user) = user else {
        return Err(format!("account not found: {}", account_id));
    };
    *state.active.lock().unwrap() = Some(account_id.clone());
    save_active_to_disk(Some(&account_id));
    Ok(user)
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

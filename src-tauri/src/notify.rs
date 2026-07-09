use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::feedback::resolve_bot_token;

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

fn notify_config_path() -> PathBuf {
    data_dir().join("notify.json")
}

// 独立于 feedback 的通知目标（决策点 3b：与维护者反馈私聊分开）。
// bot_token 留空则复用 feedback 的 bot token（同一个 bot，不同 chat_id）。
#[derive(Deserialize, Serialize, Default, Clone)]
pub struct NotifyConfig {
    #[serde(default)]
    pub chat_id: String,
    #[serde(default)]
    pub bot_token: String,
}

fn load_notify_config() -> Option<NotifyConfig> {
    let content = std::fs::read_to_string(notify_config_path()).ok()?;
    serde_json::from_str(&content).ok()
}

fn resolve_notify_target() -> Option<(String, String)> {
    let cfg = load_notify_config()?;
    if cfg.chat_id.trim().is_empty() {
        return None;
    }
    let token = if !cfg.bot_token.trim().is_empty() {
        cfg.bot_token
    } else {
        resolve_bot_token()?
    };
    Some((token, cfg.chat_id))
}

#[tauri::command]
pub fn is_notify_configured() -> bool {
    resolve_notify_target().is_some()
}

#[tauri::command]
pub fn get_notify_config() -> NotifyConfig {
    load_notify_config().unwrap_or_default()
}

#[tauri::command]
pub fn save_notify_config(config: NotifyConfig) -> Result<(), String> {
    std::fs::create_dir_all(data_dir()).map_err(|e| format!("Create data dir failed: {}", e))?;
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Serialize notify config failed: {}", e))?;
    std::fs::write(notify_config_path(), json)
        .map_err(|e| format!("Write notify config failed: {}", e))
}

#[tauri::command]
pub async fn send_telegram_message(text: String) -> Result<(), String> {
    let (token, chat_id) = resolve_notify_target()
        .ok_or("通知未配置：请先在「定时通知」里填写 Chat ID".to_string())?;
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);

    let resp = reqwest::Client::new()
        .post(&url)
        .form(&[
            ("chat_id", chat_id.as_str()),
            ("text", text.as_str()),
            ("parse_mode", "HTML"),
        ])
        .send()
        .await
        .map_err(|e| format!("Telegram request failed: {}", e))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Read Telegram response failed: {}", e))?;
    if !status.is_success() {
        return Err(format!("Telegram {}: {}", status, body));
    }
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Parse Telegram response failed: {} | body: {}", e, body))?;
    if !json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Err(format!("Telegram returned not-ok: {}", body));
    }
    Ok(())
}

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use zip::write::FileOptions;
use zip::ZipWriter;

use crate::manifest::{read_manifest, GenerateManifest};

// Compile-time configuration (preferred for release builds).
const BOT_TOKEN_COMPILED: Option<&str> = option_env!("TELEGRAM_BOT_TOKEN");
const CHAT_ID_COMPILED: Option<&str> = option_env!("TELEGRAM_CHAT_ID");

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

fn pending_dir() -> PathBuf {
    data_dir().join("feedback_pending")
}

fn sent_dir() -> PathBuf {
    data_dir().join("feedback_sent")
}

#[derive(Deserialize)]
struct TelegramConfig {
    bot_token: String,
    chat_id: String,
}

/// Resolve bot token + chat_id.
/// Priority: compile-time env vars → ~/.tester-app/telegram.json.
/// Returns (token, chat_id) or None if neither is configured.
fn resolve_telegram_config() -> Option<(String, String)> {
    if let (Some(t), Some(c)) = (BOT_TOKEN_COMPILED, CHAT_ID_COMPILED) {
        if !t.is_empty() && !c.is_empty() {
            return Some((t.to_string(), c.to_string()));
        }
    }
    let path = data_dir().join("telegram.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let cfg: TelegramConfig = serde_json::from_str(&content).ok()?;
    if cfg.bot_token.is_empty() || cfg.chat_id.is_empty() {
        return None;
    }
    Some((cfg.bot_token, cfg.chat_id))
}

#[tauri::command]
pub fn is_feedback_configured() -> bool {
    resolve_telegram_config().is_some()
}

#[derive(Deserialize)]
pub struct FeedbackInput {
    pub ai_drive_id: String,
    pub ai_html_path: String,
    pub human_html_path: String,
    pub report_path: String,
    pub issue_type: String, // "missing_case" | "wrong_expected" | "wrong_module" | "other"
    pub note: String,
    pub ai_sheet_name: Option<String>,
    pub ai_tab_name: Option<String>,
    pub human_sheet_name: Option<String>,
    pub human_tab_name: Option<String>,
}

#[derive(Serialize)]
struct FeedbackMeta {
    submitted_at: u64,
    user_email: Option<String>,
    user_name: Option<String>,
    issue_type: String,
    note: String,
    ai_drive_id: String,
    ai_sheet_name: Option<String>,
    ai_tab_name: Option<String>,
    human_sheet_name: Option<String>,
    human_tab_name: Option<String>,
    manifest: Option<GenerateManifest>,
}

#[derive(Deserialize)]
struct StoredUser {
    email: String,
    name: String,
}

fn load_user() -> Option<StoredUser> {
    let path = data_dir().join("auth-user.json");
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn add_file_to_zip<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    name: &str,
    path: &Path,
) -> Result<(), String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("Read {} failed: {}", path.display(), e))?;
    zip.start_file(name, FileOptions::default())
        .map_err(|e| format!("Start zip entry {} failed: {}", name, e))?;
    zip.write_all(&bytes)
        .map_err(|e| format!("Write zip entry {} failed: {}", name, e))?;
    Ok(())
}

fn build_feedback_zip(
    input: &FeedbackInput,
    manifest: Option<&GenerateManifest>,
    user: Option<&StoredUser>,
) -> Result<(PathBuf, Vec<u8>), String> {
    std::fs::create_dir_all(pending_dir())
        .map_err(|e| format!("Create pending dir failed: {}", e))?;

    let ts = now_secs();
    let drive_short: String = input.ai_drive_id.chars().take(8).collect();
    let zip_name = format!("feedback_{}_{}.zip", ts, drive_short);
    let zip_path = pending_dir().join(&zip_name);

    // Build zip in memory first so we can both write to disk and send the bytes.
    let mut buf: Vec<u8> = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buf);
        let mut zip = ZipWriter::new(cursor);

        // Required: the two compared HTMLs and the diff report.
        add_file_to_zip(
            &mut zip,
            "ai.html",
            Path::new(&input.ai_html_path),
        )?;
        add_file_to_zip(
            &mut zip,
            "human.html",
            Path::new(&input.human_html_path),
        )?;
        add_file_to_zip(
            &mut zip,
            "report.html",
            Path::new(&input.report_path),
        )?;

        // Optional: source files from manifest.
        if let Some(m) = manifest {
            if let Some(csv) = &m.source_csv_path {
                let p = Path::new(csv);
                if p.is_file() {
                    let name = p
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "source.csv".into());
                    add_file_to_zip(&mut zip, &format!("sources/{}", name), p).ok();
                }
            }
            for pptx in &m.pptx_paths {
                let p = Path::new(pptx);
                if p.is_file() {
                    let name = p
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "source.pptx".into());
                    add_file_to_zip(&mut zip, &format!("sources/{}", name), p).ok();
                }
            }
        }

        // meta.json
        let meta = FeedbackMeta {
            submitted_at: ts,
            user_email: user.map(|u| u.email.clone()),
            user_name: user.map(|u| u.name.clone()),
            issue_type: input.issue_type.clone(),
            note: input.note.clone(),
            ai_drive_id: input.ai_drive_id.clone(),
            ai_sheet_name: input.ai_sheet_name.clone(),
            ai_tab_name: input.ai_tab_name.clone(),
            human_sheet_name: input.human_sheet_name.clone(),
            human_tab_name: input.human_tab_name.clone(),
            manifest: manifest.cloned(),
        };
        let meta_json = serde_json::to_vec_pretty(&meta)
            .map_err(|e| format!("Serialize meta failed: {}", e))?;
        zip.start_file("meta.json", FileOptions::default())
            .map_err(|e| format!("Start meta zip entry failed: {}", e))?;
        zip.write_all(&meta_json)
            .map_err(|e| format!("Write meta zip entry failed: {}", e))?;

        zip.finish()
            .map_err(|e| format!("Finalize zip failed: {}", e))?;
    }

    std::fs::write(&zip_path, &buf)
        .map_err(|e| format!("Write zip to disk failed: {}", e))?;

    Ok((zip_path, buf))
}

fn build_caption(
    input: &FeedbackInput,
    manifest: Option<&GenerateManifest>,
    user: Option<&StoredUser>,
) -> String {
    let skill_ver = manifest
        .and_then(|m| m.skill_version.clone())
        .unwrap_or_else(|| "unknown".into());
    let issue = match input.issue_type.as_str() {
        "missing_case" => "漏用例",
        "wrong_expected" => "预期错",
        "wrong_module" => "模块分类错",
        "other" => "其他",
        _ => &input.issue_type,
    };
    let user_part = user
        .map(|u| format!("{} <{}>", u.name, u.email))
        .unwrap_or_else(|| "unknown".into());
    let note_part = if input.note.trim().is_empty() {
        "".into()
    } else {
        format!("\n备注: {}", input.note.trim())
    };
    let sheet_part = match (&input.ai_sheet_name, &input.ai_tab_name) {
        (Some(s), Some(t)) => format!("\nAI: {} › {}", s, t),
        _ => "".into(),
    };
    let human_part = match (&input.human_sheet_name, &input.human_tab_name) {
        (Some(s), Some(t)) => format!("\nHuman: {} › {}", s, t),
        _ => "".into(),
    };
    let has_sources = manifest
        .map(|m| m.source_csv_path.is_some() || !m.pptx_paths.is_empty())
        .unwrap_or(false);
    let sources_tag = if has_sources { " #with_sources" } else { " #no_sources" };
    format!(
        "#sample #{}{}\nskill: {}\n用户: {}{}{}{}",
        input.issue_type, sources_tag, skill_ver, user_part, sheet_part, human_part, note_part
    )
    + &format!("\nissue: {}", issue)
}

async fn upload_to_telegram(
    token: &str,
    chat_id: &str,
    zip_bytes: Vec<u8>,
    zip_filename: &str,
    caption: &str,
) -> Result<(), String> {
    let url = format!("https://api.telegram.org/bot{}/sendDocument", token);

    let part = reqwest::multipart::Part::bytes(zip_bytes)
        .file_name(zip_filename.to_string())
        .mime_str("application/zip")
        .map_err(|e| format!("Bad mime: {}", e))?;
    let form = reqwest::multipart::Form::new()
        .text("chat_id", chat_id.to_string())
        .text("caption", caption.to_string())
        .part("document", part);

    let resp = reqwest::Client::new()
        .post(&url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Telegram request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Telegram {}: {}", status, body));
    }
    // Telegram returns JSON like {"ok":true,...}; verify.
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Read Telegram response failed: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Parse Telegram response failed: {} | body: {}", e, body))?;
    if !json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Err(format!("Telegram returned not-ok: {}", body));
    }
    Ok(())
}

fn move_to_sent(zip_path: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(sent_dir())
        .map_err(|e| format!("Create sent dir failed: {}", e))?;
    let dest = sent_dir().join(
        zip_path
            .file_name()
            .ok_or("Zip path has no filename")?,
    );
    std::fs::rename(zip_path, &dest)
        .map_err(|e| format!("Move to sent dir failed: {}", e))?;
    Ok(dest)
}

#[derive(Serialize)]
pub struct FeedbackResult {
    pub ok: bool,
    pub zip_path: String,
    pub had_sources: bool,
    pub message: String,
}

#[tauri::command]
pub async fn send_feedback(input: FeedbackInput) -> Result<FeedbackResult, String> {
    let (token, chat_id) = resolve_telegram_config()
        .ok_or("Telegram feedback not configured (no TELEGRAM_BOT_TOKEN/CHAT_ID at build time and no ~/.tester-app/telegram.json)")?;

    let manifest = read_manifest(&input.ai_drive_id);
    let user = load_user();
    let had_sources = manifest
        .as_ref()
        .map(|m| m.source_csv_path.is_some() || !m.pptx_paths.is_empty())
        .unwrap_or(false);

    let (zip_path, zip_bytes) = build_feedback_zip(&input, manifest.as_ref(), user.as_ref())?;
    let zip_filename = zip_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "feedback.zip".into());
    let caption = build_caption(&input, manifest.as_ref(), user.as_ref());

    match upload_to_telegram(&token, &chat_id, zip_bytes, &zip_filename, &caption).await {
        Ok(()) => {
            let dest = move_to_sent(&zip_path)?;
            Ok(FeedbackResult {
                ok: true,
                zip_path: dest.to_string_lossy().to_string(),
                had_sources,
                message: "Feedback sent.".into(),
            })
        }
        Err(e) => {
            // Leave zip in pending/, surface the error.
            Err(format!(
                "Upload failed (kept in pending: {}): {}",
                zip_path.display(),
                e
            ))
        }
    }
}

#[derive(Serialize)]
pub struct RetryResult {
    pub retried: usize,
    pub succeeded: usize,
    pub failures: Vec<String>,
}

#[tauri::command]
pub async fn retry_pending_feedback() -> Result<RetryResult, String> {
    let (token, chat_id) = resolve_telegram_config()
        .ok_or("Telegram feedback not configured")?;

    let dir = pending_dir();
    if !dir.is_dir() {
        return Ok(RetryResult { retried: 0, succeeded: 0, failures: vec![] });
    }

    let mut retried = 0usize;
    let mut succeeded = 0usize;
    let mut failures: Vec<String> = Vec::new();

    let entries = std::fs::read_dir(&dir)
        .map_err(|e| format!("Read pending dir failed: {}", e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("zip") {
            continue;
        }
        retried += 1;
        let filename = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "feedback.zip".into());
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("{}: read failed: {}", filename, e));
                continue;
            }
        };
        let caption = format!("#retry\nresending: {}", filename);
        match upload_to_telegram(&token, &chat_id, bytes, &filename, &caption).await {
            Ok(()) => {
                if move_to_sent(&path).is_ok() {
                    succeeded += 1;
                }
            }
            Err(e) => failures.push(format!("{}: {}", filename, e)),
        }
    }
    Ok(RetryResult { retried, succeeded, failures })
}

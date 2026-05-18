use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

fn find_claude() -> Option<String> {
    let candidates: Vec<PathBuf> = if cfg!(windows) {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        let userprofile = std::env::var("USERPROFILE").unwrap_or_default();
        vec![
            PathBuf::from(&appdata).join("npm").join("claude.cmd"),
            PathBuf::from(&userprofile).join(".npm-global").join("claude.cmd"),
            PathBuf::from(&userprofile).join("AppData").join("Local").join("fnm_multishells").join("claude.cmd"),
        ]
    } else {
        let home = std::env::var("HOME").unwrap_or_default();
        vec![
            PathBuf::from("/usr/local/bin/claude"),
            PathBuf::from(&home).join(".npm-global").join("bin").join("claude"),
        ]
    };

    for c in &candidates {
        if c.exists() {
            return Some(c.to_string_lossy().to_string());
        }
    }

    if cfg!(windows) {
        let output = std::process::Command::new("cmd")
            .args(["/c", "where", "claude"])
            .output()
            .ok()?;
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout);
            if let Some(first) = path.lines().next() {
                let p = first.trim();
                if !p.is_empty() {
                    return Some(p.to_string());
                }
            }
        }
    } else {
        let output = std::process::Command::new("which")
            .arg("claude")
            .output()
            .ok()?;
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout);
            let p = path.trim();
            if !p.is_empty() {
                return Some(p.to_string());
            }
        }
    }

    None
}

#[derive(Serialize, Clone)]
pub struct ClaudeLogEvent {
    pub text: String,
    pub kind: String, // "system", "text", "tool", "result", "error", "info"
    pub done: bool,
}

pub struct ClaudeState {
    pub session_id: Mutex<Option<String>>,
    pub running: Mutex<bool>,
}

impl ClaudeState {
    pub fn new() -> Self {
        Self {
            session_id: Mutex::new(None),
            running: Mutex::new(false),
        }
    }
}

fn emit_log(app: &AppHandle, text: &str, kind: &str, done: bool) {
    app.emit(
        "claude-log",
        ClaudeLogEvent {
            text: text.to_string(),
            kind: kind.to_string(),
            done,
        },
    )
    .ok();
}

async fn run_claude_and_stream(
    args: Vec<String>,
    app: AppHandle,
    state: &State<'_, ClaudeState>,
) -> Result<(), String> {
    {
        let mut running = state.running.lock().unwrap();
        if *running {
            return Err("Claude is already running".into());
        }
        *running = true;
    }

    let claude_path = find_claude().ok_or("Claude CLI not found. Please install it: npm install -g @anthropic-ai/claude-code")?;

    let mut cmd = if cfg!(windows) && claude_path.ends_with(".cmd") {
        let mut c = Command::new("cmd");
        c.arg("/c").arg(&claude_path).args(&args);
        c
    } else {
        let mut c = Command::new(&claude_path);
        c.args(&args);
        c
    };
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(token) = load_claude_token() {
        cmd.env("CLAUDE_CODE_SESSION_ACCESS_TOKEN", &token);
    }

    emit_log(&app, &format!("$ claude {}", args.join(" ")), "info", false);

    let mut child = cmd
        .spawn()
        .map_err(|e| {
            *state.running.lock().unwrap() = false;
            format!("Failed to spawn claude: {}", e)
        })?;

    let stdout = child.stdout.take().ok_or("No stdout")?;
    let stderr = child.stderr.take().ok_or("No stderr")?;

    let app_out = app.clone();
    let session_id_for_parse: std::sync::Arc<Mutex<Option<String>>> =
        std::sync::Arc::new(Mutex::new(None));
    let sid_clone = session_id_for_parse.clone();

    let stdout_task = tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            // Try to parse as JSON (stream-json format)
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

                match event_type {
                    "system" => {
                        if let Some(sid) = val.get("session_id").and_then(|v| v.as_str()) {
                            *sid_clone.lock().unwrap() = Some(sid.to_string());
                            emit_log(&app_out, &format!("Session: {}", sid), "system", false);
                        }
                    }
                    "assistant" => {
                        if let Some(content) = val
                            .get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_array())
                        {
                            for block in content {
                                let block_type =
                                    block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                match block_type {
                                    "text" => {
                                        if let Some(text) =
                                            block.get("text").and_then(|v| v.as_str())
                                        {
                                            emit_log(&app_out, text, "text", false);
                                        }
                                    }
                                    "tool_use" => {
                                        let name = block
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown");
                                        emit_log(
                                            &app_out,
                                            &format!("[Tool: {}]", name),
                                            "tool",
                                            false,
                                        );
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    "result" => {
                        if let Some(result) = val.get("result").and_then(|v| v.as_str()) {
                            emit_log(&app_out, result, "result", false);
                        }
                        let cost = val
                            .get("cost_usd")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        let duration = val
                            .get("duration_ms")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        emit_log(
                            &app_out,
                            &format!("Cost: ${:.4} | Duration: {:.1}s", cost, duration / 1000.0),
                            "info",
                            false,
                        );
                    }
                    _ => {
                        // tool_result, etc — show as info
                        if let Some(content) = val.get("content").and_then(|v| v.as_str()) {
                            if !content.is_empty() {
                                emit_log(&app_out, content, "tool", false);
                            }
                        }
                    }
                }
            } else {
                // Not JSON, just show as text
                if !line.trim().is_empty() {
                    emit_log(&app_out, &line, "text", false);
                }
            }
        }
    });

    let app_err = app.clone();
    let stderr_task = tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.trim().is_empty() {
                emit_log(&app_err, &line, "error", false);
            }
        }
    });

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Claude process error: {}", e))?;

    stdout_task.await.ok();
    stderr_task.await.ok();

    // Save session_id
    if let Some(sid) = session_id_for_parse.lock().unwrap().take() {
        *state.session_id.lock().unwrap() = Some(sid);
    }

    *state.running.lock().unwrap() = false;

    let success = status.success();
    emit_log(
        &app,
        if success {
            "Claude finished. You can type below to continue the conversation."
        } else {
            "Claude exited with error."
        },
        "info",
        true,
    );

    if success {
        Ok(())
    } else {
        Err(format!(
            "Claude exited with code {}",
            status.code().unwrap_or(-1)
        ))
    }
}

#[derive(Deserialize)]
pub struct PageSelection {
    pub name: String,
    pub pages: Vec<usize>,
}

#[tauri::command]
pub async fn run_claude_task(
    csv_path: String,
    pptx_paths: Vec<String>,
    page_selections: Vec<PageSelection>,
    app: AppHandle,
    state: State<'_, ClaudeState>,
) -> Result<(), String> {
    *state.session_id.lock().unwrap() = None;

    let mut args = vec![
        "--print".to_string(),
        "--verbose".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
    ];

    args.push("--file".to_string());
    args.push(csv_path.clone());
    for p in &pptx_paths {
        args.push("--file".to_string());
        args.push(p.clone());
    }

    let mut prompt = "/test-case-generator".to_string();
    for sel in &page_selections {
        let pages_str: Vec<String> = sel.pages.iter().map(|p| p.to_string()).collect();
        prompt.push_str(&format!("\n{}: pages {}", sel.name, pages_str.join(", ")));
    }

    args.push(prompt);

    run_claude_and_stream(args, app, &state).await
}

#[tauri::command]
pub async fn send_claude_input(
    input: String,
    app: AppHandle,
    state: State<'_, ClaudeState>,
) -> Result<(), String> {
    let session_id = state
        .session_id
        .lock()
        .unwrap()
        .clone()
        .ok_or("No active Claude session")?;

    let args = vec![
        "--print".to_string(),
        "--verbose".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--resume".to_string(),
        session_id,
        input,
    ];

    run_claude_and_stream(args, app, &state).await
}

#[tauri::command]
pub async fn get_claude_status(
    state: State<'_, ClaudeState>,
) -> Result<(bool, bool), String> {
    let running = *state.running.lock().unwrap();
    let has_session = state.session_id.lock().unwrap().is_some();
    Ok((running, has_session))
}

fn load_claude_token() -> Option<String> {
    let cred_path = dirs::home_dir()?.join(".claude").join(".credentials.json");
    let content = std::fs::read_to_string(cred_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let token = json
        .get("claudeAiOauth")
        .and_then(|o| o.get("accessToken"))
        .and_then(|v| v.as_str())?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

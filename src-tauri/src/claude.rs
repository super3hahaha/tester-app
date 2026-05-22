use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

pub fn find_claude() -> Option<String> {
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
    pub kind: String, // "system", "text", "tool", "tool_done", "result", "error", "info"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub done: bool,
}

pub struct ClaudeState {
    pub session_id: Mutex<Option<String>>,
    pub running: Mutex<bool>,
    pub child_pid: Mutex<Option<u32>>,
}

impl ClaudeState {
    pub fn new() -> Self {
        Self {
            session_id: Mutex::new(None),
            running: Mutex::new(false),
            child_pid: Mutex::new(None),
        }
    }
}

fn format_tool_preview(name: &str, input: Option<&serde_json::Value>) -> String {
    fn one_line(s: &str, max: usize) -> String {
        let cleaned: String = s.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
        let trimmed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.chars().count() > max {
            let cut: String = trimmed.chars().take(max).collect();
            format!("{}…", cut)
        } else {
            trimmed
        }
    }
    let get_str = |key: &str| -> Option<String> {
        input
            .and_then(|i| i.get(key))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    let detail: Option<String> = match name {
        "Bash" => get_str("command"),
        "Read" | "Write" | "NotebookEdit" => get_str("file_path"),
        "Edit" => {
            let path = get_str("file_path").unwrap_or_default();
            let old = get_str("old_string").unwrap_or_default();
            let snippet = one_line(&old, 40);
            Some(format!("{}  «{}»", path, snippet))
        }
        "Glob" => {
            let pat = get_str("pattern").unwrap_or_default();
            let path = get_str("path").unwrap_or_default();
            if path.is_empty() {
                Some(pat)
            } else {
                Some(format!("{}  in {}", pat, path))
            }
        }
        "Grep" => {
            let pat = get_str("pattern").unwrap_or_default();
            let path = get_str("path").unwrap_or_default();
            if path.is_empty() {
                Some(pat)
            } else {
                Some(format!("{}  in {}", pat, path))
            }
        }
        "WebFetch" => get_str("url"),
        "WebSearch" => get_str("query"),
        "Skill" => get_str("skill").or_else(|| get_str("name")),
        "Task" | "Agent" => get_str("description").or_else(|| get_str("prompt")),
        "TodoWrite" => {
            let count = input
                .and_then(|i| i.get("todos"))
                .and_then(|t| t.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Some(format!("({} todos)", count))
        }
        _ => None,
    };
    match detail {
        Some(d) if !d.is_empty() => one_line(&d, 200),
        _ => String::new(),
    }
}

fn emit_log(app: &AppHandle, text: &str, kind: &str, done: bool) {
    app.emit(
        "claude-log",
        ClaudeLogEvent {
            text: text.to_string(),
            kind: kind.to_string(),
            tool: None,
            duration_ms: None,
            done,
        },
    )
    .ok();
}

fn emit_tool(app: &AppHandle, name: &str, detail: &str) {
    app.emit(
        "claude-log",
        ClaudeLogEvent {
            text: detail.to_string(),
            kind: "tool".into(),
            tool: Some(name.to_string()),
            duration_ms: None,
            done: false,
        },
    )
    .ok();
}

fn emit_tool_done(app: &AppHandle, name: &str, output: &str, duration_ms: u64) {
    app.emit(
        "claude-log",
        ClaudeLogEvent {
            text: output.to_string(),
            kind: "tool_done".into(),
            tool: Some(name.to_string()),
            duration_ms: Some(duration_ms),
            done: false,
        },
    )
    .ok();
}

fn extract_tool_result(block: &serde_json::Value) -> String {
    if let Some(s) = block.get("content").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(arr) = block.get("content").and_then(|v| v.as_array()) {
        let mut out = String::new();
        for item in arr {
            if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
        return out;
    }
    String::new()
}

async fn run_claude_and_stream(
    args: Vec<String>,
    stdin_input: Option<String>,
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

    let mut cmd = Command::new(&claude_path);
    cmd.args(&args);
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin_input.is_some() {
        cmd.stdin(Stdio::piped());
    }

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

    *state.child_pid.lock().unwrap() = child.id();

    if let Some(input) = stdin_input {
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(input.as_bytes()).await {
                emit_log(&app, &format!("Failed to write stdin: {}", e), "error", false);
            }
            drop(stdin);
        }
    }

    let stdout = child.stdout.take().ok_or("No stdout")?;
    let stderr = child.stderr.take().ok_or("No stderr")?;

    let app_out = app.clone();
    let session_id_for_parse: std::sync::Arc<Mutex<Option<String>>> =
        std::sync::Arc::new(Mutex::new(None));
    let sid_clone = session_id_for_parse.clone();

    let stdout_task = tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut tool_starts: HashMap<String, (Instant, String)> = HashMap::new();
        while let Ok(Some(line)) = lines.next_line().await {
            // Try to parse as JSON (stream-json format)
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

                match event_type {
                    "system" => {
                        if let Some(sid) = val.get("session_id").and_then(|v| v.as_str()) {
                            let mut guard = sid_clone.lock().unwrap();
                            let is_new = guard.as_deref() != Some(sid);
                            if is_new {
                                *guard = Some(sid.to_string());
                                drop(guard);
                                emit_log(&app_out, &format!("Session: {}", sid), "system", false);
                                if let Some(model) =
                                    val.get("model").and_then(|v| v.as_str())
                                {
                                    emit_log(
                                        &app_out,
                                        &format!("Model: {}", model),
                                        "system",
                                        false,
                                    );
                                }
                            }
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
                                        let input = block.get("input");
                                        let detail = format_tool_preview(name, input);
                                        if let Some(id) = block
                                            .get("id")
                                            .and_then(|v| v.as_str())
                                        {
                                            tool_starts.insert(
                                                id.to_string(),
                                                (Instant::now(), name.to_string()),
                                            );
                                        }
                                        emit_tool(&app_out, name, &detail);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    "user" => {
                        if let Some(content) = val
                            .get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_array())
                        {
                            for block in content {
                                if block.get("type").and_then(|v| v.as_str())
                                    != Some("tool_result")
                                {
                                    continue;
                                }
                                let id = match block
                                    .get("tool_use_id")
                                    .and_then(|v| v.as_str())
                                {
                                    Some(s) => s,
                                    None => continue,
                                };
                                if let Some((start, name)) = tool_starts.remove(id) {
                                    let duration_ms = start.elapsed().as_millis() as u64;
                                    let output = extract_tool_result(block);
                                    emit_tool_done(&app_out, &name, &output, duration_ms);
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
    *state.child_pid.lock().unwrap() = None;

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
    model: Option<String>,
    app: AppHandle,
    state: State<'_, ClaudeState>,
) -> Result<(), String> {
    *state.session_id.lock().unwrap() = None;

    let mut args = vec![
        "--print".to_string(),
        "--verbose".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--permission-mode".to_string(),
        "bypassPermissions".to_string(),
    ];
    if let Some(m) = model.as_ref().filter(|s| !s.is_empty()) {
        args.push("--model".to_string());
        args.push(m.clone());
    }

    let mut dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if let Some(parent) = std::path::Path::new(&csv_path).parent() {
        dirs.insert(parent.to_string_lossy().to_string());
    }
    for p in &pptx_paths {
        if let Some(parent) = std::path::Path::new(p).parent() {
            dirs.insert(parent.to_string_lossy().to_string());
        }
    }
    for d in &dirs {
        args.push("--add-dir".to_string());
        args.push(d.clone());
    }

    let mut prompt = String::from("/test-case-generator\n\n");
    prompt.push_str(&format!("CSV (existing test cases): {}\n", csv_path));
    for p in &pptx_paths {
        prompt.push_str(&format!("PPTX (new requirements): {}\n", p));
    }
    for sel in &page_selections {
        let pages_str: Vec<String> = sel.pages.iter().map(|p| p.to_string()).collect();
        prompt.push_str(&format!("{}: pages {}\n", sel.name, pages_str.join(", ")));
    }

    run_claude_and_stream(args, Some(prompt), app, &state).await
}

#[tauri::command]
pub async fn send_claude_input(
    input: String,
    model: Option<String>,
    app: AppHandle,
    state: State<'_, ClaudeState>,
) -> Result<(), String> {
    let session_id = state
        .session_id
        .lock()
        .unwrap()
        .clone()
        .ok_or("No active Claude session")?;

    let mut args = vec![
        "--print".to_string(),
        "--verbose".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--permission-mode".to_string(),
        "bypassPermissions".to_string(),
        "--resume".to_string(),
        session_id,
    ];
    if let Some(m) = model.as_ref().filter(|s| !s.is_empty()) {
        args.push("--model".to_string());
        args.push(m.clone());
    }

    run_claude_and_stream(args, Some(input), app, &state).await
}

#[tauri::command]
pub async fn stop_claude(state: State<'_, ClaudeState>) -> Result<(), String> {
    let pid = state.child_pid.lock().unwrap().take();
    let pid = match pid {
        Some(p) => p,
        None => return Err("No Claude process is running".into()),
    };

    let result = if cfg!(windows) {
        // /T also terminates child processes spawned by claude.cmd (node.exe etc.)
        std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    } else {
        std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    };

    match result {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("Stop command exited with code {}", s.code().unwrap_or(-1))),
        Err(e) => Err(format!("Failed to stop Claude: {}", e)),
    }
}

#[tauri::command]
pub async fn get_claude_status(
    state: State<'_, ClaudeState>,
) -> Result<(bool, bool), String> {
    let running = *state.running.lock().unwrap();
    let has_session = state.session_id.lock().unwrap().is_some();
    Ok((running, has_session))
}

#[derive(Serialize)]
pub struct ClaudeAccountInfo {
    pub installed: bool,
    pub cli_path: Option<String>,
    pub logged_in: bool,
    pub email: Option<String>,
    pub subscription: Option<String>,
}

fn extract_email_from_jwt(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    // JWT payload is base64url without padding
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        parts[1],
    )
    .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    // Anthropic JWTs may carry email under different keys; try a few.
    for key in ["email", "user_email", "https://api.anthropic.com/email"] {
        if let Some(s) = json.get(key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

#[tauri::command]
pub fn get_claude_account() -> ClaudeAccountInfo {
    let cli_path = find_claude();
    let installed = cli_path.is_some();

    let mut logged_in = false;
    let mut email: Option<String> = None;
    let mut subscription: Option<String> = None;

    if let Some(home) = dirs::home_dir() {
        let cred = home.join(".claude").join(".credentials.json");
        if let Ok(content) = std::fs::read_to_string(&cred) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let oauth = json.get("claudeAiOauth");
                let token = oauth
                    .and_then(|o| o.get("accessToken"))
                    .and_then(|v| v.as_str());
                logged_in = token.map_or(false, |t| !t.is_empty());

                email = oauth
                    .and_then(|o| o.get("email"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(|| token.and_then(extract_email_from_jwt));

                subscription = oauth
                    .and_then(|o| o.get("subscriptionType"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
        }
    }

    ClaudeAccountInfo {
        installed,
        cli_path,
        logged_in,
        email,
        subscription,
    }
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

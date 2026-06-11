use serde::Serialize;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::claude::{find_claude, load_claude_token};

/// Reply task has its own log channel + running flag, independent of ClaudeState
/// (GeneratePage) so the two never pollute each other's session/running state —
/// same reasoning as CompareState. See decisions.md.
pub struct ReplyState {
    pub running: Mutex<bool>,
    pub child_pid: Mutex<Option<u32>>,
}

impl ReplyState {
    pub fn new() -> Self {
        Self {
            running: Mutex::new(false),
            child_pid: Mutex::new(None),
        }
    }
}

#[derive(Serialize, Clone)]
struct ReplyLogEvent {
    text: String,
    kind: String, // "info", "text", "tool", "result", "error"
    done: bool,
}

/// What `run_reply_skill` returns: the skill's candidates JSON plus the actual
/// token/cost usage reported by the CLI's `result` event (None if not parseable).
#[derive(Serialize)]
pub struct ReplyResult {
    output: serde_json::Value,
    usage: Option<serde_json::Value>, // { input_tokens, output_tokens, cache_*, total_cost_usd }
}

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

/// Find the skill's output file for a given input stem. Tolerates either naming
/// convention (`<stem>.candidates.json` or `<stem>.json.candidates.json`): match
/// any file starting with `<stem>` and ending with `.candidates.json`.
fn find_candidates_file(dir: &std::path::Path, stem: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(stem) && name.ends_with(".candidates.json") {
            return Some(entry.path());
        }
    }
    None
}

fn emit_log(app: &AppHandle, text: &str, kind: &str, done: bool) {
    app.emit(
        "reply-log",
        ReplyLogEvent {
            text: text.to_string(),
            kind: kind.to_string(),
            done,
        },
    )
    .ok();
}

/// Generate Google Play reply candidates for a batch of reviews via the
/// `review-reply` skill (path A / batch).
///
/// Pipeline: write the pending-reviews JSON → run `claude /review-reply <json>`
/// (the skill writes `<stem>.candidates.json` next to it) → read that file back
/// and return its parsed contents to the frontend.
///
/// `groups` is passed straight through to the skill input; the frontend builds it
/// in the schema the skill expects (groups[].reviews[]). `target_language` is an
/// ISO code or "auto" (reply per-review in each reviewer's own language).
#[tauri::command]
pub async fn run_reply_skill(
    groups: serde_json::Value,
    target_language: String,
    channel: String,
    model: Option<String>,
    app: AppHandle,
    state: State<'_, ReplyState>,
) -> Result<ReplyResult, String> {
    {
        let mut running = state.running.lock().unwrap();
        if *running {
            return Err("Reply generation is already running".into());
        }
        *running = true;
    }

    let result = run_reply_skill_inner(groups, target_language, channel, model, app.clone()).await;
    *state.running.lock().unwrap() = false;
    *state.child_pid.lock().unwrap() = None;
    match &result {
        Ok(_) => emit_log(&app, "回复候选生成完成。", "result", true),
        Err(e) if e == "CANCELLED" => emit_log(&app, "已取消生成。", "info", true),
        Err(e) => emit_log(&app, &format!("失败：{}", e), "error", true),
    }
    result
}

/// Cancel an in-flight reply generation: flip running off (so the inner fn reports
/// CANCELLED) and kill the claude process tree.
#[tauri::command]
pub async fn stop_reply(state: State<'_, ReplyState>) -> Result<(), String> {
    *state.running.lock().unwrap() = false;
    let pid = state.child_pid.lock().unwrap().take();
    let pid = match pid {
        Some(p) => p,
        None => return Ok(()), // nothing running
    };
    let result = if cfg!(windows) {
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
    result
        .map(|_| ())
        .map_err(|e| format!("Failed to stop reply: {}", e))
}

async fn run_reply_skill_inner(
    groups: serde_json::Value,
    target_language: String,
    channel: String,
    model: Option<String>,
    app: AppHandle,
) -> Result<ReplyResult, String> {
    let groups_arr = groups
        .as_array()
        .ok_or("groups must be an array")?;
    if groups_arr.is_empty() {
        return Err("没有可处理的评论（groups 为空）。".into());
    }

    let channel = if channel.trim().is_empty() {
        "gp".to_string()
    } else {
        channel
    };
    let target_language = if target_language.trim().is_empty() {
        "auto".to_string()
    } else {
        target_language
    };

    let input = serde_json::json!({
        "target_language": target_language,
        "channel": channel,
        "groups": groups,
    });

    let dir = data_dir().join("reviews");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Create reviews dir failed: {}", e))?;
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stem = format!("pending-reviews-{}", ts);
    let input_path = dir.join(format!("{}.json", stem));
    // The skill writes a "<stem>*.candidates.json" next to the input. SKILL.md is
    // ambiguous about whether it's "<stem>.candidates.json" or
    // "<stem>.json.candidates.json", so we resolve by scanning (see find_candidates_file).

    let input_str = serde_json::to_string_pretty(&input)
        .map_err(|e| format!("Serialize input failed: {}", e))?;
    std::fs::write(&input_path, &input_str)
        .map_err(|e| format!("Write input JSON failed: {}", e))?;

    // Stale-output guard: drop any pre-existing candidates file for this stem so a
    // skill failure can't be masked by a leftover from a previous run.
    if let Some(p) = find_candidates_file(&dir, &stem) {
        let _ = std::fs::remove_file(p);
    }

    let claude_path = find_claude()
        .ok_or("Claude CLI not found. Please install it: npm install -g @anthropic-ai/claude-code")?;

    let input_path_str = input_path.to_string_lossy().to_string();
    let dir_str = dir.to_string_lossy().to_string();

    let mut args = vec![
        "--print".to_string(),
        "--verbose".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--permission-mode".to_string(),
        "bypassPermissions".to_string(),
        "--add-dir".to_string(),
        dir_str.clone(),
    ];
    if let Some(m) = model.as_ref().filter(|s| !s.is_empty()) {
        args.push("--model".to_string());
        args.push(m.clone());
    }

    let prompt = format!("/review-reply {}\n", input_path_str);

    emit_log(&app, &format!("$ claude {} '{}'", args.join(" "), prompt.trim()), "info", false);

    let mut cmd = Command::new(&claude_path);
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(token) = load_claude_token() {
        cmd.env("CLAUDE_CODE_SESSION_ACCESS_TOKEN", &token);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn claude: {}", e))?;

    // Record pid so stop_reply can kill the process tree on user cancel.
    *app.state::<ReplyState>().child_pid.lock().unwrap() = child.id();

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(prompt.as_bytes()).await;
        drop(stdin);
    }

    let stdout = child.stdout.take().ok_or("No stdout")?;
    let stderr = child.stderr.take().ok_or("No stderr")?;

    // Captured from the CLI's terminal "result" event: token usage + cost.
    let usage_cell: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
    let usage_for_task = usage_cell.clone();

    let app_out = app.clone();
    let stdout_task = tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match event_type {
                    "assistant" => {
                        if let Some(content) = val
                            .get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_array())
                        {
                            for block in content {
                                match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                                    "text" => {
                                        if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                                            if !t.trim().is_empty() {
                                                emit_log(&app_out, t, "text", false);
                                            }
                                        }
                                    }
                                    "tool_use" => {
                                        let name = block
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("tool");
                                        emit_log(&app_out, &format!("· {}", name), "tool", false);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    "result" => {
                        if let Some(r) = val.get("result").and_then(|v| v.as_str()) {
                            if !r.trim().is_empty() {
                                emit_log(&app_out, r, "text", false);
                            }
                        }
                        // Pull token usage + cost from the terminal result event.
                        if let Some(usage) = val.get("usage").filter(|u| u.is_object()) {
                            let mut u = usage.clone();
                            if let (Some(obj), Some(cost)) =
                                (u.as_object_mut(), val.get("total_cost_usd"))
                            {
                                obj.insert("total_cost_usd".to_string(), cost.clone());
                            }
                            let it = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                            let ot = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                            let cr = u
                                .get("cache_read_input_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let cost = u
                                .get("total_cost_usd")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0);
                            emit_log(
                                &app_out,
                                &format!(
                                    "用量：输入 {} · 输出 {} · 缓存读 {} tokens · 约 ${:.4}",
                                    it, ot, cr, cost
                                ),
                                "info",
                                false,
                            );
                            *usage_for_task.lock().unwrap() = Some(u);
                        }
                    }
                    _ => {}
                }
            } else if !line.trim().is_empty() {
                emit_log(&app_out, &line, "text", false);
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
    *app.state::<ReplyState>().child_pid.lock().unwrap() = None;

    // A user cancel kills the process; surface that as a clear message rather than
    // "exited with code 1" so the UI can show "已取消".
    if !*app.state::<ReplyState>().running.lock().unwrap() {
        return Err("CANCELLED".into());
    }

    if !status.success() {
        return Err(format!(
            "Claude exited with code {}",
            status.code().unwrap_or(-1)
        ));
    }

    let candidates_path = find_candidates_file(&dir, &stem).ok_or_else(|| {
        format!(
            "skill 已结束但未在 {} 找到 {}*.candidates.json",
            dir.to_string_lossy(),
            stem
        )
    })?;

    let content = std::fs::read_to_string(&candidates_path)
        .map_err(|e| format!("Read candidates file failed: {}", e))?;
    let parsed: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("候选文件不是合法 JSON：{}", e))?;
    let usage = usage_cell.lock().unwrap().take();
    Ok(ReplyResult {
        output: parsed,
        usage,
    })
}

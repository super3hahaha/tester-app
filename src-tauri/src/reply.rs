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

/// Result of `generate_single_reply`: a JSON array of 3 style-varied candidates
/// (each `{style, language, text, text_zh, char_count}`) plus token/cost usage.
#[derive(Serialize)]
pub struct GenReplyResult {
    candidates: serde_json::Value,
    usage: Option<serde_json::Value>,
}

fn rv_str<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str()).filter(|s| !s.trim().is_empty())
}

/// Build the prompt for single-review freeform generation. Concatenates the
/// review context, the user's direction, and the review-reply skill's hard
/// standards (≤350 chars / reply language / no fabrication / keep emoji+proper
/// nouns / quote style), then asks for a JSON array of 3 style-varied candidates.
fn build_gen_prompt(
    review: &serde_json::Value,
    product: &str,
    package_name: &str,
    instruction: &str,
    language: &str,
) -> String {
    let star = review.get("star_rating").and_then(|v| v.as_i64()).unwrap_or(0);
    let original = rv_str(review, "original_text").unwrap_or("(无)");
    let zh = rv_str(review, "text").unwrap_or("(无)");
    let rev_lang = rv_str(review, "reviewer_language").unwrap_or("(未知)");
    let version = rv_str(review, "app_version_name").unwrap_or("(未知)");
    let device = rv_str(review, "device").unwrap_or("(未知)");
    let os = review
        .get("android_os_version")
        .and_then(|v| v.as_i64())
        .map(|n| format!("Android {}", n))
        .unwrap_or_else(|| "(未知)".to_string());

    let lang_rule = if language.trim().is_empty() || language == "auto" {
        "默认用「评论本身的语言」回复（看下面的「评论语言」，为空则据原文判断）。评论是英文就回英文，俄语就回俄语，不要因为中文译文而回中文。".to_string()
    } else {
        format!("本次所有候选统一用 `{}` 这个语言回复。", language)
    };

    // 回复方向可留空：用户拿不准怎么回时，让模型自己据评论判断最合适的回应方向。
    let instruction_block = if instruction.trim().is_empty() {
        "（用户未指定方向。这是一条 Google Play 上应用的公开评论，回复须同样严格遵守上面的【硬性标准】。\
请根据评论本身判断最合适的回应：\
能定位到具体问题/诉求的，针对性回应并给出可操作的下一步——对「无法更新 / 无法安装 / 下载失败 / 闪退」这类常见问题，\
给通用排查引导（确认设备存储空间充足、网络正常；到系统「设置 > 应用 > Google Play 商店」清除缓存、必要时清除存储后重试；\
在 Play 商店「管理应用和设备」里把卡住的其他更新跑完；重启设备；仍不行再卸载后重新安装），而不是只让用户「联系我们」；\
信息不足、无法判断原因的，礼貌致谢并请用户补充「具体问题表现 / 复现步骤 / 错误提示」（不要问机型、系统版本、应用版本——后台已可见），或通过应用内反馈、邮件联系；\
纯好评则简短真诚地感谢，可自然地邀请给五星好评。不要编造具体原因或未发布的修复承诺，也不要因为没给方向就回得空泛套话。）"
    } else {
        instruction
    };

    format!(
        r#"你是 Google Play 应用的开发者，正在以官方身份回复一条用户评论。
根据下面的【评论信息】和【回复方向】，生成回复，并严格遵守【硬性标准】。
不要使用任何工具，直接给出结果。

【评论信息】
- 应用：{product}（{package_name}）
- 星级：{star}★
- 用户原文（优先据此理解语义）：{original}
- 中文译文（仅供你理解，不要据此判断语言）：{zh}
- 评论语言：{rev_lang}
- 应用版本：{version}
- 设备 / 安卓版本：{device} / {os}

【回复方向】
{instruction}

【硬性标准】
1. 长度：这是 Google Play 公开回复，每条 ≤ 350 字符（含空格/标点/emoji），超了必须改短。
2. 回复语言：{lang_rule}
3. 不编造、不乱承诺：邮箱、版本号、团队名、价格、未发布功能等不确定的事实绝不杜撰；也不要做无法兑现/无法确认的承诺（如"下个版本一定修复""X 号前上线"）。
4. 不向用户索取机型、Android 版本、应用版本——这些后台都看得到；确需更多信息时只问「具体问题表现 / 复现步骤 / 错误提示」。
5. 语气：温暖友好、真诚感谢反馈、对症回应；在自然的前提下可邀请用户给五星好评或进一步联系（应用内反馈 / 邮件），但不要生硬索评、不堆空话套话。
6. 退款诉求：不直接谈退款流程，把焦点引导到排查上——先询问具体的 bug 表现/细节，尝试帮用户解决问题。
7. 保留：emoji 原样保留；专有名词（应用名、Android、Google Play）不翻译。
8. 引号：正文里尽量不用 ASCII 直引号；要引用 UI 选项名时英文用 '...'，中文用「」。
9. 生成 3 条候选，风格/角度要有明显差异（例如：诚恳道歉式 / 务实引导式 / 简短友好式），
   不要只是改几个词。每条都各自满足 1-8 全部约束。

【输出格式】
只输出一个 JSON 数组，3 个元素，不要任何额外文字、不要 markdown 代码块：
[
  {{ "style": "风格名", "language": "实际语言 ISO 码", "text": "回复正文", "text_zh": "中文预览", "char_count": 正文字符数 }},
  ...共 3 条
]"#,
        product = product,
        package_name = package_name,
        star = star,
        original = original,
        zh = zh,
        rev_lang = rev_lang,
        version = version,
        device = device,
        os = os,
        instruction = instruction_block,
        lang_rule = lang_rule,
    )
}

/// Extract a JSON array substring from possibly-fenced / prose-wrapped model
/// output: slice from the first '[' to the last ']'.
fn extract_json_array(s: &str) -> Option<&str> {
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    if end > start {
        Some(&s[start..=end])
    } else {
        None
    }
}

/// Generate 3 style-varied Google Play reply candidates for ONE review, driven by
/// a freeform user instruction. Unlike `run_reply_skill` (template-match only),
/// this calls `claude --print` directly with a self-contained prompt and parses
/// the model's JSON array output — no skill, no file round-trip.
#[tauri::command]
pub async fn generate_single_reply(
    review: serde_json::Value,
    product: String,
    package_name: String,
    instruction: String,
    language: String,
    model: Option<String>,
    app: AppHandle,
    state: State<'_, ReplyState>,
) -> Result<GenReplyResult, String> {
    {
        let mut running = state.running.lock().unwrap();
        if *running {
            return Err("已有回复生成任务在进行中。".into());
        }
        *running = true;
    }
    let result =
        generate_single_reply_inner(review, product, package_name, instruction, language, model, app.clone())
            .await;
    *state.running.lock().unwrap() = false;
    *state.child_pid.lock().unwrap() = None;
    match &result {
        Ok(_) => emit_log(&app, "候选生成完成。", "result", true),
        Err(e) if e == "CANCELLED" => emit_log(&app, "已取消生成。", "info", true),
        Err(e) => emit_log(&app, &format!("失败：{}", e), "error", true),
    }
    result
}

async fn generate_single_reply_inner(
    review: serde_json::Value,
    product: String,
    package_name: String,
    instruction: String,
    language: String,
    model: Option<String>,
    app: AppHandle,
) -> Result<GenReplyResult, String> {
    // 回复方向允许为空——空时 build_gen_prompt 会让模型据评论自行判断方向。
    let prompt = build_gen_prompt(&review, &product, &package_name, instruction.trim(), &language);

    let claude_path = find_claude()
        .ok_or("Claude CLI not found. Please install it: npm install -g @anthropic-ai/claude-code")?;

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

    emit_log(&app, "正在生成 3 条候选回复…", "info", false);

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
    *app.state::<ReplyState>().child_pid.lock().unwrap() = child.id();

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(prompt.as_bytes()).await;
        drop(stdin);
    }

    let stdout = child.stdout.take().ok_or("No stdout")?;
    let stderr = child.stderr.take().ok_or("No stderr")?;

    // Accumulate the model's final text. The `result` event carries the complete
    // final message; we prefer it but fall back to concatenated assistant text.
    let result_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let assistant_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let usage_cell: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
    let result_for_task = result_text.clone();
    let assistant_for_task = assistant_text.clone();
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
                                if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                                        assistant_for_task.lock().unwrap().push_str(t);
                                    }
                                }
                            }
                        }
                    }
                    "result" => {
                        if let Some(r) = val.get("result").and_then(|v| v.as_str()) {
                            *result_for_task.lock().unwrap() = r.to_string();
                        }
                        if let Some(usage) = val.get("usage").filter(|u| u.is_object()) {
                            let mut u = usage.clone();
                            if let (Some(obj), Some(cost)) =
                                (u.as_object_mut(), val.get("total_cost_usd"))
                            {
                                obj.insert("total_cost_usd".to_string(), cost.clone());
                            }
                            *usage_for_task.lock().unwrap() = Some(u);
                        }
                    }
                    _ => {}
                }
            }
        }
        let _ = &app_out;
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

    if !*app.state::<ReplyState>().running.lock().unwrap() {
        return Err("CANCELLED".into());
    }
    if !status.success() {
        return Err(format!("Claude exited with code {}", status.code().unwrap_or(-1)));
    }

    let raw = {
        let r = result_text.lock().unwrap().clone();
        if r.trim().is_empty() {
            assistant_text.lock().unwrap().clone()
        } else {
            r
        }
    };
    let json_str = extract_json_array(&raw)
        .ok_or_else(|| format!("模型输出里找不到 JSON 数组：{}", raw.chars().take(300).collect::<String>()))?;
    let candidates: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("候选不是合法 JSON 数组：{}", e))?;

    let usage = usage_cell.lock().unwrap().take();
    Ok(GenReplyResult { candidates, usage })
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

//! 评论分析的「知识配置」：每个产品一份 Markdown 知识块，存在
//! `~/.tester-app/review-analysis/{slug}.md`。知识块描述应用定位、常见问题与
//! 标准排查引导、回复红线，供后续「🔍 分析」按钮在生成提示词时注入 {app_knowledge}。
//!
//! 产品列表沿用 templates 的产品（list_template_products），文件名用同一套
//! product→prefix 规则（XFolder→xfolder、通用→common…）保证与模板侧一致、稳定。

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::claude::{find_claude, load_claude_token};

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

/// 知识块目录 `~/.tester-app/review-analysis/`。
fn knowledge_dir() -> PathBuf {
    data_dir().join("review-analysis")
}

/// 产品名 → 知识文件名（不含扩展名）。复用模板侧的 product_prefix 规则：
/// 已知产品走固定映射，自定义产品走 slug，纯非 ASCII 名兜底 "tpl"。
fn knowledge_stem(product: &str) -> String {
    crate::templates::product_prefix(product, &[])
}

/// 某产品的知识文件路径。
fn knowledge_file(product: &str) -> PathBuf {
    knowledge_dir().join(format!("{}.md", knowledge_stem(product)))
}

/// 列表项：产品 + 关联应用 + 是否已写知识 + 字符数（前端 tab 标记用）。
#[derive(Serialize)]
pub struct KnowledgeInfo {
    product: String,
    apps: Vec<String>,
    has_content: bool,
    chars: usize,
}

/// 列出所有产品及其知识块状态。产品来源与模板管理一致。
#[tauri::command]
pub fn list_knowledge() -> Result<Vec<KnowledgeInfo>, String> {
    let products = crate::templates::list_template_products(None)?;
    Ok(products
        .into_iter()
        .map(|p| {
            let content = std::fs::read_to_string(knowledge_file(&p.product)).unwrap_or_default();
            let trimmed = content.trim();
            KnowledgeInfo {
                product: p.product,
                apps: p.apps,
                has_content: !trimmed.is_empty(),
                chars: content.chars().count(),
            }
        })
        .collect())
}

/// 读某产品的知识块（不存在返回空串，交前端用骨架占位）。
#[tauri::command]
pub fn read_knowledge(product: String) -> Result<String, String> {
    let path = knowledge_file(&product);
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("读取知识块失败：{}", e)),
    }
}

/// 写某产品的知识块（内容为空也写——表示显式清空）。
#[tauri::command]
pub fn write_knowledge(product: String, content: String) -> Result<(), String> {
    let dir = knowledge_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建知识目录失败：{}", e))?;
    std::fs::write(knowledge_file(&product), content)
        .map_err(|e| format!("写知识块失败：{}", e))?;
    Ok(())
}

// ── 单条「🔍 分析」：分析评论 + 给一条可直接发布的推荐回复 ────────────────────
//
// 与 reply.rs 的 generate_single_reply 同构（claude --print 直出 JSON、无 skill、
// 无文件往返），但：① 解析的是一个 JSON 对象（不是数组）；② prompt 里注入按评论
// 来源 app 解析出的产品知识块 {app_knowledge}；③ 用独立的 AnalysisState + 独立的
// `analysis-log` 事件通道，与 AI 回复互不污染（同 decisions.md 的状态隔离原则）。

/// 分析任务的独立运行状态（一次只跑一个，前端串行排队），与 ReplyState 平行。
pub struct AnalysisState {
    pub running: Mutex<bool>,
    pub child_pid: Mutex<Option<u32>>,
}

impl AnalysisState {
    pub fn new() -> Self {
        Self {
            running: Mutex::new(false),
            child_pid: Mutex::new(None),
        }
    }
}

#[derive(Serialize, Clone)]
struct AnalysisLogEvent {
    text: String,
    kind: String,
    done: bool,
}

fn emit_log(app: &AppHandle, text: &str, kind: &str, done: bool) {
    app.emit(
        "analysis-log",
        AnalysisLogEvent {
            text: text.to_string(),
            kind: kind.to_string(),
            done,
        },
    )
    .ok();
}

/// `generate_analysis` 的返回：分析 JSON 对象 + token/费用用量。
#[derive(Serialize)]
pub struct AnalysisResult {
    analysis: serde_json::Value,
    usage: Option<serde_json::Value>,
}

fn rv_str<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
}

/// 从可能被 markdown 围栏 / 前后散文包裹的模型输出里抠出 JSON 对象：取第一个 '{'
/// 到最后一个 '}'。
fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if escape {
            escape = false;
        } else if in_string {
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
        } else {
            match b {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&s[start..=i]);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// 拼接评论分析提示词。{app_knowledge} 来自按 package_name 解析出的产品知识块；
/// {lang_rule} 同 reply.rs 的口径（auto = 跟随评论语言）。
fn build_analysis_prompt(
    review: &serde_json::Value,
    product: &str,
    package_name: &str,
    app_knowledge: &str,
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
        format!("用 `{}` 这个语言回复。", language)
    };

    let knowledge = if app_knowledge.trim().is_empty() {
        "（暂无该应用的知识块，请仅据评论本身判断。）".to_string()
    } else {
        app_knowledge.to_string()
    };

    // 完整模板从 app 设置读（含占位符），用 render 替换；缺失/损坏回退默认（逐字等于原写死文本）。
    let template = crate::prompt_config::load().analysis;
    let star_s = star.to_string();
    crate::prompt_config::render(
        &template,
        &[
            ("product", product),
            ("package_name", package_name),
            ("knowledge", &knowledge),
            ("star", &star_s),
            ("original", original),
            ("zh", zh),
            ("rev_lang", rev_lang),
            ("version", version),
            ("device", device),
            ("os", &os),
            ("lang_rule", &lang_rule),
        ],
    )
}

/// 分析一条评论并给出推荐回复。按 package_name 解析模板产品 → 读该产品知识块注入
/// 提示词 → 跑 `claude --print`（无 skill、无文件往返）→ 解析 JSON 对象返回。
#[tauri::command]
pub async fn generate_analysis(
    review: serde_json::Value,
    product: String,
    package_name: String,
    language: String,
    model: Option<String>,
    app: AppHandle,
    state: State<'_, AnalysisState>,
) -> Result<AnalysisResult, String> {
    {
        let mut running = state.running.lock().unwrap();
        if *running {
            return Err("已有分析任务在进行中。".into());
        }
        *running = true;
    }
    let result =
        generate_analysis_inner(review, product, package_name, language, model, app.clone()).await;
    *state.running.lock().unwrap() = false;
    *state.child_pid.lock().unwrap() = None;
    match &result {
        Ok(_) => emit_log(&app, "分析完成。", "result", true),
        Err(e) if e == "CANCELLED" => emit_log(&app, "已取消分析。", "info", true),
        Err(e) => emit_log(&app, &format!("失败：{}", e), "error", true),
    }
    result
}

/// 取消进行中的分析：置 running=false（让 inner 报 CANCELLED）并杀进程树。
#[tauri::command]
pub async fn stop_analysis(state: State<'_, AnalysisState>) -> Result<(), String> {
    *state.running.lock().unwrap() = false;
    let pid = state.child_pid.lock().unwrap().take();
    let pid = match pid {
        Some(p) => p,
        None => return Ok(()),
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
        .map_err(|e| format!("Failed to stop analysis: {}", e))
}

async fn generate_analysis_inner(
    review: serde_json::Value,
    product: String,
    package_name: String,
    language: String,
    model: Option<String>,
    app: AppHandle,
) -> Result<AnalysisResult, String> {
    // 按评论来源 app 解析模板产品 → 读对应知识块（无产品 / 无知识块都退回空串）。
    let knowledge = match crate::templates::product_for_package(package_name.clone()) {
        Ok(Some(prod)) => read_knowledge(prod).unwrap_or_default(),
        _ => String::new(),
    };

    let prompt = build_analysis_prompt(&review, &product, &package_name, &knowledge, &language);

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

    emit_log(&app, "正在分析评论…", "info", false);

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
    *app.state::<AnalysisState>().child_pid.lock().unwrap() = child.id();

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(prompt.as_bytes()).await;
        drop(stdin);
    }

    let stdout = child.stdout.take().ok_or("No stdout")?;
    let stderr = child.stderr.take().ok_or("No stderr")?;

    let result_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let assistant_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let usage_cell: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
    let result_for_task = result_text.clone();
    let assistant_for_task = assistant_text.clone();
    let usage_for_task = usage_cell.clone();

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
    *app.state::<AnalysisState>().child_pid.lock().unwrap() = None;

    if !*app.state::<AnalysisState>().running.lock().unwrap() {
        return Err("CANCELLED".into());
    }
    if !status.success() {
        return Err(format!(
            "Claude exited with code {}",
            status.code().unwrap_or(-1)
        ));
    }

    let raw = {
        let r = result_text.lock().unwrap().clone();
        if r.trim().is_empty() {
            assistant_text.lock().unwrap().clone()
        } else {
            r
        }
    };
    let json_str = extract_json_object(&raw).ok_or_else(|| {
        format!(
            "模型输出里找不到 JSON 对象：{}",
            raw.chars().take(300).collect::<String>()
        )
    })?;
    let analysis: serde_json::Value = serde_json::from_str(json_str)
        .or_else(|_| serde_json::from_str(&crate::json_repair::repair_json(json_str)))
        .map_err(|e| format!("分析结果不是合法 JSON：{}", e))?;

    let usage = usage_cell.lock().unwrap().take();
    Ok(AnalysisResult { analysis, usage })
}

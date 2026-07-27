//! 调用 `prd-risk-profiler` skill 的「复用模式」：只给一份新 PRD（不需要 bug
//! 报告），skill 读取该 APP 已有知识库产出两份文件——完整依据的《风险分析报告》
//! + 精简速览的《补充测试点》清单（固定为嵌套 Markdown 复选框格式）。本模块只
//! 负责「正确调用 + 落盘管理」：不解析变更清单/风险判断逻辑（那是 SKILL.md 第
//! 二节复用模式本来就会做的事）；补充测试点的复选框解析/渲染/编辑放在前端做，
//! 后端只管这两份文件的存取。
//!
//! 存储结构：`~/.tester-app/prd-supplements/<APP名>/<版本号>/<生成时间戳>/`
//!   ├── 风险分析.md   （只读留档，不提供编辑保存）
//!   └── 补充测试点.md （前端解析成复选框列表，可勾选/改文字/存盘覆盖）
//! 同一 APP+版本允许多次生成，每次生成是独立的时间戳子目录，不覆盖旧的——
//! 用户可能在 PRD 改动后重新跑一次，旧的分析仍有参考价值，交由用户手动删除。
//!
//! skill 默认建议把两份文件存到云端沙盒路径 `/mnt/user-data/outputs/`，本地
//! CLI 场景下这个路径不存在，所以在调用时的 prompt 里显式指定真实的本地目录
//! 和固定文件名覆盖 skill 的默认建议——不改 SKILL.md 本身（那是 GitHub 上的
//! 共享 skill 源码，改本地文件会被 skill_sync 热更新冲掉，见 decisions.md）。

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::claude::{find_claude, load_claude_token};

pub struct PrdSupplementState {
    pub running: Mutex<bool>,
    pub child_pid: Mutex<Option<u32>>,
}

impl Default for PrdSupplementState {
    fn default() -> Self {
        Self {
            running: Mutex::new(false),
            child_pid: Mutex::new(None),
        }
    }
}

#[derive(Serialize, Clone)]
struct SupplementLogEvent {
    text: String,
    kind: String, // "info" | "text" | "tool" | "error"
    done: bool,
}

fn emit_log(app: &AppHandle, text: &str, kind: &str, done: bool) {
    app.emit(
        "prd-supplement-log",
        SupplementLogEvent {
            text: text.to_string(),
            kind: kind.to_string(),
            done,
        },
    )
    .ok();
}

fn root_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join(".tester-app")
        .join("prd-supplements")
}

fn skill_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join(".claude")
        .join("skills")
        .join("prd-risk-profiler")
}

const RISK_REPORT_FILE: &str = "风险分析.md";
const SUPPLEMENT_FILE: &str = "补充测试点.md";

/// 只挡路径穿越，不做别的净化——app 名/版本号沿用现有 "Free Ringtones.md"
/// 那种带空格的原样文件名习惯，无需额外转义。
fn validate_segment(s: &str, field: &str) -> Result<(), String> {
    let t = s.trim();
    if t.is_empty() {
        return Err(format!("{}不能为空", field));
    }
    if t.contains('/') || t.contains('\\') || t.contains("..") {
        return Err(format!("{}包含非法字符", field));
    }
    Ok(())
}

fn generation_dir(app_name: &str, version: &str, generation: &str) -> Result<PathBuf, String> {
    validate_segment(app_name, "APP 名称")?;
    validate_segment(version, "版本号")?;
    validate_segment(generation, "生成记录 ID")?;
    Ok(root_dir()
        .join(app_name.trim())
        .join(version.trim())
        .join(generation.trim()))
}

#[tauri::command]
pub async fn run_prd_supplement_reuse(
    app_name: String,
    version: String,
    prd_image_paths: Vec<String>,
    model: Option<String>,
    app: AppHandle,
    state: State<'_, PrdSupplementState>,
) -> Result<String, String> {
    {
        let mut running = state.running.lock().unwrap();
        if *running {
            return Err("已有一个补充测试点生成任务在跑".into());
        }
        *running = true;
    }

    let result = run_inner(app_name, version, prd_image_paths, model, app.clone()).await;
    *state.running.lock().unwrap() = false;
    *state.child_pid.lock().unwrap() = None;
    match &result {
        Ok(_) => emit_log(&app, "生成完成。", "info", true),
        Err(e) if e == "CANCELLED" => emit_log(&app, "已取消。", "info", true),
        Err(e) => emit_log(&app, &format!("失败：{}", e), "error", true),
    }
    result
}

async fn run_inner(
    app_name: String,
    version: String,
    prd_image_paths: Vec<String>,
    model: Option<String>,
    app: AppHandle,
) -> Result<String, String> {
    let app_name = app_name.trim().to_string();
    let version = version.trim().to_string();
    validate_segment(&app_name, "APP 名称")?;
    validate_segment(&version, "版本号")?;
    if prd_image_paths.is_empty() {
        return Err("没有 PRD 图片（导出失败或没选 Slides）".into());
    }

    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let generation = ts.to_string();
    let gen_dir = generation_dir(&app_name, &version, &generation)?;
    std::fs::create_dir_all(&gen_dir).map_err(|e| format!("创建目录失败：{}", e))?;

    let claude_path = find_claude()
        .ok_or("未找到 Claude CLI，请先安装：npm install -g @anthropic-ai/claude-code")?;

    let mut dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    dirs.insert(gen_dir.to_string_lossy().to_string());
    for p in &prd_image_paths {
        if let Some(parent) = Path::new(p).parent() {
            dirs.insert(parent.to_string_lossy().to_string());
        }
    }
    dirs.insert(skill_dir().to_string_lossy().to_string());

    let mut args = vec![
        "--print".to_string(),
        "--verbose".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--permission-mode".to_string(),
        "bypassPermissions".to_string(),
    ];
    for d in &dirs {
        args.push("--add-dir".to_string());
        args.push(d.clone());
    }
    let model = model
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::model_config::load().testcase);
    if !model.is_empty() {
        args.push("--model".to_string());
        args.push(model);
    }

    let mut prompt = format!(
        "/prd-risk-profiler\nAPP名称：{}\n版本号：{}\nPRD（按页截图，逐张 Read 查看）：\n",
        app_name, version
    );
    for p in &prd_image_paths {
        prompt.push_str(&format!("- {}\n", p));
    }
    prompt.push_str(&format!(
        "\n请执行「复用模式」（只给新 PRD，没有 bug/缺陷报告）：读取该 APP 已有知识库\
（专属层 + 通用层），对本次 PRD 变更做风险分析，产出《风险分析报告》和《补充\
测试点》两份文件。\n\n\
**重要：本次两份产出文件请保存到下面这个本地目录，文件名固定为「{}」和「{}」，\
不要使用 SKILL.md 里默认建议的 /mnt/user-data/outputs/ 路径、也不要用默认的\
「<APP名>_<版本号>_xxx.md」命名**：\n{}\n\n\
完成后用一两句话总结：识别了几个变更、命中了几条已有知识库风险、新增了几条\
知识库未覆盖的风险点。\n",
        RISK_REPORT_FILE,
        SUPPLEMENT_FILE,
        gen_dir.to_string_lossy()
    ));

    emit_log(&app, &format!("$ claude {} '/prd-risk-profiler ...'", args.join(" ")), "info", false);

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
        .map_err(|e| format!("启动 claude 失败：{}", e))?;

    *app.state::<PrdSupplementState>().child_pid.lock().unwrap() = child.id();

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes()).await;
        drop(stdin);
    }

    let stdout = child.stdout.take().ok_or("无 stdout")?;
    let stderr = child.stderr.take().ok_or("无 stderr")?;

    let result_text = std::sync::Arc::new(Mutex::new(String::new()));
    let assistant_text = std::sync::Arc::new(Mutex::new(String::new()));
    let result_for_task = result_text.clone();
    let assistant_for_task = assistant_text.clone();

    let app_out = app.clone();
    let stdout_task = tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                match val.get("type").and_then(|v| v.as_str()).unwrap_or("") {
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
                                                assistant_for_task.lock().unwrap().push_str(t);
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
                            *result_for_task.lock().unwrap() = r.to_string();
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

    let status = child.wait().await.map_err(|e| format!("Claude 进程出错：{}", e))?;
    stdout_task.await.ok();
    stderr_task.await.ok();

    if !*app.state::<PrdSupplementState>().running.lock().unwrap() {
        return Err("CANCELLED".into());
    }

    let final_text = {
        let r = result_text.lock().unwrap().clone();
        if !r.is_empty() {
            r
        } else {
            assistant_text.lock().unwrap().clone()
        }
    };

    if !status.success() && final_text.is_empty() {
        return Err(format!(
            "Claude 进程异常退出（code {}）且无输出",
            status.code().unwrap_or(-1)
        ));
    }

    if !gen_dir.join(RISK_REPORT_FILE).is_file() || !gen_dir.join(SUPPLEMENT_FILE).is_file() {
        return Err(format!(
            "skill 未按约定路径生成文件（预期 {}/{{{}, {}}}），请查看上方日志排查",
            gen_dir.to_string_lossy(),
            RISK_REPORT_FILE,
            SUPPLEMENT_FILE
        ));
    }

    Ok(generation)
}

#[tauri::command]
pub async fn stop_prd_supplement_reuse(state: State<'_, PrdSupplementState>) -> Result<(), String> {
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
    result.map(|_| ()).map_err(|e| format!("停止失败：{}", e))
}

// ── 存取管理：按 App → 版本 → 生成记录 三层浏览已产出的文档 ──────────────────

#[tauri::command]
pub fn list_prd_supplement_apps() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root_dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[tauri::command]
pub fn list_prd_supplement_versions(app_name: String) -> Result<Vec<String>, String> {
    validate_segment(&app_name, "APP 名称")?;
    let dir = root_dir().join(app_name.trim());
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    // 版本号按数值从大到小排（同 BugPage 的 compareVersionDesc 口径），非数字版本号
    // 一律沉到末尾并保持字母序，不让排序报错。
    names.sort_by(|a, b| compare_version_desc(a, b));
    Ok(names)
}

fn compare_version_desc(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Option<Vec<i64>> {
        let parts: Vec<&str> = s.split('.').collect();
        let mut nums = Vec::with_capacity(parts.len());
        for p in parts {
            match p.trim().parse::<i64>() {
                Ok(n) => nums.push(n),
                Err(_) => return None,
            }
        }
        Some(nums)
    };
    match (parse(a), parse(b)) {
        (Some(pa), Some(pb)) => {
            let len = pa.len().max(pb.len());
            for i in 0..len {
                let diff = pb.get(i).copied().unwrap_or(0) - pa.get(i).copied().unwrap_or(0);
                if diff != 0 {
                    return if diff > 0 { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less };
                }
            }
            std::cmp::Ordering::Equal
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.cmp(b),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SupplementGeneration {
    pub id: String,
    pub created_at: String,
    pub checked: usize,
    pub total: usize,
}

/// 计数只做最朴素的行匹配（`- [ ]` / `- [x]`），不需要完整解析结构——展示进度
/// 数字用，真正的结构化解析/渲染/编辑放在前端。
fn count_checkboxes(md: &str) -> (usize, usize) {
    let mut checked = 0usize;
    let mut total = 0usize;
    for line in md.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("- [") {
            if let Some(c) = rest.chars().next() {
                if rest.len() >= 2 && rest.as_bytes().get(1) == Some(&b']') {
                    total += 1;
                    if c == 'x' || c == 'X' {
                        checked += 1;
                    }
                }
            }
        }
    }
    (checked, total)
}

fn format_generation_ts(generation: &str) -> String {
    generation
        .parse::<i64>()
        .ok()
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| generation.to_string())
}

#[tauri::command]
pub fn list_prd_supplement_generations(
    app_name: String,
    version: String,
) -> Result<Vec<SupplementGeneration>, String> {
    validate_segment(&app_name, "APP 名称")?;
    validate_segment(&version, "版本号")?;
    let dir = root_dir().join(app_name.trim()).join(version.trim());
    let mut entries: Vec<SupplementGeneration> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| {
                    let id = e.file_name().into_string().ok()?;
                    let md = std::fs::read_to_string(e.path().join(SUPPLEMENT_FILE)).unwrap_or_default();
                    let (checked, total) = count_checkboxes(&md);
                    Some(SupplementGeneration {
                        created_at: format_generation_ts(&id),
                        id,
                        checked,
                        total,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(entries)
}

#[derive(Debug, Clone, Serialize)]
pub struct SupplementDoc {
    pub supplement_md: String,
    pub risk_report_md: String,
}

#[tauri::command]
pub fn read_prd_supplement_generation(
    app_name: String,
    version: String,
    generation: String,
) -> Result<SupplementDoc, String> {
    let dir = generation_dir(&app_name, &version, &generation)?;
    let supplement_md = std::fs::read_to_string(dir.join(SUPPLEMENT_FILE))
        .map_err(|e| format!("读取补充测试点失败：{}", e))?;
    let risk_report_md = std::fs::read_to_string(dir.join(RISK_REPORT_FILE))
        .map_err(|e| format!("读取风险分析报告失败：{}", e))?;
    Ok(SupplementDoc {
        supplement_md,
        risk_report_md,
    })
}

/// 只覆盖补充测试点这一份文件——风险分析报告是完整依据留档，不提供编辑入口。
/// 勾选/改文字后直接覆盖保存，不保留 AI 原始生成版本（用户明确要求从简）。
#[tauri::command]
pub fn save_prd_supplement_generation(
    app_name: String,
    version: String,
    generation: String,
    supplement_md: String,
) -> Result<(), String> {
    let dir = generation_dir(&app_name, &version, &generation)?;
    if !dir.is_dir() {
        return Err("生成记录不存在".into());
    }
    std::fs::write(dir.join(SUPPLEMENT_FILE), supplement_md).map_err(|e| format!("保存失败：{}", e))
}

#[tauri::command]
pub fn delete_prd_supplement_generation(
    app_name: String,
    version: String,
    generation: String,
) -> Result<(), String> {
    let dir = generation_dir(&app_name, &version, &generation)?;
    if !dir.is_dir() {
        return Ok(());
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("删除失败：{}", e))
}

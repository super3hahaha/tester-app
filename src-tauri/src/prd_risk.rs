//! 调用 `prd-risk-profiler` skill 的「沉淀模式」：把 BugPage 里勾选的 bug 详情
//! + 一份 PRD（导出成逐页 PNG）喂给 skill，由 skill 自己更新它自身的两份知识库
//! 文件（`references/risk-taxonomy.md` 通用层 + `references/apps/<APP名>.md`
//! 专属层，不存在就照 `_TEMPLATE.md` 新建）。本模块只负责「正确调用」，不解析/
//! 不生成这两份 md —— 那是 SKILL.md 里沉淀模式本来就会做的事，重复实现只会和
//! skill 自己的规则跑偏。结构照抄 reply.rs::run_reply_skill_inner（写输入文件→
//! spawn claude --print stream-json→流式转发→等待退出）。

use serde::Serialize;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::claude::{find_claude, load_claude_token};

/// 独立于 ReplyState/ClaudeState/AnalysisState 的运行状态，同一时间只能跑一个
/// prd-risk-profiler 任务，理由同 decisions.md 里"状态隔离"那条。
pub struct PrdRiskState {
    pub running: Mutex<bool>,
    pub child_pid: Mutex<Option<u32>>,
}

impl Default for PrdRiskState {
    fn default() -> Self {
        Self {
            running: Mutex::new(false),
            child_pid: Mutex::new(None),
        }
    }
}

#[derive(Serialize, Clone)]
struct PrdRiskLogEvent {
    text: String,
    kind: String, // "info" | "text" | "tool" | "error"
    done: bool,
}

fn emit_log(app: &AppHandle, text: &str, kind: &str, done: bool) {
    app.emit(
        "prd-risk-log",
        PrdRiskLogEvent {
            text: text.to_string(),
            kind: kind.to_string(),
            done,
        },
    )
    .ok();
}

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

fn skill_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join(".claude")
        .join("skills")
        .join("prd-risk-profiler")
}

#[tauri::command]
pub async fn run_prd_risk_profiler(
    app_name: String,
    version: String,
    bug_report: String,
    prd_image_paths: Vec<String>,
    model: Option<String>,
    app: AppHandle,
    state: State<'_, PrdRiskState>,
) -> Result<String, String> {
    {
        let mut running = state.running.lock().unwrap();
        if *running {
            return Err("已有一个风险画像生成任务在跑".into());
        }
        *running = true;
    }

    let result = run_inner(app_name, version, bug_report, prd_image_paths, model, app.clone()).await;
    *state.running.lock().unwrap() = false;
    *state.child_pid.lock().unwrap() = None;
    match &result {
        Ok(_) => emit_log(&app, "沉淀完成。", "info", true),
        Err(e) if e == "CANCELLED" => emit_log(&app, "已取消。", "info", true),
        Err(e) => emit_log(&app, &format!("失败：{}", e), "error", true),
    }
    result
}

async fn run_inner(
    app_name: String,
    version: String,
    bug_report: String,
    prd_image_paths: Vec<String>,
    model: Option<String>,
    app: AppHandle,
) -> Result<String, String> {
    let app_name = app_name.trim().to_string();
    if app_name.is_empty() {
        return Err("APP 名称不能为空".into());
    }
    if prd_image_paths.is_empty() {
        return Err("没有 PRD 图片（导出失败或没选 Slides）".into());
    }

    let dir = data_dir().join("exports").join("prd-risk");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败：{}", e))?;
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bug_report_path = dir.join(format!("bug-report-{}.txt", ts));
    std::fs::write(&bug_report_path, &bug_report)
        .map_err(|e| format!("写 bug 报告文件失败：{}", e))?;
    let bug_report_path_str = bug_report_path.to_string_lossy().to_string();

    let claude_path = find_claude()
        .ok_or("未找到 Claude CLI，请先安装：npm install -g @anthropic-ai/claude-code")?;

    // --add-dir 授权：bug 报告目录、每张 PRD 图片的父目录（去重）、skill 自身
    // 目录（防御性加上——沉淀模式要往 references/*.md 里写文件，那是 skill 自己
    // 的目录，不一定在默认可写范围内）。
    let mut dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    dirs.insert(dir.to_string_lossy().to_string());
    for p in &prd_image_paths {
        if let Some(parent) = std::path::Path::new(p).parent() {
            dirs.insert(parent.to_string_lossy().to_string());
        }
    }
    let skill_dir_str = skill_dir().to_string_lossy().to_string();
    dirs.insert(skill_dir_str.clone());

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
    // 未指定模型时回退到「模型配置」里的用例生成模型——PRD 风险画像和
    // test-case-generator 一样都是 skill 依赖类功能，复用同一个配置项，
    // 而不是让 claude CLI 退回它自己的全局默认模型（那个和这个 app 的模型
    // 配置完全脱节，见 decisions.md）。
    let model = model
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::model_config::load().testcase);
    if !model.is_empty() {
        args.push("--model".to_string());
        args.push(model);
    }

    let version = if version.trim().is_empty() {
        "未填写".to_string()
    } else {
        version.trim().to_string()
    };

    let mut prompt = format!(
        "/prd-risk-profiler\nAPP名称：{}\n版本号：{}\nPRD（按页截图，逐张 Read 查看）：\n",
        app_name, version
    );
    for p in &prd_image_paths {
        prompt.push_str(&format!("- {}\n", p));
    }
    prompt.push_str(&format!(
        "\nBug/缺陷报告文件（本轮沉淀依据，内容是勾选的多条 bug 详情拼接）：{}\n\n\
请执行沉淀模式：读 PRD 提炼变更清单，对照 bug 报告做变更-证据映射，更新 \
references/risk-taxonomy.md（通用风险）和 references/apps/{}.md（专属风险，\
不存在就复制 _TEMPLATE.md 新建），最后给出本轮沉淀摘要（识别了几个变更、\
几条归通用/几条归专属、两份文件分别有什么变化）。\n",
        bug_report_path_str, app_name
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

    *app.state::<PrdRiskState>().child_pid.lock().unwrap() = child.id();

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

    if !*app.state::<PrdRiskState>().running.lock().unwrap() {
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
    if final_text.is_empty() {
        return Err("Claude 未返回内容".into());
    }
    Ok(final_text)
}

#[tauri::command]
pub async fn stop_prd_risk_profiler(state: State<'_, PrdRiskState>) -> Result<(), String> {
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

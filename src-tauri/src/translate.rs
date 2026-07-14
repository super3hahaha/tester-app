//! 模板多语言预翻译：用 claude CLI 把某产品的模板批量翻成多语言，结果写回
//! `~/.tester-app/templates/templates.json` 每条模板的 `translations`，供 review-reply
//! 命中后直接取用（省掉运行时翻译）。
//!
//! 成本优化（关键）：**不走 skill、不 `--add-dir`、不写文件、不自检**。每批只把这几条
//! 模板的正文**内联进 prompt**（Claude 物理上只看得到这几条，接触不到整个 122KB 模板库），
//! prompt 里要求「不使用任何工具、直接输出 JSON」，后端从 stdout 解析。模型用 haiku。
//! 这套轻量直出比早期的 agent+add-dir+自检路径省一个数量级。见 docs/handoff-template-i18n.md。
//!
//! 三种场景由前端用 (ids, langs, overwrite) 表达：
//! - 首次铺底：ids=None + langs=全部 + overwrite=true
//! - 单条重译：ids=[该条] + langs=该条已有语言 + overwrite=true
//! - 新增语言：ids=None + langs=全部(或新增) + overwrite=false（只补缺失、追加）
//!
//! 每批 CHUNK 条，**每批跑完立刻落库** → 中断/停止只丢当前这一小批。

use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::claude::{find_claude, load_claude_token};

/// 每批翻多少条模板。设 1：一条模板 × 二十多语言已是一次不小的输出，再多条单次输出太大、
/// 易出错、中断损失大。一条一批 → 单次小而稳、进度每条一格、中断只丢一条。
const CHUNK: usize = 1;

/// Google Play 回复字符硬上限。超此长度的译文会被压缩重试，仍超则标红警告。
const GP_LIMIT: usize = 350;

/// 翻译任务独立的 running/pid（与 ReplyState、ClaudeState 互不干扰）。
pub struct TranslateState {
    pub running: Mutex<bool>,
    pub child_pid: Mutex<Option<u32>>,
}

impl TranslateState {
    pub fn new() -> Self {
        Self {
            running: Mutex::new(false),
            child_pid: Mutex::new(None),
        }
    }
}

#[derive(Serialize, Clone)]
struct TranslateLogEvent {
    text: String,
    kind: String, // "info" | "text" | "error" | "result"
    done: bool,
}

#[derive(Serialize)]
pub struct TranslateResult {
    templates: usize, // 实际写回译文的模板数
    units: usize,     // 模板 × 语言 的翻译条目总数
    batches: usize,
    warnings: Vec<String>,
}

/// 结构化进度（给前端画进度条）：done/total 是「译文单元」(模板×语言) 计数。
#[derive(Serialize, Clone)]
struct TranslateProgress {
    total: usize,
    done: usize,
}

fn emit_log(app: &AppHandle, text: &str, kind: &str, done: bool) {
    app.emit(
        "translate-log",
        TranslateLogEvent {
            text: text.to_string(),
            kind: kind.to_string(),
            done,
        },
    )
    .ok();
}

fn emit_progress(app: &AppHandle, total: usize, done: usize) {
    app.emit("translate-progress", TranslateProgress { total, done }).ok();
}

/// 一条模板本批要翻的活：源语言 + 正文 + 目标语言列表（已按覆盖/补缺失算好）。
struct Job {
    id: String,
    lang: String,
    text: String,
    target_langs: Vec<String>,
}

#[tauri::command]
pub async fn translate_templates(
    product: String,
    ids: Option<Vec<String>>,
    langs: Vec<String>,
    overwrite: bool,
    channel: Option<String>,
    namespace: Option<String>,
    model: Option<String>,
    app: AppHandle,
    state: State<'_, TranslateState>,
) -> Result<TranslateResult, String> {
    {
        let mut running = state.running.lock().unwrap();
        if *running {
            return Err("已有翻译任务在进行中。".into());
        }
        *running = true;
    }
    let result =
        translate_inner(product, ids, langs, overwrite, channel, namespace, model, app.clone()).await;
    *state.running.lock().unwrap() = false;
    *state.child_pid.lock().unwrap() = None;
    match &result {
        Ok(r) => emit_log(
            &app,
            &format!("完成：写回 {} 条模板、{} 条译文。", r.templates, r.units),
            "result",
            true,
        ),
        Err(e) if e == "CANCELLED" => emit_log(&app, "已取消（已完成的批次已保存）。", "info", true),
        Err(e) => emit_log(&app, &format!("失败：{}", e), "error", true),
    }
    result
}

#[tauri::command]
pub async fn stop_translate(state: State<'_, TranslateState>) -> Result<(), String> {
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
        .map_err(|e| format!("停止翻译失败：{}", e))
}

async fn translate_inner(
    product: String,
    ids: Option<Vec<String>>,
    langs: Vec<String>,
    overwrite: bool,
    channel: Option<String>,
    namespace: Option<String>,
    model: Option<String>,
    app: AppHandle,
) -> Result<TranslateResult, String> {
    let channel = channel.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "gp".into());
    let ns = namespace.as_deref().unwrap_or("gp").to_string();
    let langs: Vec<String> = langs.into_iter().filter(|s| !s.trim().is_empty()).collect();
    if langs.is_empty() {
        return Err("没有选择任何目标语言。".into());
    }

    let all = crate::templates::load_templates_for(&product, &ns)?;
    let id_filter: Option<std::collections::HashSet<String>> =
        ids.map(|v| v.into_iter().collect());

    // 为每条模板算出本次真正要翻的语言（排除等于源语言的码；overwrite=false 时跳过已有）。
    let mut jobs: Vec<Job> = Vec::new();
    for t in &all {
        if let Some(f) = &id_filter {
            if !f.contains(&t.id) {
                continue;
            }
        }
        let targets: Vec<String> = langs
            .iter()
            .filter(|l| !crate::templates::is_source_lang(l, &t.lang))
            .filter(|l| overwrite || !t.translations.contains_key(*l))
            .cloned()
            .collect();
        if targets.is_empty() {
            continue;
        }
        jobs.push(Job {
            id: t.id.clone(),
            lang: t.lang.clone(),
            text: t.text.clone(),
            target_langs: targets,
        });
    }

    if jobs.is_empty() {
        return Ok(TranslateResult {
            templates: 0,
            units: 0,
            batches: 0,
            warnings: vec!["没有需要翻译的内容（可能都已有译文）。".into()],
        });
    }

    let total_units: usize = jobs.iter().map(|j| j.target_langs.len()).sum();
    let total_batches = jobs.len().div_ceil(CHUNK);
    emit_log(
        &app,
        &format!(
            "{}：{} 条模板、{} 条译文，每批 {} 条、共 {} 批。",
            product, jobs.len(), total_units, CHUNK, total_batches
        ),
        "info",
        false,
    );
    emit_progress(&app, total_units, 0);

    let claude_path = find_claude()
        .ok_or("未找到 Claude CLI，请先安装：npm install -g @anthropic-ai/claude-code")?;
    let model = model.filter(|s| !s.trim().is_empty());

    let mut done_templates = 0usize;
    let mut done_units = 0usize;
    let mut warnings: Vec<String> = Vec::new();

    for (bi, chunk) in jobs.chunks(CHUNK).enumerate() {
        if !*app.state::<TranslateState>().running.lock().unwrap() {
            return Err("CANCELLED".into());
        }
        emit_log(
            &app,
            &format!("翻译第 {}/{} 批（{} 条）…", bi + 1, total_batches, chunk.len()),
            "info",
            false,
        );

        let updates =
            translate_one_batch(&claude_path, &channel, chunk, model.as_deref(), &app).await?;

        // 增量写回：这一批立刻落盘，取消/失败也保住已完成的。
        crate::templates::apply_translations(&product, &ns, &updates)?;
        done_templates += updates.len();
        done_units += updates.values().map(|m| m.len()).sum::<usize>();
        // 警告本批漏翻的（模型没返回某些 id/语言）
        for j in chunk {
            match updates.get(&j.id) {
                None => warnings.push(format!("{} 未返回任何译文", j.id)),
                Some(got) => {
                    let miss: Vec<&str> = j
                        .target_langs
                        .iter()
                        .filter(|l| !got.contains_key(*l))
                        .map(|s| s.as_str())
                        .collect();
                    if !miss.is_empty() {
                        warnings.push(format!("{} 缺：{}", j.id, miss.join(",")));
                    }
                }
            }
        }
        emit_log(
            &app,
            &format!("第 {}/{} 批已写回（累计 {} 条译文）。", bi + 1, total_batches, done_units),
            "info",
            false,
        );
        emit_progress(&app, total_units, done_units);
    }

    Ok(TranslateResult {
        templates: done_templates,
        units: done_units,
        batches: total_batches,
        warnings,
    })
}

/// 把翻译纪律 + 这批模板内联成 prompt（不读任何文件、不暴露整个模板库）。
fn build_prompt(channel: &str, chunk: &[Job]) -> String {
    let tpls: Vec<serde_json::Value> = chunk
        .iter()
        .map(|j| {
            serde_json::json!({
                "id": j.id,
                "lang": j.lang,
                "text": j.text,
                "target_langs": j.target_langs,
            })
        })
        .collect();
    let tpls_json = serde_json::to_string(&tpls).unwrap_or_else(|_| "[]".into());
    let len_rule = if channel == "email" {
        "无长度限制，照实翻译。".to_string()
    } else {
        format!(
            "**硬上限 {GP_LIMIT} 字符**（含空格/标点/emoji），这是 Google Play 回复的硬限制，**绝不能超过 {GP_LIMIT}**。俄语/德语/法语/西语等通常比英文长 20-30%，翻译时就要主动精简用词、压缩到 {GP_LIMIT} 以内，宁可少说也不能超限。",
            GP_LIMIT = GP_LIMIT
        )
    };

    let scenario = if channel == "email" {
        "邮件客服回复模板"
    } else {
        "Google Play 应用商店回复模板"
    };
    format!(
        r#"你是专业本地化翻译。把下面的{scenario}从各自的源语言忠实翻译到它的每个目标语言。
不要使用任何工具，不要读写任何文件，直接输出结果。

【规则】
1. 忠实翻译，保持语义和语气（道歉/感谢/引导排查等），不增删、不编造。
2. 原样保留不翻译：邮箱、版本号、产品名（XFolder / MP3 Cutter / Video to MP3 / Android / Google Play）、emoji/表情、占位符（如 {{name}} / %s / %1$s）。
3. 长度：{len_rule}
4. 语言码用「app 原生码」，**原样**作为输出 key（如 zh-rCN / zh-rTW / in / kn-rIN 等 `*-rIN`），不要改写成 ISO 码。
5. 引号：译文里尽量不用 ASCII 直引号；要引用 UI 选项名时英文用 '...'，中文用「」。译文里若出现双引号必须转义成 \"，换行写成 \n。

【待翻译模板】（JSON 数组，每条含 id / lang=源语言 / text=源正文 / target_langs=要翻成的语言码）：
{tpls_json}

【输出】只输出一个 JSON 对象，不要任何额外文字、不要 markdown 代码块。形如：
{{"<id>": {{"<语言码>": "译文", ...}}, ...}}
每个 id 必须包含它 target_langs 里的全部语言码，键名一字不差。"#,
        len_rule = len_rule,
        tpls_json = tpls_json,
    )
}

/// 从可能被 markdown/散文包裹的输出里截取 JSON 对象：按括号深度配对（感知字符串/转义），
/// 避免结尾散文里出现的 '}' 把不相关内容也框进来。
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

/// 字符数（Unicode scalar 计；与前端 .length 对 BMP 字符一致）。
fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// 解析模型输出的 `{id:{lang:text}}` JSON。
fn parse_translations(raw: &str) -> Result<BTreeMap<String, BTreeMap<String, String>>, String> {
    let json_str = extract_json_object(raw).ok_or_else(|| {
        format!("模型输出里找不到 JSON 对象：{}", raw.chars().take(300).collect::<String>())
    })?;
    let parsed: serde_json::Value = serde_json::from_str(json_str)
        .or_else(|_| serde_json::from_str(&crate::json_repair::repair_json(json_str)))
        .map_err(|e| format!("译文不是合法 JSON：{}", e))?;
    let mut updates: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    if let Some(obj) = parsed.as_object() {
        for (id, langs_obj) in obj {
            if let Some(m) = langs_obj.as_object() {
                let mut tr = BTreeMap::new();
                for (lang, text) in m {
                    if let Some(s) = text.as_str() {
                        if !s.trim().is_empty() {
                            tr.insert(lang.clone(), s.to_string());
                        }
                    }
                }
                if !tr.is_empty() {
                    updates.insert(id.clone(), tr);
                }
            }
        }
    }
    Ok(updates)
}

/// 收集超出字符上限的译文 (id, lang, text)。
fn collect_over(
    updates: &BTreeMap<String, BTreeMap<String, String>>,
    limit: usize,
) -> Vec<(String, String, String)> {
    let mut out = vec![];
    for (id, m) in updates {
        for (lang, t) in m {
            if char_len(t) > limit {
                out.push((id.clone(), lang.clone(), t.clone()));
            }
        }
    }
    out
}

/// 压缩 prompt：把超长译文在不改语义下精简到 ≤limit 字符。
fn build_compress_prompt(items: &[(String, String, String)], limit: usize) -> String {
    let arr: Vec<serde_json::Value> = items
        .iter()
        .map(|(id, lang, text)| {
            serde_json::json!({ "id": id, "lang": lang, "text": text, "chars": char_len(text) })
        })
        .collect();
    let arr_json = serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into());
    format!(
        r#"下面这些译文超过了 {limit} 字符上限。在**不改变意思、保留邮箱/版本号/产品名/emoji/占位符**的前提下，把每条精简改写到 **≤{limit} 字符**（含空格/标点/emoji）：删冗余、用更短的同义说法，要点不能丢。不要使用任何工具，直接输出。

【待精简】（JSON 数组，`chars` 是当前字符数）：
{arr_json}

【输出】只输出一个 JSON 对象：{{"<id>":{{"<lang>":"精简后译文"}}}}，键名与输入一致。"#,
        limit = limit,
        arr_json = arr_json,
    )
}

/// 跑一次 `claude --print`（无 add-dir、禁工具），返回最终文本；用量 emit 到日志。
async fn run_claude_raw(
    claude_path: &str,
    prompt: &str,
    model: Option<&str>,
    app: &AppHandle,
) -> Result<String, String> {
    let mut args = vec![
        "--print".to_string(),
        "--verbose".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--permission-mode".to_string(),
        "bypassPermissions".to_string(),
    ];
    if let Some(m) = model {
        args.push("--model".to_string());
        args.push(m.to_string());
    }

    let mut cmd = Command::new(claude_path);
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(token) = load_claude_token() {
        cmd.env("CLAUDE_CODE_SESSION_ACCESS_TOKEN", &token);
    }

    let mut child = cmd.spawn().map_err(|e| format!("启动 claude 失败：{}", e))?;
    *app.state::<TranslateState>().child_pid.lock().unwrap() = child.id();

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(prompt.as_bytes()).await;
        drop(stdin);
    }

    let stdout = child.stdout.take().ok_or("无 stdout")?;
    let stderr = child.stderr.take().ok_or("无 stderr")?;

    let result_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let assistant_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let usage_cell: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
    let result_for = result_text.clone();
    let assistant_for = assistant_text.clone();
    let usage_for = usage_cell.clone();

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
                                if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                                        assistant_for.lock().unwrap().push_str(t);
                                    }
                                }
                            }
                        }
                    }
                    "result" => {
                        if let Some(r) = val.get("result").and_then(|v| v.as_str()) {
                            *result_for.lock().unwrap() = r.to_string();
                        }
                        if let Some(usage) = val.get("usage").filter(|u| u.is_object()) {
                            let mut u = usage.clone();
                            if let (Some(obj), Some(cost)) =
                                (u.as_object_mut(), val.get("total_cost_usd"))
                            {
                                obj.insert("total_cost_usd".to_string(), cost.clone());
                            }
                            *usage_for.lock().unwrap() = Some(u);
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

    let status = child.wait().await.map_err(|e| format!("claude 进程出错：{}", e))?;
    stdout_task.await.ok();
    stderr_task.await.ok();
    *app.state::<TranslateState>().child_pid.lock().unwrap() = None;

    if !*app.state::<TranslateState>().running.lock().unwrap() {
        return Err("CANCELLED".into());
    }
    if !status.success() {
        return Err(format!("claude 退出码 {}", status.code().unwrap_or(-1)));
    }

    if let Some(u) = usage_cell.lock().unwrap().as_ref() {
        let it = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let ot = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let cr = u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let cost = u.get("total_cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0);
        emit_log(
            app,
            &format!("本批用量：输入 {} · 输出 {} · 缓存读 {} tokens · 约 ${:.4}", it, ot, cr, cost),
            "info",
            false,
        );
    }

    Ok({
        let r = result_text.lock().unwrap().clone();
        if r.trim().is_empty() {
            assistant_text.lock().unwrap().clone()
        } else {
            r
        }
    })
}

/// 跑一批：内联 prompt 翻译 → 解析 → （gp）对超 350 字符的译文压缩重试一次 → 仍超的标红警告。
async fn translate_one_batch(
    claude_path: &str,
    channel: &str,
    chunk: &[Job],
    model: Option<&str>,
    app: &AppHandle,
) -> Result<BTreeMap<String, BTreeMap<String, String>>, String> {
    let raw = run_claude_raw(claude_path, &build_prompt(channel, chunk), model, app).await?;
    let mut updates = parse_translations(&raw)?;

    // gp 渠道：硬把关 350 字符。超长的发一次压缩调用改写到限内。
    if channel != "email" {
        let over = collect_over(&updates, GP_LIMIT);
        if !over.is_empty() {
            emit_log(
                app,
                &format!("{} 条译文超 {} 字符，压缩重试…", over.len(), GP_LIMIT),
                "info",
                false,
            );
            match run_claude_raw(claude_path, &build_compress_prompt(&over, GP_LIMIT), model, app)
                .await
            {
                Ok(raw2) => {
                    if let Ok(fixed) = parse_translations(&raw2) {
                        for (id, m) in fixed {
                            let e = updates.entry(id).or_default();
                            for (lang, text) in m {
                                e.insert(lang, text);
                            }
                        }
                    }
                }
                Err(e) if e == "CANCELLED" => return Err("CANCELLED".into()),
                Err(_) => {} // 压缩失败 → 保留原译文（下面会标红）
            }
            // 压缩后仍超的：写入但标红警告，提示人工精简。
            for (id, lang, t) in collect_over(&updates, GP_LIMIT) {
                emit_log(
                    app,
                    &format!("⚠ {}/{} 仍 {} 字符超 {}，需人工精简", id, lang, char_len(&t), GP_LIMIT),
                    "error",
                    false,
                );
            }
        }
    }
    Ok(updates)
}

//! 用例知识库：自由资料库 + 资料↔产品多对多关联。
//! 存储在 `~/.tester-app/knowledge/`，扁平存资料文件（按 docId 命名）。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

fn knowledge_dir() -> PathBuf {
    data_dir().join("knowledge")
}

fn index_path() -> PathBuf {
    knowledge_dir().join("index.json")
}

fn doc_path(id: &str) -> PathBuf {
    knowledge_dir().join("docs").join(format!("{}.md", id))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbProduct {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbDoc {
    pub id: String,
    pub name: String,
    #[serde(rename = "productIds")]
    pub product_ids: Vec<String>,
    pub scenes: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct KbIndex {
    products: Vec<KbProduct>,
    docs: Vec<KbDoc>,
}

fn load_index() -> KbIndex {
    let path = index_path();
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&content).unwrap_or_default()
}

fn save_index(index: &KbIndex) -> Result<(), String> {
    let dir = knowledge_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建知识库目录失败：{}", e))?;
    let json = serde_json::to_string_pretty(index).map_err(|e| e.to_string())?;
    std::fs::write(index_path(), json).map_err(|e| format!("写 index.json 失败：{}", e))?;
    Ok(())
}

fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}{:06}", secs % 1_000_000, nanos % 1_000_000)
}

// ── 产品 CRUD ────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn kb_list_products() -> Result<Vec<KbProduct>, String> {
    Ok(load_index().products)
}

#[tauri::command]
pub fn kb_create_product(name: String) -> Result<KbProduct, String> {
    let mut index = load_index();
    let id = new_id();
    let product = KbProduct {
        id: id.clone(),
        name: name.clone(),
    };
    index.products.push(product.clone());
    save_index(&index)?;
    Ok(product)
}

#[tauri::command]
pub fn kb_rename_product(id: String, name: String) -> Result<(), String> {
    let mut index = load_index();
    let p = index
        .products
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("产品不存在")?;
    p.name = name;
    save_index(&index)
}

#[tauri::command]
pub fn kb_delete_product(id: String) -> Result<(), String> {
    let mut index = load_index();
    index.products.retain(|p| p.id != id);
    // 从所有资料的 productIds 中摘除该 id（多对多，不删资料）
    for doc in index.docs.iter_mut() {
        doc.product_ids.retain(|pid| pid != &id);
    }
    save_index(&index)
}

#[tauri::command]
pub fn kb_reorder_products(ids: Vec<String>) -> Result<(), String> {
    let mut index = load_index();
    let mut reordered: Vec<KbProduct> = Vec::new();
    for id in &ids {
        if let Some(p) = index.products.iter().find(|p| &p.id == id) {
            reordered.push(p.clone());
        }
    }
    // 追加不在 ids 里的（防御）
    for p in &index.products {
        if !ids.contains(&p.id) {
            reordered.push(p.clone());
        }
    }
    index.products = reordered;
    save_index(&index)
}

// ── 资料 CRUD ────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn kb_list_docs() -> Result<Vec<KbDoc>, String> {
    Ok(load_index().docs)
}

#[tauri::command]
pub fn kb_read_doc(id: String) -> Result<String, String> {
    let path = doc_path(&id);
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("读取资料失败：{}", e)),
    }
}

#[tauri::command]
pub fn kb_save_doc(id: String, content: String) -> Result<(), String> {
    let docs_dir = knowledge_dir().join("docs");
    std::fs::create_dir_all(&docs_dir).map_err(|e| format!("创建 docs 目录失败：{}", e))?;
    std::fs::write(doc_path(&id), content).map_err(|e| format!("写资料失败：{}", e))?;
    Ok(())
}

#[tauri::command]
pub fn kb_create_doc(name: String, product_ids: Vec<String>) -> Result<KbDoc, String> {
    let mut index = load_index();
    let id = new_id();
    let doc = KbDoc {
        id: id.clone(),
        name: name.clone(),
        product_ids,
        scenes: vec!["testcase".to_string()],
    };
    index.docs.push(doc.clone());
    save_index(&index)?;
    // 创建空文件
    let docs_dir = knowledge_dir().join("docs");
    std::fs::create_dir_all(&docs_dir).map_err(|e| format!("创建 docs 目录失败：{}", e))?;
    if !doc_path(&id).exists() {
        std::fs::write(doc_path(&id), "").map_err(|e| format!("创建资料文件失败：{}", e))?;
    }
    Ok(doc)
}

#[tauri::command]
pub fn kb_rename_doc(id: String, name: String) -> Result<(), String> {
    let mut index = load_index();
    let doc = index
        .docs
        .iter_mut()
        .find(|d| d.id == id)
        .ok_or("资料不存在")?;
    doc.name = name;
    save_index(&index)
}

#[tauri::command]
pub fn kb_delete_doc(id: String) -> Result<(), String> {
    let mut index = load_index();
    index.docs.retain(|d| d.id != id);
    save_index(&index)?;
    let path = doc_path(&id);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("删除资料文件失败：{}", e))?;
    }
    Ok(())
}

// ── 关联管理 ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn kb_set_doc_products(doc_id: String, product_ids: Vec<String>) -> Result<(), String> {
    let mut index = load_index();
    let doc = index
        .docs
        .iter_mut()
        .find(|d| d.id == doc_id)
        .ok_or("资料不存在")?;
    doc.product_ids = product_ids;
    save_index(&index)
}

// ── Generate 消费 ─────────────────────────────────────────────────────────────

#[tauri::command]
pub fn kb_resolve_doc_paths(ids: Vec<String>) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    for id in &ids {
        let path = doc_path(id);
        if path.exists() {
            paths.push(path.to_string_lossy().to_string());
        }
    }
    Ok(paths)
}

// ── v2：反馈 → 偏好半自动起草 ─────────────────────────────────────────────────

/// 临时图片目录（粘贴的截图落盘到这里，供 claude 读取）。
fn distill_tmp_dir() -> PathBuf {
    data_dir().join("knowledge").join("distill-tmp")
}

/// 把前端粘贴的截图（base64，可能带 `data:image/png;base64,` 前缀）写到临时文件，
/// 返回磁盘路径。选文件走 dialog 插件直接拿路径，不经此命令。
#[tauri::command]
pub fn kb_save_temp_image(data_base64: String, ext: String) -> Result<String, String> {
    use base64::Engine;
    let b64 = data_base64
        .rsplit("base64,")
        .next()
        .unwrap_or(&data_base64);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("图片 base64 解码失败：{}", e))?;
    let dir = distill_tmp_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败：{}", e))?;
    let safe_ext = if ext.is_empty() { "png".to_string() } else { ext };
    let path = dir.join(format!("paste-{}.{}", new_id(), safe_ext));
    std::fs::write(&path, &bytes).map_err(|e| format!("写临时图片失败：{}", e))?;
    Ok(path.to_string_lossy().to_string())
}

const DISTILL_GENERAL_HINT: &str =
    "（无关联产品=通用偏好）按以下分区归类：写作风格 / 用例顺序 / 必测通则 / 去重 / 红线。";
const DISTILL_PRODUCT_HINT: &str =
    "（关联具体产品）按以下分区归类：关键业务规则 / 必测场景清单 / 需关联测试的点 / 模块命名约定。";

fn build_distill_prompt(
    image_paths: &[String],
    note: &str,
    existing_md: &str,
    is_product: bool,
) -> String {
    let mut p = String::from(
        "你是测试用例偏好库的整理助手。用户提供的图片里同时包含「AI 生成的用例」和「人工修改后的用例」，\
用户的说明会指出哪部分是 AI、哪部分是人工。\n\n\
任务：对比两者差异——人工每一处增/删/改，都反推 AI 当初理解错了或遗漏了什么，再把这些结论沉淀成正向的用例偏好约定。\n\n\
严格执行 4 件事：\n\
1. 反写：把\"理解错/没考虑到\"改写成肯定的约定句。\n\
2. 分类：",
    );
    p.push_str(if is_product {
        DISTILL_PRODUCT_HINT
    } else {
        DISTILL_GENERAL_HINT
    });
    p.push_str(
        "\n\
3. 归并去重：与已有偏好同义的合并，不重复列。\n\
4. 不确定留空：人工改了但看不出业务原因的，用 <待补：...> 占位，严禁编造版本号/价格/数值/业务规则。\n\n",
    );

    p.push_str("【图片（含 AI 版与人工版用例）】\n");
    for path in image_paths {
        p.push_str(&format!("- {}\n", path));
    }
    p.push_str("\n【用户说明】\n");
    p.push_str(if note.trim().is_empty() {
        "（用户未补充说明，请自行从图片判断哪部分是 AI、哪部分是人工）"
    } else {
        note.trim()
    });
    p.push_str("\n\n【已有偏好 md】\n");
    if existing_md.trim().is_empty() {
        p.push_str("（空，全新起草）");
    } else {
        p.push_str(existing_md.trim());
    }
    p.push_str(
        "\n\n输出要求：\n\
- 输出完整 markdown（已有内容 + 合并后的新内容），可整体替换编辑器。\n\
- 相比已有偏好，所有新增/改动的行前加 🆕 标记，方便人工 review。\n\
- 只输出 markdown 正文，不要任何解释、不要用代码块包裹。\n",
    );
    p
}

/// AI 起草/合并：读对比图 → 提炼人工改动背后的偏好 → 合并进现有 md，新增行标 🆕。
/// 走 `claude --print stream-json`，收集最终文本返回（参考 reply.rs::generate_single_reply）。
#[tauri::command]
pub async fn kb_ai_distill(
    image_paths: Vec<String>,
    note: String,
    existing_md: String,
    is_product: bool,
    model: Option<String>,
) -> Result<String, String> {
    if image_paths.is_empty() {
        return Err("请至少提供一张对比图。".to_string());
    }

    let prompt = build_distill_prompt(&image_paths, &note, &existing_md, is_product);

    let claude_path = crate::claude::find_claude()
        .ok_or("未找到 Claude CLI，请先安装：npm install -g @anthropic-ai/claude-code")?;

    let mut args = vec![
        "--print".to_string(),
        "--verbose".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--permission-mode".to_string(),
        "bypassPermissions".to_string(),
    ];
    // 未指定模型时回退到用例模型设置（出厂默认 claude-sonnet-4-6）。
    let model = model
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::model_config::load().testcase);
    if !model.is_empty() {
        args.push("--model".to_string());
        args.push(model);
    }
    // 图片父目录加进 --add-dir，claude 才能读到本地文件
    let mut dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for p in &image_paths {
        if let Some(parent) = std::path::Path::new(p).parent() {
            dirs.insert(parent.to_string_lossy().to_string());
        }
    }
    for d in &dirs {
        args.push("--add-dir".to_string());
        args.push(d.clone());
    }

    let mut cmd = Command::new(&claude_path);
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(token) = crate::claude::load_claude_token() {
        cmd.env("CLAUDE_CODE_SESSION_ACCESS_TOKEN", &token);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 claude 失败：{}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes()).await;
        drop(stdin);
    }

    let stdout = child.stdout.take().ok_or("无 stdout")?;
    let stderr = child.stderr.take().ok_or("无 stderr")?;

    let result_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let assistant_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let result_for_task = result_text.clone();
    let assistant_for_task = assistant_text.clone();

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
                    }
                    _ => {}
                }
            }
        }
    });

    let stderr_task = tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("[distill stderr] {}", line);
        }
    });

    let status = child.wait().await.map_err(|e| e.to_string())?;
    stdout_task.await.ok();
    stderr_task.await.ok();

    let final_text = {
        let r = result_text.lock().unwrap().clone();
        if !r.is_empty() {
            r
        } else {
            assistant_text.lock().unwrap().clone()
        }
    };
    if final_text.is_empty() {
        if !status.success() {
            return Err("Claude 进程异常退出且无输出".to_string());
        }
        return Err("Claude 未返回内容".to_string());
    }
    Ok(final_text)
}

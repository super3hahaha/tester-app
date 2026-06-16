//! 模板管理：把 Google Play 回复模板存在 app 本地（`~/.tester-app/templates/`），
//! 提供分产品的增删改 + xlsx 批量导入。review-reply skill 运行时从这个目录读
//! （路径由 reply.rs 通过 prompt + `--add-dir` 传给 skill）。
//!
//! 「索引模式」：`templates.json` 是含全文的权威源，`index.json` 是 id+category 的
//! 瘦身索引（skill 匹配阶段只读它，命中后才回 templates.json 取全文）。index 永远
//! 由全文派生、自动重建（见 `write_templates_and_index`），不再依赖 python build。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use calamine::{open_workbook_auto, Data, Reader};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct Template {
    pub id: String,
    pub category: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Default)]
struct ProductTemplates {
    templates: Vec<Template>,
}

#[derive(Serialize, Deserialize)]
struct TemplatesFile {
    #[serde(default)]
    version: String,
    #[serde(default)]
    source_file: String,
    #[serde(default)]
    products: BTreeMap<String, ProductTemplates>,
}

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

/// 模板数据目录 `~/.tester-app/templates/`（reply.rs 也用它告诉 skill 去哪读）。
pub fn templates_dir() -> PathBuf {
    data_dir().join("templates")
}

/// skill_sync 把 review-reply 下载到这里，首次迁移从它拷种子。
fn skill_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join(".claude")
        .join("skills")
        .join("review-reply")
        .join("data")
}

fn now_secs() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

/// 确保模板目录存在并已初始化。首次（无 templates.json）从 skill 已同步的 data/
/// 拷三个 json 作种子；连种子也没有就建空结构。返回目录路径。
pub fn ensure_templates_dir() -> Result<PathBuf, String> {
    let dir = templates_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建模板目录失败：{}", e))?;

    let tpl = dir.join("templates.json");
    if !tpl.exists() {
        let seed = skill_data_dir();
        for f in ["templates.json", "index.json", "package_map.json"] {
            let src = seed.join(f);
            let dst = dir.join(f);
            if src.exists() && !dst.exists() {
                let _ = std::fs::copy(&src, &dst);
            }
        }
    }
    // 仍没有 templates.json（skill 未同步过）→ 建空结构
    if !tpl.exists() {
        let mut empty = TemplatesFile {
            version: now_secs(),
            source_file: String::new(),
            products: BTreeMap::new(),
        };
        write_templates_and_index(&mut empty)?;
    } else if !dir.join("index.json").exists() {
        // 有全文但缺索引（种子不全）→ 由全文重建一次
        let mut f = read_templates()?;
        write_templates_and_index(&mut f)?;
    }
    Ok(dir)
}

fn read_templates() -> Result<TemplatesFile, String> {
    let dir = templates_dir();
    let s = std::fs::read_to_string(dir.join("templates.json"))
        .map_err(|e| format!("读取 templates.json 失败：{}", e))?;
    serde_json::from_str(&s).map_err(|e| format!("templates.json 不是合法 JSON：{}", e))
}

/// 唯一的写出口：写 templates.json 的同时由全文派生重建 index.json，二者永远一致。
fn write_templates_and_index(f: &mut TemplatesFile) -> Result<(), String> {
    let dir = templates_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建模板目录失败：{}", e))?;
    f.version = now_secs();

    let tpl_str =
        serde_json::to_string_pretty(f).map_err(|e| format!("序列化 templates 失败：{}", e))?;
    std::fs::write(dir.join("templates.json"), tpl_str)
        .map_err(|e| format!("写 templates.json 失败：{}", e))?;

    let mut idx_products = serde_json::Map::new();
    for (prod, pt) in &f.products {
        let entries: Vec<serde_json::Value> = pt
            .templates
            .iter()
            .map(|t| serde_json::json!({ "id": t.id, "category": t.category }))
            .collect();
        idx_products.insert(prod.clone(), serde_json::json!({ "templates": entries }));
    }
    let idx = serde_json::json!({
        "version": f.version,
        "note": "匹配索引：全部模板的 id+category（无全文）。由 tester-app 模板管理自动重建。匹配只读这个，命中后按 id 从 templates.json 取全文。",
        "products": idx_products,
    });
    std::fs::write(
        dir.join("index.json"),
        serde_json::to_string_pretty(&idx).map_err(|e| format!("序列化 index 失败：{}", e))?,
    )
    .map_err(|e| format!("写 index.json 失败：{}", e))?;
    Ok(())
}

fn slug(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// 该产品的 id 前缀：优先沿用现有模板的前缀，否则用已知映射，最后退回 slug。
fn product_prefix(product: &str, templates: &[Template]) -> String {
    if let Some(t) = templates.first() {
        if let Some(i) = t.id.rfind('-') {
            return t.id[..i].to_string();
        }
    }
    match product {
        "XFolder" => "xfolder".to_string(),
        "MP3 Cutter" => "mp3cutter".to_string(),
        "Video to MP3" => "video2mp3".to_string(),
        _ => slug(product),
    }
}

fn max_seq(templates: &[Template]) -> u32 {
    templates
        .iter()
        .filter_map(|t| t.id.rsplit('-').next().and_then(|n| n.parse::<u32>().ok()))
        .max()
        .unwrap_or(0)
}

fn next_id(product: &str, templates: &[Template]) -> String {
    let prefix = product_prefix(product, templates);
    format!("{}-{:03}", prefix, max_seq(templates) + 1)
}

// ── package_map（包名↔产品）：第一期只读，用于在列表里显示产品关联的 app ──

fn read_package_map() -> Option<serde_json::Value> {
    let s = std::fs::read_to_string(templates_dir().join("package_map.json")).ok()?;
    serde_json::from_str(&s).ok()
}

fn apps_for_product(pkgmap: &Option<serde_json::Value>, product: &str) -> Vec<String> {
    let mut out = vec![];
    if let Some(m) = pkgmap.as_ref().and_then(|v| v.get("mapping")).and_then(|v| v.as_object()) {
        for entry in m.values() {
            if entry.get("product").and_then(|p| p.as_str()) == Some(product) {
                if let Some(d) = entry.get("display").and_then(|d| d.as_str()) {
                    out.push(d.to_string());
                }
            }
        }
    }
    out
}

// ── 命令 ──

#[derive(Serialize)]
pub struct ProductInfo {
    product: String,
    count: usize,
    apps: Vec<String>,
}

#[tauri::command]
pub fn list_template_products() -> Result<Vec<ProductInfo>, String> {
    ensure_templates_dir()?;
    let f = read_templates()?;
    let pkgmap = read_package_map();
    Ok(f
        .products
        .iter()
        .map(|(prod, pt)| ProductInfo {
            product: prod.clone(),
            count: pt.templates.len(),
            apps: apps_for_product(&pkgmap, prod),
        })
        .collect())
}

#[tauri::command]
pub fn list_templates(product: String) -> Result<Vec<Template>, String> {
    ensure_templates_dir()?;
    let f = read_templates()?;
    Ok(f.products.get(&product).map(|p| p.templates.clone()).unwrap_or_default())
}

#[tauri::command]
pub fn add_template(product: String, category: String, text: String) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("模板正文不能为空。".into());
    }
    ensure_templates_dir()?;
    let mut f = read_templates()?;
    let pt = f.products.entry(product.clone()).or_default();
    let id = next_id(&product, &pt.templates);
    pt.templates.push(Template {
        id: id.clone(),
        category: category.trim().to_string(),
        text: text.trim().to_string(),
    });
    write_templates_and_index(&mut f)?;
    Ok(id)
}

#[tauri::command]
pub fn update_template(
    product: String,
    id: String,
    category: String,
    text: String,
) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("模板正文不能为空。".into());
    }
    ensure_templates_dir()?;
    let mut f = read_templates()?;
    let pt = f.products.get_mut(&product).ok_or("产品不存在")?;
    let t = pt.templates.iter_mut().find(|t| t.id == id).ok_or("模板不存在")?;
    t.category = category.trim().to_string();
    t.text = text.trim().to_string();
    write_templates_and_index(&mut f)?;
    Ok(())
}

#[tauri::command]
pub fn delete_template(product: String, id: String) -> Result<(), String> {
    ensure_templates_dir()?;
    let mut f = read_templates()?;
    let pt = f.products.get_mut(&product).ok_or("产品不存在")?;
    let before = pt.templates.len();
    pt.templates.retain(|t| t.id != id);
    if pt.templates.len() == before {
        return Err("模板不存在".into());
    }
    write_templates_and_index(&mut f)?;
    Ok(())
}

#[derive(Serialize)]
pub struct ImportResult {
    count: usize,
    sheet: String,
    warning: Option<String>,
}

fn cell_to_string(d: &Data) -> String {
    match d {
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            if f.fract() == 0.0 {
                format!("{}", *f as i64)
            } else {
                f.to_string()
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        _ => String::new(),
    }
}

/// 从 xlsx 批量导入到指定产品（**覆盖**该产品现有模板）。解析口径与原 build 脚本一致：
/// A 列类别（空则继承上一行）、B 列英文模板（空则跳过）、C 列起忽略、无表头。
/// 选哪个 sheet：名字归一化（去空格转小写）后与产品名匹配，匹配不到用第一个 sheet。
#[tauri::command]
pub fn import_templates_xlsx(product: String, path: String) -> Result<ImportResult, String> {
    ensure_templates_dir()?;
    let mut wb = open_workbook_auto(&path).map_err(|e| format!("打开 xlsx 失败：{}", e))?;

    let norm = |s: &str| -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase()
    };
    let want = norm(&product);
    let names = wb.sheet_names().to_vec();
    let mut warning = None;
    let sheet_name = names
        .iter()
        .find(|n| norm(n) == want)
        .cloned()
        .or_else(|| {
            warning = Some(format!(
                "未找到与「{}」匹配的 sheet，已用第一个 sheet「{}」",
                product,
                names.first().cloned().unwrap_or_default()
            ));
            names.first().cloned()
        })
        .ok_or("xlsx 里没有任何工作表")?;

    let range = wb
        .worksheet_range(&sheet_name)
        .map_err(|e| format!("读取工作表「{}」失败：{}", sheet_name, e))?;

    // 沿用该产品现有前缀（覆盖后重新从 001 编号）
    let existing = {
        let f = read_templates()?;
        f.products.get(&product).map(|p| p.templates.clone()).unwrap_or_default()
    };
    let prefix = product_prefix(&product, &existing);

    let mut current_category = String::new();
    let mut counter: u32 = 0;
    let mut imported: Vec<Template> = vec![];
    for row in range.rows() {
        let category = row.first().map(cell_to_string).unwrap_or_default();
        let english = row.get(1).map(cell_to_string).unwrap_or_default();
        let category = category.trim();
        let english = english.trim();
        if !category.is_empty() {
            current_category = category.to_string();
        }
        if english.is_empty() {
            continue;
        }
        counter += 1;
        imported.push(Template {
            id: format!("{}-{:03}", prefix, counter),
            category: if current_category.is_empty() {
                "未分类".to_string()
            } else {
                current_category.clone()
            },
            text: english.to_string(),
        });
    }

    if imported.is_empty() {
        return Err(format!("从 sheet「{}」没解析出任何模板（A 列类别 / B 列英文）", sheet_name));
    }

    let count = imported.len();
    let mut f = read_templates()?;
    f.products.insert(product, ProductTemplates { templates: imported });
    write_templates_and_index(&mut f)?;
    Ok(ImportResult { count, sheet: sheet_name, warning })
}

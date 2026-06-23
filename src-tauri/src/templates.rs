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
use rust_xlsxwriter::{Format, Workbook};
use serde::{Deserialize, Serialize};

fn default_lang() -> String {
    "en".to_string()
}

/// 把任意语言码规范成模板支持的源语言：en 或 zh-CN（其它一律退回 en）。
fn norm_lang(lang: &str) -> String {
    let l = lang.trim().to_lowercase();
    if l.starts_with("zh") {
        "zh-CN".to_string()
    } else {
        "en".to_string()
    }
}

/// 源文指纹：译文写回时记下当时 text 的 hash（存进 src_hash）。之后 text 变了、
/// hash 对不上 → 译文「过期」（stale），UI 提示重译。见 `apply_translations`。
pub(crate) fn text_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    format!("{:x}", h.finalize())
}

/// 目标语言码（app 原生码，如 zh-rCN）是否其实就是模板的源语言——是的话不该翻译、
/// 也不进 translations（查询时归一到源语言直接用 text）。源语言只有 en / zh-CN。
pub(crate) fn is_source_lang(target_code: &str, source_lang: &str) -> bool {
    let t = target_code.trim().to_lowercase();
    match source_lang {
        "zh-CN" => t == "zh-rcn",
        _ => t == "en", // 源默认 en
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Template {
    pub id: String,
    pub category: String,
    pub text: String,
    // 源语言（en / zh-CN）。skill 命中后据此翻到回复语言（相同则直接用）。
    // 旧数据无此字段 → 默认 en（存量 302 条都是英文源）。
    #[serde(default = "default_lang")]
    pub lang: String,
    // 预翻译的各语言译文：app 原生语言码（ar/ru/zh-rCN/...）→ 译文，不含源语言。
    // 由 template-translate skill 生成，review-reply 命中后直接取用、省掉运行时翻译。
    #[serde(default)]
    pub translations: BTreeMap<String, String>,
    // 译文对应的源文指纹（翻译当时 text 的 hash）。text 改了对不上 → 译文过期。
    #[serde(default)]
    pub src_hash: String,
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

/// namespace → 存储目录。"email" → `~/.tester-app/email-templates/`，其它 → `~/.tester-app/templates/`。
fn ns_dir(namespace: &str) -> PathBuf {
    if namespace == "email" {
        data_dir().join("email-templates")
    } else {
        data_dir().join("templates")
    }
}

/// 模板数据目录（review/GP 用）`~/.tester-app/templates/`（reply.rs 也用它告诉 skill 去哪读）。
pub fn templates_dir() -> PathBuf {
    ns_dir("gp")
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

/// 确保指定 namespace 的模板目录存在并已初始化。
/// GP（默认）：首次从 review-reply skill data/ 拷种子；email：直接建空结构。
pub fn ensure_templates_dir_ns(namespace: &str) -> Result<PathBuf, String> {
    let dir = ns_dir(namespace);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建模板目录失败：{}", e))?;

    let tpl = dir.join("templates.json");
    if !tpl.exists() && namespace != "email" {
        let seed = skill_data_dir();
        for f in ["templates.json", "index.json", "package_map.json"] {
            let src = seed.join(f);
            let dst = dir.join(f);
            if src.exists() && !dst.exists() {
                let _ = std::fs::copy(&src, &dst);
            }
        }
    }
    if !tpl.exists() {
        let mut empty = TemplatesFile {
            version: now_secs(),
            source_file: String::new(),
            products: BTreeMap::new(),
        };
        write_templates_and_index_to(&dir, &mut empty)?;
    } else if !dir.join("index.json").exists() {
        let mut f = read_templates_from(&dir)?;
        write_templates_and_index_to(&dir, &mut f)?;
    }
    Ok(dir)
}

/// 向后兼容：不传 namespace 时用 GP 目录。
pub fn ensure_templates_dir() -> Result<PathBuf, String> {
    ensure_templates_dir_ns("gp")
}

fn read_templates_from(dir: &PathBuf) -> Result<TemplatesFile, String> {
    let s = std::fs::read_to_string(dir.join("templates.json"))
        .map_err(|e| format!("读取 templates.json 失败：{}", e))?;
    serde_json::from_str(&s).map_err(|e| format!("templates.json 不是合法 JSON：{}", e))
}

fn read_templates_ns(namespace: &str) -> Result<TemplatesFile, String> {
    read_templates_from(&ns_dir(namespace))
}

fn write_templates_and_index_to(dir: &PathBuf, f: &mut TemplatesFile) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建模板目录失败：{}", e))?;
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

fn write_templates_and_index_ns(namespace: &str, f: &mut TemplatesFile) -> Result<(), String> {
    write_templates_and_index_to(&ns_dir(namespace), f)
}

/// 向后兼容（reply.rs 里只用 GP 模板目录）。
fn read_templates() -> Result<TemplatesFile, String> {
    read_templates_ns("gp")
}
fn write_templates_and_index(f: &mut TemplatesFile) -> Result<(), String> {
    write_templates_and_index_ns("gp", f)
}

pub(crate) fn slug(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// 该产品的 id 前缀：优先沿用现有模板的前缀，否则退回 slug。
pub(crate) fn product_prefix(product: &str, templates: &[Template]) -> String {
    if let Some(t) = templates.first() {
        if let Some(i) = t.id.rfind('-') {
            return t.id[..i].to_string();
        }
    }
    match product {
        // 中文名兜底规则（非具体产品）：纯中文名 slug 会为空，给「通用」一个稳定前缀。
        "通用" => "common".to_string(),
        _ => {
            let s = slug(product);
            if s.is_empty() {
                "tpl".to_string() // 纯非 ASCII 产品名兜底，避免空前缀生成 "-001"
            } else {
                s
            }
        }
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

// ── package_map（包名↔产品）：只有 GP namespace 有，email 直接返回 None ──

fn read_package_map_ns(namespace: &str) -> Option<serde_json::Value> {
    let s = std::fs::read_to_string(ns_dir(namespace).join("package_map.json")).ok()?;
    serde_json::from_str(&s).ok()
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PackageMappingEntry {
    pub package: String,
    pub display: String,
    pub product: Option<String>,
}

#[tauri::command]
pub fn get_package_map() -> Result<Vec<PackageMappingEntry>, String> {
    let pkgmap = read_package_map_ns("gp");
    let mut out = vec![];
    if let Some(m) = pkgmap.as_ref().and_then(|v| v.get("mapping")).and_then(|v| v.as_object()) {
        for (pkg, entry) in m {
            let display = entry.get("display").and_then(|d| d.as_str()).unwrap_or("").to_string();
            let product = entry.get("product").and_then(|p| p.as_str()).map(|s| s.to_string());
            out.push(PackageMappingEntry { package: pkg.clone(), display, product });
        }
    }
    Ok(out)
}

#[tauri::command]
pub fn save_package_map(entries: Vec<PackageMappingEntry>) -> Result<(), String> {
    let path = ns_dir("gp").join("package_map.json");
    let existing_raw = std::fs::read_to_string(&path).unwrap_or_default();
    let mut root: serde_json::Value = serde_json::from_str(&existing_raw)
        .unwrap_or_else(|_| serde_json::json!({ "mapping": {} }));

    let mut mapping = serde_json::Map::new();
    for e in &entries {
        let val = serde_json::json!({
            "product": e.product,
            "display": e.display,
        });
        mapping.insert(e.package.clone(), val);
    }
    root["mapping"] = serde_json::Value::Object(mapping);

    let s = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&path, s).map_err(|e| e.to_string())?;
    Ok(())
}

fn apps_for_product_from(pkgmap: &Option<serde_json::Value>, product: &str) -> Vec<String> {
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
    pub(crate) product: String,
    pub(crate) count: usize,
    pub(crate) apps: Vec<String>,
}

#[tauri::command]
pub fn list_template_products(namespace: Option<String>) -> Result<Vec<ProductInfo>, String> {
    let ns = namespace.as_deref().unwrap_or("gp");
    ensure_templates_dir_ns(ns)?;
    let f = read_templates_ns(ns)?;
    let pkgmap = read_package_map_ns(ns);
    Ok(f
        .products
        .iter()
        .map(|(prod, pt)| ProductInfo {
            product: prod.clone(),
            count: pt.templates.len(),
            apps: apps_for_product_from(&pkgmap, prod),
        })
        .collect())
}

/// 创建空产品（若已存在则幂等）。前端「新建产品」时立刻落盘，不再依赖加第一条模板才持久化。
#[tauri::command]
pub fn create_template_product(product: String, namespace: Option<String>) -> Result<(), String> {
    let product = product.trim().to_string();
    if product.is_empty() {
        return Err("产品名不能为空。".into());
    }
    let ns = namespace.as_deref().unwrap_or("gp");
    ensure_templates_dir_ns(ns)?;
    let mut f = read_templates_ns(ns)?;
    f.products.entry(product).or_default();
    write_templates_and_index_ns(ns, &mut f)?;
    Ok(())
}

/// 按包名查它对应的模板产品（XFolder/MP3 Cutter/...）。返回 None 表示该应用没有
/// 模板产品（package_map 里 product=null，如 ringwall/xplayer），调用方据此禁用收录。
#[tauri::command]
pub fn product_for_package(package_name: String) -> Result<Option<String>, String> {
    ensure_templates_dir()?;
    let pkgmap = read_package_map_ns("gp");
    if let Some(m) = pkgmap
        .as_ref()
        .and_then(|v| v.get("mapping"))
        .and_then(|v| v.as_object())
    {
        if let Some(entry) = m.get(&package_name) {
            return Ok(entry
                .get("product")
                .and_then(|p| p.as_str())
                .map(|s| s.to_string()));
        }
    }
    Ok(None)
}

/// 给前端的视图：模板字段平铺 + 一个计算出来的 `stale`（译文是否过期）。
#[derive(Serialize)]
pub struct TemplateView {
    #[serde(flatten)]
    inner: Template,
    // 有译文但源文 hash 对不上（text 改过却没重译）→ true，UI 标过期提示重译。
    stale: bool,
}

#[tauri::command]
pub fn list_templates(product: String, namespace: Option<String>) -> Result<Vec<TemplateView>, String> {
    let ns = namespace.as_deref().unwrap_or("gp");
    ensure_templates_dir_ns(ns)?;
    let f = read_templates_ns(ns)?;
    let list = f
        .products
        .get(&product)
        .map(|p| p.templates.clone())
        .unwrap_or_default();
    Ok(list
        .into_iter()
        .map(|t| {
            let stale = !t.translations.is_empty() && t.src_hash != text_hash(&t.text);
            TemplateView { inner: t, stale }
        })
        .collect())
}

/// 读某产品的全部模板（原始 Template，含 translations）。供 translate.rs 用。
pub(crate) fn load_templates_for(product: &str, namespace: &str) -> Result<Vec<Template>, String> {
    ensure_templates_dir_ns(namespace)?;
    let f = read_templates_ns(namespace)?;
    Ok(f
        .products
        .get(product)
        .map(|p| p.templates.clone())
        .unwrap_or_default())
}

/// 把一批译文合并写回指定产品的模板（translate.rs 每批调一次，增量写盘）。
/// `updates`: id → { 语言码 → 译文 }。合并进各模板的 translations（覆盖同 key、保留其它），
/// 并把命中的模板 src_hash 刷成当前 text 的 hash（表示译文已对齐当前源）。
pub(crate) fn apply_translations(
    product: &str,
    namespace: &str,
    updates: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<(), String> {
    if updates.is_empty() {
        return Ok(());
    }
    let mut f = read_templates_ns(namespace)?;
    let pt = f.products.get_mut(product).ok_or("产品不存在")?;
    for t in pt.templates.iter_mut() {
        if let Some(tr) = updates.get(&t.id) {
            for (lang, text) in tr {
                t.translations.insert(lang.clone(), text.clone());
            }
            t.src_hash = text_hash(&t.text);
        }
    }
    write_templates_and_index_ns(namespace, &mut f)?;
    Ok(())
}

/// 手工编辑某条模板的某个语言内容（编辑行「已翻译语言下拉」里改了再保存）。
/// 选的是源语言（en/zh-CN，或归一等于源的 app 码）→ 改 text 本身并刷新 src_hash
/// （其它译文随之过期）；否则改对应的 translations 译文，不动 src_hash。
#[tauri::command]
pub fn set_template_translation(
    product: String,
    id: String,
    lang: String,
    text: String,
    namespace: Option<String>,
) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("内容不能为空。".into());
    }
    let ns = namespace.as_deref().unwrap_or("gp");
    ensure_templates_dir_ns(ns)?;
    let mut f = read_templates_ns(ns)?;
    let pt = f.products.get_mut(&product).ok_or("产品不存在")?;
    let t = pt.templates.iter_mut().find(|t| t.id == id).ok_or("模板不存在")?;
    if lang == t.lang || is_source_lang(&lang, &t.lang) {
        t.text = text.trim().to_string();
        t.src_hash = text_hash(&t.text);
    } else {
        t.translations.insert(lang, text.trim().to_string());
    }
    write_templates_and_index_ns(ns, &mut f)?;
    Ok(())
}

#[tauri::command]
pub fn add_template(
    product: String,
    category: String,
    text: String,
    lang: Option<String>,
    namespace: Option<String>,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("模板正文不能为空。".into());
    }
    let ns = namespace.as_deref().unwrap_or("gp");
    ensure_templates_dir_ns(ns)?;
    let mut f = read_templates_ns(ns)?;
    let pt = f.products.entry(product.clone()).or_default();
    let id = next_id(&product, &pt.templates);
    pt.templates.push(Template {
        id: id.clone(),
        category: category.trim().to_string(),
        text: text.trim().to_string(),
        lang: norm_lang(lang.as_deref().unwrap_or("en")),
        translations: BTreeMap::new(),
        src_hash: String::new(),
    });
    write_templates_and_index_ns(ns, &mut f)?;
    Ok(id)
}

#[tauri::command]
pub fn update_template(
    product: String,
    id: String,
    category: String,
    text: String,
    lang: Option<String>,
    namespace: Option<String>,
) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("模板正文不能为空。".into());
    }
    let ns = namespace.as_deref().unwrap_or("gp");
    ensure_templates_dir_ns(ns)?;
    let mut f = read_templates_ns(ns)?;
    let pt = f.products.get_mut(&product).ok_or("产品不存在")?;
    let t = pt.templates.iter_mut().find(|t| t.id == id).ok_or("模板不存在")?;
    t.category = category.trim().to_string();
    t.text = text.trim().to_string();
    if let Some(l) = lang {
        t.lang = norm_lang(&l);
    }
    write_templates_and_index_ns(ns, &mut f)?;
    Ok(())
}

#[tauri::command]
pub fn delete_template(product: String, id: String, namespace: Option<String>) -> Result<(), String> {
    let ns = namespace.as_deref().unwrap_or("gp");
    ensure_templates_dir_ns(ns)?;
    let mut f = read_templates_ns(ns)?;
    let pt = f.products.get_mut(&product).ok_or("产品不存在")?;
    let before = pt.templates.len();
    pt.templates.retain(|t| t.id != id);
    if pt.templates.len() == before {
        return Err("模板不存在".into());
    }
    write_templates_and_index_ns(ns, &mut f)?;
    Ok(())
}

/// 删除整个产品（含其下所有模板）。
#[tauri::command]
pub fn delete_template_product(product: String, namespace: Option<String>) -> Result<(), String> {
    let ns = namespace.as_deref().unwrap_or("gp");
    ensure_templates_dir_ns(ns)?;
    let mut f = read_templates_ns(ns)?;
    if f.products.remove(&product).is_none() {
        return Err("产品不存在".into());
    }
    write_templates_and_index_ns(ns, &mut f)?;
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
pub fn import_templates_xlsx(product: String, path: String, namespace: Option<String>) -> Result<ImportResult, String> {
    let ns = namespace.as_deref().unwrap_or("gp");
    ensure_templates_dir_ns(ns)?;
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
        let f = read_templates_ns(ns)?;
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
            lang: "en".to_string(), // xlsx B 列是英文源
            // 覆盖导入 = 全新源，译文清空（导入后需在「补全多语言」重新生成）。
            translations: BTreeMap::new(),
            src_hash: String::new(),
        });
    }

    if imported.is_empty() {
        return Err(format!("从 sheet「{}」没解析出任何模板（A 列类别 / B 列英文）", sheet_name));
    }

    let count = imported.len();
    let mut f = read_templates_ns(ns)?;
    f.products.insert(product, ProductTemplates { templates: imported });
    write_templates_and_index_ns(ns, &mut f)?;
    Ok(ImportResult { count, sheet: sheet_name, warning })
}

/// 导出某产品所有模板为 xlsx：A 列类别、B 列英文原文、C 列起按 other_langs 顺序填译文（无则空白）。
/// path 是目标文件完整路径（由前端通过 save dialog 获取）。
#[tauri::command]
pub fn export_templates_xlsx(
    product: String,
    path: String,
    other_langs: Vec<String>,
    namespace: Option<String>,
) -> Result<usize, String> {
    let ns = namespace.as_deref().unwrap_or("gp");
    let f = read_templates_ns(ns)?;
    let templates = f
        .products
        .get(&product)
        .map(|p| p.templates.as_slice())
        .unwrap_or_default();

    let mut wb = Workbook::new();
    let sheet = wb.add_worksheet();
    sheet.set_name(&product).ok();

    // 表头
    let bold = Format::new().set_bold();
    sheet.write_with_format(0, 0, "类别", &bold).map_err(|e| e.to_string())?;
    sheet.write_with_format(0, 1, "英文模板", &bold).map_err(|e| e.to_string())?;
    for (i, lang) in other_langs.iter().enumerate() {
        sheet.write_with_format(0, (2 + i) as u16, lang.as_str(), &bold).map_err(|e| e.to_string())?;
    }

    let mut last_category = String::new();
    for (row_idx, t) in templates.iter().enumerate() {
        let row = (row_idx + 1) as u32;
        // A 列：类别变了才写，否则留空（与导入口径保持一致）
        if t.category != last_category {
            sheet.write(row, 0, t.category.as_str()).map_err(|e| e.to_string())?;
            last_category = t.category.clone();
        }
        // B 列：英文原文
        sheet.write(row, 1, t.text.as_str()).map_err(|e| e.to_string())?;
        // C 列起：各语言译文
        for (i, lang) in other_langs.iter().enumerate() {
            let text = t.translations.get(lang).map(|s| s.as_str()).unwrap_or("");
            if !text.is_empty() {
                sheet.write(row, (2 + i) as u16, text).map_err(|e| e.to_string())?;
            }
        }
    }

    wb.save(&path).map_err(|e| format!("保存 xlsx 失败：{}", e))?;
    Ok(templates.len())
}

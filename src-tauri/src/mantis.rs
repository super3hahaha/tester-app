use rust_xlsxwriter::{Format, Workbook};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// MantisBT 连接配置：Base URL（如 https://bugs.example.com）+ 个人 API Token。
/// Token 在 Mantis「我的账户 → API 令牌」页生成，请求头用 `Authorization: <token>`
/// 直传（不加 Bearer 前缀）。持久化到 ~/.tester-app/mantis-config.json，全局唯一
/// 一份，不按 Google 账号隔离（Mantis 账号体系与本工具的 Google 登录无关）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MantisConfig {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_token: String,
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join(".tester-app")
        .join("mantis-config.json")
}

#[tauri::command]
pub fn get_mantis_config() -> MantisConfig {
    let path = config_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn save_mantis_config(config: MantisConfig) -> Result<(), String> {
    let path = config_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

/// 只取「协议 + 域名(+端口)」，丢弃任何路径。用户很容易从浏览器地址栏直接
/// 复制粘贴当前页面的完整 URL（比如 `.../view_all_bug_page.php`）当 Base URL
/// 填进来，如果照单全收会把接口路径拼错到那个页面路径后面去，导致请求打到
/// 一个不存在的地址、Mantis 只能兜底返回站点首页/登录页的 HTML（而不是我们
/// 要的 REST JSON），报错也会很难懂（JSON 解析失败但看不出原因）。
fn normalize_base(base_url: &str) -> String {
    let trimmed = base_url.trim();
    let scheme_end = trimmed.find("://").map(|i| i + 3).unwrap_or(0);
    match trimmed[scheme_end..].find('/') {
        Some(rel_idx) => trimmed[..scheme_end + rel_idx].to_string(),
        None => trimmed.trim_end_matches('/').to_string(),
    }
}

/// 走 `/api/rest/index.php/<route>` 而不是 `/api/rest/<route>`：后者依赖服务器
/// 开了 mod_rewrite/.htaccess 才能把「伪静态」路径转发到 index.php，实测目标
/// 实例（Apache，未启用对应 rewrite）对 `/api/rest/projects` 直接返回裸 404；
/// 直接打 `index.php/` 这个入口文件百分百可达，不依赖部署方是否配好了 rewrite。
async fn mantis_get(base_url: &str, api_token: &str, route_and_query: &str) -> Result<Value, String> {
    if base_url.trim().is_empty() || api_token.trim().is_empty() {
        return Err("请先填写 Mantis Base URL 和 API Token".to_string());
    }
    let url = format!("{}/api/rest/index.php{}", normalize_base(base_url), route_and_query);
    let resp = reqwest::Client::new()
        .get(&url)
        .header("Authorization", api_token)
        .send()
        .await
        .map_err(|e| format!("请求 Mantis 失败：{}", e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Mantis 返回 {}：{}", status.as_u16(), truncate(&text, 300)));
    }
    serde_json::from_str(&text)
        .map_err(|e| format!("解析 Mantis 响应失败：{}（原文：{}）", e, truncate(&text, 300)))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

fn as_id(v: Option<&Value>) -> Option<i64> {
    let v = v?;
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn as_str(v: Option<&Value>) -> String {
    v.and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 取 `{id, name}` 形态引用对象（category/project/version/fixed_in_version）的 name。
/// 这几个字段的 "name" 本身就是团队填的原始文本（如分类名/版本号），没有额外的
/// 本地化 label。
fn ref_name(obj: Option<&Value>) -> String {
    obj.and_then(|o| o.get("name")).and_then(|n| n.as_str()).unwrap_or("").to_string()
}

/// 取枚举类引用对象（status/severity/priority/reproducibility/view_state）的显示
/// 文本：这类字段的 "name" 是英文枚举 key（如 "closed"/"minor"），"label" 才是
/// 按 Mantis 语言设置本地化过的文本（如「已关闭」/「小错误」）——优先用 label。
fn enum_label(obj: Option<&Value>) -> String {
    let obj = match obj {
        Some(o) => o,
        None => return String::new(),
    };
    non_empty(as_str(obj.get("label"))).unwrap_or_else(|| as_str(obj.get("name")))
}

/// 取用户引用对象（reporter/handler/note 的 reporter）的显示名：优先用
/// "real_name"（团队填的真实姓名，如「张诗馨」），为空则退回登录用户名 "name"。
fn user_name(obj: Option<&Value>) -> String {
    let obj = match obj {
        Some(o) => o,
        None => return String::new(),
    };
    non_empty(as_str(obj.get("real_name"))).unwrap_or_else(|| as_str(obj.get("name")))
}

#[derive(Debug, Clone, Serialize)]
pub struct MantisProject {
    pub id: i64,
    pub name: String,
}

#[tauri::command]
pub async fn list_mantis_projects(base_url: String, api_token: String) -> Result<Vec<MantisProject>, String> {
    let data = mantis_get(&base_url, &api_token, "/projects").await?;
    let arr = data.get("projects").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    Ok(arr
        .iter()
        .filter_map(|p| {
            let id = as_id(p.get("id"))?;
            Some(MantisProject { id, name: as_str(p.get("name")) })
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct MantisIssueSummary {
    pub id: i64,
    pub summary: String,
    pub status: String,
    pub severity: String,
    pub priority: String,
    pub category: String,
    pub version: String,
    pub fixed_in_version: String,
    pub reporter: String,
    pub handler: String,
    pub updated_at: String,
}

fn parse_issue_summary(issue: &Value) -> Option<MantisIssueSummary> {
    Some(MantisIssueSummary {
        id: as_id(issue.get("id"))?,
        summary: as_str(issue.get("summary")),
        status: enum_label(issue.get("status")),
        severity: enum_label(issue.get("severity")),
        priority: enum_label(issue.get("priority")),
        category: ref_name(issue.get("category")),
        version: ref_name(issue.get("version")),
        fixed_in_version: ref_name(issue.get("fixed_in_version")),
        reporter: user_name(issue.get("reporter")),
        handler: user_name(issue.get("handler")),
        updated_at: as_str(issue.get("updated_at")),
    })
}

/// 拉某个项目下的全量 issue 列表（分页 250/页，上限 10 页 = 2500 条，超出静默
/// 截断——正常项目规模足够，真遇到再加）。**不在后端按版本筛选**：实测这条团队
/// 的 Mantis 里几乎没人填 Mantis 自带的「版本」字段（`version`/`fixed_in_version`
/// 常年是 null），版本号实际是手打在 summary 里的（如「#bug 2.3.5内测 必现 --
/// ...」），所以「按版本筛选」本质是对 summary 做关键字匹配，交给前端做（用户
/// 输入框即时过滤 / 从已拉到的 summary 里正则提取版本号做快捷筛选项），
/// 不做成后端的精确字段匹配——否则这条团队的数据会永远筛出空列表。
#[tauri::command]
pub async fn list_mantis_issues(
    base_url: String,
    api_token: String,
    project_id: i64,
) -> Result<Vec<MantisIssueSummary>, String> {
    let page_size = 250u32;
    let mut collected = Vec::new();
    for page in 1..=10u32 {
        let path = format!("/issues?project_id={}&page_size={}&page={}", project_id, page_size, page);
        let data = mantis_get(&base_url, &api_token, &path).await?;
        let arr = data.get("issues").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let got = arr.len();
        for issue in &arr {
            if let Some(summary) = parse_issue_summary(issue) {
                collected.push(summary);
            }
        }
        if (got as u32) < page_size {
            break;
        }
    }
    Ok(collected)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MantisNote {
    pub id: i64,
    pub reporter: String,
    pub text: String,
    pub created_at: String,
}

/// 项目自定义字段（如这条团队在用的「附注」——存的是版本号后缀 A/B/C 那种批次
/// 标识，跟 notes/「备注」评论列表完全是两回事）。字段名不固定，跟着 Mantis
/// 项目配置走，所以这里存成 name/value 对，不写死某个具体字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MantisCustomField {
    pub name: String,
    pub value: String,
}

/// 自定义字段的 value 在 Mantis REST API 里类型不固定（字符串/数字/布尔/
/// 复选框数组），统一转成字符串好落地到前端和 xlsx。
fn custom_field_value(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|x| custom_field_value(Some(x)))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Some(other) => other.to_string(),
    }
}

/// 导出已选 bug 为 xlsx，含完整详情（问题描述/重现步骤/补充信息/备注）——
/// Mantis 网页版自带的导出只有列表摘要字段，这个按钮存在的意义就是把详情也
/// 带出来，所以 issues 必须是前端逐条拉好的 MantisIssueDetail，而不是列表页
/// 那份 summary。
#[tauri::command]
pub fn export_mantis_issues_xlsx(
    issues: Vec<MantisIssueDetail>,
    path: String,
    base_url: String,
) -> Result<usize, String> {
    let mut wb = Workbook::new();
    let sheet = wb.add_worksheet();

    // base_url 是各人自己在「Mantis 连接配置」里填的实例地址（不同团队/不同
    // Mantis 部署域名不一样），链接跟着这个拼，不写死某一个域名。
    let base = base_url.trim_end_matches('/');

    let bold = Format::new().set_bold();
    let wrap = Format::new().set_text_wrap();

    // 自定义字段（如「附注」）不是固定列——名字跟着 Mantis 项目配置走，这里从
    // 实际导出的数据里按第一次出现的顺序收集列名，而不是写死某一个字段。
    let mut custom_field_names: Vec<String> = Vec::new();
    for issue in &issues {
        for cf in &issue.custom_fields {
            if !custom_field_names.contains(&cf.name) {
                custom_field_names.push(cf.name.clone());
            }
        }
    }

    let fixed_headers = [
        "ID", "标题", "状态", "严重程度", "优先级", "分类", "重现率", "版本", "修复于",
        "报告人", "处理人", "创建时间", "更新时间", "问题描述", "重现步骤", "补充信息", "备注",
    ];
    let custom_col_start = fixed_headers.len() as u16;
    let link_col = custom_col_start + custom_field_names.len() as u16;

    for (col, h) in fixed_headers.iter().enumerate() {
        sheet.write_with_format(0, col as u16, *h, &bold).map_err(|e| e.to_string())?;
    }
    for (i, name) in custom_field_names.iter().enumerate() {
        sheet.write_with_format(0, custom_col_start + i as u16, name.as_str(), &bold).map_err(|e| e.to_string())?;
    }
    sheet.write_with_format(0, link_col, "链接", &bold).map_err(|e| e.to_string())?;

    for (row_idx, issue) in issues.iter().enumerate() {
        let row = (row_idx + 1) as u32;
        let notes_text = issue
            .notes
            .iter()
            .map(|n| format!("[{} @ {}] {}", n.reporter, n.created_at, n.text))
            .collect::<Vec<_>>()
            .join("\n\n");

        sheet.write(row, 0, issue.id).map_err(|e| e.to_string())?;
        sheet.write(row, 1, issue.summary.as_str()).map_err(|e| e.to_string())?;
        sheet.write(row, 2, issue.status.as_str()).map_err(|e| e.to_string())?;
        sheet.write(row, 3, issue.severity.as_str()).map_err(|e| e.to_string())?;
        sheet.write(row, 4, issue.priority.as_str()).map_err(|e| e.to_string())?;
        sheet.write(row, 5, issue.category.as_str()).map_err(|e| e.to_string())?;
        sheet.write(row, 6, issue.reproducibility.as_str()).map_err(|e| e.to_string())?;
        sheet.write(row, 7, issue.version.as_str()).map_err(|e| e.to_string())?;
        sheet.write(row, 8, issue.fixed_in_version.as_str()).map_err(|e| e.to_string())?;
        sheet.write(row, 9, issue.reporter.as_str()).map_err(|e| e.to_string())?;
        sheet.write(row, 10, issue.handler.as_str()).map_err(|e| e.to_string())?;
        sheet.write(row, 11, issue.created_at.as_str()).map_err(|e| e.to_string())?;
        sheet.write(row, 12, issue.updated_at.as_str()).map_err(|e| e.to_string())?;
        sheet.write_with_format(row, 13, issue.description.as_str(), &wrap).map_err(|e| e.to_string())?;
        sheet.write_with_format(row, 14, issue.steps_to_reproduce.as_str(), &wrap).map_err(|e| e.to_string())?;
        sheet.write_with_format(row, 15, issue.additional_information.as_str(), &wrap).map_err(|e| e.to_string())?;
        sheet.write_with_format(row, 16, notes_text.as_str(), &wrap).map_err(|e| e.to_string())?;
        for (i, name) in custom_field_names.iter().enumerate() {
            if let Some(cf) = issue.custom_fields.iter().find(|cf| &cf.name == name) {
                sheet.write(row, custom_col_start + i as u16, cf.value.as_str()).map_err(|e| e.to_string())?;
            }
        }
        sheet
            .write_url(row, link_col, format!("{}/view.php?id={}", base, issue.id).as_str())
            .map_err(|e| e.to_string())?;
    }

    sheet.autofit();
    // autofit 会被前面几列的长详情文本拉得很宽，详情列改成固定宽度更可读，
    // 换行内容展开靠 Excel 的「自动调整行高」，这里不处理行高。
    for col in 13..=16u16 {
        sheet.set_column_width(col, 60).map_err(|e| e.to_string())?;
    }
    wb.save(&path).map_err(|e| format!("保存 xlsx 失败：{}", e))?;
    Ok(issues.len())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MantisIssueDetail {
    pub id: i64,
    pub summary: String,
    pub description: String,
    pub steps_to_reproduce: String,
    pub additional_information: String,
    pub project: String,
    pub category: String,
    pub status: String,
    pub severity: String,
    pub priority: String,
    pub reproducibility: String,
    pub version: String,
    pub fixed_in_version: String,
    pub reporter: String,
    pub handler: String,
    pub created_at: String,
    pub updated_at: String,
    pub notes: Vec<MantisNote>,
    pub custom_fields: Vec<MantisCustomField>,
}

#[tauri::command]
pub async fn get_mantis_issue(
    base_url: String,
    api_token: String,
    issue_id: i64,
) -> Result<MantisIssueDetail, String> {
    let data = mantis_get(&base_url, &api_token, &format!("/issues/{}", issue_id)).await?;
    let issue = data
        .get("issues")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .cloned()
        .ok_or_else(|| format!("未找到 issue #{}", issue_id))?;

    let notes = issue
        .get("notes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|n| {
            Some(MantisNote {
                id: as_id(n.get("id")).unwrap_or(0),
                reporter: user_name(n.get("reporter")),
                text: as_str(n.get("text")),
                created_at: as_str(n.get("created_at")),
            })
        })
        .collect();

    let custom_fields = issue
        .get("custom_fields")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|cf| {
            let name = cf.get("field").and_then(|f| f.get("name")).and_then(|n| n.as_str())?.to_string();
            Some(MantisCustomField { name, value: custom_field_value(cf.get("value")) })
        })
        .collect();

    Ok(MantisIssueDetail {
        id: as_id(issue.get("id")).unwrap_or(issue_id),
        summary: as_str(issue.get("summary")),
        description: as_str(issue.get("description")),
        steps_to_reproduce: as_str(issue.get("steps_to_reproduce")),
        additional_information: as_str(issue.get("additional_information")),
        project: ref_name(issue.get("project")),
        category: ref_name(issue.get("category")),
        status: enum_label(issue.get("status")),
        severity: enum_label(issue.get("severity")),
        priority: enum_label(issue.get("priority")),
        reproducibility: enum_label(issue.get("reproducibility")),
        version: ref_name(issue.get("version")),
        fixed_in_version: ref_name(issue.get("fixed_in_version")),
        reporter: user_name(issue.get("reporter")),
        handler: user_name(issue.get("handler")),
        created_at: as_str(issue.get("created_at")),
        updated_at: as_str(issue.get("updated_at")),
        notes,
        custom_fields,
    })
}

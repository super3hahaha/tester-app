//! PRD 风险画像链路里的 APP 显示名 -> skill 知识库文件名映射。
//!
//! 映射是用户数据，持久化在 `~/.tester-app/prd-risk-app-mappings.json`，由
//! SupplementPage 管理。补充测试点（复用）和 BugPage（沉淀）共用同一份配置。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrdRiskAppMapping {
    pub app_name: String,
    pub knowledge_app_name: String,
}

fn mapping_path() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join(".tester-app")
        .join("prd-risk-app-mappings.json")
}

/// 从旧版硬编码行为迁移出的初始数据。文件一旦保存（即使保存为空数组），以后
/// 完全以用户配置为准；新增 APP 不需要再改代码。
fn initial_mappings() -> Vec<PrdRiskAppMapping> {
    vec![PrdRiskAppMapping {
        app_name: "VTM".to_string(),
        knowledge_app_name: "Video to MP3 Android".to_string(),
    }]
}

fn load_mappings() -> Vec<PrdRiskAppMapping> {
    let path = mapping_path();
    if !path.exists() {
        return initial_mappings();
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn normalized(name: &str) -> String {
    name.trim().to_lowercase()
}

fn resolve_knowledge_app_name(display_name: &str, mappings: &[PrdRiskAppMapping]) -> String {
    let key = normalized(display_name);
    mappings
        .iter()
        .find(|mapping| normalized(&mapping.app_name) == key)
        .map(|mapping| mapping.knowledge_app_name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| display_name.trim().to_string())
}

/// 返回 prd-risk-profiler `references/apps/<名称>.md` 使用的稳定名称。
pub(crate) fn knowledge_app_name(display_name: &str) -> String {
    resolve_knowledge_app_name(display_name, &load_mappings())
}

fn validate_name(name: &str, field: &str) -> Result<String, String> {
    let value = name.trim();
    if value.is_empty() {
        return Err(format!("{}不能为空", field));
    }
    if value.contains('/')
        || value.contains('\\')
        || value.contains("..")
        || value.chars().any(char::is_control)
    {
        return Err(format!("{}包含非法字符", field));
    }
    Ok(value.to_string())
}

fn validate_knowledge_app_name(name: &str) -> Result<String, String> {
    let value = validate_name(name, "知识库文件名")?;
    Ok(value
        .strip_suffix(".md")
        .unwrap_or(&value)
        .trim()
        .to_string())
}

#[tauri::command]
pub fn list_prd_risk_app_mappings() -> Result<Vec<PrdRiskAppMapping>, String> {
    Ok(load_mappings())
}

#[tauri::command]
pub fn save_prd_risk_app_mappings(mappings: Vec<PrdRiskAppMapping>) -> Result<(), String> {
    let mut cleaned = Vec::with_capacity(mappings.len());
    let mut seen = std::collections::HashSet::new();
    for mapping in mappings {
        let app_name = validate_name(&mapping.app_name, "APP 名称")?;
        let knowledge_app_name = validate_knowledge_app_name(&mapping.knowledge_app_name)?;
        if !seen.insert(normalized(&app_name)) {
            return Err(format!("APP「{}」配置了重复映射", app_name));
        }
        cleaned.push(PrdRiskAppMapping {
            app_name,
            knowledge_app_name,
        });
    }
    cleaned.sort_by(|a, b| normalized(&a.app_name).cmp(&normalized(&b.app_name)));

    let path = mapping_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建映射目录失败：{}", e))?;
    }
    let raw = serde_json::to_string_pretty(&cleaned).map_err(|e| format!("序列化失败：{}", e))?;
    std::fs::write(path, raw).map_err(|e| format!("保存映射失败：{}", e))
}

#[cfg(test)]
mod tests {
    use super::{resolve_knowledge_app_name, validate_knowledge_app_name, PrdRiskAppMapping};

    fn mappings() -> Vec<PrdRiskAppMapping> {
        vec![PrdRiskAppMapping {
            app_name: "VTM".to_string(),
            knowledge_app_name: "Video to MP3 Android".to_string(),
        }]
    }

    #[test]
    fn resolves_user_mapping_case_insensitively() {
        assert_eq!(
            resolve_knowledge_app_name(" vtm ", &mappings()),
            "Video to MP3 Android"
        );
    }

    #[test]
    fn preserves_apps_without_mapping() {
        assert_eq!(
            resolve_knowledge_app_name("MP3Cutter", &mappings()),
            "MP3Cutter"
        );
    }

    #[test]
    fn strips_optional_markdown_extension() {
        assert_eq!(
            validate_knowledge_app_name("Video to MP3 Android.md").unwrap(),
            "Video to MP3 Android"
        );
    }
}

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 全局 CLI 引擎，覆盖所有 AI 功能；旧配置默认 Claude。
fn default_cli_engine() -> String {
    "claude".to_string()
}

/// 测试用例生成默认模型。老配置文件无此字段时回退 Sonnet。
fn default_testcase_model() -> String {
    "claude-sonnet-4-6".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub reply: String,
    pub analysis: String,
    pub translate: String,
    /// 测试用例生成（/test-case-generator skill）用哪个模型。
    #[serde(default = "default_testcase_model")]
    pub testcase: String,
    #[serde(default)]
    pub github_token: String,
    /// 全局使用哪个 CLI 引擎："claude" | "codex"。
    #[serde(default = "default_cli_engine")]
    pub cli_engine: String,
    /// engine=codex 时给 codex exec 传的模型；空串=用 Codex 自身默认。
    #[serde(default)]
    pub codex_model: String,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            reply: "claude-sonnet-4-6".to_string(),
            analysis: "claude-sonnet-4-6".to_string(),
            translate: "claude-haiku-4-5".to_string(),
            testcase: default_testcase_model(),
            github_token: String::new(),
            cli_engine: default_cli_engine(),
            codex_model: String::new(),
        }
    }
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join(".tester-app")
        .join("model-config.json")
}

pub fn load() -> ModelConfig {
    let path = config_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_model_config() -> ModelConfig {
    load()
}

#[tauri::command]
pub fn save_model_config(mut config: ModelConfig) -> Result<(), String> {
    if !matches!(config.cli_engine.as_str(), "claude" | "codex") {
        return Err("不支持的 AI 引擎，请选择 Claude 或 Codex".into());
    }
    config.codex_model = config.codex_model.trim().to_string();
    let path = config_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_configs_default_to_claude_and_new_fields_roundtrip() {
        let old = r#"{"reply":"sonnet","analysis":"sonnet","translate":"haiku"}"#;
        let mut cfg: ModelConfig = serde_json::from_str(old).unwrap();
        assert_eq!(cfg.cli_engine, "claude");
        assert!(cfg.codex_model.is_empty());
        cfg.cli_engine = "codex".into();
        cfg.codex_model = "custom-model".into();
        let restored: ModelConfig = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(restored.cli_engine, "codex");
        assert_eq!(restored.codex_model, "custom-model");
        assert_eq!(restored.reply, "sonnet");
    }

    #[test]
    fn invalid_engine_is_rejected_before_writing() {
        let cfg = ModelConfig { cli_engine: "unknown".into(), ..Default::default() };
        assert!(save_model_config(cfg).is_err());
    }
}

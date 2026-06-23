use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 默认 CLI 引擎。直出类功能（单条回复 / 评论分析 / 邮件回复 / 模板翻译）按此值
/// 路由到 Claude CLI 或 Codex CLI；skill 依赖类（测试用例生成、批量模板匹配回复）
/// 永远走 Claude，与此无关。
fn default_cli_engine() -> String {
    "claude".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub reply: String,
    pub analysis: String,
    pub translate: String,
    #[serde(default)]
    pub github_token: String,
    /// 直出类功能用哪个 CLI 引擎："claude" | "codex"。
    #[serde(default = "default_cli_engine")]
    pub cli_engine: String,
    /// engine=codex 时给 codex exec 传的模型（如 gpt-5 / o3）；空串=用 Codex 自身默认。
    #[serde(default)]
    pub codex_model: String,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            reply: "claude-sonnet-4-6".to_string(),
            analysis: "claude-sonnet-4-6".to_string(),
            translate: "claude-haiku-4-5".to_string(),
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
pub fn save_model_config(config: ModelConfig) -> Result<(), String> {
    let path = config_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub reply: String,
    pub analysis: String,
    pub translate: String,
    #[serde(default)]
    pub github_token: String,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            reply: "claude-sonnet-4-6".to_string(),
            analysis: "claude-sonnet-4-6".to_string(),
            translate: "claude-haiku-4-5".to_string(),
            github_token: String::new(),
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

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

fn manifests_dir() -> PathBuf {
    data_dir().join("manifests")
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SlidePages {
    pub name: String,
    pub pages: Vec<usize>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GenerateManifest {
    pub drive_id: String,
    pub web_url: String,
    pub uploaded_at: u64,
    pub source_csv_path: Option<String>,
    pub pptx_paths: Vec<String>,
    pub slide_pages: Vec<SlidePages>,
    pub model: Option<String>,
    pub skill_version: Option<String>,
}

#[tauri::command]
pub fn write_generate_manifest(
    drive_id: String,
    web_url: String,
    source_csv_path: Option<String>,
    pptx_paths: Vec<String>,
    slide_pages: Vec<SlidePages>,
    model: Option<String>,
    skill_version: Option<String>,
) -> Result<String, String> {
    let dir = manifests_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Create manifests dir failed: {}", e))?;

    let uploaded_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let manifest = GenerateManifest {
        drive_id: drive_id.clone(),
        web_url,
        uploaded_at,
        source_csv_path,
        pptx_paths,
        slide_pages,
        model,
        skill_version,
    };

    let path = dir.join(format!("{}.json", drive_id));
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("Serialize manifest failed: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Write manifest failed: {}", e))?;
    Ok(path.to_string_lossy().to_string())
}

pub fn read_manifest(drive_id: &str) -> Option<GenerateManifest> {
    let path = manifests_dir().join(format!("{}.json", drive_id));
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

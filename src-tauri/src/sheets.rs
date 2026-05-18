use crate::auth::AuthState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, State};

#[derive(Serialize)]
pub struct DriveFile {
    id: String,
    name: String,
    modified_time: String,
    mime_type: String,
}

#[derive(Serialize)]
pub struct SheetData {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    spreadsheet_url: String,
}

#[derive(Deserialize)]
struct DriveFileList {
    files: Option<Vec<DriveFileRaw>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveFileRaw {
    id: String,
    name: String,
    modified_time: Option<String>,
    mime_type: Option<String>,
}

#[derive(Deserialize)]
struct SheetsValueRange {
    values: Option<Vec<Vec<String>>>,
}

#[derive(Deserialize)]
struct SheetInfo {
    properties: SheetProperties,
}

#[derive(Deserialize)]
struct SheetProperties {
    title: String,
}

#[derive(Deserialize)]
struct SpreadsheetMeta {
    sheets: Option<Vec<SheetInfo>>,
}

fn get_token(state: &State<'_, AuthState>) -> Result<String, String> {
    state.get_access_token().ok_or_else(|| "Not logged in".into())
}

#[tauri::command]
pub async fn list_drive_files(
    mime_type: String,
    state: State<'_, AuthState>,
) -> Result<Vec<DriveFile>, String> {
    let token = get_token(&state)?;

    let query = format!("mimeType='{}'", mime_type);
    let url = format!(
        "https://www.googleapis.com/drive/v3/files?\
         q={}&orderBy=viewedByMeTime desc&pageSize=20&\
         fields=files(id,name,modifiedTime,mimeType)",
        urlencoding::encode(&query)
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Drive API failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Drive API {}: {}", status, body));
    }

    let list: DriveFileList = resp
        .json()
        .await
        .map_err(|e| format!("Drive parse failed: {}", e))?;

    Ok(list
        .files
        .unwrap_or_default()
        .into_iter()
        .map(|f| DriveFile {
            id: f.id,
            name: f.name,
            modified_time: f.modified_time.unwrap_or_default(),
            mime_type: f.mime_type.unwrap_or_default(),
        })
        .collect())
}

#[tauri::command]
pub async fn get_sheet_tabs(
    spreadsheet_id: String,
    state: State<'_, AuthState>,
) -> Result<Vec<String>, String> {
    let token = get_token(&state)?;

    let url = format!(
        "https://sheets.googleapis.com/v4/spreadsheets/{}?fields=sheets.properties.title",
        spreadsheet_id
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Sheets API failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Sheets API {}: {}", status, body));
    }

    let meta: SpreadsheetMeta = resp
        .json()
        .await
        .map_err(|e| format!("Sheets parse failed: {}", e))?;

    Ok(meta
        .sheets
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.properties.title)
        .collect())
}

#[tauri::command]
pub async fn read_sheet(
    spreadsheet_id: String,
    range: String,
    state: State<'_, AuthState>,
) -> Result<SheetData, String> {
    let token = get_token(&state)?;

    let url = format!(
        "https://sheets.googleapis.com/v4/spreadsheets/{}/values/{}",
        spreadsheet_id,
        urlencoding::encode(&range)
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Sheets API failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Sheets API {}: {}", status, body));
    }

    let data: SheetsValueRange = resp
        .json()
        .await
        .map_err(|e| format!("Sheets parse failed: {}", e))?;

    let values = data.values.unwrap_or_default();
    if values.is_empty() {
        return Ok(SheetData {
            headers: vec![],
            rows: vec![],
            spreadsheet_url: format!(
                "https://docs.google.com/spreadsheets/d/{}",
                spreadsheet_id
            ),
        });
    }

    let headers = values[0].clone();
    let rows = values[1..].to_vec();

    Ok(SheetData {
        headers,
        rows,
        spreadsheet_url: format!(
            "https://docs.google.com/spreadsheets/d/{}",
            spreadsheet_id
        ),
    })
}

#[tauri::command]
pub async fn export_sheet_csv(
    spreadsheet_id: String,
    range: String,
    state: State<'_, AuthState>,
) -> Result<String, String> {
    let token = get_token(&state)?;

    let url = format!(
        "https://sheets.googleapis.com/v4/spreadsheets/{}/values/{}",
        spreadsheet_id,
        urlencoding::encode(&range)
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Sheets API failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Sheets API {}: {}", status, body));
    }

    let data: SheetsValueRange = resp
        .json()
        .await
        .map_err(|e| format!("Sheets parse failed: {}", e))?;

    let values = data.values.unwrap_or_default();

    // Build CSV
    let mut csv = String::new();
    for row in &values {
        let line: Vec<String> = row
            .iter()
            .map(|cell| {
                if cell.contains(',') || cell.contains('"') || cell.contains('\n') {
                    format!("\"{}\"", cell.replace('"', "\"\""))
                } else {
                    cell.clone()
                }
            })
            .collect();
        csv.push_str(&line.join(","));
        csv.push('\n');
    }

    // Save to temp dir
    let dir = data_dir().join("exports");
    std::fs::create_dir_all(&dir).ok();
    let filename = format!("sheet_{}.csv", spreadsheet_id.chars().take(8).collect::<String>());
    let path = dir.join(&filename);
    std::fs::write(&path, &csv)
        .map_err(|e| format!("Write CSV failed: {}", e))?;

    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn export_slides_pptx(
    presentation_id: String,
    name: String,
    state: State<'_, AuthState>,
) -> Result<String, String> {
    let token = get_token(&state)?;

    // Drive API: export Google Slides as PPTX
    let url = format!(
        "https://www.googleapis.com/drive/v3/files/{}/export?mimeType=application/vnd.openxmlformats-officedocument.presentationml.presentation",
        presentation_id
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Drive export failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Drive export {}: {}", status, body));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    let dir = data_dir().join("exports");
    std::fs::create_dir_all(&dir).ok();
    let safe_name = name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let filename = format!("{}.pptx", safe_name);
    let path = dir.join(&filename);
    std::fs::write(&path, &bytes)
        .map_err(|e| format!("Write PPTX failed: {}", e))?;

    Ok(path.to_string_lossy().to_string())
}

#[derive(Serialize)]
pub struct SlidePageInfo {
    pub page_object_id: String,
    pub page_number: usize,
    pub thumbnail_url: String,
}

#[tauri::command]
pub async fn get_presentation_slides(
    presentation_id: String,
    app: AppHandle,
    state: State<'_, AuthState>,
) -> Result<Vec<SlidePageInfo>, String> {
    let token = get_token(&state)?;
    let client = reqwest::Client::new();

    let url = format!(
        "https://slides.googleapis.com/v1/presentations/{}?fields=slides.objectId",
        presentation_id
    );

    let resp = client
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Slides API failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Slides API {}: {}", status, body));
    }

    let pres: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Slides parse failed: {}", e))?;

    let slides = pres
        .get("slides")
        .and_then(|s| s.as_array())
        .ok_or("No slides found")?;

    let cache_dir = data_dir()
        .join("thumbs")
        .join(&presentation_id);
    std::fs::create_dir_all(&cache_dir).ok();

    let mut page_ids: Vec<(usize, String)> = Vec::new();
    let mut pages: Vec<SlidePageInfo> = Vec::new();
    for (i, slide) in slides.iter().enumerate() {
        let page_id = slide
            .get("objectId")
            .and_then(|v| v.as_str())
            .ok_or("Missing objectId")?
            .to_string();

        let cached_file = cache_dir.join(format!("{}.png", i + 1));
        let thumb = if cached_file.exists() {
            match std::fs::read(&cached_file) {
                Ok(bytes) => {
                    let b64 = base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &bytes,
                    );
                    format!("data:image/png;base64,{}", b64)
                }
                Err(_) => String::new(),
            }
        } else {
            String::new()
        };

        let need_download = thumb.is_empty();
        pages.push(SlidePageInfo {
            page_object_id: page_id.clone(),
            page_number: i + 1,
            thumbnail_url: thumb,
        });
        if need_download {
            page_ids.push((i + 1, page_id));
        }
    }

    if !page_ids.is_empty() {
        let pres_id = presentation_id.clone();
        let cache = cache_dir.clone();
        tokio::spawn(async move {
            let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(10));
            let mut handles = Vec::new();

            for (page_num, page_id) in page_ids {
                let c = client.clone();
                let t = token.clone();
                let pid = pres_id.clone();
                let sem = semaphore.clone();
                let app_clone = app.clone();
                let cache_clone = cache.clone();

                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    let data_url = (|| async {
                        let thumb_url = format!(
                            "https://slides.googleapis.com/v1/presentations/{}/pages/{}/thumbnail?thumbnailProperties.thumbnailSize=LARGE",
                            pid, page_id
                        );
                        let resp = c.get(&thumb_url).bearer_auth(&t).send().await.ok()?;
                        if !resp.status().is_success() {
                            return None;
                        }
                        let thumb_data: serde_json::Value = resp.json().await.ok()?;
                        let content_url = thumb_data.get("contentUrl")?.as_str()?;

                        let img_resp = c.get(content_url).send().await.ok()?;
                        if !img_resp.status().is_success() {
                            return None;
                        }
                        let bytes = img_resp.bytes().await.ok()?;

                        let cached_file = cache_clone.join(format!("{}.png", page_num));
                        std::fs::write(&cached_file, &bytes).ok();

                        let b64 = base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &bytes,
                        );
                        Some(format!("data:image/png;base64,{}", b64))
                    })()
                    .await
                    .unwrap_or_default();

                    app_clone
                        .emit(
                            "slide-thumbnail",
                            serde_json::json!({
                                "presentation_id": pid,
                                "page_number": page_num,
                                "thumbnail_url": data_url,
                            }),
                        )
                        .ok();
                }));
            }

            for h in handles {
                h.await.ok();
            }
        });
    }

    Ok(pages)
}

#[tauri::command]
pub async fn get_cache_size() -> Result<u64, String> {
    let dir = data_dir().join("thumbs");
    Ok(dir_size(&dir))
}

#[tauri::command]
pub async fn clear_cache() -> Result<(), String> {
    let dir = data_dir().join("thumbs");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("Failed to clear cache: {}", e))?;
    }
    Ok(())
}

fn dir_size(path: &PathBuf) -> u64 {
    if !path.exists() {
        return 0;
    }
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                total += dir_size(&p);
            } else if let Ok(meta) = p.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

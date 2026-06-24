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

async fn get_token(state: &State<'_, AuthState>) -> Result<String, String> {
    crate::auth::get_valid_access_token(state).await
}

fn err_chain<E: std::error::Error + ?Sized>(e: &E) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(c) = src {
        s.push_str(" -> ");
        s.push_str(&c.to_string());
        src = c.source();
    }
    s
}

#[tauri::command]
pub async fn list_drive_files(
    mime_type: String,
    state: State<'_, AuthState>,
) -> Result<Vec<DriveFile>, String> {
    let token = get_token(&state).await?;

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
        .map_err(|e| format!("Drive API failed: {}", err_chain(&e)))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Drive API {}: {}", status, body));
    }

    let list: DriveFileList = resp
        .json()
        .await
        .map_err(|e| format!("Drive parse failed: {}", err_chain(&e)))?;

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
    let token = get_token(&state).await?;

    let url = format!(
        "https://sheets.googleapis.com/v4/spreadsheets/{}?fields=sheets.properties.title",
        spreadsheet_id
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Sheets API failed: {}", err_chain(&e)))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Sheets API {}: {}", status, body));
    }

    let meta: SpreadsheetMeta = resp
        .json()
        .await
        .map_err(|e| format!("Sheets parse failed: {}", err_chain(&e)))?;

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
    let token = get_token(&state).await?;

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
        .map_err(|e| format!("Sheets API failed: {}", err_chain(&e)))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Sheets API {}: {}", status, body));
    }

    let data: SheetsValueRange = resp
        .json()
        .await
        .map_err(|e| format!("Sheets parse failed: {}", err_chain(&e)))?;

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
    let token = get_token(&state).await?;

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
        .map_err(|e| format!("Sheets API failed: {}", err_chain(&e)))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Sheets API {}: {}", status, body));
    }

    let data: SheetsValueRange = resp
        .json()
        .await
        .map_err(|e| format!("Sheets parse failed: {}", err_chain(&e)))?;

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
        .map_err(|e| format!("Write CSV failed: {}", err_chain(&e)))?;

    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn export_slides_pptx(
    presentation_id: String,
    name: String,
    state: State<'_, AuthState>,
) -> Result<String, String> {
    let token = get_token(&state).await?;

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
        .map_err(|e| format!("Drive export failed: {}", err_chain(&e)))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Drive export {}: {}", status, body));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Download failed: {}", err_chain(&e)))?;

    let dir = data_dir().join("exports");
    std::fs::create_dir_all(&dir).ok();
    let safe_name = name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let filename = format!("{}.pptx", safe_name);
    let path = dir.join(&filename);
    std::fs::write(&path, &bytes)
        .map_err(|e| format!("Write PPTX failed: {}", err_chain(&e)))?;

    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn export_slides_pdf(
    presentation_id: String,
    name: String,
    state: State<'_, AuthState>,
) -> Result<String, String> {
    let token = get_token(&state).await?;

    let url = format!(
        "https://www.googleapis.com/drive/v3/files/{}/export?mimeType=application/pdf",
        presentation_id
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Drive export failed: {}", err_chain(&e)))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Drive export {}: {}", status, body));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Download failed: {}", err_chain(&e)))?;

    let dir = data_dir().join("exports");
    std::fs::create_dir_all(&dir).ok();
    let safe_name = name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let filename = format!("{}.pdf", safe_name);
    let path = dir.join(&filename);
    std::fs::write(&path, &bytes)
        .map_err(|e| format!("Write PDF failed: {}", err_chain(&e)))?;

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
    let token = get_token(&state).await?;
    let client = reqwest::Client::new();

    let url = format!(
        "https://slides.googleapis.com/v1/presentations/{}?fields=slides.objectId,revisionId",
        presentation_id
    );

    let resp = client
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Slides API failed: {}", err_chain(&e)))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Slides API {}: {}", status, body));
    }

    let pres: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Slides parse failed: {}", err_chain(&e)))?;

    let remote_revision = pres
        .get("revisionId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let slides = pres
        .get("slides")
        .and_then(|s| s.as_array())
        .ok_or("No slides found")?;

    let cache_dir = data_dir()
        .join("thumbs")
        .join(&presentation_id);
    let revision_file = cache_dir.join(".revision");

    // 比对 revisionId：变了就把这个 presentation 的整个缓存目录清掉，
    // 让后面所有 slide 走重新下载逻辑
    let local_revision = std::fs::read_to_string(&revision_file).unwrap_or_default();
    let revision_changed = !remote_revision.is_empty() && local_revision != remote_revision;
    if revision_changed && cache_dir.exists() {
        std::fs::remove_dir_all(&cache_dir).ok();
    }
    std::fs::create_dir_all(&cache_dir).ok();
    // 提前写新 revision：若下载过程中崩了，下次启动时已落地的 png 仍属于这个 revision，
    // 只补缺失的几张即可，不必再全清重拉
    if revision_changed && !remote_revision.is_empty() {
        std::fs::write(&revision_file, &remote_revision).ok();
    }

    let mut page_ids: Vec<(usize, String)> = Vec::new();
    let mut pages: Vec<SlidePageInfo> = Vec::new();
    for (i, slide) in slides.iter().enumerate() {
        let page_id = slide
            .get("objectId")
            .and_then(|v| v.as_str())
            .ok_or("Missing objectId")?
            .to_string();

        // 缓存 key 用 objectId 而不是页码：slide 重排/插入/删除后位置变了，
        // 但 objectId 不变，原图仍可命中
        let cached_file = cache_dir.join(format!("{}.png", page_id));
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

                        let cached_file = cache_clone.join(format!("{}.png", page_id));
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
        std::fs::remove_dir_all(&dir).map_err(|e| format!("Failed to clear cache: {}", err_chain(&e)))?;
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

#[derive(Serialize)]
pub struct ExportInfo {
    path: String,
    name: String,
}

#[tauri::command]
pub fn find_latest_export(since_ms: Option<u64>) -> Result<Option<ExportInfo>, String> {
    let dir = data_dir().join("exports");
    if !dir.exists() {
        return Ok(None);
    }
    let since = since_ms
        .map(|ms| std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms));
    let mut latest: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("xlsx") {
            let meta = entry.metadata().map_err(|e| e.to_string())?;
            let modified = meta.modified().map_err(|e| e.to_string())?;
            if let Some(threshold) = since {
                if modified < threshold {
                    continue;
                }
            }
            if latest.as_ref().map_or(true, |(_, t)| modified > *t) {
                latest = Some((path, modified));
            }
        }
    }
    Ok(latest.map(|(p, _)| ExportInfo {
        name: p.file_name().unwrap().to_string_lossy().to_string(),
        path: p.to_string_lossy().to_string(),
    }))
}

#[derive(Serialize)]
pub struct UploadResult {
    drive_id: String,
    web_url: String,
    converted: bool,
}

async fn find_or_create_folder(token: &str, name: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let q = format!(
        "name='{}' and mimeType='application/vnd.google-apps.folder' and trashed=false",
        name.replace('\'', "\\'")
    );
    let search_url = format!(
        "https://www.googleapis.com/drive/v3/files?q={}&fields=files(id,name)&pageSize=1",
        urlencoding::encode(&q)
    );
    let resp = client
        .get(&search_url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Drive search failed: {}", err_chain(&e)))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Drive search {}: {}", status, body));
    }
    let list: DriveFileList = resp
        .json()
        .await
        .map_err(|e| format!("Drive parse failed: {}", err_chain(&e)))?;
    if let Some(files) = list.files {
        if let Some(f) = files.first() {
            return Ok(f.id.clone());
        }
    }

    let body = serde_json::json!({
        "name": name,
        "mimeType": "application/vnd.google-apps.folder",
    });
    let resp = client
        .post("https://www.googleapis.com/drive/v3/files?fields=id")
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Drive folder create failed: {}", err_chain(&e)))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Drive folder create {}: {}", status, body));
    }
    let created: DriveFileRaw = resp
        .json()
        .await
        .map_err(|e| format!("Folder parse failed: {}", err_chain(&e)))?;
    Ok(created.id)
}

async fn upload_bytes_to_drive(
    token: &str,
    file_name: &str,
    bytes: Vec<u8>,
    convert_to_sheets: bool,
    folder_name: Option<String>,
) -> Result<UploadResult, String> {
    let folder = folder_name.unwrap_or_else(|| "tester-app".to_string());
    let folder_id = find_or_create_folder(token, &folder).await?;

    let mut metadata = serde_json::json!({
        "name": file_name,
        "parents": [folder_id],
    });
    if convert_to_sheets {
        metadata["mimeType"] = serde_json::json!("application/vnd.google-apps.spreadsheet");
    }

    let boundary = "tester_app_boundary_8a3c";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata.to_string().as_bytes());
    body.extend_from_slice(format!("\r\n--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        b"Content-Type: application/vnd.openxmlformats-officedocument.spreadsheetml.sheet\r\n\r\n",
    );
    body.extend_from_slice(&bytes);
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

    let resp = reqwest::Client::new()
        .post("https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id,webViewLink,mimeType")
        .bearer_auth(token)
        .header(
            "Content-Type",
            format!("multipart/related; boundary={}", boundary),
        )
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Upload failed: {}", err_chain(&e)))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Drive upload {}: {}", status, body));
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct UploadResp {
        id: String,
        web_view_link: Option<String>,
        mime_type: Option<String>,
    }
    let parsed: UploadResp = resp
        .json()
        .await
        .map_err(|e| format!("Upload parse failed: {}", err_chain(&e)))?;

    let web_url = parsed.web_view_link.unwrap_or_else(|| {
        if convert_to_sheets {
            format!("https://docs.google.com/spreadsheets/d/{}/edit", parsed.id)
        } else {
            format!("https://drive.google.com/file/d/{}/view", parsed.id)
        }
    });

    Ok(UploadResult {
        drive_id: parsed.id,
        web_url,
        converted: parsed
            .mime_type
            .as_deref()
            .map(|m| m == "application/vnd.google-apps.spreadsheet")
            .unwrap_or(convert_to_sheets),
    })
}

#[tauri::command]
pub async fn upload_xlsx_to_drive(
    file_path: String,
    convert_to_sheets: bool,
    folder_name: Option<String>,
    state: State<'_, AuthState>,
) -> Result<UploadResult, String> {
    let token = get_token(&state).await?;
    let path = std::path::Path::new(&file_path);
    if !path.is_file() {
        return Err(format!("File not found: {}", file_path));
    }
    let file_name = path
        .file_name()
        .ok_or("Bad file path")?
        .to_string_lossy()
        .to_string();
    let bytes = std::fs::read(path).map_err(|e| format!("Read file failed: {}", err_chain(&e)))?;

    upload_bytes_to_drive(&token, &file_name, bytes, convert_to_sheets, folder_name).await
}

#[tauri::command]
pub async fn upload_xlsx_bytes_to_drive(
    file_name: String,
    bytes: Vec<u8>,
    convert_to_sheets: bool,
    folder_name: Option<String>,
    state: State<'_, AuthState>,
) -> Result<UploadResult, String> {
    let token = get_token(&state).await?;
    upload_bytes_to_drive(&token, &file_name, bytes, convert_to_sheets, folder_name).await
}

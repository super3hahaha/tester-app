use crate::auth::AuthState;
use serde::Serialize;
use std::io::Read;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Serialize, Clone)]
pub struct CompareLogEvent {
    pub text: String,
    pub kind: String, // "info", "text", "tool", "error", "result"
    pub done: bool,
}

pub struct CompareState {
    pub running: Mutex<bool>,
}

impl CompareState {
    pub fn new() -> Self {
        Self {
            running: Mutex::new(false),
        }
    }
}

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
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

fn safe_name(s: &str) -> String {
    s.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
}

fn emit_log(app: &AppHandle, text: &str, kind: &str, done: bool) {
    app.emit(
        "compare-log",
        CompareLogEvent {
            text: text.to_string(),
            kind: kind.to_string(),
            done,
        },
    )
    .ok();
}

/// Normalize a string for filename matching: lowercase, keep only alphanumerics.
/// Google sanitizes tab names into export filenames in a not-fully-documented way
/// (spaces, slashes, brackets, dots etc. are mangled), so we compare on the
/// alphanumeric residue instead of guessing the exact rule.
fn norm_for_match(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Pick the HTML entry from a Sheets-export zip that corresponds to `tab_name`.
/// Returns the entry's raw bytes.
fn extract_tab_html(zip_bytes: &[u8], tab_name: &str) -> Result<Vec<u8>, String> {
    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| format!("Open zip failed: {}", err_chain(&e)))?;

    // Collect all .html entries (skip index.html — it's just a TOC linking to the others).
    let mut html_entries: Vec<(String, String)> = Vec::new(); // (full_name, basename_stem)
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("Read zip entry failed: {}", err_chain(&e)))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let lower = name.to_lowercase();
        if !lower.ends_with(".html") {
            continue;
        }
        // strip path + extension
        let base = std::path::Path::new(&name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if base.eq_ignore_ascii_case("index") {
            continue;
        }
        html_entries.push((name, base));
    }

    if html_entries.is_empty() {
        return Err("Sheets zip contained no per-sheet HTML files".into());
    }

    // Single sheet → just take it.
    let target_name = if html_entries.len() == 1 {
        html_entries[0].0.clone()
    } else {
        let want = norm_for_match(tab_name);
        let mut matched: Option<String> = None;
        for (full, stem) in &html_entries {
            if norm_for_match(stem) == want {
                matched = Some(full.clone());
                break;
            }
        }
        matched.ok_or_else(|| {
            let listing: Vec<String> =
                html_entries.iter().map(|(_, s)| s.clone()).collect();
            format!(
                "No HTML entry matched tab '{}' (found: {})",
                tab_name,
                listing.join(", ")
            )
        })?
    };

    let mut entry = archive
        .by_name(&target_name)
        .map_err(|e| format!("Open zip entry '{}' failed: {}", target_name, err_chain(&e)))?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut buf)
        .map_err(|e| format!("Read zip entry failed: {}", err_chain(&e)))?;
    Ok(buf)
}

/// Export a single tab of a Google Sheet as HTML (waffle format).
/// Uses Drive API `mimeType=application/zip` (the documented "Web page (.html, zipped)"
/// export), then picks the matching tab's HTML out of the archive.
/// Returns the absolute path to the written .html file.
#[tauri::command]
pub async fn export_sheet_html(
    spreadsheet_id: String,
    tab_name: String,
    role: String, // "ai" or "human" — used only to make filename distinguishable
    state: State<'_, AuthState>,
) -> Result<String, String> {
    let token = state
        .get_access_token()
        .ok_or_else(|| "Not logged in".to_string())?;

    let url = format!(
        "https://www.googleapis.com/drive/v3/files/{}/export?mimeType=application/zip",
        spreadsheet_id
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

    let html_bytes = extract_tab_html(&bytes, &tab_name)?;

    let dir = data_dir().join("exports");
    std::fs::create_dir_all(&dir).ok();
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let id_short: String = spreadsheet_id.chars().take(8).collect();
    let filename = format!(
        "compare_{}_{}_{}_{}.html",
        safe_name(&role),
        id_short,
        safe_name(&tab_name),
        ts
    );
    let path = dir.join(&filename);
    std::fs::write(&path, &html_bytes)
        .map_err(|e| format!("Write HTML failed: {}", err_chain(&e)))?;

    Ok(path.to_string_lossy().to_string())
}

const DIFF_SCRIPT: &str = include_str!("../scripts/diff_testcases.py");

fn ensure_diff_script() -> Result<PathBuf, String> {
    let dir = data_dir().join("scripts");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Create scripts dir failed: {}", err_chain(&e)))?;
    let path = dir.join("diff_testcases.py");
    // Overwrite on every call so app updates always ship the latest script
    std::fs::write(&path, DIFF_SCRIPT)
        .map_err(|e| format!("Write script failed: {}", err_chain(&e)))?;
    Ok(path)
}

fn find_python() -> Option<String> {
    // On Windows the launcher `py` handles version discovery; elsewhere prefer python3.
    let candidates: &[&str] = if cfg!(windows) {
        &["py", "python", "python3"]
    } else {
        &["python3", "python"]
    };
    for c in candidates {
        let probe = std::process::Command::new(c)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Ok(s) = probe {
            if s.success() {
                return Some((*c).to_string());
            }
        }
    }
    None
}

async fn ensure_bs4(python: &str, app: &AppHandle) -> Result<(), String> {
    let check = std::process::Command::new(python)
        .args(["-c", "import bs4"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Python check failed: {}", err_chain(&e)))?;
    if check.success() {
        return Ok(());
    }
    emit_log(app, "Installing beautifulsoup4 via pip...", "info", false);
    let mut cmd = Command::new(python);
    cmd.args(["-m", "pip", "install", "beautifulsoup4", "--quiet"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn pip: {}", err_chain(&e)))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let app_out = app.clone();
    if let Some(s) = stdout {
        tokio::spawn(async move {
            let reader = BufReader::new(s);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    emit_log(&app_out, &line, "info", false);
                }
            }
        });
    }
    let app_err = app.clone();
    if let Some(s) = stderr {
        tokio::spawn(async move {
            let reader = BufReader::new(s);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    emit_log(&app_err, &line, "error", false);
                }
            }
        });
    }
    let status = child
        .wait()
        .await
        .map_err(|e| format!("pip wait failed: {}", err_chain(&e)))?;
    if !status.success() {
        return Err(format!(
            "pip install beautifulsoup4 failed (code {})",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

/// Run the diff_testcases.py script directly on the two HTML files.
/// Returns the path of the generated report HTML.
#[tauri::command]
pub async fn run_diff_skill(
    ai_html_path: String,
    human_html_path: String,
    app: AppHandle,
    state: State<'_, CompareState>,
) -> Result<String, String> {
    {
        let mut running = state.running.lock().unwrap();
        if *running {
            return Err("Compare task is already running".into());
        }
        *running = true;
    }

    let result = run_diff_skill_inner(ai_html_path, human_html_path, app.clone()).await;
    *state.running.lock().unwrap() = false;
    match &result {
        Ok(path) => emit_log(&app, &format!("Report ready: {}", path), "result", true),
        Err(e) => emit_log(&app, &format!("Failed: {}", e), "error", true),
    }
    result
}

async fn run_diff_skill_inner(
    ai_html_path: String,
    human_html_path: String,
    app: AppHandle,
) -> Result<String, String> {
    if !std::path::Path::new(&ai_html_path).is_file() {
        return Err(format!("AI HTML not found: {}", ai_html_path));
    }
    if !std::path::Path::new(&human_html_path).is_file() {
        return Err(format!("Human HTML not found: {}", human_html_path));
    }

    let python = find_python()
        .ok_or("Python 3 not found. Please install Python 3 and ensure it is on PATH.")?;
    let script_path = ensure_diff_script()?;

    ensure_bs4(&python, &app).await?;

    let dir = data_dir().join("exports");
    std::fs::create_dir_all(&dir).ok();
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let report_path = dir.join(format!("diff_report_{}.html", ts));
    let report_path_str = report_path.to_string_lossy().to_string();

    let script_str = script_path.to_string_lossy().to_string();
    emit_log(
        &app,
        &format!(
            "$ {} {} {} {} -o {}",
            python, script_str, ai_html_path, human_html_path, report_path_str
        ),
        "info",
        false,
    );

    let mut cmd = Command::new(&python);
    cmd.arg(&script_str)
        .arg(&ai_html_path)
        .arg(&human_html_path)
        .arg("-o")
        .arg(&report_path_str)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONIOENCODING", "utf-8");

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn python: {}", err_chain(&e)))?;

    let stdout = child.stdout.take().ok_or("No stdout")?;
    let stderr = child.stderr.take().ok_or("No stderr")?;

    let app_out = app.clone();
    let stdout_task = tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.trim().is_empty() {
                emit_log(&app_out, &line, "text", false);
            }
        }
    });
    let app_err = app.clone();
    let stderr_task = tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.trim().is_empty() {
                emit_log(&app_err, &line, "error", false);
            }
        }
    });

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Python process error: {}", err_chain(&e)))?;

    stdout_task.await.ok();
    stderr_task.await.ok();

    if !status.success() {
        return Err(format!(
            "diff_testcases.py exited with code {}",
            status.code().unwrap_or(-1)
        ));
    }

    if !report_path.is_file() {
        return Err(format!(
            "Script finished but report file not found at {}",
            report_path_str
        ));
    }

    Ok(report_path_str)
}

/// Open a local file in Google Chrome.
#[tauri::command]
pub fn open_in_chrome(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.is_file() {
        return Err(format!("File not found: {}", path));
    }

    if cfg!(windows) {
        // Try common Chrome install locations first
        let candidates = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ];
        for c in &candidates {
            if std::path::Path::new(c).is_file() {
                return std::process::Command::new(c)
                    .arg(&path)
                    .spawn()
                    .map(|_| ())
                    .map_err(|e| format!("Failed to launch Chrome: {}", err_chain(&e)));
            }
        }
        // Fall back to start chrome via cmd
        std::process::Command::new("cmd")
            .args(["/c", "start", "chrome", &path])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to start chrome: {}", err_chain(&e)))
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
            .args(["-a", "Google Chrome", &path])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to launch Chrome: {}", err_chain(&e)))
    } else {
        std::process::Command::new("google-chrome")
            .arg(&path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to launch Chrome: {}", err_chain(&e)))
    }
}

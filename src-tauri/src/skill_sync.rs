use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Skill registry. Hardcoded for v1; when a second skill arrives, consider
/// moving this to a remote manifest so adding skills doesn't require a rebuild.
struct SkillSource {
    name: &'static str,
    owner: &'static str,
    repo: &'static str,
}

const SKILLS: &[SkillSource] = &[SkillSource {
    name: "test-case-generator",
    owner: "super3hahaha",
    repo: "test-case-generator",
}];

// GitHub API requires a User-Agent on every request. Anything identifying works.
const UA: &str = "tester-app/0.1";

fn tester_data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

fn claude_skills_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".claude").join("skills")
}

fn skill_local_dir(name: &str) -> PathBuf {
    claude_skills_dir().join(name)
}

fn skill_version_file(name: &str) -> PathBuf {
    skill_local_dir(name).join(".tester-app-version")
}

fn read_local_version(name: &str) -> Option<String> {
    std::fs::read_to_string(skill_version_file(name))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn write_local_version(name: &str, version: &str) -> Result<(), String> {
    let path = skill_version_file(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Create skill dir failed: {}", e))?;
    }
    std::fs::write(&path, version)
        .map_err(|e| format!("Write version file failed: {}", e))?;
    Ok(())
}

struct LatestRelease {
    tag: String,
    zipball_url: String,
}

async fn fetch_latest_release(src: &SkillSource) -> Result<LatestRelease, String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        src.owner, src.repo
    );
    let resp = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", UA)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {}", e))?;
    let status = resp.status();
    if status.as_u16() == 404 {
        return Err("仓库尚未发布 release（在 GitHub 上 cut 一个 release 即可触发更新）".into());
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API {}: {}", status, body));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Parse GitHub response failed: {}", e))?;
    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "No 'tag_name' in release response".to_string())?;
    let zipball_url = json
        .get("zipball_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "No 'zipball_url' in release response".to_string())?;
    Ok(LatestRelease { tag, zipball_url })
}

fn backup_existing_skill(name: &str, old_version: Option<&str>) -> Result<Option<PathBuf>, String> {
    let dir = skill_local_dir(name);
    if !dir.is_dir() {
        return Ok(None);
    }
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ver_part = old_version
        .map(|s| s.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_"))
        .unwrap_or_else(|| "unknown".into());
    let backup = tester_data_dir()
        .join("skill_backups")
        .join(format!("{}_{}_{}", name, ver_part, ts));
    if let Some(parent) = backup.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Create backup parent failed: {}", e))?;
    }
    std::fs::rename(&dir, &backup)
        .map_err(|e| format!("Backup rename failed: {}", e))?;
    Ok(Some(backup))
}

async fn download_zipball(url: &str) -> Result<Vec<u8>, String> {
    let resp = reqwest::Client::new()
        .get(url)
        .header("User-Agent", UA)
        .send()
        .await
        .map_err(|e| format!("Zipball download failed: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Zipball {}: {}", status, body));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Read zipball bytes failed: {}", e))?;
    Ok(bytes.to_vec())
}

/// Extract a GitHub zipball into the target skill directory.
/// Zipballs wrap everything under a single top-level dir like
/// `super3hahaha-test-case-generator-<short_sha>/`, so we strip that prefix.
fn extract_zipball_to_skill_dir(zip_bytes: &[u8], target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target)
        .map_err(|e| format!("Create target dir failed: {}", e))?;
    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| format!("Open zipball failed: {}", e))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Read zipball entry failed: {}", e))?;
        let entry_name = entry.name().to_string();
        let stripped = match entry_name.split_once('/') {
            Some((_, rest)) => rest,
            None => continue,
        };
        if stripped.is_empty() {
            continue;
        }
        let out_path = target.join(stripped);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("Mkdir {} failed: {}", out_path.display(), e))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Mkdir {} failed: {}", parent.display(), e))?;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("Read entry {} failed: {}", entry_name, e))?;
        std::fs::write(&out_path, &buf)
            .map_err(|e| format!("Write {} failed: {}", out_path.display(), e))?;
    }
    Ok(())
}

#[derive(Serialize, Clone)]
pub struct SkillStatus {
    pub name: String,
    pub owner: String,
    pub repo: String,
    pub local_version: Option<String>,
    pub remote_version: Option<String>,
    pub up_to_date: bool,
    pub updated: bool,        // true if this run actually overwrote files
    pub backup_path: Option<String>,
    pub error: Option<String>,
}

fn fresh_status(src: &SkillSource) -> SkillStatus {
    SkillStatus {
        name: src.name.to_string(),
        owner: src.owner.to_string(),
        repo: src.repo.to_string(),
        local_version: read_local_version(src.name),
        remote_version: None,
        up_to_date: false,
        updated: false,
        backup_path: None,
        error: None,
    }
}

async fn check_one(src: &SkillSource) -> SkillStatus {
    let mut s = fresh_status(src);
    match fetch_latest_release(src).await {
        Ok(rel) => {
            s.up_to_date = s.local_version.as_deref() == Some(rel.tag.as_str());
            s.remote_version = Some(rel.tag);
        }
        Err(e) => s.error = Some(e),
    }
    s
}

async fn sync_one(src: &SkillSource, force: bool) -> SkillStatus {
    let mut s = fresh_status(src);
    let rel = match fetch_latest_release(src).await {
        Ok(r) => r,
        Err(e) => {
            s.error = Some(e);
            return s;
        }
    };
    s.remote_version = Some(rel.tag.clone());

    if !force && s.local_version.as_deref() == Some(rel.tag.as_str()) {
        s.up_to_date = true;
        return s;
    }

    let zip_bytes = match download_zipball(&rel.zipball_url).await {
        Ok(b) => b,
        Err(e) => {
            s.error = Some(e);
            return s;
        }
    };

    let backup = match backup_existing_skill(src.name, s.local_version.as_deref()) {
        Ok(b) => b,
        Err(e) => {
            s.error = Some(e);
            return s;
        }
    };
    s.backup_path = backup
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());

    let target = skill_local_dir(src.name);
    if let Err(e) = extract_zipball_to_skill_dir(&zip_bytes, &target) {
        if let Some(b) = backup {
            let _ = std::fs::remove_dir_all(&target);
            let _ = std::fs::rename(&b, &target);
        }
        s.error = Some(e);
        return s;
    }

    if let Err(e) = write_local_version(src.name, &rel.tag) {
        s.error = Some(format!("Sync succeeded but writing version failed: {}", e));
        return s;
    }

    s.local_version = Some(rel.tag);
    s.up_to_date = true;
    s.updated = true;
    s
}

#[tauri::command]
pub async fn check_skill_updates() -> Vec<SkillStatus> {
    let mut out = Vec::new();
    for src in SKILLS {
        out.push(check_one(src).await);
    }
    out
}

#[tauri::command]
pub async fn sync_all_skills() -> Vec<SkillStatus> {
    let mut out = Vec::new();
    for src in SKILLS {
        out.push(sync_one(src, false).await);
    }
    out
}

#[tauri::command]
pub async fn sync_skill(name: String, force: bool) -> Result<SkillStatus, String> {
    let src = SKILLS
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("Unknown skill: {}", name))?;
    Ok(sync_one(src, force).await)
}

/// Read the local version tag for a given skill (e.g. "v1.8.0").
/// Used by frontend to stamp feedback manifests with the real skill version.
#[tauri::command]
pub fn get_skill_local_version(name: String) -> Option<String> {
    read_local_version(&name)
}

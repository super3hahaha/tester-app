// 用指定 Chrome profile 打开 URL —— 解决 Gmail 多账号分散在不同 Chrome 个人资料
// (profile) 时，深链跨 profile 跳不过去的问题。
//
// 思路：读 Chrome 的 `Local State`（JSON）拿到「目录名(Default/Profile 1…) ↔ 显示名
// (Manager/tester…)」映射，前端用显示名选、存目录名；打开时 `open -na "Google Chrome"
// --profile-directory=<目录名> <url>` 直达对应 profile 的窗口。

use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct ChromeProfile {
    dir: String,  // 目录名，命令行 --profile-directory 用（如 "Default" / "Profile 3"）
    name: String, // 显示名，给用户看（如 "Manager" / "tester"）
}

fn local_state_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    #[cfg(target_os = "macos")]
    {
        Some(home.join("Library/Application Support/Google/Chrome/Local State"))
    }
    #[cfg(target_os = "windows")]
    {
        Some(home.join("AppData/Local/Google/Chrome/User Data/Local State"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Some(home.join(".config/google-chrome/Local State"))
    }
}

#[tauri::command]
pub fn list_chrome_profiles() -> Result<Vec<ChromeProfile>, String> {
    let path = local_state_path().ok_or("无法定位 Chrome 配置目录")?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取 Chrome Local State 失败（Chrome 没装或路径不同）：{}", e))?;
    let json: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 Local State 失败：{}", e))?;

    let mut out = Vec::new();
    if let Some(map) = json
        .get("profile")
        .and_then(|p| p.get("info_cache"))
        .and_then(|c| c.as_object())
    {
        for (dir, info) in map {
            let name = info
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(dir)
                .to_string();
            out.push(ChromeProfile {
                dir: dir.clone(),
                name,
            });
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

#[tauri::command]
pub fn open_url_in_chrome_profile(url: String, profile_dir: String) -> Result<(), String> {
    let profile_arg = format!("--profile-directory={}", profile_dir);

    #[cfg(target_os = "macos")]
    {
        // 直接调用 Chrome 二进制传 --profile-directory：比 `open -na` 更可靠地切到指定
        // profile，且会复用现有实例、把窗口带到前台（`open -n` 会开后台新实例，常看不到窗口）。
        let bin = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
        if std::path::Path::new(bin).is_file() {
            std::process::Command::new(bin)
                .args([&profile_arg, &url])
                .spawn()
                .map_err(|e| format!("打开 Chrome 失败：{}", e))?;
            // 保险：把 Chrome 激活到前台（直接调二进制偶尔不抢焦点）
            let _ = std::process::Command::new("open")
                .args(["-a", "Google Chrome"])
                .spawn();
            Ok(())
        } else {
            // 没装在标准路径，退回 open（带 -n 确保 --args 生效）
            std::process::Command::new("open")
                .args(["-na", "Google Chrome", "--args", &profile_arg, &url])
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("打开 Chrome 失败：{}", e))
        }
    }
    #[cfg(target_os = "windows")]
    {
        let candidates = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ];
        for c in &candidates {
            if std::path::Path::new(c).is_file() {
                return std::process::Command::new(c)
                    .args([&profile_arg, &url])
                    .spawn()
                    .map(|_| ())
                    .map_err(|e| format!("打开 Chrome 失败：{}", e));
            }
        }
        std::process::Command::new("cmd")
            .args(["/c", "start", "chrome", &profile_arg, &url])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("打开 Chrome 失败：{}", e))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("google-chrome")
            .args([&profile_arg, &url])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("打开 Chrome 失败：{}", e))
    }
}

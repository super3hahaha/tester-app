//! Codex adapter. Claude call sites keep their existing execution paths.
use serde::Serialize;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

pub fn find_codex() -> Option<String> {
    let home = dirs::home_dir()?;
    let mut candidates = vec![
        home.join(".local/bin/codex"),
        home.join(".codex/bin/codex"),
        home.join(".npm-global/bin/codex"),
        home.join("Library/pnpm/codex"),
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
    ];
    if let Ok(appdata) = std::env::var("APPDATA") {
        candidates.push(PathBuf::from(appdata).join("npm/codex.cmd"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in if cfg!(windows) {
                vec!["codex.exe", "codex.cmd"]
            } else {
                vec!["codex"]
            } {
                candidates.push(dir.join(name));
            }
        }
    }
    if let Some(p) = candidates.into_iter().find(|p| p.is_file()) {
        return Some(p.to_string_lossy().into_owned());
    }
    for shell in ["/bin/zsh", "/bin/bash"] {
        if let Ok(out) = std::process::Command::new(shell)
            .args(["-l", "-c", "command -v codex"])
            .output()
        {
            if out.status.success() {
                for line in String::from_utf8_lossy(&out.stdout).lines().rev() {
                    if PathBuf::from(line.trim()).is_file() {
                        return Some(line.trim().into());
                    }
                }
            }
        }
    }
    None
}

#[derive(Serialize)]
pub struct CodexAccountInfo {
    installed: bool,
    cli_path: Option<String>,
    logged_in: bool,
    error: Option<String>,
}

#[tauri::command]
pub async fn get_codex_account() -> CodexAccountInfo {
    let path = find_codex();
    let mut info = CodexAccountInfo {
        installed: path.is_some(),
        cli_path: path.clone(),
        logged_in: false,
        error: None,
    };
    if let Some(path) = path {
        // Ask the CLI: credentials may live in the keychain or a custom CODEX_HOME.
        let mut cmd = Command::new(path);
        cmd.args(["login", "status"]).kill_on_drop(true);
        match tokio::time::timeout(Duration::from_secs(10), cmd.output()).await {
            Ok(Ok(out)) => info.logged_in = out.status.success(),
            Ok(Err(e)) => info.error = Some(format!("检测失败：{}", e)),
            Err(_) => info.error = Some("检测登录状态超时，请重试".into()),
        }
    }
    info
}

#[derive(serde::Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CodexModel {
    model: String,
    display_name: String,
    #[serde(default)]
    hidden: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelPage {
    data: Vec<CodexModel>,
    next_cursor: Option<String>,
}

/// Discover through the CLI's supported protocol, not a hard-coded/stale model catalog.
#[tauri::command]
pub async fn get_codex_models() -> Result<Vec<CodexModel>, String> {
    let path = find_codex().ok_or("未找到 Codex CLI，请先安装并登录")?;
    read_codex_models(&path).await
}

async fn rpc_response(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    id: u64,
) -> Result<serde_json::Value, String> {
    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if value["id"].as_u64() != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(error["message"]
                .as_str()
                .unwrap_or("Codex 模型列表请求失败")
                .to_owned());
        }
        return value
            .get("result")
            .cloned()
            .ok_or("Codex 返回了无效响应".into());
    }
    Err("Codex 在返回模型列表前退出，请检查 CLI 安装和登录状态".into())
}

async fn read_codex_models(path: &str) -> Result<Vec<CodexModel>, String> {
    let mut cmd = Command::new(path);
    // No threads/turns are started; this only queries the model catalog.
    cmd.args(["app-server", "--listen", "stdio://"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(home) = dirs::home_dir() {
        cmd.current_dir(home);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("无法读取 Codex 模型：{}", e))?;
    let pid = child.id();
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        let mut input = child.stdin.take().ok_or("Codex stdin 不可用")?;
        let mut lines = BufReader::new(child.stdout.take().ok_or("Codex stdout 不可用")?).lines();
        let init = serde_json::json!({"id":1,"method":"initialize","params":{
            "clientInfo":{"name":"tester_app","version":env!("CARGO_PKG_VERSION")}
        }});
        input
            .write_all(format!("{}\n", init).as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        rpc_response(&mut lines, 1).await?;
        input
            .write_all(b"{\"method\":\"initialized\"}\n")
            .await
            .map_err(|e| e.to_string())?;
        let mut models = Vec::new();
        let mut cursor: Option<String> = None;
        let mut cursors = std::collections::HashSet::new();
        let mut seen = std::collections::HashSet::new();
        for id in 2..102 {
            let request = serde_json::json!({"id":id,"method":"model/list","params":{
                "limit":100,"includeHidden":false,"cursor":cursor
            }});
            input
                .write_all(format!("{}\n", request).as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            let page: ModelPage = serde_json::from_value(rpc_response(&mut lines, id).await?)
                .map_err(|e| format!("无法解析 Codex 模型列表：{}", e))?;
            for model in page.data {
                if !model.hidden
                    && !model.model.trim().is_empty()
                    && seen.insert(model.model.clone())
                {
                    models.push(model);
                }
            }
            match page.next_cursor.filter(|c| !c.is_empty()) {
                None => return Ok(models),
                Some(next) if cursors.insert(next.clone()) => cursor = Some(next),
                Some(_) => return Err("Codex 模型列表分页异常，请更新 CLI 后重试".into()),
            }
        }
        Err("Codex 模型列表分页数量异常".into())
    })
    .await
    .unwrap_or_else(|_| Err("读取 Codex 模型列表超时，请稍后刷新".into()));
    kill_tree(pid);
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}

#[derive(Default)]
pub struct Options {
    pub model: String,
    pub writable_dirs: Vec<String>,
    pub images: Vec<String>,
    pub resume: Option<String>,
    pub persist: bool,
}
impl Options {
    pub fn from_config(cfg: &crate::model_config::ModelConfig) -> Self {
        Self {
            model: cfg.codex_model.trim().into(),
            ..Self::default()
        }
    }
}

pub struct Output {
    pub text: String,
    pub session_id: Option<String>,
}

pub fn log(app: &AppHandle, channel: &str, text: &str, kind: &str, done: bool) {
    let _ = app.emit(
        channel,
        serde_json::json!({"text":text,"kind":kind,"done":done}),
    );
}

/// Use the same installed skill and resource paths as Claude, without relying on slash expansion.
pub fn skill_prompt(name: &str, task: &str) -> Result<String, String> {
    let dir = dirs::home_dir()
        .ok_or("找不到用户目录")?
        .join(".claude/skills")
        .join(name);
    let file = dir.join("SKILL.md");
    let instructions = std::fs::read_to_string(&file).map_err(|e| {
        format!(
            "无法加载 skill {}，请在设置中更新 Skill：{}",
            file.display(),
            e
        )
    })?;
    Ok(format!("执行以下 skill 指令。skill 的根目录是 {}，其中 scripts/、references/ 等相对路径均相对此目录解析。使用当前可用的文件/命令/图片工具执行；无需 Claude 的斜杠命令或专用工具名称。\n\n{}\n\n本次任务：\n{}", dir.display(), instructions, task))
}

fn command(path: &str, opts: &Options, output: &std::path::Path, cwd: &std::path::Path) -> Command {
    let mut cmd = Command::new(path);
    cmd.arg("exec")
        .args(["--skip-git-repo-check", "--json", "--color", "never"])
        .args(["-c", "approval_policy=\"never\""])
        .args([
            "--sandbox",
            if opts.writable_dirs.is_empty() {
                "read-only"
            } else {
                "workspace-write"
            },
        ])
        .arg("--output-last-message")
        .arg(output)
        .current_dir(cwd);
    if !opts.model.is_empty() {
        cmd.args(["--model", &opts.model]);
    }
    for dir in &opts.writable_dirs {
        cmd.args(["--add-dir", dir]);
    }
    if !opts.persist {
        cmd.arg("--ephemeral");
    }
    if let Some(id) = &opts.resume {
        cmd.args(["resume", id]);
    }
    for image in &opts.images {
        cmd.args(["--image", image]);
    }
    cmd.arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

struct TempOutput(PathBuf);
impl Drop for TempOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub async fn run(
    prompt: &str,
    opts: Options,
    app: &AppHandle,
    channel: &str,
    set_pid: impl Fn(Option<u32>),
    is_running: impl Fn() -> bool,
) -> Result<Output, String> {
    let path = find_codex()
        .ok_or("未找到 Codex CLI，请先安装 npm install -g @openai/codex，再运行 codex login")?;
    let app = app.clone();
    let channel = channel.to_owned();
    let cwd = dirs::home_dir()
        .ok_or("找不到用户目录")?
        .join(".tester-app/cli-work");
    run_with_path(
        &path,
        prompt,
        opts,
        cwd,
        move |text, kind| log(&app, &channel, text, kind, false),
        set_pid,
        is_running,
    )
    .await
}

async fn run_with_path(
    path: &str,
    prompt: &str,
    opts: Options,
    cwd: PathBuf,
    emit: impl Fn(&str, &str) + Send + Sync + 'static,
    set_pid: impl Fn(Option<u32>),
    is_running: impl Fn() -> bool,
) -> Result<Output, String> {
    if !is_running() {
        return Err("CANCELLED".into());
    }
    std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;
    let output = TempOutput(std::env::temp_dir().join(format!(
        "tester-codex-{}-{}.txt",
        std::process::id(),
        rand::random::<u64>()
    )));
    let mut child = command(path, &opts, &output.0, &cwd)
        .spawn()
        .map_err(|e| format!("启动 Codex 失败：{}", e))?;
    let pid = child.id();
    set_pid(pid);
    emit("正在使用 Codex…", "info");
    let mut stdin = child.stdin.take().ok_or("Codex stdin 不可用")?;
    let input = prompt.to_owned();
    let writer = tokio::spawn(async move { stdin.write_all(input.as_bytes()).await });
    let stdout = child.stdout.take().ok_or("Codex stdout 不可用")?;
    let stderr = child.stderr.take().ok_or("Codex stderr 不可用")?;
    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        let mut session = None;
        let mut failure = None;
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                match v["type"].as_str().unwrap_or("") {
                    "thread.started" => session = v["thread_id"].as_str().map(str::to_owned),
                    "item.completed" if v["item"]["type"] == "agent_message" => {
                        if let Some(t) = v["item"]["text"].as_str() {
                            emit(t, "text");
                        }
                    }
                    "item.started" => {
                        emit(v["item"]["type"].as_str().unwrap_or("正在处理"), "tool")
                    }
                    "turn.failed" => {
                        failure = Some(
                            v["error"]["message"]
                                .as_str()
                                .unwrap_or("Codex 任务失败")
                                .to_owned(),
                        )
                    }
                    _ => {}
                }
            }
        }
        (session, failure)
    });
    let errors = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut tail = std::collections::VecDeque::new();
        while let Ok(Some(line)) = lines.next_line().await {
            tail.push_back(line.chars().take(1000).collect::<String>());
            if tail.len() > 8 {
                tail.pop_front();
            }
        }
        tail.into_iter().collect::<Vec<_>>().join("\n")
    });
    let status = loop {
        if !is_running() {
            kill_tree(pid);
            let _ = child.kill().await;
            let _ = child.wait().await;
            break Err("CANCELLED".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => tokio::time::sleep(Duration::from_millis(100)).await,
            Err(e) => break Err(format!("Codex 进程出错：{}", e)),
        }
    };
    // Clean up any tool descendants that kept inherited stdout/stderr handles open.
    kill_tree(pid);
    set_pid(None);
    let written = writer.await;
    let (session_id, failure) = reader.await.map_err(|e| e.to_string())?;
    let stderr = errors.await.unwrap_or_default();
    if !is_running() {
        return Err("CANCELLED".into());
    }
    let status = status?;
    if !status.success() {
        return Err(format!("Codex 退出失败（{}）：{}", status, stderr));
    }
    written
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("写入 Codex 输入失败：{}", e))?;
    if let Some(error) = failure {
        return Err(error);
    }
    let text =
        std::fs::read_to_string(&output.0).map_err(|e| format!("读取 Codex 结果失败：{}", e))?;
    if text.trim().is_empty() {
        return Err("Codex 未返回内容".into());
    }
    Ok(Output { text, session_id })
}

fn kill_tree(pid: Option<u32>) {
    if let Some(pid) = pid {
        #[cfg(unix)]
        let _ = std::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", pid)])
            .output();
        #[cfg(windows)]
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn command_scopes_permissions_and_preserves_arguments() {
        let cfg = crate::model_config::ModelConfig {
            codex_model: " custom-model ".into(),
            ..Default::default()
        };
        let opts = Options::from_config(&cfg);
        let cmd = command(
            "codex",
            &opts,
            std::path::Path::new("/tmp/result with space"),
            std::path::Path::new("/tmp"),
        );
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.windows(2).any(|a| a == ["--sandbox", "read-only"]));
        assert!(args.windows(2).any(|a| a == ["--model", "custom-model"]));
        assert!(args.contains(&"/tmp/result with space".to_string()));
        assert!(!args.iter().any(|a| a.contains("bypass")));
        let opts = Options {
            writable_dirs: vec!["/tmp/output dir".into()],
            resume: Some("session-id".into()),
            persist: true,
            images: vec!["/tmp/screenshot.png".into()],
            ..Default::default()
        };
        let cmd = command(
            "codex",
            &opts,
            std::path::Path::new("/tmp/output"),
            std::path::Path::new("/tmp"),
        );
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args
            .windows(2)
            .any(|a| a == ["--sandbox", "workspace-write"]));
        assert!(args.windows(2).any(|a| a == ["resume", "session-id"]));
        assert!(args
            .windows(2)
            .any(|a| a == ["--image", "/tmp/screenshot.png"]));
        assert!(!args.contains(&"--ephemeral".into()));
    }

    #[cfg(unix)]
    struct Fixture {
        dir: PathBuf,
        cli: String,
    }
    #[cfg(unix)]
    impl Fixture {
        fn new(body: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let dir =
                std::env::temp_dir().join(format!("tester-cli-test-{}", rand::random::<u64>()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("fake codex");
            std::fs::write(&path, format!("#!/bin/sh\nwhile [ $# -gt 0 ]; do\n if [ \"$1\" = --output-last-message ]; then shift; result=$1; fi\n shift\ndone\n{}", body)).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self {
                cli: path.to_string_lossy().into_owned(),
                dir,
            }
        }
    }
    #[cfg(unix)]
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[cfg(unix)]
    #[test]
    fn subprocess_drains_pipes_and_reads_only_final_message() {
        let fixture = Fixture::new("i=0; while [ $i -lt 5000 ]; do echo 'progress progress progress progress' >&2; i=$((i+1)); done\ncat > /dev/null\nprintf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"my-session\"}' '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"intermediate\"}}'\nprintf '%s' '[{\"text\":\"你好\"}]' > \"$result\"\n");
        let pid = Mutex::new(None);
        let result = runtime()
            .block_on(run_with_path(
                &fixture.cli,
                &"输入".repeat(100000),
                Options::default(),
                fixture.dir.clone(),
                |_, _| {},
                |p| *pid.lock().unwrap() = p,
                || true,
            ))
            .unwrap();
        assert_eq!(result.text, "[{\"text\":\"你好\"}]");
        assert_eq!(result.session_id.as_deref(), Some("my-session"));
        assert!(pid.lock().unwrap().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn subprocess_reports_failure_and_missing_output() {
        for (script, expected) in [
            ("cat >/dev/null; echo 'login required' >&2; exit 7", "login required"),
            ("cat >/dev/null; exit 0", "读取 Codex 结果失败"),
            ("cat >/dev/null; printf '%s' '{\"type\":\"turn.failed\",\"error\":{\"message\":\"quota exceeded\"}}'; echo x > \"$result\"", "quota exceeded"),
            ("cat >/dev/null; : > \"$result\"", "Codex 未返回内容"),
        ] {
            let fixture = Fixture::new(script);
            let result = runtime().block_on(run_with_path(&fixture.cli, "test", Options::default(), fixture.dir.clone(), |_, _| {}, |_| {}, || true));
            assert!(result.err().unwrap().contains(expected));
        }
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_stops_process_and_clears_pid() {
        let fixture = Fixture::new("cat >/dev/null; sleep 30");
        let started = std::time::Instant::now();
        let pid = Mutex::new(None);
        let result = runtime().block_on(run_with_path(
            &fixture.cli,
            "test",
            Options::default(),
            fixture.dir.clone(),
            |_, _| {},
            |p| *pid.lock().unwrap() = p,
            || started.elapsed() < Duration::from_millis(250),
        ));
        assert_eq!(result.err().as_deref(), Some("CANCELLED"));
        assert!(pid.lock().unwrap().is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_runs_do_not_share_output_files() {
        let fixture = Fixture::new("cat > \"$result\"");
        runtime().block_on(async {
            let mut tasks = Vec::new();
            for prompt in ["first", "second"] {
                let path = fixture.cli.clone();
                let cwd = fixture.dir.clone();
                tasks.push(tokio::spawn(async move {
                    run_with_path(
                        &path,
                        prompt,
                        Options::default(),
                        cwd,
                        |_, _| {},
                        |_| {},
                        || true,
                    )
                    .await
                    .unwrap()
                    .text
                }));
            }
            assert_eq!(tasks.remove(0).await.unwrap(), "first");
            assert_eq!(tasks.remove(0).await.unwrap(), "second");
        });
    }

    #[cfg(unix)]
    #[test]
    fn model_list_handles_pagination_notifications_and_hidden_models() {
        let fixture = Fixture::new(
            r#"
read init
printf '%s\n' '{"method":"notice"}' '{"id":1,"result":{}}'
read initialized
read first
printf '%s\n' '{"id":2,"result":{"data":[{"model":"one","displayName":"One"},{"model":"hidden","displayName":"Hidden","hidden":true}],"nextCursor":"page2"}}'
read second
printf '%s\n' '{"id":3,"result":{"data":[{"model":"one","displayName":"One"},{"model":"two","displayName":"Two"}],"nextCursor":null}}'
cat >/dev/null
"#,
        );
        let models = runtime().block_on(read_codex_models(&fixture.cli)).unwrap();
        assert_eq!(
            models.iter().map(|m| m.model.as_str()).collect::<Vec<_>>(),
            vec!["one", "two"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn model_list_returns_rpc_error() {
        let fixture = Fixture::new(
            r#"
read init
printf '%s\n' '{"id":1,"result":{}}'
read initialized
read first
printf '%s\n' '{"id":2,"error":{"message":"Please log in"}}'
cat >/dev/null
"#,
        );
        let error = runtime()
            .block_on(read_codex_models(&fixture.cli))
            .unwrap_err();
        assert_eq!(error, "Please log in");
    }

    #[test]
    #[ignore]
    fn live_codex_model_list() {
        let models = runtime().block_on(get_codex_models()).unwrap();
        assert!(!models.is_empty());
        assert!(models.iter().all(|m| !m.hidden && !m.model.is_empty()));
        println!("Codex returned {} picker models", models.len());
    }

    /// Explicit opt-in smoke test: never sends business data or runs by default.
    #[test]
    #[ignore]
    fn live_codex_smoke() {
        let path = find_codex().expect("Codex CLI installed");
        let cwd =
            std::env::temp_dir().join(format!("tester-codex-smoke-{}", rand::random::<u64>()));
        let logs = Arc::new(Mutex::new(Vec::new()));
        let logs_out = logs.clone();
        let result = runtime().block_on(run_with_path(
            &path,
            "只回复 CODEX_OK。不要调用任何工具。",
            Options::default(),
            cwd.clone(),
            move |text, _| logs_out.lock().unwrap().push(text.to_owned()),
            |_| {},
            || true,
        ));
        let _ = std::fs::remove_dir_all(cwd);
        assert_eq!(result.unwrap().text.trim(), "CODEX_OK");
    }
}

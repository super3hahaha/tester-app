# Handoff: 接入 Codex CLI（设置里切换 Claude / Codex 引擎）

> 状态：**方案已定，未动代码**。调研结论已在本机用真实命令验证（codex 0.141.0，已登录）。
> 落地方式：**先纯后端 Rust**，走 app 更新机制发版 → 手动改配置实测 → 跑通后再做前端。

## 一、已确认的决策

| 项 | 结论 |
|---|---|
| 认证 | 对齐 Claude：app 不存 key，用户自己 `codex login`，app 只读 `~/.codex/auth.json` 判登录态 |
| 引擎开关 | **全局一个**，对应 `ModelConfig.cli_engine`（`"claude"` / `"codex"`，已存在），**在设置页「模型配置」区下拉切换** |
| 落地顺序 | **后端 + 设置页 UI 同一版交付**（不再拆"先发后端、UI 后补"——引擎开关是用户日常要碰的，必须有 UI，不能让用户手改 JSON） |
| 阶段一范围 | **只接「单条回复」** `generate_single_reply` 试水（功能范围不变，只是这一版连 UI 一起发） |
| 不新建文件 | 例外：允许新建职责单一的引擎模块 `cli.rs`（见四）；配置 `model_config.rs` 已有字段，不动 |

## 二、范围划分（硬约束）

| 类别 | 功能 | 入口 | Codex |
|---|---|---|---|
| 直出类 | 单条回复 `generate_single_reply` | [reply.rs:334](../src-tauri/src/reply.rs) | ✅ 阶段一 |
| 直出类 | 评论分析 `generate_analysis` | analysis.rs:282 | ⬜ 阶段二 |
| 直出类 | 邮件回复 `generate_mail_reply` | reply.rs:807 | ⬜ 阶段二 |
| 直出类 | 模板翻译 `translate_one_batch` | translate.rs:226 | ⬜ 阶段二 |
| skill 类 | 测试用例 `/test-case-generator` | claude.rs:493 | ❌ 阶段三再议 |
| skill 类 | 批量模板匹配 `/review-reply` | reply.rs:492 | ❌ 阶段三再议 |

**skill 类为何不能简单接**：依赖 Claude Code 独有的 `/skill` 斜杠命令 + SKILL.md 自动加载 + 工具注入。Codex `exec` 无等价物。要支持只能把 skill 指令内联成普通 prompt，等于每个 skill 重写引擎无关版、效果不保证一致、维护翻倍——故单列阶段三，看一/二阶段效果再定。

## 三、调研结论（已实测，本机 codex 0.141.0）

**能跑通，输出干净，速度与 Claude 差不多（~15s）。**

关键决策：**用 `-o/--output-last-message <文件>`，不解析 JSONL**。
- codex 把最终回复**只**写进该文件，无版本头、无 prompt 回显、无 reasoning 噪音。
- 实测输出就是干净的一行 JSON 数组，可直接喂现成的 [`extract_json_array`](../src-tauri/src/reply.rs)（reply.rs:226），无需任何 JSON 事件解析。
- 这比早前设想的「解析 `--json` 取 `agent_message`」更简单、更稳、不受 codex 版本字段变动影响。

实测命令（可复跑验证）：
```bash
echo '<prompt>' | codex exec - -o /tmp/out.txt \
  --skip-git-repo-check --dangerously-bypass-approvals-and-sandbox
# /tmp/out.txt = 干净的最终回复文本
```

CLI 事实：
- 本机路径 `~/.local/bin/codex`，版本 `codex-cli 0.141.0`
- 关键 flag 均存在：`-m/--model`、`-o/--output-last-message <FILE>`、`--json`、`--skip-git-repo-check`、`--dangerously-bypass-approvals-and-sandbox`、`-C/--cd`
- prompt 经 stdin 传入用 `codex exec -`（末尾 `-`）

### auth.json 结构（实测，ChatGPT 登录态）
- 路径 `~/.codex/auth.json`（或 `$CODEX_HOME/auth.json`）
- 外层两块：`OPENAI_API_KEY`（ChatGPT 登录下为 `null`）、`tokens { id_token(JWT), access_token, refresh_token, account_id }`
- **登录判定 = 文件存在 且 `tokens.access_token` 非空**（不能只看 `OPENAI_API_KEY`，它是 null）
- 订阅 badge **暂不做**（plan 藏在 id_token JWT 里，需解码，锦上添花、先省）

## 四、架构原则：加引擎 ≠ 重构现有代码

**核心约束（来自用户）：原有 claude 逻辑跑得好好的，不许为了加 codex 去动它、引入风险。**

把两件事拆开，本阶段只做第一件：
- **本阶段做**：加一个独立引擎模块 `cli.rs` 放 codex，调用点用「提前返回」插入 codex 分支——**claude 路径代码一字不改**。
- **以后独立做**：把现在重复 5 处的 claude 调用收敛进 `cli.rs`（真正动现有代码的重构）。等 codex 验证 OK、有对照了再做，**不绑本阶段**。

四个硬保证：
1. `claude.rs` **完全不碰**（`find_codex` 放 `cli.rs`，不进 claude.rs）
2. 现有 claude 路径代码 **零改动**——用 early-return 插 codex 分支，不是包 `else`（避免整块缩进）
3. codex 全是新增代码，隔离在 `cli.rs`，**通过闭包回调**设置进程 pid / 查取消，不依赖 `ReplyState` 等任何现有类型 → 零耦合
4. 默认 `cli_engine="claude"` 时走原代码，**行为与现在 100% 一致**；只有手动切 `"codex"` 才进新代码，出问题改回配置即恢复。

## 五、阶段一改动清单（纯后端，只接单条回复）

| 文件 | 改动 | 风险 |
|---|---|---|
| `cli.rs`（新建） | `find_codex` + `run_codex_oneshot` + `get_codex_account`，纯新增 | 无（全新，无人依赖） |
| `lib.rs` | 加一行 `mod cli;`，注册 `get_codex_account` command | 无 |
| `reply.rs` | 单条回复插一个 codex early-return 块 | 极低（claude 代码不动） |
| `claude.rs` | **不动** | 无 |
| `SettingsPage.vue` | 「模型配置」区加引擎下拉 + codex 模型输入 + codex 登录态显示（见六） | 极低（纯加法，claude 默认行为不变） |

### 1) `cli.rs`（新建）
- `pub fn find_codex() -> Option<String>`：仿 [`find_claude`](../src-tauri/src/claude.rs)（claude.rs:11）。候选路径：`/usr/local/bin/codex`、`/opt/homebrew/bin/codex`、`~/.npm-global/bin/codex`、`~/Library/pnpm/codex`、`~/.local/bin/codex`、`~/.codex/bin/codex`，Windows 用 `codex.cmd`，再 login shell `which codex` 兜底。Codex 自己读 auth.json，**无需注入 token**。
- `pub async fn run_codex_oneshot(prompt: &str, model: &str, app: &AppHandle, set_pid: impl Fn(Option<u32>), is_running: impl Fn() -> bool) -> Result<String, String>`：
  1. `find_codex()`，找不到报「未找到 Codex CLI，请先 `codex login`」
  2. 临时文件 `std::env::temp_dir().join("tester-codex-<pid>.txt")`
  3. spawn `codex exec - -o <tmp> --skip-git-repo-check --dangerously-bypass-approvals-and-sandbox`，`model` 非空则加 `-m <model>`
  4. stdin 写 prompt；`set_pid(child.id())`（调用方在闭包里写自己的 State）
  5. **必须 drain stderr**（codex 进度打 stderr，不读会塞满管道 → hang）
  6. `child.wait()` 后：`!is_running()` 返回 `"CANCELLED"`，再查退出码
  7. 读临时文件 → raw 文本 → 删临时文件 → 空则报错
- 闭包设计的意义：`cli.rs` 不依赖 `ReplyState`，将来 analysis/translate 可直接复用（传它们各自 State 的访问闭包）。

### 2) [reply.rs](../src-tauri/src/reply.rs) — 单条回复插 codex early-return
顶部 import 加 `use crate::cli;`。`generate_single_reply_inner`（reply.rs:334）在 build prompt（reply.rs:353）之后插入：
```rust
if crate::model_config::load().cli_engine == "codex" {
    let cfg = crate::model_config::load();
    let raw = cli::run_codex_oneshot(
        &prompt, cfg.codex_model.trim(), &app,
        |pid| { *app.state::<ReplyState>().child_pid.lock().unwrap() = pid; },
        || *app.state::<ReplyState>().running.lock().unwrap(),
    ).await?;
    let json_str = extract_json_array(&raw)
        .ok_or_else(|| format!("模型输出里找不到 JSON 数组：{}", raw.chars().take(300).collect::<String>()))?;
    let candidates: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("候选不是合法 JSON 数组：{}", e))?;
    return Ok(GenReplyResult { candidates, usage: None });   // 阶段一 usage 先 None
}
// ↓ 以下原有 claude 逻辑：一字不改、不缩进、不挪位
```

`model_config.rs` 不动（字段已存在，默认 `"claude"`）。`get_codex_account` 本版一起加（见六，UI 要显示登录态）。

## 六、前端 UI（设置页「模型配置」区，本版必做）
引擎开关是用户日常要切的，必须有 UI，不让用户手改 JSON。改 [`SettingsPage.vue`](../src/pages/SettingsPage.vue)：

1. **`ModelConfig` interface（SettingsPage.vue:93）+ 默认值（:101）** 补两字段：`cli_engine: string`（默认 `"claude"`）、`codex_model: string`（默认 `""`）。后端 `get_model_config` / `save_model_config` 已透传，无需动后端配置代码。
2. **「模型配置」区（SettingsPage.vue:357）加引擎下拉**：仿现有 `.model-row`，下拉 `Claude（默认）` / `Codex`，绑 `modelConfig.cli_engine`。选 `Codex` 时 `v-if` 再显示一行 codex 模型输入框（绑 `codex_model`，placeholder「空=用 Codex 默认模型，如 gpt-5 / o3」）。复用现有「保存」按钮（`saveModelConfig`）。
3. **codex 登录态**：仿 `claudeInfo` / `refreshClaude` 加 `codexInfo` + `refreshCodex()`，`onMounted` 调一次，后端新增 `get_codex_account` command（`cli.rs` 里实现：`find_codex()` 判安装 + 读 `~/.codex/auth.json` 判 `tokens.access_token` 非空判登录）。未安装/未登录时在引擎下拉旁给红字提示「未检测到 Codex，请先 `codex login`」。

### 自测方法（开发期，UI 没接好前的兜底）
UI 落地前可手动编辑 `~/.tester-app/model-config.json` 把 `cli_engine` 改 `"codex"` 重启验证后端通路；这只是**开发自测手段，不是给用户的最终形态**——交付必须走上面的设置页 UI。

## 七、坑 / 风险
- **stderr 必须 drain**，否则 codex 进度输出塞满管道导致 hang。
- **流式日志退化**：codex 模式下没有 Claude 那种逐工具日志，阶段一只发「正在用 Codex 生成…」+ 最终结果即可。
- **usage/cost**：codex 的用量在别处，阶段一 `usage=None`，前端 cost 展示需容忍空值。
- **认证 app 代办不了**：用户未 `codex login` 时要给清晰提示。
- `--dangerously-bypass-approvals-and-sandbox`：单条回复只读不写文件，用它跳过审批是安全的；若未来 codex 跑会改文件的任务需重新评估沙箱策略。

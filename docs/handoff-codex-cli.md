# Codex CLI 全局接入

> 2026-09-28：已实现后端 + 设置页。本文替代旧的「只接单条回复」分阶段方案；用户本次明确要求全局切换。

## 配置与范围

- 设置 → 模型配置 → 全局 AI 引擎：Claude CLI / Codex CLI，点保存生效，不需重启。
- `~/.tester-app/model-config.json`：`cli_engine` 默认 `claude`；`codex_model` 为空时沿用 Codex CLI 默认模型。Claude 的 reply/analysis/translate/testcase 配置保留，切回后继续使用。
- 全部模型入口接入：单条/批量评论回复、邮件回复、评论分析、模板翻译（含超长压缩）、测试用例及续聊、知识提炼、PRD 风险沉淀和补充测试点。
- 确定性 Python 对比脚本不使用模型，不涉及引擎切换。
- 每个任务读取配置快照；翻译多批及压缩沿用同一份。运行中改设置不迁移已启动任务。已有用例会话若切换引擎，续聊明确提示重新生成，禁止把 Claude session ID 发给 Codex，反之亦然。

## 实现与约束

| 位置 | 职责 |
|---|---|
| `src-tauri/src/cli.rs` | Codex 路径发现、登录探测、参数、子进程、取消、JSONL 日志/会话、最终输出、skill 适配 |
| 各业务模块 | Codex early-return 分支，复用原提示词、解析和落盘契约；默认 Claude 路径保留 |
| `claude.rs` | 保留现有 command/事件名；用例生成加入 Codex 分支，`codex:` 前缀隔离会话 ID；保存续聊所需目录 |
| `SettingsPage.vue` | 全局引擎切换、Codex 模型下拉选择、安装/登录状态；Claude 模型配置仍保留 |

- 模型下拉调用 `get_codex_models` → 临时 `codex app-server` → initialize / initialized / 分页 model/list（includeHidden=false），20 秒超时并清理进程；不启动生成、不硬编码模型清单。保留默认模型和未出现在清单里的已保存配置，失败可刷新且不改写配置。
- 认证：由用户安装 `npm install -g @openai/codex`、执行 `codex login`；App 不保存 key。用 `codex login status` 检测，兼容 Keychain / CODEX_HOME，不读取/返回凭证。
- Codex：stdin 输入，`--json` 只解析日志/会话；`--output-last-message` 读取最终文本，随机文件名 + RAII 清理，避免并发互串。
- stdout / stderr / stdin 并发处理；错误保留有限 stderr 尾部；取消轮询 running 标志并终止进程树。Unix 独立进程组，Windows taskkill /T。
- 普通生成使用 read-only 沙箱；skill 使用 workspace-write 和明确的目录；approval_policy=never。不使用全局 bypass。cwd 为 `~/.tester-app/cli-work`，避免继承安装包或项目目录。
- skill 共用 App 热更新维护的 `~/.claude/skills/<name>/SKILL.md`，内联其内容并明确资源根目录，不维护第二套 skill、不依赖 Codex 的 slash 展开。缺失时提示在设置中更新 Skill。
- 用例生成持久化 session，续聊 `exec resume <id>` 并传原目录及新截图；一次性任务 `--ephemeral`。
- Codex 用量暂不映射 Claude 的费用格式：业务返回 usage=null；不要据此展示 $0 为真实费用。

## 验证

- `npm run build`
- `cargo check --manifest-path src-tauri/Cargo.toml --offline`
- `cargo test --manifest-path src-tauri/Cargo.toml --offline --lib`：旧配置/无效引擎、命令权限与参数、大输入/大量 stderr、非零退出/缺输出/turn.failed、取消、并发隔离。
- 本机真实连通性测试已通过（2026-09-28，返回 `CODEX_OK`）。重跑需登录和联网，只发送固定短句、不使用业务数据：
  `cargo test --manifest-path src-tauri/Cargo.toml --offline --lib cli::tests::live_codex_smoke -- --ignored --nocapture`
- Windows 和真实业务 skill 产物的端到端验收仍需在对应机器/业务输入下进行；构建通过不等于已验证各 skill 输出质量。

参数依据：[OpenAI 官方非交互模式文档](https://learn.chatgpt.com/docs/non-interactive-mode)，同时核对本机 `codex exec --help` / `codex exec resume --help`。

模型列表接口：[OpenAI App Server 文档](https://learn.chatgpt.com/docs/app-server)。本机协议用 `codex app-server generate-json-schema` 核对。

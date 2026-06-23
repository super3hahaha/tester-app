# Gotchas

## MainPage 子页用 v-show，组件不重新挂载

`MainPage.vue` 的所有子页（Sheets/Generate/Compare/Review/BatchReply/Settings）都用 `v-show` 切换，**组件常驻挂载**，`onMounted` 只在 MainPage 首次挂载时跑一次。

后果：任何「在 onMounted 里读一次 localStorage/状态」的子页，在用户去别的子页改了配置再切回来时**不会刷新**。

典型坑：BatchReply Config 页保存配置 → 切到 Run 页仍显示「未配置启用任何应用」，且「拉取候选评论」按钮在 `groups.length===0` 时禁用，连 `handleFetch` 里的兜底重建都点不动 —— 彻底卡死。

修复模式：把 `activeOption` 当 prop 传给子页，`watch` 它，页面变为可见时重新读状态。注意加守卫（如 BatchReplyPage 只在 `fetchedAt===null` 时重建，避免清掉已拉取的候选）。

## Claude CLI 凭据在 macOS 上存钥匙串，不是文件

Claude Code 在 Linux/Windows 把 OAuth 凭据写 `~/.claude/.credentials.json`，但 **macOS 上存系统登录钥匙串**（service 名 `Claude Code-credentials`），该文件在 mac 上根本不存在。

后果：只读文件判断登录态会在 mac 上永远报"未登录"（Settings 页的 Claude CLI 卡片）。

修复：`claude.rs::read_credentials_json()` 先读文件，读不到时在 `#[cfg(target_os = "macos")]` 下回退执行 `security find-generic-password -s "Claude Code-credentials" -w`，钥匙串里存的就是和文件一模一样的 `{"claudeAiOauth":{...}}` JSON。`get_claude_account` 和 `load_claude_token` 都走这个 helper。

## BatchReply：拉取并行 + 匹配按钮没等拉取完成 → 整个 app 被静默漏掉

`handleFetch` 用 `Promise.all` 并行拉取各 app，`fetchOne` 一进来就 `g.candidates = []`。一个 app 拉完、另一个还在飞时，`totalCandidates>0` 已成立。

坑：「匹配模板并填充」「一键提交全部」按钮的 `:disabled` 早期**只看 `totalCandidates===0`，没看 `fetching`**。于是慢拉取的那个 app 还在飞（candidates 暂为 []）时点匹配 → `buildSkillGroups` 里该 group 0 条 → 整个 group 不进 skill 输入 → 永不匹配。之后它拉回来显示出候选但回复是空的，「一键提交全部」只提交有内容的 → 该 app 被静默漏掉。从落盘的 `~/.tester-app/reviews/pending-reviews-*.json`（skill 输入）能看出哪个 group 没进去。

修复：两个按钮的 `:disabled` 都加 `fetching ||`，`generateReplies` 入口加 `if (fetching || groups.some(g=>g.loading)) return`。

## `window.confirm` / `alert` 在 Tauri webview 里不弹、静默返回 false

Tauri 的 webview（WKWebView / WebView2）对同步对话框 `window.confirm()`、`window.alert()` **默认不弹窗，`confirm` 直接返回 `false`**。

后果：任何 `if (!window.confirm(...)) return;` 的代码会被静默拦死 —— 用户点按钮"没反应"。曾导致：评论页 AI 回复弹窗点「确认提交到 Play」无效、批量页「一键提交全部」、配置页「清空配置」全部点了无反应。

修复：**别用 `window.confirm`**。
- 单纯的确认动作（按钮文案已是"确认提交"）→ 直接执行，去掉 confirm。
- 危险/批量操作 → 用**内联两步确认**：第一次点把按钮置为 armed 态（文案变"再点一次确认"+ 变色），4 秒内再点才执行，超时 `setTimeout` 复位。零依赖，见 `BatchReplyPage.handleSubmitAll` / `BatchReplyConfigPage.resetAll`。
- 真要原生弹窗 → 装 `@tauri-apps/plugin-dialog` 的 `confirm/ask`（异步），需加依赖 + 权限 + 重新 build。

## Claude CLI 并发模型：跨模块并行、同模块互斥、无全局上限

调 claude cli 的有 4 个模块，**各自持有独立的状态锁**（互不感知）：`claude.rs::ClaudeState`（测试用例生成）/ `reply.rs::ReplyState`（评论回复）/ `translate.rs::TranslateState`（模板翻译）/ `analysis.rs::AnalysisState`（评论分析）。每个 State 有自己的 `running` + `child_pid`。

- **同一模块内 = 串行（互斥）**：进入先查自己的 `running`，已在跑直接 `Err`（如「已有回复生成任务在进行中」/「Claude is already running」）。第二次调用被拒，不排队。
- **不同模块之间 = 并行**：锁独立，每个任务 `Command::new(claude_path).spawn()` 起一个**独立 claude 子进程**。回复+翻译+分析同时触发 = 3 个 claude 进程真正并行。

**没有全局并发上限**（lib.rs / 各模块均无 Semaphore）。当前只有 4 个功能，最多 4 个并发进程，不至于打到速率限制，故**刻意不加**全局信号量（2026-06-23 决定）。后续若功能变多、或某模块改成内部批量并发起多进程，撞 429 会是明显信号 —— 届时再加一个跨模块共享的 `tokio::sync::Semaphore`（spawn 前先拿令牌）。

## 单条 AI 回复：后端一次只跑一个，前端必须自己排队 + 按任务路由日志

`reply.rs::ReplyState` 只有一个全局 `running` 锁 + 一个 `child_pid`，`generate_single_reply` 一进来发现 `running` 就直接 `Err("已有回复生成任务在进行中")`。`stop_reply` 也只杀那一个 `child_pid`。`reply-log` 事件是全局广播，**不带任务标识**。

坑：ReviewPage / BatchReplyPage 的 AI 回复弹窗原本是**单例**（一个 `aiReview`/`aiDlg` + 一个 `aiGenerating`）。缩小第一条去回复第二条时，复用同一份状态 → 第二条上来就显示「停止」按钮，根本开不了新生成。

修复模式（纯前端，不动 Rust）：弹窗状态改成**任务列表** `aiTasks: AiTask[]` + `activeTaskId`（展开中的那个，其余以悬浮条堆右下角）。生成走**前端队列**：`enqueueGenerate` 把任务标 `queued` → `processQueue` 用 `genBusy` 串行化，一次只 `invoke` 一个，结束后自动跑下一个排队的（这样永远不会撞上后端的 running 锁）。`reply-log` 监听里路由到 `aiTasks.find(t => t.status==='generating')` 那个任务，不再用全局 ref。停止：`queued` 直接出队不碰后端，`generating` 才 `invoke("stop_reply")`。

## 模板预翻译：app 原生语言码 vs ISO 码必须归一

模板 `translations` 的 key 用 **app 原生码**（`in` / `zh-rCN` / `zh-rTW`），而 review-reply 的回复语言来自 `target_language` / `reviewer_language`，是 **ISO 码**（`id` / `zh-CN` / `zh-TW`）。两套码不归一就永远命中不到预存译文、白白实时翻译。

- skill 端（review-reply SKILL.md 第 5 步）查 `translations` 前先归一：`zh-CN`/`zh-Hans`→`zh-rCN`、`zh-TW`/`zh-Hant`→`zh-rTW`、`id`→`in`、`pt-BR`→`pt`。
- 后端（`translate.rs` 经 `templates::is_source_lang`）同样处理：源 `zh-CN` 时目标 `zh-rCN` 视为同源，不翻不存（查询时归一到源直接用 text）。

## 模板翻译分批：每批必须立刻写盘

`translate_templates` 按 `CHUNK=1`（一条模板 × 多语言一次调用）轻量直出分批（不走 skill，见 decisions.md）。**每批 `apply_translations` 立刻落盘**——用户中途点停止（`stop_translate` 杀进程）/ 某批失败时，已完成的批次才不丢。取消返回「已完成部分已保存」，不是整批作废。

## LLM 控不住字数：350 字符必须后端硬校验 + 压缩兜底

模板翻译每条要 ≤ 350 字符（gp 回复硬限制），但**光靠 prompt 约束 LLM 字数不可靠**——haiku 实测把俄语模板翻成 371（俄/德/法/西比英文长 20-30%，直译就超）。`translate.rs` 三层兜底（单条重译 / 批量补全都走 `translate_one_batch`，故都生效）：
1. prompt 把 350 写成硬上限、提示长语言主动精简（`build_prompt`）。
2. 每批翻完按 `char_len`(=`chars().count()`) 校验，超 350 的发**一次压缩调用**（`build_compress_prompt`）改写到限内。
3. 仍超的：写入但 emit `⚠ 仍超` 日志 + 前端编辑行字符数标红；单条重译完成后红 banner（无弹窗看不到日志，靠前端扫）、批量完成消息汇总「N 条仍超」。
- 字符计数前后端要对齐：后端 `chars().count()` / 前端 `.length`，对 BMP 字符（俄/中/拉丁）一致，emoji 会差一点（少见，可接受）。

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

## Apps Script 同步 Gmail：emoji 标签必须用 `getUserLabelByName`，不能用 search query

`GmailApp.search('label:⭐mp3cutter-50字+-')` 对含 emoji / 特殊字符的标签**匹配不到**（Google 内部 label id 不是显示名）。
必须用 `GmailApp.getUserLabelByName('⭐mp3cutter-50字+-')` 拿到 Label 对象再 `.getThreads()`。
`gmail-sync.gs` 已按此实现（`LABEL` 常量填显示名，`getUserLabelByName` 取对象），新账号部署时 `LABEL` 改成该账号的标签显示名即可。

## LLM 控不住字数：350 字符必须后端硬校验 + 压缩兜底

模板翻译每条要 ≤ 350 字符（gp 回复硬限制），但**光靠 prompt 约束 LLM 字数不可靠**——haiku 实测把俄语模板翻成 371（俄/德/法/西比英文长 20-30%，直译就超）。`translate.rs` 三层兜底（单条重译 / 批量补全都走 `translate_one_batch`，故都生效）：
1. prompt 把 350 写成硬上限、提示长语言主动精简（`build_prompt`）。
2. 每批翻完按 `char_len`(=`chars().count()`) 校验，超 350 的发**一次压缩调用**（`build_compress_prompt`）改写到限内。
3. 仍超的：写入但 emit `⚠ 仍超` 日志 + 前端编辑行字符数标红；单条重译完成后红 banner（无弹窗看不到日志，靠前端扫）、批量完成消息汇总「N 条仍超」。
- 字符计数前后端要对齐：后端 `chars().count()` / 前端 `.length`，对 BMP 字符（俄/中/拉丁）一致，emoji 会差一点（少见，可接受）。

## AI 生成回复的知识库注入：必须按 packageName 解析产品，不能信前端传的 product

知识块（含反馈邮箱等）按**模板产品名**存：`~/.tester-app/review-analysis/{product_prefix}.md`（如 `xfolder.md`）。
但前端 `generate_single_reply` 传的 `product` 参数是 **app 显示名**（`appLabel` / `displayName`，如 "File Manager"），不是模板产品名（"XFolder"）。
所以后端读知识块**必须** `product_for_package(package_name)` → 产品名 → `read_knowledge(产品名)`，与 analysis.rs 一致；
**不能**直接 `read_knowledge(前端传的 product)`——会用 "File Manager" 找不到 `xfolder.md`、返回空知识块。

踩坑现象：知识库明明配了 `filemanager.feedback@gmail.com`，但 AI 回复里填成了 `xxtester2026@gmail.com`（**用户自己的账号邮箱**）。
根因两层：①知识块读空（上面的 product 错配）→ 模型没邮箱可用；②子进程 `claude --print` 继承了 Claude Code 账号上下文（`~/.claude/CLAUDE.md` + userEmail 注入），模型就抓了账号邮箱顶上。
修复①即可——知识块正确注入后模型用知识库里的邮箱。但要记住子进程会带账号身份信息，prompt 里凡是"不确定就别编"的字段（邮箱/版本号等）都得靠知识块显式喂。

## 多账号：迁移用 email-key、登录用 sub-key 会撞同一账号 → 必须启动去重

多账号存储 key 用 Google `sub`（OpenID 唯一 ID），但**首启迁移旧单账号时拿不到 sub**（旧 `auth-user.json` 没存 sub，迁移是同步逻辑不便调 userinfo API），只能回退 email 当 key。
于是同一个 Google 账号可能在 `accounts/` 下有两份：`accounts/<sub>/`（正常登录）+ `accounts/<email>/`（迁移残留），下拉里显示**重复账号**。
`start_login` 的按-email 去重只在「运行中重新登录该账号」时触发，迁移残留若用户从不重登就永久共存。
修复：`AuthState::new()` 在 `load_accounts_from_disk` 后调 `dedup_accounts()`——同 email 优先保留带 sub 的、删掉无 sub 的 email-key 残留，并把指向被删 key 的 `active` 重映射到 sub-key 后落盘。这样迁移撞车启动即自愈。
教训：两套 key 来源（迁移 vs 登录）天然不统一，**任何"按内容算 key"的存储都要在加载层做一次按业务唯一键的去重**，不能只在写入路径去重。

## 定时通知：窗口不在前台/被遮挡时，JS 定时器可能被系统挂起（不只是「app 没开」才会迟发）

最初假设"进程活着即可，窗口最小化/不展示也不影响"（见 decisions.md 方案 A）。实测发现：电脑没睡眠、app 进程也在跑，但窗口**长时间不在前台**（被其它窗口遮挡/最小化）时，`setInterval` 仍可能被系统/WebKit（Tauri macOS 用 WKWebView）挂起而不按时触发——配置 10:10 发送，实际到用户把窗口切回前台的 10:31 才触发（说明定时器在后台期间没跑，一恢复前台立刻补发）。
跟章节七「坑 1」的睡眠场景是同一类问题、触发条件更宽：不止睡眠会错过，**长时间不可见的窗口也会**。
**已解决（改到后端方案 B）**：定时逻辑已整体搬到 Rust `schedule.rs`（后台 std 线程），原生线程不受 webview 节流影响，窗口最小化/后台/被遮挡都准点。前端定时器（`scheduleDriver.ts`/`scheduledFetch.ts`）已删除。详见 decisions.md「定时通知从前端定时器改到后端线程」。
教训留档：**任何"必须按时/在后台跑"的逻辑都别放在 webview 的 JS 定时器里**——WKWebView 会在页面不可见时挂起 setInterval/setTimeout。要后台可靠执行就放后端原生线程。

## 定时通知：后端刷新了评论快照，ReviewPage 不会自动重读（要 emit 事件推一把）

ReviewPage.vue 是 v-show 常驻挂载，`onMounted` 只在首次跑一次读快照。后端定时线程在后台
拉完评论、把 `reviews-cache/{账号}__{包名}.json` 更新了，但 ReviewPage 早已挂载、内存里
还是旧数据 → 用户进页面看不到定时拉到的评论，还得手动点「拉取」。
修复：`schedule.rs::execute_and_notify` 巡检完 emit `scheduled-fetch-done`（带 account key），
ReviewPage 监听该事件、非手动拉取中就 `restoreLastView()` 重读快照。冷启动本就会在 onMounted
读盘、无此问题；这个事件专治「app 一直开着、定时在后台刷新」的场景。
教训：**后端在后台改了前端已加载的持久化数据，必须主动 emit 事件通知前端刷新**，不能指望
常驻组件自己发现（v-show 不重挂、onMounted 不重跑）。

## Vue `computed` 会永久缓存 `new Date()` —— 常驻 app 跨天不更新

**现象**：`ReviewPage.vue` 日期选择器把「今天」置灰。系统时间明明是当天，`<input type=date>` 却禁掉了最近几天。

**根因**：`computed` 只在**响应式依赖**变化时重算。若 getter 里调 `new Date()` / `Date.now()`（非响应式），首次求值后结果被**永久缓存**，永不更新。

```js
const maxSelectableDate = computed(() => todayIso());  // ❌ 首次算完就冻结
```

这个 app 是**常驻巡检工具**（用户长期挂着不重启），所以缓存问题会实际暴露：computed 停在 app 打开那天的日期，之后每跨一天，「今天」就多被置灰一格。

**修复套路**（见 `ReviewPage.vue` 的 `dayTick`）：
1. 加一个响应式信号 ref（`const dayTick = ref(0)`），getter 里 `void dayTick.value` 显式建立依赖。
2. 跨天时 `dayTick++` 触发重算。
3. 跨天检测双保险：`visibilitychange`（切回窗口时）**+** `setInterval` 每分钟兜底——因为前台常驻时 `visibilitychange` 根本不触发。`onUnmounted` 记得 `clearInterval`。

**推广**：本项目任何"依赖当前时间"的 computed/派生值都有这个风险（因为 app 常驻）。凡是 getter 里出现 `new Date()`、`Date.now()`、"今天/本周/最近 N 天"，都要问一句"跨天后会不会自动更新"。

**顺带**：跨天判断别用 `toDate.value === today`（用户手动选历史日期时会误触发平移），用 `maxSelectableDate.value === today` 只在真跨天时动。

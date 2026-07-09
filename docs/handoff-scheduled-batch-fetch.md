# Handoff：Review 模块「定时批量拉取 + Telegram 通知」

> 状态：**已实现（方案 A）**。决策见 §四（已标 ✅）。实现清单（§六）全部完成：
> - 后端 `notify.rs`（`send_telegram_message` + `get/save_notify_config` + `is_notify_configured`），`feedback.rs` 新增 `resolve_bot_token()` 供复用。
> - 前端 `utils/scheduleConfig.ts`（配置）、`utils/scheduledFetch.ts`（拉取+diff+baseline）、`utils/scheduleDriver.ts`（tick 驱动+消息组装+错过补发）。
> - `App.vue` 常驻挂驱动（tick + 启动 + visibilitychange）；`ConfigPage.vue` 新增「⏰ 定时通知」Tab → `ScheduleConfigPage.vue`。
> - `cargo check` / `vue-tsc --noEmit` 均通过。**尚未在真机 Tauri 窗口里手动跑通「立即测试发送」和到点触发**——需要用户在自己已运行的 dev 窗口里验证（见对话里的验证说明）。
> - 详细选型理由见 `decisions.md`「Review 模块定时批量拉取 + Telegram 通知」。

## 已确认决策（2026-07-09）

- **方案 A（前端定时器 + app 常开）**。「app 开着」= **进程活着即可，窗口不用展示**：最小化到程序坞 / 窗口关掉但 app 未退 都行；`Cmd+Q` 完全退出则不跑；系统睡眠期间不跑、唤醒后靠「错过补发」补一次。
- **通知目标**：设置页**单独配一个通知 chat_id**（可复用同一 bot token），与反馈私聊分开（决策点 3b）。
- **内容口径**：**仅新增**（已通知 id 集合 diff；首次启用先把现有评论设为 baseline，决策点 4a + 坑 §7）。
- **定时粒度**：**每天固定时间点**（一个或多个 `HH:MM`），不做工作日开关、不做间隔轮询（决策点 2a）。
- **错过补发**：**做**（决策点 5）。拉取口径沿用现有「📦 批量拉取」（决策点 6）。
- **UI 挂载**：`ConfigPage.vue` 第三个 Tab「定时通知」。**页面顶部必须有说明文案**讲清这是什么：例如「到点自动执行一次批量拉取（按『Play Console 拉取配置』里启用的应用及其筛选条件），把**新增差评**通过 Telegram 通知你。需保持 app 运行（可最小化）。」
- **无新增也发心跳**：无新增时发一条「✅ 今日无新差评（时间/账号）」，让用户确认定时确实在跑（覆盖 §五「默认静默」，改为默认发）。

## 一、需求（用户原话）

在 review 模块加「定时」功能：设置时间（如每天早上 10:00），到点自动执行一次「批量拉取」，把结果通过 Telegram 机器人通知用户。通知模板由 Claude 设计。

## 二、现状调研（关键事实）

| 事项 | 现状 | 对本功能的影响 |
|---|---|---|
| 批量拉取逻辑 | **全在前端** `ReviewPage.vue::handleBatchFetch()`：读 localStorage `play-console-multi-config-v1`（scopedKey 按账号隔离）→ 并行 `invoke("list_play_reviews", {packageName, maxPages:5, translationLanguage:"zh-CN"})` → `computeRange` 解析日期预设 → 按日期+星级本地筛选 → 落 per-app 快照 `save_reviews_snapshot` | 定时若要复用这套逻辑，**最省事 = 前端定时**（见方案对比） |
| Telegram 通道 | `feedback.rs` 只有 `upload_to_telegram()` = **sendDocument（发文件）**。**没有 sendMessage（发文本）**。`resolve_telegram_config()`（编译期 env `TELEGRAM_BOT_TOKEN/CHAT_ID` → `~/.tester-app/telegram.json` 兜底）是 feedback.rs 的私有 fn | 需**新增**一个后端命令发文本；token 在后端/编译期，前端拿不到，必须走后端 |
| 现有 chat_id 用途 | 发给**维护者私聊**（skill 反馈用），见 [decisions.md] "bot token 编译期嵌入" | 通知是否复用同一 chat_id = 决策点 3 |
| 定时基础设施 | **无**。后端无 tokio 定时；前端多处用 `setInterval`（PlayConsoleConfigPage/BatchReplyConfigPage 每分钟刷「实际范围」预览等） | 从零加 |
| 账号维度 | 批量拉取用**当前活跃账号**的 token；配置/快照均按账号 scopedKey 隔离 | 定时只能对**当前活跃账号**生效；切账号后对象随之改变（约束，非 bug） |
| 评论时效 | Play Reviews API 只返回**最近 7 天**（[decisions.md]） | 「新增判定」窗口天然 ≤7 天，够用 |
| 子页挂载 | MainPage 子页全 `v-show` 常驻挂载，`onMounted` 只跑一次（[gotchas.md]） | 定时器宜挂在**常驻**处（App.vue 或常驻组件），不受页面切换影响 |

## 三、核心架构决策：定时在哪跑？（决策点 1，最关键）

| 方案 | 说明 | app 需常开？ | 改动量 | 评价 |
|---|---|---|---|---|
| **A. 前端定时器**（推荐） | App.vue 挂一个每分钟 tick 的 `setInterval`，比对当前 `HH:MM` 命中配置则触发。触发逻辑抽成独立 util（复制精简 handleBatchFetch，**不碰旧代码**），拉完调新后端命令发 Telegram | 是（白天开着即可） | **小** | 复用现有前端拉取逻辑，符合「加法不重构」偏好 |
| B. 后端 tokio 定时 | Rust 起 `tokio::time` 定时循环，到点自己拉评论 | 是（进程活着即可，不管前后台） | **大** | 配置在 localStorage、筛选逻辑在前端 → 得把配置和筛选整套搬到后端，重构面大 |
| C. 系统级 cron 无人值守 | 独立 headless 二进制 + cron | 否（关机也跑） | **很大** | 要解决 OAuth token 无头刷新、独立入口，超出当前工具形态 |

**推荐 A**。桌面工具，用户上班时间 app 开着，「每天 10:00」正好落在开机时段。代价：app 没开 / 睡眠中错过时间点 → 靠「错过补发」兜底（见决策点 5 / 坑 §7）。

> ⚠️ 若用户要「电脑关着也能收到通知」→ 只有 C，本方案作废，需重新评估（大概率不做，成本过高）。**这是首要确认项。**

## 四、待确认决策点

1. **定时在哪跑**：A / B / C（见上，推荐 A）。
2. **定时粒度**：
   - (a) 每天 N 个固定时间点（`HH:MM` 列表，如 `10:00`）——推荐，简单够用
   - (b) 另需「每隔 N 小时」轮询
   - 是否需要「仅工作日」开关？
3. **通知发到哪个 Telegram**：
   - (a) 复用现有 feedback 的 bot+chat_id（发到维护者自己那条私聊）
   - (b) 在设置页**单独配**一个「通知 chat_id」（可复用同一 bot token），与反馈分开
   - 推荐 (b)：语义清晰，通知量大时不污染反馈私聊；实现上加一个 `notify.json` 或复用 `telegram.json` 加字段。
4. **通知内容口径**：
   - (a) **仅新增**：只报「自上次通知以来新出现的评论」（用 localStorage 记 `已通知 review_id 集合` 做 diff）——推荐，避免每天重复刷屏
   - (b) 全量：每次把当前符合筛选的全列一遍（会重复）
5. **错过补发**：app 启动时/唤醒后，若发现今天某个时间点已过且当天未触发 → 是否立即补跑一次？推荐**是**（否则合盖过夜就永远收不到）。
6. **拉取口径**：直接复用「Play Console 拉取配置」(`play-console-multi-config-v1`) 启用的 app + 各自星级/日期/回复状态筛选（= 现有「📦 批量拉取」口径）。确认沿用？（默认沿用）

## 五、通知模板设计（Claude 拟，可调）

Telegram `sendMessage`，`parse_mode=HTML`（比 Markdown 转义少踩坑）。**只在有新增时才发**（决策点 4a）；无新增可选「静默不发」或发一条极简「今日无新差评 ✅」（默认静默）。

```
🔔 <b>差评巡检 · 07-09 10:00</b>
账号：xxtester2026@gmail.com

📊 本次新增 <b>8</b> 条（近 7 天 · 按配置筛选）
• File Manager　★1×3 ★2×1
• MP3 Cutter 　★1×2
• XFolder 　　 ★2×2

—— 最新几条 ——
① ★1 File Manager · id 打不开
   "更新后一直闪退，垃圾"
② ★1 MP3 Cutter · 剪辑丢失
   "保存不了，浪费我时间"
③ ★2 XFolder · 广告太多
   "……"

（其余 5 条见 app）
```

设计要点：
- 顶部：时间 + 账号 + 新增总数；**按 app 分组的星级分布**一眼看规模。
- 正文：列**前 N 条**（默认 3–5，可配）最新/最低星，每条：星级 + app + 一句机翻中文摘要（截断 ~40 字）。超出折叠为「其余 M 条见 app」。
- HTML 转义：评论正文含 `<>&` 必须 escape，否则 sendMessage 400。
- 单条消息上限 4096 字符 → 超长自动截断或分条。

## 六、实现清单（确认方案 A 后）

**后端（Rust）**
- [ ] 提取 `resolve_telegram_config()` 为共享（或新增独立通知配置读取）；决策点 3b 则加通知 chat_id 字段/文件。
- [ ] 新增命令 `send_telegram_message(text: String)`：调 `https://api.telegram.org/bot{token}/sendMessage`，`parse_mode=HTML`，校验 `{"ok":true}`。放 `feedback.rs` 或新建 `notify.rs`；在 `lib.rs` 注册。
- [ ] （可选）`is_notify_configured()` 供 UI 判断按钮/开关可用性。

**前端（Vue/TS）**
- [ ] `utils/scheduledFetch.ts`：独立的「读配置 → 并行拉取 → 筛选 → 落快照 → 返回 {新增列表, 各 app 统计}」纯逻辑（**从 handleBatchFetch 复制精简，不改 ReviewPage**）。含「已通知 id 集合」diff（localStorage，账号 scopedKey）。
- [ ] `utils/scheduleConfig.ts`：定时配置类型 + localStorage 读写（`review-schedule-v1`，账号 scopedKey）：`{ enabled, times:["10:00"], weekdaysOnly, notifyOnEmpty, maxItemsInMsg }`。
- [ ] 定时驱动：App.vue（或常驻组件）挂每分钟 tick 的 `setInterval`：命中时间点且当天该点未触发 → 调 scheduledFetch → 组装模板 → `invoke("send_telegram_message")`；记录「上次触发日期+时间点」防重复；启动/唤醒时做错过补发（决策点 5）。
- [ ] 设置 UI：新增「定时通知」入口。**建议挂 review 工作区**——放进 `ConfigPage.vue` 加第三个 Tab「定时通知」，与「Play Console 拉取配置 / Batch Reply 配置」并列（复用现有 Tab 容器）。含：开关、时间点增删、仅工作日、通知 chat_id（若 3b）、每条数量、「立即测试发送」按钮。

**文档**
- [ ] 更新 `PROJECT_STRUCTURE.md`（新增文件/命令）+ `decisions.md`（定时方案选型理由）+ 本 handoff 状态。

## 七、已知坑 / 风险

1. **休眠错过**：`setInterval` 在系统睡眠时不走，mac 合盖过夜会错过 10:00 → 必须做「错过补发」，否则功能形同虚设。
2. **重复触发**：tick 每分钟跑，命中 `10:00` 的那一分钟会 tick 一次，但要防「同一时间点当天多次触发」和「10:00 这一分钟内多 tick」→ 用「(日期, 时间点) 已触发」标记落 localStorage。
3. **`window.confirm` 在 Tauri 不弹**（[gotchas.md]）→ 「立即测试发送」等交互别用 confirm。
4. **账号切换**：定时对象是当前活跃账号；切账号后配置/已通知集合各自隔离（scopedKey 已保证）。
5. **token 过期 / 需重登**：定时触发时若 `list_play_reviews` 返回 `NEED_RELOGIN_SCOPE` → 不能弹 UI，应发一条 Telegram「⚠️ 拉取失败，需重新登录」提示，别静默吞掉。
6. **Telegram 限流**：sendMessage 30 msg/s，本功能一天几条无压力；但错过补发若一次补多天要注意别刷屏。
7. **首次启用**：第一次跑时「已通知 id 集合」为空 → 会把当前 7 天全部当新增。建议首次启用时**先把现有评论标记为已通知**（静默 baseline），从下次起只报真·新增。

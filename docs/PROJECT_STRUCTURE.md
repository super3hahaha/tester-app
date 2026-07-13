# 项目结构

```
tester-app/
├── index.html                   # Vite 入口 HTML
├── package.json                 # 前端依赖与脚本
├── package-lock.json            # 依赖锁定文件
├── vite.config.ts               # Vite 开发服务器配置（端口 1420、HMR）
├── tsconfig.json                # TypeScript 编译器配置（严格模式）
├── tsconfig.node.json           # Vite 配置文件专用 TypeScript 配置
├── dev.bat                      # Windows 快速启动脚本
├── dev.command                  # macOS 快速启动脚本（双击运行）
├── README.md                    # 项目说明
├── src/                         # Vue 3 前端源码
│   ├── main.ts                  # Vue 应用入口，挂载根组件
│   ├── App.vue                  # 根组件：认证状态路由（登录页 ↔ 主页）；watch(user) 把 UserInfo.id/email 同步写入 activeAccountId/activeAccountEmail（账号隔离维度的唯一来源），并首启触发 migrateLegacyStorageOnce；启动 + watch(user) 切账号时把定时配置镜像给后端（syncScheduleRuntimeToBackend）——定时本身跑在后端线程
│   ├── vite-env.d.ts            # Vite 类型声明
│   ├── assets/                  # 静态资源
│   │   └── vue.svg              # Vue logo
│   ├── utils/                   # 跨页面工具模块
│   │   ├── batchReplyDates.ts   # 批量回复的动态日期预设（"自上一个工作日"/"昨天"等 → 执行时解析为绝对范围）
│   │   ├── playConsoleConfig.ts # Play Console 多 app 拉取配置：类型 + 增 replyState 字段 + localStorage 读写规整（与 batchReplyDates 共享日期预设）；读 `play-console-multi-config-v1` 时经 scopedKey 按账号隔离
│   │   ├── templateFavorites.ts # 模板「收藏」：标星模板 id 集合的 localStorage 读写（`tpl-fav-ids-v1`）；模板管理页写、评论页「模板回复」弹窗读（**全局，不按账号隔离**）
│   │   ├── activeAccount.ts     # 【账号隔离】模块级 `activeAccountId`（后端 UserInfo.id 下发的 opaque 账号标识）+ getActiveAccountId()；`activeAccountEmail` 仅供展示（如定时通知模板里的账号行）；唯一写入点在 App.vue 的 watch(user)
│   │   ├── accountScopedKey.ts  # 【账号隔离】scopedKey(base)=`${base}::acct:${id||"_none"}`；给账号相关的 localStorage key 注入账号维度，切账号自然读到各自数据
│   │   ├── accountStorageMigration.ts # 【账号隔离】首启一次性迁移：把旧全局配置 key 搬到当前账号命名空间下（幂等，标记 `store-migrated-v1`）；见 handoff-account-scoped-storage.md
│   │   ├── scheduleConfig.ts    # 定时通知配置（UI 侧真相源）：类型 + localStorage 读写（`review-schedule-v1`，scopedKey 隔离）：enabled/times(HH:MM 列表)/notifyOnEmpty/maxItemsInMsg/checkUpdated(顺带扫「回复后又更新」)
│   │   └── scheduleRuntimeSync.ts # 把「定时配置 + Play Console 启用应用及筛选 + 应用显示名」聚合成后端可读快照，invoke `save_schedule_runtime` 推给 Rust 定时线程。定时**真正执行在后端**（schedule.rs），前端只镜像配置。调用点：ScheduleConfigPage/PlayConsoleConfigPage 保存后、App.vue 启动 + watch(user) 切账号。（旧的纯前端定时器 scheduleDriver.ts/scheduledFetch.ts 已删除——webview JS 定时器窗口不在前台会被节流，做不到后台准点，见 gotchas.md）
│   └── pages/                   # 页面组件
│       ├── LoginPage.vue        # Google OAuth 登录页
│       ├── MainPage.vue         # 主布局：三级导航（工作区 → 选项 → 内容）
│       ├── SheetsPage.vue       # Google Sheets 浏览与选择
│       ├── SlidesPage.vue       # Google Slides 浏览与多页选择
│       ├── GeneratePage.vue     # Claude 测试用例生成（导出 + 流式日志 + 多轮对话）
│       ├── ComparePage.vue      # compare：两个 Sheet 导出 HTML → 调用 diff skill → 在 Chrome 打开报告
│       ├── ReviewPage.vue       # Play Console 评论（单应用页 + 批量按钮）：原有「拉取评论」拉选中单个 app（页面星级/回复状态/日期本地筛选）；新增「📦 批量拉取」读 Play Console 拉取配置启用的 app 并行拉、各按其配置筛选后合并成一个列表（每条带应用标签）。每条评论可 AI 单条回复 + 模板回复（收藏模板快捷取用）+ 添加模板（批量模式下用该评论来源 app 的上下文提交/收录）。**评论快照持久化**：拉取后按 app 落 `reviews-cache/{账号id}__{包名}.json`（前端 snapKey 拼账号前缀，按账号隔离；后端 reviews.rs 不感知账号），进页面/切应用下拉框自动加载本地快照（不自动重拉），批量视图由多份 per-app 快照派生；回复成功按 review_id 同步写回快照。评论页配置 `review-page-config-v3` / 上次视图 `review-last-view-v1` 亦经 scopedKey 按账号隔离。**监听后端定时巡检完的 `scheduled-fetch-done` 事件**：非手动拉取中就优先恢复批量视图重读快照，让后台拉到的评论直接显示在本页（不用手动点拉取）。设备 `androidOsVersion` 是 API Level，经 `androidVersionLabel()` 映射成 Android 版本号显示（如 35→Android 15，表外退回「Android API n」）
│       ├── ConfigPage.vue       # 纯配置页：三个 Tab 容器（Play Console 拉取配置 / Batch Reply 配置 / 定时通知），各嵌一个子配置页（v-show 常驻）
│       ├── PlayConsoleConfigPage.vue # Play Console 拉取配置：每个 app 一张卡片，勾选启用 + 日期预设 + 星级 + 回复状态；显式「保存」写 localStorage `play-console-multi-config-v1`；ReviewPage / scheduledFetch.ts 都读它
│       ├── BatchReplyConfigPage.vue # 批量回复配置（嵌在 ConfigPage 的 Batch Tab）：每个 app 一张卡片，独立勾选启用 + 日期预设 + 星级；显式「保存」按钮写 localStorage
│       ├── ScheduleConfigPage.vue # 定时通知配置（嵌在 ConfigPage 的「⏰ 定时通知」Tab）：开关 + 时间点(HH:MM)增删 + 无新增心跳开关 + 消息条数；Telegram 通知目标（chat_id + 可选独立 bot_token，get/save_notify_config）+「立即测试发送」（invoke send_telegram_message）
│       ├── BatchReplyPage.vue   # 批量回复执行：读保存的多 app 配置 → 并行拉评论 → 按 app 分组折叠 → AI 生成回复 → 逐条/一键提交
│       ├── TemplateManagerPage.vue # 模板管理：按产品 tab 增删改模板 + xlsx 导入/导出 + 多语言预翻译 + 每条 ★收藏（挂 Review 工作区）
│       ├── KnowledgeConfigPage.vue # 知识配置：按产品 tab 编辑「应用知识块」(.md)，单栏全宽编辑器 + 插入骨架 + 保存（过滤掉「通用」产品）；供评论分析注入 {app_knowledge}（挂 Review 工作区，与模板管理同级）
│       ├── KnowledgeBasePage.vue   # 用例知识库：自由资料库 + 资料↔产品多对多关联；一级「知识库」工作区 + 动态二级产品列表 + 「通用」虚拟视图；每个资料 tab 带「管理关联」弹窗（多选产品）；编辑器 + 插入骨架；「🤖 AI 起草/合并」(v2)：传对比图(选文件/粘贴)+说明→kb_ai_distill 提炼偏好填回编辑器；供 GeneratePage 偏好勾选
│       ├── GmailPage.vue        # Gmail 邮件：读 Apps Script 同步的 Sheet，卡片列表 + 详情弹窗 + 已读隐藏 + 按 Chrome profile 打开邮件；可关联邮件回复模板产品（email namespace）
│       ├── AppScriptPage.vue    # App Script 配置：管理待维护的 Apps Script 项目列表（每项含备注/Chrome profile/跳转链接），点「打开 ↗」用指定 profile 打开（默认 https://script.google.com/home?hl=zh-cn）
│       └── SettingsPage.vue     # 设置页：缓存管理（查看/清理缓存）+ 模型配置（回复/分析/翻译各自选 Sonnet/Haiku）+ GitHub Token（供 skill 热更新访问私有 release）
├── public/                      # 公共静态资源
│   ├── tauri.svg                # Tauri logo
│   └── vite.svg                 # Vite logo
├── src-tauri/                   # Rust 后端（Tauri 2）
│   ├── Cargo.toml               # Rust 依赖声明
│   ├── Cargo.lock               # Rust 依赖锁定
│   ├── build.rs                 # Tauri 构建脚本
│   ├── tauri.conf.json          # Tauri 应用配置（窗口、标识符、CSP、构建命令）
│   ├── src/                     # Rust 源码
│   │   ├── main.rs              # 程序入口（最小化，调用 lib）
│   │   ├── lib.rs               # Tauri 应用初始化与命令注册
│   │   ├── auth.rs              # Google OAuth 2.0 + PKCE 认证、令牌刷新与持久化
│   │   ├── sheets.rs            # Google Drive / Sheets / Slides API 封装与导出
│   │   ├── claude.rs            # Claude CLI 子进程管理、流式 JSON 解析与会话续接
│   │   ├── compare.rs           # 对比流程：导出 Sheet 为 HTML、直接跑 Python diff 脚本、在 Chrome 打开报告
│   │   ├── manifest.rs          # 生成-上传 manifest 落盘：记录 Drive ID ↔ 源文件（CSV/PPTX/页码）的映射
│   │   ├── feedback.rs          # 反馈打包：zip + Telegram sendDocument 上传 + 本地归档 / pending 重试；额外导出 resolve_bot_token()（bot token 单独取用，供 notify.rs 复用同一个 bot）
│   │   ├── notify.rs            # 定时通知的 Telegram 文本通道：send_telegram_message(text) 调 sendMessage（HTML 模式）；get/save_notify_config 读写 ~/.tester-app/notify.json（chat_id 独立于 feedback，bot_token 留空则复用 feedback 的）；is_notify_configured 供 UI 判断
│   │   ├── schedule.rs          # 【定时通知·后端】后台原生线程每 30s tick，到点拉当前活跃账号启用应用的评论→按日期+星级筛选→与已通知集合 diff→组装 HTML→send_telegram_message。跑在原生线程+Rust 网络栈，不受 webview 前后台/节流影响（只要进程没被 Cmd+Q 杀）。save_schedule_runtime（前端镜像配置入 ~/.tester-app/schedule/<账号>/runtime.json）/ run_schedule_now（UI「立即执行一次」）。日期预设 port 自 batchReplyDates；per-account fired/notified/baseline 文件；快照 key `{账号}__{包名}` 与前端兼容，ReviewPage 可读。**巡检完 emit `scheduled-fetch-done`（带 account key）通知前端重读快照**（否则常驻挂载的 ReviewPage 看不到后台刚拉的评论，见 gotchas.md）
│   │   ├── reviews.rs           # Google Play Developer API：评论拉取（list_play_reviews；纯逻辑抽成 pub fetch_reviews(pkg,pages,lang,token) 供 schedule.rs 复用）/ 应用列表（list_play_apps）/ 评论回复（reply_to_review）
│   │   ├── reply.rs             # 批量回复生成：写 pending JSON → 跑 claude /review-reply（--add-dir 模板目录 + prompt 传路径）→ 读回 candidates.json（reply-log 事件流）
│   │   ├── skill_sync.rs        # Skill 热更新：从 GitHub 拉取 zipball 同步到 ~/.claude/skills/，启动时静默 + Settings 手动
│   │   ├── templates.rs         # 模板管理：~/.tester-app/templates/ 的增删改 + xlsx 导入/导出（export_templates_xlsx：A 类别/B 英文/C 起各语言）；模板含 lang（en/zh-CN 双源）+ translations（预存各语言译文）；写全文同时重建瘦身 index；skill 从此目录读
│   │   ├── knowledge_base.rs    # 用例知识库：产品/资料 CRUD + 多对多关联 + Generate 消费（kb_resolve_doc_paths）；存 ~/.tester-app/knowledge/index.json + docs/<docId>.md。v2：kb_save_temp_image(粘贴截图落盘) + kb_ai_distill(--print 读对比图提炼偏好，参考 reply.rs)
│   │   ├── analysis.rs          # 评论分析：①「知识配置」每产品一份知识块 .md 的 list/read/write（存 ~/.tester-app/review-analysis/{slug}.md，文件名复用 templates 的 product_prefix）；②「🔍 分析」generate_analysis/stop_analysis（独立 AnalysisState + analysis-log 通道）：按 package→产品读知识块注入 {app_knowledge}，跑 claude --print 直出 JSON 对象（分类/问题/信息缺口/总体判断 + 一条推荐回复），与 reply.rs 同构但解析对象、状态独立
│   │   ├── model_config.rs      # 模型配置持久化（~/.tester-app/model-config.json）：get_model_config / save_model_config；字段 reply/analysis/translate（Claude 模型 ID）+ github_token（供 skill_sync 访问私有 release）+ cli_engine（"claude"|"codex"）+ codex_model（engine=codex 时传给 codex exec 的模型）
│   │   ├── prompt_config.rs     # 提示词模板持久化（~/.tester-app/prompt-config.json）：get/save_prompt_config + get_default_prompt_config（供「恢复默认」）+ render()；字段 gen/analysis/mail 各存完整 prompt 模板（含 {product}/{star} 等占位符），用户可整段编辑。reply.rs/analysis.rs 在拼 prompt 时 load() 取模板 + render 替换占位符
│   │   ├── translate.rs         # 模板多语言预翻译：轻量直出（claude --print，不走 skill、不读写文件）+ haiku，逐批翻译写回 translations；translate-log/progress 进度 + stop_translate 可取消
│   │   └── chrome.rs            # 按指定 Chrome profile 打开 URL：读 Local State 列 profile（目录名↔显示名）+ open_url_in_chrome_profile（Gmail 多账号跨 profile 打开邮件深链）
│   ├── scripts/                 # 内嵌资源（编译期 include_str!）
│   │   └── diff_testcases.py    # 来自 testcase-eval-visual-report skill 的纯 Python diff 脚本
│   ├── capabilities/            # Tauri 权限能力
│   │   └── default.json         # 默认权限：core:default、opener:default、dialog:allow-open、dialog:allow-save
│   ├── credentials/             # OAuth 凭据（已 gitignore）
│   │   └── oauth.json           # Google OAuth client_id / client_secret
│   └── icons/                   # 应用图标（多平台多尺寸）
│       ├── icon.png             # 通用 PNG
│       ├── icon.ico             # Windows ICO
│       ├── icon.icns            # macOS ICNS
│       ├── 32x32.png            # 32px
│       ├── 128x128.png          # 128px
│       ├── 128x128@2x.png      # 128px Retina
│       └── Square*.png          # Windows Store 各尺寸
└── docs/                        # 项目文档
    ├── PROJECT_STRUCTURE.md     # 本文件
    ├── decisions.md             # 架构决策记录（非显然选择的原因）
    ├── gotchas.md               # 踩过的坑（平台怪癖、外部约束、易复发 bug）
    ├── USER_GUIDE.md            # 使用说明（各功能详细指南）
    └── gmail-sync.gs            # Apps Script 脚本：定时把 Gmail 标签下未读同步到 Google Sheet（部署到各 Gmail 账号）
```

# 运行时数据目录

所有持久化数据存储在用户主目录下的 `~/.tester-app/`：

```
~/.tester-app/
├── accounts/                    # 多账号：每个已登录 Google 账号一个子目录（auth.rs）
│   └── <account-key>/           # key=Google sub（OpenID 唯一 ID）优先，迁移的旧账号回退 email
│       ├── auth-tokens.json     # 该账号 OAuth access_token / refresh_token / 过期时间
│       └── auth-user.json       # 该账号用户信息（sub、email、name、picture、id）；id=account_key，返回前端时填充供账号隔离，落盘带此字段但读回时按 account_key 重算
├── active-account.json          # 当前活跃账号指针：{ "active": "<account-key>" }
│                                # 旧版单账号的 ~/.tester-app/auth-tokens.json+auth-user.json 首启时自动迁入 accounts/ 并删除
├── telegram.json                # （可选）反馈上传配置：{ bot_token, chat_id }；compile-time env var 未设时的运行时兜底
├── notify.json                  # （可选）定时通知配置：{ chat_id, bot_token }；bot_token 留空则复用 telegram.json 的（notify.rs::resolve_bot_token 走 feedback.rs）
├── model-config.json            # 模型配置：reply/analysis/translate 模型 ID + github_token + cli_engine/codex_model（model_config.rs）
├── prompt-config.json           # 提示词模板：gen/analysis/mail/kb_distill 四个完整 prompt 模板（含占位符），Prompt 配置页编辑（prompt_config.rs）
├── exports/                     # 导出文件
│   ├── sheet_*.csv              # Google Sheets CSV 导出
│   ├── *.pdf                    # Google Slides PDF 导出（中间产物，extract_prd.py 从此读取）
│   ├── {name}_images/           # extract_prd.py 按选定页提取的 PNG 图片（传给 Claude）
│   ├── compare_{ai|human}_*.html # 对比页用：单 Tab 的 Sheet HTML 导出（waffle 格式）
│   └── diff_report_*.html       # 对比页用：diff_testcases.py 生成的报告
├── manifests/                   # 生成-上传 manifest：每个上传到 Drive 的结果对应一份
│   └── {drive_id}.json          # 字段：drive_id, web_url, source_csv_path, pptx_paths, slide_pages, model, skill_version
├── reviews/                     # 批量回复中转：每次 AI 生成的输入/输出 JSON（供调试回看）
│   ├── pending-reviews-{ts}.json            # 写给 review-reply skill 的输入（target_language + channel + groups[]）
│   └── pending-reviews-{ts}.candidates.json # skill 产出的候选回复，前端读回回填
├── reviews-cache/               # 评论快照（ReviewPage 持久化）：每个 app 一份（全量拉取列表 + fetchedAt），单 app 拉取与批量拉取写同一格式；批量视图重进时由多份 per-app 快照按当前 Config 派生拼装。免每次重拉、留住 7 天外评论。**按账号隔离**：文件名前端拼成 `{账号id}__{包名}`（账号 id 含 `@`/`.` 会被后端 sanitize 成 `_`），后端不感知账号
│   └── {accountId}__{packageName}.json # { version, reviews:[全量 list], fetchedAt }；回复成功后按 review_id 读-改-写同步
├── review-analysis/             # 评论分析「知识配置」：每个产品一份应用知识块 .md（{slug}.md，如 xfolder.md）；知识配置页维护，分析时按评论来源 app 注入提示词
├── schedule/                    # 定时通知（schedule.rs）：每个账号一个子目录（sanitize 后的 account-key）
│   └── <account-key>/
│       ├── runtime.json         # 前端镜像的运行时配置：{ schedule:{enabled,times,notifyOnEmpty,maxItemsInMsg}, apps:[{packageName,displayName,datePreset,customFromDate,customToDate,stars}] }
│       ├── fired.json           # 今日已触发时间点：{ date:"YYYY-MM-DD", times:["10:00"] }（跨天自动重置）
│       ├── notified.json        # 每个 app 的已通知集合：{ "<包名>": { baselineDone, ids:[review_id...] } }；首次 baseline 静默、按 API 窗口裁剪
│       └── updated.json         # （checkUpdated 开启时）「回复后又更新」已提醒记录：{ "<包名>": { "<review_id>": 提醒时的 user_comment_ts } }；同条只在 ts 变大时再次提醒，不做 baseline
├── templates/                   # 回复模板（模板管理页维护）；review-reply skill 运行时从这里读
│   ├── templates.json           # 全文权威源 {version, products:{产品:{templates:[{id,category,text,lang}]}}}（lang=en/zh-CN 源语言，缺省 en）
│   ├── index.json               # 瘦身索引（id+category），写 templates 时由后端自动重建；skill 匹配只读它
│   └── package_map.json         # packageName→产品（从 skill data 拷的种子；第一期 app 只读不编辑）
├── feedback_pending/            # 反馈 zip 落盘暂存；上传成功移走，上传失败留存等待重试
├── feedback_sent/               # 上传成功的反馈 zip 归档（本地备份，方便回看自己提过什么）
├── skill_backups/               # skill 热更新前的旧版备份；目录名 {skill_name}_{old_version}_{ts}
├── scripts/                     # 内嵌脚本运行时落地位置
│   └── diff_testcases.py        # 每次启动对比时由 Rust 覆盖写入
└── thumbs/                      # 幻灯片缩略图缓存（按 objectId 索引，配合 revisionId 失效）
    └── {presentation_id}/
        ├── .revision            # 上次缓存对应的 presentation revisionId；不一致即整目录失效
        ├── {objectId_a}.png     # key 是 slide 的稳定 objectId，对页面重排/插入/删除天然鲁棒
        ├── {objectId_b}.png
        └── ...
```

# 核心模块说明

## 前端页面

| 页面 | 文件 | 职责 |
|---|---|---|
| 登录页 | `LoginPage.vue` | Google 账号登录，调用后端 `start_login()` 启动 OAuth 流程 |
| 主布局 | `MainPage.vue` | 三级导航容器（工作区栏 → 选项栏 → 内容区），管理页面间数据传递 |
| Sheets 页 | `SheetsPage.vue` | 双栏布局：左侧文件列表 + URL 粘贴 + 上传 xlsx（自动转 Google 表格并刷新列表），右侧数据预览与 Tab 切换 |
| Slides 页 | `SlidesPage.vue` | 三栏布局：文件列表 + 大图预览 + 缩略图条，支持多页勾选 |
| 生成页 | `GeneratePage.vue` | 展示选择摘要 → 一键导出（CSV + 调用 `export_slides_pdf` 将选定 Slides 页提取为 PNG 图片）并调用 Claude → 终端风格日志 → 多轮对话输入 |
| 对比页 | `ComparePage.vue` | 双栏选择（AI 原始 vs 人工修改）→ 导出 HTML → 调 Claude skill 生成 diff → "在 Chrome 中打开"按钮 |
| 评论页 | `ReviewPage.vue` | Play Console 评论（原单应用页保留）。两种拉取：①「拉取评论」调 `list_play_reviews` 拉选中单个 app，按页面星级/回复状态/日期本地筛选；②「📦 批量拉取」从 `play-console-multi-config-v1` 读启用 app → 并行 `list_play_reviews` → 各 app 按其 Config 配置（星级/状态/日期）筛选后合并成**一个扁平列表**（按时间倒序，每条带应用标签 `_app`/`_pkg`，批量模式下页面筛选不适用）。每张评论卡片带「🔍 分析」+「🤖 AI 回复」+「📋 模板回复」+「添加模板」，提交/收录用该条评论来源 app 的包名（`_pkg`）；「🔍 分析」点开即弹窗并自动分析（注入该 app 知识块），输出分类/问题/信息缺口/判断 + 一条可微调直发的推荐回复，多任务可缩小（悬浮条堆左下角，与 AI 回复右下角分列）、生成串行；「在网页打开」深链仅对当前选中 app 有效，其它退回应用列表页 | → 弹窗输入「回复方向」(一句指令) + 选回复语言(默认跟随评论语言) → 调 `generate_single_reply` 现场生成 **3 条不同风格**候选 → 点选/手动微调(实时 350 字符计数) → confirm 后调 `reply_to_review` 提交并本地回填为「已回复」。**与批量回复不同**：这里是 freeform 现生成(走 `claude --print` 直出 JSON 数组)，不走模板匹配。另有「📋 模板回复」按钮 → 弹窗按「通用 / 该 app 专用」两组列出**收藏的模板**（按钮=模板 category 名）→ 点一条按评论语言（`reviewer_language` 归一成 app 码）匹配预存译文自动填入 → 可微调（350 计数）→ 提交（`reply_to_review`）；无对应语言译文则回退英文源文并提示。每条候选可点「➕ 添加模板」把好回复收录进该应用对应产品的模板库（任意语言都可收：英文存英文模板，其它语言用候选中文预览 text_zh 存中文模板；收录时内联填类别 → `product_for_package` 解析产品 → `add_template` 带 lang） |
| 配置页（容器） | `ConfigPage.vue` | 纯配置页：顶部 Tab 切换「Play Console 拉取配置」/「Batch Reply 配置」/「⏰ 定时通知」，分别嵌 `PlayConsoleConfigPage` / `BatchReplyConfigPage` / `ScheduleConfigPage`（v-show 常驻挂载，各管自己的 localStorage）。挂在 Review 工作区的 `review-config` 入口 |
| Play Console · 配置 | `PlayConsoleConfigPage.vue` | 多 app 卡片：勾选启用 + 日期预设 + 星级 + **回复状态**（全部/无回复/已回复/回复后又更新；选「回复后又更新」时星级置灰忽略）；非自定义预设显示「实际范围」预览（每分钟刷新）；顶部全选/全不选/刷新/清空配置/保存工具栏；写 localStorage `play-console-multi-config-v1`，应用列表缓存与 Batch 共用 `batch-reply-apps-cache-v1`；两者均经 scopedKey 按账号隔离（apps-cache 三页共用常量，引用处同步 scopedKey 化）。ReviewPage 读它作每个 app 的拉取/筛选默认值 |
| 批量回复 · 配置 | `BatchReplyConfigPage.vue` | 多 app 卡片：每张卡片独立**日期预设**（自上一个工作日 / 昨天 / 今天 / 近 7 天 / 自定义）+ 星级；非自定义预设下显示「实际范围」预览（每分钟刷新一次，跨午夜自动重算）；顶部「全选/全不选/刷新/清空配置/保存」工具栏（清空配置 = 删除所有版本 localStorage key 并恢复出厂默认，带 confirm）；显式保存到 localStorage `batch-reply-multi-config-v3`（读当前 v3 保留用户显式预设选择；不再读 v1/v2 老格式做无账号区分的全局兜底——见下方「账号隔离」）；应用列表缓存在 `batch-reply-apps-cache-v1` 让冷启动秒回。**账号隔离**：apps-cache / `batch-reply-multi-config-v3` 均已 scopedKey 化（三处引用同步）。`LEGACY_KEYS`（v1/v2）常量只留给「清空配置」顺手清残留，onMounted 不再拿它们做兜底读——早期版本这里照搬过 review/play 那套「复制全局配置给当前 active 账号」的迁移，因为 batch 配置升级前是真正多账号混用的一份、无法归属，结果给从没配过 batch 的账号塞了别人的配置（脏数据教训），现在改为账号迁移只删旧 key、不复制，各账号从空开始配（见 `accountStorageMigration.ts::migrateLegacyStorageOnceV2`） |
| 定时通知 · 配置 | `ScheduleConfigPage.vue` | 开关 + 时间点（`HH:MM`，可加多个）+「无新增也发心跳」开关 +「复查提醒」开关（checkUpdated：顺带扫「回复后又被用户更新」的评论，有则在通知里附一段「待复查」；无新增也会因待复查而发）+ 消息条数；Telegram 通知目标单独一块（chat_id + 可选 bot_token，留空复用 feedback 的 bot）+「立即测试发送」（发一条固定测试消息）+「立即执行一次」（`run_schedule_now`：立刻真实巡检一次，尊重去重）按钮；保存到 localStorage `review-schedule-v1`（scopedKey 隔离）+ 后端 `notify.json`（`save_notify_config`）+ 保存后 `syncScheduleRuntimeToBackend()` 镜像给定时线程 |
| 定时通知 · 驱动（后端） | `schedule.rs` + `App.vue`/`scheduleRuntimeSync.ts` | **真正的定时跑在后端原生线程**：每 30s tick，比对本地 `HH:MM` 是否有「今天已过且未触发」的配置时间点，命中则拉取+diff+发消息，`(日期,时间点)` 记 `~/.tester-app/schedule/<账号>/fired.json` 防重复；多个错过时间点合并成一条（标「错过补发」）。前端只在保存/启动/切账号时把配置镜像成 `runtime.json`。**优点**：窗口最小化/后台/被遮挡都准点（不受 webview 节流影响），只要进程没被 Cmd+Q 杀；睡眠/退出期间不跑，恢复后下一 tick 补发。**巡检完 emit `scheduled-fetch-done`**：ReviewPage 收到后自动重读快照（优先批量视图），把后台拉到的评论直接显示，无需手动拉取 |
| 定时通知 · 拉取+diff（后端） | `schedule.rs::execute_and_notify` | 取当前活跃账号 token（失效发 Telegram 警告不静默吞）→ 逐个启用 app 调 `fetch_reviews` → 落 per-app 快照（key `{账号}__{包名}`，与 ReviewPage 格式一致互通）→ 按日期预设(port 自 batchReplyDates)+星级筛选 → 与 `notified.json` 里该 app 的已通知集合 diff 出新增。**首次某 app**（baselineDone 未设）静默把命中项全标已通知、不算新增（避免把近 7 天存量当新增刷屏）；已通知集合每次按当前 API 窗口裁剪防无限增长 |
| 批量回复 · 执行 | `BatchReplyPage.vue` | **挂在 MainPage 时带 `:key="acct-batch-${accountEpoch}"`，切账号会整页重挂**（否则 v-show 常驻不重跑 onMounted，会显示上一个账号的候选）。进入时从 localStorage 读多 app 配置（scopedKey，同上）→ 「拉取候选评论」**在拉取时**调 `computeRange()` 把预设解析为绝对日期（所以配置一次永远新鲜）→ 并行调每个启用 app 的 `list_play_reviews`（不互相阻塞）→ 按 app 分组折叠展示（默认展开，组头显示 app 名 / 包名 / 当次实际日期范围 + 预设名 / 候选数）→ **「🔎 匹配模板并填充」调 `run_reply_skill`**（顶部下拉选回复语言默认 `auto`，模型固定 Sonnet）→ 命中模板的评论预填翻译好的模板正文（+中文预览）；未匹配的标「未匹配·需手动处理」留空手填 → 逐条提交直接发，「一键提交全部」弹 confirm 后跨 app 串行 + 200ms 间隔调 `reply_to_review`。每条评论可点「✋ 人工处理」手动标记（按 review_id 持久化到 localStorage `batch-reply-manual-ids-v1`，**已 scopedKey 化**，跨拉取/重启/账号切换各自独立保留）——标记后**只**从「匹配模板并填充」里排除，仍可手填 / AI 单条回复 / 逐条或一键提交。每张候选卡片另有「🤖 AI 回复」按钮 → 弹单条生成弹窗（回复方向**可留空**，留空时后端让模型据评论自行判断方向〔含「无法更新」等常见问题的标准排查引导〕；+ 语言 → `generate_single_reply` 出 3 条风格各异候选）→「选用并填入」把文案灌进该卡片回复框（标记手动、清掉未匹配标），再走原有逐条/一键提交。**单条弹窗是多任务的**：可同时开多条（缩小成右下角竖直堆叠的悬浮条），生成走前端队列排队（后端 `ReplyState` 一次只跑一个），`reply-log` 路由到当前正在生成的那个任务、不污染批量匹配日志。每条候选也有「➕ 添加模板」收录入口（任意语言：英文存英文、其它用中文预览存中文、填类别 → `product_for_package` + `add_template` 带 lang）。与「匹配模板」是两条独立路径：模板匹配=批量命中翻译，AI 回复=单条 freeform 现生成 |
| 模板管理 | `TemplateManagerPage.vue` | 挂在 Review 工作区。每条模板行首有 ★收藏开关（写 `tpl-fav-ids-v1`，供评论页「模板回复」弹窗读）。按产品 tab 切换（显示条数 + 关联 app），列出该产品模板，每条可改 category/正文 → `update_template`，删除走**内联两步确认**；顶部「+ 新增模板」→ `add_template`（后端按产品前缀自动生成 id）；「📥 从 xlsx 导入」用系统文件选择器（`@tauri-apps/plugin-dialog`）选 xlsx → 内联确认（覆盖该产品）→ `import_templates_xlsx`。**产品 tab 管理**：hover 右上角出现 × 按钮，鼠标停留二步确认后 `delete_template_product` 删除产品及其所有模板；「管理关联」按钮弹窗编辑 `package_map.json`（`get_package_map` / `save_package_map`），可增删改包名↔产品映射（包名/显示名/关联产品下拉选）。所有写操作落 `~/.tester-app/templates/`，skill 直接读 |
| Gmail 页 | `GmailPage.vue` | 读 Apps Script 同步出的 Google Sheet（手动粘贴表链接，存 localStorage `gmail-sources-v1`，每张表可配 Chrome profile + 关联邮件模板产品）：复用 `read_sheet`/`get_sheet_tabs` 读 `Mail` tab、按表头名取列；列表每封固定 3 行（发件人+日期 / 主题 / 机翻中文），「详情」弹大卡（机翻上 / 原文下），「↗」按配的 Chrome profile 打开邮件深链（`open_url_in_chrome_profile`），「已读」本地隐藏（localStorage `gmail-read-ids-v1`，「↩ 撤销上一封」LIFO）。绕开 Gmail OAuth Testing 7 天过期，全程见 `gmail-handoff.md` |
| App Script 页 | `AppScriptPage.vue` | 维护待管理的 Apps Script 项目列表（每项备注/Chrome profile/跳转链接，存 localStorage `appscript-projects-v1`）；点「打开 ↗」用指定 Chrome profile（或系统默认浏览器）打开链接（默认 Apps Script 首页），复用 `list_chrome_profiles` / `open_url_in_chrome_profile` |
| 设置页 | `SettingsPage.vue` | 挂在 Settings 二级导航 `settings-general`。缓存管理（查看大小/清理）+ 模型配置（回复/分析/翻译各自选 Sonnet 或 Haiku，调 `get_model_config` / `save_model_config` 持久化）+ GitHub Token 输入（供 skill 热更新拉私有 release）+ 版本检测更新 |
| Prompt 配置页 | `PromptConfigPage.vue` | 挂在 Settings 二级导航 `settings-prompt`（独立页）。gen/analysis/mail/kb_distill 四个**完整 prompt 模板** textarea，含 `{占位符}` 可整段编辑，每块列出可用占位符 + 「已修改」标记 + 「恢复默认」按钮；调 `get_prompt_config` / `get_default_prompt_config` / `save_prompt_config`。改坏占位符/JSON 会致解析失败，靠恢复默认兜底 |

## 后端模块

| 模块 | 文件 | 职责 |
|---|---|---|
| 认证 | `auth.rs` | Google OAuth 2.0 PKCE 流程：本地 TCP 回调服务器、令牌交换、自动刷新。**多账号**：`AuthState` 持 `accounts: HashMap<key, Account>` + `active` 指针（key=Google `sub` 优先、回退 email）；各账号独立存盘于 `accounts/<key>/`，`active-account.json` 记指针。命令：`check_auth`（返回 active 用户）/ `start_login`（追加账号并设 active，按 email 去重旧条目）/ `logout(account_id?)`（登出指定/当前账号，返回新 active 用户）/ `list_accounts`（列全部账号带 active 标记）/ `switch_account(account_id)`（仅换内存指针+落盘，不重走 OAuth）。`get_valid_access_token` 取 active 账号 token，签名不变 → 业务模块无感。首启自动迁移旧单账号明文文件。**账号隔离维度**：`UserInfo` 带 `id` 字段（= account_key，sub 优先回退 email），在 account 进入 AuthState 时（load_accounts_from_disk / start_login）统一填充 → check_auth/switch/logout 返回的 user 自带 id，前端把它当 opaque 标识做本地存储隔离，前端不自算（换 provider 只改此处）。**账号驱逐通知**：`get_valid_access_token` 刷新失败遇 `invalid_grant`（refresh_token 失效）会把该账号静默移出 `accounts`、`active` 切到下一个——这个副作用发生在业务调用内部、原调用方感知不到，故额外通过进程级 `AppHandle`（`init_app_handle` 在 `lib.rs::run()` 的 `.setup()` 钩子里存入 `OnceLock`，因为大多数触发它的 command 不带 `AppHandle` 参数）`emit("account-evicted", { evicted_email, next: Option<UserInfo> })`；`MainPage.vue` 监听该事件同步 `user` + 重挂账号世界页面（`next` 为空则登出回登录页），避免前端继续显示已被踢掉的账号 |
| Google API | `sheets.rs` | Drive 文件列表、Sheets 读取与 CSV 导出、xlsx 上传（路径或字节、自动转 Google 表格、归入 `tester-app` 文件夹）、Slides 幻灯片获取与 PDF 导出（`export_slides_pdf(presentationId, name, pages)` 接收页码列表，下载整份 PDF 后调用 `extract_prd.py` 提取选定页为 PNG，返回 `Vec<String>` PNG 路径列表）、缩略图异步缓存 |
| Claude 集成 | `claude.rs` | 定位 Claude CLI 路径、子进程管理、stream-json 输出解析、会话 ID 续接、实时事件推送 |
| 对比流程 | `compare.rs` | 单 Tab Sheet 导出 HTML（`docs.google.com/.../export?format=html&gid=`）、内嵌脚本写盘后直接执行 `python diff_testcases.py`、在 Chrome 打开报告（`compare-log` 事件流，独立于 `claude-log`） |
| 单条 AI 回复 | `reply.rs` | `generate_single_reply` command：给**一条**评论 + 用户一句「回复方向」+ 语言，在 Rust 里把评论上下文 + 方向 + review-reply skill 的硬性标准(≤350 字符/回复语言/不编造/保留 emoji 专名/引号规范) 拼成 prompt → 跑 `claude --print --output-format stream-json --permission-mode bypassPermissions`(无 skill、无文件往返) → 从终结 `result` 事件取最终文本 → `extract_json_array` 容错抠出 JSON 数组(防 markdown 代码块/前后散文) → 解析成 **3 条风格各异**的候选 `{style,language,text,text_zh,char_count}` 返回。**与「🔍 分析」一致：按 product（退回 package_name 解析）读该产品知识块注入 `{app_knowledge}`**；硬性标准段落来自 `prompt_config::load().gen_rules`（可在设置页编辑）。复用 `ReplyState`(与批量共用 running 锁 + `stop_reply` 可中断) 和 `reply-log` 事件流 |
| 批量回复生成 | `reply.rs` | `run_reply_skill` command：把前端传来的 `groups[]` 包成 `{target_language, channel:"gp", groups}` 写到 `~/.tester-app/reviews/pending-reviews-{ts}.json` → 跑 `claude --print --output-format stream-json --permission-mode bypassPermissions --add-dir <reviews> --add-dir <模板目录> --model claude-sonnet-4-6 /review-reply <json>` → 等子进程结束 → 读回同 stem 的 `*.candidates.json` 解析。**模板目录**=`~/.tester-app/templates/`，路径既用 `--add-dir` 授权、也写进 prompt（「模板数据目录：…」）显式告诉 skill 去哪读 index/templates/package_map。返回 `{output, usage}`（usage 来自 stream-json 终结 `result` 事件的 token/费用）。独立 `ReplyState`（running + child_pid）+ `reply-log` 事件流。`stop_reply` command 杀进程树并置 running=false → inner 检测到返回 `CANCELLED`。复用 claude.rs 的 `find_claude` / `load_claude_token`。skill 流程为「匹配 only」（命中→翻译模板1条；未命中→matched:false）。**注意**：skill 是 LLM 手写 candidates.json，含未转义引号会非法 → SKILL.md 已要求写完自检 JSON；后端严格 `serde_json` 解析，非法报错。`stop_reply` 可中断 |
| 评论拉取 | `reviews.rs` | 三条命令：① `list_play_apps` 调 `playdeveloperreporting.googleapis.com/v1beta1/apps:search` 列出账号下所有应用（包名 + 显示名）；② `list_play_reviews` 调 `androidpublisher.googleapis.com/v3/applications/{packageName}/reviews`（支持 `translationLanguage` 参数让 Google 直接返回译文，原文落 `originalText`），扁平化 comments 数组（取最新 userComment + developerComment）；③ `reply_to_review` 调 `…/reviews/{reviewId}:reply` POST `{replyText}`（配额：2000/天/应用）。服务端不支持任何筛选 → 一次性返回全部 7 天评论，由前端按星级/回复/日期本地过滤；遇 401 或 `ACCESS_TOKEN_SCOPE_INSUFFICIENT` 返回 `NEED_RELOGIN_SCOPE:` 前缀，前端提示重登。④ `save_reviews_snapshot(key, data)` / `load_reviews_snapshot(key)`：评论快照透明读写 `~/.tester-app/reviews-cache/{key}.json`（非法字符转 `_`；写走临时文件+rename 防损坏）；payload 是 `serde_json::Value` 原样落盘/读回，**后端不认识 Review 结构**，前端 Review 字段改动不波及 Rust。**账号隔离在前端完成**：前端传入的 key 已是 `{账号id}__{包名}`，后端不感知账号（签名不变） |
| 生成-上传 manifest | `manifest.rs` | `write_generate_manifest` command：把"生成的 xlsx 上传到 Drive 后的 drive_id"和"用来生成它的源文件（CSV + PPTX + 页码）"绑定写盘；compare 页反馈时按 ai_drive_id 反查 |
| 反馈上传 | `feedback.rs` | `send_feedback` command：反查 manifest → 打包 zip（ai.html + human.html + report.html + 源文件 + meta.json）→ Telegram sendDocument multipart 上传 → 成功移到 `feedback_sent/`，失败留 `feedback_pending/`；`retry_pending_feedback` 重试；`is_feedback_configured` 探测是否配置好 token；`resolve_bot_token()` 单独导出 bot token（不要求 chat_id），供 `notify.rs` 复用 |
| 定时通知（后端） | `notify.rs` | `send_telegram_message(text)`：POST `sendMessage`（`parse_mode=HTML`），校验 `{"ok":true}`；`get_notify_config`/`save_notify_config` 读写 `~/.tester-app/notify.json`（`{ chat_id, bot_token }`，bot_token 空则调 `feedback::resolve_bot_token()`）；`is_notify_configured` 探测 |
| Skill 热更新 | `skill_sync.rs` | 内置 skill 列表（owner/repo）；用 GitHub Releases API（`/releases/latest`）拿 tag_name 做版本判断；`check_skill_updates` 比对本地 `.tester-app-version` 与远程 tag；`sync_all_skills` / `sync_skill` 下载 release zipball → 备份旧版 → 解压覆盖到 `~/.claude/skills/{name}/` → 写新版本；`get_skill_local_version` 给前端取版本号写进反馈 manifest |
| 模板管理 | `templates.rs` | 10 个 command（无 State，纯文件读写）：`list_template_products`（产品+条数+关联 app）/ `create_template_product`（建空产品立即落盘）/ `delete_template_product`（删产品及其所有模板）/ `product_for_package`（包名→产品，null=无模板产品）/ `list_templates` / `add_template`（按产品前缀自动生成 id）/ `update_template` / `delete_template` / `import_templates_xlsx` / `export_templates_xlsx`。**包名关联**：`get_package_map` 读 `package_map.json` 返回 `[{package, display, product}]`；`save_package_map` 接收同结构数组整体覆写 mapping 字段（保留 `_comment` 等顶层字段）。模板含 `lang`（en/zh-CN）**中英双源**；**唯一写出口** `write_templates_and_index` 写 templates.json 同时重建 index.json。`ensure_templates_dir` 首次从 skill data 拷种子。`reply.rs` 复用 `templates_dir()` / `ensure_templates_dir()` |
| 模板多语言预翻译 | `translate.rs` | `translate_templates(product, ids, langs, overwrite, channel, model)` + `stop_translate`：每条按（覆盖/只补缺失 + 排除同源码）算 target_langs，CHUNK=1 逐条 spawn `claude --print`（**不 --add-dir、prompt 只内联本批、禁工具/不读写文件、stdout 解析**，避免 agent 读全量 templates.json 烧额度），每批 `apply_translations` 增量写回 + 刷 src_hash；350 字符硬校验（超长压缩重试一次仍超则标红警告）；`translate-log`（含用量）+ `translate-progress`（进度条）事件；复刻 reply.rs 的取消逻辑。模型默认 haiku。详见 `handoff-template-i18n.md` |
| 模型配置 | `model_config.rs` | `get_model_config` / `save_model_config`：读写 `~/.tester-app/model-config.json`，字段 reply/analysis/translate（Claude 模型 ID，默认 Sonnet 4.6/Sonnet 4.6/Haiku 4.5）+ github_token；`load()` 供 skill_sync.rs 内部调用拿 GitHub Token |
| 提示词配置 | `prompt_config.rs` | `get_prompt_config` / `save_prompt_config` / `get_default_prompt_config`（恢复默认用）：读写 `~/.tester-app/prompt-config.json`，3 个字段 `gen`/`analysis`/`mail` 各存对应 AI 功能的**完整 prompt 模板**（含 `{product}`/`{star}` 等占位符）。`render(template, vars)` 把 `{token}` 替换成真实值（JSON 示例的单括号不会被误替换）；`reply.rs::build_gen_prompt`、`reply.rs::build_mail_reply_prompt`、`analysis.rs::build_analysis_prompt` 在拼 prompt 时 `load()` 取模板 + render。默认值逐字等于原写死文本，未编辑则行为不变。**用户可整段改**（含占位符/JSON），改坏靠设置页「恢复默认」兜底。翻译类 prompt 仍写死在 translate.rs，不开放 |
| 评论分析 | `analysis.rs` | ①「知识配置」`list_knowledge` / `read_knowledge` / `write_knowledge`（无 State，每产品一份 `~/.tester-app/review-analysis/{slug}.md`，文件名复用 `templates::product_prefix`）；②「🔍 单条分析」`generate_analysis(review, product, package_name, language, model)` + `stop_analysis`：按 `package_name` 调 `product_for_package` 解析产品 → 读该产品知识块注入 `{app_knowledge}` → 跑 `claude --print`（无 skill、无文件往返）→ `extract_json_object` 抠出 JSON 对象解析成 `{category, issues[], info_gaps[], analysis, reply{language,text,text_zh,char_count}}` + usage。**独立 `AnalysisState`（running + child_pid）+ `analysis-log` 事件通道**，与 reply.rs 的 `ReplyState`/`reply-log` 互不污染（同 decisions.md 状态隔离原则）；与 `generate_single_reply` 同构，差异：解析对象（非数组）、注入知识块 |
| Chrome 打开 | `chrome.rs` | `list_chrome_profiles` 读 Chrome `Local State` 的 `profile.info_cache` 列出 profile（目录名 ↔ 显示名，用户按显示名选、存目录名）；`open_url_in_chrome_profile(url, profile_dir)` 用 `--profile-directory` 指定 profile 打开 URL（macOS 直接调 Chrome 二进制并 `open -a` 激活前台，Win/Linux 各自分支）。解决 Gmail 多账号分散在不同 Chrome profile 时，邮件深链跨 profile 跳不过去 |
| 应用内更新 | `updater.rs` | `check_update`（查 GitHub Releases latest 比对当前版本，带 GitHub Token 避免限流）/ `download_update`（下载安装包，`update-progress` 事件报进度）/ `apply_update`（重启安装）。设置页「版本」区块调用 |

# 依赖库

## 前端依赖（npm）

| 库 | 类型 | 作用 |
|---|---|---|
| `vue` | 核心 | Vue 3 响应式 UI 框架 |
| `@tauri-apps/api` | 核心 | Tauri 前后端通信桥接（invoke 命令、事件监听） |
| `@tauri-apps/plugin-opener` | 插件 | 在系统浏览器中打开 URL |
| `@tauri-apps/plugin-dialog` | 插件 | 系统文件选择器（模板管理页选 xlsx 导入） |
| `marked` | 核心 | Markdown 文本解析与渲染（用于更新弹窗的 release notes 显示） |
| `typescript` | 开发 | 静态类型检查 |
| `vite` | 开发 | 前端构建与开发服务器 |
| `@vitejs/plugin-vue` | 开发 | Vite 的 Vue SFC 编译插件 |
| `vue-tsc` | 开发 | Vue 模板的 TypeScript 类型检查 |

## 后端依赖（Cargo）

| 库 | 类型 | 作用 |
|---|---|---|
| `tauri` | 核心 | Tauri 2 桌面应用框架 |
| `tauri-build` | 构建 | Tauri 构建时脚本 |
| `tauri-plugin-opener` | 插件 | 打开 URL / 文件的 Tauri 插件 |
| `tauri-plugin-dialog` | 插件 | 系统原生对话框（文件选择器，模板 xlsx 导入用） |
| `calamine` | 解析 | 读 xlsx（模板批量导入，复刻原 build_templates.py 解析） |
| `tokio` | 运行时 | 异步运行时（net、io-util、process） |
| `reqwest` | 网络 | HTTP 客户端，用于 Google API 调用和缩略图下载 |
| `serde` / `serde_json` | 序列化 | JSON 序列化与反序列化 |
| `sha2` | 加密 | PKCE code_challenge 的 SHA-256 哈希 |
| `base64` | 编码 | PKCE code_verifier 的 Base64 URL 编码 |
| `rand` | 工具 | PKCE 随机字节生成 |
| `dirs` | 工具 | 跨平台用户主目录路径获取 |
| `open` | 工具 | 在默认浏览器中打开 OAuth 授权页面 |
| `urlencoding` | 工具 | URL 编码处理 |
| `zip` | 工具 | 解压 Sheets HTML 导出 zip（compare 流程） |

# 数据流

```
登录认证
  └─ LoginPage → start_login() → 浏览器 OAuth 授权 → 本地回调 → 令牌交换 → 持久化 → 进入主页

选择测试数据
  ├─ SheetsPage: 文件列表 / URL 粘贴 → 获取 Tab → 预览数据 → 确认选择
  └─ SlidesPage: 文件列表 / URL 粘贴 → 加载幻灯片 → 异步缩略图 → 勾选页面

生成测试用例
  └─ GeneratePage:
       ├─ 导出 Sheet CSV  → export_sheet_csv()  → ~/.tester-app/exports/sheet_*.csv
       ├─ 导出 Slides PDF + 提取 PNG
       │    export_slides_pdf(presentationId, name, pages)
       │      → ~/.tester-app/exports/{name}.pdf（中间产物）
       │      → extract_prd.py --input <pdf> --outdir <{name}_images> --slides <pages>
       │      → ~/.tester-app/exports/{name}_images/*.png（返回 PNG 路径列表）
       └─ 调用 Claude CLI  → run_claude_task(csv, imgPaths)
            └─ claude --print --verbose --output-format stream-json --file <csv> --file <img1> ... '/test-case-generator ...'
                 └─ 流式 JSON → claude-log 事件 → 前端终端日志

多轮对话
  └─ 用户输入 → send_claude_input(input) → claude --resume <session_id> → 流式响应

compare（ComparePage）
  └─ 用户选两个 Sheet（AI 版 + 人工版）
       ├─ export_sheet_html(ai)    → ~/.tester-app/exports/compare_ai_*.html
       ├─ export_sheet_html(human) → ~/.tester-app/exports/compare_human_*.html
       ├─ run_diff_skill(ai_path, human_path)
       │    ├─ 把内嵌脚本写到 ~/.tester-app/scripts/diff_testcases.py
       │    ├─ 检查 bs4，缺则 python -m pip install beautifulsoup4
       │    └─ python diff_testcases.py <ai> <human> -o <report_path>
       │         → stdout/stderr → compare-log 事件
       │         → 校验报告文件存在 → 返回 report_path
       └─ open_in_chrome(report_path) → 启动 Chrome 打开报告

生成 manifest 链路
  GeneratePage:
    handleGenerate() 时把 csvPath / pptxPaths（现在存 PNG 路径列表，非 PPTX）/ slidePages（空数组，页码已在导出阶段消费）/ model 存入 lastGenContext
    handleUploadToDrive() 上传 xlsx 拿到 drive_id + web_url 后，调
      write_generate_manifest({ drive_id, web_url, source_csv_path, pptx_paths（PNG 路径）, slide_pages（空）, model, skill_version })
        → ~/.tester-app/manifests/{drive_id}.json

反馈链路（ComparePage 报告生成后）
  用户点"反馈"→ 弹窗选问题类型 + 备注 → send_feedback({...})
    1. resolve_telegram_config()
         compile-time env (TELEGRAM_BOT_TOKEN/TELEGRAM_CHAT_ID) → 否则 ~/.tester-app/telegram.json
    2. read_manifest(ai_drive_id)  → 可能为 None（旧数据或粘贴 URL 选的 sheet）
    3. load_user()                 → 从 auth-user.json 拿 email + name 塞 meta.json
    4. build_feedback_zip()        → zip 含 ai.html / human.html / report.html / sources/* / meta.json
    5. upload_to_telegram()        → POST sendDocument multipart，校验 {"ok":true}
    6. 成功 → move_to_sent()       → 失败 → 留在 feedback_pending/，错误回传前端

批量回复链路（BatchReplyPage）
  拉取：handleFetch() → 并行 list_play_reviews（每 app）→ 本地按星级/未回复/日期过滤 → 候选列表
  匹配：generateReplies()（按钮「🔎 匹配模板并填充」）
    1. buildSkillGroups() 收集"未匹配/未处理且未提交"的评论，按 app 组装 groups[]
    2. invoke("run_reply_skill", { groups, targetLanguage, channel:"gp", model:"claude-sonnet-4-6" })
         → reply.rs 写 pending JSON → claude /review-reply → 读 *.candidates.json → 返回 {output:{results[],warnings[]}, usage}
         → 期间 reply-log 事件喂前端「匹配日志」面板
    3. 按 review_id 回填：命中(matched=true)→candidate.options=[模板译文]，预填 replyText；
       未命中(matched=false)→candidate.unmatched=true，留空交用户手填（不是错误）
    4. 显示本次 token 用量/费用（res.usage → 工具栏下常驻「💰 本次用量」+ notice）
  选择：命中只有 1 条（模板译文，已预填）；手动编辑 textarea → selectedIdx=-1（标"已手动编辑"）
  提交：submitOne / handleSubmitAll → reply_to_review（与原逻辑一致）

  ⚠️ 流程已是「匹配 only」（skill v0.2.0）：命中模板→翻译该模板 1 条；未命中→跳过交用户手处理。
    不再对每条评论现生成多候选。回复语言默认 "auto"（逐条跟随评论语言），模型固定 Sonnet。
    实测 6 条评论 $0.31 / 133s（旧"全生成"是 $1.01 / 552s）。
  耗时仍分钟级（agentic 读索引+匹配+翻译），UI 有已用时计时 + 「停止」按钮（invoke stop_reply）+ 流式日志。

按钮可见性
  ComparePage onMounted → is_feedback_configured()，false 时按钮隐藏

Skill 热更新链路
  App.vue onMounted（启动时，静默）
    → invoke("sync_all_skills")
       对 SKILLS 列表里的每个条目：
       1. GET https://api.github.com/repos/{owner}/{repo}/releases/latest → 拿 tag_name + zipball_url
       2. 对比 ~/.claude/skills/{name}/.tester-app-version
       3. 一致 → 跳过；不一致 →
            ├─ GET <zipball_url>
            ├─ 把 ~/.claude/skills/{name}/ 重命名到 ~/.tester-app/skill_backups/{name}_{old_version}_{ts}/
            ├─ 解压 zipball 到 ~/.claude/skills/{name}/（剥掉 zipball 自带的顶层目录）
            ├─ 写新版本到 .tester-app-version（例如 "v1.8.0"）
            └─ 失败 → 回滚备份
       结果存 console，不阻塞 UI

  Releases-based 而非 commit-based：维护者 push 普通 commit 不会触发更新，
    只有在 GitHub 上 cut 一个 release（打 tag）才会被用户拉取。给维护者明确的"发布"动作。

  SettingsPage：手动 "重新检查" / "立即更新" 按钮 + 每个 skill 当前版本 / 最新版本显示

  GeneratePage 写 manifest 时调 get_skill_local_version("test-case-generator")
    → "test-case-generator@v1.8.0" 作为 skill_version 字段，反馈样本精确定位版本
```

# 关键决策

## 单 Tab HTML 导出（compare.rs）

- **方案**：Drive API `GET /drive/v3/files/{id}/export?mimeType=application/zip` + Bearer token，下载到内存后用 `zip` crate 解压，按 Tab 名匹配单个 `.html` 写盘
- **为什么不用 `docs.google.com/spreadsheets/.../export?format=html&gid=`**：这条遗留 URL 社区流传但 Google 没文档化，可能重定向到 `htmlembed`（需 cookie 鉴权）或行为变更，不可靠
- **`mimeType=application/zip` 是官方文档化的 Sheets HTML 导出**，对应 Sheets 里的「文件 → 下载 → 网页 (.html, .zip)」，产物就是 `diff_testcases.py` 需要的 `<table class="waffle">`
- **Tab 名匹配**：zip 里的文件名是 Google 自己的清洗规则（空格/斜杠/括号等会被改写），所以匹配时把 Tab 名和文件名都做 `to_lowercase + 保留字母数字` 归一化后比较；单 Tab 表格直接取唯一文件，无需匹配

## Diff 脚本调用方式：直接跑 Python，不走 Claude CLI

- `src-tauri/scripts/diff_testcases.py` 通过 `include_str!` 在编译期嵌入二进制；运行时每次写入 `~/.tester-app/scripts/diff_testcases.py`（覆盖式，保证发版即更新）
- 用 `find_python()` 顺序探测 `py`（Windows）/`python3`/`python`，首个 `--version` 成功的即采用
- 用 `python -c "import bs4"` 探测依赖；缺失则 `python -m pip install beautifulsoup4 --quiet`，pip 输出也走 `compare-log`
- **为什么不走 Claude CLI**：避免一次 LLM 启动开销（数秒～十几秒）+ token 消耗；脚本是确定性的纯函数，没必要 LLM 介入。Skill 的"调用"本质只是"运行这个脚本"，去掉中间层

## Diff 脚本的两份拷贝 & 自动同步

- **上游源**：`C:\Users\chenj\Documents\trae_projects\diff\scripts\diff_testcases.py`（独立项目里的真源码）
- **仓内 vendored 副本**：`src-tauri/scripts/diff_testcases.py`（被 `include_str!` 编译进二进制）
- **`build.rs::sync_diff_script()`** 在每次 cargo build 前：
    1. 走 `cargo:rerun-if-changed=<upstream>` 让上游变更触发重 build
    2. 对比两边内容，不一致就 `std::fs::copy` 上游 → 仓内
    3. 上游路径可用 `DIFF_SCRIPT_SRC` 环境变量覆盖
    4. 上游不存在时静默跳过，保证 app 在别人机器上仍可独立 build
- **为什么不让 app 运行时直接读上游路径**：path 写死在二进制里 = 发版后只能在我这台机器上跑；这样既保留可移植性又避免手动同步

## CompareState 独立于 ClaudeState

- 对比任务用自己的 `compare-log` 事件 channel 和 `CompareState`（仅 `running` 标志）
- 不复用 `ClaudeState`：避免与 GeneratePage 的 session_id / running 状态相互污染


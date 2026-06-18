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
│   ├── App.vue                  # 根组件：认证状态路由（登录页 ↔ 主页）
│   ├── vite-env.d.ts            # Vite 类型声明
│   ├── assets/                  # 静态资源
│   │   └── vue.svg              # Vue logo
│   ├── utils/                   # 跨页面工具模块
│   │   ├── batchReplyDates.ts   # 批量回复的动态日期预设（"自上一个工作日"/"昨天"等 → 执行时解析为绝对范围）
│   │   ├── playConsoleConfig.ts # Play Console 多 app 拉取配置：类型 + 增 replyState 字段 + localStorage 读写规整（与 batchReplyDates 共享日期预设）
│   │   └── templateFavorites.ts # 模板「收藏」：标星模板 id 集合的 localStorage 读写（`tpl-fav-ids-v1`）；模板管理页写、评论页「模板回复」弹窗读
│   └── pages/                   # 页面组件
│       ├── LoginPage.vue        # Google OAuth 登录页
│       ├── MainPage.vue         # 主布局：三级导航（工作区 → 选项 → 内容）
│       ├── SheetsPage.vue       # Google Sheets 浏览与选择
│       ├── SlidesPage.vue       # Google Slides 浏览与多页选择
│       ├── GeneratePage.vue     # Claude 测试用例生成（导出 + 流式日志 + 多轮对话）
│       ├── ComparePage.vue      # compare：两个 Sheet 导出 HTML → 调用 diff skill → 在 Chrome 打开报告
│       ├── ReviewPage.vue       # Play Console 评论（单应用页 + 批量按钮）：原有「拉取评论」拉选中单个 app（页面星级/回复状态/日期本地筛选）；新增「📦 批量拉取」读 Play Console 拉取配置启用的 app 并行拉、各按其配置筛选后合并成一个列表（每条带应用标签）。每条评论可 AI 单条回复 + 模板回复（收藏模板快捷取用）+ 添加模板（批量模式下用该评论来源 app 的上下文提交/收录）
│       ├── ConfigPage.vue       # 纯配置页：两个 Tab 容器（Play Console 拉取配置 / Batch Reply 配置），各嵌一个子配置页（v-show 常驻）
│       ├── PlayConsoleConfigPage.vue # Play Console 拉取配置：每个 app 一张卡片，勾选启用 + 日期预设 + 星级 + 回复状态；显式「保存」写 localStorage `play-console-multi-config-v1`；ReviewPage 读它
│       ├── BatchReplyConfigPage.vue # 批量回复配置（嵌在 ConfigPage 的 Batch Tab）：每个 app 一张卡片，独立勾选启用 + 日期预设 + 星级；显式「保存」按钮写 localStorage
│       ├── BatchReplyPage.vue   # 批量回复执行：读保存的多 app 配置 → 并行拉评论 → 按 app 分组折叠 → AI 生成回复 → 逐条/一键提交
│       ├── TemplateManagerPage.vue # 模板管理：按产品 tab 增删改模板 + xlsx 导入 + 多语言预翻译 + 每条 ★收藏（挂 Review 工作区）
│       ├── GmailPage.vue        # Gmail 邮件：读 Apps Script 同步的 Sheet，卡片列表 + 详情弹窗 + 已读隐藏 + 按 Chrome profile 打开邮件
│       └── SettingsPage.vue     # 缓存管理设置
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
│   │   ├── feedback.rs          # 反馈打包：zip + Telegram sendDocument 上传 + 本地归档 / pending 重试
│   │   ├── reviews.rs           # Google Play Developer API：评论拉取（list_play_reviews）/ 应用列表（list_play_apps）/ 评论回复（reply_to_review）
│   │   ├── reply.rs             # 批量回复生成：写 pending JSON → 跑 claude /review-reply（--add-dir 模板目录 + prompt 传路径）→ 读回 candidates.json（reply-log 事件流）
│   │   ├── skill_sync.rs        # Skill 热更新：从 GitHub 拉取 zipball 同步到 ~/.claude/skills/，启动时静默 + Settings 手动
│   │   ├── templates.rs         # 模板管理：~/.tester-app/templates/ 的增删改 + xlsx 导入；模板含 lang（en/zh-CN 双源）+ translations（预存各语言译文）；写全文同时重建瘦身 index；skill 从此目录读
│   │   ├── translate.rs         # 模板多语言预翻译：轻量直出（claude --print，不走 skill、不读写文件）+ haiku，逐批翻译写回 translations；translate-log/progress 进度 + stop_translate 可取消
│   │   └── chrome.rs            # 按指定 Chrome profile 打开 URL：读 Local State 列 profile（目录名↔显示名）+ open_url_in_chrome_profile（Gmail 多账号跨 profile 打开邮件深链）
│   ├── scripts/                 # 内嵌资源（编译期 include_str!）
│   │   └── diff_testcases.py    # 来自 testcase-eval-visual-report skill 的纯 Python diff 脚本
│   ├── capabilities/            # Tauri 权限能力
│   │   └── default.json         # 默认权限：core:default、opener:default、dialog:allow-open
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
    ├── gmail-handoff.md         # Gmail 邮件接入交接文档（Apps Script 同步 + app 读表全过程 + 为何弃用直连 OAuth）
    ├── gmail-sync.gs            # Apps Script 脚本：定时把 Gmail 标签下未读同步到 Google Sheet（部署到各 Gmail 账号）
    └── handoff-template-i18n.md # 模板多语言预翻译改造交接文档（背景/设计决策/分步进度，含为何弃用 template-translate skill 改轻量直出）
```

# 运行时数据目录

所有持久化数据存储在用户主目录下的 `~/.tester-app/`：

```
~/.tester-app/
├── auth-tokens.json             # OAuth access_token / refresh_token / 过期时间
├── auth-user.json               # 用户信息（email、name、picture）
├── telegram.json                # （可选）反馈上传配置：{ bot_token, chat_id }；compile-time env var 未设时的运行时兜底
├── exports/                     # 导出文件
│   ├── sheet_*.csv              # Google Sheets CSV 导出
│   ├── *.pptx                   # Google Slides PPTX 导出
│   ├── compare_{ai|human}_*.html # 对比页用：单 Tab 的 Sheet HTML 导出（waffle 格式）
│   └── diff_report_*.html       # 对比页用：diff_testcases.py 生成的报告
├── manifests/                   # 生成-上传 manifest：每个上传到 Drive 的结果对应一份
│   └── {drive_id}.json          # 字段：drive_id, web_url, source_csv_path, pptx_paths, slide_pages, model, skill_version
├── reviews/                     # 批量回复中转：每次 AI 生成的输入/输出 JSON（供调试回看）
│   ├── pending-reviews-{ts}.json            # 写给 review-reply skill 的输入（target_language + channel + groups[]）
│   └── pending-reviews-{ts}.candidates.json # skill 产出的候选回复，前端读回回填
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
| 生成页 | `GeneratePage.vue` | 展示选择摘要 → 一键导出并调用 Claude → 终端风格日志 → 多轮对话输入 |
| 对比页 | `ComparePage.vue` | 双栏选择（AI 原始 vs 人工修改）→ 导出 HTML → 调 Claude skill 生成 diff → "在 Chrome 中打开"按钮 |
| 评论页 | `ReviewPage.vue` | Play Console 评论（原单应用页保留）。两种拉取：①「拉取评论」调 `list_play_reviews` 拉选中单个 app，按页面星级/回复状态/日期本地筛选；②「📦 批量拉取」从 `play-console-multi-config-v1` 读启用 app → 并行 `list_play_reviews` → 各 app 按其 Config 配置（星级/状态/日期）筛选后合并成**一个扁平列表**（按时间倒序，每条带应用标签 `_app`/`_pkg`，批量模式下页面筛选不适用）。每张评论卡片带「🤖 AI 回复」+「添加模板」，提交/收录用该条评论来源 app 的包名（`_pkg`）；「在网页打开」深链仅对当前选中 app 有效，其它退回应用列表页 | → 弹窗输入「回复方向」(一句指令) + 选回复语言(默认跟随评论语言) → 调 `generate_single_reply` 现场生成 **3 条不同风格**候选 → 点选/手动微调(实时 350 字符计数) → confirm 后调 `reply_to_review` 提交并本地回填为「已回复」。**与批量回复不同**：这里是 freeform 现生成(走 `claude --print` 直出 JSON 数组)，不走模板匹配。另有「📋 模板回复」按钮 → 弹窗按「通用 / 该 app 专用」两组列出**收藏的模板**（按钮=模板 category 名）→ 点一条按评论语言（`reviewer_language` 归一成 app 码）匹配预存译文自动填入 → 可微调（350 计数）→ 提交（`reply_to_review`）；无对应语言译文则回退英文源文并提示。每条候选可点「➕ 添加模板」把好回复收录进该应用对应产品的模板库（任意语言都可收：英文存英文模板，其它语言用候选中文预览 text_zh 存中文模板；收录时内联填类别 → `product_for_package` 解析产品 → `add_template` 带 lang） |
| 配置页（容器） | `ConfigPage.vue` | 纯配置页：顶部 Tab 切换「Play Console 拉取配置」/「Batch Reply 配置」，分别嵌 `PlayConsoleConfigPage` / `BatchReplyConfigPage`（v-show 常驻挂载，各管自己的 localStorage）。挂在 Review 工作区的 `review-config` 入口 |
| Play Console · 配置 | `PlayConsoleConfigPage.vue` | 多 app 卡片：勾选启用 + 日期预设 + 星级 + **回复状态**（全部/无回复/已回复/回复后又更新；选「回复后又更新」时星级置灰忽略）；非自定义预设显示「实际范围」预览（每分钟刷新）；顶部全选/全不选/刷新/清空配置/保存工具栏；写 localStorage `play-console-multi-config-v1`，应用列表缓存与 Batch 共用 `batch-reply-apps-cache-v1`。ReviewPage 读它作每个 app 的拉取/筛选默认值 |
| 批量回复 · 配置 | `BatchReplyConfigPage.vue` | 多 app 卡片：每张卡片独立**日期预设**（自上一个工作日 / 昨天 / 今天 / 近 7 天 / 自定义）+ 星级；非自定义预设下显示「实际范围」预览（每分钟刷新一次，跨午夜自动重算）；顶部「全选/全不选/刷新/清空配置/保存」工具栏（清空配置 = 删除所有版本 localStorage key 并恢复出厂默认，带 confirm）；显式保存到 localStorage `batch-reply-multi-config-v3`（从旧 key v2/v1 迁移时**强制把日期预设重置为默认**，因为早期 v2 会把"自定义"这个迁移产物写进每个 app；读当前 v3 则保留用户显式选择）；应用列表缓存在 `batch-reply-apps-cache-v1` 让冷启动秒回 |
| 批量回复 · 执行 | `BatchReplyPage.vue` | 进入时从 localStorage 读多 app 配置 → 「拉取候选评论」**在拉取时**调 `computeRange()` 把预设解析为绝对日期（所以配置一次永远新鲜）→ 并行调每个启用 app 的 `list_play_reviews`（不互相阻塞）→ 按 app 分组折叠展示（默认展开，组头显示 app 名 / 包名 / 当次实际日期范围 + 预设名 / 候选数）→ **「🔎 匹配模板并填充」调 `run_reply_skill`**（顶部下拉选回复语言默认 `auto`，模型固定 Sonnet）→ 命中模板的评论预填翻译好的模板正文（+中文预览）；未匹配的标「未匹配·需手动处理」留空手填 → 逐条提交直接发，「一键提交全部」弹 confirm 后跨 app 串行 + 200ms 间隔调 `reply_to_review`。每条评论可点「✋ 人工处理」手动标记（按 review_id 持久化到 localStorage `batch-reply-manual-ids-v1`，跨拉取/重启保留）——标记后**只**从「匹配模板并填充」里排除，仍可手填 / AI 单条回复 / 逐条或一键提交。每张候选卡片另有「🤖 AI 回复」按钮 → 弹单条生成弹窗（回复方向**可留空**，留空时后端让模型据评论自行判断方向〔含「无法更新」等常见问题的标准排查引导〕；+ 语言 → `generate_single_reply` 出 3 条风格各异候选）→「选用并填入」把文案灌进该卡片回复框（标记手动、清掉未匹配标），再走原有逐条/一键提交。**单条弹窗是多任务的**：可同时开多条（缩小成右下角竖直堆叠的悬浮条），生成走前端队列排队（后端 `ReplyState` 一次只跑一个），`reply-log` 路由到当前正在生成的那个任务、不污染批量匹配日志。每条候选也有「➕ 添加模板」收录入口（任意语言：英文存英文、其它用中文预览存中文、填类别 → `product_for_package` + `add_template` 带 lang）。与「匹配模板」是两条独立路径：模板匹配=批量命中翻译，AI 回复=单条 freeform 现生成 |
| 模板管理 | `TemplateManagerPage.vue` | 挂在 Review 工作区。每条模板行首有 ★收藏开关（写 `tpl-fav-ids-v1`，供评论页「模板回复」弹窗读）。按产品 tab 切换（显示条数 + 关联 app），列出该产品模板，每条可改 category/正文 → `update_template`，删除走**内联两步确认**；顶部「+ 新增模板」→ `add_template`（后端按产品前缀自动生成 id）；「📥 从 xlsx 导入」用系统文件选择器（`@tauri-apps/plugin-dialog`）选 xlsx → 内联确认（覆盖该产品）→ `import_templates_xlsx`。所有写操作落 `~/.tester-app/templates/`，skill 直接读 |
| Gmail 页 | `GmailPage.vue` | 读 Apps Script 同步出的 Google Sheet（手动粘贴表链接，存 localStorage `gmail-sources-v1`，每张表可配一个 Chrome profile）：复用 `read_sheet`/`get_sheet_tabs` 读 `Mail` tab、按表头名取列；列表每封固定 3 行（发件人+日期 / 主题 / 机翻中文），「详情」弹大卡（机翻上 / 原文下），「↗」按配的 Chrome profile 打开邮件深链（`open_url_in_chrome_profile`），「已读」本地隐藏（localStorage `gmail-read-ids-v1`，「↩ 撤销上一封」LIFO）。绕开 Gmail OAuth Testing 7 天过期，全程见 `gmail-handoff.md` |
| 设置页 | `SettingsPage.vue` | 缓存大小查看与清理 |

## 后端模块

| 模块 | 文件 | 职责 |
|---|---|---|
| 认证 | `auth.rs` | Google OAuth 2.0 PKCE 流程：本地 TCP 回调服务器、令牌交换、自动刷新、持久化至文件 |
| Google API | `sheets.rs` | Drive 文件列表、Sheets 读取与 CSV 导出、xlsx 上传（路径或字节、自动转 Google 表格、归入 `tester-app` 文件夹）、Slides 幻灯片获取与 PPTX 导出、缩略图异步缓存 |
| Claude 集成 | `claude.rs` | 定位 Claude CLI 路径、子进程管理、stream-json 输出解析、会话 ID 续接、实时事件推送 |
| 对比流程 | `compare.rs` | 单 Tab Sheet 导出 HTML（`docs.google.com/.../export?format=html&gid=`）、内嵌脚本写盘后直接执行 `python diff_testcases.py`、在 Chrome 打开报告（`compare-log` 事件流，独立于 `claude-log`） |
| 单条 AI 回复 | `reply.rs` | `generate_single_reply` command：给**一条**评论 + 用户一句「回复方向」+ 语言，在 Rust 里把评论上下文 + 方向 + review-reply skill 的硬性标准(≤350 字符/回复语言/不编造/保留 emoji 专名/引号规范) 拼成 prompt → 跑 `claude --print --output-format stream-json --permission-mode bypassPermissions`(无 skill、无文件往返) → 从终结 `result` 事件取最终文本 → `extract_json_array` 容错抠出 JSON 数组(防 markdown 代码块/前后散文) → 解析成 **3 条风格各异**的候选 `{style,language,text,text_zh,char_count}` 返回。复用 `ReplyState`(与批量共用 running 锁 + `stop_reply` 可中断) 和 `reply-log` 事件流 |
| 批量回复生成 | `reply.rs` | `run_reply_skill` command：把前端传来的 `groups[]` 包成 `{target_language, channel:"gp", groups}` 写到 `~/.tester-app/reviews/pending-reviews-{ts}.json` → 跑 `claude --print --output-format stream-json --permission-mode bypassPermissions --add-dir <reviews> --add-dir <模板目录> --model claude-sonnet-4-6 /review-reply <json>` → 等子进程结束 → 读回同 stem 的 `*.candidates.json` 解析。**模板目录**=`~/.tester-app/templates/`，路径既用 `--add-dir` 授权、也写进 prompt（「模板数据目录：…」）显式告诉 skill 去哪读 index/templates/package_map。返回 `{output, usage}`（usage 来自 stream-json 终结 `result` 事件的 token/费用）。独立 `ReplyState`（running + child_pid）+ `reply-log` 事件流。`stop_reply` command 杀进程树并置 running=false → inner 检测到返回 `CANCELLED`。复用 claude.rs 的 `find_claude` / `load_claude_token`。skill 流程为「匹配 only」（命中→翻译模板1条；未命中→matched:false）。**注意**：skill 是 LLM 手写 candidates.json，含未转义引号会非法 → SKILL.md 已要求写完自检 JSON；后端严格 `serde_json` 解析，非法报错。`stop_reply` 可中断 |
| 评论拉取 | `reviews.rs` | 三条命令：① `list_play_apps` 调 `playdeveloperreporting.googleapis.com/v1beta1/apps:search` 列出账号下所有应用（包名 + 显示名）；② `list_play_reviews` 调 `androidpublisher.googleapis.com/v3/applications/{packageName}/reviews`（支持 `translationLanguage` 参数让 Google 直接返回译文，原文落 `originalText`），扁平化 comments 数组（取最新 userComment + developerComment）；③ `reply_to_review` 调 `…/reviews/{reviewId}:reply` POST `{replyText}`（配额：2000/天/应用）。服务端不支持任何筛选 → 一次性返回全部 7 天评论，由前端按星级/回复/日期本地过滤；遇 401 或 `ACCESS_TOKEN_SCOPE_INSUFFICIENT` 返回 `NEED_RELOGIN_SCOPE:` 前缀，前端提示重登 |
| 生成-上传 manifest | `manifest.rs` | `write_generate_manifest` command：把"生成的 xlsx 上传到 Drive 后的 drive_id"和"用来生成它的源文件（CSV + PPTX + 页码）"绑定写盘；compare 页反馈时按 ai_drive_id 反查 |
| 反馈上传 | `feedback.rs` | `send_feedback` command：反查 manifest → 打包 zip（ai.html + human.html + report.html + 源文件 + meta.json）→ Telegram sendDocument multipart 上传 → 成功移到 `feedback_sent/`，失败留 `feedback_pending/`；`retry_pending_feedback` 重试；`is_feedback_configured` 探测是否配置好 token |
| Skill 热更新 | `skill_sync.rs` | 内置 skill 列表（owner/repo）；用 GitHub Releases API（`/releases/latest`）拿 tag_name 做版本判断；`check_skill_updates` 比对本地 `.tester-app-version` 与远程 tag；`sync_all_skills` / `sync_skill` 下载 release zipball → 备份旧版 → 解压覆盖到 `~/.claude/skills/{name}/` → 写新版本；`get_skill_local_version` 给前端取版本号写进反馈 manifest |
| 模板管理 | `templates.rs` | 7 个 command（无 State，纯文件读写）：`list_template_products`（产品+条数+关联 app）/ `product_for_package`（包名→产品，给「添加模板」收录用，null=该应用无模板产品）/ `list_templates` / `add_template`（按产品前缀 xfolder/mp3cutter/video2mp3 自动生成 id）/ `update_template` / `delete_template` / `import_templates_xlsx`（calamine 读 xlsx，复刻 build 解析口径：A 列类别继承、B 列英文、空 B 跳过，覆盖该产品，导入项 lang=en）。模板含 `lang`（en/zh-CN）**中英双源**：add/update 带 lang，skill 命中后据 lang 翻到回复语言（相同直接用）；存量无 lang 字段的按 en。**唯一写出口** `write_templates_and_index` 写 templates.json 同时由全文派生重建 index.json，二者永不漂移。`ensure_templates_dir` 首次从 `~/.claude/skills/review-reply/data/` 拷种子。`reply.rs` 复用其 `templates_dir()` / `ensure_templates_dir()` |
| 模板多语言预翻译 | `translate.rs` | `translate_templates(product, ids, langs, overwrite, channel, model)` + `stop_translate`：每条按（覆盖/只补缺失 + 排除同源码）算 target_langs，CHUNK=1 逐条 spawn `claude --print`（**不 --add-dir、prompt 只内联本批、禁工具/不读写文件、stdout 解析**，避免 agent 读全量 templates.json 烧额度），每批 `apply_translations` 增量写回 + 刷 src_hash；350 字符硬校验（超长压缩重试一次仍超则标红警告）；`translate-log`（含用量）+ `translate-progress`（进度条）事件；复刻 reply.rs 的取消逻辑。模型默认 haiku。详见 `handoff-template-i18n.md` |
| Chrome 打开 | `chrome.rs` | `list_chrome_profiles` 读 Chrome `Local State` 的 `profile.info_cache` 列出 profile（目录名 ↔ 显示名，用户按显示名选、存目录名）；`open_url_in_chrome_profile(url, profile_dir)` 用 `--profile-directory` 指定 profile 打开 URL（macOS 直接调 Chrome 二进制并 `open -a` 激活前台，Win/Linux 各自分支）。解决 Gmail 多账号分散在不同 Chrome profile 时，邮件深链跨 profile 跳不过去 |

# 依赖库

## 前端依赖（npm）

| 库 | 类型 | 作用 |
|---|---|---|
| `vue` | 核心 | Vue 3 响应式 UI 框架 |
| `@tauri-apps/api` | 核心 | Tauri 前后端通信桥接（invoke 命令、事件监听） |
| `@tauri-apps/plugin-opener` | 插件 | 在系统浏览器中打开 URL |
| `@tauri-apps/plugin-dialog` | 插件 | 系统文件选择器（模板管理页选 xlsx 导入） |
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
       ├─ 导出 Slides PPTX → export_slides_pptx() → ~/.tester-app/exports/*.pptx
       └─ 调用 Claude CLI  → run_claude_task(csv, pptxs, pages)
            └─ claude --print --verbose --output-format stream-json --file <csv> --file <pptx> '/test-case-generator ...'
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
    handleGenerate() 时把 csvPath / pptxPaths / slidePages / model 存入 lastGenContext
    handleUploadToDrive() 上传 xlsx 拿到 drive_id + web_url 后，调
      write_generate_manifest({ drive_id, web_url, source_csv_path, pptx_paths, slide_pages, model, skill_version })
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


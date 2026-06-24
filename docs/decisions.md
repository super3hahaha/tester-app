# 架构决策记录

## skill 优化的反馈回流走 Telegram bot，不走 GitHub/server

- **场景**：用户（测试人员）不懂 Git；维护者（xxtester2026）一人优化 skill；前期只有几个用户。
- **选择**：在 ComparePage 加"反馈"按钮 → 打包 zip → Telegram sendDocument 直接进维护者的私聊。
- **拒绝的方案**：(a) GitHub PR 流程 — 用户不会用；(b) 自建后端 + 审核 — 早期 ROI 太低；(c) 公共 marketplace — 不成熟。
- **可演进**：单点上限大概是几十用户/每月几十次反馈。超出后再迁到 Cloudflare R2 + 极简后端。

## 反馈关联键用 Drive ID，不用文件名/时间戳

- AI 版 Sheet 上传到 Drive 后拿到的 `drive_id` 是稳定 ID，用户改名 / 移动文件夹都不影响。
- GeneratePage 上传成功后写 `~/.tester-app/manifests/{drive_id}.json` 记录源 CSV / PPTX / 页码。
- ComparePage 选 AI Sheet 时 `file.id` 就是 drive_id，反查零成本。
- 旧数据 / 粘贴 URL 选的 sheet 没 manifest，反馈降级到"无源文件"，不阻塞流程。

## bot token 编译期嵌入，附带运行时 json 兜底

- **主路径**：`option_env!("TELEGRAM_BOT_TOKEN")` + `option_env!("TELEGRAM_CHAT_ID")`，build 时设环境变量 → token 进二进制 → 发版给用户。
- **兜底**：`~/.tester-app/telegram.json`（`{bot_token, chat_id}`），用于 dev 调试不重新 build。
- 没有任何一个时，前端 `is_feedback_configured()` 返回 false，反馈按钮直接不显示。
- token 泄漏的最坏后果：攻击者往维护者私聊塞垃圾。bot 不加任何群、chat_id 写死自己 → 风险可控；BotFather 一键 revoke 即可。

## diff 脚本不走 Claude CLI

- `diff_testcases.py` 是确定性纯函数，没必要 LLM 介入。
- compare.rs 直接 `include_str!` 内嵌脚本，运行时落盘后 `python diff_testcases.py` 跑。
- 节省 LLM 启动开销（数秒～十几秒）+ token 消耗。
- skill 的"调用"本质只是"运行这个脚本"，去掉中间层。

## Skill 分发走 GitHub Releases 热更新，不打进二进制

- **场景**：维护者高频优化 skill（每周可能多次），但 app 本身改动不多；用户都是非技术人员。
- **选择**：skill 源码放公开 GitHub 仓库，维护者用 GitHub Releases 打 tag 发版（如 `v1.8.0`）；app 启动时拉 `/releases/latest` 比对 tag_name，不一致就下 release 的 zipball 覆盖到 `~/.claude/skills/{name}/`。
- **拒绝的方案**：(a) `include_str!` 打进 binary — 每次优化都要重新发版，用户重装，太重；(b) commit SHA 比对 — 每次普通 push（typo、注释、refactor）都触发用户更新，噪声大；(c) 手动 `version.txt` 文件 — 跟 release tag 重复，且容易忘 bump。
- **为什么 release 而非 commit**：维护者明确的"发布"动作 = 用户的"更新"信号。维护者能在 master 自由迭代/试错，只在认为稳定时打 tag；用户得到的永远是经过维护者审视的版本。
- **可演进**：当 skill > 3 个时，把 hardcoded 列表挪到远程 manifest（也是 raw GitHub URL），加 skill 不用改 app。
- **回退保障**：覆盖前把旧版整目录 rename 到 `~/.tester-app/skill_backups/{name}_{old_version}_{ts}/`；解压失败自动 rename 回去；用户能手动从 backups 翻回任意旧版。
- **不需要 GitHub token**：公开 repo + 公开 API，60 req/hour/IP 远远够用（启动 1 次 + 手动按钮）。

## Slides 缩略图缓存：key 用 objectId，失效用 revisionId

- **场景**：用户在 Google Slides 里改一页文字 / 插入一页 / 删一页 / 重排，本地缓存必须能正确失效，否则 SlidesPage 会显示错位/过期的图。
- **缓存 key 用 slide 的 `objectId`**：而非页码（`1.png`）。`objectId` 在重排/插入/删除时保持稳定，原图自动跟随到新位置；页码会"右移"导致 N 号文件指向不同内容。
- **整 presentation 级失效用顶层 `revisionId`**：Slides API 的 presentation 顶层有 `revisionId`，任何修改都会变。在 `~/.tester-app/thumbs/{presentation_id}/.revision` 记录上次拉取时的值；不一致就 `rm -rf` 整个目录重拉。
- **为什么不做 per-slide 失效**：Slides API 不暴露 per-slide revisionId，无法判断到底哪页变了。要么挨个比缩略图二进制（贵），要么全清重拉（简单+正确）。一个 ppt 几十张图全重下的成本可接受。
- **崩溃恢复**：`.revision` 在清完旧目录后立即写入，下载是异步 spawn 的。崩了的话已落地的 png 仍属于当前 revision，下次启动只补缺失，不会全清重拉。
- **刷新按钮**：SlidesPage 三个刷新入口（左栏文件列表 / 预览栏当前 ppt）。后端永远比对 revisionId，refresh 没改动时是空操作（不重复下载），所以频繁点也无害。

## Play Console 评论走 API 而非 URL 跳转

- **场景**：测试人员想按星级 / 回复状态 / 日期范围筛选评论看，过去靠在 Chrome 里点 Play Console。
- **选择**：用 Google Play Developer API（`androidpublisher.googleapis.com`）的 reviews 端点直接拉评论数据进 app；服务端不支持任何筛选 → 一次拿 7 天全量，本地过滤。
- **API 硬限制**：Reviews 端点只返回最近 **7 天**评论。超过 7 天得另走 GCS CSV 归档（未实现）；ReviewPage 顶部明确写了"最近 7 天"以免误导。
- **package_name vs 数字 ID**：Play Console URL 里的 `4973223441657725559` 是 Console 内部 app id，API 必须用包名（`files.fileexplorer.filemanager`）。两个 ID 都在 ReviewPage 表单里存着 —— 包名给 API 用，数字 ID 给"在 Console 打开"兜底按钮用。
- **OAuth scope 升级**：往 `auth.rs::start_login` 的 scope 串加了 `androidpublisher`，但**老的 refresh_token 绑的是老 scope**，不会自动升级。reviews.rs 检测 401 / `ACCESS_TOKEN_SCOPE_INSUFFICIENT` 时返回 `NEED_RELOGIN_SCOPE:` 前缀错误，前端提示用户 Logout 后重登；登录页本来就带 `prompt=consent`，重登时会重新弹同意页带新 scope。
- **拒绝的方案**：(a) 纯 URL 跳转 — 每次都要在浏览器手动操作，且数据没法进入 app 后续做分析；(b) Service Account — 当前用户没权限把 SA 加到 Play Console 用户列表里。
- **应用列表来自 Reporting API**：Publisher API 没有 "list my apps" 端点，只能用 Reporting API 的 `apps:search`（`playdeveloperreporting.googleapis.com/v1beta1/apps:search`）。它需要一个**独立的 OAuth scope** `playdeveloperreporting`，且在 GCP 里是**独立的 API**，必须单独启用「Google Play Developer Reporting API」—— Publisher API 已启用并不代表 Reporting API 也启用。漏启时会报 403 `SERVICE_DISABLED`，错误信息里直接给启用 URL。
- **保留 URL 兜底按钮**：复杂筛选 / 7 天以外的评论 / 网页回复仍走 Console，按当前表单参数拼 URL 后用 `openUrl` 打开（不能用 `target=_blank`，见 [[gotcha-tauri-open-url]]）。注意 Console URL 必须用**数字 ID**（developerId + appId），API 不返回这俩 → 在 ReviewPage 的"Play Console 跳转设置"折叠区里手动维护。

## diff 脚本两份拷贝 + build.rs 自动同步

- 上游源：`C:\Users\chenj\Documents\trae_projects\diff\scripts\diff_testcases.py`（独立项目）
- 仓内 vendored：`src-tauri/scripts/diff_testcases.py`（被 include_str! 编译进二进制）
- `build.rs` 每次 build 前对比并覆盖；上游不存在时静默跳过（保证别人机器上可独立 build）。
- 为什么不直接读上游路径：path 写死在二进制 = 只能在维护者机器上跑。

## 批量回复「人工处理」标记按 review_id 持久化

- **场景**：批量回复前人工先筛一遍，把**不适合走 AI 模板批量**的评论标出来。这些评论里有的确实不用回复，有的需要回复但模板不合适（用户会手动写或用 AI 单条回复）。
- **关键语义**：标记**只排除**「匹配模板并填充」这一步，**不影响**手动填写、AI 单条回复、逐条提交、一键提交全部。所以叫「人工处理」而非「无需回复」——后者会让人以为评论被完全排除（最初就踩了这个坑：误把它从 `handleSubmitAll` 也排除并禁用了输入框）。
- **实现**：`Candidate` 加 `manual` 字段；**只有** `buildSkillGroups` 跳过 `manual` 项，`handleSubmitAll` / `totalSubmittable` / 输入框 / 提交按钮一律不看 `manual`。
- **为什么要持久化**：候选每次拉取都重建（`fetchOne` 里 `g.candidates = []`），而被标记的评论在 Play 上仍是未回复态、每次拉取都会重现。不落盘标记就会丢。
- **只存 id 列表**：`localStorage` key `batch-reply-manual-ids-v1` 存 `review_id` 字符串数组，拉取时回填到 `candidate.manual`。不存整个 candidate —— 评论内容/译文每次重新拉，只有"用户的标记意图"需要跨拉取保留。
- **取消标记即恢复**：取消后该评论重新参与模板匹配，标记从 Set 里删掉并落盘。

## 模板数据从 skill 自带 data/ 迁到 app 管理的独立目录

- **背景**：review-reply 的回复模板原本是 `xlsx → build_templates.py → templates.json + index.json`，随 skill 经 skill_sync 同步到 `~/.claude/skills/review-reply/data/`，skill 运行时读自带目录。改模板要改 xlsx → 跑脚本 → push → 等同步，非技术使用者改不动。
- **选择**：在 app 里加「模板管理」页，模板落 `~/.tester-app/templates/`（独立于 skill），skill 改成从这里读——路径由 `reply.rs` 通过 `--add-dir` 授权 + prompt（「模板数据目录：…」）显式传给 skill。
- **为什么不直接写 skill 的 data/**：skill_sync 更新 skill 时会覆盖 `~/.claude/skills/review-reply/`，写那里会被冲掉；且让 app 耦合 skill 内部路径不干净。独立目录解耦。
- **索引由全文派生、app 重建**：`templates.json` 是权威全文源，`index.json`（id+category 瘦身索引）由后端 `write_templates_and_index` 在每次写时重建——不再依赖 python build，且二者永不漂移（唯一写出口）。
- **种子迁移**：首次（无 `~/.tester-app/templates/templates.json`）从 skill 已同步的 `~/.claude/skills/review-reply/data/` 拷三个 json；之后以 app 目录为准。仓库内 xlsx/json/build 脚本降级为历史/初始种子。
- **package_map 第一期不在 app 管**：包名↔产品映射沿用现有（拷过来只读），skill 仍读它；产品/映射的增改留作后续。
- **xlsx 仍可批量导入**：保留「从 xlsx 导入」入口（calamine 读，复刻原解析口径），覆盖式灌某产品；日常增删改则在 app 里直接做。

## 模板库改为中英双源（模板加 lang 字段）

- **背景**：原本模板库约定纯英文源，skill 命中后翻到目标语言；但运营有时想收录一条好的中文回复、或直接写中文模板。
- **可行性关键**：匹配阶段只读索引（id+category，category 本就是中文），**与模板正文语言无关**——所以换/混源语言不影响匹配，只影响"命中后取全文翻译"那一步。
- **选择**：每条模板加 `lang` 字段（`en` / `zh-CN`，serde 缺省 `en`，存量 302 条自动按 en）。skill 命中后据 `lang` 把模板正文翻到回复语言（回复语言==lang 直接用原文）。
- **「添加模板」任意语言都可收**：AI 候选总带中文预览 `text_zh`，所以英文候选存英文模板（lang=en）、其它语言（含中文）用 `text_zh` 存中文模板（lang=zh-CN）。按钮不再按语言禁用。
- **不影响**：索引（不含 lang，匹配不需要）、category、模板管理 CRUD。skill 取全文命令改成同时取 `lang`+`text`。

## Gmail 走 Apps Script 同步到 Sheet，不直连 Gmail OAuth

- **背景**：要在 app 里看多个 Gmail 账号（既有 @inshot.com 域内、也有外部 @gmail.com）的邮件。最初做法是 app 独立 Gmail OAuth（`gmail.rs`，`gmail.readonly` scope，loopback+PKCE）直连拉取。
- **致命问题——7 天过期**：为接外部 @gmail.com 账号，OAuth 同意屏幕从 **Internal** 改成 **External**；External 应用必须处于 Testing 或 Production。停在 **Testing** 状态下，refresh_token **固定 7 天过期**，过期后账号要重新走浏览器授权。
  - 这是绑在**应用发布状态**上的策略，**不绑账号类型**：改成 External 后，**inshot 域内账号也一样 7 天过期**，不豁免（Internal 才豁免，但 Internal 用不了外部账号）。
  - 区分两件事：「加测试用户名单」只解决**准入**（不在名单报 `access_denied`），是一次性配置；7 天过期是**token 续期**问题，名单不受影响、不用重配。
- **为什么不走 Production**：`gmail.readonly` 是 Google **受限（Restricted）** scope（比"敏感"更严），发 Production 需品牌验证 + 每年一次 CASA 第三方安全评估，成本/周期过高。用户**两条路都不接受**（不忍 7 天、不走 Production）。
- **选择**：把"访问 Gmail"从 app 的第三方 OAuth client 整个挪到**账号本人的 Apps Script**：每个 Gmail 账号部署 [gmail-sync.gs](gmail-sync.gs)，定时触发器（每 15 分）用 `GmailApp` 增量拉邮件写入**各自一张 Google Sheet**，表共享给 inshot 账号；app 复用 `sheets.rs`/`auth.rs` 读表（完全不碰 Gmail API）。
- **为什么能消灭 7 天**：Apps Script 触发器以**账号本人**身份运行，授权由 Google 内部托管，不走第三方 OAuth client，不受同意屏幕发布状态影响；app 端读的是 Sheet，用现有 `auth.rs` 的 Sheets 授权（inshot 域内 Internal，本就稳定不过期）。两类账号都免疫 7 天，装一次长期跑。
- **拒绝的方案**：(a) 忍 7 天重授权 — 每账号每周人工点一次，易忘、过期即停同步；(b) 发 Production — 受限 scope 验证 + CASA 太重；(c) 两套 OAuth client（inshot 走 Internal、外部走 External）— 双倍配置和代码复杂度，不值。
- **已定范围（2026-06-17）**：第一阶段**只读同步**（脚本写表 / app 读表 / 回复外跳浏览器手动发，点 Gmail 深链 `#all/<threadId>`）；表**每账号各一张**；**发送回写留第二阶段**（app 写回复+「已确认」列 → 脚本扫已勾选行用 `GmailApp.reply` 代发，**不需要 `gmail.send` scope、不需要 Production**，逐封人工确认=勾选列）。
- **代价**：失实时性（定时同步，可降到每 15 分）；每账号一次性装脚本+授权；正文受单元格 5 万字符限制（HTML 转纯文本、截断）；消费级账号脚本发信配额 ~100/天（只读阶段不涉及）。
- **影响旧代码**：`gmail.rs`（OAuth 账号管理 + `list_messages`/`get_message`）已删除；app 改为读 Sheet，`GmailPage.vue` 完整实现（读表 + 卡片 UI + 详情弹窗 + 已读隐藏 + AI 回复草稿 + 模板回复 + chrome profile 跳转）。

## 模板多语言预翻译（translations 字段 + template-translate skill）

- **背景**：review-reply 命中模板后每次都实时翻译到回复语言，重复、慢、费钱；模板是固定的，没必要每次翻。
- **选择**：翻译从**运行时**挪到**一次性预生成**。每条模板存 `translations`（语言码→译文，22 种 app 原生码 `ar/de/…/zh-rCN/zh-rTW`）+ `src_hash`（翻译当时的源文指纹）。review-reply 命中后把回复语言归一成模板码，命中预存译文**直接用**，漏译/新语言才回退实时翻译。
- **翻译执行 = 轻量直出 + haiku（不走 skill）**：`translate.rs` 直接 `claude --print`，**不 `--add-dir`、prompt 只内联这批模板、要求「不用工具、直接输出 JSON」、从 stdout 解析**，不写文件、不自检。每批 `CHUNK=5` 条、**每批立刻写回盘**（中断只丢这几条）。模型用 `claude-haiku-4-5`（前端 `TRANSLATE_MODEL`）。每条模板自带 `target_langs`，后端按「覆盖 / 只补缺失」精确算要翻什么。
  - **为什么不走 agent/skill**（踩过的坑）：最初做成 `template-translate` skill（agent 模式 + `--add-dir` 模板目录 + Bash 写文件自检）。agent 因 `--add-dir` 自己读了 122KB 的 templates.json、自检失败会重写整份译文，**20 条就烧掉 30% 的 5 小时额度**。翻译是纯文本转换，不需要 agent 的工具/文件/自检——轻量直出省一个数量级。`template-translate` skill（super3hahaha，v0.0.2）已**退役**，`skill_sync` 注册可留可删（留着只是多同步一个不用的 skill）。
  - 反思见 memory `plan-cost-first-and-spike`：定方案要先按成本打分 + 先小规模真跑再铺开。
- **三种场景一套 UI**（模板管理「🌐 补全多语言」+ 每条「重译」）：首次铺底=整产品+全语言；单条重译=该条覆盖（源改了 `src_hash` 不符→stale 高亮提示）；新增语言=只补缺失、追加不覆盖。
- **源语言不进 translations**：源 `en`/`zh-CN`，translations 只存其它语言；`is_source_lang` 判定 `zh-rCN↔zh-CN` 同源，避免把中文源再翻成中文。
- **stale 机制**：改了源文 `text` 没重译 → `src_hash != hash(text)` → UI 标「源已改」。`update_template` 不主动动 `src_hash`，靠它自然变 stale；`list_templates` 返回 `TemplateView`（flatten + `stale`）。
- **xlsx 导入清空译文**：覆盖导入=全新源，`translations` 清空，提示重新补全。

## 可编辑提示词（prompt_config.rs + 设置页「提示词配置」）

- **背景**：app 里 5 处 prompt 全写死在 Rust（reply/analysis/translate）。用户要能在设置里改 prompt，且单条「AI 生成回复」(`build_gen_prompt`) 一直漏注入知识库（只有「🔍 分析」注入了），导致知识配置里的反馈邮箱等用不上。
- **整段完整模板可编辑（最终选定）**：最初做成「只开放纯 prose 规则段、占位符/JSON 锁死」，用户嫌不够灵活，改成**每个 prompt 存完整文本**（含 `{product}`/`{star}` 占位符 + JSON 输出格式），任意改。代价是改坏占位符/JSON 会导致解析失败——明确接受，靠设置页每字段「恢复默认」(`get_default_prompt_config`) 兜底。
  - **不用 `format!`**（占位符运行时才知道）：`render(template, &[(k,v)])` 对每个已知 token 做 `replace("{k}", v)`。JSON 示例里的单括号 `{ "k": ... }` 不构成 `{token}`，不会被误替换——所以模板里 JSON 用**单括号**（不是 format! 的 `{{`）。
  - `load()` 缺失/损坏/字段缺回退 `default_*()`（逐字等于原写死文本），未编辑行为完全不变。存 `~/.tester-app/prompt-config.json`，同 model_config 模式。
- **只开放 3 个回复类**（gen/analysis/mail），翻译类 #4/#5 不开放：翻译模板含语言码/`{tpls_json}`（输出 key 必须一字不差），属解析关键，开放风险大、收益低。
- **独立「Prompt」二级页**：放在 Settings 二级导航（`settings-prompt`，与 `settings-general` 并列），不挤在 General 页里——prompt 模板长，单独成页编辑更清爽。
- **知识库注入对齐**：`generate_single_reply` 现在和 `generate_analysis` 一样按 product（退回 package_name 解析）读知识块注入 `{app_knowledge}`，两个功能行为统一。

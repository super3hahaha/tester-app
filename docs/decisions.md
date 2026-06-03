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

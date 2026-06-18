# Gmail 接入 — 交接文档

> 给冷启动接手的 Claude / 开发者。本文档记录 Gmail 功能的目标、已完成的工作、当前状态、下一步。
> 写于上一个会话末尾(该会话的终端回显损坏,无法继续可靠执行命令,故转为交接)。

---

## ⚠️ 方案已变更(2026-06-17)—— 先读这段

**放弃 app 直连 Gmail OAuth,改为「Apps Script 定时同步到 Google Sheet → app 读表」。**

变更原因:Gmail OAuth 同意屏幕处于 **Testing** 状态时,refresh_token 固定 **7 天过期**,账号要反复重新授权;而 `gmail.readonly` 属 Google **受限(Restricted)** scope,发布到 Production 需走品牌验证 + 每年一次 CASA 第三方安全评估,成本过高。用户**不接受 7 天重授权,也不走 Production**。

**新架构(把"访问 Gmail"从你的 OAuth client 挪到账号本人的 Apps Script):**
```
[每个 Gmail 账号] Apps Script 定时触发器(每15分,账号本人授权,长期有效,无7天问题)
      │ GmailApp 拉新邮件
      ▼
[每账号各一张 Google Sheet]  共享给 inshot.com 账号
      │
      ▼
[tester-app] 复用 sheets.rs/auth.rs 读表(Internal 授权,稳定不过期)→ 列表+正文
              完全不碰 Gmail API
```

**为什么能消灭 7 天**:Apps Script 触发器以**账号本人**身份运行,授权由 Google 内部托管,不走你的第三方 OAuth client,不受同意屏幕发布状态影响;app 端只读 Sheet,用的是现有 `auth.rs` 的 Sheets 授权(inshot 域内 Internal,本就稳定)。两头都没有 7 天问题。

**已定的范围(2026-06-17 与用户确认):**
- 第一阶段**只读同步**:脚本写邮件入表,app 读表展示;**回复仍外跳浏览器**(点 Gmail 深链本人手动回)。
- 表格粒度:**每个 Gmail 账号各自一张表**(不是一表多 tab)。
- **发送回写留作第二阶段**:只读链路验收 OK 后,再让脚本扫描「已确认」列代发(用 `GmailApp.reply`,不需要 `gmail.send` scope、不需要 Production)。

**产出物(均已落盘):**
- Apps Script:[gmail-sync.gs](gmail-sync.gs) —— ✅ 已部署跑通(每天 9 点同步标签下未读→表,机翻静态值)。
- 前端:[GmailPage.vue](../src/pages/GmailPage.vue) —— 读表 + 卡片列表 + 详情弹窗 + 已读隐藏。
- 后端:[chrome.rs](../src-tauri/src/chrome.rs) —— 按指定 Chrome profile 打开邮件深链。
- 接线:MainPage 加「📧 邮件」workspace;lib.rs 注册 chrome 命令。
- 详见 §3b(第二步真实代码清单)。

**对下方原内容的影响:**
- 第 3 节「已写入的代码」(`gmail.rs` OAuth 账号管理 + `list_messages`/`get_message`)**基本退役**——保留作记录,但新架构下 app 不再用它直连 Gmail,改读 Sheet。`gmail.rs` 的 token 维护逻辑不再需要。
- 第 4 节那条「7 天 gotcha」**已通过本方案绕过**,见该节更新。
- 路线表(第 2 节)已按新方案重写。

## 0. 接手第一件事(重要)

上个会话的 Bash 回显损坏,**无法 100% 确认工作区落盘状态**。接手后**先跑一遍**核实:

```bash
cd /Users/zhangshixin/Projects/tester/tester-app
git status --short
ls -l src-tauri/src/gmail.rs src/pages/GmailPage.vue
grep -n gmail src-tauri/src/lib.rs
grep -n Gmail src/pages/MainPage.vue
```

预期:`gmail.rs`、`GmailPage.vue` 存在(已确认 gmail.rs EXISTS);`lib.rs`、`MainPage.vue` 已修改。
如果缺失,对照下方「第一步改动清单」补回。**改动尚未 commit。**

---

## 1. 需求(完整目标)

一条 **Gmail 邮件处理工作流**,挂在现有 Tauri app 里(QA/测试团队内部工具):

```
选 label → 拉取该 label 下的邮件 → 取邮件原文
   → Claude 翻译成中文 → 生成回复(模板匹配 + AI 生成)→ 发送回复邮件
```

关键约束(用户已确认):
- **Gmail 与现有 Sheets/Play 业务完全独立** —— 独立 OAuth、独立 token 存储、独立多账号管理。**不要动 `src-tauri/src/auth.rs`**(那是 Sheets/Play 在用的,改它会害现有用户被迫重新授权)。
- **多账号**:用户要看自己的多个账号,且**既有 @inshot.com 域内账号,也有外部 @gmail.com 账号**(两种都有)。
- **发送回复**必须**逐封人工确认**,绝不自动批量群发(对外发邮件的安全底线)。
- **回复内容来源**:模板匹配命中用模板,未命中用 Claude 生成(复用现有 review-reply 那套模式)。
- **翻译**:用项目已集成的 Claude(参照 `reply.rs` 的 `call_claude_for_reply`)。

---

## 2. 分步路线(已按新方案重写)

| 步骤 | 内容 | 状态 |
|------|------|------|
| **第一步** | **Apps Script 同步**:每账号部署 [gmail-sync.gs](gmail-sync.gs),每天定时拉标签下「未读」写入各自 Sheet、写后标记已读出队,表共享给 inshot 账号 | **✅ 已跑通**(2026-06-17,`filemanager.feedback@gmail.com` + 标签 `⭐mp3cutter-50字+-`,单账号验证) |
| 第二步 | **app 读表 + 卡片 UI**:新建 [GmailPage.vue](../src/pages/GmailPage.vue) 挂进 MainPage「📧 邮件」workspace。手动粘贴表链接、读 `Mail` tab、按表头名取列;列表每封固定 3 行(发件人+日期/主题/机翻中文)、「详情」弹大卡(机翻上/原文下)、「已读」本地隐藏+LIFO 撤销、「↗」用配的 Chrome profile 打开(新增 [chrome.rs](../src-tauri/src/chrome.rs))。详见 §3b | **✅ 已实测**(读表+Chrome profile 打开 OK;详情/已读/撤销为最新改动) |
| 第三步 | 选中邮件 → Claude 翻译正文为中文(参照 `reply.rs` 的 `call_claude_for_reply`) | 未开始 |
| 第四步 | 生成回复草稿(模板 + AI),供用户复制后外跳 Gmail 粘贴发送 | 未开始 |
| 第二阶段 | **发送回写**:app 把已确认回复写回 Sheet 的「回复内容」+「✅已确认」列;Apps Script 扫描已勾选行用 `GmailApp.reply` 代发(自动串线程,**无需 `gmail.send` scope、无需 Production**)。逐封人工确认 = 勾选列 | 未开始 |
| 后续 | Gmail 专用回复模板管理(现有 `templates` 按 package_name 维度,Gmail 无此维度,需适配:按账号分组 + keywords 匹配主题/正文) | 未开始 |

---

## 3.(作废)第一步 Gmail OAuth 改动清单 —— ⚠️ 从未落盘,不要去找这些文件

> 下面这套 `gmail.rs` / OAuth 直连方案**实际从未保存到磁盘**(上个会话终端回显损坏)。已确认
> `gmail.rs`、旧版 `GmailPage.vue` 都不存在,`lib.rs`/`MainPage.vue` 也没有 gmail OAuth 痕迹。
> 新方案(Apps Script 同步 + app 读表)**完全不用它**。本节仅留作"曾经的设计"参考。
> **真实落盘的第二步代码见下方 §3b。**

### 新增 `src-tauri/src/gmail.rs`
独立模块,~570 行。要点:
- **scope**(只读):`openid email https://www.googleapis.com/auth/gmail.readonly`
- **OAuth**:loopback + PKCE,`prompt=select_account consent`(让用户每次能选不同账号 → 支持多账号);复用 `credentials/oauth.json` 的 `client_id`/`client_secret`(自己独立读取一份,不依赖 auth.rs)。
- **Token 存储**:`~/.tester-app/gmail-accounts.json`,内容是 `Vec<GmailAccount>`(每个含 email/name/picture/access_token/refresh_token/expires_at)。
- **自动刷新**:`get_valid_token(email)` 检查过期,必要时用 refresh_token 续期并落盘。
- **6 个 Tauri 命令**:
  - `gmail_list_accounts()` → `Vec<GmailAccountInfo>`(不含 token)
  - `gmail_add_account()` → 弹浏览器授权,返回 `GmailAccountInfo`(同邮箱覆盖更新)
  - `gmail_remove_account(email)`
  - `gmail_list_labels(email)` → `Vec<GmailLabel>{id,name,type}`
  - `gmail_list_messages(email, labelId, maxResults?)` → `Vec<GmailMessageMeta>`(逐条 get metadata 拿 Subject/From/Date + snippet + unread;顺序请求,默认 25 封,后续可并发优化)
  - `gmail_get_message(email, messageId)` → `GmailMessageBody`(`format=full`,递归 MIME 树找 text/plain,退而求其次 text/html 粗略去标签 / snippet)
- base64 解码用 `URL_SAFE_NO_PAD` → `URL_SAFE` → `STANDARD` 兜底。

### 修改 `src-tauri/src/lib.rs`
- 顶部加 `mod gmail;` 和 `use gmail::GmailState;`
- `tauri::Builder` 加 `.manage(GmailState::new())`
- `invoke_handler![]` 末尾注册上述 6 个 `gmail::*` 命令

### 新增 `src/pages/GmailPage.vue`
完整工作流页:账号下拉(+添加 / 移除)→ 标签下拉(常用 label 排前)→ 左侧邮件列表(发件人/主题/摘要,未读加粗)→ 点击 → 右侧显示正文。invoke 用 `@tauri-apps/api/core`。

### 修改 `src/pages/MainPage.vue`
- `import GmailPage from "./GmailPage.vue";`
- `navItems` 加一个 workspace:`{ id:"gmail", label:"邮件", icon:"📧", children:[{ id:"gmail-inbox", label:"邮件", tabName:"Gmail" }] }`
- 模板里挂载:`<GmailPage v-show="activeOption === 'gmail-inbox'" />`

### 编译状态(上个会话验证过)
- `cargo check` 通过(EXIT=0)。有个 `unused import: Arc` 警告来自既有的 `reply.rs`,**与本次无关**。
- `npx vue-tsc --noEmit` 通过(exit=0)。

---

## 3b. 第二步改动清单(app 读表 —— 当前真实落盘的代码)

数据流:Apps Script 写表 → app 用现有 Sheets 授权读表(完全不碰 Gmail API)。

### 新增 `src/pages/GmailPage.vue`
- 挂在 MainPage「📧 邮件」workspace(`activeOption === 'gmail-inbox'`)。
- **邮件源 = 表**:手动粘贴 Google Sheet 链接添加,存 localStorage `gmail-sources-v1` = `[{id, label, profileDir?}]`。下拉切换 / ＋添加 / ✕移除 / ↻刷新 / 「在表格中打开」。
- **读表**:`get_sheet_tabs` 找 `Mail` tab(没有则取第一个)→ `read_sheet(spreadsheetId, "Mail")`。**按表头名取列**(messageId/threadId/日期/发件人/主题/正文/机翻中文/附件/邮件链接),列序变动不怕。行 `.reverse()` 让最新在上。
- **列表卡片**:每封固定 3 行——①发件人+日期+📎 ②主题 ③机翻中文(均单行省略号)。行内按钮:详情 / ↗ / 已读。
- **详情大卡**(overlay):机翻中文在上、原文在下 + 附件 +「↗ 在 Gmail 中打开」「标为已读」。
- **已读**:本地隐藏,messageId 存 localStorage `gmail-read-ids-v1`;拉取时过滤掉;「↩ 撤销上一封已读」= 对该列表 `pop`(LIFO)。**不写回表、不影响 Gmail**(纯本地,跨机不共享;与 ReviewPage「人工处理」标记同套路)。
- **打开邮件**:该表配了 `profileDir` → `open_url_in_chrome_profile`;否则系统默认浏览器 `openUrl`。

### 新增 `src-tauri/src/chrome.rs`(为什么:多账号分散在不同 Chrome profile,深链跨 profile 跳不过去)
- `list_chrome_profiles()` → `[{dir,name}]`:读 `~/Library/Application Support/Google/Chrome/Local State` 的 `profile.info_cache`,拿 目录名↔显示名(用户在 app 里按显示名 Manager/tester 选,存目录名)。
- `open_url_in_chrome_profile(url, profile_dir)`:macOS **直接调 Chrome 二进制** `…/Google Chrome --profile-directory=<dir> <url>`(比 `open -na` 可靠、能复用实例并前置窗口)+ `open -a "Google Chrome"` 激活兜底;Win/Linux 各自分支。
- `lib.rs`:`mod chrome;` + 注册这两个命令。

### 修改 `src/pages/MainPage.vue`
- `import GmailPage`;navItems 加 `{ id:"gmail", label:"邮件", icon:"📧", children:[{ id:"gmail-inbox", label:"Gmail", icon:"📨" }] }`;模板挂 `<GmailPage v-show="activeOption === 'gmail-inbox'" />`。

### 后端复用(未改动)
`sheets.rs` 的 `read_sheet` / `get_sheet_tabs` 直接用,走现有 `auth.rs` 授权 → **表必须已共享给 app 当前登录账号**,否则 403。这与"用哪个 Chrome profile 打开"是两回事:共享=app 能读到数据,profile=点开邮件跳对窗口。

### 编译/类型(本会话验证)
`npx vue-tsc --noEmit` exit=0;`cargo check` exit=0。

---

## 4. Google Cloud Console 配置(用户已完成)

- **项目**:`tester-496702`(从控制台 URL 得知)
- **OAuth client**:沿用现有 Desktop 类型 client(在 `src-tauri/credentials/oauth.json`,gitignored)。**没有新建 client。**
- **应用名**:Tester Tool;支持邮箱 zhangshixin@inshot.com
- **Gmail API**:✅ 已启用
- **scope**:✅ 已加 `gmail.readonly`(用户看到「已保存数据访问权限更改」)
- **用户类型**:原为「内部 Internal」(只能授权 inshot.com 域内账号)→ 因为要接外部 @gmail.com 账号,**用户已改为「外部 External」并添加了测试账号**。

### 由此引入的注意事项(External + Testing 模式)
- 只有**测试用户名单里**的账号能授权成功,其他报 `access_denied`。
- ⚠️ ~~**Testing 模式下 refresh_token 约 7 天过期**~~ → **已通过 Apps Script 方案绕过,见顶部「方案已变更」**。
  - 事实校准:7 天过期是 **Testing 发布状态**本身的策略(与只读/写无关);且 `gmail.readonly` 实为 Google **受限(Restricted)** scope(比"敏感"更严),发 Production 需品牌验证 + 每年 CASA 安全评估。两条路(忍 7 天 / 走 Production)用户都不接受,故改用 Apps Script:脚本以账号本人身份运行,授权不走第三方 OAuth client,无 7 天限制;app 改读 Sheet。
  - **此后 app 不再使用本节描述的 Gmail OAuth client**。下方 OAuth client / scope 配置仅作历史记录保留。

---

## 5. 下一步:第三步(Claude 翻译 + 生成回复草稿)

第一、二步已完成(见路线表 + §3b)。第三步起点:

- **目标**:在 GmailPage 详情卡里,用 Claude 对正文做**高质量翻译**(替代/补充表里的机翻),并**生成回复草稿**(模板匹配 + AI),供用户复制后点「↗ 在 Gmail 中打开」粘贴发送。
- **直接复用 ReviewPage 的 AI 弹窗那套**:`generate_single_reply`(reply.rs)、多任务排队(genBusy/processQueue)、候选选择、字数校验——把"评论"换成"邮件正文",`packageName/product` 维度换成"按账号/标签"或留空(参 reply 方向可留空)。
- **翻译**:`reply.rs` 的 `call_claude_for_reply` 范式(spawn `claude --print …`)。注意 macOS PATH 坑(见 §6)。
- **回复发送**仍走"外跳浏览器本人发"(第一阶段);自动代发是第二阶段(脚本回写 Sheet,见路线表)。
- **gmail-sync.gs 现状**:已含机翻列(`LanguageApp.translate` 写**静态值**,非公式;只翻正文前 `TRANSLATE_MAX_CHARS=5000` 字)。所以第三步的 Claude 翻译是"长文补翻/提质",不是必须。
- **gmail-sync.gs 关键配置**:`LABEL`(每账号标签名,可能不同!)、`MARK_READ_AFTER_SYNC=true`(同步后标记已读=出队)、`SYNC_HOUR=9`(每天 9 点附近跑)、按标签精确取(`getUserLabelByName`,不走 search 避开 emoji 标签匹配不到)。

---

## 6. 关键参考(实现后续步骤时复用)

- **Claude 调用范式**:`src-tauri/src/reply.rs` 的 `call_claude_for_reply` —— spawn `claude --print --output-format stream-json --verbose --permission-mode bypassPermissions <prompt>`,解析 `type==result` 行取文本。翻译/生成回复都照这个。
- **模板匹配**:`src-tauri/src/templates.rs` 的 `match_template(package_name, text, lang, dir)` + `Template{name,package_name,keywords,reply_text,lang}`,模板存 `~/.tester-app/templates`。Gmail 没有 package_name,需适配匹配维度。
- **Google API 调用风格**:`reqwest::Client` + `.bearer_auth(token)`,见 `gmail.rs` / `sheets.rs` / `reviews.rs`。
- **发送邮件(第四步)**:Gmail API `users.messages.send`,需构造 RFC822 原始邮件(base64url),带 `In-Reply-To`/`References` 头 + 请求体里带 `threadId` 才能正确串到原邮件线程。
- **macOS PATH 坑**:GUI app 启动不读 shell rc,spawn `claude` 会找不到 → 需 `fix_path_from_login_shell`(详见根目录 `Tauri2 + Vue3客户端开发指南.txt` 附录 B)。第二步起会用到。
- **架构总览**:`docs/PROJECT_STRUCTURE.md`;另有 `decisions.md` / `gotchas.md`。

---

## 7. 待办 / 未决(已按新方案更新)

- [ ] **用户先把 [gmail-sync.gs](gmail-sync.gs) 部署到一个 Gmail 账号,跑通一张表**(运行 `setup()` → `syncMail()`),确认数据列符合预期。
- [ ] 第一步验收 OK 后,开始第二步:app 端数据源从 `gmail.rs` OAuth 换成读 Sheet(复用 `sheets.rs`/`auth.rs`),改 `GmailPage.vue`。
- [ ] 评估 `gmail.rs` / `gmail_*` 命令是否直接删除(新架构不再用),还是先注释保留。
- [ ] 验证外部 @gmail.com 账号(消费级)与 inshot.com 账号都能装脚本+跑触发器(消费级发送配额 ~100/天,只读阶段不涉及)。
- [ ] 第二阶段(发送回写)再设计 Sheet 的「回复内容/已确认/发送状态」列与脚本代发逻辑。
- [ ] 本次改动(含旧 Gmail OAuth 代码)尚未 git commit。

### ⚠️ 已作废的旧待办(OAuth 方案下的,新方案不再需要)
- ~~`gmail_list_messages` 顺序请求改并发~~ —— app 不再直连 Gmail。
- ~~第三步加 `gmail.send` scope,所有账号重新授权~~ —— 改为脚本代发,无需 send scope。

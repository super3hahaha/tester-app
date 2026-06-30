# 多账号切换方案（Handoff）

> 目标：顶部账号区支持「同时登录多个 Google 账号 + 一键切换」，而不是退出再登录。
> 状态：**方案待确认**，未动手。

## 一句话结论

**好做，属于中等改造，不伤筋动骨。** 现有认证链路是干净的「单账号」模型，改成「多账号 + 当前活跃账号」是一次结构性扩展，不需要改 OAuth 流程本身。核心工作量集中在后端 `auth.rs` 的状态/存储结构和前端账号 UI，其它调 token 的业务模块（sheets / reviews 等）几乎不用动。

---

## 现状（为什么好做）

认证链路目前是「全局唯一一对 token/user」：

| 层 | 现在 | 关键位置 |
|----|------|---------|
| 后端内存 | `AuthState { tokens: Mutex<Option<AuthTokens>>, user: Mutex<Option<UserInfo>> }` | `src-tauri/src/auth.rs` |
| 磁盘 | `~/.tester-app/auth-tokens.json` + `auth-user.json`（每次登录覆盖） | `auth.rs:92` `data_dir()` |
| 前端 | `App.vue` 里一个 `user = ref<UserInfo|null>` | `src/App.vue:16-24` |
| 请求鉴权 | 业务命令统一调 `get_valid_access_token(&state)` 取当前 token | `auth.rs:61-88` |
| 顶部 UI | 头像 + 邮箱 + Logout | `src/pages/MainPage.vue:186-199` |

**关键利好**：所有 Google API 请求都走同一个入口 `get_valid_access_token()`。只要让这个函数「返回当前活跃账号的 token」，业务模块（sheets.rs / reviews.rs / 等）**一行都不用改**。这是这次改造成本可控的根本原因。

---

## 改造方案

### 1. 磁盘存储：从覆盖改成按账号分目录

```
~/.tester-app/
  accounts/
    <email-或稳定id>/
      auth-tokens.json
      auth-user.json
  active-account.json   # { "active": "<email-或id>" }
```

- 登录新账号 → 新建一个子目录，不覆盖已有账号
- 切换 → 只改 `active-account.json` 的指向
- 登出 → 删对应子目录；若登出的是当前账号，自动切到列表里下一个（没有则回登录页）

> 需确认：用 `email` 当目录名最直观，但邮箱含特殊字符且理论上可变。更稳的是用 Google 的 `sub`（OpenID 唯一 ID）当 key，`email` 仅做显示。**建议用 `sub`**（见决策点 A）。

### 2. 后端状态：从「一对」改成「一组 + 当前指针」

```rust
struct AuthState {
    accounts: Mutex<HashMap<String, Account>>, // key = sub/email
    active: Mutex<Option<String>>,
}
struct Account { tokens: AuthTokens, user: UserInfo }
```

- `get_valid_access_token()` 内部改成「取 active 账号的 tokens」，对外签名不变 → 业务模块无感
- 启动时扫描 `accounts/` 目录全部加载进内存

### 3. 新增 / 调整 Tauri 命令

| 命令 | 变化 |
|------|------|
| `start_login` | 登录成功后**追加**账号而非覆盖；自动设为 active |
| `check_auth` | 返回当前 active 账号（逻辑基本不变） |
| `logout` | 改为「登出指定账号」，参数加 `account_id`；缺省登出当前 |
| `list_accounts`（新增） | 返回已登录账号列表，供下拉展示 |
| `switch_account`（新增） | 入参 `account_id`，切换 active 并返回新当前用户 |

### 4. 前端

- `App.vue`：`user` 仍是「当前账号」，额外维护 `accounts` 列表
- `MainPage.vue` 顶部账号区：点头像/邮箱 → 下拉菜单，列出所有账号、勾选当前、提供「添加账号」「登出此账号」
- 切换账号后：**当前正在看的数据需要刷新**（见决策点 C）

---

## 决策点（A~F 全部已确认）

**A. 账号唯一标识 → 用 Google `sub`** ✅
用 OpenID 唯一 ID 当存储 key，`email` 只做显示——更稳，邮箱可能变。登录时需多解析一个 `sub` 字段。

**B. 切换是否需要重新授权 → 不需要，秒切** ✅
多账号 token 各自独立存盘，切换只换内存指针，**不重新走 OAuth**，不弹浏览器。

**C. 切换账号后的数据刷新 → 切回首页 + 只清「账号世界」** ✅
切到 B 账号后回到初始状态，由用户自己点进想看的页。实现简单，避免误显示旧账号数据。
> 落地提示：切换成功后前端重置当前路由/视图到首页。**注意清理范围**——只需清「账号世界」页面已加载的数据，**全局世界页面保持不变**（见下方「数据归属」）。不要无脑全清，否则知识库/模板/设置反而被误刷。
>
> - **需切换刷新（账号世界）**：`ReviewPage`（Play 评论）、`SheetsPage`（Drive/Sheets）、`GmailPage`、`SlidesPage` —— 全走 Google API
> - **保持不变（全局世界）**：知识库、模板库、Settings（模型/提示词配置）、各 localStorage 配置 —— 与账号无关

**F. `reviews-cache` 是否需要按账号隔离 → 不需要，维持现状** ✅
曾担心快照按 `package_name` 存会「串账号」，经确认**不是问题**：评论数据归属于 app（package）而非 Google 账号，两个账号只要都有该 app 权限，拉到的就是 Google 上同一份客观评论。即便命中另一账号写的旧快照，也是同一 app 的同一份数据，重新拉取即覆盖为最新。**缓存不分账号无害，无需改动。**

---

## 数据归属：哪些跟账号走、哪些全局共享

经代码核查，app 数据分两个世界，这决定了切换时要刷新什么：

### 🌐 全局世界（与 Google 账号无关，切换时原样保留）

| 数据 | 存储 |
|------|------|
| 测试用例知识库 | `~/.tester-app/knowledge/`（index.json + docs/*.md，纯本地，不碰 token） |
| 回复模板库 + 多语言译文 | `~/.tester-app/templates/`（translations 字段在 templates.json 内） |
| 模型配置 + GitHub Token | `~/.tester-app/model-config.json` |
| 提示词配置 | `~/.tester-app/prompt-config.json` |
| 评论分析知识块 | `~/.tester-app/review-analysis/` |
| Play 拉取配置 / Batch 邮件配置 / 模板收藏星标 | 前端 localStorage |

### 👤 账号世界（绑定当前 Google 账号，切换后需重新拉）

| 数据 | 入口 | 缓存 |
|------|------|------|
| Play 评论 | `list_play_reviews`（reviews.rs） | `~/.tester-app/reviews-cache/{pkg}.json` ⚠️ |
| Drive/Sheets | `list_drive_files`（sheets.rs） | 无元数据缓存，仅缩略图 `thumbs/` |
| Slides | sheets.rs | 缩略图 `thumbs/{id}.png` |
| Gmail | gmail API | 无 |

> 结论：知识库、模板、Settings 全在全局世界 → **切账号时它们照常可用，不用清也不用刷新**。这反而让「秒切」体验更顺。

## 关于本地缓存（reviews-cache / thumbs）：不需按账号隔离

`reviews.rs:395` 把评论快照按 `~/.tester-app/reviews-cache/{package_name}.json` 存，key 只有 package_name、无账号维度。**这是有意为之、无需改动**（决策点 F）：

- 评论数据归属于 app（package）而非 Google 账号；两账号只要都有该 app 权限，拉到的是 Google 上同一份客观数据。
- 缓存只是快照，重新拉取即覆盖为最新；即便切账号后命中旧快照，也是同一 app 的同一份真实数据，不会「显示错账号」。
- 缩略图 `thumbs/` 同理，无需处理。
- 因此切账号时连 `reviews-cache` 也不必清——决策点 C 的「只清内存」就足够。

**D. UI 形态 → 顶部下拉菜单** ✅
点头像/邮箱展开下拉，列出所有账号、勾选当前、提供「添加账号 / 登出此账号」。

**E. token 加密 → 本次（一阶段）维持现状，二阶段单独做** ✅
> 一阶段多账号改造**不动加密**，风险与现在单账号完全一致。加密增强放二阶段，详见下方「二阶段 todo」。

---

## 工作量估计

| 模块 | 改动 | 量级 |
|------|------|------|
| `auth.rs` 存储/状态/命令 | 中等重构（核心） | 半天 |
| 业务模块（sheets/reviews/…） | 基本不动 | ~0 |
| 前端账号下拉 UI + 切换逻辑 | 新增组件 + 状态 | 半天 |
| 各页面响应账号切换刷新 | 仅 Review/Sheets/Gmail/Slides 4 页清内存，全局页 + 本地缓存均不动 | 小 |

总体：**1～1.5 天**，无高风险点。

---

## 下一步（一阶段）

决策点已确认，可动手。建议落地顺序：后端存储与命令 → `list/switch` 命令自测 → 前端下拉 UI → 各页面切换刷新联动。

---

## 二阶段 todo：token 落盘加密（独立于多账号）

> 状态：**已确认要做，排在多账号之后**。与多账号无强耦合，可独立开工。

### 背景：token 是什么、为什么要加密

- 用户用 Google 登录授权后，Google 发给 app 两串 token，等于「免密码的通行证」，app 拿它以用户身份调 Google 接口（Sheets / Drive / Play 评论）。
  - `access_token`：真正用于访问，短期（约 1h）过期。
  - `refresh_token`：长期，用来自动换新的 access_token，免去反复登录。
- **现状风险**：两串 token 以**明文 JSON** 存在 `~/.tester-app/auth-tokens.json`（多账号后为 `accounts/<sub>/auth-tokens.json`）。任何能读到该文件的人/程序（恶意软件、共用电脑、误同步到云备份等）**无需密码即可冒充用户访问其全部已授权的 Google 数据**。
- 这风险单账号时就存在，多账号只是放大「一处泄露、波及账号数变多」，性质不变。

### 改造方向

- 不再把 token 明文写文件，改为存进 **macOS 系统钥匙串（Keychain）**，由系统级加密保管，其它程序读不到。
- Rust 侧可用 `keyring` crate（封装 Keychain / Windows Credential Manager / Linux Secret Service，顺带为跨平台留口）。
- 每账号一条 keychain 记录，key 用 `sub`；磁盘上仅保留非敏感的 `auth-user.json`（email/name/picture）与 `active-account.json` 指针。
- 迁移：首次启动检测到旧明文 `auth-tokens.json` → 读出写入 keychain → 删除明文文件。

### 影响范围

- 仅 `auth.rs` 的「读写 token」少数函数（`save_tokens_to_disk` / 加载逻辑 / `logout` 删除逻辑）。
- 业务模块、前端 **不受影响**（仍走 `get_valid_access_token`）。
- 量级：约半天；注意各平台 keychain 行为差异与首启迁移。

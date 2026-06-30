# 多账号 / Token 加密（Handoff）

> 一阶段「多账号切换」**已完成并合入 master**（commit `078f0e1`，2026-06-30）。
> 二阶段「token 落盘加密」**待开工**，本文保留其方案。

---

## 一阶段：多账号切换 ✅ 已完成

顶部账号区支持同时登录多个 Google 账号 + 一键秒切（不重走 OAuth）。落地实现见代码与文档，不再赘述方案：

- 存储/状态/命令：`src-tauri/src/auth.rs`（`AuthState` = `accounts: HashMap<key, Account>` + `active` 指针；命令 `list_accounts` / `switch_account` / `logout(account_id?)`）
- 数据目录结构 + 各命令职责：见 [PROJECT_STRUCTURE.md](PROJECT_STRUCTURE.md) 的「运行时数据目录」与「认证」模块
- 前端下拉切换 UI：`src/pages/MainPage.vue` 顶部账号区
- 已知坑（迁移 email-key 撞登录 sub-key → 启动去重）：见 [gotchas.md](gotchas.md)

落地的关键决策（备查）：账号 key 用 Google `sub`（迁移旧账号回退 email）；切换不重授权；切账号回首页并只清「账号世界」4 页（Review/Sheets/Slides/Gmail）内存，全局页（知识库/模板/Settings）不动；本地缓存 `reviews-cache` 不按账号隔离（评论归属 app 而非账号，无害）。

---

## 二阶段 todo：token 落盘加密（独立于多账号）

> 状态：**暂时整个搁置**（2026-06-30 决定）。维持现状——token 仍明文存 `accounts/<key>/auth-tokens.json`。
>
> **为什么搁置**：macOS app **没签名也没公证**。未签名 build 每次构建签名不稳定，系统钥匙串会判定「访问者变了」→ 每次版本更新后首次读 token 都弹授权窗，内部工具更新频繁 → 反复弹，体验不可接受。
> - Windows 这边不受影响（Credential Manager/DPAPI 不依赖签名、无弹窗、真加密），单独做是纯赚；但决定先不拆开单做。
> - macOS 在没签名前只能做「应用级加密」（机器派生/嵌入密钥），属混淆级——能防误同步到云、防顺手 `cat`，防不住本机恶意软件，价值有限。
>
> **重启条件**：先补齐 macOS **签名 + 公证**（需 Apple Developer 账号 $99/年）。之后系统钥匙串方案才成立，届时按下方技术方案两端都用 `keyring` 落地、零弹窗。
>
> 下方技术方案保留备查，重启时直接用。

### 背景：token 是什么、为什么要加密

- 用户用 Google 登录授权后，Google 发给 app 两串 token，等于「免密码的通行证」，app 拿它以用户身份调 Google 接口（Sheets / Drive / Play 评论）。
  - `access_token`：真正用于访问，短期（约 1h）过期。
  - `refresh_token`：长期，用来自动换新的 access_token，免去反复登录。
- **现状风险**：两串 token 以**明文 JSON** 存在 `~/.tester-app/accounts/<key>/auth-tokens.json`。任何能读到该文件的人/程序（恶意软件、共用电脑、误同步到云备份等）**无需密码即可冒充用户访问其全部已授权的 Google 数据**。
- 这风险单账号时就存在，多账号只是放大「一处泄露、波及账号数变多」，性质不变。

### 目标平台：macOS + Windows 都要支持

token 不再明文落盘，改存进**操作系统的凭据保险库**，由系统级加密保管、其它程序读不到：

| 平台 | 后端 | 说明 |
|------|------|------|
| macOS | Keychain（钥匙串） | 一等目标 |
| Windows | Credential Manager（凭据管理器） | 一等目标 |
| Linux | Secret Service | `keyring` 顺带支持，非目标、不强求 |

### 改造方向

- Rust 侧用 **`keyring` crate**：一套统一 API，编译期按目标平台自动选后端（macOS Keychain / Windows Credential Manager），**业务代码无需 `cfg!` 分平台分支**。
- 每账号一条凭据记录：`service = "tester-app"`、`account = <account-key>`（即现有 sub/email），value = token JSON。
- 磁盘上仅保留非敏感的 `accounts/<key>/auth-user.json`（sub/email/name/picture）与 `active-account.json` 指针；`auth-tokens.json` 不再写。
- 迁移（跨平台一致）：首次启动检测到旧明文 `accounts/<key>/auth-tokens.json` → 读出写入凭据库 → 删除明文文件。

### 跨平台注意点（动手前必看）

- **Windows 凭据大小上限**：Credential Manager 单条 `CredentialBlob` 上限约 **2560 字节**（5×512）。access_token + refresh_token 的 JSON 通常 1～1.5KB，多数在限内，但 Google 的 access_token 偶尔较长——**实测确认，必要时拆成两条凭据**（access / refresh 分存）或只存 refresh_token（access 可随时刷新，不必持久化）。
- **macOS 签名/Keychain 访问**：已签名 app 用稳定的 keychain access group，换签名/重装可能触发重新授权弹窗；dev 未签名 build 走 login keychain 一般无碍。注意别让首启迁移在无权限时静默吞错。
- **删除语义**：`logout` / `dedup_accounts` 删账号时，除删目录外还要删对应凭据记录（`keyring` 的 `delete_credential`），否则保险库里残留孤儿凭据。
- **两平台都要真机验证**：登录→重启读回→切换→登出→迁移旧明文，五条路径在 macOS 和 Windows 各跑一遍。

### 影响范围

- 仅 `auth.rs` 的「读写/删 token」少数函数（`save_account_tokens_to_disk` / `save_account_to_disk` / `load_accounts_from_disk` 的 token 部分 / `remove_account_dir`、`logout`、`dedup_accounts` 的删除逻辑）。
- 业务模块、前端 **不受影响**（仍走 `get_valid_access_token`）。
- 量级：约半天编码 + 两平台各一轮验证；主要风险在 Windows 凭据大小与 macOS 签名行为，故上面单列注意点。

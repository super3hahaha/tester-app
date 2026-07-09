# Handoff：按账号隔离本地存储

> 目标：多账号切换后，评论快照 / Play Console 拉取配置 / 各页配置随 active 账号刷新，不再串号。
> 最终做法（**B1' 加法版**）：不换存储介质、不建 KV 层、后端 `reviews.rs` 零改动；隔离只靠
> **给 localStorage key / 快照 key 注入一个「后端下发的 opaque 账号 id」前缀**。

状态：**一阶段已实施并通过编译（cargo check + vue-tsc 均通过）**。二阶段见 §6。

---

## 1. 问题根因（不是「没重挂」，是「存储没有账号维度」）

- 切账号已触发重挂：`MainPage.applyAccountChange()` 里 `accountEpoch++`，`ReviewPage / Gmail /
  Sheets / Slides` 带 `:key="acct-*-${accountEpoch}"` → 会重挂、onMounted 重跑。
- **但重挂后读的仍是全局 key**（localStorage 不带账号、后端 `reviews-cache/{pkg}.json` 只按包名）
  → 读回上个账号的数据。根因在存储层。
- **额外坑**：`ConfigPage(含 PlayConsoleConfigPage) / BatchReplyPage / AppScriptPage` 原本连
  `accountEpoch` key 都没有（`v-show` 常驻）→ 切账号后 onMounted 不重跑。一阶段已给 `ConfigPage`
  补 key；Batch/AppScript 留二阶段。

---

## 2. 隔离维度的来源（关键设计）

**account id = 后端下发的 opaque 字符串，前端只当黑盒用，不自行推算。**

- 后端 `auth.rs` 的 `account_key(user) = sub.unwrap_or(email)`。给 `UserInfo` 加了
  `#[serde(default) pub id: String`，在「account 进入 AuthState」处统一填充（`load_accounts_from_disk`
  + `start_login`），于是 `check_auth / switch_account / logout` 返回的 user 都自带正确 id。
- 前端 `App.vue` 是 `user` 的唯一真相源：`watch(user, {flush:'sync'})` 把 `user.id` 写入模块级
  `activeAccountId`（`src/utils/activeAccount.ts`）。切账号 `emit('update-user')` 同步更新 user →
  同步更新 id → 之后才 `accountEpoch++` 重挂子页，保证子页读 key 时 id 已就绪。
- **为什么不用前端 `sub ?? email` 自己算**：那会把「id 怎么算」硬编码进前端，一旦接入非 Google
  provider（key 变 `provider:id`）前端必漂移。opaque id 归后端拥有，换 provider 只改后端一处。

---

## 3. 存储盘点（哪些隔离 / 哪些不隔离）

判据沿用 MainPage 注释：「账号世界」(review/play/batch/gmail/sheets/appscript) 隔离；
「全局页」(模板/知识库/Settings) 不隔离。

### 3.1 需隔离 —— ✅ 一阶段已做 / ⏳ 二阶段

| 存储 | key | 持有页 | 状态 |
|---|---|---|---|
| 评论页配置 | `review-page-config-v3` | ReviewPage | ✅ scopedKey |
| 上次视图 | `review-last-view-v1` | ReviewPage | ✅ scopedKey |
| Play Console 拉取配置 | `play-console-multi-config-v1` | PlayConsoleConfigPage / ReviewPage(loadPlayConfig) | ✅ scopedKey |
| app 列表缓存 | `batch-reply-apps-cache-v1` | **三页共用** | ✅ scopedKey（三处引用同步改） |
| **评论快照** | 后端 `reviews-cache/{pkg}.json` | ReviewPage | ✅ 前端把 key 拼成 `${id}__${pkg}`，`reviews.rs` 零改动 |
| 批量回复配置 | `batch-reply-multi-config-v3` | BatchReply(Config)Page | ✅ scopedKey |
| 批量手动标记 | `batch-reply-manual-ids-v1` | BatchReplyPage | ✅ scopedKey |
| Gmail 源 / 已读 | `gmail-sources-v1` / `gmail-read-ids-v1` | GmailPage | ⏳ 二阶段 |
| AppScript 项目 | `appscript-projects-v1` | AppScriptPage | ⏳ 二阶段 |

> `batch-reply-apps-cache-v1` 三页共用同一字符串常量，一旦 scopedKey 化，三处引用必须同步（否则
> 同进程内读写不一致）。这是「只做 review+play」时绕不开、把两个 Batch 页也拉进来一点点的原因。

### 3.2 不隔离（全局，未动）
`tpl-fav-ids-v1`、`tpl-translate-langs`；后端 model/prompt-config、knowledge、templates 等。

---

## 4. 已落地的改动清单

**后端 `src-tauri/src/auth.rs`**
- `UserInfo` 加 `#[serde(default) pub id: String`（只出不进语义：落盘带此字段但无害，读回时
  account_key 重算、id 重填）。
- `load_accounts_from_disk` / `start_login`：填 `user.id = account_key`。

**前端新增**
- `utils/activeAccount.ts`：模块级 `activeAccountId` + `getActiveAccountId()`。
- `utils/accountScopedKey.ts`：`scopedKey(base) = `${base}::acct:${id || "_none"}``。
- `utils/accountStorageMigration.ts`：一次性迁移守卫（见 §5）。

**前端改调用点（机械包一层，读写逻辑不变、仍同步）**
- `App.vue`：inline `UserInfo` 加 `id?`；`watch(user, sync)` 写 activeAccountId；check_auth 后调迁移。
- `ReviewPage.vue`：`review-page-config-v3` / `review-last-view-v1` 读写包 scopedKey；新增
  `snapKey(pkg)` 给 `saveSnapshot/loadSnapshot` 拼 `${id}__${pkg}`。
- `PlayConsoleConfigPage.vue`：`PLAY_STORAGE_KEY` / `APPS_CACHE_KEY` 5 处包 scopedKey。
- `utils/playConsoleConfig.ts`：`loadPlayConfig` 读处包 scopedKey（保持同步）。
- `BatchReplyPage.vue` / `BatchReplyConfigPage.vue`：**仅** `APPS_CACHE_KEY` 包 scopedKey。
- `MainPage.vue`：`ConfigPage` 补 `:key="acct-config-${accountEpoch}"`。

---

## 5. 数据迁移（`accountStorageMigration.ts`）

- 首启已登录且无 `store-migrated-v1` 标记时，把旧全局 `review-page-config-v3` /
  `play-console-multi-config-v1` / `batch-reply-apps-cache-v1` 复制到当前账号 scopedKey 下、
  删旧全局 key、写标记。幂等。
- 旧全局配置属于「升级时那个账号」，迁给当前 active 最合理；其余账号从默认开始。
- 快照不迁（可重拉）。

---

## 6. 二阶段（验证 OK 后独立做）

同法把 `gmail-*` / `appscript-projects` 包 scopedKey，并给 `AppScriptPage` 补
`:key="acct-*-${accountEpoch}"`（现无 key，切账号不重挂、不重读）。

- ✅ `BatchReplyPage` 已补 `:key="acct-batch-${accountEpoch}"`（MainPage.vue），切账号会重挂、
  `rebuildGroups()` 重跑读新账号的 scoped 配置。
- ✅ `batch-reply-multi-config-v3`（`BatchReplyPage.vue` / `BatchReplyConfigPage.vue`）和
  `batch-reply-manual-ids-v1`（`BatchReplyPage.vue`）已包 scopedKey。
- ⚠️ **迁移不搬 batch 配置（踩坑后的决定）**：`migrateLegacyStorageOnceV2` 最初照搬一阶段
  「把旧全局配置复制给当前 active 账号」的做法，结果给一个从没配过 batch 的账号硬塞了
  别的账号的全局配置（脏数据）——因为 batch 配置升级前是真正多账号混用的一份，无法归属。
  已改为**只删旧全局 key、不复制**，各账号 batch 配置从空开始自己配。
  - 副作用：已经跑过旧版 V2 迁移的用户（`store-migrated-v2=1`）脏数据仍在，改代码不回滚。
    清理办法 = 在脏账号的 Batch Reply 配置页点「清空配置」（`resetAll` 删该账号 scoped 桶）。
- **踩坑记录**：第一版只把 v3 key 本身 scopedKey 化，漏了一个细节——两个文件读 v3 失败
  （即当前账号还没存过配置）时，都会**无账号区分地**回退读全局的 v1/v2 `LEGACY_KEYS`
  （pre-v3 老格式，本意是给还没升级到 v3 的老用户兜底）。结果任何新账号第一次挂载都会一起摔进
  这个全局兜底，读到同一份旧数据——表现为"切哪个账号都显示同样几个 app"。已删掉这两处的
  unscoped 兜底读取（`BatchReplyPage.vue::loadConfig` / `BatchReplyConfigPage.vue::onMounted`），
  `LEGACY_KEYS` 常量只保留给 `resetAll()` 清残留用。**代价**：如果真有用户还停留在纯 v1/v2
  格式（从未存过 v3），这版之后会读不到那份旧配置——存量数据早在 v3 上线时已经不再是"当前"
  格式，这个 tester-app 内部用户量很小，判断可接受。

## 7. 风险 / 注意

- **id 就绪时序**：靠 `App.vue` 的 `flush:'sync'` watch —— user 一变同步写 activeAccountId，早于
  `accountEpoch++` 触发的子页重挂。改动这段务必保持「先更新 user/id，后 epoch++」的顺序。
- **未登录**：`activeAccountId` 空时落 `_none` 桶；评论/Play 页登录后才用，无碍。迁移只在已登录时跑。
- **前后端 id 一致性**：前端不再自算 id，完全依赖后端 `UserInfo.id`。后端若新增返回 user 的出口，
  记得那条路径的 user 也已填 id（当前都经 AuthState，已覆盖）。
- **快照文件名**：后端 `snapshot_path` 会 sanitize，`${id}__${pkg}` 里 email 的 `@`/`.` 会转 `_`；
  同账号每次前缀一致 → 稳定命中同一文件。

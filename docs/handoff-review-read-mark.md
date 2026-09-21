# Handoff：评论「已读」标记（不回复也能从列表消失）

> 状态：**已实现**（2026-09-21）。
>
> 落地：新增 `utils/reviewReadMarks.ts`；`favoritesStorage` 的 `loadMapSafe/saveMapSafe` 加可选 `label`（默认「收藏数据」，已读传「已读标记」）；
> `ReviewPage.vue` 卡片「已读」按钮 + 筛选区「只看已读 (N)」/「撤销上一条已读」+ 摘要「已隐藏 N 条已读」；
> `BatchReplyPage.vue` 候选卡「已读」按钮 + 工具栏「撤销上一条已读」+ `visibleByPkg` 过滤。
> 按钮**不用 emoji**（用户要求）。`npm run build` 通过；存储层逻辑在 vite dev 页面里实测过
> （标记/持久化/撤销倒序/空表撤销/30 天清理均符合预期），**列表 UI 待真机窗口验证**。
>
> 决策点「批量页要不要也放已读按钮」→ 用户确认**放**（已读状态两页同步）。

## 一、需求（用户原话）

> 能不能多加一个按钮，已读。点击后从 play console 和批量页面消失，可以撤回，撤销上一条已读。
> 这个需求来源于，有一些四星的评论，不需要回复的，但是不回复的话，就会一直在这个页面不消失。

即：给「不打算回复但已经看过」的评论一个终结状态。现状只有「回复」能让评论离开
「无回复」视图，导致 4★ 好评长期占位。

## 二、已确认决策（2026-09-21）

| # | 决策点 | 结论 |
|---|---|---|
| 1 | 已读后怎么找回 | **撤销上一条已读**（顶部按钮，可连点连撤）**＋「已读」筛选视图**（切过去看全部已读，可逐条标回未读） |
| 2 | 隐藏范围 | **全部筛选下都隐藏**（无回复/已回复/回复后又更新/全部 都不显示），只有切到「已读」视图才看得到 |
| 3 | 用户改评/追评后 | **保持已读**（不因 `user_comment_ts` 变新而自动复活） |

自行决定（无需再确认，除非有异议）：

- 已读**只是本地视图状态**，不回写 Play Console（API 也没有这个概念）。
- 按**账号隔离**（`scopedKey`），与收藏同一套口径；跨 app 共用一张表（key 是 `review_id`，全局唯一）。
- 「上一条」= 已读表里 `at` 最大的那条，不另建撤销栈 → 天然跨会话有效、可连撤。

## 三、现状调研

| 事项 | 现状 | 影响 |
|---|---|---|
| 评论数据层 | `utils/reviewsStore.ts`：按 `账号__包名` 缓存共享响应式数组，两页持有**同一批对象引用**；磁盘快照由 `reviews.rs` 读写，后端定时线程也写同一份 | 已读**不进快照**（后端不认识这个字段，写进去会被下一次 upsert 冲掉/引发结构漂移）→ 走独立 localStorage 表 |
| Play Console 页筛选 | `ReviewPage.vue::filtered`（L431）两个分支：batch / single | 各插一行 early-return，旧条件不动 |
| 批量页筛选 | `BatchReplyPage.vue::visibleByPkg`（L279）→ `syncCandidates` 增量对齐候选卡 | 过滤里加一个条件即可，候选卡会自动移出（与「回复成功后消失」同机制） |
| 收藏实现 | `utils/reviewFavorites.ts` + `favoritesStorage.ts`（损坏隔离 / 配额报错 / 写失败不改 UI） | 已读直接复用这套安全读写，不重造 |
| 响应式 | 收藏的 `favIds` 是**每页各自**的 `ref<Set>`，靠 `activeOption` watch 重读 | 已读要驱动**两页的 computed**，所以放**模块级共享 ref**（像 reviewsStore 那样），两页自动联动 |

## 四、实现方案（加法为主，旧代码基本不动）

### 1）新增 `src/utils/reviewReadMarks.ts`

```ts
export interface ReadMark {
  at: number;        // 标记时间，撤销「上一条」按它取 max
  pkg: string; app: string;
  author: string; excerpt: string; // 只为撤销按钮/已读视图显示，不参与逻辑
}
export const readMarks: Ref<Record<string, ReadMark>>  // 模块级共享响应式
export const readMarksError: Ref<string>               // 写失败原因，页面用现有 banner 显示
export function isRead(reviewId: string): boolean
export function markRead(r: TaggedReview): boolean     // 写失败返回 false，调用方不改 UI
export function unmarkRead(reviewId: string): boolean
export function lastRead(): { id: string; mark: ReadMark } | null
export function undoLastRead(): boolean
export function reloadReadMarks(): void                // 切账号 / 页面重新激活时重读
```

存储：`scopedKey("review-read-v1")` → `Record<reviewId, ReadMark>`，读写走
`loadMapSafe` / `saveMapSafe`。

> 小改动：给 `favoritesStorage.ts` 的错误文案加一个**可选** `label` 参数（默认
> `"收藏数据"`），已读传 `"已读标记"`，否则会弹出「收藏数据读取失败」的错话。
> 默认参数 → 现有调用点零改动。

### 2）`ReviewPage.vue`

- 卡片头（截图红框位置，「🔍 分析」左侧）加按钮：
  - 普通视图：`✅ 已读` → `markRead(r)`，卡片立即消失
  - 已读视图：`↩ 标回未读` → `unmarkRead(r.review_id)`
- 「回复状态」行下新增一行「已读」：
  - `👁 只看已读 (N)` 切换（`readOnly` ref，不写进已保存的页面配置，切页即恢复默认）
  - `↩ 撤销上一条已读` 按钮，带最近一条的作者摘要作 title；无已读时禁用
- `filtered` 两个分支各在最前面插一行：
  ```ts
  if (readOnly.value ? !isRead(r.review_id) : isRead(r.review_id)) return false;
  ```
- 摘要行补一句「· 已隐藏 N 条已读」，避免用户以为评论丢了。
- `activeOption` watch 里顺带 `reloadReadMarks()`（与 `favIds` 同处，切账号也覆盖）。

### 3）`BatchReplyPage.vue`

- `visibleByPkg` 的 filter 首条加 `!isRead(r.review_id) &&`。
- 候选卡操作区加小按钮 `✅ 已读`（点完这条候选卡消失，不占生成/提交名额）。
- 页面顶部同样放一个 `↩ 撤销上一条已读`（与 Play Console 页共享同一张表）。

> **待确认（小）**：批量页要不要也放「已读」按钮？默认**放**——那页才是逐条过评论的主战场。
> 不想要的话就只在 Play Console 页放，批量页仅隐藏。

### 4）文档同步

`PROJECT_STRUCTURE.md`（新增 util）、`USER_GUIDE.md`（怎么用 + 已读不同步到 Console）、
`decisions.md`（为什么不进快照 / 为什么不自动复活）。

## 五、坑与边界

1. **不进后端快照**：`upsert()` 会用 API 返回覆盖字段，自定义字段活不过一次刷新。
2. **7 天窗口**：评论超出 API 窗口后会从内存数组消失，但已读表里记录仍在（无害的孤儿）。
   可在 `reloadReadMarks()` 时顺带清理 `at` 早于 30 天的条目，防止无限膨胀；
   清理阈值取 30 天（远大于 7 天窗口，撤销不会被误清）。
3. **已读 ≠ 已回复**：回复成功后不自动标已读（它本来就会从「无回复」消失）；
   反过来，标了已读的评论若从「已读」视图发起回复，回复流程照常可用。
4. **写失败**：`saveMapSafe` 返回 false 时**不改**内存 Set，用现有 `errorMsg` banner 显示原因
   （与收藏同策略，避免「UI 消失了但磁盘没记」）。
5. **定时 Telegram 通知**不看已读表（它只关心「新增」），已读不影响推送口径。

## 六、验证路径

1. Play Console 页：4★ 评论点「已读」→ 卡片消失；切「全部/已回复/回复后又更新」也不出现。
2. 切到批量回复页：同一条不在候选里；反向操作（批量页标已读 → Play Console 页也没了）。
3. 点「撤销上一条已读」→ 该条回到列表；连点两次撤回两条（顺序为标记时间倒序）。
4. 切「👁 只看已读」→ 看到全部已读，逐条「↩ 标回未读」可恢复。
5. 重启 app / 切账号再切回 → 已读状态仍在；切到另一个账号看不到这批标记。
6. `npm run build` 通过。

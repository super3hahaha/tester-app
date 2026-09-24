# Handoff：收藏评论 / 收藏邮件「标签」

> 状态：**已实现**（2026-09-24）。`npm run build` 通过；存储层在 vite dev 页面实测过
> （重名拦截/id 去重/OR+无标签筛选/删标签不回写/清空标签不取消收藏）。**四个页面的 UI 待真机窗口验证**（dev 页面卡登录）。

## 一、需求（用户原话）

> 给收藏评论和收藏邮件新增添加标签的功能。先自定义标签，然后可以给每条收藏的评论添加标签；
> 也可以在 Play Console 和邮件页中有一个添加标签按钮，点击后自动添加标签并收藏。
> 然后在收藏页，可以按照标签筛选出需要的评论和邮件。

## 二、已确认决策（2026-09-24）

| # | 决策点 | 结论 |
|---|---|---|
| 1 | 标签库 | 评论 + 邮件**共用一套**；存本地 localStorage `fav-tags-v1`，**全局不 scopedKey**；走 `favoritesStorage` 安全层（label="标签"）。只在本机，不跨设备/同事同步（与收藏本身一致）。**考虑过按账号隔离，用户放弃**：邮件收藏是全局的，标签库 scope 后切账号邮件上的标签会失联，要么记录里按账号分存（复杂），要么互相覆盖（丢数据），不值得 |
| 2 | 记录里存什么 | 收藏记录新增可选字段 `tags?: string[]`（存**标签 id**）。改名零成本；删标签**不回写**收藏记录，读时过滤掉不存在的 id |
| 3 | 评论页/邮件页入口 | 卡片 ☆ 旁加纯文字按钮「标签」→ 小浮层：标签复选框 + 底部内联新建。**勾任一标签即自动收藏**（未收藏时先 addFavorite 再写 tags） |
| 4 | 标签全去掉 | **不**自动取消收藏；取消收藏 → 标签随记录一起没（同备注） |
| 5 | 原页面显示 | 已收藏且有标签的卡片显示小色块标签 |
| 6 | 收藏页筛选 | 多选，**命中任意一个（OR）**；另有「无标签」选项。邮件收藏页：先按邮件源 tab，再在 tab 内按标签过滤（叠加，不替换） |
| 7 | 标签管理 | 两个收藏页顶部各一个「管理标签」入口（同一份数据）：增/改名/改色/删除（删除内联二次确认）。颜色新建时从固定色板轮流分配，可手改 |
| 8 | UI | 按钮纯文字，不用 emoji |

## 三、数据结构

```ts
// fav-tags-v1: Record<id, FavTag>
interface FavTag { id: string; name: string; color: string; createdAt: number }
// id: "t_" + Date.now().toString(36) + 随机后缀；name 去重（trim 后大小写不敏感）
```

`FavoriteReview` / `FavoriteMail` 各加 `tags?: string[]`，旧记录无此字段 = 无标签，无需迁移。

## 四、落地计划（加法，旧逻辑不动）

| 文件 | 改动 |
|---|---|
| `utils/favTags.ts`（新） | 模块级共享响应式 `tags` ref（多页同步，参照 reviewReadMarks）；`addTag/renameTag/setTagColor/removeTag/reloadTags`，写失败返回 false + `favTagsError` ref |
| `utils/reviewFavorites.ts` / `mailFavorites.ts` | 各加 `setFavoriteTags(key, ids): boolean`（唯一入口，同 setFavoriteNote） |
| `components/TagPicker.vue`（新） | 浮层：复选框 + 内联新建；emit 选中 id 列表。评论页/邮件页/两个收藏页共用 |
| `components/TagManager.vue`（新） | 管理弹窗 |
| `ReviewPage.vue` / `GmailPage.vue` | 卡片「标签」按钮 + 标签色块显示；勾选时未收藏先收藏（失败则中止，不动视图） |
| `FavoriteReviewsPage.vue` / `FavoriteMailsPage.vue` | 卡片标签编辑 + 顶部筛选条 + 「管理标签」；筛选在现有列表 computed 上 early-return 叠加 |

## 五、注意

- 写失败不动视图（与收藏/已读同策略）：`setFavoriteTags` 返回 false 时浮层不关、勾选回滚。
- 评论标签**不进 reviewsStore 快照**（upsert 会冲掉自定义字段），只存在收藏记录里。
- 完成后同步 `PROJECT_STRUCTURE.md`（新文件 + 4 个页面描述）。

## 六、二期：标签按 app 限定范围（2026-09-24，已实现）

用户原话：「现在标签不和 app 关联，我负责多个 app 不太方便」。

| # | 决策点 | 结论 |
|---|---|---|
| 1 | 范围模型 | 标签加 `apps?: string[]`（包名）；空 = **通用**。旧标签无此字段 = 通用，不迁移 |
| 2 | 评论页/收藏评论卡片的「标签」浮层 | 只列「通用 + 该评论 app」；已勾但已不在范围内的标签单独列「其它」组，方便取消 |
| 3 | 卡片上新建 | 默认挂当前 app，浮层底部勾「设为通用」可改 |
| 4 | 收藏评论筛选条 | 「全部」tab → 全部标签按「通用 / 各 app」分组；某 app tab → 只列通用 + 该 app；切 tab 后不在范围的已选标签自动剔除 |
| 5 | 重名 | 仍**全局唯一**（跨 app 共用的设通用） |
| 6 | app 清单来源 | GP 模板管理「管理关联」的 package_map（`get_package_map`），是唯一全局的 app 清单 |
| 7 | 邮件怎么知道属于哪个 app | 邮件源加「关联 app」（`appPkg`，GmailPage 设置区「模板」下一行）。**没设过的按邮件模板产品名 ≈ GP 产品名（不区分大小写）自动推断一次**，推断不出记 ""，之后手动改。收藏邮件不存快照，按 `_sourceKey` 实时读邮件源的 appPkg（改关联立刻生效；源删了 = 不限 app） |
| 8 | 邮件模板页「管理关联」 | **隐藏**：package_map 只有 GP 一份，邮件页打开的也是它，下拉框却列邮件产品 → 显示空白、误改会把小写邮件产品名写进 GP 映射 |

落地：新增 `utils/appRegistry.ts`（pkgApps/loadPkgApps/appLabel/appPkgForProduct/mailSourceApps/reloadMailSourceApps）；
`favTags.ts` 加 addTag(name, apps)/setTagApps/isGeneralTag/tagsForApp/groupTags；TagPicker/TagFilterBar 加 `app` prop；
TagManager 每行加范围按钮 + 展开面板（通用单选 / app 多选）。UI 在 dev 页面挂组件实测过，**四个真实页面待真机验证**。

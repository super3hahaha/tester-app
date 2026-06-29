# Handoff：知识库（自由资料库 + 多对多产品关联）+ Generate 偏好勾选

> 状态：**方案已定，未动手实现**。本文件是实现前的交接说明，写给冷启动接手者。
> 基线 commit：`5e8e6d9`（master）。

## 为什么做这个（动机，别丢）

几个同事都用 `test-case-generator` skill 生成用例，但**每个人有自己的偏好，且每人同时负责多个产品**，反馈高度个性化——靠维护者改 skill 满足不了。真实反馈样例（XPlayer IAP、Recorder 贴纸、Player 字幕、InSlide 音频提取等）暴露出两类偏好：

- **产品/需求特定的领域知识**：如「加油包必须会员才能买」「要覆盖旧 SKU 升级用户的订阅取消/试用资格场景」「云端贴纸要测断网/清数据」。
- **跨产品的个人通用习惯**：如「用例顺序按模块习惯排」「UI 描述别太冗余」「弹窗必覆盖点空白处/物理返回键」「免费/会员权益别重复生成」。

结论性认知：**「人」这个维度不在 app 里建模**——每个同事各自装 app、各自的 `~/.tester-app/`，天然就是隔离的个人偏好集。

## 产品形态：自由资料库 + 多对多产品关联

知识配置是**一个可自由编辑的资料库**，结构两层、不套娃；资料与产品是**多对多关联**：

```
一级：知识配置
└── 二级菜单（动态，用户自建/改/删）：产品分类 + 「通用」+ ＋新建产品
      点某产品 → 页面：
        顶部 tab 行 = 关联到该产品的资料（md），可新建/重命名/删除 tab
        每个 tab(md) 带一个「管理关联」按钮 → 弹窗：勾选这份资料关联哪些产品（多选）
        下方：选中 tab 的 Markdown 编辑器 + 「插入骨架」预设
```

- **资料 ↔ 产品多对多**：一份资料可关联多个产品（出现在多个产品 tab 下，改一处全同步）；一个产品可关联多份资料。等价于评论侧「管理包名关联」那套，只是关联做成多选。
- **「通用」= 不关联任何产品**的资料（对所有生成都可用），二级栏一个虚拟视图项。
- **关联入口**：在**每份 md 上**点「管理关联」按钮 → 弹窗只管这一份资料的产品关联（勾选多产品；一个都不勾 = 通用；弹窗内可「＋新建产品」）。
- **不套娃**：始终两层（产品 → 资料）。一份资料出现在多个产品下是"关联展示"，不是新层级。

这些 md 被用例 Generate 消费：generate 时按产品分组**手动勾选**要用哪几份，作为偏好文件传给 skill。

## 本期范围（关键）

- **本期只做用例侧**：资料库管理（产品/资料 CRUD + 编辑 + 管理关联）+ Generate 勾选偏好传 skill。
- **评论/邮件维持现状**：继续用现有 `review-analysis`（`analysis.rs`，绑模板产品、注入 AI 回复），**本期不碰评论侧代码**。
- **未来演进（记下别忘）**：评论/邮件以后接这个库（按产品自动注入符合关联的资料）；届时评论侧从「绑模板产品」改成「手动添加关联」，与本库模型一致。每份资料的 `scenes` 字段（本期固定 `["testcase"]`）就是为此留位（未来加 `review`）。

## 已定决策（用户逐条拍板，勿再推翻）

- **一级目录**：知识配置做独立一级入口（图标栏放 Test Case 与 Settings 之间）。
- **二级菜单 = 动态产品分类**：用户自由新建/重命名/删除，不依赖模板产品列表；外加内置「通用」虚拟项。
- **页内 = 动态 tab**：每个 tab 一份 md，自由新建/重命名/删除。
- **资料 ↔ 产品多对多关联**（用户用「管理包名关联」截图明确要的）：关联通过**每份 md 的「管理关联」按钮 + 单份弹窗**（产品多选）管理。
- **「通用」= 未关联任何产品**的资料；虚拟视图、不可删。
- **删除 vs 取消关联**：在管理关联弹窗里**取消勾选某产品** = 解除该资料与该产品的关联（资料还在库）；**删除 tab** = 删这份资料本身。
- **删非空产品 = 级联解除关联（不删资料内容）**：删产品时从所有 `docs[].productIds` 摘除该 id；只关联该产品的资料随之 `productIds` 变空 → 自动归入「通用」，绝不删资料文件。（多对多下严禁"连资料一起删"——会误伤还关联其它产品的资料。）
- **预设引导**：提供骨架模板 + 空状态引导（见「预设与引导」）。
- **偏好传给 skill = 显式传文件路径**（与现有 CSV/PDF 同构），**不**塞进 prompt。
- **彻底废弃 skill 的 `references/` 偏好机制**：skill 不再按产品名自动加载，只认 prompt 显式传入的路径。

## 数据存储

独立目录（中性命名，未来评论可共用；**不要**塞进评论的 `review-analysis/`）。资料**扁平存**（不绑产品目录，因为多对多）：

```
~/.tester-app/knowledge/
├── index.json
└── docs/
    └── <docId>.md      # 一份资料/一个 tab
```

`index.json`：

```json
{
  "products": [
    { "id": "xplayer", "name": "XPlayer" },
    { "id": "recorder1", "name": "Recorder1" }
  ],
  "docs": [
    { "id": "d1", "name": "IAP测试通则", "productIds": ["xplayer","player"], "scenes": ["testcase"] },
    { "id": "d2", "name": "录屏通用约定", "productIds": [], "scenes": ["testcase"] }
  ]
}
```

- `products`：用户自建产品。数组顺序即显示顺序（新建追加末尾，支持拖拽排序）。「通用」不入此表（= `productIds` 为空的资料的虚拟视图）。
- `docs[].productIds`：关联的产品 id（多对多，空 = 通用）。
- `docs[].scenes`：本期固定 `["testcase"]`，UI 不暴露，留给未来评论（`review`）。
- 文件名用 docId（`docs/<docId>.md`），name 可随意改不影响文件。
- 读不到/损坏回退空库，别 panic（参考 `prompt_config.rs:179-185` 的 `unwrap_or_default`）。

对照参考：评论知识存储在 `src-tauri/src/analysis.rs:19-86`（`review-analysis/{slug}.md`，绑模板产品、一产品一份）。本库**不照搬**——资料扁平存、文件名用 id、产品自管、多对多关联。

### 现有偏好迁移（一次性）

`~/.claude/skills/test-case-generator/references/` 下当前只有 `XRecorder/`：

| 文件 | 处理 |
|---|---|
| `XRecorder/preferences.md`（115 行） | **迁入库**：建产品「XRecorder」，新建一份资料（名如「XRecorder 通用约定」，`productIds:["xrecorder"]`，`scenes:["testcase"]`），内容=原文。迁完从 skill 删除。 |
| `XRecorder/changes_example.json` | skill 内部 changes.json 格式示例，**不是用户偏好，留着别动**。 |

**迁移时机：等知识库功能建好后**再做一次性迁移（写脚本或在知识库 UI 里手动建产品+资料粘贴均可）。**不做首启自动导入**。迁完从 skill 删除 `preferences.md`。

## 预设与引导

用户要求「出一些预设，教用户如何建立自己的知识库」。落地：

- **空状态引导**：库为空时，页面给一段说明（这是什么、怎么用、骨架怎么填）+ 引导按钮（「新建产品」「插入骨架」）。
- **「插入骨架」**：新建 tab 后按用途插下面两种骨架之一（编辑器内可改）。
- （可选）首启时预置一个示例产品 + 一份填了骨架的示例资料。

### ① 通用偏好骨架（不关联产品的资料）

```markdown
# 通用用例偏好（适用于我负责的所有产品）

## 写作风格
- 描述列：用「查看/检查/点击/输入」等操作动词，不用「是否为」「验证是否」判断句。
- 预期列：直接写结论状态，不重复动作，不用「正确显示」等模糊表述。
- UI 用例：不堆需求原文，去掉冗余的需求文字描述，只留可验证的界面要点。

## 用例顺序
- 按模块测试习惯排序，不要照需求文档的叙述顺序排。
- <写下习惯的模块顺序，如：UI → 核心交互 → 异常/边界 → 跨功能交叉>

## 必测通则（每类功能都要覆盖）
- 弹窗：除点弹窗内按钮外，必须覆盖「点空白处关闭」「点物理返回键关闭」。
- 升级测试：涉及新功能/新数据结构时，覆盖从旧版本升级上来的场景。
- 交叉测试：新功能与其他 tab / 已有功能的交叉影响。
- 断网 / 清数据：涉及云端或缓存的功能，覆盖断网、清除数据后的表现。

## 去重
- 需求多处重复的权益/规则（如免费 vs 会员权益），合并成一条，别每处各生成一条。

## 红线
- 不编造需求未提供的信息：版本号、价格、数值、业务术语不确定就标出来问，不臆测。
```

### ② 产品偏好骨架（关联具体产品的资料）

```markdown
# <产品名> 用例偏好

## 定位
<一句话：这个产品/这次需求做什么，核心功能与关键概念>

## 关键业务规则（容易理解错的，先写清楚）
- <如：加油包必须是会员才能购买>
- <如：会员到期但加油包未到期时，加油包是否仍可用——写明实际逻辑>
- <如：年订说明文案里替换的是「X 价格」，不是「Y 价格」>

## 必测场景清单（本产品/需求易遗漏的点）
- <如：购买项按不同国家显示当地货币价格>
- <如：已购旧 SKU 的升级用户——订阅取消/未取消、三天试用资格、从 GP 重新订阅旧 SKU>
- <如：字幕支持的语言检测>

## 需关联测试的点（别孤立测）
- <如：字幕样式每个选项设置后，关联检查字幕实际表现>
- <如：画质增强 + 小窗播放 / 后台播放 组合下的表现>
- <如：倍速 与 片段轨道 的关联逻辑>

## 模块命名 / 分群约定
<本产品用例的模块怎么命名、怎么分群，与已有用例保持一致>
```

## 改动清单（文件级）

### 后端 Rust

| 文件 | 改动 |
|---|---|
| 新建 `src-tauri/src/knowledge_base.rs` | **产品 CRUD**：`kb_list_products` / `kb_create_product(name)` / `kb_rename_product(id,name)` / `kb_delete_product(id)`（删产品时从所有 `docs[].productIds` 摘掉该 id，资料本身不删）。**资料 CRUD**：`kb_list_docs()`（返回全部资料含 `productIds`/`scenes`，前端按产品过滤）/ `kb_read_doc(id)` / `kb_save_doc(id,content)` / `kb_create_doc(name, productIds)` / `kb_rename_doc(id,name)` / `kb_delete_doc(id)`。**关联**：`kb_set_doc_products(docId, productIds)`（覆盖式，管理关联弹窗保存时调用）。**排序（可选）**：`kb_reorder_products` / `kb_reorder_docs`。**给 Generate 用**：`kb_resolve_doc_paths(ids) -> Vec<String>`（docId → `docs/<docId>.md` 绝对路径，跳过不存在的）。`data_dir()` 复用 `~/.tester-app`（analysis.rs:19）。index.json 损坏回退空库。 |
| `src-tauri/src/lib.rs` | `mod knowledge_base;` + 在 `invoke_handler` 注册全部新命令（注册区参考 lib.rs:36-107）。 |
| `src-tauri/src/claude.rs` `run_claude_task`（**claude.rs:497-560**） | 1) 签名加 `preference_paths: Vec<String>`（前端传 `preferencePaths`）。2) 每个偏好文件父目录加进 `dirs` → `--add-dir`（同款逻辑 claude.rs:524-538）。3) prompt 追加（拼接点 claude.rs:540-557）：`Preference files (apply these conventions):\n- <path1>\n- <path2>\n` |

### 前端 Vue

| 文件 | 改动 |
|---|---|
| `src/pages/MainPage.vue` | **① 加一级项** `{ id:"knowledge", label:"知识配置", icon, dynamic:true, children:[] }`，放 testcase 与 settings 间（图标参考 MainPage.vue:57）。**② 二级栏支持动态项**（现状遍历静态 `children`，:159-175）：对 `dynamic` workspace，二级项 = 固定「通用」虚拟项 + reactive 产品列表 ref（`kbProducts`，`kb_list_products` 异步填充）+ 末尾「＋新建产品」伪项。点项 → `activeOption = "kb-view:" + (id|"common")`；点伪项 → 弹框 `kb_create_product`。切到本 workspace（selectWorkspace，:108-113）触发拉取，默认选「通用」。**③ 内容区** 加 `<KnowledgeBasePage v-show="activeOption.startsWith('kb-view:')" :view-id="activeOption.slice('kb-view:'.length)" :active-option="activeOption" @products-changed="reloadKbProducts" />`。 |
| 新建 `src/pages/KnowledgeBasePage.vue` | 接收 `viewId`（产品 id 或 `"common"`）。布局：**顶部 tab 行 = 该视图下的资料**（`kb_list_docs` 拉全部，前端过滤：`common` 取 `productIds` 空的、产品视图取含该 id 的；可新建/重命名/删除 tab）+ **下方 Markdown 编辑器**。**每个 tab 带「管理关联」按钮 → 打开 ManageAssocModal**（见下）。新建 tab 时默认关联当前产品（`common` 视图新建则不关联）。产品本身的重命名/删除放页面头部（改完 `emit('products-changed')` 刷新二级栏；删产品后回退「通用」）。空视图显示引导。编辑器脏值/flash/保存禁用态/固定高度内滚——抄 `KnowledgeConfigPage.vue:36-104, 262-306`。「插入骨架」给两种骨架（见「预设与引导」）。 |
| 管理关联弹窗（`KnowledgeBasePage` 内组件或单独 `ManageAssocModal.vue`） | 针对**单份资料**：列出所有产品 checkbox（勾选=关联，预填该资料当前 `productIds`），底部「＋新建产品」+「取消/保存」。保存调 `kb_set_doc_products(docId, 勾选的 productIds)`；一个都不勾 = 通用。保存后刷新 tab 列表与二级栏（关联变化会让资料在产品视图间增减）。设计稿见对话。 |
| `src/pages/GeneratePage.vue` | CSV、Slides 区下加「偏好文件」区：`kb_list_docs`+`kb_list_products` 拉数据 → **按关联产品分组**（通用组=未关联 + 各产品组）渲染 checkbox 树，默认全不勾、可跨组多选。`handleGenerate` 把勾选 docId → `kb_resolve_doc_paths` 转路径，传 `invoke("run_claude_task", { ..., preferencePaths })`。现状调用：`run_claude_task({ csvPath, pptxPaths, pageSelections, model, extraInfo })`。 |

### skill

| 文件 | 改动 |
|---|---|
| `~/.claude/skills/test-case-generator/SKILL.md` 第零步（:17-36） | **重写**：删掉「按产品名自动读 `references/<产品>/preferences.md`」整套（含「已有偏好文件」表、「新增产品」提示）；改为「**偏好由 prompt 显式传入，若给出 `Preference files` 路径则逐个 Read 并应用其约定**」。同时删除 `references/XRecorder/preferences.md`（已迁入 app）。`changes_example.json` 保留。 |

## 必须避开的坑

- **v-show 常驻挂载**（见 `docs/gotchas.md` 第一条）：`MainPage` 子页用 `v-show`，`onMounted` 只跑一次。
  - `KnowledgeBasePage` 要 `watch(() => props.viewId)`：二级栏切视图时重新过滤/加载 tab 列表（参考 `KnowledgeConfigPage.vue:106-114`）。切 tab/视图时若当前 md 有未保存改动要提示（参考其 dirty 确认）。
  - `GeneratePage` 偏好区同理——用户可能先去知识配置加内容再切回 Generate，必须能刷新（确认是否已 watch activeOption；没有就加，或点开偏好区/点 Generate 前重拉）。
- **多对多一致性**：同一资料出现在多个产品视图，编辑的是同一份 `docs/<id>.md`，注意 UI 缓存别让某视图显示旧内容；管理关联弹窗保存后必须刷新 tab 列表 + 二级栏（资料会在产品视图间出现/消失）；删产品要从所有 `docs[].productIds` 摘除该 id。
- **动态二级栏**：`kbProducts` 异步填充，首次切入会先空再填，别在空列表崩；activeOption 默认/回退到 `"kb-view:common"`。产品增删后必须 `reloadKbProducts`。「通用」虚拟项恒在首位、不可删。
- **tab 内编辑器复用**：切 tab = 切 md，脏值判断按当前 docId 走，别把 A tab 改动串到 B tab。
- **Generate resolve**：勾选的资料若已被删，`kb_resolve_doc_paths` 静默跳过。

## 验证路径（实现后自检）

1. 资料库：新建产品（进二级栏）、进去新建 2 个 tab、写内容保存、重命名 tab；点某 tab「管理关联」→ **勾选关联到 2 个产品** → 两个产品视图都出现该 tab；在弹窗里**取消勾选其一** → 该产品视图里消失、资料还在；删除 tab → 各视图都消失。一个都不勾 → 出现在「通用」。删产品（其关联自动摘除）、拖拽排序；空视图显示引导；重启后数据都在。
2. Generate：不勾→同现状；勾跨产品的几份 → `run_claude_task` 收到 `preferencePaths`，prompt 出现 `Preference files:` 段，`--add-dir` 含偏好父目录。
3. skill 实际 Read 到偏好并应用（跑一次真实生成看日志）。
4. 迁移：`references/XRecorder/preferences.md` 已删，库里「XRecorder」产品下资料内容与原 115 行一致；不勾时 skill 不再自动加载 references。

## v2 增强（非本期）：反馈 → 偏好半自动起草（轻量方案）

> **非本期**。等知识库基础功能（产品/资料 CRUD + 管理关联 + Generate 勾选）落地后再做。这里只记方向。

**价值**：把每次用例生成后的反馈（人脑已提炼的"踩坑结论"）半自动沉淀进偏好库，越用越准。比"导历史用例让 AI 归纳"靠谱——AI 只做改写/分类/去重，不负责发现规律或补领域知识。

**AI 干 4 件事：**
1. **反写**：踩坑 → 正向约定（"加油包理解错(会员才能买)" → "加油包必须会员才能购买"）。
2. **分类**：产品特定 vs 跨产品通用；产品特定再归骨架分区（关键业务规则 / 必测场景 / 需关联测试）。
3. **归并去重**：同规则合并，同产品多次反馈累积进一份。
4. **不确定留空**：反馈指出错误但没给正确答案的，用 `<待补>` 占位，**禁止编造**（半自动的人工关核心）。

**精简示例**：

```
输入反馈：XPlayer / AI加油包理解错误(会员才能买)，没考虑加油包未到期但会员已到期；
          购买项没考虑不同国家货币；年订文案替换价格理解错了
↓
输出（填进产品骨架②）：
## 关键业务规则
- AI 加油包：必须会员才能购买。
- 加油包与会员有效期独立：覆盖「加油包未到期但会员已到期」。
- ⚠️ 年订文案替换的价格 = <待补：反馈只说理解错、未给正确值>
## 必测场景清单
- 购买项按不同国家显示当地货币价格。
```

**轻量实现（推荐先做这个，不必新建 skill）：**
- 知识库页加「AI 起草 / 合并」按钮 → 走 `claude --print` 直出 md → 填进编辑器当**草稿** → 用户精修后保存。复用 `reply.rs::generate_single_reply` 的 `--print` 直出模式（claude.rs 里那套）。
- prompt 组成：指令（上面 4 件事 + 禁编造）+ 输入（反馈文本 +（增量时）该产品**已有偏好 md** + 骨架模板）+ 输出（填好骨架的 md，标注新增/改动）。
- **增量合并是精髓**：第二次起喂「现有偏好 md + 新反馈」→ AI merge 并**标出新增**（不覆盖旧内容）→ 人 review。每次生成完的反馈都能顺手累积。

**人工关（不可省）**：AI 产出永远是**草稿**；`<待补>` 空格、跨需求取舍、业务规则正确性必须人确认后才入库。用得频繁了再升级成独立 skill（参考 `review-reply` 的 app 调 skill 批量模式）。

## 待办（实现时同步更新的文档）

- `docs/PROJECT_STRUCTURE.md`：加 `knowledge_base.rs`、`KnowledgeBasePage.vue`、`~/.tester-app/knowledge/`。
- `docs/USER_GUIDE.md`：第 6 节补知识库（产品/通用/资料/管理关联/预设）用法 + Generate 勾选偏好。
- `docs/decisions.md`：记一条「知识库=自由资料库 + 资料↔产品多对多关联（每份 md 一个管理关联弹窗）；本期只做用例消费、评论维现状；偏好走显式传路径、不走 skill references」。
- 实现完成后**删除本 handoff 文件**。

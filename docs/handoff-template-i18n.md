# Handoff：模板多语言预翻译（review 模板 i18n 改造）

> 跨对话交接文档。**每完成一条就把状态从 ⬜ 改成 ✅**（进行中用 🔄）。
> 写给未来接手的 Claude：先读「背景」「设计决策」，再看「分步计划」当前进度。

状态图例：⬜ 待办 · 🔄 进行中 · ✅ 已完成

---

## 背景与目标

现状：review-reply skill 每次命中模板后都**实时翻译**到回复语言，重复、慢、费钱。
目标：把翻译从**运行时**挪到**一次性预生成**——每条模板预存各语言译文到本地，skill 命中后
**优先吃预存译文**（零成本零延迟），只有目标语言没预存时才实时翻译兜底。
并新增「🌐 补全多语言」窗口：选语言 → 调 Claude 批量翻译 → 写回本地。

---

## 设计决策（已和用户敲定）

### 数据结构（`src-tauri/src/templates.rs` 的 `Template`）
源仍是 `text` + `lang`（`en` 为源语言权威源），新增两字段：
```rust
#[serde(default)]
translations: BTreeMap<String, String>,  // 语言码 → 译文，不含源语言
#[serde(default)]
src_hash: String,                        // 翻译时 text 的指纹；text 变了→译文过期(stale)
```
- serde default → 存量 302 条零成本兼容。
- `update_template` 改 `text` 时重算 `src_hash`（译文不删，靠 hash 不符体现「过期」，UI 提示重译）。
- 按语言手工编辑译文**不**改 `src_hash`（源没变）。

### 语言清单（translations 的 key，用 app 原生码）
`en` 是源不进 translations。**默认勾选 22 个**（前端 `DEFAULT_CODES`）：
```
ar cs de es fa fr in it ja ko ms nl pl pt ro ru th tr uk vi zh-rCN zh-rTW
```
另有 **15 个后补语言，默认不勾选**：
```
bn da el fi hi hu kn-rIN ml-rIN mr pa-rIN sk sv ta-rIN te-rIN ur-rIN
```
（前端 `LANGS` = 全 37 项；`DEFAULT_CODES` = 默认 22；「全选」按钮选全部 37。后端通过 `langs` 参数接收，不硬编码。）

### 语言码归一（skill 查 translations 时）
回复语言（ISO）→ 模板语言码：`zh-CN`/`zh-Hans`→`zh-rCN`、`zh-TW`/`zh-Hant`→`zh-rTW`、`id`→`in`，
其余取主子标签匹配。命中预存译文直接用，否则实时翻译兜底。规则写进 SKILL.md。

### 翻译场景（一套 UI：勾语言子集 + 开关「覆盖已有 / 只补缺失」）
| 场景 | 触发 | 行为 |
|---|---|---|
| 首次铺底 | 「🌐 补全多语言」弹窗 | 当前产品全部模板 × 选中语言，全部翻译 |
| 新增语言 | 弹窗 + 勾新语言 + 只补缺失 | 只翻缺失语言、**追加**不覆盖 |
| 单条重译 | 列表每条「重译」按钮 | 该条已有语言**覆盖**重译；**前端把语言按 `GROUP=8` 分组多次调用**，卡片显示「翻译中 N/总」进度 |

### 通用产品（跨 app 复用，省重复翻译）
- 模板管理「+ 新建产品」→ 输入显示名 **「通用」**（id 前缀固定 `common`，见 `templates.rs::product_prefix`），存跨 app 通用回复，**翻一次各 app 复用**。
- review-reply 匹配：**先通用、命中即止；通用没对症再看专属**，对**所有 app** 生效（含 `product=null` 的 ringwall/xplayer，它们不再一律 unmatched）。SKILL.md 已改并发版。

### 翻译执行（最终：轻量直出 + haiku，**不走 skill**）
> ⚠️ 最初做成 `template-translate` skill（agent + `--add-dir` 模板目录 + Bash 写文件自检），**实测 20 条烧 30% 的 5h 额度**——agent 因 add-dir 自己读了 122KB templates.json、自检失败重写整份译文。**已废弃**，改为轻量直出：
- `translate.rs` 直接 `claude --print`：**不 `--add-dir`、prompt 只内联这批模板、要求「不用工具/不读写文件/直接输出 JSON」、从 stdout 解析**，不写文件不自检。
- 模型 **haiku**（前端 `TRANSLATE_MODEL=claude-haiku-4-5`）；批 **`CHUNK=1`**（一条 × 二十多语言已是一次大输出，再多单次太大易错）；每批立刻 `apply_translations` 写回（中断只丢一条）。
- 实测 6 条 × 22 语言 ≈ **$0.2**（还吃到 prompt 缓存），比原 skill 省一个数量级。
- `template-translate` skill（super3hahaha v0.0.2）**已退役**，`skill_sync` 注册可留可删（留着只多同步一个不用的 skill）。
- 反思见 memory `plan-cost-first-and-spike`：定方案先按成本打分、先小规模真跑再铺开。

---

## 关键代码位置（接手前先看）

- 模板数据/CRUD：`src-tauri/src/templates.rs`（数据在 `~/.tester-app/templates/`，写出口 `write_templates_and_index` 同时重建 index.json）
- skill 调用封装（可整段复刻给 translate）：`src-tauri/src/reply.rs`（CLI spawn + stream-json + 取消 + 进度事件）
- skill 分发：`src-tauri/src/skill_sync.rs`（`SKILLS` 常量，owner=super3hahaha）
- 命令注册：`src-tauri/src/lib.rs`
- 前端模板页：`src/pages/TemplateManagerPage.vue`
- review-reply skill：`~/.claude/skills/review-reply/SKILL.md`（第 5 步是翻译逻辑）

---

## 分步计划（进度）

> ⚠️ 下面是**原计划的历史记录**。翻译执行后来重构（轻量直出 + haiku + `CHUNK=1`，**不再走 template-translate skill**）、加了「通用」产品、补了 15 个后补语言、单条重译改按语言分组、弹窗加进度条 + 完成后「好的」按钮。**最终状态以上方「设计决策」段为准**，阶段里 `CHUNK=20`/`TRANSLATE_MODEL=sonnet`/「读 result.json」等已过时。

### 阶段 0 · 前置确认 ✅
- [x] `gh auth status`：登录账户 `super3hahaha`（含 repo+workflow scope），能 push。
- [x] 仓库名定为 `super3hahaha/template-translate`。
- 注：app sync 的 `download_zipball` 不带 token → **仓库必须 public**。

### 阶段 1 · 新 skill 内容 ✅
- [x] 源仓库放 `~/Projects/template-translate/`（与 review-reply 等同级），运行时副本在 `~/.claude/skills/template-translate/`。
- [x] 写了 `SKILL.md` + `README.md` + `.gitignore` + `.github/workflows/auto-release.yml`。
- skill 输入设计：每条模板自带 `target_langs`（后端精确控制翻什么，覆盖/只补缺失零浪费）；输出 `{results:{id:{lang:text}}}` 写到 `<stem>.result.json`。

### 阶段 2 · 建库 + 首发 ✅
- [x] 建 public 库 https://github.com/super3hahaha/template-translate ，push main。
- [x] auto-release 自动出 `v0.0.1`（首个 release）。

### 阶段 3 · 注册分发 ✅
- [x] `skill_sync.rs` 的 `SKILLS` 加 `template-translate`（owner=super3hahaha）。下次 app「检查更新」可拉 v0.0.1。

### 阶段 4 · 后端数据结构 + 命令 ✅（cargo check 通过）
- [x] `Template` 加 `translations: BTreeMap<String,String>` + `src_hash`（templates.rs）。
- [x] stale 判定：`!translations.is_empty() && src_hash != text_hash(text)`；`list_templates` 改返回 `TemplateView`（flatten + `stale`）。`update_template` 不动 src_hash（text 变了自然 stale）。
- [x] helper：`text_hash` / `is_source_lang`（zh-rCN↔zh-CN 等判同源）/ `load_templates_for` / `apply_translations`（合并写回 + 刷 src_hash）。
- [x] `set_template_translation(product,id,lang,text)`：选源语言改 text+刷 hash，否则改 translations。
- [x] **新建 `src-tauri/src/translate.rs`**：`translate_templates(product, ids:Option<Vec>, langs, overwrite, channel, model)` + `stop_translate`。每条按 (覆盖/补缺失 + 排除同源码) 算 target_langs；**分批 CHUNK=20**，每批 spawn `claude /template-translate`、读 `<stem>.result.json`、`apply_translations` 增量写盘；`translate-log` 进度；复刻 reply.rs 取消。
- [x] `lib.rs` 注册 3 命令 + `TranslateState`。

### 阶段 5 · 前端 `TemplateManagerPage.vue` ✅（vue-tsc 通过）
- [x] 「🌐 补全多语言」弹窗：22 语言勾选网格（记 localStorage `tpl-translate-langs`）+ 单选「只补缺失/覆盖重译」+ 进度日志（listen `translate-log`）+ 停止。
- [x] 每条「重译」按钮（stale 的橙色高亮），调 translate_templates(ids=[该条], 全语言, overwrite=true)。
- [x] 编辑行语言选择器改成「源 + 已译语言」下拉；textarea 用 curText/setCurText 切换显示源文或译文；保存按 curLang 走 update_template / set_template_translation。
- [x] 每条覆盖度徽标 `🌐 N`，stale 显示「· 源已改」橙色。
- [x] xlsx 导入提示「译文已清空，记得补全多语言」。
- 常量：LANGS（22）+ TRANSLATE_MODEL=claude-sonnet-4-6。

### 阶段 6 · 改 review-reply 吃预存译文 ✅
- [x] `~/Projects/review-reply/SKILL.md` 第 5 步：取全文 python 命令改成输出 `{lang,text,translations}`；回复语言归一成模板码后，命中预存译文直接用、未命中才实时翻译并记 warnings。
- [x] push → auto-release 出 `v0.2.5`；运行时副本 `~/.claude/skills/review-reply/SKILL.md` 已同步（本地联调即时生效）。

### 阶段 7 · 文档 + 边界 ✅
- [x] `decisions.md` 加「模板多语言预翻译」整节。
- [x] `gotchas.md` 加两条：语言码归一（app 码 vs ISO）、分批每批写盘。
- [x] xlsx 导入覆盖时 `translations` 已清空（templates.rs），UI 提示需补全。

---

## 验证锚点
- 阶段 4 后：单条重译全链路可跑通。
- 阶段 5 后：UI 完整。
- 阶段 6 后：review-reply 实际省掉翻译成本（命中预存直接用）。

## 当前进度小结
> 接手时更新这一段：上次做到哪、下一步、有没有坑。

**2026-06-18：功能完成并实跑验证省钱。** 后端过 `cargo check`、前端过 `vue-tsc`。

最终实现（与原计划的差异都已并入上方「设计决策」段）：
- **翻译走轻量直出 + haiku**，不走 skill；`translate.rs`：不 add-dir、prompt 内联本批、禁工具、stdout 解析、`CHUNK=1`、每批写回、可取消、emit `translate-log`(含用量) + `translate-progress`(进度条)。**350 字符硬校验**：超长自动压缩重试一次，仍超标红警告（见 gotchas）。
- **前端**：补全弹窗(37 语言勾选/默认 22、覆盖 vs 只补缺失、进度条、用量日志、停止、完成后「好的」按钮)；每条「重译」按钮(stale 橙色高亮、按语言分组显示「翻译中 N/总」)；编辑行「源 + 已译语言」下拉可查看/编辑译文；覆盖度徽标 `🌐 N`；翻译进行中所有编辑控件置灰；「+ 新建产品」入口。
- **review-reply**（已发版 `v0.2.5`+，运行时副本同步）：命中模板优先吃 `translations`、语言码归一；新增「通用」产品**先通用后专属**匹配、对所有 app 生效。
- **实测成本**：6 条 × 22 语言 haiku ≈ $0.2（吃 prompt 缓存）。

待办 / 注意：
1. 后端改过的话要 `npm run tauri dev` 重启（Rust 不走 HMR）。
2. haiku 译文质量需抽查；不够就把前端 `TRANSLATE_MODEL` 换回 sonnet（一行）。
3. 「通用」产品要用户手动建（「+ 新建产品」→ 输入「通用」→ 加模板 → 补全多语言）。
4. `template-translate` skill 已退役但仍在 `skill_sync` 注册表里，可删可留。

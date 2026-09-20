# Handoff：HTML 需求文档导入（补充测试点页 + BugPage 沉淀模式）

状态：**已实现，待验收**（skill 改动已提交但**未 push 未 tag**，`~/.claude/skills/` 下是手工 rsync 的临时副本；app 两端编译通过，UI 未实机验证——见第 6 节）
日期：2026-09-20
触发：[handoff-html-prd-import.md](handoff-html-prd-import.md) 的 D5 后续步骤（当时只做了 Generate 页，另两处记在 [todo.md](todo.md#L7)）

---

## 1. 现状（改动前）

两个入口吃 PRD，**都只认 Google Slides**，没有 HTML 路径：

| 入口 | 页面 | Rust | 模式 |
|---|---|---|---|
| 补充测试点 | `SupplementPage.vue:125-201` | `prd_supplement.rs:run_prd_supplement_reuse` | 复用模式（只给 PRD） |
| 风险沉淀 | `BugPage.vue:262-425` | `prd_risk.rs:run_prd_risk_profiler` | 沉淀模式（PRD + bug 报告） |

两条管线形状一样：

```
list_drive_files(mimeType: application/vnd.google-apps.presentation)   ← 就是 PPT
  → radio 单选一份 → export_slides_pdf（pages: [] 全篇）→ 每页一张 PNG
    → invoke(..., { prdImagePaths })
      → prompt 硬编码「PRD（按页截图，逐张 Read 查看）：\n- <png>...」
        → /prd-risk-profiler skill
```

关键约束：
- 两处 Rust 都有 `if prd_image_paths.is_empty() { return Err("没有 PRD 图片…") }` 的前置校验
- prompt 里「PRD（按页截图，逐张 Read 查看）」这句是写死的，HTML 路径必须走另一句
- 两处都是**一次性 `--print` 调用**，不存 session_id、没有 `send_claude_input` 续聊 —— 这是与 Generate 页最大的区别（见 D2）
- skill `prd-risk-profiler` v1.0.1 的 SKILL.md 全文没有 HTML/提取脚本的概念，也没有 `scripts/` 目录

---

## 2. 已定决策（2026-09-20 用户确认）

| # | 决策点 | 结论 |
|---|---|---|
| D1 | 改动范围 | **两处一起改**（补充测试点 + BugPage 沉淀模式），它们共用同一份 skill、同一个坑 |
| D2 | 章节分批 | **全文提取，不分章节**。这两处是非交互一次性调用，Claude 没法像 Generate 页那样反问「这次做哪几章」；且与现有 Slides「全篇导出、不选页码」的行为一致。prompt 里要**显式写明「不要反问章节，直接全文提取」**，否则 skill 会照 test-case-generator 的习惯停下来等回答，而这里没人能回答 |
| D3 | UI 形态 | **照抄 Generate 页的并存语义**：Slides 与 HTML 都可选、可共存，至少有一个才能点「生成」。不做互斥切换 |
| D4 | 提取脚本归属 | **复制一份 `extract_html.py` 到 prd-risk-profiler 仓库的 `scripts/`**，skill 自包含、可单独分发。代价是脚本两份，改一处要记得同步另一处（记进 gotchas） |

---

## 3. 文件级改动清单

### 3.1 skill 侧 —— 改仓库 `~/Projects/prd-risk-profiler`，**不是** `~/.claude/skills/`

> ⚠️ `~/.claude/skills/prd-risk-profiler/` 是同步产物，直接改会被下一次 app 同步整体还原。见 [gotchas.md](gotchas.md)。

| 文件 | 改动 |
|---|---|
| `scripts/extract_html.py` | **新增**，从 `~/Projects/test-case-generator/scripts/extract_html.py` 原样复制（`--info` / `--sections` / `--outdir` 全套照搬，这两处虽然只用全文提取，但保持两份脚本逐字一致才好同步） |
| `SKILL.md` frontmatter description | 补 HTML 触发词：现在只说「PRD」，要说明支持「截图 / 单文件 HTML 需求文档」 |
| `SKILL.md` 新增《输入格式》一节 | 放在「一、沉淀模式」之前，两个模式共用：PRD 可能是①按页截图→逐张 Read；②`.html`/`.htm`→**严禁直接 Read 原文件**，必须先跑 `scripts/extract_html.py --input <html> --outdir <prompt 指定目录>`，再 Read `prd.md`，图按 `[[IMAGE: ...]]` 锚点按需读 |
| `SKILL.md` 一、沉淀模式 步骤 2 | 「读PRD，提炼变更清单」补一句：PRD 是 HTML 时先按《输入格式》提取 |
| `SKILL.md` 二、复用模式 步骤 2 | 同上 |
| `SKILL.md` 硬规则 | 新增：**非交互调用下不要反问用户选章节**，prompt 明确要求全文提取时直接跑全文 |
| `SKILL.md` 参考文件小节 | 补 `scripts/extract_html.py` 一行 |
| 版本 | v1.0.1 → **v1.1.0**；打 tag 后 CI 自动发 release，app 侧同步时会写 `.tester-app-version` |

### 3.2 app 侧 —— Rust

`prd_supplement.rs` 与 `prd_risk.rs` 改法对称，都按「加法不重构」：原 Slides 分支一行不动，HTML 走新分支。

| 文件 | 改动 |
|---|---|
| `prd_supplement.rs:run_prd_supplement_reuse` / `run_inner` | 新增参数 `html_path: Option<String>`（前端传 `htmlPath`）；`.filter(\|s\| !s.is_empty())` 归一化 |
| 同上 前置校验 | `if prd_image_paths.is_empty()` → `if prd_image_paths.is_empty() && html_path.is_none()`，报错文案改成「没有 PRD（没选 Slides、也没导入 HTML）」 |
| 同上 `--add-dir` | 追加 HTML 文件的父目录（多半在 Downloads）；提取产物目录用 `gen_dir.join("html_output")`，gen_dir 本来就已授权 |
| 同上 prompt | HTML 分支：`PRD（HTML 需求文档，严禁直接 Read，先跑 <skill_dir>/scripts/extract_html.py --input <html> --outdir <gen_dir>/html_output 全文提取，再 Read 产出的 prd.md）：<path>` + 一句「本次为非交互调用，不要反问要分析哪几个章节，直接全文提取」。两种来源都有时两段都拼上 |
| `prd_risk.rs:run_prd_risk_profiler` / `run_inner` | 同样四处改动；outdir 用 `dir.join(format!("html-{}", ts))`（`dir` = `~/.tester-app/exports/prd-risk`，已授权） |

**必须显式给 `--outdir`**：claude 子进程继承 app 的 cwd，打包后多半不可写，脚本默认的相对目录会落到未知位置（Generate 页踩过，见 `claude.rs:575-585`）。

### 3.3 app 侧 —— Vue

两页都已有 `form-row` + `slides-picker` 的表单布局（不是 Generate 页的 `sel-card` 卡片），所以**照抄的是并存语义、不是像素**：在现有「PRD」那一行下面加一行「HTML」，右侧放 导入 / 更换 / 清除 按钮，已选时显示文件名（`title` 挂全路径）。

| 文件 | 改动 |
|---|---|
| `SupplementPage.vue` | `htmlPath = ref<string\|null>(null)` + `htmlFileName` computed + `pickHtmlFile()` / `clearHtmlFile()`（用 `@tauri-apps/plugin-dialog` 的 `open()`，filters 限 `html`/`htm`，权限已就绪）；`startGenerate()` 里「请先选一份 PRD」的校验改成「两者都空才报错」，Slides 导出那段包进 `if (selectedSlideId)`；`invoke("run_prd_supplement_reuse", { …, htmlPath })`；生成按钮 `:disabled` 的 `!selectedSlideId` 改成 `!selectedSlideId && !htmlPath` |
| `BugPage.vue` | 同一套（变量名跟着本页的 `riskSlideId` 风格叫 `riskHtmlPath`）；`:disabled="!riskAppName.trim() || !riskSlideId"` → 末项改成 `(!riskSlideId && !riskHtmlPath)`；页面顶部那句说明文案「选一份 PRD（Slides，全篇导出，不选页码）」要补上 HTML |

### 3.4 文档

| 文件 | 改动 |
|---|---|
| `docs/todo.md` | 删掉「HTML 需求文档：另外两处入口还没支持」整节（本次做完） |
| `docs/gotchas.md` | 新增：`extract_html.py` 现在有两份（test-case-generator / prd-risk-profiler 各一），改一处要同步另一处 |
| `docs/PROJECT_STRUCTURE.md` | 若有 PRD 来源相关描述，同步 HTML 分支 |

### 3.5 不动的东西

- `export_slides_pdf` 整条 Slides → PNG 管线
- 两处的日志流 / 取消 / 产物落盘（`风险分析.md`、`补充测试点.md`）与目录层级（App → 版本 → 生成记录）
- Generate 页与 `claude.rs`（本次完全不碰）
- `test-case-generator` skill

---

## 4. 风险点 / 坑

1. **skill 与 app 的发布顺序**：app 侧 prompt 一旦指向 `scripts/extract_html.py`，必须 skill v1.1.0 已发 release 且用户同步过，否则脚本不存在、HTML 路径直接失败。建议先发 skill、再发 app；或在 prompt 里加一句「脚本不存在就报错退出，不要退化成直接 Read HTML」作为兜底护栏。
2. **两份脚本漂移**：D4 的既定代价，靠 gotchas 记一笔 + 复制时逐字一致来缓解。
3. **bs4 依赖**：`extract_html.py` 需要 beautifulsoup4，脚本自带 pip 引导；本机已有。
4. **非交互反问**：skill 现有习惯是「先列章节目录再问用户」，这里没人能回答，会卡到超时或跑偏。prompt 与 SKILL.md 两侧都要写明，只写一侧不保险。
5. **HTML 结构泛化**：目前只验证过 Slides 导出的那类自包含 HTML（规范 `<table>`、内嵌 base64 图）。换成 Notion / 飞书导出可能抽取质量下降。

---

## 5. 验证路径

1. skill 仓库改完 → 本地把 `~/Projects/prd-risk-profiler` 拷到 `~/.claude/skills/prd-risk-profiler` 手工试跑（**注意**：下次 app 同步会覆盖，正式验收前要发 tag）
2. 补充测试点页：只导入 HTML（不选 Slides）→ 生成 → 确认日志里跑的是 `extract_html.py`、没有 Read 原 HTML、两份产物正常落到生成记录目录
3. 补充测试点页：只选 Slides（回归）→ 行为与改动前一致
4. 补充测试点页：两者都给 → prompt 两段都在，skill 两种来源都读
5. BugPage 沉淀模式：重复 2–4，并确认 `references/apps/<APP>.md` 正常更新
6. 都不给 → 生成按钮置灰，点不动

---

## 6. 实现记录（2026-09-20）

### 6.1 实际改动

**skill**（repo `~/Projects/prd-risk-profiler`，commit `9ecd16b`，**未 push / 未 tag**）
- 新增 `scripts/extract_html.py`，与 test-case-generator 那份逐字一致（`diff` 验过）
- SKILL.md：frontmatter 补 HTML 来源说明；新增《〇、输入格式》一节（两模式共用，含
  「严禁直接 Read」硬规则 + 提取命令 + 「非交互调用不要反问章节」）；沉淀/复用模式
  各自的第 2 步各加一句指回该节；参考文件补脚本一行（含「两份要同步」提醒）
- README.md 加《PRD 输入格式》一节
- 版本：仓库正文里没有版本号，版本只体现在 git tag（app 同步时写 `.tester-app-version`），
  所以本次没有「改版本号」这一步，发布时直接打 v1.1.0

**app**（两端编译通过：`cargo check` + `vue-tsc --noEmit`）
- `prd_supplement.rs` / `prd_risk.rs`：命令与 `run_inner` 各加 `html_path: Option<String>`；
  空值校验改成「图片和 HTML 都没有才报错」；HTML 父目录进 `--add-dir`；prompt 拆成
  「截图段（有图才拼）+ HTML 段（有 HTML 才拼）」，HTML 段带完整提取命令、显式
  `--outdir`（supplement → `<生成记录目录>/html_output`，risk → `exports/prd-risk/html-<ts>`）
  和「非交互、不要反问章节、脚本失败就报错别退化成 Read」
- `SupplementPage.vue` / `BugPage.vue`：新增 HTML 一行（导入/更换/清除 + 文件名，
  `title` 挂全路径）；原「PRD」标签改成「Slides」以区分两行；Slides 导出包进
  `if (slide)`；生成按钮的 `!selectedSlideId` 换成 `!hasPrdSource`
- `.claude/launch.json`：新增（vite / 1420），之前没有

### 6.2 未验证的部分

**UI 只做了静态检查，没有实机跑过。** 浏览器里打开 `localhost:1420` 会停在
「Sign in with Google」——登录态在 Tauri 后端，纯浏览器进不去补充测试点页。
需要在 `npm run tauri dev` 的桌面窗口里按第 5 节的路径实测。

端到端跑之前，`~/.claude/skills/prd-risk-profiler/` 已手工 rsync 成最新版（带 scripts/）,
但 `.tester-app-version` 还是 v1.0.1 —— **在 app 里点同步就会被打回去**，正式验收前要发 tag。

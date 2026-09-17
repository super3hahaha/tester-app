# Handoff：HTML 需求文档导入（Generate 页）

状态：**已实现，待用户验收**（skill 已放到本地目录可试用，尚未发 release；app 侧两端编译通过）
日期：2026-09-17
触发：需求文档产出格式从 Google Slides / PPTX 改为单文件 HTML（含内嵌 base64 原型图）

---

## 1. 现状管线（改动前）

```
GeneratePage.vue
  props.slidesSelection (Google Slides + 页码)
    → invoke("export_slides_pdf")  → 每页一张 PNG
      → invoke("run_claude_task", { csvPath, pptxPaths: imgPaths, ... })
        → claude.rs:run_claude_task 拼 prompt：
             /test-case-generator
             CSV (existing test cases): <path>
             Image (new requirements): <png>   ← 每页一行
             Preference files: ...
             最终 xlsx 必须落到 <output_path>
          → skill SKILL.md Step 0 路径 A：直接 Read PNG → Step 3 分析
```

关键约束：
- `canGenerate` 现在 = `slidesSelection.length > 0`（`GeneratePage.vue:272`），没选 Slides 就点不动按钮
- `run_claude_task` 的 `pptx_paths: Vec<String>` 只会被渲染成 `Image (new requirements):` 行
- `--add-dir` 按传入路径的父目录自动授权，新路径同理会被覆盖

---

## 2. HTML 实测数据（已验证，非估算）

样本：`~/Downloads/MP3 Cutter 2.3.7 需求 (Target36 大屏适配).html`

| 项 | 值 |
|---|---|
| 原始大小 | 2,732,595 字符（2.7MB） |
| 去 `<style>/<script>` + base64 后 | 35,372 字符 |
| 抽取后 Markdown 正文 | **16,827 字符（≈1 万 token）** |
| 内嵌图片 | 24 张，全部 `data:image/jpeg;base64`，共 ≈2MB |
| `<script>` / 外链 `<img>` | 0（纯静态自包含，无 JS 渲染） |

抽取原型（BeautifulSoup + 正则，≈40 行）验证结论：
- ✅ `<table>` → Markdown 表格，**三档断点表 / 可用宽公式表 / 需求列表表数值全部无损**
- ✅ `<h1..h6>` → `#` 标题，章节层级保留
- ✅ 24 张 base64 → `images/img_NN.jpg`，正文原位留 `[[IMAGE: images/img_03.jpg]]` 锚点
- ✅ 抽出的 img_03 确认为「384 / 600–839 / 840+ / 1280」四档 Cutter 编辑页原型对照图，是必须保留的信息

**结论：HTML 优于 PPT 截图路线。** 数值从「让模型 OCR 截图」变成「直读文本」，误差归零；token 从几十张全页截图降到 ~1 万 + 按需读图。

⚠️ 红线：**任何情况下不得直接 Read 原 HTML**（2.7MB 一次爆上下文）。这条必须写进 SKILL.md 硬规则。

---

## 3. 待确认决策点

### D1. 预处理放在哪一侧？（最关键）

| | A. skill 侧 `scripts/extract_html.py`（推荐） | B. app 侧 Rust 预处理 |
|---|---|---|
| app 传什么 | `HTML (new requirements): <路径>` 一行 | 抽取后的 `prd.md` + 24 个图片路径 |
| skill 改动 | 加路径 D + 新脚本 + 硬规则 | 几乎不改 |
| app 改动 | 只加文件选择 + 传路径 | Rust 写 HTML 解析 + base64 解码 |
| 脱离 app 可用 | ✅ 直接丢 HTML 给 Claude Code 也能跑 | ❌ 只有 app 内能用 |
| 与现有结构对称 | ✅ 与 `scripts/extract_prd.py` 同位 | ❌ 文档处理散到两处 |

推荐 **A**：文档格式处理本来就是 skill 的职责（extract_prd.py 已是先例），Rust 侧只做「选文件 + 传路径」，改动面最小。

### D2. 图片喂给 Claude 的策略

- **A（推荐）按需读**：脚本抽出全部 24 张，正文留 `[[IMAGE: ...]]` 锚点，skill 分析到哪个功能点就 Read 哪张图
- B 全量读：24 张全 Read，token 直接翻几倍，且大部分图与当前用例无关

### D3. 导入入口支持哪些格式？

- **A（推荐）只 HTML**：`.html` / `.htm`，符合「加法不重构」，先把当前实际格式跑通
- B 一次做全：`.html/.pdf/.pptx/.md`（PDF/PPTX 复用已有 extract_prd.py，属顺手，但摊大战线）

### D4. HTML 与 Google Slides 的关系？

- **A（推荐）可共存**：两者都能作为需求来源，`canGenerate` 改为 `slidesSelection.length > 0 || htmlPath`
- B 互斥：选了 HTML 就清空 Slides 选择

### D5. 其他吃 PRD 的入口要不要同步？

现在还有两处走 `export_slides_pdf`：
- `SupplementPage.vue:173`（补充测试点 → `prdImagePaths`）
- `prd-risk-profiler` skill（风险画像）

- **A（推荐）本次只做 Generate 页**，另两处列为后续独立步骤
- B 一次全改

---

## 4. 待确认后的改动清单（按 D1=A / D2=A / D3=A / D4=A / D5=A 预设）

### 4.1 skill 侧（`~/.claude/skills/test-case-generator/`）

| 文件 | 改动 |
|---|---|
| `scripts/extract_html.py` | **新增**。`--input x.html --outdir html_output` → 产出 `prd.md` + `images/img_NN.jpg`；`--info` 打印正文字数 / 图片数。依赖 beautifulsoup4 |
| `SKILL.md` Step 0 | **新增路径 D：HTML 文件**（`HTML (new requirements): <path>`）→ 跑脚本 → Read `prd.md` → 按锚点按需 Read 图 → 进 Step 3 |
| `SKILL.md` ⚠️ 关键规则 | **新增一条**：禁止直接 Read `.html` 原文件，必须先跑 extract_html.py |
| `SKILL.md` frontmatter description | 补 HTML 触发词（现只提 CSV / 截图 / PPTX） |
| `SKILL.md` 标题行 | `CSV + 需求截图/PPTX` → 补 HTML |
| 版本 | v1.9.6 → v1.10.0；同步 `.tester-app-version`（app 靠 `get_skill_local_version` 读它显示版本） |

**全新路径（无 CSV）不受影响**，HTML 只换需求来源，Step NEW 逻辑不动。

### 4.2 app 侧

| 文件 | 改动 |
|---|---|
| `GeneratePage.vue` 模板 | Slides 卡右侧（截图红框位）加第三张 `sel-card`「需求文档」+ 导入按钮；已选时显示文件名 + 清除 |
| `GeneratePage.vue` 逻辑 | `htmlPath = ref<string\|null>`；`pickHtmlFile()` 用 `@tauri-apps/plugin-dialog` 的 `open()`（`KnowledgeBasePage.vue:213` 已有同款用法，dialog 插件与 `dialog:allow-open` 权限均已就绪）；`canGenerate` 加 `\|\| !!htmlPath`；`handleGenerate` 里 Slides 导出循环外 early-return 式跳过，不动原逻辑 |
| `claude.rs:run_claude_task` | 新增可选参数 `html_path: Option<String>`；父目录加进 `--add-dir`；prompt 追加 `HTML (new requirements): <path>` 一行。**不动 `pptx_paths` 现有分支** |
| `GenContext` | 加 `htmlPath` 字段，供 manifest 记录来源 |
| `docs/PROJECT_STRUCTURE.md` / `gotchas.md` | 完成后同步：新增「HTML 需求不可直接 Read，2.7MB 会爆上下文」一条 |

### 4.3 不改的东西

- `export_slides_pdf` 及整条 Slides → PNG 管线原样保留
- `send_claude_input` / 会话续聊 / xlsx 落盘路径逻辑
- `SupplementPage` / `prd-risk-profiler`（见 D5）

---

## 5. 风险点

1. **bs4 依赖**：extract_html.py 需要 beautifulsoup4。本机已有 4.15.0；脚本内同 extract_prd.py 风格加 `pip install --break-system-packages -q` 引导
2. **HTML 结构泛化**：本次只验证了这一份文档（自包含、无 JS、`<table>` 规范）。若后续需求文档改用 Notion / 飞书导出（div 伪表格、外链图片），抽取质量会掉——脚本要对「0 图片 / 0 表格 / 正文过短」给出告警而非静默产出空壳
3. **外链图片**：当前文档 24 张全内嵌。若将来出现 `<img src="https://...">`，脚本需下载或显式报告「N 张外链图未取回」
4. **skill 版本与 app 联动**：改完 skill 必须同步 `.tester-app-version`，否则 app 显示的版本号与实际不符

---

## 6. 实现记录（2026-09-17）

用户确认：D1=skill 侧 / D2=按需读图 / D3=只支持 HTML / D4=可共存 / D5=只做 Generate 页（其余记 todo）。

### 6.1 追加的需求：按章节分批（D6）

原方案漏了一个关键工作习惯：**用户原本按页码分批做**——「先读 PPT 1-10 页（模块1）出用例，
再读 11-15 页（模块2）」。HTML 没有页码，如果只能整篇提取就是功能倒退。

解法：`--sections` 作为 `--slides` 的等价物，把「页」换成「章节」。

分节依据逐级降级（实测这份 MP3 Cutter 文档命中第一级，7 个 `<section id="s1..s7">`）：

| 优先级 | 依据 | 说明 |
|---|---|---|
| 1 | `<section>` 标签 | 最规范 |
| 2 | `div[id]` 锚点 | 目录跳转用的锚点 div |
| 3 | hN 标题 | 取最浅且出现 ≥2 次的层级，标题 + 后续兄弟节点为一节 |
| 4 | 都没有 | 整篇一节，**明确告知用户无法分批** |

`--info` 输出的章节目录就是「共 N 页」的等价物，而且信息更足：

```
📑 章节目录（分节依据：<section> 标签，共 7 节）：
    1. §1 需求概述           约 1011 字符 | 图 0 张
    2. §2 全局宽布局框架     约 7469 字符 | 图 20 张
    3. §3 弹窗专章           约 1795 字符 | 图 4 张
```

SKILL.md 的 Step 0.H.1 强制「先列目录 → 询问用户做哪几节」，与 PPT 路径 Step 0.1
「先查页数 → 询问页码」完全对称，原有节奏一字不用改。

分批不打架的两个设计（见 gotchas.md）：
- 图片**全文全局编号**，且在分节之前完成 —— §3 的图永远是 img_21~24
- md 文件名带章节后缀 `prd_s3.md` / `prd_s5-6.md`，同一 outdir 多轮提取互不覆盖

### 6.2 实测结果

| 操作 | 结果 |
|---|---|
| `--info` | 7 节目录，标题正确（修过一次：见 gotchas 的「节标题不能取第一个 h 标签」） |
| `--sections 3` | `prd_s3.md` 2208 字符 + `img_21~24.jpg` 4 张，表格 1 个 |
| `--sections 5-6` 到同目录 | `prd_s5-6.md` 4540 字符，**第一批产物完好未被覆盖** |
| 全文提取 | 正文 1.77 万字符（原文件 271 万），压掉 99.3%，表格 9 个，图 24 张 |

### 6.3 实际改动清单

**skill（repo `/Users/zhangshixin/Projects/test-case-generator`，未发 release）**
- 新增 `scripts/extract_html.py`
- SKILL.md：frontmatter 补 HTML、标题行、关键规则 6（禁止直接 Read HTML）、
  流程图第 0 步、Step 0 路径 D + Step 0.H.1/0.H.2/0.H.3 + 告警处理表
- 版本号 v1.9.6 → v1.10.0（正文里；tag 待发）

**app**
- `claude.rs`：`run_claude_task` 加 `html_path` 参数 → `--add-dir` 授权父目录 →
  prompt 追加 `HTML (new requirements):` 与显式 outdir
  （**显式 outdir 是必需的**：claude 进程继承 app 的 cwd，打包后多半不可写）
- `manifest.rs`：`GenerateManifest` 加 `html_path`，带 `#[serde(default)]` 兼容旧 manifest
- `GeneratePage.vue`：第三张 sel-card「需求文档 (HTML)」+ 导入/更换/清除按钮、
  `canGenerate` 加 `|| !!htmlPath`、日志与 GenContext 带上 html 路径

**没动**：`export_slides_pdf` 整条 Slides 管线、`SupplementPage`、`prd-risk-profiler`（见 todo.md）

### 6.4 踩过的坑

1. **改 `~/.claude/skills/` 白改**：那是同步产物，被一次 app 同步整体还原。源头是 GitHub repo。
   详见 gotchas.md
2. **节标题取错**：按「节内第一个 h 标签」取，7 节里有 2 节被标成「▸ 布局规则」
3. **claude 进程 cwd 不确定**：脚本默认相对 outdir 会落到未知位置，必须在 prompt 里显式指定

# TODO

未完成的工作、已知缺口、想做但暂缓的事。做完一条就删掉。

---

## HTML 需求文档：另外两处入口还没支持

2026-09-17 加 HTML 导入时只做了 **Generate 页**（[handoff-html-prd-import.md](handoff-html-prd-import.md)）。
需求文档格式已经从 Slides/PPTX 改成单文件 HTML，所以这两处迟早要跟上，否则它们只能继续喂 Slides：

| 位置 | 现状 | 要做什么 |
|---|---|---|
| `SupplementPage.vue:173`（补充测试点） | 只走 `export_slides_pdf` → `prdImagePaths` | 加 HTML 导入入口，参数加 `htmlPath`，prompt 里给 `HTML (new requirements):` |
| `prd-risk-profiler` skill | SKILL.md 只认截图 / PPTX | 复用 `test-case-generator/scripts/extract_html.py` 的思路加 HTML 路径（脚本可直接抄一份，或考虑抽成共用） |

做的时候注意：
- 两处都要带上「禁止直接 Read .html」的硬规则，否则 2.7MB 文件一次读爆上下文
- `prd-risk-profiler` 也是 GitHub 同步来的 skill（repo `super3hahaha/prd-risk-profiler`），
  改上游、发 tag，别改 `~/.claude/skills/`（见 [gotchas.md](gotchas.md)）
- 如果三个 skill 都要 HTML 提取能力，考虑是否把 `extract_html.py` 抽成一份共享脚本，
  而不是复制三份各自漂移

## HTML 来源未进反馈 zip

`manifest.rs` 已加 `html_path` 字段并落盘，但 `ComparePage` 的反馈打包流程还没读它，
所以反馈 zip 里不会带上 HTML 源文件（Slides 导出的图片是带的）。
影响面小，等反馈流程真需要时再补。

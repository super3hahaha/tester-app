# TODO

未完成的工作、已知缺口、想做但暂缓的事。做完一条就删掉。

---

## prd-risk-profiler 的 HTML 支持还没发 release

2026-09-20 补充测试点页 + BugPage 沉淀模式已加 HTML 导入
（[handoff-html-prd-supplement.md](handoff-html-prd-supplement.md)），但 skill 侧改动还停在
本地：repo `super3hahaha/prd-risk-profiler` 已提交、**未 push 未 tag**，
`~/.claude/skills/` 下是手工 rsync 过去的临时副本，下次 app 同步会被覆盖回 v1.0.1。

发 v1.1.0 之前，app 侧 HTML 路径在别人机器上会因为缺 `scripts/extract_html.py` 直接失败。

## HTML 来源未进反馈 zip

`manifest.rs` 已加 `html_path` 字段并落盘，但 `ComparePage` 的反馈打包流程还没读它，
所以反馈 zip 里不会带上 HTML 源文件（Slides 导出的图片是带的）。
影响面小，等反馈流程真需要时再补。

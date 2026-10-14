# 收藏评论 / 邮件导出（2026-10-13）

## 实现范围

- `FavoriteReviewsPage.vue`、`FavoriteMailsPage.vue`：在来源 tab + 标签筛选下方提供「导出 Excel」和「导出 Google 表格」。
- **仅导出当前可见的筛选结果**：分别直接使用 `tagFilteredFavorites`；既有标签 OR/无标签、App / 邮件源逻辑不变。
- `src/utils/favoritesExport.ts`：纯映射层；评论的 Play Console 链接复用页面内的 `reviewConsoleUrl`；页面利用 `resolveTags` 将选中标签 ID 转为标签名用于导出命名。邮件页面用已有 source tab、来源显示名和所关联的 App 包名生成导出名称。
- `src-tauri/src/favorites_export.rs`：使用已安装的 `rust_xlsxwriter` 写表；冻结首行、首行加粗、开启筛选、文本换行。列宽通过 `export_column_width(header)` 单独配置，Excel 与上传转换的 Google Sheet 使用同一份列宽配置（不是统一的 54 / 21）。Excel 调系统保存对话框；Google Sheet 先在内存生成 xlsx，再复用 `sheets::upload_xlsx_bytes_to_drive(convert_to_sheets=true)` 上传并转换。Drive 文件夹默认为 `tester-app`。每次导出创建新文件，并非持续同步。
- 安全：评论/邮件是非可信文本，前导 =/+/-/@ 作为纯文本输出，避免公式注入；超过 Excel 32767 字符的单元格截断并反馈数量；导出前限制最多 100000 行。

## 表头

- 评论（精简版）：评分、评论内容、评论原文、评论语言、评论时间、应用版本、版本代码、设备型号、Android 版本、评论 ID、Play Console 链接。**应用、标签不再单独占列**，而是根据当前选中的应用和标签命名导出文件 / Google 在线表格与内部工作表（名称合法化，文件名长度与工作表 31 字符上限截断）。评论用户、回复信息、评论统计、收藏管理字段均不再导出。
- 邮件（精简版）：**邮件正文、中文机翻、邮件链接**，共 3 列。邮件来源（包含可选关联 App 包名）和所选标签只用于导出 Excel / Google Sheet 文件名称；内部工作表名称需满足 31 字符限制，采用来源与标签分配长度，避免其中一个完全被截断。全部来源使用「全部邮件源」，未选择标签使用「全部标签」，无标签筛选使用「无标签」。邮件信息、附件、收藏管理、messageId 和 threadId 均不导出。

## 导出列宽配置（2026-10-14）

2026-10-14 用户开放参考表格权限后，已成功读取原 Google Sheet（`1RlEGLA9nWP8mLbbszZnI_cOdpT0PHT6k97KAcubBCCc`，`gid=1755342125`），并导出 xlsx，用 openpyxl 读取原表每列的 `column_dimensions.width`。以下为 **参考表实际列宽**（Excel 字符宽单位）：

| 字段 | 参考列 | 列宽 |
|---|---|---:|
| 评分 | A | 5.71 |
| 评论内容 | B | 54.71 |
| 评论原文 | C | 65.00 |
| 评论语言 | D | 11.43 |
| 评论时间 | E | 17.71 |
| 应用版本 | F | 13.86 |
| 版本代码 | G | 10.57 |
| 设备型号 | H | 13.00 |
| Android 版本 | I | 10.00 |
| Play Console 链接 | J | 77.14 |

- 参考表只有上述 10 列，当前 Tester 导出另有「评论 ID」列，因此该列**保持 39**；使用表头映射，不依赖列序号，避免新增列导致对应宽度错位。
- 收藏邮件的参考表暂未提供，保持原方案：邮件正文 72、中文机翻 72、邮件链接 48。
- 参考表中评论内容和评论原文等文本列开启自动换行，现有导出保持 `set_text_wrap()`。
- 本次仅改**新导出文件**的列宽，不修改用户的参考 Google 表格或已存在的导出文件。Google Sheets 将由 xlsx 上传转换，最终显示效果仍需在实际 Tester 客户端验证。

## 测试状态

- `npm run build`：通过。
- `cargo check`：通过。
- `cargo test`：通过（16 passed，2 个显式 live 测试 ignored）。
- 未进行 Tauri 窗口端到端测试，也未进行真实 Google Drive 授权上传验证。需在具备 Rust 工具链和有效 Google 凭据的本机进行。

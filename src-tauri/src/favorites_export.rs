use crate::auth::AuthState;
use rust_xlsxwriter::{Format, Workbook};
use serde::Serialize;
use serde_json::Value;
use tauri::State;

const MAX_CELL_CHARS: usize = 32767;

// Excel 列宽单位是默认字体下的字符宽；上传转 Google Sheets 时也会保留对应列宽。
// 评论列宽取自用户指定的 Google Sheets 参考表（sheetId=1755342125）
// 在 2026-10-14 导出为 xlsx 后实测的 column_dimensions.width。
// 参考表不包含「评论 ID」列，该列沿用原有宽度；邮件列宽未变。
fn export_column_width(header: &str) -> f64 {
    match header {
        // 收藏邮件：正文与译文是主要阅读区，链接适度收窄。
        "邮件正文" | "中文机翻" => 72.0,
        "邮件链接" => 48.0,
        // 收藏评论：逐列匹配参考表宽度。
        "评分" => 5.71,
        "评论内容" => 54.71,
        "评论原文" => 65.0,
        "评论语言" => 11.43,
        "评论时间" => 17.71,
        "应用版本" => 13.86,
        "版本代码" => 10.57,
        "设备型号" => 13.0,
        "Android 版本" => 10.0,
        "评论 ID" => 39.0,
        "Play Console 链接" => 77.14,
        _ => 22.0,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoritesExportResult {
    count: usize,
    file_path: Option<String>,
    web_url: Option<String>,
    truncated_cells: usize,
}

#[tauri::command]
pub async fn export_favorites_table(
    headers: Vec<String>,
    rows: Vec<Vec<Value>>,
    file_name: String,
    sheet_name: String,
    destination: String,
    path: Option<String>,
    state: State<'_, AuthState>,
) -> Result<FavoritesExportResult, String> {
    if headers.is_empty() || headers.len() > 100 {
        return Err("表头数量异常".into());
    }
    if rows.is_empty() {
        return Err("当前没有可导出的记录".into());
    }
    if rows.len() > 100_000 {
        return Err("单次最多导出 100000 条，请先缩小筛选范围".into());
    }
    if rows.iter().any(|r| r.len() != headers.len()) {
        return Err("表头与数据列数不一致".into());
    }
    if destination != "excel" && destination != "sheets" {
        return Err("不支持的导出目标".into());
    }

    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name(&sheet_name).map_err(|e| e.to_string())?;
    let header_format = Format::new().set_bold().set_background_color("#E9EEFA");
    let wrap_format = Format::new().set_text_wrap();

    for (idx, header) in headers.iter().enumerate() {
        sheet
            .write_with_format(0, idx as u16, header.as_str(), &header_format)
            .map_err(|e| e.to_string())?;
        sheet
            .set_column_width(idx as u16, export_column_width(header))
            .map_err(|e| e.to_string())?;
    }
    let mut truncated_cells = 0usize;
    for (r, row) in rows.iter().enumerate() {
        for (c, value) in row.iter().enumerate() {
            let rr = (r + 1) as u32;
            let cc = c as u16;
            if let Some(n) = value.as_f64() {
                sheet.write_number(rr, cc, n).map_err(|e| e.to_string())?;
            } else {
                let raw = value.as_str().unwrap_or("");
                // 防止来自用户评论/邮件的文本被 Excel / Google Sheets 解释为公式。
                let safe = if matches!(raw.chars().next(), Some('=' | '+' | '-' | '@')) {
                    format!("'{}", raw)
                } else {
                    raw.to_owned()
                };
                let text = if safe.chars().count() > MAX_CELL_CHARS {
                    truncated_cells += 1;
                    safe.chars().take(MAX_CELL_CHARS).collect::<String>()
                } else {
                    safe
                };
                sheet
                    .write_string_with_format(rr, cc, &text, &wrap_format)
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    sheet
        .autofilter(0, 0, rows.len() as u32, (headers.len() - 1) as u16)
        .map_err(|e| e.to_string())?;
    sheet.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;
    let count = rows.len();

    if destination == "excel" {
        let file_path = path.ok_or("未指定保存路径")?;
        book.save(&file_path)
            .map_err(|e| format!("保存失败：{}", e))?;
        return Ok(FavoritesExportResult {
            count,
            file_path: Some(file_path),
            web_url: None,
            truncated_cells,
        });
    }

    let bytes = book
        .save_to_buffer()
        .map_err(|e| format!("生成表格失败：{}", e))?;
    let uploaded = crate::sheets::upload_xlsx_bytes_to_drive(
        file_name,
        bytes,
        true,
        Some("tester-app".into()),
        state,
    )
    .await?;
    if !uploaded.converted {
        return Err(format!(
            "已上传到 Google Drive，但未成功转换为在线表格：{}",
            uploaded.web_url
        ));
    }
    Ok(FavoritesExportResult {
        count,
        file_path: None,
        web_url: Some(uploaded.web_url),
        truncated_cells,
    })
}

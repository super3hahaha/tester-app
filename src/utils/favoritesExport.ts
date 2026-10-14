// 收藏评论 / 收藏邮件的统一导出映射：复用页面已经筛选好的记录快照。
// Google Sheets 与本地 Excel 共用表头；来源与筛选标签用于文件/工作表命名。
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import type { FavoriteReview } from "./reviewFavorites";
import type { FavoriteMail } from "./mailFavorites";

export type ExportCell = string | number;
export type ExportMode = "excel" | "sheets";

export interface FavoritesExportResult {
  count: number;
  filePath: string | null;
  webUrl: string | null;
  truncatedCells: number;
}

const REVIEW_HEADERS = [
  "评分", "评论内容", "评论原文", "评论语言", "评论时间",
  "应用版本", "版本代码", "设备型号", "Android 版本", "评论 ID", "Play Console 链接",
];

const MAIL_HEADERS = ["邮件正文", "中文机翻", "邮件链接"];

function formatDate(ms: number | null | undefined): string {
  if (!ms || !Number.isFinite(ms)) return "";
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function reviewExportRows(
  reviews: FavoriteReview[],
  consoleLink: (review: FavoriteReview) => string | null,
): ExportCell[][] {
  return reviews.map((r) => [
    r.star_rating, r.text || "", r.original_text || "",
    r.reviewer_language || "", formatDate(r.user_comment_ts * 1000),
    r.app_version_name || "", r.app_version_code ?? "", r.device || "",
    r.android_os_version == null ? "" : `API ${r.android_os_version}`,
    r.review_id, consoleLink(r) || "",
  ]);
}

export function mailExportRows(mails: FavoriteMail[]): ExportCell[][] {
  return mails.map((m) => [m.body || "", m.translated || "", m.link || ""]);
}

function safeName(value: string): string {
  return value.replace(/[\\/:*?"<>|\[\]\r\n]/g, "_").replace(/\s+/g, " ").trim();
}

/** 工作表最多 31 字符；长邮件源和长标签各保留一部分，避免截断后标签彻底消失。 */
export function compactMailSheetName(sourceInfo: string, tagNames: string): string {
  const source = Array.from(safeName(sourceInfo) || "全部邮件源");
  const tags = Array.from(safeName(tagNames) || "全部标签");
  const maxParts = 27; // 留给两个分隔符和「邮件」后缀
  if (source.length + tags.length <= maxParts) {
    return `${source.join("")}_${tags.join("")}_邮件`;
  }
  const srcLen = Math.min(source.length, Math.max(10, Math.floor(maxParts / 2)));
  const tagLen = Math.min(tags.length, maxParts - srcLen);
  const actualSrcLen = Math.min(source.length, maxParts - tagLen);
  return `${source.slice(0, actualSrcLen).join("")}_${tags.slice(0, tagLen).join("")}_邮件`;
}

function exportFilename(kind: "review" | "mail", title?: string): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  const date = `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}_${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
  const fallback = kind === "review" ? "收藏评论" : "收藏邮件";
  const base = title ? safeName(title).slice(0, 110) : fallback;
  return `${base || fallback}_${date}.xlsx`;
}

/** 返回 null 表示用户主动取消了本地保存对话框，不算导出失败。 */
export async function exportFavorites(
  kind: "review" | "mail",
  rows: ExportCell[][],
  mode: ExportMode,
  title?: string,
  customSheetName?: string,
): Promise<FavoritesExportResult | null> {
  if (!rows.length) throw new Error("当前筛选结果为空，无法导出。");

  const fallback = kind === "review" ? "收藏评论" : "收藏邮件";
  const fileName = exportFilename(kind, title);
  const sheetName = Array.from(safeName(customSheetName || title || fallback)).slice(0, 31).join("") || fallback;
  let path: string | null = null;
  if (mode === "excel") {
    path = await save({
      defaultPath: fileName,
      filters: [{ name: "Excel 工作簿", extensions: ["xlsx"] }],
    });
    if (!path) return null;
  }

  return invoke<FavoritesExportResult>("export_favorites_table", {
    headers: kind === "review" ? REVIEW_HEADERS : MAIL_HEADERS,
    rows,
    fileName,
    sheetName,
    destination: mode,
    path,
  });
}

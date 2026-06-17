// Play Console 拉取配置（多 app）。与 Batch Reply 配置同构，额外带「回复状态」筛选。
// 配置页（PlayConsoleConfigPage）写入；Play Console 页（ReviewPage）读取作为
// 每个应用的拉取/筛选默认值（页面里仍可临时改，不回写）。
//
// 与 batchReplyDates 共享日期预设逻辑：存预设、拉取时按当天解析为绝对范围。

import { type DatePreset, toIso, daysAgo } from "./batchReplyDates";

export type ReplyState = "ANY" | "ABSENT" | "REPLIED" | "UPDATED";

export interface PlayAppConfig {
  enabled: boolean;
  datePreset: DatePreset;
  customFromDate: string; // 仅 datePreset==="custom" 用（当前 UI 不提供 custom，保留兼容）
  customToDate: string;
  stars: number[];
  replyState: ReplyState;
}

export interface PlayMultiConfig {
  perApp: Record<string, PlayAppConfig>;
}

export const PLAY_STORAGE_KEY = "play-console-multi-config-v1";
// 应用列表缓存与 Batch 配置共用一份（同一 list_play_apps 数据源）。
export const APPS_CACHE_KEY = "batch-reply-apps-cache-v1";

export const REPLY_STATE_LABELS: Record<ReplyState, string> = {
  ANY: "全部",
  ABSENT: "无回复",
  REPLIED: "已回复",
  UPDATED: "回复后又更新",
};

// 配置页只提供这几个动态预设（不含 custom），与 Batch 配置一致。
export const DATE_PRESETS: DatePreset[] = ["sinceLastWorkday", "yesterday", "today", "7d"];
export const REPLY_STATES: ReplyState[] = ["ANY", "ABSENT", "REPLIED", "UPDATED"];

export function defaultPlayConfig(): PlayAppConfig {
  return {
    enabled: false,
    datePreset: "7d",
    customFromDate: toIso(daysAgo(6)),
    customToDate: toIso(new Date()),
    stars: [1, 2, 3, 4],
    replyState: "ABSENT",
  };
}

// 把任意存储数据规整为合法 PlayAppConfig。
export function normalizePlayConfig(raw: any): PlayAppConfig {
  const def = defaultPlayConfig();
  if (!raw || typeof raw !== "object") return def;
  const stored = raw.datePreset;
  const validPreset: DatePreset | null =
    stored === "sinceLastWorkday" ||
    stored === "yesterday" ||
    stored === "today" ||
    stored === "7d"
      ? stored
      : null;
  const rs = raw.replyState;
  const validRs: ReplyState =
    rs === "ANY" || rs === "ABSENT" || rs === "REPLIED" || rs === "UPDATED"
      ? rs
      : def.replyState;
  return {
    enabled: !!raw.enabled,
    datePreset: validPreset ?? def.datePreset,
    customFromDate: raw.customFromDate || raw.fromDate || def.customFromDate,
    customToDate: raw.customToDate || raw.toDate || def.customToDate,
    stars:
      Array.isArray(raw.stars) && raw.stars.length > 0
        ? raw.stars.filter((s: any) => Number.isInteger(s) && s >= 1 && s <= 5)
        : def.stars,
    replyState: validRs,
  };
}

export function loadPlayConfig(): PlayMultiConfig | null {
  const raw = localStorage.getItem(PLAY_STORAGE_KEY);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as PlayMultiConfig;
    if (!parsed?.perApp || typeof parsed.perApp !== "object") return null;
    const perApp: Record<string, PlayAppConfig> = {};
    for (const [pkg, entry] of Object.entries(parsed.perApp)) {
      perApp[pkg] = normalizePlayConfig(entry);
    }
    return { perApp };
  } catch {
    return null;
  }
}

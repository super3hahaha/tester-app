// Review 模块「定时通知」配置：每天固定时间点自动批量拉取 + Telegram 通知。
// 按账号隔离（scopedKey），与 play-console-multi-config-v1 的启用应用/筛选条件配合使用。

import { scopedKey } from "./accountScopedKey";

export interface ScheduleConfig {
  enabled: boolean;
  times: string[]; // "HH:MM"，每天到点各触发一次
  notifyOnEmpty: boolean; // 无新增时也发一条心跳消息
  maxItemsInMsg: number; // 消息里最多列出的评论条数
  checkUpdated: boolean; // 额外扫「回复后又被用户更新」的评论并提醒复查
}

const STORAGE_KEY = "review-schedule-v1";

export function defaultScheduleConfig(): ScheduleConfig {
  return {
    enabled: false,
    times: ["10:00"],
    notifyOnEmpty: true,
    maxItemsInMsg: 5,
    checkUpdated: false,
  };
}

function isValidTime(t: unknown): t is string {
  return typeof t === "string" && /^([01]\d|2[0-3]):[0-5]\d$/.test(t);
}

export function normalizeScheduleConfig(raw: any): ScheduleConfig {
  const def = defaultScheduleConfig();
  if (!raw || typeof raw !== "object") return def;
  const rawTimes: unknown[] = Array.isArray(raw.times) ? raw.times : [];
  const times =
    rawTimes.length > 0 ? [...new Set(rawTimes.filter(isValidTime))].sort() : def.times;
  return {
    enabled: !!raw.enabled,
    times: times.length > 0 ? times : def.times,
    notifyOnEmpty: raw.notifyOnEmpty !== undefined ? !!raw.notifyOnEmpty : def.notifyOnEmpty,
    maxItemsInMsg:
      Number.isInteger(raw.maxItemsInMsg) && raw.maxItemsInMsg > 0
        ? raw.maxItemsInMsg
        : def.maxItemsInMsg,
    checkUpdated: raw.checkUpdated !== undefined ? !!raw.checkUpdated : def.checkUpdated,
  };
}

export function loadScheduleConfig(): ScheduleConfig {
  const raw = localStorage.getItem(scopedKey(STORAGE_KEY));
  if (!raw) return defaultScheduleConfig();
  try {
    return normalizeScheduleConfig(JSON.parse(raw));
  } catch {
    return defaultScheduleConfig();
  }
}

export function saveScheduleConfig(cfg: ScheduleConfig): void {
  localStorage.setItem(scopedKey(STORAGE_KEY), JSON.stringify(cfg));
}

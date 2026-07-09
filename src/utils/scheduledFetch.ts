// 定时批量拉取的独立执行逻辑：从 handleBatchFetch（ReviewPage.vue）精简复制而来，
// 不碰 ReviewPage 本身。只做「读配置 → 并行拉取 → 落快照 → 按已通知 id 集合 diff 出新增」，
// 不涉及任何 UI 状态。
//
// 「新增」判定：每个 app 各自维护一份「已通知 review_id 集合」（scopedKey 隔离，随账号切换）。
// 首次为某 app 启用时（集合从未建立过），本次命中的评论全部当作 baseline 静默标记为已通知，
// 不算新增（坑 §7）——避免刚打开定时就把最近 7 天的评论当成"新增"全量刷屏。
// 集合随每次拉取用"当前 API 返回窗口"裁剪（Play Reviews 只返回近 7 天），避免无限增长。

import { invoke } from "@tauri-apps/api/core";
import { loadPlayConfig, type PlayAppConfig } from "./playConsoleConfig";
import { computeRange } from "./batchReplyDates";
import { scopedKey } from "./accountScopedKey";
import { getActiveAccountId } from "./activeAccount";

interface FetchedReview {
  review_id: string;
  text: string;
  star_rating: number;
  user_comment_ts: number;
  developer_reply: string | null;
  developer_reply_ts: number | null;
}

export interface AppNewItems {
  pkg: string;
  appName: string;
  items: FetchedReview[]; // 本次新增（未通知过）且命中日期+星级筛选
  starCounts: Record<number, number>;
}

export interface ScheduledFetchResult {
  needRelogin: boolean;
  failedApps: string[]; // 拉取失败的应用名
  perApp: AppNewItems[]; // 只含有新增的 app（items.length > 0）
  totalNew: number;
  noAppsEnabled: boolean;
}

function snapKey(pkg: string): string {
  return `${getActiveAccountId() || "_none"}__${pkg}`;
}

const NOTIFIED_KEY_PREFIX = "review-schedule-notified-v1";
const BASELINE_KEY_PREFIX = "review-schedule-baseline-done-v1";

function notifiedStorageKey(pkg: string): string {
  return scopedKey(`${NOTIFIED_KEY_PREFIX}::${pkg}`);
}
function baselineStorageKey(pkg: string): string {
  return scopedKey(`${BASELINE_KEY_PREFIX}::${pkg}`);
}

function loadNotifiedIds(pkg: string): Set<string> {
  try {
    const raw = localStorage.getItem(notifiedStorageKey(pkg));
    const arr = raw ? JSON.parse(raw) : [];
    return new Set(Array.isArray(arr) ? arr : []);
  } catch {
    return new Set();
  }
}

function saveNotifiedIds(pkg: string, ids: Set<string>): void {
  localStorage.setItem(notifiedStorageKey(pkg), JSON.stringify([...ids]));
}

function isBaselineDone(pkg: string): boolean {
  return localStorage.getItem(baselineStorageKey(pkg)) === "1";
}
function markBaselineDone(pkg: string): void {
  localStorage.setItem(baselineStorageKey(pkg), "1");
}

async function fetchOneApp(
  pkg: string,
  appName: string,
  cfg: PlayAppConfig
): Promise<AppNewItems | { failed: true; needRelogin: boolean }> {
  let list: FetchedReview[];
  try {
    list = await invoke<FetchedReview[]>("list_play_reviews", {
      packageName: pkg,
      maxPages: 5,
      translationLanguage: "zh-CN",
    });
  } catch (e: any) {
    const msg = String(e);
    const needRelogin = msg.startsWith("NEED_RELOGIN_SCOPE") || msg.startsWith("NEED_RELOGIN:");
    return { failed: true, needRelogin };
  }

  // 落 per-app 快照，格式与 handleBatchFetch 一致，供 ReviewPage 批量视图复用。
  const tagged = list.map((r) => ({ ...r, _pkg: pkg, _app: appName }));
  invoke("save_reviews_snapshot", {
    key: snapKey(pkg),
    data: { version: 1, reviews: tagged, fetchedAt: Date.now() },
  }).catch((e) => console.warn("[scheduledFetch] save snapshot failed:", e));

  const range = computeRange(cfg.datePreset, {
    fromDate: cfg.customFromDate,
    toDate: cfg.customToDate,
  });
  const from = range.fromDate
    ? Math.floor(new Date(range.fromDate + "T00:00:00").getTime() / 1000)
    : 0;
  const to = range.toDate
    ? Math.floor(new Date(range.toDate + "T23:59:59").getTime() / 1000)
    : Number.MAX_SAFE_INTEGER;
  const matched = list.filter(
    (r) => r.user_comment_ts >= from && r.user_comment_ts <= to && cfg.stars.includes(r.star_rating)
  );

  const notified = loadNotifiedIds(pkg);
  const firstRun = !isBaselineDone(pkg);
  const newItems = firstRun ? [] : matched.filter((r) => !notified.has(r.review_id));

  // 已通知集合裁剪到"当前 API 返回窗口"内，避免无限增长；同时纳入本次命中的全部 id。
  const currentIds = new Set(list.map((r) => r.review_id));
  const matchedIds = new Set(matched.map((r) => r.review_id));
  const updated = new Set<string>();
  for (const id of currentIds) {
    if (notified.has(id) || matchedIds.has(id)) updated.add(id);
  }
  saveNotifiedIds(pkg, updated);
  if (firstRun) markBaselineDone(pkg);

  const starCounts: Record<number, number> = {};
  for (const r of newItems) starCounts[r.star_rating] = (starCounts[r.star_rating] || 0) + 1;

  return { pkg, appName, items: newItems, starCounts };
}

export async function runScheduledFetch(): Promise<ScheduledFetchResult> {
  const config = loadPlayConfig();
  const enabled = config ? Object.entries(config.perApp).filter(([, c]) => c.enabled) : [];
  if (enabled.length === 0) {
    return { needRelogin: false, failedApps: [], perApp: [], totalNew: 0, noAppsEnabled: true };
  }

  let apps: { package_name: string; display_name: string }[] = [];
  try {
    apps = await invoke("list_play_apps");
  } catch {
    // 拿不到应用名不阻塞——退化为用包名展示
  }
  const nameMap = new Map(apps.map((a) => [a.package_name, a.display_name]));

  let needRelogin = false;
  const failedApps: string[] = [];
  const perApp: AppNewItems[] = [];

  const results = await Promise.all(
    enabled.map(([pkg, cfg]) => fetchOneApp(pkg, nameMap.get(pkg) || pkg, cfg))
  );
  for (const [i, r] of results.entries()) {
    const [pkg] = enabled[i];
    if ("failed" in r) {
      failedApps.push(nameMap.get(pkg) || pkg);
      if (r.needRelogin) needRelogin = true;
      continue;
    }
    if (r.items.length > 0) perApp.push(r);
  }

  const totalNew = perApp.reduce((sum, a) => sum + a.items.length, 0);
  return { needRelogin, failedApps, perApp, totalNew, noAppsEnabled: false };
}

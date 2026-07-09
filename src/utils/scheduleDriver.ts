// 定时通知的驱动逻辑：命中配置时间点 → 跑 scheduledFetch → 组装 Telegram 消息 → 发送。
// 挂载在 App.vue（常驻，不受子页切换/卸载影响）。每分钟 tick 一次，外加启动/唤醒时的
// 错过补发（坑 §7：系统睡眠期间 setInterval 不走，合盖过夜会错过整点）。

import { invoke } from "@tauri-apps/api/core";
import { loadScheduleConfig } from "./scheduleConfig";
import { runScheduledFetch, type ScheduledFetchResult, type AppNewItems } from "./scheduledFetch";
import { scopedKey } from "./accountScopedKey";
import { activeAccountEmail } from "./activeAccount";

const FIRED_KEY = "review-schedule-fired-v1";

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}
function todayIso(): string {
  const d = new Date();
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}
function nowHHMM(): string {
  const d = new Date();
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

interface FiredState {
  date: string;
  times: string[];
}

function loadFired(): FiredState {
  try {
    const raw = localStorage.getItem(scopedKey(FIRED_KEY));
    if (!raw) return { date: todayIso(), times: [] };
    const parsed = JSON.parse(raw) as FiredState;
    if (parsed.date !== todayIso()) return { date: todayIso(), times: [] };
    return { date: parsed.date, times: Array.isArray(parsed.times) ? parsed.times : [] };
  } catch {
    return { date: todayIso(), times: [] };
  }
}

function markFired(time: string): void {
  const state = loadFired();
  if (!state.times.includes(time)) state.times.push(time);
  localStorage.setItem(scopedKey(FIRED_KEY), JSON.stringify(state));
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
function truncate(s: string, n: number): string {
  return s.length > n ? s.slice(0, n) + "…" : s;
}

function buildNewItemsMessage(
  result: ScheduledFetchResult,
  timeLabel: string,
  maxItems: number,
  isCatchup: boolean
): string {
  const d = new Date();
  const dateLabel = `${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
  const account = activeAccountEmail.value || "未知账号";

  const lines: string[] = [];
  lines.push(`🔔 <b>差评巡检 · ${dateLabel} ${timeLabel}</b>${isCatchup ? "（错过补发）" : ""}`);
  lines.push(`账号：${escapeHtml(account)}`);
  lines.push("");
  lines.push(`📊 本次新增 <b>${result.totalNew}</b> 条（按配置筛选）`);
  for (const app of result.perApp) {
    const starParts = Object.entries(app.starCounts)
      .sort((a, b) => Number(a[0]) - Number(b[0]))
      .map(([star, count]) => `★${star}×${count}`)
      .join(" ");
    lines.push(`• ${escapeHtml(app.appName)}　${starParts}`);
  }

  const all: (AppNewItems["items"][number] & { appName: string })[] = result.perApp.flatMap(
    (app) => app.items.map((r) => ({ ...r, appName: app.appName }))
  );
  all.sort((a, b) => b.user_comment_ts - a.user_comment_ts);
  const top = all.slice(0, maxItems);

  if (top.length > 0) {
    lines.push("");
    lines.push("—— 最新几条 ——");
    top.forEach((r, i) => {
      lines.push(`${i + 1} ★${r.star_rating} ${escapeHtml(r.appName)}`);
      lines.push(`   "${escapeHtml(truncate(r.text || "", 40))}"`);
    });
    const rest = all.length - top.length;
    if (rest > 0) lines.push(`（其余 ${rest} 条见 app）`);
  }

  if (result.failedApps.length > 0) {
    lines.push("");
    lines.push(`⚠️ 拉取失败：${result.failedApps.join("、")}`);
  }

  let text = lines.join("\n");
  // Telegram sendMessage 单条上限 4096 字符，留余量。
  if (text.length > 4000) text = text.slice(0, 3990) + "\n…（内容过长已截断）";
  return text;
}

async function sendTelegram(text: string): Promise<void> {
  try {
    await invoke("send_telegram_message", { text });
  } catch (e) {
    console.warn("[scheduleDriver] send_telegram_message failed:", e);
  }
}

async function runAndNotify(dueTimes: string[], isCatchup: boolean): Promise<void> {
  const cfg = loadScheduleConfig();
  const timeLabel = dueTimes[dueTimes.length - 1];
  let result: ScheduledFetchResult;
  try {
    result = await runScheduledFetch();
  } catch (e) {
    console.warn("[scheduleDriver] runScheduledFetch failed:", e);
    await sendTelegram(`⚠️ 定时拉取出错（${timeLabel}）：${String(e)}`);
    return;
  }

  if (result.noAppsEnabled) return; // 未启用任何应用，静默跳过

  if (result.needRelogin) {
    await sendTelegram("⚠️ 定时拉取失败：登录已失效或缺少权限，请打开 app 重新登录后台账号。");
    return;
  }

  if (result.totalNew === 0) {
    if (!cfg.notifyOnEmpty) return;
    const d = new Date();
    const dateLabel = `${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
    await sendTelegram(`✅ 今日无新差评（${dateLabel} ${timeLabel}）`);
    return;
  }

  await sendTelegram(buildNewItemsMessage(result, timeLabel, cfg.maxItemsInMsg, isCatchup));
}

let running = false;

// 每次 tick（每分钟）或唤醒/启动时调用；命中一个或多个「今天已过且未触发」的时间点则
// 合并成一次拉取+一条通知（避免错过补发时刷屏），并把这些时间点都标记为已触发。
export async function checkAndFireSchedule(): Promise<void> {
  if (running) return;
  const cfg = loadScheduleConfig();
  if (!cfg.enabled || cfg.times.length === 0) return;

  const fired = loadFired();
  const now = nowHHMM();
  const due = cfg.times.filter((t) => t <= now && !fired.times.includes(t));
  if (due.length === 0) return;

  running = true;
  try {
    await runAndNotify(due, due.length > 1);
    due.forEach(markFired);
  } finally {
    running = false;
  }
}

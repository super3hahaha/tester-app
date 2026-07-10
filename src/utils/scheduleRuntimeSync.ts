// 把「定时配置 + Play Console 拉取配置里启用的应用及筛选 + 应用显示名」聚合成后端能读的
// 运行时快照，推给 Rust（save_schedule_runtime）。后端定时线程读它决定到点给谁、拉哪些 app。
//
// 为什么要镜像：定时改到后端跑（不受 webview 节流影响），但这些配置都在 localStorage，
// 后端读不到 → 前端在「保存配置 / 启动 / 切账号」时主动同步一份过去。
// 只镜像当前活跃账号的配置（后端也只对当前活跃账号生效）。

import { invoke } from "@tauri-apps/api/core";
import { loadScheduleConfig } from "./scheduleConfig";
import { loadPlayConfig } from "./playConsoleConfig";
import { scopedKey } from "./accountScopedKey";

// 应用显示名缓存（与 Play Console / Batch 配置页共用同一份）。
const APPS_CACHE_KEY = "batch-reply-apps-cache-v1";

interface CachedApp {
  package_name: string;
  display_name: string;
}

function loadAppNameMap(): Map<string, string> {
  try {
    const raw = localStorage.getItem(scopedKey(APPS_CACHE_KEY));
    const list: CachedApp[] = raw ? JSON.parse(raw) : [];
    return new Map(list.map((a) => [a.package_name, a.display_name]));
  } catch {
    return new Map();
  }
}

export async function syncScheduleRuntimeToBackend(): Promise<void> {
  const schedule = loadScheduleConfig();
  const playConfig = loadPlayConfig();
  const nameMap = loadAppNameMap();

  const apps = playConfig
    ? Object.entries(playConfig.perApp)
        .filter(([, c]) => c.enabled)
        .map(([pkg, c]) => ({
          packageName: pkg,
          displayName: nameMap.get(pkg) || pkg,
          datePreset: c.datePreset,
          customFromDate: c.customFromDate,
          customToDate: c.customToDate,
          stars: c.stars,
        }))
    : [];

  const runtime = {
    schedule: {
      enabled: schedule.enabled,
      times: schedule.times,
      notifyOnEmpty: schedule.notifyOnEmpty,
      maxItemsInMsg: schedule.maxItemsInMsg,
      checkUpdated: schedule.checkUpdated,
    },
    apps,
  };

  try {
    await invoke("save_schedule_runtime", { runtime });
  } catch (e) {
    // 未登录等情况静默失败——不阻塞任何 UI 流程。
    console.warn("[scheduleRuntimeSync] save_schedule_runtime failed:", e);
  }
}

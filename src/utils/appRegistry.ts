// app 清单 + 邮件源 → app 的查询，给「收藏标签按 app 限定范围」用。
//
// - app 清单取 GP 模板管理「管理关联」维护的 package_map（get_package_map）：包名 + 显示名 + 关联的
//   GP 模板产品。这是 app 里唯一一份「我负责哪些 app」的全局清单（Play Console 拉取配置是按账号的）。
// - 邮件本身不知道属于哪个 app，靠邮件源上的 `appPkg` 字段（GmailPage 设置区「关联 app」）。
//   `appPkg` 三态：undefined = 还没推断过；"" = 明确不关联；包名 = 关联。
//   推断口径：邮件源关联的邮件模板产品名 与 package_map 里 GP 产品名 **不区分大小写** 相等 → 取该包名
//   （邮件模板产品是 `video to mp3`，GP 产品是 `Video to MP3`，是两套独立模板库，只能按名字对）。

import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

export interface PkgApp {
  package: string;
  display: string;
  product: string | null;
}

/** package_map 里的 app 清单，按显示名排序。 */
export const pkgApps = ref<PkgApp[]>([]);
let loading: Promise<void> | null = null;
let loaded = false;

/** 读 package_map。默认只读一次；force=true 强制重读（切回页面时）。失败保持旧值。 */
export function loadPkgApps(force = false): Promise<void> {
  if (loaded && !force) return Promise.resolve();
  if (loading) return loading;
  loading = invoke<PkgApp[]>("get_package_map")
    .then((rows) => {
      pkgApps.value = [...rows].sort((a, b) => (a.display || a.package).localeCompare(b.display || b.package));
      loaded = true;
    })
    .catch(() => {})
    .finally(() => {
      loading = null;
    });
  return loading;
}

/** 包名 → 显示名（不在清单里就原样显示包名）。 */
export function appLabel(pkg: string): string {
  const hit = pkgApps.value.find((a) => a.package === pkg);
  return hit?.display || pkg;
}

/** 按模板产品名（不区分大小写）反查包名；多个 app 挂同一产品时取第一个，查不到返回 ""。 */
export function appPkgForProduct(product: string | undefined): string {
  if (!product) return "";
  const p = product.trim().toLowerCase();
  return pkgApps.value.find((a) => (a.product || "").trim().toLowerCase() === p)?.package || "";
}

// ── 邮件源 → app（收藏邮件页用；GmailPage 自己持有 sources，不走这里）──
const MAIL_SOURCES_KEY = "gmail-sources-v1";

/** sourceKey → appPkg（"" 表示没关联）。 */
export const mailSourceApps = ref<Record<string, string>>({});

/** 从 localStorage 重读邮件源的 app 关联（收藏邮件页 onMounted / 切回时调用）。 */
export function reloadMailSourceApps(): void {
  const out: Record<string, string> = {};
  try {
    const list = JSON.parse(localStorage.getItem(MAIL_SOURCES_KEY) || "[]") as Array<{
      key?: string;
      id?: string;
      appPkg?: string;
    }>;
    for (const s of list) out[s.key || s.id || ""] = s.appPkg || "";
  } catch {
    // 邮件源表坏了就当都没关联：只影响标签范围，退回显示全部标签
  }
  mailSourceApps.value = out;
}

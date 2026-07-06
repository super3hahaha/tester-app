import { scopedKey } from "./accountScopedKey";

const MIGRATED_FLAG = "store-migrated-v1";

// 一阶段隔离的配置 key：升级前是全局单份，属于「当时那个账号」，迁给当前 active 账号最合理。
// 快照（reviews-cache）不在此列 —— 可重拉，不迁。
const LEGACY_KEYS = [
  "review-page-config-v3",
  "play-console-multi-config-v1",
  "batch-reply-apps-cache-v1",
];

// 一次性迁移：把旧全局配置搬到「当前 active 账号」命名空间下，再删旧全局 key。幂等（MIGRATED_FLAG）。
// 必须在 activeAccountId 就绪且已登录后调用（否则会落到 _none 桶）。
export function migrateLegacyStorageOnce(): void {
  if (localStorage.getItem(MIGRATED_FLAG)) return;
  for (const base of LEGACY_KEYS) {
    const legacy = localStorage.getItem(base);
    if (legacy === null) continue;
    const scoped = scopedKey(base);
    // 已有账号级数据则不覆盖，仅清理旧全局 key。
    if (localStorage.getItem(scoped) === null) {
      localStorage.setItem(scoped, legacy);
    }
    localStorage.removeItem(base);
  }
  localStorage.setItem(MIGRATED_FLAG, "1");
}

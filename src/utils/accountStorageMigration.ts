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

// 二阶段：batch 相关配置的旧全局 key。
//
// ⚠️ 这里【不复制到任何账号桶】，只删除旧全局 key。原因（踩过的坑）：
// review/play 配置升级前虽也是全局单份，但基本是"跟着当前在用的账号"的，迁给 active
// 账号大体不出错。batch 配置不同——它是真正全局共享、多账号混用的一份，无法归属到某个
// 账号。早期版本这里照搬一阶段逻辑「复制给升级时 active 的账号」，结果给一个从没配过
// batch 的账号硬塞了别人的配置（脏数据）。所以改为：不迁移，各账号 batch 配置从空开始、
// 按需自己勾选（配置很轻，成本低），彻底杜绝塞错账号。
const MIGRATED_FLAG_V2 = "store-migrated-v2";
const LEGACY_KEYS_V2 = ["batch-reply-multi-config-v3", "batch-reply-manual-ids-v1"];

export function migrateLegacyStorageOnceV2(): void {
  if (localStorage.getItem(MIGRATED_FLAG_V2)) return;
  for (const base of LEGACY_KEYS_V2) {
    localStorage.removeItem(base);
  }
  localStorage.setItem(MIGRATED_FLAG_V2, "1");
}

import { getActiveAccountId } from "./activeAccount";

// 给 localStorage key 注入当前账号维度，实现按账号隔离。旧读写逻辑不变，只是 key 包一层。
// 未登录（id 空）时落 "_none" 桶；登录后各账号各自命名空间，切账号自然读到各自的数据。
export function scopedKey(base: string): string {
  return `${base}::acct:${getActiveAccountId() || "_none"}`;
}

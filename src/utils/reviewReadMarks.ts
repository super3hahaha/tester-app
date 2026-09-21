// 评论「已读」标记：给「看过了、但不打算回复」的评论一个终结状态，让它从
// Play Console 页和批量回复页的列表里消失（只有切到「只看已读」视图才看得到）。
//
// 解决的问题：只有「回复」能让评论离开「无回复」视图，4★ 这类不需要回复的好评
// 会一直占位。
//
// 设计要点：
//   - 纯本地视图状态，**不回写 Play Console**（API 没有这个概念），也**不进评论快照**
//     —— reviewsStore 的 upsert() 每次拉取都用 API 返回覆盖字段，自定义字段活不过一次刷新。
//   - 模块级共享响应式 map：两个页面的 filter computed 直接读它，任一页标记后另一页
//     立刻同步（与 reviewsStore 共享数组同样的路子）。
//   - 「撤销上一条」不另建栈，就取 at 最大的那条 —— 天然跨会话有效、可连点连撤。
//   - 按账号隔离（scopedKey），跨 app 共用一张表（review_id 全局唯一）。

import { ref, computed } from "vue";
import { scopedKey } from "./accountScopedKey";
import { loadMapSafe, saveMapSafe } from "./favoritesStorage";

const READ_KEY = "review-read-v1";
const LABEL = "已读标记";

// 超出这个天数的记录在重载时清掉：Play 评论 API 只回最近 7 天，30 天前的标记
// 对应的评论早已不在任何列表里，留着只会让这张表无限膨胀。阈值远大于 7 天，
// 撤销/查看已读都不会被误清。
const PRUNE_DAYS = 30;

export interface ReadMark {
  at: number; // 标记时间（ms）。「上一条」按它取 max
  pkg: string;
  app: string;
  // 下面两个只为撤销按钮 / 已读视图的展示，不参与任何逻辑
  author: string;
  excerpt: string;
}

/** 最近一次读/写出的问题，供页面 banner 显示；写成功即清空。 */
export const readMarksError = ref("");

/** 当前账号的已读表（模块级共享响应式，两页共用）。 */
export const readMarks = ref<Record<string, ReadMark>>({});

export const readCount = computed(() => Object.keys(readMarks.value).length);

function storeKey(): string {
  return scopedKey(READ_KEY);
}

function persist(map: Record<string, ReadMark>): boolean {
  const { ok, error } = saveMapSafe(storeKey(), map, LABEL);
  readMarksError.value = error;
  return ok;
}

/**
 * 从 localStorage 重读当前账号的已读表（切账号、页面重新激活时调用）。
 * 顺带清理过期记录；清理写失败不影响本次读到的内容。
 */
export function reloadReadMarks(): void {
  const { map, error } = loadMapSafe<ReadMark>(storeKey(), LABEL);
  // 只在有问题时设置：读正常不清空，否则会把刚刚写失败的提示冲掉。
  if (error) readMarksError.value = error;

  const cutoff = Date.now() - PRUNE_DAYS * 86400_000;
  let pruned = false;
  for (const [id, m] of Object.entries(map)) {
    if (!m || typeof m.at !== "number" || m.at < cutoff) {
      delete map[id];
      pruned = true;
    }
  }
  readMarks.value = map;
  if (pruned && !error) persist(map);
}

export function isRead(reviewId: string): boolean {
  return reviewId in readMarks.value;
}

// 标记来源既有 Play Console 页的 TaggedReview，也有批量页候选卡里的 review，
// 只取用到的字段（不依赖 TaggedReview 类型，避免两页的类型耦合进来）。
export interface ReadTarget {
  review_id: string;
  author_name: string;
  text: string;
  original_text?: string | null;
  _pkg: string;
  _app: string;
}

/**
 * 标为已读。返回是否真的写进去了 —— false 时 readMarksError 已带上原因，
 * 调用方应据此放弃 UI 更新（别让评论从列表消失而磁盘上没记）。
 */
export function markRead(r: ReadTarget): boolean {
  const next = { ...readMarks.value };
  next[r.review_id] = {
    at: Date.now(),
    pkg: r._pkg,
    app: r._app,
    author: r.author_name || "(匿名)",
    excerpt: (r.text || r.original_text || "").slice(0, 40),
  };
  if (!persist(next)) return false;
  readMarks.value = next;
  return true;
}

/** 标回未读。本来就不在表里视作已达成。 */
export function unmarkRead(reviewId: string): boolean {
  if (!(reviewId in readMarks.value)) return true;
  const next = { ...readMarks.value };
  delete next[reviewId];
  if (!persist(next)) return false;
  readMarks.value = next;
  return true;
}

/** 最近标记的那条（撤销按钮的目标）；表空时返回 null。 */
export function lastRead(): { id: string; mark: ReadMark } | null {
  let best: { id: string; mark: ReadMark } | null = null;
  for (const [id, mark] of Object.entries(readMarks.value)) {
    if (!best || mark.at > best.mark.at) best = { id, mark };
  }
  return best;
}

/** 撤销上一条已读；没有可撤的返回 false（此时 readMarksError 为空，非错误）。 */
export function undoLastRead(): boolean {
  const last = lastRead();
  if (!last) return false;
  return unmarkRead(last.id);
}

// 模块加载即读一次：两个页面都是 v-show 常驻挂载，onMounted 只跑一次，
// 这里先备好数据，切账号时各页面的 watch 会再调 reloadReadMarks()。
reloadReadMarks();

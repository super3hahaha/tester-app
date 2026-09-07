// 两个评论页（Play Console 的 ReviewPage、批量回复的 BatchReplyPage）共享的评论数据层。
//
// 同一账号 + 同一包名在内存里只有一份响应式数组，两页拿到的是同一批对象引用：
// 任一页回复成功后调 markReplied()，另一页的过滤 computed 立刻重算，把这条从
// 「未回复」视图里去掉，不需要手动重新拉取。
//
// 磁盘快照沿用 reviews.rs 的 save/load_reviews_snapshot（key = `账号__包名`），
// 后端定时线程写的是同一份文件 —— 所以定时巡检拉到的新评论，两页都能直接读到。

import { ref, type Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getActiveAccountId } from "./activeAccount";
import type { ReplyState } from "./playConsoleConfig";

export interface Review {
  review_id: string;
  author_name: string;
  text: string;
  original_text: string | null;
  star_rating: number;
  reviewer_language: string | null;
  device: string | null;
  android_os_version: number | null;
  app_version_name: string | null;
  app_version_code: number | null;
  thumbs_up_count: number;
  thumbs_down_count: number;
  user_comment_ts: number;
  developer_reply: string | null;
  developer_reply_ts: number | null;
}

// 每条评论带上来源应用标签 —— 单应用拉取也带（同一个 app），批量视图靠它区分各 app。
export type TaggedReview = Review & { _pkg: string; _app: string };

const SNAP_VERSION = 1;

interface Entry {
  list: Ref<TaggedReview[]>;
  fetchedAt: Ref<number | null>;
  loaded: boolean; // 本会话是否已尝试从磁盘快照载入过
}

// key 是 `账号__包名`：切账号自然拿到另一份数组，不用手动清缓存。
const cache = new Map<string, Entry>();

function cacheKey(pkg: string): string {
  return `${getActiveAccountId() || "_none"}__${pkg}`;
}

function entryOf(pkg: string): Entry {
  const key = cacheKey(pkg);
  let e = cache.get(key);
  if (!e) {
    e = { list: ref<TaggedReview[]>([]), fetchedAt: ref<number | null>(null), loaded: false };
    cache.set(key, e);
  }
  return e;
}

/** 该包名的共享响应式评论数组（同一账号下两页拿到的是同一个引用）。 */
export function getReviews(pkg: string): Ref<TaggedReview[]> {
  return entryOf(pkg).list;
}

/** 该包名最近一次拉取/快照的时间戳（ms）。 */
export function getFetchedAt(pkg: string): number | null {
  return entryOf(pkg).fetchedAt.value;
}

// 按 review_id 把新数据并进已有数组：命中的原地改写（保留对象引用，这样另一页持有的
// 引用、以及 BatchReply 已经填好的候选草稿都不会被冲掉），没命中的新增；
// 本次没返回的删除（Play 评论接口只回最近约 7 天，过期的本就该从视图里消失）。
//
// 注意是**原地** splice 而不是整体换一个新数组：两个页面都直接持有这个数组的引用，
// 换掉引用会让先拿到旧数组的那页看不到新拉到的评论。
function upsert(e: Entry, incoming: TaggedReview[]) {
  const byId = new Map(e.list.value.map((r) => [r.review_id, r]));
  const next: TaggedReview[] = incoming.map((r) => {
    const hit = byId.get(r.review_id);
    if (!hit) return r;
    Object.assign(hit, r);
    return hit;
  });
  e.list.value.splice(0, e.list.value.length, ...next);
}

async function readSnapshot(
  pkg: string
): Promise<{ reviews: TaggedReview[]; fetchedAt: number | null } | null> {
  try {
    const data = await invoke<any>("load_reviews_snapshot", { key: cacheKey(pkg) });
    if (!data || !Array.isArray(data.reviews)) return null;
    return { reviews: data.reviews as TaggedReview[], fetchedAt: data.fetchedAt ?? null };
  } catch (e) {
    console.warn("load reviews snapshot failed:", e);
    return null;
  }
}

async function writeSnapshot(pkg: string, e: Entry) {
  try {
    await invoke("save_reviews_snapshot", {
      key: cacheKey(pkg),
      data: { version: SNAP_VERSION, reviews: e.list.value, fetchedAt: e.fetchedAt.value },
    });
  } catch (err) {
    // 持久化失败不阻塞主流程
    console.warn("save reviews snapshot failed:", err);
  }
}

/** 本会话首次访问该包名时从磁盘快照载入；返回是否有可用数据。 */
export async function ensureLoaded(pkg: string): Promise<boolean> {
  const e = entryOf(pkg);
  if (e.loaded) return e.list.value.length > 0;
  e.loaded = true;
  const snap = await readSnapshot(pkg);
  if (!snap) return false;
  upsert(e, snap.reviews);
  e.fetchedAt.value = snap.fetchedAt;
  return e.list.value.length > 0;
}

/** 强制重读磁盘快照（后端定时线程刚刷新过快照时用，不必再打一次 API）。 */
export async function reloadFromSnapshot(pkg: string): Promise<boolean> {
  const e = entryOf(pkg);
  e.loaded = true;
  const snap = await readSnapshot(pkg);
  if (!snap) return false;
  upsert(e, snap.reviews);
  e.fetchedAt.value = snap.fetchedAt;
  return e.list.value.length > 0;
}

/** 打 Google API 拉最新评论 → 并进共享数组 → 落盘快照。抛错交调用方处理登录态提示。 */
export async function refreshFromApi(pkg: string, appName: string): Promise<TaggedReview[]> {
  const list = await invoke<Review[]>("list_play_reviews", {
    packageName: pkg,
    maxPages: 5,
    translationLanguage: "zh-CN",
  });
  const e = entryOf(pkg);
  e.loaded = true;
  upsert(
    e,
    list.map((r) => ({ ...r, _pkg: pkg, _app: appName }))
  );
  e.fetchedAt.value = Date.now();
  await writeSnapshot(pkg, e);
  return e.list.value;
}

/**
 * 回复成功后调用：原地改写共享数组里那条评论 + 落盘。
 * 两页共用同一批对象，所以另一页的过滤 computed 会立刻把这条从未回复列表里剔除。
 */
export async function markReplied(
  pkg: string,
  reviewId: string,
  replyText: string,
  ts: number
): Promise<void> {
  const e = entryOf(pkg);
  const hit = e.list.value.find((r) => r.review_id === reviewId);
  if (!hit) return; // 该 app 的评论没在内存里（比如从收藏页发起的回复）→ 无需同步
  hit.developer_reply = replyText;
  hit.developer_reply_ts = ts;
  await writeSnapshot(pkg, e);
}

/** 回复状态筛选口径 —— 两页共用这一份实现，保证展示的评论完全一致。 */
export function matchesReplyState(r: Review, state: ReplyState): boolean {
  if (state === "ABSENT" && r.developer_reply) return false;
  if (state === "REPLIED" && !r.developer_reply) return false;
  if (
    state === "UPDATED" &&
    !(r.developer_reply && r.developer_reply_ts && r.user_comment_ts > r.developer_reply_ts)
  )
    return false;
  return true;
}

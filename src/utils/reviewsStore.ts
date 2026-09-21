// 两个评论页（Play Console 的 ReviewPage、批量回复的 BatchReplyPage）共享的评论数据层。
//
// 同一账号 + 同一包名在内存里只有一份响应式数组，两页拿到的是同一批对象引用：
// 任一页回复成功后调 markReplied()，另一页的过滤 computed 立刻重算，把这条从
// 「未回复」视图里去掉，不需要手动重新拉取。
//
// 磁盘快照沿用 reviews.rs 的 save/load_reviews_snapshot（key = `账号__包名`），
// 后端定时线程写的是同一份文件 —— 所以定时巡检拉到的新评论，两页都能直接读到。
//
// 快照是**累积**的（不是每次覆盖）：API 只回最近 ~7 天，超窗口的评论只在本地留着，
// 这样长假期间每天定时拉取，回来能看到整段时间的评论而不只是最后 7 天。合并逻辑在
// 后端 save_reviews_snapshot 里（两条写路径都经过它），内存侧对应下面的 upsert。

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

// 本地累积保留多久（天）。与后端 reviews.rs 的 SNAPSHOT_RETAIN_DAYS 保持一致：
// 内存和磁盘用同一个口径，免得页面上还有、重启后又没了。
const RETAIN_DAYS = 180;

// 按 review_id 把新数据并进已有数组：命中的原地改写（保留对象引用，这样另一页持有的
// 引用、以及 BatchReply 已经填好的候选草稿都不会被冲掉），没命中的新增；
// **本次没返回的保留**（Play 评论接口只回最近约 7 天，但长假 10 天回来要能看到全程，
// 所以超窗口的旧评论留在本地，由磁盘快照累积 —— 见 decisions.md「评论本地累积归档」）。
// 超过 RETAIN_DAYS 的丢掉，防止无限膨胀。
//
// 注意是**原地** splice 而不是整体换一个新数组：两个页面都直接持有这个数组的引用，
// 换掉引用会让先拿到旧数组的那页看不到新拉到的评论。
function upsert(e: Entry, incoming: TaggedReview[]) {
  const byId = new Map(e.list.value.map((r) => [r.review_id, r]));
  const next: TaggedReview[] = [...e.list.value];
  for (const r of incoming) {
    const hit = byId.get(r.review_id);
    if (hit) {
      Object.assign(hit, r); // 回复状态/点赞数等会变，以新数据为准
    } else {
      byId.set(r.review_id, r);
      next.push(r);
    }
  }
  const cutoff = Math.floor(Date.now() / 1000) - RETAIN_DAYS * 86400;
  // ts 缺失/为 0 的留着（裁剪只为控体积，不该因为字段异常就丢用户数据）。
  const kept = next.filter((r) => !r.user_comment_ts || r.user_comment_ts >= cutoff);
  // 统一按评论时间倒序：合并进来的旧评论本来追加在末尾，不排序列表顺序会乱。
  kept.sort((a, b) => b.user_comment_ts - a.user_comment_ts);
  e.list.value.splice(0, e.list.value.length, ...kept);
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

// markReplied 的合并写：快照累积后能到几 MB，「一键提交全部」逐条落盘会把同一份
// 大文件反复读-改-写上百次，明显拖慢提交。改成 500ms 内合并成一次写。
//
// 代价：最后一次写落盘前进程被杀，那几条回复的状态在本地快照里会缺 —— 无害，
// 回复在 Play 侧已经生效，下次拉取就会带回来。拉取/批量拉取仍是立即写，不走这里。
const WRITE_DEBOUNCE_MS = 500;
const pendingWrites = new Map<string, ReturnType<typeof setTimeout>>();

function scheduleWrite(pkg: string, e: Entry) {
  const key = cacheKey(pkg);
  const prev = pendingWrites.get(key);
  if (prev) clearTimeout(prev);
  pendingWrites.set(
    key,
    setTimeout(() => {
      pendingWrites.delete(key);
      void writeSnapshot(pkg, e);
    }, WRITE_DEBOUNCE_MS)
  );
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
  scheduleWrite(pkg, e);
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

// 收藏评论：把评论加星收藏，供 Review 工作区「收藏评论」tab 快速回看。
// 存整条评论快照（而非只存 id）——即使原评论超出 API 7 天窗口、本地快照被覆盖，
// 收藏夹里仍能看到收藏时的完整内容。按账号隔离（scopedKey），键为 review_id。

import { ref } from "vue";
import { scopedKey } from "./accountScopedKey";
import { loadMapSafe, saveMapSafe } from "./favoritesStorage";

const FAV_KEY = "review-fav-v1";

// 最近一次读/写出的问题，供页面显示 banner。写成功即清空。
// 读到损坏时会一直有值（每次 loadFavorites 都会重新设上），直到数据被处理 + 重启。
export const favoritesError = ref("");

export interface FavoriteReview {
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
  _pkg: string;
  _app: string;
  favoritedAt: number;
  // 备注：收藏后自己补的笔记，可空（旧记录没有这两个字段）。
  // 存在收藏记录内部而非独立表——取消收藏即连带删掉备注，不留孤儿数据。
  note?: string;
  noteUpdatedAt?: number;
}

function storeKey(): string {
  return scopedKey(FAV_KEY);
}

export function loadFavorites(): Record<string, FavoriteReview> {
  const { map, error } = loadMapSafe<FavoriteReview>(storeKey());
  // 只在有问题时设置：读正常不清空，否则会把刚刚写失败的提示冲掉
  // （写失败的调用方紧接着就会 loadList() 重刷列表）。
  if (error) favoritesError.value = error;
  return map;
}

// 返回是否真的写进去了。false 时 favoritesError 已带上原因，调用方应据此
// 放弃本地状态更新（别让 UI 显示成功而磁盘上没有）。
function saveFavorites(map: Record<string, FavoriteReview>): boolean {
  const { ok, error } = saveMapSafe(storeKey(), map);
  favoritesError.value = error;
  return ok;
}

export function addFavorite(review: FavoriteReview): boolean {
  const map = loadFavorites();
  map[review.review_id] = review;
  return saveFavorites(map);
}

export function removeFavorite(reviewId: string): boolean {
  const map = loadFavorites();
  if (!(reviewId in map)) return true; // 本来就不在，视作已达成
  delete map[reviewId];
  return saveFavorites(map);
}

// 备注的增/改/删都走这一个入口：传空串（或全空白）即删除备注。
// 未收藏时 map 里没有该 id，直接跳过。
export function setFavoriteNote(reviewId: string, note: string): boolean {
  const map = loadFavorites();
  const hit = map[reviewId];
  if (!hit) return false;
  const text = note.trim();
  if (text) {
    hit.note = text;
    hit.noteUpdatedAt = Date.now();
  } else {
    delete hit.note;
    delete hit.noteUpdatedAt;
  }
  return saveFavorites(map);
}

// 回复提交成功后顺带同步收藏夹里的这条（如果它被收藏了），避免收藏页显示过时的
// 回复状态。未收藏时 map 里找不到该 id，直接跳过。
export function updateFavoriteReply(reviewId: string, replyText: string, ts: number): void {
  const map = loadFavorites();
  const hit = map[reviewId];
  if (!hit) return;
  hit.developer_reply = replyText;
  hit.developer_reply_ts = ts;
  saveFavorites(map);
}

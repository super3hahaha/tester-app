// 收藏评论：把评论加星收藏，供 Review 工作区「收藏评论」tab 快速回看。
// 存整条评论快照（而非只存 id）——即使原评论超出 API 7 天窗口、本地快照被覆盖，
// 收藏夹里仍能看到收藏时的完整内容。按账号隔离（scopedKey），键为 review_id。

import { scopedKey } from "./accountScopedKey";

const FAV_KEY = "review-fav-v1";

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
}

function storeKey(): string {
  return scopedKey(FAV_KEY);
}

export function loadFavorites(): Record<string, FavoriteReview> {
  try {
    const raw = localStorage.getItem(storeKey());
    if (!raw) return {};
    const obj = JSON.parse(raw) as unknown;
    return obj && typeof obj === "object" ? (obj as Record<string, FavoriteReview>) : {};
  } catch {
    return {};
  }
}

function saveFavorites(map: Record<string, FavoriteReview>): void {
  localStorage.setItem(storeKey(), JSON.stringify(map));
}

export function addFavorite(review: FavoriteReview): void {
  const map = loadFavorites();
  map[review.review_id] = review;
  saveFavorites(map);
}

export function removeFavorite(reviewId: string): void {
  const map = loadFavorites();
  if (reviewId in map) {
    delete map[reviewId];
    saveFavorites(map);
  }
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

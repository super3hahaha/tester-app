// 批量回复草稿的本地持久化：只为「已生成但尚未提交」的候选兜底，避免 app 进程
// 意外重启（假期期间系统更新/崩溃）导致一批 AI 生成的草稿全部丢失、要重新调
// 一遍 API。数据只存本地 localStorage，按账号分区（不分包名，review_id 本身
// 在账号内已唯一，与 BatchReplyPage 里 MANUAL_KEY 的存法一致）。
//
// 口径与评论快照一致：每轮批量生成结束后，把这一轮的结果整体合并覆盖写入
// （同 review_id 直接覆盖旧值），不做逐字符实时保存；提交成功后由调用方删除
// 对应条目，避免无限堆积，也避免评论后续再次出现时被过期草稿污染。

import { scopedKey } from "./accountScopedKey";

export interface DraftEntry {
  replyText: string;
  options: unknown[]; // 结构同 BatchReplyPage 的 ReplyOption[]
  selectedIdx: number;
  unmatched: boolean;
  errorMsg: string;
}

const DRAFTS_KEY = "batch-reply-drafts-v1";

function persist(map: Record<string, DraftEntry>) {
  try {
    localStorage.setItem(scopedKey(DRAFTS_KEY), JSON.stringify(map));
  } catch (e) {
    console.warn("save batch-reply drafts failed:", e);
  }
}

export function loadDrafts(): Record<string, DraftEntry> {
  try {
    const raw = JSON.parse(localStorage.getItem(scopedKey(DRAFTS_KEY)) || "{}");
    return raw && typeof raw === "object" ? raw : {};
  } catch {
    return {};
  }
}

/** 把一轮生成的结果合并进持久化存储（同 id 覆盖）。 */
export function mergeDrafts(entries: Record<string, DraftEntry>) {
  if (Object.keys(entries).length === 0) return;
  const map = loadDrafts();
  Object.assign(map, entries);
  persist(map);
}

/** 提交成功后清掉这条持久化草稿。 */
export function removeDraft(reviewId: string) {
  const map = loadDrafts();
  if (reviewId in map) {
    delete map[reviewId];
    persist(map);
  }
}

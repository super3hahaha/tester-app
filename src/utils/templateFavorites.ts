// 模板「收藏」：把常用模板的 id 标星，供「模板回复」弹窗快速取用。
// 模板 id 全局唯一（前缀编码产品，如 common-001 / video2mp3-003），所以用一个扁平
// id 集合就够，弹窗按产品加载模板后据此过滤即可。沿用项目里其它 id 列表的 localStorage
// 持久化口径（gmail-read-ids-v1 / batch-reply-manual-ids-v1 同套路）。

const FAV_KEY = "tpl-fav-ids-v1";

export function loadFavIds(): Set<string> {
  try {
    const raw = localStorage.getItem(FAV_KEY);
    if (!raw) return new Set();
    const arr = JSON.parse(raw) as unknown;
    return Array.isArray(arr) ? new Set(arr.filter((x) => typeof x === "string")) : new Set();
  } catch {
    return new Set();
  }
}

export function saveFavIds(ids: Set<string>): void {
  localStorage.setItem(FAV_KEY, JSON.stringify([...ids]));
}

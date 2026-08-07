// 收藏邮件：把邮件加星收藏，供「邮件 → 收藏邮件」页快速回看。
// 存整封邮件快照（而非只存 id）——邮件标为已读后会从列表隐藏、源表分页也可能被脚本重写，
// 收藏夹里仍能看到收藏时的完整内容。
//
// ⚠️ 这里【不按账号隔离】（不用 scopedKey），和收藏评论(reviewFavorites)不同。
// 原因：邮件源本身就是全局的（gmail-sources-v1 / gmail-read-ids-v1 都没 scoped），
// 一封邮件属于哪个 Gmail 账号由邮件源决定，跟 app 当前登录的 Google 账号无关；
// 若在这里 scope 一层，切账号后收藏夹会莫名其妙变空。

import { ref } from "vue";
import { loadMapSafe, saveMapSafe } from "./favoritesStorage";

const FAV_KEY = "mail-fav-v1";

// 最近一次读/写出的问题，供页面显示 banner。写成功即清空。
// 读到损坏时会一直有值（每次 loadFavorites 都会重新设上），直到数据被处理 + 重启。
export const favoritesError = ref("");

export interface FavoriteMail {
  messageId: string;
  threadId: string;
  date: string;
  from: string;
  subject: string;
  body: string;
  translated: string;
  attachments: string;
  link: string;
  // 收藏时所属邮件源的快照：收藏页要靠 profileDir 决定用哪个 Chrome 资料打开 Gmail，
  // 靠 sourceKey 分 tab、sourceLabel+sourceTab 拼显示名（与邮件源下拉框同款「邮箱 · 分页」）。
  // 源之后被删掉也不影响已收藏的这份。
  _sourceKey: string;
  _sourceLabel: string;
  _sourceTab?: string; // 源的分页名（同一张表按分页拆成多条源，光看 label 分不出来）
  _profileDir: string;
  favoritedAt: number;
  // 备注：收藏后自己补的笔记，可空（旧记录没有这两个字段）。
  // 存在收藏记录内部而非独立表——取消收藏即连带删掉备注，不留孤儿数据。
  note?: string;
  noteUpdatedAt?: number;
}

// 表里少数行可能没有 messageId（脚本旧版本写的行），退回用邮件链接做键。
export function mailFavKey(m: { messageId?: string; link?: string }): string {
  return m.messageId || m.link || "";
}

export function loadFavorites(): Record<string, FavoriteMail> {
  const { map, error } = loadMapSafe<FavoriteMail>(FAV_KEY);
  // 只在有问题时设置：读正常不清空，否则会把刚刚写失败的提示冲掉
  // （写失败的调用方紧接着就会 loadList() 重刷列表）。
  if (error) favoritesError.value = error;
  return map;
}

// 返回是否真的写进去了。false 时 favoritesError 已带上原因，调用方应据此
// 放弃本地状态更新（别让 UI 显示成功而磁盘上没有）。
function saveFavorites(map: Record<string, FavoriteMail>): boolean {
  const { ok, error } = saveMapSafe(FAV_KEY, map);
  favoritesError.value = error;
  return ok;
}

export function addFavorite(mail: FavoriteMail): boolean {
  const key = mailFavKey(mail);
  if (!key) return false;
  const map = loadFavorites();
  map[key] = mail;
  return saveFavorites(map);
}

export function removeFavorite(key: string): boolean {
  const map = loadFavorites();
  if (!(key in map)) return true; // 本来就不在，视作已达成
  delete map[key];
  return saveFavorites(map);
}

// 备注的增/改/删都走这一个入口：传空串（或全空白）即删除备注。
// 未收藏时 map 里没有该 key，直接跳过。
export function setFavoriteNote(key: string, note: string): boolean {
  const map = loadFavorites();
  const hit = map[key];
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

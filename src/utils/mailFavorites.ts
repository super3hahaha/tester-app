// 收藏邮件：把邮件加星收藏，供「邮件 → 收藏邮件」页快速回看。
// 存整封邮件快照（而非只存 id）——邮件标为已读后会从列表隐藏、源表分页也可能被脚本重写，
// 收藏夹里仍能看到收藏时的完整内容。
//
// ⚠️ 这里【不按账号隔离】（不用 scopedKey），和收藏评论(reviewFavorites)不同。
// 原因：邮件源本身就是全局的（gmail-sources-v1 / gmail-read-ids-v1 都没 scoped），
// 一封邮件属于哪个 Gmail 账号由邮件源决定，跟 app 当前登录的 Google 账号无关；
// 若在这里 scope 一层，切账号后收藏夹会莫名其妙变空。

const FAV_KEY = "mail-fav-v1";

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
}

// 表里少数行可能没有 messageId（脚本旧版本写的行），退回用邮件链接做键。
export function mailFavKey(m: { messageId?: string; link?: string }): string {
  return m.messageId || m.link || "";
}

export function loadFavorites(): Record<string, FavoriteMail> {
  try {
    const raw = localStorage.getItem(FAV_KEY);
    if (!raw) return {};
    const obj = JSON.parse(raw) as unknown;
    return obj && typeof obj === "object" ? (obj as Record<string, FavoriteMail>) : {};
  } catch {
    return {};
  }
}

function saveFavorites(map: Record<string, FavoriteMail>): void {
  localStorage.setItem(FAV_KEY, JSON.stringify(map));
}

export function addFavorite(mail: FavoriteMail): void {
  const key = mailFavKey(mail);
  if (!key) return;
  const map = loadFavorites();
  map[key] = mail;
  saveFavorites(map);
}

export function removeFavorite(key: string): void {
  const map = loadFavorites();
  if (key in map) {
    delete map[key];
    saveFavorites(map);
  }
}

// 收藏类数据（评论/邮件）在 localStorage 上的安全读写层。
//
// 要解决的是「静默丢数据」：整个收藏夹是一行 JSON 存在单个 key 里，读的时候
// 解析失败就返回 {} 当作「没有收藏」，紧接着任何一次写（收藏/取消/改备注）都会把
// 这个空 map 整个覆盖回去——残存的数据一次性没了，且全程无提示。
//
// 这里的做法：
//   1. 读不出来 → 把原始值隔离备份一份，并把该 key 标记为「保护中」；
//   2. 保护中的 key 一律拒绝写入，宁可这次操作失败，也不覆盖掉可能还能救的数据；
//   3. 写失败（保护中 / 配额满 / 其它）都返回明确原因，由调用方显示给用户。
//
// 保护标记是内存态（模块级 Set），重启 app 即清；人工修好数据后重启即可恢复写入。

const quarantined = new Set<string>();

// 损坏原值的备份 key。固定后缀而非带时间戳：只保留第一份——首次隔离到的那份
// 最接近损坏前的完整数据，后续再坏也不该覆盖它。
export function corruptBackupKey(key: string): string {
  return `${key}__corrupt`;
}

function quarantine(key: string, raw: string): string {
  quarantined.add(key);
  const bk = corruptBackupKey(key);
  try {
    // 已有备份就不动，保住最早那份
    if (localStorage.getItem(bk) === null) {
      localStorage.setItem(bk, JSON.stringify({ at: Date.now(), raw }));
    }
  } catch {
    // 连备份都写不下就算了：quarantined 已经拦住覆盖写，原值还在原 key 上
  }
  return (
    `收藏数据读取失败（内容可能已损坏），已暂停写入以免覆盖残存数据。` +
    `原始内容备份在 localStorage 的「${bk}」，处理后重启 app 恢复。`
  );
}

export interface LoadResult<T> {
  map: Record<string, T>;
  error: string; // 空串表示正常
}

export function loadMapSafe<T>(key: string): LoadResult<T> {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(key);
  } catch (e: any) {
    return { map: {}, error: "读取本地存储失败：" + String(e?.message || e) };
  }

  // 从没存过 ≠ 损坏（全新账号、清空后都是正常状态）——顺带解除历史保护标记
  if (raw === null) {
    quarantined.delete(key);
    return { map: {}, error: "" };
  }

  try {
    const obj = JSON.parse(raw) as unknown;
    // null / 数字 / 数组都过不了这关。旧代码对这几种只是悄悄返回 {}，
    // 然后被下一次写覆盖——正是丢数据的路径，这里一律按损坏处理。
    if (!obj || typeof obj !== "object" || Array.isArray(obj)) {
      throw new Error("unexpected shape");
    }
    quarantined.delete(key);
    return { map: obj as Record<string, T>, error: "" };
  } catch {
    return { map: {}, error: quarantine(key, raw) };
  }
}

export interface SaveResult {
  ok: boolean;
  error: string; // ok 为 true 时是空串
}

// 各浏览器/WebView 对配额超限的报法不一，都归一成一句人话
function isQuotaError(e: any): boolean {
  return (
    e?.name === "QuotaExceededError" ||
    e?.name === "NS_ERROR_DOM_QUOTA_REACHED" ||
    e?.code === 22 ||
    e?.code === 1014
  );
}

export function saveMapSafe<T>(key: string, map: Record<string, T>): SaveResult {
  if (quarantined.has(key)) {
    return {
      ok: false,
      error:
        `收藏数据处于损坏保护状态，本次修改未保存（避免覆盖掉可能还能恢复的数据）。` +
        `备份见 localStorage 的「${corruptBackupKey(key)}」，处理后重启 app。`,
    };
  }
  try {
    localStorage.setItem(key, JSON.stringify(map));
    return { ok: true, error: "" };
  } catch (e: any) {
    return {
      ok: false,
      error: isQuotaError(e)
        ? "本地存储空间已满，本次修改未保存。请先取消一些收藏（邮件收藏存的是整封正文快照，比较占空间）后重试。"
        : "保存失败：" + String(e?.message || e),
    };
  }
}

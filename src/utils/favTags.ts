// 收藏「标签」库：用户自定义的一套标签，收藏评论和收藏邮件共用。
//
// 设计要点（见 docs/handoff-favorite-tags.md）：
//   - **全局不按账号隔离**（不用 scopedKey）：邮件收藏本身是全局的，标签库 scope 后切账号
//     邮件上的标签 id 会失联。评论收藏虽然按账号隔离，标签只是一套词汇，跟账号无关。
//   - 收藏记录里存的是标签 **id**（`tags?: string[]`）：改名零成本；删标签不回写收藏记录，
//     展示/筛选时用 resolveTags() 过滤掉已不存在的 id。
//   - 模块级共享响应式 ref：四个页面（评论页/邮件页/两个收藏页）读同一份，任一页新建/改名
//     其它页立刻同步（与 reviewReadMarks 同样的路子）。
//   - 读写经 favoritesStorage 安全层，写失败返回 false，原因走 favTagsError。
//   - 适用范围 `apps`：空/缺省 = 通用（所有 app 可用）；否则只在这些包名的评论/邮件上出现。
//     旧标签没有这个字段 = 通用，不迁移。标签名仍全局唯一（跨 app 共用的就设通用）。

import { ref } from "vue";
import { loadMapSafe, saveMapSafe } from "./favoritesStorage";

const TAGS_KEY = "fav-tags-v1";
const LABEL = "标签数据";

export interface FavTag {
  id: string;
  name: string;
  color: string;
  createdAt: number;
  apps?: string[]; // 适用 app 包名；空/缺省 = 通用
}

// 新建标签按顺序轮流取色；用户可在「管理标签」里手动改
export const TAG_PALETTE = [
  "#3b82f6", "#ef4444", "#10b981", "#f59e0b", "#8b5cf6",
  "#ec4899", "#14b8a6", "#f97316", "#6366f1", "#84cc16",
];

/** 最近一次读/写出的问题，供页面 banner 显示；写成功即清空。 */
export const favTagsError = ref("");

/** 全部标签（id → FavTag），模块级共享响应式。 */
export const favTags = ref<Record<string, FavTag>>({});

let loaded = false;

function persist(map: Record<string, FavTag>): boolean {
  const { ok, error } = saveMapSafe(TAGS_KEY, map, LABEL);
  favTagsError.value = error;
  return ok;
}

/** 从 localStorage 重读（页面 onMounted/onActivated 调用即可，重复调用无副作用）。 */
export function reloadTags(): void {
  const { map, error } = loadMapSafe<FavTag>(TAGS_KEY, LABEL);
  if (error) favTagsError.value = error;
  favTags.value = map;
  loaded = true;
}

function ensureLoaded() {
  if (!loaded) reloadTags();
}

/** 按创建时间排序的标签列表（UI 展示顺序）。 */
export function sortedTags(): FavTag[] {
  ensureLoaded();
  return Object.values(favTags.value).sort((a, b) => a.createdAt - b.createdAt);
}

/** 把记录上的 id 列表解析成标签对象，丢掉已删除的 id，按标签库顺序排。 */
export function resolveTags(ids: string[] | undefined): FavTag[] {
  if (!ids || !ids.length) return [];
  ensureLoaded();
  const set = new Set(ids);
  return sortedTags().filter((t) => set.has(t.id));
}

/** 记录上是否还有「有效」标签（已删除的 id 不算）。 */
export function hasLiveTags(ids: string[] | undefined): boolean {
  return resolveTags(ids).length > 0;
}

function normName(s: string): string {
  return s.trim().toLowerCase();
}

/** 同名（trim + 不区分大小写）的标签，exceptId 用于改名时排除自己。 */
export function findTagByName(name: string, exceptId = ""): FavTag | undefined {
  ensureLoaded();
  const n = normName(name);
  return Object.values(favTags.value).find((t) => t.id !== exceptId && normName(t.name) === n);
}

/**
 * 新建标签。成功返回新标签；名字为空/重名/写失败返回 null，原因写进 favTagsError。
 */
export function addTag(name: string, apps: string[] = []): FavTag | null {
  ensureLoaded();
  const text = name.trim();
  if (!text) {
    favTagsError.value = "标签名不能为空";
    return null;
  }
  if (findTagByName(text)) {
    favTagsError.value = `已有同名标签「${text}」`;
    return null;
  }
  const count = Object.keys(favTags.value).length;
  const tag: FavTag = {
    id: "t_" + Date.now().toString(36) + Math.random().toString(36).slice(2, 6),
    name: text,
    color: TAG_PALETTE[count % TAG_PALETTE.length],
    createdAt: Date.now(),
  };
  const scope = Array.from(new Set(apps.filter(Boolean)));
  if (scope.length) tag.apps = scope;
  const map = { ...favTags.value, [tag.id]: tag };
  if (!persist(map)) return null;
  favTags.value = map;
  return tag;
}

export function renameTag(id: string, name: string): boolean {
  ensureLoaded();
  const hit = favTags.value[id];
  if (!hit) return false;
  const text = name.trim();
  if (!text) {
    favTagsError.value = "标签名不能为空";
    return false;
  }
  if (findTagByName(text, id)) {
    favTagsError.value = `已有同名标签「${text}」`;
    return false;
  }
  const map = { ...favTags.value, [id]: { ...hit, name: text } };
  if (!persist(map)) return false;
  favTags.value = map;
  return true;
}

export function setTagColor(id: string, color: string): boolean {
  ensureLoaded();
  const hit = favTags.value[id];
  if (!hit) return false;
  const map = { ...favTags.value, [id]: { ...hit, color } };
  if (!persist(map)) return false;
  favTags.value = map;
  return true;
}

/** 改适用范围：空数组 = 通用。 */
export function setTagApps(id: string, apps: string[]): boolean {
  ensureLoaded();
  const hit = favTags.value[id];
  if (!hit) return false;
  const next: FavTag = { ...hit };
  const scope = Array.from(new Set(apps.filter(Boolean)));
  if (scope.length) next.apps = scope;
  else delete next.apps;
  const map = { ...favTags.value, [id]: next };
  if (!persist(map)) return false;
  favTags.value = map;
  return true;
}

export function isGeneralTag(t: FavTag): boolean {
  return !t.apps || t.apps.length === 0;
}

/** 某 app 下可用的标签（通用 + 适用该 app 的）；pkg 为空 = 不限 app，返回全部。 */
export function tagsForApp(pkg?: string | null): FavTag[] {
  const all = sortedTags();
  if (!pkg) return all;
  return all.filter((t) => isGeneralTag(t) || t.apps!.includes(pkg));
}

export interface TagGroup {
  key: string; // "" = 通用，否则包名
  tags: FavTag[];
}

/**
 * 按范围分组：通用在前，其余按包名分组（组顺序按 pkgOrder，不在其中的排最后）。
 * 同时适用多个 app 的标签会出现在每个组里。
 */
export function groupTags(tags: FavTag[], pkgOrder: string[] = []): TagGroup[] {
  const general: FavTag[] = [];
  const byApp = new Map<string, FavTag[]>();
  for (const t of tags) {
    if (isGeneralTag(t)) {
      general.push(t);
      continue;
    }
    for (const p of t.apps!) {
      if (!byApp.has(p)) byApp.set(p, []);
      byApp.get(p)!.push(t);
    }
  }
  const rank = (p: string) => {
    const i = pkgOrder.indexOf(p);
    return i < 0 ? Number.MAX_SAFE_INTEGER : i;
  };
  const keys = [...byApp.keys()].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
  const out: TagGroup[] = [];
  if (general.length) out.push({ key: "", tags: general });
  for (const k of keys) out.push({ key: k, tags: byApp.get(k)! });
  return out;
}

/** 删标签只动标签库，不回写收藏记录（记录里残留的 id 由 resolveTags 过滤）。 */
export function removeTag(id: string): boolean {
  ensureLoaded();
  if (!(id in favTags.value)) return true;
  const map = { ...favTags.value };
  delete map[id];
  if (!persist(map)) return false;
  favTags.value = map;
  return true;
}

/** 收藏页筛选用的特殊值：「无标签」。 */
export const NO_TAG_FILTER = "__none__";

/**
 * 收藏页标签筛选：selected 为空 = 不筛；命中任意一个（OR）即通过；
 * 选中 NO_TAG_FILTER 时，没有有效标签的记录也通过。
 */
export function passesTagFilter(ids: string[] | undefined, selected: string[]): boolean {
  if (!selected.length) return true;
  const live = resolveTags(ids);
  if (selected.includes(NO_TAG_FILTER) && live.length === 0) return true;
  return live.some((t) => selected.includes(t.id));
}

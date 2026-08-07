<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { scopedKey } from "../utils/accountScopedKey";
import { loadFavorites, removeFavorite, type FavoriteReview } from "../utils/reviewFavorites";

const props = defineProps<{ activeOption?: string }>();

// Play Console 深链要用到 developerId / appIdByPkg，这两个字段由 ReviewPage.vue 维护
// 在 `review-page-config-v3` 里（同一账号命名空间）。这里只读，不写，避免跨页面耦合写逻辑。
const REVIEW_CONFIG_KEY = "review-page-config-v3";
interface ReviewPageConfig {
  developerId?: string;
  appIdByPkg?: Record<string, string>;
}

const favorites = ref<FavoriteReview[]>([]);
const developerId = ref("");
const appIdByPkg = ref<Record<string, string>>({});

// 按来源 app 分 tab（"全部" + 每个有收藏的 app，用 _pkg 做 key 避免同名应用冲突）
const activeTab = ref("all");
interface AppTab { pkg: string; label: string; count: number }
const appTabs = computed<AppTab[]>(() => {
  const map = new Map<string, AppTab>();
  for (const r of favorites.value) {
    const hit = map.get(r._pkg);
    if (hit) hit.count++;
    else map.set(r._pkg, { pkg: r._pkg, label: r._app || r._pkg, count: 1 });
  }
  return Array.from(map.values()).sort((a, b) => a.label.localeCompare(b.label));
});
const filteredFavorites = computed(() =>
  activeTab.value === "all"
    ? favorites.value
    : favorites.value.filter((r) => r._pkg === activeTab.value)
);

function loadList() {
  const map = loadFavorites();
  favorites.value = Object.values(map).sort((a, b) => b.favoritedAt - a.favoritedAt);
  // 该 app 的收藏可能已被全部取消——切回「全部」而不是停在一个空 tab 上
  if (activeTab.value !== "all" && !favorites.value.some((r) => r._pkg === activeTab.value)) {
    activeTab.value = "all";
  }
}

function loadConsoleConfig() {
  try {
    const raw = localStorage.getItem(scopedKey(REVIEW_CONFIG_KEY));
    if (!raw) return;
    const cfg = JSON.parse(raw) as ReviewPageConfig;
    developerId.value = cfg.developerId || "";
    appIdByPkg.value = cfg.appIdByPkg || {};
  } catch {
    // ignore corrupt config
  }
}

onMounted(() => {
  loadList();
  loadConsoleConfig();
});

// MainPage 用 v-show，组件常驻不重新 mount；从 ReviewPage 收藏后切回本页时刷新一次
watch(
  () => props.activeOption,
  (v) => {
    if (v === "review-favorites") {
      loadList();
      loadConsoleConfig();
    }
  }
);

function unfavorite(r: FavoriteReview) {
  removeFavorite(r.review_id);
  loadList();
}

function reviewConsoleUrl(r: FavoriteReview): string | null {
  const id = appIdByPkg.value[r._pkg];
  if (!developerId.value || !id) return null;
  const base = `https://play.google.com/console/u/0/developers/${developerId.value}/app/${id}/user-feedback/review-details`;
  return `${base}?reviewId=${encodeURIComponent(r.review_id)}&corpus=PUBLIC_REVIEWS`;
}

async function openInConsole(r: FavoriteReview) {
  const url = reviewConsoleUrl(r);
  if (!url) return;
  try {
    await openUrl(url);
  } catch {
    // ignore
  }
}

// Play reviews API 的 androidOsVersion 是 API Level，映射成用户看得懂的版本号
// （与 ReviewPage.vue 同一张表，保持独立文件不跨组件依赖内部实现）。
const ANDROID_API_TO_VERSION: Record<number, string> = {
  16: "4.1", 17: "4.2", 18: "4.3", 19: "4.4", 21: "5.0", 22: "5.1",
  23: "6", 24: "7.0", 25: "7.1", 26: "8.0", 27: "8.1", 28: "9",
  29: "10", 30: "11", 31: "12", 32: "12L", 33: "13", 34: "14",
  35: "15", 36: "16",
};
function androidVersionLabel(api: number): string {
  const v = ANDROID_API_TO_VERSION[api];
  return v ? `Android ${v}` : `Android API ${api}`;
}

function formatTs(ts: number): string {
  if (!ts) return "";
  const d = new Date(ts * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function starsDisplay(n: number): string {
  return "★".repeat(n) + "☆".repeat(5 - n);
}
</script>

<template>
  <div class="fav-page">
    <header class="page-header">
      <h3>收藏评论</h3>
      <p class="subtitle">
        在「Play Console」评论列表里点 ☆ 收藏的评论会出现在这里，星标本身即收藏状态，再点一次取消收藏。
      </p>
    </header>

    <div v-if="favorites.length === 0" class="empty-state">
      还没有收藏任何评论。去「Play Console」评论列表点评论右下角的 ☆ 收藏。
    </div>

    <template v-else>
      <div class="app-tabs">
        <button
          class="app-tab"
          :class="{ active: activeTab === 'all' }"
          @click="activeTab = 'all'"
        >全部 <span class="tab-count">{{ favorites.length }}</span></button>
        <button
          v-for="t in appTabs"
          :key="t.pkg"
          class="app-tab"
          :class="{ active: activeTab === t.pkg }"
          @click="activeTab = t.pkg"
        >{{ t.label }} <span class="tab-count">{{ t.count }}</span></button>
      </div>

      <div v-if="filteredFavorites.length === 0" class="empty-state">
        该应用下暂无收藏评论。
      </div>
      <div v-else class="review-list">
        <article v-for="r in filteredFavorites" :key="r.review_id" class="review-card">
          <div class="review-head">
            <span class="app-badge">{{ r._app }}</span>
            <span class="stars" :class="`stars-${r.star_rating}`">{{ starsDisplay(r.star_rating) }}</span>
            <span class="author">{{ r.author_name || "(匿名)" }}</span>
            <span class="ts">{{ formatTs(r.user_comment_ts) }}</span>
            <span v-if="r.developer_reply" class="reply-tag replied">已回复</span>
            <span v-else class="reply-tag absent">未回复</span>
          </div>
          <div class="review-text">{{ r.text || "(无文字)" }}</div>
          <div v-if="r.original_text" class="review-original">
            <span class="orig-label">原文：</span>{{ r.original_text }}
          </div>
          <div class="review-meta">
            <span v-if="r.app_version_name">v{{ r.app_version_name }}<span v-if="r.app_version_code"> ({{ r.app_version_code }})</span></span>
            <span v-if="r.device">{{ r.device }}</span>
            <span v-if="r.android_os_version">{{ androidVersionLabel(r.android_os_version) }}</span>
            <span v-if="r.reviewer_language">lang: {{ r.reviewer_language }}</span>
            <span v-if="r.thumbs_up_count">👍 {{ r.thumbs_up_count }}</span>
            <span v-if="r.thumbs_down_count">👎 {{ r.thumbs_down_count }}</span>
          </div>
          <div v-if="r.developer_reply" class="reply-block">
            <div class="reply-head">
              <span class="reply-label">开发者回复</span>
              <span v-if="r.developer_reply_ts" class="reply-ts">{{ formatTs(r.developer_reply_ts) }}</span>
            </div>
            <div class="reply-text">{{ r.developer_reply }}</div>
          </div>
          <div class="review-actions">
            <button class="fav-star-btn active" @click="unfavorite(r)" title="取消收藏">★</button>
            <button
              v-if="reviewConsoleUrl(r)"
              class="web-btn"
              @click="openInConsole(r)"
              title="在 Play Console 中打开该评论"
            >
              🌐 在网页中打开
            </button>
          </div>
        </article>
      </div>
    </template>
  </div>
</template>

<style scoped>
.fav-page {
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 16px 20px;
  overflow-y: auto;
}
.page-header h3 {
  margin: 0;
  font-size: 16px;
}
.subtitle {
  margin: 4px 0 16px 0;
  font-size: 12px;
  color: #888;
}
.empty-state {
  padding: 30px 16px;
  text-align: center;
  font-size: 13px;
  color: #999;
}
.app-tabs {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 12px;
}
.app-tab {
  padding: 6px 14px;
  border: 1px solid #e2e8f0;
  border-radius: 18px;
  background: white;
  font-size: 13px;
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 6px;
}
.app-tab:hover {
  background: #f7fafc;
}
.app-tab.active {
  border-color: #667eea;
  background: #667eea;
  color: white;
}
.tab-count {
  font-size: 11px;
  background: rgba(0, 0, 0, 0.08);
  border-radius: 10px;
  padding: 0 7px;
  line-height: 16px;
}
.app-tab.active .tab-count {
  background: rgba(255, 255, 255, 0.25);
}
.review-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.review-card {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 12px 14px;
  background: white;
}
.review-head {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  margin-bottom: 6px;
}
.app-badge {
  font-size: 10px;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 10px;
  background: #e9d8fd;
  color: #553c9a;
}
.stars {
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  letter-spacing: 1px;
  font-size: 13px;
}
.stars-1, .stars-2 { color: #e53e3e; }
.stars-3 { color: #d69e2e; }
.stars-4, .stars-5 { color: #38a169; }
.author {
  font-size: 13px;
  font-weight: 500;
  color: #2d3748;
}
.ts {
  font-size: 11px;
  color: #999;
}
.reply-tag {
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 10px;
  font-weight: 600;
}
.reply-tag.replied {
  background: #e6fffa;
  color: #234e52;
}
.reply-tag.absent {
  background: #fff5f5;
  color: #9b2c2c;
}
.review-text {
  font-size: 13px;
  line-height: 1.55;
  color: #2d3748;
  white-space: pre-wrap;
  word-break: break-word;
}
.review-original {
  margin-top: 6px;
  font-size: 12px;
  color: #777;
  line-height: 1.5;
  background: #fafafa;
  border-left: 2px solid #ddd;
  padding: 4px 10px;
  border-radius: 0 4px 4px 0;
}
.orig-label {
  color: #999;
  font-size: 11px;
}
.review-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  margin-top: 8px;
  font-size: 11px;
  color: #888;
}
.reply-block {
  margin-top: 10px;
  padding: 8px 12px;
  background: #f7fafc;
  border-left: 3px solid #4299e1;
  border-radius: 0 6px 6px 0;
}
.reply-head {
  display: flex;
  gap: 10px;
  align-items: center;
  margin-bottom: 4px;
}
.reply-label {
  font-size: 11px;
  font-weight: 600;
  color: #2b6cb0;
}
.reply-ts {
  font-size: 11px;
  color: #999;
}
.reply-text {
  font-size: 12px;
  color: #4a5568;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}
.review-actions {
  margin-top: 10px;
  display: flex;
  justify-content: flex-end;
  align-items: center;
}
.fav-star-btn {
  border: none;
  background: transparent;
  font-size: 18px;
  line-height: 1;
  padding: 2px 4px;
  cursor: pointer;
  color: #cbd5e0;
}
.fav-star-btn:hover {
  color: #d69e2e;
}
.fav-star-btn.active {
  color: #d69e2e;
}
.web-btn {
  padding: 4px 12px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.web-btn:hover {
  background: #f5f5fa;
  border-color: #cbd5e0;
  color: #2d3748;
}
</style>

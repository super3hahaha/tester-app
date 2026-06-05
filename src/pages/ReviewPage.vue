<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

interface PlayApp {
  package_name: string;
  display_name: string;
}

interface Review {
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

type ReplyState = "ANY" | "ABSENT" | "REPLIED";

interface PersistedConfig {
  packageName: string;
  developerId: string;
  appId: string;
  stars: number[];
  replyState: ReplyState;
  apps: PlayApp[];
}

const STORAGE_KEY = "review-page-config-v3";

const apps = ref<PlayApp[]>([]);
const appsLoading = ref(false);
const appsError = ref("");
const manualPackage = ref(false);

const packageName = ref("files.fileexplorer.filemanager");
const developerId = ref("7240883491244732024");
const appId = ref("4973223441657725559");

const stars = ref<number[]>([1, 2, 3, 4]);
const replyState = ref<ReplyState>("ABSENT");

function todayIso(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}
function daysAgoIso(n: number): string {
  const d = new Date();
  d.setDate(d.getDate() - n);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

const fromDate = ref(daysAgoIso(7));
const toDate = ref(todayIso());

const reviews = ref<Review[]>([]);
const loading = ref(false);
const errorMsg = ref("");
const needRelogin = ref(false);
const fetchedAt = ref<number | null>(null);
const showAdvanced = ref(false);

onMounted(async () => {
  const raw = localStorage.getItem(STORAGE_KEY);
  if (raw) {
    try {
      const cfg = JSON.parse(raw) as PersistedConfig;
      if (cfg.packageName) packageName.value = cfg.packageName;
      if (cfg.developerId) developerId.value = cfg.developerId;
      if (cfg.appId) appId.value = cfg.appId;
      if (Array.isArray(cfg.stars) && cfg.stars.length > 0) stars.value = cfg.stars;
      if (cfg.replyState) replyState.value = cfg.replyState;
      if (Array.isArray(cfg.apps)) apps.value = cfg.apps;
    } catch {
      // ignore corrupt cache
    }
  }
  loadApps();
});

watch([packageName, developerId, appId, stars, replyState, apps], () => {
  const cfg: PersistedConfig = {
    packageName: packageName.value,
    developerId: developerId.value,
    appId: appId.value,
    stars: stars.value,
    replyState: replyState.value,
    apps: apps.value,
  };
  localStorage.setItem(STORAGE_KEY, JSON.stringify(cfg));
}, { deep: true });

async function loadApps() {
  appsLoading.value = true;
  appsError.value = "";
  try {
    const list = await invoke<PlayApp[]>("list_play_apps");
    apps.value = list;
    // If the saved package is not in the list, ensure dropdown still shows something.
    if (list.length > 0 && !list.some((a) => a.package_name === packageName.value)) {
      // Keep whatever the user had — but if it was the default and doesn't match,
      // auto-pick the first app so the form is in a valid state.
      if (packageName.value === "files.fileexplorer.filemanager" && !list.some((a) => a.package_name === "files.fileexplorer.filemanager")) {
        packageName.value = list[0].package_name;
      }
    }
  } catch (e: any) {
    const msg = String(e);
    if (msg.startsWith("NEED_RELOGIN_SCOPE")) {
      appsError.value = "需要重新登录授权 playdeveloperreporting 权限。";
      needRelogin.value = true;
    } else {
      appsError.value = msg;
    }
  } finally {
    appsLoading.value = false;
  }
}

function toggleStar(s: number) {
  const i = stars.value.indexOf(s);
  if (i >= 0) stars.value.splice(i, 1);
  else stars.value.push(s);
  stars.value.sort();
}

function setDatePreset(days: number) {
  fromDate.value = daysAgoIso(days - 1);
  toDate.value = todayIso();
}

async function handleFetch() {
  if (loading.value || !packageName.value.trim()) return;
  loading.value = true;
  errorMsg.value = "";
  needRelogin.value = false;
  reviews.value = [];
  try {
    const list = await invoke<Review[]>("list_play_reviews", {
      packageName: packageName.value.trim(),
      maxPages: 5,
      translationLanguage: "zh-CN",
    });
    reviews.value = list;
    fetchedAt.value = Date.now();
  } catch (e: any) {
    const msg = String(e);
    if (msg.startsWith("NEED_RELOGIN_SCOPE")) {
      needRelogin.value = true;
      errorMsg.value = "需要重新登录授权 androidpublisher 权限。请右上角点 Logout 后重新登录。";
    } else if (msg.startsWith("NEED_RELOGIN:")) {
      needRelogin.value = true;
      errorMsg.value = "登录已失效，请右上角点 Logout 后重新登录。";
    } else {
      errorMsg.value = msg;
    }
  } finally {
    loading.value = false;
  }
}

const fromTs = computed(() => {
  if (!fromDate.value) return 0;
  return Math.floor(new Date(fromDate.value + "T00:00:00").getTime() / 1000);
});
const toTs = computed(() => {
  if (!toDate.value) return Number.MAX_SAFE_INTEGER;
  return Math.floor(new Date(toDate.value + "T23:59:59").getTime() / 1000);
});

const filtered = computed(() => {
  return reviews.value.filter((r) => {
    if (!stars.value.includes(r.star_rating)) return false;
    if (replyState.value === "ABSENT" && r.developer_reply) return false;
    if (replyState.value === "REPLIED" && !r.developer_reply) return false;
    if (r.user_comment_ts < fromTs.value) return false;
    if (r.user_comment_ts > toTs.value) return false;
    return true;
  });
});

const dateError = computed(() =>
  fromDate.value && toDate.value && fromDate.value > toDate.value
    ? "起始日期不能晚于截止日期"
    : ""
);

const playConsoleUrl = computed(() => {
  if (!developerId.value || !appId.value) {
    return `https://play.google.com/console/u/0/developers/${developerId.value || "-"}/app-list`;
  }
  const base = `https://play.google.com/console/u/0/developers/${developerId.value}/app/${appId.value}/user-feedback/reviews`;
  const params = new URLSearchParams();
  if (stars.value.length > 0) {
    params.set("starCounts", [...stars.value].sort().join(","));
  }
  if (replyState.value !== "ANY") {
    params.set("replyState", replyState.value);
  }
  if (fromDate.value) params.set("from", fromDate.value);
  if (toDate.value) params.set("to", toDate.value);
  return `${base}?${params.toString()}`;
});

async function handleOpenInConsole() {
  try {
    await openUrl(playConsoleUrl.value);
  } catch (e: any) {
    errorMsg.value = String(e);
  }
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

const summary = computed(() => {
  if (!fetchedAt.value) return "";
  const total = reviews.value.length;
  const shown = filtered.value.length;
  return `共拉到 ${total} 条（最近 7 天），当前筛选显示 ${shown} 条`;
});

const selectedAppLabel = computed(() => {
  const a = apps.value.find((x) => x.package_name === packageName.value);
  return a ? a.display_name : "";
});
</script>

<template>
  <div class="review-page">
    <header class="page-header">
      <h3>Play Console Reviews</h3>
      <p class="subtitle">
        通过 Play Developer API 拉取最近 7 天的应用评论，在本地按星级 / 回复状态 / 日期筛选。
      </p>
    </header>

    <section class="form-card">
      <div class="form-row">
        <label class="form-label">应用</label>
        <template v-if="!manualPackage">
          <select v-model="packageName" class="app-select" :disabled="appsLoading">
            <option v-if="apps.length === 0" disabled value="">
              {{ appsLoading ? "加载中..." : "暂无应用" }}
            </option>
            <option v-for="a in apps" :key="a.package_name" :value="a.package_name">
              {{ a.display_name }} — {{ a.package_name }}
            </option>
          </select>
          <button class="icon-btn" @click="loadApps" :disabled="appsLoading" title="刷新应用列表">
            ↻
          </button>
          <button class="icon-btn" @click="manualPackage = true" title="手动输入包名">
            ✎
          </button>
        </template>
        <template v-else>
          <input v-model="packageName" class="text-input" placeholder="com.example.app" />
          <button class="icon-btn" @click="manualPackage = false" title="切回下拉选择">
            ←
          </button>
        </template>
        <button class="fetch-btn" :disabled="loading || !packageName.trim()" @click="handleFetch">
          {{ loading ? "拉取中..." : "拉取评论" }}
        </button>
      </div>
      <div v-if="appsError" class="error small">应用列表加载失败：{{ appsError }}</div>

      <div class="form-row">
        <label class="form-label">评分</label>
        <div class="star-row">
          <button
            v-for="s in [1, 2, 3, 4, 5]"
            :key="s"
            class="star-btn"
            :class="{ active: stars.includes(s) }"
            @click="toggleStar(s)"
          >{{ s }} ★</button>
        </div>
      </div>

      <div class="form-row">
        <label class="form-label">回复状态</label>
        <div class="radio-row">
          <label class="radio-item">
            <input type="radio" v-model="replyState" value="ANY" /> 全部
          </label>
          <label class="radio-item">
            <input type="radio" v-model="replyState" value="ABSENT" /> 无回复
          </label>
          <label class="radio-item">
            <input type="radio" v-model="replyState" value="REPLIED" /> 已回复
          </label>
        </div>
      </div>

      <div class="form-row">
        <label class="form-label">日期范围</label>
        <div class="date-row">
          <input type="date" v-model="fromDate" class="date-input" />
          <span class="date-sep">→</span>
          <input type="date" v-model="toDate" class="date-input" />
          <div class="presets">
            <button class="preset-btn" @click="setDatePreset(1)">今天</button>
            <button class="preset-btn" @click="setDatePreset(2)">近 2 天</button>
            <button class="preset-btn" @click="setDatePreset(7)">近 7 天</button>
          </div>
        </div>
      </div>
      <div v-if="dateError" class="error">{{ dateError }}</div>

      <div class="advanced-toggle">
        <button class="link-btn" @click="showAdvanced = !showAdvanced">
          {{ showAdvanced ? "▼" : "▶" }} Play Console 跳转设置
        </button>
      </div>
      <div v-if="showAdvanced" class="advanced-block">
        <div class="form-row">
          <label class="form-label">Developer ID</label>
          <input v-model="developerId" class="text-input" placeholder="数字 ID" />
        </div>
        <div class="form-row">
          <label class="form-label">App ID</label>
          <input v-model="appId" class="text-input" placeholder="Console URL 里的数字 ID" />
          <button class="console-btn" @click="handleOpenInConsole">
            🌐 在 Console 中打开
          </button>
        </div>
        <p class="advanced-hint">
          API 不返回这两个数字 ID，留空则跳转到应用列表页。仅在需要看 7 天以上评论或在网页回复时用得上。
        </p>
      </div>
    </section>

    <div v-if="needRelogin" class="banner banner-warn">
      ⚠️ 当前登录态不包含 Play 相关权限。请点右上角 <b>Logout</b> 后重新登录，登录页会显示新的授权请求。
    </div>

    <div v-if="errorMsg && !needRelogin" class="banner banner-error">{{ errorMsg }}</div>

    <div v-if="summary" class="summary-row">
      <span class="summary-text">{{ summary }}</span>
      <span v-if="selectedAppLabel" class="summary-app">· {{ selectedAppLabel }}</span>
    </div>

    <div v-if="filtered.length > 0" class="review-list">
      <article v-for="r in filtered" :key="r.review_id" class="review-card">
        <div class="review-head">
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
          <span v-if="r.android_os_version">Android {{ r.android_os_version }}</span>
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
      </article>
    </div>

    <div v-else-if="fetchedAt && !loading" class="empty-state">
      当前筛选条件下没有评论。试试放宽星级或扩大日期范围。
    </div>

    <div v-else-if="!fetchedAt && !loading" class="empty-state">
      选择应用后点「拉取评论」—— API 一次会拉最近 7 天全部评论，之后切换筛选不需要重新请求。
    </div>
  </div>
</template>

<style scoped>
.review-page {
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

.form-card {
  background: #fafafa;
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 14px 16px;
  margin-bottom: 12px;
}
.form-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 0;
  border-bottom: 1px dashed #ececec;
}
.form-row:last-child {
  border-bottom: none;
}
.form-label {
  width: 96px;
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
}

.app-select {
  flex: 1;
  padding: 6px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  outline: none;
  cursor: pointer;
}
.app-select:focus {
  border-color: #667eea;
}
.text-input {
  flex: 1;
  padding: 6px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
}
.text-input:focus {
  border-color: #667eea;
}
.icon-btn {
  padding: 5px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  cursor: pointer;
  color: #666;
  flex-shrink: 0;
}
.icon-btn:hover:not(:disabled) {
  background: #f5f5fa;
  color: #333;
}
.icon-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.fetch-btn {
  padding: 6px 16px;
  font-size: 13px;
  font-weight: 500;
  border: none;
  border-radius: 6px;
  background: #667eea;
  color: white;
  cursor: pointer;
  flex-shrink: 0;
}
.fetch-btn:hover:not(:disabled) {
  background: #5a67d8;
}
.fetch-btn:disabled {
  background: #ccc;
  cursor: not-allowed;
}

.star-row {
  display: flex;
  gap: 6px;
}
.star-btn {
  padding: 4px 12px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 4px;
  background: white;
  cursor: pointer;
  color: #888;
}
.star-btn.active {
  background: #f6ad55;
  border-color: #f6ad55;
  color: white;
}

.radio-row {
  display: flex;
  gap: 16px;
}
.radio-item {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  cursor: pointer;
}
.radio-item input[type="radio"] {
  margin: 0;
}

.date-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.date-input {
  padding: 5px 8px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
}
.date-input:focus {
  border-color: #667eea;
}
.date-sep {
  color: #888;
  font-size: 12px;
}
.presets {
  display: flex;
  gap: 4px;
  margin-left: 8px;
}
.preset-btn {
  padding: 4px 10px;
  font-size: 11px;
  border: 1px solid #ddd;
  border-radius: 4px;
  background: white;
  cursor: pointer;
  color: #666;
}
.preset-btn:hover {
  background: #f5f5fa;
  color: #333;
}

.advanced-toggle {
  padding-top: 8px;
}
.link-btn {
  background: none;
  border: none;
  color: #667eea;
  font-size: 12px;
  cursor: pointer;
  padding: 4px 0;
}
.link-btn:hover {
  text-decoration: underline;
}
.advanced-block {
  background: white;
  border: 1px solid #eee;
  border-radius: 6px;
  padding: 4px 12px;
  margin-top: 6px;
}
.advanced-hint {
  margin: 6px 0;
  font-size: 11px;
  color: #999;
}
.console-btn {
  padding: 5px 12px;
  font-size: 12px;
  border: 1px solid #4285f4;
  border-radius: 6px;
  background: white;
  color: #4285f4;
  cursor: pointer;
  flex-shrink: 0;
}
.console-btn:hover {
  background: #4285f4;
  color: white;
}

.banner {
  padding: 10px 14px;
  border-radius: 6px;
  font-size: 13px;
  margin-bottom: 12px;
  line-height: 1.5;
}
.banner-warn {
  background: #fffaf0;
  border: 1px solid #fbd38d;
  color: #975a16;
}
.banner-error {
  background: #fff5f5;
  border: 1px solid #fed7d7;
  color: #c53030;
  word-break: break-all;
}

.summary-row {
  font-size: 12px;
  color: #666;
  margin-bottom: 8px;
  display: flex;
  gap: 8px;
  align-items: center;
}
.summary-app {
  color: #999;
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
  margin-left: auto;
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

.empty-state {
  padding: 30px 16px;
  text-align: center;
  font-size: 13px;
  color: #999;
}

.error {
  color: #e53e3e;
  font-size: 12px;
  margin-top: 6px;
}
.error.small {
  font-size: 11px;
}
</style>

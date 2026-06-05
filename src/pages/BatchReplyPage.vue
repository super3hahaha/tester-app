<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  type DatePreset,
  type DateRange,
  computeRange,
  PRESET_LABELS,
} from "../utils/batchReplyDates";

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

interface AppConfig {
  enabled: boolean;
  datePreset: DatePreset;
  customFromDate: string;
  customToDate: string;
  stars: number[];
}

interface MultiConfig {
  perApp: Record<string, AppConfig>;
}

type CandidateStatus = "pending" | "submitting" | "done" | "error";

interface Candidate {
  review: Review;
  replyText: string;
  status: CandidateStatus;
  errorMsg: string;
}

interface AppGroup {
  packageName: string;
  displayName: string;
  config: AppConfig;
  effectiveRange: DateRange; // snapshot at fetch time (preset → concrete dates)
  loading: boolean;
  error: string;
  candidates: Candidate[];
  totalFetched: number;
  collapsed: boolean;
}

// v3: preserve presets from the current key; reset presets when migrating from
// a legacy key (legacy presets are untrustworthy artifacts). See BatchReplyConfigPage.
const STORAGE_KEY = "batch-reply-multi-config-v3";
const LEGACY_KEYS = ["batch-reply-multi-config-v2", "batch-reply-multi-config-v1"];
const APPS_CACHE_KEY = "batch-reply-apps-cache-v1";

function normalizeConfig(raw: any, resetPreset = false): AppConfig {
  const stored = raw?.datePreset;
  const validStored: DatePreset | null =
    stored === "sinceLastWorkday" ||
    stored === "yesterday" ||
    stored === "today" ||
    stored === "7d" ||
    stored === "custom"
      ? stored
      : null;
  const preset: DatePreset = resetPreset || !validStored ? "sinceLastWorkday" : validStored;
  return {
    enabled: !!raw?.enabled,
    datePreset: preset,
    customFromDate: raw?.customFromDate || raw?.fromDate || "",
    customToDate: raw?.customToDate || raw?.toDate || "",
    stars:
      Array.isArray(raw?.stars) && raw.stars.length > 0
        ? raw.stars.filter((s: any) => Number.isInteger(s) && s >= 1 && s <= 5)
        : [1, 2],
  };
}

const BULK_INTERVAL_MS = 200;

const groups = ref<AppGroup[]>([]);
const overallError = ref("");
const needRelogin = ref(false);
const fetching = ref(false);
const fetchedAt = ref<number | null>(null);
const aiGenerating = ref(false);
const bulkSubmitting = ref(false);
const bulkProgress = ref({ done: 0, total: 0 });

const fetchProgress = computed(() => {
  if (!fetching.value || groups.value.length === 0) return null;
  const done = groups.value.filter((g) => !g.loading).length;
  return { done, total: groups.value.length };
});

function loadConfig(): { config: MultiConfig | null; appsCache: PlayApp[] } {
  let config: MultiConfig | null = null;
  let appsCache: PlayApp[] = [];
  let raw = localStorage.getItem(STORAGE_KEY);
  let fromLegacy = false;
  if (!raw) {
    for (const k of LEGACY_KEYS) {
      const v = localStorage.getItem(k);
      if (v) {
        raw = v;
        fromLegacy = true;
        break;
      }
    }
  }
  if (raw) {
    try {
      const parsed = JSON.parse(raw) as MultiConfig;
      if (parsed?.perApp) {
        const normalized: Record<string, AppConfig> = {};
        for (const [pkg, entry] of Object.entries(parsed.perApp)) {
          normalized[pkg] = normalizeConfig(entry, fromLegacy);
        }
        config = { perApp: normalized };
      }
    } catch {
      // ignore corrupt
    }
  }
  try {
    const rawApps = localStorage.getItem(APPS_CACHE_KEY);
    if (rawApps) appsCache = JSON.parse(rawApps) as PlayApp[];
  } catch {
    // ignore
  }
  return { config, appsCache };
}

function rebuildGroups() {
  const { config, appsCache } = loadConfig();
  if (!config || !config.perApp) {
    groups.value = [];
    return;
  }
  const nameMap = new Map(appsCache.map((a) => [a.package_name, a.display_name]));
  const next: AppGroup[] = [];
  for (const [pkg, cfg] of Object.entries(config.perApp)) {
    if (!cfg.enabled) continue;
    next.push({
      packageName: pkg,
      displayName: nameMap.get(pkg) || pkg,
      config: cfg,
      effectiveRange: computeRange(cfg.datePreset, {
        fromDate: cfg.customFromDate,
        toDate: cfg.customToDate,
      }),
      loading: false,
      error: "",
      candidates: [],
      totalFetched: 0,
      collapsed: true,
    });
  }
  // Stable order by display name.
  next.sort((a, b) => a.displayName.localeCompare(b.displayName));
  groups.value = next;
}

onMounted(rebuildGroups);
// MainPage uses v-show (component stays mounted), so onMounted only fires once.
// handleFetch() re-reads localStorage on every click, so config changes take effect on next pull.

function tsRange(r: DateRange): { from: number; to: number } {
  const from = r.fromDate
    ? Math.floor(new Date(r.fromDate + "T00:00:00").getTime() / 1000)
    : 0;
  const to = r.toDate
    ? Math.floor(new Date(r.toDate + "T23:59:59").getTime() / 1000)
    : Number.MAX_SAFE_INTEGER;
  return { from, to };
}

async function fetchOne(g: AppGroup): Promise<void> {
  g.loading = true;
  g.error = "";
  g.candidates = [];
  g.totalFetched = 0;
  try {
    const list = await invoke<Review[]>("list_play_reviews", {
      packageName: g.packageName,
      maxPages: 5,
      translationLanguage: "zh-CN",
    });
    g.totalFetched = list.length;
    const { from, to } = tsRange(g.effectiveRange);
    g.candidates = list
      .filter((r) => g.config.stars.includes(r.star_rating))
      .filter((r) => !r.developer_reply)
      .filter((r) => r.user_comment_ts >= from && r.user_comment_ts <= to)
      .map((r) => ({
        review: r,
        replyText: "",
        status: "pending" as CandidateStatus,
        errorMsg: "",
      }));
    // Auto-expand groups that have something to act on so the user sees them
    // immediately; otherwise keep collapsed to reduce noise.
    if (g.candidates.length > 0) g.collapsed = false;
  } catch (e: any) {
    const msg = String(e);
    if (msg.startsWith("NEED_RELOGIN_SCOPE")) {
      needRelogin.value = true;
      g.error = "登录态权限不足，请右上角 Logout 后重新登录。";
    } else {
      g.error = msg;
    }
  } finally {
    g.loading = false;
  }
}

async function handleFetch() {
  if (fetching.value) return;
  rebuildGroups();
  if (groups.value.length === 0) {
    overallError.value = "尚未启用任何应用。请到 Config 子页勾选并保存。";
    return;
  }
  fetching.value = true;
  overallError.value = "";
  needRelogin.value = false;
  // Parallel: independent per-app calls.
  await Promise.all(groups.value.map((g) => fetchOne(g)));
  fetching.value = false;
  fetchedAt.value = Date.now();
}

async function generateReplies() {
  // TODO: 接入 reply skill —— 把每个 group 内 pending 项的评论文本批量传给 skill，
  // 拿到 [{ review_id, reply_text }] 后回填到对应 candidate.replyText。
  aiGenerating.value = true;
  try {
    await new Promise((r) => setTimeout(r, 300));
    overallError.value = "AI 批量生成回复尚未接入（等 reply skill 完成后接通）。";
  } finally {
    aiGenerating.value = false;
  }
}

async function submitOne(g: AppGroup, idx: number): Promise<boolean> {
  const c = g.candidates[idx];
  if (!c) return false;
  if (c.status === "done" || c.status === "submitting") return false;
  if (!c.replyText.trim()) {
    c.status = "error";
    c.errorMsg = "回复内容不能为空";
    return false;
  }
  c.status = "submitting";
  c.errorMsg = "";
  try {
    await invoke("reply_to_review", {
      packageName: g.packageName,
      reviewId: c.review.review_id,
      replyText: c.replyText.trim(),
    });
    c.status = "done";
    return true;
  } catch (e: any) {
    c.status = "error";
    c.errorMsg = String(e);
    return false;
  }
}

async function handleSubmitOne(g: AppGroup, idx: number) {
  await submitOne(g, idx);
}

async function handleSubmitAll() {
  const tasks: Array<{ g: AppGroup; idx: number }> = [];
  for (const g of groups.value) {
    g.candidates.forEach((c, idx) => {
      if (c.status !== "done" && c.replyText.trim()) {
        tasks.push({ g, idx });
      }
    });
  }
  if (tasks.length === 0) {
    overallError.value = "没有可提交的回复（请先填写回复内容）。";
    return;
  }
  const ok = window.confirm(
    `将依次提交 ${tasks.length} 条回复到 Play Console（横跨 ${groups.value.length} 个应用），每条间隔 ${BULK_INTERVAL_MS}ms。\n确认继续？`
  );
  if (!ok) return;

  bulkSubmitting.value = true;
  bulkProgress.value = { done: 0, total: tasks.length };
  overallError.value = "";

  for (const t of tasks) {
    await submitOne(t.g, t.idx);
    bulkProgress.value.done += 1;
    await new Promise((r) => setTimeout(r, BULK_INTERVAL_MS));
  }
  bulkSubmitting.value = false;
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

const totalCandidates = computed(() =>
  groups.value.reduce((sum, g) => sum + g.candidates.length, 0)
);

const totalSubmittable = computed(() =>
  groups.value.reduce(
    (sum, g) =>
      sum + g.candidates.filter((c) => c.status !== "done" && c.replyText.trim()).length,
    0
  )
);

const totalDone = computed(() =>
  groups.value.reduce(
    (sum, g) => sum + g.candidates.filter((c) => c.status === "done").length,
    0
  )
);

function configSummary(g: AppGroup): string {
  const r = g.effectiveRange;
  const date = r.fromDate === r.toDate ? r.fromDate : `${r.fromDate} → ${r.toDate}`;
  const presetTag = g.config.datePreset === "custom" ? "" : `（${PRESET_LABELS[g.config.datePreset]}）`;
  const stars = [...g.config.stars].sort().map((s) => s + "★").join(" ");
  return `${date}${presetTag} · ${stars}`;
}
</script>

<template>
  <div class="batch-reply-page">
    <header class="page-header">
      <h3>Batch Reply · Run</h3>
      <p class="subtitle">
        按 <b>Config</b> 子页保存的配置，从启用的每个应用拉取未回复评论。AI 生成回复后逐条或一键提交。
      </p>
    </header>

    <div class="toolbar">
      <span class="summary-text" v-if="groups.length > 0">
        已启用 <b>{{ groups.length }}</b> 个应用
        <span v-if="fetchedAt"> · 共 {{ totalCandidates }} 条候选</span>
        <span v-if="totalDone > 0"> · 已提交 {{ totalDone }} 条</span>
      </span>
      <span class="summary-text" v-else>未配置启用任何应用</span>

      <div class="toolbar-spacer"></div>

      <button
        class="fetch-btn"
        :disabled="fetching || bulkSubmitting || groups.length === 0"
        @click="handleFetch"
      >
        <span v-if="fetching && fetchProgress" class="btn-spinner"></span>
        {{ fetching
            ? (fetchProgress ? `拉取中 ${fetchProgress.done}/${fetchProgress.total}` : "拉取中...")
            : (fetchedAt ? "重新拉取" : "拉取候选评论") }}
      </button>
      <button
        class="ai-btn"
        :disabled="aiGenerating || bulkSubmitting || totalCandidates === 0"
        @click="generateReplies"
      >
        {{ aiGenerating ? "AI 生成中..." : "✨ AI 批量生成回复" }}
      </button>
      <button
        class="bulk-btn"
        :disabled="bulkSubmitting || totalSubmittable === 0"
        @click="handleSubmitAll"
        :title="totalSubmittable === 0 ? '请先填写回复内容' : ''"
      >
        {{ bulkSubmitting
            ? `提交中 ${bulkProgress.done}/${bulkProgress.total}`
            : `一键提交全部（${totalSubmittable}）` }}
      </button>
    </div>

    <div v-if="needRelogin" class="banner banner-warn">
      ⚠️ 当前登录态不包含 Play 相关权限。请点右上角 <b>Logout</b> 后重新登录。
    </div>
    <div v-if="overallError" class="banner banner-error">{{ overallError }}</div>

    <div v-if="groups.length === 0" class="empty-state big">
      还没启用任何应用。<br />
      请到左侧 <b>Batch Reply · Config</b> 子页勾选应用并保存。
    </div>

    <div v-else-if="!fetchedAt && !fetching" class="empty-state">
      点上方「拉取候选评论」从已启用的 {{ groups.length }} 个应用拉取符合条件的评论。
    </div>

    <div class="groups">
      <section v-for="g in groups" :key="g.packageName" class="group">
        <header class="group-head" @click="g.collapsed = !g.collapsed">
          <span class="group-caret">{{ g.collapsed ? "▶" : "▼" }}</span>
          <div class="group-info">
            <div class="info-line">
              <span class="group-name">{{ g.displayName }}</span>
              <span class="group-pkg">{{ g.packageName }}</span>
              <span v-if="g.loading" class="group-tag loading">
                <span class="tag-spinner"></span>拉取中
              </span>
              <span v-else-if="g.error" class="group-tag err">出错</span>
              <span v-else-if="g.candidates.length > 0" class="group-tag ok">
                {{ g.candidates.length }} 条候选 / 共 {{ g.totalFetched }}
              </span>
              <span v-else-if="fetchedAt !== null" class="group-tag empty">无候选</span>
            </div>
            <span class="group-cfg">{{ configSummary(g) }}</span>
          </div>
        </header>

        <div v-if="!g.collapsed" class="group-body">
          <div v-if="g.loading" class="loading-block">
            <span class="spinner"></span>
            <span>正在拉取该应用最近 7 天的评论…</span>
          </div>

          <div v-else-if="g.error" class="banner banner-error small">{{ g.error }}</div>

          <div v-else-if="g.candidates.length === 0 && fetchedAt" class="empty-state small">
            该应用在所选条件下没有未回复评论。
          </div>

          <article
            v-for="(c, idx) in g.candidates"
            :key="c.review.review_id"
            class="review-card"
            :class="{ 'is-done': c.status === 'done', 'is-error': c.status === 'error' }"
          >
            <div class="review-head">
              <span class="stars" :class="`stars-${c.review.star_rating}`">
                {{ starsDisplay(c.review.star_rating) }}
              </span>
              <span class="author">{{ c.review.author_name || "(匿名)" }}</span>
              <span class="ts">{{ formatTs(c.review.user_comment_ts) }}</span>
              <span class="status-tag" :class="`status-${c.status}`">
                {{
                  c.status === "done" ? "✓ 已回复" :
                  c.status === "submitting" ? "⋯ 提交中" :
                  c.status === "error" ? "✗ 失败" :
                  "待提交"
                }}
              </span>
            </div>

            <div class="review-text">{{ c.review.text || "(无文字)" }}</div>
            <div v-if="c.review.original_text" class="review-original">
              <span class="orig-label">原文：</span>{{ c.review.original_text }}
            </div>
            <div class="review-meta">
              <span v-if="c.review.app_version_name">
                v{{ c.review.app_version_name }}<span v-if="c.review.app_version_code"> ({{ c.review.app_version_code }})</span>
              </span>
              <span v-if="c.review.device">{{ c.review.device }}</span>
              <span v-if="c.review.android_os_version">Android {{ c.review.android_os_version }}</span>
              <span v-if="c.review.reviewer_language">lang: {{ c.review.reviewer_language }}</span>
            </div>

            <div class="reply-edit">
              <textarea
                v-model="c.replyText"
                class="reply-textarea"
                placeholder="在此填写回复内容（AI 生成后会自动填入这里）"
                rows="3"
                :disabled="c.status === 'done' || c.status === 'submitting' || bulkSubmitting"
              ></textarea>
              <div class="reply-actions">
                <span v-if="c.status === 'error'" class="error-inline">{{ c.errorMsg }}</span>
                <button
                  class="submit-one-btn"
                  :disabled="
                    c.status === 'done' ||
                    c.status === 'submitting' ||
                    bulkSubmitting ||
                    !c.replyText.trim()
                  "
                  @click="handleSubmitOne(g, idx)"
                >
                  {{
                    c.status === "done" ? "已提交" :
                    c.status === "submitting" ? "提交中..." :
                    c.status === "error" ? "重试" :
                    "提交本条"
                  }}
                </button>
              </div>
            </div>
          </article>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.batch-reply-page {
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
  margin: 4px 0 12px 0;
  font-size: 12px;
  color: #888;
  line-height: 1.5;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  background: #fafafa;
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  margin-bottom: 12px;
  flex-wrap: wrap;
}
.toolbar-spacer {
  flex: 1;
  min-width: 8px;
}
.summary-text {
  font-size: 12px;
  color: #666;
}
.fetch-btn {
  padding: 6px 14px;
  font-size: 12px;
  font-weight: 500;
  border: none;
  border-radius: 6px;
  background: #667eea;
  color: white;
  cursor: pointer;
}
.fetch-btn:hover:not(:disabled) {
  background: #5a67d8;
}
.fetch-btn:disabled {
  background: #ccc;
  cursor: not-allowed;
}
.ai-btn {
  padding: 6px 14px;
  font-size: 12px;
  font-weight: 500;
  border: 1px solid #9f7aea;
  border-radius: 6px;
  background: white;
  color: #6b46c1;
  cursor: pointer;
}
.ai-btn:hover:not(:disabled) {
  background: #9f7aea;
  color: white;
}
.ai-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.bulk-btn {
  padding: 6px 16px;
  font-size: 13px;
  font-weight: 600;
  border: none;
  border-radius: 6px;
  background: #38a169;
  color: white;
  cursor: pointer;
}
.bulk-btn:hover:not(:disabled) {
  background: #2f855a;
}
.bulk-btn:disabled {
  background: #ccc;
  cursor: not-allowed;
}

.banner {
  padding: 10px 14px;
  border-radius: 6px;
  font-size: 13px;
  margin-bottom: 12px;
  line-height: 1.5;
}
.banner.small {
  padding: 6px 10px;
  font-size: 12px;
  margin-bottom: 8px;
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

.empty-state {
  padding: 30px 16px;
  text-align: center;
  font-size: 13px;
  color: #999;
}
.empty-state.big {
  font-size: 14px;
  line-height: 1.8;
}
.empty-state.small {
  padding: 12px 16px;
  font-size: 12px;
}

.groups {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.group {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  background: white;
  overflow: hidden;
}
.group-head {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 14px;
  background: #f7fafc;
  border-bottom: 1px solid #e5e5e5;
  cursor: pointer;
  user-select: none;
  min-height: 48px;
}
.group-head > * {
  align-self: center;
}
.group-head:hover {
  background: #edf2f7;
}
.group-caret {
  color: #666;
  font-size: 10px;
  width: 12px;
  flex-shrink: 0;
}
.group-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.info-line {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.group-name {
  font-size: 13px;
  font-weight: 600;
  color: #2d3748;
}
.group-pkg {
  font-size: 11px;
  color: #999;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  word-break: break-all;
}
.group-cfg {
  font-size: 11px;
  color: #4a5568;
  background: white;
  padding: 2px 8px;
  border-radius: 4px;
  border: 1px solid #e2e8f0;
  align-self: flex-start;
}
.group-tag {
  font-size: 11px;
  line-height: 18px;
  padding: 2px 8px;
  border-radius: 10px;
  font-weight: 600;
  white-space: nowrap;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-left: auto;
  align-self: center;
  height: 22px;
  max-height: 22px;
  box-sizing: border-box;
}
.group-tag.loading { background: #fefcbf; color: #744210; }
.group-tag.err { background: #fed7d7; color: #9b2c2c; }
.group-tag.ok { background: #e6fffa; color: #234e52; }
.group-tag.empty { background: #edf2f7; color: #4a5568; }

.group-body {
  padding: 10px 14px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.loading-block {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 24px 16px;
  color: #4a5568;
  font-size: 13px;
  background: #fafbfd;
  border: 1px dashed #cbd5e0;
  border-radius: 8px;
  justify-content: center;
}
.spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid #cbd5e0;
  border-top-color: #667eea;
  border-radius: 50%;
  animation: spinner-rotate 0.8s linear infinite;
}
.btn-spinner {
  display: inline-block;
  width: 11px;
  height: 11px;
  margin-right: 6px;
  border: 2px solid rgba(255, 255, 255, 0.4);
  border-top-color: white;
  border-radius: 50%;
  animation: spinner-rotate 0.8s linear infinite;
  vertical-align: -1px;
}
.tag-spinner {
  display: inline-block;
  width: 9px;
  height: 9px;
  border: 1.5px solid rgba(116, 66, 16, 0.3);
  border-top-color: #744210;
  border-radius: 50%;
  animation: spinner-rotate 0.8s linear infinite;
}
@keyframes spinner-rotate {
  to { transform: rotate(360deg); }
}

.review-card {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 12px 14px;
  background: white;
  transition: opacity 0.2s, background 0.2s;
}
.review-card.is-done {
  background: #f7faf7;
  opacity: 0.7;
}
.review-card.is-error {
  border-color: #fbb6b6;
  background: #fffafa;
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
.status-tag {
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 10px;
  font-weight: 600;
  margin-left: auto;
}
.status-pending { background: #edf2f7; color: #4a5568; }
.status-submitting { background: #fefcbf; color: #744210; }
.status-done { background: #c6f6d5; color: #22543d; }
.status-error { background: #fed7d7; color: #9b2c2c; }

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

.reply-edit {
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px dashed #e5e5e5;
}
.reply-textarea {
  width: 100%;
  padding: 8px 10px;
  font-size: 13px;
  line-height: 1.5;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
  resize: vertical;
  font-family: inherit;
  box-sizing: border-box;
}
.reply-textarea:focus {
  border-color: #667eea;
}
.reply-textarea:disabled {
  background: #f7f7f7;
  color: #888;
  cursor: not-allowed;
}
.reply-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 8px;
}
.error-inline {
  flex: 1;
  font-size: 11px;
  color: #c53030;
  word-break: break-all;
}
.submit-one-btn {
  padding: 5px 14px;
  font-size: 12px;
  border: 1px solid #667eea;
  border-radius: 6px;
  background: white;
  color: #667eea;
  cursor: pointer;
  margin-left: auto;
  flex-shrink: 0;
}
.submit-one-btn:hover:not(:disabled) {
  background: #667eea;
  color: white;
}
.submit-one-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>

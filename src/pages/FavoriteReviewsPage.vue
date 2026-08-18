<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { scopedKey } from "../utils/accountScopedKey";
import { getActiveAccountId } from "../utils/activeAccount";
import {
  loadFavorites,
  removeFavorite,
  setFavoriteNote,
  updateFavoriteReply,
  favoritesError,
  type FavoriteReview,
} from "../utils/reviewFavorites";

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

// 写失败时不刷新列表（数据没变，刷了反而像是成功了）——原因由 favoritesError banner 说明
function unfavorite(r: FavoriteReview) {
  if (!removeFavorite(r.review_id)) return;
  loadList();
}

// ── 备注：同一时刻只允许编辑一条，用 review_id 标记当前编辑项 ──────────────
const editingNoteId = ref<string | null>(null);
const noteDraft = ref("");

function startEditNote(r: FavoriteReview) {
  editingNoteId.value = r.review_id;
  noteDraft.value = r.note || "";
}
function cancelEditNote() {
  editingNoteId.value = null;
  noteDraft.value = "";
}
// 保存失败时保持编辑态不关（用户刚写的内容还在 noteDraft 里，别让它凭空消失）
function saveNote(r: FavoriteReview) {
  if (!setFavoriteNote(r.review_id, noteDraft.value)) return;
  cancelEditNote();
  loadList();
}
function deleteNote(r: FavoriteReview) {
  if (!confirm("确定删除这条备注？")) return;
  if (!setFavoriteNote(r.review_id, "")) return;
  if (editingNoteId.value === r.review_id) cancelEditNote();
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

// noteUpdatedAt 是毫秒（Date.now()），与评论时间戳的秒不同口径，单独一个格式化函数
function formatMsTs(ts: number): string {
  if (!ts) return "";
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function starsDisplay(n: number): string {
  return "★".repeat(n) + "☆".repeat(5 - n);
}

// ── AI 回复（与 ReviewPage.vue 的入口保持一致，独立实现避免跨页面耦合）──────
const SNAP_VERSION = 1;

function snapKey(pkg: string): string {
  return `${getActiveAccountId() || "_none"}__${pkg}`;
}

async function saveSnapshot(key: string, list: any[], at: number | null) {
  try {
    await invoke("save_reviews_snapshot", {
      key: snapKey(key),
      data: { version: SNAP_VERSION, reviews: list, fetchedAt: at },
    });
  } catch (e) {
    console.warn("save reviews snapshot failed:", e);
  }
}

async function loadSnapshot(key: string): Promise<{ reviews: any[]; fetchedAt: number | null } | null> {
  if (!key) return null;
  try {
    const data = await invoke<any>("load_reviews_snapshot", { key: snapKey(key) });
    if (!data || !Array.isArray(data.reviews)) return null;
    return { reviews: data.reviews, fetchedAt: data.fetchedAt ?? null };
  } catch (e) {
    console.warn("load reviews snapshot failed:", e);
    return null;
  }
}

// 回复成功后，按 review_id 改写该 app 快照文件里那一条，让「Play Console」评论列表
// 回到该 app 时也能看到最新回复状态（读-改-写，与收藏页视图无关）。
async function persistReplyToSnapshot(pkg: string, reviewId: string, replyText: string, ts: number) {
  const snap = await loadSnapshot(pkg);
  if (!snap) return;
  const hit = snap.reviews.find((r) => r.review_id === reviewId);
  if (!hit) return;
  hit.developer_reply = replyText;
  hit.developer_reply_ts = ts;
  await saveSnapshot(pkg, snap.reviews, snap.fetchedAt);
}

interface GenCandidate {
  style: string;
  language: string;
  text: string;
  text_zh: string;
  char_count: number;
}
interface GenReplyResult {
  candidates: GenCandidate[];
  usage: { input_tokens?: number; output_tokens?: number; total_cost_usd?: number } | null;
}

interface ModelConfig { reply: string; analysis: string; translate: string; }
const modelConfig = ref<ModelConfig>({ reply: "claude-sonnet-4-6", analysis: "claude-sonnet-4-6", translate: "claude-haiku-4-5" });
invoke<ModelConfig>("get_model_config").then(c => { modelConfig.value = c; }).catch(() => {});

const LANG_OPTIONS: { value: string; label: string }[] = [
  { value: "auto", label: "跟随评论语言" },
  { value: "en", label: "英文 en" },
  { value: "zh-CN", label: "中文 zh-CN" },
  { value: "ru", label: "俄文 ru" },
  { value: "pt", label: "葡萄牙文 pt" },
  { value: "es", label: "西班牙文 es" },
  { value: "fr", label: "法文 fr" },
  { value: "de", label: "德文 de" },
];

type AiStatus = "idle" | "queued" | "generating" | "done" | "error";
interface AiTask {
  id: string;
  review: FavoriteReview;
  pkg: string;
  appLabel: string;
  instruction: string;
  language: string;
  status: AiStatus;
  submitting: boolean;
  candidates: GenCandidate[];
  selectedIdx: number;
  editText: string;
  error: string;
  log: string;
  usage: GenReplyResult["usage"];
}

const aiTasks = ref<AiTask[]>([]);
const activeTaskId = ref<string | null>(null);
let taskSeq = 0;
let genBusy = false;

const activeTask = computed(
  () => aiTasks.value.find((t) => t.id === activeTaskId.value) ?? null
);
const minimizedTasks = computed(() =>
  aiTasks.value.filter((t) => t.id !== activeTaskId.value)
);

const addTplIdx = ref(-1);
const addTplCategory = ref("");
const addTplBusy = ref(false);
const addTplError = ref("");
const addTplFlash = ref("");

function tplPayload(c: GenCandidate): { text: string; lang: string } {
  const l = (c.language || "").toLowerCase();
  if (l.startsWith("en")) return { text: c.text, lang: "en" };
  if (l.startsWith("zh")) return { text: c.text, lang: "zh-CN" };
  return { text: c.text_zh && c.text_zh.trim() ? c.text_zh : c.text, lang: "zh-CN" };
}
function startAddTpl(idx: number) {
  addTplIdx.value = idx;
  addTplCategory.value = "";
  addTplError.value = "";
}
function cancelAddTpl() {
  addTplIdx.value = -1;
  addTplError.value = "";
}
async function confirmAddTpl(c: GenCandidate) {
  if (addTplBusy.value) return;
  const pkg = activeTask.value?.pkg || "";
  addTplBusy.value = true;
  addTplError.value = "";
  try {
    const product = await invoke<string | null>("product_for_package", {
      packageName: pkg,
    });
    if (!product) {
      addTplError.value = "该应用没有对应的模板产品，无法收录。";
      return;
    }
    const { text, lang } = tplPayload(c);
    await invoke<string>("add_template", {
      product,
      category: addTplCategory.value,
      text,
      lang,
    });
    addTplIdx.value = -1;
    addTplFlash.value = `已收录到「${product}」模板库（${lang === "en" ? "英文" : "中文"}模板）`;
    window.setTimeout(() => (addTplFlash.value = ""), 2500);
  } catch (e: any) {
    addTplError.value = String(e);
  } finally {
    addTplBusy.value = false;
  }
}

let unlistenReplyLog: UnlistenFn | null = null;
onMounted(async () => {
  unlistenReplyLog = await listen<{ text: string; kind: string; done: boolean }>(
    "reply-log",
    (e) => {
      const gen = aiTasks.value.find((t) => t.status === "generating");
      if (gen) gen.log = e.payload.text;
    }
  );
});
onUnmounted(() => {
  if (unlistenReplyLog) unlistenReplyLog();
});

function openAiDialog(r: FavoriteReview) {
  const existing = aiTasks.value.find((t) => t.review.review_id === r.review_id);
  if (existing) {
    activeTaskId.value = existing.id;
    return;
  }
  const task: AiTask = {
    id: `ai-${++taskSeq}`,
    review: r,
    pkg: r._pkg,
    appLabel: r._app,
    instruction: "",
    language: "auto",
    status: "idle",
    submitting: false,
    candidates: [],
    selectedIdx: -1,
    editText: "",
    error: "",
    log: "",
    usage: null,
  };
  aiTasks.value.push(task);
  activeTaskId.value = task.id;
}

function closeTask(task: AiTask) {
  if (task.status === "generating" || task.submitting) return;
  aiTasks.value = aiTasks.value.filter((t) => t.id !== task.id);
  if (activeTaskId.value === task.id) activeTaskId.value = null;
}

function minimizeAiDialog() {
  activeTaskId.value = null;
}
function restoreTask(id: string) {
  activeTaskId.value = id;
}

function enqueueGenerate(task: AiTask) {
  if (task.status === "generating" || task.status === "queued") return;
  addTplIdx.value = -1;
  task.status = "queued";
  task.error = "";
  task.log = "";
  task.candidates = [];
  task.selectedIdx = -1;
  processQueue();
}

async function processQueue() {
  if (genBusy) return;
  const next = aiTasks.value.find((t) => t.status === "queued");
  if (!next) return;
  genBusy = true;
  next.status = "generating";
  try {
    const res = await invoke<GenReplyResult>("generate_single_reply", {
      review: next.review,
      product: next.appLabel || next.pkg,
      packageName: next.pkg,
      instruction: next.instruction.trim(),
      language: next.language,
      model: modelConfig.value.reply,
    });
    next.candidates = Array.isArray(res.candidates) ? res.candidates : [];
    next.usage = res.usage;
    if (next.candidates.length === 0) {
      next.status = "error";
      next.error = "未生成任何候选，请调整方向后重试。";
    } else {
      next.status = "done";
    }
  } catch (e: any) {
    const msg = String(e);
    next.error = msg === "CANCELLED" ? "已取消生成。" : msg;
    next.status = "error";
  } finally {
    genBusy = false;
    processQueue();
  }
}

async function stopTask(task: AiTask) {
  if (task.status === "queued") {
    task.status = task.candidates.length ? "done" : "idle";
    return;
  }
  if (task.status === "generating") {
    try {
      await invoke("stop_reply");
    } catch {
      // ignore
    }
  }
}

function selectCandidate(task: AiTask, idx: number) {
  task.selectedIdx = idx;
  task.editText = task.candidates[idx]?.text ?? "";
}

function onEditInput(task: AiTask) {
  task.selectedIdx = -1;
}

function taskEditLen(task: AiTask | null): number {
  return task ? [...task.editText].length : 0;
}

async function handleSubmitReply(task: AiTask) {
  if (!task || task.submitting) return;
  const text = task.editText.trim();
  if (!text) {
    task.error = "回复内容为空。";
    return;
  }
  if ([...text].length > 350) {
    task.error = `回复超过 350 字符（当前 ${[...text].length}），请精简后再提交。`;
    return;
  }
  task.submitting = true;
  task.error = "";
  try {
    await invoke("reply_to_review", {
      packageName: task.pkg,
      reviewId: task.review.review_id,
      replyText: text,
    });
    const replyTs = Math.floor(Date.now() / 1000);
    task.review.developer_reply = text;
    task.review.developer_reply_ts = replyTs;
    persistReplyToSnapshot(task.pkg, task.review.review_id, text, replyTs);
    updateFavoriteReply(task.review.review_id, text, replyTs);
    // 收藏列表还会被「备注」编辑等操作用 loadList() 整体重建（新对象），task.review 这份引用
    // 到那时就和当前渲染的列表脱钩了——直接改它不一定能反映到界面上，重新从存储读一遍最保险。
    loadList();
    aiTasks.value = aiTasks.value.filter((t) => t.id !== task.id);
    if (activeTaskId.value === task.id) activeTaskId.value = null;
  } catch (e: any) {
    task.error = String(e);
  } finally {
    task.submitting = false;
  }
}
</script>

<template>
  <div class="fav-page">
    <header class="page-header">
      <h3>收藏评论</h3>
      <p class="subtitle">
        在「Play Console」评论列表里点 ☆ 收藏的评论会出现在这里，星标本身即收藏状态，再点一次取消收藏。
        每条可在卡片下方加备注，取消收藏时备注一并删除。
      </p>
    </header>

    <div v-if="favoritesError" class="banner banner-error">{{ favoritesError }}</div>

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
            <button class="ai-btn" @click="openAiDialog(r)">
              🤖 {{ r.developer_reply ? "AI 重新回复" : "AI 回复" }}
            </button>
            <button
              v-if="reviewConsoleUrl(r)"
              class="web-btn"
              @click="openInConsole(r)"
              title="在 Play Console 中打开该评论"
            >
              🌐 在网页中打开
            </button>
          </div>

          <!-- 备注：卡片最下方，未填时只留一个「＋ 添加备注」按钮，不占版面 -->
          <div class="note-zone">
            <div v-if="editingNoteId === r.review_id" class="note-editor">
              <textarea
                v-model="noteDraft"
                class="note-input"
                rows="3"
                placeholder="写点备注……（例如：已同步给开发 / 等 2.3.6 验证）"
              ></textarea>
              <div class="note-editor-actions">
                <button class="note-btn primary" @click="saveNote(r)">保存</button>
                <button class="note-btn" @click="cancelEditNote">取消</button>
                <button v-if="r.note" class="note-btn danger" @click="deleteNote(r)">删除备注</button>
              </div>
            </div>
            <div v-else-if="r.note" class="note-block">
              <div class="note-head">
                <span class="note-label">📝 备注</span>
                <span v-if="r.noteUpdatedAt" class="note-ts">{{ formatMsTs(r.noteUpdatedAt) }}</span>
                <div class="note-head-actions">
                  <button class="note-btn" @click="startEditNote(r)">编辑</button>
                  <button class="note-btn danger" @click="deleteNote(r)">删除</button>
                </div>
              </div>
              <div class="note-text">{{ r.note }}</div>
            </div>
            <button v-else class="note-add-btn" @click="startEditNote(r)">＋ 添加备注</button>
          </div>
        </article>
      </div>
    </template>

    <!-- AI 回复弹窗（展开中的任务） -->
    <div v-if="activeTask" class="ai-overlay" @click.self="minimizeAiDialog">
      <div class="ai-dialog">
        <div class="ai-dialog-head">
          <span class="ai-title">🤖 AI 生成回复</span>
          <div class="ai-head-btns">
            <button class="ai-min" title="缩小（生成继续）" @click="minimizeAiDialog">—</button>
            <button
              class="ai-close"
              :disabled="activeTask.status === 'generating' || activeTask.submitting"
              @click="closeTask(activeTask)"
            >✕</button>
          </div>
        </div>

        <div class="ai-review-quote">
          <span class="stars" :class="`stars-${activeTask.review.star_rating}`">{{ starsDisplay(activeTask.review.star_rating) }}</span>
          <div class="ai-quote-body">
            <div class="ai-quote-text">{{ activeTask.review.text || "(无文字)" }}</div>
            <div v-if="activeTask.review.original_text" class="ai-quote-orig">
              <span class="ai-quote-orig-label">原文：</span>{{ activeTask.review.original_text }}
            </div>
          </div>
        </div>

        <div class="ai-input-row">
          <label class="ai-label">回复方向</label>
          <textarea
            v-model="activeTask.instruction"
            class="ai-instruction"
            rows="2"
            placeholder="可留空——留空则由 AI 根据评论自行判断方向。也可指定，例如：询问用户具体想兼容哪些格式，态度诚恳，表示会反馈给团队"
            :disabled="activeTask.status === 'generating' || activeTask.status === 'queued'"
          ></textarea>
        </div>
        <div class="ai-input-row">
          <label class="ai-label">回复语言</label>
          <select
            v-model="activeTask.language"
            class="ai-lang-select"
            :disabled="activeTask.status === 'generating' || activeTask.status === 'queued'"
          >
            <option v-for="o in LANG_OPTIONS" :key="o.value" :value="o.value">{{ o.label }}</option>
          </select>
          <button
            v-if="activeTask.status === 'generating'"
            class="ai-stop-btn"
            @click="stopTask(activeTask)"
          >■ 停止</button>
          <button
            v-else-if="activeTask.status === 'queued'"
            class="ai-stop-btn"
            @click="stopTask(activeTask)"
          >排队中…取消</button>
          <button
            v-else
            class="ai-gen-btn"
            @click="enqueueGenerate(activeTask)"
          >
            {{ activeTask.candidates.length ? "重新生成" : "生成 3 条候选" }}
          </button>
        </div>

        <div v-if="activeTask.status === 'generating'" class="ai-generating">
          <span class="ai-spinner">⏳</span> 生成中…
          <span v-if="activeTask.log" class="ai-log">{{ activeTask.log }}</span>
        </div>
        <div v-else-if="activeTask.status === 'queued'" class="ai-generating">
          <span class="ai-spinner">⏳</span> 排队等待中…（前面还有任务在生成）
        </div>

        <div v-if="activeTask.error" class="ai-error">{{ activeTask.error }}</div>

        <div v-if="addTplFlash" class="ai-tpl-flash">✓ {{ addTplFlash }}</div>
        <div v-if="activeTask.candidates.length" class="ai-candidates">
          <div
            v-for="(c, idx) in activeTask.candidates"
            :key="idx"
            class="ai-cand"
            :class="{ active: activeTask.selectedIdx === idx }"
            @click="selectCandidate(activeTask, idx)"
          >
            <div class="ai-cand-head">
              <span class="ai-cand-style">{{ c.style || `候选 ${idx + 1}` }}</span>
              <span class="ai-cand-meta">{{ c.language }} · {{ c.char_count }} 字符</span>
              <div class="ai-cand-head-spacer"></div>
              <button
                class="ai-addtpl-btn"
                title="收录为模板（英文候选存英文模板，其它语言用中文预览存中文模板）"
                @click.stop="startAddTpl(idx)"
              >
                ➕ 添加模板
              </button>
            </div>
            <div class="ai-cand-text">{{ c.text }}</div>
            <div v-if="c.text_zh" class="ai-cand-zh">{{ c.text_zh }}</div>

            <!-- 收录面板（内联，填类别） -->
            <div v-if="addTplIdx === idx" class="ai-addtpl-panel" @click.stop>
              <input
                v-model="addTplCategory"
                class="ai-addtpl-category"
                placeholder="类别（如：要五星 / 无法更新；可留空=未分类）"
                @keyup.enter="confirmAddTpl(c)"
              />
              <button class="ai-addtpl-ok" :disabled="addTplBusy" @click="confirmAddTpl(c)">
                {{ addTplBusy ? "收录中…" : "收录" }}
              </button>
              <button class="ai-addtpl-cancel" :disabled="addTplBusy" @click="cancelAddTpl">取消</button>
              <span v-if="addTplError" class="ai-addtpl-err">{{ addTplError }}</span>
            </div>
          </div>
        </div>

        <div v-if="activeTask.selectedIdx >= 0 || activeTask.editText" class="ai-final">
          <label class="ai-label">最终回复（可手动微调）</label>
          <textarea
            v-model="activeTask.editText"
            class="ai-final-text"
            rows="4"
            @input="onEditInput(activeTask)"
          ></textarea>
          <div class="ai-final-foot">
            <span class="ai-charcount" :class="{ over: taskEditLen(activeTask) > 350 }">{{ taskEditLen(activeTask) }} / 350</span>
            <button
              class="ai-submit-btn"
              :disabled="activeTask.submitting || !activeTask.editText.trim() || taskEditLen(activeTask) > 350"
              @click="handleSubmitReply(activeTask)"
            >
              {{ activeTask.submitting ? "提交中…" : "确认提交到 Play" }}
            </button>
          </div>
        </div>

        <div v-if="activeTask.usage" class="ai-usage">
          💰 本次用量：输入 {{ activeTask.usage.input_tokens ?? 0 }} · 输出 {{ activeTask.usage.output_tokens ?? 0 }} tokens
          <span v-if="activeTask.usage.total_cost_usd"> · 约 ${{ activeTask.usage.total_cost_usd.toFixed(4) }}</span>
        </div>
      </div>
    </div>

    <!-- 缩小后的右下角悬浮条 -->
    <div v-if="minimizedTasks.length" class="ai-mini-stack">
      <div
        v-for="t in minimizedTasks"
        :key="t.id"
        class="ai-mini-bar"
        :class="{ 'is-error': t.status === 'error', 'is-done': t.status === 'done' }"
        @click="restoreTask(t.id)"
      >
        <span class="ai-mini-text">
          🤖 <span class="ai-mini-quote">{{ (t.review.text || t.review.original_text || "(无文字)").slice(0, 16) }}</span>
          <template v-if="t.status === 'generating'">· 生成中…</template>
          <template v-else-if="t.status === 'queued'">· 排队中</template>
          <template v-else-if="t.status === 'error'">· 失败</template>
          <template v-else-if="t.candidates.length">· {{ t.candidates.length }} 条已就绪</template>
          <template v-else>· 待生成</template>
        </span>
        <button class="ai-mini-open" @click.stop="restoreTask(t.id)">展开</button>
      </div>
    </div>
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
.banner {
  padding: 8px 12px;
  border-radius: 6px;
  font-size: 12px;
  line-height: 1.6;
  margin-bottom: 12px;
}
.banner-error {
  background: #fff5f5;
  color: #9b2c2c;
  border: 1px solid #fed7d7;
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
  gap: 6px;
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

/* ── 备注区 ─────────────────────────────────────────────────────────── */
.note-zone {
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px dashed #edf2f7;
}
.note-add-btn {
  border: 1px dashed #cbd5e0;
  background: transparent;
  color: #a0aec0;
  font-size: 12px;
  padding: 4px 12px;
  border-radius: 6px;
  cursor: pointer;
}
.note-add-btn:hover {
  border-color: #d69e2e;
  color: #b7791f;
  background: #fffdf5;
}
.note-block {
  background: #fffdf5;
  border-left: 3px solid #ecc94b;
  border-radius: 0 6px 6px 0;
  padding: 8px 12px;
}
.note-head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 4px;
}
.note-label {
  font-size: 11px;
  font-weight: 600;
  color: #b7791f;
}
.note-ts {
  font-size: 11px;
  color: #bbb;
}
.note-head-actions {
  margin-left: auto;
  display: flex;
  gap: 6px;
}
.note-text {
  font-size: 12px;
  color: #4a5568;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
}
.note-editor {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.note-input {
  width: 100%;
  box-sizing: border-box;
  font-size: 12px;
  line-height: 1.6;
  font-family: inherit;
  color: #2d3748;
  padding: 8px 10px;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  resize: vertical;
}
.note-input:focus {
  outline: none;
  border-color: #ecc94b;
  background: #fffdf5;
}
.note-editor-actions {
  display: flex;
  gap: 6px;
  justify-content: flex-end;
}
.note-btn {
  padding: 3px 12px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.note-btn:hover {
  background: #f5f5fa;
  border-color: #cbd5e0;
}
.note-btn.primary {
  border-color: #667eea;
  background: #667eea;
  color: white;
}
.note-btn.primary:hover {
  background: #5a67d8;
}
.note-btn.danger {
  color: #c53030;
}
.note-btn.danger:hover {
  background: #fff5f5;
  border-color: #feb2b2;
}

/* ── AI 回复（与 ReviewPage.vue 的入口样式保持一致）───────────────────── */
.ai-btn {
  padding: 3px 10px;
  font-size: 11px;
  font-weight: 500;
  line-height: 18px;
  border: 1px solid #667eea;
  border-radius: 6px;
  background: white;
  color: #667eea;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-btn:hover {
  background: #667eea;
  color: white;
}
.ai-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  padding: 20px;
}
.ai-dialog {
  background: white;
  border-radius: 12px;
  width: 100%;
  max-width: 640px;
  max-height: 88vh;
  overflow-y: auto;
  padding: 18px 20px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.25);
}
.ai-dialog-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}
.ai-title {
  font-size: 15px;
  font-weight: 600;
  color: #2d3748;
}
.ai-head-btns {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}
.ai-min,
.ai-close {
  border: none;
  background: none;
  color: #999;
  cursor: pointer;
  width: 28px;
  height: 28px;
  border-radius: 6px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.ai-min {
  font-size: 18px;
  line-height: 1;
}
.ai-close {
  font-size: 16px;
}
.ai-min:hover,
.ai-close:hover:not(:disabled) {
  background: #edf2f7;
  color: #4a5568;
}
.ai-close:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.ai-mini-stack {
  position: fixed;
  right: 20px;
  bottom: 20px;
  z-index: 1000;
  display: flex;
  flex-direction: column-reverse;
  gap: 8px;
  align-items: flex-end;
}
.ai-mini-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  background: white;
  border: 1px solid #e2e8f0;
  border-left: 3px solid #667eea;
  border-radius: 10px;
  box-shadow: 0 6px 24px rgba(0, 0, 0, 0.18);
  cursor: pointer;
  max-width: 360px;
}
.ai-mini-bar.is-error {
  border-left-color: #e53e3e;
}
.ai-mini-bar.is-done {
  border-left-color: #38a169;
}
.ai-mini-bar:hover {
  background: #f5f6ff;
}
.ai-mini-text {
  font-size: 12px;
  color: #4a5568;
  white-space: nowrap;
}
.ai-mini-quote {
  color: #1a202c;
  font-weight: 500;
}
.ai-mini-open {
  padding: 4px 12px;
  font-size: 12px;
  border: 1px solid #667eea;
  border-radius: 6px;
  background: white;
  color: #667eea;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-mini-open:hover {
  background: #667eea;
  color: white;
}
.ai-review-quote {
  display: flex;
  gap: 8px;
  align-items: flex-start;
  background: #fafafa;
  border-left: 3px solid #ddd;
  border-radius: 0 6px 6px 0;
  padding: 8px 12px;
  margin-bottom: 14px;
}
.ai-quote-body {
  flex: 1;
  min-width: 0;
}
.ai-quote-text {
  font-size: 13px;
  color: #2d3748;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}
.ai-quote-orig {
  margin-top: 5px;
  font-size: 12px;
  color: #777;
  line-height: 1.5;
}
.ai-quote-orig-label {
  color: #999;
  font-size: 11px;
}
.ai-input-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}
.ai-label {
  width: 72px;
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
  align-self: flex-start;
  padding-top: 6px;
}
.ai-instruction,
.ai-final-text {
  flex: 1;
  padding: 8px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
  resize: vertical;
  font-family: inherit;
  line-height: 1.5;
}
.ai-instruction:focus,
.ai-final-text:focus {
  border-color: #667eea;
}
.ai-lang-select {
  padding: 6px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  cursor: pointer;
}
.ai-gen-btn {
  padding: 6px 14px;
  font-size: 13px;
  font-weight: 500;
  border: none;
  border-radius: 6px;
  background: #667eea;
  color: white;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-gen-btn:hover:not(:disabled) {
  background: #5a67d8;
}
.ai-gen-btn:disabled {
  background: #ccc;
  cursor: not-allowed;
}
.ai-stop-btn {
  padding: 6px 14px;
  font-size: 13px;
  border: 1px solid #e53e3e;
  border-radius: 6px;
  background: white;
  color: #e53e3e;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-generating {
  font-size: 13px;
  color: #667eea;
  margin: 8px 0;
  display: flex;
  align-items: center;
  gap: 8px;
}
.ai-log {
  font-size: 11px;
  color: #999;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ai-error {
  color: #e53e3e;
  font-size: 12px;
  margin: 8px 0;
  word-break: break-word;
}
.ai-candidates {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 12px 0;
}
.ai-cand {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 10px 12px;
  cursor: pointer;
  transition: border-color 0.12s, background 0.12s;
}
.ai-cand:hover {
  border-color: #b3bcf5;
}
.ai-cand.active {
  border-color: #667eea;
  background: #f5f6ff;
}
.ai-cand-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 6px;
}
.ai-cand-style {
  font-size: 12px;
  font-weight: 600;
  color: #5a67d8;
}
.ai-cand-meta {
  font-size: 11px;
  color: #999;
}
.ai-cand-head-spacer {
  flex: 1;
}
.ai-addtpl-btn {
  font-size: 11px;
  padding: 2px 8px;
  border: 1px solid #cbd5e0;
  border-radius: 6px;
  background: white;
  color: #5a67d8;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-addtpl-btn:hover:not(:disabled) {
  background: #eef0ff;
}
.ai-addtpl-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.ai-tpl-flash {
  font-size: 12px;
  color: #22543d;
  background: #c6f6d5;
  border-radius: 6px;
  padding: 5px 10px;
  margin-bottom: 8px;
}
.ai-addtpl-panel {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 8px;
  flex-wrap: wrap;
}
.ai-addtpl-category {
  flex: 1;
  min-width: 160px;
  padding: 4px 8px;
  border: 1px solid #cbd5e0;
  border-radius: 6px;
  font-size: 12px;
}
.ai-addtpl-ok,
.ai-addtpl-cancel {
  font-size: 12px;
  padding: 4px 12px;
  border-radius: 6px;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-addtpl-ok {
  border: 1px solid #667eea;
  background: #667eea;
  color: white;
}
.ai-addtpl-ok:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.ai-addtpl-cancel {
  border: 1px solid #e2e8f0;
  background: white;
  color: #718096;
}
.ai-addtpl-err {
  font-size: 11px;
  color: #c53030;
  width: 100%;
}
.ai-cand-text {
  font-size: 13px;
  color: #2d3748;
  line-height: 1.5;
}
.ai-cand-zh {
  font-size: 12px;
  color: #888;
  line-height: 1.5;
  margin-top: 6px;
  padding-top: 6px;
  border-top: 1px dashed #eee;
}
.ai-final {
  margin-top: 12px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.ai-final .ai-label {
  width: auto;
  padding-top: 0;
}
.ai-final-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.ai-charcount {
  font-size: 12px;
  color: #888;
}
.ai-charcount.over {
  color: #e53e3e;
  font-weight: 600;
}
.ai-submit-btn {
  padding: 7px 16px;
  font-size: 13px;
  font-weight: 500;
  border: none;
  border-radius: 6px;
  background: #38a169;
  color: white;
  cursor: pointer;
}
.ai-submit-btn:hover:not(:disabled) {
  background: #2f855a;
}
.ai-submit-btn:disabled {
  background: #ccc;
  cursor: not-allowed;
}
.ai-usage {
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px solid #eee;
  font-size: 11px;
  color: #999;
}
</style>

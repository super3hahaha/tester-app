<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  type DateRange,
  computeRange,
  PRESET_LABELS,
} from "../utils/batchReplyDates";
import { scopedKey } from "../utils/accountScopedKey";
import { getActiveAccountId } from "../utils/activeAccount";
import {
  type PlayAppConfig,
  loadPlayConfig,
  APPS_CACHE_KEY,
  REPLY_STATE_LABELS,
} from "../utils/playConsoleConfig";
import {
  type TaggedReview,
  getReviews,
  getFetchedAt,
  ensureLoaded,
  reloadFromSnapshot,
  refreshFromApi,
  markReplied,
  matchesReplyState,
} from "../utils/reviewsStore";
import { type DraftEntry, loadDrafts, mergeDrafts, removeDraft } from "../utils/draftsStore";
import {
  isRead,
  markRead,
  undoLastRead,
  lastRead,
  readCount,
  reloadReadMarks,
  readMarksError,
} from "../utils/reviewReadMarks";

interface PlayApp {
  package_name: string;
  display_name: string;
}

type CandidateStatus = "pending" | "submitting" | "done" | "error";

// One AI-generated reply option for a review. Mirrors the review-reply skill's
// candidate schema (source=template|generated, optional confidence/category/direction).
interface ReplyOption {
  source: "template" | "generated";
  text: string;
  text_zh: string;
  confidence?: number;
  category?: string;
  direction?: string;
  language?: string;
}

interface Candidate {
  review: TaggedReview;
  replyText: string;
  options: ReplyOption[]; // matched-template candidate(s); [0] pre-filled into replyText
  selectedIdx: number; // index into options that's currently filled; -1 = manually edited
  showMore: boolean; // whether the alternative-options panel is expanded
  unmatched: boolean; // true once matching ran and found no template (user handles manually)
  manual: boolean; // 用户标记「人工处理」：排除「匹配 / AI 生成回复」+ 一键提交全部，仍可手动填写/逐条提交
  status: CandidateStatus;
  errorMsg: string;
}

interface AppGroup {
  packageName: string;
  displayName: string;
  config: PlayAppConfig;
  effectiveRange: DateRange; // 预设 → 具体日期，rebuildGroups 时解析
  loading: boolean;
  error: string;
  candidates: Candidate[];
  totalFetched: number;
  collapsed: boolean;
}

// 「人工处理」标记按 review_id 持久化：人工先筛一遍、不走 AI 模板批量的评论（有的
// 不用回复，有的需要但模板不合适、或者对 AI 生成的文案不满意想单独盯着看 → 手动写 /
// 用 AI 单条回复）。这些评论在 Play 上仍是未回复态，每次拉取都会重现，不落盘标记就会
// 丢。只存 id 列表，拉取时回填到 candidate.manual。标记后排除「匹配 / AI 生成回复」+
// 「一键提交全部」，仍可手动填写、逐条点「提交」。
const MANUAL_KEY = "batch-reply-manual-ids-v1";

function loadManualIds(): Set<string> {
  try {
    const arr = JSON.parse(localStorage.getItem(scopedKey(MANUAL_KEY)) || "[]");
    return new Set(Array.isArray(arr) ? arr.filter((x) => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}
const manualIds = loadManualIds();
function saveManualIds() {
  localStorage.setItem(scopedKey(MANUAL_KEY), JSON.stringify([...manualIds]));
}

// 已生成但尚未提交的草稿本地持久化（见 draftsStore.ts）：进页面时读一份内存镜像，
// 生成/提交时同步写回，避免每次都重新读 localStorage。
const draftsMap = loadDrafts();

const BULK_INTERVAL_MS = 200;
const GP_LIMIT = 350; // Google Play reply hard limit (chars)

// 与设置页「模型配置 → 回复生成」同一份配置（ReviewPage/FavoriteReviewsPage 的单条
// AI 回复也读这份），批量匹配/自拟回复现在也跟着它走，不再写死 Sonnet。
interface ModelConfig { reply: string; analysis: string; translate: string; }
const modelConfig = ref<ModelConfig>({ reply: "claude-sonnet-4-6", analysis: "claude-sonnet-4-6", translate: "claude-haiku-4-5" });
invoke<ModelConfig>("get_model_config").then((c) => { modelConfig.value = c; }).catch(() => {});

// Reply language. "auto" = the skill replies to each review in its own language
// (per-review), so one run covers a mixed-language batch. Specific codes force
// the whole batch into that one language.
interface LangOption {
  code: string;
  label: string;
}
const LANGS: LangOption[] = [
  { code: "auto", label: "跟随评论语言（auto）" },
  { code: "en", label: "English" },
  { code: "zh-CN", label: "简体中文" },
  { code: "ru", label: "Русский" },
  { code: "pt", label: "Português" },
  { code: "es", label: "Español" },
  { code: "de", label: "Deutsch" },
  { code: "fr", label: "Français" },
  { code: "ja", label: "日本語" },
  { code: "ko", label: "한국어" },
];
const targetLanguage = ref<string>("auto");

const groups = ref<AppGroup[]>([]);
const overallError = ref("");
const noticeMsg = ref(""); // non-error info (e.g. skill warnings)
const needRelogin = ref(false);
const fetching = ref(false);
const aiGenerating = ref(false);
const genLog = ref<string[]>([]); // live skill output lines during generation
const genLogOpen = ref(false);
const lastUsage = ref<UsageInfo | null>(null); // token usage from the latest generation
const genElapsed = ref(0); // seconds since current generation started
let genTimer: number | undefined;
let genStartMs = 0;

function startGenTimer() {
  genStartMs = Date.now();
  genElapsed.value = 0;
  genTimer = window.setInterval(() => {
    genElapsed.value = Math.floor((Date.now() - genStartMs) / 1000);
  }, 1000);
}
function stopGenTimer() {
  if (genTimer !== undefined) {
    window.clearInterval(genTimer);
    genTimer = undefined;
  }
}
function fmtElapsed(s: number): string {
  const m = Math.floor(s / 60);
  const sec = s % 60;
  return m > 0 ? `${m}m${String(sec).padStart(2, "0")}s` : `${sec}s`;
}

async function handleStopReply() {
  try {
    await invoke("stop_reply");
  } catch {
    // ignore — backend resets state regardless
  }
}
const bulkSubmitting = ref(false);
const bulkProgress = ref({ done: 0, total: 0 });
// 本次会话已提交条数：独立计数，不依赖 g.candidates 里还留没留着这条——提交成功
// 后评论立刻从「未回复」视图消失（见 syncCandidates），candidates 数组里不会再有
// status==="done" 的项可数。
const totalDoneThisSession = ref(0);
// 内联两步确认（window.confirm 在 Tauri webview 不弹）
const submitAllArmed = ref(false);
let submitAllTimer: number | undefined;

const fetchProgress = computed(() => {
  if (!fetching.value || groups.value.length === 0) return null;
  const done = groups.value.filter((g) => !g.loading).length;
  return { done, total: groups.value.length };
});

// 应用显示名缓存（与 Play Console 配置页同一份，list_play_apps 的结果）。
function loadAppsCache(): PlayApp[] {
  try {
    const rawApps = localStorage.getItem(scopedKey(APPS_CACHE_KEY));
    if (rawApps) return JSON.parse(rawApps) as PlayApp[];
  } catch {
    // ignore
  }
  return [];
}

// Play Console 深链所需的 developerId / 每个 app 的数字 App ID：这两个只在
// ReviewPage.vue 里维护（用户手填一次后记住），这里只读它写的那份 localStorage，
// 不重新做一套配置 UI。读不到就退化成 Console 的 app 列表页。
const REVIEW_PAGE_CONFIG_KEY = "review-page-config-v3";
const consoleDeveloperId = ref("");
const consoleAppIdByPkg = ref<Record<string, string>>({});

function loadConsoleIds() {
  try {
    const raw = localStorage.getItem(scopedKey(REVIEW_PAGE_CONFIG_KEY));
    if (!raw) return;
    const cfg = JSON.parse(raw);
    if (cfg?.developerId) consoleDeveloperId.value = cfg.developerId;
    if (cfg?.appIdByPkg && typeof cfg.appIdByPkg === "object") {
      consoleAppIdByPkg.value = cfg.appIdByPkg;
    }
  } catch {
    // ignore corrupt/missing config — falls back to the generic app-list URL
  }
}

// 跳转到 Play Console 里这条评论的详情页；developerId/该 app 的数字 App ID 没配过
// 就退化成 Console 的 app 列表页（用户从那边自己找）。
function reviewConsoleUrl(g: AppGroup, c: Candidate): string {
  const appId = consoleAppIdByPkg.value[g.packageName];
  if (!consoleDeveloperId.value || !appId) {
    return `https://play.google.com/console/u/0/developers/${consoleDeveloperId.value || "-"}/app-list`;
  }
  const base = `https://play.google.com/console/u/0/developers/${consoleDeveloperId.value}/app/${appId}/user-feedback/review-details`;
  return `${base}?reviewId=${encodeURIComponent(c.review.review_id)}&corpus=PUBLIC_REVIEWS`;
}

async function openReviewInConsole(g: AppGroup, c: Candidate) {
  try {
    await openUrl(reviewConsoleUrl(g, c));
  } catch (e: any) {
    overallError.value = String(e);
  }
}

// 按 Play Console 拉取配置（两页共用的那一份）重建分组。已存在的分组原地更新，
// 保留其候选（含已填好的 AI 草稿），所以随时重建都安全。
function rebuildGroups() {
  const config = loadPlayConfig();
  if (!config || !config.perApp) {
    groups.value = [];
    return;
  }
  const nameMap = new Map(loadAppsCache().map((a) => [a.package_name, a.display_name]));
  const prev = new Map(groups.value.map((g) => [g.packageName, g]));
  const next: AppGroup[] = [];
  for (const [pkg, cfg] of Object.entries(config.perApp)) {
    if (!cfg.enabled) continue;
    const range = computeRange(cfg.datePreset, {
      fromDate: cfg.customFromDate,
      toDate: cfg.customToDate,
    });
    const displayName = nameMap.get(pkg) || pkg;
    const hit = prev.get(pkg);
    if (hit) {
      hit.config = cfg;
      hit.effectiveRange = range;
      hit.displayName = displayName;
      next.push(hit);
    } else {
      next.push({
        packageName: pkg,
        displayName,
        config: cfg,
        effectiveRange: range,
        loading: false,
        error: "",
        candidates: [],
        totalFetched: 0,
        collapsed: true,
      });
    }
  }
  // Stable order by display name.
  next.sort((a, b) => a.displayName.localeCompare(b.displayName));
  groups.value = next;
}

// 各应用当前应展示的评论：口径与 Play Console 页完全一致（同一份共享数据 +
// 同一个 matchesReplyState），差别只在这页把它们包成可编辑的候选卡片。
const visibleByPkg = computed(() => {
  const out: Record<string, TaggedReview[]> = {};
  for (const g of groups.value) {
    const { from, to } = tsRange(g.effectiveRange);
    out[g.packageName] = getReviews(g.packageName).value.filter(
      (r) =>
        // 标了已读的不再进候选（与 Play Console 页共享同一张表，任一页标记两页同步）
        !isRead(r.review_id) &&
        matchesReplyState(r, g.config.replyState) &&
        // 「回复后又更新」忽略星级（与 Play Console 页同口径）
        (g.config.replyState === "UPDATED" || g.config.stars.includes(r.star_rating)) &&
        r.user_comment_ts >= from &&
        r.user_comment_ts <= to
    );
  }
  return out;
});

// 把共享评论同步成候选卡片：按 review_id 增量对齐 —— 已有的保留草稿/状态，
// 新出现的建卡，不再命中条件的（含刚被本页/Play Console 页回复掉的）立刻移出列表。
// 这页只展示「未回复」，提交成功后不做临时保留——已回复的去 Play Console 页查。
function syncCandidates(g: AppGroup, visible: TaggedReview[]) {
  const prevLen = g.candidates.length;
  const prev = new Map(g.candidates.map((c) => [c.review.review_id, c]));
  const next: Candidate[] = [];
  for (const r of visible) {
    const hit = prev.get(r.review_id);
    if (hit) {
      hit.review = r;
      next.push(hit);
    } else {
      // 新出现的候选先看有没有之前生成、还没提交就中断（如进程重启）的持久化草稿，
      // 有就直接回填，不用重新调 API 生成。
      const draft = draftsMap[r.review_id];
      next.push({
        review: r,
        replyText: draft?.replyText ?? "",
        options: (draft?.options as ReplyOption[]) ?? ([] as ReplyOption[]),
        selectedIdx: draft?.selectedIdx ?? -1,
        showMore: false,
        unmatched: draft?.unmatched ?? false,
        manual: manualIds.has(r.review_id),
        status: "pending" as CandidateStatus,
        errorMsg: draft?.errorMsg ?? "",
      });
    }
  }
  g.candidates = next;
  g.totalFetched = getReviews(g.packageName).value.length;
  // 从「无候选」变成「有候选」时自动展开，其余情况尊重用户的折叠状态。
  if (prevLen === 0 && next.length > 0) g.collapsed = false;
}

watch(
  visibleByPkg,
  (map) => {
    for (const g of groups.value) syncCandidates(g, map[g.packageName] || []);
  },
  { immediate: true }
);

// 各应用共享缓存里最近一次拉取/快照的时间（含后端定时线程写的那次）。
const fetchedAt = computed<number | null>(() => {
  let latest = 0;
  for (const g of groups.value) {
    const at = getFetchedAt(g.packageName);
    if (at && at > latest) latest = at;
  }
  return latest || null;
});

let unlistenReply: UnlistenFn | null = null;
let unlistenScheduledFetch: UnlistenFn | null = null;
let unlistenScheduledGen: UnlistenFn | null = null;

onMounted(async () => {
  rebuildGroups();
  loadConsoleIds();
  // 进页面直接展示共享缓存/磁盘快照里的评论（与 Play Console 页同一批），不用先手动拉取。
  await loadFromCache();

  // 后端定时线程巡检完会刷新快照并 emit 此事件 → 重读快照，候选自动对齐。
  unlistenScheduledFetch = await listen<{ account: string }>("scheduled-fetch-done", async (e) => {
    if (e.payload?.account && e.payload.account !== getActiveAccountId()) return;
    if (fetching.value) return;
    rebuildGroups();
    await loadFromCache(true);
  });

  // 定时通知里勾了「批量生成回复」→ 到点后自动跑一遍生成，全部生成完再推 Telegram。
  unlistenScheduledGen = await listen<{ account: string }>("schedule-batch-generate", async (e) => {
    if (e.payload?.account && e.payload.account !== getActiveAccountId()) return;
    await runScheduledGenerate();
  });
  // Live skill output during AI generation. The awaited invoke() resolves with
  // the final candidates, so this log is purely for showing progress.
  unlistenReply = await listen<{ text: string; kind: string; done: boolean }>(
    "reply-log",
    (event) => {
      const { text } = event.payload;
      if (!text || !text.trim()) return;
      // Single-review dialog generation also emits reply-log; route it to the
      // currently-generating task's own log line instead of the batch-match log.
      const genTask = aiDlgTasks.value.find((t) => t.status === "generating");
      if (genTask) {
        genTask.log = text.trim();
        return;
      }
      genLog.value.push(text.trim());
      if (genLog.value.length > 400) genLog.value.splice(0, genLog.value.length - 400);
    }
  );
});
onUnmounted(() => {
  unlistenReply?.();
  unlistenScheduledFetch?.();
  unlistenScheduledGen?.();
  stopGenTimer();
});
// MainPage uses v-show (component stays mounted across tab switches, remounts only
// on account switch via :key), so onMounted alone misses config changes made on other
// tabs. Re-read config whenever this page becomes visible — otherwise a config saved on
// the Config sub-page after first mount never reaches groups, leaving the page stuck
// on "未配置启用任何应用" with the (groups-empty-disabled) fetch button uncllickable.
const props = defineProps<{ activeOption: string }>();
watch(
  () => props.activeOption,
  async (opt) => {
    // rebuildGroups 现在会保留已有候选，随时重建都安全。
    if (opt !== "review-batch-reply" || fetching.value) return;
    reloadReadMarks();
    rebuildGroups();
    await loadFromCache();
  }
);

// ── 已读标记 ──────────────────────────────────────────────────────────────
// 「已读」= 看过但不打算回复。标记后这条候选立刻移出列表（与回复成功后消失同机制），
// 并在 Play Console 页同步消失。查看/逐条恢复在 Play Console 页的「只看已读」里。
const lastReadLabel = computed(() => {
  const last = lastRead();
  if (!last) return "";
  const { author, excerpt } = last.mark;
  return excerpt ? `${author}：${excerpt}` : author;
});

function handleMarkRead(c: Candidate) {
  // 写失败（存储损坏保护 / 配额满）时不动视图，否则候选消失了但磁盘上没记。
  if (!markRead(c.review)) overallError.value = readMarksError.value;
}

function handleUndoRead() {
  if (!undoLastRead() && readMarksError.value) overallError.value = readMarksError.value;
}

function tsRange(r: DateRange): { from: number; to: number } {
  const from = r.fromDate
    ? Math.floor(new Date(r.fromDate + "T00:00:00").getTime() / 1000)
    : 0;
  const to = r.toDate
    ? Math.floor(new Date(r.toDate + "T23:59:59").getTime() / 1000)
    : Number.MAX_SAFE_INTEGER;
  return { from, to };
}

// 打 API 刷新该应用的评论 → 并进共享缓存（Play Console 页同步受益）→ 候选由
// visibleByPkg 的 watch 自动对齐，这里不直接碰 candidates。
async function refreshOne(g: AppGroup): Promise<void> {
  g.loading = true;
  g.error = "";
  try {
    await refreshFromApi(g.packageName, g.displayName);
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
  await Promise.all(groups.value.map((g) => refreshOne(g)));
  fetching.value = false;
}

// 从本地共享缓存/磁盘快照载入（不打 API）：进页面、切回本页、后端定时刷新完都走这里。
async function loadFromCache(force = false) {
  for (const g of groups.value) {
    if (force) await reloadFromSnapshot(g.packageName);
    else await ensureLoaded(g.packageName);
  }
}

interface SkillResult {
  review_id: string;
  matched?: boolean;
  candidates: ReplyOption[];
}
interface SkillOutput {
  results?: SkillResult[];
  warnings?: string[];
}
interface UsageInfo {
  input_tokens?: number;
  output_tokens?: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
  total_cost_usd?: number;
}
// run_reply_skill returns the skill output plus the CLI-reported token usage.
interface ReplyResultPayload {
  output: SkillOutput;
  usage: UsageInfo | null;
}

// Build the skill input `groups` payload from candidates still needing a reply.
// Only includes reviews without an AI reply yet and not already submitted, so
// re-running "AI 生成" tops up rather than redoing everything.
function buildSkillGroups(): {
  groups: any[];
  pendingByReview: Map<string, Candidate>;
} {
  const out: any[] = [];
  const pendingByReview = new Map<string, Candidate>();
  for (const g of groups.value) {
    const reviews: any[] = [];
    for (const c of g.candidates) {
      if (c.status === "done") continue;
      if (c.manual) continue; // 用户标记「人工处理」，不参与模板匹配填充
      if (c.options.length > 0 || c.unmatched) continue; // already matched/processed
      const r = c.review;
      reviews.push({
        review_id: r.review_id,
        text: r.text,
        original_text: r.original_text,
        star_rating: r.star_rating,
        reviewer_language: r.reviewer_language,
        app_version_name: r.app_version_name,
        device: r.device,
        android_os_version: r.android_os_version,
      });
      pendingByReview.set(r.review_id, c);
    }
    if (reviews.length > 0) {
      out.push({
        package_name: g.packageName,
        display_name: g.displayName,
        reviews,
      });
    }
  }
  return { groups: out, pendingByReview };
}

// 返回本次实际生成/命中的条数（定时自动生成据此决定要不要推 Telegram）。
async function generateReplies(): Promise<number> {
  if (aiGenerating.value) return 0;
  // Block while any app is still being fetched: fetchOne() clears candidates at
  // its start, so a half-loaded group would silently drop out of buildSkillGroups
  // (the group vanishes from the batch and never gets matched). See the toolbar
  // buttons' :disabled guards — this is the belt to their suspenders.
  if (fetching.value || groups.value.some((g) => g.loading)) {
    overallError.value = "评论还在拉取中，请等拉取完成后再匹配。";
    return 0;
  }
  const { groups: skillGroups, pendingByReview } = buildSkillGroups();
  if (skillGroups.length === 0) {
    overallError.value = "没有需要匹配的候选（可能都已匹配/处理或已提交）。";
    return 0;
  }
  aiGenerating.value = true;
  overallError.value = "";
  noticeMsg.value = "";
  genLog.value = [];
  lastUsage.value = null;
  startGenTimer();

  try {
    const res = await invoke<ReplyResultPayload>("run_reply_skill", {
      groups: skillGroups,
      targetLanguage: targetLanguage.value,
      channel: "gp",
      model: modelConfig.value.reply,
    });
    const out = res.output || {};
    lastUsage.value = res.usage;

    const byId = new Map<string, SkillResult>();
    for (const r of out.results || []) {
      byId.set(r.review_id, r);
    }

    // The skill now always returns exactly 1 candidate per review — either a
    // template hit (source "template") or a minimal self-drafted one (source
    // "generated"). `opts.length === 0` is the rare failure case (skill error /
    // malformed output for that review_id), which still falls back to unmatched.
    let templated = 0;
    let selfDrafted = 0;
    let failed = 0;
    const newDrafts: Record<string, DraftEntry> = {};
    for (const [reviewId, c] of pendingByReview) {
      const r = byId.get(reviewId);
      const opts = r && Array.isArray(r.candidates) ? r.candidates : [];
      if (opts.length > 0) {
        c.options = opts;
        c.selectedIdx = 0;
        c.replyText = opts[0].text || "";
        c.unmatched = false;
        c.errorMsg = "";
        if (c.status === "error") c.status = "pending";
        if (opts[0].source === "template") templated += 1;
        else selfDrafted += 1;
      } else {
        c.unmatched = true;
        c.options = [];
        c.errorMsg = "";
        failed += 1;
      }
      newDrafts[reviewId] = {
        replyText: c.replyText,
        options: c.options,
        selectedIdx: c.selectedIdx,
        unmatched: c.unmatched,
        errorMsg: c.errorMsg,
      };
    }
    // 落盘持久化，进程重启后 syncCandidates 能直接回填这批草稿，不用重新调 API。
    mergeDrafts(newDrafts);
    Object.assign(draftsMap, newDrafts);

    const parts: string[] = [
      `命中模板 ${templated} 条`,
      `AI 自拟 ${selfDrafted} 条（请逐条核对后再提交）`,
    ];
    if (failed > 0) parts.push(`异常 ${failed} 条（请在下方手动处理）`);
    if (lastUsage.value) parts.push(usageText.value);
    if (out.warnings && out.warnings.length > 0) {
      parts.push(`warnings: ${out.warnings.join("；")}`);
    }
    noticeMsg.value = parts.join(" · ");
    return templated + selfDrafted;
  } catch (e: any) {
    const msg = String(e);
    if (msg.includes("CANCELLED")) {
      noticeMsg.value = "已取消生成（已生成的候选保留）。";
    } else {
      overallError.value = `AI 生成回复失败：${msg}`;
    }
    return 0;
  } finally {
    aiGenerating.value = false;
    stopGenTimer();
  }
}

// 定时通知里勾了「批量生成回复」时走这条：刷新评论 → 跑一遍现有的生成流程 →
// 全部生成完才推 Telegram。只生成草稿，提交仍需人工在页面上点。
async function runScheduledGenerate() {
  if (fetching.value || aiGenerating.value || bulkSubmitting.value) return;
  rebuildGroups();
  // 后端刚把最新评论写进快照，直接重读即可，不必再打一次 API。
  await loadFromCache(true);
  if (totalCandidates.value === 0) return;
  const generated = await generateReplies();
  if (generated <= 0) return;
  try {
    await invoke("notify_batch_reply_generated", {
      count: generated,
      apps: groups.value.filter((g) => g.candidates.length > 0).map((g) => g.displayName),
    });
  } catch (e) {
    console.warn("notify batch reply generated failed:", e);
  }
}

// Pick option `idx` for candidate `c`: fill its text into the textarea and
// collapse the alternatives panel.
function selectOption(c: Candidate, idx: number) {
  if (idx < 0 || idx >= c.options.length) return;
  c.selectedIdx = idx;
  c.replyText = c.options[idx].text || "";
  c.showMore = false;
}

// When the user edits the textarea away from the selected option, mark as custom
// so the selection chip stops claiming a specific option is active.
function onReplyInput(c: Candidate) {
  if (c.selectedIdx >= 0 && c.replyText !== (c.options[c.selectedIdx]?.text || "")) {
    c.selectedIdx = -1;
  }
}

function optionLabel(o: ReplyOption): string {
  if (o.source === "template") {
    const conf = o.confidence != null ? ` · ${Math.round(o.confidence * 100)}%` : "";
    const cat = o.category ? `·${o.category}` : "";
    return `模板${cat}${conf}`;
  }
  const dir = o.direction ? `·${o.direction}` : "";
  return `AI 自拟${dir}`;
}

// 顶部 opt-chip 的配色：手动编辑过 → custom(灰)；AI 自拟且未编辑 → generated(黄，
// 提醒需要核对)；模板命中 → active(紫)。
function optChipClass(c: Candidate): string {
  if (c.selectedIdx === -1) return "custom";
  return c.options[c.selectedIdx]?.source === "generated" ? "generated" : "active";
}

function overLimit(text: string): boolean {
  return text.length > GP_LIMIT;
}

// 注意按候选对象提交，不要按下标：提交成功会把这条从候选列表里摘掉，之前算出来的
// 下标会指向别的评论（一键提交时会回错人）。
async function submitCandidate(g: AppGroup, c: Candidate): Promise<boolean> {
  if (c.status === "done" || c.status === "submitting") return false;
  if (!c.replyText.trim()) {
    c.status = "error";
    c.errorMsg = "回复内容不能为空";
    return false;
  }
  c.status = "submitting";
  c.errorMsg = "";
  try {
    const text = c.replyText.trim();
    await invoke("reply_to_review", {
      packageName: g.packageName,
      reviewId: c.review.review_id,
      replyText: text,
    });
    c.status = "done";
    totalDoneThisSession.value += 1;
    // 立刻把这条从卡片列表里摘掉——批量回复页只展示未回复内容，不等 markReplied
    // 之后共享缓存的响应式更新才消失（那样中间会有一帧显示「✓ 已回复」）。
    const idx = g.candidates.indexOf(c);
    if (idx >= 0) g.candidates.splice(idx, 1);
    // 提交成功，持久化的草稿也一并清掉，避免堆积/避免评论后续再次出现时被过期草稿污染。
    removeDraft(c.review.review_id);
    delete draftsMap[c.review.review_id];
    // 同步共享缓存 + 落盘：Play Console 页的过滤 computed 会立刻把这条从未回复列表里去掉。
    await markReplied(
      g.packageName,
      c.review.review_id,
      text,
      Math.floor(Date.now() / 1000)
    );
    return true;
  } catch (e: any) {
    c.status = "error";
    c.errorMsg = String(e);
    return false;
  }
}

async function handleSubmitOne(g: AppGroup, idx: number) {
  const c = g.candidates[idx];
  if (c) await submitCandidate(g, c);
}

// 一键提交全部要不要带上这条：有内容、还没提交过、且没被标记「人工处理」（标记这条
// 就是用户对当前文案不满意，要单独盯着看，不该被一键提交带走）。模板命中和 AI
// 自拟一视同仁——自拟的仍在卡片上标黄提示核对，但不再单独因为「是自拟」被排除。
function canBulkSubmit(c: Candidate): boolean {
  return c.status !== "done" && !c.manual && !!c.replyText.trim();
}

async function handleSubmitAll() {
  const tasks: Array<{ g: AppGroup; c: Candidate }> = [];
  for (const g of groups.value) {
    for (const c of g.candidates) {
      if (canBulkSubmit(c)) tasks.push({ g, c });
    }
  }
  if (tasks.length === 0) {
    overallError.value = "没有可提交的回复（请先填写回复内容）。";
    return;
  }
  // 内联两步确认（window.confirm 在 Tauri webview 不弹，会卡住提交）
  if (!submitAllArmed.value) {
    submitAllArmed.value = true;
    if (submitAllTimer) clearTimeout(submitAllTimer);
    submitAllTimer = window.setTimeout(() => { submitAllArmed.value = false; }, 4000);
    return;
  }
  if (submitAllTimer) clearTimeout(submitAllTimer);
  submitAllArmed.value = false;

  bulkSubmitting.value = true;
  bulkProgress.value = { done: 0, total: tasks.length };
  overallError.value = "";

  for (const t of tasks) {
    await submitCandidate(t.g, t.c);
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
  groups.value.reduce((sum, g) => sum + g.candidates.filter(canBulkSubmit).length, 0)
);

const totalDone = totalDoneThisSession;

const totalManual = computed(() =>
  groups.value.reduce(
    (sum, g) => sum + g.candidates.filter((c) => c.manual).length,
    0
  )
);

// 切换「人工处理」标记，并同步落盘（按 review_id）。
function toggleManual(c: Candidate) {
  c.manual = !c.manual;
  if (c.manual) manualIds.add(c.review.review_id);
  else manualIds.delete(c.review.review_id);
  saveManualIds();
}

const usageText = computed(() => {
  const u = lastUsage.value;
  if (!u) return "";
  const inT = u.input_tokens ?? 0;
  const outT = u.output_tokens ?? 0;
  const cr = u.cache_read_input_tokens ?? 0;
  const cc = u.cache_creation_input_tokens ?? 0;
  const total = inT + outT + cr + cc;
  const cost = u.total_cost_usd != null ? ` · 约 $${u.total_cost_usd.toFixed(4)}` : "";
  const fmt = (n: number) => n.toLocaleString();
  return `本次用量：输入 ${fmt(inT)} · 输出 ${fmt(outT)} · 缓存读 ${fmt(cr)} · 合计 ${fmt(total)} tokens${cost}`;
});

function configSummary(g: AppGroup): string {
  const r = g.effectiveRange;
  const date = r.fromDate === r.toDate ? r.fromDate : `${r.fromDate} → ${r.toDate}`;
  const presetTag = g.config.datePreset === "custom" ? "" : `（${PRESET_LABELS[g.config.datePreset]}）`;
  const stars = [...g.config.stars].sort().map((s) => s + "★").join(" ");
  return `${date}${presetTag} · ${stars} · ${REPLY_STATE_LABELS[g.config.replyState]}`;
}

// ── 单条 AI 回复（freeform 现生成，复用 generate_single_reply） ──────────────
interface GenCandidate {
  style: string;
  language: string;
  text: string;
  text_zh: string;
  char_count: number;
}
interface GenReplyResult {
  candidates: GenCandidate[];
  usage: UsageInfo | null;
}

// 多任务单条 AI 回复：每条评论一个独立任务，可同时缩小成右下角悬浮条。生成排队
// （后端一次只跑一个 generate_single_reply，前端用 aiDlgBusy + processAiDlgQueue
// 串行化）。reply-log 路由到当前正在生成的那个任务。
type AiDlgStatus = "idle" | "queued" | "generating" | "done" | "error";
interface AiDlgTask {
  id: string;
  g: AppGroup;
  c: Candidate;
  instruction: string;
  lang: string;
  status: AiDlgStatus;
  candidates: GenCandidate[];
  error: string;
  log: string;
  usage: UsageInfo | null;
}

const aiDlgTasks = ref<AiDlgTask[]>([]);
const aiDlgActiveId = ref<string | null>(null); // 展开中的任务；null = 全部缩小 / 无
let aiDlgSeq = 0;
let aiDlgBusy = false; // 队列调度：是否有任务正在调用后端

const aiDlgActive = computed(
  () => aiDlgTasks.value.find((t) => t.id === aiDlgActiveId.value) ?? null
);
const aiDlgMinimizedTasks = computed(() =>
  aiDlgTasks.value.filter((t) => t.id !== aiDlgActiveId.value)
);

function openAiDlg(g: AppGroup, c: Candidate) {
  const existing = aiDlgTasks.value.find((t) => t.c.review.review_id === c.review.review_id);
  if (existing) {
    aiDlgActiveId.value = existing.id;
    return;
  }
  const task: AiDlgTask = {
    id: `aidlg-${++aiDlgSeq}`,
    g,
    c,
    instruction: "",
    lang: targetLanguage.value, // 默认沿用工具栏选的回复语言
    status: "idle",
    candidates: [],
    error: "",
    log: "",
    usage: null,
  };
  aiDlgTasks.value.push(task);
  aiDlgActiveId.value = task.id;
}

function closeAiDlgTask(task: AiDlgTask) {
  if (task.status === "generating") return;
  aiDlgTasks.value = aiDlgTasks.value.filter((t) => t.id !== task.id);
  if (aiDlgActiveId.value === task.id) aiDlgActiveId.value = null;
}

function minimizeAiDlg() {
  aiDlgActiveId.value = null;
}
function restoreAiDlgTask(id: string) {
  aiDlgActiveId.value = id;
}

function enqueueAiDlg(task: AiDlgTask) {
  // 回复方向可留空：留空时后端让 AI 据评论自行判断方向。
  if (task.status === "generating" || task.status === "queued") return;
  addTplIdx.value = -1; // 重新生成后候选会变，关掉收录面板
  task.status = "queued";
  task.error = "";
  task.log = "";
  task.candidates = [];
  processAiDlgQueue();
}

async function processAiDlgQueue() {
  if (aiDlgBusy) return;
  const next = aiDlgTasks.value.find((t) => t.status === "queued");
  if (!next) return;
  aiDlgBusy = true;
  next.status = "generating";
  try {
    const res = await invoke<GenReplyResult>("generate_single_reply", {
      review: next.c.review,
      product: next.g.displayName,
      packageName: next.g.packageName,
      instruction: next.instruction.trim(),
      language: next.lang,
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
    next.error = msg.includes("CANCELLED") ? "已取消生成。" : msg;
    next.status = "error";
  } finally {
    aiDlgBusy = false;
    processAiDlgQueue(); // 接着跑下一个排队中的任务
  }
}

async function stopAiDlgTask(task: AiDlgTask) {
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

// 「添加模板」：把觉得好的英文 AI 候选收录进该应用对应产品的模板库（收录时填类别）。
const addTplIdx = ref(-1);
const addTplCategory = ref("");
const addTplBusy = ref(false);
const addTplError = ref("");
const addTplFlash = ref("");

// 模板库中/英双源：英文候选直接存英文模板；其它语言用候选的中文预览(text_zh)
// 存成中文模板。skill 命中后再把模板按 lang 翻到目标回复语言。任意候选都能收录。
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
async function confirmAddTpl(task: AiDlgTask, cand: GenCandidate) {
  if (addTplBusy.value) return;
  addTplBusy.value = true;
  addTplError.value = "";
  try {
    const product = await invoke<string | null>("product_for_package", {
      packageName: task.g.packageName,
    });
    if (!product) {
      addTplError.value = "该应用没有对应的模板产品，无法收录。";
      return;
    }
    const { text, lang } = tplPayload(cand);
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

// 选用某条候选 → 填进该卡片的回复框，移除任务。后续走现有的逐条/一键提交。
function useAiCandidate(task: AiDlgTask, cand: GenCandidate) {
  task.c.replyText = cand.text;
  task.c.selectedIdx = -1; // 标记为手动/AI 文案，不绑定模板候选
  task.c.unmatched = false; // 去掉「未匹配」标
  if (task.c.status === "error") task.c.status = "pending";
  aiDlgTasks.value = aiDlgTasks.value.filter((t) => t.id !== task.id);
  if (aiDlgActiveId.value === task.id) aiDlgActiveId.value = null;
}
</script>

<template>
  <div class="batch-reply-page">
    <header class="page-header">
      <h3>Batch Reply · Run</h3>
      <p class="subtitle">
        与 <b>Play Console</b> 页共用同一份评论和同一份拉取配置（Config 子页 · Play Console 拉取配置）。
        AI 生成回复后逐条或一键提交；任一页回复过的评论，两边都会立刻消失。
      </p>
    </header>

    <div class="toolbar">
      <span class="summary-text" v-if="groups.length > 0">
        已启用 <b>{{ groups.length }}</b> 个应用
        <span v-if="fetchedAt"> · 共 {{ totalCandidates }} 条候选</span>
        <span v-if="totalDone > 0"> · 已提交 {{ totalDone }} 条</span>
        <span v-if="totalManual > 0"> · 人工处理 {{ totalManual }} 条</span>
      </span>
      <span class="summary-text" v-else>未配置启用任何应用</span>

      <button
        v-if="readCount > 0"
        class="read-undo-btn"
        @click="handleUndoRead"
        :title="lastReadLabel ? `撤销：${lastReadLabel}` : ''"
      >撤销上一条已读</button>

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
      <label class="lang-select" :title="'回复语言'">
        <span class="lang-label">回复语言</span>
        <select v-model="targetLanguage" :disabled="aiGenerating || bulkSubmitting">
          <option v-for="l in LANGS" :key="l.code" :value="l.code">{{ l.label }}</option>
        </select>
      </label>
      <button
        class="ai-btn"
        :disabled="fetching || aiGenerating || bulkSubmitting || totalCandidates === 0"
        @click="generateReplies"
      >
        <span v-if="aiGenerating" class="btn-spinner"></span>
        {{ aiGenerating ? `生成中… ${fmtElapsed(genElapsed)}` : "🔎 匹配 / AI 生成回复" }}
      </button>
      <button v-if="aiGenerating" class="stop-btn" @click="handleStopReply">停止</button>
      <button
        class="bulk-btn"
        :class="{ armed: submitAllArmed }"
        :disabled="fetching || bulkSubmitting || totalSubmittable === 0"
        @click="handleSubmitAll"
        :title="totalSubmittable === 0 ? '请先填写回复内容' : ''"
      >
        {{ bulkSubmitting
            ? `提交中 ${bulkProgress.done}/${bulkProgress.total}`
            : submitAllArmed
              ? `再点一次确认提交 ${totalSubmittable} 条`
              : `一键提交全部（${totalSubmittable}）` }}
      </button>
    </div>

    <div v-if="lastUsage" class="usage-line">💰 {{ usageText }}</div>

    <div v-if="needRelogin" class="banner banner-warn">
      ⚠️ 当前登录态不包含 Play 相关权限。请点右上角 <b>Logout</b> 后重新登录。
    </div>
    <div v-if="overallError" class="banner banner-error">{{ overallError }}</div>
    <div v-if="noticeMsg" class="banner banner-info">{{ noticeMsg }}</div>

    <div v-if="aiGenerating || genLog.length > 0" class="gen-log-box">
      <div class="gen-log-head" @click="genLogOpen = !genLogOpen">
        <span class="group-caret">{{ genLogOpen ? "▼" : "▶" }}</span>
        <span v-if="aiGenerating" class="tag-spinner"></span>
        <span class="gen-log-title">
          {{ aiGenerating ? `正在生成… ${fmtElapsed(genElapsed)}` : "生成日志" }}
        </span>
        <span v-if="aiGenerating" class="gen-hint">逐条匹配内置模板，没命中的自拟一条，请耐心等待</span>
        <span v-else-if="genLog.length > 0" class="gen-log-last">{{ genLog[genLog.length - 1] }}</span>
      </div>
      <pre v-if="genLogOpen" class="gen-log-body">{{ genLog.join("\n") }}</pre>
    </div>

    <div v-if="groups.length === 0" class="empty-state big">
      还没启用任何应用。<br />
      请到左侧 <b>Config</b> 子页（Play Console 拉取配置）勾选应用并保存。
    </div>

    <div v-else-if="!fetchedAt && !fetching" class="empty-state">
      本地还没有该账号的评论缓存。点上方「拉取候选评论」拉一次（Play Console 页拉过就会自动共享）。
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
                {{ g.candidates.length }} 条候选 / 本地共 {{ g.totalFetched }}
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
            :class="{ 'is-done': c.status === 'done', 'is-error': c.status === 'error', 'is-manual': c.manual }"
          >
            <div class="review-head">
              <span class="stars" :class="`stars-${c.review.star_rating}`">
                {{ starsDisplay(c.review.star_rating) }}
              </span>
              <span class="author">{{ c.review.author_name || "(匿名)" }}</span>
              <span class="ts">{{ formatTs(c.review.user_comment_ts) }}</span>
              <button
                class="web-btn"
                @click="openReviewInConsole(g, c)"
                title="在 Play Console 中打开该评论"
              >
                🌐 在网页中打开
              </button>
              <button
                class="ai-one-btn"
                :disabled="c.status === 'done' || c.status === 'submitting' || bulkSubmitting"
                @click="openAiDlg(g, c)"
              >
                🤖 AI 回复
              </button>
              <button
                class="manual-btn"
                :class="{ active: c.manual }"
                :disabled="c.status === 'done' || c.status === 'submitting' || bulkSubmitting"
                :title="c.manual ? '取消后将重新参与「匹配 / AI 生成回复」和「一键提交全部」' : '标记后不参与「匹配 / AI 生成回复」和「一键提交全部」，仍可手动填写 / AI 单条回复 / 逐条提交'"
                @click="toggleManual(c)"
              >
                {{ c.manual ? "↩ 取消人工" : "✋ 人工处理" }}
              </button>
              <button
                class="read-btn"
                :disabled="c.status === 'submitting' || bulkSubmitting"
                title="标为已读：不回复也让它从本页和 Play Console 页消失。撤销在上方「撤销上一条已读」，或去 Play Console 页「只看已读」逐条恢复"
                @click="handleMarkRead(c)"
              >已读</button>
              <span v-if="c.manual" class="status-tag status-manual">✋ 人工处理</span>
              <span v-if="c.unmatched && c.status === 'pending'" class="unmatched-tag">
                生成异常 · 需手动处理
              </span>
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
              <!-- AI option picker: selected chip + char count + 更多. Shown only
                   once the skill has returned candidates for this review. -->
              <div v-if="c.options.length > 0" class="opt-bar">
                <span class="opt-chip" :class="optChipClass(c)">
                  {{
                    c.selectedIdx === -1
                      ? "已手动编辑"
                      : optionLabel(c.options[c.selectedIdx])
                  }}
                </span>
                <span class="opt-count">候选 {{ c.options.length }} 条</span>
                <span class="char-count" :class="{ over: overLimit(c.replyText) }">
                  {{ c.replyText.length }}/{{ GP_LIMIT }}
                </span>
                <button
                  v-if="c.options.length > 1"
                  class="more-btn"
                  @click="c.showMore = !c.showMore"
                  :disabled="c.status === 'done' || c.status === 'submitting' || bulkSubmitting"
                >
                  {{ c.showMore ? "收起 ▲" : "更多 ▾" }}
                </button>
              </div>

              <!-- Alternatives panel: every option with EN body + 中文 preview. -->
              <div v-if="c.showMore && c.options.length > 1" class="opt-list">
                <div
                  v-for="(o, oi) in c.options"
                  :key="oi"
                  class="opt-item"
                  :class="{ selected: oi === c.selectedIdx }"
                  @click="selectOption(c, oi)"
                >
                  <div class="opt-item-head">
                    <span class="opt-item-tag" :class="o.source">{{ optionLabel(o) }}</span>
                    <span v-if="o.language" class="opt-item-lang">{{ o.language }}</span>
                    <span class="opt-item-len" :class="{ over: overLimit(o.text) }">
                      {{ o.text.length }} 字符
                    </span>
                    <span v-if="oi === c.selectedIdx" class="opt-item-cur">当前</span>
                  </div>
                  <div class="opt-item-text">{{ o.text }}</div>
                  <div v-if="o.text_zh" class="opt-item-zh">{{ o.text_zh }}</div>
                </div>
              </div>

              <textarea
                v-model="c.replyText"
                class="reply-textarea"
                :placeholder="c.unmatched ? '本条生成异常，请在此手动填写回复' : '在此填写回复内容（点「🔎 匹配 / AI 生成回复」后会自动填入）'"
                rows="3"
                :disabled="c.status === 'done' || c.status === 'submitting' || bulkSubmitting"
                @input="onReplyInput(c)"
              ></textarea>
              <!-- 中文译文 of the currently-filled option. The 更多 panel only shows
                   when there are 2+ candidates, so without this a single-candidate
                   reply would never surface its translation. Hidden once the user
                   edits manually (selectedIdx -1), since the译文 no longer matches. -->
              <div
                v-if="c.selectedIdx >= 0 && c.options[c.selectedIdx]?.text_zh"
                class="reply-zh"
              >
                <span class="reply-zh-label">中文</span>
                <span class="reply-zh-text">{{ c.options[c.selectedIdx].text_zh }}</span>
              </div>
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

    <!-- 单条 AI 回复弹窗（展开中的任务） -->
    <div v-if="aiDlgActive" class="ai-overlay" @click.self="minimizeAiDlg">
      <div class="ai-dialog">
        <div class="ai-dialog-head">
          <span class="ai-title">🤖 AI 生成回复 · {{ aiDlgActive.g.displayName }}</span>
          <div class="ai-head-btns">
            <button class="ai-min" title="缩小（生成继续）" @click="minimizeAiDlg">—</button>
            <button
              class="ai-close"
              :disabled="aiDlgActive.status === 'generating'"
              @click="closeAiDlgTask(aiDlgActive)"
            >✕</button>
          </div>
        </div>

        <div class="ai-review-quote">
          <span class="stars" :class="`stars-${aiDlgActive.c.review.star_rating}`">
            {{ starsDisplay(aiDlgActive.c.review.star_rating) }}
          </span>
          <div class="ai-quote-body">
            <div class="ai-quote-text">{{ aiDlgActive.c.review.text || "(无文字)" }}</div>
            <div v-if="aiDlgActive.c.review.original_text" class="ai-quote-orig">
              <span class="ai-quote-orig-label">原文：</span>{{ aiDlgActive.c.review.original_text }}
            </div>
          </div>
        </div>

        <div class="ai-input-row">
          <label class="ai-label">回复方向</label>
          <textarea
            v-model="aiDlgActive.instruction"
            class="ai-instruction"
            rows="2"
            placeholder="可留空——留空则由 AI 根据评论自行判断方向。也可指定，例如：询问用户具体想兼容哪些格式，态度诚恳，表示会反馈给团队"
            :disabled="aiDlgActive.status === 'generating' || aiDlgActive.status === 'queued'"
          ></textarea>
        </div>
        <div class="ai-input-row ai-input-row--center">
          <label class="ai-label">回复语言</label>
          <select
            v-model="aiDlgActive.lang"
            class="ai-lang-select"
            :disabled="aiDlgActive.status === 'generating' || aiDlgActive.status === 'queued'"
          >
            <option v-for="l in LANGS" :key="l.code" :value="l.code">{{ l.label }}</option>
          </select>
          <button
            v-if="aiDlgActive.status === 'generating'"
            class="ai-stop-btn"
            @click="stopAiDlgTask(aiDlgActive)"
          >■ 停止</button>
          <button
            v-else-if="aiDlgActive.status === 'queued'"
            class="ai-stop-btn"
            @click="stopAiDlgTask(aiDlgActive)"
          >排队中…取消</button>
          <button
            v-else
            class="ai-gen-btn"
            @click="enqueueAiDlg(aiDlgActive)"
          >
            {{ aiDlgActive.candidates.length ? "重新生成" : "生成 3 条候选" }}
          </button>
        </div>

        <div v-if="aiDlgActive.status === 'generating'" class="ai-generating">
          <span class="spinner"></span> 生成中…
          <span v-if="aiDlgActive.log" class="ai-log">{{ aiDlgActive.log }}</span>
        </div>
        <div v-else-if="aiDlgActive.status === 'queued'" class="ai-generating">
          <span class="spinner"></span> 排队等待中…（前面还有任务在生成）
        </div>

        <div v-if="aiDlgActive.error" class="banner banner-error small">{{ aiDlgActive.error }}</div>

        <div v-if="addTplFlash" class="ai-tpl-flash">✓ {{ addTplFlash }}</div>
        <div v-if="aiDlgActive.candidates.length" class="ai-candidates">
          <div v-for="(cand, ci) in aiDlgActive.candidates" :key="ci" class="ai-cand">
            <div class="ai-cand-head">
              <span class="ai-cand-style">{{ cand.style || `候选 ${ci + 1}` }}</span>
              <span class="ai-cand-meta" :class="{ over: overLimit(cand.text) }">
                {{ cand.language }} · {{ cand.text.length }}/{{ GP_LIMIT }} 字符
              </span>
              <div class="ai-cand-head-spacer"></div>
              <button
                class="ai-addtpl-btn"
                title="收录为模板（英文候选存英文模板，其它语言用中文预览存中文模板）"
                @click="startAddTpl(ci)"
              >
                ➕ 添加模板
              </button>
              <button
                class="ai-use-btn"
                :disabled="overLimit(cand.text)"
                @click="useAiCandidate(aiDlgActive, cand)"
              >
                选用并填入
              </button>
            </div>
            <div class="ai-cand-text">{{ cand.text }}</div>
            <div v-if="cand.text_zh" class="ai-cand-zh">{{ cand.text_zh }}</div>

            <!-- 收录面板（内联，填类别） -->
            <div v-if="addTplIdx === ci" class="ai-addtpl-panel">
              <input
                v-model="addTplCategory"
                class="ai-addtpl-category"
                placeholder="类别（如：要五星 / 无法更新；可留空=未分类）"
                @keyup.enter="confirmAddTpl(aiDlgActive, cand)"
              />
              <button class="ai-addtpl-ok" :disabled="addTplBusy" @click="confirmAddTpl(aiDlgActive, cand)">
                {{ addTplBusy ? "收录中…" : "收录" }}
              </button>
              <button class="ai-addtpl-cancel" :disabled="addTplBusy" @click="cancelAddTpl">取消</button>
              <span v-if="addTplError" class="ai-addtpl-err">{{ addTplError }}</span>
            </div>
          </div>
        </div>

        <div v-if="aiDlgActive.usage" class="ai-usage">
          💰 输入 {{ aiDlgActive.usage.input_tokens ?? 0 }} · 输出 {{ aiDlgActive.usage.output_tokens ?? 0 }} tokens
          <span v-if="aiDlgActive.usage.total_cost_usd"> · 约 ${{ aiDlgActive.usage.total_cost_usd.toFixed(4) }}</span>
        </div>
      </div>
    </div>

    <!-- 缩小后的右下角悬浮条：竖直堆叠，每个任务一条 -->
    <div v-if="aiDlgMinimizedTasks.length" class="ai-mini-stack">
      <div
        v-for="t in aiDlgMinimizedTasks"
        :key="t.id"
        class="ai-mini-bar"
        :class="{ 'is-error': t.status === 'error', 'is-done': t.status === 'done' }"
        @click="restoreAiDlgTask(t.id)"
      >
        <span v-if="t.status === 'generating'" class="spinner"></span>
        <span class="ai-mini-text">
          🤖 {{ t.g.displayName }} ·
          <template v-if="t.status === 'generating'">生成中…</template>
          <template v-else-if="t.status === 'queued'">排队中</template>
          <template v-else-if="t.status === 'error'">生成失败</template>
          <template v-else-if="t.candidates.length">{{ t.candidates.length }} 条候选已就绪</template>
          <template v-else>待生成</template>
        </span>
        <button class="ai-mini-open" @click.stop="restoreAiDlgTask(t.id)">展开</button>
      </div>
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
.stop-btn {
  padding: 6px 14px;
  font-size: 12px;
  font-weight: 500;
  border: 1px solid #e53e3e;
  border-radius: 6px;
  background: white;
  color: #e53e3e;
  cursor: pointer;
}
.stop-btn:hover {
  background: #e53e3e;
  color: white;
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
.bulk-btn.armed {
  background: #dd6b20;
}
.bulk-btn.armed:hover:not(:disabled) {
  background: #c05621;
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
.banner-info {
  background: #ebf8ff;
  border: 1px solid #bee3f8;
  color: #2c5282;
}
.usage-line {
  font-size: 12px;
  color: #4a5568;
  background: #f7fafc;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  padding: 6px 12px;
  margin-bottom: 12px;
  font-variant-numeric: tabular-nums;
}

/* language selector in toolbar */
.lang-select {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: #666;
}
.lang-label {
  white-space: nowrap;
}
.lang-select select {
  font-size: 12px;
  padding: 5px 8px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #2d3748;
  cursor: pointer;
}
.lang-select select:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* generation log */
.gen-log-box {
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  background: #fbfcfe;
  margin-bottom: 12px;
  overflow: hidden;
}
.gen-log-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  cursor: pointer;
  user-select: none;
}
.gen-log-head:hover {
  background: #f1f5f9;
}
.gen-log-title {
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
  white-space: nowrap;
}
.gen-hint {
  font-size: 11px;
  color: #a0aec0;
  flex: 1;
  min-width: 0;
}
.gen-log-last {
  font-size: 11px;
  color: #718096;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}
.gen-log-body {
  margin: 0;
  padding: 10px 12px;
  border-top: 1px solid #e2e8f0;
  background: #1a202c;
  color: #cbd5e0;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  font-size: 11px;
  line-height: 1.5;
  max-height: 220px;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-word;
}

/* AI option picker */
.opt-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
  flex-wrap: wrap;
}
.opt-chip {
  font-size: 11px;
  font-weight: 600;
  padding: 2px 9px;
  border-radius: 10px;
}
.opt-chip.active {
  background: #e9d8fd;
  color: #553c9a;
}
.opt-chip.custom {
  background: #edf2f7;
  color: #4a5568;
}
.opt-chip.generated {
  background: #fefcbf;
  color: #744210;
}
.opt-count {
  font-size: 11px;
  color: #888;
}
.char-count {
  font-size: 11px;
  color: #a0aec0;
  font-variant-numeric: tabular-nums;
}
.char-count.over {
  color: #e53e3e;
  font-weight: 600;
}
.more-btn {
  margin-left: auto;
  padding: 3px 12px;
  font-size: 11px;
  border: 1px solid #cbd5e0;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.more-btn:hover:not(:disabled) {
  background: #edf2f7;
}
.more-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.opt-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-bottom: 10px;
  padding: 8px;
  background: #faf9fd;
  border: 1px solid #e9e3f5;
  border-radius: 8px;
}
.opt-item {
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  padding: 8px 10px;
  background: white;
  cursor: pointer;
  transition: border-color 0.15s, box-shadow 0.15s;
}
.opt-item:hover {
  border-color: #9f7aea;
}
.opt-item.selected {
  border-color: #805ad5;
  box-shadow: 0 0 0 1px #805ad5 inset;
}
.opt-item-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 5px;
  flex-wrap: wrap;
}
.opt-item-tag {
  font-size: 10px;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 8px;
}
.opt-item-tag.template {
  background: #e6fffa;
  color: #234e52;
}
.opt-item-tag.generated {
  background: #fefcbf;
  color: #744210;
}
.opt-item-lang {
  font-size: 10px;
  color: #999;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
}
.opt-item-len {
  font-size: 10px;
  color: #a0aec0;
}
.opt-item-len.over {
  color: #e53e3e;
  font-weight: 600;
}
.opt-item-cur {
  font-size: 10px;
  font-weight: 600;
  color: #805ad5;
  margin-left: auto;
}
.opt-item-text {
  font-size: 12px;
  line-height: 1.5;
  color: #2d3748;
  white-space: pre-wrap;
  word-break: break-word;
}
.opt-item-zh {
  margin-top: 4px;
  font-size: 11px;
  color: #888;
  line-height: 1.4;
  border-top: 1px dashed #edf2f7;
  padding-top: 4px;
}
.reply-zh {
  margin-top: 6px;
  font-size: 12px;
  line-height: 1.5;
  color: #718096;
  background: #f7fafc;
  border: 1px solid #edf2f7;
  border-radius: 6px;
  padding: 6px 8px;
  white-space: pre-wrap;
  word-break: break-word;
}
.reply-zh-label {
  display: inline-block;
  margin-right: 6px;
  font-size: 10px;
  font-weight: 600;
  color: #a0aec0;
  vertical-align: top;
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
.review-card.is-manual {
  border-left: 3px solid #a0aec0;
  background: #fbfcfd;
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
}
.unmatched-tag {
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 10px;
  font-weight: 600;
  background: #fffaf0;
  color: #975a16;
  border: 1px solid #fbd38d;
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

/* 在 Play Console 中打开该评论（卡片右上角操作簇的第一个，把整簇推到最右） */
.web-btn {
  padding: 3px 10px;
  font-size: 11px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
  margin-left: auto;
  flex-shrink: 0;
}
.web-btn:hover {
  background: #f5f5fa;
  border-color: #cbd5e0;
  color: #2d3748;
}

/* 单条 AI 回复按钮（卡片右上角，紧邻状态标） */
.ai-one-btn {
  padding: 3px 10px;
  font-size: 11px;
  font-weight: 500;
  line-height: 18px;
  border: 1px solid #9f7aea;
  border-radius: 6px;
  background: white;
  color: #6b46c1;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-one-btn:hover:not(:disabled) {
  background: #9f7aea;
  color: white;
}
.ai-one-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.manual-btn {
  padding: 3px 10px;
  font-size: 11px;
  font-weight: 500;
  line-height: 18px;
  border: 1px solid #cbd5e0;
  border-radius: 6px;
  background: white;
  color: #718096;
  cursor: pointer;
  flex-shrink: 0;
}
.manual-btn:hover:not(:disabled) {
  background: #edf2f7;
  color: #4a5568;
}
.manual-btn.active {
  border-color: #a0aec0;
  background: #4a5568;
  color: white;
}
.manual-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.status-manual { background: #e2e8f0; color: #4a5568; }

/* 单条 AI 回复弹窗 */
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
  gap: 12px;
}
.ai-title {
  font-size: 15px;
  font-weight: 600;
  color: #2d3748;
  word-break: break-word;
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

/* 缩小后的右下角悬浮条 */
.ai-mini-stack {
  position: fixed;
  right: 20px;
  bottom: 20px;
  z-index: 1000;
  display: flex;
  flex-direction: column-reverse; /* 新的堆在下方，旧的往上叠 */
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
  border-left: 3px solid #9f7aea;
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
  background: #faf9fd;
}
.ai-mini-text {
  font-size: 12px;
  color: #4a5568;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ai-mini-open {
  padding: 4px 12px;
  font-size: 12px;
  border: 1px solid #9f7aea;
  border-radius: 6px;
  background: white;
  color: #6b46c1;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-mini-open:hover {
  background: #9f7aea;
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
  align-items: flex-start;
  gap: 8px;
  margin-bottom: 10px;
}
/* 单行控件行（语言+按钮）整体垂直居中 */
.ai-input-row--center {
  align-items: center;
}
.ai-input-row--center .ai-label {
  padding-top: 0;
}
.ai-label {
  width: 64px;
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
  padding-top: 7px;
}
.ai-instruction {
  flex: 1;
  padding: 8px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
  resize: vertical;
  font-family: inherit;
  line-height: 1.5;
  box-sizing: border-box;
}
.ai-instruction:focus {
  border-color: #9f7aea;
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
  background: #805ad5;
  color: white;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-gen-btn:hover:not(:disabled) {
  background: #6b46c1;
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
  color: #6b46c1;
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
}
.ai-cand-head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 6px;
}
.ai-cand-style {
  font-size: 12px;
  font-weight: 600;
  color: #6b46c1;
}
.ai-cand-meta {
  font-size: 11px;
  color: #999;
  font-variant-numeric: tabular-nums;
}
.ai-cand-meta.over {
  color: #e53e3e;
  font-weight: 600;
}
.ai-use-btn {
  margin-left: auto;
  padding: 4px 12px;
  font-size: 12px;
  border: none;
  border-radius: 6px;
  background: #38a169;
  color: white;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-use-btn:hover:not(:disabled) {
  background: #2f855a;
}
.ai-use-btn:disabled {
  background: #ccc;
  cursor: not-allowed;
}
.ai-cand-head-spacer {
  flex: 1;
}
.ai-addtpl-btn {
  padding: 4px 10px;
  font-size: 12px;
  border: 1px solid #cbd5e0;
  border-radius: 6px;
  background: white;
  color: #6b46c1;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-addtpl-btn:hover:not(:disabled) {
  background: #f5f0fc;
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
  border: 1px solid #9f7aea;
  background: #9f7aea;
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
  white-space: pre-wrap;
  word-break: break-word;
}
.ai-cand-zh {
  font-size: 12px;
  color: #888;
  line-height: 1.5;
  margin-top: 6px;
  padding-top: 6px;
  border-top: 1px dashed #eee;
}
.ai-usage {
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px solid #eee;
  font-size: 11px;
  color: #999;
}
/* ── 已读标记 ── */
.read-btn {
  padding: 3px 10px;
  font-size: 11px;
  font-weight: 500;
  line-height: 18px;
  border: 1px solid #cbd5e0;
  border-radius: 6px;
  background: white;
  color: #718096;
  cursor: pointer;
  flex-shrink: 0;
}
.read-btn:hover:not(:disabled) {
  background: #edf2f7;
  color: #4a5568;
}
.read-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.read-undo-btn {
  margin-left: 10px;
  padding: 3px 10px;
  font-size: 11px;
  line-height: 18px;
  border: 1px solid #cbd5e0;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
  flex-shrink: 0;
}
.read-undo-btn:hover {
  border-color: #a0aec0;
  background: #f7fafc;
}
</style>

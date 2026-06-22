<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { lastWorkdayBefore, toIso, computeRange } from "../utils/batchReplyDates";
import { loadPlayConfig } from "../utils/playConsoleConfig";
import { loadFavIds } from "../utils/templateFavorites";

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

// 每条评论带上来源应用标签 —— 单应用拉取也带（同一个 app），批量拉取时区分各 app。
type TaggedReview = Review & { _pkg: string; _app: string };

// UPDATED = 开发者已回复，但用户在回复之后又更新了评论（回复可能已过时，需重回）
type ReplyState = "ANY" | "ABSENT" | "REPLIED" | "UPDATED";

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

// Play reviews API 只返回最近约 7 天的评论，日期选择器据此限制可选范围（给 1 天
// 余量，API 边界本就不精确）。选更早的日期没意义——API 根本没返回那些评论。
const minSelectableDate = computed(() => daysAgoIso(7));
const maxSelectableDate = computed(() => todayIso());

const reviews = ref<TaggedReview[]>([]);
const loading = ref(false);
const batchLoading = ref(false); // 批量拉取中
const errorMsg = ref("");
const needRelogin = ref(false);
const fetchedAt = ref<number | null>(null);
const showAdvanced = ref(false);
// "single" = 拉取单个应用（用页面上的筛选）；"batch" = 批量拉取（按各应用 Config 配置筛选）
const mode = ref<"single" | "batch">("single");
const batchSummary = ref(""); // 批量拉取结果摘要（拉了几个 app、各多少条）
// 批量拉取时各应用 Config 的星级集合（拉取阶段保留全部星级，展示时据此过滤；
// 回复状态选「回复后又更新」时跳过星级、显示所有星级）。
const batchStarsByPkg = ref<Record<string, number[]>>({});

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

// 自上一个工作日 → 今天（与批量回复页同口径：周一回到上周五、周末回到上周五）
function setSinceLastWorkday() {
  fromDate.value = toIso(lastWorkdayBefore());
  toDate.value = todayIso();
}

async function handleFetch() {
  if (loading.value || batchLoading.value || !packageName.value.trim()) return;
  loading.value = true;
  errorMsg.value = "";
  needRelogin.value = false;
  reviews.value = [];
  const pkg = packageName.value.trim();
  const appName = selectedAppLabel.value || pkg;
  try {
    const list = await invoke<Review[]>("list_play_reviews", {
      packageName: pkg,
      maxPages: 5,
      translationLanguage: "zh-CN",
    });
    reviews.value = list.map((r) => ({ ...r, _pkg: pkg, _app: appName }));
    mode.value = "single";
    batchSummary.value = "";
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

// 批量拉取：读 Config 里启用的应用，并行拉取，按各应用配置的**星级 + 日期**固定筛选后合并；
// 回复状态不在这里固定，留给页面上方控件实时筛选（见 filtered）。
async function handleBatchFetch() {
  if (loading.value || batchLoading.value) return;
  const config = loadPlayConfig();
  const enabled = config
    ? Object.entries(config.perApp).filter(([, c]) => c.enabled)
    : [];
  if (enabled.length === 0) {
    errorMsg.value =
      "未在 Config 子页（Play Console 拉取配置）启用任何应用。请先去勾选并保存。";
    return;
  }
  batchLoading.value = true;
  errorMsg.value = "";
  needRelogin.value = false;
  reviews.value = [];
  batchStarsByPkg.value = {};
  const nameMap = new Map(apps.value.map((a) => [a.package_name, a.display_name]));
  const all: TaggedReview[] = [];
  const parts: string[] = [];
  let failed = 0;
  let matchedTotal = 0; // 各应用 Config 星级+日期命中的总数（用于摘要「拉到」）

  await Promise.all(
    enabled.map(async ([pkg, cfg]) => {
      const appName = nameMap.get(pkg) || pkg;
      try {
        const list = await invoke<Review[]>("list_play_reviews", {
          packageName: pkg,
          maxPages: 5,
          translationLanguage: "zh-CN",
        });
        const range = computeRange(cfg.datePreset, {
          fromDate: cfg.customFromDate,
          toDate: cfg.customToDate,
        });
        const from = range.fromDate
          ? Math.floor(new Date(range.fromDate + "T00:00:00").getTime() / 1000)
          : 0;
        const to = range.toDate
          ? Math.floor(new Date(range.toDate + "T23:59:59").getTime() / 1000)
          : Number.MAX_SAFE_INTEGER;
        // 只按日期界定窗口，保留全部星级（供「回复后又更新」忽略星级时显示）；
        // 各 app 的 Config 星级记到 map，展示时再过滤（见 filtered）。
        batchStarsByPkg.value[pkg] = [...cfg.stars];
        const dated = list.filter(
          (r) => r.user_comment_ts >= from && r.user_comment_ts <= to
        );
        for (const r of dated) all.push({ ...r, _pkg: pkg, _app: appName });
        // 摘要里的条数按 Config 星级算（即默认（非 UPDATED）会显示的数量）。
        const matched = dated.filter((r) => cfg.stars.includes(r.star_rating)).length;
        matchedTotal += matched;
        parts.push(`${appName} ${matched} 条`);
      } catch (e: any) {
        const msg = String(e);
        if (msg.startsWith("NEED_RELOGIN_SCOPE") || msg.startsWith("NEED_RELOGIN:")) {
          needRelogin.value = true;
        }
        failed += 1;
        parts.push(`${appName} 失败`);
      }
    })
  );

  all.sort((a, b) => b.user_comment_ts - a.user_comment_ts);
  reviews.value = all;
  mode.value = "batch";
  batchSummary.value = `批量拉取 ${enabled.length} 个应用 · 拉到 ${matchedTotal} 条` +
    (failed > 0 ? `（${failed} 个失败）` : "") + ` · ${parts.join("、")}`;
  fetchedAt.value = Date.now();
  batchLoading.value = false;
}

const fromTs = computed(() => {
  if (!fromDate.value) return 0;
  return Math.floor(new Date(fromDate.value + "T00:00:00").getTime() / 1000);
});
const toTs = computed(() => {
  if (!toDate.value) return Number.MAX_SAFE_INTEGER;
  return Math.floor(new Date(toDate.value + "T23:59:59").getTime() / 1000);
});

// 回复状态筛选（两种模式共用，跟随页面上方控件）。
function matchesReplyState(r: TaggedReview): boolean {
  if (replyState.value === "ABSENT" && r.developer_reply) return false;
  if (replyState.value === "REPLIED" && !r.developer_reply) return false;
  if (
    replyState.value === "UPDATED" &&
    !(r.developer_reply && r.developer_reply_ts && r.user_comment_ts > r.developer_reply_ts)
  )
    return false;
  return true;
}

const filtered = computed(() => {
  // 批量模式：日期已在拉取时按各应用 Config 固定；星级按各应用 Config 过滤（上方星级控件
  // 不参与）；回复状态跟随页面控件实时筛选。「回复后又更新」忽略星级、显示所有星级。
  if (mode.value === "batch") {
    return reviews.value.filter((r) => {
      if (!matchesReplyState(r)) return false;
      if (replyState.value === "UPDATED") return true; // 忽略星级，显示所有星级
      const cs = batchStarsByPkg.value[r._pkg];
      return !cs || cs.includes(r.star_rating);
    });
  }
  // 单应用模式：星级（页面控件）+ 回复状态 + 页面日期范围。
  return reviews.value.filter(
    (r) =>
      matchesReplyState(r) &&
      // 「回复后又更新」忽略星级，显示全部星级（这种通常不在乎评分，要看全部）
      (replyState.value === "UPDATED" || stars.value.includes(r.star_rating)) &&
      r.user_comment_ts >= fromTs.value &&
      r.user_comment_ts <= toTs.value
  );
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
  // Play Console 只认 ABSENT/REPLIED；UPDATED 是本地自定义筛选，跳转时退回不带该参数。
  if (replyState.value === "ABSENT" || replyState.value === "REPLIED") {
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

// 跳转到 Play Console 中的某条具体评论。developerId/appId 未填时退回到带筛选的
// 评论列表页（playConsoleUrl）。URL 格式来自 Console 实测复制：
//   .../user-feedback/review-details?reviewId=<uuid>&corpus=PUBLIC_REVIEWS
// 我们的 review_id 就是该 UUID（与 androidpublisher reviews API 一致），可直接定位。
function reviewConsoleUrl(r: TaggedReview): string {
  // appId 是为当前选中应用配的；批量模式下该评论可能来自别的应用，深链不适用 →
  // 退回应用列表页，避免跳到错误应用。
  if (!developerId.value || !appId.value || r._pkg !== packageName.value) {
    return playConsoleUrl.value;
  }
  const base = `https://play.google.com/console/u/0/developers/${developerId.value}/app/${appId.value}/user-feedback/review-details`;
  return `${base}?reviewId=${encodeURIComponent(r.review_id)}&corpus=PUBLIC_REVIEWS`;
}

async function openReviewInConsole(r: TaggedReview) {
  try {
    await openUrl(reviewConsoleUrl(r));
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

// ── AI 回复 ──────────────────────────────────────────────────────────────
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

// 多任务 AI 回复：每条评论一个独立任务，可同时存在多个（缩小成右下角悬浮条）。
// 生成是「排队」的——后端一次只跑一个 generate_single_reply，前端用 genBusy +
// processQueue 串行化，避免后端「已有任务在进行中」的拒绝。reply-log 路由到当前
// 正在生成的那个任务。
type AiStatus = "idle" | "queued" | "generating" | "done" | "error";
interface AiTask {
  id: string;
  review: TaggedReview;
  pkg: string; // 来源应用包名（批量模式下每条可能不同）
  appLabel: string;
  instruction: string;
  language: string;
  status: AiStatus;
  submitting: boolean;
  candidates: GenCandidate[];
  selectedIdx: number; // -1 = 未选 / 已手动编辑
  editText: string;
  error: string;
  log: string;
  usage: GenReplyResult["usage"];
}

const aiTasks = ref<AiTask[]>([]);
const activeTaskId = ref<string | null>(null); // 当前展开的任务；null = 全部缩小 / 无
let taskSeq = 0;
let genBusy = false; // 队列调度：是否有任务正在调用后端

const activeTask = computed(
  () => aiTasks.value.find((t) => t.id === activeTaskId.value) ?? null
);
// 非展开态的任务都以悬浮条形式堆在右下角（含进行中、已就绪、出错等）
const minimizedTasks = computed(() =>
  aiTasks.value.filter((t) => t.id !== activeTaskId.value)
);

// 「添加模板」：把觉得好的 AI 候选收录进该应用对应产品的模板库。
// 仅英文候选可收（模板库以英文为源）；收录时内联填类别。
const addTplIdx = ref(-1); // 当前展开收录面板的候选 index，-1 = 关闭
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
  // 其它语言（ru/pt/...）→ 用中文预览存中文模板，缺中文预览才退回原文
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
  const pkg = activeTask.value?.pkg || packageName.value.trim();
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

function openAiDialog(r: TaggedReview) {
  // 已为该评论建过任务就直接展开它（避免重复 / 丢状态）
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
  // 回复方向可留空：留空时后端让 AI 据评论自行判断方向。
  if (task.status === "generating" || task.status === "queued") return;
  addTplIdx.value = -1; // 重新生成后候选会变，关掉收录面板
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
    processQueue(); // 接着跑下一个排队中的任务
  }
}

async function stopTask(task: AiTask) {
  if (task.status === "queued") {
    // 还没轮到它，直接出队即可，不碰后端
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
  // 手动改动后取消"选中"高亮，但保留文本
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
  // 不用 window.confirm —— Tauri webview 里它常返回 false / 不弹，会卡住提交。
  // 按钮文案本身就是「确认提交到 Play」，已是明确的确认动作。
  task.submitting = true;
  task.error = "";
  try {
    await invoke("reply_to_review", {
      packageName: task.pkg,
      reviewId: task.review.review_id,
      replyText: text,
    });
    // 本地回填，UI 立即反映为「已回复」
    task.review.developer_reply = text;
    task.review.developer_reply_ts = Math.floor(Date.now() / 1000);
    aiTasks.value = aiTasks.value.filter((t) => t.id !== task.id);
    if (activeTaskId.value === task.id) activeTaskId.value = null;
  } catch (e: any) {
    task.error = String(e);
  } finally {
    task.submitting = false;
  }
}

// ── 模板回复（快捷取用收藏模板，按评论语言自动填好译文）─────────────────────
// 与「AI 回复」是两条独立路径：模板回复不调 LLM，直接从本地收藏的模板里挑一条，
// 按评论语言匹配预存译文填入即可提交。
interface TemplateView {
  id: string;
  category: string;
  text: string;
  lang: string; // 源语言 en / zh-CN
  translations: Record<string, string>;
  src_hash: string;
  stale: boolean;
}
type FavTemplate = TemplateView & { product: string };

const tplDlgReview = ref<TaggedReview | null>(null); // null = 弹窗关闭
const tplLoading = ref(false);
const tplError = ref("");
const tplGeneral = ref<FavTemplate[]>([]); // 「通用」产品下的收藏
const tplSpecific = ref<FavTemplate[]>([]); // 该评论 app 专属产品下的收藏
const tplSpecificProduct = ref(""); // 专属产品名（弹窗分组标题用）
const tplReplyText = ref("");
const tplSelectedId = ref("");
const tplUsedLang = ref(""); // 实际填入用的语言码
const tplFallback = ref(false); // 无对应语言译文、回退英文源文
const tplWantedLang = ref(""); // 回退时记下原本想要的语言（提示用）
const tplSubmitting = ref(false);

const tplReplyLen = computed(() => [...tplReplyText.value].length);
const tplHasFavs = computed(
  () => tplGeneral.value.length > 0 || tplSpecific.value.length > 0
);

// 评论语言（ISO/BCP-47）→ 模板译文 key（app 原生码）。规则对齐 review-reply skill：
// zh-CN/zh-Hans→zh-rCN、zh-TW/zh-Hant→zh-rTW、id→in，其余取主子标签匹配。
function normReviewLang(iso: string): string {
  let l = (iso || "").trim().toLowerCase().replace(/_/g, "-");
  if (l === "zh-cn" || l === "zh-hans" || l === "zh") return "zh-rcn";
  if (l === "zh-tw" || l === "zh-hant") return "zh-rtw";
  if (l === "id") return "in";
  return l;
}

// 给一条模板挑出该评论语言对应的正文。先判是否就是源语言（直接用 text），
// 再在 translations 里精确 / 主子标签匹配，都没有则回退英文源文（fallback=true）。
function resolveTplText(
  t: FavTemplate,
  reviewerLang: string
): { text: string; lang: string; fallback: boolean } {
  const norm = (s: string) => s.trim().toLowerCase().replace(/_/g, "-");
  const l = normReviewLang(reviewerLang);
  const prim = l ? l.split("-")[0] : "en";
  // 源语言直接用 text（无评论语言时也按源语言）
  if (t.lang === "en" && (prim === "en" || !l)) return { text: t.text, lang: "en", fallback: false };
  if (t.lang === "zh-CN" && prim === "zh") return { text: t.text, lang: "zh-CN", fallback: false };
  // translations：先精确，再主子标签
  const keys = Object.keys(t.translations || {});
  let hit = keys.find((k) => norm(k) === l);
  if (!hit) hit = keys.find((k) => norm(k).split("-")[0] === prim);
  if (hit) return { text: t.translations[hit], lang: hit, fallback: false };
  // 回退英文源文（其它源语言也回退到它的 text）
  return { text: t.text, lang: t.lang, fallback: true };
}

async function openTplDialog(r: TaggedReview) {
  tplDlgReview.value = r;
  tplReplyText.value = "";
  tplSelectedId.value = "";
  tplError.value = "";
  tplFallback.value = false;
  tplUsedLang.value = "";
  tplWantedLang.value = "";
  tplGeneral.value = [];
  tplSpecific.value = [];
  tplSpecificProduct.value = "";
  tplLoading.value = true;
  try {
    const favs = loadFavIds();
    // 通用产品（可能不存在 → 返回空）
    const gen = await invoke<TemplateView[]>("list_templates", { product: "通用" }).catch(() => []);
    tplGeneral.value = gen.filter((t) => favs.has(t.id)).map((t) => ({ ...t, product: "通用" }));
    // 该评论来源 app 的专属产品
    const product = await invoke<string | null>("product_for_package", {
      packageName: r._pkg,
    }).catch(() => null);
    if (product && product !== "通用") {
      tplSpecificProduct.value = product;
      const sp = await invoke<TemplateView[]>("list_templates", { product }).catch(() => []);
      tplSpecific.value = sp.filter((t) => favs.has(t.id)).map((t) => ({ ...t, product }));
    }
  } catch (e: any) {
    tplError.value = String(e);
  } finally {
    tplLoading.value = false;
  }
}

function applyTemplate(item: FavTemplate) {
  const r = tplDlgReview.value;
  if (!r) return;
  const res = resolveTplText(item, r.reviewer_language || "");
  tplReplyText.value = res.text;
  tplSelectedId.value = item.id;
  tplUsedLang.value = res.lang;
  tplFallback.value = res.fallback;
  tplWantedLang.value = res.fallback ? r.reviewer_language || "" : "";
  tplError.value = "";
}

function closeTplDialog() {
  if (tplSubmitting.value) return;
  tplDlgReview.value = null;
}

async function submitTplReply() {
  const r = tplDlgReview.value;
  if (!r || tplSubmitting.value) return;
  const text = tplReplyText.value.trim();
  if (!text) {
    tplError.value = "回复内容为空。";
    return;
  }
  if ([...text].length > 350) {
    tplError.value = `回复超过 350 字符（当前 ${[...text].length}），请精简后再提交。`;
    return;
  }
  tplSubmitting.value = true;
  tplError.value = "";
  try {
    await invoke("reply_to_review", {
      packageName: r._pkg,
      reviewId: r.review_id,
      replyText: text,
    });
    r.developer_reply = text;
    r.developer_reply_ts = Math.floor(Date.now() / 1000);
    tplDlgReview.value = null;
  } catch (e: any) {
    tplError.value = String(e);
  } finally {
    tplSubmitting.value = false;
  }
}

// ── 评论分析（🔍 单条分析：分析问题 + 推荐回复）─────────────────────────────
// 与「AI 回复」平行的一条独立路径：点「🔍 分析」即弹窗并自动开跑，分析 app 知识块
// 注入的提示词，给出分类/问题/信息缺口 + 一条可直接发布的推荐回复。多任务、可缩小，
// 生成串行（后端 AnalysisState 一次只跑一个，前端 anBusy + processAnQueue 排队），
// 走独立的 analysis-log 事件通道，与 AI 回复互不污染。
interface AnalysisReply {
  language: string;
  text: string;
  text_zh: string;
  char_count: number;
}
interface AnalysisData {
  category: string;
  issues: string[];
  info_gaps: string[];
  analysis: string;
  reply: AnalysisReply;
}
interface AnalysisResultRaw {
  analysis: AnalysisData;
  usage: GenReplyResult["usage"];
}

type AnStatus = "idle" | "queued" | "generating" | "done" | "error";
interface AnTask {
  id: string;
  review: TaggedReview;
  pkg: string;
  appLabel: string;
  language: string;
  status: AnStatus;
  submitting: boolean;
  data: AnalysisData | null;
  editText: string; // 推荐回复，可微调后提交
  error: string;
  log: string;
  usage: GenReplyResult["usage"];
}

const anTasks = ref<AnTask[]>([]);
const activeAnId = ref<string | null>(null);
let anSeq = 0;
let anBusy = false; // 队列调度：是否有分析正在调后端

const activeAn = computed(
  () => anTasks.value.find((t) => t.id === activeAnId.value) ?? null
);
const minimizedAnTasks = computed(() =>
  anTasks.value.filter((t) => t.id !== activeAnId.value)
);

let unlistenAnalysisLog: UnlistenFn | null = null;
onMounted(async () => {
  unlistenAnalysisLog = await listen<{ text: string; kind: string; done: boolean }>(
    "analysis-log",
    (e) => {
      const gen = anTasks.value.find((t) => t.status === "generating");
      if (gen) gen.log = e.payload.text;
    }
  );
});
onUnmounted(() => {
  if (unlistenAnalysisLog) unlistenAnalysisLog();
});

function openAnalysis(r: TaggedReview) {
  // 已为该评论建过分析任务就直接展开它（避免重复 / 丢状态）
  const existing = anTasks.value.find((t) => t.review.review_id === r.review_id);
  if (existing) {
    activeAnId.value = existing.id;
    return;
  }
  const task: AnTask = {
    id: `an-${++anSeq}`,
    review: r,
    pkg: r._pkg,
    appLabel: r._app,
    language: "auto",
    status: "idle",
    submitting: false,
    data: null,
    editText: "",
    error: "",
    log: "",
    usage: null,
  };
  anTasks.value.push(task);
  activeAnId.value = task.id;
  enqueueAnalysis(task); // 打开即自动开跑
}

function closeAnTask(task: AnTask) {
  if (task.status === "generating" || task.submitting) return;
  anTasks.value = anTasks.value.filter((t) => t.id !== task.id);
  if (activeAnId.value === task.id) activeAnId.value = null;
}
function minimizeAn() {
  activeAnId.value = null;
}
function restoreAn(id: string) {
  activeAnId.value = id;
}

function enqueueAnalysis(task: AnTask) {
  if (task.status === "generating" || task.status === "queued") return;
  task.status = "queued";
  task.error = "";
  task.log = "";
  task.data = null;
  processAnQueue();
}

async function processAnQueue() {
  if (anBusy) return;
  const next = anTasks.value.find((t) => t.status === "queued");
  if (!next) return;
  anBusy = true;
  next.status = "generating";
  try {
    const res = await invoke<AnalysisResultRaw>("generate_analysis", {
      review: next.review,
      product: next.appLabel || next.pkg,
      packageName: next.pkg,
      language: next.language,
      model: modelConfig.value.analysis,
    });
    next.data = res.analysis ?? null;
    next.usage = res.usage;
    next.editText = next.data?.reply?.text ?? "";
    if (!next.data) {
      next.status = "error";
      next.error = "未返回分析结果，请重试。";
    } else {
      next.status = "done";
    }
  } catch (e: any) {
    const msg = String(e);
    next.error = msg === "CANCELLED" ? "已取消分析。" : msg;
    next.status = "error";
  } finally {
    anBusy = false;
    processAnQueue(); // 接着跑下一个排队中的任务
  }
}

async function stopAnTask(task: AnTask) {
  if (task.status === "queued") {
    task.status = task.data ? "done" : "idle";
    return;
  }
  if (task.status === "generating") {
    try {
      await invoke("stop_analysis");
    } catch {
      // ignore
    }
  }
}

function anEditLen(task: AnTask | null): number {
  return task ? [...task.editText].length : 0;
}

async function submitAnReply(task: AnTask) {
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
    task.review.developer_reply = text;
    task.review.developer_reply_ts = Math.floor(Date.now() / 1000);
    anTasks.value = anTasks.value.filter((t) => t.id !== task.id);
    if (activeAnId.value === task.id) activeAnId.value = null;
  } catch (e: any) {
    task.error = String(e);
  } finally {
    task.submitting = false;
  }
}
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
        <button class="fetch-btn" :disabled="loading || batchLoading || !packageName.trim()" @click="handleFetch">
          {{ loading ? "拉取中..." : "拉取评论" }}
        </button>
        <button
          class="batch-fetch-btn"
          :disabled="loading || batchLoading"
          @click="handleBatchFetch"
          title="按 Config 子页（Play Console 拉取配置）启用的应用并行批量拉取，各应用按其配置的星级/状态/日期筛选"
        >
          {{ batchLoading ? "批量拉取中..." : "📦 批量拉取" }}
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
            :disabled="replyState === 'UPDATED' || mode === 'batch'"
            @click="toggleStar(s)"
          >{{ s }} ★</button>
          <span v-if="mode === 'batch' && replyState === 'UPDATED'" class="star-hint">回复后又更新 · 显示所有星级</span>
          <span v-else-if="mode === 'batch'" class="star-hint">批量模式星级按各应用 Config，不在此筛选</span>
          <span v-else-if="replyState === 'UPDATED'" class="star-hint">已忽略 · 显示全部星级</span>
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
          <label class="radio-item">
            <input type="radio" v-model="replyState" value="UPDATED" /> 回复后又更新
          </label>
        </div>
      </div>

      <div class="form-row">
        <label class="form-label">日期范围</label>
        <div class="date-row">
          <input type="date" v-model="fromDate" class="date-input" :min="minSelectableDate" :max="maxSelectableDate" />
          <span class="date-sep">→</span>
          <input type="date" v-model="toDate" class="date-input" :min="minSelectableDate" :max="maxSelectableDate" />
          <div class="presets">
            <button class="preset-btn" @click="setDatePreset(1)">今天</button>
            <button class="preset-btn" @click="setSinceLastWorkday">自上一个工作日</button>
            <button class="preset-btn" @click="setDatePreset(7)">近 7 天</button>
          </div>
          <span class="date-hint">⚠️ 仅能在最近 7 天内筛选（API 只返回最近约 7 天的评论）</span>
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

    <div v-if="mode === 'batch' && batchSummary" class="summary-row">
      <span class="summary-text">{{ batchSummary }} · 当前筛选显示 {{ filtered.length }} 条</span>
      <span class="summary-app">· 星级/日期按各应用 Config；回复状态可在上方实时筛选</span>
    </div>
    <div v-else-if="summary" class="summary-row">
      <span class="summary-text">{{ summary }}</span>
      <span v-if="selectedAppLabel" class="summary-app">· {{ selectedAppLabel }}</span>
    </div>

    <div v-if="filtered.length > 0" class="review-list">
      <article v-for="r in filtered" :key="r.review_id" class="review-card">
        <div class="review-head">
          <span v-if="mode === 'batch'" class="app-badge">{{ r._app }}</span>
          <span class="stars" :class="`stars-${r.star_rating}`">{{ starsDisplay(r.star_rating) }}</span>
          <span class="author">{{ r.author_name || "(匿名)" }}</span>
          <span class="ts">{{ formatTs(r.user_comment_ts) }}</span>
          <button class="an-btn" @click="openAnalysis(r)" title="分析这条评论暴露的问题并给出推荐回复">
            🔍 分析
          </button>
          <button class="tpl-reply-btn" @click="openTplDialog(r)" title="用收藏的常用模板快速回复（按评论语言自动填译文）">
            📋 模板回复
          </button>
          <button class="ai-btn" @click="openAiDialog(r)">
            🤖 {{ r.developer_reply ? "AI 重新回复" : "AI 回复" }}
          </button>
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
        <div class="review-actions">
          <button class="web-btn" @click="openReviewInConsole(r)" title="在 Play Console 中打开该评论">
            🌐 在网页中打开
          </button>
        </div>
      </article>
    </div>

    <div v-else-if="fetchedAt && !loading" class="empty-state">
      当前筛选条件下没有评论。
    </div>

    <div v-else-if="!fetchedAt && !loading" class="empty-state">
      选择应用后点「拉取评论」—— API 一次会拉最近 7 天全部评论，之后切换筛选不需要重新请求。
    </div>

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

    <!-- 评论分析弹窗（展开中的任务） -->
    <div v-if="activeAn" class="ai-overlay" @click.self="minimizeAn">
      <div class="ai-dialog">
        <div class="ai-dialog-head">
          <span class="ai-title">🔍 评论分析</span>
          <div class="ai-head-btns">
            <button class="ai-min" title="缩小（分析继续）" @click="minimizeAn">—</button>
            <button
              class="ai-close"
              :disabled="activeAn.status === 'generating' || activeAn.submitting"
              @click="closeAnTask(activeAn)"
            >✕</button>
          </div>
        </div>

        <div class="ai-review-quote">
          <span class="stars" :class="`stars-${activeAn.review.star_rating}`">{{ starsDisplay(activeAn.review.star_rating) }}</span>
          <div class="ai-quote-body">
            <div class="ai-quote-text">{{ activeAn.review.text || "(无文字)" }}</div>
            <div v-if="activeAn.review.original_text" class="ai-quote-orig">
              <span class="ai-quote-orig-label">原文：</span>{{ activeAn.review.original_text }}
            </div>
          </div>
        </div>

        <div class="ai-input-row">
          <label class="ai-label">回复语言</label>
          <select
            v-model="activeAn.language"
            class="ai-lang-select"
            :disabled="activeAn.status === 'generating' || activeAn.status === 'queued'"
          >
            <option v-for="o in LANG_OPTIONS" :key="o.value" :value="o.value">{{ o.label }}</option>
          </select>
          <button
            v-if="activeAn.status === 'generating'"
            class="ai-stop-btn"
            @click="stopAnTask(activeAn)"
          >■ 停止</button>
          <button
            v-else-if="activeAn.status === 'queued'"
            class="ai-stop-btn"
            @click="stopAnTask(activeAn)"
          >排队中…取消</button>
          <button
            v-else
            class="ai-gen-btn"
            @click="enqueueAnalysis(activeAn)"
          >{{ activeAn.data ? "重新分析" : "开始分析" }}</button>
        </div>

        <div v-if="activeAn.status === 'generating'" class="ai-generating">
          <span class="ai-spinner">⏳</span> 分析中…
          <span v-if="activeAn.log" class="ai-log">{{ activeAn.log }}</span>
        </div>
        <div v-else-if="activeAn.status === 'queued'" class="ai-generating">
          <span class="ai-spinner">⏳</span> 排队等待中…（前面还有任务在分析）
        </div>

        <div v-if="activeAn.error" class="ai-error">{{ activeAn.error }}</div>

        <div v-if="activeAn.data" class="an-result">
          <div class="an-cat-row">
            <span class="an-cat">{{ activeAn.data.category }}</span>
          </div>
          <div v-if="activeAn.data.issues && activeAn.data.issues.length" class="an-section">
            <div class="an-h">推断的用户问题</div>
            <ol class="an-list">
              <li v-for="(it, i) in activeAn.data.issues" :key="i">{{ it }}</li>
            </ol>
          </div>
          <div v-if="activeAn.data.info_gaps && activeAn.data.info_gaps.length" class="an-section">
            <div class="an-h">信息缺口</div>
            <ul class="an-list">
              <li v-for="(g, i) in activeAn.data.info_gaps" :key="i">{{ g }}</li>
            </ul>
          </div>
          <div v-if="activeAn.data.analysis" class="an-section">
            <div class="an-h">总体判断与处理方向</div>
            <div class="an-analysis">{{ activeAn.data.analysis }}</div>
          </div>
        </div>

        <div v-if="activeAn.data" class="ai-final">
          <label class="ai-label">推荐回复（可手动微调后提交）</label>
          <div v-if="activeAn.data.reply && activeAn.data.reply.text_zh" class="an-reply-zh">
            <span class="an-reply-zh-label">中文：</span>{{ activeAn.data.reply.text_zh }}
          </div>
          <textarea v-model="activeAn.editText" class="ai-final-text" rows="4"></textarea>
          <div class="ai-final-foot">
            <span class="ai-charcount" :class="{ over: anEditLen(activeAn) > 350 }">{{ anEditLen(activeAn) }} / 350</span>
            <button
              class="ai-submit-btn"
              :disabled="activeAn.submitting || !activeAn.editText.trim() || anEditLen(activeAn) > 350"
              @click="submitAnReply(activeAn)"
            >
              {{ activeAn.submitting ? "提交中…" : "确认提交到 Play" }}
            </button>
          </div>
        </div>

        <div v-if="activeAn.usage" class="ai-usage">
          💰 本次用量：输入 {{ activeAn.usage.input_tokens ?? 0 }} · 输出 {{ activeAn.usage.output_tokens ?? 0 }} tokens
          <span v-if="activeAn.usage.total_cost_usd"> · 约 ${{ activeAn.usage.total_cost_usd.toFixed(4) }}</span>
        </div>
      </div>
    </div>

    <!-- 缩小后的左下角悬浮条（分析任务，与 AI 回复分列两侧避免重叠） -->
    <div v-if="minimizedAnTasks.length" class="ai-mini-stack an-mini-stack">
      <div
        v-for="t in minimizedAnTasks"
        :key="t.id"
        class="ai-mini-bar"
        :class="{ 'is-error': t.status === 'error', 'is-done': t.status === 'done' }"
        @click="restoreAn(t.id)"
      >
        <span class="ai-mini-text">
          🔍 <span class="ai-mini-quote">{{ (t.review.text || t.review.original_text || "(无文字)").slice(0, 16) }}</span>
          <template v-if="t.status === 'generating'">· 分析中…</template>
          <template v-else-if="t.status === 'queued'">· 排队中</template>
          <template v-else-if="t.status === 'error'">· 失败</template>
          <template v-else-if="t.data">· 已就绪</template>
          <template v-else>· 待分析</template>
        </span>
        <button class="ai-mini-open" @click.stop="restoreAn(t.id)">展开</button>
      </div>
    </div>

    <!-- 模板回复弹窗 -->
    <div v-if="tplDlgReview" class="ai-overlay" @click.self="closeTplDialog">
      <div class="ai-dialog tpl-dialog">
        <div class="ai-dialog-head">
          <span class="ai-title">📋 模板回复</span>
          <button class="ai-close" :disabled="tplSubmitting" @click="closeTplDialog">✕</button>
        </div>

        <div class="ai-review-quote">
          <span class="stars" :class="`stars-${tplDlgReview.star_rating}`">{{ starsDisplay(tplDlgReview.star_rating) }}</span>
          <div class="ai-quote-body">
            <div class="ai-quote-text">{{ tplDlgReview.text || "(无文字)" }}</div>
            <div v-if="tplDlgReview.original_text" class="ai-quote-orig">
              <span class="ai-quote-orig-label">原文：</span>{{ tplDlgReview.original_text }}
            </div>
            <div class="tpl-lang-line">评论语言：{{ tplDlgReview.reviewer_language || "未知" }}</div>
          </div>
        </div>

        <div v-if="tplLoading" class="tpl-loading">加载收藏模板中…</div>
        <div v-else-if="tplError && !tplHasFavs" class="ai-error">{{ tplError }}</div>
        <div v-else-if="!tplHasFavs" class="tpl-empty">
          还没有收藏任何模板。去「模板管理」页给常用模板点 ★ 收藏，这里就会出现（分「通用」和该应用「专用」两组）。
        </div>
        <template v-else>
          <div v-if="tplGeneral.length" class="tpl-group">
            <div class="tpl-group-title">通用</div>
            <div class="tpl-btn-row">
              <button
                v-for="t in tplGeneral"
                :key="t.id"
                class="tpl-pick"
                :class="{ active: tplSelectedId === t.id }"
                @click="applyTemplate(t)"
              >{{ t.category || "未分类" }}</button>
            </div>
          </div>
          <div v-if="tplSpecific.length" class="tpl-group">
            <div class="tpl-group-title">专用 · {{ tplSpecificProduct }}</div>
            <div class="tpl-btn-row">
              <button
                v-for="t in tplSpecific"
                :key="t.id"
                class="tpl-pick"
                :class="{ active: tplSelectedId === t.id }"
                @click="applyTemplate(t)"
              >{{ t.category || "未分类" }}</button>
            </div>
          </div>
        </template>

        <div v-if="tplSelectedId || tplReplyText" class="ai-final">
          <label class="ai-label">回复内容（可手动微调）</label>
          <div v-if="tplFallback" class="tpl-fallback-note">
            ⚠️ 该模板没有「{{ tplWantedLang || "该语言" }}」译文，已用{{ tplUsedLang === "zh-CN" ? "中文" : "英文" }}源文填入，可手动改后再提交。
          </div>
          <div v-else-if="tplUsedLang" class="tpl-used-note">已按评论语言填入译文（{{ tplUsedLang }}）</div>
          <textarea v-model="tplReplyText" class="ai-final-text" rows="4"></textarea>
          <div class="ai-final-foot">
            <span class="ai-charcount" :class="{ over: tplReplyLen > 350 }">{{ tplReplyLen }} / 350</span>
            <button
              class="ai-submit-btn"
              :disabled="tplSubmitting || !tplReplyText.trim() || tplReplyLen > 350"
              @click="submitTplReply"
            >
              {{ tplSubmitting ? "提交中…" : "确认提交到 Play" }}
            </button>
          </div>
          <div v-if="tplError" class="ai-error">{{ tplError }}</div>
        </div>
      </div>
    </div>

    <!-- 缩小后的右下角悬浮条：竖直堆叠，每个任务一条 -->
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
.batch-fetch-btn {
  padding: 6px 14px;
  font-size: 13px;
  font-weight: 500;
  border: 1px solid #9f7aea;
  border-radius: 6px;
  background: white;
  color: #6b46c1;
  cursor: pointer;
  flex-shrink: 0;
}
.batch-fetch-btn:hover:not(:disabled) {
  background: #9f7aea;
  color: white;
}
.batch-fetch-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.app-badge {
  font-size: 10px;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 10px;
  background: #e9d8fd;
  color: #553c9a;
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
.star-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.star-hint {
  align-self: center;
  margin-left: 6px;
  font-size: 11px;
  color: #b7791f;
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
.date-hint {
  width: 100%;
  margin-top: 2px;
  font-size: 11px;
  color: #b7791f;
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

/* ── AI 回复 ── */
.review-actions {
  margin-top: 10px;
  display: flex;
  justify-content: flex-end;
}
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
.tpl-reply-btn {
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
.tpl-reply-btn:hover {
  background: #9f7aea;
  color: white;
}
.an-btn {
  padding: 3px 10px;
  font-size: 11px;
  font-weight: 500;
  line-height: 18px;
  border: 1px solid #38a169;
  border-radius: 6px;
  background: white;
  color: #2f855a;
  cursor: pointer;
  margin-left: auto;
  flex-shrink: 0;
}
.an-btn:hover {
  background: #38a169;
  color: white;
}

/* 分析结果区 */
.an-result {
  margin-top: 12px;
  padding: 10px 12px;
  background: #f7fafc;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
}
.an-cat-row {
  margin-bottom: 8px;
}
.an-cat {
  display: inline-block;
  font-size: 11px;
  font-weight: 600;
  padding: 2px 10px;
  border-radius: 10px;
  background: #c6f6d5;
  color: #22543d;
}
.an-section {
  margin-top: 10px;
}
.an-h {
  font-size: 11px;
  font-weight: 600;
  color: #4a5568;
  margin-bottom: 4px;
}
.an-list {
  margin: 0;
  padding-left: 18px;
  font-size: 12px;
  line-height: 1.6;
  color: #2d3748;
}
.an-analysis {
  font-size: 12px;
  line-height: 1.6;
  color: #2d3748;
}
.an-reply-zh {
  font-size: 12px;
  line-height: 1.5;
  color: #718096;
  margin-bottom: 6px;
}
.an-reply-zh-label {
  color: #a0aec0;
}

/* 分析任务的悬浮条堆到左下角，避开 AI 回复（右下角） */
.an-mini-stack {
  right: auto;
  left: 20px;
  align-items: flex-start;
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

/* ── 模板回复弹窗 ── */
.tpl-dialog {
  max-width: 560px;
}
.tpl-lang-line {
  margin-top: 6px;
  font-size: 11px;
  color: #6b46c1;
}
.tpl-loading,
.tpl-empty {
  font-size: 13px;
  color: #888;
  padding: 16px 4px;
  line-height: 1.6;
}
.tpl-group {
  margin-top: 12px;
}
.tpl-group-title {
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
  margin-bottom: 8px;
}
.tpl-btn-row {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.tpl-pick {
  padding: 6px 14px;
  font-size: 13px;
  border: 1px solid #cbd5e0;
  border-radius: 16px;
  background: white;
  color: #2d3748;
  cursor: pointer;
}
.tpl-pick:hover {
  border-color: #9f7aea;
  background: #faf5ff;
}
.tpl-pick.active {
  border-color: #9f7aea;
  background: #9f7aea;
  color: white;
}
.tpl-fallback-note {
  font-size: 12px;
  color: #975a16;
  background: #fffaf0;
  border: 1px solid #fbd38d;
  border-radius: 6px;
  padding: 5px 10px;
}
.tpl-used-note {
  font-size: 11px;
  color: #2f855a;
}
</style>

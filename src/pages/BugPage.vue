<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// 通过 MantisBT REST API（Base URL + 个人 API Token）拉某个项目的 bug 列表，
// 按关键字/版本号筛选后点开看详情，勾选后可一键调用 prd-risk-profiler skill
// 生成风险画像（见文件下方「多选 + PRD 风险画像」一节）。
//
// 为什么没有「选版本」下拉框：Mantis 自带的 version/fixed_in_version 字段
// 这边团队基本不用（实测常年是 null），版本号是手打在 summary 里的（如
// 「#bug 2.3.5内测 必现 -- ...」）。所以筛选做成「拉一次该项目全量 issue →
// 前端按关键字实时过滤」，版本号快捷筛选项从已拉到的 summary 里正则提取，
// 不依赖 Mantis 的版本注册表（该实例的 GET /projects/{id}/versions 端点实测
// 还 405，也用不了）。

interface MantisProject {
  id: number;
  name: string;
}
interface IssueSummary {
  id: number;
  summary: string;
  status: string;
  severity: string;
  priority: string;
  category: string;
  version: string;
  fixed_in_version: string;
  reporter: string;
  handler: string;
  updated_at: string;
}
interface MantisNote {
  id: number;
  reporter: string;
  text: string;
  created_at: string;
}
interface IssueDetail extends IssueSummary {
  description: string;
  steps_to_reproduce: string;
  additional_information: string;
  project: string;
  created_at: string;
  notes: MantisNote[];
}

// PRD 风险画像：勾选的 bug + 选一份 PRD（Slides）→ 调用 prd-risk-profiler skill
// 的「沉淀模式」，由 skill 自己更新它自己的知识库文件（通用层 risk-taxonomy.md
// + 专属层 apps/<APP名>.md）。这里只负责收集参数、正确调用、展示流式日志——
// 不解析/不生成那两份 md，见 decisions.md。
interface DriveFile {
  id: string;
  name: string;
  modified_time: string;
  mime_type: string;
}
interface RiskLogLine {
  text: string;
  kind: string; // info | text | tool | error
}
type RiskPhase = "config" | "running" | "done" | "error";

const LAST_PROJECT_KEY = "mantis-bug-last-project-v1";
const ISSUES_CACHE_KEY = "mantis-bug-issues-cache-v1";

interface IssuesCacheEntry {
  issues: IssueSummary[];
  fetchedAt: number;
}

// 与 mantis.rs::normalize_base 同一套逻辑：只取协议+域名(+端口)，丢弃路径。
// 用户很容易把浏览器地址栏当前页面的完整 URL（带 /view_all_bug_page.php 之类
// 路径）粘贴进来，这里算出「实际会请求的地址」显示出来，让人一眼看出填错了。
function normalizeBase(url: string): string {
  const trimmed = url.trim();
  const schemeEnd = trimmed.indexOf("://");
  const afterScheme = schemeEnd === -1 ? 0 : schemeEnd + 3;
  const slashIdx = trimmed.indexOf("/", afterScheme);
  if (slashIdx === -1) return trimmed.replace(/\/+$/, "");
  return trimmed.slice(0, slashIdx);
}

const baseUrl = ref("");
const apiToken = ref("");
const configSaved = ref(false);
const configExpanded = ref(true);
const configMsg = ref("");
const configError = ref(false);

const effectiveBaseUrl = computed(() => (baseUrl.value.trim() ? normalizeBase(baseUrl.value) : ""));

const projects = ref<MantisProject[]>([]);
const selectedProjectId = ref<number | null>(null);
const loadingProjects = ref(false);
const loadingIssues = ref(false);
const errorMsg = ref("");

const allIssues = ref<IssueSummary[]>([]);
const issuesCache = ref<Record<number, IssuesCacheEntry>>({});
const cachedAt = ref<number | null>(null);
const filterText = ref("");
const filterInputRef = ref<HTMLInputElement | null>(null);
const expandedId = ref<number | null>(null);
const detailCache = ref<Record<number, IssueDetail>>({});
const loadingDetailId = ref<number | null>(null);

// ── 多选 + PRD 风险画像 ────────────────────────────────────────────────────────
const selectedIssueIds = ref<Set<number>>(new Set());
const selectedCount = computed(() => selectedIssueIds.value.size);
const isAllFilteredSelected = computed(
  () =>
    filteredIssues.value.length > 0 &&
    filteredIssues.value.every((i) => selectedIssueIds.value.has(i.id))
);

function toggleIssueSelect(id: number) {
  const next = new Set(selectedIssueIds.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  selectedIssueIds.value = next;
}

/** 「全选当前筛选结果」——只增/删筛选范围内的 id，不影响筛选范围外已选的。 */
function toggleSelectAllFiltered() {
  const next = new Set(selectedIssueIds.value);
  if (isAllFilteredSelected.value) {
    for (const i of filteredIssues.value) next.delete(i.id);
  } else {
    for (const i of filteredIssues.value) next.add(i.id);
  }
  selectedIssueIds.value = next;
}

function clearSelection() {
  selectedIssueIds.value = new Set();
}

const showRiskModal = ref(false);
const riskMinimized = ref(false);
const riskPhase = ref<RiskPhase>("config");
const riskAppName = ref("");
const riskVersion = ref("");
const riskError = ref("");
const riskLogs = ref<RiskLogLine[]>([]);
const riskResultText = ref("");
const riskPreparing = ref(false);
const riskPreparingMsg = ref("");

const slidesFiles = ref<DriveFile[]>([]);
const loadingSlides = ref(false);
const slidesError = ref("");
const riskSlideId = ref<string | null>(null);

const productNameSuggestions = ref<string[]>([]);

let unlistenPrdRiskLog: (() => void) | null = null;

async function loadSlidesFiles() {
  loadingSlides.value = true;
  slidesError.value = "";
  try {
    slidesFiles.value = await invoke<DriveFile[]>("list_drive_files", {
      mimeType: "application/vnd.google-apps.presentation",
    });
  } catch (e: any) {
    slidesError.value = String(e);
    slidesFiles.value = [];
  } finally {
    loadingSlides.value = false;
  }
}

async function loadProductNameSuggestions() {
  try {
    const products = await invoke<Array<{ name: string }>>("list_template_products");
    productNameSuggestions.value = products.map((p) => p.name).filter((n) => n !== "通用");
  } catch {
    productNameSuggestions.value = [];
  }
}

function openRiskModal() {
  riskPhase.value = "config";
  riskMinimized.value = false;
  riskError.value = "";
  riskLogs.value = [];
  riskResultText.value = "";
  riskSlideId.value = null;
  const project = projects.value.find((p) => p.id === selectedProjectId.value);
  riskAppName.value = project?.name || "";
  // filterText 命中当前版本 chip 就带过去当默认版本号，否则留空手填
  riskVersion.value = versionChips.value.some((c) => c.name === filterText.value)
    ? filterText.value
    : "";
  showRiskModal.value = true;
  loadSlidesFiles();
  loadProductNameSuggestions();
}

function closeRiskModal() {
  if (riskPhase.value === "running") {
    riskMinimized.value = true;
    return;
  }
  showRiskModal.value = false;
}

/** 勾选但还没展开过详情的补拉——沉淀依据需要完整的问题描述/重现步骤/备注，
 *  不能只用列表里的摘要字段。复用 toggleExpand 里同一套 get_mantis_issue 调用。 */
async function ensureDetail(id: number): Promise<IssueDetail | null> {
  if (detailCache.value[id]) return detailCache.value[id];
  try {
    const detail = await invoke<IssueDetail>("get_mantis_issue", {
      baseUrl: baseUrl.value.trim(),
      apiToken: apiToken.value.trim(),
      issueId: id,
    });
    detailCache.value[id] = detail;
    return detail;
  } catch (e: any) {
    riskError.value = `拉取 #${id} 详情失败：${String(e)}`;
    return null;
  }
}

function pushRiskLog(text: string, kind = "text") {
  riskLogs.value.push({ text, kind });
}

async function startRiskGeneration() {
  const appName = riskAppName.value.trim();
  if (!appName) {
    riskError.value = "请填写 APP 名称";
    return;
  }
  const slide = slidesFiles.value.find((f) => f.id === riskSlideId.value);
  if (!slide) {
    riskError.value = "请先选一份 PRD（Slides）";
    return;
  }

  riskError.value = "";
  riskLogs.value = [];
  riskResultText.value = "";
  riskPhase.value = "running";

  if (!unlistenPrdRiskLog) {
    unlistenPrdRiskLog = await listen<{ text: string; kind: string; done: boolean }>(
      "prd-risk-log",
      (event) => {
        const { text, kind } = event.payload;
        if (text && text.trim()) pushRiskLog(text, kind);
      }
    );
  }

  try {
    riskPreparing.value = true;

    const ids = Array.from(selectedIssueIds.value).sort((a, b) => a - b);
    riskPreparingMsg.value = `正在拉取 ${ids.length} 条 bug 详情…`;
    const details = await Promise.all(ids.map((id) => ensureDetail(id)));
    const okDetails = details.filter((d): d is IssueDetail => d != null);
    if (okDetails.length === 0) {
      riskPreparing.value = false;
      riskPhase.value = "error";
      riskError.value = riskError.value || "所选 bug 详情全部拉取失败";
      return;
    }

    riskPreparingMsg.value = "正在导出 PRD 页面为图片…";
    const imagePaths = await invoke<string[]>("export_slides_pdf", {
      presentationId: slide.id,
      name: slide.name,
      pages: [],
    });

    riskPreparing.value = false;

    const bugReport = okDetails
      .map(
        (d, i) =>
          `=== Bug ${i + 1}/${okDetails.length} ===\n${formatForCopy(d)}`
      )
      .join("\n\n");

    const result = await invoke<string>("run_prd_risk_profiler", {
      appName,
      version: riskVersion.value.trim(),
      bugReport,
      prdImagePaths: imagePaths,
      model: null,
    });
    riskResultText.value = result;
    riskPhase.value = "done";
  } catch (e: any) {
    riskPreparing.value = false;
    riskPhase.value = "error";
    riskError.value = String(e);
  }
}

// 版本号只认「紧跟在开头 #bug 后面」的那一段数字（如 "#bug 2.3.5内测" →
// 2.3.5，"#bug2.2.0A 必现" → 2.2.0），不做"summary 里随便一个 x.y 数字都算"——
// 后者会把正文里提到的 Android 系统版本（如"4.4设备中颜色异常"里的 4.4）、
// 分辨率等无关数字也当成版本号，导致筛选出一堆根本不是那个版本的 bug。
// 数字后面常跟一个字母（A/B/C...），那是同一版本内部的批次/子包标识，不是
// 独立版本——2.3.5A/2.3.5B/2.3.5C 都算「2.3.5」这一个标签，要合并计数，
// 所以提取时直接丢弃这个字母，不放进 tag 里。
const VERSION_TAG_RE = /^#bug\s*([0-9]+(?:\.[0-9]+){1,2})[A-Za-z]?/i;

function extractVersionTag(summary: string): string | null {
  const m = summary.trim().match(VERSION_TAG_RE);
  return m ? m[1] : null;
}

const versionChips = computed(() => {
  const counts: Record<string, number> = {};
  for (const issue of allIssues.value) {
    const tag = extractVersionTag(issue.summary);
    if (tag) counts[tag] = (counts[tag] || 0) + 1;
  }
  return Object.entries(counts)
    .sort((a, b) => compareVersionDesc(a[0], b[0]))
    .slice(0, 12)
    .map(([name, count]) => ({ name, count }));
});

// 按版本号数值从大到小排（而非按出现次数），符合"最新版本排前面"的直觉；
// 逐段转数字比较，避免 "2.10" 被当字符串排到 "2.9" 前面这种坑。
function compareVersionDesc(a: string, b: string): number {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i++) {
    const diff = (pb[i] || 0) - (pa[i] || 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

const filteredIssues = computed(() => {
  const q = filterText.value.trim().toLowerCase();
  if (!q) return allIssues.value;
  return allIssues.value.filter(
    (i) =>
      String(i.id).includes(q) ||
      i.summary.toLowerCase().includes(q) ||
      i.version.toLowerCase() === q ||
      i.fixed_in_version.toLowerCase() === q
  );
});

onMounted(async () => {
  try {
    const cfg = await invoke<{ base_url: string; api_token: string }>("get_mantis_config");
    baseUrl.value = cfg.base_url || "";
    apiToken.value = cfg.api_token || "";
    configSaved.value = !!(cfg.base_url && cfg.api_token);
    configExpanded.value = !configSaved.value;
  } catch {}

  try {
    const raw = localStorage.getItem(LAST_PROJECT_KEY);
    if (raw) selectedProjectId.value = JSON.parse(raw);
  } catch {}

  try {
    const raw = localStorage.getItem(ISSUES_CACHE_KEY);
    if (raw) issuesCache.value = JSON.parse(raw);
  } catch {}

  if (selectedProjectId.value != null && issuesCache.value[selectedProjectId.value]) {
    loadFromCache(selectedProjectId.value);
  }

  if (configSaved.value) await loadProjects();

  window.addEventListener("keydown", handleFilterShortcut);
});

onUnmounted(() => {
  window.removeEventListener("keydown", handleFilterShortcut);
});

function handleFilterShortcut(e: KeyboardEvent) {
  if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "f") return;
  if (allIssues.value.length === 0) return;
  e.preventDefault();
  filterInputRef.value?.focus();
  filterInputRef.value?.select();
}

function loadFromCache(projectId: number) {
  const entry = issuesCache.value[projectId];
  if (!entry) return;
  allIssues.value = entry.issues;
  cachedAt.value = entry.fetchedAt;
}

function persistIssuesCache() {
  try {
    localStorage.setItem(ISSUES_CACHE_KEY, JSON.stringify(issuesCache.value));
  } catch {}
}

function formatCachedAt(ts: number): string {
  return new Date(ts).toLocaleString();
}

watch(selectedProjectId, () => {
  try {
    localStorage.setItem(LAST_PROJECT_KEY, JSON.stringify(selectedProjectId.value));
  } catch {}
});

async function saveConfig() {
  configMsg.value = "";
  configError.value = false;
  const url = baseUrl.value.trim();
  const token = apiToken.value.trim();
  if (!url || !token) {
    configError.value = true;
    configMsg.value = "Base URL 和 API Token 都要填";
    return;
  }
  try {
    await invoke("save_mantis_config", { config: { base_url: url, api_token: token } });
    configSaved.value = true;
    configMsg.value = "已保存，正在测试连接…";
    await loadProjects();
    if (errorMsg.value) {
      configError.value = true;
      configMsg.value = "已保存，但连接测试失败：" + errorMsg.value;
    } else {
      configMsg.value = `已保存并连接成功，读到 ${projects.value.length} 个项目`;
      configExpanded.value = false;
    }
  } catch (e: any) {
    configError.value = true;
    configMsg.value = "保存失败：" + String(e);
  }
}

async function loadProjects() {
  loadingProjects.value = true;
  errorMsg.value = "";
  try {
    projects.value = await invoke<MantisProject[]>("list_mantis_projects", {
      baseUrl: baseUrl.value.trim(),
      apiToken: apiToken.value.trim(),
    });
  } catch (e: any) {
    errorMsg.value = String(e);
    projects.value = [];
  } finally {
    loadingProjects.value = false;
  }
}

function onProjectChange() {
  filterText.value = "";
  expandedId.value = null;
  errorMsg.value = "";
  const pid = selectedProjectId.value;
  if (pid != null && issuesCache.value[pid]) {
    loadFromCache(pid);
  } else {
    allIssues.value = [];
    cachedAt.value = null;
  }
}

/** 拉一次 Mantis 全量 issue 并落 localStorage 缓存（按 project id 分开存）。
 *  「拉取」和「刷新」是同一个函数——刷新就是重新拉一遍覆盖旧缓存。 */
async function fetchIssues() {
  if (selectedProjectId.value == null) {
    errorMsg.value = "先选一个项目";
    return;
  }
  const pid = selectedProjectId.value;
  loadingIssues.value = true;
  errorMsg.value = "";
  expandedId.value = null;
  try {
    const result = await invoke<IssueSummary[]>("list_mantis_issues", {
      baseUrl: baseUrl.value.trim(),
      apiToken: apiToken.value.trim(),
      projectId: pid,
    });
    allIssues.value = result;
    cachedAt.value = Date.now();
    issuesCache.value[pid] = { issues: result, fetchedAt: cachedAt.value };
    persistIssuesCache();
  } catch (e: any) {
    errorMsg.value = String(e);
    allIssues.value = [];
    cachedAt.value = null;
  } finally {
    loadingIssues.value = false;
  }
}

function toggleChip(name: string) {
  filterText.value = filterText.value === name ? "" : name;
}

async function toggleExpand(issue: IssueSummary) {
  if (expandedId.value === issue.id) {
    expandedId.value = null;
    return;
  }
  expandedId.value = issue.id;
  if (!detailCache.value[issue.id]) {
    loadingDetailId.value = issue.id;
    try {
      const detail = await invoke<IssueDetail>("get_mantis_issue", {
        baseUrl: baseUrl.value.trim(),
        apiToken: apiToken.value.trim(),
        issueId: issue.id,
      });
      detailCache.value[issue.id] = detail;
    } catch (e: any) {
      errorMsg.value = String(e);
      expandedId.value = null;
    } finally {
      loadingDetailId.value = null;
    }
  }
}

function formatForCopy(d: IssueDetail): string {
  const lines = [
    `#${d.id} [${d.project}] ${d.summary}`,
    `状态: ${d.status || "-"} | 严重程度: ${d.severity || "-"} | 优先级: ${d.priority || "-"} | 分类: ${d.category || "-"}`,
    `版本: ${d.version || "-"} | 修复于: ${d.fixed_in_version || "-"}`,
    `报告人: ${d.reporter || "-"} | 处理人: ${d.handler || "-"}`,
    `创建: ${d.created_at || "-"} | 更新: ${d.updated_at || "-"}`,
    "",
    "【问题描述】",
    d.description || "(无)",
  ];
  if (d.steps_to_reproduce) {
    lines.push("", "【重现步骤】", d.steps_to_reproduce);
  }
  if (d.additional_information) {
    lines.push("", "【补充信息】", d.additional_information);
  }
  if (d.notes && d.notes.length > 0) {
    lines.push("", "【备注】");
    for (const n of d.notes) {
      lines.push(`- [${n.reporter || "?"} @ ${n.created_at || "?"}] ${n.text}`);
    }
  }
  return lines.join("\n");
}

</script>

<template>
  <div class="bug-page">
    <header class="page-header">
      <h3>Bug 详情（MantisBT）</h3>
      <p class="subtitle">
        通过 MantisBT REST API + 个人 API Token 拉某个项目的 bug 列表，按关键字/版本号筛选后
        点开看详情；勾选后可一键调用 prd-risk-profiler skill 生成风险画像。Token 在 Mantis「我的
        账户 → API 令牌」页生成，权限对应你自己账号能看到的范围。
      </p>
    </header>

    <section class="config-card">
      <div class="config-head" @click="configExpanded = !configExpanded">
        <span class="config-title">Mantis 连接配置</span>
        <span class="config-status" v-if="configSaved && !configExpanded">已配置 ✓</span>
        <span class="chevron" :class="{ open: configExpanded }">▾</span>
      </div>
      <div v-show="configExpanded" class="config-body">
        <div class="form-row">
          <label class="form-label">Base URL</label>
          <input v-model="baseUrl" class="text-input" placeholder="https://bugs.example.com（只填到域名，不要带页面路径）" />
        </div>
        <div class="form-row effective-url-row" v-if="effectiveBaseUrl">
          <label class="form-label"></label>
          <span class="effective-url">实际会请求：{{ effectiveBaseUrl }}/api/rest/index.php/…</span>
        </div>
        <div class="form-row">
          <label class="form-label">API Token</label>
          <input v-model="apiToken" type="password" class="text-input" placeholder="个人 API 令牌" />
        </div>
        <div class="form-foot">
          <button class="fetch-btn" @click="saveConfig">保存并测试连接</button>
        </div>
        <div v-if="configMsg" class="config-msg" :class="{ error: configError }">{{ configMsg }}</div>
      </div>
    </section>

    <section v-if="configSaved" class="picker-card">
      <div class="form-row">
        <label class="form-label">项目</label>
        <select
          v-model="selectedProjectId"
          class="src-select"
          :disabled="loadingProjects"
          @change="onProjectChange"
        >
          <option :value="null">{{ loadingProjects ? "加载中…" : "选择项目" }}</option>
          <option v-for="p in projects" :key="p.id" :value="p.id">{{ p.name }}</option>
        </select>
        <button class="icon-btn" title="刷新项目列表" @click="loadProjects">⟳</button>
        <button class="fetch-btn" :disabled="selectedProjectId == null || loadingIssues" @click="fetchIssues">
          {{ loadingIssues ? "拉取中…" : allIssues.length > 0 ? "⟳ 刷新" : "拉取 Bug 列表" }}
        </button>
      </div>
      <div class="form-row" v-if="allIssues.length > 0">
        <label class="form-label">筛选</label>
        <input
          ref="filterInputRef"
          v-model="filterText"
          class="text-input"
          placeholder="按 Bug ID/版本号/关键字筛选（如 196943 或 2.3.5），空则显示全部（⌘F 快速定位）"
        />
      </div>
      <div class="cache-hint" v-if="cachedAt">
        数据来自缓存 · 拉取于 {{ formatCachedAt(cachedAt) }} · 点「刷新」重新拉取
      </div>
      <div class="chip-row" v-if="versionChips.length > 0">
        <span
          v-for="c in versionChips"
          :key="c.name"
          class="chip"
          :class="{ active: filterText === c.name }"
          @click="toggleChip(c.name)"
        >
          {{ c.name }} · {{ c.count }}
        </span>
      </div>
      <label class="select-all-row" v-if="filteredIssues.length > 0">
        <input
          type="checkbox"
          :checked="isAllFilteredSelected"
          @change="toggleSelectAllFiltered"
        />
        全选当前筛选结果（{{ filteredIssues.length }} 条）
      </label>
    </section>

    <div v-if="errorMsg" class="banner banner-error">{{ errorMsg }}</div>

    <section v-if="filteredIssues.length > 0" class="issue-list">
      <div class="issue-count">共 {{ filteredIssues.length }} 条（全量 {{ allIssues.length }} 条）</div>
      <article v-for="issue in filteredIssues" :key="issue.id" class="issue-item">
        <div class="issue-row" @click="toggleExpand(issue)">
          <input
            type="checkbox"
            class="issue-checkbox"
            :checked="selectedIssueIds.has(issue.id)"
            @click.stop="toggleIssueSelect(issue.id)"
          />
          <span class="issue-id">#{{ issue.id }}</span>
          <span class="issue-summary">{{ issue.summary }}</span>
          <span class="issue-tag status">{{ issue.status || "-" }}</span>
          <span class="issue-tag severity">{{ issue.severity || "-" }}</span>
          <span class="chevron" :class="{ open: expandedId === issue.id }">▾</span>
        </div>
        <div v-if="expandedId === issue.id" class="issue-detail">
          <div v-if="loadingDetailId === issue.id" class="detail-loading">加载详情中…</div>
          <template v-else-if="detailCache[issue.id]">
            <div class="detail-meta">
              <span>版本: {{ detailCache[issue.id].version || "-" }}</span>
              <span>修复于: {{ detailCache[issue.id].fixed_in_version || "-" }}</span>
              <span>分类: {{ detailCache[issue.id].category || "-" }}</span>
              <span>优先级: {{ detailCache[issue.id].priority || "-" }}</span>
              <span>报告人: {{ detailCache[issue.id].reporter || "-" }}</span>
              <span>处理人: {{ detailCache[issue.id].handler || "-" }}</span>
              <span>创建: {{ detailCache[issue.id].created_at || "-" }}</span>
              <span>更新: {{ detailCache[issue.id].updated_at || "-" }}</span>
            </div>
            <div class="detail-block">
              <div class="detail-block-title">问题描述</div>
              <pre class="detail-text">{{ detailCache[issue.id].description || "(无)" }}</pre>
            </div>
            <div class="detail-block" v-if="detailCache[issue.id].steps_to_reproduce">
              <div class="detail-block-title">重现步骤</div>
              <pre class="detail-text">{{ detailCache[issue.id].steps_to_reproduce }}</pre>
            </div>
            <div class="detail-block" v-if="detailCache[issue.id].additional_information">
              <div class="detail-block-title">补充信息</div>
              <pre class="detail-text">{{ detailCache[issue.id].additional_information }}</pre>
            </div>
            <div class="detail-block" v-if="detailCache[issue.id].notes?.length">
              <div class="detail-block-title">备注（{{ detailCache[issue.id].notes.length }}）</div>
              <div v-for="n in detailCache[issue.id].notes" :key="n.id" class="note-item">
                <div class="note-meta">{{ n.reporter || "?" }} · {{ n.created_at || "?" }}</div>
                <pre class="detail-text">{{ n.text }}</pre>
              </div>
            </div>
          </template>
        </div>
      </article>
    </section>

    <div v-else-if="configSaved && !loadingIssues && allIssues.length === 0" class="empty-hint">
      选好项目后点「拉取 Bug 列表」
    </div>
    <div v-else-if="configSaved && !loadingIssues" class="empty-hint">没有匹配的 bug</div>

    <!-- 浮动操作条：勾选了 bug 就出现 -->
    <div v-if="selectedCount > 0" class="selection-bar">
      <span>已选 {{ selectedCount }} 条</span>
      <button class="link-btn" @click="clearSelection">✕ 清空</button>
      <div class="spacer"></div>
      <button class="fetch-btn" @click="openRiskModal">🧭 生成风险画像 →</button>
    </div>

    <!-- PRD 风险画像弹窗 -->
    <div v-if="showRiskModal && !riskMinimized" class="ai-overlay" @click.self="closeRiskModal">
      <div class="ai-dialog">
        <div class="ai-dialog-head">
          <span class="ai-title">🧭 生成风险画像（prd-risk-profiler）</span>
          <div class="ai-head-btns">
            <button
              v-if="riskPhase === 'running'"
              class="ai-min"
              title="缩小（后台继续跑）"
              @click="riskMinimized = true"
            >—</button>
            <button class="ai-close" :disabled="riskPhase === 'running'" @click="closeRiskModal">✕</button>
          </div>
        </div>

        <template v-if="riskPhase === 'config'">
          <p class="modal-hint">
            已选 {{ selectedCount }} 条 bug 作为本轮沉淀依据；选一份 PRD（Slides，全篇导出，不选页码），
            skill 会更新它自己的知识库（通用层 risk-taxonomy.md + 专属层 apps/&lt;APP名&gt;.md，不存在就新建）。
          </p>
          <div class="form-row">
            <label class="form-label">APP 名称</label>
            <input
              v-model="riskAppName"
              class="text-input"
              list="risk-app-name-suggestions"
              placeholder="如 MP3Cutter（对应知识库文件名）"
            />
            <datalist id="risk-app-name-suggestions">
              <option v-for="n in productNameSuggestions" :key="n" :value="n" />
            </datalist>
          </div>
          <div class="form-row">
            <label class="form-label">版本号</label>
            <input v-model="riskVersion" class="text-input" placeholder="如 2.3.6（可留空）" />
          </div>
          <div class="form-row slides-picker-row">
            <label class="form-label">PRD</label>
            <div class="slides-picker">
              <div v-if="loadingSlides" class="slides-hint">加载 Slides 列表中…</div>
              <div v-else-if="slidesError" class="banner banner-error">{{ slidesError }}</div>
              <div v-else-if="slidesFiles.length === 0" class="slides-hint">没有找到 Slides 文件</div>
              <div v-else class="slides-list">
                <label v-for="f in slidesFiles" :key="f.id" class="slide-item">
                  <input type="radio" name="risk-slide" :value="f.id" v-model="riskSlideId" />
                  {{ f.name }}
                </label>
              </div>
            </div>
          </div>
          <div v-if="riskError" class="banner banner-error">{{ riskError }}</div>
        </template>

        <template v-else-if="riskPhase === 'running'">
          <div v-if="riskPreparing" class="preparing-hint">{{ riskPreparingMsg }}</div>
          <div class="log-panel">
            <div class="log-header">
              <span>Log</span>
              <span class="log-running">运行中…</span>
            </div>
            <div class="log-content">
              <div v-for="(log, i) in riskLogs" :key="i" class="log-line" :class="`log-${log.kind}`">
                {{ log.text }}
              </div>
            </div>
          </div>
        </template>

        <template v-else-if="riskPhase === 'done'">
          <div class="banner banner-success">
            已更新 skill 知识库（通用层 + 「{{ riskAppName }}」专属层）。可在「知识库 → PRD 风险画像」查看或编辑。
          </div>
          <div class="log-panel">
            <div class="log-header"><span>沉淀摘要</span></div>
            <div class="log-content"><pre class="result-text">{{ riskResultText }}</pre></div>
          </div>
        </template>

        <template v-else-if="riskPhase === 'error'">
          <div class="banner banner-error">{{ riskError }}</div>
          <div v-if="riskLogs.length > 0" class="log-panel">
            <div class="log-header"><span>Log</span></div>
            <div class="log-content">
              <div v-for="(log, i) in riskLogs" :key="i" class="log-line" :class="`log-${log.kind}`">
                {{ log.text }}
              </div>
            </div>
          </div>
        </template>

        <div class="ai-dialog-foot">
          <template v-if="riskPhase === 'config'">
            <button class="btn-ghost" @click="closeRiskModal">取消</button>
            <button
              class="fetch-btn"
              :disabled="!riskAppName.trim() || !riskSlideId"
              @click="startRiskGeneration"
            >生成</button>
          </template>
          <template v-else-if="riskPhase === 'done' || riskPhase === 'error'">
            <button class="fetch-btn" @click="showRiskModal = false">关闭</button>
          </template>
        </div>
      </div>
    </div>

    <!-- 缩小后的浮条 -->
    <div v-if="showRiskModal && riskMinimized" class="ai-mini-stack">
      <div class="ai-mini-bar" @click="riskMinimized = false">
        <span>🧭 风险画像生成中…</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.bug-page {
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
  line-height: 1.5;
}

.banner {
  padding: 10px 14px;
  border-radius: 6px;
  font-size: 13px;
  margin-bottom: 12px;
  line-height: 1.5;
}
.banner-error {
  background: #fff5f5;
  border: 1px solid #fed7d7;
  color: #c53030;
  word-break: break-all;
}

.config-card,
.picker-card {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  background: white;
  margin-bottom: 12px;
}
.config-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  cursor: pointer;
  user-select: none;
}
.config-title {
  font-size: 13px;
  font-weight: 600;
  color: #4a5568;
}
.config-status {
  font-size: 12px;
  color: #38a169;
}
.chevron {
  margin-left: auto;
  transition: transform 0.15s;
  color: #999;
}
.chevron.open {
  transform: rotate(180deg);
}
.config-body {
  padding: 0 14px 14px 14px;
}
.config-msg {
  font-size: 12px;
  color: #38a169;
  margin-top: 6px;
}
.config-msg.error {
  color: #c53030;
}
.effective-url-row {
  margin-top: -4px;
}
.effective-url {
  font-size: 11px;
  color: #a0aec0;
  word-break: break-all;
}

.picker-card {
  padding: 12px 14px;
}
.form-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.form-row:last-child {
  margin-bottom: 0;
}
.form-label {
  width: 56px;
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
}

.text-input {
  flex: 1;
  padding: 6px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
}
.text-input:focus {
  border-color: #667eea;
}
.src-select {
  flex: 1;
  padding: 6px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  outline: none;
  cursor: pointer;
}
.src-select:focus {
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
.icon-btn:hover {
  background: #f5f5fa;
  color: #333;
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
  white-space: nowrap;
}
.fetch-btn:hover {
  background: #5a67d8;
}
.fetch-btn:disabled {
  background: #cbd5e0;
  cursor: not-allowed;
}

.chip-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 4px;
}
.chip {
  font-size: 12px;
  padding: 3px 10px;
  border-radius: 12px;
  background: #edf2f7;
  color: #4a5568;
  cursor: pointer;
  user-select: none;
}
.chip:hover {
  background: #e2e8f0;
}
.chip.active {
  background: #667eea;
  color: white;
}
.cache-hint {
  font-size: 11px;
  color: #a0aec0;
  margin-top: 6px;
}

.issue-count {
  font-size: 12px;
  color: #888;
  margin-bottom: 6px;
}
.issue-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.issue-item {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  background: white;
  overflow: hidden;
}
.issue-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  cursor: pointer;
}
.issue-row:hover {
  background: #f7f8ff;
}
.issue-id {
  font-size: 12px;
  color: #667eea;
  font-weight: 600;
  flex-shrink: 0;
}
.issue-summary {
  flex: 1;
  font-size: 13px;
  color: #2d3748;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.issue-tag {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 10px;
  background: #edf2f7;
  color: #4a5568;
  flex-shrink: 0;
}

.issue-detail {
  padding: 12px 16px 16px 16px;
  border-top: 1px solid #f0f0f0;
  background: #fafafa;
}
.detail-loading {
  font-size: 12px;
  color: #888;
}
.detail-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 16px;
  font-size: 12px;
  color: #718096;
  margin-bottom: 10px;
}
.detail-block {
  margin-bottom: 10px;
}
.detail-block-title {
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
  margin-bottom: 4px;
}
.detail-text {
  font-family: inherit;
  font-size: 13px;
  color: #2d3748;
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
  line-height: 1.6;
}
.note-item {
  border-left: 2px solid #e2e8f0;
  padding-left: 8px;
  margin-bottom: 8px;
}
.note-meta {
  font-size: 11px;
  color: #a0aec0;
  margin-bottom: 2px;
}

.empty-hint {
  font-size: 13px;
  color: #a0aec0;
  text-align: center;
  padding: 40px 0;
}

/* 多选 + PRD 风险画像 */
.select-all-row {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 8px;
  font-size: 12px;
  color: #4a5568;
  cursor: pointer;
  user-select: none;
}
.issue-checkbox {
  flex-shrink: 0;
  cursor: pointer;
}

.selection-bar {
  position: sticky;
  bottom: 0;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  margin-top: 10px;
  background: white;
  border: 1px solid #e2e8f0;
  border-radius: 10px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.08);
  font-size: 13px;
  color: #4a5568;
}
.link-btn {
  background: none;
  border: none;
  color: #a0aec0;
  font-size: 12px;
  cursor: pointer;
  padding: 0;
}
.link-btn:hover {
  color: #667eea;
}

.banner-success {
  background: #f0fff4;
  border: 1px solid #c6f6d5;
  color: #276749;
}

/* 弹窗（照抄 ReviewPage 的 ai-overlay 模式） */
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
  max-width: 560px;
  max-height: 88vh;
  overflow-y: auto;
  padding: 18px 20px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.25);
  display: flex;
  flex-direction: column;
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
  font-size: 13px;
  color: #4a5568;
}

.modal-hint {
  font-size: 12px;
  color: #718096;
  line-height: 1.6;
  margin: 0 0 12px;
}

.slides-picker-row {
  align-items: flex-start;
}
.slides-picker {
  flex: 1;
}
.slides-hint {
  font-size: 12px;
  color: #a0aec0;
  padding: 6px 0;
}
.slides-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 160px;
  overflow-y: auto;
  border: 1px solid #e5e5e5;
  border-radius: 6px;
  padding: 6px 10px;
}
.slide-item {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: #2d3748;
  cursor: pointer;
  padding: 3px 0;
}

.preparing-hint {
  font-size: 12px;
  color: #667eea;
  margin-bottom: 8px;
}

.log-panel {
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  overflow: hidden;
  margin-top: 4px;
}
.log-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  background: #f7fafc;
  border-bottom: 1px solid #e2e8f0;
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
}
.log-running {
  color: #667eea;
  font-weight: 500;
}
.log-content {
  max-height: 320px;
  overflow-y: auto;
  padding: 8px 12px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
  line-height: 1.6;
}
.log-line {
  white-space: pre-wrap;
  word-break: break-word;
  color: #4a5568;
}
.log-info {
  color: #a0aec0;
}
.log-tool {
  color: #805ad5;
}
.log-error {
  color: #c53030;
}
.result-text {
  white-space: pre-wrap;
  word-break: break-word;
  font-family: inherit;
  font-size: 13px;
  color: #2d3748;
  margin: 0;
}

.ai-dialog-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 14px;
}
.btn-ghost {
  padding: 6px 16px;
  font-size: 13px;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.btn-ghost:hover {
  background: #f7fafc;
}
</style>

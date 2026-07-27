<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { marked } from "marked";

// 补充测试点：只给一份新 PRD（不需要 bug 报告），调用 prd-risk-profiler skill 的
// 「复用模式」——读该 APP 已有知识库产出《风险分析报告》+《补充测试点》清单。
// 后端只管两份文件的落盘/存取（prd_supplement.rs），补充测试点固定的复选框
// Markdown 格式在这里解析成结构化列表渲染（可勾选/改文字/存盘覆盖），风险分析
// 报告只读展示（marked 渲染），两者按同一次生成关联在一起——「查看风险报告」
// 就是同一个生成记录目录下的另一份文件，不做逐条测试点↔分析行的精确映射。
//
// 管理层级：App → 版本号 → 生成记录（同一 App+版本允许多次生成，各自独立留存，
// 不覆盖旧的，用户手动删除旧记录）。

interface DriveFile {
  id: string;
  name: string;
}
interface SupplementGeneration {
  id: string;
  created_at: string;
  checked: number;
  total: number;
}
interface SupplementItem {
  text: string;
  checked: boolean;
}
interface SupplementModule {
  name: string;
  items: SupplementItem[];
}
interface ParsedSupplement {
  title: string;
  intro: string;
  modules: SupplementModule[];
}

// ── 解析/序列化：补充测试点.md 是固定格式（# 标题 / > 说明 / **模块** / - [ ] 条目） ──
function parseSupplementMd(md: string): ParsedSupplement {
  const lines = md.split(/\r?\n/);
  let title = "";
  const introLines: string[] = [];
  const modules: SupplementModule[] = [];
  let current: SupplementModule | null = null;

  for (const raw of lines) {
    const line = raw.trim();
    if (!line) continue;
    if (!title && line.startsWith("# ")) {
      title = line.slice(2).trim();
      continue;
    }
    if (line.startsWith(">")) {
      introLines.push(line.replace(/^>\s?/, ""));
      continue;
    }
    const moduleMatch = line.match(/^\*\*(.+)\*\*$/);
    if (moduleMatch) {
      current = { name: moduleMatch[1].trim(), items: [] };
      modules.push(current);
      continue;
    }
    const itemMatch = line.match(/^-\s*\[([ xX])\]\s*(.*)$/);
    if (itemMatch) {
      if (!current) {
        current = { name: "未分组", items: [] };
        modules.push(current);
      }
      current.items.push({
        text: itemMatch[2].trim(),
        checked: itemMatch[1].toLowerCase() === "x",
      });
    }
  }
  return { title, intro: introLines.join("\n"), modules };
}

function serializeSupplementMd(doc: ParsedSupplement): string {
  const parts: string[] = [];
  if (doc.title) parts.push(`# ${doc.title}`);
  if (doc.intro.trim()) {
    parts.push(
      doc.intro
        .split("\n")
        .map((l) => (l.trim() ? `> ${l}` : ">"))
        .join("\n")
    );
  }
  for (const mod of doc.modules) {
    const body = [`**${mod.name}**`, ...mod.items.map((it) => `- [${it.checked ? "x" : " "}] ${it.text}`)];
    parts.push(body.join("\n"));
  }
  return parts.join("\n\n") + "\n";
}

// ── 生成表单 ──────────────────────────────────────────────────────────────
const genExpanded = ref(true);
const appName = ref("");
const version = ref("");
const productNameSuggestions = ref<string[]>([]);
const slidesFiles = ref<DriveFile[]>([]);
const loadingSlides = ref(false);
const slidesError = ref("");
const selectedSlideId = ref<string | null>(null);

type GenPhase = "idle" | "running" | "error";
const genPhase = ref<GenPhase>("idle");
const genError = ref("");
const genPreparingMsg = ref("");
const genLogs = ref<{ text: string; kind: string }[]>([]);
let unlistenSupplementLog: (() => void) | null = null;

async function loadProductNameSuggestions() {
  try {
    const products = await invoke<Array<{ name: string }>>("list_template_products");
    productNameSuggestions.value = products.map((p) => p.name).filter((n) => n !== "通用");
  } catch {
    productNameSuggestions.value = [];
  }
}

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

function pushGenLog(text: string, kind = "text") {
  genLogs.value.push({ text, kind });
}

async function startGenerate() {
  const name = appName.value.trim();
  const ver = version.value.trim();
  if (!name || !ver) {
    genError.value = "请填写 APP 名称和版本号";
    return;
  }
  const slide = slidesFiles.value.find((f) => f.id === selectedSlideId.value);
  if (!slide) {
    genError.value = "请先选一份 PRD（Slides）";
    return;
  }

  genError.value = "";
  genLogs.value = [];
  genPhase.value = "running";

  if (!unlistenSupplementLog) {
    unlistenSupplementLog = await listen<{ text: string; kind: string; done: boolean }>(
      "prd-supplement-log",
      (event) => {
        const { text, kind } = event.payload;
        if (text && text.trim()) pushGenLog(text, kind);
      }
    );
  }

  try {
    genPreparingMsg.value = "正在导出 PRD 页面为图片…";
    const imagePaths = await invoke<string[]>("export_slides_pdf", {
      presentationId: slide.id,
      name: slide.name,
      pages: [],
    });
    genPreparingMsg.value = "";

    const generationId = await invoke<string>("run_prd_supplement_reuse", {
      appName: name,
      version: ver,
      prdImagePaths: imagePaths,
      model: null,
    });

    genPhase.value = "idle";
    genExpanded.value = false;
    await refreshApps();
    selectedApp.value = name;
    await refreshVersions();
    selectedVersion.value = ver;
    await refreshGenerations();
    selectedGeneration.value = generationId;
    await openGeneration();
  } catch (e: any) {
    genPreparingMsg.value = "";
    genPhase.value = "error";
    genError.value = String(e);
  }
}

async function stopGenerate() {
  try {
    await invoke("stop_prd_supplement_reuse");
  } catch {}
}

// ── 管理层级：App → 版本 → 生成记录 ────────────────────────────────────────
const apps = ref<string[]>([]);
const selectedApp = ref<string | null>(null);
const versions = ref<string[]>([]);
const selectedVersion = ref<string | null>(null);
const generations = ref<SupplementGeneration[]>([]);
const selectedGeneration = ref<string | null>(null);
const listError = ref("");

async function refreshApps() {
  try {
    apps.value = await invoke<string[]>("list_prd_supplement_apps");
  } catch (e: any) {
    listError.value = String(e);
    apps.value = [];
  }
}

async function refreshVersions() {
  versions.value = [];
  if (!selectedApp.value) return;
  try {
    versions.value = await invoke<string[]>("list_prd_supplement_versions", { appName: selectedApp.value });
  } catch (e: any) {
    listError.value = String(e);
  }
}

async function refreshGenerations() {
  generations.value = [];
  if (!selectedApp.value || !selectedVersion.value) return;
  try {
    generations.value = await invoke<SupplementGeneration[]>("list_prd_supplement_generations", {
      appName: selectedApp.value,
      version: selectedVersion.value,
    });
  } catch (e: any) {
    listError.value = String(e);
  }
}

function selectApp(name: string) {
  if (selectedApp.value === name) return;
  selectedApp.value = name;
  selectedVersion.value = null;
  selectedGeneration.value = null;
  parsedDoc.value = null;
  refreshVersions().then(() => {
    if (versions.value.length > 0) {
      selectedVersion.value = versions.value[0];
      refreshGenerations().then(() => {
        if (generations.value.length > 0) {
          selectedGeneration.value = generations.value[0].id;
          openGeneration();
        }
      });
    }
  });
}

function selectVersion(v: string) {
  if (selectedVersion.value === v) return;
  selectedVersion.value = v;
  selectedGeneration.value = null;
  parsedDoc.value = null;
  refreshGenerations().then(() => {
    if (generations.value.length > 0) {
      selectedGeneration.value = generations.value[0].id;
      openGeneration();
    }
  });
}

// ── 详情：解析后的补充测试点 + 只读风险报告 ─────────────────────────────────
const parsedDoc = ref<ParsedSupplement | null>(null);
const riskReportMd = ref("");
const riskReportHtml = computed(() => (riskReportMd.value ? marked.parse(riskReportMd.value) : ""));
const showRiskReport = ref(false);
const loadingDoc = ref(false);
const docError = ref("");
const saveMsg = ref("");
const dirty = ref(false);
const deleteConfirming = ref(false);
const newModuleName = ref("");
const addingModule = ref(false);
const newItemDrafts = ref<Record<number, string>>({});

const progress = computed(() => {
  if (!parsedDoc.value) return { checked: 0, total: 0 };
  let checked = 0;
  let total = 0;
  for (const m of parsedDoc.value.modules) {
    total += m.items.length;
    checked += m.items.filter((i) => i.checked).length;
  }
  return { checked, total };
});

async function openGeneration() {
  if (!selectedApp.value || !selectedVersion.value || !selectedGeneration.value) return;
  loadingDoc.value = true;
  docError.value = "";
  saveMsg.value = "";
  showRiskReport.value = false;
  try {
    const doc = await invoke<{ supplement_md: string; risk_report_md: string }>(
      "read_prd_supplement_generation",
      {
        appName: selectedApp.value,
        version: selectedVersion.value,
        generation: selectedGeneration.value,
      }
    );
    parsedDoc.value = parseSupplementMd(doc.supplement_md);
    riskReportMd.value = doc.risk_report_md;
    dirty.value = false;
  } catch (e: any) {
    docError.value = String(e);
    parsedDoc.value = null;
  } finally {
    loadingDoc.value = false;
  }
}

function markDirty() {
  dirty.value = true;
  saveMsg.value = "";
}

function toggleItem(item: SupplementItem) {
  item.checked = !item.checked;
  markDirty();
}

function deleteItem(mod: SupplementModule, idx: number) {
  mod.items.splice(idx, 1);
  markDirty();
}

function addItem(mod: SupplementModule, modIdx: number) {
  const text = (newItemDrafts.value[modIdx] || "").trim();
  if (!text) return;
  mod.items.push({ text, checked: false });
  newItemDrafts.value[modIdx] = "";
  markDirty();
}

function deleteModule(idx: number) {
  if (!parsedDoc.value) return;
  parsedDoc.value.modules.splice(idx, 1);
  markDirty();
}

function confirmAddModule() {
  const name = newModuleName.value.trim();
  addingModule.value = false;
  if (!name || !parsedDoc.value) return;
  parsedDoc.value.modules.push({ name, items: [] });
  newModuleName.value = "";
  markDirty();
}

async function saveDoc() {
  if (!parsedDoc.value || !selectedApp.value || !selectedVersion.value || !selectedGeneration.value) return;
  saveMsg.value = "";
  try {
    const md = serializeSupplementMd(parsedDoc.value);
    await invoke("save_prd_supplement_generation", {
      appName: selectedApp.value,
      version: selectedVersion.value,
      generation: selectedGeneration.value,
      supplementMd: md,
    });
    dirty.value = false;
    saveMsg.value = "已保存";
    // 保存后刷新列表里的进度数字（勾选计数）
    refreshGenerations();
  } catch (e: any) {
    saveMsg.value = "保存失败：" + String(e);
  }
}

async function deleteGeneration() {
  if (!selectedApp.value || !selectedVersion.value || !selectedGeneration.value) return;
  try {
    await invoke("delete_prd_supplement_generation", {
      appName: selectedApp.value,
      version: selectedVersion.value,
      generation: selectedGeneration.value,
    });
    deleteConfirming.value = false;
    selectedGeneration.value = null;
    parsedDoc.value = null;
    await refreshGenerations();
    if (generations.value.length > 0) {
      selectedGeneration.value = generations.value[0].id;
      await openGeneration();
    } else {
      // 该版本下已没有生成记录了，顺手看看这个版本/APP是否也该从列表消失
      await refreshVersions();
      if (versions.value.length === 0) {
        selectedVersion.value = null;
        await refreshApps();
      }
    }
  } catch (e: any) {
    docError.value = "删除失败：" + String(e);
  }
}

watch(selectedGeneration, (id) => {
  if (id) openGeneration();
});

onMounted(async () => {
  await Promise.all([refreshApps(), loadProductNameSuggestions(), loadSlidesFiles()]);
  if (apps.value.length > 0) {
    selectedApp.value = apps.value[0];
    genExpanded.value = false;
    await refreshVersions();
    if (versions.value.length > 0) {
      selectedVersion.value = versions.value[0];
      await refreshGenerations();
      if (generations.value.length > 0) {
        selectedGeneration.value = generations.value[0].id;
        await openGeneration();
      }
    }
  }
});

onUnmounted(() => {
  unlistenSupplementLog?.();
});
</script>

<template>
  <div class="supplement-page">
    <header class="page-header">
      <h3>补充测试点</h3>
      <p class="subtitle">
        只给一份新 PRD（不需要 bug 报告），调用 prd-risk-profiler skill 的「复用模式」——读取该 APP
        已有知识库，产出《风险分析报告》+《补充测试点》清单。测试点可勾选/改文字，保存直接覆盖。
      </p>
    </header>

    <!-- 生成新的补充测试点 -->
    <section class="gen-card">
      <div class="gen-head" @click="genExpanded = !genExpanded">
        <span class="gen-title">生成新的补充测试点</span>
        <span class="chevron" :class="{ open: genExpanded }">▾</span>
      </div>
      <div v-show="genExpanded" class="gen-body">
        <div class="form-row">
          <label class="form-label">APP 名称</label>
          <input
            v-model="appName"
            class="text-input"
            list="supplement-app-name-suggestions"
            placeholder="如 MP3Cutter（对应知识库文件名）"
            :disabled="genPhase === 'running'"
          />
          <datalist id="supplement-app-name-suggestions">
            <option v-for="n in productNameSuggestions" :key="n" :value="n" />
          </datalist>
        </div>
        <div class="form-row">
          <label class="form-label">版本号</label>
          <input v-model="version" class="text-input" placeholder="如 2.3.6" :disabled="genPhase === 'running'" />
        </div>
        <div class="form-row slides-picker-row">
          <label class="form-label">PRD</label>
          <div class="slides-picker">
            <div v-if="loadingSlides" class="slides-hint">加载 Slides 列表中…</div>
            <div v-else-if="slidesError" class="banner banner-error">{{ slidesError }}</div>
            <div v-else-if="slidesFiles.length === 0" class="slides-hint">
              没有找到 Slides 文件
              <button class="link-btn" @click="loadSlidesFiles">⟳ 刷新</button>
            </div>
            <div v-else class="slides-list">
              <label v-for="f in slidesFiles" :key="f.id" class="slide-item">
                <input
                  type="radio"
                  name="supplement-slide"
                  :value="f.id"
                  v-model="selectedSlideId"
                  :disabled="genPhase === 'running'"
                />
                {{ f.name }}
              </label>
            </div>
          </div>
        </div>

        <div v-if="genPhase === 'running'" class="running-block">
          <div v-if="genPreparingMsg" class="preparing-hint">{{ genPreparingMsg }}</div>
          <div class="log-panel">
            <div class="log-header">
              <span>Log</span>
              <span class="log-running">运行中…</span>
              <button class="link-btn stop-btn" @click="stopGenerate">停止</button>
            </div>
            <div class="log-content">
              <div v-for="(log, i) in genLogs" :key="i" class="log-line" :class="`log-${log.kind}`">
                {{ log.text }}
              </div>
            </div>
          </div>
        </div>

        <div v-if="genError" class="banner banner-error">{{ genError }}</div>

        <div class="form-foot">
          <button
            class="fetch-btn"
            :disabled="genPhase === 'running' || !appName.trim() || !version.trim() || !selectedSlideId"
            @click="startGenerate"
          >
            {{ genPhase === "running" ? "生成中…" : "生成" }}
          </button>
        </div>
      </div>
    </section>

    <div v-if="listError" class="banner banner-error">{{ listError }}</div>

    <!-- 管理层级：App → 版本 → 生成记录 -->
    <section v-if="apps.length > 0" class="manage-area">
      <div class="tab-row app-tabs">
        <span
          v-for="a in apps"
          :key="a"
          class="tab-chip"
          :class="{ active: selectedApp === a }"
          @click="selectApp(a)"
        >
          {{ a }}
        </span>
      </div>

      <div v-if="selectedApp" class="tab-row version-tabs">
        <span
          v-for="v in versions"
          :key="v"
          class="tab-chip version-chip"
          :class="{ active: selectedVersion === v }"
          @click="selectVersion(v)"
        >
          v{{ v }}
        </span>
        <span v-if="versions.length === 0" class="empty-hint-inline">该 App 下没有版本记录</span>
      </div>

      <div v-if="selectedVersion" class="generation-list">
        <div
          v-for="g in generations"
          :key="g.id"
          class="generation-item"
          :class="{ active: selectedGeneration === g.id }"
          @click="selectedGeneration = g.id"
        >
          <span class="gen-time">{{ g.created_at }}</span>
          <span class="gen-progress">已确认 {{ g.checked }}/{{ g.total }}</span>
        </div>
      </div>

      <!-- 详情：补充测试点清单 -->
      <div v-if="loadingDoc" class="empty-hint">加载中…</div>
      <div v-else-if="docError" class="banner banner-error">{{ docError }}</div>
      <div v-else-if="parsedDoc" class="detail-card">
        <div class="detail-toolbar">
          <span class="detail-title">{{ parsedDoc.title || "补充测试点" }}</span>
          <span class="detail-progress">{{ progress.checked }}/{{ progress.total }}</span>
          <div class="spacer"></div>
          <button class="link-btn" @click="showRiskReport = !showRiskReport">
            {{ showRiskReport ? "收起风险报告" : "🧭 查看风险报告" }}
          </button>
          <button class="fetch-btn save-btn" :disabled="!dirty" @click="saveDoc">保存</button>
          <template v-if="!deleteConfirming">
            <button class="link-btn danger-link" @click="deleteConfirming = true">删除</button>
          </template>
          <template v-else>
            <span class="confirm-inline">
              确认删除这份记录？
              <button class="link-btn danger-link" @click="deleteGeneration">是</button>
              <button class="link-btn" @click="deleteConfirming = false">否</button>
            </span>
          </template>
        </div>
        <div v-if="saveMsg" class="save-msg" :class="{ error: saveMsg.startsWith('保存失败') }">{{ saveMsg }}</div>
        <div v-if="parsedDoc.intro" class="detail-intro">{{ parsedDoc.intro }}</div>

        <div v-if="showRiskReport" class="risk-report-panel">
          <div class="risk-report-body markdown-body" v-html="riskReportHtml"></div>
        </div>

        <div class="module-list">
          <div v-for="(mod, mi) in parsedDoc.modules" :key="mi" class="module-block">
            <div class="module-head">
              <span class="module-name">{{ mod.name }}</span>
              <button class="icon-btn del-mod-btn" title="删除整个模块" @click="deleteModule(mi)">✕</button>
            </div>
            <div v-for="(item, ii) in mod.items" :key="ii" class="item-row">
              <input type="checkbox" :checked="item.checked" @change="toggleItem(item)" />
              <input
                v-model="item.text"
                class="item-text-input"
                :class="{ checked: item.checked }"
                @input="markDirty"
              />
              <button class="icon-btn del-item-btn" title="删除这条" @click="deleteItem(mod, ii)">✕</button>
            </div>
            <div class="item-row add-item-row">
              <input
                v-model="newItemDrafts[mi]"
                class="item-text-input add-item-input"
                placeholder="+ 新增测试点，回车添加"
                @keydown.enter="addItem(mod, mi)"
              />
            </div>
          </div>
        </div>

        <div class="add-module-row">
          <template v-if="addingModule">
            <input
              v-model="newModuleName"
              class="text-input module-name-input"
              placeholder="模块名称"
              autofocus
              @keydown.enter="confirmAddModule"
              @keydown.escape="addingModule = false"
              @blur="confirmAddModule"
            />
          </template>
          <button v-else class="link-btn" @click="addingModule = true">+ 新增模块</button>
        </div>
      </div>
      <div v-else-if="selectedVersion" class="empty-hint">选一条生成记录查看</div>
    </section>

    <div v-else-if="genPhase !== 'running'" class="empty-hint">
      还没有生成任何补充测试点，先在上方「生成新的补充测试点」里填好信息生成一份
    </div>
  </div>
</template>

<style scoped>
.supplement-page {
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

.gen-card {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  background: white;
  margin-bottom: 16px;
}
.gen-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  cursor: pointer;
  user-select: none;
}
.gen-title {
  font-size: 13px;
  font-weight: 600;
  color: #4a5568;
}
.chevron {
  margin-left: auto;
  transition: transform 0.15s;
  color: #999;
}
.chevron.open {
  transform: rotate(180deg);
}
.gen-body {
  padding: 0 14px 14px 14px;
}

.form-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.form-label {
  width: 64px;
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
.text-input:disabled {
  background: #f5f5f5;
  color: #999;
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
  display: flex;
  align-items: center;
  gap: 8px;
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

.form-foot {
  display: flex;
  justify-content: flex-end;
  margin-top: 8px;
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
.link-btn {
  background: none;
  border: none;
  color: #667eea;
  font-size: 12px;
  cursor: pointer;
  padding: 0;
}
.link-btn:hover {
  text-decoration: underline;
}
.danger-link {
  color: #c53030;
}

.running-block {
  margin: 8px 0;
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
.stop-btn {
  margin-left: auto;
}
.log-content {
  max-height: 280px;
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

.manage-area {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.tab-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.tab-chip {
  font-size: 12px;
  padding: 5px 12px;
  border-radius: 14px;
  background: #edf2f7;
  color: #4a5568;
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
}
.tab-chip:hover {
  background: #e2e8f0;
}
.tab-chip.active {
  background: #667eea;
  color: white;
}
.version-chip {
  background: #f7fafc;
  border: 1px solid #e2e8f0;
}
.version-chip.active {
  background: #5a67d8;
  border-color: #5a67d8;
  color: white;
}
.empty-hint-inline {
  font-size: 12px;
  color: #a0aec0;
}

.generation-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-bottom: 4px;
}
.generation-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 12px;
  border: 1px solid #e5e5e5;
  border-radius: 6px;
  background: white;
  cursor: pointer;
  font-size: 12px;
  color: #4a5568;
}
.generation-item:hover {
  background: #f7f8ff;
}
.generation-item.active {
  border-color: #667eea;
  background: #f0f3ff;
}
.gen-time {
  font-weight: 500;
}
.gen-progress {
  color: #a0aec0;
  margin-left: auto;
}

.detail-card {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  background: white;
  padding: 14px 16px;
}
.detail-toolbar {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 8px;
}
.detail-title {
  font-size: 14px;
  font-weight: 600;
  color: #2d3748;
}
.detail-progress {
  font-size: 12px;
  color: #667eea;
  background: #eef1ff;
  padding: 2px 8px;
  border-radius: 10px;
}
.spacer {
  flex: 1;
}
.save-btn {
  padding: 5px 14px;
}
.save-msg {
  font-size: 12px;
  color: #38a169;
  margin-bottom: 8px;
}
.save-msg.error {
  color: #c53030;
}
.confirm-inline {
  font-size: 12px;
  color: #4a5568;
  display: flex;
  align-items: center;
  gap: 6px;
}
.detail-intro {
  font-size: 12px;
  color: #718096;
  line-height: 1.6;
  margin-bottom: 12px;
  white-space: pre-wrap;
}

.risk-report-panel {
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  background: #fafbff;
  padding: 12px 16px;
  margin-bottom: 12px;
  max-height: 420px;
  overflow-y: auto;
}
.markdown-body {
  font-size: 13px;
  line-height: 1.7;
  color: #2d3748;
}
.markdown-body :deep(table) {
  border-collapse: collapse;
  width: 100%;
  margin: 8px 0;
  font-size: 12px;
}
.markdown-body :deep(th),
.markdown-body :deep(td) {
  border: 1px solid #e2e8f0;
  padding: 4px 8px;
  text-align: left;
}
.markdown-body :deep(h1),
.markdown-body :deep(h2),
.markdown-body :deep(h3) {
  margin: 12px 0 6px;
}
.markdown-body :deep(code) {
  background: #f0f0f5;
  padding: 1px 4px;
  border-radius: 3px;
}

.module-list {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.module-block {
  border-top: 1px solid #f0f0f0;
  padding-top: 10px;
}
.module-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 6px;
}
.module-name {
  font-size: 13px;
  font-weight: 600;
  color: #4a5568;
}
.icon-btn {
  border: none;
  background: none;
  color: #cbd5e0;
  cursor: pointer;
  font-size: 11px;
  padding: 2px 4px;
  flex-shrink: 0;
}
.icon-btn:hover {
  color: #c53030;
}
.del-mod-btn {
  margin-left: auto;
}
.item-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 0;
}
.item-text-input {
  flex: 1;
  border: none;
  background: transparent;
  font-size: 13px;
  color: #2d3748;
  padding: 3px 4px;
  border-radius: 4px;
  outline: none;
}
.item-text-input:hover,
.item-text-input:focus {
  background: #f7fafc;
}
.item-text-input.checked {
  color: #a0aec0;
  text-decoration: line-through;
}
.add-item-input {
  color: #a0aec0;
  font-size: 12px;
}
.add-item-row {
  padding-left: 24px;
}

.add-module-row {
  margin-top: 14px;
  padding-top: 10px;
  border-top: 1px solid #f0f0f0;
}
.module-name-input {
  max-width: 240px;
}

.empty-hint {
  font-size: 13px;
  color: #a0aec0;
  text-align: center;
  padding: 40px 0;
}
</style>

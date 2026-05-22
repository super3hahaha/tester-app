<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface DriveFile {
  id: string;
  name: string;
  modified_time: string;
  mime_type: string;
}

interface SidePick {
  file: DriveFile | null;
  tabs: string[];
  tab: string;
  loadingTabs: boolean;
  error: string;
  pasteUrl: string;
}

function newSide(): SidePick {
  return {
    file: null,
    tabs: [],
    tab: "",
    loadingTabs: false,
    error: "",
    pasteUrl: "",
  };
}

const ai = ref<SidePick>(newSide());
const human = ref<SidePick>(newSide());

const files = ref<DriveFile[]>([]);
const loadingFiles = ref(false);
const filesError = ref("");

interface LogEvent {
  text: string;
  kind: string;
  timestamp: string;
}

const logs = ref<LogEvent[]>([]);
const running = ref(false);
const progress = ref("");
const reportPath = ref<string | null>(null);
const errorMsg = ref("");
const logPanel = ref<HTMLElement | null>(null);

let unlisten: UnlistenFn | null = null;
const onDriveFilesUpdated = () => {
  loadFiles();
};

function nowHms() {
  const d = new Date();
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

function pushLog(text: string, kind = "text") {
  logs.value.push({ text, kind, timestamp: nowHms() });
  nextTick(() => {
    if (logPanel.value) {
      logPanel.value.scrollTop = logPanel.value.scrollHeight;
    }
  });
}

onMounted(async () => {
  loadFiles();
  window.addEventListener("drive-files-updated", onDriveFilesUpdated);
  unlisten = await listen<{ text: string; kind: string; done: boolean }>(
    "compare-log",
    (event) => {
      const { text, kind, done } = event.payload;
      pushLog(text, kind);
      if (done) {
        running.value = false;
      }
    }
  );
});

onUnmounted(() => {
  unlisten?.();
  window.removeEventListener("drive-files-updated", onDriveFilesUpdated);
});

async function loadFiles() {
  loadingFiles.value = true;
  filesError.value = "";
  try {
    files.value = await invoke<DriveFile[]>("list_drive_files", {
      mimeType: "application/vnd.google-apps.spreadsheet",
    });
  } catch (e: any) {
    filesError.value = String(e);
  } finally {
    loadingFiles.value = false;
  }
}

async function pickFile(side: SidePick, file: DriveFile) {
  side.file = file;
  side.tabs = [];
  side.tab = "";
  side.error = "";
  side.loadingTabs = true;
  try {
    side.tabs = await invoke<string[]>("get_sheet_tabs", {
      spreadsheetId: file.id,
    });
    if (side.tabs.length > 0) {
      side.tab = side.tabs[0];
    }
  } catch (e: any) {
    side.error = String(e);
  } finally {
    side.loadingTabs = false;
  }
}

async function handlePasteUrl(side: SidePick) {
  const match = side.pasteUrl.match(/\/spreadsheets\/d\/([a-zA-Z0-9_-]+)/);
  if (!match) {
    side.error = "Invalid Google Sheets URL";
    return;
  }
  const id = match[1];
  await pickFile(side, { id, name: "Pasted Sheet", modified_time: "", mime_type: "" });
}

const canCompare = computed(
  () =>
    !running.value &&
    ai.value.file &&
    ai.value.tab &&
    human.value.file &&
    human.value.tab &&
    !(ai.value.file.id === human.value.file.id && ai.value.tab === human.value.tab)
);

async function handleCompare() {
  if (!canCompare.value) return;
  running.value = true;
  logs.value = [];
  reportPath.value = null;
  errorMsg.value = "";

  try {
    progress.value = "Exporting AI version as HTML...";
    pushLog(`[1/3] Exporting AI version: ${ai.value.file!.name} > ${ai.value.tab}`, "info");
    const aiPath = await invoke<string>("export_sheet_html", {
      spreadsheetId: ai.value.file!.id,
      tabName: ai.value.tab,
      role: "ai",
    });
    pushLog(`  -> ${aiPath}`);

    progress.value = "Exporting Human version as HTML...";
    pushLog(`[2/3] Exporting Human version: ${human.value.file!.name} > ${human.value.tab}`, "info");
    const humanPath = await invoke<string>("export_sheet_html", {
      spreadsheetId: human.value.file!.id,
      tabName: human.value.tab,
      role: "human",
    });
    pushLog(`  -> ${humanPath}`);

    progress.value = "Running diff skill via Claude CLI...";
    pushLog("[3/3] Running testcase-eval-visual-report skill", "info");
    const path = await invoke<string>("run_diff_skill", {
      aiHtmlPath: aiPath,
      humanHtmlPath: humanPath,
    });
    reportPath.value = path;
    progress.value = "Done";
  } catch (e: any) {
    errorMsg.value = String(e);
    progress.value = "";
    running.value = false;
  }
}

async function handleOpenInChrome() {
  if (!reportPath.value) return;
  try {
    await invoke("open_in_chrome", { path: reportPath.value });
  } catch (e: any) {
    errorMsg.value = String(e);
  }
}

function formatDate(iso: string) {
  if (!iso) return "";
  const d = new Date(iso);
  return `${d.getMonth() + 1}/${d.getDate()} ${d.getHours()}:${String(d.getMinutes()).padStart(2, "0")}`;
}
</script>

<template>
  <div class="compare-page">
    <header class="page-header">
      <div class="header-row">
        <h3>compare</h3>
        <button
          class="refresh-btn"
          @click="loadFiles"
          :disabled="loadingFiles"
          title="刷新文件列表"
        >&#8635;</button>
      </div>
      <p class="subtitle">
        选择两个表格的 Sheet（AI 原始版本 vs 人工修改最终版本），点击对比生成可视化 diff 报告。
      </p>
    </header>

    <div class="picker-row">
      <!-- AI side -->
      <section class="picker-card ai">
        <div class="card-header">
          <span class="role-badge ai-badge">AI 原始版本</span>
          <span v-if="ai.file && ai.tab" class="picked">{{ ai.file.name }} › {{ ai.tab }}</span>
        </div>
        <div class="paste-section">
          <input
            v-model="ai.pasteUrl"
            placeholder="粘贴 Sheet URL..."
            @keyup.enter="handlePasteUrl(ai)"
          />
          <button @click="handlePasteUrl(ai)" :disabled="!ai.pasteUrl">Go</button>
        </div>
        <div class="file-list-wrap">
          <div v-if="loadingFiles" class="hint">Loading...</div>
          <div v-else class="file-list">
            <div
              v-for="f in files"
              :key="f.id"
              class="file-item"
              :class="{ active: ai.file?.id === f.id }"
              @click="pickFile(ai, f)"
            >
              <span class="file-name">{{ f.name }}</span>
              <span class="file-date">{{ formatDate(f.modified_time) }}</span>
            </div>
          </div>
        </div>
        <div v-if="ai.file" class="tabs-area">
          <div class="tabs-label">Tab:</div>
          <div v-if="ai.loadingTabs" class="hint">Loading tabs...</div>
          <div v-else class="tab-list">
            <button
              v-for="t in ai.tabs"
              :key="t"
              class="tab-btn"
              :class="{ active: ai.tab === t }"
              @click="ai.tab = t"
            >
              {{ t }}
            </button>
          </div>
        </div>
        <div v-if="ai.error" class="error">{{ ai.error }}</div>
      </section>

      <!-- Human side -->
      <section class="picker-card human">
        <div class="card-header">
          <span class="role-badge human-badge">人工修改最终版本</span>
          <span v-if="human.file && human.tab" class="picked">{{ human.file.name }} › {{ human.tab }}</span>
        </div>
        <div class="paste-section">
          <input
            v-model="human.pasteUrl"
            placeholder="粘贴 Sheet URL..."
            @keyup.enter="handlePasteUrl(human)"
          />
          <button @click="handlePasteUrl(human)" :disabled="!human.pasteUrl">Go</button>
        </div>
        <div class="file-list-wrap">
          <div v-if="loadingFiles" class="hint">Loading...</div>
          <div v-else class="file-list">
            <div
              v-for="f in files"
              :key="f.id"
              class="file-item"
              :class="{ active: human.file?.id === f.id }"
              @click="pickFile(human, f)"
            >
              <span class="file-name">{{ f.name }}</span>
              <span class="file-date">{{ formatDate(f.modified_time) }}</span>
            </div>
          </div>
        </div>
        <div v-if="human.file" class="tabs-area">
          <div class="tabs-label">Tab:</div>
          <div v-if="human.loadingTabs" class="hint">Loading tabs...</div>
          <div v-else class="tab-list">
            <button
              v-for="t in human.tabs"
              :key="t"
              class="tab-btn"
              :class="{ active: human.tab === t }"
              @click="human.tab = t"
            >
              {{ t }}
            </button>
          </div>
        </div>
        <div v-if="human.error" class="error">{{ human.error }}</div>
      </section>
    </div>

    <div class="action-row">
      <button class="compare-btn" :disabled="!canCompare" @click="handleCompare">
        {{ running ? "Comparing..." : "对比" }}
      </button>
      <span v-if="progress" class="progress">{{ progress }}</span>
      <button
        v-if="reportPath"
        class="chrome-btn"
        @click="handleOpenInChrome"
      >
        🌐 在 Chrome 中打开
      </button>
    </div>

    <div v-if="errorMsg" class="error global-error">{{ errorMsg }}</div>

    <div v-if="logs.length > 0" class="log-panel" ref="logPanel">
      <div
        v-for="(log, i) in logs"
        :key="i"
        class="log-line"
        :class="`kind-${log.kind}`"
      >
        <span class="log-time">{{ log.timestamp }}</span>
        <span class="log-text">{{ log.text }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.compare-page {
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
.header-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 4px;
}
.refresh-btn {
  background: none;
  border: 1px solid #ddd;
  border-radius: 4px;
  font-size: 14px;
  color: #888;
  cursor: pointer;
  padding: 1px 8px;
  line-height: 1;
}
.refresh-btn:hover:not(:disabled) {
  background: #f5f5f5;
  color: #333;
}
.refresh-btn:disabled {
  opacity: 0.3;
  cursor: not-allowed;
}
.subtitle {
  margin: 0 0 16px 0;
  font-size: 12px;
  color: #888;
}

.picker-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
  margin-bottom: 16px;
}

.picker-card {
  display: flex;
  flex-direction: column;
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 12px;
  background: #fafafa;
  min-height: 320px;
}
.picker-card.ai {
  border-top: 3px solid #ed8936;
}
.picker-card.human {
  border-top: 3px solid #48bb78;
}

.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 10px;
}
.role-badge {
  font-size: 12px;
  font-weight: 600;
  padding: 3px 8px;
  border-radius: 4px;
  color: white;
}
.ai-badge {
  background: #ed8936;
}
.human-badge {
  background: #48bb78;
}
.picked {
  font-size: 11px;
  color: #555;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 60%;
}

.paste-section {
  display: flex;
  gap: 6px;
  margin-bottom: 8px;
}
.paste-section input {
  flex: 1;
  padding: 6px 10px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
}
.paste-section input:focus {
  border-color: #667eea;
}
.paste-section button {
  padding: 6px 12px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  cursor: pointer;
}

.file-list-wrap {
  flex: 1;
  max-height: 180px;
  overflow-y: auto;
  border: 1px solid #eee;
  border-radius: 6px;
  background: white;
}
.file-list {
  display: flex;
  flex-direction: column;
}
.file-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 6px 10px;
  cursor: pointer;
  font-size: 12px;
  border-bottom: 1px solid #f3f3f3;
}
.file-item:last-child {
  border-bottom: none;
}
.file-item:hover {
  background: #f5f5fa;
}
.file-item.active {
  background: #667eea;
  color: white;
}
.file-item.active .file-date {
  color: rgba(255, 255, 255, 0.7);
}
.file-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  margin-right: 8px;
}
.file-date {
  font-size: 11px;
  color: #999;
  white-space: nowrap;
}

.hint {
  font-size: 12px;
  color: #999;
  padding: 8px;
}

.tabs-area {
  margin-top: 10px;
}
.tabs-label {
  font-size: 11px;
  color: #888;
  margin-bottom: 4px;
}
.tab-list {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.tab-btn {
  padding: 3px 10px;
  font-size: 11px;
  border: 1px solid #ddd;
  border-radius: 4px;
  background: white;
  cursor: pointer;
  white-space: nowrap;
}
.tab-btn.active {
  background: #667eea;
  color: white;
  border-color: #667eea;
}

.action-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 12px;
}
.compare-btn {
  padding: 8px 24px;
  font-size: 14px;
  font-weight: 500;
  border: none;
  border-radius: 6px;
  background: #667eea;
  color: white;
  cursor: pointer;
}
.compare-btn:hover:not(:disabled) {
  background: #5a67d8;
}
.compare-btn:disabled {
  background: #ccc;
  cursor: not-allowed;
}
.progress {
  font-size: 12px;
  color: #666;
}
.chrome-btn {
  padding: 8px 16px;
  font-size: 13px;
  border: 1px solid #4285f4;
  border-radius: 6px;
  background: white;
  color: #4285f4;
  cursor: pointer;
  font-weight: 500;
}
.chrome-btn:hover {
  background: #4285f4;
  color: white;
}

.error {
  color: #e53e3e;
  font-size: 12px;
  margin-top: 6px;
}
.global-error {
  padding: 10px;
  background: #fff5f5;
  border: 1px solid #fed7d7;
  border-radius: 6px;
  margin-bottom: 12px;
}

.log-panel {
  flex: 1;
  min-height: 160px;
  max-height: 320px;
  overflow-y: auto;
  background: #1e1e2e;
  color: #ddd;
  padding: 10px 12px;
  border-radius: 6px;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  font-size: 12px;
}
.log-line {
  display: flex;
  gap: 8px;
  padding: 2px 0;
  white-space: pre-wrap;
  word-break: break-all;
}
.log-time {
  color: #666;
  flex-shrink: 0;
}
.log-text {
  flex: 1;
}
.kind-info .log-text {
  color: #88c0d0;
}
.kind-tool .log-text {
  color: #ebcb8b;
}
.kind-error .log-text {
  color: #ff6b6b;
}
.kind-result .log-text {
  color: #a3e635;
}
</style>

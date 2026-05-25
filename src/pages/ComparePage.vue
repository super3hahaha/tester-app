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

// Paths of the two HTML files exported for the most recent compare run.
// Kept around so the feedback action can bundle them without re-exporting.
const lastAiHtmlPath = ref<string | null>(null);
const lastHumanHtmlPath = ref<string | null>(null);

// Feedback state.
const feedbackConfigured = ref(false);
const showFeedbackModal = ref(false);
const feedbackIssueType = ref<"missing_case" | "wrong_expected" | "wrong_module" | "other">(
  "missing_case"
);
const feedbackNote = ref("");
const feedbackSending = ref(false);
const feedbackResult = ref<string | null>(null);
const feedbackError = ref("");

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
  try {
    feedbackConfigured.value = await invoke<boolean>("is_feedback_configured");
  } catch {
    feedbackConfigured.value = false;
  }
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
  lastAiHtmlPath.value = null;
  lastHumanHtmlPath.value = null;
  feedbackResult.value = null;
  feedbackError.value = "";

  try {
    progress.value = "Exporting AI version as HTML...";
    pushLog(`[1/3] Exporting AI version: ${ai.value.file!.name} > ${ai.value.tab}`, "info");
    const aiPath = await invoke<string>("export_sheet_html", {
      spreadsheetId: ai.value.file!.id,
      tabName: ai.value.tab,
      role: "ai",
    });
    lastAiHtmlPath.value = aiPath;
    pushLog(`  -> ${aiPath}`);

    progress.value = "Exporting Human version as HTML...";
    pushLog(`[2/3] Exporting Human version: ${human.value.file!.name} > ${human.value.tab}`, "info");
    const humanPath = await invoke<string>("export_sheet_html", {
      spreadsheetId: human.value.file!.id,
      tabName: human.value.tab,
      role: "human",
    });
    lastHumanHtmlPath.value = humanPath;
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

function openFeedbackModal() {
  feedbackIssueType.value = "missing_case";
  feedbackNote.value = "";
  feedbackError.value = "";
  feedbackResult.value = null;
  showFeedbackModal.value = true;
}

function closeFeedbackModal() {
  if (feedbackSending.value) return;
  showFeedbackModal.value = false;
}

async function handleSubmitFeedback() {
  if (
    feedbackSending.value ||
    !reportPath.value ||
    !lastAiHtmlPath.value ||
    !lastHumanHtmlPath.value ||
    !ai.value.file ||
    !human.value.file
  )
    return;
  feedbackSending.value = true;
  feedbackError.value = "";
  feedbackResult.value = null;
  try {
    const result = await invoke<{ ok: boolean; had_sources: boolean; message: string }>(
      "send_feedback",
      {
        input: {
          ai_drive_id: ai.value.file.id,
          ai_html_path: lastAiHtmlPath.value,
          human_html_path: lastHumanHtmlPath.value,
          report_path: reportPath.value,
          issue_type: feedbackIssueType.value,
          note: feedbackNote.value,
          ai_sheet_name: ai.value.file.name,
          ai_tab_name: ai.value.tab,
          human_sheet_name: human.value.file.name,
          human_tab_name: human.value.tab,
        },
      }
    );
    feedbackResult.value = result.had_sources
      ? "✅ 已发送（含源文件）"
      : "✅ 已发送（无源文件 — 该 AI Sheet 没有关联的 manifest）";
    setTimeout(() => {
      showFeedbackModal.value = false;
    }, 1200);
  } catch (e: any) {
    feedbackError.value = String(e);
  } finally {
    feedbackSending.value = false;
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
      <button
        v-if="reportPath && feedbackConfigured"
        class="feedback-btn"
        @click="openFeedbackModal"
        title="把这次的样本发给开发者用于优化 skill"
      >
        📝 反馈
      </button>
    </div>

    <!-- Feedback modal -->
    <div v-if="showFeedbackModal" class="modal-backdrop" @click.self="closeFeedbackModal">
      <div class="modal">
        <div class="modal-header">
          <h4>反馈这次结果</h4>
          <button class="modal-close" @click="closeFeedbackModal" :disabled="feedbackSending">×</button>
        </div>
        <div class="modal-body">
          <p class="modal-hint">
            会上传 AI 版本 / 人工版本 / diff 报告 + 关联的需求源文件（PPTX/CSV，如有）+ 你的备注给开发者。
          </p>
          <div class="form-group">
            <label>问题类型</label>
            <div class="radio-group">
              <label class="radio-item">
                <input type="radio" v-model="feedbackIssueType" value="missing_case" />
                漏用例 / 应有未生成
              </label>
              <label class="radio-item">
                <input type="radio" v-model="feedbackIssueType" value="wrong_expected" />
                预期写得不准
              </label>
              <label class="radio-item">
                <input type="radio" v-model="feedbackIssueType" value="wrong_module" />
                模块分类错
              </label>
              <label class="radio-item">
                <input type="radio" v-model="feedbackIssueType" value="other" />
                其他
              </label>
            </div>
          </div>
          <div class="form-group">
            <label>补充说明（可选）</label>
            <textarea
              v-model="feedbackNote"
              rows="3"
              placeholder="例如：边界场景考虑不全，缺少异常路径用例…"
              :disabled="feedbackSending"
            ></textarea>
          </div>
          <div v-if="feedbackError" class="error modal-error">{{ feedbackError }}</div>
          <div v-if="feedbackResult" class="modal-success">{{ feedbackResult }}</div>
        </div>
        <div class="modal-footer">
          <button class="cancel-btn" @click="closeFeedbackModal" :disabled="feedbackSending">
            取消
          </button>
          <button class="submit-btn" @click="handleSubmitFeedback" :disabled="feedbackSending">
            {{ feedbackSending ? "发送中..." : "发送给开发者" }}
          </button>
        </div>
      </div>
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
.feedback-btn {
  padding: 8px 14px;
  font-size: 13px;
  border: 1px solid #c96342;
  border-radius: 6px;
  background: white;
  color: #c96342;
  cursor: pointer;
  font-weight: 500;
}
.feedback-btn:hover {
  background: #c96342;
  color: white;
}

.modal-backdrop {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}
.modal {
  background: white;
  border-radius: 10px;
  width: 480px;
  max-width: 92vw;
  max-height: 86vh;
  display: flex;
  flex-direction: column;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
  overflow: hidden;
}
.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 18px;
  border-bottom: 1px solid #eee;
}
.modal-header h4 {
  margin: 0;
  font-size: 15px;
}
.modal-close {
  background: none;
  border: none;
  font-size: 22px;
  color: #888;
  cursor: pointer;
  line-height: 1;
  padding: 0 4px;
}
.modal-close:hover:not(:disabled) {
  color: #333;
}
.modal-close:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.modal-body {
  padding: 16px 18px;
  overflow-y: auto;
  font-size: 13px;
  color: #2d3748;
}
.modal-hint {
  margin: 0 0 14px 0;
  font-size: 12px;
  color: #718096;
  line-height: 1.5;
}
.form-group {
  margin-bottom: 14px;
}
.form-group > label {
  display: block;
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
  margin-bottom: 6px;
}
.radio-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.radio-item {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  cursor: pointer;
  font-weight: normal;
}
.radio-item input[type="radio"] {
  margin: 0;
}
.form-group textarea {
  width: 100%;
  padding: 8px 10px;
  font-size: 13px;
  font-family: inherit;
  border: 1px solid #ddd;
  border-radius: 6px;
  resize: vertical;
  outline: none;
  box-sizing: border-box;
}
.form-group textarea:focus {
  border-color: #c96342;
}
.form-group textarea:disabled {
  background: #f5f5f5;
  opacity: 0.6;
}
.modal-error {
  background: #fff5f5;
  border: 1px solid #fed7d7;
  padding: 8px 10px;
  border-radius: 6px;
  font-size: 12px;
  color: #c53030;
  margin-top: 6px;
  word-break: break-all;
}
.modal-success {
  font-size: 13px;
  color: #2d9248;
  background: #f0fdf4;
  border: 1px solid #bbf7d0;
  padding: 8px 10px;
  border-radius: 6px;
  margin-top: 6px;
}
.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 18px;
  border-top: 1px solid #eee;
  background: #fafafa;
}
.cancel-btn {
  padding: 7px 14px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.cancel-btn:hover:not(:disabled) {
  background: #f5f5f5;
}
.cancel-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.submit-btn {
  padding: 7px 16px;
  font-size: 13px;
  font-weight: 500;
  border: none;
  border-radius: 6px;
  background: #c96342;
  color: white;
  cursor: pointer;
}
.submit-btn:hover:not(:disabled) {
  background: #b85838;
}
.submit-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
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

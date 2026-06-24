<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// 模块级单例，防止组件重建时产生多个监听
let _claudeLogUnlisten: UnlistenFn | null = null;
import { openUrl } from "@tauri-apps/plugin-opener";

interface SheetSelection {
  spreadsheetId: string;
  spreadsheetName: string;
  tabName: string;
  data: { headers: string[]; rows: string[][]; spreadsheet_url: string };
}

interface SlidesSelection {
  id: string;
  name: string;
  pages: number[];
}

const props = defineProps<{
  sheetSelection: SheetSelection | null;
  slidesSelection: SlidesSelection[];
}>();

interface LogEvent {
  text: string;
  kind: string;
  tool?: string;
  duration_ms?: number;
  timestamp: string;
}

function nowHms(): string {
  const d = new Date();
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

function pushLog(text: string, kind = "text", tool?: string, duration_ms?: number) {
  logs.value.push({ text, kind, tool, duration_ms, timestamp: nowHms() });
}

function iconFor(kind: string): string {
  switch (kind) {
    case "tool":
      return "🔧";
    case "tool_done":
      return "✓";
    case "error":
      return "✗";
    case "system":
      return "ℹ";
    case "result":
      return "✓";
    case "user":
      return "›";
    case "info":
      return "·";
    default:
      return " ";
  }
}

function formatLogText(log: LogEvent): string {
  if (log.kind === "tool_done") {
    const dur = log.duration_ms != null ? ` (${(log.duration_ms / 1000).toFixed(1)}s)` : "";
    const trimmed = log.text.trim();
    if (!trimmed) {
      return `${log.tool || "Tool"} completed with no output${dur}`;
    }
    const first = trimmed.split("\n")[0];
    const more = trimmed.includes("\n") ? " …" : "";
    return `${first}${more}${dur}`;
  }
  return log.text;
}

const generating = ref(false);
const progress = ref("");
const logs = ref<LogEvent[]>([]);
const error = ref("");
const done = ref(false);
const hasSession = ref(false);
const userInput = ref("");
const extraInfo = ref("");
const sending = ref(false);
const logPanel = ref<HTMLElement | null>(null);
const stopping = ref(false);
const cancelRequested = ref(false);
const claudeStarted = ref(false);

interface ModelOption {
  id: string;
  label: string;
}
const MODELS: ModelOption[] = [
  { id: "claude-sonnet-4-6", label: "Sonnet 4.6" },
  { id: "claude-opus-4-7", label: "Opus 4.7" },
  { id: "claude-haiku-4-5-20251001", label: "Haiku 4.5" },
];
const selectedModel = ref<string>(MODELS[0].id);

interface ExportInfo {
  path: string;
  name: string;
}
interface UploadResult {
  drive_id: string;
  web_url: string;
  converted: boolean;
}
const latestExport = ref<ExportInfo | null>(null);
const uploading = ref(false);
const uploadResult = ref<UploadResult | null>(null);
const uploadError = ref("");
// Timestamp (ms) when the current task started. Used to filter find_latest_export
// so we only surface xlsx files created/modified during this run — not stale outputs
// from a previous run that didn't get cleaned up.
const generationStartMs = ref<number | null>(null);

// Snapshot of the generation context — captured at handleGenerate time and used
// when writing the manifest after Drive upload. Keeps the link between the
// uploaded sheet (Drive ID) and its source files (CSV + PPTX + page selections).
interface GenContext {
  csvPath: string | null;
  pptxPaths: string[];
  slidePages: { name: string; pages: number[] }[];
  model: string;
}
const lastGenContext = ref<GenContext | null>(null);

async function resolveSkillVersion(): Promise<string> {
  try {
    const ver = await invoke<string | null>("get_skill_local_version", {
      name: "test-case-generator",
    });
    if (ver) {
      return `test-case-generator@${ver}`;
    }
  } catch {}
  return "test-case-generator@unknown";
}

const idleSeconds = ref(0);
const idleActive = ref(false);
const IDLE_THRESHOLD = 3;
let idleTimer: ReturnType<typeof setInterval> | null = null;
let idleStart = 0;

function startIdleWatch() {
  stopIdleWatch();
  idleStart = Date.now();
  idleSeconds.value = 0;
  idleActive.value = true;
  idleTimer = setInterval(() => {
    idleSeconds.value = Math.floor((Date.now() - idleStart) / 1000);
  }, 1000);
}

function stopIdleWatch() {
  if (idleTimer) {
    clearInterval(idleTimer);
    idleTimer = null;
  }
  idleActive.value = false;
  idleSeconds.value = 0;
}

const showIdle = computed(
  () => generating.value && idleActive.value && idleSeconds.value >= IDLE_THRESHOLD
);

let unlisten: UnlistenFn | null = null;

onMounted(async () => {
  _claudeLogUnlisten?.();
  _claudeLogUnlisten = await listen<{ text: string; kind: string; tool?: string; duration_ms?: number; done: boolean }>(
    "claude-log",
    (event) => {
      const { text, kind, tool, duration_ms, done: isDone } = event.payload;
      pushLog(text, kind, tool, duration_ms);
      if (isDone) {
        done.value = true;
        generating.value = false;
        sending.value = false;
        progress.value = "Done!";
        stopIdleWatch();
        checkSession();
        refreshLatestExport();
      } else {
        startIdleWatch();
      }
      nextTick(() => {
        if (logPanel.value) {
          logPanel.value.scrollTop = logPanel.value.scrollHeight;
        }
      });
    }
  );
  unlisten = _claudeLogUnlisten;

  checkSession();
});

onUnmounted(() => {
  stopIdleWatch();
  // 不在 unmount 时取消监听——组件销毁后后台任务仍可能在跑，
  // 重建时 onMounted 会先清掉旧监听再注册新的
});

async function checkSession() {
  try {
    const [running, session] = await invoke<[boolean, boolean]>("get_claude_status");
    generating.value = running;
    hasSession.value = session;
  } catch {}
}

async function refreshLatestExport() {
  try {
    latestExport.value = await invoke<ExportInfo | null>("find_latest_export", {
      sinceMs: generationStartMs.value,
    });
  } catch {
    latestExport.value = null;
  }
}

async function handleUploadToDrive() {
  if (!latestExport.value || uploading.value) return;
  uploading.value = true;
  uploadError.value = "";
  uploadResult.value = null;
  try {
    uploadResult.value = await invoke<UploadResult>("upload_xlsx_to_drive", {
      filePath: latestExport.value.path,
      convertToSheets: true,
      folderName: "tester-app",
    });
    // Write manifest linking the uploaded Drive sheet back to its source files,
    // so the feedback flow in ComparePage can pull source CSV/PPTX into the zip.
    // Silent on failure — manifest is non-critical; compare-feedback degrades to "no sources".
    if (uploadResult.value && lastGenContext.value) {
      try {
        const skillVersion = await resolveSkillVersion();
        await invoke("write_generate_manifest", {
          driveId: uploadResult.value.drive_id,
          webUrl: uploadResult.value.web_url,
          sourceCsvPath: lastGenContext.value.csvPath,
          pptxPaths: lastGenContext.value.pptxPaths,
          slidePages: lastGenContext.value.slidePages,
          model: lastGenContext.value.model,
          skillVersion,
        });
      } catch (e) {
        console.warn("write_generate_manifest failed:", e);
      }
    }
  } catch (e: any) {
    uploadError.value = String(e);
  } finally {
    uploading.value = false;
  }
}

const canGenerate = computed(
  () => props.slidesSelection.length > 0
);

const canSendInput = computed(
  () => hasSession.value && !generating.value && !sending.value && userInput.value.trim().length > 0
);

async function handleGenerate() {
  if (props.slidesSelection.length === 0) return;

  generating.value = true;
  cancelRequested.value = false;
  claudeStarted.value = false;
  error.value = "";
  done.value = false;
  hasSession.value = false;
  logs.value = [];
  latestExport.value = null;
  uploadResult.value = null;
  uploadError.value = "";
  generationStartMs.value = Date.now();
  startIdleWatch();

  try {
    let csvPath: string | null = null;
    if (props.sheetSelection) {
      progress.value = "Exporting Sheet as CSV...";
      pushLog(
        `[1/3] Exporting "${props.sheetSelection.spreadsheetName}" > "${props.sheetSelection.tabName}" as CSV`,
        "info"
      );

      csvPath = await invoke<string>("export_sheet_csv", {
        spreadsheetId: props.sheetSelection.spreadsheetId,
        range: props.sheetSelection.tabName,
      });
      pushLog(`  -> ${csvPath}`);
      if (cancelRequested.value) throw new Error("__cancelled__");
    } else {
      pushLog("[1/3] No Sheet selected — generating from requirements only", "info");
    }

    const pptxPaths: string[] = [];
    for (let i = 0; i < props.slidesSelection.length; i++) {
      if (cancelRequested.value) throw new Error("__cancelled__");
      const slide = props.slidesSelection[i];
      progress.value = `Exporting PPTX ${i + 1}/${props.slidesSelection.length}...`;
      pushLog(`[2/3] Exporting "${slide.name}" as PDF`, "info");

      const pptxPath = await invoke<string>("export_slides_pdf", {
        presentationId: slide.id,
        name: slide.name,
      });
      pptxPaths.push(pptxPath);
      pushLog(`  -> ${pptxPath}`);
    }

    if (cancelRequested.value) throw new Error("__cancelled__");
    claudeStarted.value = true;
    progress.value = "Generating test cases with Claude...";
    pushLog("[3/3] Launching Claude CLI with /test-case-generator skill", "info");
    if (csvPath) pushLog(`  CSV: ${csvPath}`);
    pushLog(`  PPTX: ${pptxPaths.join(", ")}`);

    const pageSelections = props.slidesSelection.map((s) => ({
      name: s.name,
      pages: s.pages,
    }));

    lastGenContext.value = {
      csvPath,
      pptxPaths,
      slidePages: pageSelections,
      model: selectedModel.value,
    };

    const extra = extraInfo.value.trim();
    extraInfo.value = "";

    await invoke("run_claude_task", {
      csvPath,
      pptxPaths,
      pageSelections,
      model: selectedModel.value,
      extraInfo: extra || null,
    });
  } catch (e: any) {
    if (String(e).includes("__cancelled__")) {
      pushLog("Stopped.", "info");
    } else {
      error.value = String(e);
    }
    progress.value = "";
    generating.value = false;
    stopping.value = false;
    cancelRequested.value = false;
    claudeStarted.value = false;
  }
}

async function handleSendInput() {
  if (!canSendInput.value) return;

  const input = userInput.value.trim();
  userInput.value = "";
  sending.value = true;
  generating.value = true;
  done.value = false;
  error.value = "";
  progress.value = "Sending to Claude...";

  pushLog(`> ${input}`, "user");
  startIdleWatch();

  try {
    await invoke("send_claude_input", { input, model: selectedModel.value });
  } catch (e: any) {
    error.value = String(e);
    sending.value = false;
    generating.value = false;
    progress.value = "";
  }
}

function handleInputKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    handleSendInput();
  }
}

async function handleStop() {
  if (!generating.value || stopping.value) return;
  stopping.value = true;
  cancelRequested.value = true;
  if (claudeStarted.value) {
    try {
      await invoke("stop_claude");
      pushLog("Stop requested — terminating Claude process...", "info");
    } catch (e: any) {
      error.value = String(e);
      stopping.value = false;
    }
  }
  // if not yet at Claude stage, cancelRequested flag will abort the export loop
}
</script>

<template>
  <div class="generate-page">
    <!-- Top bar: selections + action in one row -->
    <div class="top-bar">
      <div class="sel-card" :class="{ empty: !sheetSelection }">
        <span class="sel-title">Google Sheet</span>
        <span v-if="sheetSelection" class="sel-meta">
          {{ sheetSelection.spreadsheetName }} · {{ sheetSelection.tabName }} · {{ sheetSelection.data.rows.length }} rows · {{ sheetSelection.data.headers.length }} columns
        </span>
        <span v-else class="sel-empty">No sheet selected (optional)</span>
      </div>

      <div class="sel-card" :class="{ empty: slidesSelection.length === 0 }">
        <span class="sel-title">Google Slides</span>
        <span v-if="slidesSelection.length > 0" class="sel-meta">
          <span v-for="s in slidesSelection" :key="s.id">{{ s.name }} · pages {{ s.pages.join(", ") }}</span>
        </span>
        <span v-else class="sel-empty">No slides selected</span>
      </div>

      <div class="action-group">
        <div class="model-picker">
          <label>Model</label>
          <select v-model="selectedModel" :disabled="generating">
            <option v-for="m in MODELS" :key="m.id" :value="m.id">{{ m.label }}</option>
          </select>
        </div>
        <button
          class="generate-btn"
          :disabled="!canGenerate || generating"
          @click="handleGenerate"
        >
          {{ generating ? progress : "Generate Test Cases" }}
        </button>
        <button
          v-if="generating"
          class="stop-btn"
          :disabled="stopping"
          @click="handleStop"
        >
          {{ stopping ? "Stopping..." : "■ Stop" }}
        </button>
      </div>
    </div>

    <!-- Extra info (only before session starts) -->
    <div v-if="!generating && !hasSession" class="extra-info-area">
      <textarea
        v-model="extraInfo"
        class="extra-info-input"
        placeholder="Additional info (optional) — extra context, constraints, or notes"
        rows="2"
      ></textarea>
    </div>

    <!-- Error -->
    <div v-if="error" class="error">{{ error }}</div>

    <!-- Logs -->
    <div v-if="logs.length > 0" class="log-panel">
      <div class="log-header">
        <span>Log</span>
        <span v-if="done && !hasSession" class="log-done">Complete</span>
        <span v-else-if="done && hasSession" class="log-waiting">Waiting for input...</span>
        <span v-else-if="generating" class="log-running">Running...</span>
      </div>
      <div ref="logPanel" class="log-content">
        <div
          v-for="(log, i) in logs"
          :key="i"
          class="log-line"
          :class="`log-${log.kind}`"
        >
          <span class="log-ts">{{ log.timestamp }}</span>
          <span class="log-icon" :class="`icon-${log.kind}`">{{ iconFor(log.kind) }}</span>
          <span v-if="log.tool" class="log-tool">{{ log.tool }}</span>
          <span class="log-text">{{ formatLogText(log) }}</span>
        </div>
        <div v-if="showIdle" class="log-line log-idle">
          <span class="log-icon idle-pulse">⏳</span>
          <span class="log-text">Claude is thinking… ({{ idleSeconds }}s)</span>
        </div>
      </div>

      <!-- Drive upload panel -->
      <div v-if="done && latestExport" class="drive-panel">
        <div class="drive-row">
          <span class="drive-icon">📤</span>
          <span class="drive-file">{{ latestExport.name }}</span>
          <button
            class="drive-btn"
            :disabled="uploading"
            @click="handleUploadToDrive"
          >
            {{ uploading ? "Uploading..." : "Upload to Drive" }}
          </button>
        </div>
        <div v-if="uploadResult" class="drive-success">
          ✅ Uploaded
          <a href="#" @click.prevent="openUrl(uploadResult.web_url)">
            Open in Google Sheets ↗
          </a>
        </div>
        <div v-if="uploadError" class="drive-error">{{ uploadError }}</div>
      </div>

      <!-- Input area for Claude interaction -->
      <div v-if="hasSession && done" class="input-area">
        <textarea
          v-model="userInput"
          class="claude-input"
          placeholder="Type your response to Claude..."
          rows="2"
          :disabled="sending"
          @keydown="handleInputKeydown"
        ></textarea>
        <button
          class="send-btn"
          :disabled="!canSendInput"
          @click="handleSendInput"
        >
          {{ sending ? "Sending..." : "Send" }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.generate-page {
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 16px 20px;
  box-sizing: border-box;
  overflow: hidden;
}
h3 {
  font-size: 16px;
  margin-bottom: 4px;
}
.top-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}
.sel-card {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 8px;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  padding: 8px 12px;
  background: white;
  min-width: 0;
}
.sel-card.empty {
  border-style: dashed;
  border-color: #ccc;
  background: #fafafa;
}
.sel-title {
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
  flex-shrink: 0;
}
.sel-meta {
  font-size: 12px;
  color: #888;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sel-empty {
  font-size: 12px;
  color: #bbb;
}
.action-group {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.generate-btn {
  padding: 8px 20px;
  font-size: 13px;
  font-weight: 600;
  color: white;
  background: linear-gradient(135deg, #667eea, #764ba2);
  border: none;
  border-radius: 8px;
  cursor: pointer;
  transition: opacity 0.2s;
}
.generate-btn:hover:not(:disabled) {
  opacity: 0.9;
}
.generate-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.action-hint {
  font-size: 12px;
  color: #bbb;
}
.model-picker {
  display: flex;
  align-items: center;
  gap: 6px;
}
.model-picker label {
  font-size: 12px;
  color: #666;
}
.model-picker select {
  padding: 6px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  cursor: pointer;
  outline: none;
}
.model-picker select:focus {
  border-color: #667eea;
}
.model-picker select:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.stop-btn {
  padding: 8px 18px;
  font-size: 13px;
  font-weight: 600;
  color: white;
  background: #e53e3e;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  transition: opacity 0.2s;
}
.stop-btn:hover:not(:disabled) {
  opacity: 0.9;
}
.stop-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.extra-info-input {
  width: 100%;
  padding: 10px 12px;
  font-size: 13px;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Helvetica Neue", Arial, sans-serif;
  background: #ffffff;
  color: #2d3748;
  border: 1px solid #d4d4d8;
  border-radius: 6px;
  resize: vertical;
  outline: none;
  box-sizing: border-box;
  transition: border-color 0.15s;
}
.extra-info-input:focus {
  border-color: #667eea;
}
.error {
  color: #e53e3e;
  font-size: 13px;
  margin-bottom: 12px;
  padding: 10px;
  background: #fff5f5;
  border-radius: 6px;
}
.log-panel {
  flex: 1;
  min-height: 0;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
.log-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px 14px;
  background: #f5f5f5;
  font-size: 12px;
  font-weight: 600;
  color: #666;
}
.log-done {
  color: #48bb78;
}
.log-running {
  color: #667eea;
}
.log-waiting {
  color: #ecc94b;
}
.log-content {
  padding: 10px 14px;
  font-size: 12.5px;
  font-family: ui-monospace, SFMono-Regular, "SF Mono", Consolas, "Liberation Mono", Menlo, monospace;
  line-height: 1.7;
  background: #ffffff;
  color: #2d3748;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  margin: 0;
}
.log-line {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  white-space: pre-wrap;
  word-break: break-word;
}
.log-ts {
  color: #a0a0a0;
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
  user-select: none;
}
.log-icon {
  width: 1.1em;
  flex-shrink: 0;
  text-align: center;
  user-select: none;
}
.icon-tool { color: #c96342; }
.icon-tool_done,
.icon-result { color: #48bb78; }
.icon-error { color: #e53e3e; }
.icon-system { color: #667eea; }
.icon-user { color: #667eea; }
.log-tool {
  font-weight: 600;
  color: #c96342;
  flex-shrink: 0;
}
.log-text {
  flex: 1;
  min-width: 0;
}
.log-tool_done .log-text { color: #888; }
.log-error .log-text { color: #c53030; }
.log-system .log-text { color: #4a5568; }
.log-info .log-text { color: #718096; }
.log-user .log-text { color: #2d3748; font-weight: 500; }
.log-idle .log-text {
  color: #888;
  font-style: italic;
}
.idle-pulse {
  animation: idle-pulse 1.4s ease-in-out infinite;
}
@keyframes idle-pulse {
  0%, 100% { opacity: 0.4; }
  50% { opacity: 1; }
}
.input-area {
  display: flex;
  gap: 8px;
  padding: 12px 14px;
  background: #fafafa;
  border-top: 1px solid #e0e0e0;
}
.claude-input {
  flex: 1;
  padding: 10px 12px;
  font-size: 13px;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Helvetica Neue", Arial, sans-serif;
  background: #ffffff;
  color: #2d3748;
  border: 1px solid #d4d4d8;
  border-radius: 6px;
  resize: none;
  outline: none;
  transition: border-color 0.15s;
}
.claude-input:focus {
  border-color: #c96342;
}
.claude-input:disabled {
  opacity: 0.5;
  background: #f5f5f5;
}
.send-btn {
  padding: 8px 18px;
  font-size: 13px;
  font-weight: 600;
  color: white;
  background: #c96342;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  align-self: flex-end;
  transition: background 0.15s;
}
.send-btn:hover:not(:disabled) {
  background: #b85838;
}
.send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.drive-panel {
  padding: 12px 14px;
  background: #fafafa;
  border-top: 1px solid #e0e0e0;
  color: #2d3748;
  font-size: 13px;
}
.drive-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.drive-icon {
  font-size: 16px;
}
.drive-file {
  flex: 1;
  font-size: 12px;
  color: #6b7280;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.drive-btn {
  padding: 6px 14px;
  font-size: 12px;
  font-weight: 600;
  color: white;
  background: #34a853;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  transition: background 0.15s;
}
.drive-btn:hover:not(:disabled) {
  background: #2d9248;
}
.drive-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.drive-success {
  margin-top: 8px;
  font-size: 12px;
  color: #2d9248;
}
.drive-success a {
  color: #c96342;
  margin-left: 6px;
  text-decoration: none;
}
.drive-success a:hover {
  text-decoration: underline;
}
.drive-error {
  margin-top: 8px;
  font-size: 12px;
  color: #c53030;
  word-break: break-all;
}
</style>

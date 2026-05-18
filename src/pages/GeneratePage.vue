<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

const generating = ref(false);
const progress = ref("");
const logs = ref<string[]>([]);
const error = ref("");
const done = ref(false);
const hasSession = ref(false);
const userInput = ref("");
const sending = ref(false);
const logPanel = ref<HTMLElement | null>(null);

let unlisten: UnlistenFn | null = null;

onMounted(async () => {
  unlisten = await listen<{ text: string; kind: string; done: boolean }>(
    "claude-log",
    (event) => {
      const { text, done: isDone } = event.payload;
      logs.value.push(text);
      if (isDone) {
        done.value = true;
        generating.value = false;
        sending.value = false;
        progress.value = "Done!";
        checkSession();
      }
      nextTick(() => {
        if (logPanel.value) {
          logPanel.value.scrollTop = logPanel.value.scrollHeight;
        }
      });
    }
  );

  checkSession();
});

onUnmounted(() => {
  unlisten?.();
});

async function checkSession() {
  try {
    const [running, session] = await invoke<[boolean, boolean]>("get_claude_status");
    generating.value = running;
    hasSession.value = session;
  } catch {}
}

const canGenerate = computed(
  () => props.sheetSelection && props.slidesSelection.length > 0
);

const canSendInput = computed(
  () => hasSession.value && !generating.value && !sending.value && userInput.value.trim().length > 0
);

async function handleGenerate() {
  if (!props.sheetSelection || props.slidesSelection.length === 0) return;

  generating.value = true;
  error.value = "";
  done.value = false;
  hasSession.value = false;
  logs.value = [];

  try {
    progress.value = "Exporting Sheet as CSV...";
    logs.value.push(
      `[1/3] Exporting "${props.sheetSelection.spreadsheetName}" > "${props.sheetSelection.tabName}" as CSV`
    );

    const csvPath = await invoke<string>("export_sheet_csv", {
      spreadsheetId: props.sheetSelection.spreadsheetId,
      range: props.sheetSelection.tabName,
    });
    logs.value.push(`  -> ${csvPath}`);

    const pptxPaths: string[] = [];
    for (let i = 0; i < props.slidesSelection.length; i++) {
      const slide = props.slidesSelection[i];
      progress.value = `Exporting PPTX ${i + 1}/${props.slidesSelection.length}...`;
      logs.value.push(`[2/3] Exporting "${slide.name}" as PPTX`);

      const pptxPath = await invoke<string>("export_slides_pptx", {
        presentationId: slide.id,
        name: slide.name,
      });
      pptxPaths.push(pptxPath);
      logs.value.push(`  -> ${pptxPath}`);
    }

    progress.value = "Generating test cases with Claude...";
    logs.value.push("[3/3] Launching Claude CLI with /test-case-generator skill");
    logs.value.push(`  CSV: ${csvPath}`);
    logs.value.push(`  PPTX: ${pptxPaths.join(", ")}`);
    logs.value.push("");

    const pageSelections = props.slidesSelection.map((s) => ({
      name: s.name,
      pages: s.pages,
    }));

    await invoke("run_claude_task", {
      csvPath,
      pptxPaths,
      pageSelections,
    });
  } catch (e: any) {
    error.value = String(e);
    progress.value = "";
    generating.value = false;
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

  logs.value.push("");
  logs.value.push(`> ${input}`);
  logs.value.push("");

  try {
    await invoke("send_claude_input", { input });
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
</script>

<template>
  <div class="generate-page">
    <h3>Generate Test Cases</h3>
    <p class="subtitle">
      Review selections and generate test cases using Claude
    </p>

    <!-- Selections summary -->
    <div class="selections">
      <div class="sel-card" :class="{ empty: !sheetSelection }">
        <div class="sel-header">
          <span class="sel-icon">📊</span>
          <span class="sel-title">Google Sheet</span>
        </div>
        <div v-if="sheetSelection" class="sel-detail">
          <div class="sel-name">{{ sheetSelection.spreadsheetName }}</div>
          <div class="sel-meta">
            Tab: {{ sheetSelection.tabName }} ·
            {{ sheetSelection.data.rows.length }} rows ·
            {{ sheetSelection.data.headers.length }} columns
          </div>
        </div>
        <div v-else class="sel-empty">
          No sheet selected — go to Google Sheets tab and select one
        </div>
      </div>

      <div class="sel-card" :class="{ empty: slidesSelection.length === 0 }">
        <div class="sel-header">
          <span class="sel-icon">📑</span>
          <span class="sel-title">Google Slides</span>
        </div>
        <div v-if="slidesSelection.length > 0" class="sel-detail">
          <div
            v-for="s in slidesSelection"
            :key="s.id"
            class="sel-slide-item"
          >
            {{ s.name }}
            <span class="sel-pages">pages {{ s.pages.join(", ") }}</span>
          </div>
        </div>
        <div v-else class="sel-empty">
          No slides selected — go to Google Slides tab and select pages
        </div>
      </div>
    </div>

    <!-- Generate button -->
    <div class="action-area">
      <button
        class="generate-btn"
        :disabled="!canGenerate || generating"
        @click="handleGenerate"
      >
        {{ generating ? progress : "Generate Test Cases" }}
      </button>
      <span v-if="!canGenerate && !generating" class="action-hint">
        Select both a Sheet and at least one Slides to continue
      </span>
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
      <pre ref="logPanel" class="log-content"><template v-for="(line, i) in logs" :key="i">{{ line }}
</template></pre>

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
  overflow-y: auto;
  padding: 24px;
}
h3 {
  font-size: 16px;
  margin-bottom: 4px;
}
.subtitle {
  font-size: 13px;
  color: #888;
  margin-bottom: 20px;
}
.selections {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-bottom: 24px;
}
.sel-card {
  border: 1px solid #e0e0e0;
  border-radius: 10px;
  padding: 16px;
  background: white;
}
.sel-card.empty {
  border-style: dashed;
  border-color: #ccc;
  background: #fafafa;
}
.sel-header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.sel-icon {
  font-size: 18px;
}
.sel-title {
  font-size: 14px;
  font-weight: 600;
}
.sel-detail {
  padding-left: 26px;
}
.sel-name {
  font-size: 14px;
  font-weight: 500;
  color: #333;
}
.sel-meta {
  font-size: 12px;
  color: #888;
  margin-top: 2px;
}
.sel-slide-item {
  font-size: 13px;
  color: #333;
  padding: 2px 0;
}
.sel-slide-item::before {
  content: "✓ ";
  color: #667eea;
}
.sel-pages {
  font-size: 11px;
  color: #888;
  margin-left: 6px;
}
.sel-empty {
  font-size: 13px;
  color: #bbb;
  padding-left: 26px;
}
.action-area {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 20px;
}
.generate-btn {
  padding: 12px 28px;
  font-size: 14px;
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
.error {
  color: #e53e3e;
  font-size: 13px;
  margin-bottom: 12px;
  padding: 10px;
  background: #fff5f5;
  border-radius: 6px;
}
.log-panel {
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  overflow: hidden;
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
  padding: 12px 14px;
  font-size: 12px;
  font-family: "Cascadia Code", "Fira Code", Consolas, monospace;
  line-height: 1.6;
  background: #1e1e2e;
  color: #cdd6f4;
  max-height: 400px;
  overflow-y: auto;
  margin: 0;
  white-space: pre-wrap;
  word-break: break-all;
}
.input-area {
  display: flex;
  gap: 8px;
  padding: 10px 14px;
  background: #2a2a3a;
  border-top: 1px solid #3a3a4a;
}
.claude-input {
  flex: 1;
  padding: 8px 12px;
  font-size: 13px;
  font-family: "Cascadia Code", "Fira Code", Consolas, monospace;
  background: #1e1e2e;
  color: #cdd6f4;
  border: 1px solid #3a3a4a;
  border-radius: 6px;
  resize: none;
  outline: none;
}
.claude-input:focus {
  border-color: #667eea;
}
.claude-input:disabled {
  opacity: 0.5;
}
.send-btn {
  padding: 8px 20px;
  font-size: 13px;
  font-weight: 600;
  color: white;
  background: linear-gradient(135deg, #667eea, #764ba2);
  border: none;
  border-radius: 6px;
  cursor: pointer;
  align-self: flex-end;
  transition: opacity 0.2s;
}
.send-btn:hover:not(:disabled) {
  opacity: 0.9;
}
.send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>

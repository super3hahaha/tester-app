<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

interface DriveFile {
  id: string;
  name: string;
  modified_time: string;
  mime_type: string;
}

interface SheetData {
  headers: string[];
  rows: string[][];
  spreadsheet_url: string;
}

export interface SheetSelection {
  spreadsheetId: string;
  spreadsheetName: string;
  tabName: string;
  data: SheetData;
}

const emit = defineEmits<{
  (e: "select", selection: SheetSelection): void;
  (e: "clear"): void;
}>();

const files = ref<DriveFile[]>([]);
const selectedFile = ref<DriveFile | null>(null);
const tabs = ref<string[]>([]);
const selectedTab = ref("");
const sheetData = ref<SheetData | null>(null);
const loading = ref(false);
const loadingSheet = ref(false);
const error = ref("");
const pasteUrl = ref("");

const confirmed = ref<{ spreadsheetId: string; tabName: string } | null>(null);

const fileInput = ref<HTMLInputElement | null>(null);
const uploading = ref(false);
const uploadStatus = ref("");

function triggerUpload() {
  fileInput.value?.click();
}

async function handleFileSelected(e: Event) {
  const input = e.target as HTMLInputElement;
  const all = Array.from(input.files ?? []);
  input.value = "";
  if (all.length === 0) return;

  const xlsx = all.filter((f) => f.name.toLowerCase().endsWith(".xlsx"));
  const skipped = all.length - xlsx.length;
  if (xlsx.length === 0) {
    error.value = "Please select .xlsx file(s)";
    return;
  }

  uploading.value = true;
  error.value = "";
  const succeeded: string[] = [];
  const failed: { name: string; reason: string }[] = [];

  try {
    for (let i = 0; i < xlsx.length; i++) {
      const file = xlsx[i];
      uploadStatus.value = `Uploading ${i + 1}/${xlsx.length}: ${file.name}`;
      try {
        const buf = await file.arrayBuffer();
        const bytes = Array.from(new Uint8Array(buf));
        await invoke("upload_xlsx_bytes_to_drive", {
          fileName: file.name,
          bytes,
          convertToSheets: true,
          folderName: "tester-app",
        });
        succeeded.push(file.name);
      } catch (err: any) {
        failed.push({ name: file.name, reason: String(err) });
      }
    }

    await loadFiles();
    if (succeeded.length > 0) {
      window.dispatchEvent(new CustomEvent("drive-files-updated"));
    }

    const parts: string[] = [];
    if (succeeded.length > 0) parts.push(`Uploaded ${succeeded.length}/${xlsx.length}`);
    if (failed.length > 0) parts.push(`${failed.length} failed`);
    if (skipped > 0) parts.push(`${skipped} skipped (not .xlsx)`);
    uploadStatus.value = parts.join(" · ");

    if (failed.length > 0) {
      error.value = failed.map((f) => `${f.name}: ${f.reason}`).join("\n");
    }

    setTimeout(() => {
      if (uploadStatus.value.startsWith("Uploaded") || uploadStatus.value.includes("skipped")) {
        uploadStatus.value = "";
      }
    }, 3000);
  } finally {
    uploading.value = false;
  }
}

onMounted(() => loadFiles());

async function loadFiles() {
  loading.value = true;
  error.value = "";
  try {
    files.value = await invoke<DriveFile[]>("list_drive_files", {
      mimeType: "application/vnd.google-apps.spreadsheet",
    });
  } catch (e: any) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function selectFile(file: DriveFile) {
  selectedFile.value = file;
  sheetData.value = null;
  selectedTab.value = "";
  error.value = "";

  try {
    tabs.value = await invoke<string[]>("get_sheet_tabs", {
      spreadsheetId: file.id,
    });
    if (tabs.value.length > 0) {
      selectedTab.value = tabs.value[0];
      await loadSheet();
    }
  } catch (e: any) {
    error.value = String(e);
  }
}

async function loadSheet() {
  if (!selectedFile.value || !selectedTab.value) return;
  loadingSheet.value = true;
  error.value = "";
  try {
    sheetData.value = await invoke<SheetData>("read_sheet", {
      spreadsheetId: selectedFile.value.id,
      range: selectedTab.value,
    });
  } catch (e: any) {
    error.value = String(e);
  } finally {
    loadingSheet.value = false;
  }
}

function confirmSelection() {
  if (!selectedFile.value || !selectedTab.value || !sheetData.value) return;
  confirmed.value = {
    spreadsheetId: selectedFile.value.id,
    tabName: selectedTab.value,
  };
  emit("select", {
    spreadsheetId: selectedFile.value.id,
    spreadsheetName: selectedFile.value.name,
    tabName: selectedTab.value,
    data: sheetData.value,
  });
}

function clearSelection() {
  confirmed.value = null;
  emit("clear");
}

function isConfirmed(fileId: string, tab: string) {
  return confirmed.value?.spreadsheetId === fileId && confirmed.value?.tabName === tab;
}

async function handlePasteUrl() {
  const match = pasteUrl.value.match(/\/spreadsheets\/d\/([a-zA-Z0-9_-]+)/);
  if (!match) {
    error.value = "Invalid Google Sheets URL";
    return;
  }
  const id = match[1];
  await selectFile({ id, name: "Pasted Sheet", modified_time: "", mime_type: "" });
}

async function openInBrowser() {
  const url = sheetData.value?.spreadsheet_url;
  if (!url) return;
  try {
    await openUrl(url);
  } catch (e: any) {
    error.value = `Failed to open browser: ${String(e)}`;
  }
}

function formatDate(iso: string) {
  if (!iso) return "";
  const d = new Date(iso);
  return `${d.getMonth() + 1}/${d.getDate()} ${d.getHours()}:${String(d.getMinutes()).padStart(2, "0")}`;
}
</script>

<template>
  <div class="sheets-page">
    <!-- Left: File picker -->
    <div class="sidebar">
      <h3>Google Sheets</h3>

      <!-- Selection badge -->
      <div v-if="confirmed" class="selection-badge">
        <div class="badge-label">Selected</div>
        <div class="badge-content">
          <span class="badge-file">{{ selectedFile?.name }}</span>
          <span class="badge-tab">{{ confirmed.tabName }}</span>
        </div>
        <button class="badge-clear" @click="clearSelection">&times;</button>
      </div>

      <div class="paste-section">
        <input
          v-model="pasteUrl"
          placeholder="Paste Sheet URL..."
          @keyup.enter="handlePasteUrl"
        />
        <button @click="handlePasteUrl" :disabled="!pasteUrl">Go</button>
      </div>
      <div class="upload-section">
        <input
          ref="fileInput"
          type="file"
          accept=".xlsx"
          multiple
          style="display: none"
          @change="handleFileSelected"
        />
        <button
          class="upload-btn"
          @click="triggerUpload"
          :disabled="uploading"
          :title="uploading ? 'Uploading...' : 'Upload one or more .xlsx (auto-converts to Google Sheets)'"
        >
          {{ uploading ? "Uploading..." : "+ Upload xlsx" }}
        </button>
        <div v-if="uploadStatus" class="upload-status">{{ uploadStatus }}</div>
      </div>
      <div class="divider">
        <span>or select recent</span>
        <button class="refresh-btn" @click="loadFiles" :disabled="loading" title="Refresh">&#8635;</button>
      </div>
      <div v-if="loading" class="hint">Loading...</div>
      <div v-else class="file-list">
        <div
          v-for="f in files"
          :key="f.id"
          class="file-item"
          :class="{ active: selectedFile?.id === f.id }"
          @click="selectFile(f)"
        >
          <span class="file-name">{{ f.name }}</span>
          <span class="file-date">{{ formatDate(f.modified_time) }}</span>
        </div>
        <div v-if="files.length === 0" class="hint">No sheets found</div>
      </div>
    </div>

    <!-- Right: Preview -->
    <div class="main-area">
      <div v-if="!selectedFile" class="empty-state">
        Select a Google Sheet to preview
      </div>
      <template v-else>
        <div class="toolbar">
          <div class="tab-bar">
            <button
              v-for="tab in tabs"
              :key="tab"
              class="tab-btn"
              :class="{
                active: selectedTab === tab,
                confirmed: isConfirmed(selectedFile!.id, tab),
              }"
              @click="selectedTab = tab; loadSheet()"
            >
              {{ tab }}
              <span v-if="isConfirmed(selectedFile!.id, tab)" class="check-icon">&#10003;</span>
            </button>
          </div>
          <div class="toolbar-actions">
            <button
              class="use-btn"
              :class="{ active: isConfirmed(selectedFile!.id, selectedTab) }"
              @click="isConfirmed(selectedFile!.id, selectedTab) ? clearSelection() : confirmSelection()"
              :disabled="!sheetData"
            >
              {{ isConfirmed(selectedFile!.id, selectedTab) ? "Deselect" : "Use this sheet" }}
            </button>
            <button @click="openInBrowser">Open in Browser</button>
          </div>
        </div>

        <div v-if="error" class="error">{{ error }}</div>

        <div v-if="sheetData" class="table-wrapper">
          <table>
            <thead>
              <tr>
                <th class="row-num">#</th>
                <th v-for="(h, i) in sheetData.headers" :key="i">{{ h }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(row, ri) in sheetData.rows" :key="ri">
                <td class="row-num">{{ ri + 1 }}</td>
                <td v-for="(cell, ci) in row" :key="ci">{{ cell }}</td>
              </tr>
            </tbody>
          </table>
          <div class="table-info">
            {{ sheetData.rows.length }} rows x {{ sheetData.headers.length }} columns
          </div>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.sheets-page {
  display: flex;
  height: 100%;
  overflow: hidden;
}
.sidebar {
  width: 280px;
  min-width: 280px;
  border-right: 1px solid #e5e5e5;
  padding: 16px;
  overflow-y: auto;
  background: #fafafa;
}
.sidebar h3 {
  font-size: 15px;
  margin-bottom: 12px;
}

/* Selection badge */
.selection-badge {
  background: #eef2ff;
  border: 1px solid #667eea;
  border-radius: 8px;
  padding: 10px 12px;
  margin-bottom: 12px;
  position: relative;
}
.badge-label {
  font-size: 10px;
  font-weight: 600;
  color: #667eea;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  margin-bottom: 4px;
}
.badge-content {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.badge-file {
  font-size: 13px;
  font-weight: 500;
  color: #333;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.badge-tab {
  font-size: 11px;
  color: #667eea;
}
.badge-clear {
  position: absolute;
  top: 6px;
  right: 8px;
  background: none;
  border: none;
  font-size: 16px;
  color: #999;
  cursor: pointer;
  padding: 0 4px;
}
.badge-clear:hover {
  color: #e53e3e;
}

.paste-section {
  display: flex;
  gap: 6px;
  margin-bottom: 12px;
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
.upload-section {
  margin-bottom: 8px;
}
.upload-btn {
  width: 100%;
  padding: 8px 12px;
  font-size: 12px;
  font-weight: 500;
  border: 1px dashed #667eea;
  border-radius: 6px;
  background: white;
  color: #667eea;
  cursor: pointer;
  transition: background 0.15s;
}
.upload-btn:hover:not(:disabled) {
  background: #eef2ff;
}
.upload-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.upload-status {
  margin-top: 6px;
  padding: 6px 8px;
  font-size: 11px;
  color: #276749;
  background: #f0fff4;
  border-radius: 4px;
  word-break: break-all;
}
.divider {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  font-size: 11px;
  color: #aaa;
  margin: 8px 0;
}
.refresh-btn {
  background: none;
  border: 1px solid #ddd;
  border-radius: 4px;
  font-size: 14px;
  color: #888;
  cursor: pointer;
  padding: 1px 5px;
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
.file-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.file-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px 10px;
  border-radius: 6px;
  cursor: pointer;
  font-size: 13px;
}
.file-item:hover {
  background: #eee;
}
.file-item.active {
  background: #667eea;
  color: white;
}
.file-item.active .file-date {
  color: rgba(255, 255, 255, 0.7);
}
.file-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  margin-right: 8px;
}
.file-date {
  font-size: 11px;
  color: #999;
  white-space: nowrap;
}
.hint {
  font-size: 13px;
  color: #999;
  padding: 8px;
}
.main-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.empty-state {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: #bbb;
  font-size: 15px;
}
.toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px 16px;
  border-bottom: 1px solid #e5e5e5;
  background: white;
  gap: 8px;
}
.tab-bar {
  display: flex;
  gap: 4px;
  overflow-x: auto;
  flex: 1;
}
.tab-btn {
  padding: 4px 12px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 4px;
  background: white;
  cursor: pointer;
  white-space: nowrap;
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.tab-btn.active {
  background: #667eea;
  color: white;
  border-color: #667eea;
}
.tab-btn.confirmed {
  border-color: #48bb78;
  background: #f0fff4;
  color: #276749;
}
.tab-btn.active.confirmed {
  background: #48bb78;
  color: white;
  border-color: #48bb78;
}
.check-icon {
  font-size: 11px;
}
.toolbar-actions {
  display: flex;
  gap: 6px;
}
.toolbar-actions button {
  padding: 4px 12px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 4px;
  background: white;
  cursor: pointer;
}
.toolbar-actions button:hover {
  background: #f5f5f5;
}
.use-btn {
  font-weight: 500;
  border-color: #667eea !important;
  color: #667eea;
}
.use-btn:hover {
  background: #eef2ff !important;
}
.use-btn.active {
  background: #48bb78 !important;
  color: white !important;
  border-color: #48bb78 !important;
}
.use-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.error {
  color: #e53e3e;
  padding: 12px 16px;
  font-size: 13px;
}
.table-wrapper {
  flex: 1;
  overflow: auto;
  padding: 0;
}
table {
  border-collapse: collapse;
  width: max-content;
  min-width: 100%;
  font-size: 13px;
}
thead {
  position: sticky;
  top: 0;
  z-index: 1;
}
th {
  background: #f0f0f0;
  font-weight: 600;
  text-align: left;
  padding: 8px 12px;
  border: 1px solid #e0e0e0;
  white-space: nowrap;
}
td {
  padding: 6px 12px;
  border: 1px solid #e8e8e8;
  max-width: 300px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
tr:hover td {
  background: #f8f8ff;
}
.row-num {
  color: #aaa;
  text-align: center;
  width: 40px;
  min-width: 40px;
  font-size: 11px;
  background: #fafafa;
}
.table-info {
  padding: 8px 16px;
  font-size: 12px;
  color: #999;
  border-top: 1px solid #e5e5e5;
  background: white;
}
</style>

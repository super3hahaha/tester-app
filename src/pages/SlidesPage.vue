<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface DriveFile {
  id: string;
  name: string;
  modified_time: string;
  mime_type: string;
}

interface SlidePageInfo {
  page_object_id: string;
  page_number: number;
  thumbnail_url: string;
}

export interface SlidesSelection {
  id: string;
  name: string;
  pages: number[];
}

const emit = defineEmits<{
  (e: "select", files: SlidesSelection[]): void;
}>();

const files = ref<DriveFile[]>([]);
const loading = ref(false);
const error = ref("");
const pasteUrl = ref("");

const activeFileId = ref<string | null>(null);
const slidePages = ref<SlidePageInfo[]>([]);
const loadingSlides = ref(false);
const slidesError = ref("");
const focusedPage = ref<SlidePageInfo | null>(null);
const thumbStrip = ref<HTMLElement | null>(null);

const pageSelections = ref<Map<string, Set<number>>>(new Map());

let unlistenThumb: UnlistenFn | null = null;

onMounted(async () => {
  loadFiles();
  unlistenThumb = await listen<{
    presentation_id: string;
    page_number: number;
    thumbnail_url: string;
  }>("slide-thumbnail", (event) => {
    const { presentation_id, page_number, thumbnail_url } = event.payload;
    if (presentation_id !== activeFileId.value) return;
    const page = slidePages.value.find((p) => p.page_number === page_number);
    if (page) {
      page.thumbnail_url = thumbnail_url;
    }
  });
});

onUnmounted(() => {
  unlistenThumb?.();
});

async function loadFiles() {
  loading.value = true;
  error.value = "";
  try {
    files.value = await invoke<DriveFile[]>("list_drive_files", {
      mimeType: "application/vnd.google-apps.presentation",
    });
  } catch (e: any) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function selectFile(file: DriveFile) {
  if (activeFileId.value === file.id) return;
  activeFileId.value = file.id;
  await loadSlides(file.id);
}

async function loadSlides(fileId: string) {
  slidePages.value = [];
  focusedPage.value = null;
  loadingSlides.value = true;
  slidesError.value = "";

  try {
    slidePages.value = await invoke<SlidePageInfo[]>("get_presentation_slides", {
      presentationId: fileId,
    });
    if (slidePages.value.length > 0) {
      focusedPage.value = slidePages.value[0];
    }
  } catch (e: any) {
    slidesError.value = String(e);
  } finally {
    loadingSlides.value = false;
  }
}

async function refreshActiveSlides() {
  if (!activeFileId.value || loadingSlides.value) return;
  // 选中状态保留：后端会比对 revisionId，若内容没变则秒返回；
  // 若变了则后端清缓存重拉，已选页码若对应的 objectId 还在仍然有意义，
  // 但若用户删了被选的页，这里也无法智能修复——交给用户重新确认
  await loadSlides(activeFileId.value);
}

function focusPage(page: SlidePageInfo) {
  focusedPage.value = page;
}

function togglePage(fileId: string, pageNum: number) {
  const map = new Map(pageSelections.value);
  const pages = new Set(map.get(fileId) || []);
  if (pages.has(pageNum)) {
    pages.delete(pageNum);
  } else {
    pages.add(pageNum);
  }
  if (pages.size === 0) {
    map.delete(fileId);
  } else {
    map.set(fileId, pages);
  }
  pageSelections.value = map;
  emitSelection();
}

function toggleFocusedPage() {
  if (!focusedPage.value || !activeFileId.value) return;
  togglePage(activeFileId.value, focusedPage.value.page_number);
}

function selectAllPages() {
  if (!activeFileId.value || slidePages.value.length === 0) return;
  const map = new Map(pageSelections.value);
  const pages = new Set(slidePages.value.map((s) => s.page_number));
  map.set(activeFileId.value, pages);
  pageSelections.value = map;
  emitSelection();
}

function deselectAllPages() {
  if (!activeFileId.value) return;
  const map = new Map(pageSelections.value);
  map.delete(activeFileId.value);
  pageSelections.value = map;
  emitSelection();
}

function selectInterval() {
  if (!activeFileId.value) return;
  const current = pageSelections.value.get(activeFileId.value);
  if (!current || current.size < 2) return;
  // 取已选页码的最小值和最大值，把区间内所有页都勾上
  const arr = Array.from(current);
  const min = Math.min(...arr);
  const max = Math.max(...arr);
  const filled = new Set<number>();
  for (let i = min; i <= max; i++) {
    filled.add(i);
  }
  const map = new Map(pageSelections.value);
  map.set(activeFileId.value, filled);
  pageSelections.value = map;
  emitSelection();
}

const canSelectInterval = computed(() => activeSelectedCount.value >= 2);

const activeSelectedCount = computed(() => {
  if (!activeFileId.value) return 0;
  return pageSelections.value.get(activeFileId.value)?.size || 0;
});

const allPagesSelected = computed(() => {
  if (!activeFileId.value || slidePages.value.length === 0) return false;
  return activeSelectedCount.value === slidePages.value.length;
});

function isPageSelected(pageNum: number): boolean {
  if (!activeFileId.value) return false;
  return pageSelections.value.get(activeFileId.value)?.has(pageNum) || false;
}

function getFileSelectedCount(fileId: string): number {
  return pageSelections.value.get(fileId)?.size || 0;
}

function emitSelection() {
  const selected: SlidesSelection[] = [];
  for (const [id, pages] of pageSelections.value.entries()) {
    if (pages.size === 0) continue;
    const file = files.value.find((f) => f.id === id);
    selected.push({
      id,
      name: file?.name || "Unknown",
      pages: Array.from(pages).sort((a, b) => a - b),
    });
  }
  emit("select", selected);
}

async function handlePasteUrl() {
  const match = pasteUrl.value.match(/\/presentation\/d\/([a-zA-Z0-9_-]+)/);
  if (!match) {
    error.value = "Invalid Google Slides URL";
    return;
  }
  const id = match[1];
  const exists = files.value.find((f) => f.id === id);
  if (!exists) {
    files.value.unshift({
      id,
      name: "Pasted Slides",
      modified_time: "",
      mime_type: "application/vnd.google-apps.presentation",
    });
  }
  pasteUrl.value = "";
  selectFile({ id, name: exists?.name || "Pasted Slides", modified_time: "", mime_type: "" });
}

function openInBrowser(id: string) {
  window.open(`https://docs.google.com/presentation/d/${id}`, "_blank");
}

function formatDate(iso: string) {
  if (!iso) return "";
  const d = new Date(iso);
  return `${d.getMonth() + 1}/${d.getDate()} ${d.getHours()}:${String(d.getMinutes()).padStart(2, "0")}`;
}

const activeFileName = computed(() => {
  if (!activeFileId.value) return "";
  return files.value.find((f) => f.id === activeFileId.value)?.name || "";
});
</script>

<template>
  <div class="slides-page">
    <!-- Left: file list -->
    <div class="file-panel">
      <div class="panel-header">
        <div class="header-row">
          <h3>Google Slides</h3>
          <button class="refresh-btn" @click="loadFiles" :disabled="loading" title="Refresh file list">&#8635;</button>
        </div>
        <p class="subtitle">Select presentations and pick pages</p>
      </div>

      <div class="paste-section">
        <input
          v-model="pasteUrl"
          placeholder="Paste Slides URL..."
          @keyup.enter="handlePasteUrl"
        />
        <button @click="handlePasteUrl" :disabled="!pasteUrl">Add</button>
      </div>

      <div v-if="error" class="error">{{ error }}</div>
      <div v-if="loading" class="hint">Loading...</div>

      <div v-else class="file-list">
        <div
          v-for="f in files"
          :key="f.id"
          class="file-item"
          :class="{ active: activeFileId === f.id, 'has-pages': getFileSelectedCount(f.id) > 0 }"
          @click="selectFile(f)"
        >
          <div class="file-icon">📑</div>
          <div class="file-info">
            <span class="file-name">{{ f.name }}</span>
            <span class="file-date">{{ formatDate(f.modified_time) }}</span>
          </div>
          <span v-if="getFileSelectedCount(f.id) > 0" class="page-badge">
            {{ getFileSelectedCount(f.id) }}p
          </span>
          <button class="file-open" @click.stop="openInBrowser(f.id)" title="Open in browser">↗</button>
        </div>
        <div v-if="files.length === 0" class="hint">No Google Slides found</div>
      </div>
    </div>

    <!-- Middle: big preview -->
    <div class="preview-panel">
      <template v-if="!activeFileId">
        <div class="empty-state">Click a presentation to preview its slides</div>
      </template>

      <template v-else-if="loadingSlides">
        <div class="empty-state">Loading slides...</div>
      </template>

      <template v-else-if="slidesError">
        <div class="error" style="padding: 20px">{{ slidesError }}</div>
      </template>

      <template v-else-if="focusedPage">
        <div class="preview-header">
          <div class="preview-title">
            <span class="preview-name">{{ activeFileName }}</span>
            <span class="preview-page-num">Page {{ focusedPage.page_number }} / {{ slidePages.length }}</span>
          </div>
          <div class="preview-actions">
            <button
              class="select-page-btn"
              :class="{ active: isPageSelected(focusedPage.page_number) }"
              @click="toggleFocusedPage"
            >
              {{ isPageSelected(focusedPage.page_number) ? '&#10003; Selected' : 'Select this page' }}
            </button>
            <button v-if="!allPagesSelected" class="action-link" @click="selectAllPages">Select All</button>
            <button
              class="action-link"
              :disabled="!canSelectInterval"
              :title="canSelectInterval ? 'Select all pages between the lowest and highest selected page' : 'Select at least 2 pages first'"
              @click="selectInterval"
            >Select Interval</button>
            <button
              v-if="activeSelectedCount > 0"
              class="action-link"
              @click="deselectAllPages"
            >Unselect All</button>
            <span v-if="activeSelectedCount > 0" class="selected-hint">
              {{ activeSelectedCount }} selected
            </span>
            <button
              class="refresh-btn"
              @click="refreshActiveSlides"
              :disabled="loadingSlides"
              title="Refresh slides (re-fetch if the presentation has changed)"
            >&#8635;</button>
          </div>
        </div>
        <div class="preview-body">
          <img
            v-if="focusedPage.thumbnail_url"
            :src="focusedPage.thumbnail_url"
            class="preview-img"
          />
          <div v-else class="preview-placeholder">No preview available</div>
        </div>
      </template>
    </div>

    <!-- Right: thumbnail strip -->
    <div v-if="activeFileId && slidePages.length > 0" class="thumb-strip" ref="thumbStrip">
      <div
        v-for="page in slidePages"
        :key="page.page_object_id"
        class="strip-item"
        :class="{
          focused: focusedPage?.page_number === page.page_number,
          selected: isPageSelected(page.page_number),
        }"
        @click="focusPage(page)"
      >
        <div class="strip-num">{{ page.page_number }}</div>
        <div class="strip-thumb-wrap">
          <div class="strip-checkbox" @click.stop="togglePage(activeFileId!, page.page_number)">
            <div class="cb" :class="{ on: isPageSelected(page.page_number) }">
              <span v-if="isPageSelected(page.page_number)">&#10003;</span>
            </div>
          </div>
          <img
            v-if="page.thumbnail_url"
            :src="page.thumbnail_url"
            class="strip-thumb"
            loading="lazy"
          />
          <div v-else class="strip-placeholder"></div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.slides-page {
  height: 100%;
  display: flex;
  overflow: hidden;
}

/* Left panel: file list */
.file-panel {
  width: 240px;
  min-width: 240px;
  border-right: 1px solid #e5e5e5;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.panel-header {
  padding: 16px 14px 0;
}
.header-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 4px;
}
.panel-header h3 {
  font-size: 15px;
  margin: 0;
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
.subtitle {
  font-size: 11px;
  color: #888;
  margin-bottom: 12px;
}
.paste-section {
  display: flex;
  gap: 6px;
  padding: 0 14px 12px;
}
.paste-section input {
  flex: 1;
  padding: 6px 10px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
  min-width: 0;
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
.paste-section button:hover {
  background: #f5f5f5;
}
.error {
  color: #e53e3e;
  font-size: 12px;
  padding: 0 14px 8px;
}
.hint {
  color: #999;
  font-size: 13px;
  padding: 20px 14px;
}
.file-list {
  flex: 1;
  overflow-y: auto;
  padding: 0 6px 8px;
}
.file-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 9px 8px;
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.15s;
}
.file-item:hover {
  background: #f5f5f5;
}
.file-item.active {
  background: #eef2ff;
}
.file-item.has-pages {
  font-weight: 500;
}
.file-icon {
  font-size: 18px;
  flex-shrink: 0;
}
.file-info {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.file-name {
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.file-date {
  font-size: 10px;
  color: #999;
}
.page-badge {
  background: #667eea;
  color: white;
  font-size: 10px;
  padding: 2px 6px;
  border-radius: 8px;
  flex-shrink: 0;
  font-weight: 600;
}
.file-open {
  flex-shrink: 0;
  background: none;
  border: 1px solid #ddd;
  border-radius: 4px;
  padding: 2px 6px;
  font-size: 12px;
  cursor: pointer;
  color: #666;
  opacity: 0;
  transition: opacity 0.15s;
}
.file-item:hover .file-open {
  opacity: 1;
}
.file-open:hover {
  background: #eee;
}

/* Middle: big preview */
.preview-panel {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: #f0f0f0;
  min-width: 0;
}
.empty-state {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: #bbb;
  font-size: 14px;
}
.preview-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 10px 16px;
  background: white;
  border-bottom: 1px solid #e5e5e5;
  flex-shrink: 0;
  gap: 12px;
}
.preview-title {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.preview-name {
  font-size: 13px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.preview-page-num {
  font-size: 11px;
  color: #888;
}
.preview-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-shrink: 0;
}
.select-page-btn {
  padding: 5px 12px;
  font-size: 12px;
  font-weight: 600;
  border: 1px solid #667eea;
  border-radius: 6px;
  background: white;
  color: #667eea;
  cursor: pointer;
  transition: all 0.15s;
  white-space: nowrap;
}
.select-page-btn.active {
  background: #667eea;
  color: white;
}
.select-page-btn:hover {
  opacity: 0.85;
}
.action-link {
  background: none;
  border: none;
  color: #667eea;
  font-size: 12px;
  cursor: pointer;
  padding: 0;
  white-space: nowrap;
}
.action-link:hover:not(:disabled) {
  text-decoration: underline;
}
.action-link:disabled {
  color: #bbb;
  cursor: not-allowed;
}
.selected-hint {
  font-size: 12px;
  color: #667eea;
  font-weight: 500;
  white-space: nowrap;
}
.preview-body {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  overflow: auto;
}
.preview-img {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
  border-radius: 4px;
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.15);
}
.preview-placeholder {
  color: #bbb;
  font-size: 14px;
}

/* Right: thumbnail strip */
.thumb-strip {
  width: 160px;
  min-width: 160px;
  background: #fafafa;
  border-left: 1px solid #e5e5e5;
  overflow-y: auto;
  padding: 10px 8px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.strip-item {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  cursor: pointer;
  padding: 4px;
  border-radius: 6px;
  transition: background 0.15s;
}
.strip-item:hover {
  background: #eee;
}
.strip-item.focused {
  background: #e0e7ff;
}
.strip-num {
  font-size: 11px;
  color: #999;
  min-width: 18px;
  text-align: right;
  padding-top: 2px;
  flex-shrink: 0;
}
.strip-item.selected .strip-num {
  color: #667eea;
  font-weight: 600;
}
.strip-thumb-wrap {
  flex: 1;
  position: relative;
  border: 2px solid #ddd;
  border-radius: 4px;
  overflow: hidden;
}
.strip-item.focused .strip-thumb-wrap {
  border-color: #667eea;
}
.strip-item.selected .strip-thumb-wrap {
  border-color: #667eea;
}
.strip-checkbox {
  position: absolute;
  top: 3px;
  left: 3px;
  z-index: 1;
}
.cb {
  width: 18px;
  height: 18px;
  border: 2px solid rgba(150, 150, 150, 0.7);
  border-radius: 3px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  color: white;
  background: rgba(255, 255, 255, 0.85);
  transition: all 0.15s;
}
.cb.on {
  background: #667eea;
  border-color: #667eea;
}
.strip-thumb {
  width: 100%;
  aspect-ratio: 16 / 9;
  object-fit: contain;
  display: block;
  background: white;
}
.strip-placeholder {
  width: 100%;
  aspect-ratio: 16 / 9;
  background: #eee;
}
</style>

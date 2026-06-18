<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

// app 读的是 Apps Script（gmail-sync.gs）同步出来的 Google Sheet。
// 表的列顺序固定（见 gmail-sync.gs 的 HEADERS）：
//   messageId | threadId | 日期 | 发件人 | 主题 | 正文 | 机翻中文 | 附件 | 邮件链接
// 这里按表头名取列，列顺序变动也不怕。

interface SheetData {
  headers: string[];
  rows: string[][];
  spreadsheet_url: string;
}

// 一个邮件源 = 一张表（对应一个 Gmail 账号）。手动粘贴链接维护。
interface MailSource {
  id: string; // spreadsheet id
  label: string; // 备注（账号邮箱等）
  profileDir?: string; // 用哪个 Chrome profile 打开邮件（目录名；空=系统默认浏览器）
}

interface ChromeProfile {
  dir: string;
  name: string;
}

interface Mail {
  messageId: string;
  threadId: string;
  date: string;
  from: string;
  subject: string;
  body: string;
  translated: string;
  attachments: string;
  link: string;
}

const STORAGE_KEY = "gmail-sources-v1";
// 本地「已读/已处理」隐藏标记：存 messageId，拉取时过滤掉（表里的行不会删，靠这个隐藏）
const STORAGE_KEY_READ = "gmail-read-ids-v1";
// gmail-sync.gs 写入的工作表名
const DEFAULT_TAB = "Mail";

const sources = ref<MailSource[]>([]);
const selectedId = ref("");

const mails = ref<Mail[]>([]);
const selectedMail = ref<Mail | null>(null);
const readIds = ref<string[]>([]); // 已读隐藏的 messageId（全局）
const rawCount = ref(0); // 当前表过滤前的邮件数（算「已隐藏」用）
const loading = ref(false);
const errorMsg = ref("");
const fetchedAt = ref<number | null>(null);
const sheetUrl = ref("");

// 添加表单（仿 ReviewPage 手填包名切换）
const adding = ref(false);
const newUrl = ref("");
const newLabel = ref("");
const newProfile = ref("");

// 本机 Chrome 的 profile 列表（显示名 ↔ 目录名）
const chromeProfiles = ref<ChromeProfile[]>([]);

const currentSource = computed(
  () => sources.value.find((s) => s.id === selectedId.value) || null
);

onMounted(() => {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) sources.value = JSON.parse(raw);
  } catch {
    sources.value = [];
  }
  try {
    readIds.value = JSON.parse(localStorage.getItem(STORAGE_KEY_READ) || "[]");
  } catch {
    readIds.value = [];
  }
  loadChromeProfiles();
  if (sources.value.length > 0) {
    selectedId.value = sources.value[0].id;
    loadMails();
  } else {
    adding.value = true;
  }
});

async function loadChromeProfiles() {
  try {
    chromeProfiles.value = await invoke<ChromeProfile[]>("list_chrome_profiles");
  } catch {
    chromeProfiles.value = [];
  }
}

function onProfileChange(e: Event) {
  const dir = (e.target as HTMLSelectElement).value;
  if (currentSource.value) currentSource.value.profileDir = dir;
}

watch(
  sources,
  () => localStorage.setItem(STORAGE_KEY, JSON.stringify(sources.value)),
  { deep: true }
);

function parseId(s: string): string | null {
  const m = s.match(/\/spreadsheets\/d\/([a-zA-Z0-9_-]+)/);
  if (m) return m[1];
  // 也允许直接粘 ID
  const t = s.trim();
  return /^[a-zA-Z0-9_-]{20,}$/.test(t) ? t : null;
}

function addSource() {
  const id = parseId(newUrl.value);
  if (!id) {
    errorMsg.value = "无效的 Google Sheet 链接（应形如 …/spreadsheets/d/<ID>/edit）";
    return;
  }
  if (sources.value.some((s) => s.id === id)) {
    errorMsg.value = "";
    selectSource(id);
    cancelAdd();
    return;
  }
  const label = newLabel.value.trim() || `表 ${id.slice(0, 6)}…`;
  sources.value.push({ id, label, profileDir: newProfile.value || undefined });
  cancelAdd();
  selectSource(id);
}

function cancelAdd() {
  adding.value = false;
  newUrl.value = "";
  newLabel.value = "";
  newProfile.value = "";
  errorMsg.value = "";
}

function removeCurrent() {
  if (!selectedId.value) return;
  sources.value = sources.value.filter((s) => s.id !== selectedId.value);
  selectedId.value = "";
  mails.value = [];
  fetchedAt.value = null;
  if (sources.value.length > 0) {
    selectSource(sources.value[0].id);
  } else {
    adding.value = true;
  }
}

function selectSource(id: string) {
  selectedId.value = id;
  loadMails();
}

function colIndex(headers: string[], name: string): number {
  return headers.findIndex((h) => h.trim() === name);
}

async function loadMails() {
  if (!selectedId.value) return;
  loading.value = true;
  errorMsg.value = "";
  mails.value = [];
  try {
    // 优先读 Mail tab；万一表里 tab 名不同，退回第一个 tab
    let tab = DEFAULT_TAB;
    try {
      const tabs = await invoke<string[]>("get_sheet_tabs", {
        spreadsheetId: selectedId.value,
      });
      if (tabs.length > 0 && !tabs.includes(DEFAULT_TAB)) tab = tabs[0];
    } catch {
      // 取 tab 失败就直接试 Mail
    }
    const data = await invoke<SheetData>("read_sheet", {
      spreadsheetId: selectedId.value,
      range: tab,
    });
    sheetUrl.value = data.spreadsheet_url;
    const h = data.headers;
    const iMid = colIndex(h, "messageId");
    const iTid = colIndex(h, "threadId");
    const iDate = colIndex(h, "日期");
    const iFrom = colIndex(h, "发件人");
    const iSubj = colIndex(h, "主题");
    const iBody = colIndex(h, "正文");
    const iTrans = colIndex(h, "机翻中文");
    const iAtt = colIndex(h, "附件");
    const iLink = colIndex(h, "邮件链接");
    const g = (row: string[], i: number) => (i >= 0 && i < row.length ? row[i] : "");
    // 脚本是 append 到表底部，最新在最后 → 反转让最新排在最上
    const all = data.rows
      .map((r) => ({
        messageId: g(r, iMid),
        threadId: g(r, iTid),
        date: g(r, iDate),
        from: g(r, iFrom),
        subject: g(r, iSubj),
        body: g(r, iBody),
        translated: g(r, iTrans),
        attachments: g(r, iAtt),
        link: g(r, iLink),
      }))
      .filter((m) => m.messageId || m.subject || m.from)
      .reverse();
    rawCount.value = all.length;
    const readSet = new Set(readIds.value);
    mails.value = all.filter((m) => !readSet.has(m.messageId));
    fetchedAt.value = Date.now();
  } catch (e: any) {
    errorMsg.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function openInGmail(m: Mail) {
  if (!m.link) {
    errorMsg.value = "该邮件没有链接列。";
    return;
  }
  const dir = currentSource.value?.profileDir;
  try {
    if (dir) {
      // 指定 Chrome profile 打开，跳到登录了该账号的窗口
      await invoke("open_url_in_chrome_profile", { url: m.link, profileDir: dir });
    } else {
      // 没指定 profile：退回系统默认浏览器
      await openUrl(m.link);
    }
  } catch (e: any) {
    errorMsg.value = "打开失败：" + String(e);
  }
}

function openDetail(m: Mail) {
  selectedMail.value = m;
}
function closeDetail() {
  selectedMail.value = null;
}

function persistRead() {
  localStorage.setItem(STORAGE_KEY_READ, JSON.stringify(readIds.value));
}
// 标为已读：记下 messageId（下次拉取过滤掉）并从当前列表移除
function markRead(m: Mail) {
  if (m.messageId && !readIds.value.includes(m.messageId)) {
    readIds.value.push(m.messageId);
    persistRead();
  }
  mails.value = mails.value.filter((x) => x.messageId !== m.messageId);
  if (selectedMail.value && selectedMail.value.messageId === m.messageId) {
    selectedMail.value = null;
  }
}
// 撤销：每次只恢复最近标记已读的那一封（LIFO），再点恢复上一封
function undoRead() {
  if (readIds.value.length === 0) return;
  readIds.value.pop();
  persistRead();
  loadMails();
}
const hiddenCount = computed(() => Math.max(0, rawCount.value - mails.value.length));

async function openSheet() {
  if (!sheetUrl.value) return;
  try {
    await openUrl(sheetUrl.value);
  } catch (e: any) {
    errorMsg.value = String(e);
  }
}

function hasAttachment(m: Mail) {
  return m.attachments && m.attachments !== "无";
}

const currentLabel = computed(
  () => sources.value.find((s) => s.id === selectedId.value)?.label || ""
);
</script>

<template>
  <div class="gmail-page">
    <header class="page-header">
      <h3>Gmail 邮件</h3>
      <p class="subtitle">
        读取 Apps Script 同步出的 Google Sheet，把每封邮件按卡片展示；回复点「在 Gmail 中打开」外跳本人发送。
      </p>
    </header>

    <section class="form-card">
      <div class="form-row">
        <label class="form-label">邮件表</label>
        <template v-if="!adding">
          <select
            v-model="selectedId"
            class="src-select"
            :disabled="sources.length === 0"
            @change="loadMails"
          >
            <option v-if="sources.length === 0" disabled value="">暂无邮件表</option>
            <option v-for="s in sources" :key="s.id" :value="s.id">
              {{ s.label }}
            </option>
          </select>
          <button class="icon-btn" @click="loadMails" :disabled="loading || !selectedId" title="重新读取">
            ↻
          </button>
          <button class="icon-btn" @click="adding = true" title="添加邮件表（粘贴链接）">＋</button>
          <button
            class="icon-btn danger"
            @click="removeCurrent"
            :disabled="!selectedId"
            title="移除当前邮件表"
          >
            ✕
          </button>
          <button class="link-btn" @click="openSheet" v-if="sheetUrl">在表格中打开 ↗</button>
        </template>
        <template v-else>
          <input
            v-model="newUrl"
            class="text-input"
            placeholder="粘贴 Google Sheet 链接…"
            @keyup.enter="addSource"
          />
          <input v-model="newLabel" class="label-input" placeholder="备注（账号邮箱，可选）" />
          <button class="fetch-btn" @click="addSource">添加</button>
          <button class="icon-btn" @click="cancelAdd" v-if="sources.length > 0" title="取消">←</button>
        </template>
      </div>

      <!-- 用哪个 Chrome 资料打开邮件（解决多账号分散在不同 profile 时跳转失败） -->
      <div class="form-row profile-row">
        <label class="form-label">浏览器</label>
        <select
          v-if="!adding"
          :value="currentSource?.profileDir || ''"
          @change="onProfileChange"
          class="src-select"
          :disabled="!selectedId"
          title="用哪个 Chrome 个人资料打开该账号的邮件"
        >
          <option value="">系统默认浏览器</option>
          <option v-for="p in chromeProfiles" :key="p.dir" :value="p.dir">
            Chrome · {{ p.name }}
          </option>
        </select>
        <select v-else v-model="newProfile" class="src-select" title="用哪个 Chrome 个人资料打开该账号的邮件">
          <option value="">系统默认浏览器</option>
          <option v-for="p in chromeProfiles" :key="p.dir" :value="p.dir">
            Chrome · {{ p.name }}
          </option>
        </select>
        <span class="profile-hint">
          选登录了该 Gmail 账号的那个 Chrome 资料，「在 Gmail 中打开」就会跳到对的窗口
        </span>
      </div>
    </section>

    <div v-if="errorMsg" class="banner banner-error">{{ errorMsg }}</div>

    <div v-if="fetchedAt && !loading" class="summary-row">
      <span class="summary-text">
        {{ currentLabel }} · 显示 {{ mails.length }} 封<span v-if="hiddenCount > 0"> · 已隐藏 {{ hiddenCount }}</span>
      </span>
      <button v-if="readIds.length > 0" class="link-btn" @click="undoRead">↩ 撤销上一封已读</button>
    </div>

    <div v-if="loading" class="empty-state">读取中…</div>

    <div v-else-if="mails.length > 0" class="mail-list">
      <article v-for="m in mails" :key="m.messageId || m.link" class="mail-item">
        <div class="mi-row1">
          <span class="from">{{ m.from || "(未知发件人)" }}</span>
          <span class="ts">{{ m.date }}</span>
          <span v-if="hasAttachment(m)" class="att-dot" :title="m.attachments">📎</span>
          <div class="mi-actions">
            <button class="detail-btn" @click="openDetail(m)">详情</button>
            <button class="open-btn" @click="openInGmail(m)" title="在 Gmail 中打开该会话，本人回复">↗</button>
            <button class="read-btn" @click="markRead(m)" title="标为已读，下次拉取不再显示">已读</button>
          </div>
        </div>
        <div class="mi-subject">{{ m.subject || "(无主题)" }}</div>
        <div class="mi-trans">{{ m.translated || "(无机翻中文)" }}</div>
      </article>
    </div>

    <div v-else-if="fetchedAt && !loading" class="empty-state">
      这张表里还没有邮件（脚本可能还没同步，或标签下没有未读）。
    </div>

    <div v-else-if="sources.length === 0" class="empty-state">
      还没有添加邮件表。粘贴 Apps Script 同步生成的 Google Sheet 链接即可开始。
    </div>

    <!-- 详情大卡片：机翻中文在上，原文在下 -->
    <div v-if="selectedMail" class="detail-overlay" @click.self="closeDetail">
      <div class="detail-card">
        <div class="detail-head">
          <div class="detail-meta">
            <span class="from">{{ selectedMail.from || "(未知发件人)" }}</span>
            <span class="ts">{{ selectedMail.date }}</span>
          </div>
          <div class="detail-head-actions">
            <button class="read-btn" @click="markRead(selectedMail)" title="标为已读，下次拉取不再显示">
              标为已读
            </button>
            <button class="web-btn" @click="openInGmail(selectedMail)" title="在 Gmail 中打开该会话，本人回复">
              ↗ 在 Gmail 中打开
            </button>
            <button class="detail-close" @click="closeDetail">✕</button>
          </div>
        </div>
        <div class="detail-subject">{{ selectedMail.subject || "(无主题)" }}</div>
        <div v-if="hasAttachment(selectedMail)" class="detail-att">📎 {{ selectedMail.attachments }}</div>

        <div class="detail-section">
          <div class="detail-label">机翻中文</div>
          <div class="detail-text">{{ selectedMail.translated || "(无机翻中文)" }}</div>
        </div>
        <div class="detail-section">
          <div class="detail-label">原文</div>
          <div class="detail-text orig">{{ selectedMail.body || "(无正文)" }}</div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.gmail-page {
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
}
.profile-row {
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px dashed #ececec;
}
.profile-hint {
  font-size: 11px;
  color: #999;
}
.form-label {
  width: 64px;
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
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
.text-input {
  flex: 1;
  padding: 6px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
}
.label-input {
  width: 200px;
  padding: 6px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
}
.text-input:focus,
.label-input:focus {
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
.icon-btn.danger:hover:not(:disabled) {
  background: #fff5f5;
  color: #c53030;
  border-color: #fed7d7;
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
.fetch-btn:hover {
  background: #5a67d8;
}
.link-btn {
  background: none;
  border: none;
  color: #667eea;
  font-size: 12px;
  cursor: pointer;
  padding: 4px 0;
  flex-shrink: 0;
}
.link-btn:hover {
  text-decoration: underline;
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

.summary-row {
  font-size: 12px;
  color: #666;
  margin-bottom: 8px;
}

.mail-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.mail-item {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 8px 12px;
  background: white;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.mi-row1 {
  display: flex;
  align-items: center;
  gap: 8px;
  white-space: nowrap;
  overflow: hidden;
}
.from {
  font-size: 13px;
  font-weight: 500;
  color: #2d3748;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 320px;
}
.ts {
  font-size: 11px;
  color: #999;
  flex-shrink: 0;
}
.att-dot {
  font-size: 12px;
  flex-shrink: 0;
}
.mi-actions {
  margin-left: auto;
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}
.detail-btn {
  padding: 3px 12px;
  font-size: 12px;
  border: 1px solid #667eea;
  border-radius: 6px;
  background: white;
  color: #667eea;
  cursor: pointer;
}
.detail-btn:hover {
  background: #667eea;
  color: white;
}
.open-btn {
  padding: 3px 10px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.open-btn:hover {
  background: #f5f5fa;
  border-color: #cbd5e0;
}
.read-btn {
  padding: 3px 12px;
  font-size: 12px;
  border: 1px solid #cbd5e0;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.read-btn:hover {
  background: #edf2f7;
  color: #2d3748;
}
.mi-subject {
  font-size: 13px;
  font-weight: 600;
  color: #1a202c;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.mi-trans {
  font-size: 12px;
  color: #718096;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 详情大卡片 */
.detail-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  padding: 20px;
}
.detail-card {
  background: white;
  border-radius: 12px;
  width: 100%;
  max-width: 680px;
  max-height: 88vh;
  overflow-y: auto;
  padding: 18px 20px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.25);
}
.detail-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
}
.detail-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
}
.detail-close {
  border: none;
  background: none;
  color: #999;
  cursor: pointer;
  font-size: 16px;
  width: 28px;
  height: 28px;
  border-radius: 6px;
  flex-shrink: 0;
}
.detail-close:hover {
  background: #edf2f7;
  color: #4a5568;
}
.detail-subject {
  font-size: 15px;
  font-weight: 600;
  color: #1a202c;
  margin: 8px 0 4px;
  word-break: break-word;
}
.detail-att {
  font-size: 11px;
  color: #888;
  word-break: break-all;
  margin-bottom: 8px;
}
.detail-section {
  margin-top: 12px;
}
.detail-label {
  font-size: 11px;
  font-weight: 600;
  color: #2b6cb0;
  margin-bottom: 4px;
}
.detail-text {
  font-size: 13px;
  line-height: 1.6;
  color: #2d3748;
  white-space: pre-wrap;
  word-break: break-word;
  background: #f7fafc;
  border-left: 3px solid #4299e1;
  border-radius: 0 6px 6px 0;
  padding: 8px 12px;
}
.detail-text.orig {
  color: #4a5568;
  border-left-color: #cbd5e0;
  background: #fafafa;
}
.detail-head-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.web-btn {
  padding: 6px 14px;
  font-size: 13px;
  border: 1px solid #667eea;
  border-radius: 6px;
  background: white;
  color: #667eea;
  cursor: pointer;
}
.web-btn:hover {
  background: #667eea;
  color: white;
}

.empty-state {
  padding: 30px 16px;
  text-align: center;
  font-size: 13px;
  color: #999;
}
</style>

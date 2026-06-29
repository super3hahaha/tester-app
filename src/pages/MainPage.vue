<script setup lang="ts">
import { ref, computed, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import SheetsPage from "./SheetsPage.vue";
import SlidesPage from "./SlidesPage.vue";
import GeneratePage from "./GeneratePage.vue";
import ComparePage from "./ComparePage.vue";
import ReviewPage from "./ReviewPage.vue";
import ConfigPage from "./ConfigPage.vue";
import BatchReplyPage from "./BatchReplyPage.vue";
import TemplateManagerPage from "./TemplateManagerPage.vue";
import KnowledgeConfigPage from "./KnowledgeConfigPage.vue";
import KnowledgeBasePage from "./KnowledgeBasePage.vue";
import GmailPage from "./GmailPage.vue";
import AppScriptPage from "./AppScriptPage.vue";
import SettingsPage from "./SettingsPage.vue";
import PromptConfigPage from "./PromptConfigPage.vue";

interface UserInfo {
  email: string;
  name: string;
  picture?: string;
}

defineProps<{ user: UserInfo }>();
const emit = defineEmits<{ (e: "logout"): void }>();

interface SheetSelection {
  spreadsheetId: string;
  spreadsheetName: string;
  tabName: string;
  data: { headers: string[]; rows: string[][]; spreadsheet_url: string };
}

interface KbProduct {
  id: string;
  name: string;
}

interface SubItem {
  id: string;
  label: string;
  icon: string;
}

interface NavItem {
  id: string;
  label: string;
  icon: string;
  children: SubItem[];
  dynamic?: boolean;
}

const kbProducts = ref<KbProduct[]>([]);
const creatingProduct = ref(false);
const newProductName = ref("");

async function reloadKbProducts() {
  try {
    kbProducts.value = await invoke<KbProduct[]>("kb_list_products");
  } catch {}
}

const navItems: NavItem[] = [
  {
    id: "review",
    label: "Review",
    icon: `<svg width="22" height="22" viewBox="0 0 24 24" fill="none"><path d="M4 4h16c.55 0 1 .45 1 1v10c0 .55-.45 1-1 1H8l-4 4V5c0-.55.45-1 1-1z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/><circle cx="8.5" cy="9.5" r="1" fill="currentColor"/><circle cx="12" cy="9.5" r="1" fill="currentColor"/><circle cx="15.5" cy="9.5" r="1" fill="currentColor"/></svg>`,
    children: [
      { id: "review-play", label: "Play Console", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><polygon points="4,3 12,8 4,13" fill="currentColor"/></svg>` },
      { id: "review-config", label: "Config", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M3 4h4M3 8h10M3 12h7" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/><path d="M9 2v4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>` },
      { id: "review-batch-reply", label: "Batch Reply · Run", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><rect x="2" y="3" width="12" height="9" rx="1.5" stroke="currentColor" stroke-width="1.3"/><path d="M5 6h2M5 9h2M9 6h2M9 9h2" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>` },
      { id: "review-templates", label: "GP模板管理", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M2 5c0-.55.45-1 1-1h10c.55 0 1 .45 1 1v7c0 .55-.45 1-1 1H3c-.55 0-1-.45-1-1V5z" stroke="currentColor" stroke-width="1.3"/><path d="M5 4V3.5a.5.5 0 011 0V4M10 4V3.5a.5.5 0 011 0V4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/><path d="M2 7h12" stroke="currentColor" stroke-width="1.2"/><path d="M5.5 10h5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>` },
      { id: "review-knowledge", label: "知识配置", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M3 2h7l3 3v9H3V2z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/><path d="M10 2v3h3" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/><path d="M5 7h6M5 10h4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>` },
    ],
  },
  {
    id: "gmail",
    label: "邮件",
    icon: `<svg width="22" height="22" viewBox="0 0 24 24" fill="none"><rect x="3" y="5" width="18" height="14" rx="2" stroke="currentColor" stroke-width="1.5"/><path d="M3 8l9 6 9-6" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/></svg>`,
    children: [
      { id: "gmail-inbox", label: "Gmail", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><rect x="2" y="3" width="12" height="10" rx="1.5" stroke="currentColor" stroke-width="1.3"/><path d="M2 6l6 4 6-4" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/></svg>` },
      { id: "gmail-appscript", label: "App Script", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M4 2h5l3 3v9H4V2z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/><path d="M9 2v3h3" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/><path d="M6 8l1 1 3-3" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/></svg>` },
      { id: "gmail-templates", label: "Gmail模板管理", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M2 5c0-.55.45-1 1-1h10c.55 0 1 .45 1 1v7c0 .55-.45 1-1 1H3c-.55 0-1-.45-1-1V5z" stroke="currentColor" stroke-width="1.3"/><path d="M5 4V3.5a.5.5 0 011 0V4M10 4V3.5a.5.5 0 011 0V4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/><path d="M2 7h12" stroke="currentColor" stroke-width="1.2"/><path d="M5.5 10h5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>` },
    ],
  },
  {
    id: "testcase",
    label: "Test Case",
    icon: `<svg width="22" height="22" viewBox="0 0 24 24" fill="none"><path d="M9 3h6v2H9V3z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/><rect x="6" y="5" width="12" height="16" rx="1.5" stroke="currentColor" stroke-width="1.5"/><path d="M10 10h4M10 13h4M10 16h2" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/><circle cx="8.5" cy="10" r="1" fill="currentColor"/><circle cx="8.5" cy="13" r="1" fill="currentColor"/><circle cx="8.5" cy="16" r="1" fill="currentColor"/></svg>`,
    children: [
      { id: "sheets", label: "Google Sheets", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><rect x="2" y="2" width="12" height="12" rx="1.5" stroke="currentColor" stroke-width="1.3"/><path d="M2 6h12M2 10h12M6 2v12M10 2v12" stroke="currentColor" stroke-width="1.1"/></svg>` },
      { id: "slides", label: "Google Slides", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><rect x="2" y="2" width="12" height="12" rx="1.5" stroke="currentColor" stroke-width="1.3"/><path d="M5 6h6M5 8.5h4M5 11h5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>` },
      { id: "generate", label: "Generate", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M2 14l5-5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/><path d="M8 2l.5 1.5L10 4l-1.5.5L8 6l-.5-1.5L6 4l1.5-.5L8 2z" stroke="currentColor" stroke-width="1" fill="currentColor"/><path d="M13 8l.35 1.05 1.05.35-1.05.35L13 10.8l-.35-1.05-1.05-.35 1.05-.35L13 8z" fill="currentColor"/></svg>` },
      { id: "compare", label: "compare", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><rect x="1.5" y="4.5" width="5" height="7" rx="1" stroke="currentColor" stroke-width="1.2"/><rect x="9.5" y="4.5" width="5" height="7" rx="1" stroke="currentColor" stroke-width="1.2"/><path d="M7 8h2" stroke="currentColor" stroke-width="1.1" stroke-linecap="round"/><path d="M6.5 6.5L8 8l-1.5 1.5M9.5 6.5L8 8l1.5 1.5" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" stroke-linejoin="round"/></svg>` },
    ],
  },
  {
    id: "knowledge",
    label: "知识库",
    icon: `<svg width="22" height="22" viewBox="0 0 24 24" fill="none"><path d="M4 19.5A2.5 2.5 0 016.5 17H20" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 014 19.5v-15A2.5 2.5 0 016.5 2z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/><path d="M9 7h6M9 11h4" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/></svg>`,
    dynamic: true,
    children: [],
  },
  {
    id: "settings",
    label: "Settings",
    icon: `<svg width="22" height="22" viewBox="0 0 24 24" fill="none"><path d="M12 15a3 3 0 100-6 3 3 0 000 6z" stroke="currentColor" stroke-width="1.5"/><path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" stroke="currentColor" stroke-width="1.5"/></svg>`,
    children: [
      { id: "settings-general", label: "General", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><circle cx="8" cy="8" r="2.5" stroke="currentColor" stroke-width="1.3"/><path d="M8 2v1.5M8 12.5V14M14 8h-1.5M3.5 8H2M11.95 4.05l-1.06 1.06M5.11 10.89l-1.06 1.06M11.95 11.95l-1.06-1.06M5.11 5.11L4.05 4.05" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>` },
      { id: "settings-prompt", label: "Prompt", icon: `<svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M10.5 2.5l3 3-7 7H3.5V9.5l7-7z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/><path d="M8.5 4.5l3 3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>` },
    ],
  },
];

interface SlidesSelection {
  id: string;
  name: string;
  pages: number[];
}

const activeWorkspace = ref("review");
const activeOption = ref("review-play");
const sheetSelection = ref<SheetSelection | null>(null);
const slidesSelection = ref<SlidesSelection[]>([]);

async function handleLogout() {
  await invoke("logout");
  emit("logout");
}

async function selectWorkspace(ws: NavItem) {
  activeWorkspace.value = ws.id;
  if (ws.id === "knowledge") {
    await reloadKbProducts();
    activeOption.value = "kb-view:common";
  } else if (ws.children.length > 0) {
    activeOption.value = ws.children[0].id;
  }
}

function startCreateProduct() {
  creatingProduct.value = true;
  newProductName.value = "";
  nextTick(() => {
    (document.getElementById("new-product-input") as HTMLInputElement)?.focus();
  });
}

async function finishCreateProduct() {
  const name = newProductName.value.trim();
  creatingProduct.value = false;
  if (!name) return;
  try {
    await invoke("kb_create_product", { name });
    const prods = await invoke<KbProduct[]>("kb_list_products");
    kbProducts.value = prods;
    const last = prods[prods.length - 1];
    if (last) activeOption.value = `kb-view:${last.id}`;
  } catch (e) {
    console.error(e);
  }
}

const kbViewId = computed(() => {
  if (activeOption.value.startsWith("kb-view:")) {
    return activeOption.value.slice("kb-view:".length);
  }
  return "";
});

function onSheetSelect(sel: SheetSelection) {
  sheetSelection.value = sel;
}

function onSheetClear() {
  sheetSelection.value = null;
}

function onSlidesSelect(files: SlidesSelection[]) {
  slidesSelection.value = files;
}
</script>

<template>
  <div class="main-page">
    <header class="topbar">
      <span class="app-title">Tester App</span>
      <div class="user-section">
        <img
          v-if="user.picture"
          :src="user.picture"
          class="avatar"
          referrerpolicy="no-referrer"
        />
        <span class="user-email">{{ user.email }}</span>
        <button class="logout-btn" @click="handleLogout">Logout</button>
      </div>
    </header>
    <div class="body">
      <!-- Level 1: Workspace sidebar -->
      <nav class="workspace-bar">
        <div
          v-for="ws in navItems"
          :key="ws.id"
          class="ws-item"
          :class="{ active: activeWorkspace === ws.id }"
          @click="selectWorkspace(ws)"
        >
          <span class="ws-icon" v-html="ws.icon"></span>
          <span class="ws-label">{{ ws.label }}</span>
        </div>
      </nav>

      <!-- Level 2: Options sidebar -->
      <nav
        v-if="activeWorkspace !== 'knowledge' && navItems.find((w) => w.id === activeWorkspace)?.children.length"
        class="options-bar"
      >
        <div
          v-for="opt in navItems.find((w) => w.id === activeWorkspace)?.children"
          :key="opt.id"
          class="opt-item"
          :class="{ active: activeOption === opt.id }"
          @click="activeOption = opt.id"
        >
          <span class="opt-icon" v-html="opt.icon"></span>
          <span class="opt-label">{{ opt.label }}</span>
          <span v-if="opt.id === 'sheets' && sheetSelection" class="opt-badge">1</span>
          <span v-if="opt.id === 'slides' && slidesSelection.length > 0" class="opt-badge">{{ slidesSelection.length }}</span>
        </div>
      </nav>

      <!-- 知识库二级侧栏（动态） -->
      <nav v-if="activeWorkspace === 'knowledge'" class="options-bar">
        <div
          class="opt-item"
          :class="{ active: activeOption === 'kb-view:common' }"
          @click="activeOption = 'kb-view:common'"
        >
          <span class="opt-icon"><svg width="14" height="14" viewBox="0 0 16 16" fill="none"><circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.3"/><path d="M5.5 8h5M8 5.5v5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg></span>
          <span class="opt-label">通用</span>
        </div>
        <div
          v-for="p in kbProducts"
          :key="p.id"
          class="opt-item"
          :class="{ active: activeOption === `kb-view:${p.id}` }"
          @click="activeOption = `kb-view:${p.id}`"
        >
          <span class="opt-icon"><svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M3 2h7l3 3v9H3V2z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/><path d="M10 2v3h3" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/></svg></span>
          <span class="opt-label">{{ p.name }}</span>
        </div>
        <template v-if="creatingProduct">
          <div class="opt-item opt-creating">
            <input
              id="new-product-input"
              v-model="newProductName"
              class="product-create-input"
              placeholder="产品名称"
              @keydown.enter="finishCreateProduct"
              @keydown.escape="creatingProduct = false"
              @blur="finishCreateProduct"
            />
          </div>
        </template>
        <div v-else class="opt-item opt-add" @click="startCreateProduct">
          <span class="opt-icon"><svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M8 3v10M3 8h10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg></span>
          <span class="opt-label">新建产品</span>
        </div>
      </nav>

      <!-- Content -->
      <div class="page-content">
        <SheetsPage
          v-show="activeOption === 'sheets'"
          @select="onSheetSelect"
          @clear="onSheetClear"
        />
        <SlidesPage
          v-show="activeOption === 'slides'"
          @select="onSlidesSelect"
        />
        <GeneratePage
          v-show="activeOption === 'generate'"
          :sheetSelection="sheetSelection"
          :slidesSelection="slidesSelection"
          :activeOption="activeOption"
        />
        <ComparePage
          v-show="activeOption === 'compare'"
        />
        <ReviewPage
          v-show="activeOption === 'review-play'"
        />
        <ConfigPage
          v-show="activeOption === 'review-config'"
        />
        <BatchReplyPage
          v-show="activeOption === 'review-batch-reply'"
          :active-option="activeOption"
        />
        <TemplateManagerPage
          v-show="activeOption === 'review-templates'"
          :active-option="activeOption"
        />
        <KnowledgeConfigPage
          v-show="activeOption === 'review-knowledge'"
          :active-option="activeOption"
        />
        <GmailPage
          v-show="activeOption === 'gmail-inbox'"
        />
        <AppScriptPage
          v-show="activeOption === 'gmail-appscript'"
        />
        <TemplateManagerPage
          v-show="activeOption === 'gmail-templates'"
          :active-option="activeOption"
          trigger-option="gmail-templates"
          namespace="email"
        />
        <SettingsPage
          v-show="activeOption === 'settings-general'"
        />
        <PromptConfigPage
          v-show="activeOption === 'settings-prompt'"
        />
        <KnowledgeBasePage
          v-show="activeOption.startsWith('kb-view:')"
          :view-id="kbViewId"
          :active-option="activeOption"
          @products-changed="reloadKbProducts"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.main-page {
  height: 100vh;
  display: flex;
  flex-direction: column;
}
.topbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 10px 20px;
  background: white;
  border-bottom: 1px solid #e5e5e5;
}
.app-title {
  font-weight: 600;
  font-size: 16px;
}
.user-section {
  display: flex;
  align-items: center;
  gap: 10px;
}
.avatar {
  width: 28px;
  height: 28px;
  border-radius: 50%;
}
.user-email {
  font-size: 13px;
  color: #666;
}
.logout-btn {
  padding: 5px 12px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  cursor: pointer;
  color: #666;
}
.logout-btn:hover {
  background: #f5f5f5;
}
.body {
  flex: 1;
  display: flex;
  overflow: hidden;
}

/* Level 1: Workspace */
.workspace-bar {
  width: 72px;
  min-width: 72px;
  background: #1e1e2e;
  display: flex;
  flex-direction: column;
  padding: 8px 0;
}
.ws-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 12px 4px;
  cursor: pointer;
  color: #888;
  gap: 4px;
  border-left: 3px solid transparent;
  transition: all 0.15s;
}
.ws-item:hover {
  color: #ccc;
  background: rgba(255, 255, 255, 0.05);
}
.ws-item.active {
  color: white;
  background: rgba(255, 255, 255, 0.08);
  border-left-color: #667eea;
}
.ws-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
}
.ws-label {
  font-size: 10px;
  text-align: center;
  line-height: 1.2;
}

/* Level 2: Options */
.options-bar {
  width: 168px;
  min-width: 168px;
  background: #2a2a3a;
  display: flex;
  flex-direction: column;
  padding: 8px 0;
}
.opt-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  cursor: pointer;
  color: #999;
  font-size: 13px;
  transition: all 0.15s;
}
.opt-item:hover {
  color: #ddd;
  background: rgba(255, 255, 255, 0.05);
}
.opt-item.active {
  color: white;
  background: rgba(255, 255, 255, 0.1);
}
.opt-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  flex-shrink: 0;
}
.opt-label {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}
.opt-badge {
  background: #667eea;
  color: white;
  font-size: 10px;
  min-width: 16px;
  height: 16px;
  border-radius: 8px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin-left: auto;
  padding: 0 4px;
  flex-shrink: 0;
}

.opt-add {
  border-top: 1px solid rgba(255,255,255,0.06);
  margin-top: 4px;
  padding-top: 12px;
  color: #777;
}
.opt-add:hover { color: #bbb; }

.opt-creating {
  border-top: 1px solid rgba(255,255,255,0.06);
  margin-top: 4px;
  padding-top: 8px;
  padding-bottom: 8px;
}

.product-create-input {
  width: 100%;
  box-sizing: border-box;
  background: rgba(255,255,255,0.1);
  border: 1px solid rgba(255,255,255,0.3);
  border-radius: 5px;
  color: white;
  font-size: 12px;
  padding: 4px 8px;
  outline: none;
}
.product-create-input::placeholder { color: rgba(255,255,255,0.4); }
.product-create-input:focus { border-color: #667eea; }

.page-content {
  flex: 1;
  overflow: hidden;
}
.placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: #bbb;
  font-size: 15px;
}
</style>

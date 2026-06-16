<script setup lang="ts">
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import SheetsPage from "./SheetsPage.vue";
import SlidesPage from "./SlidesPage.vue";
import GeneratePage from "./GeneratePage.vue";
import ComparePage from "./ComparePage.vue";
import ReviewPage from "./ReviewPage.vue";
import BatchReplyConfigPage from "./BatchReplyConfigPage.vue";
import BatchReplyPage from "./BatchReplyPage.vue";
import TemplateManagerPage from "./TemplateManagerPage.vue";
import SettingsPage from "./SettingsPage.vue";

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
}

const navItems: NavItem[] = [
  {
    id: "testcase",
    label: "Test Case",
    icon: "🧪",
    children: [
      { id: "sheets", label: "Google Sheets", icon: "📊" },
      { id: "slides", label: "Google Slides", icon: "📑" },
      { id: "generate", label: "Generate", icon: "▶" },
      { id: "compare", label: "compare", icon: "🔀" },
    ],
  },
  {
    id: "review",
    label: "Review",
    icon: "💬",
    children: [
      { id: "review-play", label: "Play Console", icon: "▶" },
      { id: "review-batch-reply-config", label: "Batch Reply · Config", icon: "⚙" },
      { id: "review-batch-reply", label: "Batch Reply · Run", icon: "🤖" },
      { id: "review-templates", label: "模板管理", icon: "🗂" },
    ],
  },
  {
    id: "settings",
    label: "Settings",
    icon: "⚙",
    children: [
      { id: "settings-general", label: "General", icon: "⚙" },
    ],
  },
];

interface SlidesSelection {
  id: string;
  name: string;
  pages: number[];
}

const activeWorkspace = ref("testcase");
const activeOption = ref("sheets");
const sheetSelection = ref<SheetSelection | null>(null);
const slidesSelection = ref<SlidesSelection[]>([]);

async function handleLogout() {
  await invoke("logout");
  emit("logout");
}

function selectWorkspace(ws: NavItem) {
  activeWorkspace.value = ws.id;
  if (ws.children.length > 0) {
    activeOption.value = ws.children[0].id;
  }
}

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
          <span class="ws-icon">{{ ws.icon }}</span>
          <span class="ws-label">{{ ws.label }}</span>
        </div>
      </nav>

      <!-- Level 2: Options sidebar -->
      <nav
        v-if="navItems.find((w) => w.id === activeWorkspace)?.children.length"
        class="options-bar"
      >
        <div
          v-for="opt in navItems.find((w) => w.id === activeWorkspace)?.children"
          :key="opt.id"
          class="opt-item"
          :class="{ active: activeOption === opt.id }"
          @click="activeOption = opt.id"
        >
          <span class="opt-icon">{{ opt.icon }}</span>
          <span class="opt-label">{{ opt.label }}</span>
          <span v-if="opt.id === 'sheets' && sheetSelection" class="opt-badge">1</span>
          <span v-if="opt.id === 'slides' && slidesSelection.length > 0" class="opt-badge">{{ slidesSelection.length }}</span>
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
        />
        <ComparePage
          v-show="activeOption === 'compare'"
        />
        <ReviewPage
          v-show="activeOption === 'review-play'"
        />
        <BatchReplyConfigPage
          v-show="activeOption === 'review-batch-reply-config'"
        />
        <BatchReplyPage
          v-show="activeOption === 'review-batch-reply'"
          :active-option="activeOption"
        />
        <TemplateManagerPage
          v-show="activeOption === 'review-templates'"
          :active-option="activeOption"
        />
        <SettingsPage
          v-show="activeOption === 'settings-general'"
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
  font-size: 22px;
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
  font-size: 16px;
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

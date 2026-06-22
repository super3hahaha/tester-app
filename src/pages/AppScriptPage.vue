<script setup lang="ts">
import { ref, onMounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

// Apps Script 项目管理：维护一组待打开/维护的 Apps Script 项目（通常每个对应一个 Gmail
// 账号的 gmail-sync 脚本）。每个项目可配「用哪个 Chrome profile 打开」+「跳转链接」，
// 点「打开」就用所选浏览器跳到该链接（默认 Apps Script 首页），方便用登录了对应账号的
// 窗口去新建/编辑脚本。纯本地配置，不碰任何 Google API。

interface AppScriptProject {
  id: string;
  label: string; // 备注（账号邮箱等）
  profileDir?: string; // 用哪个 Chrome profile 打开（目录名；空=系统默认浏览器）
  url: string; // 跳转链接（默认 Apps Script 首页）
}

interface ChromeProfile {
  dir: string;
  name: string;
}

const STORAGE_KEY = "appscript-projects-v1";
const DEFAULT_URL = "https://script.google.com/home?hl=zh-cn";

const projects = ref<AppScriptProject[]>([]);
const chromeProfiles = ref<ChromeProfile[]>([]);
const errorMsg = ref("");

// 添加表单
const adding = ref(false);
const newLabel = ref("");
const newProfile = ref("");
const newUrl = ref(DEFAULT_URL);

function genId(): string {
  return `as_${Date.now().toString(36)}_${Math.floor(Math.random() * 1e6).toString(36)}`;
}

onMounted(() => {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) projects.value = JSON.parse(raw);
  } catch {
    projects.value = [];
  }
  loadChromeProfiles();
  if (projects.value.length === 0) adding.value = true;
});

async function loadChromeProfiles() {
  try {
    chromeProfiles.value = await invoke<ChromeProfile[]>("list_chrome_profiles");
  } catch {
    chromeProfiles.value = [];
  }
}

watch(
  projects,
  () => localStorage.setItem(STORAGE_KEY, JSON.stringify(projects.value)),
  { deep: true }
);

function startAdd() {
  adding.value = true;
  newLabel.value = "";
  newProfile.value = "";
  newUrl.value = DEFAULT_URL;
  errorMsg.value = "";
}

function addProject() {
  const label = newLabel.value.trim() || `项目 ${projects.value.length + 1}`;
  const url = newUrl.value.trim() || DEFAULT_URL;
  projects.value.push({
    id: genId(),
    label,
    profileDir: newProfile.value || undefined,
    url,
  });
  cancelAdd();
}

function cancelAdd() {
  adding.value = false;
  newLabel.value = "";
  newProfile.value = "";
  newUrl.value = DEFAULT_URL;
  errorMsg.value = "";
}

function removeProject(id: string) {
  projects.value = projects.value.filter((p) => p.id !== id);
  if (projects.value.length === 0) adding.value = true;
}

async function openProject(p: AppScriptProject) {
  const url = (p.url || "").trim() || DEFAULT_URL;
  errorMsg.value = "";
  try {
    if (p.profileDir) {
      await invoke("open_url_in_chrome_profile", { url, profileDir: p.profileDir });
    } else {
      await openUrl(url);
    }
  } catch (e: any) {
    errorMsg.value = "打开失败：" + String(e);
  }
}
</script>

<template>
  <div class="appscript-page">
    <header class="page-header">
      <h3>App Script 项目</h3>
      <p class="subtitle">
        维护要打开/维护的 Apps Script 项目（一般每个对应一个 Gmail 账号的同步脚本）。设好用哪个
        Chrome 资料打开，点「打开 ↗」就跳到 Apps Script（默认首页），用登录了该账号的窗口新建或编辑脚本。
      </p>
    </header>

    <div v-if="errorMsg" class="banner banner-error">{{ errorMsg }}</div>

    <section class="list-card" v-if="projects.length > 0">
      <article v-for="p in projects" :key="p.id" class="project-item">
        <div class="pi-row">
          <input v-model="p.label" class="label-input" placeholder="备注（账号邮箱，可选）" />
          <button class="open-btn" @click="openProject(p)" title="用所选浏览器打开该链接">打开 ↗</button>
          <button class="icon-btn danger" @click="removeProject(p.id)" title="删除该项目">✕</button>
        </div>
        <div class="pi-row sub">
          <label class="mini-label">浏览器</label>
          <select v-model="p.profileDir" class="src-select" title="用哪个 Chrome 个人资料打开">
            <option :value="undefined">系统默认浏览器</option>
            <option v-for="cp in chromeProfiles" :key="cp.dir" :value="cp.dir">
              Chrome · {{ cp.name }}
            </option>
          </select>
        </div>
        <div class="pi-row sub">
          <label class="mini-label">链接</label>
          <input v-model="p.url" class="text-input" :placeholder="DEFAULT_URL" />
        </div>
      </article>
    </section>

    <section class="form-card" v-if="adding">
      <div class="form-title">新增项目</div>
      <div class="form-row">
        <label class="form-label">备注</label>
        <input
          v-model="newLabel"
          class="text-input"
          placeholder="账号邮箱或备注（可选）"
          @keyup.enter="addProject"
        />
      </div>
      <div class="form-row">
        <label class="form-label">浏览器</label>
        <select v-model="newProfile" class="src-select" title="用哪个 Chrome 个人资料打开">
          <option value="">系统默认浏览器</option>
          <option v-for="cp in chromeProfiles" :key="cp.dir" :value="cp.dir">
            Chrome · {{ cp.name }}
          </option>
        </select>
      </div>
      <div class="form-row">
        <label class="form-label">链接</label>
        <input v-model="newUrl" class="text-input" :placeholder="DEFAULT_URL" />
      </div>
      <div class="form-foot">
        <button class="fetch-btn" @click="addProject">添加</button>
        <button class="icon-btn" v-if="projects.length > 0" @click="cancelAdd" title="取消">取消</button>
      </div>
    </section>

    <div class="add-bar" v-if="!adding">
      <button class="add-btn" @click="startAdd">＋ 新增项目</button>
    </div>
  </div>
</template>

<style scoped>
.appscript-page {
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

.list-card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-bottom: 12px;
}
.project-item {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 10px 12px;
  background: white;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.pi-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.pi-row.sub {
  padding-left: 2px;
}
.mini-label {
  width: 48px;
  flex-shrink: 0;
  font-size: 12px;
  color: #718096;
}

.form-card {
  background: #fafafa;
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 14px 16px;
  margin-bottom: 12px;
}
.form-title {
  font-size: 13px;
  font-weight: 600;
  color: #4a5568;
  margin-bottom: 10px;
}
.form-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.form-label {
  width: 56px;
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
}
.form-foot {
  display: flex;
  gap: 8px;
  margin-top: 4px;
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
.text-input,
.label-input {
  flex: 1;
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
.icon-btn:hover {
  background: #f5f5fa;
  color: #333;
}
.icon-btn.danger:hover {
  background: #fff5f5;
  color: #c53030;
  border-color: #fed7d7;
}
.open-btn {
  padding: 5px 14px;
  font-size: 13px;
  border: 1px solid #667eea;
  border-radius: 6px;
  background: white;
  color: #667eea;
  cursor: pointer;
  flex-shrink: 0;
}
.open-btn:hover {
  background: #667eea;
  color: white;
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
}
.fetch-btn:hover {
  background: #5a67d8;
}

.add-bar {
  margin-top: 4px;
}
.add-btn {
  padding: 8px 16px;
  font-size: 13px;
  border: 1px dashed #cbd5e0;
  border-radius: 8px;
  background: white;
  color: #4a5568;
  cursor: pointer;
  width: 100%;
}
.add-btn:hover {
  border-color: #667eea;
  color: #667eea;
  background: #f7f8ff;
}
</style>

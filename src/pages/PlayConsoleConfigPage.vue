<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  type DatePreset,
  computeRange,
  PRESET_LABELS,
} from "../utils/batchReplyDates";
import {
  type ReplyState,
  type PlayAppConfig,
  type PlayMultiConfig,
  PLAY_STORAGE_KEY,
  APPS_CACHE_KEY,
  DATE_PRESETS,
  REPLY_STATES,
  REPLY_STATE_LABELS,
  defaultPlayConfig,
  normalizePlayConfig,
} from "../utils/playConsoleConfig";

interface PlayApp {
  package_name: string;
  display_name: string;
}

const apps = ref<PlayApp[]>([]);
const appsLoading = ref(false);
const appsError = ref("");
const needRelogin = ref(false);

const perApp = ref<Record<string, PlayAppConfig>>({});
const savedSnapshot = ref<string>("");
const saveFlash = ref<"" | "saved" | "error" | "cleared">("");
const saveError = ref("");
const searchQ = ref("");

// 每分钟 tick 一次，让「实际范围」预览跨午夜自动重算。
const now = ref(new Date());
let nowTimer: number | undefined;

onMounted(() => {
  const raw = localStorage.getItem(PLAY_STORAGE_KEY);
  if (raw) {
    try {
      const cfg = JSON.parse(raw) as PlayMultiConfig;
      if (cfg.perApp && typeof cfg.perApp === "object") {
        const normalized: Record<string, PlayAppConfig> = {};
        for (const [pkg, entry] of Object.entries(cfg.perApp)) {
          normalized[pkg] = normalizePlayConfig(entry);
        }
        perApp.value = normalized;
      }
    } catch {
      // ignore
    }
  }
  savedSnapshot.value = JSON.stringify(perApp.value);

  const cachedApps = localStorage.getItem(APPS_CACHE_KEY);
  if (cachedApps) {
    try {
      apps.value = JSON.parse(cachedApps) as PlayApp[];
    } catch {
      // ignore
    }
  }
  loadApps();

  nowTimer = window.setInterval(() => {
    now.value = new Date();
  }, 60_000);
});

onUnmounted(() => {
  if (nowTimer !== undefined) window.clearInterval(nowTimer);
});

async function loadApps() {
  appsLoading.value = true;
  appsError.value = "";
  try {
    const list = await invoke<PlayApp[]>("list_play_apps");
    apps.value = list;
    localStorage.setItem(APPS_CACHE_KEY, JSON.stringify(list));
    for (const a of list) {
      if (!perApp.value[a.package_name]) {
        perApp.value[a.package_name] = defaultPlayConfig();
      }
    }
  } catch (e: any) {
    const msg = String(e);
    if (msg.startsWith("NEED_RELOGIN_SCOPE")) {
      appsError.value = "需要重新登录授权 playdeveloperreporting 权限。";
      needRelogin.value = true;
    } else {
      appsError.value = msg;
    }
  } finally {
    appsLoading.value = false;
  }
}

function ensureCfg(pkg: string): PlayAppConfig {
  if (!perApp.value[pkg]) {
    perApp.value[pkg] = defaultPlayConfig();
  }
  return perApp.value[pkg];
}

function setPreset(pkg: string, preset: DatePreset) {
  ensureCfg(pkg).datePreset = preset;
}

function setReplyState(pkg: string, rs: ReplyState) {
  ensureCfg(pkg).replyState = rs;
}

function toggleStar(pkg: string, s: number) {
  const cfg = ensureCfg(pkg);
  const i = cfg.stars.indexOf(s);
  if (i >= 0) cfg.stars.splice(i, 1);
  else cfg.stars.push(s);
  cfg.stars.sort();
}

function selectAll() {
  for (const a of apps.value) ensureCfg(a.package_name).enabled = true;
}
function unselectAll() {
  for (const a of apps.value) ensureCfg(a.package_name).enabled = false;
}

// 内联两步确认（window.confirm 在 Tauri webview 不弹，会卡住操作）
const resetArmed = ref(false);
let resetTimer: number | undefined;

function resetAll() {
  if (!resetArmed.value) {
    resetArmed.value = true;
    if (resetTimer) clearTimeout(resetTimer);
    resetTimer = window.setTimeout(() => { resetArmed.value = false; }, 4000);
    return;
  }
  if (resetTimer) clearTimeout(resetTimer);
  resetArmed.value = false;
  localStorage.removeItem(PLAY_STORAGE_KEY);
  const fresh: Record<string, PlayAppConfig> = {};
  for (const a of apps.value) fresh[a.package_name] = defaultPlayConfig();
  perApp.value = fresh;
  savedSnapshot.value = JSON.stringify(perApp.value);
  saveFlash.value = "cleared";
  setTimeout(() => {
    if (saveFlash.value === "cleared") saveFlash.value = "";
  }, 2500);
}

const enabledCount = computed(() =>
  apps.value.filter((a) => perApp.value[a.package_name]?.enabled).length
);

const filteredApps = computed(() => {
  const q = searchQ.value.trim().toLowerCase();
  if (!q) return apps.value;
  return apps.value.filter(
    (a) =>
      a.display_name.toLowerCase().includes(q) ||
      a.package_name.toLowerCase().includes(q)
  );
});

const currentSnapshot = computed(() => JSON.stringify(perApp.value));
const isDirty = computed(() => currentSnapshot.value !== savedSnapshot.value);

watch(currentSnapshot, () => {
  if (saveFlash.value === "saved") saveFlash.value = "";
});

function handleSave() {
  saveError.value = "";
  try {
    const knownPkgs = new Set(apps.value.map((a) => a.package_name));
    const pruned: Record<string, PlayAppConfig> = {};
    for (const [pkg, cfg] of Object.entries(perApp.value)) {
      if (knownPkgs.has(pkg) || cfg.enabled) pruned[pkg] = cfg;
    }
    perApp.value = pruned;
    const payload: PlayMultiConfig = { perApp: perApp.value };
    localStorage.setItem(PLAY_STORAGE_KEY, JSON.stringify(payload));
    savedSnapshot.value = JSON.stringify(perApp.value);
    saveFlash.value = "saved";
    setTimeout(() => {
      if (saveFlash.value === "saved") saveFlash.value = "";
    }, 2500);
  } catch (e: any) {
    saveFlash.value = "error";
    saveError.value = String(e);
  }
}

function rangePreview(cfg: PlayAppConfig): string {
  const r = computeRange(
    cfg.datePreset,
    { fromDate: cfg.customFromDate, toDate: cfg.customToDate },
    now.value,
  );
  return r.fromDate === r.toDate ? r.fromDate : `${r.fromDate} → ${r.toDate}`;
}

function validateConfig(cfg: PlayAppConfig): string {
  if (cfg.replyState !== "UPDATED" && cfg.stars.length === 0) return "未选择星级";
  return "";
}
</script>

<template>
  <div class="config-page">
    <header class="page-header">
      <h3>Play Console · 拉取配置</h3>
      <p class="subtitle">
        勾选要在 <b>Play Console</b> 页拉取的应用，并为每个应用配置默认日期范围（动态预设，执行时按当天计算）、星级与回复状态。保存后到 <b>Play Console</b> 页点「拉取评论」即按此并行拉取（页面内仍可临时调整筛选）。
      </p>
    </header>

    <div class="toolbar">
      <input
        v-model="searchQ"
        class="search-input"
        type="text"
        placeholder="搜索应用名 / 包名"
      />
      <div class="toolbar-spacer"></div>
      <button class="link-btn" @click="selectAll" :disabled="apps.length === 0">全部勾选</button>
      <button class="link-btn" @click="unselectAll" :disabled="apps.length === 0">全部取消</button>
      <button class="refresh-btn" @click="loadApps" :disabled="appsLoading" title="刷新应用列表">
        ↻ {{ appsLoading ? "加载中" : "刷新" }}
      </button>
      <button
        class="reset-btn"
        :class="{ armed: resetArmed }"
        @click="resetAll"
        :title="resetArmed ? '再点一次确认清空（4 秒内）' : '清空所有保存的 Play Console 拉取配置，恢复出厂默认'"
      >
        {{ resetArmed ? "再点一次确认" : "清空配置" }}
      </button>
      <button class="save-btn" @click="handleSave" :disabled="!isDirty">
        {{ isDirty ? "保存配置" : "已保存" }}
      </button>
    </div>

    <div class="status-row">
      <span class="status-text">
        已启用 <b>{{ enabledCount }}</b> / {{ apps.length }} 个应用
        <span v-if="isDirty" class="dirty-tag">· 有未保存改动</span>
      </span>
      <span v-if="saveFlash === 'saved'" class="flash flash-ok">✓ 已保存到本地</span>
      <span v-if="saveFlash === 'cleared'" class="flash flash-ok">✓ 配置已清空，恢复默认</span>
      <span v-if="saveFlash === 'error'" class="flash flash-err">保存失败：{{ saveError }}</span>
    </div>

    <div v-if="needRelogin" class="banner banner-warn">
      ⚠️ 当前登录态不包含 Play 相关权限。请点右上角 <b>Logout</b> 后重新登录。
    </div>
    <div v-else-if="appsError" class="banner banner-error">应用列表加载失败：{{ appsError }}</div>

    <div v-if="apps.length === 0 && !appsLoading" class="empty-state">
      暂无应用。点上方「刷新」重新拉取。
    </div>

    <div v-else class="app-grid">
      <article
        v-for="a in filteredApps"
        :key="a.package_name"
        class="app-card"
        :class="{ disabled: !perApp[a.package_name]?.enabled }"
      >
        <header class="app-card-head">
          <label class="enable-toggle">
            <input
              type="checkbox"
              :checked="!!perApp[a.package_name]?.enabled"
              @change="ensureCfg(a.package_name).enabled = ($event.target as HTMLInputElement).checked"
            />
            <span class="app-name">{{ a.display_name }}</span>
          </label>
          <span class="pkg-name">{{ a.package_name }}</span>
        </header>

        <div class="cfg-row">
          <label class="cfg-label">日期</label>
          <div class="preset-row">
            <button
              v-for="p in DATE_PRESETS"
              :key="p"
              class="preset-btn"
              :class="{ active: ensureCfg(a.package_name).datePreset === p }"
              :disabled="!perApp[a.package_name]?.enabled"
              @click="setPreset(a.package_name, p)"
            >{{ PRESET_LABELS[p] }}</button>
          </div>
        </div>

        <div class="cfg-row indent preview-row">
          <label class="cfg-label"></label>
          <span class="preview-text">
            实际范围：<b>{{ rangePreview(ensureCfg(a.package_name)) }}</b>
          </span>
        </div>

        <div class="cfg-row">
          <label class="cfg-label">评分</label>
          <div class="star-row">
            <button
              v-for="s in [1, 2, 3, 4, 5]"
              :key="s"
              class="star-btn"
              :class="{ active: ensureCfg(a.package_name).stars.includes(s) }"
              :disabled="!perApp[a.package_name]?.enabled || ensureCfg(a.package_name).replyState === 'UPDATED'"
              @click="toggleStar(a.package_name, s)"
            >{{ s }} ★</button>
            <span
              v-if="perApp[a.package_name]?.enabled && ensureCfg(a.package_name).replyState === 'UPDATED'"
              class="star-hint"
            >「回复后又更新」忽略星级</span>
          </div>
        </div>

        <div class="cfg-row">
          <label class="cfg-label">状态</label>
          <div class="state-row">
            <button
              v-for="rs in REPLY_STATES"
              :key="rs"
              class="state-btn"
              :class="{ active: ensureCfg(a.package_name).replyState === rs }"
              :disabled="!perApp[a.package_name]?.enabled"
              @click="setReplyState(a.package_name, rs)"
            >{{ REPLY_STATE_LABELS[rs] }}</button>
          </div>
        </div>

        <div
          v-if="perApp[a.package_name]?.enabled && validateConfig(perApp[a.package_name])"
          class="error small"
        >{{ validateConfig(perApp[a.package_name]) }}</div>
      </article>
    </div>
  </div>
</template>

<style scoped>
.config-page {
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
  margin: 4px 0 12px 0;
  font-size: 12px;
  color: #888;
  line-height: 1.5;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  background: #fafafa;
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  margin-bottom: 8px;
  flex-wrap: wrap;
}
.toolbar-spacer { flex: 1; min-width: 8px; }
.search-input {
  flex: 1;
  min-width: 200px;
  max-width: 320px;
  padding: 6px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
}
.search-input:focus { border-color: #667eea; }
.link-btn {
  background: none;
  border: none;
  color: #667eea;
  font-size: 12px;
  cursor: pointer;
  padding: 4px 6px;
}
.link-btn:hover:not(:disabled) { text-decoration: underline; }
.link-btn:disabled { color: #aaa; cursor: not-allowed; }
.refresh-btn {
  padding: 5px 12px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #666;
  cursor: pointer;
}
.refresh-btn:hover:not(:disabled) { background: #f5f5fa; color: #333; }
.refresh-btn:disabled { opacity: 0.5; cursor: not-allowed; }
.save-btn {
  padding: 6px 18px;
  font-size: 13px;
  font-weight: 600;
  border: none;
  border-radius: 6px;
  background: #38a169;
  color: white;
  cursor: pointer;
}
.save-btn:hover:not(:disabled) { background: #2f855a; }
.save-btn:disabled { background: #cbd5e0; cursor: default; }
.reset-btn {
  padding: 5px 12px;
  font-size: 12px;
  border: 1px solid #fbb6b6;
  border-radius: 6px;
  background: white;
  color: #c53030;
  cursor: pointer;
}
.reset-btn:hover { background: #fff5f5; border-color: #f56565; }
.reset-btn.armed { background: #e53e3e; border-color: #e53e3e; color: white; font-weight: 600; }

.status-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 12px;
  font-size: 12px;
  color: #666;
}
.dirty-tag { color: #c05621; margin-left: 4px; font-weight: 600; }
.flash { font-size: 12px; padding: 2px 10px; border-radius: 10px; }
.flash-ok { background: #c6f6d5; color: #22543d; }
.flash-err { background: #fed7d7; color: #9b2c2c; }

.banner {
  padding: 10px 14px;
  border-radius: 6px;
  font-size: 13px;
  margin-bottom: 12px;
  line-height: 1.5;
}
.banner-warn { background: #fffaf0; border: 1px solid #fbd38d; color: #975a16; }
.banner-error { background: #fff5f5; border: 1px solid #fed7d7; color: #c53030; word-break: break-all; }

.app-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(440px, 1fr));
  gap: 10px;
}
.app-card {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 12px 14px;
  background: white;
  display: flex;
  flex-direction: column;
  gap: 4px;
  transition: opacity 0.15s, background 0.15s;
}
.app-card.disabled { background: #fafafa; opacity: 0.65; }
.app-card-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  padding-bottom: 6px;
  margin-bottom: 4px;
  border-bottom: 1px dashed #ececec;
}
.enable-toggle {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  font-size: 13px;
}
.enable-toggle input[type="checkbox"] { width: 16px; height: 16px; cursor: pointer; }
.app-name { font-weight: 600; color: #2d3748; }
.pkg-name {
  font-size: 11px;
  color: #999;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
}

.cfg-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 0;
  flex-wrap: wrap;
}
.cfg-row.indent { padding-top: 0; }
.cfg-label {
  width: 36px;
  font-size: 11px;
  color: #888;
  flex-shrink: 0;
}
.preset-row, .state-row {
  display: flex;
  gap: 4px;
  flex-wrap: wrap;
}
.preset-btn, .state-btn {
  padding: 3px 10px;
  font-size: 11px;
  border: 1px solid #ddd;
  border-radius: 4px;
  background: white;
  cursor: pointer;
  color: #666;
}
.preset-btn.active {
  background: #667eea;
  border-color: #667eea;
  color: white;
}
.preset-btn:hover:not(:disabled):not(.active),
.state-btn:hover:not(:disabled):not(.active) {
  background: #f5f5fa;
  color: #333;
}
.preset-btn:disabled, .state-btn:disabled { opacity: 0.4; cursor: not-allowed; }
.state-btn.active {
  background: #4299e1;
  border-color: #4299e1;
  color: white;
}

.preview-row { padding-top: 0; padding-bottom: 4px; }
.preview-text {
  font-size: 11px;
  color: #4a5568;
  background: #f7fafc;
  padding: 2px 8px;
  border-radius: 4px;
  border: 1px solid #e2e8f0;
}

.star-row { display: flex; gap: 4px; align-items: center; }
.star-btn {
  padding: 3px 10px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 4px;
  background: white;
  cursor: pointer;
  color: #888;
}
.star-btn.active {
  background: #f6ad55;
  border-color: #f6ad55;
  color: white;
}
.star-btn:disabled { opacity: 0.4; cursor: not-allowed; }
.star-hint { font-size: 11px; color: #b7791f; margin-left: 4px; }

.empty-state {
  padding: 30px 16px;
  text-align: center;
  font-size: 13px;
  color: #999;
}
.error { color: #e53e3e; font-size: 12px; margin-top: 4px; }
.error.small { font-size: 11px; }
</style>

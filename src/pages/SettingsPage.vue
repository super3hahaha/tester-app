<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";

const cacheSize = ref("");
const loading = ref(false);
const clearing = ref(false);
const message = ref("");

interface UpdateInfo {
  version: string;
  asset_name: string;
  asset_url: string;
  asset_size: number;
  body: string;
}

type UpdateState = "idle" | "checking" | "latest" | "available" | "downloading" | "done" | "error";
const updateState = ref<UpdateState>("idle");
const updateInfo = ref<UpdateInfo | null>(null);
const updateError = ref("");
const downloadProgress = ref({ downloaded: 0, total: 0 });
const showUpdateModal = ref(false);

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / Math.pow(1024, i);
  return `${val.toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

const downloadPercent = computed(() => {
  const { downloaded, total } = downloadProgress.value;
  if (!total) return 0;
  return Math.round((downloaded / total) * 100);
});

async function checkUpdate() {
  updateState.value = "checking";
  updateError.value = "";
  updateInfo.value = null;
  try {
    const info = await invoke<UpdateInfo | null>("check_update");
    if (info) {
      updateInfo.value = info;
      updateState.value = "available";
      showUpdateModal.value = true;
    } else {
      updateState.value = "latest";
      setTimeout(() => { updateState.value = "idle"; }, 3000);
    }
  } catch (e: any) {
    updateError.value = String(e);
    updateState.value = "error";
  }
}

async function startDownload() {
  if (!updateInfo.value) return;
  updateState.value = "downloading";
  downloadProgress.value = { downloaded: 0, total: updateInfo.value.asset_size };

  const unlisten = await listen<{ downloaded: number; total: number }>("update-progress", (e) => {
    downloadProgress.value = e.payload;
  });

  try {
    const savePath = await invoke<string>("download_update", {
      url: updateInfo.value.asset_url,
      assetName: updateInfo.value.asset_name,
    });
    unlisten();
    updateState.value = "done";
    await invoke("apply_update", { savePath });
  } catch (e: any) {
    unlisten();
    updateError.value = String(e);
    updateState.value = "error";
  }
}

const MODEL_OPTIONS = [
  { label: "Sonnet 4.6（推荐）", value: "claude-sonnet-4-6" },
  { label: "Haiku 4.5（快速省额）", value: "claude-haiku-4-5" },
];

interface ModelConfig {
  reply: string;
  analysis: string;
  translate: string;
  github_token: string;
}

const modelConfig = ref<ModelConfig>({
  reply: "claude-sonnet-4-6",
  analysis: "claude-sonnet-4-6",
  translate: "claude-haiku-4-5",
  github_token: "",
});
const modelSaving = ref(false);
const modelMessage = ref("");

interface ClaudeAccountInfo {
  installed: boolean;
  cli_path: string | null;
  logged_in: boolean;
  email: string | null;
  subscription: string | null;
}

const claudeInfo = ref<ClaudeAccountInfo | null>(null);
const claudeLoading = ref(false);

interface SkillStatus {
  name: string;
  owner: string;
  repo: string;
  local_version: string | null;
  remote_version: string | null;
  up_to_date: boolean;
  updated: boolean;
  backup_path: string | null;
  error: string | null;
}

const skillStatuses = ref<SkillStatus[]>([]);
const skillChecking = ref(false);
const skillSyncing = ref(false);
const skillMessage = ref("");

function repoUrl(s: SkillStatus): string {
  return `https://github.com/${s.owner}/${s.repo}`;
}

function releasesUrl(s: SkillStatus): string {
  return `https://github.com/${s.owner}/${s.repo}/releases`;
}

const claudeStatus = computed<"ok" | "no-login" | "no-install" | "unknown">(() => {
  const info = claudeInfo.value;
  if (!info) return "unknown";
  if (!info.installed) return "no-install";
  if (!info.logged_in) return "no-login";
  return "ok";
});

onMounted(() => {
  refreshCacheSize();
  refreshClaude();
  refreshSkillStatuses();
  loadModelConfig();
});

async function loadModelConfig() {
  try {
    modelConfig.value = await invoke<ModelConfig>("get_model_config");
  } catch {
    // use defaults
  }
}

async function saveModelConfig() {
  modelSaving.value = true;
  modelMessage.value = "";
  try {
    await invoke("save_model_config", { config: modelConfig.value });
    modelMessage.value = "已保存";
    setTimeout(() => { modelMessage.value = ""; }, 2000);
  } catch (e: any) {
    modelMessage.value = "保存失败：" + String(e);
  } finally {
    modelSaving.value = false;
  }
}

async function refreshSkillStatuses() {
  skillChecking.value = true;
  skillMessage.value = "";
  try {
    skillStatuses.value = await invoke<SkillStatus[]>("check_skill_updates");
  } catch (e: any) {
    skillMessage.value = "检查失败：" + String(e);
  } finally {
    skillChecking.value = false;
  }
}

async function handleSyncSkills() {
  skillSyncing.value = true;
  skillMessage.value = "";
  try {
    const results = await invoke<SkillStatus[]>("sync_all_skills");
    skillStatuses.value = results;
    const updated = results.filter((s) => s.updated);
    const failed = results.filter((s) => s.error);
    if (failed.length > 0) {
      skillMessage.value = `部分失败：${failed.map((s) => s.name).join(", ")}`;
    } else if (updated.length > 0) {
      skillMessage.value = `已更新：${updated
        .map((s) => `${s.name} → ${s.local_version ?? "—"}`)
        .join(", ")}`;
    } else {
      skillMessage.value = "所有 skill 都是最新的";
    }
  } catch (e: any) {
    skillMessage.value = "更新失败：" + String(e);
  } finally {
    skillSyncing.value = false;
  }
}

async function refreshCacheSize() {
  loading.value = true;
  try {
    const bytes = await invoke<number>("get_cache_size");
    cacheSize.value = formatBytes(bytes);
  } catch (e: any) {
    cacheSize.value = "Unknown";
  } finally {
    loading.value = false;
  }
}

async function clearCache() {
  clearing.value = true;
  message.value = "";
  try {
    await invoke("clear_cache");
    message.value = "Cache cleared successfully";
    await refreshCacheSize();
  } catch (e: any) {
    message.value = "Failed to clear cache: " + String(e);
  } finally {
    clearing.value = false;
  }
}

async function refreshClaude() {
  claudeLoading.value = true;
  try {
    claudeInfo.value = await invoke<ClaudeAccountInfo>("get_claude_account");
  } catch {
    claudeInfo.value = null;
  } finally {
    claudeLoading.value = false;
  }
}

const copyMessage = ref("");

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    copyMessage.value = "已复制";
    setTimeout(() => { copyMessage.value = ""; }, 2000);
  } catch {
    copyMessage.value = "复制失败";
  }
}

</script>

<template>
  <div class="settings-page">
    <div class="page-header">
      <h3>Settings</h3>
    </div>

    <div class="settings-wrap">
    <div class="two-col">
    <!-- Claude CLI -->
    <div class="section">
      <div class="section-title">Claude CLI</div>
      <div class="section-desc">
        App 通过本机 Claude CLI 调用模型。这里展示当前 CLI 的安装与登录状态。
      </div>

      <!-- Status banner -->
      <div v-if="claudeLoading" class="claude-status loading">
        Checking Claude CLI...
      </div>

      <div v-else-if="claudeStatus === 'ok'" class="claude-status ok">
        <span class="status-icon">✅</span>
        <div class="status-text">
          <div class="status-line">已登录</div>
          <div class="status-sub">
            <span v-if="claudeInfo!.email">{{ claudeInfo!.email }}</span>
            <span v-else>账号信息无法解析（CLI 仍可正常使用）</span>
            <span v-if="claudeInfo!.subscription" class="badge">
              {{ claudeInfo!.subscription }}
            </span>
          </div>
        </div>
      </div>

      <div v-else-if="claudeStatus === 'no-login'" class="claude-status warn">
        <span class="status-icon">⚠️</span>
        <div class="status-text">
          <div class="status-line">已安装，但未登录</div>
          <div class="status-sub">
            打开终端运行下面这条命令，按提示完成登录：
          </div>
          <div class="code-row">
            <code>claude</code>
            <button class="copy-btn" @click="copyText('claude')">Copy</button>
          </div>
        </div>
      </div>

      <div v-else-if="claudeStatus === 'no-install'" class="claude-status warn">
        <span class="status-icon">❌</span>
        <div class="status-text">
          <div class="status-line">未检测到 Claude CLI</div>
          <div class="status-sub">用 npm 全局安装，然后跑一次 <code>claude</code> 登录：</div>
          <div class="code-row">
            <code>npm install -g @anthropic-ai/claude-code</code>
            <button class="copy-btn" @click="copyText('npm install -g @anthropic-ai/claude-code')">Copy</button>
          </div>
          <div class="code-row">
            <code>claude</code>
            <button class="copy-btn" @click="copyText('claude')">Copy</button>
          </div>
        </div>
      </div>

      <!-- Details -->
      <div v-if="claudeInfo?.cli_path" class="claude-detail">
        <span class="detail-label">CLI:</span>
        <code class="detail-value">{{ claudeInfo.cli_path }}</code>
      </div>

      <div class="claude-actions">
        <button class="refresh-btn" @click="refreshClaude" :disabled="claudeLoading">
          Refresh
        </button>
        <a
          href="#"
          class="doc-link"
          @click.prevent="openUrl('https://docs.claude.com/en/docs/claude-code/quickstart')"
        >
          Claude Code 文档 ↗
        </a>
        <span v-if="copyMessage" class="copy-message">{{ copyMessage }}</span>
      </div>
    </div>

    <!-- 模型配置 -->
    <div class="section">
      <div class="section-title">模型配置</div>
      <div class="section-desc">为各功能选择调用的 Claude 模型。Haiku 更快更省额度，Sonnet 质量更高。</div>

      <div class="model-row">
        <span class="model-label">回复生成</span>
        <select v-model="modelConfig.reply" class="model-select">
          <option v-for="o in MODEL_OPTIONS" :key="o.value" :value="o.value">{{ o.label }}</option>
        </select>
      </div>
      <div class="model-row">
        <span class="model-label">评论分析</span>
        <select v-model="modelConfig.analysis" class="model-select">
          <option v-for="o in MODEL_OPTIONS" :key="o.value" :value="o.value">{{ o.label }}</option>
        </select>
      </div>
      <div class="model-row">
        <span class="model-label">模板翻译</span>
        <select v-model="modelConfig.translate" class="model-select">
          <option v-for="o in MODEL_OPTIONS" :key="o.value" :value="o.value">{{ o.label }}</option>
        </select>
      </div>

      <div class="model-actions">
        <button class="sync-btn" @click="saveModelConfig" :disabled="modelSaving">
          {{ modelSaving ? "保存中..." : "保存" }}
        </button>
        <span v-if="modelMessage" class="model-msg">{{ modelMessage }}</span>
      </div>
    </div>
    </div><!-- end two-col top -->

    <div class="two-col">
    <!-- Skill 更新 -->
    <div class="section">
      <div class="section-title">Skill 更新</div>
      <div class="section-desc">
        App 启动时会自动从 GitHub 拉取 skill 最新版本到 <code>~/.claude/skills/</code>。也可以在这里手动检查。
      </div>

      <div class="token-row">
        <span class="token-label">GitHub Token</span>
        <input
          v-model="modelConfig.github_token"
          type="password"
          class="token-input"
          placeholder="ghp_xxxx（可选，解决 API 限流）"
        />
        <button class="token-save-btn" @click="saveModelConfig" :disabled="modelSaving">
          {{ modelSaving ? "保存中..." : "保存" }}
        </button>
      </div>
      <div class="token-hint">
        不配置时匿名调用（60次/小时），遇到限流报 403 时填入
        <a href="#" @click.prevent="openUrl('https://github.com/settings/tokens/new?description=tester-app&scopes=')">GitHub PAT</a>（无需任何权限）。
      </div>

      <div v-if="skillChecking && skillStatuses.length === 0" class="skill-loading">
        正在检查 skill 状态...
      </div>

      <div v-for="s in skillStatuses" :key="s.name" class="skill-row">
        <div class="skill-row-main">
          <span class="skill-icon" :class="{
            ok: s.up_to_date && !s.error,
            warn: !s.up_to_date && !s.error,
            err: !!s.error,
          }">
            {{ s.error ? "✗" : s.up_to_date ? "✓" : "⟳" }}
          </span>
          <div class="skill-info">
            <div class="skill-name-row">
              <span class="skill-name">{{ s.name }}</span>
              <span class="skill-version">{{ s.local_version ?? "未安装" }}</span>
              <span v-if="s.updated" class="updated-badge">刚刚更新</span>
              <span
                v-else-if="!s.up_to_date && s.remote_version && s.local_version && !s.error"
                class="outdated-badge"
                :title="`本地 ${s.local_version} → 远程 ${s.remote_version}`"
              >
                可更新到 {{ s.remote_version }}
              </span>
            </div>
            <div class="skill-meta">
              <a href="#" @click.prevent="openUrl(repoUrl(s))">
                {{ s.owner }}/{{ s.repo }}
              </a>
              ·
              <a href="#" @click.prevent="openUrl(releasesUrl(s))">Releases</a>
              <template v-if="s.remote_version && s.remote_version !== s.local_version">
                <span class="skill-remote-hint">
                  最新发布: <code>{{ s.remote_version }}</code>
                </span>
              </template>
            </div>
            <div v-if="s.error" class="skill-error">{{ s.error }}</div>
          </div>
        </div>
      </div>

      <div class="skill-actions">
        <button
          class="refresh-btn"
          @click="refreshSkillStatuses"
          :disabled="skillChecking || skillSyncing"
        >
          {{ skillChecking ? "检查中..." : "重新检查" }}
        </button>
        <button
          class="sync-btn"
          @click="handleSyncSkills"
          :disabled="skillChecking || skillSyncing"
        >
          {{ skillSyncing ? "更新中..." : "立即更新" }}
        </button>
      </div>

      <div v-if="skillMessage" class="message" :class="{ error: skillMessage.includes('失败') }">
        {{ skillMessage }}
      </div>
    </div>

    <!-- 右侧：Cache + 版本 垂直堆叠 -->
    <div class="right-stack">
    <div class="section">
      <div class="section-title">Cache Management</div>
      <div class="section-desc">Cached slide thumbnails are stored locally to speed up loading.</div>

      <div class="cache-row">
        <div class="cache-info">
          <span class="cache-label">Cache size</span>
          <span class="cache-value">{{ loading ? "Calculating..." : cacheSize }}</span>
        </div>
        <div class="cache-actions">
          <button class="refresh-btn" @click="refreshCacheSize" :disabled="loading">Refresh</button>
          <button class="clear-btn" @click="clearCache" :disabled="clearing">
            {{ clearing ? "Clearing..." : "Clear Cache" }}
          </button>
        </div>
      </div>

      <div v-if="message" class="message" :class="{ error: message.startsWith('Failed') }">
        {{ message }}
      </div>
    </div>

    <div class="section">
      <div class="section-title">版本</div>
      <div class="version-row">
        <div class="version-left">
          <span class="version-label">当前版本</span>
          <span class="version-value">v1.0.2</span>
        </div>
        <div class="version-right">
          <!-- idle / latest -->
          <button
            v-if="updateState === 'idle' || updateState === 'latest'"
            class="check-update-btn"
            @click="checkUpdate"
          >
            检测更新
          </button>
          <span v-if="updateState === 'latest'" class="update-latest">已是最新版本</span>

          <!-- checking -->
          <span v-if="updateState === 'checking'" class="update-checking">检查中...</span>

          <!-- available -->
          <template v-if="updateState === 'available' && updateInfo">
            <span class="update-new-badge">v{{ updateInfo.version }} 可更新</span>
            <button class="download-btn" @click="showUpdateModal = true">查看更新</button>
          </template>

          <!-- downloading -->
          <template v-if="updateState === 'downloading'">
            <div class="download-progress">
              <div class="progress-bar">
                <div class="progress-fill" :style="{ width: downloadPercent + '%' }"></div>
              </div>
              <span class="progress-text">
                {{ downloadPercent }}%
                <template v-if="downloadProgress.total">
                  （{{ formatBytes(downloadProgress.downloaded) }} / {{ formatBytes(downloadProgress.total) }}）
                </template>
              </span>
            </div>
          </template>

          <!-- done -->
          <span v-if="updateState === 'done'" class="update-done">正在重启安装...</span>

          <!-- error -->
          <span v-if="updateState === 'error'" class="update-error" :title="updateError">更新失败</span>
          <button v-if="updateState === 'error'" class="check-update-btn" @click="checkUpdate">重试</button>
        </div>
      </div>
      <div v-if="updateState === 'error' && updateError" class="update-error-msg">{{ updateError }}</div>
    </div>
    </div><!-- end right-stack -->
    </div><!-- end two-col bottom -->

    </div><!-- end settings-wrap -->

    <!-- 更新弹窗 -->
    <div v-if="showUpdateModal && updateInfo" class="modal-mask" @click.self="showUpdateModal = false">
      <div class="modal-box">
        <div class="modal-header">
          <span class="modal-title">发现新版本 v{{ updateInfo.version }}</span>
          <button class="modal-close" @click="showUpdateModal = false">✕</button>
        </div>
        <div class="modal-body">
          <pre v-if="updateInfo.body" class="release-notes">{{ updateInfo.body }}</pre>
          <p v-else class="release-empty">暂无更新说明</p>
        </div>
        <div class="modal-footer">
          <button class="modal-cancel" @click="showUpdateModal = false">稍后再说</button>
          <button class="download-btn" @click="showUpdateModal = false; startDownload()">下载并安装</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-page {
  height: 100%;
  overflow-y: auto;
  padding: 24px 28px;
  background: #f5f6f8;
}
.page-header {
  margin-bottom: 20px;
}
h3 {
  font-size: 15px;
  font-weight: 700;
  color: #1a202c;
  margin: 0;
  letter-spacing: -0.1px;
}
.settings-wrap {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.two-col {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 14px;
  align-items: stretch;
}
.right-stack {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.section {
  background: white;
  border: 1px solid #eaecef;
  border-radius: 12px;
  padding: 20px 22px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.05);
}
.model-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 0;
  border-bottom: 1px solid #f3f3f3;
}
.model-row:last-of-type {
  border-bottom: none;
}
.model-label {
  font-size: 13px;
  color: #2d3748;
}
.model-select {
  font-size: 12px;
  padding: 5px 8px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #333;
  cursor: pointer;
  outline: none;
}
.model-select:focus {
  border-color: #667eea;
}
.token-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 4px;
}
.token-label {
  font-size: 12px;
  color: #4a5568;
  white-space: nowrap;
  flex-shrink: 0;
}
.token-save-btn {
  padding: 4px 12px;
  font-size: 12px;
  border: 1px solid #667eea;
  background: #667eea;
  color: white;
  border-radius: 6px;
  cursor: pointer;
  flex-shrink: 0;
}
.token-save-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.token-input {
  font-size: 12px;
  padding: 5px 8px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #333;
  outline: none;
  flex: 1;
  min-width: 0;
  max-width: 280px;
}
.token-input:focus {
  border-color: #667eea;
}
.token-hint {
  font-size: 11px;
  color: #a0aec0;
  margin: 4px 0 0 0;
}
.token-hint a {
  color: #667eea;
}
.model-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 14px;
}
.model-msg {
  font-size: 12px;
  color: #48bb78;
}

/* Claude CLI section */
.claude-status {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 12px 14px;
  border-radius: 8px;
  margin-bottom: 12px;
}
.claude-status.loading {
  background: #f5f5f5;
  color: #888;
  font-size: 13px;
}
.claude-status.ok {
  background: #f0fff4;
  border: 1px solid #c6f6d5;
}
.claude-status.warn {
  background: #fffaf0;
  border: 1px solid #feebc8;
}
.status-icon {
  font-size: 20px;
  line-height: 1.2;
}
.status-text {
  flex: 1;
}
.status-line {
  font-size: 14px;
  font-weight: 600;
  color: #2d3748;
  margin-bottom: 4px;
}
.status-sub {
  font-size: 12px;
  color: #555;
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.badge {
  display: inline-block;
  padding: 1px 6px;
  background: #667eea;
  color: white;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.3px;
}
.code-row {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 6px;
}
.code-row code {
  flex: 1;
  display: block;
  padding: 6px 10px;
  background: #1e1e2e;
  color: #e0e0e0;
  border-radius: 4px;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  font-size: 12px;
  overflow-x: auto;
  white-space: nowrap;
}
.copy-btn {
  padding: 5px 10px;
  font-size: 11px;
  border: 1px solid #ddd;
  border-radius: 4px;
  background: white;
  cursor: pointer;
  color: #666;
}
.copy-btn:hover {
  background: #f5f5f5;
}
.claude-detail {
  display: flex;
  gap: 8px;
  align-items: center;
  margin: 8px 0;
  font-size: 12px;
}
.detail-label {
  color: #888;
}
.detail-value {
  flex: 1;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  font-size: 11px;
  padding: 3px 6px;
  background: #f5f5f5;
  border-radius: 4px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: #555;
}
.claude-actions {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 12px;
}
.doc-link {
  font-size: 12px;
  color: #667eea;
  text-decoration: none;
}
.doc-link:hover {
  text-decoration: underline;
}
.copy-message {
  font-size: 12px;
  color: #48bb78;
}
.section-title {
  font-size: 13px;
  font-weight: 700;
  color: #1a202c;
  margin-bottom: 4px;
  letter-spacing: -0.1px;
}
.section-desc {
  font-size: 12px;
  color: #9aa3b0;
  margin-bottom: 16px;
  line-height: 1.5;
}
.cache-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
}
.cache-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.cache-label {
  font-size: 12px;
  color: #888;
}
.cache-value {
  font-size: 20px;
  font-weight: 600;
  color: #333;
}
.cache-actions {
  display: flex;
  gap: 8px;
}
.refresh-btn {
  padding: 7px 14px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  cursor: pointer;
  color: #666;
}
.refresh-btn:hover:not(:disabled) {
  background: #f5f5f5;
}
.clear-btn {
  padding: 7px 14px;
  font-size: 12px;
  font-weight: 600;
  border: 1px solid #e53e3e;
  border-radius: 6px;
  background: white;
  color: #e53e3e;
  cursor: pointer;
}
.clear-btn:hover:not(:disabled) {
  background: #fff5f5;
}
.clear-btn:disabled,
.refresh-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.message {
  margin-top: 12px;
  font-size: 12px;
  color: #48bb78;
  padding: 8px 12px;
  background: #f0fff4;
  border-radius: 6px;
}
.message.error {
  color: #e53e3e;
  background: #fff5f5;
}

/* Skill update section */
.skill-loading {
  font-size: 12px;
  color: #888;
  padding: 8px 0;
}
.skill-row {
  padding: 8px 0;
  border-bottom: 1px solid #f3f3f3;
}
.skill-row:last-of-type {
  border-bottom: none;
}
.skill-row-main {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}
.skill-icon {
  width: 20px;
  height: 20px;
  line-height: 20px;
  text-align: center;
  border-radius: 50%;
  font-size: 12px;
  font-weight: 600;
  flex-shrink: 0;
}
.skill-icon.ok {
  background: #c6f6d5;
  color: #22543d;
}
.skill-icon.warn {
  background: #feebc8;
  color: #7b341e;
}
.skill-icon.err {
  background: #fed7d7;
  color: #c53030;
}
.skill-info {
  flex: 1;
  min-width: 0;
}
.skill-name-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.skill-name {
  font-size: 13px;
  font-weight: 600;
  color: #2d3748;
}
.skill-version {
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  font-size: 12px;
  font-weight: 600;
  padding: 2px 8px;
  background: #ebf4ff;
  color: #4c51bf;
  border-radius: 4px;
}
.skill-meta {
  font-size: 11px;
  color: #888;
  margin-top: 4px;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.skill-meta a {
  color: #667eea;
  text-decoration: none;
}
.skill-meta a:hover {
  text-decoration: underline;
}
.skill-remote-hint {
  margin-left: 4px;
}
.skill-remote-hint code {
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  background: #f5f5f5;
  padding: 1px 4px;
  border-radius: 3px;
  font-size: 11px;
  color: #555;
}
.updated-badge {
  display: inline-block;
  padding: 1px 6px;
  background: #48bb78;
  color: white;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 600;
}
.outdated-badge {
  display: inline-block;
  padding: 1px 6px;
  background: #ed8936;
  color: white;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 600;
}
.skill-error {
  font-size: 11px;
  color: #c53030;
  margin-top: 4px;
  word-break: break-all;
}
.skill-actions {
  display: flex;
  gap: 8px;
  margin-top: 12px;
}
.sync-btn {
  padding: 7px 14px;
  font-size: 12px;
  font-weight: 600;
  border: 1px solid #667eea;
  border-radius: 6px;
  background: #667eea;
  color: white;
  cursor: pointer;
}
.sync-btn:hover:not(:disabled) {
  background: #5a67d8;
  border-color: #5a67d8;
}
.sync-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.version-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.version-left {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-shrink: 0;
}
.version-right {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.version-label {
  font-size: 13px;
  color: #4a5568;
}
.version-value {
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  font-size: 13px;
  font-weight: 600;
  padding: 2px 10px;
  background: #ebf4ff;
  color: #4c51bf;
  border-radius: 6px;
}
.check-update-btn {
  padding: 5px 12px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #555;
  cursor: pointer;
}
.check-update-btn:hover {
  background: #f5f5f5;
}
.update-latest {
  font-size: 12px;
  color: #48bb78;
}
.update-checking {
  font-size: 12px;
  color: #888;
}
.update-new-badge {
  font-size: 12px;
  font-weight: 600;
  padding: 2px 8px;
  background: #feebc8;
  color: #c05621;
  border-radius: 4px;
}
.download-btn {
  padding: 5px 12px;
  font-size: 12px;
  font-weight: 600;
  border: 1px solid #667eea;
  border-radius: 6px;
  background: #667eea;
  color: white;
  cursor: pointer;
}
.download-btn:hover {
  background: #5a67d8;
}
.download-progress {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  min-width: 160px;
}
.progress-bar {
  flex: 1;
  height: 6px;
  background: #e2e8f0;
  border-radius: 3px;
  overflow: hidden;
}
.progress-fill {
  height: 100%;
  background: #667eea;
  border-radius: 3px;
  transition: width 0.2s;
}
.progress-text {
  font-size: 11px;
  color: #555;
  white-space: nowrap;
}
.update-done {
  font-size: 12px;
  color: #667eea;
}
.update-error {
  font-size: 12px;
  color: #e53e3e;
}
.update-error-msg {
  margin-top: 8px;
  font-size: 11px;
  color: #c53030;
  word-break: break-all;
}

/* 更新弹窗 */
.modal-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}
.modal-box {
  background: white;
  border-radius: 12px;
  width: 480px;
  max-width: 90vw;
  max-height: 70vh;
  display: flex;
  flex-direction: column;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.2);
}
.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 18px 20px 14px;
  border-bottom: 1px solid #f0f0f0;
}
.modal-title {
  font-size: 15px;
  font-weight: 600;
  color: #2d3748;
}
.modal-close {
  background: none;
  border: none;
  font-size: 14px;
  color: #aaa;
  cursor: pointer;
  padding: 2px 6px;
  border-radius: 4px;
}
.modal-close:hover {
  background: #f5f5f5;
  color: #555;
}
.modal-body {
  flex: 1;
  overflow-y: auto;
  padding: 16px 20px;
}
.release-notes {
  font-size: 13px;
  line-height: 1.7;
  color: #444;
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
  font-family: inherit;
}
.release-empty {
  font-size: 13px;
  color: #aaa;
  margin: 0;
}
.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 14px 20px 18px;
  border-top: 1px solid #f0f0f0;
}
.modal-cancel {
  padding: 7px 16px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #555;
  cursor: pointer;
}
.modal-cancel:hover {
  background: #f5f5f5;
}
</style>

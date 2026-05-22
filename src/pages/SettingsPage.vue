<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

const cacheSize = ref("");
const loading = ref(false);
const clearing = ref(false);
const message = ref("");

interface ClaudeAccountInfo {
  installed: boolean;
  cli_path: string | null;
  logged_in: boolean;
  email: string | null;
  subscription: string | null;
}

const claudeInfo = ref<ClaudeAccountInfo | null>(null);
const claudeLoading = ref(false);

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
});

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

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    message.value = "Copied: " + text;
  } catch {
    message.value = "Copy failed";
  }
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / Math.pow(1024, i);
  return `${val.toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}
</script>

<template>
  <div class="settings-page">
    <h3>Settings</h3>

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
      </div>
    </div>

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
  </div>
</template>

<style scoped>
.settings-page {
  height: 100%;
  overflow-y: auto;
  padding: 24px;
}
h3 {
  font-size: 16px;
  margin-bottom: 20px;
}
.section {
  background: white;
  border: 1px solid #e5e5e5;
  border-radius: 10px;
  padding: 20px;
  max-width: 560px;
  margin-bottom: 16px;
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
.section-title {
  font-size: 14px;
  font-weight: 600;
  margin-bottom: 4px;
}
.section-desc {
  font-size: 12px;
  color: #888;
  margin-bottom: 16px;
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
</style>

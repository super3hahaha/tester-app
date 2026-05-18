<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";

const cacheSize = ref("");
const loading = ref(false);
const clearing = ref(false);
const message = ref("");

onMounted(() => refreshCacheSize());

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
  max-width: 500px;
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

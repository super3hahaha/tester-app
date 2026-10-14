<script setup lang="ts">
import { open } from "@tauri-apps/plugin-dialog";

withDefaults(defineProps<{ disabled?: boolean }>(), {
  disabled: false,
});

const imagePaths = defineModel<string[]>({ required: true });
const emit = defineEmits<{
  error: [message: string];
}>();

function fileName(path: string) {
  return path.split(/[/\\]/).pop() || path;
}

async function pickImages() {
  try {
    const selected = await open({
      multiple: true,
      filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "webp", "gif"] }],
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    imagePaths.value = [...new Set([...imagePaths.value, ...paths])];
  } catch (e) {
    emit("error", String(e));
  }
}

function removeImage(path: string) {
  imagePaths.value = imagePaths.value.filter((item) => item !== path);
}
</script>

<template>
  <div class="image-source-row">
    <label class="image-source-label">图片</label>
    <div class="image-picker">
      <div class="image-picker-head">
        <span v-if="imagePaths.length" class="image-count">已选择 {{ imagePaths.length }} 张</span>
        <span v-else class="image-empty">未选择图片</span>
        <button type="button" class="link-btn" :disabled="disabled" @click="pickImages">
          {{ imagePaths.length ? "继续添加" : "选择" }}
        </button>
        <button
          v-if="imagePaths.length"
          type="button"
          class="link-btn danger-link"
          :disabled="disabled"
          @click="imagePaths = []"
        >
          清除
        </button>
      </div>
      <div v-if="imagePaths.length" class="image-file-list">
        <div v-for="path in imagePaths" :key="path" class="image-file-item" :title="path">
          <span class="image-file-name">{{ fileName(path) }}</span>
          <button
            class="image-remove-btn"
            type="button"
            :aria-label="`移除 ${fileName(path)}`"
            :disabled="disabled"
            @click="removeImage(path)"
          >
            ×
          </button>
        </div>
      </div>
      <div v-else class="image-help">支持 PNG、JPG、WEBP、GIF，可一次选择一张或多张</div>
    </div>
  </div>
</template>

<style scoped>
.image-source-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-bottom: 8px;
}
.image-source-label {
  width: 64px;
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
}
.image-picker {
  flex: 1;
  min-width: 0;
}
.image-picker-head {
  min-height: 24px;
  display: flex;
  align-items: center;
  gap: 10px;
}
.image-count {
  font-size: 13px;
  color: #2d3748;
}
.image-empty {
  font-size: 12px;
  color: #a0aec0;
}
.image-help {
  margin-top: 2px;
  font-size: 11px;
  color: #a0aec0;
}
.image-file-list {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 6px;
}
.image-file-item {
  max-width: 240px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 7px 3px 9px;
  border: 1px solid #e2e8f0;
  border-radius: 12px;
  background: #f7fafc;
  color: #4a5568;
  font-size: 11px;
}
.image-file-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.image-remove-btn {
  flex-shrink: 0;
  width: 16px;
  height: 16px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: #a0aec0;
  font-size: 14px;
  line-height: 16px;
  cursor: pointer;
}
.image-remove-btn:hover:not(:disabled) {
  background: #fed7d7;
  color: #c53030;
}
.image-remove-btn:disabled,
.link-btn:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}
.link-btn {
  padding: 0;
  border: none;
  background: none;
  color: #667eea;
  font-size: 12px;
  cursor: pointer;
}
.link-btn:hover:not(:disabled) {
  text-decoration: underline;
}
.danger-link {
  color: #c53030;
}
</style>

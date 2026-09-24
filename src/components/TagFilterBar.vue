<script setup lang="ts">
// 收藏页的标签筛选条：多选（命中任意一个即显示）+「无标签」+「管理标签」入口。
// counts 可选：id → 条数（含 NO_TAG_FILTER），用于在标签后面显示数字。
// app 可选：传了 → 只列「通用 + 该 app」的标签；没传 → 列全部，按「通用 / 各 app」分组
// （同时适用多个 app 的标签只在第一个组里出现一次）。
import { ref, computed, watch } from "vue";
import { tagsForApp, groupTags, NO_TAG_FILTER, type FavTag } from "../utils/favTags";
import { pkgApps, loadPkgApps, appLabel } from "../utils/appRegistry";
import TagManager from "./TagManager.vue";

const props = defineProps<{ modelValue: string[]; counts?: Record<string, number>; app?: string }>();
const emit = defineEmits<{ (e: "update:modelValue", v: string[]): void }>();

loadPkgApps();
const visible = computed(() => tagsForApp(props.app));
const groups = computed<{ key: string; label: string; tags: FavTag[] }[]>(() => {
  if (props.app) return [{ key: "", label: "", tags: visible.value }];
  const seen = new Set<string>();
  const out = [];
  for (const g of groupTags(visible.value, pkgApps.value.map((a) => a.package))) {
    const tags = g.tags.filter((t) => !seen.has(t.id));
    tags.forEach((t) => seen.add(t.id));
    if (tags.length) out.push({ key: g.key, label: g.key ? appLabel(g.key) : "通用", tags });
  }
  return out;
});
// 只有一个组时不显示组名（比如全是通用标签）
const showGroupLabel = computed(() => groups.value.length > 1);
const showManager = ref(false);

// 选中的标签被删掉 / 切 tab 后不在当前范围 → 按钮不显示了但筛选还卡着它 → 列表莫名变空。这里自动剔除。
watch(
  [visible, () => props.modelValue],
  () => {
    const ok = new Set(visible.value.map((t) => t.id));
    const live = props.modelValue.filter((id) => id === NO_TAG_FILTER || ok.has(id));
    if (live.length !== props.modelValue.length) emit("update:modelValue", live);
  },
  { immediate: true }
);

function toggle(id: string) {
  const cur = props.modelValue;
  emit("update:modelValue", cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id]);
}
function cnt(id: string): string {
  if (!props.counts) return "";
  return ` ${props.counts[id] || 0}`;
}
</script>

<template>
  <div class="tf-bar">
    <span class="tf-label">标签</span>
    <button class="tf-chip" :class="{ on: !modelValue.length }" @click="emit('update:modelValue', [])">全部</button>
    <template v-for="g in groups" :key="g.key">
      <span v-if="showGroupLabel" class="tf-group">{{ g.label }}</span>
      <button
        v-for="t in g.tags"
        :key="t.id"
        class="tf-chip"
        :class="{ on: modelValue.includes(t.id) }"
        :style="modelValue.includes(t.id) ? { background: t.color, borderColor: t.color, color: 'white' } : { color: t.color, borderColor: t.color + '66' }"
        @click="toggle(t.id)"
      >{{ t.name }}{{ cnt(t.id) }}</button>
    </template>
    <button class="tf-chip" :class="{ on: modelValue.includes(NO_TAG_FILTER) }" @click="toggle(NO_TAG_FILTER)">
      无标签{{ cnt(NO_TAG_FILTER) }}
    </button>
    <button class="tf-manage" @click="showManager = true">管理标签</button>
    <TagManager v-if="showManager" @close="showManager = false" />
  </div>
</template>

<style scoped>
.tf-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  margin-bottom: 12px;
}
.tf-label {
  font-size: 12px;
  color: #718096;
  margin-right: 2px;
}
.tf-group {
  font-size: 11px;
  color: #a0aec0;
  margin-left: 6px;
}
.tf-group::after {
  content: "：";
}
.tf-chip {
  font-size: 12px;
  padding: 3px 10px;
  border-radius: 12px;
  border: 1px solid #e2e8f0;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.tf-chip.on {
  background: #2d3748;
  border-color: #2d3748;
  color: white;
}
.tf-manage {
  margin-left: auto;
  font-size: 12px;
  padding: 3px 10px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.tf-manage:hover {
  background: #f5f5fa;
}
</style>

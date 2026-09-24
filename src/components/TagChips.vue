<script setup lang="ts">
// 只读展示一组标签色块（收藏记录上的 tags id → 标签库里查名字/颜色，已删除的 id 自动不显示）
import { computed } from "vue";
import { resolveTags } from "../utils/favTags";

const props = defineProps<{ ids?: string[] }>();
const list = computed(() => resolveTags(props.ids));
</script>

<template>
  <span v-if="list.length" class="tag-chips">
    <span
      v-for="t in list"
      :key="t.id"
      class="tag-chip"
      :style="{ background: t.color + '1f', color: t.color, borderColor: t.color + '66' }"
    >{{ t.name }}</span>
  </span>
</template>

<style scoped>
.tag-chips {
  display: inline-flex;
  flex-wrap: wrap;
  gap: 4px;
  vertical-align: middle;
}
.tag-chip {
  font-size: 11px;
  line-height: 1;
  padding: 3px 7px;
  border-radius: 10px;
  border: 1px solid;
  white-space: nowrap;
}
</style>

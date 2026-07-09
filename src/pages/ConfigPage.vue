<script setup lang="ts">
import { ref } from "vue";
import PlayConsoleConfigPage from "./PlayConsoleConfigPage.vue";
import BatchReplyConfigPage from "./BatchReplyConfigPage.vue";
import ScheduleConfigPage from "./ScheduleConfigPage.vue";

// 纯配置页：三个 Tab —— Play Console 拉取配置 + Batch Reply 配置 + 定时通知。
// 子页都用 v-show 常驻挂载，各自管理自己的 localStorage。
const tab = ref<"play" | "batch" | "schedule">("play");
</script>

<template>
  <div class="config-wrap">
    <nav class="config-tabs">
      <button
        class="config-tab"
        :class="{ active: tab === 'play' }"
        @click="tab = 'play'"
      >▶ Play Console 拉取配置</button>
      <button
        class="config-tab"
        :class="{ active: tab === 'batch' }"
        @click="tab = 'batch'"
      >🤖 Batch Reply 配置</button>
      <button
        class="config-tab"
        :class="{ active: tab === 'schedule' }"
        @click="tab = 'schedule'"
      >⏰ 定时通知</button>
    </nav>
    <div class="config-body">
      <PlayConsoleConfigPage v-show="tab === 'play'" />
      <BatchReplyConfigPage v-show="tab === 'batch'" />
      <ScheduleConfigPage v-show="tab === 'schedule'" />
    </div>
  </div>
</template>

<style scoped>
.config-wrap {
  height: 100%;
  display: flex;
  flex-direction: column;
}
.config-tabs {
  display: flex;
  gap: 4px;
  padding: 10px 20px 0 20px;
  border-bottom: 1px solid #e5e5e5;
  flex-shrink: 0;
}
.config-tab {
  padding: 8px 16px;
  font-size: 13px;
  font-weight: 600;
  border: none;
  border-bottom: 2px solid transparent;
  background: none;
  color: #888;
  cursor: pointer;
  margin-bottom: -1px;
}
.config-tab:hover { color: #4a5568; }
.config-tab.active {
  color: #667eea;
  border-bottom-color: #667eea;
}
.config-body {
  flex: 1;
  min-height: 0;
  overflow: hidden;
}
.config-body > * {
  height: 100%;
}
</style>

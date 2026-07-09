<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import LoginPage from "./pages/LoginPage.vue";
import MainPage from "./pages/MainPage.vue";
import { activeAccountId, activeAccountEmail } from "./utils/activeAccount";
import { migrateLegacyStorageOnce, migrateLegacyStorageOnceV2 } from "./utils/accountStorageMigration";
import { checkAndFireSchedule } from "./utils/scheduleDriver";

interface UserInfo {
  email: string;
  name: string;
  picture?: string;
  // 后端下发的 opaque 账号 id（= account_key），按账号隔离本地存储的维度。
  id?: string;
}

const user = ref<UserInfo | null>(null);
const checking = ref(true);

// 账号隔离维度的唯一来源：user 一变（登录/切换/登出）立即同步下发 id。
// flush:'sync' 确保子页因 accountEpoch 重挂、onMounted 读 scopedKey 前，id 已就绪。
watch(user, (u) => { activeAccountId.value = u?.id ?? ""; }, { immediate: true, flush: "sync" });
watch(user, (u) => { activeAccountEmail.value = u?.email ?? ""; }, { immediate: true, flush: "sync" });

// 定时通知驱动：常驻挂在这里（不受子页 v-show 切换/卸载影响）。每分钟 tick 一次；
// 启动时和从后台/睡眠唤醒时都补跑一次检查，覆盖「app 没开/睡眠中错过整点」（坑 §7）。
let scheduleTimer: number | undefined;
function onScheduleVisibilityChange() {
  if (document.visibilityState === "visible") checkAndFireSchedule();
}

onMounted(async () => {
  try {
    user.value = await invoke<UserInfo | null>("check_auth");
    // 已登录才迁移（此时 activeAccountId 已由上面的 watch 同步就绪），否则会落到 _none 桶。
    if (user.value) {
      migrateLegacyStorageOnce();
      migrateLegacyStorageOnceV2();
    }
  } catch {
    user.value = null;
  } finally {
    checking.value = false;
  }

  // Silent background skill sync on startup. Fire-and-forget — never blocks UI,
  // failures are logged to console only (manual trigger in Settings is the
  // user-visible path for surfacing problems).
  invoke("sync_all_skills")
    .then((results) => {
      console.log("[skill-sync]", results);
    })
    .catch((e) => {
      console.warn("[skill-sync] failed:", e);
    });

  checkAndFireSchedule();
  document.addEventListener("visibilitychange", onScheduleVisibilityChange);
  scheduleTimer = window.setInterval(checkAndFireSchedule, 60_000);
});

onUnmounted(() => {
  document.removeEventListener("visibilitychange", onScheduleVisibilityChange);
  if (scheduleTimer !== undefined) window.clearInterval(scheduleTimer);
});
</script>

<template>
  <div v-if="checking" class="loading">Loading...</div>
  <MainPage
    v-else-if="user"
    :user="user"
    @logout="user = null"
    @update-user="(u: UserInfo) => (user = u)"
  />
  <LoginPage v-else @login="(u: UserInfo) => (user = u)" />
</template>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}
body {
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  background: #f5f5f5;
  color: #333;
}
.loading {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100vh;
  font-size: 16px;
  color: #999;
}
</style>

<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { loadScheduleConfig, saveScheduleConfig, type ScheduleConfig } from "../utils/scheduleConfig";
import { syncScheduleRuntimeToBackend } from "../utils/scheduleRuntimeSync";

interface NotifyConfig {
  chat_id: string;
  bot_token: string;
}

const cfg = ref<ScheduleConfig>(loadScheduleConfig());
const savedSnapshot = ref(JSON.stringify(cfg.value));
const saveFlash = ref<"" | "saved" | "error">("");
const saveError = ref("");

const notify = ref<NotifyConfig>({ chat_id: "", bot_token: "" });
const notifyLoading = ref(false);
const notifySaveFlash = ref<"" | "saved" | "error">("");
const notifySaveError = ref("");

const newTime = ref("10:00");

const testSending = ref(false);
const testResult = ref<"" | "ok" | "error">("");
const testError = ref("");

onMounted(async () => {
  notifyLoading.value = true;
  try {
    notify.value = await invoke<NotifyConfig>("get_notify_config");
  } catch (e) {
    console.warn("load notify config failed:", e);
  } finally {
    notifyLoading.value = false;
  }
});

const currentSnapshot = computed(() => JSON.stringify(cfg.value));
const isDirty = computed(() => currentSnapshot.value !== savedSnapshot.value);

function addTime() {
  const t = newTime.value.trim();
  if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(t)) return;
  if (!cfg.value.times.includes(t)) {
    cfg.value.times.push(t);
    cfg.value.times.sort();
  }
}
function removeTime(t: string) {
  cfg.value.times = cfg.value.times.filter((x) => x !== t);
}

function handleSave() {
  saveError.value = "";
  try {
    saveScheduleConfig(cfg.value);
    savedSnapshot.value = JSON.stringify(cfg.value);
    // 把配置镜像给后端定时线程（真正的定时在后端跑）。
    syncScheduleRuntimeToBackend();
    saveFlash.value = "saved";
    setTimeout(() => { if (saveFlash.value === "saved") saveFlash.value = ""; }, 2500);
  } catch (e: any) {
    saveFlash.value = "error";
    saveError.value = String(e);
  }
}

const runningNow = ref(false);
const runResult = ref<"" | "ok" | "error">("");
const runMsg = ref("");
async function handleRunNow() {
  runningNow.value = true;
  runResult.value = "";
  runMsg.value = "";
  try {
    // 先把最新配置同步过去，再让后端立刻真实巡检一次（尊重去重/baseline）。
    await syncScheduleRuntimeToBackend();
    runMsg.value = await invoke<string>("run_schedule_now");
    runResult.value = "ok";
  } catch (e: any) {
    runResult.value = "error";
    runMsg.value = String(e);
  } finally {
    runningNow.value = false;
  }
}

async function handleSaveNotify() {
  notifySaveError.value = "";
  try {
    await invoke("save_notify_config", { config: notify.value });
    notifySaveFlash.value = "saved";
    setTimeout(() => { if (notifySaveFlash.value === "saved") notifySaveFlash.value = ""; }, 2500);
  } catch (e: any) {
    notifySaveFlash.value = "error";
    notifySaveError.value = String(e);
  }
}

async function handleTestSend() {
  testSending.value = true;
  testResult.value = "";
  testError.value = "";
  try {
    await invoke("send_telegram_message", {
      text: "🔔 <b>定时通知测试</b>\n这是一条测试消息，收到说明配置已生效。",
    });
    testResult.value = "ok";
  } catch (e: any) {
    testResult.value = "error";
    testError.value = String(e);
  } finally {
    testSending.value = false;
  }
}
</script>

<template>
  <div class="config-page">
    <header class="page-header">
      <h3>定时通知</h3>
      <p class="subtitle">
        到点自动执行一次批量拉取（按「Play Console 拉取配置」里启用的应用及其筛选条件），把<b>新增差评</b>通过 Telegram 通知你。
        只对当前登录的账号生效。
      </p>
    </header>

    <section class="block">
      <div class="block-head">
        <label class="enable-toggle">
          <input type="checkbox" v-model="cfg.enabled" />
          <span>启用定时通知</span>
        </label>
      </div>

      <div class="row">
        <label class="row-label">时间点</label>
        <div class="times-row">
          <span v-for="t in cfg.times" :key="t" class="time-chip">
            {{ t }}
            <button class="chip-remove" @click="removeTime(t)">×</button>
          </span>
          <input v-model="newTime" type="time" class="time-input" />
          <button class="link-btn" @click="addTime">+ 添加</button>
        </div>
      </div>

      <div class="row">
        <label class="row-label">无新增</label>
        <label class="inline-toggle">
          <input type="checkbox" v-model="cfg.notifyOnEmpty" />
          <span>也发一条心跳消息（"今日无新差评"），确认定时确实在跑</span>
        </label>
      </div>

      <div class="row">
        <label class="row-label">复查提醒</label>
        <label class="inline-toggle">
          <input type="checkbox" v-model="cfg.checkUpdated" />
          <span>顺带扫「回复后又被用户更新」的评论（你回复后用户又改了评论，回复可能过时），有则提醒去复查</span>
        </label>
      </div>

      <div class="row">
        <label class="row-label">消息条数</label>
        <input v-model.number="cfg.maxItemsInMsg" type="number" min="1" max="20" class="num-input" />
        <span class="hint">单条消息里最多列出的评论数，超出会折叠为"其余 N 条见 app"</span>
      </div>

      <div class="save-row">
        <button class="save-btn" @click="handleSave" :disabled="!isDirty">
          {{ isDirty ? "保存配置" : "已保存" }}
        </button>
        <button class="test-btn" @click="handleRunNow" :disabled="runningNow" title="立刻按当前配置真实巡检一次（尊重去重，首次会先建基线）">
          {{ runningNow ? "执行中…" : "立即执行一次" }}
        </button>
        <span v-if="saveFlash === 'saved'" class="flash flash-ok">✓ 已保存到本地</span>
        <span v-if="saveFlash === 'error'" class="flash flash-err">保存失败：{{ saveError }}</span>
        <span v-if="runResult === 'ok'" class="flash flash-ok">{{ runMsg }}</span>
        <span v-if="runResult === 'error'" class="flash flash-err">执行失败：{{ runMsg }}</span>
      </div>
      <p class="hint">
        定时在后端运行：只要 app 进程没退出（<b>Cmd+Q</b> 才会退），窗口最小化/被遮挡/放后台都能准点；
        电脑睡眠或 app 退出期间不跑，重新运行后会补发一次当天错过的时间点。
      </p>
    </section>

    <section class="block">
      <h4 class="block-title">Telegram 通知目标</h4>
      <p class="hint">与「问题反馈」用的私聊分开，避免通知刷屏污染反馈渠道；Bot Token 留空则复用反馈的同一个 bot。</p>
      <div class="row">
        <label class="row-label">Chat ID</label>
        <input v-model="notify.chat_id" type="text" class="text-input" placeholder="接收通知的 chat_id" :disabled="notifyLoading" />
      </div>
      <div class="row">
        <label class="row-label">Bot Token</label>
        <input v-model="notify.bot_token" type="text" class="text-input" placeholder="留空 = 复用反馈的 bot token" :disabled="notifyLoading" />
      </div>
      <div class="save-row">
        <button class="save-btn" @click="handleSaveNotify" :disabled="notifyLoading">保存通知目标</button>
        <button class="test-btn" @click="handleTestSend" :disabled="testSending">
          {{ testSending ? "发送中…" : "立即测试发送" }}
        </button>
        <span v-if="notifySaveFlash === 'saved'" class="flash flash-ok">✓ 已保存</span>
        <span v-if="notifySaveFlash === 'error'" class="flash flash-err">保存失败：{{ notifySaveError }}</span>
        <span v-if="testResult === 'ok'" class="flash flash-ok">✓ 发送成功，去 Telegram 查看</span>
        <span v-if="testResult === 'error'" class="flash flash-err">发送失败：{{ testError }}</span>
      </div>
    </section>
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
.page-header h3 { margin: 0; font-size: 16px; }
.subtitle { margin: 4px 0 16px 0; font-size: 12px; color: #888; line-height: 1.6; }

.block {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 14px 16px;
  background: white;
  margin-bottom: 14px;
}
.block-title { margin: 0 0 4px 0; font-size: 13px; color: #2d3748; }
.block-head { margin-bottom: 10px; }

.enable-toggle, .inline-toggle {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  cursor: pointer;
}
.enable-toggle input, .inline-toggle input { width: 16px; height: 16px; cursor: pointer; }

.row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 0;
  flex-wrap: wrap;
}
.row-label {
  width: 70px;
  flex-shrink: 0;
  font-size: 12px;
  color: #888;
}
.hint { font-size: 11px; color: #999; margin: 4px 0 8px 0; }

.times-row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
.time-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 4px 3px 10px;
  border-radius: 12px;
  background: #eef1ff;
  color: #4c51bf;
  font-size: 12px;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
}
.chip-remove {
  border: none;
  background: none;
  color: #4c51bf;
  cursor: pointer;
  font-size: 14px;
  line-height: 1;
  padding: 0 4px;
}
.chip-remove:hover { color: #e53e3e; }
.time-input {
  padding: 4px 8px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
}
.num-input {
  width: 70px;
  padding: 4px 8px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
}
.text-input {
  flex: 1;
  min-width: 200px;
  max-width: 360px;
  padding: 5px 10px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  outline: none;
}
.text-input:disabled { background: #f3f3f3; color: #aaa; }

.link-btn {
  background: none;
  border: none;
  color: #667eea;
  font-size: 12px;
  cursor: pointer;
  padding: 4px 6px;
}
.link-btn:hover { text-decoration: underline; }

.save-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 10px;
  flex-wrap: wrap;
}
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
.test-btn {
  padding: 6px 14px;
  font-size: 12px;
  border: 1px solid #667eea;
  border-radius: 6px;
  background: white;
  color: #667eea;
  cursor: pointer;
}
.test-btn:hover:not(:disabled) { background: #f5f5fa; }
.test-btn:disabled { opacity: 0.5; cursor: not-allowed; }

.flash { font-size: 12px; padding: 2px 10px; border-radius: 10px; }
.flash-ok { background: #c6f6d5; color: #22543d; }
.flash-err { background: #fed7d7; color: #9b2c2c; word-break: break-all; }
</style>

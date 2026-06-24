<script setup lang="ts">
// 提示词配置：各 AI 功能的完整 prompt 模板，含 {占位符}，可整段编辑。
// 后端 prompt_config.rs 存 ~/.tester-app/prompt-config.json；render() 运行时替换占位符。
// 改坏占位符 / JSON 输出格式会导致解析失败 → 每个模板配「恢复默认」兜底。
import { ref, computed, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";

interface PromptConfig {
  gen: string;
  analysis: string;
  mail: string;
}

// 每个模板的元信息：标签、说明、可用占位符（提示用户别删坏）。
const FIELDS: { key: keyof PromptConfig; label: string; desc: string; vars: string[] }[] = [
  {
    key: "gen",
    label: "单条「AI 生成回复」",
    desc: "评论列表里点「AI 生成回复」时用的 prompt，输出 3 条候选（JSON 数组）。",
    vars: ["knowledge", "product", "package_name", "star", "original", "zh", "rev_lang", "version", "device", "os", "instruction", "lang_rule"],
  },
  {
    key: "analysis",
    label: "评论「🔍 分析」",
    desc: "点「🔍 分析」时用的 prompt，输出分析 + 一条推荐回复（JSON 对象）。",
    vars: ["knowledge", "product", "package_name", "star", "original", "zh", "rev_lang", "version", "device", "os", "lang_rule"],
  },
  {
    key: "mail",
    label: "邮件回复草稿",
    desc: "Gmail 页生成邮件回复草稿时用的 prompt（JSON 对象）。",
    vars: ["body", "instruction", "lang"],
  },
];

const config = ref<PromptConfig>({ gen: "", analysis: "", mail: "" });
const defaults = ref<PromptConfig | null>(null);
const saving = ref(false);
const message = ref("");

const activeKey = ref<keyof PromptConfig>("gen");
const activeField = computed(() => FIELDS.find((f) => f.key === activeKey.value)!);

onMounted(load);

async function load() {
  try {
    config.value = await invoke<PromptConfig>("get_prompt_config");
    defaults.value = await invoke<PromptConfig>("get_default_prompt_config");
  } catch (e: any) {
    message.value = "加载失败：" + String(e);
  }
}

async function save() {
  saving.value = true;
  message.value = "";
  try {
    await invoke("save_prompt_config", { config: config.value });
    message.value = "已保存";
    setTimeout(() => { message.value = ""; }, 2000);
  } catch (e: any) {
    message.value = "保存失败：" + String(e);
  } finally {
    saving.value = false;
  }
}

function resetField(key: keyof PromptConfig) {
  if (defaults.value) config.value[key] = defaults.value[key];
}

function isModified(key: keyof PromptConfig): boolean {
  return !!defaults.value && config.value[key] !== defaults.value[key];
}
</script>

<template>
  <div class="prompt-page">
    <div class="page-header">
      <h3>Prompt 配置</h3>
    </div>

    <div class="intro">
      编辑各 AI 功能的<strong>完整提示词模板</strong>。<code v-pre>{product}</code>、<code v-pre>{star}</code> 等占位符运行时会替换成真实值，
      JSON 输出格式必须保留——<strong>改坏占位符或 JSON 会导致结果解析失败</strong>，可点对应模板的「恢复默认」还原。翻译类 prompt 未开放（含解析关键的语言码）。
    </div>

    <div class="tabs">
      <button
        v-for="f in FIELDS"
        :key="f.key"
        class="tab"
        :class="{ active: activeKey === f.key }"
        @click="activeKey = f.key"
      >
        {{ f.label }}
        <span v-if="isModified(f.key)" class="tab-dot" title="已修改"></span>
      </button>
    </div>

    <div class="field">
      <div class="field-head">
        <div class="field-desc">{{ activeField.desc }}</div>
        <button class="reset-btn" :disabled="!isModified(activeKey)" @click="resetField(activeKey)">恢复默认</button>
      </div>
      <div class="vars">
        可用占位符：
        <code v-for="v in activeField.vars" :key="v">{{ "{" + v + "}" }}</code>
      </div>
      <textarea v-model="config[activeKey]" class="textarea" spellcheck="false"></textarea>
    </div>

    <div class="actions">
      <button class="save-btn" @click="save" :disabled="saving">
        {{ saving ? "保存中..." : "保存" }}
      </button>
      <span v-if="message" class="msg" :class="{ error: message.includes('失败') }">{{ message }}</span>
    </div>
  </div>
</template>

<style scoped>
.prompt-page {
  height: 100%;
  overflow-y: auto;
  padding: 24px 28px;
  background: #f5f6f8;
}
.page-header {
  margin-bottom: 16px;
}
h3 {
  font-size: 15px;
  font-weight: 700;
  color: #1a202c;
  margin: 0;
  letter-spacing: -0.1px;
}
.intro {
  font-size: 12px;
  color: #718096;
  line-height: 1.6;
  background: #fffaf0;
  border: 1px solid #feebc8;
  border-radius: 8px;
  padding: 10px 14px;
  margin-bottom: 16px;
}
.intro code {
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  background: #fff;
  border: 1px solid #f0e0c0;
  padding: 0 4px;
  border-radius: 3px;
  font-size: 11px;
}
.tabs {
  display: flex;
  gap: 4px;
  margin-bottom: -1px;
}
.tab {
  position: relative;
  padding: 8px 18px;
  font-size: 13px;
  font-weight: 600;
  color: #718096;
  background: #eef0f3;
  border: 1px solid #eaecef;
  border-bottom: none;
  border-radius: 10px 10px 0 0;
  cursor: pointer;
}
.tab:hover {
  color: #4a5568;
  background: #e6e9ee;
}
.tab.active {
  color: #4c51bf;
  background: white;
  border-color: #eaecef;
}
.tab-dot {
  display: inline-block;
  width: 6px;
  height: 6px;
  margin-left: 6px;
  vertical-align: middle;
  background: #ed8936;
  border-radius: 50%;
}
.field {
  background: white;
  border: 1px solid #eaecef;
  border-radius: 0 12px 12px 12px;
  padding: 18px 20px;
  margin-bottom: 14px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.05);
}
.field-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}
.reset-btn {
  padding: 4px 12px;
  font-size: 11px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #666;
  cursor: pointer;
  flex-shrink: 0;
}
.reset-btn:hover:not(:disabled) {
  background: #f5f5f5;
}
.reset-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.field-desc {
  font-size: 12px;
  color: #9aa3b0;
  margin: 0;
  line-height: 1.5;
  flex: 1;
}
.vars {
  font-size: 11px;
  color: #a0aec0;
  margin: 10px 0 8px;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px;
}
.vars code {
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  background: #ebf4ff;
  color: #4c51bf;
  padding: 1px 5px;
  border-radius: 3px;
  font-size: 11px;
}
.textarea {
  width: 100%;
  box-sizing: border-box;
  min-height: 420px;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  font-size: 12px;
  line-height: 1.6;
  padding: 12px 14px;
  border: 1px solid #ddd;
  border-radius: 8px;
  background: #fafbfc;
  color: #2d3748;
  outline: none;
  resize: vertical;
}
.textarea:focus {
  border-color: #667eea;
  background: white;
}
.actions {
  position: sticky;
  bottom: 0;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 0;
  background: linear-gradient(to top, #f5f6f8 60%, transparent);
}
.save-btn {
  padding: 8px 24px;
  font-size: 13px;
  font-weight: 600;
  border: 1px solid #667eea;
  border-radius: 8px;
  background: #667eea;
  color: white;
  cursor: pointer;
}
.save-btn:hover:not(:disabled) {
  background: #5a67d8;
}
.save-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.msg {
  font-size: 12px;
  color: #48bb78;
}
.msg.error {
  color: #e53e3e;
}
</style>

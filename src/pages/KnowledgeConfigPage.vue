<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";

// 知识配置：每个产品一份「应用知识块」（Markdown 纯文本），存
// ~/.tester-app/review-analysis/{slug}.md。评论页「🔍 分析」时按评论来源应用注入
// 到提示词的 {app_knowledge}，帮助模型判断用户问题、给出更对症的推荐回复。
interface KnowledgeInfo {
  product: string;
  apps: string[];
  has_content: boolean;
  chars: number;
}

// 「插入骨架」用的三段式默认模板：定位 + 常见问题清单 + 红线。
const SKELETON = `应用是一款 <一句话定位：做什么的、核心功能>。

常见问题与标准排查引导：
- 「<用户常见症状 / 抱怨>」：<可能原因>。引导用户<可操作的排查步骤>。
- 「<症状 2>」：<排查步骤>。
- 「<症状 3>」：<排查步骤>。

红线：不要承诺具体修复版本号或时间；未确认机型行为时不要断言「这是 XX 系统的 bug」；不杜撰邮箱 / 客服渠道 / 未发布功能。`;

const props = defineProps<{ activeOption?: string }>();

const products = ref<KnowledgeInfo[]>([]);
const selectedProduct = ref("");
const editContent = ref("");
const savedContent = ref(""); // 上次加载/保存的内容，用于 dirty 判断
const loading = ref(false);
const saving = ref(false);
const error = ref("");
const notice = ref("");

const dirty = computed(() => editContent.value !== savedContent.value);
const charCount = computed(() => editContent.value.length);
const selectedInfo = computed(
  () => products.value.find((p) => p.product === selectedProduct.value) || null
);

function productSlug(name: string): string {
  if (name === "通用") return "common";
  const s = name.split("").filter(c => /[a-zA-Z0-9]/.test(c)).join("").toLowerCase();
  return s || "tpl";
}
const filePath = computed(() =>
  selectedProduct.value
    ? `~/.tester-app/review-analysis/${productSlug(selectedProduct.value)}.md`
    : ""
);

function flash(msg: string) {
  notice.value = msg;
  setTimeout(() => {
    if (notice.value === msg) notice.value = "";
  }, 2500);
}

async function loadProducts() {
  loading.value = true;
  error.value = "";
  try {
    const all = await invoke<KnowledgeInfo[]>("list_knowledge");
    products.value = all.filter((p) => p.product !== "通用");
    if (
      products.value.length &&
      !products.value.some((p) => p.product === selectedProduct.value)
    ) {
      await selectProduct(products.value[0].product, true);
    }
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function selectProduct(p: string, force = false) {
  if (!force && p === selectedProduct.value) return;
  if (!force && dirty.value && !confirm("当前知识块有未保存改动，切换将丢弃。继续？")) return;
  selectedProduct.value = p;
  error.value = "";
  try {
    const content = await invoke<string>("read_knowledge", { product: p });
    editContent.value = content;
    savedContent.value = content;
  } catch (e) {
    error.value = String(e);
  }
}

function insertSkeleton() {
  if (editContent.value.trim() && !confirm("当前已有内容，用骨架覆盖？")) return;
  editContent.value = SKELETON;
}

async function save() {
  if (!selectedProduct.value || saving.value) return;
  saving.value = true;
  error.value = "";
  try {
    await invoke("write_knowledge", {
      product: selectedProduct.value,
      content: editContent.value,
    });
    savedContent.value = editContent.value;
    flash("已保存");
    await loadProducts(); // 刷新 tab 上的已配置标记 / 字数
  } catch (e) {
    error.value = String(e);
  } finally {
    saving.value = false;
  }
}

onMounted(() => {
  if (props.activeOption === "review-knowledge") loadProducts();
});
watch(
  () => props.activeOption,
  (v) => {
    if (v === "review-knowledge") loadProducts();
  }
);
</script>

<template>
  <div class="kn-page">
    <header class="page-header">
      <h3>知识配置</h3>
      <p class="subtitle">
        为每个产品维护一份「应用知识块」，存本地
        <code>~/.tester-app/review-analysis/</code>。评论页「🔍 分析」时按评论来源应用注入，帮助模型判断问题、给出更对症的回复。
      </p>
    </header>

    <div v-if="error" class="banner banner-error">{{ error }}</div>
    <div v-if="notice" class="banner banner-ok">{{ notice }}</div>

    <!-- 产品选择 -->
    <div class="product-tabs">
      <button
        v-for="p in products"
        :key="p.product"
        class="product-tab"
        :class="{ active: p.product === selectedProduct }"
        @click="selectProduct(p.product)"
      >
        <span class="dot" :class="{ on: p.has_content }" :title="p.has_content ? '已配置' : '未配置'"></span>
        {{ p.product }}
      </button>
      <span v-if="!loading && products.length === 0" class="empty-hint">
        暂无产品。先到「模板管理」新建产品或同步 review-reply skill。
      </span>
    </div>

    <div v-if="selectedInfo" class="meta">
      <span v-if="selectedInfo.apps.length">关联应用：{{ selectedInfo.apps.join("、") }}</span>
      <span v-else class="muted">（package_map 里暂无关联应用）</span>
      <span class="file-path" :title="filePath">存储：<code>{{ filePath }}</code></span>
    </div>

    <!-- 编辑器 -->
    <div v-if="selectedProduct" class="editor-wrap">
      <div class="pane">
        <div class="pane-head">
          <span>编辑</span>
          <span class="count" :class="{ over: charCount > 4000 }">{{ charCount }} 字</span>
        </div>
        <textarea
          v-model="editContent"
          class="editor"
          spellcheck="false"
          placeholder="点「插入骨架」从模板开始，或直接写这个应用的定位、常见问题与排查引导…"
        ></textarea>
      </div>
    </div>

    <!-- 操作栏 -->
    <div v-if="selectedProduct" class="actions">
      <button class="btn-ghost" @click="insertSkeleton">插入骨架</button>
      <div class="spacer"></div>
      <span v-if="dirty" class="dirty-hint">● 未保存</span>
      <button class="btn-primary" :disabled="saving || !dirty" @click="save">
        {{ saving ? "保存中…" : "保存" }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.kn-page {
  padding: 4px 2px;
}
.page-header h3 {
  margin: 0 0 4px;
}
.subtitle {
  margin: 0 0 12px;
  font-size: 13px;
  color: #718096;
  line-height: 1.5;
}
.subtitle code {
  background: #edf2f7;
  padding: 1px 5px;
  border-radius: 4px;
}
.banner {
  padding: 8px 12px;
  border-radius: 8px;
  margin-bottom: 10px;
  font-size: 13px;
}
.banner-error {
  background: #fed7d7;
  color: #9b2c2c;
}
.banner-ok {
  background: #c6f6d5;
  color: #22543d;
}
.product-tabs {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  align-items: center;
  margin-bottom: 10px;
}
.product-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 14px;
  border: 1px solid #e2e8f0;
  border-radius: 18px;
  background: white;
  cursor: pointer;
  font-size: 13px;
}
.product-tab:hover {
  background: #f7fafc;
}
.product-tab.active {
  border-color: #667eea;
  background: #667eea;
  color: white;
}
.dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #cbd5e0;
}
.dot.on {
  background: #48bb78;
}
.product-tab.active .dot.on {
  background: #9ae6b4;
}
.empty-hint {
  font-size: 13px;
  color: #a0aec0;
}
.meta {
  font-size: 12px;
  color: #718096;
  margin-bottom: 10px;
}
.meta .muted {
  color: #a0aec0;
}
.file-path {
  color: #a0aec0;
  margin-left: 12px;
}
.file-path code {
  background: #edf2f7;
  padding: 1px 5px;
  border-radius: 4px;
  font-size: 11px;
  color: #4a5568;
}
.editor-wrap {
  display: block;
}
.pane {
  border: 1px solid #e2e8f0;
  border-radius: 10px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  /* 固定高度：内容多时在内部滚动，不再把保存按钮往下顶 */
  height: 72vh;
  min-height: 480px;
  max-height: 720px;
}
.pane-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 6px 12px;
  background: #f7fafc;
  border-bottom: 1px solid #e2e8f0;
  font-size: 12px;
  color: #4a5568;
  font-weight: 600;
}
.count {
  font-weight: 400;
  color: #a0aec0;
}
.count.over {
  color: #c05621;
}
.editor {
  flex: 1;
  min-height: 0;
  border: none;
  outline: none;
  resize: none;
  overflow-y: auto;
  padding: 12px;
  font-size: 13px;
  line-height: 1.6;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  color: #2d3748;
}
.actions {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 12px;
}
.spacer {
  flex: 1;
}
.dirty-hint {
  font-size: 12px;
  color: #dd6b20;
}
.btn-ghost {
  padding: 7px 14px;
  border: 1px dashed #cbd5e0;
  border-radius: 8px;
  background: white;
  color: #5a67d8;
  cursor: pointer;
  font-size: 13px;
}
.btn-ghost:hover {
  background: #eef0fc;
}
.btn-primary {
  padding: 7px 18px;
  border: none;
  border-radius: 8px;
  background: #667eea;
  color: white;
  cursor: pointer;
  font-size: 13px;
}
.btn-primary:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>

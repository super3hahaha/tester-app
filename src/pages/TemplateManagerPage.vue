<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

// 模板管理：分产品增删改 + xlsx 批量导入。数据存 ~/.tester-app/templates/，
// review-reply skill 运行时从那里读（index 由后端从全文自动重建）。
interface ProductInfo {
  product: string;
  count: number;
  apps: string[];
}
interface Template {
  id: string;
  category: string;
  text: string;
  lang: string; // 源语言 en / zh-CN
}
interface ImportResult {
  count: number;
  sheet: string;
  warning: string | null;
}

const props = defineProps<{ activeOption?: string }>();

const products = ref<ProductInfo[]>([]);
const selectedProduct = ref("");
const templates = ref<Template[]>([]);
const loading = ref(false);
const error = ref("");
const notice = ref("");

// 新增表单
const newCategory = ref("");
const newText = ref("");
const newLang = ref("en"); // 源语言，默认英文

// 内联两步确认（Tauri webview 里 window.confirm 不弹，见 gotchas.md）
const armedDeleteId = ref("");
let deleteTimer: number | null = null;

// xlsx 导入
const importing = ref(false);
const pendingImportPath = ref("");

const selectedInfo = computed(() =>
  products.value.find((p) => p.product === selectedProduct.value) || null
);

async function loadProducts() {
  loading.value = true;
  error.value = "";
  try {
    products.value = await invoke<ProductInfo[]>("list_template_products");
    if (
      products.value.length > 0 &&
      !products.value.some((p) => p.product === selectedProduct.value)
    ) {
      selectedProduct.value = products.value[0].product;
    }
    if (selectedProduct.value) await loadTemplates();
  } catch (e: any) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function loadTemplates() {
  if (!selectedProduct.value) {
    templates.value = [];
    return;
  }
  try {
    templates.value = await invoke<Template[]>("list_templates", {
      product: selectedProduct.value,
    });
  } catch (e: any) {
    error.value = String(e);
  }
}

function selectProduct(p: string) {
  selectedProduct.value = p;
  notice.value = "";
  error.value = "";
  armedDeleteId.value = "";
  loadTemplates();
}

function flash(msg: string) {
  notice.value = msg;
  window.setTimeout(() => {
    if (notice.value === msg) notice.value = "";
  }, 2500);
}

async function addTemplate() {
  if (!selectedProduct.value) return;
  if (!newText.value.trim()) {
    error.value = "模板正文不能为空。";
    return;
  }
  error.value = "";
  try {
    await invoke<string>("add_template", {
      product: selectedProduct.value,
      category: newCategory.value,
      text: newText.value,
      lang: newLang.value,
    });
    newCategory.value = "";
    newText.value = "";
    await loadProducts(); // 刷新条数 + 列表
    flash("已新增模板");
  } catch (e: any) {
    error.value = String(e);
  }
}

async function saveTemplate(t: Template) {
  error.value = "";
  try {
    await invoke("update_template", {
      product: selectedProduct.value,
      id: t.id,
      category: t.category,
      text: t.text,
      lang: t.lang,
    });
    flash(`已保存 ${t.id}`);
  } catch (e: any) {
    error.value = String(e);
  }
}

function armDelete(id: string) {
  if (armedDeleteId.value === id) {
    // 第二次点击：真正删除
    if (deleteTimer) clearTimeout(deleteTimer);
    armedDeleteId.value = "";
    doDelete(id);
    return;
  }
  armedDeleteId.value = id;
  if (deleteTimer) clearTimeout(deleteTimer);
  deleteTimer = window.setTimeout(() => {
    armedDeleteId.value = "";
  }, 4000);
}

async function doDelete(id: string) {
  error.value = "";
  try {
    await invoke("delete_template", { product: selectedProduct.value, id });
    await loadProducts();
    flash(`已删除 ${id}`);
  } catch (e: any) {
    error.value = String(e);
  }
}

async function pickXlsx() {
  error.value = "";
  try {
    const picked = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "Excel", extensions: ["xlsx"] }],
    });
    if (typeof picked === "string") {
      pendingImportPath.value = picked;
    }
  } catch (e: any) {
    error.value = String(e);
  }
}

function cancelImport() {
  pendingImportPath.value = "";
}

async function confirmImport() {
  if (!pendingImportPath.value || !selectedProduct.value) return;
  importing.value = true;
  error.value = "";
  try {
    const res = await invoke<ImportResult>("import_templates_xlsx", {
      product: selectedProduct.value,
      path: pendingImportPath.value,
    });
    pendingImportPath.value = "";
    await loadProducts();
    let msg = `已导入 ${res.count} 条（sheet：${res.sheet}）`;
    if (res.warning) msg += ` · ${res.warning}`;
    flash(msg);
  } catch (e: any) {
    error.value = String(e);
  } finally {
    importing.value = false;
  }
}

const importFileName = computed(() => {
  const p = pendingImportPath.value;
  if (!p) return "";
  const parts = p.split(/[\\/]/);
  return parts[parts.length - 1];
});

onMounted(loadProducts);
// MainPage 用 v-show，组件常驻；切回本页时刷新一次（拿到最新种子/外部改动）
watch(
  () => props.activeOption,
  (v) => {
    if (v === "review-templates") loadProducts();
  }
);
</script>

<template>
  <div class="template-page">
    <header class="page-header">
      <h3>模板管理</h3>
      <p class="subtitle">
        分产品维护 Google Play 回复模板。改动存本地 <code>~/.tester-app/templates/</code>，
        批量回复「匹配模板并填充」时 skill 直接读这里（索引自动重建）。
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
        {{ p.product }}
        <span class="tab-count">{{ p.count }}</span>
      </button>
      <span v-if="!loading && products.length === 0" class="empty-hint">
        暂无产品模板。先到 Settings 同步 review-reply skill，或用下方「从 xlsx 导入」。
      </span>
    </div>

    <div v-if="selectedInfo" class="product-meta">
      <span v-if="selectedInfo.apps.length">
        关联应用：{{ selectedInfo.apps.join("、") }}
      </span>
      <span v-else class="muted">（package_map 里暂无关联应用）</span>

      <div class="meta-spacer"></div>

      <!-- xlsx 导入 -->
      <template v-if="!pendingImportPath">
        <button class="import-btn" :disabled="importing" @click="pickXlsx">
          📥 从 xlsx 导入
        </button>
      </template>
      <template v-else>
        <span class="import-confirm-text">
          将用 <b>{{ importFileName }}</b> <b class="warn">覆盖</b>「{{ selectedProduct }}」现有模板
        </span>
        <button class="import-confirm-btn" :disabled="importing" @click="confirmImport">
          {{ importing ? "导入中…" : "确认导入" }}
        </button>
        <button class="import-cancel-btn" :disabled="importing" @click="cancelImport">
          取消
        </button>
      </template>
    </div>

    <!-- 新增模板 -->
    <div v-if="selectedProduct" class="add-card">
      <div class="add-row">
        <input
          v-model="newCategory"
          class="add-category"
          placeholder="类别（如：要五星 / 广告太多；可留空）"
        />
        <select v-model="newLang" class="add-lang" title="模板源语言">
          <option value="en">英文 en</option>
          <option value="zh-CN">中文 zh-CN</option>
        </select>
        <button class="add-btn" :disabled="!newText.trim()" @click="addTemplate">
          + 新增模板
        </button>
      </div>
      <textarea
        v-model="newText"
        class="add-text"
        rows="2"
        placeholder="模板英文正文（运行时由 skill 翻译到目标语言）"
      ></textarea>
    </div>

    <!-- 模板列表 -->
    <div v-if="loading" class="empty-state">加载中…</div>
    <div v-else-if="selectedProduct && templates.length === 0" class="empty-state">
      该产品还没有模板，用上方「+ 新增模板」或「从 xlsx 导入」。
    </div>

    <div class="tpl-list">
      <article v-for="t in templates" :key="t.id" class="tpl-card">
        <div class="tpl-head">
          <span class="tpl-id">{{ t.id }}</span>
          <input v-model="t.category" class="tpl-category" placeholder="类别" />
          <select v-model="t.lang" class="tpl-lang" title="源语言">
            <option value="en">en</option>
            <option value="zh-CN">zh-CN</option>
          </select>
          <span class="tpl-len">{{ t.text.length }} 字符</span>
          <div class="tpl-head-spacer"></div>
          <button class="save-btn" @click="saveTemplate(t)">保存</button>
          <button
            class="del-btn"
            :class="{ armed: armedDeleteId === t.id }"
            @click="armDelete(t.id)"
          >
            {{ armedDeleteId === t.id ? "再点一次确认" : "删除" }}
          </button>
        </div>
        <textarea v-model="t.text" class="tpl-text" rows="3"></textarea>
      </article>
    </div>
  </div>
</template>

<style scoped>
.template-page {
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 16px 20px;
  overflow-y: auto;
}
.page-header h3 {
  margin: 0;
  font-size: 16px;
}
.subtitle {
  margin: 4px 0 14px 0;
  font-size: 12px;
  color: #718096;
}
.subtitle code {
  background: #edf2f7;
  padding: 1px 5px;
  border-radius: 4px;
}
.banner {
  padding: 8px 12px;
  border-radius: 8px;
  font-size: 13px;
  margin-bottom: 10px;
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
  margin-bottom: 10px;
  align-items: center;
}
.product-tab {
  padding: 6px 14px;
  border: 1px solid #e2e8f0;
  border-radius: 18px;
  background: white;
  font-size: 13px;
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 6px;
}
.product-tab:hover {
  background: #f7fafc;
}
.product-tab.active {
  border-color: #667eea;
  background: #667eea;
  color: white;
}
.tab-count {
  font-size: 11px;
  background: rgba(0, 0, 0, 0.08);
  border-radius: 10px;
  padding: 0 7px;
  line-height: 16px;
}
.product-tab.active .tab-count {
  background: rgba(255, 255, 255, 0.25);
}
.empty-hint {
  font-size: 12px;
  color: #a0aec0;
}

.product-meta {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12px;
  color: #4a5568;
  margin-bottom: 12px;
  flex-wrap: wrap;
}
.muted {
  color: #a0aec0;
}
.meta-spacer {
  flex: 1;
}
.import-btn,
.import-confirm-btn,
.import-cancel-btn {
  padding: 4px 12px;
  font-size: 12px;
  border-radius: 6px;
  cursor: pointer;
  border: 1px solid #cbd5e0;
  background: white;
}
.import-btn:hover {
  background: #f7fafc;
}
.import-confirm-text {
  font-size: 12px;
}
.import-confirm-text .warn {
  color: #c53030;
}
.import-confirm-btn {
  border-color: #dd6b20;
  background: #dd6b20;
  color: white;
}
.import-cancel-btn:hover {
  background: #f7fafc;
}

.add-card {
  border: 1px dashed #cbd5e0;
  border-radius: 8px;
  padding: 10px 12px;
  margin-bottom: 14px;
  background: #fbfcfe;
}
.add-row {
  display: flex;
  gap: 8px;
  margin-bottom: 8px;
}
.add-category {
  flex: 1;
  padding: 6px 10px;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  font-size: 13px;
}
.add-lang,
.tpl-lang {
  padding: 5px 8px;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  font-size: 12px;
  background: white;
  flex-shrink: 0;
}
.add-btn {
  padding: 6px 16px;
  border: 1px solid #667eea;
  border-radius: 6px;
  background: #667eea;
  color: white;
  font-size: 13px;
  cursor: pointer;
  flex-shrink: 0;
}
.add-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.add-text {
  width: 100%;
  box-sizing: border-box;
  padding: 8px 10px;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  font-size: 13px;
  font-family: inherit;
  resize: vertical;
}

.tpl-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.tpl-card {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 10px 12px;
  background: white;
}
.tpl-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 6px;
}
.tpl-id {
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  font-size: 11px;
  color: #a0aec0;
  flex-shrink: 0;
}
.tpl-category {
  width: 180px;
  padding: 3px 8px;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  font-size: 12px;
}
.tpl-len {
  font-size: 11px;
  color: #a0aec0;
  flex-shrink: 0;
}
.tpl-head-spacer {
  flex: 1;
}
.save-btn,
.del-btn {
  padding: 3px 12px;
  font-size: 12px;
  border-radius: 6px;
  cursor: pointer;
  flex-shrink: 0;
}
.save-btn {
  border: 1px solid #38a169;
  background: white;
  color: #2f855a;
}
.save-btn:hover {
  background: #38a169;
  color: white;
}
.del-btn {
  border: 1px solid #e2e8f0;
  background: white;
  color: #a0aec0;
}
.del-btn:hover {
  border-color: #fc8181;
  color: #c53030;
}
.del-btn.armed {
  border-color: #c53030;
  background: #c53030;
  color: white;
}
.tpl-text {
  width: 100%;
  box-sizing: border-box;
  padding: 8px 10px;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  font-size: 13px;
  line-height: 1.5;
  font-family: inherit;
  resize: vertical;
}
.empty-state {
  padding: 30px;
  text-align: center;
  color: #a0aec0;
  font-size: 13px;
}
</style>

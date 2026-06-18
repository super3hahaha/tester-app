<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { loadFavIds, saveFavIds } from "../utils/templateFavorites";

// 模板管理：分产品增删改 + xlsx 批量导入 + 多语言预翻译。数据存 ~/.tester-app/templates/，
// review-reply skill 命中模板后优先取预存译文（translations），省掉运行时翻译。
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
  translations: Record<string, string>; // 语言码 → 译文
  src_hash: string;
  stale: boolean; // 源文改过、译文未重译
}
interface ImportResult {
  count: number;
  sheet: string;
  warning: string | null;
}
interface TranslateResult {
  templates: number;
  units: number;
  batches: number;
  warnings: string[];
}

// 目标语言清单（app 原生码）。en 是源、不在此列。
const LANGS: { code: string; name: string }[] = [
  { code: "ar", name: "阿拉伯" },
  { code: "cs", name: "捷克" },
  { code: "de", name: "德语" },
  { code: "es", name: "西班牙" },
  { code: "fa", name: "波斯" },
  { code: "fr", name: "法语" },
  { code: "in", name: "印尼" },
  { code: "it", name: "意大利" },
  { code: "ja", name: "日语" },
  { code: "ko", name: "韩语" },
  { code: "ms", name: "马来" },
  { code: "nl", name: "荷兰" },
  { code: "pl", name: "波兰" },
  { code: "pt", name: "葡萄牙" },
  { code: "ro", name: "罗马尼亚" },
  { code: "ru", name: "俄语" },
  { code: "th", name: "泰语" },
  { code: "tr", name: "土耳其" },
  { code: "uk", name: "乌克兰" },
  { code: "vi", name: "越南" },
  { code: "zh-rCN", name: "简体中文" },
  { code: "zh-rTW", name: "繁体中文" },
  // ── 以下为后补语言，默认不勾选（见 DEFAULT_CODES）──
  { code: "bn", name: "孟加拉" },
  { code: "da", name: "丹麦" },
  { code: "el", name: "希腊" },
  { code: "fi", name: "芬兰" },
  { code: "hi", name: "印地" },
  { code: "hu", name: "匈牙利" },
  { code: "kn-rIN", name: "卡纳达" },
  { code: "ml-rIN", name: "马拉雅拉姆" },
  { code: "mr", name: "马拉地" },
  { code: "pa-rIN", name: "旁遮普" },
  { code: "sk", name: "斯洛伐克" },
  { code: "sv", name: "瑞典" },
  { code: "ta-rIN", name: "泰米尔" },
  { code: "te-rIN", name: "泰卢固" },
  { code: "ur-rIN", name: "乌尔都" },
];
const ALL_CODES = LANGS.map((l) => l.code);
// 默认勾选的语言（原始一组）；上面后补的 15 个默认不选，要用时在弹窗里手动勾。
const DEFAULT_CODES = [
  "ar", "cs", "de", "es", "fa", "fr", "in", "it", "ja", "ko", "ms",
  "nl", "pl", "pt", "ro", "ru", "th", "tr", "uk", "vi", "zh-rCN", "zh-rTW",
];
// 翻译用 haiku：纯文本转换任务，haiku 足够且单价是 sonnet 的 1/3；质量不够再换。
const TRANSLATE_MODEL = "claude-haiku-4-5";
const LANGS_STORE_KEY = "tpl-translate-langs";

function langName(code: string): string {
  return LANGS.find((l) => l.code === code)?.name || "";
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

// 新建产品（如「通用」：跨 app 通用模板，翻一次各 app 复用）
const showNewProduct = ref(false);
const newProductName = ref("");

// 编辑行「查看语言」：id → 当前查看的语言码（默认源语言）。
const viewLang = ref<Record<string, string>>({});

// 收藏：标星的模板 id 集合（localStorage 持久化）。收藏的模板会出现在评论页的
// 「模板回复」快捷弹窗里。
const favIds = ref<Set<string>>(loadFavIds());
function isFav(id: string): boolean {
  return favIds.value.has(id);
}
function toggleFav(id: string) {
  const next = new Set(favIds.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  favIds.value = next;
  saveFavIds(next);
}

// 内联两步确认（Tauri webview 里 window.confirm 不弹，见 gotchas.md）
const armedDeleteId = ref("");
let deleteTimer: number | null = null;

// xlsx 导入
const importing = ref(false);
const pendingImportPath = ref("");

// 补全多语言
const showTranslateModal = ref(false);
const selectedLangs = ref<string[]>([...DEFAULT_CODES]);
const overwriteMode = ref(false); // false=只补缺失（追加）, true=覆盖重译
const translating = ref(false);
const translateDone = ref(false); // 本次批量翻译已完成 → 按钮变「好的」
const translateMinimized = ref(false); // 弹窗缩小成右下浮条（仍在翻、编辑仍禁用）
const translateLog = ref<string[]>([]);
// 批量进度（来自后端 translate-progress 事件，按译文单元计数）
const progressTotal = ref(0);
const progressDone = ref(0);
const progressPct = computed(() =>
  progressTotal.value ? Math.round((progressDone.value / progressTotal.value) * 100) : 0
);
// 单条重译进度（前端分组驱动）
const retransId = ref("");
const retransDone = ref(0);
const retransTotal = ref(0);

const selectedInfo = computed(() =>
  products.value.find((p) => p.product === selectedProduct.value) || null
);

// 编辑行当前查看/编辑的语言与内容（源语言→text，否则→译文）
function curLang(t: Template): string {
  return viewLang.value[t.id] || t.lang;
}
function curText(t: Template): string {
  const l = curLang(t);
  return l === t.lang ? t.text : t.translations?.[l] ?? "";
}
function setCurText(t: Template, val: string) {
  const l = curLang(t);
  if (l === t.lang) {
    t.text = val;
  } else {
    if (!t.translations) t.translations = {};
    t.translations[l] = val;
  }
}

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

function startNewProduct() {
  showNewProduct.value = true;
  newProductName.value = "";
}
function cancelNewProduct() {
  showNewProduct.value = false;
  newProductName.value = "";
}
// 新建产品：前端临时占位，加第一条模板后由 add_template 真正落地（loadProducts 刷新）。
function createProduct() {
  const name = newProductName.value.trim();
  if (!name) return;
  showNewProduct.value = false;
  newProductName.value = "";
  if (!products.value.some((p) => p.product === name)) {
    products.value.push({ product: name, count: 0, apps: [] });
  }
  selectProduct(name);
  flash(`已切到「${name}」，加第一条模板即创建该产品`);
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
    flash("已新增模板（到「补全多语言」生成译文）");
  } catch (e: any) {
    error.value = String(e);
  }
}

async function saveTemplate(t: Template) {
  error.value = "";
  const l = curLang(t);
  try {
    // 在译文视图下编辑：先存该语言译文（非空才存）
    if (l !== t.lang) {
      const v = t.translations?.[l] ?? "";
      if (v.trim()) {
        await invoke("set_template_translation", {
          product: selectedProduct.value,
          id: t.id,
          lang: l,
          text: v,
        });
      }
    }
    // 始终回存类别（+ 源正文/源语言）。源正文改了会让译文标过期。
    await invoke("update_template", {
      product: selectedProduct.value,
      id: t.id,
      category: t.category,
      text: t.text,
      lang: t.lang,
    });
    await loadTemplates(); // 刷新 stale / 覆盖度
    flash(`已保存 ${t.id}`);
  } catch (e: any) {
    error.value = String(e);
  }
}

function armDelete(id: string) {
  if (armedDeleteId.value === id) {
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

// 单条重译：该条目标语言覆盖重译。语言切成小组多次调用，进度按组推进、每段更快返回。
async function retranslateOne(t: Template) {
  if (translating.value) return;
  // 只刷新该条已有的语言；从没翻过则用默认集铺底。不引入未勾选的后补语种。
  const existing = Object.keys(t.translations || {});
  const langs = existing.length ? existing : [...DEFAULT_CODES];
  translating.value = true;
  translateLog.value = [];
  retransId.value = t.id;
  retransTotal.value = langs.length;
  retransDone.value = 0;
  error.value = "";
  try {
    const GROUP = 8;
    for (let i = 0; i < langs.length; i += GROUP) {
      const group = langs.slice(i, i + GROUP);
      await invoke<TranslateResult>("translate_templates", {
        product: selectedProduct.value,
        ids: [t.id],
        langs: group,
        overwrite: true,
        channel: "gp",
        model: TRANSLATE_MODEL,
      });
      retransDone.value = Math.min(langs.length, i + group.length);
      await loadTemplates(); // 每组写回后刷新该条覆盖度
    }
    // 单条重译没弹窗看不到日志 → 完成后扫该条仍超 350 的语言，红色 banner 提示
    const fresh = templates.value.find((x) => x.id === t.id);
    const over = fresh
      ? Object.entries(fresh.translations || {})
          .filter(([, v]) => (v as string).length > 350)
          .map(([l]) => l)
      : [];
    if (over.length) {
      error.value = `${t.id}：${over.join("/")} 仍超 350 字符（已标红），请手动精简`;
    } else {
      flash(`已重译 ${t.id}（${retransDone.value} 种语言）`);
    }
  } catch (e: any) {
    error.value = e === "CANCELLED" ? "已取消（已完成的组已保存）" : String(e);
    await loadTemplates();
  } finally {
    translating.value = false;
    retransId.value = "";
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
    let msg = `已导入 ${res.count} 条（sheet：${res.sheet}）· 译文已清空，记得「补全多语言」`;
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

// ── 补全多语言弹窗 ──

function openTranslateModal() {
  translateLog.value = [];
  translateDone.value = false;
  translateMinimized.value = false;
  showTranslateModal.value = true;
}
function closeTranslateModal() {
  if (translating.value) return; // 翻译中不让关
  translateDone.value = false;
  translateMinimized.value = false;
  showTranslateModal.value = false;
}
function toggleLang(code: string) {
  const i = selectedLangs.value.indexOf(code);
  if (i >= 0) selectedLangs.value.splice(i, 1);
  else selectedLangs.value.push(code);
}
function selectAllLangs() {
  selectedLangs.value = [...ALL_CODES];
}
function clearLangs() {
  selectedLangs.value = [];
}

async function runBatchTranslate() {
  if (!selectedProduct.value || translating.value) return;
  if (selectedLangs.value.length === 0) {
    error.value = "请至少选择一个语言。";
    return;
  }
  translating.value = true;
  translateDone.value = false;
  translateLog.value = [];
  progressTotal.value = 0;
  progressDone.value = 0;
  error.value = "";
  try {
    const r = await invoke<TranslateResult>("translate_templates", {
      product: selectedProduct.value,
      ids: null,
      langs: selectedLangs.value,
      overwrite: overwriteMode.value,
      channel: "gp",
      model: TRANSLATE_MODEL,
    });
    await loadTemplates();
    translateDone.value = true;
    translateMinimized.value = false; // 完成自动弹回大窗，显示「好的」
    const overCount = templates.value.reduce(
      (n, t) =>
        n + Object.values(t.translations || {}).filter((v) => (v as string).length > 350).length,
      0
    );
    let msg = `补全完成：${r.templates} 条模板 / ${r.units} 条译文`;
    if (overCount) msg += ` · ⚠ ${overCount} 条仍超 350（已标红）`;
    flash(msg);
  } catch (e: any) {
    error.value = e === "CANCELLED" ? "已取消（已完成的批次已保存）" : String(e);
  } finally {
    translating.value = false;
  }
}

function stopTranslate() {
  invoke("stop_translate").catch(() => {});
}

// 记住上次选的语言
watch(
  selectedLangs,
  (v) => {
    translateDone.value = false; // 改了语言 → 恢复「开始翻译」
    try {
      localStorage.setItem(LANGS_STORE_KEY, JSON.stringify(v));
    } catch {}
  },
  { deep: true }
);
watch(overwriteMode, () => {
  translateDone.value = false; // 改了覆盖/补缺失模式 → 恢复「开始翻译」
});

let unlistenLog: UnlistenFn | null = null;
let unlistenProgress: UnlistenFn | null = null;

onMounted(async () => {
  try {
    const saved = localStorage.getItem(LANGS_STORE_KEY);
    if (saved) {
      const arr = JSON.parse(saved);
      if (Array.isArray(arr) && arr.length) {
        selectedLangs.value = arr.filter((c: string) => ALL_CODES.includes(c));
      }
    }
  } catch {}
  unlistenLog = await listen<{ text: string; kind: string; done: boolean }>(
    "translate-log",
    (e) => {
      translateLog.value.push(e.payload.text);
      if (translateLog.value.length > 300) {
        translateLog.value.splice(0, translateLog.value.length - 300);
      }
    }
  );
  unlistenProgress = await listen<{ total: number; done: number }>(
    "translate-progress",
    (e) => {
      progressTotal.value = e.payload.total;
      progressDone.value = e.payload.done;
    }
  );
  loadProducts();
});

onUnmounted(() => {
  if (unlistenLog) unlistenLog();
  if (unlistenProgress) unlistenProgress();
});

// MainPage 用 v-show，组件常驻；切回本页时刷新一次
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
        分产品维护 Google Play 回复模板 + 预翻译多语言。改动存本地
        <code>~/.tester-app/templates/</code>，批量回复时 skill 命中模板优先取预存译文。
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
      <template v-if="!showNewProduct">
        <button class="new-product-btn" :disabled="translating" @click="startNewProduct">
          + 新建产品
        </button>
      </template>
      <template v-else>
        <input
          v-model="newProductName"
          class="new-product-input"
          placeholder="产品名（如 通用）"
          @keyup.enter="createProduct"
        />
        <button class="np-ok" @click="createProduct">建</button>
        <button class="np-cancel" @click="cancelNewProduct">取消</button>
      </template>
    </div>

    <div v-if="selectedInfo" class="product-meta">
      <span v-if="selectedInfo.apps.length">
        关联应用：{{ selectedInfo.apps.join("、") }}
      </span>
      <span v-else class="muted">（package_map 里暂无关联应用）</span>

      <div class="meta-spacer"></div>

      <button class="translate-btn" :disabled="translating" @click="openTranslateModal">
        🌐 补全多语言
      </button>

      <!-- xlsx 导入 -->
      <template v-if="!pendingImportPath">
        <button class="import-btn" :disabled="importing || translating" @click="pickXlsx">
          📥 从 xlsx 导入
        </button>
      </template>
      <template v-else>
        <span class="import-confirm-text">
          将用 <b>{{ importFileName }}</b> <b class="warn">覆盖</b>「{{ selectedProduct }}」现有模板
        </span>
        <button
          class="import-confirm-btn"
          :disabled="importing || translating"
          @click="confirmImport"
        >
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
          :disabled="translating"
        />
        <select v-model="newLang" class="add-lang" title="模板源语言" :disabled="translating">
          <option value="en">英文 en</option>
          <option value="zh-CN">中文 zh-CN</option>
        </select>
        <button
          class="add-btn"
          :disabled="!newText.trim() || translating"
          @click="addTemplate"
        >
          + 新增模板
        </button>
      </div>
      <textarea
        v-model="newText"
        class="add-text"
        rows="2"
        :disabled="translating"
        placeholder="模板源语言正文（新增后到「补全多语言」生成各语言译文）"
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
          <button
            class="fav-btn"
            :class="{ on: isFav(t.id) }"
            :title="isFav(t.id) ? '取消收藏（不再出现在「模板回复」弹窗）' : '收藏（出现在评论页「模板回复」弹窗）'"
            @click="toggleFav(t.id)"
          >{{ isFav(t.id) ? "★" : "☆" }}</button>
          <span class="tpl-id">{{ t.id }}</span>
          <input
            v-model="t.category"
            class="tpl-category"
            placeholder="类别"
            :disabled="translating"
          />
          <select
            :value="curLang(t)"
            class="tpl-lang"
            title="查看/编辑某语言（源语言 + 已翻译语言）"
            :disabled="translating"
            @change="viewLang[t.id] = ($event.target as HTMLSelectElement).value"
          >
            <option :value="t.lang">源 {{ t.lang }}</option>
            <option
              v-for="code in Object.keys(t.translations || {})"
              :key="code"
              :value="code"
            >
              {{ code }} {{ langName(code) }}
            </option>
          </select>
          <span
            class="tpl-cov"
            :class="{ stale: t.stale }"
            :title="t.stale ? '源文改过、译文未重译' : '已翻译语言数'"
          >
            🌐 {{ Object.keys(t.translations || {}).length }}<template v-if="t.stale"> · 源已改</template>
          </span>
          <span class="tpl-len" :class="{ over: curText(t).length > 350 }">
            {{ curText(t).length }} 字符
          </span>
          <div class="tpl-head-spacer"></div>
          <button
            class="retrans-btn"
            :class="{ stale: t.stale, busy: retransId === t.id }"
            :disabled="translating"
            title="重译该条所有语言（覆盖）"
            @click="retranslateOne(t)"
          >
            {{ retransId === t.id ? `翻译中 ${retransDone}/${retransTotal}` : "重译" }}
          </button>
          <button class="save-btn" :disabled="translating" @click="saveTemplate(t)">
            保存
          </button>
          <button
            class="del-btn"
            :class="{ armed: armedDeleteId === t.id }"
            :disabled="translating"
            @click="armDelete(t.id)"
          >
            {{ armedDeleteId === t.id ? "再点一次确认" : "删除" }}
          </button>
        </div>
        <textarea
          :value="curText(t)"
          class="tpl-text"
          rows="3"
          :disabled="translating"
          :placeholder="curLang(t) === t.lang ? '源正文' : `${curLang(t)} 译文`"
          @input="setCurText(t, ($event.target as HTMLTextAreaElement).value)"
        ></textarea>
      </article>
    </div>

    <!-- 补全多语言弹窗 -->
    <div
      v-if="showTranslateModal && !translateMinimized"
      class="modal-overlay"
      @click.self="closeTranslateModal"
    >
      <div class="modal">
        <div class="modal-head">
          <h4>补全多语言 · {{ selectedProduct }}</h4>
          <button
            v-if="translating"
            class="modal-min"
            title="缩小（后台继续翻译）"
            @click="translateMinimized = true"
          >
            —
          </button>
          <button class="modal-close" :disabled="translating" @click="closeTranslateModal">
            ✕
          </button>
        </div>

        <div class="modal-body">
          <div class="lang-toolbar">
            <span>选择目标语言（{{ selectedLangs.length }}/{{ ALL_CODES.length }}）</span>
            <div class="meta-spacer"></div>
            <button class="mini-btn" :disabled="translating" @click="selectAllLangs">
              全选
            </button>
            <button class="mini-btn" :disabled="translating" @click="clearLangs">
              清空
            </button>
          </div>
          <div class="lang-grid">
            <label
              v-for="l in LANGS"
              :key="l.code"
              class="lang-chip"
              :class="{ on: selectedLangs.includes(l.code) }"
            >
              <input
                type="checkbox"
                :checked="selectedLangs.includes(l.code)"
                :disabled="translating"
                @change="toggleLang(l.code)"
              />
              {{ l.code }}<span class="lang-cn">{{ l.name }}</span>
            </label>
          </div>

          <div class="mode-row">
            <label>
              <input type="radio" :value="false" v-model="overwriteMode" :disabled="translating" />
              只补缺失（保留已有译文、追加新语言）
            </label>
            <label>
              <input type="radio" :value="true" v-model="overwriteMode" :disabled="translating" />
              覆盖重译（全部重新翻译）
            </label>
          </div>

          <p class="est">
            约 {{ templates.length }} 条模板 × {{ selectedLangs.length }} 语言
            <template v-if="!overwriteMode">（只补缺失，实际更少）</template>
          </p>

          <div v-if="translating || progressDone" class="progress-row">
            <div class="progress-track">
              <div class="progress-fill" :style="{ width: progressPct + '%' }"></div>
            </div>
            <span class="progress-text">
              {{ progressDone }}/{{ progressTotal }} 译文（{{ progressPct }}%）
            </span>
          </div>

          <div v-if="translateLog.length" class="log-box">
            <div v-for="(line, i) in translateLog" :key="i" class="log-line">{{ line }}</div>
          </div>
        </div>

        <div class="modal-foot">
          <button v-if="translating" class="stop-btn" @click="stopTranslate">停止</button>
          <button
            v-if="translateDone && !translating"
            class="run-btn"
            @click="closeTranslateModal"
          >
            好的
          </button>
          <button
            v-else
            class="run-btn"
            :disabled="translating || selectedLangs.length === 0"
            @click="runBatchTranslate"
          >
            {{ translating ? "翻译中…" : "开始翻译" }}
          </button>
        </div>
      </div>
    </div>

    <!-- 缩小后的右下浮条（仍在翻译，编辑仍禁用） -->
    <div
      v-if="showTranslateModal && translateMinimized"
      class="mini-bar"
      @click="translateMinimized = false"
    >
      <div class="mini-top">
        <span class="mini-title">补全多语言 · {{ selectedProduct }}</span>
        <span class="mini-pct">{{ progressPct }}%</span>
      </div>
      <div class="progress-track">
        <div class="progress-fill" :style="{ width: progressPct + '%' }"></div>
      </div>
      <div class="mini-actions">
        <span class="mini-count">{{ progressDone }}/{{ progressTotal }}</span>
        <div class="meta-spacer"></div>
        <button v-if="translating" class="mini-stop" @click.stop="stopTranslate">停止</button>
        <button class="mini-expand" @click.stop="translateMinimized = false">展开</button>
      </div>
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
.new-product-btn {
  padding: 6px 12px;
  border: 1px dashed #cbd5e0;
  border-radius: 18px;
  background: white;
  font-size: 13px;
  color: #5a67d8;
  cursor: pointer;
}
.new-product-btn:hover {
  background: #eef0fc;
}
.new-product-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.new-product-input {
  padding: 5px 10px;
  border: 1px solid #667eea;
  border-radius: 14px;
  font-size: 13px;
  width: 150px;
}
.np-ok,
.np-cancel {
  padding: 5px 12px;
  font-size: 12px;
  border-radius: 14px;
  cursor: pointer;
  border: 1px solid #cbd5e0;
  background: white;
}
.np-ok {
  border-color: #667eea;
  background: #667eea;
  color: white;
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
.translate-btn,
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
.translate-btn {
  border-color: #667eea;
  color: #5a67d8;
}
.translate-btn:hover {
  background: #eef0fc;
}
.translate-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
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
.fav-btn {
  border: none;
  background: none;
  cursor: pointer;
  font-size: 16px;
  line-height: 1;
  color: #cbd5e0;
  padding: 0 2px;
  flex-shrink: 0;
}
.fav-btn.on {
  color: #f6ad55;
}
.fav-btn:hover {
  color: #f6ad55;
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
.tpl-cov {
  font-size: 11px;
  color: #5a67d8;
  background: #eef0fc;
  border-radius: 10px;
  padding: 1px 8px;
  flex-shrink: 0;
}
.tpl-cov.stale {
  color: #c05621;
  background: #feebc8;
}
.tpl-len {
  font-size: 11px;
  color: #a0aec0;
  flex-shrink: 0;
}
.tpl-len.over {
  color: #c53030;
  font-weight: 600;
}
.tpl-head-spacer {
  flex: 1;
}
.retrans-btn,
.save-btn,
.del-btn {
  padding: 3px 12px;
  font-size: 12px;
  border-radius: 6px;
  cursor: pointer;
  flex-shrink: 0;
}
.retrans-btn {
  border: 1px solid #cbd5e0;
  background: white;
  color: #5a67d8;
}
.retrans-btn:hover {
  background: #eef0fc;
}
.retrans-btn.stale {
  border-color: #dd6b20;
  color: #c05621;
  background: #feebc8;
}
.retrans-btn.busy {
  border-color: #667eea;
  color: #5a67d8;
  background: #eef0fc;
}
.retrans-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
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
/* 翻译进行中：编辑控件置灰禁用 */
.save-btn:disabled,
.del-btn:disabled,
.add-btn:disabled,
.add-lang:disabled,
.add-category:disabled,
.add-text:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.tpl-text:disabled,
.tpl-category:disabled,
.tpl-lang:disabled {
  background: #f7fafc;
  color: #a0aec0;
  cursor: not-allowed;
}
.empty-state {
  padding: 30px;
  text-align: center;
  color: #a0aec0;
  font-size: 13px;
}

/* 补全多语言弹窗 */
.modal-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.35);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}
.modal {
  width: 560px;
  max-width: 92vw;
  max-height: 86vh;
  background: white;
  border-radius: 12px;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.25);
}
.modal-head {
  display: flex;
  align-items: center;
  padding: 12px 16px;
  border-bottom: 1px solid #edf2f7;
}
.modal-head h4 {
  margin: 0;
  font-size: 14px;
  flex: 1;
}
.modal-min {
  border: none;
  background: none;
  font-size: 18px;
  line-height: 1;
  cursor: pointer;
  color: #a0aec0;
  margin-right: 4px;
}
.modal-min:hover {
  color: #4a5568;
}
.modal-close {
  border: none;
  background: none;
  font-size: 16px;
  cursor: pointer;
  color: #a0aec0;
}
.modal-body {
  padding: 14px 16px;
  overflow-y: auto;
}
.lang-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: #4a5568;
  margin-bottom: 8px;
}
.mini-btn {
  padding: 2px 10px;
  font-size: 11px;
  border: 1px solid #cbd5e0;
  border-radius: 5px;
  background: white;
  cursor: pointer;
}
.mini-btn:hover {
  background: #f7fafc;
}
.lang-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 6px;
  margin-bottom: 12px;
}
.lang-chip {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  padding: 4px 6px;
  cursor: pointer;
  background: white;
}
.lang-chip.on {
  border-color: #667eea;
  background: #eef0fc;
}
.lang-chip .lang-cn {
  color: #a0aec0;
  font-size: 11px;
}
.mode-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 12px;
  color: #4a5568;
  margin-bottom: 8px;
}
.est {
  font-size: 12px;
  color: #718096;
  margin: 4px 0 8px;
}
.progress-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.progress-track {
  flex: 1;
  height: 8px;
  background: #e2e8f0;
  border-radius: 4px;
  overflow: hidden;
}
.progress-fill {
  height: 100%;
  background: #667eea;
  border-radius: 4px;
  transition: width 0.3s ease;
}
.progress-text {
  font-size: 11px;
  color: #4a5568;
  white-space: nowrap;
  flex-shrink: 0;
}
.log-box {
  background: #1a202c;
  color: #cbd5e0;
  border-radius: 8px;
  padding: 8px 10px;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  font-size: 11px;
  max-height: 180px;
  overflow-y: auto;
}
.log-line {
  white-space: pre-wrap;
  word-break: break-all;
  line-height: 1.4;
}
.modal-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 16px;
  border-top: 1px solid #edf2f7;
}
.run-btn {
  padding: 6px 18px;
  border: 1px solid #667eea;
  background: #667eea;
  color: white;
  border-radius: 6px;
  font-size: 13px;
  cursor: pointer;
}
.run-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.stop-btn {
  padding: 6px 18px;
  border: 1px solid #c53030;
  background: white;
  color: #c53030;
  border-radius: 6px;
  font-size: 13px;
  cursor: pointer;
}

/* 缩小后的右下浮条 */
.mini-bar {
  position: fixed;
  right: 20px;
  bottom: 20px;
  width: 280px;
  background: white;
  border: 1px solid #e2e8f0;
  border-radius: 10px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
  padding: 10px 12px;
  z-index: 100;
  cursor: pointer;
}
.mini-top {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 12px;
  margin-bottom: 6px;
}
.mini-title {
  color: #2d3748;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mini-pct {
  color: #5a67d8;
  flex-shrink: 0;
  margin-left: 8px;
}
.mini-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
  font-size: 11px;
}
.mini-count {
  color: #718096;
}
.mini-stop,
.mini-expand {
  padding: 2px 10px;
  font-size: 11px;
  border-radius: 5px;
  cursor: pointer;
  border: 1px solid #cbd5e0;
  background: white;
}
.mini-stop {
  border-color: #c53030;
  color: #c53030;
}
</style>

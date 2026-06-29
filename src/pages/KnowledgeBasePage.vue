<script setup lang="ts">
import { ref, computed, watch, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

interface KbProduct {
  id: string;
  name: string;
}

interface KbDoc {
  id: string;
  name: string;
  productIds: string[];
  scenes: string[];
}

const SKELETON_GENERAL = `# 通用用例偏好（适用于我负责的所有产品）

## 写作风格
- 描述列：用「查看/检查/点击/输入」等操作动词，不用「是否为」「验证是否」判断句。
- 预期列：直接写结论状态，不重复动作，不用「正确显示」等模糊表述。
- UI 用例：不堆需求原文，去掉冗余的需求文字描述，只留可验证的界面要点。

## 用例顺序
- 按模块测试习惯排序，不要照需求文档的叙述顺序排。
- <写下习惯的模块顺序，如：UI → 核心交互 → 异常/边界 → 跨功能交叉>

## 必测通则（每类功能都要覆盖）
- 弹窗：除点弹窗内按钮外，必须覆盖「点空白处关闭」「点物理返回键关闭」。
- 升级测试：涉及新功能/新数据结构时，覆盖从旧版本升级上来的场景。
- 交叉测试：新功能与其他 tab / 已有功能的交叉影响。
- 断网 / 清数据：涉及云端或缓存的功能，覆盖断网、清除数据后的表现。

## 去重
- 需求多处重复的权益/规则（如免费 vs 会员权益），合并成一条，别每处各生成一条。

## 红线
- 不编造需求未提供的信息：版本号、价格、数值、业务术语不确定就标出来问，不臆测。`;

const SKELETON_PRODUCT = `# <产品名> 用例偏好

## 定位
<一句话：这个产品/这次需求做什么，核心功能与关键概念>

## 关键业务规则（容易理解错的，先写清楚）
- <如：加油包必须是会员才能购买>
- <如：会员到期但加油包未到期时，加油包是否仍可用——写明实际逻辑>
- <如：年订说明文案里替换的是「X 价格」，不是「Y 价格」>

## 必测场景清单（本产品/需求易遗漏的点）
- <如：购买项按不同国家显示当地货币价格>
- <如：已购旧 SKU 的升级用户——订阅取消/未取消、三天试用资格、从 GP 重新订阅旧 SKU>
- <如：字幕支持的语言检测>

## 需关联测试的点（别孤立测）
- <如：字幕样式每个选项设置后，关联检查字幕实际表现>
- <如：画质增强 + 小窗播放 / 后台播放 组合下的表现>
- <如：倍速 与 片段轨道 的关联逻辑>

## 模块命名 / 分群约定
<本产品用例的模块怎么命名、怎么分群，与已有用例保持一致>`;

const props = defineProps<{
  viewId: string;         // 产品 id 或 "common"
  activeOption: string;
}>();

const emit = defineEmits<{
  (e: "products-changed"): void;
}>();

const products = ref<KbProduct[]>([]);
const allDocs = ref<KbDoc[]>([]);

// 当前视图下的资料列表
const viewDocs = computed(() => {
  if (props.viewId === "common") {
    return allDocs.value.filter((d) => d.productIds.length === 0);
  }
  return allDocs.value.filter((d) => d.productIds.includes(props.viewId));
});

const currentProduct = computed(() =>
  products.value.find((p) => p.id === props.viewId) || null
);

const activeDocId = ref<string | null>(null);
const activeDoc = computed(() =>
  allDocs.value.find((d) => d.id === activeDocId.value) || null
);

const editContent = ref("");
const savedContent = ref("");
const dirty = computed(() => editContent.value !== savedContent.value);
const saving = ref(false);
const error = ref("");

// 管理关联弹窗
const showAssocModal = ref(false);
const assocDocId = ref<string | null>(null);
const assocChecked = ref<string[]>([]);

// 内联重命名
const renamingDocId = ref<string | null>(null);
const renameDocValue = ref("");

// 产品重命名
const renamingProduct = ref(false);
const renameProductValue = ref("");

// 通用 inline 输入弹窗
interface InlinePrompt {
  title: string;
  placeholder: string;
  value: string;
  resolve: (v: string | null) => void;
}
const inlinePrompt = ref<InlinePrompt | null>(null);

function showPrompt(title: string, placeholder = ""): Promise<string | null> {
  return new Promise((resolve) => {
    inlinePrompt.value = { title, placeholder, value: "", resolve };
    nextTick(() => {
      (document.getElementById("inline-prompt-input") as HTMLInputElement)?.focus();
    });
  });
}

function confirmPrompt() {
  const v = inlinePrompt.value?.value.trim() || null;
  inlinePrompt.value?.resolve(v || null);
  inlinePrompt.value = null;
}

function cancelPrompt() {
  inlinePrompt.value?.resolve(null);
  inlinePrompt.value = null;
}

// 通用 inline 确认弹窗
interface InlineConfirm {
  message: string;
  resolve: (ok: boolean) => void;
}
const inlineConfirm = ref<InlineConfirm | null>(null);

function showConfirm(message: string): Promise<boolean> {
  return new Promise((resolve) => {
    inlineConfirm.value = { message, resolve };
  });
}

function okConfirm() {
  inlineConfirm.value?.resolve(true);
  inlineConfirm.value = null;
}

function cancelConfirm() {
  inlineConfirm.value?.resolve(false);
  inlineConfirm.value = null;
}


// ── AI 起草/合并 (v2) ─────────────────────────────────────────────────────────
const showDistillModal = ref(false);
const distillImages = ref<string[]>([]);   // 磁盘路径
const distillNote = ref("");
const distilling = ref(false);
const distillError = ref("");

function openDistillModal() {
  if (!activeDocId.value) return;
  distillImages.value = [];
  distillNote.value = "";
  distillError.value = "";
  showDistillModal.value = true;
}

function closeDistillModal() {
  if (distilling.value) return;
  showDistillModal.value = false;
}

async function pickDistillImages() {
  try {
    const selected = await open({
      multiple: true,
      filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "webp", "gif"] }],
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    distillImages.value.push(...paths);
  } catch (e) {
    distillError.value = String(e);
  }
}

// 粘贴截图：从剪贴板取图片 → base64 → 后端落盘拿路径
async function onDistillPaste(e: ClipboardEvent) {
  const items = e.clipboardData?.items;
  if (!items) return;
  for (const item of items) {
    if (item.type.startsWith("image/")) {
      e.preventDefault();
      const file = item.getAsFile();
      if (!file) continue;
      try {
        const buf = await file.arrayBuffer();
        let binary = "";
        const bytes = new Uint8Array(buf);
        for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
        const b64 = btoa(binary);
        const ext = item.type.split("/")[1] || "png";
        const path = await invoke<string>("kb_save_temp_image", {
          dataBase64: b64,
          ext,
        });
        distillImages.value.push(path);
      } catch (err) {
        distillError.value = String(err);
      }
    }
  }
}

function removeDistillImage(idx: number) {
  distillImages.value.splice(idx, 1);
}

function imageName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

async function runDistill() {
  if (distillImages.value.length === 0 && !distillNote.value.trim()) {
    distillError.value = "请添加对比图，或在说明里直接描述偏好。";
    return;
  }
  distilling.value = true;
  distillError.value = "";
  try {
    const draft = await invoke<string>("kb_ai_distill", {
      imagePaths: [...distillImages.value],
      note: distillNote.value,
      existingMd: editContent.value,
      model: null,
    });
    editContent.value = draft;   // 填回编辑器当草稿，dirty 自动变红，不自动保存
    showDistillModal.value = false;
  } catch (e) {
    distillError.value = String(e);
  } finally {
    distilling.value = false;
  }
}

async function loadData() {
  error.value = "";
  try {
    const [prods, docs] = await Promise.all([
      invoke<KbProduct[]>("kb_list_products"),
      invoke<KbDoc[]>("kb_list_docs"),
    ]);
    products.value = prods;
    allDocs.value = docs;
  } catch (e) {
    error.value = String(e);
  }
}

async function selectDoc(id: string, force = false) {
  if (!force && id === activeDocId.value) return;
  if (!force && dirty.value && !await showConfirm("当前资料有未保存改动，切换将丢弃。继续？")) return;
  activeDocId.value = id;
  error.value = "";
  try {
    const content = await invoke<string>("kb_read_doc", { id });
    editContent.value = content;
    savedContent.value = content;
  } catch (e) {
    error.value = String(e);
  }
}

// viewId 变化时重新确定默认选中 tab
watch(
  () => props.viewId,
  async () => {
    await loadData();
    const docs = viewDocs.value;
    if (docs.length > 0) {
      if (!docs.some((d) => d.id === activeDocId.value)) {
        await selectDoc(docs[0].id, true);
      }
    } else {
      activeDocId.value = null;
      editContent.value = "";
      savedContent.value = "";
    }
  },
  { immediate: true }
);

// activeOption 变化（用户从其他页切回）时刷新数据
watch(
  () => props.activeOption,
  async (v) => {
    if (v.startsWith("kb-view:")) {
      await loadData();
    }
  }
);

async function save() {
  if (!activeDocId.value || saving.value) return;
  saving.value = true;
  error.value = "";
  try {
    await invoke("kb_save_doc", { id: activeDocId.value, content: editContent.value });
    savedContent.value = editContent.value;
  } catch (e) {
    error.value = String(e);
  } finally {
    saving.value = false;
  }
}

async function insertSkeleton(type: "general" | "product") {
  if (editContent.value.trim() && !await showConfirm("当前已有内容，用骨架覆盖？")) return;
  editContent.value = type === "general" ? SKELETON_GENERAL : SKELETON_PRODUCT;
}

async function createDoc() {
  const name = await showPrompt("新建资料名称", "如：IAP测试通则");
  if (!name) return;
  try {
    const initialProductIds =
      props.viewId === "common" ? [] : [props.viewId];
    const doc = await invoke<KbDoc>("kb_create_doc", {
      name,
      productIds: initialProductIds,
    });
    allDocs.value.push(doc);
    await selectDoc(doc.id, true);
  } catch (e) {
    error.value = String(e);
  }
}

function startRenameDoc(doc: KbDoc) {
  renamingDocId.value = doc.id;
  renameDocValue.value = doc.name;
  nextTick(() => {
    (document.getElementById(`rename-input-${doc.id}`) as HTMLInputElement)?.focus();
  });
}

async function finishRenameDoc(id: string) {
  const name = renameDocValue.value.trim();
  if (!name) { renamingDocId.value = null; return; }
  try {
    await invoke("kb_rename_doc", { id, name });
    const doc = allDocs.value.find((d) => d.id === id);
    if (doc) doc.name = name;
  } catch (e) {
    error.value = String(e);
  } finally {
    renamingDocId.value = null;
  }
}

async function deleteDoc(doc: KbDoc) {
  if (!await showConfirm(`删除资料「${doc.name}」？此操作不可撤销。`)) return;
  try {
    await invoke("kb_delete_doc", { id: doc.id });
    allDocs.value = allDocs.value.filter((d) => d.id !== doc.id);
    if (activeDocId.value === doc.id) {
      const remaining = viewDocs.value;
      if (remaining.length > 0) {
        await selectDoc(remaining[0].id, true);
      } else {
        activeDocId.value = null;
        editContent.value = "";
        savedContent.value = "";
      }
    }
  } catch (e) {
    error.value = String(e);
  }
}

// ── 管理关联弹窗 ─────────────────────────────────────────────────────────────

function openAssocModal(doc: KbDoc) {
  assocDocId.value = doc.id;
  assocChecked.value = [...doc.productIds];
  showAssocModal.value = true;
}

function closeAssocModal() {
  showAssocModal.value = false;
  assocDocId.value = null;
}

async function saveAssoc() {
  if (!assocDocId.value) return;
  try {
    const productIds = [...assocChecked.value];
    await invoke("kb_set_doc_products", { docId: assocDocId.value, productIds });
    const doc = allDocs.value.find((d) => d.id === assocDocId.value);
    if (doc) doc.productIds = productIds;
    closeAssocModal();
    emit("products-changed");
    // 重新加载（关联变动可能让资料在当前视图消失）
    await loadData();
    const docs = viewDocs.value;
    if (docs.length > 0 && !docs.some((d) => d.id === activeDocId.value)) {
      await selectDoc(docs[0].id, true);
    } else if (docs.length === 0) {
      activeDocId.value = null;
      editContent.value = "";
      savedContent.value = "";
    }
  } catch (e) {
    error.value = String(e);
  }
}

async function addProductInAssocModal() {
  const name = await showPrompt("新建产品名称", "如：XPlayer");
  if (!name) return;
  try {
    const prod = await invoke<KbProduct>("kb_create_product", { name });
    products.value.push(prod);
    assocChecked.value.push(prod.id);
    emit("products-changed");
  } catch (e) {
    error.value = String(e);
  }
}

// ── 产品管理（头部）────────────────────────────────────────────────────────────

function startRenameProduct() {
  renamingProduct.value = true;
  renameProductValue.value = currentProduct.value?.name || "";
  nextTick(() => {
    (document.getElementById("rename-product-input") as HTMLInputElement)?.focus();
  });
}

async function finishRenameProduct() {
  const name = renameProductValue.value.trim();
  renamingProduct.value = false;
  if (!name || !currentProduct.value || name === currentProduct.value.name) return;
  try {
    await invoke("kb_rename_product", { id: currentProduct.value.id, name });
    currentProduct.value.name = name;
    emit("products-changed");
  } catch (e) {
    error.value = String(e);
  }
}

async function deleteCurrentProduct() {
  if (!currentProduct.value) return;
  const p = currentProduct.value;
  if (!await showConfirm(`删除产品「${p.name}」？关联该产品的资料会自动解除关联（内容不删除）。`)) return;
  try {
    await invoke("kb_delete_product", { id: p.id });
    emit("products-changed");
    // MainPage 会切换回 common 视图
  } catch (e) {
    error.value = String(e);
  }
}
</script>

<template>
  <div class="kb-page">
    <!-- 头部：产品信息 + 操作 -->
    <div class="kb-header" v-if="viewId !== 'common'">
      <div class="product-title-row">
        <template v-if="renamingProduct">
          <input
            id="rename-product-input"
            v-model="renameProductValue"
            class="product-rename-input"
            @blur="finishRenameProduct"
            @keydown.enter="finishRenameProduct"
            @keydown.escape="renamingProduct = false"
          />
        </template>
        <template v-else>
          <h3 class="product-title">{{ currentProduct?.name || viewId }}</h3>
          <button class="icon-btn" title="重命名产品" @click="startRenameProduct">✏️</button>
          <button class="icon-btn danger" title="删除产品" @click="deleteCurrentProduct">🗑</button>
        </template>
      </div>
    </div>
    <div class="kb-header" v-else>
      <h3 class="product-title">通用资料</h3>
      <p class="subtitle">未关联任何产品的资料，对所有生成都可用。</p>
    </div>

    <div v-if="error" class="banner banner-error">{{ error }}</div>

    <!-- Tab 行 -->
    <div class="tab-row">
      <div
        v-for="doc in viewDocs"
        :key="doc.id"
        class="tab-item"
        :class="{ active: doc.id === activeDocId }"
        @click="selectDoc(doc.id)"
      >
        <template v-if="renamingDocId === doc.id">
          <input
            :id="`rename-input-${doc.id}`"
            v-model="renameDocValue"
            class="tab-rename-input"
            @blur="finishRenameDoc(doc.id)"
            @keydown.enter="finishRenameDoc(doc.id)"
            @keydown.escape="renamingDocId = null"
            @click.stop
          />
        </template>
        <template v-else>
          <span class="tab-name">{{ doc.name }}</span>
          <span class="tab-actions" @click.stop>
            <button class="tab-btn" title="重命名" @click="startRenameDoc(doc)">✏</button>
            <button class="tab-btn" title="管理关联" @click="openAssocModal(doc)">🔗</button>
            <button class="tab-btn danger" title="删除" @click="deleteDoc(doc)">✕</button>
          </span>
        </template>
      </div>
      <button class="tab-new" @click="createDoc">＋ 新建资料</button>
    </div>

    <!-- 空状态 -->
    <div v-if="viewDocs.length === 0" class="empty-state">
      <p class="empty-title">暂无资料</p>
      <p class="empty-desc">
        点「＋ 新建资料」创建一份，编辑后保存。<br />
        生成用例时可在 Generate 页勾选要应用的资料。
      </p>
      <div class="empty-actions">
        <button class="btn-primary" @click="createDoc">＋ 新建资料</button>
      </div>
    </div>

    <!-- 编辑器 -->
    <div v-if="activeDoc" class="editor-wrap">
      <div class="pane">
        <div class="pane-head">
          <span>{{ activeDoc.name }}</span>
          <span class="char-count">{{ editContent.length }} 字</span>
        </div>
        <textarea
          v-model="editContent"
          class="editor"
          spellcheck="false"
          placeholder="开始编写偏好内容，或点下方「插入骨架」…"
        ></textarea>
      </div>
      <div class="editor-actions">
        <button class="btn-ghost" @click="insertSkeleton('general')">插入通用骨架</button>
        <button class="btn-ghost" @click="insertSkeleton('product')">插入产品骨架</button>
        <button class="btn-ghost" @click="openDistillModal">🤖 AI 起草/合并</button>
        <div class="spacer"></div>
        <span v-if="dirty" class="dirty-hint">● 未保存</span>
        <button class="btn-primary" :disabled="saving || !dirty" @click="save">
          {{ saving ? "保存中…" : "保存" }}
        </button>
      </div>
    </div>

    <!-- Inline 输入弹窗 -->
    <div v-if="inlinePrompt" class="modal-overlay" @click.self="cancelPrompt">
      <div class="modal prompt-modal">
        <div class="modal-header">
          <span>{{ inlinePrompt.title }}</span>
          <button class="modal-close" @click="cancelPrompt">✕</button>
        </div>
        <div class="modal-body">
          <input
            id="inline-prompt-input"
            v-model="inlinePrompt.value"
            class="prompt-input"
            :placeholder="inlinePrompt.placeholder"
            @keydown.enter="confirmPrompt"
            @keydown.escape="cancelPrompt"
          />
        </div>
        <div class="modal-footer">
          <div class="spacer"></div>
          <button class="btn-ghost" @click="cancelPrompt">取消</button>
          <button class="btn-primary" :disabled="!inlinePrompt.value.trim()" @click="confirmPrompt">确定</button>
        </div>
      </div>
    </div>

    <!-- Inline 确认弹窗 -->
    <div v-if="inlineConfirm" class="modal-overlay" @click.self="cancelConfirm">
      <div class="modal prompt-modal">
        <div class="modal-header">
          <span>确认</span>
          <button class="modal-close" @click="cancelConfirm">✕</button>
        </div>
        <div class="modal-body">
          <p class="confirm-msg">{{ inlineConfirm.message }}</p>
        </div>
        <div class="modal-footer">
          <div class="spacer"></div>
          <button class="btn-ghost" @click="cancelConfirm">取消</button>
          <button class="btn-danger" @click="okConfirm">确定</button>
        </div>
      </div>
    </div>

    <!-- 管理关联弹窗 -->
    <div v-if="showAssocModal" class="modal-overlay" @click.self="closeAssocModal">
      <div class="modal">
        <div class="modal-header">
          <span>管理关联产品</span>
          <button class="modal-close" @click="closeAssocModal">✕</button>
        </div>
        <div class="modal-body">
          <p class="modal-hint">勾选后此资料出现在对应产品视图；一个都不勾 = 通用。</p>
          <div class="assoc-list">
            <label
              v-for="p in products"
              :key="p.id"
              class="assoc-item"
            >
              <input
                type="checkbox"
                :value="p.id"
                v-model="assocChecked"
              />
              {{ p.name }}
            </label>
            <div v-if="products.length === 0" class="assoc-empty">
              暂无产品，可以先新建。
            </div>
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-ghost" @click="addProductInAssocModal">＋ 新建产品</button>
          <div class="spacer"></div>
          <button class="btn-ghost" @click="closeAssocModal">取消</button>
          <button class="btn-primary" @click="saveAssoc">保存</button>
        </div>
      </div>
    </div>

    <!-- AI 起草/合并弹窗 -->
    <div v-if="showDistillModal" class="modal-overlay" @click.self="closeDistillModal">
      <div class="modal distill-modal">
        <div class="modal-header">
          <span>🤖 AI 起草 / 合并偏好</span>
          <button class="modal-close" @click="closeDistillModal">✕</button>
        </div>
        <div class="modal-body">
          <p class="modal-hint">
            两种用法：①上传「AI 生成 vs 人工修改」对比截图、说明谁是谁，AI 对比差异提炼偏好；
            ②直接在说明里描述偏好（可不传图）。结果会
            {{ editContent.trim() ? "合并进当前内容（新增行标 🆕）" : "全新起草" }}。
          </p>

          <!-- 图片区 -->
          <div
            class="distill-drop"
            tabindex="0"
            @paste="onDistillPaste"
          >
            <div v-if="distillImages.length === 0" class="drop-empty">
              点下方「选择图片」，或在此处 Ctrl/⌘+V 粘贴截图
            </div>
            <div v-else class="img-chips">
              <span v-for="(p, i) in distillImages" :key="i" class="img-chip">
                {{ imageName(p) }}
                <button class="chip-x" @click="removeDistillImage(i)">✕</button>
              </span>
            </div>
          </div>
          <div class="distill-img-actions">
            <button class="btn-ghost" @click="pickDistillImages">＋ 选择图片</button>
            <span class="img-count">{{ distillImages.length }} 张</span>
          </div>

          <!-- 说明 -->
          <textarea
            v-model="distillNote"
            class="distill-note"
            placeholder="对比模式：说明哪个是 AI、哪个是人工；直写模式：直接描述偏好/约定"
            @paste="onDistillPaste"
          ></textarea>

          <div v-if="distillError" class="banner banner-error">{{ distillError }}</div>
        </div>
        <div class="modal-footer">
          <div class="spacer"></div>
          <button class="btn-ghost" :disabled="distilling" @click="closeDistillModal">取消</button>
          <button
            class="btn-primary"
            :disabled="distilling || (distillImages.length === 0 && !distillNote.trim())"
            @click="runDistill"
          >
            {{ distilling ? "生成中…" : "生成草稿" }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.kb-page {
  display: flex;
  flex-direction: column;
  height: 100%;
  padding: 4px 2px;
  position: relative;
}

.kb-header {
  margin-bottom: 10px;
}

.product-title-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.product-title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: #2d3748;
}

.product-rename-input {
  font-size: 15px;
  font-weight: 600;
  border: 1px solid #667eea;
  border-radius: 6px;
  padding: 2px 8px;
  outline: none;
}

.subtitle {
  margin: 2px 0 0;
  font-size: 12px;
  color: #718096;
}

.icon-btn {
  background: none;
  border: none;
  cursor: pointer;
  padding: 2px 4px;
  border-radius: 4px;
  font-size: 12px;
  color: #718096;
  line-height: 1;
}
.icon-btn:hover { background: #edf2f7; }
.icon-btn.danger:hover { background: #fed7d7; color: #c53030; }

.banner {
  padding: 8px 12px;
  border-radius: 8px;
  margin-bottom: 10px;
  font-size: 13px;
}
.banner-error { background: #fed7d7; color: #9b2c2c; }

.tab-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px;
  margin-bottom: 10px;
  padding-bottom: 6px;
  border-bottom: 1px solid #e2e8f0;
}

.tab-item {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 5px 10px;
  border: 1px solid #e2e8f0;
  border-radius: 16px;
  background: white;
  cursor: pointer;
  font-size: 13px;
  color: #4a5568;
}
.tab-item:hover { background: #f7fafc; }
.tab-item.active { border-color: #667eea; background: #667eea; color: white; }
.tab-item.active .tab-btn { color: rgba(255,255,255,0.8); }
.tab-item.active .tab-btn:hover { color: white; background: rgba(255,255,255,0.2); }

.tab-name { line-height: 1; }

.tab-actions {
  display: none;
  gap: 2px;
}
.tab-item:hover .tab-actions,
.tab-item.active .tab-actions {
  display: flex;
}

.tab-btn {
  background: none;
  border: none;
  cursor: pointer;
  padding: 1px 3px;
  border-radius: 3px;
  font-size: 11px;
  color: #718096;
  line-height: 1;
}
.tab-btn:hover { background: #edf2f7; color: #2d3748; }
.tab-btn.danger:hover { background: #fed7d7; color: #c53030; }

.tab-rename-input {
  width: 100px;
  font-size: 12px;
  border: none;
  outline: 1px solid #667eea;
  border-radius: 3px;
  padding: 1px 4px;
  background: white;
  color: #2d3748;
}

.tab-new {
  background: none;
  border: 1px dashed #cbd5e0;
  border-radius: 16px;
  padding: 5px 12px;
  font-size: 13px;
  color: #a0aec0;
  cursor: pointer;
}
.tab-new:hover { border-color: #667eea; color: #667eea; background: #ebf4ff; }

.empty-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: #a0aec0;
  text-align: center;
  padding: 60px 20px;
}
.empty-title { font-size: 16px; font-weight: 600; margin: 0 0 8px; color: #718096; }
.empty-desc { font-size: 13px; line-height: 1.7; margin: 0 0 20px; }
.empty-actions { display: flex; gap: 10px; }

.editor-wrap {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.pane {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  border: 1px solid #e2e8f0;
  border-radius: 10px;
  overflow: hidden;
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
  flex-shrink: 0;
}

.char-count { font-weight: 400; color: #a0aec0; }

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

.editor-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 10px;
  flex-shrink: 0;
}

.spacer { flex: 1; }

.dirty-hint { font-size: 12px; color: #e53e3e; }

.btn-primary {
  padding: 7px 16px;
  background: #667eea;
  color: white;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
}
.btn-primary:hover:not(:disabled) { background: #5a67d8; }
.btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }

.btn-ghost {
  padding: 7px 14px;
  background: white;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  cursor: pointer;
  font-size: 13px;
  color: #4a5568;
}
.btn-ghost:hover { background: #f7fafc; }

/* 弹窗 */
.modal-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0,0,0,0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.modal {
  background: white;
  border-radius: 12px;
  width: 360px;
  max-height: 80vh;
  display: flex;
  flex-direction: column;
  box-shadow: 0 8px 32px rgba(0,0,0,0.2);
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px 10px;
  font-weight: 600;
  font-size: 14px;
  border-bottom: 1px solid #e2e8f0;
}

.modal-close {
  background: none;
  border: none;
  cursor: pointer;
  font-size: 14px;
  color: #718096;
  padding: 2px 6px;
  border-radius: 4px;
}
.modal-close:hover { background: #edf2f7; }

.modal-body {
  flex: 1;
  overflow-y: auto;
  padding: 12px 16px;
}

.modal-hint {
  font-size: 12px;
  color: #718096;
  margin: 0 0 10px;
}

.assoc-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.assoc-item {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  cursor: pointer;
  padding: 4px 0;
}
.assoc-item input { cursor: pointer; }

.assoc-empty { font-size: 13px; color: #a0aec0; }

.modal-footer {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 16px 14px;
  border-top: 1px solid #e2e8f0;
}

.prompt-modal {
  width: 320px;
}

.prompt-input {
  width: 100%;
  box-sizing: border-box;
  padding: 8px 10px;
  font-size: 13px;
  border: 1px solid #cbd5e0;
  border-radius: 6px;
  outline: none;
}
.prompt-input:focus { border-color: #667eea; }

.confirm-msg {
  margin: 0;
  font-size: 13px;
  color: #4a5568;
  line-height: 1.6;
}

.btn-danger {
  padding: 7px 16px;
  background: #e53e3e;
  color: white;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
}
.btn-danger:hover { background: #c53030; }

/* AI 起草弹窗 */
.distill-modal { width: 460px; }

.distill-drop {
  border: 1.5px dashed #cbd5e0;
  border-radius: 8px;
  padding: 12px;
  min-height: 48px;
  outline: none;
  transition: border-color 0.15s;
}
.distill-drop:focus { border-color: #667eea; background: #f7faff; }

.drop-empty {
  font-size: 12px;
  color: #a0aec0;
  text-align: center;
  line-height: 1.6;
}

.img-chips { display: flex; flex-wrap: wrap; gap: 6px; }

.img-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  background: #ebf4ff;
  color: #4a5568;
  border-radius: 6px;
  padding: 3px 6px 3px 8px;
  font-size: 12px;
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chip-x {
  background: none;
  border: none;
  cursor: pointer;
  color: #718096;
  font-size: 11px;
  padding: 0 2px;
  line-height: 1;
}
.chip-x:hover { color: #c53030; }

.distill-img-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 8px 0 10px;
}
.img-count { font-size: 12px; color: #a0aec0; }

.distill-note {
  width: 100%;
  box-sizing: border-box;
  min-height: 70px;
  resize: vertical;
  padding: 8px 10px;
  font-size: 13px;
  line-height: 1.5;
  border: 1px solid #cbd5e0;
  border-radius: 6px;
  outline: none;
  font-family: inherit;
}
.distill-note:focus { border-color: #667eea; }
</style>

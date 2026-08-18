<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  loadFavorites,
  removeFavorite,
  setFavoriteNote,
  mailFavKey,
  favoritesError,
  type FavoriteMail,
} from "../utils/mailFavorites";

const props = defineProps<{ activeOption?: string }>();

const favorites = ref<FavoriteMail[]>([]);
const errorMsg = ref("");
// 详情大卡片（与 GmailPage 同款：列表只占 3 行，全文进弹窗看）
const selectedMail = ref<FavoriteMail | null>(null);

// 源显示名与 GmailPage 的邮件表下拉框保持一致：「邮箱 · 分页名」。
// 同一张表按分页拆成多条源时，光有 label 分不出是哪个分页。
// （_sourceTab 是后加的字段，早先收藏的记录没有 → 退回只显示 label）
function sourceName(m: FavoriteMail): string {
  const label = m._sourceLabel || m._sourceKey;
  return m._sourceTab ? `${label} · ${m._sourceTab}` : label;
}

// 按来源邮件源分 tab（"全部" + 每个有收藏的源，用 _sourceKey 做 key）
const activeTab = ref("all");
interface SourceTab { key: string; label: string; count: number }
const sourceTabs = computed<SourceTab[]>(() => {
  const map = new Map<string, SourceTab>();
  for (const m of favorites.value) {
    const hit = map.get(m._sourceKey);
    if (hit) hit.count++;
    else map.set(m._sourceKey, { key: m._sourceKey, label: sourceName(m), count: 1 });
  }
  return Array.from(map.values()).sort((a, b) => a.label.localeCompare(b.label));
});
const filteredFavorites = computed(() =>
  activeTab.value === "all"
    ? favorites.value
    : favorites.value.filter((m) => m._sourceKey === activeTab.value)
);

function loadList() {
  const map = loadFavorites();
  favorites.value = Object.values(map).sort((a, b) => b.favoritedAt - a.favoritedAt);
  // 该源的收藏可能已被全部取消——切回「全部」而不是停在一个空 tab 上
  if (activeTab.value !== "all" && !favorites.value.some((m) => m._sourceKey === activeTab.value)) {
    activeTab.value = "all";
  }
}

onMounted(loadList);

// MainPage 用 v-show，组件常驻不重新 mount；从 Gmail 页收藏后切回本页时刷新一次
watch(
  () => props.activeOption,
  (v) => {
    if (v === "gmail-favorites") loadList();
  }
);

// 写失败时不刷新列表（数据没变，刷了反而像是成功了）——原因由 favoritesError banner 说明
function unfavorite(m: FavoriteMail) {
  if (!removeFavorite(mailFavKey(m))) return;
  loadList();
}

// 与 GmailPage 同一套打开逻辑：收藏时记下的 profileDir 决定跳哪个 Chrome 资料，
// 没记则退回系统默认浏览器。
async function openInGmail(m: FavoriteMail) {
  if (!m.link) {
    errorMsg.value = "该邮件没有链接列。";
    return;
  }
  errorMsg.value = "";
  try {
    if (m._profileDir) {
      await invoke("open_url_in_chrome_profile", { url: m.link, profileDir: m._profileDir });
    } else {
      await openUrl(m.link);
    }
  } catch (e: any) {
    errorMsg.value = "打开失败：" + String(e);
  }
}

// 与 GmailPage 同口径：gmail-sync.gs 无附件时往「附件」列写的是字符串「无」而非空，
// 只判非空会导致每封都亮 📎。
function hasAttachment(m: FavoriteMail): boolean {
  const v = (m.attachments || "").trim();
  return !!v && v !== "无";
}

function openDetail(m: FavoriteMail) {
  selectedMail.value = m;
}
function closeDetail() {
  selectedMail.value = null;
}
// 在详情里取消收藏就直接关掉弹窗（这封已经不在列表里了）
function unfavoriteFromDetail(m: FavoriteMail) {
  if (!removeFavorite(mailFavKey(m))) return; // 没删成就别关，banner 在底下的列表页上
  loadList();
  selectedMail.value = null;
}

// ── 备注：同一时刻只允许编辑一条，用收藏键标记当前编辑项 ────────────────
// 列表卡片和详情弹窗共用这套状态，两处都能编辑。
const editingNoteKey = ref<string | null>(null);
const noteDraft = ref("");

function startEditNote(m: FavoriteMail) {
  editingNoteKey.value = mailFavKey(m);
  noteDraft.value = m.note || "";
}
function cancelEditNote() {
  editingNoteKey.value = null;
  noteDraft.value = "";
}
// loadList() 会重建 favorites 里的对象，详情弹窗持有的是旧引用 → 按 key 重新指过去，
// 否则在弹窗里改完备注，弹窗仍显示旧内容。
function afterNoteChanged(key: string) {
  loadList();
  if (selectedMail.value && mailFavKey(selectedMail.value) === key) {
    selectedMail.value = favorites.value.find((x) => mailFavKey(x) === key) || null;
  }
}
// 保存失败时保持编辑态不关（用户刚写的内容还在 noteDraft 里，别让它凭空消失）
function saveNote(m: FavoriteMail) {
  const key = mailFavKey(m);
  if (!setFavoriteNote(key, noteDraft.value)) return;
  cancelEditNote();
  afterNoteChanged(key);
}
function deleteNote(m: FavoriteMail) {
  if (!confirm("确定删除这条备注？")) return;
  const key = mailFavKey(m);
  if (!setFavoriteNote(key, "")) return;
  if (editingNoteKey.value === key) cancelEditNote();
  afterNoteChanged(key);
}

// noteUpdatedAt 与 favoritedAt 同为毫秒，复用同一个格式化
function formatFavTs(ts: number): string {
  if (!ts) return "";
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

// ── AI 回复（与 GmailPage 同一套逻辑，逻辑/样式都照搬过来） ──────────────────────
interface MailReplyResult {
  language: string;
  text: string;
  text_zh: string;
  char_count: number;
  usage: { input_tokens?: number; output_tokens?: number; total_cost_usd?: number } | null;
}

type AiMailStatus = "idle" | "queued" | "generating" | "done" | "error";
interface AiMailTask {
  id: string;
  mail: FavoriteMail;
  instruction: string;
  language: string;
  status: AiMailStatus;
  log: string;
  result: MailReplyResult | null;
  editText: string;
  error: string;
  copied: boolean;
}

const AI_LANG_OPTIONS = [
  { value: "auto", label: "跟随邮件语言" },
  { value: "en", label: "English" },
  { value: "zh", label: "中文" },
  { value: "ar", label: "العربية" },
  { value: "fa", label: "فارسی" },
  { value: "ru", label: "Русский" },
  { value: "ko", label: "한국어" },
  { value: "ja", label: "日本語" },
  { value: "de", label: "Deutsch" },
  { value: "fr", label: "Français" },
  { value: "es", label: "Español" },
  { value: "pt", label: "Português" },
  { value: "tr", label: "Türkçe" },
  { value: "id", label: "Indonesia" },
];

function detectLang(text: string): string {
  if (!text) return "en";
  const counts: Record<string, number> = {};
  const inc = (k: string) => { counts[k] = (counts[k] || 0) + 1; };
  for (const ch of text) {
    const cp = ch.codePointAt(0)!;
    if (cp >= 0x0600 && cp <= 0x06FF) inc("fa");
    else if (cp >= 0x4E00 && cp <= 0x9FFF) inc("zh");
    else if (cp >= 0x3040 && cp <= 0x30FF) inc("ja");
    else if (cp >= 0xAC00 && cp <= 0xD7A3) inc("ko");
    else if (cp >= 0x0400 && cp <= 0x04FF) inc("ru");
    else if (cp >= 0x0E00 && cp <= 0x0E7F) inc("th");
  }
  const winner = Object.entries(counts).sort((a, b) => b[1] - a[1])[0];
  return winner ? winner[0] : "en";
}

const aiTasks = ref<AiMailTask[]>([]);
const activeAiId = ref<string | null>(null);
let aiTaskSeq = 0;
let aiGenBusy = false;

// 收藏夹没有「当前选中的邮件源」这个概念（收藏可能来自任意源），
// 按每条收藏记的 _sourceKey 回查 gmail-sources-v1 找对应模板产品。
const GMAIL_SOURCES_KEY = "gmail-sources-v1";
function templateProductFor(m: FavoriteMail): string | undefined {
  try {
    const list = JSON.parse(localStorage.getItem(GMAIL_SOURCES_KEY) || "[]") as Array<{ key: string; templateProduct?: string }>;
    return list.find((s) => s.key === m._sourceKey)?.templateProduct;
  } catch {
    return undefined;
  }
}

const addTplOpen = ref(false);
const addTplCategory = ref("");
const addTplBusy = ref(false);
const addTplError = ref("");
const addTplFlash = ref("");

function tplPayloadForResult(result: MailReplyResult): { text: string; lang: string } {
  const l = (result.language || "").toLowerCase();
  if (l.startsWith("en")) return { text: result.text, lang: "en" };
  if (l.startsWith("zh")) return { text: result.text, lang: "zh-CN" };
  return { text: result.text_zh && result.text_zh.trim() ? result.text_zh : result.text, lang: "zh-CN" };
}
function startAddTpl() {
  addTplOpen.value = true;
  addTplCategory.value = "";
  addTplError.value = "";
}
function cancelAddTpl() {
  addTplOpen.value = false;
  addTplError.value = "";
}
async function confirmAddTpl(task: AiMailTask) {
  if (addTplBusy.value || !task.result) return;
  const product = templateProductFor(task.mail);
  if (!product) {
    addTplError.value = "该邮件所属邮件表未关联模板产品，无法收录。";
    return;
  }
  addTplBusy.value = true;
  addTplError.value = "";
  try {
    const { text, lang } = tplPayloadForResult(task.result);
    await invoke<string>("add_template", {
      product,
      category: addTplCategory.value,
      text,
      lang,
      namespace: "email",
    });
    addTplOpen.value = false;
    addTplFlash.value = `已收录到「${product}」模板库（${lang === "en" ? "英文" : "中文"}模板）`;
    window.setTimeout(() => (addTplFlash.value = ""), 2500);
  } catch (e: any) {
    addTplError.value = String(e);
  } finally {
    addTplBusy.value = false;
  }
}

const activeAiTask = computed(
  () => aiTasks.value.find((t) => t.id === activeAiId.value) ?? null
);
const minimizedAiTasks = computed(
  () => aiTasks.value.filter((t) => t.id !== activeAiId.value)
);

let unlistenAiLog: UnlistenFn | null = null;
onMounted(async () => {
  unlistenAiLog = await listen<{ text: string; kind: string; done: boolean }>(
    "reply-log",
    (e) => {
      const gen = aiTasks.value.find((t) => t.status === "generating");
      if (gen) gen.log = e.payload.text;
    }
  );
});
onUnmounted(() => {
  if (unlistenAiLog) unlistenAiLog();
});

function openAiDialog(m: FavoriteMail) {
  addTplOpen.value = false;
  addTplError.value = "";
  addTplFlash.value = "";
  const existing = aiTasks.value.find((t) => mailFavKey(t.mail) === mailFavKey(m));
  if (existing) { activeAiId.value = existing.id; return; }
  const task: AiMailTask = {
    id: `aimail-${++aiTaskSeq}`,
    mail: m,
    instruction: "",
    language: detectLang(m.body || ""),
    status: "idle",
    log: "",
    result: null,
    editText: "",
    error: "",
    copied: false,
  };
  aiTasks.value.push(task);
  activeAiId.value = task.id;
}

function closeAiTask(task: AiMailTask) {
  if (task.status === "generating" || task.status === "queued") return;
  aiTasks.value = aiTasks.value.filter((t) => t.id !== task.id);
  if (activeAiId.value === task.id) activeAiId.value = null;
}

function minimizeAiDialog() { activeAiId.value = null; }
function restoreAiTask(id: string) {
  activeAiId.value = id;
  addTplOpen.value = false;
  addTplError.value = "";
}

function enqueueAiReply(task: AiMailTask) {
  if (task.status === "generating" || task.status === "queued") return;
  task.status = "queued";
  task.error = "";
  task.log = "";
  task.result = null;
  task.editText = "";
  task.copied = false;
  addTplOpen.value = false;
  addTplError.value = "";
  processAiQueue();
}

async function processAiQueue() {
  if (aiGenBusy) return;
  const next = aiTasks.value.find((t) => t.status === "queued");
  if (!next) return;
  aiGenBusy = true;
  next.status = "generating";
  try {
    const lang = next.language === "auto" ? detectLang(next.mail.body || "") : next.language;
    const result = await invoke<MailReplyResult>("generate_mail_reply", {
      body: next.mail.body || "",
      instruction: next.instruction,
      language: lang,
      model: null,
    });
    next.result = result;
    next.editText = result.text;
    next.status = "done";
  } catch (e: any) {
    const msg = String(e);
    next.error = msg === "CANCELLED" ? "已取消生成。" : msg;
    next.status = "error";
  } finally {
    aiGenBusy = false;
    processAiQueue();
  }
}

async function copyAiReply(task: AiMailTask) {
  if (!task.editText) return;
  try {
    await navigator.clipboard.writeText(task.editText);
    task.copied = true;
    setTimeout(() => { task.copied = false; }, 2000);
  } catch { /* 复制失败静默 */ }
}

async function copyAndJumpAi(task: AiMailTask) {
  await copyAiReply(task);
  await openInGmail(task.mail);
  closeAiTask(task);
}
</script>

<template>
  <div class="fav-page">
    <header class="page-header">
      <h3>收藏邮件</h3>
      <p class="subtitle">
        在「Gmail」邮件列表里点 ☆ 收藏的邮件会出现在这里，星标本身即收藏状态，再点一次取消收藏。
        收藏的是整封快照，标为已读隐藏后依然能在这里回看。
        每封可在卡片下方（或详情弹窗里）加备注，取消收藏时备注一并删除。
      </p>
    </header>

    <div v-if="favoritesError" class="banner banner-error">{{ favoritesError }}</div>
    <div v-if="errorMsg" class="banner banner-error">{{ errorMsg }}</div>

    <div v-if="favorites.length === 0" class="empty-state">
      还没有收藏任何邮件。去「Gmail」邮件列表点邮件右上角的 ☆ 收藏。
    </div>

    <template v-else>
      <div class="app-tabs">
        <button
          class="app-tab"
          :class="{ active: activeTab === 'all' }"
          @click="activeTab = 'all'"
        >全部 <span class="tab-count">{{ favorites.length }}</span></button>
        <button
          v-for="t in sourceTabs"
          :key="t.key"
          class="app-tab"
          :class="{ active: activeTab === t.key }"
          @click="activeTab = t.key"
        >{{ t.label }} <span class="tab-count">{{ t.count }}</span></button>
      </div>

      <div v-if="filteredFavorites.length === 0" class="empty-state">
        该邮件源下暂无收藏邮件。
      </div>
      <!-- 与 Gmail 页同款紧凑三行卡片：发件人+日期 / 主题 / 机翻中文（单行截断），全文进详情弹窗 -->
      <div v-else class="mail-list">
        <article v-for="m in filteredFavorites" :key="mailFavKey(m)" class="mail-item">
          <div class="mi-row1">
            <span v-if="activeTab === 'all'" class="src-badge">{{ sourceName(m) }}</span>
            <span class="from">{{ m.from || "(未知发件人)" }}</span>
            <span class="ts">{{ m.date }}</span>
            <span v-if="hasAttachment(m)" class="att-dot" :title="m.attachments">📎</span>
            <span class="fav-ts">收藏于 {{ formatFavTs(m.favoritedAt) }}</span>
            <div class="mi-actions">
              <button class="fav-star-btn active" @click="unfavorite(m)" title="取消收藏">★</button>
              <button class="ai-btn" @click="openAiDialog(m)" title="AI 生成回复草稿">✨ AI</button>
              <button class="detail-btn" @click="openDetail(m)">详情</button>
              <button
                v-if="m.link"
                class="open-btn"
                @click="openInGmail(m)"
                title="在 Gmail 中打开该会话，本人回复"
              >↗</button>
            </div>
          </div>
          <div class="mi-subject">{{ m.subject || "(无主题)" }}</div>
          <div class="mi-trans">{{ m.translated || "(无机翻中文)" }}</div>

          <!-- 备注：卡片最下方，未填时只留一个「＋ 添加备注」按钮，不破坏三行紧凑感 -->
          <div class="note-zone">
            <div v-if="editingNoteKey === mailFavKey(m)" class="note-editor">
              <textarea
                v-model="noteDraft"
                class="note-input"
                rows="3"
                placeholder="写点备注……（例如：已转给客服 / 待确认退款）"
              ></textarea>
              <div class="note-editor-actions">
                <button class="note-btn primary" @click="saveNote(m)">保存</button>
                <button class="note-btn" @click="cancelEditNote">取消</button>
                <button v-if="m.note" class="note-btn danger" @click="deleteNote(m)">删除备注</button>
              </div>
            </div>
            <div v-else-if="m.note" class="note-block">
              <div class="note-head">
                <span class="note-label">📝 备注</span>
                <span v-if="m.noteUpdatedAt" class="note-ts">{{ formatFavTs(m.noteUpdatedAt) }}</span>
                <div class="note-head-actions">
                  <button class="note-btn" @click="startEditNote(m)">编辑</button>
                  <button class="note-btn danger" @click="deleteNote(m)">删除</button>
                </div>
              </div>
              <div class="note-text">{{ m.note }}</div>
            </div>
            <button v-else class="note-add-btn" @click="startEditNote(m)">＋ 添加备注</button>
          </div>
        </article>
      </div>
    </template>

    <!-- 详情大卡片：机翻中文在上，原文在下（与 GmailPage 同款） -->
    <div v-if="selectedMail" class="detail-overlay" @click.self="closeDetail">
      <div class="detail-card">
        <div class="detail-head">
          <div class="detail-meta">
            <span class="src-badge">{{ sourceName(selectedMail) }}</span>
            <span class="from">{{ selectedMail.from || "(未知发件人)" }}</span>
            <span class="ts">{{ selectedMail.date }}</span>
          </div>
          <div class="detail-head-actions">
            <button
              class="fav-star-btn active"
              @click="unfavoriteFromDetail(selectedMail)"
              title="取消收藏"
            >★</button>
            <button class="web-btn ai-web-btn" @click="openAiDialog(selectedMail)">
              ✨ AI 回复
            </button>
            <button
              v-if="selectedMail.link"
              class="web-btn"
              @click="openInGmail(selectedMail)"
              title="在 Gmail 中打开该会话，本人回复"
            >
              ↗ 在 Gmail 中打开
            </button>
            <button class="detail-close" @click="closeDetail">✕</button>
          </div>
        </div>
        <div class="detail-subject">{{ selectedMail.subject || "(无主题)" }}</div>
        <div v-if="hasAttachment(selectedMail)" class="detail-att">📎 {{ selectedMail.attachments }}</div>

        <div class="detail-section">
          <div class="detail-label">机翻中文</div>
          <div class="detail-text">{{ selectedMail.translated || "(无机翻中文)" }}</div>
        </div>
        <div class="detail-section">
          <div class="detail-label">原文</div>
          <div class="detail-text orig">{{ selectedMail.body || "(无正文)" }}</div>
        </div>

        <!-- 备注：与列表卡片共用同一套编辑状态，改哪边都同步 -->
        <div class="detail-section">
          <div class="detail-label">备注</div>
          <div v-if="editingNoteKey === mailFavKey(selectedMail)" class="note-editor">
            <textarea
              v-model="noteDraft"
              class="note-input"
              rows="3"
              placeholder="写点备注……（例如：已转给客服 / 待确认退款）"
            ></textarea>
            <div class="note-editor-actions">
              <button class="note-btn primary" @click="saveNote(selectedMail)">保存</button>
              <button class="note-btn" @click="cancelEditNote">取消</button>
              <button v-if="selectedMail.note" class="note-btn danger" @click="deleteNote(selectedMail)">删除备注</button>
            </div>
          </div>
          <div v-else-if="selectedMail.note" class="note-block">
            <div class="note-head">
              <span class="note-label">📝 备注</span>
              <span v-if="selectedMail.noteUpdatedAt" class="note-ts">{{ formatFavTs(selectedMail.noteUpdatedAt) }}</span>
              <div class="note-head-actions">
                <button class="note-btn" @click="startEditNote(selectedMail)">编辑</button>
                <button class="note-btn danger" @click="deleteNote(selectedMail)">删除</button>
              </div>
            </div>
            <div class="note-text">{{ selectedMail.note }}</div>
          </div>
          <button v-else class="note-add-btn" @click="startEditNote(selectedMail)">＋ 添加备注</button>
        </div>
      </div>
    </div>

    <!-- AI 回复对话框（与 GmailPage 同款） -->
    <div v-if="activeAiTask" class="detail-overlay ai-overlay" @click.self="minimizeAiDialog">
      <div class="ai-dialog">
        <div class="ai-dialog-head">
          <span class="ai-dialog-title">✨ AI 回复草稿</span>
          <div class="ai-head-btns">
            <button class="ai-min" title="缩小（生成继续）" @click="minimizeAiDialog">—</button>
            <button class="detail-close" @click="closeAiTask(activeAiTask)" :disabled="activeAiTask.status === 'generating' || activeAiTask.status === 'queued'">✕</button>
          </div>
        </div>
        <div class="ai-mail-quote">
          <div class="ai-quote-subj">{{ activeAiTask.mail.subject || "(无主题)" }}</div>
          <div class="ai-quote-from">{{ activeAiTask.mail.from }}</div>
        </div>

        <div class="ai-form-row">
          <label class="ai-form-label">语言</label>
          <select class="ai-lang-select" v-model="activeAiTask.language" :disabled="activeAiTask.status === 'generating' || activeAiTask.status === 'queued'">
            <option v-for="opt in AI_LANG_OPTIONS" :key="opt.value" :value="opt.value">
              {{ opt.label }}
            </option>
          </select>
        </div>

        <div class="ai-form-row">
          <label class="ai-form-label">回复方向</label>
          <textarea
            class="ai-instruction"
            v-model="activeAiTask.instruction"
            placeholder="（选填）告诉 AI 要怎么回，例如「道歉并引导排查」；留空由 AI 根据正文自行判断"
            rows="2"
            :disabled="activeAiTask.status === 'generating' || activeAiTask.status === 'queued'"
          ></textarea>
        </div>

        <div class="ai-btn-row">
          <button class="ai-generate-btn" @click="enqueueAiReply(activeAiTask)" :disabled="activeAiTask.status === 'generating' || activeAiTask.status === 'queued'">
            {{ activeAiTask.status === 'generating' ? "生成中…" : activeAiTask.status === 'queued' ? "排队中…" : (activeAiTask.result ? "重新生成" : "生成回复") }}
          </button>
        </div>

        <div v-if="(activeAiTask.status === 'generating' || activeAiTask.status === 'queued') && activeAiTask.log" class="ai-log">{{ activeAiTask.log }}</div>
        <div v-if="activeAiTask.error" class="ai-error">{{ activeAiTask.error }}</div>

        <template v-if="activeAiTask.result">
          <div class="ai-result-label-row">
            <span class="ai-result-label">回复草稿</span>
            <span class="ai-char-count">{{ activeAiTask.result.char_count }} 字符</span>
          </div>
          <textarea class="ai-result-text" v-model="activeAiTask.editText" rows="6"></textarea>
          <div v-if="activeAiTask.result.text_zh" class="ai-result-zh">{{ activeAiTask.result.text_zh }}</div>

          <div v-if="addTplFlash" class="ai-tpl-flash">✓ {{ addTplFlash }}</div>
          <div v-if="!addTplOpen" class="ai-result-foot">
            <button class="ai-copy-btn secondary" @click="copyAiReply(activeAiTask)">
              {{ activeAiTask.copied ? "已复制 ✓" : "仅复制" }}
            </button>
            <button class="ai-copy-btn" @click="copyAndJumpAi(activeAiTask)">
              复制并跳转 ↗
            </button>
            <button
              class="ai-addtpl-btn"
              :disabled="!templateProductFor(activeAiTask.mail)"
              :title="templateProductFor(activeAiTask.mail) ? '收录为模板（英文/中文草稿存对应源；其它语言用机翻中文入库）' : '该邮件所属邮件表未关联模板产品'"
              @click="startAddTpl"
            >
              ➕ 添加模板
            </button>
          </div>
          <div v-else class="ai-addtpl-panel">
            <input
              v-model="addTplCategory"
              class="ai-addtpl-category"
              placeholder="类别（如：售后道歉 / 排查引导；可留空=未分类）"
              @keyup.enter="confirmAddTpl(activeAiTask)"
            />
            <button class="ai-addtpl-ok" :disabled="addTplBusy" @click="confirmAddTpl(activeAiTask)">
              {{ addTplBusy ? "收录中…" : "收录" }}
            </button>
            <button class="ai-addtpl-cancel" :disabled="addTplBusy" @click="cancelAddTpl">取消</button>
            <span v-if="addTplError" class="ai-addtpl-err">{{ addTplError }}</span>
          </div>
        </template>
      </div>
    </div>

    <!-- 缩小后的右下角悬浮条 -->
    <div v-if="minimizedAiTasks.length" class="ai-mini-stack">
      <div
        v-for="t in minimizedAiTasks"
        :key="t.id"
        class="ai-mini-bar"
        :class="{ 'is-error': t.status === 'error', 'is-done': t.status === 'done' }"
        @click="restoreAiTask(t.id)"
      >
        <span class="ai-mini-text">
          ✨ <span class="ai-mini-quote">{{ (t.mail.subject || t.mail.from || "(无主题)").slice(0, 16) }}</span>
          <template v-if="t.status === 'generating'">· 生成中…</template>
          <template v-else-if="t.status === 'queued'">· 排队中</template>
          <template v-else-if="t.status === 'error'">· 失败</template>
          <template v-else-if="t.result">· 已就绪</template>
          <template v-else>· 待生成</template>
        </span>
        <button class="ai-mini-open" @click.stop="restoreAiTask(t.id)">展开</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.fav-page {
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
  margin: 4px 0 16px 0;
  font-size: 12px;
  color: #888;
  line-height: 1.6;
}
.banner {
  padding: 8px 12px;
  border-radius: 6px;
  font-size: 12px;
  margin-bottom: 12px;
}
.banner-error {
  background: #fff5f5;
  color: #9b2c2c;
  border: 1px solid #fed7d7;
}
.empty-state {
  padding: 30px 16px;
  text-align: center;
  font-size: 13px;
  color: #999;
}
.app-tabs {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 12px;
}
.app-tab {
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
.app-tab:hover {
  background: #f7fafc;
}
.app-tab.active {
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
.app-tab.active .tab-count {
  background: rgba(255, 255, 255, 0.25);
}
/* 列表卡片：与 GmailPage 的 .mail-item 同尺寸，固定三行不撑高 */
.mail-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.mail-item {
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 8px 12px;
  background: white;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.mi-row1 {
  display: flex;
  align-items: center;
  gap: 8px;
  white-space: nowrap;
  overflow: hidden;
}
.src-badge {
  font-size: 10px;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 10px;
  background: #e9d8fd;
  color: #553c9a;
  flex-shrink: 0;
}
.from {
  font-size: 13px;
  font-weight: 500;
  color: #2d3748;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 320px;
}
.ts {
  font-size: 11px;
  color: #999;
  flex-shrink: 0;
}
.att-dot {
  font-size: 12px;
  flex-shrink: 0;
}
.fav-ts {
  font-size: 11px;
  color: #bbb;
  flex-shrink: 0;
}
.mi-actions {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}
.mi-subject {
  font-size: 13px;
  font-weight: 600;
  color: #1a202c;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.mi-trans {
  font-size: 12px;
  color: #718096;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.detail-btn {
  padding: 3px 12px;
  font-size: 12px;
  border: 1px solid #667eea;
  border-radius: 6px;
  background: white;
  color: #667eea;
  cursor: pointer;
}
.detail-btn:hover {
  background: #667eea;
  color: white;
}
.open-btn {
  padding: 3px 10px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.open-btn:hover {
  background: #f5f5fa;
  border-color: #cbd5e0;
}

/* 详情大卡片 */
.detail-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  padding: 20px;
}
.detail-card {
  background: white;
  border-radius: 12px;
  width: 100%;
  max-width: 900px;
  max-height: 88vh;
  overflow-y: auto;
  padding: 18px 20px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.25);
}
.detail-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
}
.detail-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
}
.detail-head-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.detail-close {
  border: none;
  background: none;
  color: #999;
  cursor: pointer;
  font-size: 16px;
  width: 28px;
  height: 28px;
  border-radius: 6px;
  flex-shrink: 0;
}
.detail-close:hover {
  background: #edf2f7;
  color: #4a5568;
}
.detail-subject {
  font-size: 15px;
  font-weight: 600;
  color: #1a202c;
  margin: 8px 0 4px;
  word-break: break-word;
}
.detail-att {
  font-size: 11px;
  color: #888;
  word-break: break-all;
  margin-bottom: 8px;
}
.detail-section {
  margin-top: 12px;
}
.detail-label {
  font-size: 11px;
  font-weight: 600;
  color: #2b6cb0;
  margin-bottom: 4px;
}
.detail-text {
  font-size: 13px;
  line-height: 1.6;
  color: #2d3748;
  white-space: pre-wrap;
  word-break: break-word;
  background: #f7fafc;
  border-left: 3px solid #4299e1;
  border-radius: 0 6px 6px 0;
  padding: 8px 12px;
}
.detail-text.orig {
  color: #4a5568;
  border-left-color: #cbd5e0;
  background: #fafafa;
}
.fav-star-btn {
  border: none;
  background: transparent;
  font-size: 17px;
  line-height: 1;
  padding: 0 2px;
  cursor: pointer;
  color: #cbd5e0;
}
.fav-star-btn:hover {
  color: #d69e2e;
}
.fav-star-btn.active {
  color: #d69e2e;
}
.web-btn {
  padding: 4px 12px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.web-btn:hover {
  background: #f5f5fa;
  border-color: #cbd5e0;
  color: #2d3748;
}

/* ── 备注区（列表卡片 + 详情弹窗共用） ────────────────────────────────── */
.note-zone {
  margin-top: 6px;
  padding-top: 6px;
  border-top: 1px dashed #edf2f7;
}
.note-add-btn {
  border: 1px dashed #cbd5e0;
  background: transparent;
  color: #a0aec0;
  font-size: 12px;
  padding: 3px 12px;
  border-radius: 6px;
  cursor: pointer;
}
.note-add-btn:hover {
  border-color: #d69e2e;
  color: #b7791f;
  background: #fffdf5;
}
.note-block {
  background: #fffdf5;
  border-left: 3px solid #ecc94b;
  border-radius: 0 6px 6px 0;
  padding: 6px 12px;
}
.note-head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 3px;
}
.note-label {
  font-size: 11px;
  font-weight: 600;
  color: #b7791f;
}
.note-ts {
  font-size: 11px;
  color: #bbb;
}
.note-head-actions {
  margin-left: auto;
  display: flex;
  gap: 6px;
}
.note-text {
  font-size: 12px;
  color: #4a5568;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
}
.note-editor {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.note-input {
  width: 100%;
  box-sizing: border-box;
  font-size: 12px;
  line-height: 1.6;
  font-family: inherit;
  color: #2d3748;
  padding: 8px 10px;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  resize: vertical;
}
.note-input:focus {
  outline: none;
  border-color: #ecc94b;
  background: #fffdf5;
}
.note-editor-actions {
  display: flex;
  gap: 6px;
  justify-content: flex-end;
}
.note-btn {
  padding: 3px 12px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.note-btn:hover {
  background: #f5f5fa;
  border-color: #cbd5e0;
}
.note-btn.primary {
  border-color: #667eea;
  background: #667eea;
  color: white;
}
.note-btn.primary:hover {
  background: #5a67d8;
}
.note-btn.danger {
  color: #c53030;
}
.note-btn.danger:hover {
  background: #fff5f5;
  border-color: #feb2b2;
}

/* ── AI 回复（照搬 GmailPage 同名样式） ────────────────────────────────── */
.ai-btn {
  padding: 3px 8px;
  font-size: 12px;
  border: 1px solid #ed8936;
  border-radius: 6px;
  background: white;
  color: #c05621;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-btn:hover {
  background: #fffaf0;
}
.ai-web-btn {
  border-color: #ed8936;
  color: #c05621;
}
.ai-web-btn:hover {
  background: #ed8936;
  color: white;
}
.ai-overlay {
  z-index: 1020;
}
.ai-dialog {
  background: white;
  border-radius: 12px;
  width: 100%;
  max-width: min(620px, calc(100vw - 80px));
  max-height: 88vh;
  overflow-y: auto;
  padding: 18px 20px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.25);
}
.ai-dialog-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}
.ai-dialog-title {
  font-size: 14px;
  font-weight: 600;
  color: #2d3748;
}
.ai-head-btns {
  display: flex;
  align-items: center;
  gap: 4px;
}
.ai-min {
  border: none;
  background: none;
  color: #999;
  cursor: pointer;
  font-size: 18px;
  line-height: 1;
  width: 28px;
  height: 28px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
}
.ai-min:hover {
  background: #edf2f7;
  color: #4a5568;
}
.ai-mini-stack {
  position: fixed;
  right: 20px;
  bottom: 20px;
  z-index: 1050;
  display: flex;
  flex-direction: column-reverse;
  gap: 8px;
  align-items: flex-end;
}
.ai-mini-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  background: white;
  border: 1px solid #e2e8f0;
  border-left: 3px solid #ed8936;
  border-radius: 10px;
  box-shadow: 0 6px 24px rgba(0, 0, 0, 0.18);
  cursor: pointer;
  max-width: 360px;
}
.ai-mini-bar.is-error {
  border-left-color: #e53e3e;
}
.ai-mini-bar.is-done {
  border-left-color: #38a169;
}
.ai-mini-bar:hover {
  background: #fffaf0;
}
.ai-mini-text {
  font-size: 12px;
  color: #4a5568;
  white-space: nowrap;
}
.ai-mini-quote {
  color: #1a202c;
  font-weight: 500;
}
.ai-mini-open {
  padding: 4px 12px;
  font-size: 12px;
  border: 1px solid #ed8936;
  border-radius: 6px;
  background: white;
  color: #c05621;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-mini-open:hover {
  background: #ed8936;
  color: white;
}
.ai-mail-quote {
  background: #fffaf0;
  border-left: 3px solid #ed8936;
  border-radius: 0 6px 6px 0;
  padding: 8px 12px;
  margin-bottom: 14px;
}
.ai-quote-subj {
  font-size: 13px;
  font-weight: 600;
  color: #1a202c;
}
.ai-quote-from {
  font-size: 11px;
  color: #718096;
  margin-top: 2px;
}
.ai-form-row {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  margin-bottom: 10px;
}
.ai-form-label {
  width: 64px;
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
  color: #4a5568;
  padding-top: 6px;
}
.ai-lang-select {
  flex: 1;
  padding: 5px 8px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  outline: none;
  cursor: pointer;
}
.ai-lang-select:focus {
  border-color: #ed8936;
}
.ai-instruction {
  flex: 1;
  padding: 6px 10px;
  font-size: 13px;
  border: 1px solid #ddd;
  border-radius: 6px;
  font-family: inherit;
  resize: vertical;
  outline: none;
}
.ai-instruction:focus {
  border-color: #ed8936;
}
.ai-btn-row {
  display: flex;
  justify-content: flex-end;
  margin-bottom: 10px;
}
.ai-generate-btn {
  padding: 6px 20px;
  font-size: 13px;
  font-weight: 500;
  border: none;
  border-radius: 6px;
  background: #ed8936;
  color: white;
  cursor: pointer;
}
.ai-generate-btn:hover:not(:disabled) {
  background: #dd6b20;
}
.ai-generate-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.ai-log {
  font-size: 12px;
  color: #718096;
  background: #f7fafc;
  border-radius: 6px;
  padding: 8px 10px;
  margin-bottom: 10px;
  white-space: pre-wrap;
  word-break: break-all;
}
.ai-error {
  font-size: 12px;
  color: #c53030;
  background: #fff5f5;
  border: 1px solid #fed7d7;
  border-radius: 6px;
  padding: 8px 10px;
  margin-bottom: 10px;
}
.ai-result-label-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
}
.ai-result-label {
  font-size: 11px;
  font-weight: 600;
  color: #4a5568;
}
.ai-char-count {
  font-size: 11px;
  color: #999;
}
.ai-result-text {
  width: 100%;
  box-sizing: border-box;
  padding: 8px 10px;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  font-size: 13px;
  line-height: 1.6;
  font-family: inherit;
  resize: vertical;
  outline: none;
  color: #2d3748;
}
.ai-result-text:focus {
  border-color: #ed8936;
}
.ai-result-zh {
  font-size: 12px;
  color: #718096;
  margin-top: 6px;
  padding: 6px 10px;
  background: #f7fafc;
  border-radius: 6px;
  line-height: 1.5;
}
.ai-result-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 10px;
}
.ai-copy-btn {
  padding: 6px 20px;
  font-size: 13px;
  border: 1px solid #ed8936;
  background: #ed8936;
  color: white;
  border-radius: 6px;
  cursor: pointer;
}
.ai-copy-btn:hover {
  background: #dd6b20;
}
.ai-copy-btn.secondary {
  background: white;
  color: #c05621;
}
.ai-copy-btn.secondary:hover {
  background: #fffaf0;
}
.ai-addtpl-btn {
  font-size: 12px;
  padding: 5px 14px;
  border: 1px solid #cbd5e0;
  border-radius: 6px;
  background: white;
  color: #5a67d8;
  cursor: pointer;
}
.ai-addtpl-btn:hover:not(:disabled) {
  background: #eef0ff;
}
.ai-addtpl-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.ai-tpl-flash {
  font-size: 12px;
  color: #22543d;
  background: #c6f6d5;
  border-radius: 6px;
  padding: 5px 10px;
  margin-top: 8px;
}
.ai-addtpl-panel {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 10px;
  flex-wrap: wrap;
}
.ai-addtpl-category {
  flex: 1;
  min-width: 160px;
  padding: 4px 8px;
  border: 1px solid #cbd5e0;
  border-radius: 6px;
  font-size: 12px;
}
.ai-addtpl-ok,
.ai-addtpl-cancel {
  font-size: 12px;
  padding: 4px 12px;
  border-radius: 6px;
  cursor: pointer;
  flex-shrink: 0;
}
.ai-addtpl-ok {
  border: 1px solid #ed8936;
  background: #ed8936;
  color: white;
}
.ai-addtpl-ok:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.ai-addtpl-cancel {
  border: 1px solid #e2e8f0;
  background: white;
  color: #718096;
}
.ai-addtpl-err {
  font-size: 11px;
  color: #c53030;
  width: 100%;
}
</style>

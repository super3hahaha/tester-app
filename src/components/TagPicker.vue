<script setup lang="ts">
// 「标签」按钮 + 下拉浮层：复选框勾选标签 + 底部内联新建。
// 每次勾选都立刻 emit("change", 新的完整 id 列表)，由父页面负责写入（含「未收藏先收藏」）。
// 父页面写失败时不改 selected，这里勾选状态完全跟 props 走，天然回滚。
// 浮层 Teleport 到 body + fixed 定位：邮件卡片行 / 详情弹窗都有 overflow 裁切，absolute 会被裁掉。
// 滚动时直接关掉（fixed 坐标不跟随滚动）。
//
// app（包名）限定范围：传了 app → 只列「通用 + 该 app」的标签，新建默认挂该 app（可勾「设为通用」）；
// 没传（邮件源没关联 app 等）→ 列全部标签按 app 分组，新建为通用。
// 已勾选但不在当前范围的标签（后来改了范围）单独列在「其它」组，方便取消。
import { ref, computed, onBeforeUnmount, nextTick } from "vue";
import { sortedTags, tagsForApp, groupTags, addTag, favTagsError, reloadTags, type TagGroup } from "../utils/favTags";
import { pkgApps, loadPkgApps, appLabel } from "../utils/appRegistry";

const props = defineProps<{
  selected?: string[];
  // 按钮文字；默认「标签」
  label?: string;
  // 这条评论/邮件所属 app 的包名；空 = 不限
  app?: string;
}>();
const emit = defineEmits<{ (e: "change", ids: string[]): void }>();

const open = ref(false);
const root = ref<HTMLElement | null>(null);
const pop = ref<HTMLElement | null>(null);
const pos = ref<Record<string, string>>({});
const POP_W = 240;
const POP_MAX_H = 300;
const newName = ref("");
const inputEl = ref<HTMLInputElement | null>(null);
const localErr = ref("");

const newAsGeneral = ref(false);

const selSet = computed(() => new Set(props.selected || []));
const inScope = computed(() => tagsForApp(props.app));
const hasAnyTag = computed(() => sortedTags().length > 0);
interface ViewGroup extends TagGroup { label: string }
const groups = computed<ViewGroup[]>(() => {
  const order = pkgApps.value.map((a) => a.package);
  let gs = groupTags(inScope.value, order);
  // 限定 app 时，同时适用多个 app 的标签只归到本 app 组，不展开其它 app 的组
  if (props.app) gs = gs.filter((g) => g.key === "" || g.key === props.app);
  const out: ViewGroup[] = gs.map((g) => ({ ...g, label: g.key ? appLabel(g.key) : "通用" }));
  const scopeIds = new Set(inScope.value.map((t) => t.id));
  const stray = sortedTags().filter((t) => selSet.value.has(t.id) && !scopeIds.has(t.id));
  if (stray.length) out.push({ key: "__stray__", tags: stray, label: "其它（不在本 app 范围）" });
  return out;
});

function onDocDown(e: MouseEvent) {
  const t = e.target as Node;
  if (root.value?.contains(t) || pop.value?.contains(t)) return;
  close();
}
function onScroll(e: Event) {
  // 浮层自己内部滚动（标签多时）不算
  if (pop.value && e.target instanceof Node && pop.value.contains(e.target)) return;
  close();
}
function place() {
  const r = root.value?.getBoundingClientRect();
  if (!r) return;
  const left = Math.max(8, Math.min(r.right - POP_W, window.innerWidth - POP_W - 8));
  // 下方放不下就往上弹
  const below = window.innerHeight - r.bottom;
  pos.value =
    below < POP_MAX_H + 12 && r.top > below
      ? { left: left + "px", bottom: window.innerHeight - r.top + 4 + "px" }
      : { left: left + "px", top: r.bottom + 4 + "px" };
}
function toggleOpen() {
  if (open.value) return close();
  reloadTags();
  loadPkgApps();
  place();
  open.value = true;
  localErr.value = "";
  newAsGeneral.value = false;
  document.addEventListener("mousedown", onDocDown);
  window.addEventListener("scroll", onScroll, true);
  window.addEventListener("resize", close);
  nextTick(() => {
    if (!inScope.value.length) inputEl.value?.focus();
  });
}
function close() {
  open.value = false;
  newName.value = "";
  document.removeEventListener("mousedown", onDocDown);
  window.removeEventListener("scroll", onScroll, true);
  window.removeEventListener("resize", close);
}
onBeforeUnmount(close);

function toggle(id: string) {
  const cur = props.selected || [];
  emit("change", selSet.value.has(id) ? cur.filter((x) => x !== id) : [...cur, id]);
}

// 中文输入法里按回车是「确认拼音上屏」，不能当成提交（否则会建出拼音标签）
function onEnter(e: KeyboardEvent) {
  if (e.isComposing || e.keyCode === 229) return;
  create();
}
function create() {
  if (!newName.value.trim()) return;
  const t = addTag(newName.value, props.app && !newAsGeneral.value ? [props.app] : []);
  if (!t) {
    localErr.value = favTagsError.value;
    return;
  }
  localErr.value = "";
  newName.value = "";
  // 新建即勾上：用户建标签基本就是为了给当前这条打
  emit("change", [...(props.selected || []), t.id]);
}
</script>

<template>
  <span ref="root" class="tag-picker">
    <button
      class="tag-btn"
      :class="{ has: (selected || []).length }"
      @click="toggleOpen"
      title="给这条打标签（未收藏会自动收藏）"
    >{{ label || "标签" }}</button>
    <Teleport to="body">
    <div v-if="open" ref="pop" class="tag-pop" :style="pos" @click.stop>
      <div v-if="!groups.length" class="tag-empty">
        {{ hasAnyTag && app ? `${appLabel(app)} 还没有可用的标签，在下面新建一个` : "还没有标签，在下面新建一个" }}
      </div>
      <template v-for="g in groups" :key="g.key">
        <div class="tag-group">{{ g.label }}</div>
        <label v-for="t in g.tags" :key="g.key + t.id" class="tag-row">
          <input type="checkbox" :checked="selSet.has(t.id)" @change="toggle(t.id)" />
          <span class="tag-dot" :style="{ background: t.color }"></span>
          <span class="tag-name">{{ t.name }}</span>
        </label>
      </template>
      <div class="tag-new">
        <input
          ref="inputEl"
          v-model="newName"
          placeholder="新建标签，回车确认"
          maxlength="20"
          @keydown.enter.prevent="onEnter"
          @keydown.esc="close"
        />
        <button :disabled="!newName.trim()" @click="create">新建</button>
      </div>
      <label v-if="app" class="tag-scope">
        <input type="checkbox" v-model="newAsGeneral" />
        <span>设为通用（否则只用于 {{ appLabel(app) }}）</span>
      </label>
      <div v-if="localErr" class="tag-err">{{ localErr }}</div>
    </div>
    </Teleport>
  </span>
</template>

<style scoped>
.tag-picker {
  position: relative;
  display: inline-block;
}
.tag-btn {
  padding: 4px 10px;
  font-size: 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: #4a5568;
  cursor: pointer;
}
.tag-btn:hover {
  background: #f5f5fa;
  border-color: #cbd5e0;
}
.tag-btn.has {
  border-color: #90cdf4;
  color: #2b6cb0;
}
.tag-pop {
  position: fixed;
  z-index: 1200;
  width: 240px;
  max-height: 300px;
  overflow-y: auto;
  background: white;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.15);
  padding: 6px;
  text-align: left;
}
.tag-group {
  font-size: 11px;
  color: #a0aec0;
  padding: 6px 6px 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tag-scope {
  display: flex;
  align-items: flex-start;
  gap: 5px;
  font-size: 11px;
  color: #718096;
  padding: 6px 2px 0;
  cursor: pointer;
  line-height: 1.4;
}
.tag-scope input {
  margin-top: 1px;
}
.tag-empty {
  font-size: 12px;
  color: #a0aec0;
  padding: 6px;
}
.tag-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 6px;
  border-radius: 5px;
  cursor: pointer;
  font-size: 13px;
  color: #2d3748;
}
.tag-row:hover {
  background: #f7fafc;
}
.tag-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  flex-shrink: 0;
}
.tag-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tag-new {
  display: flex;
  gap: 4px;
  margin-top: 6px;
  padding-top: 6px;
  border-top: 1px solid #edf2f7;
}
.tag-new input {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  padding: 4px 6px;
  border: 1px solid #e2e8f0;
  border-radius: 5px;
}
.tag-new button {
  font-size: 12px;
  padding: 4px 8px;
  border: 1px solid #cbd5e0;
  border-radius: 5px;
  background: #f7fafc;
  cursor: pointer;
}
.tag-new button:disabled {
  opacity: 0.5;
  cursor: default;
}
.tag-err {
  font-size: 11px;
  color: #c53030;
  padding: 4px 2px 0;
}
</style>

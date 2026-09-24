<script setup lang="ts">
// 「管理标签」弹窗：新建 / 改名 / 改色 / 改适用范围（通用 or 指定 app）/ 删除（内联二次确认）。
// 删除只动标签库，收藏记录里残留的 id 由 resolveTags 过滤（不回写）。
import { ref, computed } from "vue";
import {
  sortedTags, addTag, renameTag, setTagColor, setTagApps, removeTag, favTagsError, reloadTags, TAG_PALETTE,
  isGeneralTag, type FavTag,
} from "../utils/favTags";
import { pkgApps, loadPkgApps, appLabel } from "../utils/appRegistry";

const emit = defineEmits<{ (e: "close"): void }>();

reloadTags();
loadPkgApps(true);
const tags = computed(() => sortedTags());
const scopeOpen = ref("");

// 可选 app：package_map 里的 + 标签上引用了但已不在 package_map 里的（不然没法取消勾选）
const appOptions = computed(() => {
  const list = pkgApps.value.map((a) => a.package);
  for (const t of tags.value) for (const p of t.apps || []) if (!list.includes(p)) list.push(p);
  return list;
});
function scopeText(t: FavTag): string {
  return isGeneralTag(t) ? "通用" : t.apps!.map(appLabel).join("、");
}
const newScopeText = computed(() => (newApps.value.length ? newApps.value.map(appLabel).join("、") : "通用"));
function toggleNewApp(pkg: string) {
  newApps.value = newApps.value.includes(pkg) ? newApps.value.filter((p) => p !== pkg) : [...newApps.value, pkg];
}
function toggleApp(t: FavTag, pkg: string) {
  const cur = t.apps || [];
  const next = cur.includes(pkg) ? cur.filter((p) => p !== pkg) : [...cur, pkg];
  if (!setTagApps(t.id, next)) err.value = favTagsError.value;
  else err.value = "";
}
function setGeneral(t: FavTag) {
  if (!setTagApps(t.id, [])) err.value = favTagsError.value;
  else err.value = "";
}
const err = ref("");
const newName = ref("");
// 新建标签的范围：空 = 通用。建完保留（连着给同一个 app 建几个标签时不用每次重选）
const newApps = ref<string[]>([]);
const newScopeOpen = ref(false);
const editing = ref<Record<string, string>>({});
const colorOpen = ref("");
const confirmDel = ref("");

// 中文输入法里按回车是「确认拼音上屏」，不能当成提交
function isIme(e: KeyboardEvent): boolean {
  return e.isComposing || e.keyCode === 229;
}
function create() {
  if (!newName.value.trim()) return;
  if (!addTag(newName.value, newApps.value)) return (err.value = favTagsError.value);
  err.value = "";
  newName.value = "";
}
function commitRename(id: string, oldName: string) {
  const v = editing.value[id];
  delete editing.value[id];
  if (v === undefined || v.trim() === oldName) return;
  if (!renameTag(id, v)) err.value = favTagsError.value;
  else err.value = "";
}
function pickColor(id: string, c: string) {
  colorOpen.value = "";
  if (!setTagColor(id, c)) err.value = favTagsError.value;
}
function doDelete(id: string) {
  confirmDel.value = "";
  if (!removeTag(id)) err.value = favTagsError.value;
  else err.value = "";
}
</script>

<template>
  <div class="tm-overlay" @click.self="emit('close')">
    <div class="tm-dialog">
      <div class="tm-head">
        <span>管理标签</span>
        <button class="tm-x" @click="emit('close')">×</button>
      </div>
      <div class="tm-hint">
        评论收藏和邮件收藏共用这套标签。点标签右侧的范围按钮可设为「通用」或只用于某几个 app——
        只用于某 app 的标签，只在该 app 的评论（和关联了该 app 的邮件源）上出现。
        删除标签不会取消收藏，只是这条上不再显示该标签。
      </div>
      <div v-if="err" class="tm-err">{{ err }}</div>
      <div v-if="!tags.length" class="tm-empty">还没有标签</div>
      <div v-for="t in tags" :key="t.id" class="tm-item">
        <div class="tm-row">
          <span class="tm-color-wrap">
            <button class="tm-color" :style="{ background: t.color }" @click="colorOpen = colorOpen === t.id ? '' : t.id" title="改颜色"></button>
            <span v-if="colorOpen === t.id" class="tm-palette">
              <button
                v-for="c in TAG_PALETTE"
                :key="c"
                class="tm-swatch"
                :class="{ on: c === t.color }"
                :style="{ background: c }"
                @click="pickColor(t.id, c)"
              ></button>
            </span>
          </span>
          <input
            class="tm-name"
            :value="editing[t.id] ?? t.name"
            maxlength="20"
            @input="editing[t.id] = ($event.target as HTMLInputElement).value"
            @blur="commitRename(t.id, t.name)"
            @keydown.enter="!isIme($event) && ($event.target as HTMLInputElement).blur()"
          />
          <button
            class="tm-scope"
            :class="{ general: isGeneralTag(t), on: scopeOpen === t.id }"
            :title="scopeText(t)"
            @click="scopeOpen = scopeOpen === t.id ? '' : t.id"
          >{{ scopeText(t) }}</button>
          <template v-if="confirmDel === t.id">
            <button class="tm-del sure" @click="doDelete(t.id)">确认删除</button>
            <button class="tm-del" @click="confirmDel = ''">取消</button>
          </template>
          <button v-else class="tm-del" @click="confirmDel = t.id">删除</button>
        </div>
        <div v-if="scopeOpen === t.id" class="tm-scope-panel">
          <label class="tm-scope-opt">
            <input type="radio" :checked="isGeneralTag(t)" @change="setGeneral(t)" />
            <span>通用（所有 app）</span>
          </label>
          <div class="tm-scope-sub">或只用于：</div>
          <div v-if="!appOptions.length" class="tm-scope-sub">
            还没有 app 清单，去 Play Console 模板管理「管理关联」里添加包名
          </div>
          <label v-for="p in appOptions" :key="p" class="tm-scope-opt">
            <input type="checkbox" :checked="(t.apps || []).includes(p)" @change="toggleApp(t, p)" />
            <span>{{ appLabel(p) }}</span>
          </label>
        </div>
      </div>
      <div class="tm-new">
        <input v-model="newName" maxlength="20" placeholder="新标签名" @keydown.enter.prevent="!isIme($event) && create()" />
        <button
          class="tm-scope"
          :class="{ general: !newApps.length, on: newScopeOpen }"
          :title="'新标签的适用范围：' + newScopeText"
          @click="newScopeOpen = !newScopeOpen"
        >{{ newScopeText }}</button>
        <button class="tm-create" :disabled="!newName.trim()" @click="create">新建</button>
      </div>
      <div v-if="newScopeOpen" class="tm-scope-panel tm-new-scope">
        <label class="tm-scope-opt">
          <input type="radio" :checked="!newApps.length" @change="newApps = []" />
          <span>通用（所有 app）</span>
        </label>
        <div class="tm-scope-sub">或只用于：</div>
        <div v-if="!appOptions.length" class="tm-scope-sub">
          还没有 app 清单，去 Play Console 模板管理「管理关联」里添加包名
        </div>
        <label v-for="p in appOptions" :key="p" class="tm-scope-opt">
          <input type="checkbox" :checked="newApps.includes(p)" @change="toggleNewApp(p)" />
          <span>{{ appLabel(p) }}</span>
        </label>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tm-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1100;
  padding: 20px;
}
.tm-dialog {
  background: white;
  border-radius: 12px;
  width: 100%;
  max-width: 480px;
  max-height: 80vh;
  overflow-y: auto;
  padding: 16px 18px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.25);
}
.tm-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 15px;
  font-weight: 600;
  color: #2d3748;
  margin-bottom: 6px;
}
.tm-x {
  border: none;
  background: transparent;
  font-size: 20px;
  cursor: pointer;
  color: #a0aec0;
}
.tm-hint {
  font-size: 12px;
  color: #718096;
  margin-bottom: 10px;
  line-height: 1.5;
}
.tm-err {
  font-size: 12px;
  color: #c53030;
  background: #fff5f5;
  border: 1px solid #fed7d7;
  border-radius: 6px;
  padding: 6px 8px;
  margin-bottom: 8px;
}
.tm-empty {
  font-size: 12px;
  color: #a0aec0;
  padding: 8px 0;
}
.tm-item + .tm-item {
  border-top: 1px dashed #edf2f7;
}
.tm-new-scope {
  margin: 8px 0 0;
}
.tm-scope {
  max-width: 120px;
  font-size: 12px;
  padding: 4px 8px;
  border: 1px solid #bee3f8;
  border-radius: 5px;
  background: #ebf8ff;
  color: #2b6cb0;
  cursor: pointer;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.tm-scope.general {
  border-color: #e2e8f0;
  background: #f7fafc;
  color: #4a5568;
}
.tm-scope.on {
  box-shadow: 0 0 0 2px #bee3f8;
}
.tm-scope-panel {
  margin: 0 0 8px 24px;
  padding: 8px 10px;
  background: #f7fafc;
  border-radius: 6px;
}
.tm-scope-opt {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: #2d3748;
  padding: 3px 0;
  cursor: pointer;
}
.tm-scope-sub {
  font-size: 11px;
  color: #a0aec0;
  padding: 4px 0 2px;
}
.tm-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 0;
}
.tm-color-wrap {
  position: relative;
}
.tm-color {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  border: 2px solid white;
  box-shadow: 0 0 0 1px #cbd5e0;
  cursor: pointer;
}
.tm-palette {
  position: absolute;
  top: 24px;
  left: 0;
  z-index: 5;
  display: grid;
  grid-template-columns: repeat(5, 20px);
  gap: 5px;
  padding: 6px;
  background: white;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.12);
}
.tm-swatch {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  border: 2px solid white;
  cursor: pointer;
}
.tm-swatch.on {
  box-shadow: 0 0 0 2px #2d3748;
}
.tm-name {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  padding: 5px 7px;
  border: 1px solid #e2e8f0;
  border-radius: 5px;
}
.tm-del {
  font-size: 12px;
  padding: 4px 8px;
  border: 1px solid #e2e8f0;
  border-radius: 5px;
  background: white;
  color: #718096;
  cursor: pointer;
  white-space: nowrap;
}
.tm-del.sure {
  background: #e53e3e;
  border-color: #e53e3e;
  color: white;
}
.tm-new {
  display: flex;
  gap: 6px;
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid #edf2f7;
}
.tm-new input {
  flex: 1;
  font-size: 13px;
  padding: 5px 7px;
  border: 1px solid #e2e8f0;
  border-radius: 5px;
}
.tm-new .tm-create {
  font-size: 13px;
  padding: 5px 12px;
  border: 1px solid #3182ce;
  border-radius: 5px;
  background: #3182ce;
  color: white;
  cursor: pointer;
}
.tm-new .tm-create:disabled {
  opacity: 0.5;
  cursor: default;
}
</style>

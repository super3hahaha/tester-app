# 踩过的坑

> 平台怪癖、外部 API 隐式约束、非显然的 bug 根因。写给未来接手的自己。

## Vue `computed` 会永久缓存 `new Date()` —— 常驻 app 跨天不更新

**现象**：`ReviewPage.vue` 日期选择器把「今天」置灰。系统时间明明是当天，`<input type=date>` 却禁掉了最近几天。

**根因**：`computed` 只在**响应式依赖**变化时重算。若 getter 里调 `new Date()` / `Date.now()`（非响应式），首次求值后结果被**永久缓存**，永不更新。

```js
const maxSelectableDate = computed(() => todayIso());  // ❌ 首次算完就冻结
```

这个 app 是**常驻巡检工具**（用户长期挂着不重启），所以缓存问题会实际暴露：computed 停在 app 打开那天的日期，之后每跨一天，「今天」就多被置灰一格。

**修复套路**（见 `ReviewPage.vue` 的 `dayTick`）：
1. 加一个响应式信号 ref（`const dayTick = ref(0)`），getter 里 `void dayTick.value` 显式建立依赖。
2. 跨天时 `dayTick++` 触发重算。
3. 跨天检测双保险：`visibilitychange`（切回窗口时）**+** `setInterval` 每分钟兜底——因为前台常驻时 `visibilitychange` 根本不触发。`onUnmounted` 记得 `clearInterval`。

**推广**：本项目任何"依赖当前时间"的 computed/派生值都有这个风险（因为 app 常驻）。凡是 getter 里出现 `new Date()`、`Date.now()`、"今天/本周/最近 N 天"，都要问一句"跨天后会不会自动更新"。

**顺带**：跨天判断别用 `toDate.value === today`（用户手动选历史日期时会误触发平移），用 `maxSelectableDate.value === today` 只在真跨天时动。

# Gotchas

## MainPage 子页用 v-show，组件不重新挂载

`MainPage.vue` 的所有子页（Sheets/Generate/Compare/Review/BatchReply/Settings）都用 `v-show` 切换，**组件常驻挂载**，`onMounted` 只在 MainPage 首次挂载时跑一次。

后果：任何「在 onMounted 里读一次 localStorage/状态」的子页，在用户去别的子页改了配置再切回来时**不会刷新**。

典型坑：BatchReply Config 页保存配置 → 切到 Run 页仍显示「未配置启用任何应用」，且「拉取候选评论」按钮在 `groups.length===0` 时禁用，连 `handleFetch` 里的兜底重建都点不动 —— 彻底卡死。

修复模式：把 `activeOption` 当 prop 传给子页，`watch` 它，页面变为可见时重新读状态。注意加守卫（如 BatchReplyPage 只在 `fetchedAt===null` 时重建，避免清掉已拉取的候选）。

## Claude CLI 凭据在 macOS 上存钥匙串，不是文件

Claude Code 在 Linux/Windows 把 OAuth 凭据写 `~/.claude/.credentials.json`，但 **macOS 上存系统登录钥匙串**（service 名 `Claude Code-credentials`），该文件在 mac 上根本不存在。

后果：只读文件判断登录态会在 mac 上永远报"未登录"（Settings 页的 Claude CLI 卡片）。

修复：`claude.rs::read_credentials_json()` 先读文件，读不到时在 `#[cfg(target_os = "macos")]` 下回退执行 `security find-generic-password -s "Claude Code-credentials" -w`，钥匙串里存的就是和文件一模一样的 `{"claudeAiOauth":{...}}` JSON。`get_claude_account` 和 `load_claude_token` 都走这个 helper。

## BatchReply：拉取并行 + 匹配按钮没等拉取完成 → 整个 app 被静默漏掉

`handleFetch` 用 `Promise.all` 并行拉取各 app，`fetchOne` 一进来就 `g.candidates = []`。一个 app 拉完、另一个还在飞时，`totalCandidates>0` 已成立。

坑：「匹配模板并填充」「一键提交全部」按钮的 `:disabled` 早期**只看 `totalCandidates===0`，没看 `fetching`**。于是慢拉取的那个 app 还在飞（candidates 暂为 []）时点匹配 → `buildSkillGroups` 里该 group 0 条 → 整个 group 不进 skill 输入 → 永不匹配。之后它拉回来显示出候选但回复是空的，「一键提交全部」只提交有内容的 → 该 app 被静默漏掉。从落盘的 `~/.tester-app/reviews/pending-reviews-*.json`（skill 输入）能看出哪个 group 没进去。

修复：两个按钮的 `:disabled` 都加 `fetching ||`，`generateReplies` 入口加 `if (fetching || groups.some(g=>g.loading)) return`。

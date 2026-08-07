/**
 * Gmail → Google Sheet 同步脚本（第一阶段）
 *
 * 用途：绕开 Gmail OAuth「Testing 模式 refresh_token 7 天过期」限制。
 *       本脚本以 Gmail 账号本人身份运行（Apps Script 触发器授权长期有效，
 *       不受第三方 OAuth 同意屏幕发布状态影响），定时把邮件写入 Sheet，
 *       再把该 Sheet 共享给 inshot.com 账号，由外部 app 读取。
 *
 * 多标签同步（重要）：
 *   - 支持同时同步「多个标签」，每个标签各写各的表格分页（同一个 Spreadsheet
 *     下的不同 sheet），互不干扰。要同步哪些标签 → 写到哪个分页，改下面
 *     SYNC_JOBS 数组即可；以后再加标签也只需加一行配置，不用复制整套逻辑。
 *
 * 同步策略（重要）：
 *   - 「有没有同步过」以【表里有没有这个 messageId】为准，不看邮件的已读状态。
 *   - 为什么不用「未读」当队列（2026-08 踩坑，勿改回去）：
 *     Gmail 的「标记为已读」是按【整条会话】生效的，而且网页端点开会话就会自动标已读。
 *     所以只要一封新邮件在脚本跑之前被人看过一眼（回复完顺手标已读、或纯粹点开瞄了下），
 *     它就会被 is:unread 永久跳过，【这封邮件再也不会进表】。实测确认过：同一次运行里，
 *     未读的那封正常入表，被手动点开过的那封（用户对我方回复的追问）永久丢失。
 *     换成 messageId 判重后，谁怎么点已读都不影响同步的正确性。
 *   - 扫描范围：标签下最近 MAX_THREADS 个会话中，最后活动在 SYNC_WINDOW_DAYS 天内的；
 *     窗口外的老会话直接跳过，避免历史积压（如 gmail-clear-backlog.gs 处理过的）被翻出来重灌。
 *   - 自己发出的邮件不入表（按发件人排除）。原先靠「自己发的天然已读」顺带过滤，
 *     不再依赖已读状态后必须显式排除，否则我方回复会被当成用户来信写进表。
 *   - 追回复识别：会话里在我方回复【之后】才到的用户来信，标记为「追回复」写入末列，
 *     并给会话打上 FOLLOWUP_LABEL 标签、且【不】标已读——这类邮件最容易被淹没在
 *     新反馈里，Gmail 侧和 app 侧都要能一眼认出来。
 *   - 标记已读现在只是「已同步」的视觉提示（未读列表 ≈ 还没入表的），不再影响正确性。
 *
 * 部署步骤：
 *   1. 用「该 Gmail 账号本人」打开 https://script.google.com 新建项目，粘贴本文件。
 *   2. 左侧「服务」→ 加一个 Advanced Google Service：Gmail API（v1，见 getReceivedDate_ 的注释）。
 *      若项目绑定的是自建 GCP 项目（不是 Apps Script 默认项目），还要去
 *      Cloud Console 手动启用一次 Gmail API（Apps Script 会在报错里给出直达链接）。
 *   3. 先手动运行一次 setup()，弹出授权时同意（授权的是账号本人，不会 7 天过期）。
 *      新增/更换高级服务后必须在编辑器里手动跑一次函数走一遍新的授权弹窗，
 *      否则触发器静默跑会因为没授权而失败。
 *   4. setup() 会：给 SYNC_JOBS 里每个标签建表/补表头 + 安装每天 SYNC_HOUR
 *      点运行一次的触发器。若 SPREADSHEET_ID 留空，会自动新建表格并在日志
 *      打印 URL —— 把表共享给 inshot 账号即可（只需共享一次，所有标签的
 *      分页都在同一个表格里）。
 *   5. 之后触发器会自动跑 syncMail()，也可随时手动运行 syncMail() 立即同步一次。
 *
 * 每个 Gmail 账号各自部署一份本脚本、各自一张表格（多账号方案）。
 */

// ==================== 配置区 ====================
// 目标表格 ID：留空则首次自动新建并把 ID 存入脚本属性（日志会打印 URL）
// 注意：所有标签共用同一个 Spreadsheet，只是各写各的分页（sheet）。
const SPREADSHEET_ID = "";
// 要同步的标签列表：每项 { label: Gmail 里显示的标签名（原样填，含 emoji/空格/大小写；
// 填 "INBOX" 表示整个收件箱）, sheetName: 写入的工作表名（分页名，需互不相同） }
const SYNC_JOBS = [
  { label: "⭐VideoToMP3(50字+)", sheetName: "Mail" },
  { label: "⭐mp3cutter(50字+)", sheetName: "Mail_MP3Cutter" },
];
// 单次同步每个标签最多扫描的会话（线程）数。注意是「会话」不是「邮件」——一个会话里
// 的所有邮件都会被遍历，且会话按「最近活动时间」排序（老会话被回复会冒到最前）。
//
// 这个值的真实含义是【容错期】：它决定脚本能往回看多少天。第 MAX_THREADS+1 名开外的
// 会话，脚本根本不知道其存在——平时无所谓（每天都跑，新邮件永远在前排），但脚本一旦
// 连续停跑超过这个天数（触发器失效 / 授权过期 / 放假撞上 bug），更早的邮件就永久看不见了。
//
// 定值依据（2026-08 实测，mp3cutter 标签）：约 8.3 个会话/天，高峰期（发版后）约 9.8/天。
// 按「最多离开 14 天」的需求：14 × 8.3 ≈ 116，取 150 留余量 → 常态覆盖 ~18 天、高峰 ~14 天。
// 成本很低：会话不足时 getThreads 返回实际数量（稀疏标签零代价），遍历约 18 秒/标签。
// 想知道当前实际覆盖多少天，跑 gmail-audit-missing.gs 看「⏳ 容错期」那行。
const MAX_THREADS = 150;
// 正文截断上限，避免超出单元格 5 万字符上限
const BODY_MAX_CHARS = 45000;
// 每天定时同步的小时（24 小时制，按脚本项目时区）。例：9 = 每天早上 9 点附近跑一次。
// 注意：Apps Script 每日触发器不是精确整点，而是该小时内的某时刻（如 9:00–10:00 之间）。
const SYNC_HOUR = 8;
// 机翻单封最多翻译的字符数（翻译服务单次有上限，过长会失败；超出部分不翻）
const TRANSLATE_MAX_CHARS = 5000;
// 只处理最后活动在多少天内的会话。作用有二：
//   1) 防止历史已读邮件被一次性翻出来重灌（换判重方式后，老邮件不再被已读状态挡住）；
//   2) 限制每次扫描量。日常增量用不到这么大，30 天很宽裕。
// ⚠️ 首次改用本版本时，最近 SYNC_WINDOW_DAYS 天内「已读但从没入表」的邮件会被补写进来，
//    这是预期行为（就是在捞回之前被已读吞掉的那些），补完一次之后就稳定了。
const SYNC_WINDOW_DAYS = 30;
// 给「用户追回复」的会话打的 Gmail 标签名（不存在会自动创建）；留空则不打标签。
// 这类会话不会被标已读，配合标签在 Gmail 侧一眼可见。
const FOLLOWUP_LABEL = "🔁待跟进";
// 同步成功后是否把邮件标记为已读。现在仅作为「已同步」的视觉提示（未读列表 ≈ 还没入表的），
// 不再是判重依据——设成 false 也不会导致重复写入或漏同步，只是未读会堆着。
// 注意：追回复不受此开关影响，永远保持未读，避免它在 Gmail 里消失。
const MARK_READ_AFTER_SYNC = true;
// ================================================

// 表头（顺序固定，外部 app 按列读取）
// 前两列 messageId/threadId 为机器字段（去重 + 拼会话链接），其余为人可读列。
// 机翻中文列(G)在写入时由 LanguageApp 翻译为静态值（非公式，避免打开表反复重算）；正文列(F)为原文。
// 末列「追回复」为后加的列：app 侧按表头名取列（GmailPage.vue 的 colIndex），
// 加在末尾对老数据和老版本 app 都兼容——老行读出来是空串，等价于「否」。
var HEADERS = ["messageId", "threadId", "日期", "发件人", "主题", "正文", "机翻中文", "附件", "邮件链接", "追回复"];
var COL_BODY = 6;   // 正文列(F)，正文/机翻两列自动换行起点
// PropertiesService 中记录自动新建表格 ID 的 key
var PROP_SPREADSHEET_ID = "SPREADSHEET_ID";

/**
 * 一次性初始化入口：授权 + 给每个标签建表/补表头 + 安装触发器。
 * 首次部署后手动运行这一个函数即可；以后往 SYNC_JOBS 加新标签后重新运行一次即可补建分页。
 */
function setup() {
  console.log("🚀 开始初始化...");
  try {
    var ssUrl = null;
    var sheetNames = [];
    for (var i = 0; i < SYNC_JOBS.length; i++) {
      var job = SYNC_JOBS[i];
      var sheet = getSheet_(job.sheetName);
      ensureHeader_(sheet);
      sheetNames.push(job.sheetName);
      if (!ssUrl) ssUrl = sheet.getParent().getUrl();
    }
    installTrigger_();
    console.log("👉 目标表格：" + ssUrl);
    console.log("👉 当前账号：" + getAccountEmail_());
    console.log("👉 已建/补齐分页：" + sheetNames.join("、"));
    console.log("🎉 初始化完成！触发器已装，每天 " + SYNC_HOUR + " 点附近自动同步一次（脚本时区：" + Session.getScriptTimeZone() + "）。");
    console.log("提示：把上面的表格共享给 inshot 账号（共享一次即可，覆盖所有分页）；现在可手动运行 syncMail() 立即同步一次。");
  } catch (e) {
    console.log("❌ 初始化失败：" + e);
    throw e;
  }
}

/**
 * 主同步函数：依次同步 SYNC_JOBS 里的每个标签。触发器调用，也可手动运行。
 * 某个标签同步失败不影响其他标签（各自 try/catch），避免一个标签出错拖垒全部。
 */
function syncMail() {
  console.log("🚀 syncMail 开始（共 " + SYNC_JOBS.length + " 个标签）...");
  for (var i = 0; i < SYNC_JOBS.length; i++) {
    var job = SYNC_JOBS[i];
    try {
      syncOneLabel_(job.label, job.sheetName);
    } catch (e) {
      console.log("❌ 标签「" + job.label + "」同步出错：" + e);
    }
  }
  console.log("🎉 全部标签同步完成。");
}

/**
 * 同步单个标签到指定分页：增量拉取未读邮件并写入表格。
 */
function syncOneLabel_(label, sheetName) {
  console.log("—— 标签「" + label + "」→ 分页「" + sheetName + "」——");
  var sheet = getSheet_(sheetName);
  ensureHeader_(sheet);

  // 按标签名精确取线程（避开 search 对 emoji/特殊字符标签匹配不到的问题），再从中筛未读
  var threads = getTargetThreads_(label);
  console.log("📨 标签「" + label + "」下取到最近 " + threads.length + " 个会话（将从中筛未读）。");

  if (threads.length === 0) {
    console.log("🎉 该标签下没有邮件，无需同步。");
    return;
  }

  var existing = getExistingMessageIds_(sheet);   // 判重游标：表里已有的 messageId
  var account = getAccountEmail_();
  var rows = [];
  var toMarkRead = [];        // 已入表且当前未读的普通来信，写表后统一标已读（视觉提示）
  var followUpThreads = [];   // 含追回复的会话，写表后打 FOLLOWUP_LABEL
  var followUpCount = 0;
  var cutoff = new Date(Date.now() - SYNC_WINDOW_DAYS * 24 * 60 * 60 * 1000);
  var skippedOld = 0;
  var failedThreads = 0;

  for (var i = 0; i < threads.length; i++) {
    var thread = threads[i];

    // 会话级容错：单个会话读不了不能拖垮整个标签（旧版只有标签级 try/catch，
    // 一封坏邮件 = 这个标签当次 150 个会话全废）。
    // 踩坑：Gmail 里偶尔混着读不了的消息（Chat 消息、已删但标签还挂着的邮件等），
    // 会抛 "Gmail operation not allowed"。旧版靠 isUnread 把老邮件全跳过了，
    // 从没碰过它们，所以从没暴露；改成按 messageId 判重后第一次真正去读就炸了。
    try {
      // 剪枝：整条会话最后活动都在窗口外 → 不看。老会话被新回复激活时
      // getLastMessageDate 会变成新时间，所以不会漏掉「隔很久才来的追回复」。
      if (thread.getLastMessageDate() < cutoff) {
        skippedOld++;
        continue;
      }

      var threadId = thread.getId();
      var messages = thread.getMessages();
      var iRepliedBefore = false; // 沿会话时间轴前进，记录「在这封之前我方是否已回复过」
      var threadHasFollowUp = false;

      for (var j = 0; j < messages.length; j++) {
        var msg = messages[j];

        // 我方发出的：不入表，只用来把 iRepliedBefore 置位。
        // （不再依赖已读状态过滤后，这一步是必需的，否则我方回复会被当成用户来信写进表。）
        if (isFromMe_(msg, account)) {
          iRepliedBefore = true;
          continue;
        }

        var isFollowUp = iRepliedBefore; // 我方回复之后才到的用户来信 = 追回复
        var mid = msg.getId();

        if (existing[mid]) {
          // 已同步过：不重复写，只做已读收尾。追回复永远保持未读。
          if (msg.isUnread() && !isFollowUp) toMarkRead.push(msg);
          continue;
        }

        // 逐封再按可靠时间（internalDate）卡一次窗口：老会话被激活时，
        // 只补写窗口内的新消息，不把同会话里几个月前的历史一起灌进来。
        var received = getReceivedDate_(msg);
        if (received < cutoff) continue;

        existing[mid] = true;

        var body = extractBody_(msg);
        var attachments = extractAttachments_(msg);
        var link = "https://mail.google.com/mail/u/?authuser=" + encodeURIComponent(account) + "#all/" + threadId;

        rows.push([
          mid,
          threadId,
          received,
          msg.getFrom(),
          msg.getSubject(),
          body,
          translateZh_(body),   // 机翻中文：写入时翻成静态值（非公式）
          attachments,
          link,
          isFollowUp ? "是" : ""
        ]);

        if (isFollowUp) {
          followUpCount++;
          threadHasFollowUp = true;
        } else if (msg.isUnread()) {
          toMarkRead.push(msg);
        }
      }

      if (threadHasFollowUp) followUpThreads.push(thread);
    } catch (e) {
      failedThreads++;
      var badId = "";
      try { badId = thread.getId(); } catch (e2) { badId = "(连 threadId 都读不出来)"; }
      console.log("⚠️ 第 " + (i + 1) + " 个会话读取失败，已跳过（不影响其他会话）：" + e
        + " ｜ https://mail.google.com/mail/u/?authuser=" + encodeURIComponent(account) + "#all/" + badId);
    }
  }

  if (skippedOld > 0) {
    console.log("⏭️ 跳过 " + skippedOld + " 个最后活动早于 " + SYNC_WINDOW_DAYS + " 天的老会话。");
  }
  if (failedThreads > 0) {
    console.log("⚠️ 有 " + failedThreads + " 个会话读取失败被跳过（详见上方，逐条带链接）。");
  }

  // 先写表，确保数据落地后再动邮件状态，避免「改了状态却没入表」
  if (rows.length > 0) {
    var startRow = sheet.getLastRow() + 1;
    sheet.getRange(startRow, 1, rows.length, HEADERS.length).setValues(rows);
    SpreadsheetApp.flush();
    console.log("📦 新写入 " + rows.length + " 封邮件（从第 " + startRow + " 行）"
      + (followUpCount > 0 ? "，其中 🔁 用户追回复 " + followUpCount + " 封。" : "。"));
  } else {
    console.log("📦 窗口内没有新邮件（表里都有了）。");
  }

  // 下面两步都是「锦上添花」：数据此时已经落表，判重也不依赖它们，
  // 所以各自 try/catch——打标签/标已读失败不该把整次同步判成失败。
  // 给追回复的会话打标签，方便在 Gmail 侧直接筛出来跟进
  if (FOLLOWUP_LABEL && followUpThreads.length > 0) {
    try {
      var fl = getOrCreateLabel_(FOLLOWUP_LABEL);
      for (var f = 0; f < followUpThreads.length; f++) {
        followUpThreads[f].addLabel(fl);
      }
      console.log("🔁 已给 " + followUpThreads.length + " 个会话打上「" + FOLLOWUP_LABEL + "」标签（保持未读）。");
    } catch (e) {
      console.log("⚠️ 打「" + FOLLOWUP_LABEL + "」标签失败（数据已入表，不影响同步）：" + e);
    }
  }

  // 标已读：现在只是「已入表」的视觉提示，不影响判重；追回复不在其中
  if (MARK_READ_AFTER_SYNC && toMarkRead.length > 0) {
    try {
      GmailApp.markMessagesRead(toMarkRead);
      console.log("✅ 已把 " + toMarkRead.length + " 封标记为已读（已入表标记）。");
    } catch (e) {
      console.log("⚠️ 标记已读失败（数据已入表，不影响同步）：" + e);
    }
  }
}

/**
 * 判断是不是我方（脚本所有者）发出的邮件。
 * getFrom() 形如 "Name <a@b.com>"，用邮箱地址子串匹配即可。
 */
function isFromMe_(msg, account) {
  if (!account || account === "me") return false;
  var from = msg.getFrom() || "";
  return from.toLowerCase().indexOf(account.toLowerCase()) >= 0;
}

/**
 * 取标签，不存在则创建。
 */
function getOrCreateLabel_(name) {
  var l = GmailApp.getUserLabelByName(name);
  return l ? l : GmailApp.createLabel(name);
}

/**
 * 按标签精确取最近 MAX_THREADS 个会话。
 * INBOX 走收件箱；其余按标签名精确匹配（含 emoji/中文/空格都 OK）。
 * 注意：返回的是该范围内的全部会话（含已读），未读筛选在 syncOneLabel_ 里按 isUnread 做。
 */
function getTargetThreads_(label) {
  if (label.toUpperCase() === "INBOX") {
    return GmailApp.getInboxThreads(0, MAX_THREADS);
  }
  var l = GmailApp.getUserLabelByName(label);
  if (!l) {
    throw new Error("找不到标签：「" + label + "」。请确认 SYNC_JOBS 里的 label 与 Gmail 里显示的名字完全一致（含 emoji、空格、大小写）。");
  }
  return l.getThreads(0, MAX_THREADS);
}

/**
 * 邮件真实收信时间：优先用 Gmail 服务器侧 internalDate（收到时打的服务器时间戳，不可伪造）；
 * 失败时回退到邮件 Date 头（msg.getDate()）。
 * 踩坑记录：Date 头是发件方设备自己写的，若对方系统时钟不对（常见于老旧/被改过时间的手机），
 * 这个头就会是错的（曾出现同步进表后日期显示成两年前，但 Gmail 网页端打开该邮件显示的时间是对的——
 * 因为网页端展示用的正是 internalDate）。需要开通 Gmail 高级服务（Advanced Google Service，
 * 左侧「服务」→ + → Gmail API），若项目绑定自建 GCP 项目还要去 Cloud Console 手动启用一次
 * gmail.googleapis.com（Apps Script 报错信息里会给直达链接）。
 */
function getReceivedDate_(msg) {
  try {
    var meta = Gmail.Users.Messages.get("me", msg.getId(), { format: "minimal" });
    if (meta && meta.internalDate) return new Date(Number(meta.internalDate));
  } catch (e) {
    console.log("⚠️ 读取 internalDate 失败，回退到邮件 Date 头：" + e);
  }
  return msg.getDate();
}

/**
 * 提取邮件正文：优先纯文本，否则 HTML 去标签；清洗 + 截断。
 */
function extractBody_(msg) {
  var text = "";
  try {
    text = msg.getPlainBody();
  } catch (e) {
    text = "";
  }
  if (!text) {
    var html = "";
    try { html = msg.getBody(); } catch (e2) { html = ""; }
    text = html
      .replace(/<style[\s\S]*?<\/style>/gi, " ")
      .replace(/<script[\s\S]*?<\/script>/gi, " ")
      .replace(/<br\s*\/?>/gi, "\n")
      .replace(/<\/(p|div|tr|li|h[1-6])>/gi, "\n")
      .replace(/<[^>]+>/g, " ")
      .replace(/&nbsp;/g, " ")
      .replace(/&amp;/g, "&")
      .replace(/&lt;/g, "<")
      .replace(/&gt;/g, ">");
  }
  text = text.replace(/\r/g, "").replace(/[ \t]+/g, " ").trim();
  if (text.length > BODY_MAX_CHARS) {
    text = text.substring(0, BODY_MAX_CHARS) + "…（已截断）";
  }
  return text;
}

/**
 * 把正文机翻成简体中文（静态值，非公式）。
 * 超长截断到 TRANSLATE_MAX_CHARS（翻译服务单次有上限）；失败/空则返回空串。
 */
function translateZh_(text) {
  if (!text) return "";
  var src = text.length > TRANSLATE_MAX_CHARS ? text.substring(0, TRANSLATE_MAX_CHARS) : text;
  try {
    // 源语言留空 = 自动检测；目标 zh-CN 简体
    return LanguageApp.translate(src, "", "zh-CN");
  } catch (e) {
    console.log("⚠️ 机翻失败（跳过该封）：" + e);
    return "";
  }
}

/**
 * 提取附件文件名（排除签名里的内嵌图片）；无附件返回"无"。
 */
function extractAttachments_(msg) {
  var atts = [];
  try {
    atts = msg.getAttachments({ includeInlineImages: false, includeAttachments: true });
  } catch (e) {
    atts = [];
  }
  if (!atts || atts.length === 0) return "无";
  var names = [];
  for (var i = 0; i < atts.length; i++) {
    names.push(atts[i].getName());
  }
  return names.join("; ");
}

/**
 * 读取表格 A 列已有 messageId，返回 {id: true} 映射用于去重。
 */
function getExistingMessageIds_(sheet) {
  var map = {};
  var lastRow = sheet.getLastRow();
  if (lastRow < 2) return map; // 只有表头
  var values = sheet.getRange(2, 1, lastRow - 1, 1).getValues();
  for (var i = 0; i < values.length; i++) {
    var v = values[i][0];
    if (v) map[v] = true;
  }
  return map;
}

/**
 * 获取指定名字的工作表对象（同一个 Spreadsheet 下的分页）；
 * Spreadsheet 不存在则新建，分页不存在则新建。
 */
function getSheet_(sheetName) {
  var ss = null;
  var id = SPREADSHEET_ID;
  if (!id) {
    id = PropertiesService.getScriptProperties().getProperty(PROP_SPREADSHEET_ID);
  }
  if (id) {
    ss = SpreadsheetApp.openById(id);
  } else {
    ss = SpreadsheetApp.create("Gmail同步_" + getAccountEmail_());
    PropertiesService.getScriptProperties().setProperty(PROP_SPREADSHEET_ID, ss.getId());
    console.log("👉 已自动新建表格：" + ss.getUrl());
    console.log("   （建议把此 URL 共享给 inshot 账号；或把 ID 填回配置区 SPREADSHEET_ID 固化。）");
  }
  var sheet = ss.getSheetByName(sheetName);
  if (!sheet) {
    sheet = ss.insertSheet(sheetName);
  }
  return sheet;
}

/**
 * 确保表头存在且正确；缺失或与 HEADERS 不一致（含「老表少了后加的列」）时重写表头。
 * 不能只看第一格是不是 messageId——那样往 HEADERS 末尾加列时，老表永远补不上新列头，
 * app 侧按表头名取列就会一直找不到（GmailPage.vue 的 colIndex 返回 -1）。
 */
function ensureHeader_(sheet) {
  var lastCol = sheet.getLastColumn();
  var current = lastCol > 0 ? sheet.getRange(1, 1, 1, lastCol).getValues()[0] : [];
  var ok = current.length >= HEADERS.length;
  if (ok) {
    for (var i = 0; i < HEADERS.length; i++) {
      if (String(current[i]).trim() !== HEADERS[i]) { ok = false; break; }
    }
  }
  if (ok) return;
  sheet.getRange(1, 1, 1, HEADERS.length).setValues([HEADERS]);
  sheet.setFrozenRows(1);
  sheet.getRange(1, 1, 1, HEADERS.length).setFontWeight("bold");
  // 正文/机翻中文 两列自动换行
  sheet.getRange(1, COL_BODY, sheet.getMaxRows(), 2).setWrap(true);
  console.log("📋 已写入表头。");
}

/**
 * 安装/重装定时触发器（先删同名旧触发器避免重复）。
 */
function installTrigger_() {
  var triggers = ScriptApp.getProjectTriggers();
  for (var i = 0; i < triggers.length; i++) {
    if (triggers[i].getHandlerFunction() === "syncMail") {
      ScriptApp.deleteTrigger(triggers[i]);
    }
  }
  ScriptApp.newTrigger("syncMail")
    .timeBased()
    .everyDays(1)
    .atHour(SYNC_HOUR)
    .create();
  console.log("⏰ 触发器已安装：每天 " + SYNC_HOUR + " 点附近运行 syncMail()。");
}

/**
 * 获取当前账号邮箱（脚本所有者），用于拼 Gmail 深链。
 */
function getAccountEmail_() {
  var email = Session.getEffectiveUser().getEmail();
  if (!email) email = Session.getActiveUser().getEmail();
  return email || "me";
}

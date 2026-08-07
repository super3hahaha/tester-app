/**
 * 【只读诊断】排查 gmail-sync.gs 是否漏同步了邮件（尤其是「用户追回复」）。
 *
 * 背景：gmail-sync.gs 用「未读」当同步队列（扫 is:unread → 写表 → 标已读出队）。
 *       这套机制和「人手动标已读」冲突——Gmail 的标已读是按【整条会话】生效的，
 *       所以只要一封新邮件在脚本跑之前就被标成已读（你回复完顺手标已读整条会话、
 *       或在网页端点开过该会话触发自动已读），脚本下次就会跳过它，这封邮件
 *       【永远不会进表】。本脚本用来验证这件事到底有没有真的发生过。
 *
 * 做什么：拿「标签下最近 AUDIT_THREADS 个会话里的全部收信」和「表里已有的 messageId」
 *         对账，列出所有「在 Gmail 里存在、但表里没有」的收信，并标注它是不是
 *         「你回复之后对方的追回复」（= 最该被看到、丢了损失最大的那一类）。
 *
 * 安全性：全程只读。不写任何表、不改任何邮件的已读状态、不加删标签、不装触发器。
 *         可以随时反复运行。
 *
 * 用法：
 *   1. 把本文件粘进【和 gmail-sync.gs 同一个】Apps Script 项目（新建一个 .gs 文件即可，
 *      不要覆盖 gmail-sync.gs）。同项目才能复用它的 SPREADSHEET_ID / SYNC_JOBS / 授权。
 *   2. 手动运行 auditMissingMail()，看执行日志。
 *   3. 看日志末尾的汇总；重点看「已读 + 追回复 + 未进表」这一类的条数，
 *      每一条都是一次「白回复」的实锤。
 */

// 每个标签回溯检查的会话数。设得比 gmail-sync.gs 的 MAX_THREADS 大，
// 这样能顺带看出「漏掉的邮件是不是因为排在 MAX_THREADS 窗口之外」。
var AUDIT_THREADS = 200;
// 明细最多打印多少条（避免日志被刷爆），汇总统计不受此限制
var AUDIT_PRINT_LIMIT = 60;

function auditMissingMail() {
  var me = auditMyEmail_();
  console.log("🔍 只读诊断开始（当前账号：" + me + "，回溯每标签最近 " + AUDIT_THREADS + " 个会话）");
  console.log("   ⚠️ 本脚本不会修改任何邮件或表格。");

  for (var i = 0; i < SYNC_JOBS.length; i++) {
    var job = SYNC_JOBS[i];
    try {
      auditOneLabel_(job.label, job.sheetName, me);
    } catch (e) {
      console.log("❌ 标签「" + job.label + "」诊断出错：" + e);
    }
  }
  console.log("🔍 诊断结束。");
}

function auditOneLabel_(label, sheetName, me) {
  console.log("");
  console.log("═══ 标签「" + label + "」 ↔ 分页「" + sheetName + "」 ═══");

  var sheet = getSheet_(sheetName);           // 复用 gmail-sync.gs 的函数（只读打开，不建不写）
  var existing = getExistingMessageIds_(sheet); // 表里已同步的 messageId
  var existingCount = 0;
  for (var k in existing) existingCount++;
  console.log("📊 表里已有 " + existingCount + " 条记录。");

  var threads = auditGetThreads_(label);
  console.log("📨 Gmail 侧取到最近 " + threads.length + " 个会话，开始对账…");

  // 容错期：gmail-sync.gs 每次只看最近 MAX_THREADS 个会话，这个数覆盖到多久以前，
  // 就是「脚本最多能挂多少天、恢复后还能把邮件补回来」。它才是真正的瓶颈——
  // SYNC_WINDOW_DAYS 设得比它大的话根本不会生效。
  if (threads.length >= MAX_THREADS) {
    var edge = threads[MAX_THREADS - 1].getLastMessageDate();
    var days = (Date.now() - edge.getTime()) / 86400000;
    console.log("⏳ 容错期：MAX_THREADS(" + MAX_THREADS + ") 覆盖到 "
      + Utilities.formatDate(edge, "GMT+8", "yyyy-MM-dd HH:mm")
      + "，即最近 " + days.toFixed(1) + " 天"
      + "（脚本连续停跑超过这个天数，更早的邮件就再也捞不回来了）。");
  }

  var missing = [];        // 所有「Gmail 有、表里没有」的收信
  var totalIncoming = 0;   // 收信总数（不含我自己发的）
  // ── 「白回复率」的分母：只有「我回复过」的会话才有资格出现追回复，
  //    用 200 封收信当分母会把严重程度严重稀释，必须按会话口径单独统计 ──
  var threadsIReplied = 0;     // 我回复过的会话数
  var threadsFollowedUp = 0;   // 其中对方又追回复了的会话数
  var threadsFollowUpLost = 0; // 其中追回复没进表的会话数（= 白回复的会话）

  for (var t = 0; t < threads.length; t++) {
    var thread = threads[t];
    var messages = thread.getMessages();
    var iRepliedBefore = false; // 沿着会话时间轴往后走，记录「在这封之前我是否已经回复过」
    var thHasFollowUp = false;
    var thFollowUpLost = false;

    for (var j = 0; j < messages.length; j++) {
      var msg = messages[j];
      var from = msg.getFrom() || "";

      // 我自己发出的：不该进表，只用来把 iRepliedBefore 置位
      if (from.indexOf(me) >= 0) {
        iRepliedBefore = true;
        continue;
      }

      totalIncoming++;
      var mid = msg.getId();
      if (iRepliedBefore) thHasFollowUp = true;
      if (existing[mid]) continue; // 已同步，正常
      if (iRepliedBefore) thFollowUpLost = true;

      missing.push({
        threadRank: t + 1,               // 该会话在标签里的新鲜度排名（1 = 最新）
        date: msg.getDate(),
        from: from,
        subject: msg.getSubject() || "",
        isUnread: msg.isUnread(),
        isFollowUp: iRepliedBefore,      // true = 我回复过之后对方又来的信 ← 最关键
        link: "https://mail.google.com/mail/u/?authuser=" + encodeURIComponent(me) + "#all/" + thread.getId()
      });
    }

    if (iRepliedBefore) threadsIReplied++;   // 会话里出现过我发的信 = 我回复过
    if (thHasFollowUp) threadsFollowedUp++;
    if (thFollowUpLost) threadsFollowUpLost++;
  }

  // ── 分类统计 ──
  var nUnread = 0, nReadMissing = 0, nFollowUp = 0, nSmokingGun = 0, nOutOfWindow = 0;
  for (var m = 0; m < missing.length; m++) {
    var x = missing[m];
    if (x.isUnread) nUnread++; else nReadMissing++;
    if (x.isFollowUp) nFollowUp++;
    // 实锤：已读 + 是追回复 + 没进表 —— 脚本永远扫不到它了
    if (!x.isUnread && x.isFollowUp) nSmokingGun++;
    if (x.threadRank > MAX_THREADS) nOutOfWindow++;
  }

  console.log("──────── 对账结果 ────────");
  console.log("收信总数（窗口内，不含我自己发的）：" + totalIncoming);
  console.log("❗ 未进表的收信：" + missing.length + " 封");
  console.log("   ├─ 仍是未读（下次同步还能捞回来，暂时安全）：" + nUnread);
  console.log("   ├─ 已是已读（❌ 脚本再也扫不到，已永久丢失）：" + nReadMissing);
  console.log("   ├─ 其中属于「我回复过之后对方的追回复」：" + nFollowUp);
  console.log("   └─ 🔴 实锤（已读 + 追回复 + 未进表）：" + nSmokingGun + " 封 ← 每一封都是一次白回复");

  // ── 白回复率：用「我回复过的会话」当分母，而不是全部收信 ──
  // 只有我回复过的会话才可能产生追回复，用 200 封收信当分母会把严重程度稀释掉。
  console.log("──────── 白回复率（按会话口径）────────");
  console.log("窗口内我回复过的会话：" + threadsIReplied + " 条");
  console.log("   ├─ 对方又追了回复的：" + threadsFollowedUp + " 条"
    + (threadsIReplied ? "（追问率 " + Math.round(threadsFollowedUp * 100 / threadsIReplied) + "%）" : ""));
  console.log("   └─ 🔴 追回复没进表的：" + threadsFollowUpLost + " 条"
    + (threadsFollowedUp ? "（白回复率 " + Math.round(threadsFollowUpLost * 100 / threadsFollowedUp) + "%）" : ""));

  // 注意：下面这个数字只描述漏掉的邮件「现在排在哪」，不是丢失原因。
  // 邮件不会因为滑出窗口就变已读——它们已是已读，说明是被某次操作主动标掉的
  // （如 gmail-clear-backlog.gs 把历史积压导到了独立表格、或手动批量标已读），
  // 与「未读队列被吞」是两回事，别混为一谈。
  console.log("参考：漏掉的邮件里有 " + nOutOfWindow + " 封当前排在 MAX_THREADS(" + MAX_THREADS + ") 之后"
    + "（仅供定位，不代表丢失原因）。");

  if (missing.length === 0) {
    console.log("✅ 该标签窗口内没有漏同步的收信。");
    return;
  }

  // ── 明细（实锤优先排前面）──
  missing.sort(function (a, b) {
    var sa = (!a.isUnread && a.isFollowUp) ? 0 : (a.isFollowUp ? 1 : 2);
    var sb = (!b.isUnread && b.isFollowUp) ? 0 : (b.isFollowUp ? 1 : 2);
    if (sa !== sb) return sa - sb;
    return b.date - a.date;
  });

  console.log("──────── 明细（最多 " + AUDIT_PRINT_LIMIT + " 条，实锤排最前）────────");
  var n = Math.min(missing.length, AUDIT_PRINT_LIMIT);
  for (var p = 0; p < n; p++) {
    var it = missing[p];
    var tag = (!it.isUnread && it.isFollowUp) ? "🔴实锤" : (it.isFollowUp ? "🟠追回复(未读)" : (it.isUnread ? "⚪未读待同步" : "🟡已读丢失"));
    console.log(
      tag +
      " | " + Utilities.formatDate(it.date, "GMT+8", "yyyy-MM-dd HH:mm") +
      " | 会话排名#" + it.threadRank +
      " | " + auditTrim_(it.from, 40) +
      " | " + auditTrim_(it.subject, 50) +
      " | " + it.link
    );
  }
  if (missing.length > n) console.log("…（还有 " + (missing.length - n) + " 条未打印）");
}

/**
 * 取标签下最近 AUDIT_THREADS 个会话（逻辑同 gmail-sync.gs 的 getTargetThreads_，
 * 只是窗口更大；单独实现一份，避免改动生产脚本）。
 */
function auditGetThreads_(label) {
  if (label.toUpperCase() === "INBOX") {
    return GmailApp.getInboxThreads(0, AUDIT_THREADS);
  }
  var l = GmailApp.getUserLabelByName(label);
  if (!l) throw new Error("找不到标签：「" + label + "」");
  return l.getThreads(0, AUDIT_THREADS);
}

function auditMyEmail_() {
  var email = Session.getEffectiveUser().getEmail();
  if (!email) email = Session.getActiveUser().getEmail();
  return email || "me";
}

function auditTrim_(s, n) {
  s = String(s || "").replace(/\s+/g, " ");
  return s.length > n ? s.substring(0, n) + "…" : s;
}

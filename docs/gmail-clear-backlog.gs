/**
 * 一次性清空某标签下的历史未读积压，导出到独立表格（不会写进 gmail-sync.gs 的 Mail 分页）。
 *
 * 用途：gmail-sync.gs 是「日常增量」脚本，MAX_THREADS 通常设得很小（如 50），
 *       只适合每天消化少量新邮件；如果某个标签下已经攒了几百封未读（比如刚开始
 *       同步一个新标签时），gmail-sync.gs 得跑很多天才能追平。这个脚本专门用来
 *       把历史积压一次性倒出来 + 标记已读，跑完之后未读队列就清空了，之后交给
 *       gmail-sync.gs 做日常增量即可，两者不冲突（各写各的表格）。
 *
 * 用法：
 *   1. 改下面的 labelName 为要清空的标签（跟 Gmail 里显示的名字完全一致）。
 *   2. 手动运行一次 exportEmailsWithProgress()。
 *   3. 如果一次 6 分钟执行时间跑不完，脚本会自动装一个 1 分钟后的触发器接着跑，
 *      不用你守着；全部清空后触发器会自动删除，导出表格链接会打在日志里。
 *   4. 想清空另一个标签，改 labelName 后重新手动运行一次即可（旧的进度记录按
 *      标签名区分，不会互相干扰）。
 *
 * 安全性：
 *   - 只搬「未读」邮件（is:unread），写表 + flush() 成功后才标记已读——半途
 *     报错/被打断，未处理的邮件仍然是未读，重新运行不会漏、也不会重复导出。
 *   - 跨批次复用同一张表格（表格 ID 存在脚本属性里），不会每醒一次就新建一张表。
 */

function exportEmailsWithProgress() {
  const labelName = "⭐mp3cutter-50字+-";
  const query = `label:${labelName} is:unread`;
  const pageSize = 100;
  const TRIGGER_FN = "exportEmailsWithProgress";
  const TIME_LIMIT_MS = 5 * 60 * 1000; // Apps Script 单次执行硬上限 6 分钟，留 1 分钟收尾
  const startTime = Date.now();

  const props = PropertiesService.getScriptProperties();
  const PROP_KEY = "EXPORT_SS_ID_" + labelName; // 按标签名区分进度，清不同标签互不影响

  // 1. 复用上一轮没跑完的表格；没有就新建
  let ss = null;
  const existingId = props.getProperty(PROP_KEY);
  if (existingId) {
    try {
      ss = SpreadsheetApp.openById(existingId);
      console.log("↩️ 继续处理未完成的导出：" + ss.getUrl());
    } catch (e) {
      console.log("⚠️ 之前记录的表格打不开了（可能被删了），新建一份：" + e);
    }
  }
  let sheet;
  if (ss) {
    sheet = ss.getActiveSheet();
  } else {
    const timestamp = Utilities.formatDate(new Date(), "GMT+8", "yyyy-MM-dd HH:mm");
    ss = SpreadsheetApp.create("邮件导出_" + labelName + "_" + timestamp);
    sheet = ss.getActiveSheet();
    const headers = ["日期", "发件人", "主题", "正文(原文)", "机翻中文", "附件", "邮件链接"];
    sheet.appendRow(headers);
    sheet.getRange(1, 1, 1, headers.length).setFontWeight("bold").setBackground("#d9ead3");
    sheet.setFrozenRows(1);
    props.setProperty(PROP_KEY, ss.getId());
    console.log("🆕 新建导出表格：" + ss.getUrl());
  }

  console.log("开始抓取未读邮件并同步标记已读...");
  let totalProcessed = 0;
  let hasMoreWork = true;

  // 未读集合会随着下面 markMessagesRead 变小，所以每轮都从 0 重新取「当前剩下的」未读，
  // 不用像原脚本那样自增 start——已经处理过的不会再出现在 is:unread 结果里。
  while (Date.now() - startTime < TIME_LIMIT_MS) {
    const threads = GmailApp.search(query, 0, pageSize);
    if (!threads || threads.length === 0) {
      hasMoreWork = false;
      break;
    }

    const currentPageRows = [];
    const toMarkRead = [];

    threads.forEach((thread) => {
      const messages = thread.getMessages();
      messages.forEach((message) => {
        if (!message.isUnread()) return; // 同一会话里只处理未读的那几封
        toMarkRead.push(message);

        const date = Utilities.formatDate(message.getDate(), "GMT+8", "yyyy-MM-dd HH:mm:ss");
        const from = message.getFrom();
        const subject = message.getSubject();
        const body = message.getPlainBody().substring(0, 3000); // 截取前3000字
        const threadUrl = `https://mail.google.com/mail/u/0/#inbox/${thread.getId()}`;
        const hasAttachment = message.getAttachments().length > 0 ? "有" : "无";

        currentPageRows.push([date, from, subject, body, "", hasAttachment, threadUrl]);
      });
    });

    if (currentPageRows.length === 0) {
      hasMoreWork = false;
      break;
    }

    // --- 实时写入 ---
    const startRow = sheet.getLastRow() + 1;
    sheet.getRange(startRow, 1, currentPageRows.length, 7).setValues(currentPageRows);

    // 写入翻译公式（E列）
    const formulaRange = sheet.getRange(startRow, 5, currentPageRows.length, 1);
    formulaRange.setFormulaR1C1('=IF(R[0]C[-1]="","",GOOGLETRANSLATE(R[0]C[-1],"auto","zh-CN"))');

    sheet.getRange(startRow, 4, currentPageRows.length, 2).setWrap(true);
    SpreadsheetApp.flush(); // 强制落盘，确保下面标记已读之前数据已经写进表里

    // 写表成功后才标记已读 = 「已同步」，避免「标了已读却没写进表」丢邮件
    GmailApp.markMessagesRead(toMarkRead);

    totalProcessed += currentPageRows.length;
    console.log(`进度：本轮写入并标记已读 ${currentPageRows.length} 封，累计 ${totalProcessed} 封...`);
  }

  // 2. 收尾：清空了就清理状态 + 触发器；没清完就装一个 1 分钟后的一次性触发器接着跑
  if (!hasMoreWork) {
    props.deleteProperty(PROP_KEY);
    removeTrigger_(TRIGGER_FN);
    sheet.setColumnWidth(4, 400);
    sheet.setColumnWidth(5, 400);
    console.log(`🎉 全部处理完成！本次运行处理了 ${totalProcessed} 封，表格：` + ss.getUrl());
    try {
      SpreadsheetApp.getUi().alert(`导出成功！共写入 ${totalProcessed} 条邮件数据。`);
    } catch (e) {
      // 非手动运行（比如被触发器唤醒）拿不到 UI，忽略即可
    }
  } else {
    removeTrigger_(TRIGGER_FN); // 先清掉可能存在的旧触发器，避免重复安装
    ScriptApp.newTrigger(TRIGGER_FN).timeBased().after(60 * 1000).create();
    console.log(`⏳ 本次执行时间到，已处理 ${totalProcessed} 封，未读还没清完，1 分钟后自动继续…`);
  }
}

/**
 * 删掉指定函数名的所有触发器（避免重复安装堆积）。
 */
function removeTrigger_(fnName) {
  ScriptApp.getProjectTriggers().forEach((t) => {
    if (t.getHandlerFunction() === fnName) ScriptApp.deleteTrigger(t);
  });
}

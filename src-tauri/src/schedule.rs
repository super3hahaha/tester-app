// Review 模块「定时通知」后端实现（方案 B）：一个后台 OS 线程每 30 秒 tick 一次，
// 到点就拉当前活跃账号启用的应用评论、按配置筛选、与已通知集合 diff 出新增、组装 HTML
// 消息发到 Telegram。跑在原生线程 + Rust 网络栈，**不受 webview 窗口前后台/节流影响**——
// 只要 app 进程没被 Cmd+Q 杀掉就能准点（睡眠/退出期间不跑，靠启动/下一 tick 补发）。
//
// 为什么不复用前端那套：webview 的 JS 定时器窗口不在前台时会被系统挂起（见 gotchas.md），
// 前端定时器做不到「后台准点」。配置在 localStorage 后端读不到 → 前端在保存/启动/切账号时
// 把「定时配置 + 启用应用及筛选 + 显示名」镜像成 runtime.json（save_schedule_runtime）。
//
// 账号维度：只对**当前活跃账号**生效（读 AuthState::active_key）。切账号后定时对象随之改变，
// 各账号的 runtime / 已触发 / 已通知集合按 key 各自隔离（与前端 scopedKey 同一个 key）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::auth::AuthState;

// ---- 前端镜像过来的运行时配置（camelCase，与前端 TS 一致）----

#[derive(Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub times: Vec<String>, // "HH:MM"
    #[serde(default)]
    pub notify_on_empty: bool,
    #[serde(default = "default_max_items")]
    pub max_items_in_msg: usize,
    // 额外扫「回复后又被用户更新」的评论（开发者回复过、用户又改了评论 → 回复可能过时需复查）。
    #[serde(default)]
    pub check_updated: bool,
}
fn default_max_items() -> usize {
    5
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppEntry {
    pub package_name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub date_preset: String, // sinceLastWorkday|yesterday|today|7d|custom
    #[serde(default)]
    pub custom_from_date: String,
    #[serde(default)]
    pub custom_to_date: String,
    #[serde(default)]
    pub stars: Vec<i32>,
    // ANY|ABSENT|REPLIED|UPDATED，语义与前端 ReviewPage.vue::matchesReplyState 一致。
    // 默认 ANY 兼容旧 runtime.json（字段是后补的）。
    #[serde(default = "default_reply_state")]
    pub reply_state: String,
}
fn default_reply_state() -> String {
    "ANY".to_string()
}

fn matches_reply_state(r: &crate::reviews::Review, state: &str) -> bool {
    match state {
        "ABSENT" => r.developer_reply.is_none(),
        "REPLIED" => r.developer_reply.is_some(),
        "UPDATED" => {
            r.developer_reply.is_some()
                && r.developer_reply_ts.map(|rt| r.user_comment_ts > rt).unwrap_or(false)
        }
        _ => true, // ANY / 未知值不筛
    }
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct ScheduleRuntime {
    #[serde(default)]
    pub schedule: ScheduleCfg,
    #[serde(default)]
    pub apps: Vec<AppEntry>,
}

// ---- per-account 状态文件 ----

#[derive(Deserialize, Serialize, Default)]
struct FiredState {
    date: String,       // YYYY-MM-DD（本地）
    times: Vec<String>, // 今天已触发的时间点
}

#[derive(Deserialize, Serialize, Default, Clone)]
struct AppNotified {
    baseline_done: bool,
    ids: Vec<String>,
}

type NotifiedMap = HashMap<String, AppNotified>;

// 「回复后又更新」已提醒记录：pkg -> (review_id -> 提醒时该评论的 user_comment_ts)。
// 同一条只提醒一次；用户之后又更新（user_comment_ts 变大）则再次提醒。不做 baseline
// —— 已存在的「待复查」是真实待办，首次就该提醒（与"新增差评"不同，那个才需静默基线）。
type UpdatedMap = HashMap<String, HashMap<String, i64>>;

// ---- 路径 ----

fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".tester-app")
}

fn sanitize_key(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn account_dir(key: &str) -> PathBuf {
    data_dir().join("schedule").join(sanitize_key(key))
}

fn runtime_path(key: &str) -> PathBuf {
    account_dir(key).join("runtime.json")
}
fn fired_path(key: &str) -> PathBuf {
    account_dir(key).join("fired.json")
}
fn notified_path(key: &str) -> PathBuf {
    account_dir(key).join("notified.json")
}
fn updated_path(key: &str) -> PathBuf {
    account_dir(key).join("updated.json")
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &PathBuf) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_json_atomic<T: Serialize>(path: &PathBuf, value: &T) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

// ---- 命令：前端镜像配置 ----

#[tauri::command]
pub fn save_schedule_runtime(
    runtime: ScheduleRuntime,
    state: State<'_, AuthState>,
) -> Result<(), String> {
    let key = state
        .active_key()
        .ok_or("未登录，无法保存定时配置".to_string())?;
    write_json_atomic(&runtime_path(&key), &runtime)
}

// ---- 日期预设解析（port 自 batchReplyDates.ts，本地时区）----

fn days_ago(n: i64, base: NaiveDate) -> NaiveDate {
    base - chrono::Duration::days(n)
}

// 「上一个工作日」：周二~周五=昨天，周一=上周五，周六=昨天(周五)，周日=前天(周五)
fn last_workday_before(today: NaiveDate) -> NaiveDate {
    let dow = today.weekday().num_days_from_sunday(); // 0=Sun,1=Mon,...,6=Sat
    let back = match dow {
        1 => 3, // Mon
        0 => 2, // Sun
        _ => 1,
    };
    days_ago(back, today)
}

/// 返回 (from_ts, to_ts) 秒（本地时区，from 当天 00:00:00 / to 当天 23:59:59）。
/// 空/无法解析的自定义日期 → 边界放开（0 / i64::MAX）。
fn resolve_range(entry: &AppEntry, now: NaiveDate) -> (i64, i64) {
    let (from_date, to_date): (Option<NaiveDate>, Option<NaiveDate>) = match entry.date_preset.as_str()
    {
        "sinceLastWorkday" => (Some(last_workday_before(now)), Some(now)),
        "yesterday" => {
            let y = days_ago(1, now);
            (Some(y), Some(y))
        }
        "today" => (Some(now), Some(now)),
        "7d" => (Some(days_ago(6, now)), Some(now)),
        _ => (
            parse_iso(&entry.custom_from_date),
            parse_iso(&entry.custom_to_date),
        ),
    };
    let from_ts = from_date
        .and_then(|d| local_start_of_day(d))
        .unwrap_or(0);
    let to_ts = to_date
        .and_then(|d| local_end_of_day(d))
        .unwrap_or(i64::MAX);
    (from_ts, to_ts)
}

fn parse_iso(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

fn local_start_of_day(d: NaiveDate) -> Option<i64> {
    let ndt = d.and_hms_opt(0, 0, 0)?;
    Local.from_local_datetime(&ndt).single().map(|dt| dt.timestamp())
}
fn local_end_of_day(d: NaiveDate) -> Option<i64> {
    let ndt = d.and_hms_opt(23, 59, 59)?;
    Local.from_local_datetime(&ndt).single().map(|dt| dt.timestamp())
}

// ---- 消息组装 ----

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn truncate_chars(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > n {
        let mut out: String = chars[..n].iter().collect();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

// 单个 app 的结果。item = (star_rating, text, user_comment_ts)
struct AppResult {
    display_name: String,
    new_items: Vec<(i32, String, i64)>,
    updated_items: Vec<(i32, String, i64)>,
}

fn star_counts_str(items: &[(i32, String, i64)]) -> String {
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for (star, _, _) in items {
        *counts.entry(*star).or_insert(0) += 1;
    }
    let mut keys: Vec<i32> = counts.keys().cloned().collect();
    keys.sort();
    keys.iter()
        .map(|s| format!("★{}×{}", s, counts[s]))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 组装一段（新增 or 待复查）：按 app 的星级分布 + 时间倒序前 N 条明细。
fn build_section(
    lines: &mut Vec<String>,
    results: &[AppResult],
    pick: impl Fn(&AppResult) -> &Vec<(i32, String, i64)>,
    header: &str,
    list_header: &str,
    max_items: usize,
) {
    lines.push(String::new());
    lines.push(header.to_string());
    for r in results {
        let items = pick(r);
        if items.is_empty() {
            continue;
        }
        lines.push(format!(
            "• {}　{}",
            escape_html(&r.display_name),
            star_counts_str(items)
        ));
    }
    let mut all: Vec<(&str, i32, &str, i64)> = Vec::new();
    for r in results {
        for (star, text, ts) in pick(r) {
            all.push((&r.display_name, *star, text, *ts));
        }
    }
    all.sort_by(|a, b| b.3.cmp(&a.3));
    let top_count = all.len().min(max_items);
    if top_count > 0 {
        lines.push(String::new());
        lines.push(list_header.to_string());
        for (i, (name, star, text, _)) in all.iter().take(max_items).enumerate() {
            lines.push(format!("{} ★{} {}", i + 1, star, escape_html(name)));
            lines.push(format!("   \"{}\"", escape_html(&truncate_chars(text, 40))));
        }
        let rest = all.len().saturating_sub(top_count);
        if rest > 0 {
            lines.push(format!("（其余 {} 条见 app）", rest));
        }
    }
}

fn build_message(
    results: &[AppResult],
    failed: &[String],
    total_new: usize,
    total_updated: usize,
    email: &str,
    time_label: &str,
    max_items: usize,
    is_catchup: bool,
    now_month_day: &str,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    let catchup = if is_catchup { "（错过补发）" } else { "" };
    lines.push(format!(
        "🔔 <b>差评巡检 · {} {}</b>{}",
        now_month_day, time_label, catchup
    ));
    lines.push(format!("账号：{}", escape_html(email)));

    if total_new > 0 {
        build_section(
            &mut lines,
            results,
            |r| &r.new_items,
            &format!("📊 本次新增 <b>{}</b> 条（按配置筛选）", total_new),
            "—— 最新几条 ——",
            max_items,
        );
    }
    if total_updated > 0 {
        build_section(
            &mut lines,
            results,
            |r| &r.updated_items,
            &format!(
                "🔁 回复后又被用户更新 <b>{}</b> 条（回复可能过时，建议去 app 复查）",
                total_updated
            ),
            "—— 待复查 ——",
            max_items,
        );
    }

    if !failed.is_empty() {
        lines.push(String::new());
        lines.push(format!("⚠️ 拉取失败：{}", failed.join("、")));
    }

    let mut text = lines.join("\n");
    if text.chars().count() > 4000 {
        let cut: String = text.chars().take(3990).collect();
        text = format!("{}\n…（内容过长已截断）", cut);
    }
    text
}

// ---- 核心：拉取 + diff + 通知 ----

/// 执行一次巡检。`force_all_times`=true 时（run_now）无视 fired，用当前时刻作 label。
/// 返回给 UI 的简短状态串。
async fn execute_and_notify(
    app: &AppHandle,
    key: &str,
    runtime: &ScheduleRuntime,
    time_label: &str,
    is_catchup: bool,
) -> String {
    let state = app.state::<AuthState>();
    let token = match crate::auth::get_valid_access_token(&state).await {
        Ok(t) => t,
        Err(e) => {
            if e.starts_with("NEED_RELOGIN") {
                let _ = crate::notify::send_telegram_message(
                    "⚠️ 定时拉取失败：登录已失效或缺少权限，请打开 app 重新登录后台账号。".to_string(),
                )
                .await;
                return "登录失效，已发提示".to_string();
            }
            return format!("取 token 失败：{}", e);
        }
    };
    let email = state.active_email().unwrap_or_else(|| "未知账号".to_string());

    let now = Local::now();
    let today = now.date_naive();
    let now_month_day = format!("{:02}-{:02}", now.month(), now.day());
    let now_ms = now.timestamp_millis();

    let mut notified: NotifiedMap = read_json(&notified_path(key));
    let mut updated_map: UpdatedMap = read_json(&updated_path(key));
    let check_updated = runtime.schedule.check_updated;
    let mut results: Vec<AppResult> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut total_new = 0usize;
    let mut total_updated = 0usize;

    for entry in &runtime.apps {
        let pkg = entry.package_name.clone();
        let name = if entry.display_name.is_empty() {
            pkg.clone()
        } else {
            entry.display_name.clone()
        };
        let reviews = match crate::reviews::fetch_reviews(&pkg, Some(5), Some("zh-CN"), &token).await
        {
            Ok(r) => r,
            Err(_) => {
                failed.push(name.clone());
                continue;
            }
        };

        // 落 per-app 快照（与前端 handleBatchFetch/ReviewPage 格式一致，可互读）
        save_snapshot(key, &pkg, &name, &reviews, now_ms);

        let (from_ts, to_ts) = resolve_range(entry, today);
        let matched: Vec<&crate::reviews::Review> = reviews
            .iter()
            .filter(|r| {
                r.user_comment_ts >= from_ts
                    && r.user_comment_ts <= to_ts
                    && entry.stars.contains(&r.star_rating)
                    && matches_reply_state(r, &entry.reply_state)
            })
            .collect();

        let st = notified.entry(pkg.clone()).or_default();
        let first_run = !st.baseline_done;
        // 用 owned 集合，避免 known 借用 st.ids 挡住后面对 st 的写。
        let known: std::collections::HashSet<String> = st.ids.iter().cloned().collect();
        let new_matches: Vec<&crate::reviews::Review> =
            matched.iter().filter(|r| !known.contains(&r.review_id)).cloned().collect();
        let new_items: Vec<(i32, String, i64)> = if first_run {
            Vec::new()
        } else {
            new_matches
                .iter()
                .map(|r| (r.star_rating, r.text.clone(), r.user_comment_ts))
                .collect()
        };
        total_new += new_items.len();

        // 已通知集合只增不减：把本次新命中的 id 并入即可，不再按「是否在这次 API 返回里」
        // 裁剪旧记录——Google 评论接口分页/排序不保证稳定，按存在与否裁剪会把已推送过的
        // 记录冲掉，导致同一条评论在之后某次巡检里被误判成「新增」重新推送（即便它早已被
        // 回复）。增长量可忽略：id 是字符串，长期攒积也就几十 KB。
        for r in &new_matches {
            st.ids.push(r.review_id.clone());
        }
        st.baseline_done = true;

        // 「回复后又被用户更新」：开发者回复过 + 用户在回复之后又改了评论（user_comment_ts >
        // developer_reply_ts）。同窗口内、忽略星级；去重靠 updated_map（同一条只在 user_comment_ts
        // 变大时再次提醒）。不做 baseline —— 已存在的待复查是真实待办，首次就提醒。
        let updated_items: Vec<(i32, String, i64)> = if check_updated {
            let seen = updated_map.entry(pkg.clone()).or_default();
            let mut out = Vec::new();
            for r in &reviews {
                if r.user_comment_ts < from_ts || r.user_comment_ts > to_ts {
                    continue;
                }
                let is_updated = r.developer_reply.is_some()
                    && r.developer_reply_ts.map(|rt| r.user_comment_ts > rt).unwrap_or(false);
                if !is_updated {
                    continue;
                }
                let prev = seen.get(&r.review_id).copied().unwrap_or(0);
                if r.user_comment_ts > prev {
                    out.push((r.star_rating, r.text.clone(), r.user_comment_ts));
                    seen.insert(r.review_id.clone(), r.user_comment_ts);
                }
            }
            out
        } else {
            Vec::new()
        };
        total_updated += updated_items.len();

        results.push(AppResult {
            display_name: name,
            new_items,
            updated_items,
        });
    }

    let _ = write_json_atomic(&notified_path(key), &notified);
    if check_updated {
        let _ = write_json_atomic(&updated_path(key), &updated_map);
    }

    // 通知前端：本账号的评论快照已在后台刷新过 → ReviewPage 重读快照，免得用户还得手动拉取。
    let _ = app.emit("scheduled-fetch-done", serde_json::json!({ "account": key }));

    let cfg = &runtime.schedule;

    // 既无新增差评、也无待复查 → 心跳 or 静默。
    if total_new == 0 && total_updated == 0 {
        if cfg.notify_on_empty {
            let suffix = if is_catchup { "（错过补发）" } else { "" };
            let msg = format!(
                "✅ 今日无新差评（{} {}）{}",
                now_month_day, time_label, suffix
            );
            let _ = crate::notify::send_telegram_message(msg).await;
        }
        let base = if runtime.apps.is_empty() {
            "未启用任何应用".to_string()
        } else {
            "本次无新增差评".to_string()
        };
        if failed.is_empty() {
            return base;
        }
        return format!("{}（{} 个应用拉取失败）", base, failed.len());
    }

    // 有新增 or 有待复查（待复查是可执行待办，即便无新增也发，不受 notify_on_empty 影响）。
    let msg = build_message(
        &results,
        &failed,
        total_new,
        total_updated,
        &email,
        time_label,
        cfg.max_items_in_msg.max(1),
        is_catchup,
        &now_month_day,
    );
    let summary = match (total_new, total_updated) {
        (n, 0) => format!("新增 {} 条", n),
        (0, u) => format!("待复查 {} 条", u),
        (n, u) => format!("新增 {} 条 · 待复查 {} 条", n, u),
    };
    match crate::notify::send_telegram_message(msg).await {
        Ok(()) => format!("已推送：{}", summary),
        Err(e) => format!("{}，但推送失败：{}", summary, e),
    }
}

fn save_snapshot(key: &str, pkg: &str, app_name: &str, reviews: &[crate::reviews::Review], now_ms: i64) {
    let tagged: Vec<serde_json::Value> = reviews
        .iter()
        .filter_map(|r| {
            let mut v = serde_json::to_value(r).ok()?;
            if let Some(obj) = v.as_object_mut() {
                obj.insert("_pkg".to_string(), serde_json::json!(pkg));
                obj.insert("_app".to_string(), serde_json::json!(app_name));
            }
            Some(v)
        })
        .collect();
    let payload = serde_json::json!({
        "version": 1,
        "reviews": tagged,
        "fetchedAt": now_ms,
    });
    let snap_key = format!("{}__{}", key, pkg);
    let _ = crate::reviews::save_reviews_snapshot(snap_key, payload);
}

// ---- 触发判定 + 定时线程 ----

static BUSY: AtomicBool = AtomicBool::new(false);

struct BusyGuard;
impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}
fn try_acquire() -> Option<BusyGuard> {
    if BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        Some(BusyGuard)
    } else {
        None
    }
}

fn hhmm_to_minutes(s: &str) -> Option<i64> {
    let mut parts = s.split(':');
    let h: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    Some(h * 60 + m)
}

/// 一次 tick：读活跃账号 runtime，命中「今天已过且未触发」的时间点则跑一次并标记已触发。
async fn tick(app: &AppHandle) {
    let key = match app.state::<AuthState>().active_key() {
        Some(k) => k,
        None => return,
    };
    let runtime: ScheduleRuntime = read_json(&runtime_path(&key));
    if !runtime.schedule.enabled || runtime.schedule.times.is_empty() {
        return;
    }

    let now = Local::now();
    let today = format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day());
    let now_min = now.hour() as i64 * 60 + now.minute() as i64;

    let mut fired: FiredState = read_json(&fired_path(&key));
    if fired.date != today {
        fired = FiredState {
            date: today.clone(),
            times: Vec::new(),
        };
    }

    let mut due: Vec<String> = runtime
        .schedule
        .times
        .iter()
        .filter(|t| {
            hhmm_to_minutes(t).map(|m| m <= now_min).unwrap_or(false) && !fired.times.contains(t)
        })
        .cloned()
        .collect();
    if due.is_empty() {
        return;
    }
    due.sort();

    let earliest = hhmm_to_minutes(&due[0]).unwrap_or(now_min);
    let lateness = now_min - earliest;
    let is_catchup = due.len() > 1 || lateness >= 2;
    let time_label = due.last().cloned().unwrap_or_default();

    let Some(_guard) = try_acquire() else {
        return; // 上一次还在跑，跳过这一 tick
    };
    execute_and_notify(app, &key, &runtime, &time_label, is_catchup).await;

    // 标记已触发（重新读一遍防覆盖跨天重置）
    let mut fired: FiredState = read_json(&fired_path(&key));
    if fired.date != today {
        fired = FiredState {
            date: today.clone(),
            times: Vec::new(),
        };
    }
    for t in &due {
        if !fired.times.contains(t) {
            fired.times.push(t.clone());
        }
    }
    let _ = write_json_atomic(&fired_path(&key), &fired);
}

/// 立即执行一次真实巡检（供 UI「立即执行一次」测试）：无视时间点/已触发，尊重去重与 baseline。
#[tauri::command]
pub async fn run_schedule_now(app: AppHandle) -> Result<String, String> {
    let key = app
        .state::<AuthState>()
        .active_key()
        .ok_or("未登录".to_string())?;
    let runtime: ScheduleRuntime = read_json(&runtime_path(&key));
    if runtime.apps.is_empty() {
        return Err("未在「Play Console 拉取配置」启用任何应用（或尚未保存定时配置）".to_string());
    }
    let Some(_guard) = try_acquire() else {
        return Err("上一次巡检还在进行中，请稍候".to_string());
    };
    let now = Local::now();
    let label = format!("{:02}:{:02}", now.hour(), now.minute());
    Ok(execute_and_notify(&app, &key, &runtime, &label, false).await)
}

/// 启动后台定时线程（在 lib.rs setup 里调一次）。原生 OS 线程 + block_on，不受 webview 节流影响。
pub fn start_scheduler(app: AppHandle) {
    std::thread::spawn(move || {
        // 启动稍等，让 AuthState/账号加载就绪。
        std::thread::sleep(Duration::from_secs(10));
        loop {
            tauri::async_runtime::block_on(tick(&app));
            std::thread::sleep(Duration::from_secs(30));
        }
    });
}

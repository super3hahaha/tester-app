use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::State;

use crate::auth::AuthState;

#[derive(Deserialize, Debug)]
struct GoogleReviewsResponse {
    reviews: Option<Vec<GoogleReview>>,
    #[serde(rename = "tokenPagination")]
    token_pagination: Option<TokenPagination>,
}

#[derive(Deserialize, Debug)]
struct TokenPagination {
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize, Debug)]
struct GoogleReview {
    #[serde(rename = "reviewId")]
    review_id: String,
    #[serde(rename = "authorName")]
    author_name: Option<String>,
    comments: Option<Vec<GoogleComment>>,
}

#[derive(Deserialize, Debug)]
struct GoogleComment {
    #[serde(rename = "userComment")]
    user_comment: Option<GoogleUserComment>,
    #[serde(rename = "developerComment")]
    developer_comment: Option<GoogleDeveloperComment>,
}

#[derive(Deserialize, Debug)]
struct GoogleUserComment {
    text: Option<String>,
    #[serde(rename = "lastModified")]
    last_modified: Option<GoogleTimestamp>,
    #[serde(rename = "starRating")]
    star_rating: Option<i32>,
    #[serde(rename = "reviewerLanguage")]
    reviewer_language: Option<String>,
    device: Option<String>,
    #[serde(rename = "androidOsVersion")]
    android_os_version: Option<i32>,
    #[serde(rename = "appVersionCode")]
    app_version_code: Option<i32>,
    #[serde(rename = "appVersionName")]
    app_version_name: Option<String>,
    #[serde(rename = "thumbsUpCount")]
    thumbs_up_count: Option<i32>,
    #[serde(rename = "thumbsDownCount")]
    thumbs_down_count: Option<i32>,
    #[serde(rename = "originalText")]
    original_text: Option<String>,
}

#[derive(Deserialize, Debug)]
struct GoogleDeveloperComment {
    text: Option<String>,
    #[serde(rename = "lastModified")]
    last_modified: Option<GoogleTimestamp>,
}

// Timestamps from this API come back as { seconds: "1717286400", nanos: 0 }.
// `seconds` is a string because it's an int64 in protobuf JSON encoding.
#[derive(Deserialize, Debug)]
struct GoogleTimestamp {
    seconds: Option<String>,
    #[allow(dead_code)]
    nanos: Option<i64>,
}

#[derive(Serialize, Clone)]
pub struct Review {
    pub review_id: String,
    pub author_name: String,
    pub text: String,
    pub original_text: Option<String>,
    pub star_rating: i32,
    pub reviewer_language: Option<String>,
    pub device: Option<String>,
    pub android_os_version: Option<i32>,
    pub app_version_name: Option<String>,
    pub app_version_code: Option<i32>,
    pub thumbs_up_count: i32,
    pub thumbs_down_count: i32,
    pub user_comment_ts: i64,
    pub developer_reply: Option<String>,
    pub developer_reply_ts: Option<i64>,
}

/// Fetch reviews from the Google Play Developer API.
///
/// API caveats (Google-side, not us):
/// - Only returns reviews from the **last 7 days**, regardless of any date filter
/// - No server-side filtering by star / reply state — caller filters client-side
/// - `package_name` is the actual app id (e.g. `com.example.app`), NOT the numeric
///   id you see in the Play Console URL.
///
/// `max_pages` caps pagination (each page = up to 100 reviews). Default 5 = 500
/// reviews max, which is plenty for a 7-day window of all but the largest apps.
#[tauri::command]
pub async fn list_play_reviews(
    package_name: String,
    max_pages: Option<u32>,
    translation_language: Option<String>,
    state: State<'_, AuthState>,
) -> Result<Vec<Review>, String> {
    let token = crate::auth::get_valid_access_token(&state).await?;
    let max_pages = max_pages.unwrap_or(5).max(1);

    let client = reqwest::Client::new();
    let mut all_reviews: Vec<Review> = Vec::new();
    let mut next_token: Option<String> = None;

    for _ in 0..max_pages {
        let mut url = format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/reviews?maxResults=100",
            urlencoding::encode(&package_name)
        );
        if let Some(t) = &next_token {
            url.push_str(&format!("&token={}", urlencoding::encode(t)));
        }
        if let Some(lang) = &translation_language {
            if !lang.is_empty() {
                url.push_str(&format!("&translationLanguage={}", urlencoding::encode(lang)));
            }
        }

        let resp = client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("Reviews request failed: {}", e))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("Read reviews body failed: {}", e))?;

        if !status.is_success() {
            // 401 = token expired (auth module would have caught this on refresh,
            //   but token could be wrong-scope from a pre-androidpublisher login).
            // 403 = ACCESS_TOKEN_SCOPE_INSUFFICIENT or app not linked / no permission.
            // Surface a clean hint when it's a scope problem.
            let needs_relogin = body.contains("ACCESS_TOKEN_SCOPE_INSUFFICIENT")
                || body.contains("insufficient authentication scopes")
                || status.as_u16() == 401;
            if needs_relogin {
                return Err(format!(
                    "NEED_RELOGIN_SCOPE: 当前登录态没有 androidpublisher 权限，请退出登录后重新登录。原始错误：{}",
                    body
                ));
            }
            return Err(format!("Reviews API {}: {}", status, body));
        }

        let parsed: GoogleReviewsResponse = serde_json::from_str(&body)
            .map_err(|e| format!("Parse reviews failed: {}: body={}", e, body))?;

        if let Some(reviews) = parsed.reviews {
            for r in reviews {
                if let Some(review) = flatten_review(r) {
                    all_reviews.push(review);
                }
            }
        }

        next_token = parsed.token_pagination.and_then(|p| p.next_page_token);
        if next_token.is_none() {
            break;
        }
    }

    Ok(all_reviews)
}

fn flatten_review(r: GoogleReview) -> Option<Review> {
    // `comments` may contain history entries; take the latest userComment and
    // any developerComment present.
    let mut user: Option<GoogleUserComment> = None;
    let mut developer: Option<GoogleDeveloperComment> = None;
    if let Some(comments) = r.comments {
        for c in comments {
            if let Some(u) = c.user_comment {
                user = Some(u);
            }
            if let Some(d) = c.developer_comment {
                developer = Some(d);
            }
        }
    }
    let u = user?;

    let user_ts = ts_seconds(&u.last_modified);
    let (dev_text, dev_ts) = match developer {
        Some(d) => (d.text, Some(ts_seconds(&d.last_modified))),
        None => (None, None),
    };

    Some(Review {
        review_id: r.review_id,
        author_name: r.author_name.unwrap_or_default(),
        text: u.text.unwrap_or_default(),
        original_text: u.original_text,
        star_rating: u.star_rating.unwrap_or(0),
        reviewer_language: u.reviewer_language,
        device: u.device,
        android_os_version: u.android_os_version,
        app_version_name: u.app_version_name,
        app_version_code: u.app_version_code,
        thumbs_up_count: u.thumbs_up_count.unwrap_or(0),
        thumbs_down_count: u.thumbs_down_count.unwrap_or(0),
        user_comment_ts: user_ts,
        developer_reply: dev_text,
        developer_reply_ts: dev_ts,
    })
}

fn ts_seconds(t: &Option<GoogleTimestamp>) -> i64 {
    t.as_ref()
        .and_then(|t| t.seconds.as_ref())
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

// ---- reviews.reply ----
//
// POSTs a developer reply to a single review.
// Endpoint: applications/{packageName}/reviews/{reviewId}:reply
// Body: { "replyText": "..." }
// Quota: 2000 POST/day per app — easily enough for batch flows.

#[derive(serde::Serialize)]
struct ReplyBody<'a> {
    #[serde(rename = "replyText")]
    reply_text: &'a str,
}

#[tauri::command]
pub async fn reply_to_review(
    package_name: String,
    review_id: String,
    reply_text: String,
    state: State<'_, AuthState>,
) -> Result<(), String> {
    let token = crate::auth::get_valid_access_token(&state).await?;
    let url = format!(
        "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/reviews/{}:reply",
        urlencoding::encode(&package_name),
        urlencoding::encode(&review_id),
    );

    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&token)
        .json(&ReplyBody { reply_text: &reply_text })
        .send()
        .await
        .map_err(|e| format!("Reply request failed: {}", e))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Read reply body failed: {}", e))?;

    if !status.is_success() {
        let needs_relogin = body.contains("ACCESS_TOKEN_SCOPE_INSUFFICIENT")
            || body.contains("insufficient authentication scopes")
            || status.as_u16() == 401;
        if needs_relogin {
            return Err(format!(
                "NEED_RELOGIN_SCOPE: 当前登录态没有 androidpublisher 权限，请退出登录后重新登录。原始错误：{}",
                body
            ));
        }
        return Err(format!("Reply API {}: {}", status, body));
    }

    Ok(())
}

// ---- apps:search via the Reporting API ----
//
// The publisher API has no "list my apps" endpoint, but the Reporting API does.
// Requires the `playdeveloperreporting` scope (separate from `androidpublisher`).

#[derive(Deserialize, Debug)]
struct AppsSearchResponse {
    apps: Option<Vec<GoogleApp>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize, Debug)]
struct GoogleApp {
    #[serde(rename = "packageName")]
    package_name: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct PlayApp {
    pub package_name: String,
    pub display_name: String,
}

#[tauri::command]
pub async fn list_play_apps(state: State<'_, AuthState>) -> Result<Vec<PlayApp>, String> {
    let token = crate::auth::get_valid_access_token(&state).await?;
    let client = reqwest::Client::new();

    let mut out: Vec<PlayApp> = Vec::new();
    let mut next: Option<String> = None;
    // Hard cap on pages to avoid runaway loops; 1000/page * 10 pages = 10k apps.
    for _ in 0..10 {
        let mut url =
            "https://playdeveloperreporting.googleapis.com/v1beta1/apps:search?pageSize=1000"
                .to_string();
        if let Some(t) = &next {
            url.push_str(&format!("&pageToken={}", urlencoding::encode(t)));
        }

        let resp = client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("apps.search request failed: {}", e))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("Read apps.search body failed: {}", e))?;

        if !status.is_success() {
            let needs_relogin = body.contains("ACCESS_TOKEN_SCOPE_INSUFFICIENT")
                || body.contains("insufficient authentication scopes")
                || status.as_u16() == 401;
            if needs_relogin {
                return Err(format!(
                    "NEED_RELOGIN_SCOPE: 当前登录态没有 playdeveloperreporting 权限，请退出登录后重新登录。原始错误：{}",
                    body
                ));
            }
            return Err(format!("apps.search {}: {}", status, body));
        }

        let parsed: AppsSearchResponse = serde_json::from_str(&body)
            .map_err(|e| format!("Parse apps.search failed: {}: body={}", e, body))?;

        if let Some(apps) = parsed.apps {
            for a in apps {
                if let Some(pkg) = a.package_name {
                    out.push(PlayApp {
                        display_name: a.display_name.unwrap_or_else(|| pkg.clone()),
                        package_name: pkg,
                    });
                }
            }
        }

        next = parsed.next_page_token;
        if next.is_none() || next.as_deref() == Some("") {
            break;
        }
    }

    // Alphabetical for stable UI ordering.
    out.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
    Ok(out)
}

// ---- 评论快照持久化（reviews-cache）----
//
// Play reviews API 只返回最近约 7 天，且每次进页面都要重拉很烦。每个 app 的
// 「全量拉取列表」按包名各存一份 `~/.tester-app/reviews-cache/{key}.json`，单 app
// 拉取与批量拉取写的是同一种文件、可互换；批量视图由前端读多份按需拼装。
//
// 后端只做透明读写：payload 是 serde_json::Value，原样落盘 / 读回，不认识 Review
// 结构 —— 这样以后前端 Review 字段怎么改都不用动 Rust。

fn reviews_cache_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join(".tester-app")
        .join("reviews-cache")
}

/// 把 key（包名或 `__batch__`）规整成安全文件名：非 [A-Za-z0-9._-] 一律转 `_`。
fn snapshot_path(key: &str) -> PathBuf {
    let safe: String = key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    reviews_cache_dir().join(format!("{}.json", safe))
}

#[tauri::command]
pub fn save_reviews_snapshot(key: String, data: serde_json::Value) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("snapshot key 不能为空".to_string());
    }
    let path = snapshot_path(&key);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string(&data).map_err(|e| e.to_string())?;
    // 先写临时文件再 rename，避免写一半被读到损坏的 JSON。
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn load_reviews_snapshot(key: String) -> Result<Option<serde_json::Value>, String> {
    let path = snapshot_path(&key);
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).map(Some).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

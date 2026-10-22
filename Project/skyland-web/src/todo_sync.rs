//! 待办云同步 — 第三方笔记/日记软件（Memos 优先）→ SKYLAND 待办。
//!
//! 数据流：远端 Memos → 拉取最近条目 → 转换 skyland-todo/v1 中间格式 →
//! 复用 todo_api::import_core（内容寻址 id，天然幂等）→ 合并进用户待办清单。
//!
//! 模式分工（用户约定）：
//! - standalone：本服务进程引擎轮询 + 打开待办页时前端触发 /api/todo/sync/pull
//! - server：服务端进程引擎为每个配置了轮询的用户执行（数据在服务端合并）
//! - client：无本地服务——配置与拉取都落在远端服务端（同一路由/引擎代码）
//!
//! 凭据安全：sync.toml 存于用户数据目录（与 llm.toml 同级信任域）。

use axum::{extract::State, Extension, Json};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

use crate::api::{user_data_dir_for, AppContext};
use crate::auth::CurrentUser;

const SYNC_INTERVAL_SECS: u64 = 15 * 60; // 用户指定：每 15 分钟
const SYNC_MAX_MEMOS: usize = 300;       // 单次拉取上限（防首次同步拖死引擎）

// ─── 配置 ───

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct SyncConfig {
    #[serde(default)]
    pub provider: String,      // "memos"（其余 provider 前端占位，适配后扩展）
    #[serde(default)]
    pub url: String,           // 如 http://192.168.1.10:5230
    #[serde(default)]
    pub auth_mode: String,     // "token" | "password"
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub poll_enabled: bool,    // 每 15 分钟轮询（默认关）
    #[serde(default)]
    pub last_sync: u64,
}

pub fn config_path_for(ctx: &AppContext, user: Option<&CurrentUser>) -> std::path::PathBuf {
    user_data_dir_for(ctx, user).join("sync.toml")
}

pub fn load_config_for(ctx: &AppContext, user: Option<&CurrentUser>) -> SyncConfig {
    read_config(&config_path_for(ctx, user))
}

pub fn read_config(path: &std::path::Path) -> SyncConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(path: &std::path::Path, cfg: &SyncConfig) -> Result<(), String> {
    let body = toml::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

// ─── Memos 客户端 ───

fn base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn auth_headers(cfg: &SyncConfig, cookie: &Option<String>) -> Vec<(String, String)> {
    let mut h = vec![("Content-Type".to_string(), "application/json".to_string())];
    if let Some(c) = cookie {
        h.push(("Cookie".to_string(), c.clone()));
    } else if cfg.auth_mode == "token" && !cfg.token.is_empty() {
        h.push(("Authorization".to_string(), format!("Bearer {}", cfg.token)));
    }
    h
}

/// 密码模式：调 memos sign-in 换会话 Cookie（reqwest 默认不带 cookie store，手动提取）
async fn sign_in(cfg: &SyncConfig) -> Result<Option<String>, String> {
    if cfg.auth_mode != "password" { return Ok(None); }
    if cfg.username.is_empty() || cfg.password.is_empty() {
        return Err("密码模式需要用户名与密码".into());
    }
    let url = format!("{}/api/v1/auth/sign-in", base_url(&cfg.url));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({ "username": cfg.username, "password": cfg.password }))
        .timeout(Duration::from_secs(15))
        .send().await.map_err(|e| format!("登录请求失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("登录失败：HTTP {}", resp.status()));
    }
    let cookie = resp.headers().get(reqwest::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| raw.split(';').next().map(String::from));
    Ok(cookie)
}

/// 拉取 memos 列表（翻页直到没有 next 或达到上限）
async fn fetch_memos(cfg: &SyncConfig, cookie: &Option<String>) -> Result<Vec<serde_json::Value>, String> {
    let client = reqwest::Client::new();
    let base = base_url(&cfg.url);
    let mut all: Vec<serde_json::Value> = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut url = format!("{}/api/v1/memos?pageSize=100", base);
        if let Some(t) = &page_token {
            url.push_str(&format!("&pageToken={}", t));
        }
        let req = client.get(&url).timeout(Duration::from_secs(20));
        let req = auth_headers(cfg, cookie).iter().fold(req, |r, (k, v)| r.header(k, v));
        let resp = req.send().await.map_err(|e| format!("拉取失败: {}", e))?;
        let status = resp.status();
        if !status.is_success() {
            let hint = if status.as_u16() == 401 { "（凭据无效或已过期）" } else { "" };
            return Err(format!("HTTP {} {}", status.as_u16(), hint));
        }
        let d: serde_json::Value = resp.json().await.map_err(|e| format!("响应解析失败: {}", e))?;
        let items = d.get("memos").and_then(|m| m.as_array()).cloned().unwrap_or_default();
        let got = items.len();
        all.extend(items);
        page_token = d.get("nextPageToken").and_then(|t| t.as_str()).map(String::from)
            .filter(|t| !t.is_empty() && got > 0);
        if all.len() >= SYNC_MAX_MEMOS { break; }
        if page_token.is_none() { break; }
    }
    all.truncate(SYNC_MAX_MEMOS);
    Ok(all)
}

fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

const IMG_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];

/// 下载 memo 内的相对路径图片到用户图片目录，并把 content 中的路径改写为本地资产 URL。
/// 外链（http/https）保持原样；下载失败保留原路径（不阻塞同步）。
async fn localize_todo_images(
    ctx: &AppContext,
    user: Option<&CurrentUser>,
    cfg: &SyncConfig,
    cookie: &Option<String>,
    items: &mut [serde_json::Value],
) {
    let re = match regex::Regex::new(r"!\[([^\]]*)\]\(([^)\s]+)\)") {
        Ok(r) => r,
        Err(_) => return,
    };
    let base = base_url(&cfg.url);
    let img_dir = crate::image_api::img_dir_for(ctx, user);
    std::fs::create_dir_all(&img_dir).ok();
    let client = reqwest::Client::new();
    let mut cache: std::collections::HashMap<String, String> = Default::default();

    for item in items.iter_mut() {
        let Some(content) = item.get("content").and_then(|v| v.as_str()).map(String::from) else { continue };
        let mut new_content = content.clone();
        let mut changed = false;
        for cap in re.captures_iter(&content) {
            let whole = cap.get(0).unwrap().as_str();
            let url = cap.get(2).unwrap().as_str().to_string();
            if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("/api/") {
                continue; // 外链 / 已本地化的保持原样
            }
            let local_url = if let Some(hit) = cache.get(&url) {
                hit.clone()
            } else {
                let ext = url.rsplit('.').next()
                    .map(|e| e.to_lowercase())
                    .map(|e| e.chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>())
                    .map(|e| if IMG_EXTS.contains(&e.as_str()) { e } else { "png".to_string() })
                    .unwrap_or_else(|| "png".to_string());
                let name = format!("todo_{:016x}.{}", fnv1a64(&url), ext);
                let dl_url = format!("{}{}", base, if url.starts_with('/') { url.clone() } else { format!("/{}", url) });
                let req = client.get(&dl_url).timeout(Duration::from_secs(30));
                let req = auth_headers(cfg, cookie).iter().fold(req, |r, (k, v)| r.header(k, v));
                match req.send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.bytes().await {
                            Ok(bytes) if !bytes.is_empty() && bytes.len() < 20 * 1024 * 1024 => {
                                let path = img_dir.join(&name);
                                if std::fs::write(&path, &bytes).is_ok() {
                                    let u = format!("/api/image/file/{}", name);
                                    cache.insert(url.clone(), u.clone());
                                    u
                                } else { continue; }
                            }
                            _ => continue,
                        }
                    }
                    _ => continue,
                }
            };
            new_content = new_content.replace(whole, &format!("![图片]({})", local_url));
            changed = true;
        }
        if changed {
            item["content"] = serde_json::Value::String(new_content);
        }
    }
}

/// memo → skyland-todo/v1 条目。
/// 标题取首行；首个 #tag 提取为分类（对齐速记语法习惯）；
/// 日期 = createTime 按服务器本地时区落档（与 today_str 同一口径）。
fn memo_to_item(memo: &serde_json::Value) -> Option<serde_json::Value> {
    let content_full = memo.get("content")?.as_str()?.trim().to_string();
    if content_full.is_empty() { return None; }
    let created_ts = chrono::DateTime::parse_from_rfc3339(
        memo.get("createTime").and_then(|v| v.as_str()).unwrap_or(""))
        .map(|dt| dt.timestamp().max(0) as u64)
        .unwrap_or_else(|_| crate::todo_api::now_ts());
    let local_date = chrono::DateTime::<chrono::Utc>::from_timestamp(created_ts as i64, 0)
        .map(|utc| utc.with_timezone(&chrono::Local).date_naive().to_string())
        .unwrap_or_else(|| crate::todo_api::today_str());

    let mut lines = content_full.lines();
    let first = lines.next().unwrap_or("").trim().to_string();
    let rest = lines.collect::<Vec<_>>().join("\n");
    // 首行提取 #tag 作分类
    let mut category: Option<String> = None;
    let mut title = first.clone();
    if let Some(idx) = first.find('#') {
        let tag: String = first[idx + 1..].chars().take_while(|c| !c.is_whitespace()).collect();
        if !tag.is_empty() {
            let tag_len = tag.len();
            category = Some(tag);
            title = format!("{}{}", first[..idx].trim(), first[idx + 1 + tag_len..].trim()).trim().to_string();
        }
    }
    let title = if title.is_empty() { "无标题备忘".to_string() } else { title };
    Some(serde_json::json!({
        "date": local_date,
        "title": title,
        "content": rest,
        "category": category,
        "priority": 1,
        "created": created_ts,
    }))
}

// ─── 拉取核心（HTTP 端点与引擎共用）───

pub struct PullReport {
    pub imported: u32,
    pub skipped: u32,
    #[allow(dead_code)]
    pub rejected: Vec<String>,
    pub total: usize,
}

pub async fn sync_pull_for(ctx: &AppContext, user: Option<&CurrentUser>) -> Result<PullReport, String> {
    let cfg = load_config_for(ctx, user);
    if cfg.provider.is_empty() || cfg.url.trim().is_empty() {
        return Err("尚未配置同步源".into());
    }
    if cfg.provider != "memos" {
        return Err(format!("暂不支持的数据源: {}（适配中）", cfg.provider));
    }
    let cookie = sign_in(&cfg).await?;
    let memos = fetch_memos(&cfg, &cookie).await?;
    let total = memos.len();
    let mut items: Vec<serde_json::Value> = memos.iter().filter_map(memo_to_item).collect();
    // 图片本地化：memo 内相对路径图片下载到用户图片目录并持久保存，
    // 正文改写为本地资产 URL（/api/image/file/:name，按用户隔离）
    localize_todo_images(ctx, user, &cfg, &cookie, &mut items).await;
    let (imported, skipped, rejected) = {
        let dir = crate::todo_api::todo_dir_for_user(ctx, user);
        crate::todo_api::import_core(&dir, &items)
    };
    let mut cfg2 = cfg.clone();
    cfg2.last_sync = crate::todo_api::now_ts();
    let _ = save_config(&config_path_for(ctx, user), &cfg2);
    Ok(PullReport { imported, skipped, rejected, total })
}

// ─── HTTP 端点 ───

/// GET /api/todo/sync/config — 读取同步配置（凭据脱敏）
pub async fn sync_config_get(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let cfg = load_config_for(&ctx, user.as_ref().map(|u| &u.0));
    Json(serde_json::json!({
        "status": "ok",
        "provider": cfg.provider, "url": cfg.url, "auth_mode": cfg.auth_mode,
        "username": cfg.username,
        "has_token": !cfg.token.is_empty(),
        "has_password": !cfg.password.is_empty(),
        "poll_enabled": cfg.poll_enabled,
        "last_sync": cfg.last_sync,
    }))
}

#[derive(Deserialize)]
pub struct SyncConfigSave {
    pub provider: String,
    pub url: String,
    pub auth_mode: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub poll_enabled: bool,
}

/// POST /api/todo/sync/config — 保存同步配置（留空 token/password = 保留原值）
pub async fn sync_config_save(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<SyncConfigSave>,
) -> Json<serde_json::Value> {
    let user_ref = user.as_ref().map(|u| &u.0);
    let path = config_path_for(&ctx, user_ref);
    let mut cfg = read_config(&path);
    // provider 白名单：memos 已适配，其余为占位（适配后扩展）
    if req.provider != "memos" {
        return Json(serde_json::json!({ "status": "error", "message": "该数据源适配中，暂仅支持 Memos" }));
    }
    if req.auth_mode != "token" && req.auth_mode != "password" {
        return Json(serde_json::json!({ "status": "error", "message": "认证方式必须是 token 或 password" }));
    }
    cfg.provider = req.provider;
    cfg.url = req.url.trim().trim_end_matches('/').to_string();
    cfg.auth_mode = req.auth_mode;
    cfg.username = req.username.trim().to_string();
    if !req.token.is_empty() { cfg.token = req.token; }
    if !req.password.is_empty() { cfg.password = req.password; }
    if cfg.auth_mode == "token" { cfg.password.clear(); } else { cfg.token.clear(); }
    cfg.poll_enabled = req.poll_enabled;
    match save_config(&path, &cfg) {
        Ok(()) => Json(serde_json::json!({ "status": "ok", "message": "同步配置已保存" })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("保存失败: {}", e) })),
    }
}

/// POST /api/todo/sync/test — 测试连接（验证 URL 与凭据，列出备忘数）
pub async fn sync_test(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<SyncConfigSave>,
) -> Json<serde_json::Value> {
    let saved = load_config_for(&ctx, user.as_ref().map(|u| &u.0));
    let cfg = SyncConfig {
        provider: req.provider.clone(),
        url: req.url.trim().trim_end_matches('/').to_string(),
        auth_mode: req.auth_mode.clone(),
        username: req.username.trim().to_string(),
        token: if req.token.is_empty() { saved.token } else { req.token },
        password: if req.password.is_empty() { saved.password } else { req.password },
        poll_enabled: false,
        last_sync: 0,
    };
    if cfg.provider != "memos" {
        return Json(serde_json::json!({ "status": "error", "message": "该数据源适配中，暂仅支持 Memos" }));
    }
    let cookie = match sign_in(&cfg).await {
        Ok(c) => c,
        Err(e) => return Json(serde_json::json!({ "status": "error", "message": e })),
    };
    match fetch_memos(&cfg, &cookie).await {
        Ok(items) => Json(serde_json::json!({ "status": "ok", "message": format!("连接成功，发现 {} 条备忘", items.len()) })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": e })),
    }
}

/// POST /api/todo/sync/pull — 立即拉取合并（打开待办页 / 手动同步按钮触发）
pub async fn sync_pull_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let user_ref = user.as_ref().map(|u| &u.0);
    // client 模式：请求打到远端服务端执行 —— 与 server 模式同一代码路径
    match sync_pull_for(&ctx, user_ref).await {
        Ok(rep) => Json(serde_json::json!({
            "status": "ok", "imported": rep.imported, "skipped": rep.skipped,
            "rejected": rep.rejected, "total": rep.total,
        })),
        // 未配置同步源 = 无操作成功（打开页面自动拉取不报错）
        Err(e) if e.contains("尚未配置同步源") => {
            Json(serde_json::json!({ "status": "ok", "imported": 0, "skipped": 0, "total": 0, "noop": true }))
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": e })),
    }
}

// ─── 15 分钟轮询引擎（standalone + server；client 由远端引擎承担）───

static ENGINE_STARTED: OnceLock<()> = OnceLock::new();

pub fn start_engine(ctx: AppContext) {
    if ENGINE_STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(move || {
        // 约定（P0 教训）：服务线程内自建 Runtime，不依赖调用方 tokio 上下文
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("sync engine runtime");
        loop {
            rt.block_on(tick(&ctx));
            std::thread::sleep(Duration::from_secs(SYNC_INTERVAL_SECS));
        }
    });
}

/// 扫描所有配置了轮询的用户（server 模式逐用户；standalone 单配置）
fn poll_targets(ctx: &AppContext) -> Vec<(Option<CurrentUser>,)> {
    let mut out = Vec::new();
    if ctx.auth.is_some() {
        let users_root = crate::datadir::AppDataLayout::from_context(ctx).root().join("users");
        if let Ok(entries) = std::fs::read_dir(users_root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.path().is_dir() && !name.starts_with('.') {
                    out.push((Some(CurrentUser { username: name, role: "user".to_string() }),));
                }
            }
        }
    } else {
        out.push((None,));
    }
    out
}

async fn tick(ctx: &AppContext) {
    for (user,) in poll_targets(ctx) {
        let cfg = load_config_for(ctx, user.as_ref());
        if !cfg.poll_enabled || cfg.provider != "memos" || cfg.url.trim().is_empty() {
            continue;
        }
        match sync_pull_for(ctx, user.as_ref()).await {
            Ok(rep) if rep.imported > 0 => {
                let (_ok, via) = skyland_lib::native_notify::send(
                    "SKYLAND 同步",
                    &format!("从 {} 拉取 {} 条新待办（远端共 {} 条）", cfg.provider, rep.imported, rep.total),
                );
                eprintln!("[待办同步][{}] imported={}", via, rep.imported);
            }
            Ok(_) => {}
            Err(e) => eprintln!("[待办同步][失败] {}", e),
        }
    }
}

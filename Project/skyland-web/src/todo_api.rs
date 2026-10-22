//! 待办事项 API 模块
//! 数据存储在 {work_dir}/ToDoList/ 目录下，每日一个 JSON 文件
//! v2: 修复时区(chrono) + 日期过期 + 优先级 + 分类 + 截止日期 + 统计 + 搜索
//! v3: 可见性(public/private) + server 模式按用户隔离 + 原子写 + 日期参数校验

use axum::{Extension, Json, extract::{State, Path as AxumPath, Query}};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use chrono::{Local, NaiveDate, Duration as ChronoDuration, Timelike};

use crate::api::AppContext;
use crate::auth::CurrentUser;

// ─── 数据模型 ───

const VISIBILITY_PUBLIC: &str = "public";

#[derive(Serialize, Deserialize, Clone)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub content: String,
    pub done: bool,
    pub expired: bool,
    pub created: u64,
    pub notify_time: Option<String>,
    pub notify_enabled: bool,
    #[serde(default)]
    pub notify_times: Vec<String>,
    #[serde(default)]
    pub advance_minutes: Vec<u32>,
    #[serde(default)]
    pub repeat_enabled: bool,
    #[serde(default)]
    pub repeat_interval: u32,
    #[serde(default)]
    pub last_notified: Option<u64>,
    // ── v2 新增字段 ──
    #[serde(default = "default_priority")]
    pub priority: u8,               // 0=低, 1=中, 2=高
    #[serde(default)]
    pub category: Option<String>,    // 分类标签
    #[serde(default)]
    pub due_date: Option<String>,    // 截止日期 "YYYY-MM-DD"
    #[serde(default)]
    pub due_time: Option<String>,    // 截止时间 "HH:MM"
    #[serde(default)]
    pub sort_order: u32,             // 手动排序权重
    // ── v3 新增字段 ──
    #[serde(default = "default_visibility")]
    pub visibility: String,          // "private"（默认）| "public"（进发现页共享流）
}

fn default_priority() -> u8 { 1 }
fn default_visibility() -> String { "private".to_string() }

/// 仅接受 public，其余一律回落 private（防脏数据流入共享流）
fn normalize_visibility(v: &Option<String>) -> String {
    match v.as_deref() {
        Some(VISIBILITY_PUBLIC) => VISIBILITY_PUBLIC.to_string(),
        Some(other) if !other.is_empty() && other != "private" => {
            eprintln!("⚠ todo visibility 非法值 '{}'，回落 private", other);
            default_visibility()
        }
        _ => default_visibility(),
    }
}

#[derive(Serialize, Deserialize)]
pub struct TodoList {
    pub date: String,
    pub items: Vec<TodoItem>,
    pub created: u64,
}

#[derive(Deserialize)]
pub struct SaveTodoItemRequest {
    pub title: String,
    pub content: String,
    pub notify_time: Option<String>,
    pub notify_enabled: Option<bool>,
    #[serde(default)]
    pub notify_times: Option<Vec<String>>,
    #[serde(default)]
    pub advance_minutes: Option<Vec<u32>>,
    #[serde(default)]
    pub repeat_enabled: Option<bool>,
    #[serde(default)]
    pub repeat_interval: Option<u32>,
    pub priority: Option<u8>,
    pub category: Option<String>,
    pub due_date: Option<String>,
    pub due_time: Option<String>,
    #[serde(default)]
    pub visibility: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateTodoItemRequest {
    pub title: Option<String>,
    pub content: Option<String>,
    pub done: Option<bool>,
    pub notify_time: Option<String>,
    pub notify_enabled: Option<bool>,
    #[serde(default)]
    pub notify_times: Option<Vec<String>>,
    #[serde(default)]
    pub advance_minutes: Option<Vec<u32>>,
    #[serde(default)]
    pub repeat_enabled: Option<bool>,
    #[serde(default)]
    pub repeat_interval: Option<u32>,
    #[serde(default)]
    pub last_notified: Option<u64>,
    #[serde(default)]
    pub priority: Option<u8>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub due_date: Option<String>,
    #[serde(default)]
    pub due_time: Option<String>,
    #[serde(default)]
    pub sort_order: Option<u32>,
    #[serde(default)]
    pub visibility: Option<String>,
    /// 跨日期迁移（编辑弹窗"所属日期"/跨天提醒预设）：把事项挪到该日期的清单。
    /// 为空或等于当前日期 = 不迁移。
    #[serde(default)]
    pub move_to: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
}

#[derive(Deserialize, Default)]
pub struct TimelineQuery {
    pub tag: Option<String>,
    pub before: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Deserialize, Default)]
pub struct ExploreQuery {
    pub tag: Option<String>,
    pub before: Option<String>,
    pub limit: Option<usize>,
}

// ─── 工具函数 ───

fn todo_dir(ctx: &AppContext) -> PathBuf {
    let dir = ctx.todo_dir.read().unwrap_or_else(|e| e.into_inner());
    let full = dir.clone();
    std::fs::create_dir_all(&full).ok();
    full
}

/// 数据目录解析：server 模式按认证用户进入 `users/<username>/todo`；
/// standalone / 无身份兜底使用启动时绑定的 todo_dir。
/// 用户名由认证模块限制为 [A-Za-z0-9_-]，无路径注入面。
pub(crate) fn todo_dir_for_user(ctx: &AppContext, user: Option<&CurrentUser>) -> PathBuf {
    if ctx.auth.is_none() {
        return todo_dir(ctx);
    }
    let Some(user) = user else { return todo_dir(ctx) };
    let dir = crate::api::user_data_dir_for(ctx, Some(user)).join("todo");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// server 模式升级接管：把根目录遗留的日期 JSON 迁入 admin 个人目录（一次性）。
/// 逐文件迁移 + 目标同名跳过：并发双跑不重复搬、admin 已有个人数据时不覆盖，
/// 也修复旧实现"个人目录非空 → 根目录遗留永不接管"的顺序依赖（P2-7）。
#[allow(dead_code)]
fn claim_legacy_files(base: &Path, target: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(base) else { return false };
    let date_files: Vec<PathBuf> = entries.flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().map(|e| e == "json").unwrap_or(false)
                && p.file_name().and_then(|n| n.to_str())
                    .map(|n| valid_date(n.trim_end_matches(".json")).is_some())
                    .unwrap_or(false)
        })
        .collect();
    if date_files.is_empty() { return false; }
    let mut moved = false;
    for f in date_files {
        if let Some(name) = f.file_name() {
            let dest = target.join(&name);
            if dest.exists() { continue; } // 并发接管/个人数据已有同名 — 不覆盖，幂等
            if std::fs::rename(&f, &dest).is_ok() { moved = true; }
        }
    }
    moved
}

pub(crate) fn today_str() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub(crate) fn now_ts() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 所有 :date 路径参数的唯一入口校验（防路径穿越 + 防垃圾日期落盘）
pub(crate) fn valid_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// 非法日期参数的统一 400 响应
fn bad_date_response() -> Response {
    (axum::http::StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"status":"error","message":"非法日期"}))).into_response()
}

/// 目录下全部合法日期文件（去 .json 后缀，倒序；跳过 tmp 与非法文件名）
pub(crate) fn list_dates(dir: &Path) -> Vec<String> {
    let mut dates: Vec<String> = std::fs::read_dir(dir).map(|entries| {
        entries.flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with(".json") && !name.contains(".tmp") { Some(name) } else { None }
            })
            .filter_map(|name| {
                let d = name.trim_end_matches(".json").to_string();
                if valid_date(&d).is_some() { Some(d) } else { None }
            })
            .collect()
    }).unwrap_or_default();
    dates.sort();
    dates.reverse();
    dates
}

fn clamp_limit(limit: Option<usize>, default: usize) -> usize {
    limit.unwrap_or(default).clamp(1, 31)
}

fn item_matches_tag(item: &TodoItem, tag: &Option<String>) -> bool {
    match tag {
        Some(t) if t != "all" => item.category.as_deref() == Some(t.as_str()),
        _ => true,
    }
}

// ─── v3: 时间线 / 发现页 ───

/// 扫描行：social=true 只收其他用户的 visibility==public 条目（含作者）；
/// social=false 收目录自身的全部条目（作者为空）
fn scan_rows(base: &Path, social: bool, exclude_user: Option<&str>) -> Vec<(String, TodoItem, Option<String>)> {
    let mut rows: Vec<(String, TodoItem, Option<String>)> = Vec::new();
    if social {
        let Ok(entries) = std::fs::read_dir(base) else { return rows };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let Some(author) = path.file_name().and_then(|n| n.to_str()).map(String::from) else { continue };
            if Some(author.as_str()) == exclude_user { continue; }
            // 双布局兼容：生产 users/<name>/todo/<date>.json 与测试/遗留 <author>/<date>.json
            let date_dir = if path.join("todo").is_dir() { path.join("todo") } else { path.clone() };
            for date in list_dates(&date_dir) {
                if let Some(list) = read_list_in(&date_dir, &date) {
                    for item in list.items.into_iter().filter(|i| i.visibility == VISIBILITY_PUBLIC) {
                        rows.push((date.clone(), item, Some(author.clone())));
                    }
                }
            }
        }
    } else {
        for date in list_dates(base) {
            if let Some(list) = read_list_in(base, &date) {
                for item in list.items.into_iter() {
                    rows.push((date.clone(), item, None));
                }
            }
        }
    }
    rows
}

/// 排序（日期倒序、同日内创建倒序）→ before 翻页 → 按日期分组。
/// 返回 (分组, 是否还有更早, 翻页游标)
fn group_page(
    rows: &mut Vec<(String, TodoItem, Option<String>)>,
    before: Option<&str>,
    limit: usize,
) -> (Vec<(String, Vec<(TodoItem, Option<String>)>)>, bool, Option<String>) {
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.created.cmp(&a.1.created)));
    if let Some(b) = before {
        if valid_date(b).is_some() {
            rows.retain(|r| r.0.as_str() < b);
        }
    }
    let dates: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        rows.iter().map(|r| r.0.clone()).filter(|d| seen.insert(d.clone())).collect()
    };
    let page: std::collections::HashSet<String> = dates.iter().take(limit).cloned().collect();
    let has_more = dates.len() > page.len();
    let mut groups: Vec<(String, Vec<(TodoItem, Option<String>)>)> = Vec::new();
    for date in &dates {
        if !page.contains(date) { continue; }
        let items: Vec<(TodoItem, Option<String>)> = rows.iter()
            .filter(|r| &r.0 == date)
            .map(|r| (r.1.clone(), r.2.clone()))
            .collect();
        if !items.is_empty() { groups.push((date.clone(), items)); }
    }
    let next_before = groups.last().map(|(d, _)| d.clone());
    (groups, has_more, next_before)
}

/// timeline 核心收集：当前用户全量 → tag 过滤 → 分组翻页
fn collect_timeline(
    dir: &Path,
    tag: &Option<String>,
    before: Option<&str>,
    limit: usize,
) -> (Vec<(String, Vec<(TodoItem, Option<String>)>)>, bool, Option<String>) {
    let mut rows: Vec<(String, TodoItem, Option<String>)> = scan_rows(dir, false, None)
        .into_iter()
        .filter(|(_, item, _)| item_matches_tag(item, tag))
        .map(|(d, i, a)| (d, i, a))
        .collect();
    group_page(&mut rows, before, limit)
}

/// explore 核心收集：统计/标签云始终基于全量数据；feed 按 tag 过滤。
/// 返回 (分组, has_more, 游标, 标签云, 统计(total, done, days))
#[allow(clippy::type_complexity)]
fn collect_explore(
    base: &Path,
    social: bool,
    exclude_user: Option<&str>,
    tag: &Option<String>,
    before: Option<&str>,
    limit: usize,
) -> (Vec<(String, Vec<(TodoItem, Option<String>)>)>, bool, Option<String>, Vec<(String, u32)>, (usize, usize, usize)) {
    let mut rows = scan_rows(base, social, exclude_user);
    let total = rows.len();
    let done = rows.iter().filter(|(_, i, _)| i.done).count();
    let days = rows.iter().map(|r| r.0.clone()).collect::<std::collections::HashSet<_>>().len();
    let mut tag_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for (_, item, _) in &rows {
        if let Some(c) = &item.category { *tag_counts.entry(c.clone()).or_insert(0) += 1; }
    }
    let mut tags: Vec<(String, u32)> = tag_counts.into_iter().collect();
    tags.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    // feed 过滤放统计之后，保持统计为全量口径
    rows.retain(|(_, item, _)| item_matches_tag(item, tag));
    let (groups, has_more, next_before) = group_page(&mut rows, before, limit);
    (groups, has_more, next_before, tags, (total, done, days))
}

/// GET /api/todo/timeline — 当前用户跨日期卡片流（日期倒序分组，按日期数翻页）
pub async fn todo_timeline_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    Query(q): Query<TimelineQuery>,
) -> Response {
    let dir = todo_dir_for_user(&ctx, user.map(|u| u.0).as_ref());
    if let Some(b) = &q.before {
        if valid_date(b).is_none() { return bad_date_response(); }
    }
    let limit = clamp_limit(q.limit, 7);
    let (groups, has_more, next_before) = collect_timeline(&dir, &q.tag, q.before.as_deref(), limit);
    let groups_json: Vec<serde_json::Value> = groups.into_iter()
        .map(|(date, items)| {
            serde_json::json!({ "date": date, "items": items.into_iter().map(|(i, _)| i).collect::<Vec<_>>() })
        })
        .collect();
    Json(serde_json::json!({
        "status": "ok", "groups": groups_json, "has_more": has_more, "next_before": next_before,
    })).into_response()
}

/// GET /api/todo/explore — solo: 自己全量历史+标签云+统计；social(server): 他人 public 条目
pub async fn todo_explore_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    Query(q): Query<ExploreQuery>,
) -> Response {
    let is_server = ctx.auth.is_some();
    // server 模式社交流扫描用户根 users/*（每用户 todo/ 子目录）；
    // 旧实现扫 admin 自己的 todo_dir —— 其下没有作者子目录，社交流恒为空。
    let base = if is_server {
        crate::datadir::AppDataLayout::from_context(&ctx).root().join("users")
    } else {
        todo_dir(&ctx)
    };
    let mut exclude_user: Option<String> = None;
    if is_server {
        let Some(u) = user.map(|x| x.0) else {
            return (axum::http::StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"status":"error","message":"未登录"}))).into_response();
        };
        exclude_user = Some(u.username);
    }
    if let Some(b) = &q.before {
        if valid_date(b).is_none() { return bad_date_response(); }
    }
    let limit = clamp_limit(q.limit, 14);
    let (groups, has_more, next_before, tags, stats) =
        collect_explore(&base, is_server, exclude_user.as_deref(), &q.tag, q.before.as_deref(), limit);
    let groups_json: Vec<serde_json::Value> = groups.into_iter()
        .map(|(date, items)| {
            let arr: Vec<serde_json::Value> = items.into_iter().map(|(item, author)| {
                let mut v = serde_json::to_value(&item).unwrap_or_default();
                if let Some(a) = author { v["author"] = serde_json::Value::String(a); }
                v
            }).collect();
            serde_json::json!({ "date": date, "items": arr })
        })
        .collect();
    let tags_json: Vec<serde_json::Value> = tags.into_iter()
        .map(|(name, count)| serde_json::json!({"name": name, "count": count}))
        .collect();
    Json(serde_json::json!({
        "status": "ok",
        "mode": if is_server { "social" } else { "solo" },
        "groups": groups_json,
        "tags": tags_json,
        "stats": { "total": stats.0, "done": stats.1, "days": stats.2 },
        "has_more": has_more,
        "next_before": next_before,
    })).into_response()
}

/// 日期比较过期：事项属于过去日期且未完成 → 过期
fn is_expired_by_date(date: &str) -> bool {
    if let Ok(item_date) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        let today = Local::now().date_naive();
        return item_date < today;
    }
    false
}

/// 检查截止日期是否已过
fn is_overdue(item: &TodoItem) -> bool {
    if item.done { return false; }
    if let Some(ref dd) = item.due_date {
        if let Ok(due) = NaiveDate::parse_from_str(dd, "%Y-%m-%d") {
            let today = Local::now().date_naive();
            if due < today { return true; }
            if due == today {
                if let Some(ref dt) = item.due_time {
                    let now = Local::now();
                    let parts: Vec<u32> = dt.split(':').filter_map(|s| s.parse::<u32>().ok()).collect();
                    let h = parts.get(0).copied().unwrap_or(23);
                    let m = parts.get(1).copied().unwrap_or(59);
                    let now_min = now.hour() * 60 + now.minute();
                    let due_min = h * 60 + m;
                    return now_min > due_min;
                }
            }
        }
    }
    false
}

pub(crate) fn read_list_in(dir: &Path, date: &str) -> Option<TodoList> {
    std::fs::read_to_string(dir.join(format!("{}.json", date))).ok()
        .and_then(|s| serde_json::from_str::<TodoList>(&s).ok())
}

/// 原子写：tmp + rename，杜绝并发写撕裂/丢数据（rename 失败退回直接写）。
/// 返回是否成功 — 变更类 handler 必须检查（写失败仍报 ok 会让前端与磁盘静默分歧，P2-6）
pub(crate) fn write_list_in(dir: &Path, date: &str, list: &TodoList) -> bool {
    let file_path = dir.join(format!("{}.json", date));
    let tmp_path = dir.join(format!("{}.json.tmp", date));
    let Ok(content) = serde_json::to_string_pretty(list) else { return false };
    if std::fs::write(&tmp_path, &content).is_ok()
        && std::fs::rename(&tmp_path, &file_path).is_ok() { return true; }
    let _ = std::fs::remove_file(&tmp_path);
    std::fs::write(&file_path, &content).is_ok()
}

/// HH:MM 合法性（H 0-23 / M 0-59）— "25:99" 落盘会被前端 new Date(y,m,d,25,99)
/// 滚成次日 01:39，静默错时提醒（P2-1）
fn valid_hhmm(s: &str) -> bool {
    let Some((h, m)) = s.split_once(':') else { return false };
    match (h.trim().parse::<u32>(), m.trim().parse::<u32>()) {
        (Ok(h), Ok(m)) => h <= 23 && m <= 59,
        _ => false,
    }
}

/// 清洗提醒时间列表：非法 HH:MM 条目丢弃，合法项归一为补零两位（"7:5" → "07:05"）
fn sanitize_notify_times(times: &[String]) -> Vec<String> {
    let mut out: Vec<String> = times.iter().filter_map(|s| {
        let s = s.trim();
        if !valid_hhmm(s) { return None; }
        let (h, m) = s.split_once(':').unwrap();
        Some(format!("{:02}:{:02}", h.trim().parse::<u32>().unwrap(), m.trim().parse::<u32>().unwrap()))
    }).collect();
    out.dedup();
    out
}

/// 提前提醒分钟白名单 — 设计约定仅 5/10/30（P2-2；任意值如 100000 会把提醒推到几十年前）
fn sanitize_advance_minutes(v: &[u32]) -> Vec<u32> {
    v.iter().copied().filter(|m| matches!(m, 5 | 10 | 30)).collect()
}

/// 重复提醒间隔上限 24 小时（分钟），防脏值打爆通知引擎（P2-2）
fn sanitize_repeat_interval(v: u32) -> u32 { v.min(24 * 60) }

fn check_and_mark_expired(list: &mut TodoList, date: &str) -> bool {
    let mut changed = false;
    let date_expired = is_expired_by_date(date);
    for item in &mut list.items {
        if !item.done && !item.expired && date_expired {
            item.expired = true;
            changed = true;
        }
    }
    changed
}

// ─── 第三方笔记云同步数据面（export/import）───
// 通用中间格式：{version, exported_at, items:[{date, ...条目}]}。
// 刻意不绑定任何第三方 schema —— memos/flomo/Notion 等由适配层转换为该格式后
// 走 /api/todo/import；导出侧同理。条目剥离运行时字段（last_notified/expired），
// 保证跨实例可移植。合并语义：按 (date, id) 去重，已存在跳过 —— 幂等可重放。

/// 条目 → 可移植 JSON（剥离运行时状态）
fn portable_item(date: &str, item: &TodoItem) -> serde_json::Value {
    serde_json::json!({
        "date": date,
        "id": item.id,
        "title": item.title,
        "content": item.content,
        "done": item.done,
        "created": item.created,
        "priority": item.priority,
        "category": item.category,
        "due_date": item.due_date,
        "due_time": item.due_time,
        "notify_enabled": item.notify_enabled,
        "notify_times": item.notify_times,
        "advance_minutes": item.advance_minutes,
        "repeat_enabled": item.repeat_enabled,
        "repeat_interval": item.repeat_interval,
        "visibility": item.visibility,
        "sort_order": item.sort_order,
    })
}

#[derive(serde::Deserialize)]
pub struct TodoImportRequest {
    pub items: Vec<serde_json::Value>,
    /// merge（默认，幂等合并）| skip_existing 行为一致；保留字段以便未来扩展 replace
    #[serde(default)]
    pub mode: Option<String>,
}

/// GET /api/todo/export — 全量导出（当前用户 / standalone 本地）
pub async fn todo_export_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Response {
    let dir = todo_dir_for_user(&ctx, user.map(|u| u.0).as_ref());
    let mut items: Vec<serde_json::Value> = Vec::new();
    for date in list_dates(&dir) {
        if let Some(list) = read_list_in(&dir, &date) {
            for item in &list.items {
                items.push(portable_item(&date, item));
            }
        }
    }
    Json(serde_json::json!({
        "status": "ok",
        "format": "skyland-todo/v1",
        "exported_at": now_ts(),
        "count": items.len(),
        "items": items,
    })).into_response()
}

/// POST /api/todo/import — 合并导入（按 date+id 去重；无 id 或撞 id 则生成新 id）
/// 导入核心 — HTTP handler 与云同步引擎（todo_sync）共用。
/// items 为 skyland-todo/v1 条目（date/title/...），按 date 分组合并、内容寻址 id 幂等。
/// 返回 (imported, skipped, rejected)
pub(crate) fn import_core(
    dir: &std::path::Path,
    items: &[serde_json::Value],
) -> (u32, u32, Vec<String>) {
    let mut imported: u32 = 0;
    let mut skipped: u32 = 0;
    let mut rejected: Vec<String> = Vec::new();
    let mut by_date: std::collections::BTreeMap<String, Vec<serde_json::Value>> = Default::default();
    for (idx, raw) in items.iter().enumerate() {
        let Some(date) = raw.get("date").and_then(|v| v.as_str()).map(String::from) else {
            rejected.push(format!("#{}: 缺少 date", idx));
            continue;
        };
        if valid_date(&date).is_none() {
            rejected.push(format!("#{}: 非法日期 {}", idx, date));
            continue;
        }
        let title = raw.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if title.is_empty() {
            rejected.push(format!("#{}: 缺少 title", idx));
            continue;
        }
        by_date.entry(date).or_default().push(raw.clone());
    }
    for (date, entries) in by_date {
        let mut list = read_list_in(dir, &date).unwrap_or_else(|| TodoList {
            date: date.clone(), items: vec![], created: now_ts(),
        });
        for raw in entries {
            let title = raw.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let content = raw.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let category = raw.get("category").and_then(|v| v.as_str())
                .map(String::from).filter(|c| !c.is_empty());
            let given_id = raw.get("id").and_then(|v| v.as_str()).map(String::from);
            // 先解析最终 id（显式 id 优先，否则内容寻址 id），再按它去重 —
            // 无 id 的第三方同步重放（同日期+同标题+正文+分类）天然幂等
            let resolved_id = given_id.clone().unwrap_or_else(|| format!("imp_{}_{:08x}", date, {
                let mut h: u32 = 0x811c9dc5;
                for b in format!("{}|{}|{}", title, content, category.as_deref().unwrap_or("")).bytes() {
                    h ^= b as u32;
                    h = h.wrapping_mul(0x01000193);
                }
                h
            }));
            if list.items.iter().any(|i| i.id == resolved_id) {
                skipped += 1;
                continue;
            }
            let priority = raw.get("priority").and_then(|v| v.as_u64()).map(|p| (p as u8).min(2)).unwrap_or(1);
            let due_date = raw.get("due_date").and_then(|v| v.as_str())
                .map(String::from).filter(|s| valid_date(s).is_some());
            let due_time = raw.get("due_time").and_then(|v| v.as_str())
                .map(String::from).filter(|s| valid_hhmm(s));
            let notify_times = sanitize_notify_times(&raw.get("notify_times")
                .and_then(|v| v.as_array()).map(|a| a.iter()
                    .filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>())
                .unwrap_or_default());
            let advance = sanitize_advance_minutes(&raw.get("advance_minutes")
                .and_then(|v| v.as_array()).map(|a| a.iter()
                    .filter_map(|x| x.as_u64().map(|n| n as u32)).collect::<Vec<_>>())
                .unwrap_or_default());
            let visibility = normalize_visibility(&raw.get("visibility")
                .and_then(|v| v.as_str()).map(String::from));
            let item = TodoItem {
                id: resolved_id,
                title,
                content,
                done: raw.get("done").and_then(|v| v.as_bool()).unwrap_or(false),
                expired: false,
                created: raw.get("created").and_then(|v| v.as_u64()).unwrap_or_else(now_ts),
                notify_time: None,
                notify_enabled: !notify_times.is_empty(),
                notify_times,
                advance_minutes: advance,
                repeat_enabled: raw.get("repeat_enabled").and_then(|v| v.as_bool()).unwrap_or(false),
                repeat_interval: sanitize_repeat_interval(raw.get("repeat_interval")
                    .and_then(|v| v.as_u64()).unwrap_or(0) as u32),
                last_notified: None,
                priority,
                category,
                due_date,
                due_time,
                sort_order: raw.get("sort_order").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                visibility,
            };
            list.items.push(item);
            imported += 1;
        }
        if !write_list_in(dir, &date, &list) {
            // 写入失败：回滚该日期已计数部分不可行（逐条写入），保守报错整批该日期
            rejected.push(format!("{}: 写入失败", date));
        }
    }
    (imported, skipped, rejected)
}

/// POST /api/todo/import — 合并导入（按 date+id 去重；无 id 或撞 id 则生成新 id）
pub async fn todo_import_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<TodoImportRequest>,
) -> Response {
    // 预留合并策略位：当前仅支持幂等合并；未知模式显式拒绝（防客户端误期待替换语义）
    if let Some(m) = req.mode.as_deref() {
        if m != "merge" && m != "skip_existing" {
            return (axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "status": "error", "message": format!("不支持的导入模式: {}", m) }))).into_response();
        }
    }
    let dir = todo_dir_for_user(&ctx, user.map(|u| u.0).as_ref());
    let (imported, skipped, rejected) = import_core(&dir, &req.items);
    Json(serde_json::json!({
        "status": "ok", "imported": imported, "skipped": skipped, "rejected": rejected,
    })).into_response()
}

// ─── API 处理器 ───

/// GET /api/todo/today — 获取或创建今日待办
pub async fn todo_today_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Response {
    let dir = todo_dir_for_user(&ctx, user.map(|u| u.0).as_ref());
    let date = today_str();
    let mut list = read_list_in(&dir, &date).unwrap_or_else(|| TodoList {
        date: date.clone(), items: vec![], created: now_ts(),
    });

    let changed = check_and_mark_expired(&mut list, &date);
    if changed { write_list_in(&dir, &date, &list); }

    // 检查过去 7 天的未完成项（不仅仅是昨天）
    let mut carryover_items: Vec<TodoItem> = Vec::new();
    let mut carryover_dates: Vec<String> = Vec::new();
    for days_back in 1..=7 {
        let past = Local::now().date_naive() - ChronoDuration::days(days_back);
        let past_str = past.format("%Y-%m-%d").to_string();
        if let Some(mut l) = read_list_in(&dir, &past_str) {
            check_and_mark_expired(&mut l, &past_str);
            let undone: Vec<TodoItem> = l.items.iter().filter(|i| !i.done).cloned().collect();
            if !undone.is_empty() {
                carryover_dates.push(past_str.clone());
                carryover_items.extend(undone);
            }
        }
    }

    Json(serde_json::json!({
        "status": "ok",
        "list": list,
        "carryover": carryover_items,
        "carryover_dates": carryover_dates,
    })).into_response()
}

/// GET /api/todo/list — 列出所有待办日期
pub async fn todo_list_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Response {
    let dir = todo_dir_for_user(&ctx, user.map(|u| u.0).as_ref());
    let mut dates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") || name.contains(".tmp") { continue; }
            let date = name.trim_end_matches(".json").to_string();
            if valid_date(&date).is_none() { continue; }
            let modified = entry.metadata().map(|m| m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)).unwrap_or(0);
            let (item_count, done_count) = std::fs::read_to_string(entry.path()).ok()
                .and_then(|s| serde_json::from_str::<TodoList>(&s).ok())
                .map(|l| (l.items.len(), l.items.iter().filter(|i| i.done).count()))
                .unwrap_or((0, 0));
            dates.push(serde_json::json!({ "date": date, "items": item_count, "done": done_count, "modified": modified }));
        }
    }
    dates.sort_by(|a, b| b["date"].as_str().unwrap_or("").cmp(a["date"].as_str().unwrap_or("")));
    Json(serde_json::json!({ "dates": dates })).into_response()
}

/// POST /api/todo/:date/item — 添加事项
pub async fn todo_add_item_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    AxumPath(date): AxumPath<String>,
    Json(req): Json<SaveTodoItemRequest>,
) -> Response {
    if valid_date(&date).is_none() { return bad_date_response(); }
    let dir = todo_dir_for_user(&ctx, user.map(|u| u.0).as_ref());
    let mut list = read_list_in(&dir, &date).unwrap_or_else(|| TodoList {
        date: date.clone(), items: vec![], created: now_ts(),
    });

    let max_sort = list.items.iter().map(|i| i.sort_order).max().unwrap_or(0);
    let item = TodoItem {
        // 毫秒时间戳 + 随机后缀，防多标签页同毫秒碰撞
        id: format!("item_{}_{}", SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0), rand::random::<u32>()),
        title: req.title,
        content: req.content,
        done: false,
        expired: false,
        created: now_ts(),
        notify_time: req.notify_time.filter(|s| valid_hhmm(s)),
        notify_enabled: req.notify_enabled.unwrap_or(false),
        notify_times: sanitize_notify_times(&req.notify_times.unwrap_or_default()),
        advance_minutes: sanitize_advance_minutes(&req.advance_minutes.unwrap_or_default()),
        repeat_enabled: req.repeat_enabled.unwrap_or(false),
        repeat_interval: sanitize_repeat_interval(req.repeat_interval.unwrap_or(0)),
        last_notified: None,
        priority: req.priority.unwrap_or(1),
        category: req.category.filter(|s| !s.is_empty()),
        due_date: req.due_date.filter(|s| valid_date(s).is_some()),
        due_time: req.due_time.filter(|s| valid_hhmm(s)),
        sort_order: max_sort + 1,
        visibility: normalize_visibility(&req.visibility),
    };
    list.items.push(item.clone());
    if !write_list_in(&dir, &date, &list) {
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "status": "error", "message": "待办写入失败（磁盘/权限）" }))).into_response();
    }
    Json(serde_json::json!({ "status": "ok", "item": item })).into_response()
}

/// PUT /api/todo/:date/item/:id — 更新事项
pub async fn todo_update_item_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    AxumPath((date, item_id)): AxumPath<(String, String)>,
    Json(req): Json<UpdateTodoItemRequest>,
) -> Response {
    if valid_date(&date).is_none() { return bad_date_response(); }
    let dir = todo_dir_for_user(&ctx, user.map(|u| u.0).as_ref());
    let mut list = match read_list_in(&dir, &date) {
        Some(l) => l,
        None => return Json(serde_json::json!({ "status": "error", "message": "待办列表不存在" })).into_response(),
    };

    for item in &mut list.items {
        if item.id == item_id {
            if let Some(t) = &req.title { item.title = t.clone(); }
            if let Some(c) = &req.content { item.content = c.clone(); }
            if let Some(d) = req.done { item.done = d; if d { item.expired = false; item.last_notified = None; } }
            if let Some(nt) = &req.notify_time { item.notify_time = if valid_hhmm(nt) { Some(nt.clone()) } else { None }; }
            if let Some(ne) = req.notify_enabled { item.notify_enabled = ne; }
            if let Some(nt) = &req.notify_times { item.notify_times = sanitize_notify_times(nt); }
            if let Some(am) = &req.advance_minutes { item.advance_minutes = sanitize_advance_minutes(am); }
            if let Some(re) = req.repeat_enabled { item.repeat_enabled = re; }
            if let Some(ri) = req.repeat_interval { item.repeat_interval = sanitize_repeat_interval(ri); }
            if let Some(ln) = req.last_notified { item.last_notified = Some(ln); }
            if let Some(p) = req.priority { item.priority = p; }
            if let Some(c) = &req.category { item.category = if c.is_empty() { None } else { Some(c.clone()) }; }
            if let Some(dd) = &req.due_date { item.due_date = if dd.is_empty() { None } else { valid_date(dd).map(|_| dd.clone()) }; }
            if let Some(dt) = &req.due_time { item.due_time = if dt.is_empty() { None } else { if valid_hhmm(dt) { Some(dt.clone()) } else { item.due_time.clone() } }; }
            if let Some(so) = req.sort_order { item.sort_order = so; }
            if let Some(v) = &req.visibility { item.visibility = normalize_visibility(&Some(v.clone())); }
            let mut updated_item = item.clone();

            // 跨日期迁移：目标先写（P1-1 先例 — 目标失败则源原样保留，绝不凭空消失），
            // 目标落盘后再从源移除写回；源写失败时事项新旧两天各一份，宁可重复不可丢失。
            if let Some(mt) = req.move_to.as_deref().filter(|s| !s.is_empty()) {
                if valid_date(mt).is_none() { return bad_date_response(); }
                if mt != date {
                    let mut target_list = read_list_in(&dir, mt).unwrap_or_else(|| TodoList {
                        date: mt.to_string(), items: vec![], created: now_ts(),
                    });
                    let max_sort = target_list.items.iter().map(|i| i.sort_order).max().unwrap_or(0);
                    updated_item.sort_order = max_sort + 1;
                    target_list.items.push(updated_item.clone());
                    if !write_list_in(&dir, mt, &target_list) {
                        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "status": "error",
                                "message": "迁移目标日期写入失败 — 事项保留在原日期，请重试" }))).into_response();
                    }
                    list.items.retain(|i| i.id != item_id);
                    if !write_list_in(&dir, &date, &list) {
                        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "status": "error",
                                "message": "源日期写入失败 — 事项可能在新旧两天各存在一份，请手动删除一份" }))).into_response();
                    }
                    return Json(serde_json::json!({ "status": "ok", "item": updated_item, "moved_to": mt })).into_response();
                }
            }

            if !write_list_in(&dir, &date, &list) {
                return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "status": "error", "message": "待办写入失败（磁盘/权限）" }))).into_response();
            }
            return Json(serde_json::json!({ "status": "ok", "item": updated_item })).into_response();
        }
    }
    Json(serde_json::json!({ "status": "error", "message": "事项不存在" })).into_response()
}

/// DELETE /api/todo/:date/item/:id — 删除事项
pub async fn todo_delete_item_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    AxumPath((date, item_id)): AxumPath<(String, String)>,
) -> Response {
    if valid_date(&date).is_none() { return bad_date_response(); }
    let dir = todo_dir_for_user(&ctx, user.map(|u| u.0).as_ref());
    let mut list = match read_list_in(&dir, &date) {
        Some(l) => l,
        None => return Json(serde_json::json!({ "status": "error", "message": "待办列表不存在" })).into_response(),
    };

    let before = list.items.len();
    list.items.retain(|i| i.id != item_id);
    if list.items.len() < before {
        if !write_list_in(&dir, &date, &list) {
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "status": "error", "message": "待办写入失败（磁盘/权限）" }))).into_response();
        }
        Json(serde_json::json!({ "status": "ok" })).into_response()
    } else {
        Json(serde_json::json!({ "status": "error", "message": "事项不存在" })).into_response()
    }
}

/// GET /api/todo/:date — 获取指定日期待办
pub async fn todo_get_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    AxumPath(date): AxumPath<String>,
) -> Response {
    if valid_date(&date).is_none() { return bad_date_response(); }
    let dir = todo_dir_for_user(&ctx, user.map(|u| u.0).as_ref());
    if let Some(mut list) = read_list_in(&dir, &date) {
        let changed = check_and_mark_expired(&mut list, &date);
        if changed { write_list_in(&dir, &date, &list); }
        Json(serde_json::json!({ "status": "ok", "list": list })).into_response()
    } else {
        Json(serde_json::json!({ "status": "ok", "list": { "date": date, "items": [], "created": 0 } })).into_response()
    }
}

/// POST /api/todo/:date/carryover — 延续未完成项到指定日期
pub async fn todo_carryover_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    AxumPath(target_date): AxumPath<String>,
    Json(req): Json<serde_json::Value>,
) -> Response {
    if valid_date(&target_date).is_none() { return bad_date_response(); }
    let dir = todo_dir_for_user(&ctx, user.map(|u| u.0).as_ref());
    let source_dates: Vec<String> = req.get("source_dates")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|i| i.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let item_ids: Vec<String> = req.get("item_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|i| i.as_str().map(String::from)).collect())
        .unwrap_or_default();

    if source_dates.is_empty() {
        return Json(serde_json::json!({ "status": "error", "message": "缺少源日期" })).into_response();
    }
    for d in &source_dates {
        if valid_date(d).is_none() { return bad_date_response(); }
    }

    let mut target_list = read_list_in(&dir, &target_date).unwrap_or_else(|| TodoList {
        date: target_date.clone(), items: vec![], created: now_ts(),
    });

    let max_sort = target_list.items.iter().map(|i| i.sort_order).max().unwrap_or(0);
    let mut carried = 0;
    let mut sort_offset = 1;

    let mut sources_to_mark: Vec<(String, TodoList)> = Vec::new();
    for source_date in &source_dates {
        let mut source_list = match read_list_in(&dir, source_date) {
            Some(l) => l,
            None => continue,
        };
        let mut marked_any = false;
        for item in &mut source_list.items {
            if !item.done && (item_ids.is_empty() || item_ids.contains(&item.id)) {
                let mut new_item = item.clone();
                new_item.id = format!("item_{}_{}", SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0), carried);
                new_item.created = now_ts();
                new_item.expired = false;
                new_item.last_notified = None;
                new_item.sort_order = max_sort + sort_offset;
                target_list.items.push(new_item);
                // Mark source item as done to prevent duplicate carryover
                item.done = true;
                marked_any = true;
                carried += 1;
                sort_offset += 1;
            }
        }
        if marked_any { sources_to_mark.push((source_date.clone(), source_list)); }
    }

    // P1-1 修复：先写 target 且成功，再标记源 done — 旧实现先把源标完成再写 target，
    // target 写失败时结转的事项凭空消失（源已 done + 目标未落盘）。现在 target 失败 = 源原样保留可重试。
    if carried > 0 {
        if !write_list_in(&dir, &target_date, &target_list) {
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                "status": "error", "message": "结转写入目标日期失败 — 源事项未改动，请检查磁盘后重试"
            }))).into_response();
        }
        let mut failed_sources = 0usize;
        for (source_date, source_list) in &sources_to_mark {
            if !write_list_in(&dir, source_date, source_list) { failed_sources += 1; }
        }
        let mut resp = serde_json::json!({ "status": "ok", "carried": carried, "list": target_list });
        if failed_sources > 0 {
            resp["message"] = serde_json::Value::String(format!(
                "{} 个源日期写入失败，其事项可能被再次结转", failed_sources));
        }
        return Json(resp).into_response();
    }
    Json(serde_json::json!({ "status": "ok", "carried": carried, "list": target_list })).into_response()
}

/// GET /api/todo/search?q=xxx — 跨日期搜索（当前用户范围内）
pub async fn todo_search_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    Query(query): Query<SearchQuery>,
) -> Response {
    let dir = todo_dir_for_user(&ctx, user.map(|u| u.0).as_ref());
    let keyword = query.q.to_lowercase();
    let mut results: Vec<serde_json::Value> = Vec::new();

    for date in list_dates(&dir) {
        if let Some(list) = read_list_in(&dir, &date) {
            for item in &list.items {
                let title_match = item.title.to_lowercase().contains(&keyword);
                let content_match = item.content.to_lowercase().contains(&keyword);
                let category_match = item.category.as_ref().map(|c| c.to_lowercase().contains(&keyword)).unwrap_or(false);
                if title_match || content_match || category_match {
                    results.push(serde_json::json!({
                        "date": date,
                        "item": item,
                    }));
                }
            }
        }
    }

    Json(serde_json::json!({ "status": "ok", "results": results, "count": results.len() })).into_response()
}

/// GET /api/todo/stats — 统计面板数据
pub async fn todo_stats_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Response {
    let dir = todo_dir_for_user(&ctx, user.map(|u| u.0).as_ref());
    let today = Local::now().date_naive();
    // 最近 7 天 = today-6 ..= today（含今天，共 7 天）
    let week_ago = today - ChronoDuration::days(6);

    let mut total_7d = 0u32;
    let mut done_7d = 0u32;
    let mut today_total = 0u32;
    let mut today_done = 0u32;
    let mut today_overdue = 0u32;
    let mut streak = 0u32;
    let mut categories: std::collections::HashMap<String, (u32, u32)> = std::collections::HashMap::new();

    // 从今天往前找连续完成天数（0 条也算完成，跳过继续；有未完成才中断）
    let mut check_date = today;
    loop {
        let date_str = check_date.format("%Y-%m-%d").to_string();
        let has_data = read_list_in(&dir, &date_str).map(|l| {
            let total = l.items.len() as u32;
            let done = l.items.iter().filter(|i| i.done).count() as u32;
            (total, done)
        });
        match has_data {
            Some((total, done)) if total > 0 && done == total => {
                streak += 1;
                check_date -= ChronoDuration::days(1);
            }
            Some((total, done)) if total > 0 && done < total => break, // 有未完成的
            Some((0, _)) => {
                // 0 条事项 — 如果是今天，跳过（可能还没创建）；如果是过去日期，也算完成
                if check_date == today { check_date -= ChronoDuration::days(1); continue; }
                streak += 1;
                check_date -= ChronoDuration::days(1);
            }
            _ => break, // 无数据文件
        }
    }

    // 扫描最近 7 天（today-6 ..= today）
    for i in 0..7 {
        let d = week_ago + ChronoDuration::days(i);
        let date_str = d.format("%Y-%m-%d").to_string();
        if let Some(list) = read_list_in(&dir, &date_str) {
            for item in &list.items {
                total_7d += 1;
                if item.done { done_7d += 1; }
                if d == today {
                    today_total += 1;
                    if item.done { today_done += 1; }
                    if is_overdue(item) { today_overdue += 1; }
                }
                let cat = item.category.clone().unwrap_or_else(|| "未分类".to_string());
                let entry = categories.entry(cat).or_insert((0, 0));
                entry.0 += 1;
                if item.done { entry.1 += 1; }
            }
        }
    }

    let completion_rate = if total_7d > 0 { (done_7d as f64 / total_7d as f64 * 100.0).round() as u32 } else { 0 };

    let cat_stats: Vec<serde_json::Value> = categories.iter()
        .map(|(k, v)| serde_json::json!({ "category": k, "total": v.0, "done": v.1 }))
        .collect();

    Json(serde_json::json!({
        "status": "ok",
        "today": { "total": today_total, "done": today_done, "overdue": today_overdue },
        "week": { "total": total_7d, "done": done_7d, "completion_rate": completion_rate },
        "streak": streak,
        "categories": cat_stats,
    })).into_response()
}

/// GET /api/todo/path — 获取待办数据目录（server 模式仅 admin：响应含服务器绝对路径，最小暴露，P2-8）
pub async fn todo_path_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Response {
    if ctx.auth.is_some() {
        let Some(u) = user.map(|x| x.0) else {
            return (axum::http::StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"status":"error","message":"未登录"}))).into_response();
        };
        if u.role != "admin" {
            return (axum::http::StatusCode::FORBIDDEN,
                Json(serde_json::json!({"status":"error","message":"仅管理员可查看数据目录"}))).into_response();
        }
    }
    let dir = ctx.todo_dir.read().unwrap_or_else(|e| e.into_inner());
    let base = &ctx.work_dir;
    let display = dir.strip_prefix(base)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| dir.to_string_lossy().replace('\\', "/"));
    Json(serde_json::json!({ "path": display, "directory": dir.display().to_string() })).into_response()
}

/// POST /api/todo/path — 设置待办数据目录
/// server 模式下数据目录是全体用户的全局资源，仅 admin 可改
pub async fn todo_path_save_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<serde_json::Value>,
) -> Response {
    if ctx.auth.is_some() {
        let Some(u) = user.map(|x| x.0) else {
            return (axum::http::StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"status":"error","message":"未登录"}))).into_response();
        };
        if u.role != "admin" {
            return (axum::http::StatusCode::FORBIDDEN,
                Json(serde_json::json!({"status":"error","message":"仅管理员可修改数据目录"}))).into_response();
        }
    }
    let raw = req.get("path").and_then(|v| v.as_str()).unwrap_or("").trim();
    if raw.is_empty() || raw == "." || raw == ".." {
        return Json(serde_json::json!({ "status": "error", "message": "路径不能为空" })).into_response();
    }

    let relative = std::path::Path::new(raw);
    let target = if relative.is_absolute() {
        // Only allow absolute paths in desktop mode (consistent with image path handler)
        if !crate::env_compat::desktop_mode() {
            return Json(serde_json::json!({ "status": "error", "message": "网页模式只能选择项目目录下的相对路径" })).into_response();
        }
        PathBuf::from(raw)
    } else {
        // 验证路径安全
        for component in relative.components() {
            if !matches!(component, std::path::Component::Normal(_)) {
                return Json(serde_json::json!({ "status": "error", "message": "路径不能包含 .、.. 或特殊字符" })).into_response();
            }
        }
        ctx.work_dir.join(raw)
    };

    std::fs::create_dir_all(&target).ok();
    if let Ok(canonical) = std::fs::canonicalize(&target) {
        let mut dir = ctx.todo_dir.write().unwrap_or_else(|e| e.into_inner());
        *dir = canonical;
    } else {
        let mut dir = ctx.todo_dir.write().unwrap_or_else(|e| e.into_inner());
        *dir = target;
    }

    let dir = ctx.todo_dir.read().unwrap_or_else(|e| e.into_inner());
    let display = dir.strip_prefix(&ctx.work_dir)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| dir.to_string_lossy().replace('\\', "/"));

    Json(serde_json::json!({ "status": "ok", "path": display, "directory": dir.display().to_string() })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Extension;
    use std::sync::Arc;

    fn test_ctx(dir: &std::path::Path) -> AppContext {
        AppContext {
            providers: std::collections::HashMap::new(),
            default_model: String::new(),
            user_data_dir: dir.to_path_buf(),
            work_dir: dir.to_path_buf(),
            context_lengths: std::collections::HashMap::new(),
            image_dir: Arc::new(std::sync::RwLock::new(dir.to_path_buf())),
            todo_dir: Arc::new(std::sync::RwLock::new(dir.join("ToDoList"))),
            auth: None,
            server_mode: "standalone".into(),
            cloud_workdir: dir.join("CloudWork"),
        }
    }

    #[test]
    fn valid_date_rejects_traversal_and_garbage() {
        assert!(valid_date("2026-09-11").is_some());
        assert!(valid_date("..").is_none());
        assert!(valid_date("../x").is_none());
        assert!(valid_date("2026-13-99").is_none());
        assert!(valid_date("").is_none());
    }

    #[test]
    fn visibility_defaults_and_normalizes() {
        assert_eq!(default_visibility(), "private");
        assert_eq!(normalize_visibility(&None), "private");
        assert_eq!(normalize_visibility(&Some("public".into())), "public");
        assert_eq!(normalize_visibility(&Some("private".into())), "private");
        assert_eq!(normalize_visibility(&Some("hacker".into())), "private");
    }

    #[test]
    fn notify_field_sanitization() {
        // P2-1/P2-2 防线：非法 HH:MM / 越界 advance / 脏 repeat 值不得落盘
        assert_eq!(sanitize_notify_times(&[
            "09:00".into(), "25:99".into(), "7:5".into(), "".into(), "12:60".into(),
        ]), vec!["09:00".to_string(), "07:05".to_string()]);
        assert_eq!(sanitize_advance_minutes(&[5, 10, 30, 100000]), vec![5, 10, 30]);
        assert_eq!(sanitize_repeat_interval(99999), 1440);
        assert_eq!(sanitize_repeat_interval(15), 15);
        assert!(valid_hhmm("23:59") && valid_hhmm("7:05"));
        assert!(!valid_hhmm("24:00") && !valid_hhmm("09:60") && !valid_hhmm("abc"));
    }

    #[test]
    fn claim_does_not_overwrite_existing_and_still_migrates() {
        // P2-7：admin 先建个人数据后，根目录遗留仍应被接管（旧实现会永久跳过）
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("ToDoList");
        let target = base.join("root");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("2026-09-10.json"),
            r#"{"date":"2026-09-10","items":[],"created":9}"#).unwrap();
        std::fs::write(base.join("2026-09-10.json"),
            r#"{"date":"2026-09-10","items":[{"id":"x"}],"created":1}"#).unwrap();
        std::fs::write(base.join("2026-09-12.json"),
            r#"{"date":"2026-09-12","items":[],"created":2}"#).unwrap();
        let moved = claim_legacy_files(&base, &target);
        assert!(moved, "无冲突的遗留文件仍应被接管");
        let kept = std::fs::read_to_string(target.join("2026-09-10.json")).unwrap();
        assert!(kept.contains("\"created\":9"), "既有个人数据不得被覆盖");
        assert!(target.join("2026-09-12.json").exists(), "无冲突遗留应迁入个人目录");
        assert!(!base.join("2026-09-12.json").exists(), "迁入后根目录不得残留");
    }

    #[tokio::test]
    async fn carryover_target_first_then_mark_source() {
        // P1-1：结转顺序 = 先写 target 成功再标记源 done
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        let dir = todo_dir_for_user(&ctx, None);
        write_date(&dir, "2026-09-10", vec![
            seed_item("2026-09-10", "i1", "事项", None, "private", false),
            seed_item("2026-09-10", "i2", "已完成", None, "private", true),
        ]);
        let _resp = todo_carryover_handler(
            State(ctx),
            None,
            AxumPath("2026-09-11".to_string()),
            Json(serde_json::json!({"source_dates": ["2026-09-10"], "item_ids": []})),
        ).await;
        let target = read_list_in(&dir, "2026-09-11")
            .expect("结转后目标日期必须落盘");
        assert_eq!(target.items.len(), 1, "只有未完成项被结转");
        let source = read_list_in(&dir, "2026-09-10").unwrap();
        assert!(source.items.iter().find(|i| i.id == "i1").unwrap().done, "源事项在 target 落盘后被标记完成");
        assert!(source.items.iter().find(|i| i.id == "i2").unwrap().done, "已完成项保持原样");
    }

    #[test]
    fn atomic_write_roundtrip_and_no_tmp_left() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let list = TodoList { date: "2026-09-11".into(), items: vec![], created: 1 };
        write_list_in(dir, "2026-09-11", &list);
        let back = read_list_in(dir, "2026-09-11").unwrap();
        assert_eq!(back.date, "2026-09-11");
        let leftovers: Vec<_> = std::fs::read_dir(dir).unwrap().flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp")).collect();
        assert!(leftovers.is_empty(), "原子写失败后不得残留 tmp 文件");
    }

    #[test]
    fn server_mode_isolates_per_user_and_claim_takes_over() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("ToDoList");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("2026-09-10.json"),
            r#"{"date":"2026-09-10","items":[],"created":1}"#).unwrap();
        let mut ctx = test_ctx(tmp.path());
        ctx.user_data_dir = tmp.path().join("users").join("local");
        ctx.auth = Some(crate::auth::AuthState::new(tmp.path().join("cfg")));

        // 无身份 → 基目录（中间件保证 server 模式必有身份，此为兜底）
        assert_eq!(todo_dir_for_user(&ctx, None), base);
        // 普通用户 → 独立目录；遗留共享数据不得迁移给她
        let alice = CurrentUser { username: "alice".into(), role: "user".into() };
        let d = todo_dir_for_user(&ctx, Some(&alice));
        assert_eq!(d, tmp.path().join("users/alice/todo"));
        assert!(d.exists());
        assert!(base.join("2026-09-10.json").exists(), "非 admin 不得搬走遗留数据");
        // admin 也使用自己的用户目录，不在请求期间搬动旧根目录
        let admin = CurrentUser { username: "root".into(), role: "admin".into() };
        let d = todo_dir_for_user(&ctx, Some(&admin));
        assert_eq!(d, tmp.path().join("users/root/todo"));
        let d2 = todo_dir_for_user(&ctx, Some(&admin));
        assert_eq!(d2, d);
        assert!(base.join("2026-09-10.json").exists());
    }

    fn seed_item(date: &str, id: &str, title: &str, category: Option<&str>, visibility: &str, done: bool) -> TodoItem {
        let _ = date;
        TodoItem {
            id: id.into(), title: title.into(), content: String::new(), done, expired: false,
            created: 1, notify_time: None, notify_enabled: false, notify_times: vec![],
            advance_minutes: vec![], repeat_enabled: false, repeat_interval: 0, last_notified: None,
            priority: 1, category: category.map(|s| s.into()), due_date: None, due_time: None,
            sort_order: 0, visibility: visibility.into(),
        }
    }

    fn write_date(dir: &std::path::Path, date: &str, items: Vec<TodoItem>) {
        let list = TodoList { date: date.into(), items, created: 1 };
        write_list_in(dir, date, &list);
    }

    #[test]
    fn timeline_groups_desc_filters_and_paginates() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        let dir = todo_dir_for_user(&ctx, None);
        write_date(&dir, "2026-09-11", vec![
            seed_item("2026-09-11", "a", "工作事项", Some("工作"), "private", false),
            seed_item("2026-09-11", "b", "生活事项", Some("生活"), "private", false)]);
        write_date(&dir, "2026-09-10", vec![
            seed_item("2026-09-10", "c", "昨日工作", Some("工作"), "private", true)]);
        write_date(&dir, "2026-09-09", vec![
            seed_item("2026-09-09", "d", "更早", None, "private", false)]);

        // 全量：日期倒序
        let (groups, has_more, next_before) = collect_timeline(&dir, &None, None, 7);
        let dates: Vec<&str> = groups.iter().map(|(d, _)| d.as_str()).collect();
        assert_eq!(dates, vec!["2026-09-11", "2026-09-10", "2026-09-09"]);
        assert!(!has_more);
        assert_eq!(next_before.as_deref(), Some("2026-09-09"));

        // tag 过滤
        let tag = Some("工作".to_string());
        let (groups, _, _) = collect_timeline(&dir, &tag, None, 7);
        let count: usize = groups.iter().map(|(_, items)| items.len()).sum();
        assert_eq!(count, 2);

        // before + limit 翻页
        let (groups, has_more, _) = collect_timeline(&dir, &None, Some("2026-09-10"), 1);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, "2026-09-09");
        assert!(!has_more);
    }

    #[test]
    fn explore_solo_and_social() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("ToDoList");
        std::fs::create_dir_all(&base).unwrap();
        // alice: 1 public + 1 private；bob: 1 public（已完成）
        let alice = base.join("alice");
        let bob = base.join("bob");
        std::fs::create_dir_all(&alice).unwrap();
        std::fs::create_dir_all(&bob).unwrap();
        write_date(&alice, "2026-09-11", vec![
            seed_item("2026-09-11", "a1", "alice公开", Some("共享"), "public", false),
            seed_item("2026-09-11", "a2", "alice私有", None, "private", false)]);
        write_date(&bob, "2026-09-10", vec![
            seed_item("2026-09-10", "b1", "bob公开", None, "public", true)]);
        // standalone 基目录自己的历史
        write_date(&base, "2026-09-09", vec![
            seed_item("2026-09-09", "m1", "我的历史", Some("共享"), "private", false)]);

        // solo：自己的全量 + 标签云 + 统计
        let (groups, _, _, tags, stats) = collect_explore(&base, false, None, &None, None, 14);
        let count: usize = groups.iter().map(|(_, items)| items.len()).sum();
        assert_eq!(count, 1, "solo 模式只收基目录自己的数据");
        assert!(tags.iter().any(|(n, _)| n == "共享"));
        assert_eq!(stats, (1, 0, 1));

        // social：bob 视角 → 只看到 alice 的 public，带 author，不含自己和私有
        let (groups, _, _, _, stats) = collect_explore(&base, true, Some("bob"), &None, None, 14);
        let flat: Vec<&(TodoItem, Option<String>)> = groups.iter().flat_map(|(_, items)| items.iter()).collect();
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].0.title, "alice公开");
        assert_eq!(flat[0].0.visibility, "public");
        assert_eq!(flat[0].1.as_deref(), Some("alice"));
        assert_eq!(stats, (1, 0, 1), "social 统计只针对他人公开条目");
    }

    // 生产布局回归：server 模式社交流扫描 users/<name>/todo/<date>.json
    // （旧实现只扫 admin 自己的 todo_dir，社交流恒为空 —— P1 事故）
    #[test]
    fn scan_rows_social_supports_users_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let users = tmp.path().join("users");
        let alice = users.join("alice").join("todo");
        let bob = users.join("bob").join("todo");
        std::fs::create_dir_all(&alice).unwrap();
        std::fs::create_dir_all(&bob).unwrap();
        write_date(&alice, "2026-09-30", vec![
            seed_item("2026-09-30", "a1", "alice公开", None, "public", false),
            seed_item("2026-09-30", "a2", "alice私有", None, "private", false)]);
        write_date(&bob, "2026-09-29", vec![
            seed_item("2026-09-29", "b1", "bob公开", None, "public", true)]);
        let rows = scan_rows(&users, true, Some("bob"));
        assert_eq!(rows.len(), 1, "只应看到 alice 的公开条目");
        assert_eq!(rows[0].1.title, "alice公开");
        assert_eq!(rows[0].2.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn move_item_between_dates_target_first() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        let dir = todo_dir_for_user(&ctx, None);
        write_date(&dir, "2026-09-10", vec![
            seed_item("2026-09-10", "m1", "挪走我", None, "private", false),
            seed_item("2026-09-10", "m2", "留下我", None, "private", false)]);

        // 正常迁移：字段更新 + move_to
        let r = todo_update_item_handler(
            State(ctx),
            None,
            AxumPath(("2026-09-10".to_string(), "m1".to_string())),
            Json(serde_json::from_value::<UpdateTodoItemRequest>(serde_json::json!(
                {"title": "挪走我(改名)", "move_to": "2026-09-11"})).unwrap()),
        ).await;
        assert_eq!(r.status(), axum::http::StatusCode::OK);
        let src = read_list_in(&dir, "2026-09-10").unwrap();
        assert_eq!(src.items.len(), 1, "迁移后源日期只留下未迁移项");
        assert_eq!(src.items[0].id, "m2");
        let dst = read_list_in(&dir, "2026-09-11").unwrap();
        assert_eq!(dst.items.len(), 1, "目标日期必须落盘");
        assert_eq!(dst.items[0].title, "挪走我(改名)", "字段更新随迁移生效");
        assert_eq!(dst.items[0].id, "m1", "id 保持不变（前端引用不失效）");

        // 非法目标日期 → 400，且源不动
        let tmp2 = tempfile::tempdir().unwrap();
        let ctx2 = test_ctx(tmp2.path());
        let dir2 = todo_dir_for_user(&ctx2, None);
        write_date(&dir2, "2026-09-10", vec![seed_item("2026-09-10", "x", "事项", None, "private", false)]);
        let r = todo_update_item_handler(
            State(ctx2.clone()),
            None,
            AxumPath(("2026-09-10".to_string(), "x".to_string())),
            Json(serde_json::from_value::<UpdateTodoItemRequest>(serde_json::json!(
                {"move_to": "../etc"})).unwrap()),
        ).await;
        assert_eq!(r.status(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(read_list_in(&dir2, "2026-09-10").unwrap().items.len(), 1);

        // move_to == 当前日期 → 视为普通更新，不迁移不丢失
        let r = todo_update_item_handler(
            State(ctx2),
            None,
            AxumPath(("2026-09-10".to_string(), "x".to_string())),
            Json(serde_json::from_value::<UpdateTodoItemRequest>(serde_json::json!(
                {"title": "就地改名", "move_to": "2026-09-10"})).unwrap()),
        ).await;
        assert_eq!(r.status(), axum::http::StatusCode::OK);
        assert_eq!(read_list_in(&dir2, "2026-09-10").unwrap().items[0].title, "就地改名");
    }

    #[tokio::test]
    async fn path_save_requires_admin_in_server_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ctx = test_ctx(tmp.path());
        ctx.auth = Some(crate::auth::AuthState::new(tmp.path().join("cfg")));

        // 普通用户改全局目录 → 403
        let r = todo_path_save_handler(
            State(ctx.clone()),
            Some(Extension(CurrentUser { username: "alice".into(), role: "user".into() })),
            Json(serde_json::json!({"path": "ToDoList"})),
        ).await;
        assert_eq!(r.status(), axum::http::StatusCode::FORBIDDEN);
        // 未登录（中间件缺失身份的兜底路径）→ 401
        let r = todo_path_save_handler(
            State(ctx.clone()),
            None,
            Json(serde_json::json!({"path": "ToDoList"})),
        ).await;
        assert_eq!(r.status(), axum::http::StatusCode::UNAUTHORIZED);
        // admin → 放行
        let r = todo_path_save_handler(
            State(ctx.clone()),
            Some(Extension(CurrentUser { username: "root".into(), role: "admin".into() })),
            Json(serde_json::json!({"path": "ToDoList"})),
        ).await;
        assert_eq!(r.status(), axum::http::StatusCode::OK);
    }
}

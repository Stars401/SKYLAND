//! 知识库 API 模块
//! 提供知识库 CRUD、文档管理、图片上传等功能
use axum::{
    Json, extract::{Extension, State, Path as AxumPath, Query},
    http::{HeaderMap, StatusCode},
    body::Bytes,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use super::AppContext;
use crate::auth::CurrentUser;
// ═══════════════════════════════════════════════════════════════
// 类型定义
// ═══════════════════════════════════════════════════════════════
#[derive(Serialize)]
pub struct KbInfo {
    pub name: String,
    pub doc_count: usize,
    pub created: u64,
    pub modified: u64,
}
#[derive(Serialize)]
pub struct FileTreeNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub children: Vec<FileTreeNode>,
    pub modified: u64,
    pub icon: String,
}
#[derive(Deserialize)]
pub struct CreateKbRequest {
    pub name: String,
}
#[derive(Deserialize)]
pub struct SaveDocRequest {
    pub path: String,
    pub content: String,
}
#[derive(Deserialize)]
pub struct CreateFolderRequest {
    pub path: String,
}
#[derive(Deserialize)]
pub struct DocPathQuery {
    pub path: String,
}
#[derive(Deserialize)]
pub struct RenameDocRequest {
    pub old_path: String,
    pub new_path: String,
}
#[derive(Deserialize)]
pub struct MoveDocRequest {
    pub path: String,
    pub target_dir: String,
}
#[derive(Deserialize)]
pub struct SetIconRequest {
    pub path: String,
    pub icon: String,
}
// ═══════════════════════════════════════════════════════════════
// 安全工具函数
// ═══════════════════════════════════════════════════════════════
/// 清理知识库名称 — 防止路径穿越
pub(crate) fn sanitize_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 200 { return None; }
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.starts_with('.') {
        return None;
    }
    Some(name.to_string())
}
/// 清理文档路径 — 防止路径穿越和 null 字节注入
pub(crate) fn sanitize_path(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() || path.len() > 500 { return None; }
    if path.starts_with('/') || path.starts_with('\\') || path.contains("..") || path.contains('\0') || path.starts_with("./") {
        return None;
    }
    Some(path.replace('\\', "/"))
}
/// 获取知识库目录
pub(crate) fn kb_root(ctx: &AppContext, user: Option<&CurrentUser>) -> PathBuf {
    crate::api::user_data_dir_for(ctx, user).join("knowledge")
}

pub(crate) fn kb_dir_for(ctx: &AppContext, user: Option<&CurrentUser>, name: &str) -> Option<PathBuf> {
    let name = sanitize_name(name)?;
    Some(kb_root(ctx, user).join(&name))
}

#[allow(dead_code)]
pub(crate) fn kb_dir(ctx: &AppContext, name: &str) -> Option<PathBuf> {
    kb_dir_for(ctx, None, name)
}
/// 系统时间转 Unix 时间戳
fn meta_created(meta: &std::fs::Metadata) -> u64 {
    meta.created()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
fn meta_modified(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
/// 递归统计 .md 文件数
pub(crate) fn count_docs(dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name == "images" { continue; }
                count += count_docs(&path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                count += 1;
            }
        }
    }
    count
}
/// 文档所有权 — agent 创建的文档改/删免审批；user（含无记录的遗留文档）改/删需审批
pub(crate) fn doc_owner(kb_dir: &Path, rel: &str) -> String {
    let p = kb_dir.join(".owner.json");
    std::fs::read_to_string(p).ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get(rel).and_then(|o| o.as_str()).map(String::from))
        .unwrap_or_else(|| "user".into())
}

/// 写入所有权记录（owner: "agent" | "user"）
pub(crate) fn set_doc_owner(kb_dir: &Path, rel: &str, owner: &str) {
    let p = kb_dir.join(".owner.json");
    let mut v = std::fs::read_to_string(&p).ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or(serde_json::json!({}));
    if let Some(t) = v.as_object_mut() {
        t.insert(rel.to_string(), serde_json::Value::String(owner.into()));
    }
    let _ = std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// 读取知识库的元数据文件 (.meta.json) — 存储文档图标等
fn load_meta(kb_dir: &Path) -> serde_json::Value {
    let meta_path = kb_dir.join(".meta.json");
    std::fs::read_to_string(&meta_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::json!({}))
}
/// 递归构建文件树 — 包含修改时间和图标
fn build_tree(dir: &Path, base: &str, meta: &serde_json::Value) -> Vec<FileTreeNode> {
    let mut nodes = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| (!e.path().is_dir(), e.file_name().to_string_lossy().to_string()));
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') { continue; }
            let rel_path = if base.is_empty() { name.clone() } else { format!("{}/{}", base, name) };
            if path.is_dir() {
                if name == "images" { continue; }
                let children = build_tree(&path, &rel_path, meta);
                let modified = entry.metadata().map(|m| meta_modified(&m)).unwrap_or(0);
                let icon = meta.get(&rel_path)
                    .and_then(|v| v.get("icon"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                nodes.push(FileTreeNode {
                    name,
                    path: rel_path,
                    is_dir: true,
                    children,
                    modified,
                    icon,
                });
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                let display_name = name.trim_end_matches(".md").to_string();
                let modified = entry.metadata().map(|m| meta_modified(&m)).unwrap_or(0);
                let icon = meta.get(&rel_path)
                    .and_then(|v| v.get("icon"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                nodes.push(FileTreeNode {
                    name: display_name,
                    path: rel_path,
                    is_dir: false,
                    children: vec![],
                    modified,
                    icon,
                });
            }
        }
    }
    nodes
}
/// 根据扩展名获取 Content-Type
fn content_type(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}
// ═══════════════════════════════════════════════════════════════
// API 处理函数
// ═══════════════════════════════════════════════════════════════
/// GET /api/kb/list — 列出所有知识库
pub async fn kb_list_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let mut kbs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(kb_root(&ctx, current_user)) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') { continue; }
            let meta = entry.metadata();
            let created = meta.as_ref().map(|m| meta_created(m)).unwrap_or(0);
            let modified = meta.as_ref().map(|m| meta_modified(m)).unwrap_or(0);
            let doc_count = count_docs(&path);
            kbs.push(KbInfo { name, doc_count, created, modified });
        }
    }
    kbs.sort_by(|a, b| b.modified.cmp(&a.modified));
    Json(serde_json::json!({ "kbs": kbs }))
}
/// POST /api/kb/create — 创建知识库
pub async fn kb_create_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<CreateKbRequest>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let name = match sanitize_name(&req.name) {
        Some(n) => n,
        None => return Json(serde_json::json!({ "status": "error", "message": "名称无效" })),
    };
    let dir = kb_root(&ctx, current_user).join(&name);
    if dir.exists() {
        return Json(serde_json::json!({ "status": "error", "message": "知识库已存在" }));
    }
    match std::fs::create_dir_all(&dir) {
        Ok(_) => {
            std::fs::create_dir_all(dir.join("images")).ok();
            Json(serde_json::json!({ "status": "ok", "message": "创建成功", "name": name }))
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("创建失败: {}", e) })),
    }
}
/// DELETE /api/kb/:name — 删除知识库
pub async fn kb_delete_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return Json(serde_json::json!({ "status": "error", "message": "名称无效" })),
    };
    if !dir.exists() {
        return Json(serde_json::json!({ "status": "error", "message": "知识库不存在" }));
    }
    match std::fs::remove_dir_all(&dir) {
        Ok(_) => Json(serde_json::json!({ "status": "ok" })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("删除失败: {}", e) })),
    }
}
/// GET /api/kb/:name/tree — 获取文档树
pub async fn kb_tree_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return Json(serde_json::json!({ "tree": [] })),
    };
    if !dir.exists() {
        return Json(serde_json::json!({ "tree": [] }));
    }
    let tree = build_tree(&dir, "", &load_meta(&dir));
    Json(serde_json::json!({ "tree": tree }))
}
/// GET /api/kb/:name/doc?path=xxx — 获取文档内容
pub async fn kb_doc_get_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    Query(q): Query<DocPathQuery>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return Json(serde_json::json!({ "status": "error", "message": "知识库无效" })),
    };
    let path = match sanitize_path(&q.path) {
        Some(p) => p,
        None => return Json(serde_json::json!({ "status": "error", "message": "路径无效" })),
    };
    let file_path = dir.join(&path);
    // 安全检查：确保解析后的路径在知识库目录内
    if !file_path.starts_with(&dir) || !file_path.exists() {
        return Json(serde_json::json!({ "status": "error", "message": "文件不存在" }));
    }
    match std::fs::read_to_string(&file_path) {
        Ok(content) => {
            let modified = std::fs::metadata(&file_path)
                .map(|m| meta_modified(&m))
                .unwrap_or(0);
            Json(serde_json::json!({ "status": "ok", "content": content, "path": path, "modified": modified }))
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("读取失败: {}", e) })),
    }
}
/// POST /api/kb/:name/doc — 保存/创建文档
pub async fn kb_doc_save_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<SaveDocRequest>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return Json(serde_json::json!({ "status": "error", "message": "知识库无效" })),
    };
    let path = match sanitize_path(&req.path) {
        Some(p) => p,
        None => return Json(serde_json::json!({ "status": "error", "message": "路径无效" })),
    };
    // 确保以 .md 结尾
    let path = if path.ends_with(".md") { path } else { format!("{}.md", path) };
    let file_path = dir.join(&path);
    // 安全检查
    if !file_path.starts_with(&dir) {
        return Json(serde_json::json!({ "status": "error", "message": "路径越界" }));
    }
    // 创建父目录
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    // 版本历史：如果文件已存在，备份到 .history/ 目录
    if file_path.exists() {
        let history_dir = dir.join(".history");
        std::fs::create_dir_all(&history_dir).ok();
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let safe_name = path.replace('/', "_");
        let bak_name = format!("{}.{}.bak", safe_name, ts);
        let bak_path = history_dir.join(&bak_name);
        let _ = std::fs::copy(&file_path, &bak_path);
        // 最多保留 20 个历史版本
        if let Ok(mut baks) = std::fs::read_dir(&history_dir).map(|e| e.flatten().collect::<Vec<_>>()) {
            baks.sort_by_key(|e| e.file_name().to_string_lossy().to_string());
            let prefix = format!("{}.", safe_name);
            let matching: Vec<_> = baks.iter().filter(|e| e.file_name().to_string_lossy().starts_with(&prefix)).collect();
            if matching.len() > 20 {
                for old in matching.iter().take(matching.len() - 20) {
                    let _ = std::fs::remove_file(old.path());
                }
            }
        }
    }
    match std::fs::write(&file_path, &req.content) {
        Ok(_) => Json(serde_json::json!({ "status": "ok", "path": path })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("保存失败: {}", e) })),
    }
}
/// DELETE /api/kb/:name/doc?path=xxx — 删除文档
pub async fn kb_doc_delete_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    Query(q): Query<DocPathQuery>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return Json(serde_json::json!({ "status": "error", "message": "知识库无效" })),
    };
    let path = match sanitize_path(&q.path) {
        Some(p) => p,
        None => return Json(serde_json::json!({ "status": "error", "message": "路径无效" })),
    };
    let file_path = dir.join(&path);
    if !file_path.starts_with(&dir) || !file_path.exists() {
        return Json(serde_json::json!({ "status": "error", "message": "文件不存在" }));
    }
    match std::fs::remove_file(&file_path) {
        Ok(_) => Json(serde_json::json!({ "status": "ok" })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("删除失败: {}", e) })),
    }
}
/// POST /api/kb/:name/folder — 创建文件夹
pub async fn kb_folder_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<CreateFolderRequest>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return Json(serde_json::json!({ "status": "error", "message": "知识库无效" })),
    };
    let path = match sanitize_path(&req.path) {
        Some(p) => p,
        None => return Json(serde_json::json!({ "status": "error", "message": "路径无效" })),
    };
    let folder_path = dir.join(&path);
    if !folder_path.starts_with(&dir) {
        return Json(serde_json::json!({ "status": "error", "message": "路径越界" }));
    }
    match std::fs::create_dir_all(&folder_path) {
        Ok(_) => Json(serde_json::json!({ "status": "ok", "path": path })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("创建失败: {}", e) })),
    }
}
/// POST /api/kb/:name/rename — 重命名文档/文件夹
pub async fn kb_rename_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<RenameDocRequest>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return Json(serde_json::json!({ "status": "error", "message": "知识库无效" })),
    };
    let old_path = match sanitize_path(&req.old_path) {
        Some(p) => p,
        None => return Json(serde_json::json!({ "status": "error", "message": "旧路径无效" })),
    };
    let new_path = match sanitize_path(&req.new_path) {
        Some(p) => p,
        None => return Json(serde_json::json!({ "status": "error", "message": "新路径无效" })),
    };
    let old_file = dir.join(&old_path);
    let new_file = dir.join(&new_path);
    if !old_file.starts_with(&dir) || !new_file.starts_with(&dir) {
        return Json(serde_json::json!({ "status": "error", "message": "路径越界" }));
    }
    if !old_file.exists() {
        return Json(serde_json::json!({ "status": "error", "message": "源文件不存在" }));
    }
    if new_file.exists() {
        return Json(serde_json::json!({ "status": "error", "message": "目标路径已存在" }));
    }
    if let Some(parent) = new_file.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    match std::fs::rename(&old_file, &new_file) {
        Ok(_) => Json(serde_json::json!({ "status": "ok", "new_path": new_path })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("重命名失败: {}", e) })),
    }
}
/// POST /api/kb/:name/image — 上传图片（raw bytes + X-Filename header）
pub async fn kb_image_upload_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    user: Option<Extension<CurrentUser>>,
    headers: HeaderMap,
    body: Bytes,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return Json(serde_json::json!({ "status": "error", "message": "知识库无效" })),
    };
    let img_dir = dir.join("images");
    std::fs::create_dir_all(&img_dir).ok();
    // 从 header 获取文件名
    let filename = headers.get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            format!("image_{}.png", ts)
        });
    // 清理文件名 — 只保留字母数字和点号
    let safe_name: String = filename.chars().map(|c| {
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' }
    }).collect();
    let safe_name = if safe_name.is_empty() {
        format!("image_{}.png", SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0))
    } else {
        safe_name
    };
    let file_path = img_dir.join(&safe_name);
    match std::fs::write(&file_path, &body) {
        Ok(_) => {
            let url = format!("/api/kb/{}/image/{}", name, safe_name);
            Json(serde_json::json!({ "status": "ok", "url": url, "filename": safe_name }))
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("上传失败: {}", e) })),
    }
}
/// GET /api/kb/:name/history?path=xxx — 获取文档版本历史
pub async fn kb_doc_history_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    Query(q): Query<DocPathQuery>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return Json(serde_json::json!({ "history": [] })),
    };
    let path = match sanitize_path(&q.path) {
        Some(p) => p,
        None => return Json(serde_json::json!({ "history": [] })),
    };
    let history_dir = dir.join(".history");
    let safe_name = path.replace('/', "_");
    let prefix = format!("{}.", safe_name);
    let mut history = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&history_dir) {
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name().to_string_lossy().to_string());
        entries.reverse();
        for entry in entries {
            let fname = entry.file_name().to_string_lossy().to_string();
            if !fname.starts_with(&prefix) || !fname.ends_with(".bak") { continue; }
            // 提取时间戳
            let ts_str = fname.trim_start_matches(&prefix).trim_end_matches(".bak");
            let ts: u64 = ts_str.parse().unwrap_or(0);
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            history.push(serde_json::json!({
                "filename": fname,
                "timestamp": ts,
                "size": size,
                "path": path,
            }));
        }
    }
    Json(serde_json::json!({ "history": history }))
}
/// POST /api/kb/:name/history/restore — 恢复历史版本
#[derive(Deserialize)]
pub struct RestoreHistoryRequest {
    pub path: String,
    pub filename: String,
}
pub async fn kb_doc_restore_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<RestoreHistoryRequest>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return Json(serde_json::json!({ "status": "error", "message": "知识库无效" })),
    };
    let path = match sanitize_path(&req.path) {
        Some(p) => p,
        None => return Json(serde_json::json!({ "status": "error", "message": "路径无效" })),
    };
    // 清理 filename 防止路径穿越
    let filename: String = req.filename.chars().map(|c| {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' { c } else { '_' }
    }).collect();
    let bak_path = dir.join(".history").join(&filename);
    let file_path = dir.join(&path);
    if !bak_path.starts_with(&dir) || !bak_path.exists() {
        return Json(serde_json::json!({ "status": "error", "message": "历史版本不存在" }));
    }
    // 先备份当前版本
    if file_path.exists() {
        let history_dir = dir.join(".history");
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let safe_name = path.replace('/', "_");
        let cur_bak = history_dir.join(format!("{}.{}.bak", safe_name, ts));
        let _ = std::fs::copy(&file_path, &cur_bak);
    }
    match std::fs::copy(&bak_path, &file_path) {
        Ok(_) => {
            let content = std::fs::read_to_string(&file_path).unwrap_or_default();
            Json(serde_json::json!({ "status": "ok", "content": content }))
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("恢复失败: {}", e) })),
    }
}
/// POST /api/kb/:name/move — 移动文档到新位置（拖拽排序）
pub async fn kb_doc_move_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<MoveDocRequest>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return Json(serde_json::json!({ "status": "error", "message": "知识库无效" })),
    };
    let src_path = match sanitize_path(&req.path) {
        Some(p) => p,
        None => return Json(serde_json::json!({ "status": "error", "message": "源路径无效" })),
    };
    let target_dir = match sanitize_path(&req.target_dir) {
        Some(p) => p,
        None => return Json(serde_json::json!({ "status": "error", "message": "目标路径无效" })),
    };
    let src_file = dir.join(&src_path);
    let filename = src_file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let dest_file = if target_dir.is_empty() {
        dir.join(&filename)
    } else {
        dir.join(&target_dir).join(&filename)
    };
    if !src_file.starts_with(&dir) || !dest_file.starts_with(&dir) {
        return Json(serde_json::json!({ "status": "error", "message": "路径越界" }));
    }
    if !src_file.exists() {
        return Json(serde_json::json!({ "status": "error", "message": "源文件不存在" }));
    }
    if dest_file.exists() {
        return Json(serde_json::json!({ "status": "error", "message": "目标位置已有同名文件" }));
    }
    if let Some(parent) = dest_file.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let new_path = if target_dir.is_empty() { filename } else { format!("{}/{}", target_dir, filename) };
    match std::fs::rename(&src_file, &dest_file) {
        Ok(_) => Json(serde_json::json!({ "status": "ok", "new_path": new_path })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("移动失败: {}", e) })),
    }
}
/// POST /api/kb/:name/icon — 设置文档图标
pub async fn kb_doc_icon_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<SetIconRequest>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return Json(serde_json::json!({ "status": "error", "message": "知识库无效" })),
    };
    let path = match sanitize_path(&req.path) {
        Some(p) => p,
        None => return Json(serde_json::json!({ "status": "error", "message": "路径无效" })),
    };
    let meta_path = dir.join(".meta.json");
    let mut meta = load_meta(&dir);
    if let Some(obj) = meta.as_object_mut() {
        obj.insert(path.clone(), serde_json::json!({ "icon": req.icon }));
    } else {
        meta = serde_json::json!({ path: { "icon": req.icon } });
    }
    match serde_json::to_string_pretty(&meta) {
        Ok(s) => {
            match std::fs::write(&meta_path, s) {
                Ok(_) => Json(serde_json::json!({ "status": "ok" })),
                Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("保存图标失败: {}", e) })),
            }
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("序列化失败: {}", e) })),
    }
}
/// GET /api/kb/:name/image/:img — 提供图片
pub async fn kb_image_serve_handler(
    State(ctx): State<AppContext>,
    AxumPath((name, img)): AxumPath<(String, String)>,
    user: Option<Extension<CurrentUser>>,
) -> axum::response::Response {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) {
        Some(d) => d,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    // img is already URL-decoded by axum (e.g., "image 3.png")
    // Search for the file in images/ directory by case-insensitive match
    let img_lower = img.to_lowercase();
    let img_dir = dir.join("images");
    // Try exact match in images/
    let mut file_path = img_dir.join(&img);
    // If not found, try case-insensitive directory listing
    if !file_path.exists() {
        let found = std::fs::read_dir(&img_dir).ok()
            .and_then(|entries| {
                entries.flatten().find(|e| {
                    e.file_name().to_string_lossy().to_lowercase() == img_lower
                }).map(|e| e.path())
            });
        if let Some(p) = found {
            file_path = p;
        } else {
            // Try searching in subdirectories (e.g., 图片和附件/)
            let found_sub = std::fs::read_dir(&dir).ok()
                .and_then(|entries| {
                    entries.flatten().filter(|e| e.path().is_dir()).find_map(|e| {
                        let subdir = e.path();
                        let candidate = subdir.join(&img);
                        if candidate.exists() { return Some(candidate); }
                        // Also case-insensitive in subdirectory
                        std::fs::read_dir(&subdir).ok()
                            .and_then(|sub_entries| {
                                sub_entries.flatten().find(|se| {
                                    se.file_name().to_string_lossy().to_lowercase() == img_lower
                                }).map(|se| se.path())
                            })
                    })
                });
            if let Some(p) = found_sub {
                file_path = p;
            }
        }
    }
    // Safety check: ensure the resolved path is within the KB directory
    if !file_path.starts_with(&dir) || !file_path.exists() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let ct = content_type(ext);
    match std::fs::read(&file_path) {
        Ok(data) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, ct)],
            data,
        ).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
// ═══════════════════════════════════════════════════════════════
// 对话历史 API — 存储到 UserData/.conversations/
// ═══════════════════════════════════════════════════════════════
#[derive(Serialize)]
pub struct ConversationInfo {
    pub id: u64,
    pub title: String,
    pub messages: usize,
    pub created: u64,
    pub modified: u64,
}
#[derive(Deserialize)]
pub struct SaveConversationRequest {
    pub id: u64,
    pub title: String,
    pub messages: serde_json::Value,
    /// 会话面板状态等附加字段（_ctx/_plan/_round/_outcome…）— 透传持久化，
    /// 工作区会话跨设备重开时右栏状态保真
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}
/// 工作区感知的会话目录：工作区激活 → {工作区}/.skyland/conversations
/// （对齐 Claude Code .claude — 工作区自带会话数据；.skyland/cache 一并建立供后续缓存留存）
fn conversations_dir_for(ctx: &AppContext, user: Option<&CurrentUser>, workdir: Option<&str>) -> PathBuf {
    if let Some(ws) = workdir.map(str::trim).filter(|w| !w.is_empty()) {
        if let Some(dir) = crate::filegen::resolve_session_workdir_for(ctx, user, Some(ws), None) {
            let skyland = dir.join(".skyland");
            let _ = std::fs::create_dir_all(skyland.join("conversations"));
            let _ = std::fs::create_dir_all(skyland.join("cache"));
            return skyland.join("conversations");
        }
    }
    let dir = crate::api::user_data_dir_for(ctx, user).join("conversations");
    std::fs::create_dir_all(&dir).ok();
    dir
}

#[allow(dead_code)]
fn conversations_dir_in(ctx: &AppContext, workdir: Option<&str>) -> PathBuf {
    conversations_dir_for(ctx, None, workdir)
}
type WsQuery = axum::extract::Query<std::collections::HashMap<String, String>>;
/// GET /api/conversations/list — 列出所有对话（?workdir= 工作区会话）
pub async fn conversations_list_handler(
    State(ctx): State<AppContext>,
    axum::extract::Query(params): WsQuery,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = conversations_dir_for(&ctx, current_user, params.get("workdir").map(|s| s.as_str()));
    let mut convs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") { continue; }
            let fname = entry.file_name().to_string_lossy().to_string();
            let id: u64 = fname.trim_end_matches(".json").parse().unwrap_or(0);
            if id == 0 { continue; }
            let meta = entry.metadata();
            let created = meta.as_ref().map(|m| meta_created(m)).unwrap_or(0);
            let modified = meta.as_ref().map(|m| meta_modified(m)).unwrap_or(0);
            // Read title and message count from file
            let (title, msg_count) = if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    let t = v.get("title").and_then(|v| v.as_str()).unwrap_or("对话").to_string();
                    let c = v.get("messages").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                    (t, c)
                } else { ("对话".into(), 0) }
            } else { ("对话".into(), 0) };
            convs.push(ConversationInfo { id, title, messages: msg_count, created, modified });
        }
    }
    convs.sort_by(|a, b| b.modified.cmp(&a.modified));
    Json(serde_json::json!({ "conversations": convs }))
}
/// GET /api/conversations/:id — 获取对话内容
pub async fn conversations_get_handler(
    State(ctx): State<AppContext>,
    axum::extract::Query(params): WsQuery,
    AxumPath(id): AxumPath<u64>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = conversations_dir_for(&ctx, current_user, params.get("workdir").map(|s| s.as_str()));
    let file_path = dir.join(format!("{}.json", id));
    if !file_path.exists() {
        return Json(serde_json::json!({ "status": "error", "message": "对话不存在" }));
    }
    match std::fs::read_to_string(&file_path) {
        Ok(content) => {
            let data: serde_json::Value = serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
            Json(serde_json::json!({ "status": "ok", "data": data }))
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("读取失败: {}", e) })),
    }
}
/// POST /api/conversations/save — 保存对话（?workdir= 工作区会话）
pub async fn conversations_save_handler(
    State(ctx): State<AppContext>,
    axum::extract::Query(params): WsQuery,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<SaveConversationRequest>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = conversations_dir_for(&ctx, current_user, params.get("workdir").map(|s| s.as_str()));
    let file_path = dir.join(format!("{}.json", req.id));
    let mut data = serde_json::json!({
        "id": req.id,
        "title": req.title,
        "messages": req.messages,
        "saved_at": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    });
    // 附加字段（_ctx/_plan 等）透传 — 不与核心字段冲突
    for (k, v) in &req.extra {
        if !matches!(k.as_str(), "id" | "title" | "messages" | "saved_at") {
            data[k.as_str()] = v.clone();
        }
    }
    match serde_json::to_string_pretty(&data) {
        Ok(s) => {
            match std::fs::write(&file_path, s) {
                Ok(_) => Json(serde_json::json!({ "status": "ok" })),
                Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("保存失败: {}", e) })),
            }
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("序列化失败: {}", e) })),
    }
}
/// DELETE /api/conversations/:id — 删除对话
pub async fn conversations_delete_handler(
    State(ctx): State<AppContext>,
    axum::extract::Query(params): WsQuery,
    AxumPath(id): AxumPath<u64>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = conversations_dir_for(&ctx, current_user, params.get("workdir").map(|s| s.as_str()));
    let file_path = dir.join(format!("{}.json", id));
    if !file_path.exists() {
        return Json(serde_json::json!({ "status": "error", "message": "对话不存在" }));
    }
    match std::fs::remove_file(&file_path) {
        Ok(_) => Json(serde_json::json!({ "status": "ok" })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("删除对话失败: {}", e) })),
    }
}
// ═══════════════════════════════════════════════════════════════
// 知识库全文搜索
// ═══════════════════════════════════════════════════════════════
/// UTF-8 safe slice - clamp byte offsets to char boundaries
pub(crate) fn safe_slice(s: &str, start: usize, end: usize) -> &str {
    let mut b = start.min(s.len());
    let mut e = end.min(s.len());
    if b > e { b = e; }
    while b > 0 && !s.is_char_boundary(b) { b -= 1; }
    while e > b && !s.is_char_boundary(e) { e -= 1; }
    &s[b..e]
}

#[derive(Serialize)]
pub(crate) struct SearchResult {
    pub path: String,
    pub name: String,
    pub snippet: String,
    pub matches: usize,
    pub before: String,
    pub match_text: String,
    pub after: String,
}
pub(crate) fn search_in_dir(dir: &Path, base: &str, query: &str, results: &mut Vec<SearchResult>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "images" { continue; }
            let rel_path = if base.is_empty() { name.clone() } else { format!("{}/{}", base, name) };
            if path.is_dir() {
                search_in_dir(&path, &rel_path, query, results);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let lower = content.to_lowercase();
                    let lq = query.to_lowercase();
                    let count = lower.matches(&lq).count();
                    if count > 0 {
                        let idx = lower.find(&lq).unwrap_or(0);
                        let start = idx.saturating_sub(40);
                        let end = (idx + query.len() + 60).min(content.len());
                        let snippet = format!("...{}...", safe_slice(&content, start, end).replace('\n', " "));
                        let (before, match_text, after) = split_around(&content, idx, query.len());
                        results.push(SearchResult { path: rel_path, name: name.trim_end_matches(".md").to_string(), snippet, before, match_text, after, matches: count });
                    }
                }
            }
        }
    }
}
/// Split content around the first match into (before, match, after), clipped to ~60 chars on each side,
/// with newlines flattened to spaces so the snippet renders cleanly in a single-line preview.
fn split_around(content: &str, idx: usize, q_len: usize) -> (String, String, String) {
    let start = idx.saturating_sub(60);
    let end = (idx + q_len + 60).min(content.len());
    let before = safe_slice(content, start, idx).replace('\n', " ");
    let m = safe_slice(content, idx, idx + q_len).replace('\n', " ");
    let after = safe_slice(content, idx + q_len, end).replace('\n', " ");
    (before, m, after)
}
/// GET /api/kb/:name/search?path=xxx
pub async fn kb_search_handler(
    State(ctx): State<AppContext>,
    AxumPath(name): AxumPath<String>,
    Query(q): Query<DocPathQuery>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = match kb_dir_for(&ctx, current_user, &name) { Some(d) => d, None => return Json(serde_json::json!({ "results": [] })) };
    let query = q.path.trim();
    if query.is_empty() { return Json(serde_json::json!({ "results": [] })); }
    let mut results = Vec::new();
    search_in_dir(&dir, "", query, &mut results);
    results.sort_by(|a, b| b.matches.cmp(&a.matches));
    Json(serde_json::json!({ "results": results }))
}
// ═══════════════════════════════════════════════════════════════
// 对话导入/导出
// ═══════════════════════════════════════════════════════════════
#[derive(Deserialize)]
pub struct ImportConversationsRequest {
    pub conversations: serde_json::Value,
}
/// GET /api/conversations/export
pub async fn conversations_export_handler(
    State(ctx): State<AppContext>,
    axum::extract::Query(params): WsQuery,
    user: Option<Extension<CurrentUser>>,
) -> impl IntoResponse {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = conversations_dir_for(&ctx, current_user, params.get("workdir").map(|s| s.as_str()));
    let mut all = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") { continue; }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) { all.push(v); }
            }
        }
    }
    let export = serde_json::json!({ "app": "SKYLAND", "version": "1.0", "exported_at": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0), "conversations": all });
    (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "application/json")], serde_json::to_string_pretty(&export).unwrap_or_default()).into_response()
}
/// POST /api/conversations/import
pub async fn conversations_import_handler(
    State(ctx): State<AppContext>,
    axum::extract::Query(params): WsQuery,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<ImportConversationsRequest>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = conversations_dir_for(&ctx, current_user, params.get("workdir").map(|s| s.as_str()));
    let convs = match req.conversations.as_array() { Some(a) => a, None => return Json(serde_json::json!({ "status": "error", "message": "格式无效" })) };
    let mut imported = 0;
    let mut skipped = 0;
    for conv in convs {
        let id = match conv.get("id").and_then(|v| v.as_u64()) { Some(i) => i, None => { skipped += 1; continue; } };
        let title = conv.get("title").and_then(|v| v.as_str()).unwrap_or("导入的对话").to_string();
        let messages = conv.get("messages").cloned().unwrap_or(serde_json::json!([]));
        let data = serde_json::json!({ "id": id, "title": title, "messages": messages, "saved_at": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) });
        let file_path = dir.join(format!("{}.json", id));
        if let Ok(s) = serde_json::to_string_pretty(&data) {
            if std::fs::write(&file_path, s).is_ok() { imported += 1; } else { skipped += 1; }
        } else { skipped += 1; }
    }
    Json(serde_json::json!({ "status": "ok", "imported": imported, "skipped": skipped }))
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn test_search_chinese_doc_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        let doc = dir.path().join("doc.md");
        let padding = "这是一段很长的中文铺垫内容。".repeat(10);
        std::fs::write(&doc, format!("{}\n水面制作的关键是翻滚修改。", padding)).unwrap();
        let mut results = Vec::new();
        search_in_dir(dir.path(), "", "水面", &mut results);
        assert_eq!(results.len(), 1);
        assert!(results[0].snippet.contains("水面"));
        let long = format!("{}水面测试", padding);
        let (before, m, _after) = split_around(&long, padding.len(), "水面测试".len());
        assert!(!before.is_empty() && m == "水面测试");
    }

    #[test]
    fn user_kb_and_conversation_paths_are_isolated() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ctx = AppContext {
            providers: std::collections::HashMap::new(),
            default_model: String::new(),
            user_data_dir: tmp.path().join("users").join("local"),
            work_dir: tmp.path().to_path_buf(),
            context_lengths: std::collections::HashMap::new(),
            image_dir: std::sync::Arc::new(std::sync::RwLock::new(tmp.path().to_path_buf())),
            todo_dir: std::sync::Arc::new(std::sync::RwLock::new(tmp.path().join("todo"))),
            auth: None,
            server_mode: "server".into(),
            cloud_workdir: tmp.path().join("CloudWork"),
        };
        ctx.auth = Some(crate::auth::AuthState::new(tmp.path().join("auth")));
        let alice = CurrentUser { username: "alice".into(), role: "user".into() };
        let bob = CurrentUser { username: "bob".into(), role: "user".into() };
        assert_eq!(kb_root(&ctx, Some(&alice)), tmp.path().join("users/alice/knowledge"));
        assert_eq!(kb_dir_for(&ctx, Some(&alice), "docs").unwrap(), tmp.path().join("users/alice/knowledge/docs"));
        assert_eq!(conversations_dir_for(&ctx, Some(&alice), None), tmp.path().join("users/alice/conversations"));
        assert_eq!(kb_root(&ctx, Some(&bob)), tmp.path().join("users/bob/knowledge"));
    }
}

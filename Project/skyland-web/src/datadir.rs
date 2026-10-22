//! 数据目录体系 — Windows 规范位置定向（配置/数据不得写入程序所在工作目录）
//!
//! 解析优先级：
//! 1. 指针文件（`{SKYLAND根}/data-dir.json` 或旧版 `{Skyland根}/data-dir.json`）的自选位置
//! 2. 旧版数据已存在于默认根或 Skyland 根（老用户零迁移，直接采用）
//! 3. 默认根 `%APPDATA%/SKYLAND`（未配置 — 前端首页首启弹窗引导用户确认或自选路径）
//!
//! 品牌更名：数据根 Skyland → SKYLAND；旧版 Skyland 目录有数据时自动采用（防孤儿化）。
//! 未配置时这里不创建任何目录：数据目录的选择权属于首启弹窗。

use std::path::{Path, PathBuf};

/// 固定根目录（指针文件必须永远可寻 — 不随自选路径移动）
pub fn default_root() -> PathBuf {
    if let Some(appdata) = std::env::var("APPDATA").ok().filter(|s| !s.is_empty()) {
        return PathBuf::from(appdata).join("SKYLAND");
    }
    if let Some(xdg) = std::env::var("XDG_DATA_HOME").ok().filter(|s| !s.is_empty()) {
        return PathBuf::from(xdg).join("SKYLAND");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local").join("share").join("SKYLAND")
}

/// 旧版数据根（品牌更名前 %APPDATA%/Skyland）— 有数据时自动采用，防孤儿化
fn legacy_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(appdata) = std::env::var("APPDATA").ok().filter(|s| !s.is_empty()) {
        out.push(PathBuf::from(appdata).join("Skyland"));
    }
    if let Some(xdg) = std::env::var("XDG_DATA_HOME").ok().filter(|s| !s.is_empty()) {
        out.push(PathBuf::from(xdg).join("Skyland"));
    }
    out
}

fn has_legacy_data(dir: &Path) -> bool {
    ["UserData", "ToDoList", "config"].iter().any(|d| dir.join(d).exists())
}

fn pointer_path() -> PathBuf {
    default_root().join("data-dir.json")
}

#[derive(Clone, Debug)]
pub struct AppDataLayout {
    root: PathBuf,
    configured: bool,
}

impl AppDataLayout {
    pub fn resolve() -> Self {
        let (root, configured) = resolve_legacy_or_default();
        Self { root, configured }
    }

    pub fn from_root_for_test(root: PathBuf, configured: bool) -> Self {
        Self { root, configured }
    }

    pub fn from_context(ctx: &crate::api::AppContext) -> Self {
        let path = &ctx.user_data_dir;
        let inferred = path
            .parent()
            .filter(|parent| parent.file_name().map(|name| name == "users").unwrap_or(false))
            .and_then(Path::parent);
        match inferred {
            Some(root) => Self::from_root_for_test(root.to_path_buf(), true),
            None => Self::resolve(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }

    pub fn auth_dir(&self) -> PathBuf {
        self.root.join("auth")
    }

    pub fn user_dir(&self, username: &str) -> Result<PathBuf, String> {
        let invalid = username.is_empty()
            || username == "."
            || username == ".."
            || username
                .chars()
                .any(|ch| ch == '/' || ch == '\\' || ch == '\0');
        if invalid {
            return Err("非法用户名".into());
        }
        Ok(self.root.join("users").join(username))
    }

    pub fn ensure_structure(&self) -> Result<(), String> {
        for sub in ["config", "auth", "users", "logs"] {
            std::fs::create_dir_all(self.root.join(sub))
                .map_err(|e| format!("无法创建 {}: {}", sub, e))?;
        }
        Ok(())
    }

    pub fn is_configured(&self) -> bool {
        self.configured
    }
}

fn resolve_legacy_or_default() -> (PathBuf, bool) {
    if let Ok(env_dir) = std::env::var("SKYLAND_DATA_DIR") {
        let d = env_dir.trim();
        if !d.is_empty() {
            return (PathBuf::from(d), true);
        }
    }
    if let Ok(env_dir) = std::env::var("MOLINE_DATA_DIR") {
        let d = env_dir.trim();
        if !d.is_empty() {
            return (PathBuf::from(d), true);
        }
    }
    // 指针：新位置（SKYLAND 根）优先，旧位置（Skyland 根，v=118 短暂存在过）兼容
    for pp in [pointer_path(), default_root().join("../Skyland/data-dir.json")] {
        if let Ok(content) = std::fs::read_to_string(&pp) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(dir) = v.get("dir").and_then(|d| d.as_str()).map(str::trim)
                    .filter(|d| !d.is_empty()) {
                    return (PathBuf::from(dir), true);
                }
            }
        }
    }
    let root = default_root();
    // 旧版桌面用户：默认根（SKYLAND）或 Skyland 根已有数据/配置 → 直接采用（零迁移）
    if has_legacy_data(&root) {
        return (root, true);
    }
    for old in legacy_roots() {
        if has_legacy_data(&old) {
            return (old, true);
        }
    }
    (root, false)
}

/// 解析 (数据目录, 是否已配置)。已配置 = 指针存在 或 旧版数据已在默认根/Skyland 根。
/// 未配置时返回默认根但不落盘 — 等首启弹窗决定。
/// 测试/嵌入隔离：`SKYLAND_DATA_DIR` 环境变量强制定向（优先于指针文件，
/// 与 SKYLAND_CONFIG_PATH 同款模式 — 集成测试用它把数据目录钉在临时目录）。
/// 两个变量都支持读取，历史名 MOLINE_DATA_DIR 保留为兼容兜底。
pub fn resolve() -> (PathBuf, bool) {
    let layout = AppDataLayout::resolve();
    (layout.root, layout.configured)
}

/// 规范结构 — 数据目录建立时的固定骨架
pub fn ensure_structure(dir: &Path) -> Result<(), String> {
    AppDataLayout::from_root_for_test(dir.to_path_buf(), true).ensure_structure()
}

/// 设置数据目录：建规范结构 + 写指针（各路径启动时烘焙 — 重启后生效）
pub fn set(dir: &str) -> Result<PathBuf, String> {
    let p = Path::new(dir.trim());
    if p.as_os_str().is_empty() {
        return Err("数据目录路径为空".into());
    }
    if !p.is_absolute() {
        return Err("请提供绝对路径（例如 D:\\SkylandData）".into());
    }
    ensure_structure(p)?;
    let pp = pointer_path();
    if let Some(parent) = pp.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建指针目录: {}", e))?;
    }
    let content = serde_json::json!({ "dir": p.display().to_string() }).to_string();
    std::fs::write(&pp, content).map_err(|e| format!("写入指针失败: {}", e))?;
    Ok(p.to_path_buf())
}

/// 存量迁移：旧版把数据落在程序目录（work_dir）— 搬入/合并到 data_dir。
/// 策略（数据安全第一 — 永不覆盖任何既有文件）：
/// - 目标目录不存在 → 整目录 rename（同盘）或复制+删除（跨盘）
/// - 目标已存在 → 递归合并：仅复制目标缺失的文件（旧位置保留，供用户确认后手动清理）
pub fn migrate_legacy(work_dir: &Path, data_dir: &Path) {
    for sub in ["UserData", "ToDoList", "ImgGenerator", "CloudWork", "logs"] {
        let src = work_dir.join(sub);
        let dst = data_dir.join(sub);
        if !src.is_dir() { continue; }
        if !dst.exists() {
            if std::fs::rename(&src, &dst).is_ok() { continue; }
            if copy_dir_recursive(&src, &dst).is_ok() {
                let _ = std::fs::remove_dir_all(&src);
            }
        } else {
            // 目标已有数据（如桌面端与仓库各自产生过数据）→ 按文件合并缺失项
            let _ = merge_missing(&src, &dst);
        }
    }
}

/// 将旧程序目录数据迁移到 Skyland layout。
/// 只复制目标缺失项，不覆盖目标文件，也不删除旧源目录。
pub fn migrate_legacy_to_layout(
    work_dir: &Path,
    layout: &AppDataLayout,
    username: Option<&str>,
) -> Result<(), String> {
    let root = layout.root().to_path_buf();
    let config_src = work_dir.join("config");
    if config_src.is_dir() {
        let app_src = config_src.join("app.toml");
        if app_src.is_file() {
            merge_file_if_missing(&app_src, &root.join("app.toml"))?;
        }
        for entry in std::fs::read_dir(&config_src).map_err(|e| e.to_string())?.flatten() {
            let name = entry.file_name();
            if name == "app.toml" {
                continue;
            }
            let src = entry.path();
            let dst = if matches!(name.to_string_lossy().as_ref(),
                "users.json" | "sessions.json" | "whitelist.json" | "auth_audit.jsonl") {
                root.join("auth").join(&name)
            } else {
                root.join("config").join(&name)
            };
            if src.is_dir() {
                merge_tree_if_missing(&src, &dst)?;
            } else if src.is_file() {
                merge_file_if_missing(&src, &dst)?;
            }
        }
    }
    let logs_src = work_dir.join("logs");
    if logs_src.is_dir() {
        merge_tree_if_missing(&logs_src, &root.join("logs"))?;
    }

    let Some(username) = username else { return Ok(()); };
    let user_root = layout.user_dir(username)?;
    let user_data_src = work_dir.join("UserData");
    if user_data_src.is_dir() {
        for entry in std::fs::read_dir(&user_data_src).map_err(|e| e.to_string())?.flatten() {
            let name = entry.file_name();
            let src = entry.path();
            if !src.is_dir() {
                continue;
            }
            let dst = match name.to_string_lossy().as_ref() {
                ".conversations" => user_root.join("conversations"),
                "agents" => user_root.join("agents"),
                "skills" => user_root.join("skills"),
                _ => user_root.join("knowledge").join(&name),
            };
            merge_tree_if_missing(&src, &dst)?;
        }
    }
    merge_tree_if_missing(&work_dir.join("ToDoList"), &user_root.join("todo"))?;
    merge_tree_if_missing(&work_dir.join("ImgGenerator"), &user_root.join("images"))?;
    merge_tree_if_missing(
        &work_dir.join("CloudWork").join("sessions"),
        &user_root.join("workspaces").join("sessions"),
    )?;
    Ok(())
}

fn merge_file_if_missing(src: &Path, dst: &Path) -> Result<(), String> {
    if src == dst || !src.is_file() || dst.exists() {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::copy(src, dst).map_err(|e| e.to_string())?;
    Ok(())
}

fn merge_tree_if_missing(src: &Path, dst: &Path) -> Result<(), String> {
    if src == dst || !src.is_dir() {
        return Ok(());
    }
    if dst.exists() {
        merge_missing(src, dst)
    } else {
        copy_dir_recursive(src, dst)
    }
}

/// 递归合并：只复制目标缺失的文件/目录，绝不覆盖（会话按 id 命名 — 天然按文件级合并）
fn merge_missing(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            merge_missing(&from, &to)?;
        } else if !to.exists() {
            std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct SetDataDirRequest {
    pub dir: String,
}

/// GET /api/datadir — 首启检测 + 当前数据目录（前端首页弹窗依据）
pub async fn datadir_get_handler() -> axum::Json<serde_json::Value> {
    let (dir, configured) = resolve();
    axum::Json(serde_json::json!({
        "status": "ok",
        "configured": configured,
        "dir": dir.display().to_string(),
        "default": default_root().display().to_string(),
    }))
}

/// POST /api/datadir — 设置数据目录（server 模式仅 admin；standalone 免认证直接放行）
pub async fn datadir_set_handler(
    axum::extract::State(ctx): axum::extract::State<crate::api::AppContext>,
    headers: axum::http::HeaderMap,
    axum::Json(req): axum::Json<SetDataDirRequest>,
) -> axum::Json<serde_json::Value> {
    if ctx.auth.is_some() && crate::auth::require_admin(&ctx, &headers).is_err() {
        return axum::Json(serde_json::json!({ "status": "error", "message": "仅管理员可设置数据目录" }));
    }
    match set(&req.dir) {
        Ok(p) => axum::Json(serde_json::json!({
            "status": "ok",
            "dir": p.display().to_string(),
            "message": "数据目录已创建并记录 — 重启应用后全部数据定向到新位置",
        })),
        Err(e) => axum::Json(serde_json::json!({ "status": "error", "message": e })),
    }
}

use serde::Deserialize;

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct EnvVarGuard {
        name: &'static str,
        old: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(name: &'static str, value: &Path) -> Self {
            let old = std::env::var_os(name);
            std::env::set_var(name, value);
            Self { name, old }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.old.take() {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }

    #[test]
    fn test_set_and_resolve_pointer() {
        let _env_lock = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        // 隔离：default_root/指针必须指向临时 APPDATA — 否则会写用户真实 %APPDATA%\Skyland
        let saved_appdata = std::env::var("APPDATA").ok();
        let appdata_dir = std::env::temp_dir()
            .join(format!("skyland_test_appdata_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&appdata_dir);
        std::env::set_var("APPDATA", appdata_dir.display().to_string());
        let tmp = tempfile::tempdir().unwrap();
        let custom = tmp.path().join("MyData");
        let dir_str = custom.display().to_string();
        let p = set(&dir_str).unwrap();
        assert_eq!(p, custom);
        for sub in ["config", "auth", "users", "logs"] {
            assert!(custom.join(sub).is_dir(), "缺少 {}", sub);
        }
        let (resolved, configured) = resolve();
        assert!(configured);
        assert_eq!(resolved, custom);
        // 清理 + 恢复环境（防污染其他并行测试与下一轮测试）
        let _ = std::fs::remove_dir_all(&appdata_dir);
        match saved_appdata {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }
    }

    #[test]
    fn test_reject_relative_and_empty() {
        assert!(set("").is_err());
        assert!(set("relative/path").is_err());
    }

    #[test]
    fn layout_exposes_skyland_config_auth_and_user_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = AppDataLayout::from_root_for_test(tmp.path().to_path_buf(), true);
        assert_eq!(layout.config_dir(), tmp.path().join("config"));
        assert_eq!(layout.auth_dir(), tmp.path().join("auth"));
        assert_eq!(layout.user_dir("alice").unwrap(), tmp.path().join("users").join("alice"));
    }

    #[test]
    fn layout_rejects_invalid_username() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = AppDataLayout::from_root_for_test(tmp.path().to_path_buf(), true);
        assert!(layout.user_dir("../alice").is_err());
    }

    #[test]
    fn skyland_data_env_precedes_legacy_env() {
        let _env_lock = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let skyland = tmp.path().join("skyland");
        let legacy = tmp.path().join("legacy");
        let _skyland_guard = EnvVarGuard::set("SKYLAND_DATA_DIR", &skyland);
        let _legacy_guard = EnvVarGuard::set("MOLINE_DATA_DIR", &legacy);
        let layout = AppDataLayout::resolve();
        assert_eq!(layout.root(), skyland);
        assert!(layout.is_configured());
    }

    #[test]
    fn migrate_to_layout_copies_missing_user_data_without_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let target = tmp.path().join("skyland");
        let layout = AppDataLayout::from_root_for_test(target.clone(), true);

        std::fs::create_dir_all(old.join("UserData").join(".conversations")).unwrap();
        std::fs::create_dir_all(old.join("UserData").join("kb")).unwrap();
        std::fs::create_dir_all(old.join("UserData").join("agents")).unwrap();
        std::fs::create_dir_all(old.join("ToDoList")).unwrap();
        std::fs::create_dir_all(old.join("ImgGenerator")).unwrap();
        std::fs::create_dir_all(old.join("config")).unwrap();
        std::fs::write(old.join("UserData").join(".conversations").join("1.json"), "旧会话").unwrap();
        std::fs::write(old.join("UserData").join(".conversations").join("2.json"), "旧会话2").unwrap();
        std::fs::write(old.join("UserData").join("kb").join(".meta.json"), "{}").unwrap();
        std::fs::write(old.join("UserData").join("kb").join("doc.md"), "知识库").unwrap();
        std::fs::write(old.join("UserData").join("agents").join("reviewer.md"), "代理").unwrap();
        std::fs::write(old.join("ToDoList").join("2026-09-24.json"), "待办").unwrap();
        std::fs::write(old.join("ImgGenerator").join("image.png"), "图片").unwrap();
        std::fs::write(old.join("config").join("app.toml"), "mode = \"standalone\"").unwrap();
        std::fs::write(old.join("config").join("llm.toml"), "model = \"test\"").unwrap();
        std::fs::write(old.join("config").join("users.json"), "[]").unwrap();

        let user_conversations = target.join("users").join("alice").join("conversations");
        std::fs::create_dir_all(&user_conversations).unwrap();
        std::fs::write(user_conversations.join("1.json"), "新会话").unwrap();
        migrate_legacy_to_layout(&old, &layout, Some("alice")).unwrap();

        assert_eq!(std::fs::read_to_string(user_conversations.join("1.json")).unwrap(), "新会话");
        assert!(user_conversations.join("2.json").exists());
        assert!(target.join("users/alice/knowledge/kb/doc.md").exists());
        assert!(target.join("users/alice/agents/reviewer.md").exists());
        assert!(target.join("users/alice/todo/2026-09-24.json").exists());
        assert!(target.join("users/alice/images/image.png").exists());
        assert!(target.join("app.toml").exists());
        assert!(target.join("config/llm.toml").exists());
        assert!(target.join("auth/users.json").exists());
        assert!(old.join("UserData/.conversations/1.json").exists());
    }

    #[test]
    fn test_migrate_moves_and_merges_without_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        // 场景一：目标不存在 → 整目录搬移
        let old = tmp.path().join("old_root");
        let new = tmp.path().join("new_root");
        std::fs::create_dir_all(old.join("UserData").join("kb1")).unwrap();
        std::fs::write(old.join("UserData").join("kb1").join("doc.md"), "内容").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        migrate_legacy(&old, &new);
        assert!(new.join("UserData").join("kb1").join("doc.md").exists());
        assert!(!old.join("UserData").exists());
        // 场景二：目标已有数据 → 按文件合并缺失项，既有文件绝不覆盖
        let old2 = tmp.path().join("old2");
        std::fs::create_dir_all(old2.join("UserData").join(".conversations")).unwrap();
        std::fs::write(old2.join("UserData").join(".conversations").join("100.json"), "旧会话").unwrap();
        std::fs::write(old2.join("UserData").join(".conversations").join("200.json"), "新会话ID").unwrap();
        std::fs::create_dir_all(new.join("UserData").join(".conversations")).unwrap();
        std::fs::write(new.join("UserData").join(".conversations").join("100.json"), "既有会话").unwrap();
        migrate_legacy(&old2, &new);
        // 100 已存在 → 保留既有内容；200 缺失 → 并入
        assert_eq!(std::fs::read_to_string(new.join("UserData").join(".conversations").join("100.json")).unwrap(), "既有会话");
        assert_eq!(std::fs::read_to_string(new.join("UserData").join(".conversations").join("200.json")).unwrap(), "新会话ID");
        // 旧位置在合并场景下保留（用户确认后自行清理）
        assert!(old2.join("UserData").join(".conversations").join("100.json").exists());
    }
}


/// GET /api/datadir/resolve?key=... — 解析数据目录键为绝对路径。
/// 供设置页「打开此目录位置」（前端拿绝对路径调 Tauri shell.open，
/// server/client 模式下目录在服务端——前端降级为复制路径）。
pub async fn datadir_resolve_handler(
    axum::extract::State(ctx): axum::extract::State<crate::api::AppContext>,
    user: Option<axum::Extension<crate::auth::CurrentUser>>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::Json<serde_json::Value> {
    use crate::api::user_data_dir_for;
    let user_ref = user.as_ref().map(|u| &u.0);
    let key = q.get("key").map(String::as_str).unwrap_or("");
    let abs = match key {
        "config" => crate::api::resolve_config_path(),
        "userdata" => user_data_dir_for(&ctx, user_ref),
        "conversations" => user_data_dir_for(&ctx, user_ref).join("conversations"),
        "todo" => crate::todo_api::todo_dir_for_user(&ctx, user_ref),
        "images" => crate::image_api::img_dir_for(&ctx, user_ref),
        _ => {
            return axum::Json(serde_json::json!({ "status": "error", "message": "未知目录键" }));
        }
    };
    axum::Json(serde_json::json!({
        "status": "ok", "key": key, "abs": abs.display().to_string(),
        "exists": abs.exists(),
    }))
}

//! skyland-web 共享库 — 暴露 axum Router 和 API 类型
//! v3: 双模式运行（standalone / server / client）+ 认证中间件

pub mod api;
mod agentmem;
pub mod agents;
pub mod approval;
mod chat_tools;
pub mod datadir;
pub mod discovery;
mod mcp;
mod model_catalog;
mod filegen;
mod scheduler;
mod settings;
pub mod status;
mod skills;
mod todo_api;
mod todo_notify;
mod system_fonts;
mod todo_sync;
mod notify_api;
mod image_api;
pub mod auth;
pub mod app_config;
pub mod env_compat;
pub mod write_guard;

pub use api::{AppContext, load_providers};

use axum::Router;
use std::sync::Arc;

/// standalone 模式端口扫描起点与上限（3000-3099 共 100 个候选）
pub const STANDALONE_PORT_BASE: u16 = 3000;
pub const STANDALONE_PORT_LIMIT: u16 = 100;

/// 端口自增绑定：3000 被占用时自动依次尝试 3001、3002…直到找到空闲端口。
/// 返回绑定成功的 (port, listener)——listener 直接由调用方持有并交给服务器，
/// 不存在"先探测再重绑"的 TOCTOU 竞态窗口；全部候选端口被占用时返回 None。
pub fn bind_open_port() -> Option<(u16, std::net::TcpListener)> {
    for port in STANDALONE_PORT_BASE..STANDALONE_PORT_BASE + STANDALONE_PORT_LIMIT {
        if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port)) {
            return Some((port, listener));
        }
    }
    None
}

/// 静态前端资源 — 编译期嵌入二进制（环境自适应的核心）：
/// - release 构建：资源字节直接打包进 exe，目标机器无需源码目录，
///   在任何位置运行都不会因找不到磁盘路径而 404（单文件绿色部署）
/// - debug 构建：默认运行时读磁盘（static/ 可热改，开发体验不变）
#[derive(rust_embed::RustEmbed)]
#[folder = "static"]
struct StaticAssets;

/// 内嵌静态资源服务 — 接管所有非 /api 路径（原 ServeDir 磁盘方案的环境适配替代品）。
/// 根路径映射 index.html；查不到返回 404。
/// 资源键为编译期生成的固定集合，不存在目录穿越面。
async fn static_file_handler(uri: axum::http::Uri) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;

    let raw = uri.path().trim_start_matches('/');
    let key = if raw.is_empty() { "index.html" } else { raw };

    match StaticAssets::get(key) {
        Some(file) => {
            let mime = mime_guess::from_path(key).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], file.data.to_vec()).into_response()
        }
        None => (StatusCode::NOT_FOUND, "404 not found").into_response(),
    }
}

/// 创建完整的 axum Router，可在独立服务器或 Tauri 嵌入模式下使用
/// - `auth`: server 模式传 Some(AuthState)，standalone 传 None
/// - `server_mode`: "standalone" | "server" | "client"（供 /api/server/info 报告）

/// 跨源 API 防护 — Origin 与 Host 不同源的请求拒绝访问 /api。
/// 浏览器自动附加且不可伪造 Origin；非浏览器调用（curl/调度任务）不带 Origin
/// 直接放行。这是 standalone 无认证模式对"恶意网页读取本机数据"的主防线。
async fn same_origin_guard<B>(
    req: axum::http::Request<B>,
    next: axum::middleware::Next<B>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let is_api = req.uri().path().starts_with("/api");
    if is_api {
        if let (Some(origin), Some(host)) = (
            req.headers().get(axum::http::header::ORIGIN).and_then(|v| v.to_str().ok()),
            req.headers().get(axum::http::header::HOST).and_then(|v| v.to_str().ok()),
        ) {
            let origin_authority = origin
                .strip_prefix("http://").or_else(|| origin.strip_prefix("https://"))
                .unwrap_or(origin);
            // 忽略端口差异会造成跨端口读取面（3000-3099 自增段），精确比较
            if !origin_authority.eq_ignore_ascii_case(host) {
                return (StatusCode::FORBIDDEN, "跨源请求被拒绝").into_response();
            }
        }
    }
    next.run(req).await
}

pub fn create_router(
    work_dir: std::path::PathBuf,
    auth: Option<Arc<auth::AuthState>>,
    server_mode: &str,
) -> Router {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");

    // 数据目录定向（Windows 规范 APPDATA/SKYLAND 或用户自选）— 程序目录不再落新数据。
    let layout = crate::datadir::AppDataLayout::resolve();
    if let Err(error) = layout.ensure_structure() {
        eprintln!("⚠ Skyland 数据目录结构创建失败: {}", error);
    }
    let fallback_user = auth.as_ref()
        .and_then(|state| {
            state.users.read().ok()
                .and_then(|users| users.iter().find(|user| user.role == "admin")
                    .map(|user| user.username.clone()))
        })
        .unwrap_or_else(|| "local".to_string());
    if let Err(error) = crate::datadir::migrate_legacy_to_layout(
        &work_dir,
        &layout,
        Some(&fallback_user),
    ) {
        eprintln!("⚠ Skyland 旧数据迁移失败: {}", error);
    }

    let user_data_dir = layout.user_dir(&fallback_user)
        .unwrap_or_else(|_| layout.root().join("users").join("local"));
    std::fs::create_dir_all(&user_data_dir).ok();

    // 加载搜索配置
    let search_paths = [
        format!("{}/config/llm.toml", layout.root().display()),
        format!("{}/config/llm.toml", work_dir.display()),
        "config/llm.toml".to_string(),
        "../config/llm.toml".to_string(),
        format!("{}/../../config/llm.toml", manifest_dir),
    ];
    for p in &search_paths {
        if let Ok(c) = std::fs::read_to_string(p) {
            if let Ok(v) = c.parse::<toml::Value>() {
                if let Some(search) = v.get("search").and_then(|s| s.as_table()) {
                    if let Some(key) = search.get("api_key").and_then(|k| k.as_str()) {
                        let key = if key.starts_with("${") {
                            std::env::var(&key[2..key.len()-1]).unwrap_or_default()
                        } else {
                            key.to_string()
                        };
                        if !key.is_empty() {
                            skyland_lib::llm::tool_executor::set_search_api_key(&key);
                        }
                    }
                }
                break;
            }
        }
    }

    let mut ctx = load_providers(user_data_dir);
    ctx.auth = auth;
    ctx.server_mode = server_mode.to_string();

    // 品牌通知图标物化 — 注册表 IconUri 要求磁盘上的真实文件；
    // release 下静态资源只有内嵌字节，启动时落一份到 UserData（每次覆盖，随版本更新）
    if let Some(logo) = StaticAssets::get("img/logo.png") {
        let icon = ctx.user_data_dir.join("skyland-notify-icon.png");
        if std::fs::write(&icon, &*logo.data).is_ok() {
            skyland_lib::native_notify::set_brand_icon(&icon);
        }
    }

    use axum::{routing::{get, post}, http::Method};
    use tower_http::cors::{CorsLayer, Any};

    let router = Router::new()
        .route("/api/chat", post(api::chat_handler))
        .route("/api/chat/simple", post(api::simple_chat_handler))
        .route("/api/chat/stream", post(api::chat_stream_handler))
        .route("/api/chat/context-estimate", post(api::context_estimate_handler))
        .route("/api/chat/artifact", get(api::chat_artifact_handler))
        .route("/api/health", get(api::health_handler))
        .route("/api/providers/config", get(api::providers_config_handler))
        .route("/api/providers/models", post(api::provider_models_handler))
        .route("/api/providers/test", post(api::provider_test_chat_handler))
        .route("/api/providers/save", post(api::provider_save_handler))
        .route("/api/providers/delete", post(api::provider_delete_handler))
        .route("/api/providers/presets", get(api::presets_handler))
        .route("/api/search/status", get(api::search_status_handler))
        .route("/api/search/test", post(api::search_test_handler))
        .route("/api/search/save", post(api::search_save_handler))
        .route("/api/kb/list", get(api::kb::kb_list_handler))
        .route("/api/kb/create", post(api::kb::kb_create_handler))
        .route("/api/kb/:name", axum::routing::delete(api::kb::kb_delete_handler))
        .route("/api/kb/:name/tree", get(api::kb::kb_tree_handler))
        .route("/api/kb/:name/doc", get(api::kb::kb_doc_get_handler).post(api::kb::kb_doc_save_handler).delete(api::kb::kb_doc_delete_handler))
        .route("/api/kb/:name/folder", post(api::kb::kb_folder_handler))
        .route("/api/kb/:name/rename", post(api::kb::kb_rename_handler))
        .route("/api/kb/:name/image", post(api::kb::kb_image_upload_handler))
        .route("/api/kb/:name/image/:img", get(api::kb::kb_image_serve_handler))
        .route("/api/kb/:name/history", get(api::kb::kb_doc_history_handler))
        .route("/api/kb/:name/history/restore", post(api::kb::kb_doc_restore_handler))
        .route("/api/kb/:name/move", post(api::kb::kb_doc_move_handler))
        .route("/api/kb/:name/icon", post(api::kb::kb_doc_icon_handler))
        .route("/api/kb/:name/search", get(api::kb::kb_search_handler))
        // 数据目录（首启检测/定向 — Windows 规范 APPDATA，程序目录零数据落盘）
        .route("/api/datadir", get(crate::datadir::datadir_get_handler).post(crate::datadir::datadir_set_handler))
        .route("/api/settings/user", get(crate::settings::settings_get_handler).put(crate::settings::settings_put_handler))
        // 工作区初始化（.skyland 结构 — conversations/cache）
        .route("/api/workspace/prepare", post(crate::filegen::workspace_prepare_handler))
        // 最近工作区列表（数据目录持久化 — 跟随数据目录而非浏览器）
        .route("/api/workspace/list", get(crate::filegen::workspace_list_handler).post(crate::filegen::workspace_list_save_handler))
        // 对话历史
        .route("/api/conversations/list", get(api::kb::conversations_list_handler))
        .route("/api/conversations/:id", get(api::kb::conversations_get_handler).delete(api::kb::conversations_delete_handler))
        .route("/api/conversations/save", post(api::kb::conversations_save_handler))
        .route("/api/conversations/export", get(api::kb::conversations_export_handler))
        .route("/api/conversations/import", post(api::kb::conversations_import_handler))
        // 待办事项
        .route("/api/todo/today", get(todo_api::todo_today_handler))
        .route("/api/todo/list", get(todo_api::todo_list_handler))
        .route("/api/todo/stats", get(todo_api::todo_stats_handler))
        .route("/api/todo/search", get(todo_api::todo_search_handler))
        .route("/api/todo/path", get(todo_api::todo_path_handler).post(todo_api::todo_path_save_handler))
        .route("/api/todo/:date", get(todo_api::todo_get_handler))
        .route("/api/todo/:date/item", post(todo_api::todo_add_item_handler))
        .route("/api/todo/:date/item/:id", axum::routing::put(todo_api::todo_update_item_handler).delete(todo_api::todo_delete_item_handler))
        .route("/api/todo/:date/carryover", post(todo_api::todo_carryover_handler))
        .route("/api/mcp/config", get(crate::mcp::mcp_get_handler).post(crate::mcp::mcp_save_handler))
        .route("/api/memory/scan", get(crate::agentmem::memory_scan_handler))
        .route("/api/memory/import", post(crate::agentmem::memory_import_handler))
        .route("/api/kb/approval", post(crate::approval::approval_decide_handler))
        .route("/api/system/status", get(crate::status::system_status_handler))
        .route("/api/kb/approval/pending", get(crate::approval::approval_pending_handler))
        .route("/api/skills/list", get(crate::skills::skills_list_handler))
        .route("/api/skills/doc", get(crate::skills::skill_doc_handler).post(crate::skills::skill_doc_save_handler))
        .route("/api/skills/dir", get(crate::skills::skills_dir_handler))
        .route("/api/tasks/list", get(crate::scheduler::tasks_list_handler))
        .route("/api/tasks/create", post(crate::scheduler::tasks_create_handler))
        .route("/api/tasks/:id", axum::routing::put(crate::scheduler::tasks_update_handler).delete(crate::scheduler::tasks_delete_handler))
        .route("/api/tasks/:id/run", post(crate::scheduler::tasks_run_handler))
        .route("/api/todo/timeline", get(todo_api::todo_timeline_handler))
        .route("/api/todo/export", get(todo_api::todo_export_handler))
        .route("/api/system/fonts", get(crate::system_fonts::fonts_handler))
        .route("/api/todo/sync/config", get(crate::todo_sync::sync_config_get).post(crate::todo_sync::sync_config_save))
        .route("/api/todo/sync/test", post(crate::todo_sync::sync_test))
        .route("/api/todo/sync/pull", post(crate::todo_sync::sync_pull_handler))
        .route("/api/datadir/resolve", get(crate::datadir::datadir_resolve_handler))
        .route("/api/todo/import", post(todo_api::todo_import_handler))
        .route("/api/todo/explore", get(todo_api::todo_explore_handler))
        // 系统通知（本机直发 — window.notifyWindows 链第一级与"测试通知"按钮落点）
        .route("/api/notify/send", post(notify_api::notify_send_handler))
        // 图片生成
        .route("/api/image/config", get(image_api::image_config_handler))
        .route("/api/image/path", get(image_api::image_path_handler).post(image_api::image_path_save_handler))
        .route("/api/image/config/save", post(image_api::image_config_save_handler))
        .route("/api/image/settings/save", post(image_api::image_settings_save_handler))
        .route("/api/agents", get(agents::agents_list_handler))
        .route("/api/image/generate", post(image_api::image_generate_handler))
        .route("/api/image/save", post(image_api::image_save_handler))
        .route("/api/image/history", get(image_api::image_history_handler))
        .route("/api/image/file/:filename", get(image_api::image_file_handler).delete(image_api::image_delete_handler))
        .route("/api/image/workflows", get(image_api::workflows_list_handler))
        .route("/api/image/workflows/save", post(image_api::workflows_save_handler))
        .route("/api/image/workflows/:filename", get(image_api::workflows_load_handler).delete(image_api::workflows_delete_handler))
        // 认证与运行模式（server 模式启用认证中间件）
        .route("/api/server/info", get(auth::server_info_handler))
        .route("/api/server/config", post(app_config::server_config_save_handler))
        .route("/api/auth/login", post(auth::login_handler))
        .route("/api/auth/logout", post(auth::logout_handler))
        .route("/api/auth/me", get(auth::me_handler))
        .route("/api/auth/change_password", post(auth::change_password_handler))
        .route("/api/auth/setup", post(auth::setup_handler))
        .route("/api/auth/users", get(auth::users_list_handler).post(auth::users_create_handler))
        .route("/api/auth/users/:username", axum::routing::delete(auth::users_delete_handler))
        .route("/api/auth/whitelist", get(auth::whitelist_get_handler).post(auth::whitelist_save_handler))
        .fallback(static_file_handler)
        // 认证中间件 — 位于 CORS 之内，覆盖全部路由（含静态资源，白名单需要）
        .layer(axum::middleware::from_fn_with_state(ctx.clone(), auth::auth_middleware))
        .layer(
            CorsLayer::new()
                // CORS 头保持宽松 — 真正的跨源裁决在下方 same_origin_guard：
                // 任意网页都能跨源**读取** 127.0.0.1:3000 的 API 响应是 standalone
                // 模式最大的被动泄露面，仅靠 CORS 头（无凭据模式）拦不住。
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::PUT]),
        )
        // 跨源 API 防护（P1）：带 Origin 且与 Host 不同源的请求一律 403 —
        // 浏览器会自动附 Origin 且不可伪造；桌面壳/浏览器直连页面本身就是
        // 同源，不受影响；非浏览器调用（curl/调度任务/局域网 fetch 同源）不带
        // Origin 或同源，均放行。
        .layer(axum::middleware::from_fn(same_origin_guard))
        // Image references are sent as base64 JSON. 30 MiB of decoded image
        // data needs roughly 40 MiB on the wire, plus JSON and metadata.
        .layer(axum::extract::DefaultBodyLimit::max(45 * 1024 * 1024))
        .with_state(ctx.clone());

    // 内置技能首启释放（eager — 定时任务/对话 use_skill 任何路径都依赖容器存在）
    crate::skills::ensure_builtin_skills(&ctx);
    // 待办提醒引擎（单例）— 后端直发系统 Toast，WebView 死活无关；
    // client 模式不跑本地服务（create_router 不会被调用），由前端远端兜底引擎负责
    crate::todo_notify::start(ctx.clone());
    crate::todo_sync::start_engine(ctx.clone());
    // 定时任务调度循环（单例）— 三模式均在服务进程内运行；
    // client 模式 Tauri 不启动本地服务，任务在云端服务端执行
    crate::scheduler::start_scheduler(ctx);
    router
}

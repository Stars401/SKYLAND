//! skyland-app — Tauri 桌面应用入口
//! 后台启动 axum 服务器，系统托盘常驻，无控制台窗口
//!
//! 托盘行为：
//! - 左键单击: 显示/隐藏主窗口
//! - 右键菜单: 智能体 / 技术知识库 / 退出 SKYLAND
//! - 关闭按钮(X): 隐藏到托盘，不退出进程

use std::path::PathBuf;
use skyland_lib::llm::tool_executor;
use tauri::{
    Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};

fn get_work_dir() -> PathBuf {
    let manifest_dir = option_env!("CARGO_MANIFEST_DIR").unwrap_or("");
    if !manifest_dir.is_empty() {
        let dev_dir = PathBuf::from(manifest_dir)
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        if dev_dir.join("config").exists() {
            return dev_dir;
        }
    }

    // 生产模式统一使用 Skyland 数据根；旧目录由 datadir 迁移层兼容读取。
    let data_dir = skyland_web::datadir::AppDataLayout::resolve().root().to_path_buf();
    std::fs::create_dir_all(&data_dir).ok();
    let config_dir = data_dir.join("config");
    std::fs::create_dir_all(&config_dir).ok();
    data_dir
}

/// Avoid opening an empty WebView when a configured client server is offline.
/// A short TCP probe keeps the existing client mode intact while allowing the
/// app to start locally and remain usable when the remote endpoint is down.
fn remote_server_reachable(raw: &str) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

    let url = match tauri::Url::parse(&format!("{}/", raw.trim_end_matches('/'))) {
        Ok(url) => url,
        Err(_) => return false,
    };
    let host = match url.host_str() {
        Some(host) if !host.is_empty() => host,
        _ => return false,
    };
    let port = match url.port_or_known_default() {
        Some(port) => port,
        None => return false,
    };
    let addresses = match (host, port).to_socket_addrs() {
        Ok(addresses) => addresses,
        Err(_) => return false,
    };
    addresses.into_iter().any(|address| {
        TcpStream::connect_timeout(&address, Duration::from_millis(1200)).is_ok()
    })
}

#[tauri::command]
fn save_workflow_file(path: String, content: String) -> Result<(), String> {
    let target = PathBuf::from(path.trim());
    if target.as_os_str().is_empty() || target.file_name().is_none() {
        return Err("无效的工作流文件路径".into());
    }
    if target.extension().and_then(|value| value.to_str()).map(|value| value.eq_ignore_ascii_case("json")) != Some(true) {
        return Err("工作流文件必须使用 .json 扩展名".into());
    }
    std::fs::write(&target, content).map_err(|error| format!("无法写入工作流文件: {}", error))
}

/// 读取本地运行模式配置（设置面板用，与 HTTP 服务无关，client 模式也可用）
#[tauri::command]
fn get_app_config() -> Result<serde_json::Value, String> {
    let work_dir = get_work_dir();
    let cfg = skyland_web::app_config::AppConfig::load(&work_dir);
    Ok(serde_json::json!({
        "mode": cfg.mode.as_str(),
        "server_bind": cfg.server_bind,
        "server_url": cfg.server_url,
    }))
}

/// 保存运行模式配置（写入数据根 app.toml，重启生效）
#[tauri::command]
fn save_app_config(
    mode: String,
    server_bind: Option<String>,
    server_url: Option<String>,
) -> Result<serde_json::Value, String> {
    let work_dir = get_work_dir();
    let mut cfg = skyland_web::app_config::AppConfig::load(&work_dir);
    cfg.mode = match mode.as_str() {
        "standalone" => skyland_web::app_config::AppMode::Standalone,
        "server" => skyland_web::app_config::AppMode::Server,
        "client" => skyland_web::app_config::AppMode::Client,
        _ => return Err("无效的运行模式".into()),
    };
    if let Some(bind) = server_bind {
        if !bind.is_empty() && bind.len() < 64 {
            cfg.server_bind = bind;
        }
    }
    if let Some(url) = server_url {
        if url.len() < 256 {
            // client 模式下校验 URL 格式，防止无效配置导致启动失败
            if cfg.mode == skyland_web::app_config::AppMode::Client && !url.trim().is_empty() {
                let u = url.trim();
                if (!(u.starts_with("http://") || u.starts_with("https://"))) || u.len() < 8 {
                    return Err("服务器地址需以 http:// 或 https:// 开头".into());
                }
            }
            cfg.server_url = url;
        }
    }
    skyland_web::app_config::AppConfig::save(&work_dir, &cfg)?;
    Ok(serde_json::json!({ "status": "ok", "message": "配置已保存，重启应用后生效" }))
}

/// 重启应用 — 运行模式切换后由前端调用，消除"需手动退出重启"的引导断层
#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
}

/// Windows 原生通知 — WinRT Toast 直发（skyland-lib native_notify）。
/// 旧实现走 tauri-plugin-notification：其 Windows 底层需要 AUMID 在系统注册，
/// 未注册时 Toast 被静默丢弃；且本命令此前从未进 ACL 权限清单，remote WebView
/// 的 invoke() 一律被拒 —— 这是通知功能十轮迭代失败的两大根因。现改为：
/// ① native_notify 自带 AUMID 注册表注册 + PowerShell 通道兜底；
/// ② build.rs AppManifest + capabilities 已补 allow-notify-windows。
#[tauri::command]
fn notify_windows(title: String, body: String) -> Result<(), String> {
    match skyland_lib::native_notify::send(&title, &body) {
        (true, _) => Ok(()),
        (false, via) => Err(format!("通知发送失败（通道: {}）— 请检查 Windows 通知设置", via)),
    }
}

/// 原生目录选择器（工作目录"浏览"按钮用）——独立线程跑 rfd（主线程禁止阻塞对话框）
#[tauri::command]
fn pick_directory() -> Result<Option<String>, String> {
    let handle = std::thread::spawn(|| rfd::FileDialog::new()
        .set_title("选择工作目录")
        .pick_folder());
    Ok(handle.join().map_err(|_| "选择线程异常".to_string())?
        .map(|p| p.to_string_lossy().to_string()))
}

/// 局域网服务端发现 — 广播 UDP 探测，收集应答（client 模式设置页/引导页调用）
#[tauri::command]
fn discover_lan_servers() -> Result<serde_json::Value, String> {
    let handle = std::thread::spawn(|| skyland_web::discovery::discover_servers(1800));
    let found = handle.join().map_err(|_| "扫描线程异常".to_string())?;
    let servers: Vec<serde_json::Value> = found.iter().map(|s| {
        serde_json::json!({
            "url": format!("http://{}:{}", s.ip, s.port),
            "name": s.name,
            "ip": s.ip,
            "port": s.port,
        })
    }).collect();
    Ok(serde_json::json!({ "servers": servers }))
}

/// 构建 Tauri 应用 — 启动服务器 + 创建窗口 + 创建托盘
/// `server`: 调用方预绑定的 (TcpListener, Router)，消除探测/重绑之间的 TOCTOU 窗口
/// `client_mode`: client 模式下 WebView 加载远端——远端不可达时页面无界面可切回，
///               托盘提供"重置为一体模式"逃生通道，防止不可恢复状态
fn build_tauri_app(url: tauri::Url, server: Option<(std::net::TcpListener, axum::Router)>, client_mode: bool) {
    let url = url.to_string();

    if let Some((listener, router)) = server {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                axum::Server::from_tcp(listener)
                    .unwrap()
                    .serve(router.into_make_service_with_connect_info::<std::net::SocketAddr>())
                    .await
                    .unwrap();
            });
        });
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![save_workflow_file, get_app_config, save_app_config, restart_app, discover_lan_servers, pick_directory, notify_windows])
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .setup(move |app| {
            // WebView 加载的是 External URL（一体/云端模式为本机 http://127.0.0.1:port，
            // 客户端模式为局域网服务器）。Tauri ACL 只把它信任的 app 自身源视为 local，
            // External URL 一律算 remote origin —— 不显式授予 remote capability 时，
            // 页面里所有 window.__TAURI__ invoke()（保存工作流、选择目录、原生通知、
            // 模式切换等）都会被 ACL 静默拒绝，表现为大量按钮"点了没反应"。
            // 这里按实际加载的 URL 运行时注入 capability（端口动态分配，静态文件写死不了），
            // 权限清单与 capabilities/default.json 对齐，外加全部自定义命令。
            {
                let mut remote_capability = tauri::ipc::CapabilityBuilder::new("remote-webview")
                    .remote(url.clone())
                    .window("main");
                for permission in [
                    "core:default",
                    "shell:allow-open",
                    "core:window:allow-show",
                    "core:window:allow-hide",
                    "core:window:allow-set-focus",
                    "core:window:allow-start-dragging",
                    "dialog:default",
                    // 原生通知命令（曾长期缺席此清单 → invoke 被静默拒绝，通知十连败根因之一）
                    "allow-notify-windows",
                    "allow-save-workflow-file",
                    "allow-get-app-config",
                    "allow-save-app-config",
                    "allow-restart-app",
                    "allow-discover-lan-servers",
                    "allow-pick-directory",
                ] {
                    remote_capability = remote_capability.permission(permission);
                }
                app.add_capability(remote_capability)?;
            }

            // 创建主窗口
            let _window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(url.parse().unwrap()),
            )
            .title("SKYLAND")
            .inner_size(1200.0, 800.0)
            .min_inner_size(900.0, 600.0)
            .resizable(true)
            .decorations(true)
            .visible(true)
            .build()?;

            // 创建系统托盘菜单
            let agent_item = MenuItem::with_id(app, "agent", "Agent 终端", true, None::<&str>)?;
            let img_item = MenuItem::with_id(app, "image", "AI 生图", true, None::<&str>)?;
            let nodes_item = MenuItem::with_id(app, "nodes", "节点流生图", true, None::<&str>)?;
            let separator1 = PredefinedMenuItem::separator(app)?;
            let kb_item = MenuItem::with_id(app, "kb", "技术知识库", true, None::<&str>)?;
            let todo_item = MenuItem::with_id(app, "todo", "待办事项", true, None::<&str>)?;
                        let tasks_item = MenuItem::with_id(app, "tasks", "待时任务", true, None::<&str>)?;
            let separator2 = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出 SKYLAND", true, None::<&str>)?;
            // client 模式独有逃生通道：远端不可达时 WebView 无界面可切回，托盘直达重置
            let menu = if client_mode {
                let sep = PredefinedMenuItem::separator(app)?;
                let lan_item = MenuItem::with_id(app, "lan_discover", "搜索局域网服务端…", true, None::<&str>)?;
                let reset_item = MenuItem::with_id(app, "reset_mode", "重置为一体模式", true, None::<&str>)?;
                Menu::with_items(app, &[&agent_item, &img_item, &nodes_item, &separator1, &kb_item, &todo_item, &tasks_item, &sep, &lan_item, &reset_item, &separator2, &quit_item])?
            } else {
                Menu::with_items(app, &[&agent_item, &img_item, &nodes_item, &separator1, &kb_item, &todo_item, &tasks_item, &separator2, &quit_item])?
            };

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("SKYLAND — 点击显示窗口")
                .menu(&menu)
                .show_menu_on_left_click(false) // 左键不触发菜单，用 on_tray_icon_event 处理
                .on_menu_event(move |app, event| {
                    match event.id.as_ref() {
                        "agent" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                                let _ = w.eval("window.location.hash = '#/agent';");
                            }
                        }
                        "image" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                                let _ = w.eval("window.location.hash = '#/image';");
                            }
                        }
                        "nodes" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                                let _ = w.eval("window.location.hash = '#/nodes';");
                            }
                        }
                        "kb" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                                let _ = w.eval("window.location.hash = '#/kb';");
                            }
                        }
                        "todo" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                                let _ = w.eval("window.location.hash = '#/todo';");
                            }
                        }
                        "tasks" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                                let _ = w.eval("window.location.hash = '#/tasks';");
                            }
                        }
                        "lan_discover" => {
                            // 局域网扫描（阻塞放专用线程），结果经确认对话框连接第一个
                            let handle = app.clone();
                            std::thread::spawn(move || {
                                let found = skyland_web::discovery::discover_servers(1800);
                                if found.is_empty() {
                                    rfd::MessageDialog::new()
                                        .set_title("搜索局域网服务端")
                                        .set_description("未发现局域网内的 SKYLAND 服务端。\n请确认服务端已在同一网络并处于云端模式。")
                                        .set_buttons(rfd::MessageButtons::Ok)
                                        .show();
                                    return;
                                }
                                let list: Vec<String> = found.iter()
                                    .map(|s| format!("· {}  ({}:{})", s.name, s.ip, s.port))
                                    .collect();
                                let target = &found[0];
                                let url = format!("http://{}:{}", target.ip, target.port);
                                let yes = rfd::MessageDialog::new()
                                    .set_title("搜索局域网服务端")
                                    .set_description(format!(
                                        "发现 {} 个服务端：\n{}\n\n连接第一个（{}）并切换为客户端模式？",
                                        found.len(), list.join("\n"), url))
                                    .set_buttons(rfd::MessageButtons::YesNo)
                                    .show();
                                if matches!(yes, rfd::MessageDialogResult::Yes) {
                                    let work_dir = get_work_dir();
                                    let mut cfg = skyland_web::app_config::AppConfig::load(&work_dir);
                                    cfg.mode = skyland_web::app_config::AppMode::Client;
                                    cfg.server_url = url.clone();
                                    if skyland_web::app_config::AppConfig::save(&work_dir, &cfg).is_ok() {
                                        handle.restart();
                                    }
                                }
                            });
                        }
                        "quit" => {
                            // 彻底退出 — 结束进程链
                            app.exit(0);
                        }
                        "reset_mode" => {
                            // client 模式逃生通道 — 确认后切回一体模式并重启
                            let yes = rfd::MessageDialog::new()
                                .set_title("重置运行模式")
                                .set_description("将切换回【一体模式】（本机独立运行）并重启应用。\n\n云端服务器上的数据不受影响。确定继续？")
                                .set_level(rfd::MessageLevel::Warning)
                                .set_buttons(rfd::MessageButtons::YesNo)
                                .show();
                            if matches!(yes, rfd::MessageDialogResult::Yes) {
                                let work_dir = get_work_dir();
                                let mut cfg = skyland_web::app_config::AppConfig::load(&work_dir);
                                cfg.mode = skyland_web::app_config::AppMode::Standalone;
                                if skyland_web::app_config::AppConfig::save(&work_dir, &cfg).is_ok() {
                                    log_startup(&work_dir, "client 模式经托盘逃生通道重置为 standalone");
                                    app.restart();
                                }
                            }
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    // 左键单击: 显示窗口（如果已显示则聚焦，如果隐藏则显示）
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.set_focus();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                    // 双击: 同样显示窗口
                    if let tauri::tray::TrayIconEvent::DoubleClick {
                        button: tauri::tray::MouseButton::Left,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 关闭按钮 → 隐藏到托盘，不退出
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let work_dir = get_work_dir();
    tool_executor::set_work_dir(&work_dir);
    // The web server stays localhost-only, but the desktop shell may opt into
    // a user-selected image root through the native directory picker.
    // 写入只用权威名；读取侧（env_compat）负责兼容历史名，单向收敛。
    std::env::set_var("SKYLAND_DESKTOP_MODE", "1");
    eprintln!("✓ 工作目录: {}", work_dir.display());

    let app_cfg = skyland_web::app_config::AppConfig::load(&work_dir);
    log_startup(&work_dir, &format!("启动 mode={} work_dir={}", app_cfg.mode.as_str(), work_dir.display()));

    // ── client 模式：不启动本地服务，WebView 直接加载远端服务器 ──
    // 通知引擎在前端 JS 运行（相对 URL 指向远端），Windows 通知照常触发
    if app_cfg.mode == skyland_web::app_config::AppMode::Client {
        let raw = app_cfg.server_url.trim().trim_end_matches('/').to_string();
        if raw.is_empty() {
            eprintln!("⚠ client 模式未配置 server_url，回退为 standalone 模式");
            log_startup(&work_dir, "WARN client 模式未配置 server_url，回退 standalone");
        } else if !(raw.starts_with("http://") || raw.starts_with("https://")) {
            eprintln!("⚠ client 模式 server_url 无效 '{}'（需 http(s):// 开头），回退为 standalone 模式", raw);
            log_startup(&work_dir, &format!("WARN client server_url 无效 '{}'，回退 standalone", raw));
        } else if !remote_server_reachable(&raw) {
            eprintln!("⚠ 客户端服务器不可达 '{}'，回退为 standalone 模式", raw);
            log_startup(&work_dir, &format!("WARN client server 不可达 '{}'，回退 standalone", raw));
        } else if let Some(parsed) = tauri::Url::parse(&format!("{}/", raw)).ok() {
            eprintln!("✓ 客户端模式: 连接云端 {}", raw);
            log_startup(&work_dir, &format!("客户端模式: 连接云端 {}", raw));
            build_tauri_app(parsed, None, true);
            return;
        } else {
            eprintln!("⚠ server_url 解析失败 '{}'，回退为 standalone 模式", raw);
            log_startup(&work_dir, &format!("WARN server_url 解析失败 '{}'，回退 standalone", raw));
        }
    }

    // ── server 模式：云端服务端（绑定配置地址 + 启用认证 + admin 引导） ──
    if app_cfg.mode == skyland_web::app_config::AppMode::Server {
        let bind = if app_cfg.server_bind.trim().is_empty() {
            "0.0.0.0:3000".to_string()
        } else {
            app_cfg.server_bind.trim().to_string()
        };
        let bind_addr = match bind.parse::<std::net::SocketAddr>() {
            Ok(b) => b,
            Err(e) => {
                let msg = format!("无效的监听地址 '{}'：{}\n\n请修正数据目录中的 app.toml 的 server_bind 后重启应用。", bind, e);
                eprintln!("✗ {}", msg);
                log_startup(&work_dir, &format!("FATAL 无效 server_bind '{}': {}", bind, e));
                fatal_dialog(&msg);
                std::process::exit(1);
            }
        };
        let layout = skyland_web::datadir::AppDataLayout::resolve();
        if let Err(error) = skyland_web::datadir::migrate_legacy_to_layout(
            &work_dir,
            &layout,
            None,
        ) {
            eprintln!("⚠ Skyland 全局旧配置迁移失败: {}", error);
        }
        // 认证数据（users.json/审计）落数据目录 — 桌面端 work_dir 即数据目录，
        // 此处显式走 datadir::resolve 保证与指针定向一致（dev 模式下 work_dir=仓库亦不写认证进程序目录）
        let auth = skyland_web::auth::AuthState::new(layout.auth_dir());
        // 局域网发现应答（server 模式专属；standalone 只绑本机无发现意义）
        {
            let dport = bind_addr.port();
            let dname = app_cfg.server_name.clone();
            std::thread::spawn(move || {
                if let Err(e) = skyland_web::discovery::responder_loop(dport, &dname) {
                    eprintln!("⚠ 局域网发现应答启动失败: {}", e);
                }
            });
        }
        let app_router = skyland_web::create_router(work_dir.clone(), Some(auth), "server");

        match std::net::TcpListener::bind(bind_addr) {
            Ok(listener) => {
                let port = listener.local_addr().map(|a| a.port()).unwrap_or(3000);
                skyland_web::status::set_http_port(port);
                eprintln!("✓ 云端服务端已启动: {} (认证已启用)", bind_addr);
                log_startup(&work_dir, &format!("云端服务端已启动: {} (认证已启用)", bind_addr));
                if let Some(parsed) = tauri::Url::parse(&format!("http://127.0.0.1:{}/", port)).ok() {
                    build_tauri_app(parsed, Some((listener, app_router)), false);
                }
            }
            Err(e) => {
                // 不盲连 3000 — 配置端口失败时给出明确指引后退出（原生对话框，桌面端无控制台）
                let msg = format!("无法绑定 {}\n\n{}\n\n请检查端口占用，或修改数据目录中的 app.toml 的 server_bind 后重启应用。", bind_addr, e);
                eprintln!("✗ {}", msg);
                log_startup(&work_dir, &format!("FATAL 无法绑定 {}: {}", bind_addr, e));
                fatal_dialog(&msg);
                std::process::exit(1);
            }
        }
        return;
    }

    // ── standalone 模式（默认）──
    // 端口自增：3000 被占用（无论占用者是其他程序还是 SKYLAND 自身）时依次尝试
    // 3001、3002…直到找到空闲端口。选中的 port 同时用于服务器绑定与 WebView
    // 加载，全局唯一来源，不存在残留指向 3000 的线路。
    // 重复启动的第二个实例会被单实例插件拉起已有窗口后自动退出。
    match skyland_web::bind_open_port() {
        Some((port, listener)) => {
            log_startup(&work_dir, &format!("服务器已启动: http://127.0.0.1:{} (standalone)", port));
            // 直接持有已绑定 listener，消除探测/重绑之间的 TOCTOU
            let app_router = skyland_web::create_router(work_dir, None, "standalone");
            if port != skyland_web::STANDALONE_PORT_BASE {
                eprintln!(
                    "⚠ 端口 {} 被占用，已自动切换到 {}",
                    skyland_web::STANDALONE_PORT_BASE,
                    port
                );
            }
            eprintln!("✓ 服务器已启动: http://127.0.0.1:{}", port);
            if let Some(parsed) = tauri::Url::parse(&format!("http://127.0.0.1:{}/", port)).ok() {
                build_tauri_app(parsed, Some((listener, app_router)), false);
            }
        }
        None => {
            let msg = format!(
                "端口 {}-{} 全部被占用，无法启动本地服务器。\n\n请关闭占用这些端口的程序后重启应用。",
                skyland_web::STANDALONE_PORT_BASE,
                skyland_web::STANDALONE_PORT_BASE + skyland_web::STANDALONE_PORT_LIMIT - 1
            );
            eprintln!("✗ {}", msg);
            log_startup(&work_dir, "FATAL 3000-3099 全部被占用");
            fatal_dialog(&msg);
            std::process::exit(1);
        }
    }
}

/// 无控制台环境（windows 子系统）下致命错误必须可见——原生对话框兜底
fn fatal_dialog(message: &str) {
    let _ = rfd::MessageDialog::new()
        .set_title("SKYLAND 启动失败")
        .set_level(rfd::MessageLevel::Error)
        .set_description(message)
        .show();
}

/// 启动诊断日志 — 桌面端无控制台，关键启动事实落盘 {work_dir}/logs/startup.log。
/// 超过 256KB 自动截断，防无限增长。
fn log_startup(work_dir: &std::path::Path, line: &str) {
    use std::io::Write;
    let dir = work_dir.join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("startup.log");
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 256 * 1024 {
            let _ = std::fs::write(&path, b"");
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(f, "[{}] {}", ts, line);
    }
}

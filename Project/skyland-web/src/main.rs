//! SKYLAND Web 服务端 — standalone / server 模式入口
//! client 模式请使用 SKYLAND 桌面应用

use skyland_lib::llm::tool_executor;
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let work_dir = resolve_work_dir();
    tool_executor::set_work_dir(&work_dir);
    eprintln!("✓ 工作目录: {}", work_dir.display());

    let app_cfg = skyland_web::app_config::AppConfig::load(&work_dir);

    match app_cfg.mode {
        skyland_web::app_config::AppMode::Client => {
            eprintln!("✗ client 模式需要 SKYLAND 桌面应用（skyland-app）运行");
            eprintln!("  请使用桌面端连接云端服务器，或修改数据目录中的 app.toml 的 mode");
            std::process::exit(1);
        }
        skyland_web::app_config::AppMode::Standalone => {
            let app = skyland_web::create_router(work_dir.clone(), None, "standalone");
            // 端口自增：3000 被占用时自动尝试 3001、3002…（与桌面端策略一致）
            let (port, listener) = match skyland_web::bind_open_port() {
                Some(v) => {
                    skyland_web::status::set_http_port(v.0);
                    v
                }
                None => {
                    eprintln!(
                        "✗ 端口 {}-{} 全部被占用，无法启动服务器",
                        skyland_web::STANDALONE_PORT_BASE,
                        skyland_web::STANDALONE_PORT_BASE + skyland_web::STANDALONE_PORT_LIMIT - 1
                    );
                    std::process::exit(1);
                }
            };
            if port != skyland_web::STANDALONE_PORT_BASE {
                eprintln!("⚠ 端口 {} 被占用，已自动切换到 {}", skyland_web::STANDALONE_PORT_BASE, port);
            }
            println!("SKYLAND Web (一体模式): http://127.0.0.1:{}", port);
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .unwrap();
        }
        skyland_web::app_config::AppMode::Server => {
            let bind = match app_cfg.server_bind.parse::<std::net::SocketAddr>() {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("✗ 无效的 server_bind '{}': {} — 请修正数据目录中的 app.toml", app_cfg.server_bind, e);
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
            // 认证数据（users.json/会话）落数据目录 — 不再写程序目录
            let auth = skyland_web::auth::AuthState::new(layout.auth_dir());
            let app = skyland_web::create_router(work_dir.clone(), Some(auth), "server");
            // 局域网发现应答（server 模式专属；standalone 只绑本机无发现意义）
            {
                let dport = bind.port();
                let dname = app_cfg.server_name.clone();
                std::thread::spawn(move || {
                    if let Err(e) = skyland_web::discovery::responder_loop(dport, &dname) {
                        eprintln!("⚠ 局域网发现应答启动失败: {}", e);
                    }
                });
            }
            let listener = match std::net::TcpListener::bind(bind) {
                Ok(l) => {
                    skyland_web::status::set_http_port(l.local_addr().map(|a| a.port()).unwrap_or(0));
                    l
                }
                Err(e) => {
                    eprintln!("✗ 无法绑定 {}: {} — 请检查端口占用或修改数据目录中的 app.toml", bind, e);
                    std::process::exit(1);
                }
            };
            println!("SKYLAND Server (云端模式): http://{} — 认证已启用", bind);
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .unwrap();
        }
    }
}

/// 程序目录解析（只读探测，绝不创建 — 数据一律落数据目录，杜绝污染程序所在位置）：
/// 1. 开发目录（编译期路径，仅当其中 config/ 实际存在——开发机 cargo run）
/// 2. exe 同级目录（便携部署：config/ 与 exe 并排放置）
/// 3. exe 上两级目录（cargo target/release 布局）
/// 4. exe 目录（兜底 — 无数据落盘语义，仅作程序目录标识）
/// 避免把编译期烙死的绝对路径当作运行时路径——目标机器上该路径不存在
/// 会导致数据目录创建静默失败（KB/待办/生图全部写不进去）。
fn resolve_work_dir() -> std::path::PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let dev_dir = std::path::Path::new(manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf());
    if let Some(d) = &dev_dir {
        if d.join("config").exists() {
            return d.clone();
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        // exe 同级
        if let Some(d) = exe.parent() {
            if d.join("config").exists() {
                return d.to_path_buf();
            }
            // exe 上两级（target/release 布局）
            if let Some(d2) = d.parent().and_then(|p| p.parent()) {
                if d2.join("config").exists() {
                    return d2.to_path_buf();
                }
            }
            return d.to_path_buf();
        }
    }

    std::path::PathBuf::from(".")
}

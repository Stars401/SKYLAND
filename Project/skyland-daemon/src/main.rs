use anyhow::Result;
use skyland_daemon::{agent, server};
use std::sync::{Arc, Mutex};
use tonic::transport::Server;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    // SKYLAND_HANDOFF_DIR 权威，MOLINE_HANDOFF_DIR 兼容旧启动脚本
    let handoff_dir = std::env::var("SKYLAND_HANDOFF_DIR")
        .or_else(|_| std::env::var("MOLINE_HANDOFF_DIR"))
        .or_else(|_| std::env::current_dir().map(|d| d.join("handoff").display().to_string()))
        .unwrap_or_else(|_| "./handoff".to_string());

    let addr = "127.0.0.1:50051".parse()?;
    let agent_manager = Arc::new(Mutex::new(agent::AgentManager::new(
        std::env::current_dir().unwrap_or_default().display().to_string()
    )));
    let service = server::create_service(handoff_dir.clone(), agent_manager);

    tracing::info!("skyland-daemon 启动中...");
    tracing::info!("监听地址: {}", addr);
    tracing::info!("handoff目录: {}", handoff_dir);

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let server_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(service)
            .serve_with_shutdown(addr, async { rx.await.ok(); })
            .await
    });

    tokio::signal::ctrl_c().await?;
    tracing::info!("收到关闭信号，优雅退出中...");
    tx.send(()).ok();
    server_handle.await??;

    tracing::info!("skyland-daemon 已停止");
    Ok(())
}

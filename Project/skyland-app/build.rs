fn main() {
    // AppManifest::commands 为全部自定义命令自动生成 allow-<command> / deny-<command> 权限。
    // 桌面壳 WebView 加载的是本机/局域网 HTTP 服务（WebviewUrl::External），对 Tauri ACL
    // 而言属于"远端源"——没有这些权限条目时，来自该页面的 invoke() 会被 ACL 静默拒绝。
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "save_workflow_file",
                "get_app_config",
                "save_app_config",
                "restart_app",
                "discover_lan_servers",
                "pick_directory",
                // notify_windows 曾缺席 → allow-notify-windows 权限从未生成，
                // remote WebView 的通知 invoke 全被 ACL 静默拒绝（通知十连败根因）
                "notify_windows",
            ]),
        ),
    )
    .expect("failed to run tauri-build");
}

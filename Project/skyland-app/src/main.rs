// 隐藏 Windows 控制台窗口 — 绿色软件体验
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    skyland_app_lib::run()
}

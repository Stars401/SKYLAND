//! 系统状态面板 — 开发者快速查看：监听端口/客户端连接数/运行内存/存储占用/API 配置/系统信息/版本构建
//! server 模式下系统与连接信息仅管理员可见（多用户安全面）；standalone 全量开放。

use axum::{Json, extract::State, response::{IntoResponse, Response}};
use serde_json::json;
use std::path::Path;
use std::sync::OnceLock;

use crate::api::AppContext;

static HTTP_PORT: OnceLock<u16> = OnceLock::new();

/// 服务启动时登记 HTTP 监听端口（main.rs / Tauri 壳调用）
pub fn set_http_port(port: u16) {
    let _ = HTTP_PORT.set(port);
}

fn http_port() -> u16 {
    HTTP_PORT.get().copied().unwrap_or(0)
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

/// 进程常驻内存（Windows：psapi FFI；Unix：/proc/self/status VmRSS）
fn process_rss_mb() -> f64 {
    #[cfg(target_os = "windows")]
    {
        #[repr(C)]
        #[allow(non_snake_case)]
        struct PROCESS_MEMORY_COUNTERS {
            cb: u32,
            PageFaultCount: u32,
            PeakWorkingSetSize: usize,
            WorkingSetSize: usize,
            QuotaPeakPagedPoolUsage: usize,
            QuotaPagedPoolUsage: usize,
            QuotaPeakNonPagedPoolUsage: usize,
            QuotaNonPagedPoolUsage: usize,
            PagefileUsage: usize,
            PeakPagefileUsage: usize,
        }
        #[link(name = "kernel32")]
        extern "system" { fn GetCurrentProcess() -> isize; }
        #[link(name = "psapi")]
        extern "system" { fn GetProcessMemoryInfo(
            hProcess: isize, ppsmemCounters: *mut PROCESS_MEMORY_COUNTERS, cb: u32) -> i32; }
        unsafe {
            let mut pmc = PROCESS_MEMORY_COUNTERS {
                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                PageFaultCount: 0, PeakWorkingSetSize: 0, WorkingSetSize: 0,
                QuotaPeakPagedPoolUsage: 0, QuotaPagedPoolUsage: 0,
                QuotaPeakNonPagedPoolUsage: 0, QuotaNonPagedPoolUsage: 0,
                PagefileUsage: 0, PeakPagefileUsage: 0,
            };
            let ok = GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb);
            if ok != 0 { pmc.WorkingSetSize as f64 / 1024.0 / 1024.0 } else { 0.0 }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::fs::read_to_string("/proc/self/status").ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).map(String::from))
            .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse::<f64>().ok()))
            .map(|kb| kb / 1024.0)
            .unwrap_or(0.0)
    }
}

/// 目录占用扫描（条目数封顶防大目录卡顿）— 返回 (字节, 文件数)
fn dir_size(path: &Path, max_entries: usize) -> (u64, usize) {
    let mut bytes = 0u64;
    let mut files = 0usize;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if files >= max_entries { return (bytes, files); }
            let p = entry.path();
            if p.is_dir() { stack.push(p); }
            else if let Ok(m) = entry.metadata() {
                bytes += m.len(); files += 1;
            }
        }
    }
    (bytes, files)
}

fn mb(bytes: u64) -> f64 { (bytes as f64 / 1024.0 / 1024.0 * 10.0).round() / 10.0 }

/// GET /api/system/status — server 模式仅管理员；standalone 全量
pub async fn system_status_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<crate::auth::CurrentUser>>,
) -> Response {
    // server 模式：系统/连接信息仅管理员可见（多用户安全面）；standalone 全量
    if ctx.auth.is_some() {
        let Some(u) = user.map(|x| x.0) else {
            return (axum::http::StatusCode::UNAUTHORIZED,
                Json(json!({"status":"error","message":"未登录"}))).into_response();
        };
        if u.role != "admin" {
            return (axum::http::StatusCode::FORBIDDEN,
                Json(json!({"status":"error","message":"server 模式下状态信息仅管理员可见"}))).into_response();
        }
    }

    // ── 服务器 ──
    let port = http_port();
    let (client_connections, sessions_total) = match &ctx.auth {
        Some(auth) => {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs()).unwrap_or(0);
            let sessions = auth.sessions.read().unwrap_or_else(|e| e.into_inner());
            let active = sessions.values().filter(|s| s.expires > now).count();
            (active, sessions.len())
        }
        None => (0, 0),
    };

    // ── 存储 ──
    let (ud_b, ud_files) = dir_size(&ctx.user_data_dir, 200_000);
    let (todo_b, _) = dir_size(&ctx.todo_dir.read().unwrap_or_else(|e| e.into_inner()).clone(), 20_000);
    let (img_b, _) = dir_size(&ctx.image_dir.read().unwrap_or_else(|e| e.into_inner()).clone(), 50_000);
    let (cloud_b, _) = dir_size(&ctx.cloud_workdir, 50_000);
    let skills = crate::skills::list_skills(&ctx).len();
    let conv_dir = ctx.user_data_dir.join("conversations");
    let conversations = std::fs::read_dir(&conv_dir).map(|d| d.flatten().count()).unwrap_or(0);

    // ── 构建 ──
    let version = env!("CARGO_PKG_VERSION");
    let build_time = option_env!("SKYLAND_BUILD_TIME").unwrap_or("dev 构建");
    let git_branch = option_env!("SKYLAND_GIT_BRANCH").unwrap_or("unknown");

    Json(json!({
        "status": "ok",
        "server": {
            "mode": ctx.server_mode,
            "http_port": port,
            "bind": if port > 0 { format!(":{}", port) } else { String::new() },
            "client_connections": client_connections,
            "sessions_total": sessions_total,
        },
        "runtime": {
            "memory_mb": (process_rss_mb() * 10.0).round() / 10.0,
            "pid": std::process::id(),
        },
        "storage": {
            "user_data_mb": mb(ud_b), "user_data_files": ud_files,
            "todo_mb": mb(todo_b),
            "image_mb": mb(img_b),
            "cloud_mb": mb(cloud_b),
            "skills": skills,
            "conversations": conversations,
        },
        "api": {
            "models_configured": ctx.providers.len(),
            "default_model": ctx.default_model,
        },
        "system": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "hostname": hostname(),
        },
        "build": {
            "version": version,
            "build_time": build_time,
            "git_branch": git_branch,
        },
    })).into_response()
}

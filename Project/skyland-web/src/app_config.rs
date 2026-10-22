//! 应用运行模式配置 — config/app.toml
//! 三种模式：standalone（一体）| server（云端服务端）| client（客户端）
//! 首次启动无此文件时默认 standalone，前端引导页可通过 API 写入后重启生效。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use axum::response::IntoResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppMode {
    Standalone,
    Server,
    Client,
}

impl AppMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppMode::Standalone => "standalone",
            AppMode::Server => "server",
            AppMode::Client => "client",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_mode")]
    pub mode: AppMode,
    #[serde(default = "default_bind")]
    pub server_bind: String,
    #[serde(default)]
    pub server_url: String,
    /// 局域网发现时显示的服务端名称
    #[serde(default = "default_server_name")]
    pub server_name: String,
}

fn default_mode() -> AppMode { AppMode::Standalone }
fn default_bind() -> String { "0.0.0.0:3000".to_string() }

fn default_server_name() -> String { "SKYLAND 服务端".to_string() }

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            mode: AppMode::Standalone,
            server_bind: default_bind(),
            server_url: String::new(),
            server_name: default_server_name(),
        }
    }
}

impl AppConfig {
    /// 查找 app.toml：新数据根优先，旧程序目录/旧 config 目录仅兼容读取。
    pub fn find_config_path(work_dir: &Path) -> Option<PathBuf> {
        let layout = crate::datadir::AppDataLayout::resolve();
        let candidates = [
            layout.root().join("app.toml"),
            layout.root().join("config").join("app.toml"),
            work_dir.join("config").join("app.toml"),
            PathBuf::from("config/app.toml"),
            PathBuf::from("../config/app.toml"),
        ];
        candidates.into_iter().find(|p| p.exists())
    }

    pub fn load(work_dir: &Path) -> AppConfig {
        match Self::find_config_path(work_dir) {
            Some(path) => {
                match std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| toml::from_str::<AppConfig>(&s).ok())
                {
                    Some(cfg) => cfg,
                    None => {
                        eprintln!("⚠ config/app.toml 解析失败，回退为 standalone 模式");
                        AppConfig::default()
                    }
                }
            }
            None => AppConfig::default(),
        }
    }

    /// 原子写（tmp + rename）；新写入始终落数据根 `app.toml`。
    pub fn save(work_dir: &Path, cfg: &AppConfig) -> Result<(), String> {
        let _ = work_dir;
        let target = crate::datadir::AppDataLayout::resolve().root().join("app.toml");
        let dir = target.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("config"));
        std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录: {}", e))?;
        let content = toml::to_string_pretty(cfg).map_err(|e| format!("序列化失败: {}", e))?;
        let tmp = dir.join("app.toml.tmp");
        std::fs::write(&tmp, content).map_err(|e| format!("写入失败: {}", e))?;
        std::fs::rename(&tmp, target).map_err(|e| format!("替换失败: {}", e))
    }
}

/// POST /api/server/config — 前端引导页/设置面板写入运行模式（仅本地服务可用）
/// server 模式下仅管理员可修改；standalone 无认证直接放行
pub async fn server_config_save_handler(
    axum::extract::State(ctx): axum::extract::State<crate::api::AppContext>,
    headers: axum::http::HeaderMap,
    axum::Json(req): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    // server 模式下要求管理员权限
    if ctx.auth.is_some() {
        if let Err(resp) = crate::auth::require_admin(&ctx, &headers) {
            return resp;
        }
    }
    let mode = req.get("mode").and_then(|v| v.as_str()).unwrap_or("");
    let app_mode = match mode {
        "standalone" | "server" | "client" => mode,
        _ => return axum::Json(serde_json::json!({ "status": "error", "message": "无效的模式" })).into_response(),
    };

    let mut cfg = AppConfig::load(&ctx.work_dir);
    cfg.mode = match app_mode {
        "standalone" => AppMode::Standalone,
        "server" => AppMode::Server,
        _ => AppMode::Client,
    };
    if let Some(bind) = req.get("server_bind").and_then(|v| v.as_str()) {
        if !bind.is_empty() && bind.len() < 64 {
            cfg.server_bind = bind.to_string();
        }
    }
    if let Some(url) = req.get("server_url").and_then(|v| v.as_str()) {
        if url.len() < 256 {
            cfg.server_url = url.to_string();
        }
    }
    if let Some(name) = req.get("server_name").and_then(|v| v.as_str()) {
        let n = name.trim();
        if !n.is_empty() && n.chars().count() < 40 {
            cfg.server_name = n.to_string();
        }
    }

    match AppConfig::save(&ctx.work_dir, &cfg) {
        Ok(()) => axum::Json(serde_json::json!({
            "status": "ok",
            "mode": app_mode,
            "message": "配置已保存，重启应用后生效"
        })).into_response(),
        Err(e) => axum::Json(serde_json::json!({ "status": "error", "message": e })).into_response(),
    }
}

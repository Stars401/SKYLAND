//! MCP（Model Context Protocol）服务器配置管理
//! 本期深度：配置管理 + 提示注入（AI 知道有哪些 MCP 服务器及其能力）；
//! streamable HTTP 工具实时桥接为下一迭代预留（数据结构已含 enabled 字段）。
//! 存储：{work_dir}/config/mcp.json — 服务端资产；server 模式写入仅 admin（对齐数据目录惯例），
//! 读取所有登录用户可见；client 模式页面经远端 API 读写（逻辑在客户端、配置在服务端）。

use axum::{Json, extract::State, response::{IntoResponse, Response}};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::api::AppContext;
use crate::auth::CurrentUser;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct McpServer {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: Vec<McpServer>,
}

fn config_path(ctx: &AppContext) -> PathBuf {
    crate::datadir::AppDataLayout::from_context(ctx).config_dir().join("mcp.json")
}

pub fn load_config(ctx: &AppContext) -> McpConfig {
    std::fs::read_to_string(config_path(ctx)).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(ctx: &AppContext, cfg: &McpConfig) -> Result<(), String> {
    let path = config_path(ctx);
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).ok(); }
    // 原子写
    let tmp = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(cfg).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(&tmp, &content).map_err(|e| format!("写入失败: {}", e))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("替换失败: {}", e))
}

/// 系统提示注入用的摘要（每行：- 名称（URL）：描述）
pub fn prompt_summary(ctx: &AppContext) -> Option<String> {
    let cfg = load_config(ctx);
    let enabled: Vec<&McpServer> = cfg.servers.iter().filter(|s| s.enabled).collect();
    if enabled.is_empty() { return None; }
    Some(enabled.iter()
        .map(|s| format!("- {}（{}）：{}", s.name, s.url, s.description))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn validate(servers: &[McpServer]) -> Result<(), String> {
    if servers.len() > 32 { return Err("MCP 服务器最多 32 个".into()); }
    let mut names = std::collections::HashSet::new();
    for s in servers {
        let name = s.name.trim();
        if name.is_empty() || name.chars().count() > 40 { return Err("服务器名需 1-40 字符".into()); }
        if !names.insert(name.to_string()) { return Err(format!("服务器名重复: {}", name)); }
        let url = s.url.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!("{} 的 URL 需以 http(s):// 开头", name));
        }
        if url.len() > 256 { return Err("URL 过长".into()); }
    }
    Ok(())
}

/// GET /api/mcp/config
pub async fn mcp_get_handler(State(ctx): State<AppContext>) -> Json<serde_json::Value> {
    let cfg = load_config(&ctx);
    Json(serde_json::json!({ "status": "ok", "servers": cfg.servers }))
}

/// POST /api/mcp/config — 全量保存（server 模式仅 admin）
pub async fn mcp_save_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<CurrentUser>>,
    Json(req): Json<serde_json::Value>,
) -> Response {
    if ctx.auth.is_some() {
        let Some(u) = user.map(|x| x.0) else {
            return (axum::http::StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "status": "error", "message": "未登录" }))).into_response();
        };
        if u.role != "admin" {
            return (axum::http::StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "status": "error", "message": "server 模式下仅管理员可修改 MCP 配置" }))).into_response();
        }
    }
    let Some(servers_val) = req.get("servers") else {
        return (axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "status": "error", "message": "缺少 servers 字段" }))).into_response();
    };
    let servers: Vec<McpServer> = match serde_json::from_value(servers_val.clone()) {
        Ok(v) => v,
        Err(e) => return (axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "status": "error", "message": format!("格式无效: {}", e) }))).into_response(),
    };
    if let Err(e) = validate(&servers) {
        return (axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "status": "error", "message": e }))).into_response();
    }
    match save_config(&ctx, &McpConfig { servers }) {
        Ok(()) => Json(serde_json::json!({ "status": "ok", "message": "MCP 配置已保存" })).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "status": "error", "message": e }))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::AppContext;
    use std::sync::{Arc, RwLock};

    fn test_ctx(dir: &std::path::Path) -> AppContext {
        AppContext {
            providers: std::collections::HashMap::new(),
            default_model: String::new(),
            user_data_dir: dir.join("users").join("local"),
            work_dir: dir.to_path_buf(),
            context_lengths: std::collections::HashMap::new(),
            image_dir: Arc::new(RwLock::new(dir.to_path_buf())),
            todo_dir: Arc::new(RwLock::new(dir.join("ToDoList"))),
            auth: None,
            server_mode: "standalone".into(),
            cloud_workdir: dir.join("CloudWork"),
        }
    }

    #[test]
    fn test_roundtrip_and_summary() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        assert!(load_config(&ctx).servers.is_empty());
        assert!(prompt_summary(&ctx).is_none());

        let cfg = McpConfig { servers: vec![
            McpServer { name: "tavily".into(), url: "https://mcp.example.com/sse".into(),
                        description: "联网搜索".into(), enabled: true },
            McpServer { name: "off".into(), url: "https://x.example.com".into(),
                        description: "停用的".into(), enabled: false },
        ]};
        save_config(&ctx, &cfg).unwrap();
        let loaded = load_config(&ctx);
        assert_eq!(loaded.servers.len(), 2);
        let summary = prompt_summary(&ctx).unwrap();
        assert!(summary.contains("tavily") && !summary.contains("off"), "停用服务器不进提示");

        // 非法 URL 校验
        let bad = vec![McpServer { name: "x".into(), url: "ftp://bad".into(), description: String::new(), enabled: true }];
        assert!(validate(&bad).is_err());
        let dup = vec![
            McpServer { name: "a".into(), url: "http://a".into(), description: String::new(), enabled: true },
            McpServer { name: "a".into(), url: "http://b".into(), description: String::new(), enabled: true },
        ];
        assert!(validate(&dup).is_err());
    }
}

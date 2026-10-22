//! POST /api/notify/send — 在后端所在机器上弹一条 Windows 系统 Toast。
//!
//! 前端 `window.notifyWindows` 链的第一级（本机后端时）：不经 WebView、
//! 无 Tauri ACL、无权限协商 —— 这是 10 轮迭代后重新设计的可靠通道。
//! 同时也是待办页"测试通知"按钮的后端落点。

use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct NotifySendRequest {
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
}

pub async fn notify_send_handler(Json(req): Json<NotifySendRequest>) -> Response {
    // 截断防滥用 — 提醒用途 80/500 字符足够
    let title: String = req.title.chars().take(80).collect();
    let body: String = req.body.unwrap_or_default().chars().take(500).collect();
    if title.trim().is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"status":"error","delivered":false,"via":"","message":"标题不能为空"})),
        )
            .into_response();
    }

    let (ok, via) = skyland_lib::native_notify::send(&title, &body);
    if ok {
        Json(serde_json::json!({"status":"ok","delivered":true,"via":via})).into_response()
    } else {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status":"error","delivered":false,"via":via,
                "message":"系统 Toast 发送失败 — 请检查 Windows 设置→系统→通知 是否开启"
            })),
        )
            .into_response()
    }
}

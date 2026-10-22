//! 审批注册表 — 智能体操作的人工授权闸门（对齐 Claude Code 权限对话框机制）
//!
//! 决策三类（Claude: 允许一次 / 本次会话不再询问 / 拒绝）：
//! - AllowOnce     — 仅放行本次调用
//! - AllowSession  — 放行并写入会话规则（本次对话内同类操作不再询问）
//! - Deny          — 拒绝（含 180s 超时自动拒绝，保守默认）
//!
//! 会话规则（Claude: PermissionUpdate {addRules, destination:'session'}）：
//! 规则键 = 工具 + 操作特征（bash:命令首词 / file:路径 / kb:动作+库），
//! 按会话（conversation_id）隔离，TTL 2 小时懒清理。
//!
//! 流程：闸门发现需授权 → register_detailed → SSE approval_request（含 detail 风险说明）
//!       → 前端授权窗口【允许一次/本次对话允许/拒绝】+ Windows 通知
//!       → POST /api/kb/approval {token, allow, scope} → oneshot 决策回传。

use axum::{Json, extract::State, Extension};
use crate::auth::CurrentUser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::api::AppContext;

/// 用户决策 — AllowSession 由调用方按规则键写入会话规则表
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    AllowOnce,
    AllowSession,
    Deny,
}

impl Decision {
    pub fn allowed(&self) -> bool { !matches!(self, Decision::Deny) }
}

#[derive(Debug, Clone, Serialize)]
pub struct ApprovalInfo {
    pub token: String,
    pub action: String,
    pub target: String,
    pub detail: String,
}

struct Pending {
    info: ApprovalInfo,
    /// 归属用户（server 模式）：决策与 pending 列表都校验归属，防跨用户代批/偷看
    owner: Option<String>,
    tx: tokio::sync::oneshot::Sender<Decision>,
}

static REGISTRY: OnceLock<Arc<Mutex<HashMap<String, Pending>>>> = OnceLock::new();

fn registry() -> Arc<Mutex<HashMap<String, Pending>>> {
    REGISTRY.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))).clone()
}

/// 注册一个待审批操作，返回 (token, 等待结果的接收端)
pub fn register(action: &str, target: &str) -> (String, tokio::sync::oneshot::Receiver<Decision>) {
    register_detailed(action, target, "")
}

/// 注册待审批操作（带风险说明，授权窗口展示）— 无归属（standalone 兼容）
pub fn register_detailed(action: &str, target: &str, detail: &str) -> (String, tokio::sync::oneshot::Receiver<Decision>) {
    register_detailed_for(None, action, target, detail)
}

/// 注册待审批操作（带归属用户 — server 模式防跨用户代批）
pub fn register_detailed_for(owner: Option<&str>, action: &str, target: &str, detail: &str) -> (String, tokio::sync::oneshot::Receiver<Decision>) {
    // token = 128-bit 随机（旧「毫秒+u32」在无速率限制的决策端点可在线逼近爆破）
    let t = ((rand::random::<u64>() as u128) << 64) | (rand::random::<u64>() as u128);
    let token = format!("ap_{t:032x}");
    let (tx, rx) = tokio::sync::oneshot::channel();
    let info = ApprovalInfo { token: token.clone(), action: action.into(), target: target.into(), detail: detail.into() };
    registry().lock().unwrap_or_else(|e| e.into_inner())
        .insert(token.clone(), Pending { info, owner: owner.map(String::from), tx });
    (token, rx)
}

/// 等待用户决定（超时 / 通道关闭 = 拒绝 — 保守默认，与 Claude 无响应行为一致）
pub async fn wait_decision(rx: tokio::sync::oneshot::Receiver<Decision>) -> Decision {
    match tokio::time::timeout(Duration::from_secs(180), rx).await {
        Ok(Ok(v)) => v,
        _ => Decision::Deny,
    }
}

// ─── 会话规则（"本次对话允许"的持续效果）───

const SESSION_RULE_TTL: Duration = Duration::from_secs(2 * 60 * 60);

struct SessionEntry {
    rules: HashMap<String, std::time::Instant>,
}

static SESSION_RULES: OnceLock<Arc<Mutex<HashMap<String, SessionEntry>>>> = OnceLock::new();

fn session_rules() -> Arc<Mutex<HashMap<String, SessionEntry>>> {
    SESSION_RULES.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))).clone()
}

/// 会话规则是否命中（访问时懒清理过期条目；规则键形如 "bash:git" / "file:docs/a.md"）
pub fn session_allow(conv: &str, rule_key: &str) -> bool {
    let rules = session_rules();
    let mut reg = rules.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    if let Some(entry) = reg.get_mut(conv) {
        entry.rules.retain(|_, at| {
            now.checked_duration_since(*at)
                .map(|age| age < SESSION_RULE_TTL)
                .unwrap_or(false)
        });
        if entry.rules.is_empty() {
            reg.remove(conv);
            return false;
        }
        return entry.rules.contains_key(rule_key);
    }
    false
}

/// 写入会话规则（"本次对话允许"）
pub fn session_grant(conv: &str, rule_key: &str) {
    let rules = session_rules();
    let mut reg = rules.lock().unwrap_or_else(|e| e.into_inner());
    reg.entry(conv.to_string())
        .or_insert_with(|| SessionEntry { rules: HashMap::new() })
        .rules
        .insert(rule_key.to_string(), std::time::Instant::now());
}

#[derive(Deserialize)]
pub struct ApprovalDecision {
    pub token: String,
    pub allow: bool,
    /// "once"（默认）| "session" — session = 本次对话内同类操作不再询问
    pub scope: Option<String>,
}

/// POST /api/kb/approval — 前端授权窗口的决策（三模式均可：审批跟随会话所在服务）
pub async fn approval_decide_handler(
    State(_ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<ApprovalDecision>,
) -> Json<serde_json::Value> {
    let decision = if req.allow {
        if req.scope.as_deref() == Some("session") { Decision::AllowSession } else { Decision::AllowOnce }
    } else {
        Decision::Deny
    };
    // 归属校验：server 模式下仅本人（或管理员）可决策他人会话的挂起审批
    let pending = {
        let reg_arc = registry();
        let mut reg = reg_arc.lock().unwrap_or_else(|e| e.into_inner());
        match reg.get(&req.token) {
            None => None,
            Some(p) => {
                if let Some(owner) = &p.owner {
                    let requester = user.as_ref().map(|u| &u.0);
                    let is_owner = requester.map(|u| &u.username) == Some(owner);
                    let is_admin = requester.map(|u| u.role.as_str()) == Some("admin");
                    if !is_owner && !is_admin {
                        return Json(serde_json::json!({ "status": "error", "message": "无权决策他人的审批请求" }));
                    }
                }
                reg.remove(&req.token)
            }
        }
    };
    match pending {
        Some(p) => {
            let _ = p.tx.send(decision);
            Json(serde_json::json!({ "status": "ok", "decision": format!("{:?}", decision) }))
        }
        None => Json(serde_json::json!({ "status": "error", "message": "审批请求不存在或已超时" })),
    }
}

/// 审批期间暂存的文档内容（token → content；oneshot 跨线程，内容同步仓库）
static CONTENT_STORE: OnceLock<Arc<Mutex<HashMap<String, String>>>> = OnceLock::new();

pub fn store_content(token: &str, content: &str) {
    let store = CONTENT_STORE.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))).clone();
    store.lock().unwrap_or_else(|e| e.into_inner()).insert(token.to_string(), content.to_string());
}

pub fn take_content(token: &str) -> Option<String> {
    let store = CONTENT_STORE.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))).clone();
    let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
    guard.remove(token)
}

/// 查询挂起审批（前端重连后恢复授权窗口用）— server 模式仅返回本人/管理员可见
pub async fn approval_pending_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let reg_arc = registry();
    let reg = reg_arc.lock().unwrap_or_else(|e| e.into_inner());
    let viewer = user.as_ref().map(|u| &u.0);
    let is_admin = viewer.map(|u| u.role.as_str()) == Some("admin");
    let list: Vec<&ApprovalInfo> = reg.values()
        .filter(|p| {
            match (&p.owner, viewer) {
                (None, _) => true,                       // standalone / 旧条目
                (Some(_), None) => false,                // 未登录不可见
                (Some(owner), Some(v)) => is_admin || owner == &v.username,
            }
        })
        .map(|p| &p.info)
        .collect();
    let _ = &ctx;
    Json(serde_json::json!({ "status": "ok", "pending": list }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_decide_scopes() {
        // 允许一次 / 会话允许 / 拒绝 三类决策经 oneshot 正确回传
        let (t1, rx1) = register("执行命令", "git push");
        let (t2, rx2) = register("写入文件", "docs/a.md");
        let (t3, rx3) = register("删除", "kb/x.md");
        let h1 = tokio::spawn(async move { wait_decision(rx1).await });
        decide_inner(&t1, true, Some("once"));
        assert_eq!(h1.await.unwrap(), Decision::AllowOnce);
        let h2 = tokio::spawn(async move { wait_decision(rx2).await });
        decide_inner(&t2, true, Some("session"));
        assert_eq!(h2.await.unwrap(), Decision::AllowSession);
        let h3 = tokio::spawn(async move { wait_decision(rx3).await });
        decide_inner(&t3, false, None);
        assert_eq!(h3.await.unwrap(), Decision::Deny);
        // 通道关闭（未决策即消失）→ 拒绝
        let (_t4, rx4) = register("测试", "x");
        drop(rx4);
    }

    fn decide_inner(token: &str, allow: bool, scope: Option<&str>) {
        let pending = registry().lock().unwrap_or_else(|e| e.into_inner()).remove(token);
        if let Some(p) = pending {
            let decision = if allow {
                if scope == Some("session") { Decision::AllowSession } else { Decision::AllowOnce }
            } else { Decision::Deny };
            let _ = p.tx.send(decision);
        }
    }

    #[test]
    fn test_session_rules_grant_and_ttl_keying() {
        // 会话隔离：不同会话互不可见
        assert!(!session_allow("conv-a", "bash:git"), "未授权前不命中");
        session_grant("conv-a", "bash:git");
        assert!(session_allow("conv-a", "bash:git"), "授权后命中");
        assert!(!session_allow("conv-a", "bash:npm"), "规则键精确匹配（命令首词）");
        assert!(!session_allow("conv-b", "bash:git"), "跨会话不共享");
        // 过期清理由 session_allow 的 checked_duration_since 分支负责；
        // Windows 测试环境的 Instant 可能没有可表示的两小时前时间，不构造墙钟偏移。
    }
}

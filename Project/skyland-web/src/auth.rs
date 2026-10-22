//! 认证模块 — 用户管理 / 会话 / 登录限速 / IP 白名单 / 审计日志
//! 仅 server 模式启用（AppContext.auth = Some）；standalone 零开销跳过。

use axum::{
    Json,
    extract::{State, ConnectInfo, Path as AxumPath},
    http::{Request, StatusCode, header, HeaderMap, HeaderValue},
    middleware::Next,
    response::{IntoResponse, Response},
};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::{OsRng, RngCore}},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api::AppContext;

// ─── 常量 ───

/// 会话 Cookie 权威名。Cookie 名是浏览器与服务端之间的约定，改名本身安全，
/// 但历史名仍需读取 —— 会话是**持久化**的（`save_sessions` 落盘），
/// 服务端重启后旧 Cookie 依然有效；若只认新名，升级即等于把在线用户全部踢下线。
const SESSION_COOKIE: &str = "skyland_session";
/// 历史 Cookie 名（moline 时代）—— 仅用于读取兼容，见 [`parse_session_token`]。
const LEGACY_SESSION_COOKIE: &str = "moline_session";
const SESSION_TTL_SECS: u64 = 7 * 24 * 3600;
const MAX_LOGIN_FAILURES: u32 = 5;
const LOCKOUT_SECS: u64 = 15 * 60;

fn now_ts() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ─── 数据模型 ───

#[derive(Serialize, Deserialize, Clone)]
pub struct User {
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub created: u64,
    #[serde(default)]
    pub must_change_password: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Session {
    pub token: String,
    pub username: String,
    pub role: String,
    pub expires: u64,
}

#[derive(Clone)]
pub struct CurrentUser {
    pub username: String,
    pub role: String,
}

#[derive(Clone)]
struct RateEntry {
    failures: u32,
    locked_until: u64,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

// ─── 认证状态 ───

pub struct AuthState {
    pub users: Arc<RwLock<Vec<User>>>,
    pub sessions: Arc<RwLock<HashMap<String, Session>>>,
    rate_limiter: Arc<Mutex<HashMap<String, RateEntry>>>,
    pub whitelist: Arc<RwLock<Vec<String>>>,
    pub config_dir: PathBuf,
}

impl AuthState {
    /// 初始化：加载 users.json / sessions.json（清理过期）/ whitelist.json
    pub fn new(config_dir: PathBuf) -> Arc<AuthState> {
        std::fs::create_dir_all(&config_dir).ok();
        let users: Vec<User> = std::fs::read_to_string(config_dir.join("users.json")).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut sessions: HashMap<String, Session> = std::fs::read_to_string(config_dir.join("sessions.json")).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let now = now_ts();
        sessions.retain(|_, s| s.expires > now);
        let whitelist: Vec<String> = std::fs::read_to_string(config_dir.join("whitelist.json")).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Arc::new(AuthState {
            users: Arc::new(RwLock::new(users)),
            sessions: Arc::new(RwLock::new(sessions)),
            rate_limiter: Arc::new(Mutex::new(HashMap::new())),
            whitelist: Arc::new(RwLock::new(whitelist)),
            config_dir,
        })
    }

    fn save_users(&self) {
        let users = self.users.read().unwrap_or_else(|e| e.into_inner());
        let _ = std::fs::write(
            self.config_dir.join("users.json"),
            serde_json::to_string_pretty(&*users).unwrap_or_else(|_| "[]".into()),
        );
    }

    fn save_sessions(&self) {
        let sessions = self.sessions.read().unwrap_or_else(|e| e.into_inner());
        let _ = std::fs::write(
            self.config_dir.join("sessions.json"),
            serde_json::to_string_pretty(&*sessions).unwrap_or_else(|_| "{}".into()),
        );
    }

    fn save_whitelist(&self) {
        let wl = self.whitelist.read().unwrap_or_else(|e| e.into_inner());
        let _ = std::fs::write(
            self.config_dir.join("whitelist.json"),
            serde_json::to_string_pretty(&*wl).unwrap_or_else(|_| "[]".into()),
        );
    }

    /// 审计日志 — JSONL 追加
    fn audit(&self, event: &str, detail: &str, ip: &str) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true).append(true)
            .open(self.config_dir.join("auth_audit.jsonl"))
        {
            let _ = writeln!(f, "{}", json!({"ts": now_ts(), "event": event, "detail": detail, "ip": ip}));
        }
    }
}

// ─── 密码与会话工具 ───

fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("密码哈希失败: {}", e))
}

fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

fn gen_token() -> String {
    let mut buf = [0u8; 32];
    OsRng.fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 用户名：2-32 位，仅字母数字下划线连字符
fn validate_username(username: &str) -> Result<(), String> {
    let len = username.chars().count();
    if !(2..=32).contains(&len) {
        return Err("用户名长度需为 2-32 个字符".into());
    }
    if !username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("用户名仅允许字母、数字、下划线和连字符".into());
    }
    Ok(())
}

/// 密码：8-128 字节
fn validate_password(password: &str) -> Result<(), String> {
    let len = password.len();
    if !(8..=128).contains(&len) {
        return Err("密码长度需为 8-128 个字符".into());
    }
    Ok(())
}

// ─── 登录限速（内存态）───

/// Err(剩余锁定分钟数) = 已被锁定
fn rate_check(auth: &AuthState, key: &str) -> Result<(), u64> {
    let map = auth.rate_limiter.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = map.get(key) {
        let now = now_ts();
        if entry.locked_until > now {
            return Err((entry.locked_until - now + 59) / 60);
        }
    }
    Ok(())
}

fn rate_record_failure(auth: &AuthState, key: &str) {
    let mut map = auth.rate_limiter.lock().unwrap_or_else(|e| e.into_inner());
    // 内存上限：异常流量下防止无限增长（保留仍在锁定期的条目）
    if map.len() > 4096 {
        let now = now_ts();
        map.retain(|_, e| e.locked_until > now);
    }
    let now = now_ts();
    let entry = map.entry(key.to_string()).or_insert(RateEntry { failures: 0, locked_until: 0 });
    entry.failures += 1;
    if entry.failures >= MAX_LOGIN_FAILURES {
        entry.locked_until = now + LOCKOUT_SECS;
        entry.failures = 0;
    }
}

fn rate_clear(auth: &AuthState, key: &str) {
    let mut map = auth.rate_limiter.lock().unwrap_or_else(|e| e.into_inner());
    map.remove(key);
}

// ─── 会话验证 ───

fn parse_session_token(headers: &HeaderMap) -> Option<String> {
    let val = headers.get(header::COOKIE)?;
    let cookies = val.to_str().ok()?;
    // 权威名优先、历史名兜底：同一浏览器可能并存两个 Cookie，取权威名为准
    for name in [SESSION_COOKIE, LEGACY_SESSION_COOKIE] {
        let needle = format!("{}=", name);
        for part in cookies.split(';') {
            let part = part.trim();
            if let Some(token) = part.strip_prefix(needle.as_str()) {
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
    }
    None
}

fn validate_session(auth: &AuthState, headers: &HeaderMap) -> Option<CurrentUser> {
    let token = parse_session_token(headers)?;
    let sessions = auth.sessions.read().unwrap_or_else(|e| e.into_inner());
    let session = sessions.get(&token)?;
    if session.expires <= now_ts() {
        return None;
    }
    Some(CurrentUser { username: session.username.clone(), role: session.role.clone() })
}

fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "status": "error", "message": message }))).into_response()
}

fn json_ok(value: serde_json::Value) -> Response {
    Json(value).into_response()
}

/// 管理端点守卫 — (AuthState, 当前管理员) 或错误响应
pub(crate) fn require_admin(ctx: &AppContext, headers: &HeaderMap) -> Result<(Arc<AuthState>, CurrentUser), Response> {
    let auth = match &ctx.auth {
        Some(a) => a.clone(),
        None => return Err(json_error(StatusCode::INTERNAL_SERVER_ERROR, "当前未启用认证")),
    };
    match validate_session(&auth, headers) {
        Some(u) if u.role == "admin" => Ok((auth, u)),
        Some(_) => Err(json_error(StatusCode::FORBIDDEN, "需要管理员权限")),
        None => Err(json_error(StatusCode::UNAUTHORIZED, "未登录或会话已过期")),
    }
}

fn client_ip(req: &Request<axum::body::Body>) -> String {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "unknown".into())
}

/// IPv4-mapped IPv6（::ffff:a.b.c.d）归一化为 IPv4，供白名单比较
fn normalize_ip(ip: std::net::IpAddr) -> std::net::IpAddr {
    if let std::net::IpAddr::V6(v6) = ip {
        if let Some(v4) = v6.to_ipv4() {
            return std::net::IpAddr::V4(v4);
        }
    }
    ip
}

fn set_cookie(mut resp: Response, token: &str, max_age: u64) -> Response {
    // 登出（max_age == 0）必须**同时**清掉历史名 Cookie：
    // 因为 parse_session_token 仍接受历史名，只清新名会让用户"登出后仍处于登录态"。
    let names: &[&str] =
        if max_age == 0 { &[SESSION_COOKIE, LEGACY_SESSION_COOKIE] } else { &[SESSION_COOKIE] };
    for name in names {
        let cookie = format!(
            "{}={}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}",
            name, token, max_age
        );
        if let Ok(v) = HeaderValue::from_str(&cookie) {
            resp.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    resp
}

// ─── 管理员引导 ───

/// POST /api/auth/setup — server 模式首次启动：界面引导创建首个管理员
///
/// 取代旧方案（随机密码 + eprintln 控制台输出 + 隐藏文件）：
/// 桌面端为 windows 子系统无控制台，随机密码完全不可见，导致首启即被锁在门外。
/// 新方案由用户自设密码，无需任何"告知"渠道。
///
/// 安全约束：
/// - 仅当用户列表为空时可用（已初始化则 409）
/// - 仅限本机（loopback）调用 — 与旧方案"需本机文件/控制台访问权才能拿到密码"
///   的安全水位一致，防止首启窗口期内局域网内抢先注册
/// - 创建即登录（直接建立会话，免去二次输入）
pub async fn setup_handler(
    State(ctx): State<AppContext>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginRequest>,
) -> Response {
    let Some(auth) = ctx.auth.clone() else {
        return json_error(StatusCode::BAD_REQUEST, "当前为本地模式，无需设置管理员");
    };

    let ip = addr.ip();
    let is_local = ip.is_loopback()
        || matches!(normalize_ip(ip), std::net::IpAddr::V4(v4) if v4.is_loopback());
    if !is_local {
        auth.audit("setup_denied", "非本机请求初始管理员创建被拒绝", &ip.to_string());
        return json_error(StatusCode::FORBIDDEN, "初始管理员创建仅允许在服务器本机上进行");
    }

    let username = req.username.trim().to_string();
    if let Err(e) = validate_username(&username) { return json_error(StatusCode::BAD_REQUEST, &e); }
    if let Err(e) = validate_password(&req.password) { return json_error(StatusCode::BAD_REQUEST, &e); }

    let password_hash = match hash_password(&req.password) {
        Ok(h) => h,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };

    {
        let mut users = auth.users.write().unwrap_or_else(|e| e.into_inner());
        // 空检查与写入同锁，杜绝并发双注册
        if !users.is_empty() {
            return json_error(StatusCode::CONFLICT, "管理员已存在，请直接登录");
        }
        users.push(User {
            username: username.clone(),
            password_hash,
            role: "admin".to_string(),
            created: now_ts(),
            must_change_password: false, // 密码由用户自设，无需强制改密
        });
    }
    auth.save_users();
    auth.audit("admin_setup", &format!("初始管理员 {} 创建", username), &ip.to_string());

    // 创建即登录
    let token = gen_token();
    let session = Session {
        token: token.clone(),
        username: username.clone(),
        role: "admin".to_string(),
        expires: now_ts() + SESSION_TTL_SECS,
    };
    {
        let mut sessions = auth.sessions.write().unwrap_or_else(|e| e.into_inner());
        sessions.insert(token.clone(), session);
    }
    auth.save_sessions();

    let resp = json_ok(json!({
        "status": "ok",
        "authenticated": true,
        "username": username,
        "role": "admin",
    }));
    set_cookie(resp, &token, SESSION_TTL_SECS)
}

// ─── 公开端点 ───

/// GET /api/server/info — 前端引导用（公开）
/// users_initialized / can_setup 驱动首启管理员引导页：
/// 已初始化 → 登录页；未初始化 + 本机 → 创建管理员页；未初始化 + 远程 → 登录页 + 提示
pub async fn server_info_handler(
    State(ctx): State<AppContext>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
    let first_run = crate::app_config::AppConfig::find_config_path(&ctx.work_dir).is_none();
    let (users_initialized, can_setup) = match &ctx.auth {
        Some(auth) => {
            let initialized = !auth.users.read().unwrap_or_else(|e| e.into_inner()).is_empty();
            let ip = addr.ip();
            let is_local = ip.is_loopback()
                || matches!(normalize_ip(ip), std::net::IpAddr::V4(v4) if v4.is_loopback());
            (initialized, is_local && !initialized)
        }
        None => (true, false),
    };
    json_ok(json!({
        "mode": ctx.server_mode,
        "auth_required": ctx.auth.is_some(),
        "first_run": first_run,
        "users_initialized": users_initialized,
        "can_setup": can_setup,
        "client_ip": normalize_ip(addr.ip()).to_string(),
    }))
}

/// POST /api/auth/login — 登录（公开，限速保护）
pub async fn login_handler(
    State(ctx): State<AppContext>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginRequest>,
) -> Response {
    let Some(auth) = ctx.auth.clone() else {
        return json_error(StatusCode::BAD_REQUEST, "当前为本地模式，无需登录");
    };
    let username = req.username.trim().to_string();
    let ip = addr.ip().to_string();
    let rate_key = format!("{}:{}", ip, username);

    if let Err(minutes) = rate_check(&auth, &rate_key) {
        auth.audit("login_locked", &format!("用户 {} 登录锁定中", username), &ip);
        return json_error(StatusCode::TOO_MANY_REQUESTS, &format!("尝试次数过多，请 {} 分钟后再试", minutes));
    }

    if username.len() > 64 || req.password.len() > 128 {
        return json_error(StatusCode::BAD_REQUEST, "用户名或密码过长");
    }

    let user = auth.users.read().unwrap_or_else(|e| e.into_inner())
        .iter().find(|u| u.username == username).cloned();

    let Some(user) = user else {
        rate_record_failure(&auth, &rate_key);
        auth.audit("login_fail", &format!("用户不存在: {}", username), &ip);
        return json_error(StatusCode::UNAUTHORIZED, "用户名或密码错误");
    };

    if !verify_password(&req.password, &user.password_hash) {
        rate_record_failure(&auth, &rate_key);
        auth.audit("login_fail", &format!("密码错误: {}", username), &ip);
        return json_error(StatusCode::UNAUTHORIZED, "用户名或密码错误");
    }

    rate_clear(&auth, &rate_key);

    let token = gen_token();
    let session = Session {
        token: token.clone(),
        username: user.username.clone(),
        role: user.role.clone(),
        expires: now_ts() + SESSION_TTL_SECS,
    };
    {
        let mut sessions = auth.sessions.write().unwrap_or_else(|e| e.into_inner());
        sessions.retain(|_, s| s.expires > now_ts());
        sessions.insert(token.clone(), session);
    }
    auth.save_sessions();
    auth.audit("login_ok", &format!("用户 {} 登录", username), &ip);

    let resp = json_ok(json!({
        "status": "ok",
        "authenticated": true,
        "username": user.username,
        "role": user.role,
        "must_change_password": user.must_change_password,
    }));
    set_cookie(resp, &token, SESSION_TTL_SECS)
}

/// POST /api/auth/logout — 退出登录（删除凭证 + 清 Cookie）
pub async fn logout_handler(
    State(ctx): State<AppContext>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let Some(auth) = ctx.auth.clone() else {
        return json_ok(json!({ "status": "ok" }));
    };
    if let Some(token) = parse_session_token(&headers) {
        let removed = auth.sessions.write().unwrap_or_else(|e| e.into_inner()).remove(&token);
        if let Some(session) = removed {
            auth.save_sessions();
            auth.audit("logout", &format!("用户 {} 退出", session.username), &addr.ip().to_string());
        }
    }
    set_cookie(json_ok(json!({ "status": "ok" })), "", 0)
}

/// GET /api/auth/me — 当前用户（公开端点，200 + authenticated 标记，便于前端探测）
pub async fn me_handler(State(ctx): State<AppContext>, headers: HeaderMap) -> Response {
    let Some(auth) = ctx.auth.clone() else {
        return json_ok(json!({ "authenticated": false, "mode": ctx.server_mode }));
    };
    match validate_session(&auth, &headers) {
        Some(u) => {
            let must_change = auth.users.read().unwrap_or_else(|e| e.into_inner())
                .iter().find(|x| x.username == u.username)
                .map(|x| x.must_change_password)
                .unwrap_or(false);
            json_ok(json!({
                "authenticated": true,
                "username": u.username,
                "role": u.role,
                "must_change_password": must_change,
            }))
        }
        None => json_ok(json!({ "authenticated": false, "mode": ctx.server_mode })),
    }
}

// ─── 受保护端点（登录即可）───

/// POST /api/auth/change_password — 修改自己密码
pub async fn change_password_handler(
    State(ctx): State<AppContext>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<ChangePasswordRequest>,
) -> Response {
    let Some(auth) = ctx.auth.clone() else {
        return json_error(StatusCode::BAD_REQUEST, "当前未启用认证");
    };
    let Some(user) = validate_session(&auth, &headers) else {
        return json_error(StatusCode::UNAUTHORIZED, "未登录或会话已过期");
    };
    if let Err(e) = validate_password(&req.new_password) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    let new_hash = match hash_password(&req.new_password) {
        Ok(h) => h,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    {
        let mut users = auth.users.write().unwrap_or_else(|e| e.into_inner());
        let Some(u) = users.iter_mut().find(|u| u.username == user.username) else {
            return json_error(StatusCode::NOT_FOUND, "用户不存在");
        };
        if !verify_password(&req.old_password, &u.password_hash) {
            return json_error(StatusCode::UNAUTHORIZED, "原密码错误");
        }
        u.password_hash = new_hash;
        u.must_change_password = false;
    }
    auth.save_users();

    // 使该用户除当前会话外的其他会话失效
    let current_token = parse_session_token(&headers);
    {
        let mut sessions = auth.sessions.write().unwrap_or_else(|e| e.into_inner());
        sessions.retain(|token, s| {
            !(s.username == user.username && Some(token.as_str()) != current_token.as_deref())
        });
    }
    auth.save_sessions();
    auth.audit("change_password", &format!("用户 {} 修改密码", user.username), &addr.ip().to_string());
    json_ok(json!({ "status": "ok", "message": "密码已修改" }))
}

// ─── 管理端点（仅 admin）───

/// GET /api/auth/users — 用户列表（不含密码哈希）
pub async fn users_list_handler(State(ctx): State<AppContext>, headers: HeaderMap) -> Response {
    let (auth, _) = match require_admin(&ctx, &headers) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let users = auth.users.read().unwrap_or_else(|e| e.into_inner());
    let list: Vec<serde_json::Value> = users.iter().map(|u| json!({
        "username": u.username,
        "role": u.role,
        "created": u.created,
        "must_change_password": u.must_change_password,
    })).collect();
    json_ok(json!({ "status": "ok", "users": list }))
}

/// POST /api/auth/users — 创建用户
pub async fn users_create_handler(
    State(ctx): State<AppContext>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<serde_json::Value>,
) -> Response {
    let (auth, admin) = match require_admin(&ctx, &headers) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let username = req.get("username").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let password = req.get("password").and_then(|v| v.as_str()).unwrap_or("");
    let role = req.get("role").and_then(|v| v.as_str()).unwrap_or("user").to_string();

    if let Err(e) = validate_username(&username) { return json_error(StatusCode::BAD_REQUEST, &e); }
    if let Err(e) = validate_password(password) { return json_error(StatusCode::BAD_REQUEST, &e); }
    if role != "admin" && role != "user" { return json_error(StatusCode::BAD_REQUEST, "角色仅支持 admin/user"); }

    let password_hash = match hash_password(password) {
        Ok(h) => h,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };

    {
        let mut users = auth.users.write().unwrap_or_else(|e| e.into_inner());
        if users.iter().any(|u| u.username == username) {
            return json_error(StatusCode::CONFLICT, "用户名已存在");
        }
        users.push(User {
            username: username.clone(),
            password_hash,
            role: role.clone(),
            created: now_ts(),
            must_change_password: true,
        });
    }
    auth.save_users();
    auth.audit("user_create", &format!("{} 创建用户 {} ({})", admin.username, username, role), &addr.ip().to_string());
    json_ok(json!({ "status": "ok", "message": format!("用户 {} 已创建", username) }))
}

/// DELETE /api/auth/users/:username — 删除用户
pub async fn users_delete_handler(
    State(ctx): State<AppContext>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    AxumPath(username): AxumPath<String>,
) -> Response {
    let (auth, admin) = match require_admin(&ctx, &headers) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if username == admin.username {
        return json_error(StatusCode::BAD_REQUEST, "不能删除当前登录的账号");
    }
    {
        let mut users = auth.users.write().unwrap_or_else(|e| e.into_inner());
        let Some(idx) = users.iter().position(|u| u.username == username) else {
            return json_error(StatusCode::NOT_FOUND, "用户不存在");
        };
        // 先校验再删除，避免失败路径下内存态已被破坏
        if users[idx].role == "admin" && users.iter().filter(|u| u.role == "admin").count() <= 1 {
            return json_error(StatusCode::BAD_REQUEST, "不能删除最后一个管理员");
        }
        users.remove(idx);
    }
    auth.save_users();
    {
        let mut sessions = auth.sessions.write().unwrap_or_else(|e| e.into_inner());
        sessions.retain(|_, s| s.username != username);
    }
    auth.save_sessions();
    auth.audit("user_delete", &format!("{} 删除用户 {}", admin.username, username), &addr.ip().to_string());
    json_ok(json!({ "status": "ok" }))
}

/// GET /api/auth/whitelist — 白名单列表
pub async fn whitelist_get_handler(State(ctx): State<AppContext>, headers: HeaderMap) -> Response {
    let (auth, _) = match require_admin(&ctx, &headers) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let wl = auth.whitelist.read().unwrap_or_else(|e| e.into_inner());
    json_ok(json!({ "status": "ok", "whitelist": &*wl }))
}

/// POST /api/auth/whitelist — 设置白名单（空数组 = 不启用）
pub async fn whitelist_save_handler(
    State(ctx): State<AppContext>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<serde_json::Value>,
) -> Response {
    let (auth, admin) = match require_admin(&ctx, &headers) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let ips = req.get("ips").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut list: Vec<String> = Vec::new();
    for ip in ips {
        let Some(s) = ip.as_str() else { continue };
        let s = s.trim();
        if s.is_empty() { continue; }
        match s.parse::<std::net::IpAddr>() {
            Ok(_) => list.push(s.to_string()),
            Err(_) => return json_error(StatusCode::BAD_REQUEST, &format!("无效 IP: {}", s)),
        }
    }
    {
        let mut wl = auth.whitelist.write().unwrap_or_else(|e| e.into_inner());
        *wl = list;
    }
    auth.save_whitelist();
    auth.audit("whitelist_save", &format!("{} 更新访问白名单", admin.username), &addr.ip().to_string());
    json_ok(json!({ "status": "ok", "message": "白名单已更新" }))
}

// ─── 认证中间件 ───

const PUBLIC_PATHS: &[&str] = &[
    "/api/auth/login",
    "/api/auth/me",
    "/api/auth/setup",
    "/api/server/info",
    "/api/health",
];

pub async fn auth_middleware(
    State(ctx): State<AppContext>,
    req: Request<axum::body::Body>,
    next: Next<axum::body::Body>,
) -> Response {
    // standalone / client 模式 — 未启用认证，零开销放行
    let Some(auth) = ctx.auth.clone() else { return next.run(req).await; };

    let path = req.uri().path().to_string();
    let ip = client_ip(&req);

    // 1. IP 白名单（启用时拦截所有请求，含登录页与静态资源）
    //    以 IpAddr 规范化比较，兼容 IPv4-mapped IPv6（::ffff:1.2.3.4）
    {
        let wl = auth.whitelist.read().unwrap_or_else(|e| e.into_inner());
        if !wl.is_empty() {
            let client = ip.parse::<std::net::IpAddr>().ok().map(normalize_ip);
            let allowed = match client {
                Some(ci) => wl.iter().any(|w| {
                    w.parse::<std::net::IpAddr>().map(|wi| normalize_ip(wi) == ci).unwrap_or(false)
                }),
                None => wl.iter().any(|w| w == &ip),
            };
            if !allowed {
                return json_error(StatusCode::FORBIDDEN, "IP 不在访问白名单内");
            }
        }
    }

    // 2. 公开路径 / 静态资源放行
    let is_public = PUBLIC_PATHS.contains(&path.as_str()) || !path.starts_with("/api/");
    if is_public {
        return next.run(req).await;
    }

    // 3. 受保护 API — 会话校验
    match validate_session(&auth, req.headers()) {
        Some(user) => {
            let mut req = req;
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        None => json_error(StatusCode::UNAUTHORIZED, "未登录或会话已过期"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_cookie(cookie: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
        h
    }

    /// 旧 Cookie 必须仍然可用 —— 会话持久化在服务端，升级不能让在线用户掉线。
    #[test]
    fn legacy_session_cookie_is_still_accepted() {
        let h = headers_with_cookie("moline_session=tok-abc");
        assert_eq!(parse_session_token(&h).as_deref(), Some("tok-abc"));
    }

    #[test]
    fn new_session_cookie_is_accepted() {
        let h = headers_with_cookie("skyland_session=tok-new");
        assert_eq!(parse_session_token(&h).as_deref(), Some("tok-new"));
    }

    /// 新旧 Cookie 并存时以权威名为准，避免"登出后又被旧 Cookie 顶回来"。
    #[test]
    fn authoritative_cookie_wins_when_both_present() {
        let h = headers_with_cookie("moline_session=old; skyland_session=new");
        assert_eq!(parse_session_token(&h).as_deref(), Some("new"));
    }

    #[test]
    fn empty_token_is_rejected() {
        let h = headers_with_cookie("moline_session=");
        assert!(parse_session_token(&h).is_none());
    }

    #[test]
    fn unrelated_cookie_does_not_match() {
        let h = headers_with_cookie("moline_session_backup=nope");
        assert!(parse_session_token(&h).is_none());
    }

    /// 回归守卫：登出必须同时下发两个过期 Cookie。
    /// 少发历史名那条，浏览器里的 moline_session 仍有效 → 用户"点了退出但没退出"。
    #[test]
    fn logout_clears_both_cookie_names() {
        let resp = set_cookie(json_ok(json!({})), "", 0);
        let set_cookies: Vec<String> = resp
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect();
        assert!(
            set_cookies.iter().any(|c| c.starts_with("skyland_session=") && c.contains("Max-Age=0")),
            "登出必须清空 skyland_session: {:?}",
            set_cookies
        );
        assert!(
            set_cookies.iter().any(|c| c.starts_with("moline_session=") && c.contains("Max-Age=0")),
            "登出必须同时清空历史 Cookie，否则旧凭证仍可继续访问: {:?}",
            set_cookies
        );
    }

    /// 登录只下发权威名（新名写双份会让"权威名优先"的语义失去意义）。
    #[test]
    fn login_sets_only_authoritative_cookie() {
        let resp = set_cookie(json_ok(json!({})), "tok", 3600);
        let set_cookies: Vec<String> = resp
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect();
        assert_eq!(set_cookies.len(), 1, "登录只应下发一个 Cookie: {:?}", set_cookies);
        assert!(set_cookies[0].starts_with("skyland_session=tok"));
    }
}

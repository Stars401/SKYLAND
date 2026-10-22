//! /api/auth/setup 初始管理员引导 — 集成测试
//! 取代旧方案（随机密码 + 控制台输出）后的完整引导链路验证：
//! 本机创建 → 会话建立 → 重复创建拒绝 → 所设密码可正常登录。

use skyland_web::auth::AuthState;
use skyland_web::create_router;
use std::net::SocketAddr;
use std::time::Duration;

#[tokio::test]
async fn admin_setup_flow() {
    // 全新临时环境 — 无 users.json，模拟 server 模式首启
    let tmp = tempfile::tempdir().expect("创建临时目录失败");
    let work_dir = tmp.path().to_path_buf();
    // 数据目录钉在临时目录 — create_router 内部 datadir::resolve 会读真实 %APPDATA%，测试必须隔离
    std::env::set_var("SKYLAND_DATA_DIR", work_dir.display().to_string());

    let auth = AuthState::new(work_dir.join("config"));
    let app = create_router(work_dir.clone(), Some(auth), "server");

    // 随机端口起真实 HTTP 服务（ConnectInfo 中间件必需）
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定失败");
    let addr: SocketAddr = listener.local_addr().expect("获取端口失败");
    let server = axum::Server::from_tcp(listener)
        .expect("服务器创建失败")
        .serve(app.into_make_service_with_connect_info::<SocketAddr>());
    tokio::spawn(server);

    let base = format!("http://{}", addr);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("客户端创建失败");

    // 1. server_info：未初始化 + 本机（loopback）可设置
    let info: serde_json::Value = client
        .get(format!("{}/api/server/info", base))
        .send().await.expect("请求失败")
        .json().await.expect("解析失败");
    assert_eq!(info["auth_required"], serde_json::json!(true));
    assert_eq!(info["users_initialized"], serde_json::json!(false));
    assert_eq!(info["can_setup"], serde_json::json!(true));

    // 2. 未初始化时受保护 API 返回 401
    let r = client
        .get(format!("{}/api/auth/users", base))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 3. 弱密码被拒绝（密码校验 8-128 位）
    let r = client
        .post(format!("{}/api/auth/setup", base))
        .json(&serde_json::json!({"username": "boss", "password": "short"}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);

    // 4. 正常创建 → 200 + 会话 Cookie
    let r = client
        .post(format!("{}/api/auth/setup", base))
        .json(&serde_json::json!({"username": "boss", "password": "super-secret-9"}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let cookie = r
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .expect("缺少会话 Cookie")
        .to_string();
    let data: serde_json::Value = r.json().await.expect("解析失败");
    assert_eq!(data["status"], serde_json::json!("ok"));
    assert_eq!(data["role"], serde_json::json!("admin"));

    // 5. 携带会话 → me 已认证 + 受保护 API 放行
    let me: serde_json::Value = client
        .get(format!("{}/api/auth/me", base))
        .header("cookie", &cookie)
        .send().await.expect("请求失败")
        .json().await.expect("解析失败");
    assert_eq!(me["authenticated"], serde_json::json!(true));
    assert_eq!(me["username"], serde_json::json!("boss"));

    let users: serde_json::Value = client
        .get(format!("{}/api/auth/users", base))
        .header("cookie", &cookie)
        .send().await.expect("请求失败")
        .json().await.expect("解析失败");
    assert_eq!(users["status"], serde_json::json!("ok"));
    assert_eq!(users["users"].as_array().map(|a| a.len()), Some(1));

    // 6. 重复创建 → 409 拒绝（防并发/重入）
    let r = client
        .post(format!("{}/api/auth/setup", base))
        .json(&serde_json::json!({"username": "evil", "password": "another-pass-1"}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::CONFLICT);

    // 7. 新会话用所设密码登录成功
    let r = client
        .post(format!("{}/api/auth/login", base))
        .json(&serde_json::json!({"username": "boss", "password": "super-secret-9"}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let data: serde_json::Value = r.json().await.expect("解析失败");
    assert_eq!(data["status"], serde_json::json!("ok"));

    // 8. users.json 已持久化到磁盘
    let persisted = std::fs::read_to_string(work_dir.join("config").join("users.json"))
        .expect("users.json 未生成");
    assert!(persisted.contains("boss"));
}

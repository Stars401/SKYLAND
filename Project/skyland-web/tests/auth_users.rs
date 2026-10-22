//! 账号管理 API — 用户增删/角色/白名单 集成测试
//! 覆盖：管理员引导后的用户全生命周期（创建/列表/删除）、自我删除与
//! 重复用户名保护、响应不泄露密码哈希、白名单 IP 校验。

use skyland_web::auth::AuthState;
use skyland_web::create_router;
use std::net::SocketAddr;
use std::time::Duration;

#[tokio::test]
async fn user_management_flow() {
    let tmp = tempfile::tempdir().expect("创建临时目录失败");
    let work_dir = tmp.path().to_path_buf();
    // 数据目录钉在临时目录 — create_router 内部 datadir::resolve 会读真实 %APPDATA%，测试必须隔离
    std::env::set_var("SKYLAND_DATA_DIR", work_dir.display().to_string());

    let auth = AuthState::new(work_dir.join("config"));
    let app = create_router(work_dir.clone(), Some(auth), "server");

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

    // 0. 管理员引导（建立会话 Cookie）
    let r = client
        .post(format!("{}/api/auth/setup", base))
        .json(&serde_json::json!({"username": "root", "password": "admin-pass-123"}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let cookie = r
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .expect("缺少会话 Cookie")
        .to_string();

    // 1. 弱密码创建 → 400
    let r = client
        .post(format!("{}/api/auth/users", base))
        .header("cookie", &cookie)
        .json(&serde_json::json!({"username": "user1", "password": "short", "role": "user"}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);

    // 2. 无效角色 → 400
    let r = client
        .post(format!("{}/api/auth/users", base))
        .header("cookie", &cookie)
        .json(&serde_json::json!({"username": "user1", "password": "pass-12345", "role": "boss"}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);

    // 3. 合法创建 → ok，must_change_password=true
    let r = client
        .post(format!("{}/api/auth/users", base))
        .header("cookie", &cookie)
        .json(&serde_json::json!({"username": "user1", "password": "pass-12345", "role": "user"}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let data: serde_json::Value = r.json().await.expect("解析失败");
    assert_eq!(data["status"], serde_json::json!("ok"));

    // 4. 重复用户名 → 409
    let r = client
        .post(format!("{}/api/auth/users", base))
        .header("cookie", &cookie)
        .json(&serde_json::json!({"username": "user1", "password": "pass-12345", "role": "user"}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::CONFLICT);

    // 5. 用户列表 → 2 个（root + user1），响应不含密码哈希，需改密标记正确
    let r = client
        .get(format!("{}/api/auth/users", base))
        .header("cookie", &cookie)
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let body_text = r.text().await.expect("读取失败");
    assert!(!body_text.contains("password_hash"), "响应不得泄露密码哈希字段");
    let list: serde_json::Value = serde_json::from_str(&body_text).expect("解析失败");
    assert_eq!(list["users"].as_array().map(|a| a.len()), Some(2));
    assert_eq!(list["users"][1]["username"], serde_json::json!("user1"));
    assert_eq!(list["users"][1]["must_change_password"], serde_json::json!(true));

    // 6. user1 用所设密码登录成功
    let r = client
        .post(format!("{}/api/auth/login", base))
        .json(&serde_json::json!({"username": "user1", "password": "pass-12345"}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::OK);

    // 7. 删除自己（admin）→ 400 拒绝
    let r = client
        .delete(format!("{}/api/auth/users/root", base))
        .header("cookie", &cookie)
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);

    // 8. 删除 user1 → ok；再删除 → 404
    let r = client
        .delete(format!("{}/api/auth/users/user1", base))
        .header("cookie", &cookie)
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let r = client
        .delete(format!("{}/api/auth/users/user1", base))
        .header("cookie", &cookie)
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::NOT_FOUND);

    // 9. user1 会话已被级联注销，再登录 → 用户不存在 → 401
    let r = client
        .post(format!("{}/api/auth/login", base))
        .json(&serde_json::json!({"username": "user1", "password": "pass-12345"}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 10. 白名单：非法 IP → 400
    let r = client
        .post(format!("{}/api/auth/whitelist", base))
        .header("cookie", &cookie)
        .json(&serde_json::json!({"ips": ["not-an-ip"]}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);

    // 11. 合法白名单（含当前 loopback，不影响后续请求）→ ok 并持久化
    let r = client
        .post(format!("{}/api/auth/whitelist", base))
        .header("cookie", &cookie)
        .json(&serde_json::json!({"ips": ["127.0.0.1"]}))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let r = client
        .get(format!("{}/api/auth/whitelist", base))
        .header("cookie", &cookie)
        .send().await.expect("请求失败");
    let wl: serde_json::Value = r.json().await.expect("解析失败");
    assert_eq!(wl["whitelist"], serde_json::json!(["127.0.0.1"]));
}

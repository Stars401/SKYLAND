//! 待办事项 API — 部分合并更新语义 / 通知字段契约 集成测试
//! 覆盖：单字段 PUT 不破坏其余字段（前端 updateLastNotified 依赖）、
//! notify_time 空字符串清除契约（is_empty → None）、完成后停用通知状态。

use skyland_web::create_router;
use std::net::SocketAddr;
use std::time::Duration;

#[tokio::test]
async fn todo_partial_update_semantics() {
    let tmp = tempfile::tempdir().expect("创建临时目录失败");
    let work_dir = tmp.path().to_path_buf();
    // 数据目录钉在临时目录 — create_router 内部 datadir::resolve 会读真实 %APPDATA%，测试必须隔离
    std::env::set_var("SKYLAND_DATA_DIR", work_dir.display().to_string());
    let app = create_router(work_dir.clone(), None, "standalone");

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

    // 用运行时日期，避免未来运行时被 is_expired_by_date 误标过期
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    // 1. 创建带完整通知配置的事项
    let r = client
        .post(format!("{}/api/todo/{}/item", base, today))
        .json(&serde_json::json!({
            "title": "原始标题", "content": "内容",
            "notify_enabled": true, "notify_times": ["09:00"],
            "notify_time": "09:00", "advance_minutes": [10],
            "repeat_enabled": true, "repeat_interval": 15,
            "priority": 2
        }))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let data: serde_json::Value = r.json().await.expect("解析失败");
    assert_eq!(data["status"], serde_json::json!("ok"));
    let id = data["item"]["id"].as_str().expect("缺少 id").to_string();

    // 2. 单字段更新 last_notified —— 其余字段必须原样保留（部分合并语义）
    let r = client
        .put(format!("{}/api/todo/{}/item/{}", base, today, id))
        .json(&serde_json::json!({ "last_notified": 123456 }))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::OK);

    let r = client.get(format!("{}/api/todo/{}", base, today)).send().await.expect("请求失败");
    let list: serde_json::Value = r.json().await.expect("解析失败");
    let item = &list["list"]["items"][0];
    assert_eq!(item["title"], serde_json::json!("原始标题"));
    assert_eq!(item["notify_times"], serde_json::json!(["09:00"]));
    assert_eq!(item["advance_minutes"], serde_json::json!([10]));
    assert_eq!(item["repeat_interval"], serde_json::json!(15));
    assert_eq!(item["priority"], serde_json::json!(2));
    assert_eq!(item["last_notified"], serde_json::json!(123456));

    // 3. notify_time 空字符串 → 清除为 null（前端契约: is_empty→None）
    let r = client
        .put(format!("{}/api/todo/{}/item/{}", base, today, id))
        .json(&serde_json::json!({ "notify_time": "" }))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::OK);

    let r = client.get(format!("{}/api/todo/{}", base, today)).send().await.expect("请求失败");
    let list: serde_json::Value = r.json().await.expect("解析失败");
    assert_eq!(list["list"]["items"][0]["notify_time"], serde_json::json!(null));

    // 4. 完成 → expired 与 last_notified 清除（停用后续提醒）
    let r = client
        .put(format!("{}/api/todo/{}/item/{}", base, today, id))
        .json(&serde_json::json!({ "done": true }))
        .send().await.expect("请求失败");
    assert_eq!(r.status(), reqwest::StatusCode::OK);

    let r = client.get(format!("{}/api/todo/{}", base, today)).send().await.expect("请求失败");
    let list: serde_json::Value = r.json().await.expect("解析失败");
    assert_eq!(list["list"]["items"][0]["done"], serde_json::json!(true));
    assert_eq!(list["list"]["items"][0]["last_notified"], serde_json::json!(null));

    // 5. 数据已持久化到磁盘（ToDoList/{date}.json）
    let persisted = std::fs::read_to_string(work_dir.join("users/local/todo").join(format!("{}.json", today)))
        .expect("数据文件未生成");
    assert!(persisted.contains("原始标题"));
}

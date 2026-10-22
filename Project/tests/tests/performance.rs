//! 性能测试 - 响应时间、解析吞吐、并发能力

use std::time::Instant;
use skyland_lib::handoff::board::BoardParser;
use skyland_lib::handoff::session::SessionParser;

// ─────────── 响应时间测试 ───────────

#[test]
fn test_board_parse_response_time() {
    let content = generate_board_content(50);
    let start = Instant::now();
    let doc = BoardParser::parse(&content).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(doc.tasks.len(), 50);
    // 响应时间应在合理范围内 (debug模式放宽到200ms)
    assert!(elapsed.as_millis() < 200, "Board解析 {}ms 超阈值", elapsed.as_millis());
}

#[test]
fn test_session_parse_response_time() {
    let content = generate_session_content();
    let start = Instant::now();
    let doc = SessionParser::parse(&content).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(doc.session_id, "TEST_0001");
    // 解析应在合理时间内完成 (debug模式放宽到50ms)
    assert!(elapsed.as_millis() < 50, "Session解析 {}ms 超阈值", elapsed.as_millis());
}

#[test]
fn test_board_parse_throughput() {
    let content = generate_board_content(200);
    let start = Instant::now();
    let doc = BoardParser::parse(&content).unwrap();
    let elapsed = start.elapsed().as_secs_f64();

    assert_eq!(doc.tasks.len(), 200);
    let tasks_per_sec = 200.0 / elapsed.max(0.001);
    // 至少每秒解析500个任务
    assert!(tasks_per_sec > 500.0, "吞吐量仅 {:.0} tasks/s", tasks_per_sec);
}

// ─────────── 并发能力测试 ───────────

#[test]
fn test_concurrent_parse_operations() {
    let content = generate_board_content(100);
    let content = std::sync::Arc::new(content);

    let threads: Vec<_> = (0..10)
        .map(|_| {
            let c = content.clone();
            std::thread::spawn(move || BoardParser::parse(&c).unwrap())
        })
        .collect();

    for t in threads {
        let doc = t.join().unwrap();
        assert_eq!(doc.tasks.len(), 100);
    }
}

// ─────────── gRPC响应延迟 (需要mock server) ───────────

#[tokio::test]
async fn test_grpc_health_response_latency() {
    use skyland_tests::common::mock_server::start_mock_server;
    use skyland_lib::skyland::{skyland_service_client::SkylandServiceClient, HealthCheckRequest};
    use tonic::Request;

    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();

    let start = Instant::now();
    let resp = client.health_check(Request::new(HealthCheckRequest {})).await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(resp.into_inner().status, "healthy");
    // gRPC本地回环应在50ms内
    assert!(elapsed.as_millis() < 50, "gRPC HealthCheck {}ms > 50ms", elapsed.as_millis());

    handle.abort();
}

#[tokio::test]
async fn test_grpc_task_board_latency() {
    use skyland_tests::common::mock_server::start_mock_server;
    use skyland_lib::skyland::{skyland_service_client::SkylandServiceClient, GetTaskBoardRequest};
    use tonic::Request;

    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();

    let start = Instant::now();
    let resp = client.get_task_board(Request::new(GetTaskBoardRequest {})).await.unwrap();
    let elapsed = start.elapsed();

    assert!(!resp.into_inner().tasks.is_empty());
    assert!(elapsed.as_millis() < 50, "gRPC TaskBoard {}ms > 50ms", elapsed.as_millis());

    handle.abort();
}

#[tokio::test]
async fn test_grpc_concurrent_requests() {
    use skyland_tests::common::mock_server::start_mock_server;
    use skyland_lib::skyland::{skyland_service_client::SkylandServiceClient, HealthCheckRequest};
    use tonic::Request;
    use std::sync::Arc;
    use tokio::sync::Barrier;

    let (handle, port) = start_mock_server().await.unwrap();
    let addr = Arc::new(format!("http://127.0.0.1:{}", port));
    let barrier = Arc::new(Barrier::new(10));

    let mut handles = vec![];
    for _ in 0..10 {
        let a = addr.clone();
        let b = barrier.clone();
        handles.push(tokio::spawn(async move {
            b.wait().await;
            let mut client = SkylandServiceClient::connect((*a).clone()).await.unwrap();
            client.health_check(Request::new(HealthCheckRequest {})).await.unwrap()
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    handle.abort();
}

// ─────────── 测试数据生成器 ───────────

fn generate_board_content(task_count: usize) -> String {
    let mut content = String::from("## 任务列表\n\n| 任务ID | 任务描述 | 负责人 | 状态 | 创建时间 | 更新时间 | 备注 |\n|--------|----------|--------|------|----------|----------|------|\n");
    for i in 1..=task_count {
        content.push_str(&format!(
            "| T{:04} | 测试任务{} | code_agent | ✅ | 2026-05-06 | 2026-05-06 | |\n",
            i, i
        ));
    }
    content
}

fn generate_session_content() -> String {
    "# SESSION_TEST_0001\n\n## 元信息\n- **会话ID**: TEST_0001\n- **创建时间**: 2026-05-06 00:00:00\n- **创建者**: code_agent\n- **状态**: 待审查\n\n## 任务描述\n### 任务目标\n性能测试任务\n\n## 代码生成摘要\n### 生成内容\n- test.rs\n".to_string()
}

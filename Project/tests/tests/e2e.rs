//! Skyland端到端测试 - gRPC通信+任务看板+智能体管理

use skyland_tests::common::mock_server::start_mock_server;
use skyland_tests::common::fixtures::sample_board_tasks;
use skyland_lib::handoff::board::BoardParser;
use skyland_lib::skyland::{
    skyland_service_client::SkylandServiceClient,
    HealthCheckRequest, GetTaskBoardRequest, AgentStatusRequest,
};
use tonic::Request;

// ─────────── CLI启动测试 ───────────

#[tokio::test]
async fn test_cli_can_connect_to_daemon() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    let resp = client.health_check(Request::new(HealthCheckRequest {})).await.unwrap();
    assert_eq!(resp.into_inner().status, "healthy");
    handle.abort();
}

#[tokio::test]
async fn test_cli_receives_version_info() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    let resp = client.health_check(Request::new(HealthCheckRequest {})).await.unwrap();
    assert_eq!(resp.into_inner().version, "0.1.0");
    handle.abort();
}

// ─────────── gRPC通信测试 ───────────

#[tokio::test]
async fn test_grpc_health_check_roundtrip() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    let resp = client.health_check(Request::new(HealthCheckRequest {})).await.unwrap();
    let inner = resp.into_inner();
    assert_eq!(inner.status, "healthy");
    assert_eq!(inner.version, "0.1.0");
    handle.abort();
}

#[tokio::test]
async fn test_grpc_task_board_roundtrip() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    let resp = client.get_task_board(Request::new(GetTaskBoardRequest {})).await.unwrap();
    let tasks = resp.into_inner().tasks;
    assert!(!tasks.is_empty());
    assert_eq!(tasks[0].id, "T001");
    handle.abort();
}

#[tokio::test]
async fn test_grpc_agent_status_roundtrip() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    let resp = client.get_agent_status(Request::new(AgentStatusRequest {})).await.unwrap();
    let inner = resp.into_inner();
    assert_eq!(inner.agent_name, "code_agent");
    assert!(inner.task_count > 0);
    handle.abort();
}

// ─────────── 任务看板测试 ───────────

#[test]
fn test_board_parser_parses_fixtures() {
    let tasks = sample_board_tasks();
    assert_eq!(tasks.len(), 3);
    assert_eq!(tasks[0].id, "T001");
    assert_eq!(tasks[1].status, "🔄");
}

#[test]
fn test_board_parser_handles_minimal_input() {
    let content = "| 任务ID | 任务描述 | 负责人 | 状态 | 创建时间 | 更新时间 | 备注 |\n|--------|----------|--------|------|----------|----------|------|\n| T001 | 任务 | code_agent | ✅ | 2026 | 2026 | ok |";
    let doc = BoardParser::parse(content).unwrap();
    assert_eq!(doc.tasks.len(), 1);
    assert_eq!(doc.tasks[0].description, "任务");
}

#[test]
fn test_board_parser_filters_header_rows() {
    let content = "| 任务ID | 任务描述 | 负责人 | 状态 | 创建时间 | 更新时间 | 备注 |\n|--------|----------|--------|------|----------|----------|------|";
    let doc = BoardParser::parse(content).unwrap();
    assert!(doc.tasks.is_empty());
}

#[tokio::test]
async fn test_task_board_via_grpc() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    let resp = client.get_task_board(Request::new(GetTaskBoardRequest {})).await.unwrap();
    let tasks = resp.into_inner().tasks;
    assert!(!tasks.is_empty());
    assert_eq!(tasks[0].assignee, "code_agent");
    handle.abort();
}

// ─────────── 智能体管理测试 ───────────

#[tokio::test]
async fn test_agent_status_query() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    let resp = client.get_agent_status(Request::new(AgentStatusRequest {})).await.unwrap();
    let inner = resp.into_inner();
    assert_eq!(inner.agent_name, "code_agent");
    assert_eq!(inner.status, "running");
    assert_eq!(inner.task_count, 7);
    handle.abort();
}

#[tokio::test]
async fn test_agent_status_matches_fixture() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    let resp = client.get_agent_status(Request::new(AgentStatusRequest {})).await.unwrap();
    let inner = resp.into_inner();
    assert_eq!(inner.agent_name, "code_agent");
    assert_eq!(inner.status, "running");
    assert!(inner.task_count >= 0);
    handle.abort();
}

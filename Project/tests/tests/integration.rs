//! 系统集成测试 - 模块间协作和数据流验证

use skyland_tests::common::mock_server::start_mock_server;
use skyland_tests::common::helpers::{setup_test_environment, create_test_handoff_docs};
use skyland_lib::skyland::{
    skyland_service_client::SkylandServiceClient,
    HealthCheckRequest, GetTaskBoardRequest, AgentStatusRequest,
};
use skyland_lib::handoff::board::BoardParser;
use skyland_lib::handoff::session::SessionParser;
use skyland_lib::handoff::agents::AgentsParser;
use skyland_lib::types::TaskStatus;
use tonic::Request;

// ─────────── Daemon服务器集成测试 ───────────

#[tokio::test]
async fn test_daemon_health_check_sequence() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    for _ in 0..3 {
        let resp = client.health_check(Request::new(HealthCheckRequest {})).await.unwrap();
        assert_eq!(resp.into_inner().status, "healthy");
    }
    handle.abort();
}

#[tokio::test]
async fn test_multiple_client_connections() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut c1 = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    let mut c2 = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    assert_eq!(c1.health_check(Request::new(HealthCheckRequest {})).await.unwrap().into_inner().status, "healthy");
    assert_eq!(c2.health_check(Request::new(HealthCheckRequest {})).await.unwrap().into_inner().status, "healthy");
    handle.abort();
}

#[tokio::test]
async fn test_task_board_consistency() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    let r1 = client.get_task_board(Request::new(GetTaskBoardRequest {})).await.unwrap();
    let r2 = client.get_task_board(Request::new(GetTaskBoardRequest {})).await.unwrap();
    assert_eq!(r1.into_inner().tasks.len(), r2.into_inner().tasks.len());
    handle.abort();
}

// ─────────── CLI-Daemon集成测试 ───────────

#[tokio::test]
async fn test_full_cli_daemon_interaction() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();

    let health = client.health_check(Request::new(HealthCheckRequest {})).await.unwrap();
    assert_eq!(health.into_inner().status, "healthy");

    let board = client.get_task_board(Request::new(GetTaskBoardRequest {})).await.unwrap();
    assert!(!board.into_inner().tasks.is_empty());

    let agent = client.get_agent_status(Request::new(AgentStatusRequest {})).await.unwrap();
    assert_eq!(agent.into_inner().agent_name, "code_agent");

    handle.abort();
}

#[tokio::test]
async fn test_cli_reconnection() {
    let (handle, port) = start_mock_server().await.unwrap();
    let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    assert!(client.health_check(Request::new(HealthCheckRequest {})).await.is_ok());
    drop(client);

    let mut client2 = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port)).await.unwrap();
    assert!(client2.health_check(Request::new(HealthCheckRequest {})).await.is_ok());
    handle.abort();
}

#[tokio::test]
async fn test_cli_invalid_address_error() {
    let result = SkylandServiceClient::connect("http://invalid.addr.99999:1").await;
    assert!(result.is_err());
}

// ─────────── 交接流程集成测试 ───────────

#[test]
fn test_handoff_document_roundtrip() {
    let (dir, base) = setup_test_environment().unwrap();
    create_test_handoff_docs(&base).unwrap();

    let board = BoardParser::parse(&std::fs::read_to_string(base.join("handoff/董事会.md")).unwrap()).unwrap();
    assert_eq!(board.tasks.len(), 2);
    assert_eq!(board.tasks[0].id, "T001");

    let session = SessionParser::parse(&std::fs::read_to_string(base.join("handoff/SESSION_TEST_0001.md")).unwrap()).unwrap();
    assert_eq!(session.session_id, "TEST_0001");

    let stats = board.statistics.unwrap();
    assert_eq!(stats.done_count, 1);
    assert_eq!(stats.total_count, 2);

    drop(dir);
}

#[test]
fn test_task_status_transitions() {
    assert_eq!(TaskStatus::from_str("📋"), TaskStatus::Todo);
    assert_eq!(TaskStatus::from_str("🔄"), TaskStatus::InProgress);
    assert_eq!(TaskStatus::from_str("👀"), TaskStatus::Reviewing);
    assert_eq!(TaskStatus::from_str("✅"), TaskStatus::Done);
    assert_eq!(TaskStatus::from_str("🚫"), TaskStatus::Blocked);
}

#[test]
fn test_agents_spec_basic() {
    let doc = AgentsParser::parse("## 文档元信息\n- **版本**: 1.1.0\n\n### 2.1 代码智能体 (code_agent)\n**核心职责**：\n- 代码生成与实现").unwrap();
    assert_eq!(doc.version, "1.1.0");
    assert!(!doc.agent_roles.is_empty());
}

// ─────────── 智能体生命周期集成测试 ───────────

#[test]
fn test_agent_manager_lifecycle_integration() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let base = temp_dir.path().to_str().unwrap().to_string();
    let mut mgr = skyland_daemon::agent::AgentManager::new(base);

    assert_eq!(mgr.running_count(), 0);
    assert!(mgr.list_agents().is_empty());

    let pid = mgr.start_agent("code_agent").unwrap();
    assert!(pid > 0);
    assert_eq!(mgr.running_count(), 1);

    mgr.stop_agent("code_agent").unwrap();
    assert_eq!(mgr.running_count(), 0);
}

#[test]
fn test_fs_manager_integration() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let base = temp_dir.path().to_str().unwrap().to_string();
    let mgr = skyland_daemon::fs::FileSystemManager::new(base.clone());

    mgr.initialize_directories().unwrap();
    assert!(std::path::Path::new(&format!("{}/agents/code_agent", base)).exists());
    assert!(std::path::Path::new(&format!("{}/handoff", base)).exists());
}

#[test]
fn test_thread_agent_spawn_stop() {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        loop {
            if rx.recv_timeout(std::time::Duration::from_millis(100)).is_ok() { break; }
        }
    });
    tx.send(()).unwrap();
    handle.join().unwrap();
}

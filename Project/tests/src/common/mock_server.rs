//! 模拟gRPC服务器 - 用于集成测试

use skyland_lib::skyland::{
    skyland_service_server::{SkylandService, SkylandServiceServer},
    HealthCheckRequest, HealthCheckResponse,
    GetTaskBoardRequest, GetTaskBoardResponse, TaskInfo,
    AgentStatusRequest, AgentStatusResponse,
};
use tonic::{Request, Response, Status, transport::Server};

/// 可配置的模拟 SkylandService
pub struct MockSkylandService {
    pub health_status: String,
    pub tasks: Vec<TaskInfo>,
    pub agent_name: String,
    pub agent_status: String,
    pub task_count: i32,
}

impl Default for MockSkylandService {
    fn default() -> Self {
        Self {
            health_status: "healthy".into(),
            tasks: vec![
                TaskInfo {
                    id: "T001".into(),
                    description: "测试任务".into(),
                    assignee: "code_agent".into(),
                    status: "Done".into(),
                }
            ],
            agent_name: "code_agent".into(),
            agent_status: "running".into(),
            task_count: 7,
        }
    }
}

#[tonic::async_trait]
impl SkylandService for MockSkylandService {
    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        Ok(Response::new(HealthCheckResponse {
            status: self.health_status.clone(),
            version: "0.1.0".into(),
        }))
    }

    async fn get_task_board(
        &self,
        _request: Request<GetTaskBoardRequest>,
    ) -> Result<Response<GetTaskBoardResponse>, Status> {
        Ok(Response::new(GetTaskBoardResponse {
            tasks: self.tasks.clone(),
        }))
    }

    async fn get_agent_status(
        &self,
        _request: Request<AgentStatusRequest>,
    ) -> Result<Response<AgentStatusResponse>, Status> {
        Ok(Response::new(AgentStatusResponse {
            agent_name: self.agent_name.clone(),
            status: self.agent_status.clone(),
            task_count: self.task_count,
        }))
    }
}

/// 启动模拟gRPC服务器，返回服务器句柄和端口号
pub async fn start_mock_server() -> anyhow::Result<(tokio::task::JoinHandle<()>, u16)> {
    let service = SkylandServiceServer::new(MockSkylandService::default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let handle = tokio::spawn(async move {
        Server::builder()
            .add_service(service)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .ok();
    });

    // Wait for server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    Ok((handle, port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use skyland_lib::skyland::skyland_service_client::SkylandServiceClient;

    #[tokio::test]
    async fn test_mock_server_health() {
        let (handle, port) = start_mock_server().await.unwrap();

        let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port))
            .await
            .unwrap();

        let resp = client.health_check(Request::new(HealthCheckRequest {}))
            .await
            .unwrap();
        assert_eq!(resp.into_inner().status, "healthy");

        handle.abort();
    }

    #[tokio::test]
    async fn test_mock_server_tasks() {
        let (handle, port) = start_mock_server().await.unwrap();

        let mut client = SkylandServiceClient::connect(format!("http://127.0.0.1:{}", port))
            .await
            .unwrap();

        let resp = client.get_task_board(Request::new(GetTaskBoardRequest {}))
            .await
            .unwrap();
        let tasks = resp.into_inner().tasks;
        assert!(!tasks.is_empty());
        assert_eq!(tasks[0].id, "T001");

        handle.abort();
    }
}

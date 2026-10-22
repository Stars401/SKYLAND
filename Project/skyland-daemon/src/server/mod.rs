//! gRPC服务器模块

use std::fs;
use skyland_lib::skyland::{
    skyland_service_server::{SkylandService, SkylandServiceServer},
    HealthCheckRequest, HealthCheckResponse,
    GetTaskBoardRequest, GetTaskBoardResponse, TaskInfo,
    AgentStatusRequest, AgentStatusResponse,
};
use skyland_lib::handoff::board::BoardParser;
use tonic::{Request, Response, Status};
use crate::agent::AgentManager;
use std::sync::{Arc, Mutex};

pub struct SkylandServiceImpl {
    pub handoff_dir: String,
    pub agent_manager: Arc<Mutex<AgentManager>>,
}

#[tonic::async_trait]
impl SkylandService for SkylandServiceImpl {
    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        Ok(Response::new(HealthCheckResponse {
            status: "healthy".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        }))
    }

    async fn get_task_board(
        &self,
        _request: Request<GetTaskBoardRequest>,
    ) -> Result<Response<GetTaskBoardResponse>, Status> {
        let board_path = std::path::Path::new(&self.handoff_dir).join("董事会.md");
        let content = fs::read_to_string(&board_path)
            .map_err(|e| Status::internal(format!("Failed to read task board: {}", e)))?;

        let doc = BoardParser::parse(&content)
            .map_err(|e| Status::internal(format!("Parse error: {}", e)))?;

        let task_infos: Vec<TaskInfo> = doc.tasks
            .into_iter()
            .map(|t| TaskInfo {
                id: t.id,
                description: t.description,
                assignee: t.assignee,
                status: t.status,
            })
            .collect();

        Ok(Response::new(GetTaskBoardResponse { tasks: task_infos }))
    }

    async fn get_agent_status(
        &self,
        _request: Request<AgentStatusRequest>,
    ) -> Result<Response<AgentStatusResponse>, Status> {
        let mgr = self.agent_manager.lock()
            .map_err(|e| Status::internal(format!("Lock error: {}", e)))?;

        let agents = mgr.list_agents();
        let task_count = mgr.running_count() as i32;
        let agent_name = agents.first().map(|a| a.name.clone()).unwrap_or_else(|| "code_agent".into());
        let status_text = if agents.is_empty() { "idle" } else { "running" };

        Ok(Response::new(AgentStatusResponse {
            agent_name,
            status: status_text.into(),
            task_count,
        }))
    }
}

pub fn create_service(handoff_dir: String, agent_manager: Arc<Mutex<AgentManager>>) -> SkylandServiceServer<SkylandServiceImpl> {
    SkylandServiceServer::new(SkylandServiceImpl { handoff_dir, agent_manager })
}

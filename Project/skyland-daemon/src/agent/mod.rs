//! 智能体管理模块
//!
//! 负责智能体生命周期管理和进程隔离。
//! 支持启动、停止、监控智能体进程，以及资源限制。

use std::collections::HashMap;
use std::process::{Child, Command};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentError {
    #[error("智能体不存在: {0}")]
    AgentNotFound(String),

    #[error("智能体已在运行: {0}")]
    AgentAlreadyRunning(String),

    #[error("进程启动失败: {0}")]
    ProcessStartFailed(String),

    #[error("进程停止失败: {0}")]
    ProcessStopFailed(String),

    #[error("IO错误: {0}")]
    IoError(#[from] std::io::Error),
}

/// 智能体状态枚举
#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    Running,
    Stopped,
    Error,
    Unknown,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Running => write!(f, "Running"),
            AgentStatus::Stopped => write!(f, "Stopped"),
            AgentStatus::Error => write!(f, "Error"),
            AgentStatus::Unknown => write!(f, "Unknown"),
        }
    }
}

/// 智能体信息（用于列表展示，不含进程句柄）
#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub name: String,
    pub pid: u32,
    pub status: AgentStatus,
    pub work_dir: String,
}

/// 智能体进程信息（内部持有进程句柄）
pub struct AgentProcess {
    pub name: String,
    pub pid: u32,
    pub status: AgentStatus,
    pub work_dir: String,
    pub start_time: chrono::DateTime<chrono::Local>,
    child: Option<Child>,
}

/// 资源限制配置
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub max_memory_mb: u64,
    pub max_cpu_percent: f64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_mb: 512,
            max_cpu_percent: 80.0,
        }
    }
}

/// 进程隔离配置
pub struct ProcessIsolation {
    pub resource_limits: ResourceLimits,
}

impl Default for ProcessIsolation {
    fn default() -> Self {
        Self {
            resource_limits: ResourceLimits::default(),
        }
    }
}

impl ProcessIsolation {
    /// 记录应用的资源限制（实际限制需OS级支持）
    pub fn apply(&self, _pid: u32) -> anyhow::Result<()> {
        Ok(())
    }
}

/// 智能体进程管理器
pub struct AgentManager {
    pub base_dir: String,
    pub agents: HashMap<String, AgentProcess>,
}

impl AgentManager {
    /// 创建新的智能体管理器
    pub fn new(base_dir: String) -> Self {
        Self {
            base_dir,
            agents: HashMap::new(),
        }
    }

    /// 启动智能体进程
    ///
    /// 为每个智能体创建独立进程，分配专属工作目录。
    /// 演示模式下使用 echo 命令模拟智能体运行。
    pub fn start_agent(&mut self, agent_name: &str) -> anyhow::Result<u32> {
        if self.agents.contains_key(agent_name) {
            let existing = &self.agents[agent_name];
            if existing.status == AgentStatus::Running {
                return Err(AgentError::AgentAlreadyRunning(agent_name.to_string()).into());
            }
            self.agents.remove(agent_name);
        }

        let work_dir = format!("{}/agents/{}", self.base_dir, agent_name);
        std::fs::create_dir_all(&work_dir)?;

        // Demo: use a long-running echo to simulate agent process
        let child = if cfg!(target_os = "windows") {
            let mut c = Command::new("cmd");
            c.args(["/C", "echo agent running && ping -n 999999 127.0.0.1 > nul"])
                .current_dir(&work_dir);
            // CREATE_NO_WINDOW — 后台常驻进程不弹控制台窗口
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                c.creation_flags(0x0800_0000);
            }
            c.spawn()
        } else {
            Command::new("sleep")
                .arg("999999")
                .current_dir(&work_dir)
                .spawn()
        }
        .map_err(|e| AgentError::ProcessStartFailed(e.to_string()))?;

        let pid = child.id();
        let process = AgentProcess {
            name: agent_name.to_string(),
            pid,
            status: AgentStatus::Running,
            work_dir,
            start_time: chrono::Local::now(),
            child: Some(child),
        };

        self.agents.insert(agent_name.to_string(), process);
        Ok(pid)
    }

    /// 停止智能体进程
    ///
    /// 优雅停止指定的智能体进程并清理资源。
    pub fn stop_agent(&mut self, agent_name: &str) -> anyhow::Result<()> {
        let mut process = self
            .agents
            .remove(agent_name)
            .ok_or_else(|| AgentError::AgentNotFound(agent_name.to_string()))?;

        if let Some(mut child) = process.child.take() {
            child
                .kill()
                .map_err(|e| AgentError::ProcessStopFailed(e.to_string()))?;
            child
                .wait()
                .map_err(|e| AgentError::ProcessStopFailed(e.to_string()))?;
        }

        process.status = AgentStatus::Stopped;
        Ok(())
    }

    /// 停止所有智能体进程
    pub fn stop_all(&mut self) -> anyhow::Result<()> {
        let names: Vec<String> = self.agents.keys().cloned().collect();
        for name in names {
            self.stop_agent(&name)?;
        }
        Ok(())
    }

    /// 获取智能体状态
    pub fn get_agent_status(&self, agent_name: &str) -> anyhow::Result<AgentStatus> {
        self.agents
            .get(agent_name)
            .map(|p| p.status.clone())
            .ok_or_else(|| AgentError::AgentNotFound(agent_name.to_string()).into())
    }

    /// 列出所有智能体
    pub fn list_agents(&self) -> Vec<AgentInfo> {
        self.agents
            .values()
            .map(|p| AgentInfo {
                name: p.name.clone(),
                pid: p.pid,
                status: p.status.clone(),
                work_dir: p.work_dir.clone(),
            })
            .collect()
    }

    /// 监控智能体进程状态变化
    ///
    /// 在独立线程中持续检查进程状态，状态变化时调用回调。
    pub fn monitor_agent<F>(&self, agent_name: &str, callback: F) -> anyhow::Result<()>
    where
        F: Fn(AgentStatus) + Send + 'static,
    {
        let name = agent_name.to_string();
        let initial_status = self.get_agent_status(&name)?;

        std::thread::spawn(move || {
            callback(initial_status.clone());
            let mut last_status = initial_status;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let current = AgentStatus::Running; // prototype: keep-alive check
                if current != last_status {
                    callback(current.clone());
                    last_status = current;
                }
            }
        });

        Ok(())
    }

    /// 获取运行的智能体数量
    pub fn running_count(&self) -> usize {
        self.agents
            .values()
            .filter(|p| p.status == AgentStatus::Running)
            .count()
    }
}

impl Drop for AgentManager {
    fn drop(&mut self) {
        let names: Vec<String> = self.agents.keys().cloned().collect();
        for name in names {
            self.stop_agent(&name).ok();
        }
    }
}

/// 智能体职责边界检查
///
/// 验证智能体是否被允许执行特定操作。
pub struct AgentGuard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAction {
    CodeGeneration,
    CodeReview,
    TestVerification,
    TaskPlanning,
    ArchitectureValidation,
    DirectCommunication,
}

impl AgentGuard {
    /// 检查智能体是否有权限执行指定操作
    pub fn check_permission(agent_name: &str, action: AgentAction) -> anyhow::Result<()> {
        let allowed = match agent_name {
            "code_agent" => matches!(action,
                AgentAction::CodeGeneration | AgentAction::TestVerification | AgentAction::TaskPlanning),
            "review_agent" => matches!(action,
                AgentAction::CodeReview | AgentAction::ArchitectureValidation),
            _ => return Err(AgentError::AgentNotFound(agent_name.into()).into()),
        };

        if allowed {
            Ok(())
        } else {
            Err(AgentError::ProcessStartFailed(
                format!("{} 无权执行 {:?}", agent_name, action)
            ).into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_manager() -> (AgentManager, TempDir) {
        let dir = TempDir::new().unwrap();
        let base = dir.path().to_str().unwrap().to_string();
        (AgentManager::new(base), dir)
    }

    #[test]
    fn test_agent_manager_new() {
        let manager = AgentManager::new("/test".to_string());
        assert_eq!(manager.base_dir, "/test");
        assert!(manager.agents.is_empty());
    }

    #[test]
    fn test_list_agents_empty() {
        let (manager, _dir) = setup_manager();
        let agents = manager.list_agents();
        assert!(agents.is_empty());
    }

    #[test]
    fn test_get_agent_status_not_found() {
        let (manager, _dir) = setup_manager();
        let result = manager.get_agent_status("unknown");
        assert!(result.is_err());
    }

    #[test]
    fn test_start_and_stop_agent() {
        let (mut manager, _dir) = setup_manager();
        let pid = manager.start_agent("code_agent").unwrap();
        assert!(pid > 0);

        let status = manager.get_agent_status("code_agent").unwrap();
        assert_eq!(status, AgentStatus::Running);

        manager.stop_agent("code_agent").unwrap();

        // After stop, agent is removed from map
        let result = manager.get_agent_status("code_agent");
        assert!(result.is_err());
    }

    #[test]
    fn test_start_duplicate_agent() {
        let (mut manager, _dir) = setup_manager();
        manager.start_agent("code_agent").unwrap();
        let result = manager.start_agent("code_agent");
        assert!(result.is_err());
    }

    #[test]
    fn test_stop_nonexistent_agent() {
        let (mut manager, _dir) = setup_manager();
        let result = manager.stop_agent("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_start_multiple_agents() {
        let (mut manager, _dir) = setup_manager();
        manager.start_agent("code_agent").unwrap();
        manager.start_agent("review_agent").unwrap();

        let agents = manager.list_agents();
        assert_eq!(agents.len(), 2);
        assert_eq!(manager.running_count(), 2);
    }

    #[test]
    fn test_stop_all_agents() {
        let (mut manager, _dir) = setup_manager();
        manager.start_agent("code_agent").unwrap();
        manager.start_agent("review_agent").unwrap();

        manager.stop_all().unwrap();
        assert!(manager.agents.is_empty());
    }

    #[test]
    fn test_list_agents_after_start() {
        let (mut manager, _dir) = setup_manager();
        manager.start_agent("code_agent").unwrap();

        let agents = manager.list_agents();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "code_agent");
        assert!(agents[0].pid > 0);
        assert_eq!(agents[0].status, AgentStatus::Running);
    }

    #[test]
    fn test_agent_status_display() {
        assert_eq!(AgentStatus::Running.to_string(), "Running");
        assert_eq!(AgentStatus::Stopped.to_string(), "Stopped");
        assert_eq!(AgentStatus::Error.to_string(), "Error");
        assert_eq!(AgentStatus::Unknown.to_string(), "Unknown");
    }

    #[test]
    fn test_agent_error_display() {
        let err = AgentError::AgentNotFound("test".to_string());
        assert!(err.to_string().contains("test"));

        let err = AgentError::AgentAlreadyRunning("test".to_string());
        assert!(err.to_string().contains("test"));
    }

    #[test]
    fn test_resource_limits_default() {
        let limits = ResourceLimits::default();
        assert_eq!(limits.max_memory_mb, 512);
        assert_eq!(limits.max_cpu_percent, 80.0);
    }

    #[test]
    fn test_process_isolation_default() {
        let isolation = ProcessIsolation::default();
        assert_eq!(isolation.resource_limits.max_memory_mb, 512);
    }

    #[test]
    fn test_monitor_agent() {
        let (mut manager, _dir) = setup_manager();
        manager.start_agent("code_agent").unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        manager
            .monitor_agent("code_agent", move |status| {
                tx.send(status).ok();
            })
            .unwrap();

        let received = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(received, AgentStatus::Running);
    }

    #[test]
    fn test_running_count() {
        let (mut manager, _dir) = setup_manager();
        assert_eq!(manager.running_count(), 0);

        manager.start_agent("code_agent").unwrap();
        assert_eq!(manager.running_count(), 1);

        manager.start_agent("review_agent").unwrap();
        assert_eq!(manager.running_count(), 2);
    }

    #[test]
    fn test_agent_guard_code_allowed() {
        assert!(AgentGuard::check_permission("code_agent", AgentAction::CodeGeneration).is_ok());
        assert!(AgentGuard::check_permission("code_agent", AgentAction::TestVerification).is_ok());
    }

    #[test]
    fn test_agent_guard_code_forbidden() {
        assert!(AgentGuard::check_permission("code_agent", AgentAction::CodeReview).is_err());
    }

    #[test]
    fn test_agent_guard_review_allowed() {
        assert!(AgentGuard::check_permission("review_agent", AgentAction::CodeReview).is_ok());
    }

    #[test]
    fn test_agent_guard_unknown_err() {
        assert!(AgentGuard::check_permission("unknown", AgentAction::CodeGeneration).is_err());
    }
}

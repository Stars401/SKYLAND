//! 测试数据fixtures

use skyland_lib::handoff::board::BoardTask;
use skyland_lib::types::{Task, TaskStatus};

/// 创建标准测试任务
pub fn make_task(id: &str, desc: &str, status: TaskStatus) -> Task {
    Task {
        id: id.to_string(),
        description: desc.to_string(),
        assignee: "code_agent".to_string(),
        status,
        priority: "P0".to_string(),
    }
}

/// 创建标准BoardTask
pub fn make_board_task(id: &str, desc: &str, status: &str) -> BoardTask {
    BoardTask {
        id: id.to_string(),
        description: desc.to_string(),
        assignee: "code_agent".to_string(),
        status: status.to_string(),
        created_at: "2026-05-06".to_string(),
        updated_at: "2026-05-06".to_string(),
        notes: String::new(),
    }
}

/// 创建任务列表的fixture
pub fn sample_tasks() -> Vec<Task> {
    vec![
        make_task("T001", "初始化系统", TaskStatus::Done),
        make_task("T002", "实现核心功能", TaskStatus::InProgress),
        make_task("T003", "编写测试", TaskStatus::Todo),
    ]
}

/// 创建BoardTask列表fixture
pub fn sample_board_tasks() -> Vec<BoardTask> {
    vec![
        make_board_task("T001", "初始化系统", "✅"),
        make_board_task("T002", "实现核心功能", "🔄"),
        make_board_task("T003", "编写测试", "📋"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_task() {
        let t = make_task("T001", "测试", TaskStatus::Done);
        assert_eq!(t.id, "T001");
        assert_eq!(t.description, "测试");
        assert_eq!(t.status, TaskStatus::Done);
    }

    #[test]
    fn test_sample_tasks_count() {
        let tasks = sample_tasks();
        assert_eq!(tasks.len(), 3);
    }
}

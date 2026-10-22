//! 共享类型定义模块

use serde::{Deserialize, Serialize};

/// 任务状态枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Todo,
    InProgress,
    Reviewing,
    Done,
    Blocked,
}

impl TaskStatus {
    pub fn from_str(s: &str) -> Self {
        match s {
            "📋" | "待办" => TaskStatus::Todo,
            "🔄" | "进行中" => TaskStatus::InProgress,
            "👀" | "待审查" | "🔍" | "审查中" => TaskStatus::Reviewing,
            "✅" | "已完成" => TaskStatus::Done,
            "🚫" | "已阻塞" => TaskStatus::Blocked,
            _ => TaskStatus::Todo,
        }
    }

    pub fn to_emoji(&self) -> &'static str {
        match self {
            TaskStatus::Todo => "📋",
            TaskStatus::InProgress => "🔄",
            TaskStatus::Reviewing => "👀",
            TaskStatus::Done => "✅",
            TaskStatus::Blocked => "🚫",
        }
    }
}

/// 任务信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub assignee: String,
    pub status: TaskStatus,
    pub priority: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_status_from_str_todo() {
        assert_eq!(TaskStatus::from_str("📋"), TaskStatus::Todo);
        assert_eq!(TaskStatus::from_str("待办"), TaskStatus::Todo);
    }

    #[test]
    fn test_task_status_from_str_in_progress() {
        assert_eq!(TaskStatus::from_str("🔄"), TaskStatus::InProgress);
        assert_eq!(TaskStatus::from_str("进行中"), TaskStatus::InProgress);
    }

    #[test]
    fn test_task_status_from_str_reviewing() {
        assert_eq!(TaskStatus::from_str("👀"), TaskStatus::Reviewing);
        assert_eq!(TaskStatus::from_str("待审查"), TaskStatus::Reviewing);
        assert_eq!(TaskStatus::from_str("🔍"), TaskStatus::Reviewing);
        assert_eq!(TaskStatus::from_str("审查中"), TaskStatus::Reviewing);
    }

    #[test]
    fn test_task_status_from_str_done() {
        assert_eq!(TaskStatus::from_str("✅"), TaskStatus::Done);
        assert_eq!(TaskStatus::from_str("已完成"), TaskStatus::Done);
    }

    #[test]
    fn test_task_status_from_str_blocked() {
        assert_eq!(TaskStatus::from_str("🚫"), TaskStatus::Blocked);
        assert_eq!(TaskStatus::from_str("已阻塞"), TaskStatus::Blocked);
    }

    #[test]
    fn test_task_status_from_str_unknown() {
        assert_eq!(TaskStatus::from_str("未知状态"), TaskStatus::Todo);
        assert_eq!(TaskStatus::from_str(""), TaskStatus::Todo);
    }

    #[test]
    fn test_task_status_to_emoji() {
        assert_eq!(TaskStatus::Todo.to_emoji(), "📋");
        assert_eq!(TaskStatus::InProgress.to_emoji(), "🔄");
        assert_eq!(TaskStatus::Reviewing.to_emoji(), "👀");
        assert_eq!(TaskStatus::Done.to_emoji(), "✅");
        assert_eq!(TaskStatus::Blocked.to_emoji(), "🚫");
    }
}

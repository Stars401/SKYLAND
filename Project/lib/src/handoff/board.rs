//! 董事会解析器

use serde::{Deserialize, Serialize};
use super::ParseError;
use regex::Regex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardTask {
    pub id: String,
    pub description: String,
    pub assignee: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardStatistics {
    pub todo_count: u32,
    pub in_progress_count: u32,
    pub pending_review_count: u32,
    pub done_count: u32,
    pub blocked_count: u32,
    pub total_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardMilestone {
    pub name: String,
    pub target_date: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardDocument {
    pub tasks: Vec<BoardTask>,
    pub statistics: Option<BoardStatistics>,
    pub milestones: Vec<BoardMilestone>,
}

pub struct BoardParser;

impl BoardParser {
    pub fn parse(content: &str) -> Result<BoardDocument, ParseError> {
        Ok(BoardDocument {
            tasks: Self::parse_tasks(content),
            statistics: Self::parse_statistics(content),
            milestones: Self::parse_milestones(content),
        })
    }

    fn parse_tasks(content: &str) -> Vec<BoardTask> {
        let re = Regex::new(
            r"\|\s*(T\d+)\s*\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|\s*([^|]*?)\s*\|\s*([^|]*?)\s*\|\s*([^|]*?)\s*\|"
        ).unwrap();

        re.captures_iter(content)
            .filter_map(|caps| {
                let id = caps.get(1)?.as_str().trim().to_string();
                if id == "任务ID" { return None; }
                Some(BoardTask {
                    id,
                    description: caps.get(2).map_or(String::new(), |m| m.as_str().trim().to_string()),
                    assignee: caps.get(3).map_or(String::new(), |m| m.as_str().trim().to_string()),
                    status: caps.get(4).map_or(String::new(), |m| m.as_str().trim().to_string()),
                    created_at: caps.get(5).map_or(String::new(), |m| m.as_str().trim().to_string()),
                    updated_at: caps.get(6).map_or(String::new(), |m| m.as_str().trim().to_string()),
                    notes: caps.get(7).map_or(String::new(), |m| m.as_str().trim().to_string()),
                })
            })
            .collect()
    }

    fn parse_statistics(content: &str) -> Option<BoardStatistics> {
        let keys = ["待办", "进行中", "待审查", "已完成", "已阻塞", "总计"];
        let mut counts: Vec<Option<u32>> = vec![None; 6];

        for (i, key) in keys.iter().enumerate() {
            // Match table rows like: | 待办 | 2 | 13% | or | **总计** | **15** | **100%** |
            let pattern = format!(
                r"\|\s*\**{}\**\s*\|\s*\**(\d+)\**\s*\|",
                regex::escape(key)
            );
            if let Ok(re) = Regex::new(&pattern) {
                if let Some(caps) = re.captures(content) {
                    counts[i] = caps.get(1).and_then(|m| m.as_str().parse().ok());
                }
            }
        }

        Some(BoardStatistics {
            todo_count: counts[0]?,
            in_progress_count: counts[1]?,
            pending_review_count: counts[2]?,
            done_count: counts[3]?,
            blocked_count: counts[4]?,
            total_count: counts[5]?,
        })
    }

    fn parse_milestones(content: &str) -> Vec<BoardMilestone> {
        let re = Regex::new(
            r"###\s*里程碑(\d+):\s*([^\n]+)[\s\S]*?\*\*目标时间\*\*:\s*([^\n]+)[\s\S]*?\*\*状态\*\*:\s*(\S+)"
        ).unwrap();

        re.captures_iter(content)
            .map(|caps| BoardMilestone {
                name: format!("里程碑{}: {}", caps.get(1).unwrap().as_str(), caps.get(2).unwrap().as_str().trim()),
                target_date: caps.get(3).unwrap().as_str().trim().to_string(),
                status: caps.get(4).unwrap().as_str().trim().to_string(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        "## 当前任务列表\n\n### 高优先级 (P0)\n\n",
        "| 任务ID | 任务描述 | 负责人 | 状态 | 创建时间 | 更新时间 | 备注 |\n",
        "|--------|----------|--------|------|----------|----------|------|\n",
        "| T001 | 系统目录结构初始化 | code_agent | ✅ | 2026-05-06 | 2026-05-06 | 已完成 |\n",
        "| T007 | Project目录结构初始化 | code_agent | ✅ | 2026-05-06 | 2026-05-06 | 已完成 |\n\n",
        "## 任务统计\n\n",
        "| 状态 | 数量 | 占比 |\n",
        "|------|------|------|\n",
        "| 待办 | 2 | 13% |\n",
        "| 进行中 | 0 | 0% |\n",
        "| 待审查 | 5 | 33% |\n",
        "| 已完成 | 8 | 53% |\n",
        "| 已阻塞 | 0 | 0% |\n",
        "| **总计** | **15** | **100%** |\n\n",
        "### 里程碑1: 基础架构搭建\n- **目标时间**: 2026-05-06\n- **状态**: ✅ 已完成\n",
    );

    #[test]
    fn test_parse_tasks() {
        let doc = BoardParser::parse(SAMPLE).unwrap();
        assert!(!doc.tasks.is_empty());
        let t1 = doc.tasks.iter().find(|t| t.id == "T001").unwrap();
        assert_eq!(t1.description, "系统目录结构初始化");
    }

    #[test]
    fn test_parse_stats() {
        let doc = BoardParser::parse(SAMPLE).unwrap();
        let s = doc.statistics.unwrap();
        assert_eq!(s.todo_count, 2);
        assert_eq!(s.done_count, 8);
        assert_eq!(s.total_count, 15);
    }

    #[test]
    fn test_parse_milestones() {
        let doc = BoardParser::parse(SAMPLE).unwrap();
        assert!(!doc.milestones.is_empty());
        assert!(doc.milestones[0].name.contains("基础架构搭建"));
        assert_eq!(doc.milestones[0].target_date, "2026-05-06");
    }

    #[test]
    fn test_parse_empty() {
        let doc = BoardParser::parse("").unwrap();
        assert!(doc.tasks.is_empty());
        assert!(doc.statistics.is_none());
        assert!(doc.milestones.is_empty());
    }

    #[test]
    fn test_filter_header() {
        let doc = BoardParser::parse(
            "| 任务ID | 任务描述 | 负责人 | 状态 | 创建时间 | 更新时间 | 备注 |\n|--------|----------|--------|------|----------|----------|------|\n| T001 | 测试 | code_agent | ✅ | 2026-05-06 | 2026-05-06 | 备注 |"
        ).unwrap();
        assert_eq!(doc.tasks.len(), 1);
        assert_eq!(doc.tasks[0].id, "T001");
    }
}

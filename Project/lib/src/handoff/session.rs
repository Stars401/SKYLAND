//! SESSION文档解析器

use serde::{Deserialize, Serialize};
use super::ParseError;
use regex::Regex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDocument {
    pub session_id: String,
    pub created_at: String,
    pub creator: String,
    pub status: String,
    pub task_description: Option<String>,
    pub code_summary: Option<String>,
    pub test_results: Option<String>,
    pub review_suggestions: Option<String>,
}

pub struct SessionParser;

impl SessionParser {
    pub fn parse(content: &str) -> Result<SessionDocument, ParseError> {
        let session_id = Self::extract_field(content, r"\*\*会话ID\*\*:\s*(\S+)")
            .ok_or_else(|| ParseError::MissingField("会话ID".into()))?;
        let created_at = Self::extract_field(content, r"\*\*创建时间\*\*:\s*(.+)").unwrap_or_default();
        let creator = Self::extract_field(content, r"\*\*创建者\*\*:\s*(\S+)")
            .ok_or_else(|| ParseError::MissingField("创建者".into()))?;
        let status = Self::extract_field(content, r"\*\*状态\*\*:\s*(\S+)").unwrap_or_default();
        let task_description = Self::extract_section(content, "## 任务描述");
        let code_summary = Self::extract_section(content, "## 代码生成摘要");
        let test_results = Self::extract_section(content, "## 测试结果");
        let review_suggestions = Self::extract_section(content, "## 审查建议");

        Ok(SessionDocument {
            session_id, created_at, creator, status,
            task_description, code_summary, test_results, review_suggestions,
        })
    }

    fn extract_field(content: &str, pattern: &str) -> Option<String> {
        let re = Regex::new(pattern).ok()?;
        re.captures(content)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().trim().to_string())
    }

    fn extract_section(content: &str, header: &str) -> Option<String> {
        let pos = content.find(header)?;
        let after_header = &content[pos + header.len()..];
        let next_h2 = after_header.find("\n## ");
        let section_text = match next_h2 {
            Some(end) => &after_header[..end],
            None => after_header,
        };
        let trimmed = section_text.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# SESSION_20260506_0041\n\n## 元信息\n- **会话ID**: 20260506_0041\n- **创建时间**: 2026-05-06 00:41:00\n- **创建者**: code_agent\n- **状态**: 待审查\n\n## 任务描述\n### 任务目标\n实现文件系统隔离模块\n\n## 代码生成摘要\n### 生成内容\n- fs/mod.rs\n- tests/fs_integration.rs\n\n## 测试结果\n- 单元测试: 19 passed\n- 集成测试: 4 passed\n\n## 审查建议\n无问题\n";

    #[test]
    fn test_parse_basic() {
        let doc = SessionParser::parse(SAMPLE).unwrap();
        assert_eq!(doc.session_id, "20260506_0041");
        assert_eq!(doc.creator, "code_agent");
        assert_eq!(doc.status, "待审查");
    }

    #[test]
    fn test_parse_task_desc() {
        let doc = SessionParser::parse(SAMPLE).unwrap();
        assert!(doc.task_description.unwrap().contains("文件系统隔离"));
    }

    #[test]
    fn test_parse_code_summary() {
        let doc = SessionParser::parse(SAMPLE).unwrap();
        let s = doc.code_summary.unwrap();
        assert!(s.contains("fs/mod.rs") && s.contains("fs_integration.rs"));
    }

    #[test]
    fn test_parse_test_results() {
        let doc = SessionParser::parse(SAMPLE).unwrap();
        let r = doc.test_results.unwrap();
        assert!(r.contains("19 passed") && r.contains("4 passed"));
    }

    #[test]
    fn test_parse_review() {
        let doc = SessionParser::parse(SAMPLE).unwrap();
        assert!(doc.review_suggestions.unwrap().contains("无问题"));
    }

    #[test]
    fn test_parse_empty() {
        assert!(SessionParser::parse("").is_err());
    }

    #[test]
    fn test_parse_minimal() {
        let doc = SessionParser::parse("## 元信息\n- **会话ID**: S_001\n- **创建者**: test").unwrap();
        assert_eq!(doc.session_id, "S_001");
        assert_eq!(doc.creator, "test");
    }

    #[test]
    fn test_extract_field() {
        let r = SessionParser::extract_field("- **key**: val123", r"\*\*key\*\*:\s*(\S+)");
        assert_eq!(r, Some("val123".into()));
    }

    #[test]
    fn test_extract_section() {
        let r = SessionParser::extract_section("## A\nhello\n\n## B\nworld", "## A");
        assert_eq!(r, Some("hello".into()));
    }
}

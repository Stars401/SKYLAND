//! AGENTS规范解析器

use serde::{Deserialize, Serialize};
use super::ParseError;
use regex::Regex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRole {
    pub name: String,
    pub responsibilities: Vec<String>,
    pub prohibited_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentsDocument {
    pub version: String,
    pub system_architecture: Option<String>,
    pub agent_roles: Vec<AgentRole>,
    pub handoff_rules: Option<String>,
    pub memory_system: Option<String>,
    pub quality_standards: Option<String>,
}

pub struct AgentsParser;

impl AgentsParser {
    pub fn parse(content: &str) -> Result<AgentsDocument, ParseError> {
        Ok(AgentsDocument {
            version: Self::extract_field(content, r"\*\*版本\*\*:\s*(\S+)").unwrap_or_default(),
            system_architecture: Self::section(content, "## 1. 系统架构概述"),
            agent_roles: Self::parse_agent_roles(content),
            handoff_rules: Self::section(content, "## 3. 交接文档规范"),
            memory_system: Self::section(content, "## 4. 记忆系统规范"),
            quality_standards: Self::section(content, "## 6. 质量保障规范"),
        })
    }

    fn parse_agent_roles(content: &str) -> Vec<AgentRole> {
        let mut roles = Vec::new();
        let names = ["code_agent", "review_agent"];

        for &name in &names {
            // Find position of agent name and extract duties list
            if let Some(pos) = content.find(name) {
                let after = &content[pos + name.len()..];
                // Find "核心职责" after the agent name
                if let Some(duty_pos) = after.find("核心职责") {
                    let after_duty = &after[duty_pos..];
                    // Extract bullet points until next section or end
                    let duties: Vec<String> = after_duty
                        .lines()
                        .skip(1) // skip the "核心职责" line itself
                        .take_while(|l| l.trim().starts_with("-") || l.trim().is_empty())
                        .filter(|l| l.trim().starts_with("-"))
                        .map(|l| l.trim().trim_start_matches("- ").to_string())
                        .collect();

                    if !duties.is_empty() {
                        roles.push(AgentRole {
                            name: name.to_string(),
                            responsibilities: duties,
                            prohibited_actions: Vec::new(),
                        });
                    }
                }
            }
        }
        roles
    }

    fn extract_field(content: &str, pattern: &str) -> Option<String> {
        Regex::new(pattern).ok()?
            .captures(content)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
    }

    fn section(content: &str, header: &str) -> Option<String> {
        let pos = content.find(header)?;
        let after = &content[pos + header.len()..];
        let end = after.find("\n## ");
        let text = match end {
            Some(e) => &after[..e],
            None => after,
        }.trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        "## 文档元信息\n- **版本**: 1.0.0\n\n",
        "## 1. 系统架构概述\n\n核心组件包括 skyland-daemon 和 skyland-web。\n\n",
        "## 2. 智能体职责规范\n\n",
        "### 2.1 代码智能体 (code_agent)\n**核心职责**：\n- 代码生成与实现\n- 测试可行性验证\n\n",
        "### 2.2 审查智能体 (review_agent)\n**核心职责**：\n- 代码全局审查\n- 架构合规性验证\n\n",
        "## 3. 交接文档规范\n\n会话文档: SESSION_YYYYMMDD_HHMMSS.md\n\n",
        "## 4. 记忆系统规范\n\n短期记忆和长期记忆的维护机制。\n\n",
        "## 6. 质量保障规范\n\n代码审查和测试覆盖标准。\n",
    );

    #[test]
    fn test_version() {
        let doc = AgentsParser::parse(SAMPLE).unwrap();
        assert_eq!(doc.version, "1.0.0");
    }

    #[test]
    fn test_roles_exist() {
        let doc = AgentsParser::parse(SAMPLE).unwrap();
        assert!(!doc.agent_roles.is_empty());
    }

    #[test]
    fn test_code_agent_duties() {
        let doc = AgentsParser::parse(SAMPLE).unwrap();
        let code = doc.agent_roles.iter().find(|r| r.name == "code_agent").unwrap();
        let duties: Vec<&str> = code.responsibilities.iter().map(|s| s.as_str()).collect();
        assert!(duties.iter().any(|d| d.contains("代码生成")));
    }

    #[test]
    fn test_architecture() {
        let doc = AgentsParser::parse(SAMPLE).unwrap();
        let arch = doc.system_architecture.unwrap();
        assert!(arch.contains("skyland-daemon"));
    }

    #[test]
    fn test_handoff() {
        let doc = AgentsParser::parse(SAMPLE).unwrap();
        let h = doc.handoff_rules.unwrap();
        assert!(h.contains("SESSION_YYYYMMDD_HHMMSS"));
    }

    #[test]
    fn test_memory() {
        let doc = AgentsParser::parse(SAMPLE).unwrap();
        assert!(doc.memory_system.is_some());
    }

    #[test]
    fn test_empty() {
        let doc = AgentsParser::parse("").unwrap();
        assert_eq!(doc.version, "");
        assert!(doc.agent_roles.is_empty());
    }
}

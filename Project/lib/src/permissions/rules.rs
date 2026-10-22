//! 权限规则和配置

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PermissionMode {
    Default,
    Auto,
    Plan,
    Bypass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub tool: String,
    pub pattern: Option<String>,
    pub decision: PermissionDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionConfig {
    pub mode: PermissionMode,
    pub always_allow: Vec<PermissionRule>,
    pub always_deny: Vec<PermissionRule>,
    pub always_ask: Vec<PermissionRule>,
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            mode: PermissionMode::Default,
            always_allow: vec![
                PermissionRule { tool: "read_file".into(), pattern: None, decision: PermissionDecision::Allow },
                PermissionRule { tool: "glob".into(), pattern: None, decision: PermissionDecision::Allow },
                PermissionRule { tool: "grep".into(), pattern: None, decision: PermissionDecision::Allow },
            ],
            always_deny: vec![
                PermissionRule { tool: "bash".into(), pattern: Some("rm -rf".into()), decision: PermissionDecision::Deny },
            ],
            always_ask: vec![
                PermissionRule { tool: "bash".into(), pattern: None, decision: PermissionDecision::Ask },
                PermissionRule { tool: "write_file".into(), pattern: None, decision: PermissionDecision::Ask },
                PermissionRule { tool: "edit_file".into(), pattern: None, decision: PermissionDecision::Ask },
            ],
        }
    }
}

/// 判断工具是否为只读操作
pub fn is_read_only_tool(name: &str) -> bool {
    matches!(name, "read_file" | "glob" | "grep")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_read_only() {
        assert!(is_read_only_tool("read_file"));
        assert!(is_read_only_tool("glob"));
        assert!(!is_read_only_tool("bash"));
        assert!(!is_read_only_tool("write_file"));
    }

    #[test]
    fn test_default_config() {
        let cfg = PermissionConfig::default();
        assert!(matches!(cfg.mode, PermissionMode::Default));
        assert!(!cfg.always_allow.is_empty());
        assert!(!cfg.always_deny.is_empty());
    }
}

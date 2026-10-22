//! 权限检查器

use super::rules::{is_read_only_tool, PermissionConfig, PermissionDecision, PermissionMode};
use super::classifier::auto_classify;

pub struct PermissionChecker {
    config: PermissionConfig,
}

impl PermissionChecker {
    pub fn new(config: PermissionConfig) -> Self { Self { config } }

    pub fn check(&self, tool_name: &str, args: &str) -> PermissionDecision {
        if matches!(self.config.mode, PermissionMode::Bypass) {
            return PermissionDecision::Allow;
        }
        if matches!(self.config.mode, PermissionMode::Plan) {
            return if is_read_only_tool(tool_name) { PermissionDecision::Allow } else { PermissionDecision::Deny };
        }
        for rule in &self.config.always_deny {
            if rule.tool == tool_name {
                if let Some(ref pat) = rule.pattern {
                    if args.contains(pat) { return PermissionDecision::Deny; }
                } else {
                    return PermissionDecision::Deny;
                }
            }
        }
        for rule in &self.config.always_allow {
            if rule.tool == tool_name && rule.pattern.is_none() {
                return PermissionDecision::Allow;
            }
        }
        match self.config.mode {
            PermissionMode::Default | PermissionMode::Auto => auto_classify(tool_name, args),
            _ => PermissionDecision::Ask,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bypass_allows_all() {
        let mut cfg = PermissionConfig::default();
        cfg.mode = PermissionMode::Bypass;
        let checker = PermissionChecker::new(cfg);
        assert_eq!(checker.check("bash", "rm -rf /"), PermissionDecision::Allow);
        assert_eq!(checker.check("write_file", "anything"), PermissionDecision::Allow);
    }

    #[test]
    fn test_plan_denies_write() {
        let mut cfg = PermissionConfig::default();
        cfg.mode = PermissionMode::Plan;
        let checker = PermissionChecker::new(cfg);
        assert_eq!(checker.check("read_file", ""), PermissionDecision::Allow);
        assert_eq!(checker.check("write_file", ""), PermissionDecision::Deny);
        assert_eq!(checker.check("bash", "ls"), PermissionDecision::Deny);
    }

    #[test]
    fn test_default_deny_dangerous() {
        let cfg = PermissionConfig::default();
        let checker = PermissionChecker::new(cfg);
        assert_eq!(checker.check("bash", "rm -rf /tmp"), PermissionDecision::Deny);
        assert_eq!(checker.check("bash", "ls -la"), PermissionDecision::Allow);
    }

    #[test]
    fn test_always_allow_readonly() {
        let cfg = PermissionConfig::default();
        let checker = PermissionChecker::new(cfg);
        assert_eq!(checker.check("grep", "pattern src/"), PermissionDecision::Allow);
        assert_eq!(checker.check("glob", "**/*.rs"), PermissionDecision::Allow);
    }
}

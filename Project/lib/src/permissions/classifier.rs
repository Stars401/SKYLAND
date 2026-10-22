//! 自动分类器 - 基于规则的安全分类

use super::rules::PermissionDecision;

pub fn auto_classify(tool_name: &str, args: &str) -> PermissionDecision {
    match tool_name {
        "bash" => classify_bash(args),
        "write_file" | "edit_file" => classify_file_write(args),
        "read_file" | "glob" | "grep" => PermissionDecision::Allow,
        _ => PermissionDecision::Ask,
    }
}

fn classify_bash(args: &str) -> PermissionDecision {
    let dangerous = ["rm -rf", "rm -r /", "mkfs", "dd if=", ">/dev/",
                     "chmod 777", "chown", "sudo ", "su -", "git push --force"];
    for p in &dangerous {
        if args.contains(p) { return PermissionDecision::Deny; }
    }
    let readonly = ["ls", "cat", "head", "tail", "grep", "find", "echo", "pwd", "wc", "du", "df"];
    for cmd in &readonly {
        if args.starts_with(cmd) { return PermissionDecision::Allow; }
    }
    PermissionDecision::Ask
}

fn classify_file_write(args: &str) -> PermissionDecision {
    if args.starts_with("/etc") || args.starts_with("/root") || args.contains(".env") {
        return PermissionDecision::Deny;
    }
    PermissionDecision::Ask
}

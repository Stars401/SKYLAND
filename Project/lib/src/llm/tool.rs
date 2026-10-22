//! 工具定义 — AI可调用的文件操作和命令执行工具

use serde::{Deserialize, Serialize};

/// 工具定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// 工具执行结果
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub output: String,
    pub success: bool,
}

impl Tool {
    pub fn new(name: &str, description: &str, parameters: serde_json::Value) -> Self {
        Self { name: name.into(), description: description.into(), parameters }
    }
}

/// 获取默认工具集
pub fn default_tools() -> Vec<Tool> {
    vec![
        bash_tool(),
        read_file_tool(),
        write_file_tool(),
        edit_file_tool(),
        glob_tool(),
        grep_tool(),
        web_search_tool(),
        spawn_agent_tool(),
    ]
}

/// 根据权限模式返回可用工具集
/// - "readonly": 只读工具（read_file, glob, grep, web_search, spawn_agent）
/// - "edit": 读 + 写工具（不含 bash）
/// - "auto" / 其他: 全部工具
pub fn tools_for_mode(mode: &str) -> Vec<Tool> {
    match mode {
        "readonly" => vec![read_file_tool(), glob_tool(), grep_tool(), web_search_tool(), spawn_agent_tool()],
        "edit" => vec![read_file_tool(), write_file_tool(), edit_file_tool(), bash_tool(), glob_tool(), grep_tool(), web_search_tool(), spawn_agent_tool()],
        _ => default_tools(),
    }
}

/// web_search 工具定义
pub fn web_search_tool() -> Tool {
    Tool::new("web_search", "搜索互联网获取最新信息。返回搜索结果摘要和链接。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "搜索关键词" },
                "max_results": { "type": "integer", "description": "最大结果数（可选，默认5）" }
            },
            "required": ["query"]
        })
    )
}

/// spawn_agent 工具 — 派生子智能体执行子任务

pub fn spawn_agent_tool() -> Tool {

    Tool::new("spawn_agent", "派生一个子智能体去独立完成一项子任务，并拿回它的结果。子智能体拥有独立上下文与只读工具（读文件/查找/搜索），会多轮调用工具直到完成。适合：① 可拆分并行调研的复杂任务（同轮可派生多个并行执行）② 你不想让大量中间过程占用主对话上下文的调研。用法：name 起简短角色名（如 researcher/api-reviewer），task 写清楚完整、自包含的子任务描述（子智能体看不到你们的对话，必须能独立理解），subagent_type 按任务性质选型。",

        serde_json::json!({

            "type": "object",

            "properties": {

                "name": { "type": "string", "description": "智能体名称，如 researcher/writer/reviewer" },

                "task": { "type": "string", "description": "自包含的子任务描述（子智能体看不到主对话）" },

                "subagent_type": { "type": "string", "enum": ["explore", "researcher", "general-purpose"], "description": "子代理类型：explore=快速代码检索定位（只要 file:line 结论）/ researcher=联网查证调研 / general-purpose=通用只读调研（默认）" }

            },

            "required": ["name", "task"]

        }))

}

pub fn bash_tool() -> Tool {
    Tool::new("bash", "执行shell命令并返回输出。30秒超时。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "要执行的shell命令" }
            },
            "required": ["command"]
        })
    )
}

pub fn read_file_tool() -> Tool {
    Tool::new("read_file", "读取文件内容。大文件自动截断。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "文件路径" },
                "offset": { "type": "integer", "description": "起始行号（可选）" },
                "limit": { "type": "integer", "description": "最大读取行数（可选）" }
            },
            "required": ["path"]
        })
    )
}

pub fn write_file_tool() -> Tool {
    Tool::new("write_file", "写入文件（覆盖已有内容）。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "文件路径" },
                "content": { "type": "string", "description": "要写入的内容" }
            },
            "required": ["path", "content"]
        })
    )
}

pub fn edit_file_tool() -> Tool {
    Tool::new("edit_file", "查找并替换文件中的文本（仅替换第一处匹配）。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "文件路径" },
                "old": { "type": "string", "description": "要查找的文本" },
                "new": { "type": "string", "description": "替换后的文本" }
            },
            "required": ["path", "old", "new"]
        })
    )
}

pub fn glob_tool() -> Tool {
    Tool::new("glob", "用glob模式查找文件，如 **/*.rs",
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "glob模式，如 src/**/*.rs" }
            },
            "required": ["pattern"]
        })
    )
}

pub fn grep_tool() -> Tool {
    Tool::new("grep", "用正则表达式搜索文件内容",
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "正则表达式" },
                "path": { "type": "string", "description": "搜索目录（可选，默认当前目录）" }
            },
            "required": ["pattern"]
        })
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_tools_count() {
        let tools = default_tools();
        assert_eq!(tools.len(), 8); // bash, read_file, write_file, edit_file, glob, grep, web_search, spawn_agent
    }

    #[test]
    fn test_tool_names_unique() {
        let tools = default_tools();
        let names: Vec<_> = tools.iter().map(|t| &t.name).collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len());
    }

    #[test]
    fn test_descriptions_are_chinese() {
        let tools = default_tools();
        for t in &tools {
            assert!(!t.description.is_empty(), "工具 {} 描述为空", t.name);
        }
    }

    #[test]
    fn test_tools_for_readonly() {
        let tools = tools_for_mode("readonly");
        assert_eq!(tools.len(), 5); // read_file, glob, grep, web_search, spawn_agent
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"glob"));
        assert!(names.contains(&"grep"));
        assert!(names.contains(&"web_search"));
        assert!(names.contains(&"spawn_agent"));
        assert!(!names.contains(&"bash"));
        assert!(!names.contains(&"write_file"));
    }

    #[test]
    fn test_tools_for_edit() {
        let tools = tools_for_mode("edit");
        // edit 模式含 bash（所有编辑与命令均经上层授权闸门逐项审批）
        assert_eq!(tools.len(), 8); // read_file, write_file, edit_file, bash, glob, grep, web_search, spawn_agent
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"edit_file"));
        assert!(names.contains(&"bash"));
        assert!(names.contains(&"web_search"));
        assert!(names.contains(&"spawn_agent"));
    }

    #[test]
    fn test_tools_for_auto() {
        let tools = tools_for_mode("auto");
        assert_eq!(tools.len(), 8);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"bash"));
        assert!(names.contains(&"web_search"));
        assert!(names.contains(&"spawn_agent"));
    }
}

//! 共享工具模块

/// 解析Markdown表格行，提取单元格内容
pub fn parse_table_row(line: &str) -> Vec<String> {
    line.split('|')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_table_row_normal() {
        let result = parse_table_row("| T001 | 任务A | code_agent | ✅ |");
        assert_eq!(result, vec!["T001", "任务A", "code_agent", "✅"]);
    }

    #[test]
    fn test_parse_table_row_empty() {
        let result = parse_table_row("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_table_row_single_cell() {
        let result = parse_table_row("| hello |");
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_parse_table_row_no_pipes() {
        let result = parse_table_row("no pipes here");
        assert_eq!(result, vec!["no pipes here"]);
    }
}

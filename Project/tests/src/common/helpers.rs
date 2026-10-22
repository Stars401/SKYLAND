//! 测试辅助函数

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// 创建临时测试环境，包含完整的目录结构
pub fn setup_test_environment() -> anyhow::Result<(TempDir, PathBuf)> {
    let dir = TempDir::new()?;
    let base = dir.path().to_path_buf();

    let dirs = [
        "agents/code_agent/logs",
        "agents/code_agent/memory",
        "agents/code_agent/task_history",
        "agents/review_agent/logs",
        "agents/review_agent/memory",
        "agents/review_agent/task_history",
        "handoff",
    ];

    for d in &dirs {
        fs::create_dir_all(base.join(d))?;
    }

    Ok((dir, base))
}

/// 创建测试用的handoff文档
pub fn create_test_handoff_docs(base: &Path) -> anyhow::Result<()> {
    let board_content = r#"## 当前任务列表

### 高优先级 (P0)

| 任务ID | 任务描述 | 负责人 | 状态 | 创建时间 | 更新时间 | 备注 |
|--------|----------|--------|------|----------|----------|------|
| T001 | 测试任务A | code_agent | ✅ | 2026-05-06 | 2026-05-06 | 已完成 |
| T002 | 测试任务B | code_agent | 🔄 | 2026-05-06 | 2026-05-06 | 进行中 |

## 任务统计

| 状态 | 数量 | 占比 |
|------|------|------|
| 待办 | 0 | 0% |
| 进行中 | 1 | 50% |
| 待审查 | 0 | 0% |
| 已完成 | 1 | 50% |
| 已阻塞 | 0 | 0% |
| **总计** | **2** | **100%** |
"#;

    let session_content = r#"# SESSION_TEST_0001

## 元信息
- **会话ID**: TEST_0001
- **创建时间**: 2026-05-06 00:00:00
- **创建者**: code_agent
- **状态**: 待审查

## 任务描述
### 任务目标
测试任务

## 代码生成摘要
### 生成内容
- test_file.rs
"#;

    fs::write(base.join("handoff/董事会.md"), board_content)?;
    fs::write(base.join("handoff/SESSION_TEST_0001.md"), session_content)?;
    Ok(())
}

/// 清理测试环境
pub fn teardown_test_environment(_dir: TempDir) {
    // TempDir自动清理
}

/// 等待异步条件满足
pub async fn wait_for<F, T>(timeout_ms: u64, mut check: F) -> anyhow::Result<T>
where
    F: FnMut() -> Option<T>,
{
    let start = std::time::Instant::now();
    loop {
        if let Some(val) = check() {
            return Ok(val);
        }
        if start.elapsed().as_millis() > timeout_ms as u128 {
            anyhow::bail!("等待超时 ({}ms)", timeout_ms);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_and_teardown() {
        let result = setup_test_environment();
        assert!(result.is_ok());
        let (dir, base) = result.unwrap();
        assert!(base.join("handoff").exists());
        assert!(base.join("agents/code_agent").exists());
        drop(dir);
    }

    #[test]
    fn test_create_handoff_docs() {
        let (dir, base) = setup_test_environment().unwrap();
        create_test_handoff_docs(&base).unwrap();
        assert!(base.join("handoff/董事会.md").exists());
        assert!(base.join("handoff/SESSION_TEST_0001.md").exists());
        drop(dir);
    }
}

use skyland_daemon::fs::FileSystemManager;
use std::fs;
use std::path::Path;

#[test]
fn test_full_isolation_workflow() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let base = temp_dir.path().to_str().unwrap().to_string();
    let manager = FileSystemManager::new(base.clone());

    // 1. 初始化目录
    let result = manager.initialize_directories();
    assert!(result.is_ok(), "初始化失败: {:?}", result.err());

    // 2. 验证code_agent目录存在
    let code_dir = format!("{}/agents/code_agent", base);
    assert!(Path::new(&code_dir).exists());
    assert!(Path::new(&format!("{}/logs", code_dir)).exists());
    assert!(Path::new(&format!("{}/memory", code_dir)).exists());
    assert!(Path::new(&format!("{}/task_history", code_dir)).exists());

    // 3. 验证review_agent目录存在
    let review_dir = format!("{}/agents/review_agent", base);
    assert!(Path::new(&review_dir).exists());

    // 4. 验证handoff目录存在
    let handoff_dir = format!("{}/handoff", base);
    assert!(Path::new(&handoff_dir).exists());

    // 5. 验证智能体隔离
    assert!(manager.verify_agent_isolation("code_agent").unwrap());
    assert!(manager.verify_agent_isolation("review_agent").unwrap());

    // 6. 验证handoff权限
    assert!(manager.verify_handoff_permissions().unwrap());

    // 7. 验证无效智能体返回false
    assert!(!manager.verify_agent_isolation("unknown").unwrap());
}

#[test]
fn test_separate_agent_directories() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let base = temp_dir.path().to_str().unwrap().to_string();
    let manager = FileSystemManager::new(base.clone());

    // Create code_agent and review_agent separately
    manager.create_agent_directory("code_agent").unwrap();
    manager.create_agent_directory("review_agent").unwrap();

    let code_dir = format!("{}/agents/code_agent", base);
    let review_dir = format!("{}/agents/review_agent", base);

    // Both should exist but be separate directories
    assert!(Path::new(&code_dir).exists());
    assert!(Path::new(&review_dir).exists());
    assert!(code_dir != review_dir);
}

#[test]
fn test_write_to_handoff_as_code_agent() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let base = temp_dir.path().to_str().unwrap().to_string();
    let manager = FileSystemManager::new(base.clone());

    manager.initialize_directories().unwrap();

    // Simulate code_agent writing to handoff
    let handoff_dir = format!("{}/handoff", base);
    let test_file = format!("{}/test_session.md", handoff_dir);
    fs::write(&test_file, "# Test").unwrap();
    assert!(Path::new(&test_file).exists());
}

#[test]
fn test_permissions_after_initialize() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let base = temp_dir.path().to_str().unwrap().to_string();
    let manager = FileSystemManager::new(base.clone());

    manager.initialize_directories().unwrap();

    // Verify all required directories exist with correct structure
    let required_dirs = vec![
        "agents/code_agent",
        "agents/code_agent/logs",
        "agents/code_agent/memory",
        "agents/code_agent/task_history",
        "agents/review_agent",
        "agents/review_agent/logs",
        "agents/review_agent/memory",
        "agents/review_agent/task_history",
        "handoff",
    ];

    for dir in &required_dirs {
        let path = format!("{}/{}", base, dir);
        assert!(Path::new(&path).exists(), "Missing directory: {}", dir);
    }
}

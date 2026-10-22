use skyland_daemon::agent::AgentManager;

#[test]
fn test_agent_full_lifecycle() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let base = temp_dir.path().to_str().unwrap().to_string();
    let mut manager = AgentManager::new(base.clone());

    // 1. Start agents
    let code_pid = manager.start_agent("code_agent").unwrap();
    let review_pid = manager.start_agent("review_agent").unwrap();
    assert!(code_pid > 0);
    assert!(review_pid > 0);
    assert_ne!(code_pid, review_pid);

    // 2. Check running count
    assert_eq!(manager.running_count(), 2);

    // 3. List agents
    let agents = manager.list_agents();
    assert_eq!(agents.len(), 2);

    // 4. Verify agent info
    let code_info = agents.iter().find(|a| a.name == "code_agent").unwrap();
    assert_eq!(code_info.pid, code_pid);
    assert!(code_info.work_dir.ends_with("code_agent"));

    // 5. Stop code_agent, review_agent still running
    manager.stop_agent("code_agent").unwrap();
    assert_eq!(manager.running_count(), 1);

    let agents = manager.list_agents();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "review_agent");

    // 6. Stop all
    manager.stop_all().unwrap();
    assert_eq!(manager.running_count(), 0);
    assert!(manager.agents.is_empty());
}

#[test]
fn test_agent_work_directory_created() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let base = temp_dir.path().to_str().unwrap().to_string();
    let mut manager = AgentManager::new(base.clone());

    manager.start_agent("code_agent").unwrap();

    let work_dir = format!("{}/agents/code_agent", base);
    assert!(std::path::Path::new(&work_dir).exists());

    manager.stop_agent("code_agent").unwrap();
}

#[test]
fn test_multiple_start_stop_cycles() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let base = temp_dir.path().to_str().unwrap().to_string();

    for _ in 0..3 {
        let mut manager = AgentManager::new(base.clone());
        manager.start_agent("code_agent").unwrap();
        assert_eq!(manager.running_count(), 1);
    } // Drop triggers cleanup
}

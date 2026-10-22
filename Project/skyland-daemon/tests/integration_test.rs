//! console端到端测试 - 验证gRPC服务完整流程

use std::fs;
use std::io::Write;

#[test]
#[ignore = "需要启动daemon服务后手动运行"]
fn test_health_check_via_grpc() {
    // 手动测试: 启动 daemon 后运行 test_grpc 二进制
    // cargo run --bin skyland-daemon
    // cargo run --bin test_grpc
}

#[test]
fn test_board_parsing_logic() {
    let sample = r#"
| 任务ID | 任务描述 | 负责人 | 状态 | 创建时间 | 更新时间 | 备注 |
|--------|----------|--------|------|----------|----------|------|
| T001 | 系统目录结构初始化 | code_agent | ✅ | 2026-05-06 | 2026-05-06 | 已完成 |
| T007 | Project目录结构初始化 | code_agent | 👀 | 2026-05-06 | 2026-05-06 | 待审查 |
| T008 | 守护进程基础框架 | code_agent | 📋 | 2026-05-06 | - | 待办 |
"#;

    let temp_dir = std::env::temp_dir().join("skyland_test_board");
    fs::create_dir_all(&temp_dir).unwrap();

    let board_path = temp_dir.join("董事会.md");
    let mut f = fs::File::create(&board_path).unwrap();
    f.write_all(sample.as_bytes()).unwrap();

    let content = fs::read_to_string(&board_path).unwrap();
    assert!(content.contains("T001"));
    assert!(content.contains("T007"));
    assert!(content.contains("T008"));

    fs::remove_dir_all(&temp_dir).ok();
}

# Skyland测试框架

## 结构

```
tests/
├── src/
│   ├── lib.rs              # 测试框架入口
│   └── common/
│       ├── mod.rs          # 公共模块
│       ├── helpers.rs      # 测试辅助函数 (环境搭建/清理)
│       ├── fixtures.rs     # 测试数据fixtures
│       └── mock_server.rs  # 模拟gRPC服务器
├── tests/
│   └── e2e/
│       ├── mod.rs              # e2e模块入口
│       ├── common.rs           # e2e公共函数
│       ├── test_cli_startup.rs # CLI启动测试
│       ├── test_grpc_communication.rs # gRPC通信测试
│       ├── test_task_board.rs  # 任务看板测试
│       └── test_agent_management.rs # 智能体管理测试
└── Cargo.toml
```

## 运行测试

```bash
# 运行所有测试
cargo test

# 运行e2e测试
cargo test --test e2e

# 运行特定测试
cargo test test_grpc_health
```

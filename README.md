# SKYLAND

AI 智能开发工作台，包含智能体对话、技术知识库和任务管理。桌面端采用 Rust、Tauri 2 与 WebView2。

## 下载

在 [GitHub Releases](https://github.com/Stars401/SKYLAND/releases) 下载 Windows x64 安装包。首次公开版本为预览版。

首次启动后自行配置模型服务与 API Key。发行包不包含开发者密钥、账号、聊天记录或个人配置。界面默认跟随系统字体，不分发商业字体文件。

## 数据

Windows 默认数据目录为 `%APPDATA%/SKYLAND`，也可在首次启动时选择目录。配置、认证信息与用户内容保存在本机数据目录，源码与安装包不携带这些文件。升级可能沿用已有本机数据。

当前预览版推荐个人单机使用。不同电脑和操作系统账号各自保存数据；同一服务上的多用户权限边界尚未经过完整安全审计，请勿直接作为公开多租户服务部署。不要向仓库提交真实密钥或运行数据。

## 源码构建

需要 Rust 稳定版、Windows C++ 构建工具、Protocol Buffers 编译器、WebView2 与 Tauri CLI 2。

```powershell
cargo install tauri-cli --version "^2"
cd Project/skyland-app
cargo tauri build --bundles nsis
```

开发时可在 `Project` 下执行 `cargo run -p skyland-web`。模型请求可能将输入发送至用户配置的模型服务，请自行选择服务商。

## 许可证

项目代码使用 [MIT License](LICENSE)。第三方依赖保留各自许可证；随附前端库包括 marked（MIT）和 highlight.js（BSD-3-Clause）。

//! 工具执行器 — 带工作目录限制、权限检查、输出截断、超时控制、备份机制

use super::provider::ToolCall;
use super::tool::ToolResult;
use crate::permissions::{PermissionChecker, PermissionConfig, PermissionDecision};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// 输出最大字符数，防止撑爆LLM context
const MAX_OUTPUT: usize = 5000;

/// bash命令超时秒数
const BASH_TIMEOUT_SECS: u64 = 30;

/// 文件大小上限 (1MB)，超过拒绝读取
const MAX_FILE_SIZE: u64 = 1_048_576;

/// 工作目录 — 所有文件操作限制在此目录内
static WORK_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 网络搜索 API Key (Tavily)
static SEARCH_API_KEY: OnceLock<String> = OnceLock::new();

/// 设置工作目录
pub fn set_work_dir(dir: impl AsRef<Path>) {
    let _ = WORK_DIR.set(dir.as_ref().to_path_buf());
}

/// 设置搜索 API Key
pub fn set_search_api_key(key: &str) {
    let _ = SEARCH_API_KEY.set(key.to_string());
}

/// 获取工作目录
fn work_dir() -> Option<&'static Path> {
    WORK_DIR.get().map(|p| p.as_path())
}

/// 请求级 base 与全局 WORK_DIR 的合成（base 优先；'static 可缩到 'a）
fn effective_dir<'a>(base: Option<&'a Path>) -> Option<&'a Path> {
    match base { Some(b) => Some(b), None => work_dir() }
}

/// 相对路径按 base 解析（base 未设置时回退 WORK_DIR；进程 CWD 与工作目录不同时，裸 fs 操作会指向错误位置）
fn resolve_path_in(path: &str, work_dir: Option<&Path>) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        match work_dir {
            Some(wd) => wd.join(p),
            None => p.to_path_buf(),
        }
    }
}

/// 截断输出 — 按字符数截断（按字节切会劈开多字节字符导致 panic）
fn truncate(output: String) -> String {
    let char_count = output.chars().count();
    if output.len() > MAX_OUTPUT {
        let head: String = output.chars().take(MAX_OUTPUT).collect();
        format!("{}\n\n... [输出已截断，共 {} 字符]", head, char_count)
    } else {
        output
    }
}

/// 从JSON参数中提取字符串字段
fn extract_arg(args: &str, field: &str) -> String {
    serde_json::from_str::<serde_json::Value>(args)
        .ok()
        .and_then(|v| v[field].as_str().map(String::from))
        .unwrap_or_else(|| args.to_string())
}

/// 路径安全校验 — 阻止访问敏感文件
fn is_path_safe(path: &str) -> bool {
    let path_lower = path.to_lowercase();
    let dangerous = [".env", ".ssh", "id_rsa", "id_ed25519", ".key", ".pem",
                     "/etc/shadow", "/etc/passwd", "/etc/sudoers",
                     "llm.toml", "credentials", ".npmrc", ".pypirc",
                     // 认证面：会话 token / 用户表 / 白名单 / 审计日志（server 模式）
                     "sessions.json", "users.json", "whitelist.json", "auth_audit"];
    for d in &dangerous {
        if path_lower.contains(d) { return false; }
    }
    if path.to_lowercase().starts_with("c:\\windows\\system32") { return false; }
    true
}

/// 解析后路径的二次校验 — 防 Windows 8.3 短名绕过（config/LLM~1.TOM 不含敏感子串，
/// canonicalize 会展开为长名）。对 join(wd)/canonicalize 后的真实路径再跑一遍黑名单。
fn is_resolved_path_safe(path: &str, wd: Option<&Path>) -> bool {
    let p = Path::new(path);
    let abs = match wd {
        Some(wd) => {
            let wd_canon = wd.canonicalize().unwrap_or_else(|_| wd.to_path_buf());
            if p.is_absolute() { p.to_path_buf() } else { wd_canon.join(p) }
        }
        None => p.to_path_buf(),
    };
    let resolved = abs.canonicalize()
        .map(|c| c.to_string_lossy().to_string())
        .unwrap_or_else(|_| lexical_normalize(&abs));
    is_path_safe(&resolved)
}

/// 请求级版本：wd 来自调用方（工作区 base 或全局 WORK_DIR）
  fn is_within_work_dir_in(path: &str, wd: Option<&Path>) -> bool {
      // URLs are never within work dir — reject immediately
      if path.starts_with("http://") || path.starts_with("https://") || path.starts_with("file://") {
          return false;
      }
      let Some(wd) = wd else { return true; }; // 未设置工作目录则放行
      // ★ 相对路径必须先按 wd 解析，再做范围检查。
      //   原实现直接对 `Path::new(path)` 做 canonicalize，而 canonicalize 以**进程 CWD**
      //   为基准，与 `resolve_path_in`（写入时按 wd 解析）不是同一个基准 →
      //   检查的路径和真正写入的路径不一致：既会把合法写入误拒（工作区在 CWD 之外时
      //   所有相对路径写入全被拒，子代理无法写文件），也存在检查放行/实际写到别处的风险。
      //   两处必须共用同一基准，所以这里显式先 join(wd)。
      let p = Path::new(path);
      // 比较基准也必须归一：Windows 上 canonicalize 会补 `\\?\` 前缀/解析 8.3 短名，
      // 直接拿原始 wd 去 starts_with 归一后的路径会误判为"越界"。
      let wd_canon = wd.canonicalize().unwrap_or_else(|_| wd.to_path_buf());
      // 目标必须挂在**同一个**归一基准上，否则两侧一边带 `\\?\` 一边不带，比不过。
      let abs_target = if p.is_absolute() { p.to_path_buf() } else { wd_canon.join(p) };

      // Try canonicalize — works for existing files
      if let Ok(canonical) = abs_target.canonicalize() {
          return canonical.starts_with(&wd_canon);
      }

      // File doesn't exist (write_file case) — resolve parent + join
      if let Some(parent) = abs_target.parent() {
          if let Ok(canonical_parent) = parent.canonicalize() {
              let full = canonical_parent.join(abs_target.file_name().unwrap_or_default());
              return full.starts_with(&wd_canon);
          }
      }

      // Last resort: string comparison (case-insensitive on Windows)
      // 同样按 wd 兜底（不再退回进程 CWD）
      // 目标可能整条路径都不存在（如 a/b/c.rs，父目录也没建），canonicalize 全部失败，
      // 此时必须做**词法归一**（消解 . 与 ..、统一分隔符），否则
      // "wd\sub\dir\b.rs" 与 "wd" 的字符串前缀比较会因分隔符混用而误判越界。
      let abs = lexical_normalize(&abs_target);
      let wd_str = wd_canon.to_string_lossy().to_string();

    // Normalize both paths for comparison
    let abs_norm = std::path::Path::new(&abs).to_string_lossy().to_lowercase();
    let wd_norm = wd_str.to_lowercase();

      abs_norm.starts_with(&wd_norm)
  }

  /// 纯词法路径归一：消解 `.` 与 `..`，统一为 `\`（Windows）/ 原样（Unix）。
  /// 不触碰文件系统 —— 用于目标尚不存在时仍能正确判定是否越界。
  /// 遇到无法回退的 `..`（如 `../../x` 越过根）时保留该段，由后续前缀比较拒绝。
  fn lexical_normalize(p: &Path) -> String {
      let mut root = String::new();
      let mut parts: Vec<String> = Vec::new();
      for comp in p.components() {
          use std::path::Component::*;
          match comp {
              CurDir => {}
              ParentDir => {
                  // 只有能安全回退时才弹出；不能回退则原样保留（后续比较会判越界）
                  match parts.last().map(|s| s.as_str()) {
                      Some("..") | None => parts.push("..".to_string()),
                      Some(_) => { parts.pop(); }
                  }
              }
              Normal(s) => parts.push(s.to_string_lossy().to_string()),
              // 根与盘符单独收集（不能 pop 回退），最后拼回去
              RootDir => root.push('\\'),
              Prefix(pre) => root = pre.as_os_str().to_string_lossy().to_string(),
          }
      }
      let joined = parts.join("\\");
      if root.is_empty() { joined } else { format!("{}\\{}", root.trim_end_matches('\\'), joined) }
  }

/// bash 命令白名单检查
/// cmd /C 会执行 `&` `;` `|` 分隔的全部命令 — 每个片段的首命令都必须在白名单内。
/// 重定向（`>`/`<`）可绕过 write_file 的路径安全检查，一律拒绝（`2>&1` fd 复制除外）。
pub fn is_bash_allowed(command: &str) -> bool {
    // 拒绝命令替换 — 防止 $(...) 和反引号绕过白名单
    if command.contains("$(") || command.contains('`') {
        return false;
    }

    // 敏感文件名扫描（整条命令）：防 cat/type/grep/cp/mv 等白名单命令把
    // llm.toml（全部 API key）、会话 token、用户表读入模型上下文或外传
    {
        let cmd_lower = command.to_lowercase();
        const SENSITIVE: &[&str] = &["llm.toml", "sessions.json", "users.json",
                                     "whitelist.json", "auth_audit", ".env", "id_rsa",
                                     "id_ed25519", "/etc/shadow", "/etc/passwd", "credentials"];
        if SENSITIVE.iter().any(|s| cmd_lower.contains(s)) {
            return false;
        }
    }

    // 重定向检查：`>` 后跟非 `&`（即写入文件）或任何 `<` 输入重定向 → 拒绝
    let bytes: Vec<char> = command.chars().collect();
    for (i, c) in bytes.iter().enumerate() {
        if *c == '>' {
            let next = bytes.get(i + 1).copied().unwrap_or(' ');
            if next != '&' {
                return false;
            }
        }
        if *c == '<' {
            return false;
        }
    }

    // 白名单
    const ALLOWED: &[&str] = &[
        "ls", "cat", "head", "tail", "grep", "find", "echo", "pwd", "wc",
        "du", "df", "whoami", "date", "which", "file", "stat",
        "git", "cargo", "rustc", "rustup",
        "node", "npm", "npx", "yarn", "pnpm",
        "python", "python3", "pip", "pip3",
        "go", "java", "javac", "mvn", "gradle",
        "curl", "wget",
        "mkdir", "cp", "mv", "touch", "tree",
        "sed", "awk", "sort", "uniq", "cut", "tr",
        "type", "where", "dir", "cd",
        // env / printenv 曾在名单内 — 可倾倒进程全部环境变量（含密钥形态的
        // provider 配置），已移除；需要环境信息时用具体命令查询单项。
    ];

    let check_cmd = |first_cmd: &str| -> bool {
        let cmd_name = first_cmd.split_whitespace().next().unwrap_or("");
        if cmd_name.is_empty() { return false; }
        let cmd_name = cmd_name.rsplit('/').next().unwrap_or(cmd_name).rsplit('\\').next().unwrap_or(cmd_name);
        if ALLOWED.contains(&cmd_name) { return true; }
        // Windows cmd 内置命令（set 会倾倒全部环境变量，不在名单）
        if cfg!(target_os = "windows") {
            const WIN_ALLOWED: &[&str] = &["dir", "type", "where", "echo", "cd"];
            if WIN_ALLOWED.contains(&cmd_name.to_lowercase().as_str()) { return true; }
        }
        false
    };

    // 参数级危险旗标（逐片段检查）：
    // git -c alias.x='!cmd' → 别名经 sh 执行任意命令（无元字符，&;| 分段检查不可见）
    // find -exec/-ok → 以任意命令处理结果
    // curl/wget -o/--output/file:// → 任意路径写、读本地文件外发（不经 `>` 重定向检查）
    let flags_ok = |seg: &str| -> bool {
        let toks: Vec<&str> = seg.split_whitespace().collect();
        let Some(raw) = toks.first() else { return true };
        let first = raw.rsplit('/').next().unwrap_or(raw).rsplit('\\').next().unwrap_or(raw);
        match first {
            "git" => !toks.iter().skip(1).any(|t| {
                *t == "-c" || t.starts_with("--config") || *t == "alias"
            }),
            "find" => !toks.iter().skip(1).any(|t| {
                matches!(*t, "-exec" | "-execdir" | "-ok" | "-okdir")
            }),
            "curl" | "wget" => !toks.iter().skip(1).any(|t| {
                matches!(*t, "-o" | "-O" | "-T" | "-J" | "--output" | "--output-document"
                              | "--upload-file" | "--remote-name")
                    || t.starts_with("--output=")
            }) && !seg.to_lowercase().contains("file://"),
            _ => true,
        }
    };

    // 每个链式片段分别校验（空片段来自 && / || 的分隔符，跳过）。
    // 先保护 `2>&1` 形态的 fd 复制（其中的 & 不是命令分隔符）。
    let sanitized = command.replace(">&", "\u{0}FD\u{0}");
    sanitized.split(|c: char| c == '|' || c == ';' || c == '&' || c == '\n')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .all(|s| check_cmd(s) && flags_ok(s))
}

/// 白名单豁免执行 — 仅供上层权限闸门放行后调用（人工授权 / full 模式 / 安全自评通过）。
/// 保留 PermissionChecker 危险清单与命令替换/重定向防护，仅跳过白名单 —
/// 白名单是"无人在场"的默认护栏，人工授权即最高裁决。
pub async fn execute_bash_approved(tool_call: &ToolCall) -> anyhow::Result<ToolResult> {
    let checker = PermissionChecker::new(PermissionConfig::default());
    let check_arg = extract_arg(&tool_call.arguments, "command");
    if matches!(checker.check("bash", &check_arg), PermissionDecision::Deny) {
        return Ok(ToolResult {
            tool_call_id: tool_call.id.clone(),
            output: "⛔ 权限拒绝：bash 操作被安全策略阻止".into(),
            success: false,
        });
    }
    // 命令替换与文件写入重定向防护保留（防 $(...) 与 `>` 绕过写文件路径检查）
    let has_write_redirect = |cmd: &str| -> bool {
        let chars: Vec<char> = cmd.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            if *c == '>' && chars.get(i + 1).copied().unwrap_or(' ') != '&' { return true; }
            if *c == '<' { return true; }
        }
        false
    };
    if check_arg.contains("$(") || check_arg.contains('`') || has_write_redirect(&check_arg) {
        return Ok(ToolResult {
            tool_call_id: tool_call.id.clone(),
            output: "⛔ 命令包含命令替换或文件重定向，禁止执行".into(),
            success: false,
        });
    }
    execute_bash(&tool_call.arguments).await
}

/// 执行工具调用（带权限检查 + 工作目录限制）
/// base = None：全局 WORK_DIR 语义（调度器/子代理等无工作区的调用方）
pub async fn execute_tool(tool_call: &ToolCall) -> anyhow::Result<ToolResult> {
    execute_tool_in(tool_call, None).await
}

/// base = 请求级工作区（对话中用户选定的"智能体工作位置"，对齐 Claude Code 项目目录）：
/// - 相对路径一律按 base 解析（base 缺省回退全局 WORK_DIR）
/// - 文件操作范围/ bash CWD / glob·grep 均以 base 为界 — 工作区外路径拒绝
pub async fn execute_tool_in(tool_call: &ToolCall, base: Option<&Path>) -> anyhow::Result<ToolResult> {
    let checker = PermissionChecker::new(PermissionConfig::default());
    let eff_dir: Option<&Path> = effective_dir(base);

    // 提取关键参数用于权限检查
    let check_arg = match tool_call.name.as_str() {
        "bash" => extract_arg(&tool_call.arguments, "command"),
        "read_file" | "write_file" | "edit_file" => extract_arg(&tool_call.arguments, "path"),
        _ => tool_call.arguments.clone(),
    };

    let decision = checker.check(&tool_call.name, &check_arg);
    if matches!(decision, PermissionDecision::Deny) {
        return Ok(ToolResult {
            tool_call_id: tool_call.id.clone(),
            output: format!("⛔ 权限拒绝：{} 操作被安全策略阻止", tool_call.name),
            success: false,
        });
    }

    // 路径安全检查
    if matches!(tool_call.name.as_str(), "read_file" | "write_file" | "edit_file") {
        if !is_path_safe(&check_arg) || !is_resolved_path_safe(&check_arg, eff_dir) {
            return Ok(ToolResult {
                tool_call_id: tool_call.id.clone(),
                output: format!("⛔ 路径安全拒绝：{} 访问了敏感文件", tool_call.name),
                success: false,
            });
        }
        // 工作目录检查（请求级工作区优先）
        if !is_within_work_dir_in(&check_arg, eff_dir) {
            return Ok(ToolResult {
                tool_call_id: tool_call.id.clone(),
                output: format!("⛔ 工作目录限制：{} 路径超出当前工作区允许范围", tool_call.name),
                success: false,
            });
        }
    }

    // bash 白名单检查
    if tool_call.name == "bash" {
        if !is_bash_allowed(&check_arg) {
            return Ok(ToolResult {
                tool_call_id: tool_call.id.clone(),
                output: format!("⛔ 命令不在白名单中：{}", check_arg),
                success: false,
            });
        }
    }

    let result = match tool_call.name.as_str() {
        "bash" => execute_bash_in(&tool_call.arguments, base).await,
        "read_file" => execute_read_file_in(&tool_call.arguments, base),
        "write_file" => execute_write_file_in(&tool_call.arguments, base),
        "edit_file" => execute_edit_file_in(&tool_call.arguments, base),
        "glob" => execute_glob_in(&tool_call.arguments, base),
        "grep" => execute_grep_in(&tool_call.arguments, base),
        "web_search" => execute_web_search(&tool_call.arguments).await,
        _ => Err(anyhow::anyhow!("未知工具: {}", tool_call.name)),
    };

    result.map(|mut r| {
        r.tool_call_id = tool_call.id.clone();
        r
    })
}

/// 执行shell命令（带30秒超时）
async fn execute_bash(args: &str) -> anyhow::Result<ToolResult> {
    execute_bash_in(args, None).await
}

async fn execute_bash_in(args: &str, base: Option<&Path>) -> anyhow::Result<ToolResult> {
    let v: serde_json::Value = serde_json::from_str(args)?;
    let command = v["command"].as_str().unwrap_or(args).to_string();

    // 设置工作目录（请求级工作区优先 — bash 在用户选定的工作区内执行）
    let cwd = effective_dir(base).map(|p| p.to_path_buf());

    let output = tokio::time::timeout(
        Duration::from_secs(BASH_TIMEOUT_SECS),
        async {
            let mut cmd = if cfg!(target_os = "windows") {
                let mut c = tokio::process::Command::new("cmd");
                c.args(["/C", &command]);
                // CREATE_NO_WINDOW — 智能体执行命令不得弹出控制台窗口：
                // 桌面壳是窗口子系统进程，子控制台程序默认新开 cmd 窗口，闪屏且遮挡应用
                #[cfg(windows)]
                { c.creation_flags(0x0800_0000); }
                c
            } else {
                let mut c = tokio::process::Command::new("sh");
                c.args(["-c", &command]);
                c
            };
            if let Some(ref dir) = cwd {
                cmd.current_dir(dir);
            }
            cmd.output().await
        },
    )
    .await;

    match output {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let combined = if stderr.is_empty() {
                stdout
            } else {
                format!("{}\n[stderr] {}", stdout, stderr)
            };
            Ok(ToolResult {
                tool_call_id: String::new(),
                output: truncate(combined),
                success: output.status.success(),
            })
        }
        Ok(Err(e)) => Ok(ToolResult {
            tool_call_id: String::new(),
            output: format!("命令执行失败: {}", e),
            success: false,
        }),
        Err(_) => Ok(ToolResult {
            tool_call_id: String::new(),
            output: format!("命令超时（{}秒）", BASH_TIMEOUT_SECS),
            success: false,
        }),
    }
}

/// 读取文件（支持offset/limit，输出截断，文件大小检查）
fn execute_read_file_in(args: &str, base: Option<&Path>) -> anyhow::Result<ToolResult> {
    let v: serde_json::Value = serde_json::from_str(args)
        .map_err(|e| anyhow::anyhow!("参数解析失败: {}", e))?;
    let path = v["path"].as_str().unwrap_or(args);
    let offset = v["offset"].as_u64().unwrap_or(0) as usize;
    let limit = v["limit"].as_u64().unwrap_or(0) as usize;

    // URL detection — return friendly error instead of OS error 123
    if path.starts_with("http://") || path.starts_with("https://") || path.starts_with("file://") {
        return Ok(ToolResult {
            tool_call_id: String::new(),
            output: format!("路径 '{}' 是 URL，read_file 仅支持本地文件路径。如需搜索网络请使用 web_search 工具。", path),
            success: false,
        });
    }

    // 相对路径按工作区/WORK_DIR 解析（CWD 可能与工作目录不同）
    let resolved = resolve_path_in(path, effective_dir(base));
    let path = resolved.to_str().unwrap_or(path);

    // File existence + metadata — friendly errors instead of raw OS errors
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            let friendly = if e.kind() == std::io::ErrorKind::NotFound {
                format!("文件不存在: {}", path)
            } else if e.to_string().contains("os error 123") || e.to_string().contains("syntax") {
                format!("路径语法无效: '{}' — 请检查路径格式", path)
            } else {
                format!("无法访问文件 {}: {}", path, e)
            };
            return Ok(ToolResult {
                tool_call_id: String::new(),
                output: friendly,
                success: false,
            });
        }
    };

    if metadata.len() > MAX_FILE_SIZE {
        return Ok(ToolResult {
            tool_call_id: String::new(),
            output: format!("文件过大（{} 字节），超过 1MB 限制", metadata.len()),
            success: false,
        });
    }

    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            return Ok(ToolResult {
                tool_call_id: String::new(),
                output: format!("读取文件失败 {}: {} — 文件可能是二进制格式", path, e),
                success: false,
            });
        }
    };

    let lines: Vec<&str> = content.lines().collect();
    let start = offset.min(lines.len());
    let end = if limit > 0 { (start + limit).min(lines.len()) } else { lines.len() };
    let result = lines[start..end].join("\n");

    Ok(ToolResult {
        tool_call_id: String::new(),
        output: truncate(result),
        success: true,
    })
}

/// 写入文件（写入前创建 .bak 备份）
fn execute_write_file_in(args: &str, base: Option<&Path>) -> anyhow::Result<ToolResult> {
    let v: serde_json::Value = serde_json::from_str(args)?;
    let path = v["path"].as_str().unwrap_or("");
    let content = v["content"].as_str().unwrap_or("");

    let resolved = resolve_path_in(path, effective_dir(base));
    let path = resolved.to_str().unwrap_or(path);

    // 创建备份
    let p = Path::new(path);
    if p.exists() {
        let bak = format!("{}.bak", path);
        let _ = fs::copy(path, &bak);
    }

    fs::write(path, content)?;
    Ok(ToolResult {
        tool_call_id: String::new(),
        output: format!("已写入 {} ({} 字节)", path, content.len()),
        success: true,
    })
}

/// 查找并替换（仅替换第一处，编辑前创建 .bak 备份）
fn execute_edit_file_in(args: &str, base: Option<&Path>) -> anyhow::Result<ToolResult> {
    let v: serde_json::Value = serde_json::from_str(args)?;
    let path = v["path"].as_str().unwrap_or("");
    let old = v["old"].as_str().unwrap_or("");
    let new = v["new"].as_str().unwrap_or("");

    let resolved = resolve_path_in(path, effective_dir(base));
    let path = resolved.to_str().unwrap_or(path);
    let content = fs::read_to_string(path)?;
    if !content.contains(old) {
        return Ok(ToolResult {
            tool_call_id: String::new(),
            output: format!("未找到匹配文本: {}", old),
            success: false,
        });
    }

    // 创建备份
    let bak = format!("{}.bak", path);
    let _ = fs::copy(path, &bak);

    // 仅替换第一处
    let updated = content.replacen(old, new, 1);
    fs::write(path, updated)?;
    Ok(ToolResult {
        tool_call_id: String::new(),
        output: format!("已替换 {} 中的第一处匹配（已备份 .bak）", path),
        success: true,
    })
}

/// glob模式查找文件（Rust原生实现，跨平台，限制在工作目录内）
fn execute_glob_in(args: &str, base: Option<&Path>) -> anyhow::Result<ToolResult> {
    let v: serde_json::Value = serde_json::from_str(args)?;
    let pattern = v["pattern"].as_str().unwrap_or(args);

    // 相对 pattern 在工作区内展开（glob 相对进程 CWD 解析 — 必须显式拼上工作区前缀）
    let eff_dir = effective_dir(base);
    let full_pattern = if Path::new(pattern).is_absolute() || eff_dir.is_none() {
        pattern.to_string()
    } else {
        format!("{}/{}", eff_dir.unwrap().display(), pattern)
    };

    let mut results: Vec<String> = Vec::new();
    for entry in glob::glob(&full_pattern).map_err(|e| anyhow::anyhow!("glob模式错误: {}", e))? {
        match entry {
            Ok(path) => {
                let path_str = path.display().to_string();
                // 工作目录检查（请求级工作区优先）
                if is_within_work_dir_in(&path_str, eff_dir) {
                    results.push(path_str);
                }
            }
            Err(e) => results.push(format!("[error] {}", e)),
        }
        if results.len() >= 200 {
            results.push("... [结果已截断，最多200条]".into());
            break;
        }
    }

    let output = if results.is_empty() {
        "未找到匹配文件".to_string()
    } else {
        results.join("\n")
    };

    Ok(ToolResult {
        tool_call_id: String::new(),
        output: truncate(output),
        success: true,
    })
}

/// 正则搜索文件内容（Rust原生实现，跨平台，限制在工作目录内）
fn execute_grep_in(args: &str, base: Option<&Path>) -> anyhow::Result<ToolResult> {
    let v: serde_json::Value = serde_json::from_str(args)?;
    let pattern = v["pattern"].as_str().unwrap_or(args);
    let search_path = v["path"].as_str().unwrap_or(".");

    // 工作目录限制（相对路径先按工作区/WORK_DIR 解析）
    let eff_dir = effective_dir(base);
    let resolved_search = resolve_path_in(search_path, eff_dir);
    let search_path = resolved_search.to_str().unwrap_or(search_path);
    let search_path = if let Some(wd) = eff_dir {
        if !Path::new(search_path).starts_with(wd) {
            wd.to_str().unwrap_or(".").to_string()
        } else {
            search_path.to_string()
        }
    } else {
        search_path.to_string()
    };

    let re = regex::Regex::new(pattern).map_err(|e| anyhow::anyhow!("正则错误: {}", e))?;
    let mut results: Vec<String> = Vec::new();

    grep_recursive(&search_path, &re, &mut results, 200);

    let output = if results.is_empty() {
        "未找到匹配".to_string()
    } else {
        results.join("\n")
    };

    Ok(ToolResult {
        tool_call_id: String::new(),
        output: truncate(output),
        success: true,
    })
}

/// 递归搜索目录
fn grep_recursive(dir: &str, re: &regex::Regex, results: &mut Vec<String>, max: usize) {
    if results.len() >= max {
        if results.len() == max {
            results.push("... [结果已截断]".into());
        }
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries {
        if results.len() >= max { return; }
        let entry = match entry { Ok(e) => e, Err(_) => continue };
        let path = entry.path();

        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') || name == "target" || name == "node_modules" { continue; }
            }
            grep_recursive(path.to_str().unwrap_or(""), re, results, max);
        } else if path.is_file() {
            // 文件大小检查
            if let Ok(meta) = fs::metadata(&path) {
                if meta.len() > MAX_FILE_SIZE { continue; }
            }
            if let Ok(content) = fs::read_to_string(&path) {
                for (lineno, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        results.push(format!("{}:{}: {}", path.display(), lineno + 1,
                            line.chars().take(200).collect::<String>()));
                        if results.len() >= max { return; }
                    }
                }
            }
        }
    }
}

/// 网络搜索（Tavily API）
async fn execute_web_search(args: &str) -> anyhow::Result<ToolResult> {
    let v: serde_json::Value = serde_json::from_str(args)?;
    let query = v["query"].as_str().unwrap_or(args);
    let max_results = v["max_results"].as_u64().unwrap_or(5).min(10);

    let api_key = SEARCH_API_KEY.get()
        .cloned()
        .unwrap_or_default();

    if api_key.is_empty() {
        return Ok(ToolResult {
            tool_call_id: String::new(),
            output: "网络搜索未配置 — 请在设置中配置搜索 API Key".to_string(),
            success: false,
        });
    }

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.tavily.com/search")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "api_key": api_key,
            "query": query,
            "max_results": max_results,
            "search_depth": "basic",
        }))
        .send()
        .await;

    match resp {
        Ok(r) => {
            let json: serde_json::Value = r.json().await?;
            let results = json["results"].as_array().map(|arr| {
                arr.iter().enumerate().map(|(i, item)| {
                    let title = item["title"].as_str().unwrap_or("");
                    let url = item["url"].as_str().unwrap_or("");
                    let content = item["content"].as_str().unwrap_or("");
                    let snippet: String = content.chars().take(200).collect();
                    format!("{}. {}\n   {}\n   {}", i + 1, title, url, snippet)
                }).collect::<Vec<_>>().join("\n\n")
            }).unwrap_or_else(|| "未找到结果".to_string());

            Ok(ToolResult {
                tool_call_id: String::new(),
                output: truncate(results),
                success: true,
            })
        }
        Err(e) => Ok(ToolResult {
            tool_call_id: String::new(),
            output: format!("搜索失败: {}", e),
            success: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 相对路径必须按**工作区**解析，而不是进程 CWD。
    /// 回归背景：原先对相对路径直接 canonicalize（基准是进程 CWD），导致
    /// 工作区不在 CWD 内时，子代理的所有相对路径写入被全部误拒。
    #[test]
    fn work_dir_scope_resolves_relative_paths_against_work_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let wd = tmp.path().join("ws");
        std::fs::create_dir_all(&wd).unwrap();

        // 工作区内的相对路径（新建文件尚不存在，最易踩坑）→ 放行
        assert!(is_within_work_dir_in("a.rs", Some(&wd)), "工作区内相对路径应放行");
        assert!(is_within_work_dir_in("sub/dir/b.rs", Some(&wd)), "工作区内多级相对路径应放行");
        // 工作区内的绝对路径 → 放行
        assert!(is_within_work_dir_in(&wd.join("c.rs").to_string_lossy(), Some(&wd)));
    }

    /// 越界必须仍然被拒 —— 修基准不能把安全边界一起放松。
    #[test]
    fn work_dir_scope_still_rejects_escapes() {
        let tmp = tempfile::tempdir().unwrap();
        let wd = tmp.path().join("ws");
        std::fs::create_dir_all(&wd).unwrap();

        // 相对路径回退到工作区之外 → 拒绝
        assert!(!is_within_work_dir_in("../evil.rs", Some(&wd)), "相对路径越界必须拒绝");
        assert!(!is_within_work_dir_in("../../evil.rs", Some(&wd)), "多层回退越界必须拒绝");
        assert!(!is_within_work_dir_in("sub/../../evil.rs", Some(&wd)), "中途回退越界必须拒绝");
        // 绝对路径在工作区外 → 拒绝
        let outside = tmp.path().join("outside.rs");
        assert!(!is_within_work_dir_in(&outside.to_string_lossy(), Some(&wd)), "工作区外绝对路径必须拒绝");
        // URL 一律拒绝
        assert!(!is_within_work_dir_in("https://example.com/x.rs", Some(&wd)));
    }

    /// 写与检查必须落在同一位置：用 execute 真正写一次，再确认范围检查认可该文件。
    #[test]
    fn write_and_scope_check_agree_on_location() {
        let tmp = tempfile::tempdir().unwrap();
        let wd = tmp.path().join("ws");
        std::fs::create_dir_all(&wd).unwrap();
        let target = wd.join("out.txt");
        let tc = crate::llm::provider::ToolCall {
            id: "t1".into(),
            name: "write_file".into(),
            arguments: serde_json::json!({"path": "out.txt", "content": "hi"}).to_string(),
        };
        let r = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute_tool_in(&tc, Some(&wd)))
            .unwrap();
        assert!(r.success, "写入应成功: {}", r.output);
        assert!(target.exists(), "文件应落在工作区内: {}", r.output);
        // 写成功了，范围检查也必须认可同一个位置（否则说明两处基准不一致）
        assert!(is_within_work_dir_in("out.txt", Some(&wd)), "写入位置与范围检查基准必须一致");
    }

    #[test]
    fn test_truncate_chinese_no_panic() {
        // 4000 个中文字符 = 12000 字节 > MAX_OUTPUT，按字节切片会劈开多字节字符导致 panic
        let big = "水".repeat(4000);
        let out = truncate(big);
        assert!(out.contains("输出已截断"));
    }

    #[test]
    fn test_truncate_short_passthrough() {
        assert_eq!(truncate("hello".into()), "hello");
    }

    #[test]
    fn test_bash_chain_second_command_denied() {
        // cmd /C 会执行 & 分隔的全部命令 — 每个片段都必须过白名单
        assert!(!is_bash_allowed("echo x & reg delete HKLM"));
        assert!(!is_bash_allowed("echo x && del f.txt"));
        assert!(!is_bash_allowed("echo x; format c:"));
    }

    #[test]
    fn test_bash_redirect_denied_fd_dup_allowed() {
        // > 可绕过 write_file 的路径安全检查；2>&1（fd 复制）无害放行
        assert!(!is_bash_allowed("echo x > out.txt"));
        assert!(!is_bash_allowed("echo x >> out.txt"));
        assert!(!is_bash_allowed("grep a < in.txt"));
        assert!(is_bash_allowed("ls missing 2>&1"));
    }

    #[test]
    fn test_bash_pipeline_all_segments_whitelisted() {
        assert!(is_bash_allowed("git log --oneline | head -5"));
        assert!(is_bash_allowed("cat a.txt | grep foo | sort"));
        assert!(!is_bash_allowed("cat a.txt | foobar evil.py"));
    }

    #[test]
    fn test_resolve_path_prefers_work_dir_over_cwd() {
        let wd = Path::new("/mock/work");
        // 相对路径 → WORK_DIR 拼接（而非进程 CWD）
        assert_eq!(resolve_path_in("a.txt", Some(wd)), wd.join("a.txt"));
        // 绝对路径原样保留
        let abs = if cfg!(windows) { "C:/x/y.txt" } else { "/x/y.txt" };
        assert_eq!(resolve_path_in(abs, Some(wd)), PathBuf::from(abs));
        // 未设置 WORK_DIR → 保持相对（与旧行为一致）
        assert_eq!(resolve_path_in("a.txt", None), PathBuf::from("a.txt"));
    }

    #[tokio::test]
    async fn test_bash_echo() {
        let tc = ToolCall { id: "test1".into(), name: "bash".into(), arguments: r#"{"command":"echo hello"}"#.into() };
        let result = execute_tool(&tc).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello"));
        assert_eq!(result.tool_call_id, "test1");
    }

    #[tokio::test]
    async fn test_bash_denied() {
        let tc = ToolCall { id: "test2".into(), name: "bash".into(), arguments: r#"{"command":"rm -rf /tmp"}"#.into() };
        let result = execute_tool(&tc).await.unwrap();
        assert!(!result.success);
        // rm is caught by either PermissionChecker (dangerous list) or whitelist (not in allowed list)
        assert!(result.output.contains("权限拒绝") || result.output.contains("白名单"));
    }

    #[tokio::test]
    async fn test_bash_unknown_cmd_denied() {
        let tc = ToolCall { id: "test2b".into(), name: "bash".into(), arguments: r#"{"command":"format c:"}"#.into() };
        let result = execute_tool(&tc).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("白名单"));
    }

    #[tokio::test]
    async fn test_bash_git_allowed() {
        let tc = ToolCall { id: "test2c".into(), name: "bash".into(), arguments: r#"{"command":"git status"}"#.into() };
        let result = execute_tool(&tc).await.unwrap();
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_read_file_truncate() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("big.txt");
        let big_content = "x".repeat(6000);
        fs::write(&path, &big_content).unwrap();
        let path_str = path.display().to_string().replace('\\', "\\\\");
        let tc = ToolCall {
            id: "test3".into(), name: "read_file".into(),
            arguments: format!(r#"{{"path":"{}"}}"#, path_str),
        };
        let result = execute_tool(&tc).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("截断"));
    }

    #[tokio::test]
    async fn test_read_file_too_large() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("huge.txt");
        fs::write(&path, "x".repeat(2_000_000)).unwrap();
        let path_str = path.display().to_string().replace('\\', "\\\\");
        let tc = ToolCall {
            id: "test3b".into(), name: "read_file".into(),
            arguments: format!(r#"{{"path":"{}"}}"#, path_str),
        };
        let result = execute_tool(&tc).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("过大"));
    }

    #[tokio::test]
    async fn test_read_llm_toml_denied() {
        let tc = ToolCall { id: "test3c".into(), name: "read_file".into(), arguments: r#"{"path":"config/llm.toml"}"#.into() };
        let result = execute_tool(&tc).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("敏感"));
    }

    #[tokio::test]
    async fn test_edit_file_first_only() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "aaa bbb aaa bbb aaa").unwrap();
        let path_str = path.display().to_string().replace('\\', "\\\\");
        let tc = ToolCall {
            id: "test4".into(), name: "edit_file".into(),
            arguments: format!(r#"{{"path":"{}","old":"aaa","new":"XXX"}}"#, path_str),
        };
        let result = execute_tool(&tc).await.unwrap();
        assert!(result.success);
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "XXX bbb aaa bbb aaa");
        // 备份应该存在
        assert!(dir.path().join("test.txt.bak").exists());
    }

    #[tokio::test]
    async fn test_edit_file_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "hello world").unwrap();
        let path_str = path.display().to_string().replace('\\', "\\\\");
        let tc = ToolCall {
            id: "test5".into(), name: "edit_file".into(),
            arguments: format!(r#"{{"path":"{}","old":"xyz","new":"abc"}}"#, path_str),
        };
        let result = execute_tool(&tc).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("未找到"));
    }

    #[tokio::test]
    async fn test_write_file_creates_bak() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("write_test.txt");
        fs::write(&path, "original").unwrap();
        let path_str = path.display().to_string().replace('\\', "\\\\");
        let tc = ToolCall {
            id: "test_w1".into(), name: "write_file".into(),
            arguments: format!(r#"{{"path":"{}","content":"new content"}}"#, path_str),
        };
        let result = execute_tool(&tc).await.unwrap();
        assert!(result.success);
        assert_eq!(fs::read_to_string(&path).unwrap(), "new content");
        assert_eq!(fs::read_to_string(dir.path().join("write_test.txt.bak")).unwrap(), "original");
    }

    #[tokio::test]
    async fn test_glob_finds_files() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("a.rs"), "").unwrap();
        fs::write(dir.path().join("b.rs"), "").unwrap();
        fs::write(dir.path().join("c.txt"), "").unwrap();
        let pattern = format!("{}/*.rs", dir.path().display().to_string().replace('\\', "/"));
        let tc = ToolCall { id: "test6".into(), name: "glob".into(), arguments: format!(r#"{{"pattern":"{}"}}"#, pattern) };
        let result = execute_tool(&tc).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("a.rs"));
        assert!(result.output.contains("b.rs"));
        assert!(!result.output.contains("c.txt"));
    }

    #[tokio::test]
    async fn test_grep_finds_matches() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("test.txt"), "hello world\nfoo bar\nhello again").unwrap();
        let tc = ToolCall {
            id: "test7".into(), name: "grep".into(),
            arguments: format!(r#"{{"pattern":"hello","path":"{}"}}"#, dir.path().display().to_string().replace('\\', "/")),
        };
        let result = execute_tool(&tc).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("test.txt:1:"));
        assert!(result.output.contains("test.txt:3:"));
    }

    #[tokio::test]
    async fn test_path_safety_denied() {
        let tc = ToolCall { id: "test8".into(), name: "read_file".into(), arguments: r#"{"path":".env"}"#.into() };
        let result = execute_tool(&tc).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("敏感"));
    }

    #[tokio::test]
    async fn test_unknown_tool() {
        let tc = ToolCall { id: "test9".into(), name: "unknown".into(), arguments: "{}".into() };
        let result = execute_tool(&tc).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tool_call_id_passed() {
        let tc = ToolCall { id: "call_abc123".into(), name: "bash".into(), arguments: r#"{"command":"echo test"}"#.into() };
        let result = execute_tool(&tc).await.unwrap();
        assert_eq!(result.tool_call_id, "call_abc123");
    }

    #[tokio::test]
    async fn test_bash_pipeline_allowed() {
        // git log --oneline | head -5 — 管道命令，但首命令 git 在白名单
        let tc = ToolCall { id: "test_p1".into(), name: "bash".into(), arguments: r#"{"command":"echo hello | head -5"}"#.into() };
        let result = execute_tool(&tc).await.unwrap();
        assert!(result.success);
    }
}

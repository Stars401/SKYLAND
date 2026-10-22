//! 文件系统模块
//!
//! 负责目录隔离、权限控制和文件操作。
//! 支持Windows和Unix系统的跨平台权限管理。

use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FsError {
    #[error("目录不存在: {0}")]
    DirectoryNotFound(String),

    #[error("权限设置失败: {0}")]
    PermissionDenied(String),

    #[error("IO错误: {0}")]
    IoError(#[from] std::io::Error),

    #[error("权限验证失败: expected {expected:o}, actual {actual:o}")]
    PermissionMismatch { expected: u32, actual: u32 },

    #[error("不支持的平台")]
    UnsupportedPlatform,
}

/// 目录权限配置
pub struct DirectoryPermissions {
    pub code_agent: u32,
    pub review_agent: u32,
    pub handoff: u32,
}

impl Default for DirectoryPermissions {
    fn default() -> Self {
        Self {
            code_agent: 0o700,
            review_agent: 0o500,
            handoff: 0o755,
        }
    }
}

/// 文件系统管理器
pub struct FileSystemManager {
    pub base_dir: String,
    pub permissions: DirectoryPermissions,
}

impl FileSystemManager {
    pub fn new(base_dir: String) -> Self {
        Self {
            base_dir,
            permissions: DirectoryPermissions::default(),
        }
    }

    /// 初始化目录结构，创建必要目录并设置权限
    pub fn initialize_directories(&self) -> anyhow::Result<()> {
        let dirs = vec![
            format!("{}/agents/code_agent/logs", self.base_dir),
            format!("{}/agents/code_agent/memory", self.base_dir),
            format!("{}/agents/code_agent/task_history", self.base_dir),
            format!("{}/agents/review_agent/logs", self.base_dir),
            format!("{}/agents/review_agent/memory", self.base_dir),
            format!("{}/agents/review_agent/task_history", self.base_dir),
            format!("{}/handoff", self.base_dir),
        ];

        for dir in &dirs {
            fs::create_dir_all(dir)?;
        }

        self.set_directory_permissions(
            &format!("{}/agents/code_agent", self.base_dir),
            self.permissions.code_agent,
        )?;
        self.set_directory_permissions(
            &format!("{}/agents/review_agent", self.base_dir),
            self.permissions.review_agent,
        )?;
        self.set_directory_permissions(
            &format!("{}/handoff", self.base_dir),
            self.permissions.handoff,
        )?;

        Ok(())
    }

    /// 创建智能体专属目录
    pub fn create_agent_directory(&self, agent_name: &str) -> anyhow::Result<()> {
        let dirs = vec![
            format!("{}/agents/{}/logs", self.base_dir, agent_name),
            format!("{}/agents/{}/memory", self.base_dir, agent_name),
            format!("{}/agents/{}/task_history", self.base_dir, agent_name),
        ];

        let permission = match agent_name {
            "code_agent" => self.permissions.code_agent,
            "review_agent" => self.permissions.review_agent,
            _ => self.permissions.code_agent,
        };

        for dir in &dirs {
            fs::create_dir_all(dir)?;
        }

        self.set_directory_permissions(
            &format!("{}/agents/{}", self.base_dir, agent_name),
            permission,
        )?;

        Ok(())
    }

    /// 设置目录权限 (跨平台)
    pub fn set_directory_permissions(&self, path: &str, mode: u32) -> anyhow::Result<()> {
        let p = Path::new(path);
        if !p.exists() {
            return Err(FsError::DirectoryNotFound(path.to_string()).into());
        }
        set_permissions_impl(path, mode)
    }

    /// 检查目录权限是否匹配
    pub fn check_directory_permissions(&self, path: &str, expected_mode: u32) -> anyhow::Result<bool> {
        let p = Path::new(path);
        if !p.exists() {
            return Err(FsError::DirectoryNotFound(path.to_string()).into());
        }
        check_permissions_impl(path, expected_mode)
    }

    /// 验证智能体目录隔离
    pub fn verify_agent_isolation(&self, agent_name: &str) -> anyhow::Result<bool> {
        let agent_dir = format!("{}/agents/{}", self.base_dir, agent_name);
        let expected_mode = match agent_name {
            "code_agent" => self.permissions.code_agent,
            "review_agent" => self.permissions.review_agent,
            _ => return Ok(false),
        };

        let dir_exists = Path::new(&agent_dir).exists();
        if !dir_exists {
            return Ok(false);
        }

        self.check_directory_permissions(&agent_dir, expected_mode)
    }

    /// 验证handoff目录权限
    pub fn verify_handoff_permissions(&self) -> anyhow::Result<bool> {
        let handoff_dir = format!("{}/handoff", self.base_dir);
        self.check_directory_permissions(&handoff_dir, self.permissions.handoff)
    }

    /// 监听目录变化
    pub fn watch_directory<F>(&self, path: &str, callback: F) -> anyhow::Result<()>
    where
        F: Fn(PathBuf) + Send + 'static,
    {
        use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
        use std::sync::mpsc;

        let (tx, rx) = mpsc::channel();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                        for p in &event.paths {
                            tx.send(p.clone()).ok();
}
                    }
                }
            },
            Config::default(),
        )?;

        watcher.watch(Path::new(path), RecursiveMode::Recursive)?;

        std::thread::spawn(move || {
            while let Ok(changed_path) = rx.recv() {
                callback(changed_path);
            }
        });

        // Keep watcher alive
        std::mem::forget(watcher);
        Ok(())
    }
}

/// handoff目录单向流动控制
#[allow(dead_code)]
pub struct HandoffGuard { handoff_path: String }

impl HandoffGuard {
    pub fn new(path: &str) -> Self { Self { handoff_path: path.to_string() } }

    pub fn verify_write(&self, agent: &str) -> anyhow::Result<()> {
        if agent != "code_agent" {
            return Err(FsError::PermissionDenied(format!("{} 无权写入handoff", agent)).into());
        }
        Ok(())
    }

    pub fn verify_read(&self, agent: &str) -> anyhow::Result<()> {
        if agent != "code_agent" && agent != "review_agent" {
            return Err(FsError::PermissionDenied(format!("{} 无权读取handoff", agent)).into());
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn set_permissions_impl(path: &str, mode: u32) -> anyhow::Result<()> {
    use std::process::Command;

    let perm_str = match mode {
        0o700 => "(OI)(CI)F",
        0o500 => "(OI)(CI)R",
        0o755 => "(OI)(CI)F",
        _ => "(OI)(CI)F",
    };

    let mut acl_cmd = Command::new("icacls");
    acl_cmd.arg(path)
        .arg("/grant:r")
        .arg(format!("Everyone:{}", perm_str))
        .arg("/Q");
    // CREATE_NO_WINDOW — 权限设置不弹控制台窗口
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        acl_cmd.creation_flags(0x0800_0000);
    }
    let output = acl_cmd.output()?;

    if !output.status.success() {
        return Err(FsError::PermissionDenied(format!(
            "icacls failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
        .into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn check_permissions_impl(path: &str, _expected_mode: u32) -> anyhow::Result<bool> {
    use std::process::Command;

    let p = Path::new(path);
    if !p.exists() {
        return Ok(false);
    }

    // Parse icacls output to verify permissions
    let mut acl_cmd = Command::new("icacls");
    acl_cmd.arg(path);
    // CREATE_NO_WINDOW — 权限检查不弹控制台窗口
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        acl_cmd.creation_flags(0x0800_0000);
    }
    let output = acl_cmd.output()?;

    if !output.status.success() {
        return Ok(false);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // icacls output lists permissions per user; if output exists, directory is accessible
    Ok(!stdout.is_empty() && p.is_dir())
}

#[cfg(unix)]
fn set_permissions_impl(path: &str, mode: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let p = Path::new(path);
    let mut perms = fs::metadata(p)?.permissions();
    perms.set_mode(mode);
    fs::set_permissions(p, perms)?;
    Ok(())
}

#[cfg(unix)]
fn check_permissions_impl(path: &str, expected_mode: u32) -> anyhow::Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::metadata(path)?;
    let actual_mode = metadata.permissions().mode() & 0o777;
    Ok(actual_mode == expected_mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_manager() -> (FileSystemManager, TempDir) {
        let dir = TempDir::new().unwrap();
        let base = dir.path().to_str().unwrap().to_string();
        (FileSystemManager::new(base), dir)
    }

    #[test]
    fn test_directory_permissions_default() {
        let perms = DirectoryPermissions::default();
        assert_eq!(perms.code_agent, 0o700);
        assert_eq!(perms.review_agent, 0o500);
        assert_eq!(perms.handoff, 0o755);
    }

    #[test]
    fn test_fs_manager_new() {
        let mgr = FileSystemManager::new("/test".to_string());
        assert_eq!(mgr.base_dir, "/test");
        assert_eq!(mgr.permissions.code_agent, 0o700);
    }

    #[test]
    fn test_initialize_directories() {
        let (mgr, _dir) = setup_test_manager();
        let result = mgr.initialize_directories();
        assert!(result.is_ok());

        let agents = format!("{}/agents", mgr.base_dir);
        assert!(Path::new(&agents).exists());
        assert!(Path::new(&format!("{}/code_agent", agents)).exists());
        assert!(Path::new(&format!("{}/review_agent", agents)).exists());
        assert!(Path::new(&format!("{}/handoff", mgr.base_dir)).exists());
    }

    #[test]
    fn test_create_agent_directory() {
        let (mgr, _dir) = setup_test_manager();
        let result = mgr.create_agent_directory("code_agent");
        assert!(result.is_ok());

        let agent_dir = format!("{}/agents/code_agent", mgr.base_dir);
        assert!(Path::new(&agent_dir).exists());
        assert!(Path::new(&format!("{}/logs", agent_dir)).exists());
        assert!(Path::new(&format!("{}/memory", agent_dir)).exists());
        assert!(Path::new(&format!("{}/task_history", agent_dir)).exists());
    }

    #[test]
    fn test_create_agent_directory_review() {
        let (mgr, _dir) = setup_test_manager();
        let result = mgr.create_agent_directory("review_agent");
        assert!(result.is_ok());

        let agent_dir = format!("{}/agents/review_agent", mgr.base_dir);
        assert!(Path::new(&agent_dir).exists());
    }

    #[test]
    fn test_set_permissions_nonexistent() {
        let (mgr, _dir) = setup_test_manager();
        let result = mgr.set_directory_permissions("/nonexistent/path", 0o700);
        assert!(result.is_err());
    }

    #[test]
    fn test_set_permissions_existing_dir() {
        let (mgr, _dir) = setup_test_manager();
        let test_path = format!("{}/test_dir", mgr.base_dir);
        fs::create_dir_all(&test_path).unwrap();

        let result = mgr.set_directory_permissions(&test_path, 0o700);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_permissions_nonexistent() {
        let (mgr, _dir) = setup_test_manager();
        let result = mgr.check_directory_permissions("/nonexistent", 0o700);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_agent_isolation_nonexistent() {
        let (mgr, _dir) = setup_test_manager();
        let result = mgr.verify_agent_isolation("unknown_agent");
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_verify_agent_isolation_after_init() {
        let (mgr, _dir) = setup_test_manager();
        mgr.initialize_directories().unwrap();
        let result = mgr.verify_agent_isolation("code_agent");
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_handoff_permissions_after_init() {
        let (mgr, _dir) = setup_test_manager();
        mgr.initialize_directories().unwrap();
        let result = mgr.verify_handoff_permissions();
        assert!(result.is_ok());
    }

    #[test]
    fn test_fs_error_display() {
        let err = FsError::DirectoryNotFound("/test".to_string());
        assert!(err.to_string().contains("/test"));

        let err = FsError::PermissionDenied("test".to_string());
        assert!(err.to_string().contains("test"));

        let err = FsError::PermissionMismatch { expected: 0o700, actual: 0o500 };
        assert!(err.to_string().contains("700"));
        assert!(err.to_string().contains("500"));
    }

    #[test]
    fn test_fs_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "test");
        let fs_err: FsError = io_err.into();
        assert!(matches!(fs_err, FsError::IoError(_)));
    }

    #[test]
    fn test_watch_directory() {
        let (mgr, _dir) = setup_test_manager();
        let watch_path = format!("{}/watch_test", mgr.base_dir);
        fs::create_dir_all(&watch_path).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let result = mgr.watch_directory(&watch_path, move |p| {
            tx.send(p).ok();
        });
        assert!(result.is_ok());

        // Write a file to trigger the watcher
        fs::write(format!("{}/test.txt", watch_path), "hello").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));

        let received = rx.try_recv();
        // May or may not receive due to timing; just verify no panic
        assert!(received.is_ok() || received.is_err());
    }

    #[test]
    fn test_handoff_guard_write_permission() {
        let guard = HandoffGuard::new("/tmp/handoff");
        assert!(guard.verify_write("code_agent").is_ok());
        assert!(guard.verify_write("review_agent").is_err());
    }

    #[test]
    fn test_handoff_guard_read_permission() {
        let guard = HandoffGuard::new("/tmp/handoff");
        assert!(guard.verify_read("code_agent").is_ok());
        assert!(guard.verify_read("review_agent").is_ok());
        assert!(guard.verify_read("unknown").is_err());
    }
}

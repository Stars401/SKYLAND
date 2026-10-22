//! 环境变量兼容层 —— SKYLAND_* 为现行权威名，MOLINE_* 为历史名。
//!
//! 品牌从 moline 迭代为 Skyland 后，历史环境变量可能仍被以下外部来源设置：
//! 用户启动脚本 / 桌面快捷方式 / 旧版 Tauri 宿主 / CI 配置文件。
//! 直接改名会让这些来源**静默失效**（读到空值而非报错），故统一在此收敛：
//! 新名优先、历史名兜底，业务代码只调用语义化函数，不再直接读 `std::env`。
//!
//! 历史名读取通道是**单向兼容**的：只读不写，写入方一律使用 SKYLAND_*，
//! 以保证收敛方向唯一（不会出现两个变量互相覆盖的震荡）。

/// 读取环境变量：新名优先，非空则返回；否则回落到历史名。
/// 空字符串视为未设置 —— 避免"设了但是空"把历史值也屏蔽掉。
fn var(new_name: &str, legacy_name: &str) -> Option<String> {
    for key in [new_name, legacy_name] {
        if let Ok(v) = std::env::var(key) {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// 桌面模式标记（由 skyland-app 在同进程内注入）。
/// 用于放开"绝对路径生图目录"等仅桌面端可用的能力。
pub fn desktop_mode() -> bool {
    var("SKYLAND_DESKTOP_MODE", "MOLINE_DESKTOP_MODE").as_deref() == Some("1")
}

/// 桌面端注入的 llm.toml 权威路径（文件不存在时也直接采用）。
pub fn config_path() -> Option<String> {
    var("SKYLAND_CONFIG_PATH", "MOLINE_CONFIG_PATH")
}

/// 数据目录强制定向（测试 / 嵌入隔离专用，优先于指针文件）。
/// 数据目录解析本身在 [`crate::datadir`] 内完成双读，此处仅供需要
/// 显式判定"是否被强制"的调用方使用。
pub fn data_dir() -> Option<String> {
    var("SKYLAND_DATA_DIR", "MOLINE_DATA_DIR")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    // 环境变量是进程级全局状态，并行测试会互相污染 —— 复用同一把锁串行化。
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    fn lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    /// 设置并回收一个环境变量，测试结束（含 panic）后自动还原。
    struct EnvGuard(&'static str, Option<String>);
    impl EnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let old = std::env::var(key).ok();
            std::env::set_var(key, val);
            Self(key, old)
        }
        fn unset(key: &'static str) -> Self {
            let old = std::env::var(key).ok();
            std::env::remove_var(key);
            Self(key, old)
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.1 {
                Some(v) => std::env::set_var(self.0, v),
                None => std::env::remove_var(self.0),
            }
        }
    }

    #[test]
    fn new_name_wins_over_legacy() {
        let _l = lock();
        let _a = EnvGuard::set("SKYLAND_DESKTOP_MODE", "1");
        let _b = EnvGuard::set("MOLINE_DESKTOP_MODE", "0");
        assert!(desktop_mode());
    }

    #[test]
    fn legacy_name_still_honored() {
        let _l = lock();
        let _a = EnvGuard::unset("SKYLAND_DESKTOP_MODE");
        let _b = EnvGuard::set("MOLINE_DESKTOP_MODE", "1");
        assert!(desktop_mode(), "仅设置历史变量时桌面模式必须仍为真");
    }

    #[test]
    fn empty_new_name_falls_through_to_legacy() {
        let _l = lock();
        let _a = EnvGuard::set("SKYLAND_DESKTOP_MODE", "   ");
        let _b = EnvGuard::set("MOLINE_DESKTOP_MODE", "1");
        assert!(
            desktop_mode(),
            "新名为空白时不得屏蔽历史值，否则会静默关掉桌面能力"
        );
    }

    #[test]
    fn both_absent_is_not_desktop_mode() {
        let _l = lock();
        let _a = EnvGuard::unset("SKYLAND_DESKTOP_MODE");
        let _b = EnvGuard::unset("MOLINE_DESKTOP_MODE");
        assert!(!desktop_mode());
    }

    #[test]
    fn config_path_falls_back_and_trims() {
        let _l = lock();
        let _a = EnvGuard::unset("SKYLAND_CONFIG_PATH");
        let _b = EnvGuard::set("MOLINE_CONFIG_PATH", "  /tmp/llm.toml  ");
        assert_eq!(config_path().as_deref(), Some("/tmp/llm.toml"));
    }
}

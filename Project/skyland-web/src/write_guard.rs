//! 并行写保护 —— 让多个子智能体安全地并发写代码
//!
//! 子代理从"只读"扩展到"可写"后，引入了三个**主智能体串行执行时不存在**的问题。
//! 本模块就是为解决它们而存在，与权限分级（`classify_gate`）正交：
//! 分级回答"该不该写"，本模块回答"多个写者同时写会不会打架"。
//!
//! 1. **同文件丢失更新**
//!    两个子代理都读了 `a.rs`，各自改一处后写回 —— 后写者用自己读到的**旧内容**
//!    覆盖前者，修改凭空消失。按归一化路径加锁即可：同文件串行、跨文件仍并行。
//!
//! 2. **授权窗口堆叠**
//!    3 个子代理同时触发 level-2 授权 → 前端连弹 3 个模态框，且主智能体在
//!    等待子代理返回、看起来像卡死。授权窗口全局串行，一次只弹一个。
//!
//! 3. **反复弹窗**
//!    子代理新建 `a.rs` 后再编辑它，若"本会话产物"集合不共享，每步都会重新
//!    触发授权。共享 `agent_files` 后，子代理产出的文件在主/子代理之间通用。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

/// 归一化路径键（与 `api::mod` 的规则键保持一致：小写 + 正斜杠）
pub fn path_key(p: &str) -> String {
    let k = p.trim().to_lowercase().replace('\\', "/");
    // 去掉尾斜杠：否则 "a/b/" 与 "a/b" 会被当成两个不同文件拿到两把锁，
    // 同文件并发写照样会丢失更新 —— 归一必须覆盖这一种写法差异。
    if k.len() > 1 {
        k.trim_end_matches('/').to_string()
    } else {
        k
    }
}

/// 本会话智能体（含各子代理）产出的文件集合。
/// 共享给子代理后，某个子代理新建的文件再被修改时仍判为 level 0（无需重复授权）。
#[derive(Clone, Default)]
pub struct AgentFiles(Arc<Mutex<HashSet<String>>>);

impl AgentFiles {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, p: &str) {
        if let Ok(mut g) = self.0.lock() {
            g.insert(path_key(p));
        }
    }

    pub fn contains(&self, p: &str) -> bool {
        self.0.lock().map(|g| g.contains(&path_key(p))).unwrap_or(false)
    }

    /// 闸门分级需要 `&HashSet` 快照；持锁时间极短（仅 clone），不会与写锁互相等待。
    pub fn snapshot(&self) -> HashSet<String> {
        self.0.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

/// 同文件写串行锁表。按路径惰性创建，进程内长期驻留（工作区文件数量有限）。
fn lock_table() -> &'static Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>> {
    static T: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取得某文件的写锁。持有它直到写入结束，即可保证同文件不并发。
pub async fn file_lock(path: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let key = path_key(path);
    let entry = {
        let mut g = lock_table().lock().unwrap_or_else(|e| e.into_inner());
        g.entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    entry.lock_owned().await
}

/// 授权窗口全局串行闸门。
/// 多个子代理并发等待授权时排队，避免前端同时弹出多个模态框。
pub fn approval_gate() -> &'static tokio::sync::Mutex<()> {
    static G: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    G.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_key_normalizes_case_and_separator() {
        assert_eq!(path_key("C:\\A\\B.rs"), path_key("c:/a/b.rs"));
        assert_eq!(path_key("  X.Y  "), "x.y");
        // 尾斜杠不同视为同一文件，避免漏加锁
        assert_eq!(path_key("a/b/"), path_key("a/b"));
    }

    #[tokio::test]
    async fn same_path_serializes_different_paths_do_not() {
        // 同文件（大小写/分隔符不同视为同一文件）必须互斥：第二个拿不到锁
        let _first = file_lock("A.rs").await;
        let blocked = tokio::time::timeout(
            std::time::Duration::from_millis(120),
            file_lock("a.rs"),
        ).await;
        assert!(blocked.is_err(), "同一文件的第二次加锁必须被阻塞（否则并发写会丢失更新）");
        drop(_first);
        // 释放后应能拿到，且大小写写法拿到的是同一把锁
        let again = tokio::time::timeout(std::time::Duration::from_millis(500), file_lock("a.rs")).await;
        assert!(again.is_ok(), "前一把锁释放后必须能再次获取");
    }

    #[tokio::test]
    async fn different_paths_do_not_block_each_other() {
        // 不同文件之间不应互相阻塞（并行写不同文件才能提效）
        let a = file_lock("A.rs").await;
        let ok = tokio::time::timeout(std::time::Duration::from_millis(200), file_lock("B.rs")).await;
        assert!(ok.is_ok(), "不同文件应可并行加锁");
        drop(a);
    }

    #[tokio::test]
    async fn agent_files_shared_and_normalized() {
        let f = AgentFiles::new();
        assert!(!f.contains("X.rs"));
        f.insert("src\\Module.rs");
        assert!(f.contains("src/Module.rs"), "新建后应可被任意写法识别为本会话产物");
        assert!(f.contains("SRC/module.rs"), "大小写与分隔符差异应归一");
        assert!(f.snapshot().contains("src/module.rs"));
        assert!(!f.contains("other.rs"), "不同文件不得互相命中");
    }
}
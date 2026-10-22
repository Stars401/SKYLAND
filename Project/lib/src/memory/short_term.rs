//! 短期记忆 - 基于内存的键值存储
//!
//! 线程安全，支持自动过期和容量限制。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// 短期记忆条目
#[derive(Debug, Clone)]
struct MemoryEntry {
    value: String,
    created_at: Instant,
    accessed_at: Instant,
    access_count: u64,
}

/// 短期记忆存储
///
/// 线程安全的HashMap实现，支持容量限制和自动过期。
pub struct ShortTermMemory {
    entries: Arc<RwLock<HashMap<String, MemoryEntry>>>,
    max_capacity: usize,
    ttl: Duration,
}

impl ShortTermMemory {
    /// 创建新的短期记忆实例
    pub fn new(max_capacity: usize, ttl_secs: u64) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            max_capacity,
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// 存储键值对
    pub fn set(&self, key: &str, value: &str) {
        let mut entries = match self.entries.write() {
            Ok(e) => e,
            Err(_) => return, // Lock poisoned — degrade silently
        };
        if entries.len() >= self.max_capacity && !entries.contains_key(key) {
            self.evict_one(&mut entries);
        }
        entries.insert(
            key.to_string(),
            MemoryEntry {
                value: value.to_string(),
                created_at: Instant::now(),
                accessed_at: Instant::now(),
                access_count: 0,
            },
        );
    }

    /// 获取键对应的值
    pub fn get(&self, key: &str) -> Option<String> {
        let mut entries = match self.entries.write() {
            Ok(e) => e,
            Err(_) => return None, // Lock poisoned — degrade silently
        };
        if let Some(entry) = entries.get_mut(key) {
            if entry.created_at.elapsed() > self.ttl {
                entries.remove(key);
                return None;
            }
            entry.accessed_at = Instant::now();
            entry.access_count += 1;
            Some(entry.value.clone())
        } else {
            None
        }
    }

    /// 删除指定键
    pub fn remove(&self, key: &str) -> Option<String> {
        let mut entries = match self.entries.write() {
            Ok(e) => e,
            Err(_) => return None,
        };
        entries.remove(key).map(|e| e.value)
    }

    /// 清空所有记忆
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.write() {
            entries.clear();
        }
    }

    /// 当前条目数
    pub fn len(&self) -> usize {
        self.entries.read().map(|e| e.len()).unwrap_or(0)
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.entries.read().map(|e| e.is_empty()).unwrap_or(true)
    }

    fn evict_one(&self, entries: &mut HashMap<String, MemoryEntry>) {
        if let Some(oldest) = entries
            .iter()
            .min_by_key(|(_, e)| e.accessed_at)
            .map(|(k, _)| k.clone())
        {
            entries.remove(&oldest);
        }
    }

    /// 获取指定key的访问计数
    pub fn access_count(&self, key: &str) -> Option<u64> {
        self.entries.read().ok()?.get(key).map(|e| e.access_count)
    }
}

impl Clone for ShortTermMemory {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            max_capacity: self.max_capacity,
            ttl: self.ttl,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_get() {
        let mem = ShortTermMemory::new(100, 3600);
        mem.set("key1", "value1");
        assert_eq!(mem.get("key1"), Some("value1".into()));
    }

    #[test]
    fn test_get_nonexistent() {
        let mem = ShortTermMemory::new(100, 3600);
        assert_eq!(mem.get("nope"), None);
    }

    #[test]
    fn test_remove() {
        let mem = ShortTermMemory::new(100, 3600);
        mem.set("k", "v");
        assert_eq!(mem.remove("k"), Some("v".into()));
        assert_eq!(mem.get("k"), None);
    }

    #[test]
    fn test_clear() {
        let mem = ShortTermMemory::new(100, 3600);
        mem.set("a", "1");
        mem.set("b", "2");
        mem.clear();
        assert!(mem.is_empty());
    }

    #[test]
    fn test_capacity_eviction() {
        let mem = ShortTermMemory::new(2, 3600);
        mem.set("a", "1");
        mem.set("b", "2");
        mem.set("c", "3");
        assert!(mem.len() <= 2);
    }

    #[test]
    fn test_access_count() {
        let mem = ShortTermMemory::new(100, 3600);
        mem.set("k", "v");
        mem.get("k");
        mem.get("k");
        assert_eq!(mem.access_count("k"), Some(2));
    }

    #[test]
    fn test_clone_shares_data() {
        let mem = ShortTermMemory::new(100, 3600);
        mem.set("x", "y");
        let clone = mem.clone();
        assert_eq!(clone.get("x"), Some("y".into()));
    }

    #[test]
    fn test_expired_entry_returns_none() {
        // TTL=0 means instant expiration
        let mem = ShortTermMemory::new(100, 0);
        mem.set("k", "v");
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert_eq!(mem.get("k"), None);
    }
}

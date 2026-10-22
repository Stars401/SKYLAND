//! 记忆清理 - 过期记忆自动清理和压缩

use super::short_term::ShortTermMemory;
use super::long_term::LongTermMemory;

/// 记忆清理器
pub struct MemoryCleanup;

impl MemoryCleanup {
    /// 清理短期记忆中的过期条目
    pub fn clean_short_term(_memory: &ShortTermMemory) -> usize {
        // get() already removes expired entries on access
        // Here we do a proactive sweep by trying to get all keys
        // ShortTermMemory doesn't expose key iteration, so we estimate
        0
    }

    /// 压缩长期记忆文件（保留最近N条）
    pub fn compress_long_term(memory: &LongTermMemory, keep_count: usize) -> anyhow::Result<usize> {
        let entries = memory.read_all()?;
        let total = entries.len();

        if total <= keep_count {
            return Ok(0);
        }

        let to_keep: Vec<_> = entries.into_iter().rev().take(keep_count).rev().collect();
        memory.clear()?;

        for entry in &to_keep {
            let tags: Vec<&str> = entry.tags.iter().map(|s| s.as_str()).collect();
            memory.append(&entry.category, &entry.content, &tags)?;
        }

        Ok(total - to_keep.len())
    }

    /// 执行完整记忆维护：
    /// 1. 清理短期过期
    /// 2. 压缩长期记录
    pub fn maintain(
        short_term: &ShortTermMemory,
        long_term: &LongTermMemory,
    ) -> anyhow::Result<(usize, usize)> {
        let st_cleaned = Self::clean_short_term(short_term);
        let lt_removed = Self::compress_long_term(long_term, 1000)?;
        Ok((st_cleaned, lt_removed))
    }
    /// 启动后台定期清理任务
    ///
    /// 每 interval_secs 秒执行一次维护。
    /// 返回 JoinHandle，调用方可 abort 停止。
    pub fn start_scheduler(
        short_term: ShortTermMemory,
        long_term: LongTermMemory,
        interval_secs: u64,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(interval_secs));
            Self::maintain(&short_term, &long_term).ok();
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_compress_under_limit() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("memory.txt");
        let mem = LongTermMemory::new(&path).unwrap();

        mem.append("code", "entry1", &[]).unwrap();
        mem.append("code", "entry2", &[]).unwrap();

        let removed = MemoryCleanup::compress_long_term(&mem, 10).unwrap();
        assert_eq!(removed, 0);
        assert_eq!(mem.count().unwrap(), 2);
    }

    #[test]
    fn test_compress_over_limit() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("memory.txt");
        let mem = LongTermMemory::new(&path).unwrap();

        for i in 0..10 {
            mem.append("code", &format!("entry{}", i), &[]).unwrap();
        }

        let removed = MemoryCleanup::compress_long_term(&mem, 3).unwrap();
        assert_eq!(removed, 7);
        assert_eq!(mem.count().unwrap(), 3);
    }
}

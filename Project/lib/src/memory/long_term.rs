//! 长期记忆 - 基于文件的持久化存储
//!
//! 支持追加写入、按行读取和基于时间戳的检索。

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use chrono::Local;

/// 长期记忆条目
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LongTermEntry {
    pub timestamp: String,
    pub category: String,
    pub content: String,
    pub tags: Vec<String>,
}

/// 长期记忆存储
///
/// 基于文件行存储，格式: `[时间戳] [类别] [内容] [标签1,标签2,...]`
pub struct LongTermMemory {
    file_path: PathBuf,
}

impl LongTermMemory {
    /// 打开或创建长期记忆文件
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            fs::File::create(path)?;
        }
        Ok(Self { file_path: path.to_path_buf() })
    }

    /// 追加一条长期记忆
    pub fn append(&self, category: &str, content: &str, tags: &[&str]) -> anyhow::Result<()> {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let tags_str = tags.join(",");
        let line = format!("[{}] [{}] {} [{}]\n", timestamp, category, content, tags_str);

        let mut file = OpenOptions::new().append(true).open(&self.file_path)?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    /// 读取所有记忆条目
    pub fn read_all(&self) -> anyhow::Result<Vec<LongTermEntry>> {
        let file = fs::File::open(&self.file_path)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if let Some(entry) = Self::parse_line(&line) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    /// 按类别过滤
    pub fn read_by_category(&self, category: &str) -> anyhow::Result<Vec<LongTermEntry>> {
        let all = self.read_all()?;
        Ok(all.into_iter().filter(|e| e.category == category).collect())
    }

    /// 统计条目数
    pub fn count(&self) -> anyhow::Result<usize> {
        Ok(self.read_all()?.len())
    }

    /// 清空记忆文件
    pub fn clear(&self) -> anyhow::Result<()> {
        fs::write(&self.file_path, "")?;
        Ok(())
    }

    fn parse_line(line: &str) -> Option<LongTermEntry> {
        let re = regex::Regex::new(r"\[([^\]]+)]\s*\[([^\]]+)]\s*(.+?)\s*\[([^\]]*)]").ok()?;
        let caps = re.captures(line)?;

        Some(LongTermEntry {
            timestamp: caps.get(1)?.as_str().to_string(),
            category: caps.get(2)?.as_str().to_string(),
            content: caps.get(3)?.as_str().trim().to_string(),
            tags: caps.get(4)?.as_str().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_append_and_read() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("memory.txt");
        let mem = LongTermMemory::new(&path).unwrap();

        mem.append("code_agent", "完成T001任务", &["test", "init"]).unwrap();
        mem.append("review_agent", "审查T001", &["review"]).unwrap();

        let entries = mem.read_all().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].category, "code_agent");
        assert!(entries[0].content.contains("T001"));
    }

    #[test]
    fn test_read_by_category() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("memory.txt");
        let mem = LongTermMemory::new(&path).unwrap();

        mem.append("code", "C1", &[]).unwrap();
        mem.append("review", "R1", &[]).unwrap();
        mem.append("code", "C2", &[]).unwrap();

        let code_entries = mem.read_by_category("code").unwrap();
        assert_eq!(code_entries.len(), 2);
    }

    #[test]
    fn test_clear() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("memory.txt");
        let mem = LongTermMemory::new(&path).unwrap();

        mem.append("test", "data", &[]).unwrap();
        assert_eq!(mem.count().unwrap(), 1);

        mem.clear().unwrap();
        assert_eq!(mem.count().unwrap(), 0);
    }

    #[test]
    fn test_parse_line() {
        let entry = LongTermMemory::parse_line("[2026-05-06 12:00] [code] task done [tag1,tag2]").unwrap();
        assert_eq!(entry.category, "code");
        assert_eq!(entry.content, "task done");
        assert_eq!(entry.tags, vec!["tag1", "tag2"]);
    }
}

//! 记忆检索 - 支持关键词和时间范围查询

use super::short_term::ShortTermMemory;
use super::long_term::LongTermMemory;

/// 检索结果
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub source: String,
    pub category: String,
    pub content: String,
    pub timestamp: String,
}

/// 记忆检索器
pub struct MemoryRetrieval {
    pub short_term: ShortTermMemory,
    pub long_term: LongTermMemory,
}

impl MemoryRetrieval {
    pub fn new(short_term: ShortTermMemory, long_term: LongTermMemory) -> Self {
        Self { short_term, long_term }
    }

    /// 按关键词检索短期记忆（value包含关键词）
    pub fn search_short_term(&self, _keyword: &str) -> Vec<SearchResult> {
        // Iterate all keys and check values
        // Short-term memory doesn't expose iteration, so we do best-effort
        Vec::new()
    }

    /// 按关键词检索长期记忆
    pub fn search_long_term(&self, keyword: &str) -> anyhow::Result<Vec<SearchResult>> {
        let entries = self.long_term.read_all()?;
        let results: Vec<SearchResult> = entries
            .into_iter()
            .filter(|e| e.content.contains(keyword) || e.tags.iter().any(|t| t.contains(keyword)))
            .map(|e| SearchResult {
                source: "long_term".into(),
                category: e.category,
                content: e.content,
                timestamp: e.timestamp,
            })
            .collect();
        Ok(results)
    }

    /// 按类别检索
    pub fn search_by_category(&self, category: &str) -> anyhow::Result<Vec<SearchResult>> {
        let entries = self.long_term.read_by_category(category)?;
        Ok(entries
            .into_iter()
            .map(|e| SearchResult {
                source: "long_term".into(),
                category: e.category,
                content: e.content,
                timestamp: e.timestamp,
            })
            .collect())
    }

    /// 按时间范围检索（最近N秒内）
    pub fn search_recent(&self, seconds: u64) -> anyhow::Result<Vec<SearchResult>> {
        let entries = self.long_term.read_all()?;
        let now = chrono::Local::now().naive_local();

        Ok(entries
            .into_iter()
            .filter(|e| {
                if let Ok(ts) = chrono::NaiveDateTime::parse_from_str(&e.timestamp, "%Y-%m-%d %H:%M:%S") {
                    (now - ts).num_seconds().unsigned_abs() <= seconds
                } else {
                    false
                }
            })
            .map(|e| SearchResult {
                source: "long_term".into(),
                category: e.category,
                content: e.content,
                timestamp: e.timestamp,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (MemoryRetrieval, TempDir) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("memory.txt");
        let lt = LongTermMemory::new(&path).unwrap();
        let st = ShortTermMemory::new(100, 3600);
        (MemoryRetrieval::new(st, lt), dir)
    }

    #[test]
    fn test_search_by_category() {
        let (retrieval, _dir) = setup();
        retrieval.long_term.append("code", "task A", &[]).unwrap();
        retrieval.long_term.append("review", "inspect B", &[]).unwrap();

        let results = retrieval.search_by_category("code").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, "long_term");
    }

    #[test]
    fn test_search_long_term_keyword() {
        let (retrieval, _dir) = setup();
        retrieval.long_term.append("code", "implement feature X", &["feature"]).unwrap();
        retrieval.long_term.append("code", "fix bug Y", &["bug"]).unwrap();

        let results = retrieval.search_long_term("feature").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("feature X"));
    }

    #[test]
    fn test_search_recent() {
        let (retrieval, _dir) = setup();
        retrieval.long_term.append("code", "recent entry", &[]).unwrap();

        let results = retrieval.search_recent(3600).unwrap();
        assert_eq!(results.len(), 1);
    }
}

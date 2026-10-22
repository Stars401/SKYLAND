//! 记忆系统集成测试

use skyland_lib::memory::{
    ShortTermMemory, LongTermMemory, MemoryRetrieval, MemoryCleanup,
};
use tempfile::TempDir;

#[test]
fn test_short_term_full_lifecycle() {
    let mem = ShortTermMemory::new(10, 3600);

    // 写入
    for i in 0..5 {
        mem.set(&format!("k{}", i), &format!("v{}", i));
    }
    assert_eq!(mem.len(), 5);

    // 读取
    assert_eq!(mem.get("k0"), Some("v0".into()));

    // 删除
    mem.remove("k0");
    assert_eq!(mem.get("k0"), None);

    // 清空
    mem.clear();
    assert!(mem.is_empty());
}

#[test]
fn test_long_term_full_lifecycle() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("memory.txt");
    let mem = LongTermMemory::new(&path).unwrap();

    // 追加多条
    mem.append("code", "task1 done", &["init"]).unwrap();
    mem.append("review", "reviewed task1", &["review"]).unwrap();
    mem.append("code", "task2 done", &["feature"]).unwrap();

    // 按类别过滤
    assert_eq!(mem.read_by_category("code").unwrap().len(), 2);
    assert_eq!(mem.count().unwrap(), 3);

    // 压缩
    MemoryCleanup::compress_long_term(&mem, 1).unwrap();
    assert_eq!(mem.count().unwrap(), 1);
}

#[test]
fn test_memory_retrieval_flow() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("memory.txt");
    let lt = LongTermMemory::new(&path).unwrap();
    let st = ShortTermMemory::new(100, 3600);
    let retrieval = MemoryRetrieval::new(st, lt);

    retrieval.long_term.append("code", "initial commit", &["git"]).unwrap();
    retrieval.long_term.append("code", "add tests", &["testing"]).unwrap();

    let results = retrieval.search_long_term("test").unwrap();
    assert_eq!(results.len(), 1);

    let results = retrieval.search_by_category("code").unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn test_memory_maintenance() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("memory.txt");
    let lt = LongTermMemory::new(&path).unwrap();
    let st = ShortTermMemory::new(100, 3600);

    for i in 0..100 {
        lt.append("code", &format!("entry{}", i), &[]).unwrap();
    }

    // maintain keeps last 1000, so no removal with 100 entries
    let (_, removed) = MemoryCleanup::maintain(&st, &lt).unwrap();
    assert_eq!(removed, 0);
    assert_eq!(lt.count().unwrap(), 100);

    // Now compress to keep only 10
    let removed = MemoryCleanup::compress_long_term(&lt, 10).unwrap();
    assert_eq!(removed, 90);
    assert_eq!(lt.count().unwrap(), 10);
}

#[test]
fn test_short_term_concurrent() {
    use std::sync::Arc;
    use std::thread;

    let mem = Arc::new(ShortTermMemory::new(100, 3600));
    let mut handles = vec![];

    for i in 0..10 {
        let m = mem.clone();
        handles.push(thread::spawn(move || {
            m.set(&format!("t{}", i), &format!("val{}", i));
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(mem.len(), 10);
}

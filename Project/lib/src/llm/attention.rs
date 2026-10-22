use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// 单一工具在一次会话中被调用超过此次数即视为发散。
/// 取值依据：默认一轮一工具，`max_rounds` 上限 50。正常任务（读若干文件、跑若干命令）
/// 远达不到 15；而「反复读同一个文件换写法」「同一工具无脑重试」会迅速越过。
const TOOL_CALL_WARN: u32 = 15;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttentionDecision {
    Continue,
    InjectFeedback(String),
    Stop(String),
}

#[derive(Debug, Default)]
pub struct AttentionGuard {
    /// "工具名:规范化参数" -> "成功?:结果" 指纹
    seen: HashMap<String, String>,
    /// 已经就同一件事给过反馈的键（同一件事只提醒一次，再犯才停）
    feedback_keys: HashSet<String>,
    /// 工具名 -> 本次会话已调用次数
    tool_counts: HashMap<String, u32>,
    /// 已就调用频次提醒过的工具（提醒一次，再超才停）
    tool_warned: HashSet<String>,
    no_progress_rounds: u8,
}

impl AttentionGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe_tool(
        &mut self,
        name: &str,
        args: &str,
        result: &str,
        success: bool,
    ) -> AttentionDecision {
        // ── 1. 调用频次 ──
        // 必须放在去重之前：参数每轮都不同的「换皮重试」在去重逻辑里是全新的键，
        // 只有按**工具名**计数才拦得住。这也是思维链发散最常见的形态。
        let count = self.tool_counts.entry(name.to_string()).or_insert(0);
        *count = count.saturating_add(1);
        if *count > TOOL_CALL_WARN {
            if !self.tool_warned.insert(name.to_string()) {
                return AttentionDecision::Stop(format!(
                    "工具 {} 已被调用 {} 次且在收到反馈后仍继续，判定为无效循环，已停止以免继续消耗 token。",
                    name, *count
                ));
            }
            return AttentionDecision::InjectFeedback(format!(
                "工具 {} 已调用 {} 次。请停止重复尝试：若信息已足够请直接给出结论；若确实受阻，请说明卡在哪一步并请求必要信息。",
                name, *count
            ));
        }

        // ── 2. 同参同结果去重 ──
        let key = format!("{}:{}", name, canonical_args(args));
        let fingerprint = format!("{}:{}", success, result);
        if self.seen.get(&key) == Some(&fingerprint) {
            if !self.feedback_keys.insert(key.clone()) {
                return AttentionDecision::Stop(
                    "同一工具调用在反馈后仍重复，已停止以避免无效循环。".into(),
                );
            }
            return AttentionDecision::InjectFeedback(
                "检测到相同工具、参数和结果的重复调用；请改变策略或直接给出结论。".into(),
            );
        }
        self.seen.insert(key, fingerprint);
        AttentionDecision::Continue
    }

    pub fn observe_output(&mut self) {
        self.no_progress_rounds = 0;
    }

    pub fn observe_plan(&mut self) {
        self.no_progress_rounds = 0;
    }

    pub fn observe_round_without_progress(&mut self) {
        self.no_progress_rounds = self.no_progress_rounds.saturating_add(1);
    }

    pub fn should_stop_for_no_progress(&self) -> bool {
        self.no_progress_rounds >= 3
    }
}

fn canonical_args(args: &str) -> String {
    serde_json::from_str::<Value>(args)
        .map(canonical_value)
        .unwrap_or_else(|_| normalize_str(args))
}

/// 字符串值规范化：去首尾空白 + 连续空白折叠为一个空格。
///
/// 为什么要做：模型重试时极常见的"换皮"是给同一路径加个空格
/// （`" a.rs "` vs `"a.rs"`）。不归一的话去重键不同，重复检测被直接绕过，
/// 于是同一个文件被反复读取、白烧 token —— 而表面看每次参数都不一样。
/// 只做空白归一、不动大小写与内容：后者会把 Linux 上 `A.rs` 与 `a.rs`
/// 这类**真实不同**的文件误判为同一个，风险大于收益。
fn normalize_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.trim().chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

fn canonical_value(value: Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            let fields = keys.into_iter().map(|key| {
                format!("{}:{}", key, canonical_value(map[key].clone()))
            }).collect::<Vec<_>>();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(values) => format!(
            "[{}]",
            values.into_iter().map(canonical_value).collect::<Vec<_>>().join(",")
        ),
        Value::String(s) => normalize_str(&s),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_identical_result_is_inhibited() {
        let mut guard = AttentionGuard::new();
        assert_eq!(
            guard.observe_tool("read_file", "{\"path\":\"a.rs\"}", "same", true),
            AttentionDecision::Continue
        );
        assert!(matches!(
            guard.observe_tool("read_file", "{\"path\":\"a.rs\"}", "same", true),
            AttentionDecision::InjectFeedback(_)
        ));
    }

    #[test]
    fn new_output_resets_no_progress_counter() {
        let mut guard = AttentionGuard::new();
        guard.observe_output();
        assert!(!guard.should_stop_for_no_progress());
    }

    /// 参数里的多余空白不应成为"新调用"——否则去重被绕过。
    #[test]
    fn whitespace_variation_is_canonicalized() {
        assert_eq!(
            canonical_args("{\"path\":\"a.rs\"}"),
            canonical_args("{\"path\":\"  a.rs  \"}")
        );
        assert_eq!(
            canonical_args("{\"cmd\":\"ls   -la\"}"),
            canonical_args("{\"cmd\":\" ls -la \"}")
        );
    }

    /// 键序不同也不应算新调用（原本就有，保持不回归）。
    #[test]
    fn key_order_is_canonicalized() {
        assert_eq!(
            canonical_args("{\"a\":1,\"b\":2}"),
            canonical_args("{\"b\":2,\"a\":1}")
        );
    }

    /// 大小写不同是**真实不同**的文件，不可归一（避免误杀 Linux 场景）。
    #[test]
    fn case_difference_is_preserved() {
        assert_ne!(
            canonical_args("{\"path\":\"A.rs\"}"),
            canonical_args("{\"path\":\"a.rs\"}")
        );
    }

    /// 核心：同工具、参数每轮都不同（换皮重试）也必须被频次计数拦住。
    #[test]
    fn same_tool_with_varied_args_is_capped() {
        let mut guard = AttentionGuard::new();
        for i in 0..TOOL_CALL_WARN {
            assert_eq!(
                guard.observe_tool("read_file", &format!("{{\"path\":\"f{}.rs\"}}", i), "r", true),
                AttentionDecision::Continue,
                "第 {} 次不该被拦",
                i
            );
        }
        // 越过阈值 → 首次提醒
        assert!(matches!(
            guard.observe_tool("read_file", "{\"path\":\"over.rs\"}", "r", true),
            AttentionDecision::InjectFeedback(_)
        ));
        // 收到反馈后仍继续 → 停止
        assert!(matches!(
            guard.observe_tool("read_file", "{\"path\":\"over2.rs\"}", "r", true),
            AttentionDecision::Stop(_)
        ));
    }

    /// 不同工具各自计数，互不牵连。
    #[test]
    fn tool_counters_are_independent() {
        let mut guard = AttentionGuard::new();
        for i in 0..(TOOL_CALL_WARN + 1) {
            guard.observe_tool("read_file", &format!("{{\"path\":\"f{}.rs\"}}", i), "r", true);
        }
        assert_eq!(
            guard.observe_tool("run_bash", "{\"cmd\":\"ls\"}", "r", true),
            AttentionDecision::Continue
        );
    }
}
//! 流式响应 — SSE事件流

use tokio::sync::mpsc;
use super::provider::{ToolCall, Usage};

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Text(String),
    Reasoning(String),
    ToolCalls(Vec<ToolCall>),
    /// API 真实用量（DeepSeek 等在流末尾 chunk 附带）— 供上下文条真实计量
    Usage(Usage),
    /// 截断原因（"length" = 被 max_tokens 截断；外层未流出内容时可提升上限重试本轮）
    FinishReason(String),
    Done,
    Error(String),
}

/// 流式接收器
pub struct StreamReceiver {
    pub(crate) rx: mpsc::Receiver<StreamEvent>,
}

impl StreamReceiver {
    pub async fn recv(&mut self) -> Option<StreamEvent> {
        self.rx.recv().await
    }
}

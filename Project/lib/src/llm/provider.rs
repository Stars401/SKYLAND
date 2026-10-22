//! Provider trait定义

use super::message::Message;
use super::tool::Tool;
use super::stream::StreamReceiver;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// LLM响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: String,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String, // JSON string
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// LLM Provider抽象
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[Tool],
    ) -> anyhow::Result<LlmResponse>;

    async fn chat_stream(
        &self,
        messages: &[Message],
        tools: &[Tool],
    ) -> anyhow::Result<StreamReceiver>;

    fn name(&self) -> &str;
    fn models(&self) -> Vec<String>;
}

#[derive(Debug, Clone, Default)]
pub struct LlmConfig {
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub api_key: String,
    pub base_url: String,
    /// 完整 URL 模式：base_url 已含版本路径（如 /v1）或为完整端点 — 禁止自动补 /v1。
    /// false（默认）= 智能模式：裸主机自动补 /v1，已含路径则原样使用。
    pub is_full_url: bool,
    pub context_length: u32,
    /// 思考强度（low/medium/high/max/ultra）— None = 不发送思考参数
    /// 请求级参数：Provider 按请求克隆后设置，避免共享状态被并发污染
    pub reasoning_effort: Option<String>,
}

impl LlmConfig {
    pub fn new(model: &str, api_key: &str) -> Self {
        Self {
            model: model.to_string(),
            max_tokens: 4096,
            temperature: 0.7,
            api_key: api_key.to_string(),
            base_url: String::new(),
            is_full_url: false,
            context_length: 128000,
            reasoning_effort: None,
        }
    }
}

/// 解析 OpenAI 兼容 API 根地址（版本段之后、端点之前），调用方在其后拼接 /chat/completions 或 /models。
/// - is_full_url=true（完整 URL 模式）：信任用户输入，不自动补 /v1（勾选即承诺已含版本路径）
/// - is_full_url=false（智能模式）：裸主机（scheme://host[:port] 无任何路径）自动补 /v1 —
///   newapi 网关/Ollama/vLLM 等 OpenAI 兼容服务的标准布局；已含路径（/v1、/v4、/v1beta 等）一律不动
/// 两种模式都会剥离误粘贴的 /chat/completions 端点尾巴，保证「粘贴完整端点」与「粘贴根地址」等价。
pub fn resolve_api_base(raw: &str, is_full_url: bool) -> String {
    let mut base = raw.trim().trim_end_matches('/').to_string();
    if base.is_empty() { return base; }
    for tail in ["/chat/completions", "/completions"] {
        if base.ends_with(tail) {
            base.truncate(base.len() - tail.len());
            break;
        }
    }
    if !is_full_url && is_bare_host(&base) {
        base.push_str("/v1");
    }
    base
}

/// scheme://host[:port] 之后没有任何路径 → 裸主机
fn is_bare_host(url: &str) -> bool {
    let rest = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => url,
    };
    !rest.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_mode_adds_v1_to_bare_host() {
        // newapi 网关典型配置：裸域名（可带端口）→ 自动补 /v1（此前聊天不补、拉模型补 → 配置成功但聊天 404）
        assert_eq!(resolve_api_base("https://gw.example.com", false), "https://gw.example.com/v1");
        assert_eq!(resolve_api_base("https://gw.example.com:3000/", false), "https://gw.example.com:3000/v1");
        assert_eq!(resolve_api_base("http://localhost:11434", false), "http://localhost:11434/v1");
        assert_eq!(resolve_api_base("https://api.deepseek.com", false), "https://api.deepseek.com/v1");
    }

    #[test]
    fn smart_mode_keeps_existing_path_verbatim() {
        // 已含版本路径 — 含非 /v1 的 GLM /v4 与 Gemini /v1beta/openai，一律不误补 /v1
        assert_eq!(resolve_api_base("https://gw.example.com/v1", false), "https://gw.example.com/v1");
        assert_eq!(resolve_api_base("https://open.bigmodel.cn/api/paas/v4", false), "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(resolve_api_base("https://generativelanguage.googleapis.com/v1beta/openai", false), "https://generativelanguage.googleapis.com/v1beta/openai");
        assert_eq!(resolve_api_base("https://dashscope.aliyuncs.com/compatible-mode/v1", false), "https://dashscope.aliyuncs.com/compatible-mode/v1");
    }

    #[test]
    fn smart_mode_strips_pasted_endpoint_then_normalizes() {
        // 用户直接粘贴完整端点：剥尾 → 裸主机补 /v1 → 调用方重新拼接
        assert_eq!(resolve_api_base("https://gw.example.com/v1/chat/completions", false), "https://gw.example.com/v1");
        assert_eq!(resolve_api_base("https://gw.example.com/chat/completions", false), "https://gw.example.com/v1");
    }

    #[test]
    fn full_url_mode_never_injects_v1() {
        // 勾选完整 URL：禁止自动补 /v1 — 勾选框的真实语义（与生图面板一致）
        assert_eq!(resolve_api_base("https://gw.example.com", true), "https://gw.example.com");
        assert_eq!(resolve_api_base("https://gw.example.com:3000", true), "https://gw.example.com:3000");
        assert_eq!(resolve_api_base("https://gw.example.com/v1", true), "https://gw.example.com/v1");
    }

    #[test]
    fn full_url_mode_endpoint_paste_equals_verbatim() {
        // 勾选 + 粘贴完整端点 → 剥尾后由调用方拼回 = 原样使用
        assert_eq!(resolve_api_base("https://gw.example.com/v1/chat/completions/", true), "https://gw.example.com/v1");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(resolve_api_base("", false), "");
        assert_eq!(resolve_api_base("   ", true), "");
    }
}

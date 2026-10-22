//! 消息类型定义

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallRequest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn system(content: &str) -> Self {
        Self { role: Role::System, content: content.into(), tool_calls: None, tool_call_id: None, name: None }
    }

    pub fn user(content: &str) -> Self {
        Self { role: Role::User, content: content.into(), tool_calls: None, tool_call_id: None, name: None }
    }

    /// Create a multimodal user message with text + image (OpenAI vision format).
    /// The `content` field stores a JSON array string; `openai.rs` `body()` detects
    /// this via `is_multimodal_content` and embeds it as a raw JSON array instead
    /// of an escaped string.
    ///
    /// 键序手工固定：serde_json 默认按字母序输出对象键，`{"type","text"}` 会变成
    /// `{"text","type"}`，导致下游前缀检测失效、图片以转义字符串发给 LLM。
    pub fn user_multimodal(text: &str, image_data_url: &str) -> Self {
        Self::user_multimodal_multi(text, &[image_data_url.to_string()])
    }

    /// 多图版多模态消息：text part + N 个 image_url part（OpenAI vision 格式）。
    /// 单图构造复用本函数，保持相同的键序与转义规则。
    pub fn user_multimodal_multi(text: &str, images: &[String]) -> Self {
        let mut parts = vec![format!(
            "{{\"type\":\"text\",\"text\":{}}}",
            serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into()),
        )];
        for img in images {
            parts.push(format!(
                "{{\"type\":\"image_url\",\"image_url\":{{\"url\":{}}}}}",
                serde_json::to_string(img).unwrap_or_else(|_| "\"\"".into()),
            ));
        }
        let content = format!("[{}]", parts.join(","));
        Self { role: Role::User, content, tool_calls: None, tool_call_id: None, name: None }
    }

    /// 判断 content 是否为多模态 JSON 数组（顺序无关判定，勿用字符串前缀）。
    /// 判据：可解析为非空数组，且每个元素是带字符串 "type" 字段的对象。
    pub fn is_multimodal_content(content: &str) -> bool {
        let trimmed = content.trim_start();
        if !trimmed.starts_with('[') { return false; }
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(v) => v.as_array().map(|arr| {
                !arr.is_empty() && arr.iter().all(|p| p.get("type").and_then(|t| t.as_str()).is_some())
            }).unwrap_or(false),
            Err(_) => false,
        }
    }

    pub fn assistant(content: &str) -> Self {
        Self { role: Role::Assistant, content: content.into(), tool_calls: None, tool_call_id: None, name: None }
    }

    pub fn tool(content: &str, tool_call_id: &str) -> Self {
        Self { role: Role::Tool, content: content.into(), tool_calls: None, tool_call_id: Some(tool_call_id.into()), name: None }
    }

    pub fn assistant_with_tools(content: &str, tool_calls: &[crate::llm::provider::ToolCall]) -> Self {
        let calls: Vec<ToolCallRequest> = tool_calls.iter().map(|tc| ToolCallRequest {
            id: tc.id.clone(), call_type: "function".into(),
            function: FunctionCall { name: tc.name.clone(), arguments: tc.arguments.clone() },
        }).collect();
        Self { role: Role::Assistant, content: content.into(), tool_calls: Some(calls), tool_call_id: None, name: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_creation() {
        let sys = Message::system("You are a helpful assistant.");
        assert_eq!(sys.role, Role::System);

        let user = Message::user("Hello!");
        assert_eq!(user.role, Role::User);
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message::user("test");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("test"));
        assert!(json.contains("user"));
    }

    #[test]
    fn test_multimodal_content_roundtrip_and_detection() {
        let m = Message::user_multimodal("看这张图", "data:image/png;base64,AAAA");
        // 固定键序：前缀可直接匹配，且顺序无关判定也成立
        assert!(m.content.starts_with("[{\"type\":\"text\""));
        assert!(Message::is_multimodal_content(&m.content));
        let parsed: serde_json::Value = serde_json::from_str(&m.content).unwrap();
        let arr = parsed.as_array().unwrap();
        assert!(arr.iter().any(|p| p.get("image_url").is_some()));
        assert!(arr.iter().any(|p| p["text"] == "看这张图"));
    }

    #[test]
    fn test_multimodal_detection_negative() {
        assert!(!Message::is_multimodal_content("普通文本"));
        assert!(!Message::is_multimodal_content("[1, 2, 3]"));
        assert!(!Message::is_multimodal_content("[{\"foo\": \"bar\"}]")); // 无 type 字段
        assert!(!Message::is_multimodal_content(""));
    }
}

//! OpenAI Provider — 完整流式实现
use super::message::Message;
use super::provider::{LlmConfig, LlmProvider, LlmResponse, Usage, ToolCall, resolve_api_base};
use super::tool::Tool;
use super::stream::{StreamEvent, StreamReceiver};
use async_trait::async_trait;
use serde_json::Value;
use futures::StreamExt;

/// Clone 共享 reqwest::Client（内部为连接池句柄），深拷贝 config — 请求级克隆后
/// 可安全设置 reasoning_effort 而不影响共享的 Provider 实例
#[derive(Clone)]
pub struct OpenAiProvider { config: LlmConfig, client: reqwest::Client }

impl OpenAiProvider {
    pub fn new(config: LlmConfig) -> Self { Self { config, client: reqwest::Client::new() } }

    /// 请求级思考强度：consumes self（按请求克隆后链式调用）
    pub fn with_effort(mut self, effort: Option<String>) -> Self {
        self.config.reasoning_effort = effort;
        self
    }

    /// 当前输出上限（截断恢复时读取）
    pub fn max_tokens(&self) -> u32 { self.config.max_tokens }

    /// 请求级输出上限 — max_tokens 截断恢复用（对齐 Claude Code MAX_OUTPUT_TOKENS_RECOVERY）
    pub fn with_max_tokens(mut self, n: u32) -> Self {
        self.config.max_tokens = n;
        self
    }

    fn url(&self) -> String {
        if self.config.base_url.is_empty() { "https://api.openai.com/v1/chat/completions".into() }
        else { format!("{}/chat/completions", resolve_api_base(&self.config.base_url, self.config.is_full_url)) }
    }

    fn body(&self, msgs: &[Message], tools: &[Tool], stream: bool) -> Value {
        let m: Vec<Value> = msgs.iter().map(|m| {
            // 多模态内容以 JSON 数组字符串存储 — 顺序无关判定后嵌入原始数组，
            // 其余文本一律作为字符串发送（避免把用户粘贴的 JSON 误判成多模态）
            let content_value: Value = if Message::is_multimodal_content(&m.content) {
                serde_json::from_str(&m.content).unwrap_or_else(|_| Value::String(m.content.clone()))
            } else {
                Value::String(m.content.clone())
            };
            let mut v = serde_json::json!({"role": match m.role {
                super::message::Role::System => "system", super::message::Role::User => "user",
                super::message::Role::Assistant => "assistant", super::message::Role::Tool => "tool",
            }, "content": content_value});
            if let Some(ref id) = m.tool_call_id { v["tool_call_id"] = serde_json::Value::String(id.clone()); }
            // 序列化tool_calls字段
            if let Some(ref tool_calls) = m.tool_calls {
                let calls: Vec<Value> = tool_calls.iter().map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "type": tc.call_type,
                        "function": {
                            "name": tc.function.name,
                            "arguments": tc.function.arguments
                        }
                    })
                }).collect();
                v["tool_calls"] = serde_json::Value::Array(calls);
            }
            v
        }).collect();
        let mut b = serde_json::json!({"model": self.config.model, "messages": m, "max_tokens": self.config.max_tokens, "temperature": self.config.temperature, "stream": stream});
        if !tools.is_empty() {
            b["tools"] = serde_json::json!(tools.iter().map(|t| serde_json::json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.parameters}})).collect::<Vec<_>>());
        }
        self.apply_reasoning(&mut b);
        b
    }

    /// 思考参数按厂商适配：
    /// - DeepSeek（思考型：deepseek-flash / deepseek-v4 系）：`reasoning_effort`（low/high/max）
    ///   + `thinking:{type:enabled}`；effort=off 时仅发 `thinking:{type:disabled}`
    /// - 退役名（deepseek-chat，2026-07-24 退役）/ 非思考型：一个思考参数都不发 —
    ///   官方对这类模型静默忽略参数，发了只会掩盖"设置未生效"的事实
    /// - 其余 OpenAI 兼容端点：仅当模型名匹配已知推理型时发送 `reasoning_effort`（low/medium/high）；
    ///   off 时不发任何思考参数
    fn apply_reasoning(&self, b: &mut Value) {
        let Some(effort) = self.config.reasoning_effort.as_deref() else { return };
        if effort.is_empty() { return; }
        let off = effort == "off" || effort == "none";
        let is_deepseek = self.config.base_url.to_lowercase().contains("deepseek")
            || self.config.model.to_lowercase().contains("deepseek");
        if is_deepseek {
            if !self.thinking_supported() { return; }
            if off {
                b["thinking"] = serde_json::json!({ "type": "disabled" });
                return;
            }
            let ds = match effort { "low" => "low", "max" | "ultra" => "max", _ => "high" };
            b["reasoning_effort"] = serde_json::json!(ds);
            b["thinking"] = serde_json::json!({ "type": "enabled" });
        } else if !off && reasoning_capable_model(&self.config.model) {
            let oe = match effort { "low" => "low", "high" | "max" | "ultra" => "high", _ => "medium" };
            b["reasoning_effort"] = serde_json::json!(oe);
        }
    }

    /// 模型是否真正支持思考模式（参数能生效，而非被兼容层静默忽略）。
    /// DeepSeek 官方仅 deepseek-flash / deepseek-v4 系支持思考（r1/reasoner 为纯思考型）；
    /// deepseek-chat / deepseek-v3 走兼容路由，思考参数被丢弃。
    pub fn thinking_supported(&self) -> bool {
        let m = self.config.model.to_lowercase();
        let is_deepseek = self.config.base_url.to_lowercase().contains("deepseek") || m.contains("deepseek");
        if is_deepseek {
            m.contains("deepseek-flash") || m.contains("deepseek-v4")
                || m.contains("deepseek-r1") || m.contains("reasoner")
        } else {
            reasoning_capable_model(&self.config.model)
        }
    }

    fn parse(&self, json: &Value) -> anyhow::Result<LlmResponse> {
        let c = &json["choices"][0]; let m = &c["message"];
        Ok(LlmResponse {
            content: m["content"].as_str().unwrap_or("").into(),
            tool_calls: m.get("tool_calls").and_then(|t| t.as_array()).map(|a| a.iter().filter_map(|t| Some(ToolCall { id: t["id"].as_str()?.into(), name: t["function"]["name"].as_str()?.into(), arguments: t["function"]["arguments"].as_str().unwrap_or("{}").into() })).collect()).unwrap_or_default(),
            finish_reason: c["finish_reason"].as_str().unwrap_or("stop").into(),
            usage: json.get("usage").map(|u| Usage { prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32, completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32, total_tokens: u["total_tokens"].as_u64().unwrap_or(0) as u32 }),
        })
    }
}

/// 非 2xx 响应的统一错误信息（含状态码与响应体前 300 字符）
async fn api_error(status: reqwest::StatusCode, body: String) -> anyhow::Error {
    anyhow::anyhow!("API 返回 {}: {}", status, body.chars().take(300).collect::<String>())
}

/// 已知支持 reasoning_effort 的非 DeepSeek 模型名特征（o 系 / gpt-5+ / reasoner / R1 / Qwen3 / GLM-5 / Kimi-K3 / Grok-4+）
fn reasoning_capable_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m.contains("o1") || m.contains("o3") || m.contains("o4")
        || m.contains("gpt-5") || m.contains("gpt-6")
        || m.contains("reasoner") || m.contains("r1")
        || m.contains("qwen3") || m.contains("glm-5") || m.contains("glm6")
        || m.contains("kimi-k3") || m.contains("grok-4") || m.contains("thinking")
}

/// 从字节缓冲中取出完整的 UTF-8 前缀（多字节字符被 chunk 边界截断时保留尾部等待下个 chunk）。
/// 直接 from_utf8_lossy 逐 chunk 转换会把中文劈成 U+FFFD 乱码。
fn drain_complete_utf8(buf: &mut Vec<u8>) -> String {
    match std::str::from_utf8(buf) {
        Ok(s) => { let out = s.to_string(); buf.clear(); out }
        Err(e) => {
            let valid = e.valid_up_to();
            let mut consumed = valid;
            let out = String::from_utf8_lossy(&buf[..valid]).into_owned();
            if let Some(n) = e.error_len() { consumed += n; } // 无效字节序列，跳过防卡死
            buf.drain(..consumed);
            out
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat(&self, msgs: &[Message], tools: &[Tool]) -> anyhow::Result<LlmResponse> {
        let resp = self.client.post(&self.url()).header("Authorization", format!("Bearer {}", self.config.api_key)).json(&self.body(msgs, tools, false)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(api_error(status, body).await);
        }
        self.parse(&resp.json().await?)
    }

    async fn chat_stream(&self, msgs: &[Message], tools: &[Tool]) -> anyhow::Result<StreamReceiver> {
        let resp = self.client.post(&self.url()).header("Authorization", format!("Bearer {}", self.config.api_key)).json(&self.body(msgs, tools, true)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            // 401/429/5xx 等：错误体不是 SSE，逐字节流出会导致"静默空回复"。
            // 立即读取错误体并以 Err 返回，让上层发出 Error 事件。
            let body = resp.text().await.unwrap_or_default();
            return Err(api_error(status, body).await);
        }
        let mut byte_stream = resp.bytes_stream();
        let (tx, rx) = tokio::sync::mpsc::channel(128);

        tokio::spawn(async move {
            let mut buffer = String::new();
            let mut pending: Vec<u8> = Vec::new();
            let mut tool_calls_buf: Vec<ToolCall> = Vec::new();
            let mut length_sent = false;

            while let Some(Ok(chunk)) = byte_stream.next().await {
                pending.extend_from_slice(&chunk);
                // 统一 \r\n → \n：部分兼容厂商用 CRLF 分隔 SSE 事件，固定找 "\n\n" 会永远匹配不到
                buffer.push_str(&drain_complete_utf8(&mut pending).replace("\r\n", "\n"));

                while let Some(pos) = buffer.find("\n\n") {
                    let event_text = buffer[..pos].trim().to_string();
                    buffer = buffer[pos+2..].to_string();

                    // SSE 规范：多个 data: 行以 \n 连接为完整载荷
                    let mut data_parts: Vec<String> = Vec::new();
                    for line in event_text.lines() {
                        let trimmed = line.trim();
                        if trimmed.is_empty() || trimmed.starts_with(':') { continue; }
                        if let Some(d) = trimmed.strip_prefix("data:") {
                            data_parts.push(d.strip_prefix(' ').unwrap_or(d).to_string());
                        }
                    }
                    let data = data_parts.join("\n");
                    if data.is_empty() { continue; }
                    if data.trim() == "[DONE]" {
                        if !tool_calls_buf.is_empty() {
                            let _ = tx.send(StreamEvent::ToolCalls(tool_calls_buf.clone())).await;
                        }
                        let _ = tx.send(StreamEvent::Done).await;
                        return;
                    }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
                        let delta = &v["choices"][0]["delta"];

                        if let Some(content) = delta["content"].as_str() {
                            if !content.is_empty() {
                                let _ = tx.send(StreamEvent::Text(content.to_string())).await;
                            }
                        }
                        // 思维链（DeepSeek-R1 / Gemini thinking 等）：reasoning_content / thinking
                        if let Some(reasoning) = delta["reasoning_content"].as_str()
                            .or_else(|| delta["thinking"].as_str()) {
                            if !reasoning.is_empty() {
                                let _ = tx.send(StreamEvent::Reasoning(reasoning.to_string())).await;
                            }
                        }

                        if let Some(tool_calls) = delta["tool_calls"].as_array() {
                            for tc in tool_calls {
                                let index = tc["index"].as_u64().unwrap_or(0) as usize;
                                while tool_calls_buf.len() <= index {
                                    tool_calls_buf.push(ToolCall {
                                        id: String::new(),
                                        name: String::new(),
                                        arguments: String::new(),
                                    });
                                }
                                if let Some(id) = tc["id"].as_str() {
                                    tool_calls_buf[index].id = id.to_string();
                                }
                                if let Some(name) = tc["function"]["name"].as_str() {
                                    tool_calls_buf[index].name = name.to_string();
                                }
                                if let Some(args) = tc["function"]["arguments"].as_str() {
                                    tool_calls_buf[index].arguments.push_str(args);
                                }
                            }
                        }

                        if let Some(reason) = v["choices"][0]["finish_reason"].as_str() {
                            if reason == "tool_calls" && !tool_calls_buf.is_empty() {
                                // 不发 Done — 外层 agent 循环会带着工具结果开启下一轮
                                let _ = tx.send(StreamEvent::ToolCalls(tool_calls_buf.clone())).await;
                                tool_calls_buf.clear();
                                return;
                            }
                            // 长度截断标记 — 外层在未流出任何内容时可提升 max_tokens 重试本轮
                            if reason == "length" && !length_sent {
                                length_sent = true;
                                let _ = tx.send(StreamEvent::FinishReason("length".into())).await;
                            }
                        }
                        // usage（DeepSeek 等厂商在末尾 chunk 附带）— 前端上下文条改用真实计量
                        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
                            let usage = Usage {
                                prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                                completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
                                total_tokens: u["total_tokens"].as_u64().unwrap_or(0) as u32,
                            };
                            let _ = tx.send(StreamEvent::Usage(usage)).await;
                        }
                    }
                }
            }

            // 流结束：处理残留字节与未完结的缓冲事件
            if !pending.is_empty() {
                buffer.push_str(&String::from_utf8_lossy(&pending).replace("\r\n", "\n"));
            }
            if let Some(pos) = buffer.find("\n\n") {
                let event_text = buffer[..pos].trim().to_string();
                let data: String = event_text.lines()
                    .filter_map(|l| l.trim().strip_prefix("data:"))
                    .map(|d| d.strip_prefix(' ').unwrap_or(d).to_string())
                    .collect();
                if data.trim() != "[DONE]" {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
                        if let Some(content) = v["choices"][0]["delta"]["content"].as_str() {
                            if !content.is_empty() {
                                let _ = tx.send(StreamEvent::Text(content.to_string())).await;
                            }
                        }
                    }
                }
            }
            if !tool_calls_buf.is_empty() {
                let _ = tx.send(StreamEvent::ToolCalls(tool_calls_buf)).await;
            }
            let _ = tx.send(StreamEvent::Done).await;
        });

        Ok(StreamReceiver { rx })
    }

    fn name(&self) -> &str { "openai" }
    fn models(&self) -> Vec<String> { vec!["gpt-4o".into(), "gpt-4-turbo".into(), "gpt-3.5-turbo".into()] }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_provider_name() { let p = OpenAiProvider::new(LlmConfig::new("gpt-4o","sk")); assert_eq!(p.name(), "openai"); }
    #[test] fn test_parse_response() { let p = OpenAiProvider::new(LlmConfig::new("g","k")); let r = p.parse(&serde_json::json!({"choices":[{"message":{"content":"Hi"},"finish_reason":"stop"}],"usage":{"total_tokens":10}})).unwrap(); assert_eq!(r.content, "Hi"); }
    #[test] fn test_body() { let p = OpenAiProvider::new(LlmConfig::new("m","k")); let b = p.body(&[Message::user("Hi")], &[], false); assert_eq!(b["model"], "m"); }

    // ─── 聊天端点 URL 解析（自定义网关修复的核心回归测试）───
    fn url_of(base: &str, is_full_url: bool) -> String {
        let mut cfg = LlmConfig::new("m", "k");
        cfg.base_url = base.into();
        cfg.is_full_url = is_full_url;
        OpenAiProvider::new(cfg).url()
    }

    #[test]
    fn url_bare_host_gets_v1_injected() {
        // newapi 网关裸域名：聊天端点必须补 /v1（此前直接拼 /chat/completions → 404 无回应）
        assert_eq!(url_of("https://gw.example.com", false), "https://gw.example.com/v1/chat/completions");
        assert_eq!(url_of("https://gw.example.com:3000", false), "https://gw.example.com:3000/v1/chat/completions");
    }

    #[test]
    fn url_existing_path_untouched() {
        // DeepSeek / OpenAI / GLM 等已含路径的地址 → 原样拼接
        assert_eq!(url_of("https://api.deepseek.com/v1", false), "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(url_of("https://api.openai.com/v1", false), "https://api.openai.com/v1/chat/completions");
        assert_eq!(url_of("https://open.bigmodel.cn/api/paas/v4", false), "https://open.bigmodel.cn/api/paas/v4/chat/completions");
    }

    #[test]
    fn url_full_url_mode_respects_user() {
        // 勾选完整 URL：不补 /v1；粘贴完整端点 → 原样使用
        assert_eq!(url_of("https://gw.example.com", true), "https://gw.example.com/chat/completions");
        assert_eq!(url_of("https://gw.example.com/v1/chat/completions", true), "https://gw.example.com/v1/chat/completions");
        assert_eq!(url_of("https://gw.example.com/v1/chat/completions", false), "https://gw.example.com/v1/chat/completions");
    }

    #[test]
    fn test_drain_utf8_split_multibyte() {
        // 中文 3 字节，chunk 在字符中间截断 → 必须保留尾部等待下一段
        let s = "水面制作";
        let bytes = s.as_bytes();
        let mut buf: Vec<u8> = bytes[..4].to_vec(); // 1.33 个字符
        let first = drain_complete_utf8(&mut buf);
        assert_eq!(first, "水");
        buf.extend_from_slice(&bytes[4..]);
        let rest = drain_complete_utf8(&mut buf);
        assert_eq!(format!("{}{}", first, rest), "水面制作"); // 无一字节丢失
        assert!(buf.is_empty());
    }

    #[test]
    fn test_drain_utf8_invalid_bytes_skipped() {
        let mut buf: Vec<u8> = vec![0xFF, b'a', b'b'];
        let out = drain_complete_utf8(&mut buf);
        assert_eq!(out, "");
        assert_eq!(buf, b"ab".to_vec());
    }

    /// 本地 mock HTTP 服务器 — 单连接，返回预置响应
    fn spawn_mock(status_line: &'static str, body: &'static str) -> String {
        // 免疫系统代理：reqwest 会读取 Windows 系统代理（如 Clash 127.0.0.1:7890），
        // 把发往本机 mock 的请求转交代理，高并发下偶发 502 导致测试闪烁。
        // 显式豁免回环地址（reqwest 在 Client 构建时读取，先于下方 provider 创建）。
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
        std::env::set_var("no_proxy", "127.0.0.1,localhost");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let response = format!("{}\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}", status_line, body.len(), body);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn test_stream_crlf_and_split_multibyte() {
        // CRLF 分隔 + 中文跨 chunk 截断 — 两种真实厂商场景
        let payload = "data: {\"choices\":[{\"delta\":{\"content\":\"水\"}}]}\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"面\"}}]}\r\n\r\ndata: [DONE]\r\n\r\n";
        let url = spawn_mock("HTTP/1.1 200 OK", payload);
        let mut cfg = LlmConfig::new("m", "k");
        cfg.base_url = url;
        let p = OpenAiProvider::new(cfg);
        let mut stream = p.chat_stream(&[Message::user("hi")], &[]).await.expect("chat_stream 不应失败");
        let mut text = String::new();
        while let Some(ev) = stream.recv().await {
            match ev {
                StreamEvent::Text(t) => text.push_str(&t),
                StreamEvent::Done => break,
                _ => {}
            }
        }
        assert_eq!(text, "水面");
    }

    #[tokio::test]
    async fn test_stream_http_error_surfaces() {
        let url = spawn_mock("HTTP/1.1 401 Unauthorized", "{\"error\":{\"message\":\"bad key\"}}");
        let mut cfg = LlmConfig::new("m", "k");
        cfg.base_url = url;
        let p = OpenAiProvider::new(cfg);
        let err = match p.chat_stream(&[Message::user("hi")], &[]).await {
            Err(e) => e,
            Ok(_) => panic!("应返回错误而非静默空流"),
        };
        assert!(err.to_string().contains("401"), "错误信息应包含状态码: {}", err);
        assert!(err.to_string().contains("bad key"));
    }

    #[tokio::test]
    async fn test_chat_http_error_message() {
        let url = spawn_mock("HTTP/1.1 429 Too Many Requests", "rate limited");
        let mut cfg = LlmConfig::new("m", "k");
        cfg.base_url = url;
        let p = OpenAiProvider::new(cfg);
        let err = match p.chat(&[Message::user("hi")], &[]).await {
            Err(e) => e,
            Ok(_) => panic!("应返回可读错误"),
        };
        assert!(err.to_string().contains("429"));
        assert!(err.to_string().contains("rate limited"));
    }

    // ─── 思考强度参数映射（apply_reasoning）───
    fn body_with(model: &str, base_url: &str, effort: &str) -> serde_json::Value {
        let mut cfg = LlmConfig::new(model, "k");
        cfg.base_url = base_url.into();
        let p = OpenAiProvider::new(cfg).with_effort(Some(effort.into()));
        p.body(&[Message::user("hi")], &[], false)
    }

    #[test]
    fn reasoning_deepseek_sends_effort_and_thinking_switch() {
        let b = body_with("deepseek-v4-pro", "https://api.deepseek.com/v1", "max");
        assert_eq!(b["reasoning_effort"], serde_json::json!("max"));
        assert_eq!(b["thinking"], serde_json::json!({"type": "enabled"}));
        // 档位折叠：medium/high → high；ultra → max
        assert_eq!(body_with("deepseek-v4-flash", "https://api.deepseek.com/v1", "medium")["reasoning_effort"], serde_json::json!("high"));
        assert_eq!(body_with("deepseek-v4-flash", "https://api.deepseek.com/v1", "ultra")["reasoning_effort"], serde_json::json!("max"));
    }

    #[test]
    fn reasoning_capable_models_get_openai_style_effort_only() {
        let b = body_with("gpt-5", "https://api.openai.com/v1", "high");
        assert_eq!(b["reasoning_effort"], serde_json::json!("high"));
        assert!(b.get("thinking").is_none(), "非 DeepSeek 不发送 thinking 开关");
        assert_eq!(body_with("o3-mini", "https://api.openai.com/v1", "low")["reasoning_effort"], serde_json::json!("low"));
    }

    #[test]
    fn reasoning_skipped_for_plain_models_and_no_effort() {
        // 非推理型模型：不硬塞参数（防 400）
        let b = body_with("llama-3.3-70b", "https://api.groq.com/openai/v1", "medium");
        assert!(b.get("reasoning_effort").is_none());
        assert!(b.get("thinking").is_none());
        // 未设置 effort：不发送任何思考参数
        let mut cfg = LlmConfig::new("deepseek-v4-pro", "k");
        cfg.base_url = "https://api.deepseek.com/v1".into();
        let b = OpenAiProvider::new(cfg).body(&[Message::user("hi")], &[], false);
        assert!(b.get("reasoning_effort").is_none());
        assert!(b.get("thinking").is_none());
    }

    #[test]
    fn reasoning_retired_deepseek_chat_sends_nothing() {
        // deepseek-chat 已于 2026-07-24 退役：官方兼容路由静默忽略思考参数 — 一个都不发
        let b = body_with("deepseek-chat", "https://api.deepseek.com/v1", "max");
        assert!(b.get("reasoning_effort").is_none());
        assert!(b.get("thinking").is_none());
        let b2 = body_with("deepseek-v3", "https://api.deepseek.com/v1", "high");
        assert!(b2.get("reasoning_effort").is_none());
        assert!(b2.get("thinking").is_none());
    }

    #[test]
    fn reasoning_off_explicitly_disables_thinking() {
        let b = body_with("deepseek-flash", "https://api.deepseek.com/v1", "off");
        assert_eq!(b["thinking"], serde_json::json!({ "type": "disabled" }));
        assert!(b.get("reasoning_effort").is_none(), "关闭档不应再发 reasoning_effort");
        // 非推理模型关闭档：什么都不发
        let b2 = body_with("llama-3.3-70b", "https://api.groq.com/openai/v1", "off");
        assert!(b2.get("thinking").is_none() && b2.get("reasoning_effort").is_none());
    }

    #[test]
    fn thinking_supported_detects_deepseek_lineup() {
        let mk = |m: &str| {
            let mut c = LlmConfig::new(m, "k");
            c.base_url = "https://api.deepseek.com/v1".into();
            OpenAiProvider::new(c)
        };
        assert!(mk("deepseek-flash").thinking_supported());
        assert!(mk("deepseek-v4-pro").thinking_supported());
        assert!(mk("deepseek-r1").thinking_supported());
        assert!(!mk("deepseek-chat").thinking_supported());
        assert!(!mk("deepseek-v3").thinking_supported());
    }

    #[test]
    fn multimodal_multi_image_content_array() {
        let m = Message::user_multimodal_multi("看这两张图", &["data:image/png;base64,AAA".into(), "data:image/png;base64,BBB".into()]);
        assert!(Message::is_multimodal_content(&m.content));
        let v: serde_json::Value = serde_json::from_str(&m.content).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 3, "text + 2 张图");
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[1]["image_url"]["url"], "data:image/png;base64,AAA");
        assert_eq!(arr[2]["image_url"]["url"], "data:image/png;base64,BBB");
        // 单图构造向后兼容
        let s = Message::user_multimodal("一张", "data:image/png;base64,CCC");
        let v: serde_json::Value = serde_json::from_str(&s.content).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
    }

    #[test]
    fn body_embeds_multimodal_content_array_raw() {
        // 链路终点验证：多模态消息在请求 JSON 里必须是原生 content 数组（image_url part 完整），
        // 而不是被转义的字符串 —— 这是 vision 真正生效的最终判定点
        let mut cfg = LlmConfig::new("deepseek-chat", "k");
        cfg.base_url = "https://api.deepseek.com/v1".into();
        let p = OpenAiProvider::new(cfg).with_effort(Some("max".into()));
        let msgs = vec![
            Message::system("sys"),
            Message::user_multimodal_multi("这是什么", &["data:image/png;base64,AAA".into()]),
        ];
        let b = p.body(&msgs, &[], false);
        let user_msg = &b["messages"][1];
        assert!(user_msg["content"].is_array(), "content 必须是数组而非字符串");
        let parts = user_msg["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,AAA");
    }
}

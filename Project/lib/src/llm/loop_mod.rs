//! Agentic Loop — LLM ↔ Tool循环执行

use super::message::Message;
use super::provider::LlmProvider;
use super::tool::Tool;
use super::tool_executor::execute_tool;
use super::attention::{AttentionDecision, AttentionGuard};

/// 执行Agentic Loop
/// LLM生成响应 → 有tool_calls则执行 → 结果反馈给LLM → 循环
pub async fn run_loop(
    provider: &dyn LlmProvider,
    messages: &mut Vec<Message>,
    tools: &[Tool],
    max_iter: usize,
) -> anyhow::Result<String> {
    let mut attention = AttentionGuard::new();
    for _ in 0..max_iter {
        let resp = provider.chat(messages, tools).await?;
        messages.push(Message::assistant(&resp.content));
        if !resp.content.trim().is_empty() {
            attention.observe_output();
        }

        if resp.tool_calls.is_empty() {
            return Ok(resp.content);
        }

        let mut feedback = false;
        for tc in &resp.tool_calls {
            let result = execute_tool(tc).await?;
            match attention.observe_tool(&tc.name, &tc.arguments, &result.output, result.success) {
                AttentionDecision::Continue => {}
                AttentionDecision::InjectFeedback(message) => {
                    messages.push(Message::tool(&message, &tc.id));
                    feedback = true;
                    break;
                }
                AttentionDecision::Stop(message) => {
                    return Err(anyhow::anyhow!(message));
                }
            }
            let summary = format!("{} (success={})", result.output.chars().take(500).collect::<String>(), result.success);
            messages.push(Message::tool(&summary, &tc.id));
        }
        if feedback {
            continue;
        }
    }
    Err(anyhow::anyhow!("达到最大迭代次数 ({})", max_iter))
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_loop_compiles() { assert!(true); }
}

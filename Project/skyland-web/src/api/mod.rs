//! API 路由处理模块

pub mod kb;

use axum::{Json, extract::State, response::sse::{Event, Sse}};
use std::collections::HashMap;
use std::sync::Arc;
use std::convert::Infallible;
use std::path::PathBuf;
use std::time::Duration;
use futures::stream::Stream;

use skyland_lib::llm::{
    Message, openai::OpenAiProvider, LlmProvider, LlmConfig, resolve_api_base,
    tool_executor, tool::tools_for_mode, stream::StreamEvent,
    AttentionDecision, AttentionGuard,
};
use serde::{Deserialize, Serialize};

/// 共享应用状态 — 持有多个 Provider
#[derive(Clone)]
pub struct AppContext {
    pub providers: HashMap<String, Arc<OpenAiProvider>>,
    pub default_model: String,
    pub user_data_dir: PathBuf,
    pub work_dir: PathBuf,
    pub context_lengths: HashMap<String, u32>,
    pub image_dir: Arc<std::sync::RwLock<PathBuf>>,
    pub todo_dir: Arc<std::sync::RwLock<PathBuf>>,
    /// server 模式下的认证状态；standalone/client 为 None
    pub auth: Option<Arc<crate::auth::AuthState>>,
    /// 当前运行模式标识（standalone/server/client）
    pub server_mode: String,
    /// 云工作根目录（server/client 模式下 AI 生成文件的服务端落点）
    pub cloud_workdir: PathBuf,
}

/// 返回当前请求用户的数据根目录。
/// standalone 使用启动时绑定的本地用户目录；server 模式只使用认证用户名。
pub fn user_data_dir_for(
    ctx: &AppContext,
    user: Option<&crate::auth::CurrentUser>,
) -> PathBuf {
    if ctx.auth.is_none() {
        return ctx.user_data_dir.clone();
    }
    let Some(user) = user else {
        return ctx.user_data_dir.clone();
    };
    crate::datadir::AppDataLayout::from_context(ctx)
        .user_dir(&user.username)
        .unwrap_or_else(|_| ctx.user_data_dir.clone())
}

/// 历史消息（从前端传入）
#[derive(Deserialize)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize, Clone)]
pub struct ChatAttachment {
    pub name: String,
    pub content: String,
}

#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub history: Vec<HistoryMessage>,
    pub model: Option<String>,
    pub execution_mode: Option<String>,
    pub thinking_effort: Option<String>,
    pub max_tool_rounds: Option<usize>,
    pub system_prompt: Option<String>,
    pub web_search_enabled: Option<bool>,
    pub image: Option<String>,
    /// 多图（data URL 数组）— 与 image 单张向后兼容，两者并存时合并
    pub images: Option<Vec<String>>,
    pub attachments: Option<Vec<ChatAttachment>>,
    pub workspace: Option<String>,
    /// 斜杠菜单手动引入的技能名 — 确定性注入完整技能指令（不依赖模型主动调用 use_skill）
    pub skill: Option<String>,
    /// 会话工作目录：本地相对子路径，或 "@cloud"（server/client 模式云工作）
    pub workdir: Option<String>,
    /// 会话 id（云工作目录 sessions/{id} 的落点；前端 streamChat.id）
    pub conversation_id: Option<u64>,
    /// 用户覆盖的上下文窗口（token 数；影响右栏上下文条与自动压缩阈值）
    pub context_length: Option<u32>,
    /// 上下文压缩专用模型（达阈值时用它做历史摘要；空 = 当前对话模型）
    pub compact_model: Option<String>,
    /// 会话执行计划（todo_write 维护，前端随请求回传）— 计划是会话级状态，
    /// 随请求续接防止"已完成项在新消息中被重新拆解回未完成"
    pub plan: Option<Vec<serde_json::Value>>,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub reply: String,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub models: Vec<String>,
    pub default_model: String,
    pub search_enabled: bool,
    pub context_length: u32,
}

/// 会话级执行计划条目 — todo_write 工具维护、右栏「执行计划」渲染（区别于用户待办系统）
#[derive(Clone, Serialize)]
struct PlanTodo {
    content: String,
    status: String, // pending | in_progress | completed
}

/// SSE 事件类型（结构化 JSON）
#[derive(Serialize)]
#[serde(tag = "type")]
enum SseEvent {
    #[serde(rename = "text")]
    Text { content: String },
    #[serde(rename = "reasoning")]
    Reasoning { content: String },
    #[serde(rename = "status")]
    Status { action: String, detail: String },
    #[serde(rename = "approval_request")]
    ApprovalRequest { token: String, action: String, target: String, detail: String },
    #[serde(rename = "tool_start")]
    ToolStart { id: String, name: String, args: String },
    #[serde(rename = "tool_end")]
    ToolEnd { id: String, name: String, success: bool, output: String },
    #[serde(rename = "context_update")]
    ContextUpdate { used: u32, total: u32 },
    /// 上下文已自动压缩 — 前端据此把会话历史中段替换为摘要（持久化，下轮请求不再膨胀）
    #[serde(rename = "context_compacted")]
    ContextCompacted { summary: String },
    /// 智能体产出文件/图片 — 会话内即时卡片 + markdown 链接持久化
    #[serde(rename = "file_output")]
    FileOutput { name: String, url: String, kind: String, path: String },
    #[serde(rename = "agent_spawn")]
    AgentSpawn {
        id: String,
        name: String,
        task: String,
        /// 该子代理是否**真的拿到了写权限**（两把钥匙齐备且主体非只读）。
        /// 前端必须显示出来：会改文件的子代理与只读分析必须一眼可分。
        writable: bool,
    },
    #[serde(rename = "agent_status")]
    AgentStatus {
        id: String,
        name: String,
        /// 机器可读状态（running / completed / failed / cancelled）。
        /// 与 `status` 分开的原因：`status` 是给人看的自由文本
        /// （"第2轮 · read_file、grep"），前端此前只能靠字符串猜状态，
        /// 一旦文案调整就会静默误判成"执行中"。逻辑一律走 `state`。
        state: String,
        /// 给人看的进度文案，可变，前端只展示不解析。
        status: String,
    },
    #[serde(rename = "agent_end")]
    AgentEnd {
        id: String,
        name: String,
        status: String,
        rounds: u32,
        elapsed_ms: u64,
        result: String,
        error_code: Option<String>,
        error_message: Option<String>,
    },
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "warning")]
    Warning { message: String },
    #[serde(rename = "round_start")]
    RoundStart { round: u32 },
    #[serde(rename = "plan_update")]
    PlanUpdate { todos: Vec<PlanTodo> },
}

impl SseEvent {
    fn to_data(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| r#"{"type":"error","message":"序列化失败"}"#.into())
    }
}

/// 构造 SSE data 事件
fn sse_data(json: &str) -> Result<Event, Infallible> {
    Ok(Event::default().data(json.to_string()))
}

fn sse_done() -> Result<Event, Infallible> {
    Ok(Event::default().data("[DONE]"))
}

/// POST /api/chat — 同步聊天（带工具循环：模型发起 tool_calls 时执行并回喂，直至给出最终回复）
pub async fn chat_handler(
    State(ctx): State<AppContext>,
    Json(req): Json<ChatRequest>,
) -> Json<ChatResponse> {
    let images = collect_images(&req);
    let model = req.model.clone().unwrap_or_else(|| ctx.default_model.clone());
    let mode = req.execution_mode.as_deref().unwrap_or("auto");
    let effort = req.thinking_effort.as_deref().unwrap_or("medium");
    let web_search = req.web_search_enabled.unwrap_or(true);

    if let Some(provider) = ctx.providers.get(&model) {
        // 请求级思考强度 — 与 /api/chat/stream 对齐（曾只注入提示词层 hint，API 参数整条丢失）
        let p = provider.as_ref().clone().with_effort(Some(effort.to_string()));
        let mut msgs = build_messages(&req.history, &req.message, effort, mode, req.system_prompt.as_deref(), &images, req.attachments.as_deref(), req.workspace.as_deref(), &ctx.work_dir);
        let mut tools = tools_for_mode(match mode {
            "readonly" | "plan" => "readonly",
            "edit" => "edit",
            "full" => "full",
            _ => "auto",
        });
        if !web_search { tools.retain(|t| t.name != "web_search"); }
        // 旧实现单轮调用且丢弃 tool_calls → 模型请求工具时回复为空；接入 agentic loop 修复
        match skyland_lib::llm::run_loop(&p, &mut msgs, &tools, 10).await {
            Ok(reply) => Json(ChatResponse { reply }),
            Err(e) => Json(ChatResponse { reply: format!("Error: {}", e) }),
        }
    } else {
        Json(ChatResponse { reply: format!("模型 {} 不可用", model) })
    }
}

/// 轻量聊天请求（用于节点流 LLM-Call 节点）
#[derive(Deserialize)]
pub struct SimpleChatRequest {
    pub message: String,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub image: Option<String>,
    pub temperature: Option<f32>,
}

/// POST /api/chat/simple — 轻量同步聊天（无工具调用，用于节点流）
pub async fn simple_chat_handler(
    State(ctx): State<AppContext>,
    Json(req): Json<SimpleChatRequest>,
) -> Json<ChatResponse> {
    let model = req.model.unwrap_or_else(|| ctx.default_model.clone());

    if let Some(provider) = ctx.providers.get(&model) {
        // 构建最小消息序列：system(可选) + user(message，多模态如果有图片)
        let mut msgs: Vec<Message> = Vec::new();
        if let Some(ref sys) = req.system_prompt {
            if !sys.is_empty() {
                msgs.push(Message::system(sys));
            }
        }
        if let Some(ref img) = req.image {
            if !img.is_empty() {
                msgs.push(Message::user_multimodal(&req.message, img));
            } else {
                msgs.push(Message::user(&req.message));
            }
        } else {
            msgs.push(Message::user(&req.message));
        }
        // 无工具调用
        match provider.chat(&msgs, &[]).await {
            Ok(r) => Json(ChatResponse { reply: r.content }),
            Err(e) => Json(ChatResponse { reply: format!("Error: {}", e) }),
        }
    } else {
        Json(ChatResponse { reply: format!("模型 {} 不可用", model) })
    }
}

/// POST /api/chat/stream — SSE流式聊天（结构化事件 + 权限模式 + 可配置轮数）
pub async fn chat_stream_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<crate::auth::CurrentUser>>,
    Json(req): Json<ChatRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(128);
    let images = collect_images(&req);
    let model = req.model.clone().unwrap_or_else(|| ctx.default_model.clone());
    let provider = ctx.providers.get(&model).cloned();
    let mode = req.execution_mode.as_deref().unwrap_or("auto").to_string();
    let effort = req.thinking_effort.as_deref().unwrap_or("medium").to_string();
    let max_rounds = req.max_tool_rounds.unwrap_or(50).max(1).min(100);
    let web_search = req.web_search_enabled.unwrap_or(true);
    let current_user = user.map(|u| u.0);

    tokio::spawn(async move {
        let mut attention = AttentionGuard::new();
        if let Some(provider) = provider {
            // 图片请求可见性：多模态请求进入后端即留痕 — 若前端发了图但这里无日志，
            // 说明在跑旧二进制（serde 静默忽略未知字段），避免"静默丢图"再次无迹可查
            if !images.is_empty() {
                eprintln!("[vision] 收到多模态请求：{} 张图片（模型 {}）", images.len(), model);
            }
            // 请求级克隆注入思考强度 — 真正把 reasoning_effort 发给 LLM API
            // （DeepSeek：reasoning_effort + thinking 开关；其他推理型模型仅 reasoning_effort）
            let mut p = (*provider).clone().with_effort(Some(effort.clone()));
            // max_tokens 截断恢复计数（上限 3 次，对齐 Claude Code MAX_OUTPUT_TOKENS_RECOVERY_LIMIT）
            let mut output_recoveries = 0u32;
            // API 真实用量（末轮 usage ≈ 下一轮上下文占用）— 上下文条优先采用
            let mut last_usage: Option<skyland_lib::llm::provider::Usage> = None;
            // 会话级执行计划（todo_write 工具维护）— 从前端随请求回传续接：
            // 旧实现是请求级局部变量，每条新消息清零 → 模型按提示词重新拆解 → 已完成项"复活"
            let mut session_plan: Vec<PlanTodo> = req.plan.clone().unwrap_or_default().into_iter()
                .filter_map(|t| {
                    let content = t.get("content")?.as_str()?.trim().to_string();
                    if content.is_empty() { return None; }
                    let status = match t.get("status").and_then(|s| s.as_str()).unwrap_or("pending") {
                        "in_progress" => "in_progress",
                        "completed" => "completed",
                        _ => "pending",
                    };
                    Some(PlanTodo { content, status: status.to_string() })
                })
                .collect();
            // 模型思考能力感知 — 思考档已选但模型不支持时前端可见，不再静默吞掉设置
            // （退役名 deepseek-chat / 非思考型：官方兼容路由会忽略思考参数）
            if effort != "off" && !p.thinking_supported() {
                let warn = SseEvent::Warning {
                    message: format!("当前模型 {} 不支持思考模式（DeepSeek 需 deepseek-flash / deepseek-v4-pro），思考强度设置未生效", model),
                }.to_data();
                let _ = tx.send(sse_data(&warn)).await;
            }
            let tool_mode = match mode.as_str() {
                "readonly" | "plan" => "readonly",
                "edit" => "edit",
                "full" => "full",
                _ => "auto",
            };
            let sys_prompt = req.system_prompt.clone();
            let work_dir = ctx.work_dir.clone();
            let mut msgs = build_messages(&req.history, &req.message, &effort, &mode, sys_prompt.as_deref(), &images, req.attachments.as_deref(), req.workspace.as_deref(), &work_dir);
            // 斜杠手动引入技能 — 完整指令直注系统提示（确定性加载，模型无需再调 use_skill）
            if let Some(skill_name) = req.skill.as_deref() {
                if let Some(body) = crate::skills::read_skill_body_for(&ctx, current_user.as_ref(), skill_name) {
                    if let Some(first) = msgs.first_mut() {
                        first.content.push_str(&format!(
                            "\n\n# 用户指定加载技能「{}」（用户已手动引入，直接遵循其指令执行）\n{}",
                            skill_name, body
                        ));
                    }
                }
            }
            let mut tools = tools_for_mode(tool_mode);
            if !web_search { tools.retain(|t| t.name != "web_search"); }
            // 联动工具（知识库/待办速记/技能/定时任务）— 按执行模式装配，readonly/plan 无写工具
            tools.extend(crate::chat_tools::tools_for_mode(tool_mode));
            // 会话级执行计划（Claude Code TodoWrite 同语义）— 全模式可用，只改本流计划状态
            tools.push(todo_write_tool());
            // 会话工作目录（本地子目录或云工作）+ 文件生成工具（auto/edit 模式）
            let session_wd = crate::filegen::resolve_session_workdir_for(
                &ctx,
                current_user.as_ref(),
                req.workdir.as_deref(),
                req.conversation_id,
            );
            if matches!(tool_mode, "auto" | "edit" | "full") && session_wd.is_some() {
                tools.extend(crate::filegen::tools_for_mode());
            }
            // 工具默认作用域（P0）：server 模式未选工作区时，执行 base 必须落到
            // **当前用户**的数据目录 —— 桌面 server 部署的全局 WORK_DIR 即数据根，
            // auth/sessions.json、他人 users/<name>/ 全在其下，绝不能作为默认沙箱。
            let tool_base: Option<std::path::PathBuf> = if ctx.auth.is_some() {
                Some(session_wd.clone()
                    .unwrap_or_else(|| crate::api::user_data_dir_for(&ctx, current_user.as_ref())))
            } else {
                session_wd.clone()
            };
            // 自定义子代理（UserData/agents/*.md，对齐 Claude Code .claude/agents）—
            // 存在时动态重建 spawn_agent 定义，把自定义类型注入枚举与描述（模型可感知可选型）
            let custom_agents = crate::agents::load_custom_agents_for(&ctx, current_user.as_ref());
            if !custom_agents.is_empty() {
                if let Some(t) = tools.iter_mut().find(|t| t.name == "spawn_agent") {
                    *t = dynamic_spawn_agent_tool(&custom_agents);
                }
            }
            // auto 模式专属：模型主动申报风险的授权请求通道（对齐 Codex「帮我审批」的自审机制）
            if tool_mode == "auto" {
                tools.push(request_approval_tool());
            }
            // 模式感知能力清单 — 只声明本模式下真实可用的工具（防止模型调用不存在的工具）
            {
                let names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
                if let Some(first) = msgs.first_mut() {
                    first.content.push_str(&format!(
                        "

# 本模式可用工具
{}
（不在清单中的工具当前不可调用；写入类操作在计划/只读模式被禁用）",
                        names.join("、")
                    ));
                }
            }
            // 技能清单注入系统提示（AI 知道有哪些技能可用 use_skill 加载）
            {
                let skills = crate::skills::list_skills_for(&ctx, current_user.as_ref());
                if !skills.is_empty() {
                    let lines: Vec<String> = skills.iter()
                        .map(|s| format!("- {}（{}）：{}", s.name, s.title, s.description))
                        .collect();
                    if let Some(first) = msgs.first_mut() {
                        first.content.push_str(&format!(
                            "\n\n# 可用技能\n用户提及以下场景时，先调用 use_skill 加载技能指令再执行：\n{}\n\n使用技能时必须遵守技能正文开头的「执行纪律（强制）」：严格按其工作流程逐步执行、产物写入当前工作目录、结束时按「做了什么 → 产物在哪 → 下一步建议」三段式汇报；不得跳步、不得虚构数据、不得只回复\"已完成\"。",
                            lines.join("\n")
                        ));
                    }
                }
            }
            // MCP 服务器清单注入（本期：提示层；工具实时桥接下一迭代）
            if let Some(mcp_summary) = crate::mcp::prompt_summary(&ctx) {
                if let Some(first) = msgs.first_mut() {
                    first.content.push_str(&format!(
                        "\n\n# 可用 MCP 服务器\n以下外部工具服务器已在系统中登记（工具实时调用将在后续版本开放）：\n{}",
                        mcp_summary
                    ));
                }
            }
            // 执行计划状态注入 — 模型可见上一轮计划进度，续作时不再从零重列
            if !session_plan.is_empty() {
                let plan_lines: Vec<String> = session_plan.iter().map(|t| {
                    let tag = match t.status.as_str() {
                        "completed" => "[已完成]",
                        "in_progress" => "[进行中]",
                        _ => "[待办]",
                    };
                    format!("- {} {}", tag, t.content)
                }).collect();
                if let Some(first) = msgs.first_mut() {
                    first.content.push_str(&format!(
                        "\n\n# 当前会话执行计划（todo_write 维护的最新状态）\n{}\n续作纪律：已完成项保持 completed 不得重置为未完成；只在确有新步骤时追加条目；用 todo_write 提交最新完整清单。",
                        plan_lines.join("\n")
                    ));
                }
            }

            // 会话规则键（"本次对话允许"的作用域，对齐 Claude session 规则）：优先会话 id，无则本流临时键
            let conv_key = req.conversation_id.map(|id| id.to_string()).unwrap_or_else(|| {
                format!("stream_{}", std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0))
            });
            // 本会话智能体已写过的文件集合 — auto 模式下对自有产物的连续编辑不再打扰用户
            // 主/子代理共享的「本会话产物」集合：子代理新建的文件再编辑时仍判 level 0，不重复弹授权
            let agent_files = crate::write_guard::AgentFiles::new();

            'rounds: for round_no in 1..=max_rounds {
                // 客户端已断开（前端停止按钮/关闭页面）→ 立即终止，不再发起新一轮 LLM 请求
                if tx.is_closed() { break; }
                // 每轮起始事件 — 前端显示「第 N 轮」并累计工具计数（对齐 Claude stream_request_start）
                let _ = tx.send(sse_data(&SseEvent::RoundStart { round: round_no as u32 }.to_data())).await;
                // 上下文压缩（对齐 Claude Code autoCompact，阈值 90%）：达阈值时把中段历史摘要化，
                // 保系统提示 + 最近几条；判定取真实 usage（上一轮）与估算的较大者。
                // 压缩用专用模型（用户设置，未设置 = 当前对话模型）。
                let ctx_total_c = req.context_length
                    .or_else(|| ctx.context_lengths.get(&model).copied())
                    .unwrap_or(128000);
                let used_c = last_usage.as_ref().map(|u| u.total_tokens).unwrap_or(0)
                    .max(msgs.iter().map(|m| estimate_content_tokens(&m.content)).sum::<u32>());
                // 每轮实时刷新右栏上下文条 — 不再等流结束才可见
                let _ = tx.send(sse_data(&SseEvent::ContextUpdate { used: used_c, total: ctx_total_c }.to_data())).await;
                if used_c as f64 > ctx_total_c as f64 * 0.9 && msgs.len() > 8 {
                    let before_count = msgs.len();
                    let compact_p = req.compact_model.as_deref()
                        .filter(|m| !m.is_empty())
                        .and_then(|m| ctx.providers.get(m))
                        .map(|a| (**a).clone())
                        .unwrap_or_else(|| p.clone());
                    if let Some(summary) = compact_history(&compact_p, &mut msgs).await {
                        let st = SseEvent::Status {
                            action: "已压缩上下文".into(),
                            detail: format!("{} 条历史 → 摘要（防上下文溢出）", before_count),
                        }.to_data();
                        let _ = tx.send(sse_data(&st)).await;
                        // 摘要发前端 — 持久化收缩会话历史，下轮请求不再重发全量
                        let cc = SseEvent::ContextCompacted { summary }.to_data();
                        let _ = tx.send(sse_data(&cc)).await;
                        let used_after = msgs.iter().map(|m| estimate_content_tokens(&m.content)).sum::<u32>();
                        let _ = tx.send(sse_data(&SseEvent::ContextUpdate { used: used_after, total: ctx_total_c }.to_data())).await;
                    }
                }
                let mut length_truncated = false;
                let mut round_any_reasoning = false;
                match p.chat_stream(&msgs, &tools).await {
                    Ok(mut stream) => {
                        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
                        let mut full_response = String::new();

                        loop {
                            tokio::select! {
                                // 客户端断开（停止按钮/关页）→ 中断本轮生成；
                                // break 后 drop stream 即取消上游 LLM HTTP 请求，不再白烧 token
                                _ = tx.closed() => { break; }
                                event = stream.recv() => {
                                    match event {
                                        Some(StreamEvent::Text(t)) => {
                                            full_response.push_str(&t);
                                            attention.observe_output();
                                            let data = SseEvent::Text { content: t }.to_data();
                                            let _ = tx.send(sse_data(&data)).await;
                                        }
                                        Some(StreamEvent::Reasoning(t)) => {
                                            round_any_reasoning = true;
                                            let data = SseEvent::Reasoning { content: t }.to_data();
                                            let _ = tx.send(sse_data(&data)).await;
                                        }
                                        Some(StreamEvent::Usage(u)) => { last_usage = Some(u); }
                                        Some(StreamEvent::FinishReason(r)) => {
                                            if r == "length" { length_truncated = true; }
                                        }
                                        Some(StreamEvent::ToolCalls(tool_calls)) => {
                                            msgs.push(Message::assistant_with_tools(&full_response, &tool_calls));

                                            // spawn_agent — 真实子代理并行执行（信号量限 3 并发）：
                                            // 同轮多个 spawn 并行调研。结果按 tool_call_id 回喂（顺序无关）。
                                            let spawn_sem = std::sync::Arc::new(tokio::sync::Semaphore::new(3));
                                            let mut spawn_handles: Vec<tokio::task::JoinHandle<(String, String)>> = Vec::new();
                                            for tc in tool_calls.iter().filter(|tc| tc.name == "spawn_agent") {
                                                spawn_handles.push(tokio::spawn(run_sub_agent(
                                                    p.clone(), tc.clone(), web_search, tool_base.clone(), tool_base.clone().unwrap_or_else(|| ctx.work_dir.clone()), tx.clone(), spawn_sem.clone(), custom_agents.clone(),
                                                    agent_files.clone(), conv_key.clone(), tool_mode.to_string(),
                                                    current_user.as_ref().map(|u| u.username.clone()),
                                                )));
                                            }

                                            // 只读工具并发执行（对齐 Claude Code isConcurrencySafe 分区：
                                            // 读类并发、写类串行）。join_all 借用式并发，无 'static 要求。
                                            let read_futs = tool_calls.iter()
                                                .filter(|tc| is_concurrency_safe_tool(&tc.name))
                                                .map(|tc| execute_read_tool(&ctx, current_user.as_ref(), tc, &tx, tool_base.as_deref()));
                                            for r in futures::future::join_all(read_futs).await {
                                                msgs.push(Message::tool(&compact_tool_output(&r.output), &r.tool_call_id));
                                            }

                                            let mut attention_feedback = false;
                                            for tc in &tool_calls {
                                                // 客户端断开 → 跳过本批剩余工具（执行中的 bash 受 30s 超时约束自然结束）
                                                if tx.is_closed() { break; }
                                                // todo_write — 会话级执行计划（本流状态，右栏实时渲染）
                                                if tc.name == "todo_write" {
                                                    let parsed: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or_default();
                                                    let mut new_plan: Vec<PlanTodo> = Vec::new();
                                                    if let Some(arr) = parsed["todos"].as_array() {
                                                        for t in arr {
                                                            let content = t["content"].as_str().unwrap_or("").trim().to_string();
                                                            if content.is_empty() { continue; }
                                                            let status = match t["status"].as_str().unwrap_or("pending") {
                                                                "in_progress" => "in_progress",
                                                                "completed" => "completed",
                                                                _ => "pending",
                                                            }.to_string();
                                                            new_plan.push(PlanTodo { content, status });
                                                        }
                                                    }
                                                    // 完成态保护（逻辑验证）——覆盖式提交防"已完成复活"：
                                                    // 与上一版内容匹配（trim 比较）且已 completed 的条目，
                                                    // 不允许被模型重新提交回 pending/in_progress
                                                    let mut protected = 0usize;
                                                    for nt in new_plan.iter_mut() {
                                                        if nt.status == "completed" { continue; }
                                                        if session_plan.iter().any(|pt| pt.status == "completed" && pt.content.trim() == nt.content.trim()) {
                                                            nt.status = "completed".into();
                                                            protected += 1;
                                                        }
                                                    }
                                                    // 全部完成 → 清单归档清空（Claude Code TodoWrite 同语义）
                                                    let all_done = !new_plan.is_empty() && new_plan.iter().all(|t| t.status == "completed");
                                                    if all_done { new_plan.clear(); }
                                                    session_plan = new_plan;
                                                    let pu = SseEvent::PlanUpdate { todos: session_plan.clone() }.to_data();
                                                    let _ = tx.send(sse_data(&pu)).await;
                                                    attention.observe_plan();
                                                    let out = if session_plan.is_empty() {
                                                        "计划已全部完成，清单已归档。".to_string()
                                                    } else {
                                                        let note = if protected > 0 {
                                                            format!("（注意：{} 项已完成条目被重置为未完成，已自动保留完成状态）", protected)
                                                        } else { String::new() };
                                                        format!("计划已更新（共 {} 项）{}。继续按清单执行：开始一项时将其标为 in_progress，完成立即标 completed，不要攒批。", session_plan.len(), note)
                                                    };
                                                    msgs.push(Message::tool(&out, &tc.id));
                                                    continue;
                                                }

                                                if tc.name == "spawn_agent" { continue; } // 已并入上方并行执行
                                                if is_concurrency_safe_tool(&tc.name) { continue; } // 已并发执行

                                                // request_approval — 模型主动申报风险的授权请求（auto 模式自审通道）
                                                if tc.name == "request_approval" {
                                                    let parsed: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or_default();
                                                    let ra_action = parsed["action"].as_str().unwrap_or("未命名操作").to_string();
                                                    let ra_reason = parsed["reason"].as_str().unwrap_or("").to_string();
                                                    let ra_risk = parsed["risk"].as_str().unwrap_or("medium").to_string();
                                                    let _ = tx.send(sse_data(&SseEvent::ToolStart {
                                                        id: tc.id.clone(), name: tc.name.clone(), args: tc.arguments.clone(),
                                                    }.to_data())).await;
                                                    let (token, rx) = crate::approval::register_detailed_for(
                                                        current_user.as_ref().map(|u| u.username.as_str()).as_deref(),
                                                        &format!("请求授权：{}", ra_action), &ra_reason,
                                                        &format!("风险等级：{} — 智能体主动申报的风险操作，等待你的授权", ra_risk));
                                                    let ar = SseEvent::ApprovalRequest {
                                                        token: token.clone(),
                                                        action: format!("请求授权：{}", ra_action),
                                                        target: ra_reason.clone(),
                                                        detail: format!("风险等级：{} — 智能体主动申报的风险操作", ra_risk),
                                                    }.to_data();
                                                    let _ = tx.send(sse_data(&ar)).await;
                                                    let out = match crate::approval::wait_decision(rx).await {
                                                        crate::approval::Decision::AllowSession => {
                                                            crate::approval::session_grant(&conv_key, &format!("request:{}", ra_action));
                                                            "用户已批准该操作（本次对话内同类请求不再询问）。请继续执行，但仍须遵守工作纪律。".to_string()
                                                        }
                                                        crate::approval::Decision::AllowOnce =>
                                                            "用户已批准该操作（仅此一次）。请继续执行，但仍须遵守工作纪律。".to_string(),
                                                        crate::approval::Decision::Deny =>
                                                            "用户拒绝了该操作。请改用更安全的方案，或向用户说明无法完成的原因。".to_string(),
                                                    };
                                                    let _ = tx.send(sse_data(&SseEvent::ToolEnd {
                                                        id: tc.id.clone(), name: tc.name.clone(), success: true,
                                                        output: out.chars().take(2000).collect(),
                                                    }.to_data())).await;
                                                    msgs.push(Message::tool(&out, &tc.id));
                                                    continue;
                                                }

                                                // Normal tool execution（写类/命令类 — 串行，防写竞态）
                                                let data = SseEvent::ToolStart {
                                                    id: tc.id.clone(),
                                                    name: tc.name.clone(),
                                                    args: tc.arguments.clone(),
                                                }.to_data();
                                                let _ = tx.send(sse_data(&data)).await;

                                                // 状态提示 — 让用户知道智能体正在做什么
                                                let (action, detail) = describe_tool_action(&tc.name, &tc.arguments);
                                                let status_data = SseEvent::Status { action, detail }.to_data();
                                                let _ = tx.send(sse_data(&status_data)).await;

                                                // ┃ 权限闸门：bash/write/edit 按模式分级管控
                                                // ┃ full 全放行 / edit 全询问 / auto 分层（安全直行→自评→授权窗口）
                                                let mut approved_bash = false;
                                                if matches!(tc.name.as_str(), "bash" | "write_file" | "edit_file") {
                                                    // 闸门的"新建/覆盖"存在性判定跟随工作区（不能看错目录）
                                                    let gate_base = tool_base.as_deref().unwrap_or(ctx.work_dir.as_path());
                                                    match gate_operation(tool_mode, tc, &agent_files, gate_base, &conv_key, current_user.as_ref().map(|u| u.username.as_str()).as_deref(), &p, &tx).await {
                                                        GateOutcome::Proceed => {}
                                                        GateOutcome::ProceedApprovedBash => { approved_bash = true; }
                                                        GateOutcome::Denied(msg) => {
                                                            let data = SseEvent::ToolEnd {
                                                                id: tc.id.clone(), name: tc.name.clone(), success: false,
                                                                output: msg.chars().take(2000).collect(),
                                                            }.to_data();
                                                            let _ = tx.send(sse_data(&data)).await;
                                                            msgs.push(Message::tool(&msg, &tc.id));
                                                            continue;
                                                        }
                                                    }
                                                }

                                                let result = if approved_bash && tc.name == "bash" {
                                                    // 人工授权 / 安全自评通过 / full 模式 — 白名单豁免执行
                                                    // （危险清单与命令替换/重定向防护仍在 execute_bash_approved 内保留）
                                                    skyland_lib::llm::tool_executor::execute_bash_approved(tc).await.unwrap_or_else(|e| {
                                                        skyland_lib::llm::tool::ToolResult {
                                                            tool_call_id: tc.id.clone(),
                                                            output: format!("Error: {}", e),
                                                            success: false,
                                                        }
                                                    })
                                                } else if crate::chat_tools::is_chat_tool(&tc.name) {
                                                    match crate::chat_tools::route_chat_tool(&ctx, current_user.as_ref(), tc, tool_mode != "readonly" && tool_mode != "plan").await {
                                                        crate::chat_tools::ChatOutcome::Done(r) => r,
                                                        crate::chat_tools::ChatOutcome::Approval { token, rx, action, target, call_id, kb: ckb, rel: crel } => {
                                                            // 会话规则命中（"本次对话允许"过的同类 KB 操作）→ 直接放行
                                                            let kb_rule = format!("kb:{}:{}", action, ckb);
                                                            if crate::approval::session_allow(&conv_key, &kb_rule) {
                                                                crate::chat_tools::perform_approved(&ctx, current_user.as_ref(), &action, &ckb, &crel, &call_id).await
                                                            } else {
                                                                // 授权窗口：发事件（含风险说明）→ 等用户决定（180s 超时 = 拒绝）
                                                                let ar = SseEvent::ApprovalRequest {
                                                                    token: token.clone(), action: action.clone(), target: target.clone(),
                                                                    detail: "智能体请求操作你的知识库文档（用户所有文档强制人审）".into(),
                                                                }.to_data();
                                                                let _ = tx.send(sse_data(&ar)).await;
                                                                match crate::approval::wait_decision(rx).await {
                                                                    crate::approval::Decision::AllowSession => {
                                                                        crate::approval::session_grant(&conv_key, &kb_rule);
                                                                        crate::chat_tools::perform_approved(&ctx, current_user.as_ref(), &action, &ckb, &crel, &call_id).await
                                                                    }
                                                                    crate::approval::Decision::AllowOnce =>
                                                                        crate::chat_tools::perform_approved(&ctx, current_user.as_ref(), &action, &ckb, &crel, &call_id).await,
                                                                    crate::approval::Decision::Deny =>
                                                                        skyland_lib::llm::tool::ToolResult {
                                                                            tool_call_id: call_id.clone(),
                                                                            output: format!("用户拒绝了此次{}（审批未通过）", action),
                                                                            success: false,
                                                                        }
                                                                }
                                                            }
                                                        }
                                                    }
                                                } else if crate::filegen::is_filegen_tool(&tc.name) {
                                                    if tool_mode == "readonly" || tool_mode == "plan" {
                                                        skyland_lib::llm::tool::ToolResult {
                                                            tool_call_id: tc.id.clone(),
                                                            output: "⛔ 当前执行模式（计划/只读）不允许生成文件".into(),
                                                            success: false,
                                                        }
                                                    } else if session_wd.is_none() {
                                                        skyland_lib::llm::tool::ToolResult {
                                                            tool_call_id: tc.id.clone(),
                                                            output: "⛔ 未设置工作目录 — 请在输入区选择工作目录（本地子目录或云工作）后重试".into(),
                                                            success: false,
                                                        }
                                                    } else {
                                                        crate::filegen::execute_filegen_tool(&ctx, current_user.as_ref(), session_wd.as_deref(), tc).await
                                                    }
                                                } else {
                                                    // 请求级工作区贯通：bash/文件工具以用户选定的工作区为执行位置
                                                    //（server 模式未选工作区 → 当前用户数据目录，见 tool_base 注释）
                                                    tool_executor::execute_tool_in(tc, tool_base.as_deref()).await.unwrap_or_else(|e| {
                                                        skyland_lib::llm::tool::ToolResult {
                                                            tool_call_id: tc.id.clone(),
                                                            output: format!("Error: {}", e),
                                                            success: false,
                                                        }
                                                    })
                                                };

                                                match attention.observe_tool(&tc.name, &tc.arguments, &result.output, result.success) {
                                                    AttentionDecision::Continue => {}
                                                    AttentionDecision::InjectFeedback(message) => {
                                                        let _ = tx.send(sse_data(&SseEvent::Warning { message: message.clone() }.to_data())).await;
                                                        msgs.push(Message::tool(&message, &tc.id));
                                                        attention_feedback = true;
                                                    }
                                                    AttentionDecision::Stop(message) => {
                                                        let _ = tx.send(sse_data(&SseEvent::Error { message }.to_data())).await;
                                                        let _ = tx.send(sse_done()).await;
                                                        return;
                                                    }
                                                }
                                                if attention_feedback {
                                                    break;
                                                }

                                                // 文件产出事件 — create_* / generate_image 成功后向会话推送文件/图片卡片
                                                if result.success && matches!(tc.name.as_str(),
                                                    "create_document" | "create_spreadsheet" | "create_presentation" | "create_pdf" | "generate_image") {
                                                    if let Some((name, url, kind, path)) = extract_file_output(&tc.name, &result.output,
                                                        req.workdir.as_deref(), req.conversation_id, session_wd.as_deref()) {
                                                        let fo = SseEvent::FileOutput { name, url, kind, path }.to_data();
                                                        let _ = tx.send(sse_data(&fo)).await;
                                                    }
                                                }

                                                let data = SseEvent::ToolEnd {
                                                    id: tc.id.clone(),
                                                    name: tc.name.clone(),
                                                    success: result.success,
                                                    output: result.output.chars().take(2000).collect(),
                                                }.to_data();
                                                let _ = tx.send(sse_data(&data)).await;

                                                msgs.push(Message::tool(&compact_tool_output(&result.output), &tc.id));
                                                // 记录智能体本会话写过的文件 — auto 模式下连续编辑自有产物不再打扰
                                                if result.success && matches!(tc.name.as_str(), "write_file" | "edit_file") {
                                                    if let Ok(a) = serde_json::from_str::<serde_json::Value>(&tc.arguments) {
                                                        if let Some(pth) = a["path"].as_str() {
                                                            agent_files.insert(pth);
                                                            // 产出文件可见化 — 右栏"输出内容"显示完整保存路径（文件名之外用户最关心的信息）
                                                            let p = std::path::Path::new(pth);
                                                            let abs = if p.is_absolute() {
                                                                p.to_path_buf()
                                                            } else {
                                                                session_wd.clone().unwrap_or_else(|| ctx.work_dir.clone()).join(p)
                                                            };
                                                            let fo = SseEvent::FileOutput {
                                                                name: abs.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| pth.to_string()),
                                                                url: String::new(), // 无下载路由 — 仅面板展示路径
                                                                kind: "file".into(),
                                                                path: abs.display().to_string(),
                                                            }.to_data();
                                                            let _ = tx.send(sse_data(&fo)).await;
                                                        }
                                                    }
                                                }
                                            }

                                            // 收集并行子代理结果（树事件已由各任务经 tx 实时发出）
                                            for h in spawn_handles {
                                                let (call_id, output) = h.await.unwrap_or_else(|e| (String::new(), format!("子智能体任务失败: {}", e)));
                                                if call_id.is_empty() { continue; }
                                                msgs.push(Message::tool(&compact_tool_output(&output), &call_id));
                                            }
                                            if attention_feedback {
                                                continue 'rounds;
                                            }
                                            if full_response.trim().is_empty() {
                                                attention.observe_round_without_progress();
                                                if attention.should_stop_for_no_progress() {
                                                    let _ = tx.send(sse_data(&SseEvent::Error {
                                                        message: "连续多轮没有产生新进展，已停止工具循环。".into(),
                                                    }.to_data())).await;
                                                    let _ = tx.send(sse_done()).await;
                                                    return;
                                                }
                                            }
                                            break;
                                        }
                                        Some(StreamEvent::Done) | None => {
                                            // max_tokens 截断恢复（对齐 Claude Code MAX_OUTPUT_TOKENS_RECOVERY）：
                                            // 尚未流出任何正文时提升上限重试本轮（最多 3 次）；
                                            // 已流出内容不能重放（前端会重复显示），转为提示截断。
                                            // 放宽：思考型模型可能把预算耗在思维链上（正文为空但 reasoning 非空），
                                            // 同样需要重试 — 旧条件 !round_any_reasoning 会放走这种"只想不说"的截断。
                                            if length_truncated && output_recoveries < 3 && full_response.is_empty() {
                                                output_recoveries += 1;
                                                let bumped = (p.max_tokens() * 2).min(16_384);
                                                p = p.clone().with_max_tokens(bumped);
                                                eprintln!("[chat] 回复被 max_tokens 截断且未流出正文 — 上限提升至 {} 重试（第 {} 次）", bumped, output_recoveries);
                                                continue 'rounds;
                                            }
                                            if length_truncated {
                                                let _ = tx.send(sse_data(&SseEvent::Text {
                                                    content: "\n\n（回复因长度上限被截断 — 可让我继续）".into(),
                                                }.to_data())).await;
                                            } else if full_response.trim().is_empty() {
                                                // 空输出兜底 — 不再静默"已完成"（假完成事故根因）：
                                                // 给用户明确的可见反馈，右栏状态与对话流不再互相矛盾
                                                let notice = if round_any_reasoning {
                                                    "（思考结束但未产生正文 — 输出可能被长度上限截断，可让我继续）"
                                                } else {
                                                    "（模型本轮未返回内容 — 请重试或换个说法）"
                                                };
                                                let _ = tx.send(sse_data(&SseEvent::Text {
                                                    content: notice.to_string(),
                                                }.to_data())).await;
                                            }
                                            // 上下文计量：优先 API 真实 usage（末轮 total ≈ 下一轮上下文占用），无则估算
                                            let ctx_total = req.context_length
                                                .or_else(|| ctx.context_lengths.get(&model).copied())
                                                .unwrap_or(128000);
                                            let used = match last_usage.as_ref() {
                                                Some(u) if u.total_tokens > 0 => u.total_tokens,
                                                _ => msgs.iter()
                                                    .map(|m| estimate_content_tokens(&m.content))
                                                    .sum::<u32>() + (full_response.len() as u32) / 4,
                                            };
                                            let ctx_data = SseEvent::ContextUpdate {
                                                used,
                                                total: ctx_total,
                                            }.to_data();
                                            let _ = tx.send(sse_data(&ctx_data)).await;
                                            let _ = tx.send(sse_done()).await;
                                            return;
                                        }
                                        Some(StreamEvent::Error(e)) => {
                                            let data = SseEvent::Error { message: e }.to_data();
                                            let _ = tx.send(sse_data(&data)).await;
                                            let _ = tx.send(sse_done()).await;
                                            return;
                                        }
                                    }
                                }
                                _ = heartbeat.tick() => {
                                    let _ = tx.send(Ok(Event::default().comment("heartbeat"))).await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let data = SseEvent::Error { message: e.to_string() }.to_data();
                        let _ = tx.send(sse_data(&data)).await;
                        let _ = tx.send(sse_done()).await;
                        return;
                    }
                }
            }
            let data = SseEvent::Error {
                message: format!("工具调用轮数超限（{}）", max_rounds),
            }.to_data();
            let _ = tx.send(sse_data(&data)).await;
            let _ = tx.send(sse_done()).await;
        } else {
            let data = SseEvent::Error { message: "模型不可用".into() }.to_data();
            let _ = tx.send(sse_data(&data)).await;
            let _ = tx.send(sse_done()).await;
        }
    });

    Sse::new(async_stream::stream! {
        while let Some(event) = rx.recv().await {
            yield event;
        }
    })
    .keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)))
}

// ─── 子智能体注册表（对齐 Claude Code AgentDefinition：类型 / 适用场景 / 工具白名单 / 轮次上限 / 专用提示）───
struct SubAgentDef {
    agent_type: &'static str,
    #[allow(dead_code)]
    when_to_use: &'static str,
    tools: &'static [&'static str],
    max_rounds: usize,
    system_head: &'static str,
}

    /// 子代理可用的**写**工具集合（对齐 Claude Code：只读代理把 Write/Edit/NotebookEdit
    /// 放进 disallowedTools；可写代理则 tools: ['*']）。
    /// 这里集中登记，`mode=readwrite` 的判定与实际下发工具都以此为唯一依据，
    /// 避免"提示词说有写权限、工具列表里却没有"的错位。
    const WRITABLE_SUB_AGENT_TOOLS: &[&str] = &["write_file", "edit_file", "bash"];

const SUB_AGENT_DEFS: &[SubAgentDef] = &[
    SubAgentDef {
        agent_type: "explore",
        when_to_use: "快速代码/文件检索定位，只要 file:line 结论",
        tools: &["read_file", "glob", "grep"],
        max_rounds: 5,
        system_head: "你是主智能体派生的 explore 子智能体：快速定位与检索，直接给出带 file:line 的结论，不展开长篇分析。",
    },
    SubAgentDef {
        agent_type: "researcher",
        when_to_use: "需要联网查证的调研",
        tools: &["read_file", "glob", "grep", "web_search"],
        max_rounds: 8,
        system_head: "你是主智能体派生的 researcher 子智能体：联网与本地资料查证，给出有来源支撑的结论。",
    },
    SubAgentDef {
        agent_type: "general-purpose",
        when_to_use: "通用只读调研（默认）",
        tools: &["read_file", "glob", "grep", "web_search"],
        max_rounds: 10,
        system_head: "你是被主智能体派生的子智能体，使用与主智能体相同的模型，专注完成指定子任务并输出最终结果。",
    },
        SubAgentDef {
            agent_type: "coder",
            when_to_use: "写代码/改文件。必须配合 mode=readwrite 才有写权限",
            tools: &["read_file", "glob", "grep", "write_file", "edit_file"],
            max_rounds: 20,
            system_head: "你是被主智能体派生的编码子智能体，独立完成一个结构清晰的代码子任务（新建或修改文件）。\n\
             纪律：① 严格限定在 task 指定的文件范围内，不要顺手改无关文件；② 动手前先读现有代码，对齐其风格与命名；\n\
             ③ 写完在结果里列出改了哪些文件、每个文件做了什么，主智能体要据此向你提问；\n\
             ④ 宁可少改，也不要留下半成品或重复定义。",
        },
];

/// 会话级执行计划工具 — Claude Code TodoWrite 同语义（区别于用户待办系统）
fn todo_write_tool() -> skyland_lib::llm::tool::Tool {
    skyland_lib::llm::tool::Tool::new("todo_write", "更新当前任务的执行计划清单（会话级，区别于用户待办系统）。多步任务（≥3 步）开始前先用它拆解计划；执行中每完成一步立即把对应项标为 completed（不要攒批）；同一时间只保持一项 in_progress。覆盖式提交：每次传完整清单。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "完整计划清单（覆盖式提交）",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string", "description": "步骤描述（简洁祈使句）" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"], "description": "pending=待办 / in_progress=进行中 / completed=已完成" }
                        },
                        "required": ["content", "status"]
                    }
                }
            },
            "required": ["todos"]
        }))
}

    /// 子代理写权限判定（纯函数，便于单测）。
    /// 需要**两把钥匙同时给足**：子代理类型声明了写工具 ∧ 调用方传 mode=readwrite；
    /// 再叠加一条硬边界：主智能体自身只读/规划时，子代理一律不得写。
    /// 返回 (是否可写, 不可写时的具体原因)。
    fn resolve_sub_agent_write(
        agent_type: &str,
        mode: &str,
        tool_mode: &str,
        tools_list: &[String],
    ) -> (bool, Option<String>) {
        if mode != "readwrite" {
            return (false, None); // 只读是默认值，不算拒绝
        }
        if tool_mode == "readonly" || tool_mode == "plan" {
            return (false, Some("主智能体当前为只读/规划模式，子代理写权限已禁用".into()));
        }
        let declares_write = tools_list.iter().any(|n| WRITABLE_SUB_AGENT_TOOLS.contains(&n.as_str()));
        if !declares_write {
            return (false, Some(format!(
                "子代理类型 {} 未声明写工具，mode=readwrite 不生效（需要 coder 类型）",
                agent_type
            )));
        }
        (true, None)
    }

/// 真实子智能体执行：独立上下文 + 按类型限定的只读工具集，多轮工具循环直至完成。
/// 返回 (tool_call_id, 最终结果)；智能体树 SSE 事件（agent_spawn/status/end）经 tx 实时发出。
/// 并发由信号量限制（3）— 同轮派生更多子代理时排队执行。
async fn run_sub_agent(
    p: skyland_lib::llm::openai::OpenAiProvider,
    tc: skyland_lib::llm::provider::ToolCall,
    web_search: bool,
    wd: Option<std::path::PathBuf>,
    gate_base: std::path::PathBuf,
    tx: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    sem: std::sync::Arc<tokio::sync::Semaphore>,
    custom: Vec<crate::agents::LoadedAgent>,
    agent_files: crate::write_guard::AgentFiles,
    conv_key: String,
    tool_mode: String,
    owner: Option<String>,
) -> (String, String) {
    let _permit = sem.clone().acquire_owned().await.ok();
    let started_at = std::time::Instant::now();
    let agent_args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or_default();
    let agent_name = agent_args["name"].as_str().unwrap_or("agent").to_string();
    let agent_task = agent_args["task"].as_str().unwrap_or("").to_string();
    let agent_type = agent_args["subagent_type"].as_str().unwrap_or("general-purpose").to_string();
    // 类型解析：先查自定义代理（UserData/agents），再查内置注册表，兜底 general-purpose
    let (tools_list, max_rounds, system_head, type_label) = if let Some(a) = custom.iter().find(|a| a.agent_type == agent_type) {
        (a.tools.clone(), a.max_rounds, a.system_head.clone(), a.agent_type.clone())
    } else {
        let def = SUB_AGENT_DEFS.iter().find(|d| d.agent_type == agent_type)
            .unwrap_or(&SUB_AGENT_DEFS[SUB_AGENT_DEFS.len() - 1]);
        (def.tools.iter().map(|s| s.to_string()).collect(), def.max_rounds, def.system_head.to_string(), def.agent_type.to_string())
    };
    let (write_enabled, write_denied) = resolve_sub_agent_write(&agent_type, agent_args["mode"].as_str().unwrap_or("readonly"), &tool_mode, &tools_list);
    if agent_task.trim().is_empty() {
        let message = "spawn_agent 参数错误：task 不能为空（子智能体看不到主对话，任务必须自包含）";
        let _ = tx.send(sse_data(&SseEvent::AgentSpawn {
            id: tc.id.clone(),
            name: agent_name.clone(),
            task: String::new(),
            writable: false,
        }.to_data())).await;
        let _ = tx.send(sse_data(&SseEvent::AgentEnd {
            id: tc.id.clone(),
            name: agent_name,
            status: "failed".into(),
            rounds: 0,
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            result: message.into(),
            error_code: Some("empty_task".into()),
            error_message: Some(message.into()),
        }.to_data())).await;
        return (tc.id, message.into());
    }

    let _ = tx.send(sse_data(&SseEvent::AgentSpawn {
        id: tc.id.clone(),
        name: agent_name.clone(),
        task: agent_task.clone(),
        writable: write_enabled,
    }.to_data())).await;
    let _ = tx.send(sse_data(&SseEvent::AgentStatus {
        id: tc.id.clone(),
        name: agent_name.clone(),
        state: "running".into(),
        status: format!("{} · 执行中", type_label),
    }.to_data())).await;

    // 写能力**同时**受两把钥匙约束：子代理类型白名单（tools_list）与 mode=readwrite。
    // 少任一把都拿不到写工具 —— 模型无法只靠把 subagent_type 改成 coder 就自我提权。
    // 一律排除 spawn_agent —— 禁止递归派生，防失控
    let sub_tools: Vec<_> = tools_for_mode(if write_enabled { "edit" } else { "readonly" })
        .into_iter()
        .filter(|t| t.name != "spawn_agent"
            && (tools_list.is_empty() || tools_list.contains(&t.name))
            && (web_search || t.name != "web_search"))
        .filter(|t| write_enabled || !WRITABLE_SUB_AGENT_TOOLS.contains(&t.name.as_str()))
        .collect();
        let granted = sub_tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(" / ");
        let tool_rule = if write_enabled {
            format!(
                "- 可用工具：{}（可写）。写文件前先读现有代码并对齐其风格；\
                 覆盖既有文件可能需要用户授权，被拒绝就换方案，不要反复重试。\n\
                 - 只改 task 明确要求的范围，不要顺手改无关文件。\n\
                 - 完成时在结果里列出：改了哪些文件、每个文件做了什么。",
                granted
            )
        } else {
            format!(
                "- 可用工具：{}（只读）。需要事实依据时先查证再回答；\
                 你不负责改文件，改动由主智能体执行。\n\
                 - 你是一个**快**代理：优先并行发起多个只读工具调用，尽快把结论交回主线程，不要长篇分析。",
                granted
            )
        };
        let sub_sys = format!(
            "{}\n角色名：{}\n任务：{}\n\n约束：\n\
             - 专注完成本任务，直接给出最终结果，不要派生其他子智能体\n{}\n\
             - 结果用简洁的中文 Markdown 输出，只包含任务相关的发现与结论{}",
            system_head, agent_name, agent_task, tool_rule,
            write_denied.map(|r| format!("\n- 注意：本次未获得写权限（{}）。请只做分析并把结论交回主智能体。", r)).unwrap_or_default()
        );
    let mut sub_msgs = vec![Message::system(&sub_sys), Message::user(&agent_task)];
    let mut collected = String::new();
    let mut status = "failed";
    let mut error_code = Some("no_result".to_string());
    let mut error_message = Some("子智能体没有产出内容".to_string());
    let mut rounds_used = 0u32;
    for round in 0..max_rounds {
        // 主连接已断开（用户停止/关页）→ 子智能体随行终止，不再消耗 API 轮次
        if tx.is_closed() {
            status = "cancelled";
            error_code = Some("cancelled".into());
            error_message = Some("主连接已关闭".into());
            break;
        }
        let resp = p.chat(&sub_msgs, &sub_tools).await;
        match resp {
            Ok(r) => {
                rounds_used = round as u32 + 1;
                if !r.content.is_empty() { collected.push_str(&r.content); }
                if r.tool_calls.is_empty() {
                    if r.content.trim().is_empty() {
                        error_code = Some("empty_result".into());
                        error_message = Some("模型返回了空结果".into());
                    } else {
                        status = "completed";
                        error_code = None;
                        error_message = None;
                    }
                    break;
                }
                // 报告本轮动作进度（前端智能体树实时显示）
                let prog = SseEvent::AgentStatus {
                    id: tc.id.clone(),
                    name: agent_name.clone(),
                    state: "running".into(),
                    status: format!("第{}轮 · {}", round + 1,
                        r.tool_calls.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join("、")),
                }.to_data();
                let _ = tx.send(sse_data(&prog)).await;
                sub_msgs.push(Message::assistant_with_tools(&r.content, &r.tool_calls));
                for stc in &r.tool_calls {
                    if tx.is_closed() { break; }
                    let is_write = WRITABLE_SUB_AGENT_TOOLS.contains(&stc.name.as_str());
                    // 写工具：先过同文件写锁（防并发写同一文件丢失更新），
                    // 再走**与主智能体同一个**闸门 —— 授权请求经同一条 tx 冒泡到用户，
                    // 不给子代理另开一套 UI（对齐 Claude Code permissionMode: 'bubble'）。
                    // 授权窗口全局串行：多个子代理同时需要授权时排队，避免前端连弹多个模态框。
                    if is_write {
                        let wargs: serde_json::Value = serde_json::from_str(&stc.arguments).unwrap_or_default();
                        let wpath = wargs["path"].as_str().unwrap_or("").to_string();
                        let _wlock = if wpath.is_empty() {
                            None
                        } else {
                            Some(crate::write_guard::file_lock(&wpath).await)
                        };
                        let approved = if wpath.is_empty() {
                            GateOutcome::Denied("缺少 path 参数".into())
                        } else {
                            let base = gate_base.clone();
                            let _appr = crate::write_guard::approval_gate().lock().await;
                            gate_operation(&tool_mode, stc, &agent_files, &base, &conv_key, owner.as_deref(), &p, &tx).await
                        };
                        if let GateOutcome::Denied(msg) = approved {
                            sub_msgs.push(Message::tool(
                                &format!("写入被拒绝：{}。请改用其他方案，或把该文件作为只读分析后交回主智能体。", msg),
                                &stc.id,
                            ));
                            continue;
                        }
                    }
                    let sub_result = tool_executor::execute_tool_in(stc, wd.as_deref()).await.unwrap_or_else(|e| {
                        skyland_lib::llm::tool::ToolResult {
                            tool_call_id: stc.id.clone(),
                            output: format!("Error: {}", e),
                            success: false,
                        }
                    });
                    // 子代理新建的文件登记为「本会话产物」：主智能体与其它子代理随后编辑它
                    // 都是 level 0，不会因子代理先写过一次就反复弹授权。
                    if sub_result.success && is_write {
                        if let Ok(a) = serde_json::from_str::<serde_json::Value>(&stc.arguments) {
                            if let Some(pth) = a["path"].as_str() {
                                agent_files.insert(pth);
                            }
                        }
                    }
                    sub_msgs.push(Message::tool(&sub_result.output, &stc.id));
                }
            }
            Err(e) => {
                error_code = Some("provider_error".into());
                error_message = Some(e.to_string());
                if collected.is_empty() { collected = format!("子智能体执行失败: {}", e); }
                break;
            }
        }
    }
    if rounds_used == 0 && status != "cancelled" {
        error_code = Some("round_limit".into());
        error_message = Some("子智能体未获得执行轮次".into());
    } else if rounds_used as usize >= max_rounds && status == "failed" && error_code.as_deref() == Some("no_result") {
        error_code = Some("round_limit".into());
        error_message = Some(format!("达到子智能体轮数上限 ({})", max_rounds));
    }
    if collected.trim().is_empty() {
        collected = error_message.clone().unwrap_or_else(|| "（子智能体没有产出内容）".into());
    }

    let _ = tx.send(sse_data(&SseEvent::AgentEnd {
        id: tc.id.clone(),
        name: agent_name.clone(),
        status: status.into(),
        rounds: rounds_used,
        elapsed_ms: started_at.elapsed().as_millis() as u64,
        result: collected.chars().take(500).collect(),
        error_code,
        error_message,
    }.to_data())).await;

    // 完整结果回喂父智能体（SSE 展示截断 500，模型侧不截断）
    (tc.id, collected)
}

// ─── 权限闸门（模式边界 × 安全分层 × 会话规则；对齐 Claude 权限对话框机制）───

/// 危险命令特征 — 命中即必须人工授权（LLM 自评也不能豁免；对齐 Claude strippedDangerousRules 思路）
const DANGEROUS_CMD_PATTERNS: &[&str] = &[
    "rm -rf", "rm -fr", "rmdir /s", "rd /s", "del /s", "del /q", "erase /s",
    "format ", "shutdown", "reg delete", "reg add", "mkfs", "diskpart",
    "remove-item -recurse", "remove-item /s", "dd if=", "git push --force",
    "git reset --hard", "git clean -fd", "taskkill /f", "chkdsk /f",
];

/// 路径归一化为会话规则键成分（小写 + 正斜杠）
fn normalize_path_key(p: &str) -> String {
    p.trim().to_lowercase().replace('\\', "/")
}

/// 闸门分级结果
/// - level 0：确定安全 — 直接执行（新建文件 / 本会话智能体自有产物）
/// - level 1：不确定 — auto 模式先经 LLM 安全自评，自评安全则放行
/// - level 2：必须人工授权（危险命令 / 覆盖既有用户文件 / 编辑模式全量）
struct GateClass {
    rule_key: String,
    op_desc: String,
    action_label: String,
    detail: String,
    level: u8,
}

/// 确定性分级（纯函数，可测）— 不含网络与等待
fn classify_gate(
    tool_mode: &str,
    name: &str,
    args: &serde_json::Value,
    agent_files: &crate::write_guard::AgentFiles,
    work_dir: &std::path::Path,
) -> Result<GateClass, String> {
    match name {
        "bash" => {
            let cmd = args["command"].as_str().unwrap_or("").trim().to_string();
            if cmd.is_empty() { return Err("bash 命令为空".into()); }
            let first = cmd.split_whitespace().next().unwrap_or("").to_lowercase();
            let key = format!("bash:{}", first);
            let op = format!("执行命令：{}", cmd);
            let lower = cmd.to_lowercase();
            if DANGEROUS_CMD_PATTERNS.iter().any(|pat| lower.contains(pat)) {
                Ok(GateClass { rule_key: key, op_desc: op, action_label: "执行命令".into(), detail: "命中危险命令特征（不可逆 / 系统级），必须人工授权".into(), level: 2 })
            } else if tool_mode == "edit" {
                Ok(GateClass { rule_key: key, op_desc: op, action_label: "执行命令".into(), detail: "编辑模式下所有命令均需授权".into(), level: 2 })
            } else {
                Ok(GateClass { rule_key: key, op_desc: op, action_label: "执行命令".into(), detail: String::new(), level: 1 })
            }
        }
        "write_file" | "edit_file" => {
            let path = args["path"].as_str().unwrap_or("").trim().to_string();
            if path.is_empty() { return Err("文件路径为空".into()); }
            let key = format!("file:{}", normalize_path_key(&path));
            let op_word = if name == "write_file" { "写入文件" } else { "编辑文件" };
            let op = format!("{}：{}", op_word, path);
            if tool_mode == "edit" {
                Ok(GateClass { rule_key: key, op_desc: op, action_label: "修改文件".into(), detail: "编辑模式下所有文件修改均需授权".into(), level: 2 })
            } else {
                let abs = if std::path::Path::new(&path).is_absolute() {
                    std::path::PathBuf::from(&path)
                } else {
                    work_dir.join(&path)
                };
                if !abs.exists() {
                    Ok(GateClass { rule_key: key, op_desc: format!("{}（新建文件）", op), action_label: "修改文件".into(), detail: String::new(), level: 0 })
                } else if agent_files.contains(&path) {
                    Ok(GateClass { rule_key: key, op_desc: format!("{}（本会话智能体产物）", op), action_label: "修改文件".into(), detail: String::new(), level: 0 })
                } else {
                    Ok(GateClass { rule_key: key, op_desc: format!("{}（覆盖既有文件）", op), action_label: "修改文件".into(), detail: "将覆盖已存在的用户文件 — 可能损坏用户数据".into(), level: 2 })
                }
            }
        }
        _ => Err("非闸门管辖工具".into()),
    }
}

enum GateOutcome {
    /// 常规执行（tool_executor 正常路径，含白名单）
    Proceed,
    /// 已授权执行 — bash 走白名单豁免路径（人工批准 / 安全自评通过 / full 模式）
    ProceedApprovedBash,
    /// 拒绝（含用户拒绝）— 调用方以失败工具结果回喂
    Denied(String),
}

/// 隐藏安全自评（auto 模式的「帮我审批」，对齐 Claude auto 分类器 / Codex 自审）：
/// 让模型判断操作风险，安全则自动放行。解析失败视为不安全（保守）。
async fn safety_check(p: &skyland_lib::llm::openai::OpenAiProvider, op_desc: &str) -> (bool, String) {
    let sys = "你是操作安全审查员，评估在用户工作区执行给定操作的风险。\n\
               判定标准 — 安全：可逆、只影响本次会话产物、不触碰用户既有数据与系统状态、不对外发送数据；\n\
               不安全：不可逆删除/覆盖、修改用户既有文件、系统级/全局命令、对外传输数据、大范围变更。\n\
               只输出一行 JSON：{\"safe\":true,\"reason\":\"一句话理由\"}";
    match p.chat(&[Message::system(sys), Message::user(&format!("操作：{}", op_desc))], &[]).await {
        Ok(r) => parse_safety_json(&r.content).unwrap_or((false, "自评输出无法解析，按不安全处理".into())),
        Err(e) => (false, format!("安全自评调用失败：{}", e)),
    }
}

fn parse_safety_json(txt: &str) -> Option<(bool, String)> {
    let start = txt.find('{')?;
    let end = txt.rfind('}')?;
    let v: serde_json::Value = serde_json::from_str(&txt[start..=end]).ok()?;
    let safe = v["safe"].as_bool()?;
    let reason = v["reason"].as_str().unwrap_or("").trim().to_string();
    Some((safe, if reason.is_empty() { "（未给出理由）".into() } else { reason }))
}

/// 闸门执行：分级 → 会话规则命中 → （auto 不确定带）LLM 安全自评 → 授权窗口等待三选一决策。
/// full 模式全放行（bash 走豁免路径）；拒绝消息回喂模型促其换方案。
#[allow(clippy::too_many_arguments)]
async fn gate_operation(
    tool_mode: &str,
    tc: &skyland_lib::llm::provider::ToolCall,
    agent_files: &crate::write_guard::AgentFiles,
    work_dir: &std::path::Path,
    conv_key: &str,
    owner: Option<&str>,
    p: &skyland_lib::llm::openai::OpenAiProvider,
    tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
) -> GateOutcome {
    use crate::approval::{self, Decision};

    let args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or_default();
    let class = match classify_gate(tool_mode, &tc.name, &args, agent_files, work_dir) {
        Ok(c) => c,
        Err(e) => return GateOutcome::Denied(e),
    };

    // full 模式：全放行（危险命令仍经豁免路径执行 — 保留 execute_bash_approved 内的
    // 危险清单/命令替换/重定向硬防护，与 Claude bypassPermissions 剥离危险规则同理但更保守）
    if tool_mode == "full" {
        return if tc.name == "bash" { GateOutcome::ProceedApprovedBash } else { GateOutcome::Proceed };
    }

    // 会话规则命中（"本次对话允许"过的同类操作）→ 放行
    if class.level > 0 && approval::session_allow(conv_key, &class.rule_key) {
        let _ = tx.send(sse_data(&SseEvent::Status {
            action: "会话规则放行".into(),
            detail: format!("{} — 本次对话中你已允许", class.rule_key),
        }.to_data())).await;
        return if tc.name == "bash" { GateOutcome::ProceedApprovedBash } else { GateOutcome::Proceed };
    }

    // level 0：确定安全 — 直接执行（bash 走常规白名单路径）
    if class.level == 0 {
        return GateOutcome::Proceed;
    }

    // level 1（auto 模式不确定带）：LLM 安全自评 — 安全则自动放行（"帮我审批"）
    let mut detail = class.detail.clone();
    if class.level == 1 && tool_mode == "auto" {
        let (safe, reason) = safety_check(p, &class.op_desc).await;
        if safe {
            let _ = tx.send(sse_data(&SseEvent::Status {
                action: "安全自评通过".into(),
                detail: reason.chars().take(120).collect::<String>(),
            }.to_data())).await;
            return GateOutcome::ProceedApprovedBash;
        }
        detail = format!("安全自评：{} — 需要你确认", reason);
    }

    // 授权窗口（Claude 式三选项：允许一次 / 本次对话允许 / 拒绝）— 前端同时触发 Windows 通知
    let (token, rx) = approval::register_detailed_for(owner, &class.action_label, &class.op_desc, &detail);
    let ar = SseEvent::ApprovalRequest {
        token: token.clone(),
        action: class.action_label.clone(),
        target: class.op_desc.clone(),
        detail,
    }.to_data();
    let _ = tx.send(sse_data(&ar)).await;
    match approval::wait_decision(rx).await {
        Decision::AllowSession => {
            approval::session_grant(conv_key, &class.rule_key);
            if tc.name == "bash" { GateOutcome::ProceedApprovedBash } else { GateOutcome::Proceed }
        }
        Decision::AllowOnce => {
            if tc.name == "bash" { GateOutcome::ProceedApprovedBash } else { GateOutcome::Proceed }
        }
        Decision::Deny => GateOutcome::Denied(format!(
            "用户拒绝执行「{}」— 请改用更安全的方案，或说明需要用户手动完成",
            class.op_desc
        )),
    }
}

/// auto 模式的模型主动授权请求工具（对齐 Codex「帮我审批」的自审通道）
fn request_approval_tool() -> skyland_lib::llm::tool::Tool {
    skyland_lib::llm::tool::Tool::new("request_approval",
        "就你认为有风险的操作主动请求用户人工授权（弹授权窗口 + 系统通知）。用于规则允许但你判断存在风险的操作：不可逆、影响用户重要数据、大范围变更、对外发送数据。用户批准后再执行；拒绝则换方案。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "description": "操作描述（做什么）" },
                "reason": { "type": "string", "description": "为什么需要人工判断（风险点）" },
                "risk": { "type": "string", "enum": ["low", "medium", "high"], "description": "风险等级" }
            },
            "required": ["action", "reason"]
        }))
}

// ─── 工具执行辅助（并行只读 / 结果预算 / 上下文压缩 / 动态代理工具）───

/// 并发安全的只读工具（对齐 Claude Code isConcurrencySafe）：读取类并发、写入类串行
fn is_concurrency_safe_tool(name: &str) -> bool {
    matches!(name, "read_file" | "glob" | "grep" | "web_search" | "kb_list" | "kb_search" | "kb_read" | "todo_read")
}

/// 只读工具的单次并发执行（join_all 借用式并发调用；只读工具不触发审批分支）
async fn execute_read_tool(
    ctx: &AppContext,
    current_user: Option<&crate::auth::CurrentUser>,
    tc: &skyland_lib::llm::provider::ToolCall,
    tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    wd: Option<&std::path::Path>,
) -> skyland_lib::llm::tool::ToolResult {
    let _ = tx.send(sse_data(&SseEvent::ToolStart {
        id: tc.id.clone(), name: tc.name.clone(), args: tc.arguments.clone(),
    }.to_data())).await;
    let (action, detail) = describe_tool_action(&tc.name, &tc.arguments);
    let _ = tx.send(sse_data(&SseEvent::Status { action, detail }.to_data())).await;
    let result = if crate::chat_tools::is_chat_tool(&tc.name) {
        match crate::chat_tools::route_chat_tool(ctx, current_user, tc, false).await {
            crate::chat_tools::ChatOutcome::Done(r) => r,
            // 只读工具不会触发审批（kb 写工具不在并发安全集合内）— 兜底防御
            crate::chat_tools::ChatOutcome::Approval { call_id, .. } => skyland_lib::llm::tool::ToolResult {
                tool_call_id: call_id,
                output: "内部错误：只读工具不应触发审批".into(),
                success: false,
            },
        }
    } else {
        tool_executor::execute_tool_in(tc, wd).await.unwrap_or_else(|e| {
            skyland_lib::llm::tool::ToolResult {
                tool_call_id: tc.id.clone(),
                output: format!("Error: {}", e),
                success: false,
            }
        })
    };
    let _ = tx.send(sse_data(&SseEvent::ToolEnd {
        id: tc.id.clone(), name: tc.name.clone(), success: result.success,
        output: result.output.chars().take(2000).collect(),
    }.to_data())).await;
    result
}

/// 工具结果预算（对齐 Claude Code applyToolResultBudget）：超长输出截断保头尾，
/// 防单个大结果（全仓 grep / 超长文件）撑爆上下文。SSE 展示另有 2000 字截断，这里管模型侧。
fn compact_tool_output(output: &str) -> String {
    const LIMIT: usize = 20_000;
    let count = output.chars().count();
    if count <= LIMIT { return output.to_string(); }
    let head: String = output.chars().take(14_000).collect();
    let tail: String = output.chars().skip(count - 4_000).collect();
    format!("{}\n\n…（中间内容已截断，原文 {} 字符）\n\n{}", head, count, tail)
}

/// 中段历史摘要压缩（对齐 Claude Code autoCompact）：保系统提示 + 最近 KEEP_TAIL 条，
/// 中段折叠为一条摘要 user 消息。切点避让 tool 结果块（tool 消息落在保留区开头、
/// 其 assistant(tool_calls) 被摘要掉会造成孤儿工具调用，OpenAI 兼容 API 会直接报错）。
/// 返回摘要正文（Some）— 经 context_compacted 事件发前端做会话历史持久化收缩。
async fn compact_history(p: &skyland_lib::llm::openai::OpenAiProvider, msgs: &mut Vec<Message>) -> Option<String> {
    const KEEP_TAIL: usize = 4;
    if msgs.len() <= KEEP_TAIL + 2 { return None; }
    let mut split = msgs.len() - KEEP_TAIL;
    // 保留区开头不能是 Tool 消息 — 整体后移切点，让 tool 块与其发起者一起进摘要区
    while split < msgs.len() && msgs[split].role == skyland_lib::llm::message::Role::Tool { split += 1; }
    if split <= 2 { return None; }
    let transcript = msgs[1..split].iter().map(|m| {
        let label = match m.role {
            skyland_lib::llm::message::Role::User => "用户",
            skyland_lib::llm::message::Role::Assistant => "助手",
            skyland_lib::llm::message::Role::Tool => "工具结果",
            skyland_lib::llm::message::Role::System => "系统",
        };
        let head: String = m.content.chars().take(4000).collect();
        format!("[{}]\n{}", label, head)
    }).collect::<Vec<_>>().join("\n\n");
    let prompt = format!(
        "把以下对话中间历史压缩成不超过 600 字的要点摘要。保留：关键事实与数字、已做的决定、工具调用的重要发现、尚未完成的事项。直接输出摘要正文：\n\n{}",
        transcript.chars().take(60_000).collect::<String>()
    );
    let Ok(resp) = p.chat(&[Message::user(&prompt)], &[]).await else { return None };
    if resp.content.trim().is_empty() { return None; }
    let summary = resp.content;
    let compact = Message::user(&format!("（此前对话历史已自动压缩为摘要）\n{}", summary));
    let tail: Vec<Message> = msgs[split..].to_vec();
    msgs.truncate(1);
    msgs.push(compact);
    msgs.extend(tail);
    Some(summary)
}

/// 自定义子代理存在时，动态重建 spawn_agent 工具定义（类型枚举与描述带上自定义项）
fn dynamic_spawn_agent_tool(custom: &[crate::agents::LoadedAgent]) -> skyland_lib::llm::tool::Tool {
    let mut types: Vec<String> = ["explore", "researcher", "general-purpose", "coder"].iter().map(|s| s.to_string()).collect();
    let mut desc = "派生一个子智能体去独立完成一项子任务，并拿回它的结果。子智能体拥有独立上下文，会多轮调用工具直到完成；同轮可派生多个并行执行以压缩耗时。\n\n**mode 决定它能干什么**（默认 readonly）：\n- readonly=只读（read_file/glob/grep/web_search）：读架构、查资料、定位问题。**优先用它做分析**，结论交回你，由你决定怎么改 —— 不要派只读代理去干它改不了的活。\n- readwrite=可写（coder 类型专属，增加 write_file/edit_file）：**创建结构性代码时用它**，把一份份独立的新建/修改任务分给多个 coder 并行写入，效率远高于串行。\n\n**写权限需要两把钥匙同时给足**：subagent_type=coder **且** mode=readwrite。只给一把（只写 mode 或只选 coder）都拿不到写工具。覆盖既有文件会走用户授权，被拒就换方案不要重试。\n\n用法：name 起简短角色名；task 写完整自包含的子任务描述（子智能体看不到你们的对话）；每个子代理一个明确、边界清晰的文件范围，避免多代理改同一文件互相覆盖。内置类型：explore=快速代码检索定位（只要 file:line 结论）/ researcher=联网查证调研 / general-purpose=通用只读调研（默认）/ coder=写代码改文件（需 mode=readwrite）".to_string();
    if !custom.is_empty() {
        desc.push_str("。自定义类型：");
        desc.push_str(&custom.iter().map(|a| format!("{}={}", a.agent_type, a.description)).collect::<Vec<_>>().join(" / "));
        for a in custom { types.push(a.agent_type.clone()); }
    }
    skyland_lib::llm::tool::Tool::new("spawn_agent", &desc,
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "智能体名称，如 researcher/writer/reviewer" },
                "task": { "type": "string", "description": "自包含的子任务描述（子智能体看不到主对话）" },
                "subagent_type": { "type": "string", "enum": types, "description": "子代理类型（内置 + 自定义）；coder=写代码" },
                "mode": { "type": "string", "enum": ["readonly", "readwrite"], "description": "readonly=只读分析（默认）；readwrite=允许写文件，仅当 subagent_type=coder 时生效" }
            },
            "required": ["name", "task"]
        }))
}

/// GET /api/health — 健康检查，返回可用模型列表 + 搜索状态
pub async fn health_handler(State(ctx): State<AppContext>) -> Json<HealthResponse> {
    let mut models: Vec<String> = ctx.providers.keys().cloned().collect();
    models.sort();

    // Check if search is configured server-side
    let search_configured = check_search_configured();

    // Get context length for the default model
    let ctx_len = ctx.context_lengths.get(&ctx.default_model).copied().unwrap_or(128000);

    Json(HealthResponse {
        status: if ctx.providers.is_empty() { "offline".into() } else { "ok".into() },
        models,
        default_model: ctx.default_model.clone(),
        search_enabled: search_configured,
        context_length: ctx_len,
    })
}

/// 返回 llm.toml 的搜索路径列表（优先级从高到低）：
/// Skyland 数据根优先，程序目录和外部注入路径仅兼容读取。
fn llm_config_paths() -> Vec<String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let layout = crate::datadir::AppDataLayout::resolve();
    let mut paths = vec![
        layout.config_dir().join("llm.toml").display().to_string(),
        "config/llm.toml".to_string(),
        "../config/llm.toml".to_string(),
        format!("{}/../../config/llm.toml", manifest_dir),
    ];
    // 外部注入路径（Tauri 生产模式）：SKYLAND_CONFIG_PATH 优先，MOLINE_CONFIG_PATH 兜底
    if let Some(p) = crate::env_compat::config_path() {
        paths.push(p);
    }
    paths
}

/// Provider 配置信息（用于 UI 显示，密钥脱敏）
#[derive(Serialize)]
pub struct ProviderInfo {
    pub name: String,
    pub api_key_masked: String,
    pub base_url: String,
    pub default_model: String,
    pub context_length: Option<u32>,
    pub is_default: bool,
    pub is_full_url: bool,
    /// 是否已配置 API Key — 编辑回填时前端据此决定密钥框提示
    pub has_key: bool,
}

#[derive(Serialize)]
pub struct ProvidersConfigResponse {
    pub providers: Vec<ProviderInfo>,
    pub default_provider: String,
    pub search_configured: bool,
    pub search_api_key_masked: String,
}

/// GET /api/providers/config — 获取当前配置文件中的 Provider 列表（密钥脱敏）
pub async fn providers_config_handler() -> Json<ProvidersConfigResponse> {
    let search_paths = llm_config_paths();

    let mut providers = Vec::new();
    let mut default_provider = String::new();
    let mut search_configured = false;
    let mut search_key_masked = String::new();

    for p in &search_paths {
        if let Ok(c) = std::fs::read_to_string(p) {
            if let Ok(v) = c.parse::<toml::Value>() {
                default_provider = v.get("agent")
                    .and_then(|a| a.get("default_provider"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if let Some(prs) = v.get("providers").and_then(|v| v.as_table()) {
                    for (name, cfg) in prs.iter() {
                        let key = cfg.get("api_key").and_then(|v| v.as_str()).unwrap_or("");
                        let key_resolved = if key.starts_with("${") {
                            std::env::var(&key[2..key.len()-1]).unwrap_or_default()
                        } else {
                            key.to_string()
                        };
                        let masked = if key_resolved.is_empty() {
                            "(未配置)".to_string()
                        } else if key_resolved.len() > 8 {
                            format!("{}...{}", &key_resolved[..4], &key_resolved[key_resolved.len()-4..])
                        } else {
                            "****".to_string()
                        };

                        let base_url = cfg.get("base_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let base_url_resolved = if base_url.starts_with("${") {
                            std::env::var(&base_url[2..base_url.len()-1]).unwrap_or_default()
                        } else {
                            base_url
                        };
                        let model = cfg.get("default_model").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let context_length = cfg.get("context_length").and_then(|v| v.as_integer()).map(|v| v as u32);

                        providers.push(ProviderInfo {
                            name: name.clone(),
                            api_key_masked: masked,
                            base_url: base_url_resolved,
                            default_model: model,
                            context_length,
                            is_default: name.as_str() == default_provider,
                            is_full_url: cfg.get("is_full_url").and_then(|v| v.as_bool()).unwrap_or(false),
                            has_key: !key_resolved.is_empty(),
                        });
                    }
                }

                // Search config
                if let Some(search) = v.get("search").and_then(|s| s.as_table()) {
                    if let Some(key) = search.get("api_key").and_then(|k| k.as_str()) {
                        if !key.is_empty() {
                            search_configured = true;
                            search_key_masked = if key.len() > 8 {
                                format!("{}...{}", &key[..4], &key[key.len()-4..])
                            } else {
                                "****".to_string()
                            };
                        }
                    }
                }
                break;
            }
        }
    }

    Json(ProvidersConfigResponse {
        providers,
        default_provider,
        search_configured,
        search_api_key_masked: search_key_masked,
    })
}

fn check_search_configured() -> bool {
    let search_paths = llm_config_paths();
    for p in &search_paths {
        if let Ok(c) = std::fs::read_to_string(p) {
            if let Ok(v) = c.parse::<toml::Value>() {
                if let Some(search) = v.get("search").and_then(|s| s.as_table()) {
                    if let Some(key) = search.get("api_key").and_then(|k| k.as_str()) {
                        if !key.is_empty() { return true; }
                    }
                }
                break;
            }
        }
    }
    false
}

/// 预设厂商配置
#[derive(Deserialize)]
pub struct ProviderConfigRequest {
    pub api_key: String,
    pub base_url: String,
    pub is_full_url: Option<bool>,
    pub model: Option<String>,
}

/// Provider 保存请求
/// - `original_key` = Some：编辑模式 — 按原 toml key 定位更新；`api_key` 留空 = 保留原值；
///   改名时迁移 default_provider 指向；不切换默认配置（避免悄悄改变用户当前模型）
/// - `original_key` = None：新建模式 — api_key 必填；新配置成为默认（刚保存的即想用的）；
///   同名 key 已存在时返回 409（避免静默覆盖，走【修改】入口）
#[derive(Deserialize)]
pub struct ProviderSaveRequest {
    pub name: String,
    pub api_key: String,
    pub base_url: String,
    pub default_model: String,
    pub context_length: Option<u32>,
    pub is_full_url: Option<bool>,
    pub original_key: Option<String>,
}

/// 删除 Provider 配置请求（name = llm.toml 中的原始 key）
#[derive(Deserialize)]
pub struct ProviderDeleteRequest {
    pub name: String,
}

#[derive(Serialize)]
pub struct ModelsResponse {
    pub models: Vec<String>,
}

/// POST /api/providers/models — 代理请求获取模型列表（避免 CORS）
pub async fn provider_models_handler(
    Json(req): Json<ProviderConfigRequest>,
) -> Result<Json<ModelsResponse>, (axum::http::StatusCode, String)> {
    // 与聊天链路同源的根地址解析（测试通过 ⇒ 实际对话可用）
    let base = resolve_api_base(&req.base_url, req.is_full_url.unwrap_or(false));
    let client = reqwest::Client::new();

    // 主端点 + 兼容回退（裸主机被智能补了 /v1 时，个别网关模型列表仍在根 /models）
    let endpoints = [
        format!("{}/models", base),
        format!("{}/v1/models", base.strip_suffix("/v1").unwrap_or(&base)),
    ];

    let mut last_err = String::new();
    for url in &endpoints {
        let resp = match client
            .get(url)
            .header("Authorization", format!("Bearer {}", req.api_key))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("网络错误: {} (URL: {})", e, url);
                continue;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            last_err = format!("API 返回 {} ({}): {}", status, url, body.chars().take(300).collect::<String>());
            if status.as_u16() == 404 { continue; } // try next endpoint
            return Err((axum::http::StatusCode::BAD_GATEWAY, last_err));
        }

        let json: serde_json::Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => { last_err = format!("JSON 解析失败 ({}): {}", url, e); continue; }
        };

        let models: Vec<String> = json["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if models.is_empty() {
            // Try to extract from alternative formats
            if let Some(arr) = json.as_array() {
                let alt_models: Vec<String> = arr.iter()
                    .filter_map(|m| m["id"].as_str().or_else(|| m.as_str()).map(String::from))
                    .collect();
                if !alt_models.is_empty() {
                    return Ok(Json(ModelsResponse { models: alt_models }));
                }
            }
        }

        if !models.is_empty() {
            return Ok(Json(ModelsResponse { models }));
        }
        last_err = format!("未找到模型列表 ({}): 响应格式可能不兼容", url);
    }

    Err((axum::http::StatusCode::BAD_GATEWAY, format!("所有端点均失败: {}", last_err)))
}

/// POST /api/providers/test — 测试连接（发送最小化 chat 请求）
pub async fn provider_test_chat_handler(
    Json(req): Json<ProviderConfigRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    // 必须与实际聊天（OpenAiProvider::url）用同一套根地址解析 —
    // 此前测试走 normalize 补 /v1 而聊天不补，出现「测试成功但发消息无回应」的假象
    let base = resolve_api_base(&req.base_url, req.is_full_url.unwrap_or(false));
    let url = format!("{}/chat/completions", base);
    let client = reqwest::Client::new();

    // Use the model name from request, or fall back to common defaults
    let model_name = req.model.as_deref().unwrap_or("").trim();
    let model_name = if model_name.is_empty() {
        // Try common model names based on the base URL
        if base.contains("deepseek") { "deepseek-flash" }
        else if base.contains("openai") { "gpt-4o-mini" }
        else if base.contains("groq") { "llama-3.3-70b-versatile" }
        else { "gpt-3.5-turbo" }
    } else {
        model_name
    };

    let body = serde_json::json!({
        "model": model_name,
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 1,
        "stream": false
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", req.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, format!("网络错误: {}", e)))?;

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();

    if status.is_success() {
        Ok(Json(serde_json::json!({ "status": "ok", "message": format!("连接成功（{}）", url) })))
    } else {
        // 不做端点回退探测：测试结果必须与实际聊天完全一致。
        // 404 时明确提示用户改用「完整URL」勾选或修正地址，而不是静默换个 URL 蒙混过关。
        let hint = if status.as_u16() == 404 {
            format!(" — 端点 {} 不存在；若网关路径非 /v1，请勾选「完整URL」后填入完整端点重试", url)
        } else { String::new() };
        Err((axum::http::StatusCode::BAD_GATEWAY, format!("API 返回 {} ({}): {}{}", status, url, body_text.chars().take(300).collect::<String>(), hint)))
    }
}

/// 配置键清洗：保留字母/数字/下划线/连字符（Unicode 感知，中文名可直接作 key），
/// 其余字符替换为下划线并去除尾部下划线。
/// （旧版把非 ASCII 全打成 '_'，「自定义」会变成 ___ — UI 不可读，编辑功能治理的就是这批遗留）
fn sanitize_provider_key(name: &str) -> String {
    let key: String = name.trim().chars().map(|c| {
        if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }
    }).collect();
    let key = key.trim_end_matches('_').to_string();
    if key.is_empty() { "custom".into() } else { key }
}

/// 解析 llm.toml 实际路径（存在者优先，无则回退第 2 候选）
pub(crate) fn resolve_config_path() -> PathBuf {
    let search_paths = llm_config_paths();
    match search_paths.iter().find(|p| std::fs::metadata(p).is_ok()) {
        Some(p) => PathBuf::from(p),
        // 全新环境：落数据目录（旧版落 CWD/config 会污染程序所在目录）
        None => crate::datadir::AppDataLayout::resolve().config_dir().join("llm.toml"),
    }
}

/// 保存/编辑 Provider 到指定 toml 文件 — 纯逻辑（handler 与测试共用，路径显式注入）
fn save_provider_to(path: &std::path::Path, req: &ProviderSaveRequest) -> Result<serde_json::Value, (axum::http::StatusCode, String)> {
    let name_trim = req.name.trim();
    if name_trim.is_empty() {
        return Err((axum::http::StatusCode::BAD_REQUEST, "配置名称不能为空".into()));
    }
    if req.base_url.trim().is_empty() {
        return Err((axum::http::StatusCode::BAD_REQUEST, "Base URL 不能为空".into()));
    }

    let original_key = req.original_key.as_deref().map(str::trim).filter(|k| !k.is_empty());
    let is_edit = original_key.is_some();

    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut v: toml::Value = content.parse().unwrap_or(toml::Value::Table(toml::value::Table::new()));

    // 编辑模式前置校验：原配置必须存在（可能已被删除/改名，给前端可读的 404）
    let original_cfg = match original_key {
        Some(k) => match v.get("providers").and_then(|p| p.get(k)) {
            Some(cfg) => Some(cfg.clone()),
            None => return Err((axum::http::StatusCode::NOT_FOUND,
                format!("配置「{}」不存在（可能已被删除），请刷新列表后重试", k))),
        },
        None => None,
    };

    // 新建模式：同名 key 已存在 → 409（防静默覆盖，走【修改】入口）
    let provider_key = sanitize_provider_key(name_trim);
    if !is_edit && v.get("providers").and_then(|p| p.get(&provider_key)).is_some() {
        return Err((axum::http::StatusCode::CONFLICT,
            format!("已存在同名配置「{}」。如需更新请在上方列表点击【修改】", provider_key)));
    }
    // 新建模式 api_key 必填（编辑模式留空 = 保留原值）
    if !is_edit && req.api_key.trim().is_empty() {
        return Err((axum::http::StatusCode::BAD_REQUEST, "新建配置必须填写 API Key".into()));
    }

    // 以原配置为底覆盖更新（编辑模式）— 未在表单暴露的字段（context_length 等）与未来新增字段不丢失
    let mut provider_table = match &original_cfg {
        Some(cfg) => cfg.as_table().cloned().unwrap_or_default(),
        None => toml::value::Table::new(),
    };
    // api_key：编辑且留空 → 沿用原值；其余用请求值
    let api_key_to_write = if is_edit && req.api_key.trim().is_empty() {
        original_cfg.as_ref()
            .and_then(|c| c.get("api_key")).and_then(|k| k.as_str())
            .unwrap_or_default().to_string()
    } else {
        req.api_key.trim().to_string()
    };
    provider_table.insert("api_key".into(), toml::Value::String(api_key_to_write));
    provider_table.insert("base_url".into(), toml::Value::String(req.base_url.trim().to_string()));
    provider_table.insert("default_model".into(), toml::Value::String(req.default_model.trim().to_string()));
    if let Some(ctx_len) = req.context_length {
        provider_table.insert("context_length".into(), toml::Value::Integer(ctx_len as i64));
    }
    // 完整 URL 模式必须持久化 — 聊天链路构造端点时依赖此开关（此前丢弃导致勾选框形同虚设）
    provider_table.insert("is_full_url".into(), toml::Value::Boolean(req.is_full_url.unwrap_or(false)));

    if let Some(table) = v.as_table_mut() {
        // 改名：移除旧 key
        if let Some(k) = original_key {
            if k != provider_key {
                if let Some(providers) = table.get_mut("providers").and_then(|p| p.as_table_mut()) {
                    providers.remove(k);
                }
            }
        }
        // 写入（新建或原位/改名更新）
        match table.get_mut("providers").and_then(|p| p.as_table_mut()) {
            Some(providers) => { providers.insert(provider_key.clone(), toml::Value::Table(provider_table)); }
            None => {
                let mut providers_table = toml::value::Table::new();
                providers_table.insert(provider_key.clone(), toml::Value::Table(provider_table));
                table.insert("providers".into(), toml::Value::Table(providers_table));
            }
        }

        // default_provider 策略：
        // - 新建 → 指向新配置（刚保存的即想用的）
        // - 编辑 → 不切换默认；仅当默认正指向本配置（含改名）时跟随迁移到新 key
        // - 幽灵治理：默认指向不存在的 key（历史 sanitize 脱节的脏数据）时修正为本次配置 —
        //   否则永远指向空目标，用户无法通过任何 UI 操作自愈
        let current_default = table.get("agent")
            .and_then(|a| a.get("default_provider")).and_then(|x| x.as_str())
            .unwrap_or("").to_string();
        let mut new_default = if is_edit {
            if current_default == original_key.unwrap_or("") { provider_key.clone() } else { current_default }
        } else {
            provider_key.clone()
        };
        if table.get("providers").and_then(|p| p.get(&new_default)).is_none() {
            new_default = provider_key.clone();
        }
        match table.get_mut("agent").and_then(|a| a.as_table_mut()) {
            Some(agent) => { agent.insert("default_provider".into(), toml::Value::String(new_default)); }
            None => {
                let mut agent_table = toml::value::Table::new();
                agent_table.insert("default_provider".into(), toml::Value::String(new_default));
                table.insert("agent".into(), toml::Value::Table(agent_table));
            }
        }
    }

    let new_content = toml::to_string(&v)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {}", e)))?;
    std::fs::write(path, new_content)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {}", e)))?;

    let action = if is_edit { "已更新配置" } else { "已保存配置" };
    Ok(serde_json::json!({
        "status": "ok",
        "message": format!("{}「{}」（模型 {}）。重启应用后生效", action, provider_key, req.default_model),
        "provider_key": provider_key,
        "restart_required": true
    }))
}

/// 从指定 toml 文件删除 Provider 配置 — 纯逻辑（handler 与测试共用）
fn delete_provider_from(path: &std::path::Path, key: &str) -> Result<serde_json::Value, (axum::http::StatusCode, String)> {
    let key = key.trim();
    if key.is_empty() {
        return Err((axum::http::StatusCode::BAD_REQUEST, "配置名称不能为空".into()));
    }
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut v: toml::Value = content.parse().unwrap_or(toml::Value::Table(toml::value::Table::new()));

    let removed = v.get_mut("providers").and_then(|p| p.as_table_mut()).and_then(|t| t.remove(key));
    if removed.is_none() {
        return Err((axum::http::StatusCode::NOT_FOUND, format!("配置「{}」不存在（可能已被删除）", key)));
    }

    // 默认配置被删，或默认本就指向无效 key（历史 sanitize 脱节的幽灵指向）→ 迁移到剩余第一个
    //（toml Table 为 BTreeMap，字母序确定性）
    let mut new_default = String::new();
    if let Some(table) = v.as_table_mut() {
        let current_default = table.get("agent")
            .and_then(|a| a.get("default_provider")).and_then(|x| x.as_str())
            .unwrap_or("").to_string();
        let default_alive = !current_default.is_empty()
            && table.get("providers").and_then(|p| p.get(&current_default)).is_some();
        if current_default == key || !default_alive {
            new_default = table.get("providers")
                .and_then(|p| p.as_table())
                .and_then(|t| t.keys().next().cloned())
                .unwrap_or_default();
            match table.get_mut("agent").and_then(|a| a.as_table_mut()) {
                Some(agent) => { agent.insert("default_provider".into(), toml::Value::String(new_default.clone())); }
                None => {
                    let mut agent_table = toml::value::Table::new();
                    agent_table.insert("default_provider".into(), toml::Value::String(new_default.clone()));
                    table.insert("agent".into(), toml::Value::Table(agent_table));
                }
            }
        }
    }

    let new_content = toml::to_string(&v)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {}", e)))?;
    std::fs::write(path, new_content)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {}", e)))?;

    let default_hint = if new_default.is_empty() {
        "（无剩余配置，默认置空）".to_string()
    } else {
        format!("，默认配置迁移至「{}」", new_default)
    };
    Ok(serde_json::json!({
        "status": "ok",
        "message": format!("已删除配置「{}」{}。重启应用后生效", key, default_hint),
        "default_provider": new_default,
        "restart_required": true
    }))
}

/// POST /api/providers/save — 保存/编辑 Provider 配置到 llm.toml
pub async fn provider_save_handler(
    State(ctx): State<AppContext>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ProviderSaveRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    // llm.toml 是全局共享配置：base_url 一旦被改写，所有用户的对话与 key
    // 都会流经攻击者服务器 —— server 模式仅管理员可写（standalone 不受影响）
    if ctx.auth.is_some() && crate::auth::require_admin(&ctx, &headers).is_err() {
        return Err((axum::http::StatusCode::FORBIDDEN, "server 模式下仅管理员可修改模型配置".into()));
    }
    Ok(Json(save_provider_to(&resolve_config_path(), &req)?))
}

/// POST /api/providers/delete — 删除 llm.toml 中的 Provider 配置
pub async fn provider_delete_handler(
    State(ctx): State<AppContext>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ProviderDeleteRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    if ctx.auth.is_some() && crate::auth::require_admin(&ctx, &headers).is_err() {
        return Err((axum::http::StatusCode::FORBIDDEN, "server 模式下仅管理员可删除模型配置".into()));
    }
    Ok(Json(delete_provider_from(&resolve_config_path(), &req.name)?))
}

/// 搜索配置请求
#[derive(Deserialize)]
pub struct SearchConfigRequest {
    pub api_key: String,
}

/// GET /api/search/status — 检查搜索是否已配置（不暴露 API Key）
pub async fn search_status_handler() -> Json<serde_json::Value> {
    let search_paths = llm_config_paths();
    let mut configured = false;
    for p in &search_paths {
        if let Ok(c) = std::fs::read_to_string(p) {
            if let Ok(v) = c.parse::<toml::Value>() {
                if let Some(search) = v.get("search").and_then(|s| s.as_table()) {
                    if let Some(key) = search.get("api_key").and_then(|k| k.as_str()) {
                        if !key.is_empty() { configured = true; break; }
                    }
                }
                break;
            }
        }
    }
    Json(serde_json::json!({ "configured": configured }))
}

/// POST /api/search/test — 测试 Tavily API 连通性
pub async fn search_test_handler(
    Json(req): Json<SearchConfigRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.tavily.com/search")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "api_key": req.api_key,
            "query": "test",
            "max_results": 1,
        }))
        .send()
        .await
        .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, format!("请求失败: {}", e)))?;

    if resp.status().is_success() {
        Ok(Json(serde_json::json!({ "status": "ok", "message": "连接成功" })))
    } else {
        let status = resp.status();
        Err((axum::http::StatusCode::BAD_GATEWAY, format!("Tavily 返回 {}", status)))
    }
}

/// POST /api/search/save — 保存搜索 API Key 到配置文件
pub async fn search_save_handler(
    State(ctx): State<AppContext>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SearchConfigRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    if ctx.auth.is_some() && crate::auth::require_admin(&ctx, &headers).is_err() {
        return Err((axum::http::StatusCode::FORBIDDEN, "server 模式下仅管理员可修改搜索配置".into()));
    }
    // 写入 config/llm.toml 的 [search] 段
    let search_paths = llm_config_paths();

    let config_path = search_paths.iter().find(|p| std::fs::metadata(p).is_ok());
    let config_path = match config_path {
        Some(p) => p.clone(),
        None => search_paths[2].clone(), // fallback
    };

    // 读取现有配置
    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut v: toml::Value = content.parse().unwrap_or(toml::Value::Table(toml::value::Table::new()));

    if let Some(table) = v.as_table_mut() {
        let mut search_table = toml::value::Table::new();
        search_table.insert("api_key".into(), toml::Value::String(req.api_key.clone()));
        search_table.insert("max_results".into(), toml::Value::Integer(5));
        table.insert("search".into(), toml::Value::Table(search_table));
    }

    let new_content = toml::to_string(&v).map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {}", e)))?;
    std::fs::write(&config_path, new_content).map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {}", e)))?;

    // 更新运行时
    tool_executor::set_search_api_key(&req.api_key);

    Ok(Json(serde_json::json!({ "status": "ok", "message": "已保存" })))
}

/// 预设厂商列表
#[derive(Serialize)]
pub struct PresetResponse {
    pub presets: Vec<Preset>,
}

#[derive(Serialize)]
pub struct Preset {
    pub name: String,
    pub base_url: String,
    pub default_model: String,
    pub source: String,
    pub verified: bool,
    pub checked_at: Option<String>,
}

/// GET /api/providers/presets — 获取预设厂商列表
pub async fn presets_handler() -> Json<PresetResponse> {
    let layout = crate::datadir::AppDataLayout::resolve();
    let catalog = crate::model_catalog::load_model_catalog(&layout);
    let mut presets: Vec<Preset> = catalog.models.into_iter().map(|entry| Preset {
        name: entry.id,
        base_url: entry.base_url,
        default_model: entry.model,
        source: entry.source,
        verified: entry.verified,
        checked_at: entry.checked_at,
    }).collect();
    if presets.is_empty() {
        presets.push(Preset {
            name: "自定义".into(),
            base_url: String::new(),
            default_model: String::new(),
            source: "manual".into(),
            verified: false,
            checked_at: None,
        });
    }
    Json(PresetResponse { presets })
}

/// 合并多图（images 数组）与单图（image 字段）— 单图向后兼容，两者并存时 images 在前
fn collect_images(req: &ChatRequest) -> Vec<String> {
    let mut out: Vec<String> = req.images.clone().unwrap_or_default();
    if let Some(img) = req.image.as_deref() {
        if !img.is_empty() && !out.iter().any(|x| x == img) {
            out.push(img.to_string());
        }
    }
    out.retain(|x| !x.is_empty());
    out
}

/// 从历史消息 + 当前消息构建 LLM 消息序列
fn build_messages(history: &[HistoryMessage], current_message: &str, thinking_effort: &str, execution_mode: &str, custom_system: Option<&str>, images: &[String], attachments: Option<&[ChatAttachment]>, workspace: Option<&str>, work_dir: &std::path::Path) -> Vec<Message> {
    let effort_hint = match thinking_effort {
        "low" => "简洁回答，不需要详细推理。",
        "medium" => "正常回答，适度推理。",
        "high" => "请进行深度推理，逐步思考后再回答。",
        "max" => "请进行最深入的推理，详细分析每个步骤，确保答案的正确性和完整性。",
        "ultra" => "最高档位（ultra）：先穷举候选方案与风险，逐项评估后再给出结论；涉及不可逆操作时先列出影响面征求确认；完成后自查一遍再输出。",
        "off" | "none" => "思考已关闭：直接作答，不展开长推理。",
        _ => "",
    };

    let mode_hint = match execution_mode {
        "plan" => "你处于【计划模式】：只能读取与分析，输出结构化执行计划（编号步骤、每步可执行、含验证方式），禁止修改任何文件或执行命令。输出计划后即停止 — 用户批准后会自动切换执行。",
        "edit" => "你处于【编辑模式】：可读写文件与执行命令，但【所有】文件修改与命令都会弹出授权窗口，经用户逐项批准后才会真正执行。被拒绝时换合规方案，不要原样重试被拒操作；任务需要不受限执行时，提示用户切换到自动或完全模式。",
        "auto" => "你处于【自动模式】：自主执行，安全分级放行 — 读取与新建文件直接执行；修改既有文件或执行非白名单命令时会先经安全自评，不确定或危险的操作将弹出授权窗口等待用户。涉及不可逆、大范围变更或用户重要数据时，应主动调用 request_approval 请求人工授权。可自由派生子代理（spawn_agent）并行调研。被拒绝后换方案，不要原样重试。",
        "readonly" => "你处于【只读模式】：只能读取与分析（读文件/查找/搜索/知识库与待办读取），禁止任何写入、编辑、删除与命令执行。若任务必须修改数据或执行命令，明确告知用户：请在输入区「模式」菜单切换到编辑、自动或完全模式后再试。",
        "full" => "你处于【完全模式】：全部权限已放开，无任何审批流程。你仍须遵守工作纪律 — 先读后写、不破坏用户既有数据、不可逆操作前说明影响。",
        _ => "",
    };

    let base_system = custom_system.unwrap_or(
        "你是 SKYLAND Agent 终端的核心智能体，在用户的工作台（本地或云端）中完成任务。\n\
         \n\
         【核心能力】\n\
         \n\
         ■ 文件与代码\n\
         - read_file / glob / grep：读取与检索本地文件（read_file 单文件全文，glob 按模式匹配，grep 按内容搜索）\n\
         - write_file / edit_file：写入与编辑文件（write_file 新建/覆盖，edit_file 精确替换）\n\
         - bash：执行 shell 命令（30 秒超时，有白名单；非白名单命令按模式管控）\n\
         \n\
         ■ 文档生成（推荐用这些，不要用 write_file 写文档内容）\n\
         - create_document：生成 Word 文档（.docx），输入 Markdown 内容，自动排版\n\
         - create_spreadsheet：生成 Excel 表格（.xlsx），输入行列数据\n\
         - create_presentation：生成 PPT 演示文稿（.pptx），输入幻灯片数组\n\
         - create_pdf：生成 PDF 文档，输入 Markdown 内容\n\
         - 产物自动落入会话工作目录，用户可在工作目录面板查看和下载\n\
         \n\
         ■ 知识库\n\
         - kb_list：列出全部知识库 → kb_search：全文检索 → kb_read：读取指定文档\n\
         - 引用知识库内容时注明来源文档路径\n\
         \n\
         ■ 待办与计划\n\
         - todo_read：读取用户待办（scope=today 今日 / recent 最近 7 天）\n\
         - todo_create：创建待办（速记语法：#分类 !!优先级 @HH:MM 提醒时间）\n\
         - todo_write：维护当前任务的执行计划清单（≥3 步任务必用；开始标 in_progress，完成即标 completed）\n\
         \n\
         ■ 技能与定时任务\n\
         - use_skill：加载技能的完整指令（技能清单见下方「可用技能」）→ 加载后严格遵循其指令执行\n\
         - create_task：创建定时任务（AI 自动执行，支持 daily/weekly/interval/once）\n\
         \n\
         ■ 联网与调研\n\
         - web_search：搜索互联网获取最新信息\n\
         - spawn_agent：派生子智能体独立干活（默认 readonly=只读分析；subagent_type=coder 且 mode=readwrite 才可写文件）\n\
           · **只读分析**用 explore/researcher/general-purpose：读架构、查资料、定位问题，把结论交回你，由你决定怎么改\n\
           · **写结构性代码**用 coder + mode=readwrite：把互相独立的新建/修改任务分给多个 coder 并行写入\n\
           · 每个子代理一个明确的文件范围；多代理不要改同一文件（会互相覆盖）\n\
           · 只读子代理看不到完整项目规范，你的 task 描述是它唯一依据，必须写全

         \n\
         ■ AI 生图\n\
         - generate_image：调用系统内置生图引擎文生图（自动保存到生图目录、自动在对话中展示图片卡片）\n\
         - 画面描述要具体：主体 + 风格 + 构图 + 光线；用户没指定风格时选择贴合主题的风格\n\
         \n\
         ■ 风险管控\n\
         - request_approval：主动请求用户人工授权（auto 模式下涉及不可逆/大范围/重要数据时使用）\n\
         \n\
         ■ 工具选择指引\n\
         - 需要生成文档文件 → 用 create_document / create_spreadsheet / create_presentation / create_pdf\n\
         - 需要修改代码 → 用 read_file 先读，再 edit_file 精确修改（不要用 write_file 整个覆盖代码文件）\n\
         - 需要新建纯文本/配置/代码文件 → 用 write_file\n\
         - 需要查资料 → 优先 kb_search 知识库，其次 web_search 联网\n\
         - 需要多步任务 → 先 todo_write 拆计划，再逐步执行\n\
         - 需要并行调研 → 同轮派生多个 spawn_agent（全部只读，互不冲突）
         - 需要**并行写代码** → 同轮派生多个 coder（mode=readwrite），每个负责一组互不重叠的文件\n\
         - 需要生成图片 → 用 generate_image（禁止用 bash/curl 手动调用图片 API — 那是脆弱的旁路做法）\n\
         - Office/PDF 文件一律用内置 create_* 工具生成：禁止 pip install 安装 Python 库（openpyxl/python-docx 等）、禁止写脚本生成 Office 文件 — 环境没有这些依赖，安装必然失败且浪费时间；内置工具零依赖、产物规范且用户可直接下载\n\
         \n\
         注：各模式下实际可用的工具以本提示末尾的「本模式可用工具」清单为准，不在清单中的工具不可调用。\n\
         \n\
         【工作纪律】\n\
         1. 先理解后动手：修改文件前先 read_file 读取目标文件；用户信息不足时先问一句，禁止编造事实。\n\
         2. 生成文档一律使用 create_* 工具（而非 write_file 手写文档格式）；覆盖已有文件前说明影响。\n\
         3. 多步任务（≥3 步）先用 todo_write 建立执行计划清单再动手；执行中每完成一步立即标记 completed（不攒批）、同一时间仅一项 in_progress；完成后用不超过三行总结结果与后续建议。\n\
         4. 命令与写入按模式分级管控（编辑/自动模式下部分操作需用户授权），被拒时换合规方案，不要原样重试被拒操作。\n\
         5. 必须收尾：任何任务都要以一段面向用户的最终答复结束（做了什么、产出在哪、下一步建议）；严禁在最后一轮工具调用后不再说话就结束。\n\
         6. 产出即告知：生成了文档、图片或任何文件后，必须在最终答复中写明文件的完整保存路径（含目录的绝对路径，直接可复制定位）——只报文件名等于没说，用户靠路径找到产物。\n\
         \n\
         【输出规范】\n\
         - 中文回复，Markdown 结构化（### 小节标题、列表、必要时用表格）\n\
         - 代码给完整可运行片段并标注语言；数学公式用 $...$ \n\
         - 推测性内容明确标注（推测）；引用外部信息注明来源"
    );

    let workspace_hint = workspace.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() { return None; }
        if trimmed == "@cloud" {
            return Some("用户已启用云工作模式：文件生成在云端会话目录。".into());
        }
        let root = work_dir.canonicalize().ok()?;
        let candidate = if std::path::Path::new(trimmed).is_absolute() {
            std::path::PathBuf::from(trimmed)
        } else {
            root.join(trimmed)
        };
        // 绝对工作区（用户自选目录）允许在程序目录之外 — 工具层以工作区为界执行，
        // 提示层如实告知；旧逻辑 starts_with(&root) 会把这类工作区静默丢弃
        let canonical = candidate.canonicalize().unwrap_or(candidate);
        Some(format!("用户指定的工作区：{}。文件读写与命令执行都应在此工作区内进行（相对路径按工作区解析）。", canonical.display()))
    }).unwrap_or_default();
    let system = format!("{}\n\n{}\n{}\n{}", base_system, effort_hint, mode_hint, workspace_hint);

    let mut msgs = vec![Message::system(&system)];

    for h in history {
        match h.role.as_str() {
            "user" => msgs.push(Message::user(&h.content)),
            "assistant" => msgs.push(Message::assistant(&h.content)),
            _ => {}
        }
    }

    let mut context = String::new();
    if let Some(items) = attachments {
        let mut total = 0usize;
        for item in items.iter().take(10) {
            if item.name.len() > 240 || item.content.len() > 1_048_576 || total + item.content.len() > 5_242_880 { continue; }
            total += item.content.len();
            context.push_str(&format!("\n\n--- 用户附加文件：{} ---\n{}\n--- 文件结束 ---", item.name, item.content));
        }
    }
    let user_text = format!("{}{}", current_message, context);
    if !images.is_empty() { msgs.push(Message::user_multimodal_multi(&user_text, images)); }
    else { msgs.push(Message::user(&user_text)); }
    msgs
}

/// 工具调用的人类可读动作描述（前端状态行）
fn describe_tool_action(name: &str, args: &str) -> (String, String) {
    let parsed: serde_json::Value = serde_json::from_str(args).unwrap_or_default();
    let field = |key: &str| parsed[key].as_str().unwrap_or("").chars().take(40).collect::<String>();
    let (action, detail) = match name {
        "read_file" => ("读取文件", field("path")),
        "write_file" => ("写入文件", field("path")),
        "edit_file" => ("编辑文件", field("path")),
        "bash" => ("执行命令", field("command")),
        "glob" => ("查找文件", field("pattern")),
        "grep" => ("搜索代码", field("pattern")),
        "web_search" => ("搜索网络", field("query")),
        "kb_list" => ("列出知识库", String::new()),
        "kb_search" => ("检索知识库", field("query")),
        "kb_read" => ("阅读知识库文档", field("path")),
        "todo_read" => ("查看待办速记", String::new()),
        "todo_create" => ("创建待办", field("text")),
        "use_skill" => ("加载技能", field("name")),
        "create_task" => ("创建定时任务", field("name")),
        "create_document" => ("生成 Word 文档", field("path")),
        "create_spreadsheet" => ("生成表格", field("path")),
        "create_presentation" => ("生成演示文稿", field("path")),
        "create_pdf" => ("生成 PDF", field("path")),
        "generate_image" => ("生成图片", field("prompt")),
        "kb_create_kb" => ("创建知识库", field("name")),
        "kb_create_doc" => ("创建知识库文档", field("path")),
        "kb_delete_doc" => ("删除知识库文档", field("path")),
        "kb_edit_doc" => ("编辑知识库文档", field("path")),
        _ => ("调用工具", name.to_string()),
    };
    (action.to_string(), detail)
}

/// 从工具输出提取产出文件并生成对话卡片 URL。
/// generate_image → /api/image/file 直链（图片卡片）；create_* → 会话工作目录相对路径 +
/// /api/chat/artifact 服务（文件卡片）。
/// 返回 (name, url, kind, path)：path 为产物完整磁盘路径（右栏"输出内容"展示 —
/// 用户最关心的"文件在哪"，只有文件名等于没说）。
fn extract_file_output(
    tool: &str,
    output: &str,
    workdir: Option<&str>,
    conv: Option<u64>,
    session_wd: Option<&std::path::Path>,
) -> Option<(String, String, String, String)> {
    if tool == "generate_image" {
        let idx = output.find("/api/image/file/")?;
        let tail = &output[idx..];
        let url: String = tail.chars()
            .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
            .collect();
        let name = url.rsplit('/').next()?.to_string();
        if name.is_empty() || !url.starts_with("/api/image/file/") { return None; }
        return Some((name, url, "image".into(), String::new()));
    }
    // create_*：输出形如 "已生成 Word 文档: {绝对路径}（…）"
    let start = output.find(": ")? + 2;
    let rest = &output[start..];
    let mut hit: Option<(usize, usize)> = None;
    for e in [".docx", ".xlsx", ".pptx", ".pdf"] {
        if let Some(i) = rest.find(e) {
            if hit.map(|(h, _)| i < h).unwrap_or(true) { hit = Some((i, e.len())); }
        }
    }
    let (ei, elen) = hit?;
    let raw = &rest[..ei + elen];
    if raw.is_empty() { return None; }
    // 会话工作目录内的相对路径（URL 用）；不在其内则退回纯文件名
    let rel = session_wd
        .and_then(|wd| std::path::Path::new(raw).strip_prefix(wd).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|p| !p.is_empty())
        .or_else(|| std::path::Path::new(raw).file_name().map(|f| f.to_string_lossy().to_string()))?;
    let url = format!("/api/chat/artifact?name={}&workdir={}&conv={}",
        urlencode(&rel),
        urlencode(workdir.unwrap_or("")),
        conv.map(|c| c.to_string()).unwrap_or_default());
    Some((rel, url, "file".into(), raw.to_string()))
}

/// 查询参数百分号编码（非保留字符一律转义 — 零依赖实现）
fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[derive(Deserialize)]
pub struct ContextEstimateRequest {
    pub history: Vec<HistoryMessage>,
    pub model: Option<String>,
    pub context_length: Option<u32>,
}

/// POST /api/chat/context-estimate — 按历史估算下一轮上下文占用。
/// 会话切换时前端调用以恢复右栏上下文条（旧会话无 _ctx 持久化数据时）。
pub async fn context_estimate_handler(
    State(ctx): State<AppContext>,
    Json(req): Json<ContextEstimateRequest>,
) -> Json<serde_json::Value> {
    let model = req.model.unwrap_or_else(|| ctx.default_model.clone());
    let total = req.context_length
        .or_else(|| ctx.context_lengths.get(&model).copied())
        .unwrap_or(128000);
    // 历史正文估算 + 固定系统提示/工具清单开销（约 2.5k token）
    let used: u32 = req.history.iter().map(|m| estimate_content_tokens(&m.content)).sum::<u32>() + 2500;
    Json(serde_json::json!({ "status": "ok", "used": used, "total": total }))
}

#[derive(Deserialize)]
pub struct ArtifactQuery {
    pub name: String,
    pub workdir: Option<String>,
    pub conv: Option<u64>,
}

/// GET /api/chat/artifact — 会话产出文件服务（create_* 文件卡片的点击/下载落点）。
/// name 仅允许 Normal 组件（拒绝 ../、绝对路径、盘符）；扩展名白名单。
pub async fn chat_artifact_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<crate::auth::CurrentUser>>,
    axum::extract::Query(q): axum::extract::Query<ArtifactQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let name = q.name.trim();
    let p = std::path::Path::new(name);
    if name.is_empty() || p.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return (axum::http::StatusCode::BAD_REQUEST, "非法文件路径").into_response();
    }
    let ext_ok = p.extension().and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(),
            "docx" | "xlsx" | "pptx" | "pdf" | "png" | "jpg" | "jpeg" | "txt" | "md" | "csv"))
        .unwrap_or(false);
    if !ext_ok {
        return (axum::http::StatusCode::FORBIDDEN, "不支持的文件类型").into_response();
    }
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = crate::filegen::resolve_session_workdir_for(
        &ctx,
        current_user,
        q.workdir.as_deref(),
        q.conv,
    );
    let Some(dir) = dir else {
        return (axum::http::StatusCode::BAD_REQUEST, "会话工作目录不可用").into_response();
    };
    let path = dir.join(p);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            ([(axum::http::header::CONTENT_TYPE, mime.as_ref().to_string())], bytes).into_response()
        }
        Err(_) => (axum::http::StatusCode::NOT_FOUND, "文件不存在").into_response(),
    }
}

/// 从配置文件加载多个 LLM Provider
pub fn load_providers(user_data_dir: PathBuf) -> AppContext {
    let mut providers = HashMap::new();
    let mut default_model = String::new();
    let mut context_lengths: HashMap<String, u32> = HashMap::new();
    let mut provider_models: Vec<(String, String)> = Vec::new(); // (provider名, 模型名)

    let work_dir = user_data_dir.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
    let image_dir = user_data_dir.join("images");
    std::fs::create_dir_all(&image_dir).ok();
    let todo_dir_path = user_data_dir.join("todo");
    std::fs::create_dir_all(&todo_dir_path).ok();
    let cloud_workdir = {
        let p = user_data_dir.join("workspaces").join("cloud");
        std::fs::create_dir_all(&p).ok();
        p
    };
    let mut search_paths = vec![work_dir.join("config").join("llm.toml").display().to_string()];
    search_paths.extend(llm_config_paths());

    for p in &search_paths {
        if let Ok(c) = std::fs::read_to_string(p) {
            if let Ok(v) = c.parse::<toml::Value>() {
                default_model = v.get("agent")
                    .and_then(|a| a.get("default_provider"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("deepseek")
                    .to_string();

                if let Some(prs) = v.get("providers").and_then(|v| v.as_table()) {
                    for (name, cfg) in prs.iter() {
                        if let Some((model_name, provider, ctx_len)) = build_provider(cfg) {
                            provider_models.push((name.clone(), model_name.clone()));
                            providers.insert(model_name.clone(), provider);
                            context_lengths.insert(model_name, ctx_len);
                        }
                    }
                }
                break;
            }
        }
    }

    if providers.is_empty() {
        if let Ok(k) = std::env::var("OPENAI_API_KEY") {
            if !k.is_empty() {
                providers.insert(
                    "gpt-4o".into(),
                    Arc::new(OpenAiProvider::new(LlmConfig::new("gpt-4o", &k))),
                );
                context_lengths.insert("gpt-4o".into(), 128000);
                default_model = "gpt-4o".into();
            }
        }
    } else {
        // 修复：默认模型 = 配置的 default_provider 对应厂商的 default_model。
        // 旧实现取 HashMap 第一个 key（随机序）且把 provider 名当模型名，多厂商时默认模型随重启漂移。
        default_model = resolve_default_model(&default_model, &provider_models).unwrap_or_else(|| {
            providers.keys().next().cloned().unwrap_or_default()
        });
    }
    AppContext {
        providers,
        default_model,
        user_data_dir,
        work_dir: work_dir.to_path_buf(),
        context_lengths,
        image_dir: Arc::new(std::sync::RwLock::new(image_dir)),
        todo_dir: Arc::new(std::sync::RwLock::new(todo_dir_path)),
        auth: None,
        server_mode: "standalone".to_string(),
        cloud_workdir: cloud_workdir.clone(),
    }
}

fn build_provider(cfg: &toml::Value) -> Option<(String, Arc<OpenAiProvider>, u32)> {
    let key = cfg.get("api_key").and_then(|v| v.as_str()).unwrap_or("");
    let key = if key.starts_with("${") {
        std::env::var(&key[2..key.len() - 1]).unwrap_or_default()
    } else {
        key.into()
    };
    if key.is_empty() { return None; }

    let model = cfg.get("default_model").and_then(|v| v.as_str()).unwrap_or("gpt-4o");
    let ctx_len = cfg.get("context_length").and_then(|v| v.as_integer()).map(|v| v as u32).unwrap_or(128000);
    let mut c = LlmConfig::new(model, &key);
    c.context_length = ctx_len;
    if let Some(url) = cfg.get("base_url").and_then(|v| v.as_str()) {
        c.base_url = url.into();
    }
    c.is_full_url = cfg.get("is_full_url").and_then(|v| v.as_bool()).unwrap_or(false);
    Some((model.to_string(), Arc::new(OpenAiProvider::new(c)), ctx_len))
}

/// 默认模型解析：配置的 default_provider（厂商名）→ 该厂商的 default_model；
/// 未命中时取列表第一个（顺序来自配置解析，确定性）。
fn resolve_default_model(configured_provider: &str, provider_models: &[(String, String)]) -> Option<String> {
    provider_models.iter()
        .find(|(name, _)| name == configured_provider)
        .map(|(_, model)| model.clone())
        .or_else(|| provider_models.first().map(|(_, model)| model.clone()))
}

/// 上下文占用估算 — 多模态消息按文本长度 + 每图固定 1000 token 估算，
/// 直接对 base64 JSON 取 len/4 会把图片算成几十万 token 撑爆上下文进度条。
fn estimate_content_tokens(content: &str) -> u32 {
    if Message::is_multimodal_content(content) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(content) {
            if let Some(arr) = v.as_array() {
                let text_len: usize = arr.iter()
                    .filter_map(|p| p["text"].as_str().map(|s| s.len()))
                    .sum();
                let images = arr.iter().filter(|p| p.get("image_url").is_some()).count();
                return (text_len as u32) / 4 + (images as u32) * 1000;
            }
        }
    }
    (content.len() as u32) / 4
}

#[cfg(test)]
mod tests {
    use super::*;
    fn _tl(v: &[&str]) -> Vec<String> { v.iter().map(|x| x.to_string()).collect() }

    // ── 子代理写权限：两把钥匙 + 主体硬边界 ──

    #[test]
    fn sub_agent_write_needs_both_keys() {
        // 只给 mode=readwrite，类型没声明写工具 → 拿不到写权限（防自我提权）
        let (ok, why) = resolve_sub_agent_write("explore", "readwrite", "auto", &_tl(&["read_file", "glob", "grep"]));
        assert!(!ok, "explore 不得因 mode 而获得写权限");
        assert!(why.unwrap().contains("未声明写工具"), "必须说明原因供模型改参");
    }

    #[test]
    fn sub_agent_readonly_mode_never_writes() {
        let (ok, why) = resolve_sub_agent_write("coder", "readonly", "auto", &_tl(&["read_file", "write_file"]));
        assert!(!ok, "没传 mode=readwrite 时即便类型是 coder 也不可写");
        assert!(why.is_none(), "只读是默认值，不该报成拒绝");
    }

    #[test]
    fn sub_agent_coder_readwrite_gets_write() {
        let (ok, why) = resolve_sub_agent_write("coder", "readwrite", "auto", &_tl(&["read_file", "write_file", "edit_file"]));
        assert!(ok, "两把钥匙齐备应放行，实际原因: {:?}", why);
        assert!(why.is_none());
    }

    #[test]
    fn readonly_main_agent_blocks_sub_agent_writes() {
        for mode in ["readonly", "plan"] {
            let (ok, why) = resolve_sub_agent_write("coder", "readwrite", mode, &_tl(&["read_file", "write_file"]));
            assert!(!ok, "主智能体 {} 时子代理不得写", mode);
            assert!(why.unwrap().contains("只读/规划模式"), "原因需指明是主体模式限制");
        }
    }

    #[test]
    fn writable_agent_allowed_in_writable_main_modes() {
        for mode in ["auto", "edit", "full"] {
            let (ok, _) = resolve_sub_agent_write("coder", "readwrite", mode, &_tl(&["read_file", "write_file"]));
            assert!(ok, "主模式 {} 下 coder 应可写", mode);
        }
    }

    #[test]
    fn writable_registry_never_includes_spawn_agent() {
        assert!(!WRITABLE_SUB_AGENT_TOOLS.contains(&"spawn_agent"));
        assert!(WRITABLE_SUB_AGENT_TOOLS.contains(&"write_file"));
        assert!(WRITABLE_SUB_AGENT_TOOLS.contains(&"edit_file"));
    }

    #[test]
    fn coder_sub_agent_is_registered_with_write_tools() {
        let coder = SUB_AGENT_DEFS.iter().find(|d| d.agent_type == "coder")
            .expect("必须注册 coder 子代理类型");
        assert!(coder.tools.iter().any(|t| WRITABLE_SUB_AGENT_TOOLS.contains(t)),
            "coder 必须在类型白名单里声明写工具，否则 mode=readwrite 永远不生效");
        for d in SUB_AGENT_DEFS {
            assert!(!d.tools.contains(&"spawn_agent"), "{} 不得白名单 spawn_agent", d.agent_type);
        }
    }

    #[test]
    fn test_resolve_default_model_by_provider_name() {
        let models = vec![
            ("deepseek".to_string(), "deepseek-chat".to_string()),
            ("zhipu".to_string(), "glm-4".to_string()),
        ];
        assert_eq!(resolve_default_model("zhipu", &models), Some("glm-4".to_string()));
        assert_eq!(resolve_default_model("deepseek", &models), Some("deepseek-chat".to_string()));
        // 未配置的 provider → 回落列表第一个（确定性，不再随机）
        assert_eq!(resolve_default_model("missing", &models), Some("deepseek-chat".to_string()));
        assert_eq!(resolve_default_model("missing", &[]), None);
    }

    #[test]
    fn agent_end_serializes_failure_status() {
        let event = SseEvent::AgentEnd {
            id: "a".into(),
            name: "worker".into(),
            status: "failed".into(),
            rounds: 2,
            elapsed_ms: 10,
            result: String::new(),
            error_code: Some("empty_task".into()),
            error_message: Some("task empty".into()),
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["status"], "failed");
        assert_eq!(value["error_code"], "empty_task");
    }

    #[test]
    fn user_data_dir_for_server_isolates_users() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ctx = AppContext {
            providers: HashMap::new(),
            default_model: String::new(),
            user_data_dir: tmp.path().join("users").join("local"),
            work_dir: tmp.path().to_path_buf(),
            context_lengths: HashMap::new(),
            image_dir: Arc::new(std::sync::RwLock::new(tmp.path().to_path_buf())),
            todo_dir: Arc::new(std::sync::RwLock::new(tmp.path().join("todo"))),
            auth: None,
            server_mode: "server".into(),
            cloud_workdir: tmp.path().join("CloudWork"),
        };
        ctx.auth = Some(crate::auth::AuthState::new(tmp.path().join("auth")));
        let alice = user_data_dir_for(&ctx, Some(&crate::auth::CurrentUser {
            username: "alice".into(), role: "user".into(),
        }));
        let bob = user_data_dir_for(&ctx, Some(&crate::auth::CurrentUser {
            username: "bob".into(), role: "user".into(),
        }));
        assert_eq!(alice, tmp.path().join("users").join("alice"));
        assert_eq!(bob, tmp.path().join("users").join("bob"));
        assert_ne!(alice, bob);
    }

    #[test]
    fn load_providers_uses_local_user_scoped_default_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let user_dir = tmp.path().join("users").join("local");
        std::fs::create_dir_all(&user_dir).unwrap();
        let ctx = load_providers(user_dir.clone());
        assert_eq!(*ctx.image_dir.read().unwrap(), user_dir.join("images"));
        assert_eq!(*ctx.todo_dir.read().unwrap(), user_dir.join("todo"));
        assert_eq!(ctx.cloud_workdir, user_dir.join("workspaces").join("cloud"));
    }

    #[test]
    fn test_estimate_tokens_multimodal_vs_text() {
        let image_b64 = "A".repeat(400_000); // 40万字节 base64
        let multimodal = serde_json::json!([
            {"type": "text", "text": "这是什么"},
            {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", image_b64)}}
        ]).to_string();
        let est = estimate_content_tokens(&multimodal);
        // 文本 12 字节/4 = 3 + 图片 1000，远小于按字节估算的 10 万+
        assert!(est <= 2000, "多模态估算不应被 base64 撑爆: {}", est);

        let text_est = estimate_content_tokens("hello world this is text");
        assert_eq!(text_est, 24 / 4);
    }

    #[test]
    fn test_gate_edit_mode_requires_approval_for_all_writes() {
        // 【编辑模式】核心承诺：所有命令与文件修改（哪怕新建文件）一律 level 2 → 弹授权窗口
        let wd = std::path::Path::new("/tmp/wd");
        let no_agent_files = crate::write_guard::AgentFiles::new();
        let bash_args = serde_json::json!({"command": "date"});
        assert_eq!(classify_gate("edit", "bash", &bash_args, &no_agent_files, wd).unwrap().level, 2,
            "编辑模式下任何命令（含无害 date）都必须走授权窗口");
        let write_args = serde_json::json!({"path": "brand_new_file.md", "content": "x"});
        assert_eq!(classify_gate("edit", "write_file", &write_args, &no_agent_files, wd).unwrap().level, 2,
            "编辑模式下新建文件同样需要授权");
        let edit_args = serde_json::json!({"path": "any.md", "old": "a", "new": "b"});
        assert_eq!(classify_gate("edit", "edit_file", &edit_args, &no_agent_files, wd).unwrap().level, 2);
    }

    #[test]
    fn test_gate_auto_mode_classification() {
        let tmp = tempfile::tempdir().unwrap();
        let no_files = crate::write_guard::AgentFiles::new();
        // 新建文件（不存在）→ level 0 安全直行
        let new_file = serde_json::json!({"path": "not_exists_zz.md", "content": "x"});
        assert_eq!(classify_gate("auto", "write_file", &new_file, &no_files, tmp.path()).unwrap().level, 0);
        // 覆盖既有用户文件 → level 2 强制授权
        std::fs::write(tmp.path().join("user.md"), "用户数据").unwrap();
        let user_path = tmp.path().join("user.md").to_str().unwrap().to_string();
        let overwrite = serde_json::json!({"path": user_path, "content": "x"});
        assert_eq!(classify_gate("auto", "write_file", &overwrite, &no_files, tmp.path()).unwrap().level, 2, "覆盖用户既有文件必须人工授权");
        // 智能体本会话自有产物（agent_files 命中）→ level 0 直行
        let agent_files = crate::write_guard::AgentFiles::new();
        agent_files.insert(&user_path);
        assert_eq!(classify_gate("auto", "edit_file", &overwrite, &agent_files, tmp.path()).unwrap().level, 0);
        // 无害命令 → level 1（auto 走 LLM 安全自评）；危险命令 → level 2
        assert_eq!(classify_gate("auto", "bash", &serde_json::json!({"command": "date"}), &no_files, tmp.path()).unwrap().level, 1);
        assert_eq!(classify_gate("auto", "bash", &serde_json::json!({"command": "rm -rf C:\\x"}), &no_files, tmp.path()).unwrap().level, 2);
    }

    // ─── 自定义网关（newapi）配置链路端到端自测 ───
    // 复现用户报告的缺陷形态：拉模型列表时补了 /v1（成功），实际聊天却不补（无回应）。
    // 用真实 HTTP mock 网关锁死整条链路：llm.toml → load_providers → build_provider → 聊天请求路径。

    /// 单连接 mock OpenAI 兼容网关：记录请求行（取首行），返回固定回复
    fn spawn_gateway_mock() -> (std::net::SocketAddr, std::sync::Arc<std::sync::Mutex<String>>) {
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
        std::env::set_var("no_proxy", "127.0.0.1,localhost");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                *seen2.lock().unwrap() = req.lines().next().unwrap_or_default().to_string();
                let body = r#"{"choices":[{"message":{"content":"pong"},"finish_reason":"stop"}]}"#;
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        (addr, seen)
    }

    /// 在临时目录搭 {work_dir}/config/llm.toml（网关配置）并加载 Provider
    fn load_test_providers(work_dir: &std::path::Path, addr: &std::net::SocketAddr, is_full_url: bool) -> AppContext {
        std::fs::create_dir_all(work_dir.join("config")).unwrap();
        std::fs::create_dir_all(work_dir.join("UserData")).unwrap();
        let cfg = format!(
            r#"
[agent]
default_provider = "newapi"

[providers.newapi]
api_key = "sk-test"
base_url = "http://{}"
default_model = "test-model"
is_full_url = {}
"#,
            addr, is_full_url
        );
        std::fs::write(work_dir.join("config/llm.toml"), cfg).unwrap();
        load_providers(work_dir.join("UserData"))
    }

    // ── 子代理写文件 端到端（真实 HTTP mock 网关 + 真实 write_file 执行）──
    // 只测 resolve_sub_agent_write 不够：那只证明"判定函数"对，
    // 证明不了 判定 → 闸门 → 工具执行 → 落盘 → 登记产物 整条链真的通。

    /// 多轮 mock 网关：第 1 轮返回 write_file 工具调用，第 2 轮返回收尾文本
    fn spawn_coder_mock() -> std::net::SocketAddr {
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
        std::env::set_var("no_proxy", "127.0.0.1,localhost");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let bodies = [
                r#"{"choices":[{"message":{"content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"write_file","arguments":"{\"path\":\"made_by_coder.txt\",\"content\":\"code-by-subagent\"}"}}]},"finish_reason":"tool_calls"}]}"#,
                r#"{"choices":[{"message":{"content":"done"},"finish_reason":"stop"}]}"#,
            ];
            for body in bodies {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buf = [0u8; 16384];
                    let _ = stream.read(&mut buf);
                    let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                }
            }
        });
        addr
    }

    /// 只返回文本（不调工具）的 mock：用于验证只读代理不会写文件
    fn spawn_noop_mock() -> std::net::SocketAddr {
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let body = r#"{"choices":[{"message":{"content":"只读分析结论"},"finish_reason":"stop"}]}"#;
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf);
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        addr
    }

    fn sub_agent_call(agent_type: &str, mode: &str) -> skyland_lib::llm::provider::ToolCall {
        let args = serde_json::json!({"name": "coder", "task": "写一个文件", "subagent_type": agent_type, "mode": mode}).to_string();
        skyland_lib::llm::provider::ToolCall { id: "call_x".into(), name: "spawn_agent".into(), arguments: args }
    }

    #[tokio::test]
    async fn coder_readwrite_actually_writes_file_end_to_end() {
        let addr = spawn_coder_mock();
        let tmp = tempfile::tempdir().unwrap();
        let ctx = load_test_providers(tmp.path(), &addr, true);
        let provider = skyland_lib::llm::openai::OpenAiProvider::clone(ctx.providers.get("test-model").expect("模型应已加载"));
        let wd = tmp.path().join("ws");
        std::fs::create_dir_all(&wd).unwrap();
        let files = crate::write_guard::AgentFiles::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<axum::response::sse::Event, std::convert::Infallible>>(64);
        // 收空并记录事件：既避免 channel 填满阻塞，也能断言"没弹授权窗口"
        let seen_events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen2 = seen_events.clone();
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if let Ok(v) = ev {
                    seen2.lock().unwrap().push(format!("{:?}", v));
                }
            }
        });

        let (_id, result) = run_sub_agent(
            provider,
            sub_agent_call("coder", "readwrite"),
            false,
            Some(wd.clone()),
            wd.clone(),
            tx,
            std::sync::Arc::new(tokio::sync::Semaphore::new(3)),
            Vec::new(),
            files.clone(),
            "conv-e2e".into(),
            "auto".into(),
            None,
        ).await;

        let written = wd.join("made_by_coder.txt");
        let events = seen_events.lock().unwrap().clone();
        assert!(written.exists(),
            "coder 子代理应真实落盘，实际未写出；结果={}；事件={:?}", result, events);
        assert_eq!(std::fs::read_to_string(&written).unwrap(), "code-by-subagent");
        assert!(files.contains("made_by_coder.txt"),
            "产物应登记进会话集合，后续编辑才是 level 0 而非反复弹授权");
    }

    #[tokio::test]
    async fn readonly_sub_agent_never_receives_write_tools() {
        // 只读代理的工具列表里不得出现任何写工具 —— 这是第一道防线（工具下发层）
        let readwrite = tools_for_mode("readonly");
        assert!(!readwrite.iter().any(|t| WRITABLE_SUB_AGENT_TOOLS.contains(&t.name.as_str())),
            "只读工具集里不得含写工具");
        let (ok, _) = resolve_sub_agent_write("coder", "readonly", "auto",
            &["read_file".to_string(), "write_file".to_string()]);
        assert!(!ok, "未声明 readwrite 时即便类型是 coder 也不得写");
    }

    #[tokio::test]
    async fn test_full_chain_newapi_bare_host_chat() {
        // newapi 典型填法：裸域名无路径 + 未勾选完整URL → 聊天必须自动补 /v1 且收到正确回复
        let (addr, seen) = spawn_gateway_mock();
        let tmp = tempfile::tempdir().unwrap();
        let ctx = load_test_providers(tmp.path(), &addr, false);
        let provider = ctx.providers.get("test-model").expect("配置的模型应已加载");
        let reply = provider.chat(&[Message::user("ping")], &[]).await.expect("聊天应成功收到回应");
        assert_eq!(reply.content, "pong");
        let line = seen.lock().unwrap().clone();
        assert!(line.starts_with("POST /v1/chat/completions "),
            "裸主机未自动补 /v1 — 复现「配置成功但发消息无回应」缺陷，实际请求行: {}", line);
    }

    #[tokio::test]
    async fn test_full_chain_full_url_mode_no_v1_injection() {
        // 勾选完整URL → is_full_url 持久化生效：聊天不得自动补 /v1（勾选框真实语义）
        let (addr, seen) = spawn_gateway_mock();
        let tmp = tempfile::tempdir().unwrap();
        let ctx = load_test_providers(tmp.path(), &addr, true);
        let provider = ctx.providers.get("test-model").expect("配置的模型应已加载");
        let reply = provider.chat(&[Message::user("ping")], &[]).await.expect("聊天应成功收到回应");
        assert_eq!(reply.content, "pong");
        let line = seen.lock().unwrap().clone();
        assert!(line.starts_with("POST /chat/completions "),
            "完整URL模式下不得注入 /v1，实际请求行: {}", line);
    }

    // ─── Provider 保存/编辑/删除生命周期（save_provider_to / delete_provider_from 纯逻辑）───
    fn save_req(name: &str, key: &str, url: &str, model: &str, original: Option<&str>) -> ProviderSaveRequest {
        ProviderSaveRequest {
            name: name.into(), api_key: key.into(), base_url: url.into(),
            default_model: model.into(), context_length: None, is_full_url: Some(false),
            original_key: original.map(String::from),
        }
    }
    fn read_toml(path: &std::path::Path) -> toml::Value {
        std::fs::read_to_string(path).unwrap().parse().unwrap()
    }

    #[test]
    fn test_provider_save_edit_delete_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg_path = tmp.path().join("llm.toml");

        // 1. 新建 → key 生成 + 设为默认
        let r = save_provider_to(&cfg_path, &save_req("DeepSeek", "sk-111", "https://api.deepseek.com", "deepseek-flash", None)).unwrap();
        assert_eq!(r["provider_key"], "DeepSeek");
        assert_eq!(r["restart_required"], true);
        let v = read_toml(&cfg_path);
        assert_eq!(v["providers"]["DeepSeek"]["api_key"].as_str(), Some("sk-111"));
        assert_eq!(v["agent"]["default_provider"].as_str(), Some("DeepSeek"));

        // 2. 重复新建同名 → 409 拒绝，原配置不被静默覆盖
        let err = save_provider_to(&cfg_path, &save_req("DeepSeek", "sk-222", "https://x.com", "m2", None)).unwrap_err();
        assert_eq!(err.0, axum::http::StatusCode::CONFLICT);
        assert_eq!(read_toml(&cfg_path)["providers"]["DeepSeek"]["api_key"].as_str(), Some("sk-111"));

        // 3. 编辑：api_key 留空保留原值；URL/模型/完整URL 开关更新；context_length 写入
        let mut req = save_req("DeepSeek", "", "https://api.deepseek.com/v1", "deepseek-v4-pro", Some("DeepSeek"));
        req.context_length = Some(65536);
        req.is_full_url = Some(true);
        save_provider_to(&cfg_path, &req).unwrap();
        let v = read_toml(&cfg_path);
        assert_eq!(v["providers"]["DeepSeek"]["api_key"].as_str(), Some("sk-111"), "留空密钥必须保留原值");
        assert_eq!(v["providers"]["DeepSeek"]["base_url"].as_str(), Some("https://api.deepseek.com/v1"));
        assert_eq!(v["providers"]["DeepSeek"]["default_model"].as_str(), Some("deepseek-v4-pro"));
        assert_eq!(v["providers"]["DeepSeek"]["is_full_url"].as_bool(), Some(true));
        assert_eq!(v["providers"]["DeepSeek"]["context_length"].as_integer(), Some(65536));
        assert_eq!(v["agent"]["default_provider"].as_str(), Some("DeepSeek"), "编辑不得切换默认");

        // 3b. 再次编辑不传 context_length → 原值保留（表单未暴露的字段不得被静默重置）
        let mut req2 = save_req("DeepSeek", "", "https://api.deepseek.com/v1", "deepseek-flash", Some("DeepSeek"));
        req2.context_length = None;
        save_provider_to(&cfg_path, &req2).unwrap();
        assert_eq!(read_toml(&cfg_path)["providers"]["DeepSeek"]["context_length"].as_integer(), Some(65536));

        // 4. 编辑改名（中文名）→ 旧 key 移除、默认指向随改名迁移
        save_provider_to(&cfg_path, &save_req("DeepSeek主推", "sk-333", "https://api.deepseek.com", "deepseek-flash", Some("DeepSeek"))).unwrap();
        let v = read_toml(&cfg_path);
        assert!(v["providers"].get("DeepSeek").is_none(), "改名后旧 key 必须移除");
        assert_eq!(v["providers"]["DeepSeek主推"]["api_key"].as_str(), Some("sk-333"));
        assert_eq!(v["agent"]["default_provider"].as_str(), Some("DeepSeek主推"), "默认指向需随改名迁移");

        // 5. 再建一个非默认用途配置（新建立即成为默认）
        save_provider_to(&cfg_path, &save_req("newapi", "sk-9", "https://gw.example.com", "gpt-x", None)).unwrap();
        assert_eq!(read_toml(&cfg_path)["agent"]["default_provider"].as_str(), Some("newapi"));

        // 6. 删除默认配置 → 默认迁移到剩余第一个（字母序）
        let r = delete_provider_from(&cfg_path, "newapi").unwrap();
        assert_eq!(r["default_provider"], "DeepSeek主推");
        assert_eq!(read_toml(&cfg_path)["agent"]["default_provider"].as_str(), Some("DeepSeek主推"));

        // 7. 删除不存在的配置 → 404
        assert_eq!(delete_provider_from(&cfg_path, "ghost").unwrap_err().0, axum::http::StatusCode::NOT_FOUND);

        // 8. 新建缺 api_key → 400；编辑不存在的 key → 404
        assert_eq!(save_provider_to(&cfg_path, &save_req("x", "", "https://x.com", "m", None)).unwrap_err().0, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(save_provider_to(&cfg_path, &save_req("x", "k", "https://x.com", "m", Some("ghost"))).unwrap_err().0, axum::http::StatusCode::NOT_FOUND);

        // 9. sanitize：全符号名 → custom；空白名 → 400
        let r = save_provider_to(&cfg_path, &save_req("!!!", "k", "https://x.com", "m", None)).unwrap();
        assert_eq!(r["provider_key"], "custom");
        assert_eq!(save_provider_to(&cfg_path, &save_req("  ", "k", "https://x.com", "m", None)).unwrap_err().0, axum::http::StatusCode::BAD_REQUEST);

        // 10. 幽灵默认治理（真实事故形态：default_provider 曾指向 sanitize 脱节的 "____________"）：
        //     默认指向不存在的 key 时，编辑保存自动修正为本次配置
        let mut v = read_toml(&cfg_path);
        if let Some(agent) = v.as_table_mut().and_then(|t| t.get_mut("agent")).and_then(|a| a.as_table_mut()) {
            agent.insert("default_provider".into(), toml::Value::String("ghost_key".into()));
        }
        std::fs::write(&cfg_path, toml::to_string(&v).unwrap()).unwrap();
        save_provider_to(&cfg_path, &save_req("DeepSeek主推", "", "https://api.deepseek.com", "deepseek-flash", Some("DeepSeek主推"))).unwrap();
        assert_eq!(read_toml(&cfg_path)["agent"]["default_provider"].as_str(), Some("DeepSeek主推"),
            "编辑时幽灵默认必须被修正为本次配置");
        assert_eq!(read_toml(&cfg_path)["providers"]["DeepSeek主推"]["api_key"].as_str(), Some("sk-333"), "治理路径中密钥仍保留");

        // 11. 删除时幽灵默认同样治理：删除任一配置后默认修正为剩余第一个
        let mut v = read_toml(&cfg_path);
        if let Some(agent) = v.as_table_mut().and_then(|t| t.get_mut("agent")).and_then(|a| a.as_table_mut()) {
            agent.insert("default_provider".into(), toml::Value::String("ghost_2".into()));
        }
        std::fs::write(&cfg_path, toml::to_string(&v).unwrap()).unwrap();
        delete_provider_from(&cfg_path, "custom").unwrap();
        assert_eq!(read_toml(&cfg_path)["agent"]["default_provider"].as_str(), Some("DeepSeek主推"),
            "删除后幽灵默认必须被修正为剩余第一个");
    }
}

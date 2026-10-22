//! 对话联动工具 — AI 读写知识库与待办速记
//! 分发：api/mod.rs 对话处理器拦截 is_chat_tool → execute_chat_tool；
//! 其余工具仍走 skyland_lib::llm::tool_executor（lib 层不感知业务工具）。
//! server 模式下待办按 CurrentUser 隔离（todo_dir_for_user），KB 全局只读。

use serde::{Deserialize, Serialize};

use crate::api::AppContext;
use crate::auth::CurrentUser;
use crate::todo_api as tapi;
use skyland_lib::llm::tool::{Tool, ToolResult};
use skyland_lib::llm::provider::ToolCall;

/// 速记语法解析结果
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct ParsedQuick {
    pub title: String,
    pub category: String,
    pub priority: u8, // 0=低 1=中 2=高
    pub notify_times: Vec<String>,
    pub notify_enabled: bool,
    /// `@YYYY-MM-DD` / `@MM-DD`（当年）日期 token → 条目落到该日期清单（改期/复习语义）
    pub date: Option<String>,
}

/// 速记语法解析（Rust 版，与前端 _parseQuickSyntax 对齐）：
/// - `@HH:MM` 提醒时间，可多个，小时>23 视为普通文本保留
/// - `@YYYY-MM-DD` / `@MM-DD`（当年）目标日期，首个生效；非法日期保留原文
/// - `#分类` 仅首个生效，其余保留在标题
/// - `!!`/`!` 必须在剩余文本前缀（与前端约定一致），中部视为标题内容
pub fn parse_quick_syntax(raw: &str) -> ParsedQuick {
    let mut text = raw.trim().to_string();
    let mut times: Vec<String> = Vec::new();
    let mut target_date: Option<String> = None;

    // @日期：先长格式（YYYY-MM-DD）再短格式（MM-DD 补当前年）——
    // 均先经 NaiveDate 严格校验（2月30日等非法日期保留原文），首个生效、其余移除
    let take_date = |m: &str, y: i32, mo: u32, d: u32, target: &mut Option<String>| -> String {
        let iso = format!("{:04}-{:02}-{:02}", y, mo, d);
        if chrono::NaiveDate::parse_from_str(&iso, "%Y-%m-%d").is_ok() {
            if target.is_none() { *target = Some(iso); }
            String::new()
        } else { m.to_string() }
    };
    let re_date_full = regex::Regex::new(r"@(\d{4})-(\d{1,2})-(\d{1,2})").unwrap();
    text = re_date_full.replace_all(&text, |c: &regex::Captures| {
        take_date(c.get(0).map(|m| m.as_str()).unwrap_or(""), c[1].parse().unwrap_or(0),
            c[2].parse().unwrap_or(0), c[3].parse().unwrap_or(0), &mut target_date)
    }).to_string();
    let re_date_short = regex::Regex::new(r"@(\d{1,2})-(\d{1,2})").unwrap();
    let year_now: i32 = tapi::today_str().split('-').next().and_then(|y| y.parse().ok()).unwrap_or(2026);
    text = re_date_short.replace_all(&text, |c: &regex::Captures| {
        take_date(c.get(0).map(|m| m.as_str()).unwrap_or(""), year_now,
            c[1].parse().unwrap_or(0), c[2].parse().unwrap_or(0), &mut target_date)
    }).to_string();

    // @HH:MM（全局扫描，逐个移除）
    let re_time = regex::Regex::new(r"@(\d{1,2}):([0-5]\d)").unwrap();
    for cap in re_time.captures_iter(&text.clone()) {
        let h_str = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let m_str = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        let h: u32 = h_str.parse().unwrap_or(99);
        if h <= 23 {
            times.push(format!("{:0>2}:{}", h_str, m_str));
        }
    }
    if !times.is_empty() {
        text = re_time.replace_all(&text, |caps: &regex::Captures| {
            let h: u32 = caps[1].parse().unwrap_or(99);
            if h <= 23 { String::new() } else { caps[0].to_string() } // 无效时间保留原文
        }).to_string();
    }

    // #分类（仅首个）
    let re_tag = regex::Regex::new(r"#([^\s#@]+)").unwrap();
    let mut category = String::new();
    if let Some(m) = re_tag.find(&text) {
        category = m.as_str()[1..].to_string();
        text.replace_range(m.range(), "");
    }

    // !!/! 优先级前缀
    let mut priority = 1u8;
    text = text.trim_start().to_string(); // 分类在前的输入（#工作 !! 交周报）移除标签后残留前导空格
    let re_pri = regex::Regex::new(r"^(!{1,2})\s*").unwrap();
    if let Some(caps) = re_pri.captures(&text) {
        priority = if caps[1].len() == 2 { 2 } else { 0 };
        text = re_pri.replace(&text, "").to_string();
    }

    // 压缩连续空白
    let re_ws = regex::Regex::new(r"\s+").unwrap();
    let title = re_ws.replace_all(text.trim(), " ").trim().to_string();

    ParsedQuick {
        title,
        category,
        priority,
        notify_enabled: !times.is_empty(),
        notify_times: times,
        date: target_date,
    }
}

// ─── 工具定义 ───

pub const CHAT_TOOL_NAMES: &[&str] = &["kb_list", "kb_search", "kb_read", "todo_read", "todo_create", "use_skill", "create_task", "kb_create_kb", "kb_create_doc", "kb_delete_doc", "kb_edit_doc", "generate_image"];

pub fn is_chat_tool(name: &str) -> bool {
    CHAT_TOOL_NAMES.contains(&name)
}

/// 工具执行结果：直接完成，或需要用户审批（覆盖/删除用户文档）
pub enum ChatOutcome {
    Done(ToolResult),
    /// 覆盖/删除用户文档 — 携带审批 token 与 oneshot；kb_delete_doc 的用户分支也走这里
    Approval { token: String, rx: tokio::sync::oneshot::Receiver<crate::approval::Decision>, action: String, target: String, call_id: String, kb: String, rel: String },
}

/// 供调度器调用：可产生审批挂起
pub async fn route_chat_tool(
    ctx: &AppContext,
    user: Option<&CurrentUser>,
    tc: &ToolCall,
    allow_write: bool,
) -> ChatOutcome {
    if (tc.name == "todo_create" || tc.name == "create_task" || tc.name == "generate_image") && !allow_write {
        return ChatOutcome::Done(ToolResult {
            tool_call_id: tc.id.clone(),
            output: "⛔ 当前执行模式（计划/只读）不允许此写入操作".into(),
            success: false,
        });
    }
    if tc.name == "kb_edit_doc" {
        // 全模式可用；用户文档经审批条放行（审批即授权，优先级高于模式门禁）
        return kb_edit_doc(ctx, user, &tc.arguments, tc.id.clone()).await;
    }
    if (tc.name == "kb_create_kb" || tc.name == "kb_create_doc" || tc.name == "kb_delete_doc") && !allow_write {
        return ChatOutcome::Done(ToolResult {
            tool_call_id: tc.id.clone(),
            output: "⛔ 当前执行模式（计划/只读）不允许修改知识库结构".into(),
            success: false,
        });
    }
    let result = match tc.name.as_str() {
        "kb_create_kb" => kb_create_kb(ctx, user, &tc.arguments),
        "kb_create_doc" => kb_create_doc(ctx, user, &tc.arguments),
        "kb_delete_doc" => match kb_delete_doc(ctx, user, &tc.arguments, tc.id.clone()).await {
            ChatOutcome::Done(r) => r,
            ChatOutcome::Approval { call_id, action, .. } => tool_err(&call_id,
                &format!("此操作需要用户在对话中实时审批（{} 用户文档），定时任务无法完成", action)),
        },
        _ => return match execute_chat_tool_inner(ctx, user, tc, allow_write).await {
            Ok(r) => ChatOutcome::Done(r),
            Err(msg) => ChatOutcome::Done(ToolResult { tool_call_id: tc.id.clone(), output: msg, success: false }),
        },
    };
    ChatOutcome::Done(result)
}

async fn execute_chat_tool_inner(ctx: &AppContext, user: Option<&CurrentUser>, tc: &ToolCall, allow_write: bool) -> Result<ToolResult, String> {
    Ok(execute_chat_tool(ctx, user, tc, allow_write).await)
}

/// 定时任务执行上下文（execute_chat_tool 直分发）可用的工具名 — 调度器装配工具集时过滤用。
/// kb_* 写工具在任务上下文没有审批/分发路径（route_chat_tool 的 Approval 分支不可达），
/// 装配了也只是让模型调用必失败的诱饵。
pub fn task_dispatchable(name: &str) -> bool {
    matches!(name, "kb_list" | "kb_search" | "kb_read" | "todo_read" | "todo_create" | "use_skill" | "create_task" | "generate_image")
}

/// 按执行模式返回联动工具集（plan/readonly 无写工具）
pub fn tools_for_mode(mode: &str) -> Vec<Tool> {
    let mut tools = vec![
        Tool::new("kb_list", "列出用户的所有知识库（技术知识库）。想查资料但不知道有哪些库时先调用它。",
            serde_json::json!({"type": "object", "properties": {}, "required": []})),
        Tool::new("kb_search", "在知识库中全文搜索文档。返回命中文档路径、片段与命中数。kb 省略时搜索全部知识库。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "搜索关键词" },
                    "kb": { "type": "string", "description": "知识库名称（可选，省略=全部）" }
                },
                "required": ["query"]
            })),
        Tool::new("kb_read", "读取知识库中的指定文档（Markdown 全文，超长自动截断）。kb+path 来自 kb_search/kb_list 的结果。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string", "description": "知识库名称" },
                    "path": { "type": "string", "description": "文档相对路径，如 子目录/文档.md" }
                },
                "required": ["kb", "path"]
            })),
        Tool::new("todo_read", "读取用户的待办与速记。scope=today 返回今日待办+逾期项+统计；scope=recent 返回最近 7 天。用户问“我今天有什么事/最近记了什么”时调用。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "enum": ["today", "recent"], "description": "默认 today" }
                },
                "required": []
            })),
        Tool::new("use_skill", "加载一个技能的完整指令并遵循执行。技能清单见系统提示。用户说「写日报/会议纪要/晨间简报/学习笔记」等技能相关请求时调用。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "技能名（见系统提示的可用技能清单）" }
                },
                "required": ["name"]
            })),
        Tool::new("kb_edit_doc", "编辑（覆盖）知识库中已存在的文档。你创建的文档可直接改；用户创建的文档会触发审批条，经用户允许后完成。适合帮用户撰写/修改文章。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string", "description": "知识库名称" },
                    "path": { "type": "string", "description": "文档相对路径（须已存在；不存在则自动创建，所有权归你）" },
                    "content": { "type": "string", "description": "完整的新文档内容（Markdown）" }
                },
                "required": ["kb", "path", "content"]
            })),
    ];
    if mode == "auto" || mode == "edit" {
        tools.push(Tool::new("kb_create_kb", "创建一个新的知识库。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "知识库名称" }
                },
                "required": ["name"]
            })));
        tools.push(Tool::new("kb_create_doc", "在知识库中创建新文档（Markdown）。所有权归你（智能体），后续修改无需审批。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string", "description": "知识库名称（不存在则自动创建）" },
                    "path": { "type": "string", "description": "文档相对路径，如 报告/周报.md" },
                    "content": { "type": "string", "description": "完整文档内容（Markdown）" }
                },
                "required": ["kb", "path", "content"]
            })));
        tools.push(Tool::new("kb_delete_doc", "删除知识库文档。你创建的文档可直接删；用户创建的文档会触发审批条。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string", "description": "知识库名称" },
                    "path": { "type": "string", "description": "文档相对路径" }
                },
                "required": ["kb", "path"]
            })));
        tools.push(Tool::new("create_task", "创建定时任务（待时任务）。到点后系统会用 AI 自动完整执行 prompt，结果写入任务会话并通知用户。schedule_type: once(at=Unix秒)/daily(time)/weekly(weekdays 1-7+time)/interval(minutes)。用户说「每天早上八点给我简报/每周一总结/半小时后执行X」时调用。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "任务名（60 字内）" },
                    "prompt": { "type": "string", "description": "到点后交给 AI 的执行指令" },
                    "skill": { "type": "string", "description": "关联技能名（可选）" },
                    "schedule_type": { "type": "string", "enum": ["once", "daily", "weekly", "interval"] },
                    "at": { "type": "integer", "description": "once 模式：Unix 秒级时间戳" },
                    "time": { "type": "string", "description": "daily/weekly 模式：HH:MM" },
                    "weekdays": { "type": "array", "items": {"type": "integer"}, "description": "weekly 模式：星期几 1=周一..7=周日" },
                    "minutes": { "type": "integer", "description": "interval 模式：间隔分钟数" }
                },
                "required": ["name", "prompt", "schedule_type"]
            })));
        tools.push(Tool::new("todo_create", "为用户创建一条待办/速记。text 使用速记语法：`#分类`（可省）、`!!`或`!` 开头设高/低优先级（可省）、`@HH:MM` 设置提醒时间（可多个）、`@YYYY-MM-DD` 或 `@MM-DD`（当年）指定目标日期（条目落到那天清单，改期/复习/未来计划用）、其余为标题。例：\"#工作 !! 交周报 @09:00\"、\"#学习 复习记忆卡 @2026-09-25\"。用户说“帮我记一下/加个待办/提醒我”时调用。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "速记语法文本" },
                    "date": { "type": "string", "description": "目标日期 YYYY-MM-DD（可选，默认今天）" }
                },
                "required": ["text"]
            })));
    }
    // AI 生图 — auto/edit/full 三模式可用（计划/只读禁生成）。原生管线自动落盘
    // 生图目录并在会话内出图，取代旧"bash 手动 curl 图片 API"的脆弱工作流。
    if mode == "auto" || mode == "edit" || mode == "full" {
        tools.push(Tool::new("generate_image", "AI 文生图（SKYLAND 内置生图引擎）：调用系统已配置的图片服务商生成图片，自动保存到生图目录并在对话中展示。用户要求画图/生成图片/做配图/出海报时必须优先用本工具，禁止用 bash 手动调用图片 API。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "画面描述（主体/风格/构图/光线，越具体越好）" },
                    "provider": { "type": "string", "description": "图片服务商名（可选，默认已配置的第一个）" },
                    "size": { "type": "string", "description": "尺寸如 1024x1024（可选）" }
                },
                "required": ["prompt"]
            })));
    }
    tools
}

/// 执行联动工具。allow_write=false（plan/readonly 模式）时拒绝 todo_create
/// —— 模式门禁不依赖"工具是否被 offer"，模型越权调用也会被拒。
pub async fn execute_chat_tool(
    ctx: &AppContext,
    user: Option<&CurrentUser>,
    tc: &ToolCall,
    allow_write: bool,
) -> ToolResult {
    if (tc.name == "todo_create" || tc.name == "create_task") && !allow_write {
        return ToolResult {
            tool_call_id: tc.id.clone(),
            output: "⛔ 当前执行模式（计划/只读）不允许创建待办".into(),
            success: false,
        };
    }
    if tc.name == "generate_image" && !allow_write {
        return ToolResult {
            tool_call_id: tc.id.clone(),
            output: "⛔ 当前执行模式（计划/只读）不允许生成图片".into(),
            success: false,
        };
    }
    let mut result = match tc.name.as_str() {
        "kb_list" => kb_list(ctx, user),
        "kb_search" => kb_search(ctx, user, &tc.arguments),
        "kb_read" => kb_read(ctx, user, &tc.arguments),
        "todo_read" => todo_read(ctx, user, &tc.arguments),
        "todo_create" => todo_create(ctx, user, &tc.arguments).await,
        "use_skill" => use_skill(ctx, user, &tc.arguments),
        "create_task" => create_task(ctx, user, &tc.arguments),
        "generate_image" => generate_image(ctx, user, &tc.arguments).await,
        _ => return ToolResult {
            tool_call_id: tc.id.clone(),
            output: format!("未知联动工具: {}", tc.name),
            success: false,
        },
    };
    result.tool_call_id = tc.id.clone();
    result
}

fn tool_err(tc_id: &str, msg: impl Into<String>) -> ToolResult {
    ToolResult { tool_call_id: tc_id.into(), output: msg.into(), success: false }
}

// ─── AI 生图（智能体原生工具）───

/// generate_image 工具实现：复用生图核心链（校验/重试/落盘/元数据），
/// 输出包含 /api/image/file/{name} 访问地址供 mod.rs 提取 file_output 事件。
async fn generate_image(ctx: &AppContext, user: Option<&CurrentUser>, args: &str) -> ToolResult {
    let v: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v,
        Err(e) => return tool_err("", format!("参数解析失败: {}", e)),
    };
    let prompt = v["prompt"].as_str().unwrap_or("").trim().to_string();
    if prompt.is_empty() {
        return tool_err("", "缺少 prompt 参数（画面描述）");
    }
    let provider = v["provider"].as_str().map(|s| s.trim()).filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(crate::image_api::default_image_provider);
    let Some(provider) = provider else {
        return tool_err("", "尚未配置任何图片服务商 — 请先在「AI 生图」页完成服务商配置（API Key / Base URL / 模型）");
    };
    let req = crate::image_api::ImageGenRequest {
        provider,
        prompt,
        size: v["size"].as_str().map(String::from),
        quality: None,
        style: None,
        reference_image: None,
        n: None,
        negative_prompt: None,
        seed: None,
        mode: None,
    };
    match crate::image_api::generate_image_core_for(ctx, user, &req).await {
        Ok((filename, url, message)) => {
            let extra = message.map(|m| format!("（{}）", m)).unwrap_or_default();
            ToolResult {
                tool_call_id: String::new(),
                output: format!("图片已生成 {}（已保存到生图目录 ImgGenerator/）。访问地址：{}{}", filename, url, extra),
                success: true,
            }
        }
        Err((_, msg)) => tool_err("", format!("生图失败：{}", msg)),
    }
}

// ─── 知识库工具 ───

fn kb_list(ctx: &AppContext, user: Option<&CurrentUser>) -> ToolResult {
    let mut kbs: Vec<(String, usize)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(crate::api::kb::kb_root(ctx, user)) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !path.is_dir() || name.starts_with('.') { continue; }
            kbs.push((name, crate::api::kb::count_docs(&path)));
        }
    }
    if kbs.is_empty() {
        return ToolResult { tool_call_id: String::new(), output: "暂无知识库".into(), success: true };
    }
    kbs.sort();
    let out = kbs.into_iter()
        .map(|(n, c)| format!("{}（{} 篇文档）", n, c))
        .collect::<Vec<_>>()
        .join("\n");
    ToolResult { tool_call_id: String::new(), output: out, success: true }
}

fn kb_search(ctx: &AppContext, user: Option<&CurrentUser>, args: &str) -> ToolResult {
    let v: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v,
        Err(e) => return tool_err("", format!("参数解析失败: {}", e)),
    };
    let query = v["query"].as_str().unwrap_or("").trim().to_string();
    if query.is_empty() {
        return tool_err("", "缺少 query 参数");
    }
    let kb_filter = v["kb"].as_str().map(|s| s.to_string());

    // 目标 KB 列表：指定 kb 或全部
    let mut targets: Vec<String> = Vec::new();
    match &kb_filter {
        Some(kb) => match crate::api::kb::sanitize_name(kb) {
            Some(k) => targets.push(k),
            None => return tool_err("", format!("非法知识库名: {}", kb)),
        },
        None => {
            if let Ok(entries) = std::fs::read_dir(crate::api::kb::kb_root(ctx, user)) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if entry.path().is_dir() && !name.starts_with('.') {
                        targets.push(name);
                    }
                }
            }
            targets.sort();
        }
    }

    let mut lines: Vec<String> = Vec::new();
    let mut total = 0usize;
    for kb in &targets {
        let dir = crate::api::kb::kb_root(ctx, user).join(kb);
        let mut results: Vec<crate::api::kb::SearchResult> = Vec::new();
        crate::api::kb::search_in_dir(&dir, "", &query, &mut results);
        for r in results.iter().take(10) {
            total += 1;
            lines.push(format!("[{}] {} · 命中{}次\n  {}", kb, r.path, r.matches, r.snippet));
            if total >= 20 { break; }
        }
        if total >= 20 { break; }
    }
    if lines.is_empty() {
        ToolResult { tool_call_id: String::new(), output: format!("未找到包含“{}”的文档", query), success: true }
    } else {
        let out = format!("共 {} 处命中：\n{}", total, lines.join("\n"));
        ToolResult { tool_call_id: String::new(), output: out, success: true }
    }
}

const KB_READ_MAX_CHARS: usize = 8192;

fn kb_read(ctx: &AppContext, user: Option<&CurrentUser>, args: &str) -> ToolResult {
    let v: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v,
        Err(e) => return tool_err("", format!("参数解析失败: {}", e)),
    };
    let Some(kb) = crate::api::kb::sanitize_name(v["kb"].as_str().unwrap_or("")) else {
        return tool_err("", "非法知识库名");
    };
    let Some(rel) = crate::api::kb::sanitize_path(v["path"].as_str().unwrap_or("")) else {
        return tool_err("", "非法文档路径（禁止 .. 与绝对路径）");
    };
    if !rel.to_lowercase().ends_with(".md") {
        return tool_err("", "仅支持读取 .md 文档");
    }
    let path = crate::api::kb::kb_root(ctx, user).join(&kb).join(&rel);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let char_count = content.chars().count();
            let body = if char_count > KB_READ_MAX_CHARS {
                let head: String = content.chars().take(KB_READ_MAX_CHARS).collect();
                format!("{}\n\n... [已截断，全文共 {} 字符]", head, char_count)
            } else {
                content
            };
            ToolResult { tool_call_id: String::new(), output: body, success: true }
        }
        Err(e) => tool_err("", format!("读取失败 {}: {}", rel, e)),
    }
}

// ─── 待办工具 ───

fn format_item_line(item: &tapi::TodoItem) -> String {
    let status = if item.done { "✓" } else if item.expired { "✗逾期" } else { "○" };
    let pri = match item.priority { 0 => "!低", 2 => "!!高", _ => "" };
    let cat = item.category.as_deref().map(|c| format!(" #{}", c)).unwrap_or_default();
    let due = match (&item.due_date, &item.due_time) {
        (Some(d), Some(t)) => format!(" ⏰{} {}", d, t),
        (Some(d), None) => format!(" ⏰{}", d),
        _ => String::new(),
    };
    let notify = if item.notify_enabled && !item.notify_times.is_empty() {
        format!(" 🔔{}", item.notify_times.join(","))
    } else {
        String::new()
    };
    format!("{} {}{}{}{}{}", status, pri, item.title, cat, due, notify)
}

fn todo_read(ctx: &AppContext, user: Option<&CurrentUser>, args: &str) -> ToolResult {
    let v: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::json!({}));
    let scope = v["scope"].as_str().unwrap_or("today");
    let dir = tapi::todo_dir_for_user(ctx, user);
    let mut out = String::new();

    if scope == "recent" {
        let dates = tapi::list_dates(&dir);
        let mut groups = 0;
        for date in dates.iter().take(7) {
            if let Some(list) = tapi::read_list_in(&dir, date) {
                if list.items.is_empty() { continue; }
                out.push_str(&format!("\n【{}】\n", date));
                for item in &list.items {
                    out.push_str(&format!("  {}\n", format_item_line(item)));
                }
                groups += 1;
            }
        }
        if groups == 0 { out.push_str("最近 7 天没有待办记录"); }
    } else {
        let today = tapi::today_str();
        if let Some(list) = tapi::read_list_in(&dir, &today) {
            let done = list.items.iter().filter(|i| i.done).count();
            out.push_str(&format!("今日待办 {}/{}：\n", done, list.items.len()));
            for item in &list.items {
                out.push_str(&format!("  {}\n", format_item_line(item)));
            }
            if list.items.is_empty() { out.push_str("  （今日暂无）\n"); }
        } else {
            out.push_str("今日暂无待办\n");
        }
        // 逾期项（过去日期未完成）
        let mut overdue: Vec<String> = Vec::new();
        for date in tapi::list_dates(&dir) {
            if date >= today { break; }
            if let Some(list) = tapi::read_list_in(&dir, &date) {
                for item in list.items.iter().filter(|i| !i.done) {
                    overdue.push(format!("  [{}] {}", date, format_item_line(item)));
                }
            }
        }
        if !overdue.is_empty() {
            out.push_str(&format!("\n历史未完成（{} 项）：\n{}\n", overdue.len(), overdue.join("\n")));
        }
    }
    ToolResult { tool_call_id: String::new(), output: out, success: true }
}

async fn todo_create(ctx: &AppContext, user: Option<&CurrentUser>, args: &str) -> ToolResult {
    let v: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v,
        Err(e) => return tool_err("", format!("参数解析失败: {}", e)),
    };
    let text = v["text"].as_str().unwrap_or("").trim().to_string();
    if text.is_empty() {
        return tool_err("", "缺少 text 参数（速记语法文本）");
    }
    let parsed = parse_quick_syntax(&text);
    if parsed.title.is_empty() {
        return tool_err("", "解析后标题为空（只含语法标记？）");
    }
    // 目标日期优先级：显式 date 参数 > 速记 @日期 token > 今天
    let date = match v["date"].as_str() {
        Some(d) if !d.trim().is_empty() => match tapi::valid_date(d.trim()) {
            Some(_) => d.trim().to_string(),
            None => return tool_err("", format!("非法日期: {}（需 YYYY-MM-DD）", d)),
        },
        _ => parsed.date.clone().unwrap_or_else(tapi::today_str),
    };

    let dir = tapi::todo_dir_for_user(ctx, user);
    let mut list = tapi::read_list_in(&dir, &date).unwrap_or_else(|| tapi::TodoList {
        date: date.clone(), items: vec![], created: tapi::now_ts(),
    });
    let max_sort = list.items.iter().map(|i| i.sort_order).max().unwrap_or(0);
    let item = tapi::TodoItem {
        id: format!("item_{}_{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0),
            rand::random::<u32>()),
        title: parsed.title.clone(),
        content: String::new(),
        done: false,
        expired: false,
        created: tapi::now_ts(),
        notify_time: parsed.notify_times.first().cloned(),
        notify_enabled: parsed.notify_enabled,
        notify_times: parsed.notify_times.clone(),
        advance_minutes: vec![],
        repeat_enabled: false,
        repeat_interval: 0,
        last_notified: None,
        priority: parsed.priority,
        category: if parsed.category.is_empty() { None } else { Some(parsed.category.clone()) },
        due_date: None,
        due_time: None,
        sort_order: max_sort + 1,
        visibility: "private".into(),
    };
    list.items.push(item);
    tapi::write_list_in(&dir, &date, &list);

    let mut desc = format!("已创建待办：{}", parsed.title);
    if !parsed.category.is_empty() { desc.push_str(&format!(" #{}", parsed.category)); }
    if parsed.priority == 2 { desc.push_str("（高优先）"); }
    if !parsed.notify_times.is_empty() { desc.push_str(&format!("（提醒 {}）", parsed.notify_times.join(", "))); }
    desc.push_str(&format!(" → {}", date));
    ToolResult { tool_call_id: String::new(), output: desc, success: true }
}

// ─── 测试 ───


// ─── 知识库写工具（所有权 + 审批）───

/// 覆盖/创建文档（含所有权登记）
fn kb_write_file(ctx: &AppContext, user: Option<&CurrentUser>, kb: &str, rel: &str, content: &str, owner: &str) -> Result<String, String> {
    let Some(kbn) = crate::api::kb::sanitize_name(kb) else { return Err("非法知识库名".into()) };
    let Some(relc) = crate::api::kb::sanitize_path(rel) else { return Err("非法文档路径".into()) };
    if !relc.to_lowercase().ends_with(".md") { return Err("仅支持 .md 文档".into()) }
    let dir = crate::api::kb::kb_root(ctx, user).join(&kbn);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建知识库目录失败: {}", e))?;
    let path = dir.join(&relc);
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).ok(); }
    std::fs::write(&path, content).map_err(|e| format!("写入失败: {}", e))?;
    crate::api::kb::set_doc_owner(&dir, &relc, owner);
    Ok(format!("{}（{} 字符）", path.display(), content.chars().count()))
}

fn kb_create_kb(ctx: &AppContext, user: Option<&CurrentUser>, args: &str) -> ToolResult {
    let v: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v, Err(e) => return tool_err("", format!("参数解析失败: {}", e)),
    };
    let name = v["name"].as_str().unwrap_or("").trim();
    let Some(kbn) = crate::api::kb::sanitize_name(name) else {
        return tool_err("", "非法知识库名（禁空/禁/禁../禁点开头）");
    };
    let dir = crate::api::kb::kb_root(ctx, user).join(&kbn);
    if dir.exists() { return tool_err("", format!("知识库已存在: {}", kbn)); }
    std::fs::create_dir_all(&dir).ok();
    ToolResult { tool_call_id: String::new(), output: format!("知识库「{}」已创建", kbn), success: true }
}

fn kb_create_doc(ctx: &AppContext, user: Option<&CurrentUser>, args: &str) -> ToolResult {
    let v: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v, Err(e) => return tool_err("", format!("参数解析失败: {}", e)),
    };
    let kb = v["kb"].as_str().unwrap_or("").trim();
    let rel = v["path"].as_str().unwrap_or("").trim();
    let content = v["content"].as_str().unwrap_or("");
    if content.trim().is_empty() { return tool_err("", "content 不能为空"); }
    match kb_write_file(ctx, user, kb, rel, content, "agent") {
        Ok(loc) => ToolResult { tool_call_id: String::new(),
            output: format!("文档已创建（所有权：智能体）：{}", loc), success: true },
        Err(e) => tool_err("", e),
    }
}

async fn kb_delete_doc(ctx: &AppContext, user: Option<&CurrentUser>, args: &str, call_id: String) -> ChatOutcome {
    let v: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v, Err(e) => return ChatOutcome::Done(tool_err(&call_id, format!("参数解析失败: {}", e))),
    };
    let kb = v["kb"].as_str().unwrap_or("").trim();
    let rel = v["path"].as_str().unwrap_or("").trim();
    let Some(kbn) = crate::api::kb::sanitize_name(kb) else { return ChatOutcome::Done(tool_err(&call_id, "非法知识库名")); };
    let Some(relc) = crate::api::kb::sanitize_path(rel) else { return ChatOutcome::Done(tool_err(&call_id, "非法文档路径")); };
    let dir = crate::api::kb::kb_root(ctx, user).join(&kbn);
    let path = dir.join(&relc);
    if !path.exists() { return ChatOutcome::Done(tool_err(&call_id, format!("文档不存在: {}", rel))); }
    let owner = crate::api::kb::doc_owner(&dir, &relc);
    if owner != "agent" {
        // 用户文档删除 → 审批挂起（全模式）
        let (token, rx) = crate::approval::register_detailed_for(user.map(|u| u.username.as_str()), "删除", &format!("{}/{}", kbn, relc), "智能体请求删除你的知识库文档");
        return ChatOutcome::Approval { token, rx, action: "删除".into(), target: format!("{}/{}", kbn, relc), call_id, kb: kbn, rel: relc };
    }
    match std::fs::remove_file(&path) {
        Ok(()) => ChatOutcome::Done(ToolResult { tool_call_id: call_id, output: format!("已删除文档: {}", rel), success: true }),
        Err(e) => ChatOutcome::Done(tool_err(&call_id, format!("删除失败: {}", e))),
    }
}

/// 审批通过后的执行（由调度层调用；覆盖取回暂存内容，删除移除文件与所有权记录）
pub async fn perform_approved(ctx: &AppContext, user: Option<&CurrentUser>, action: &str, kb: &str, rel: &str, call_id: &str) -> ToolResult {
    let store_key = format!("{}/{}", kb, rel);
    match action {
        "覆盖" => {
            let Some(content) = crate::approval::take_content(&store_key) else {
                return tool_err(call_id, String::from("审批内容已过期（超时），请重试"));
            };
            match kb_write_file(ctx, user, kb, rel, &content, "user") {
                Ok(loc) => ToolResult { tool_call_id: call_id.into(),
                    output: format!("（经用户批准）已覆盖文档: {}", loc), success: true },
                Err(e) => tool_err(call_id, e),
            }
        }
        "删除" => {
            let dir = crate::api::kb::kb_root(ctx, user).join(kb);
            let p = dir.join(rel);
            match std::fs::remove_file(&p) {
                Ok(()) => { crate::api::kb::set_doc_owner(&dir, rel, ""); 
                    ToolResult { tool_call_id: call_id.into(), output: format!("（经用户批准）已删除文档: {}", rel), success: true } }
                Err(e) => tool_err(call_id, format!("删除失败: {}", e)),
            }
        }
        _ => tool_err(call_id, "未知审批动作"),
    }
}

async fn kb_edit_doc(ctx: &AppContext, user: Option<&CurrentUser>, args: &str, call_id: String) -> ChatOutcome {
    let v: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v, Err(e) => return ChatOutcome::Done(tool_err(&call_id, format!("参数解析失败: {}", e))),
    };
    let kb = v["kb"].as_str().unwrap_or("").trim();
    let rel = v["path"].as_str().unwrap_or("").trim();
    let content = v["content"].as_str().unwrap_or("");
    let Some(kbn) = crate::api::kb::sanitize_name(kb) else { return ChatOutcome::Done(tool_err(&call_id, "非法知识库名")); };
    let Some(relc) = crate::api::kb::sanitize_path(rel) else { return ChatOutcome::Done(tool_err(&call_id, "非法文档路径")); };
    if !relc.to_lowercase().ends_with(".md") { return ChatOutcome::Done(tool_err(&call_id, "仅支持 .md 文档")) }
    let dir = crate::api::kb::kb_root(ctx, user).join(&kbn);
    let path = dir.join(&relc);
    if path.exists() && crate::api::kb::doc_owner(&dir, &relc) != "agent" {
        // 用户文档 → 审批挂起（全模式生效）。内容由调度层在批准后取回执行
        let store_key = format!("{}/{}", kbn, relc);
        crate::approval::store_content(&store_key, content);
        let (token, rx) = crate::approval::register_detailed_for(user.map(|u| u.username.as_str()), "覆盖", &store_key, "智能体请求覆盖你的知识库文档");
        return ChatOutcome::Approval { token, rx, action: "覆盖".into(), target: format!("{}/{}", kbn, relc), call_id, kb: kbn.clone(), rel: relc.clone() };
    }
    let out = if path.exists() {
        match kb_write_file(ctx, user, kb, rel, content, "agent") {
            Ok(loc) => format!("已覆盖智能体自有文档: {}", loc),
            Err(e) => return ChatOutcome::Done(tool_err(&call_id, e)),
        }
    } else {
        match kb_write_file(ctx, user, kb, rel, content, "agent") {
            Ok(loc) => format!("文档不存在，已新建（所有权：智能体）：{}", loc),
            Err(e) => return ChatOutcome::Done(tool_err(&call_id, e)),
        }
    };
    ChatOutcome::Done(ToolResult { tool_call_id: call_id, output: out, success: true })
}

// ─── 技能与定时任务工具 ───

fn use_skill(ctx: &AppContext, user: Option<&CurrentUser>, args: &str) -> ToolResult {
    let v: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v,
        Err(e) => return tool_err("", format!("参数解析失败: {}", e)),
    };
    let name = v["name"].as_str().unwrap_or("").trim();
    match crate::skills::read_skill_body_for(ctx, user, name) {
        Some(body) => ToolResult {
            tool_call_id: String::new(),
            output: format!("已加载技能「{}」，请严格按以下指令继续：{}", name, body),
            success: true,
        },
        None => tool_err("", format!("技能不存在: {}（见系统提示的可用技能清单）", name)),
    }
}

fn create_task(ctx: &AppContext, user: Option<&CurrentUser>, args: &str) -> ToolResult {
    let v: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v,
        Err(e) => return tool_err("", format!("参数解析失败: {}", e)),
    };
    let name = v["name"].as_str().unwrap_or("").trim();
    let prompt = v["prompt"].as_str().unwrap_or("").trim();
    if name.is_empty() || prompt.is_empty() {
        return tool_err("", "name 与 prompt 不能为空");
    }
    let stype = v["schedule_type"].as_str().unwrap_or("");
    let now = crate::scheduler::now_ts();
    let schedule = match stype {
        "once" => {
            let at = v["at"].as_u64().unwrap_or(0);
            if at <= now { return tool_err("", "once 模式的 at 必须是未来的 Unix 时间戳"); }
            crate::scheduler::TaskSchedule::Once { at }
        }
        "daily" => crate::scheduler::TaskSchedule::Daily { time: v["time"].as_str().unwrap_or("09:00").to_string() },
        "weekly" => {
            let weekdays: Vec<u32> = v["weekdays"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64().map(|n| n as u32)).collect()).unwrap_or_default();
            if weekdays.is_empty() { return tool_err("", "weekly 模式需要 weekdays（1-7）"); }
            crate::scheduler::TaskSchedule::Weekly { weekdays, time: v["time"].as_str().unwrap_or("09:00").to_string() }
        }
        "interval" => {
            let minutes = v["minutes"].as_u64().unwrap_or(0) as u32;
            if minutes == 0 { return tool_err("", "interval 模式需要 minutes > 0"); }
            crate::scheduler::TaskSchedule::Interval { minutes }
        }
        _ => return tool_err("", "schedule_type 需为 once/daily/weekly/interval"),
    };

    let skill = v["skill"].as_str().map(|s| s.to_string());
    if let Some(s) = &skill {
        if crate::skills::read_skill_body_for(ctx, user, s).is_none() {
            return tool_err("", format!("技能不存在: {}", s));
        }
    }
    let next_run = crate::scheduler::compute_next_run(&schedule, now, None);
    let task = crate::scheduler::ScheduledTask {
        id: format!("task_{}_{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0), rand::random::<u32>()),
        name: name.to_string(), prompt: prompt.to_string(),
        skill,
        schedule,
        enabled: true, created: now,
        last_run: None, next_run, last_result: None,
        conversation_id: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
        run_count: 0,
    };
    crate::scheduler::append_task_file(ctx, user, &task);
    let next_desc = task.next_run
        .map(|n| chrono::DateTime::from_timestamp(n as i64, 0)
            .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| n.to_string()))
        .unwrap_or_else(|| String::from("—"));
    ToolResult {
        tool_call_id: String::new(),
        output: format!("定时任务已创建：{}（下次执行：{}）。到点后 AI 将自动执行并把结果写入任务会话。", task.name, next_desc),
        success: true,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthState;
    use std::sync::Arc;
    use std::sync::RwLock;

    fn test_ctx(dir: &std::path::Path) -> AppContext {
        AppContext {
            providers: std::collections::HashMap::new(),
            default_model: String::new(),
            user_data_dir: dir.to_path_buf(),
            work_dir: dir.to_path_buf(),
            context_lengths: std::collections::HashMap::new(),
            image_dir: Arc::new(RwLock::new(dir.to_path_buf())),
            todo_dir: Arc::new(RwLock::new(dir.join("ToDoList"))),
            auth: None,
            server_mode: "standalone".into(),
            cloud_workdir: dir.join("CloudWork"),
        }
    }

    // ── 速记解析器（移植前端 todo-syntax-smoke 用例，双端一致）──
    #[test]
    fn test_parse_quick_syntax_matches_frontend() {
        let r1 = parse_quick_syntax("买菜");
        assert_eq!(r1.title, "买菜"); assert_eq!(r1.priority, 1);
        assert!(r1.category.is_empty()); assert!(!r1.notify_enabled);

        let r2 = parse_quick_syntax("!! 修复登录bug #工作");
        assert_eq!(r2.title, "修复登录bug"); assert_eq!(r2.priority, 2); assert_eq!(r2.category, "工作");

        let r3 = parse_quick_syntax("! 买菜 #生活");
        assert_eq!(r3.title, "买菜"); assert_eq!(r3.priority, 0); assert_eq!(r3.category, "生活");

        let r4 = parse_quick_syntax("!! 交周报 @09:30 #工作");
        assert_eq!(r4.title, "交周报"); assert_eq!(r4.priority, 2); assert_eq!(r4.category, "工作");
        assert!(r4.notify_enabled); assert_eq!(r4.notify_times, vec!["09:30"]);

        let r5 = parse_quick_syntax("开会 @9:05 @14:40");
        assert_eq!(r5.title, "开会"); assert_eq!(r5.notify_times, vec!["09:05", "14:40"]);

        let r6 = parse_quick_syntax("#工作 前缀标签在前的标题");
        assert_eq!(r6.title, "前缀标签在前的标题"); assert_eq!(r6.category, "工作");

        let r7 = parse_quick_syntax("@25:00 无效时间保留");
        assert!(r7.title.contains("@25:00")); assert!(!r7.notify_enabled);

        let r8 = parse_quick_syntax("   !!   ");
        assert_eq!(r8.title, "");

        let r9 = parse_quick_syntax("#工作 #次要 双标签只取首个");
        assert_eq!(r9.category, "工作"); assert!(r9.title.contains("#次要"));

        let r10 = parse_quick_syntax("!! 标题 @09:00 #工作");
        assert_eq!(r10.title, "标题"); assert_eq!(r10.category, "工作"); assert_eq!(r10.priority, 2);

        let r11 = parse_quick_syntax("#工作 !! 交周报 @09:00");
        assert_eq!(r11.title, "交周报"); assert_eq!(r11.priority, 2); assert_eq!(r11.category, "工作");
        assert_eq!(r11.notify_times, vec!["09:00"]);

        // @日期 token：长格式 / 短格式（当年）/ 非法日期保留原文 / 多日期取首个
        let r12 = parse_quick_syntax("#学习 复习记忆卡 @2026-09-25");
        assert_eq!(r12.title, "复习记忆卡"); assert_eq!(r12.category, "学习");
        assert_eq!(r12.date.as_deref(), Some("2026-09-25"));
        let r13 = parse_quick_syntax("项目评审 @10-08");
        assert_eq!(r13.title, "项目评审"); assert_eq!(r13.date.as_deref(), Some("2026-10-08"));
        let r14 = parse_quick_syntax("@02-30 非法日期保留");
        assert!(r14.title.contains("@02-30")); assert!(r14.date.is_none());
        let r15 = parse_quick_syntax("多日期 @2026-09-25 @09-30");
        assert_eq!(r15.title, "多日期"); assert_eq!(r15.date.as_deref(), Some("2026-09-25"));
        // 日期 + 时间混合：互不干扰
        let r16 = parse_quick_syntax("明天汇报 @2026-09-20 @09:00");
        assert_eq!(r16.title, "明天汇报"); assert_eq!(r16.date.as_deref(), Some("2026-09-20"));
        assert_eq!(r16.notify_times, vec!["09:00"]);
    }

    #[test]
    fn test_tools_for_mode_write_gating() {
        // 只读/计划：kb 只读×3 + todo_read + use_skill + kb_edit_doc（用户文档审批制，全模式可用）
        assert_eq!(tools_for_mode("readonly").len(), 6);
        assert_eq!(tools_for_mode("plan").len(), 6);
        // 自动/编辑/完全：+ kb_create_kb/kb_create_doc/kb_delete_doc + todo_create + create_task
        // + generate_image（AI 生图 — 原生管线，计划/只读禁生成）
        assert_eq!(tools_for_mode("auto").len(), 12);
        assert_eq!(tools_for_mode("edit").len(), 12);
        assert_eq!(tools_for_mode("full").len(), 7);
        let ro = tools_for_mode("readonly");
        for w in ["todo_create", "create_task", "kb_create_kb", "kb_create_doc", "kb_delete_doc", "generate_image"] {
            assert!(ro.iter().all(|t| t.name != w), "{} 不应出现在只读模式", w);
        }
        let au = tools_for_mode("auto");
        assert!(au.iter().any(|t| t.name == "kb_edit_doc" && true));
        assert!(au.iter().any(|t| t.name == "create_task"));
        assert!(au.iter().any(|t| t.name == "generate_image"), "自动模式必须提供原生生图工具");
        let fl = tools_for_mode("full");
        assert!(fl.iter().any(|t| t.name == "generate_image"), "完全模式必须提供原生生图工具");
    }

    #[tokio::test]
    async fn test_kb_read_rejects_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        let tc = ToolCall { id: "t1".into(), name: "kb_read".into(),
            arguments: r#"{"kb":"test","path":"../../config/llm.toml"}"#.into() };
        let r = execute_chat_tool(&ctx, None, &tc, true).await;
        assert!(!r.success);
        assert!(r.output.contains("非法"));
    }

    #[tokio::test]
    async fn test_kb_search_and_read_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ctx = test_ctx(tmp.path());
        ctx.user_data_dir = tmp.path().join("users").join("local");
        let kb_root = crate::api::kb::kb_root(&ctx, None);
        std::fs::create_dir_all(kb_root.join("测试库/子目录")).unwrap();
        std::fs::write(kb_root.join("测试库/子目录/水面.md"), "# 水面制作\nHoudini 翻滚水面要点。").unwrap();

        let tc = ToolCall { id: "t2".into(), name: "kb_search".into(),
            arguments: r#"{"query":"水面"}"#.into() };
        let r = execute_chat_tool(&ctx, None, &tc, true).await;
        assert!(r.success);
        assert!(r.output.contains("测试库"), "应标注来源 KB: {}", r.output);
        assert!(r.output.contains("子目录/水面.md"));

        let tc = ToolCall { id: "t3".into(), name: "kb_read".into(),
            arguments: r#"{"kb":"测试库","path":"子目录/水面.md"}"#.into() };
        let r = execute_chat_tool(&ctx, None, &tc, true).await;
        assert!(r.success && r.output.contains("Houdini 翻滚水面要点"));
    }

    #[tokio::test]
    async fn test_todo_create_standalone_and_isolated() {
        // standalone：落全局 ToDoList
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        let tc = ToolCall { id: "t4".into(), name: "todo_create".into(),
            arguments: r##"{"text":"#工作 !! 交周报 @09:00","date":"2026-09-12"}"##.into() };
        let r = execute_chat_tool(&ctx, None, &tc, true).await;
        assert!(r.success, "{}", r.output);
        let list = tapi::read_list_in(&tmp.path().join("ToDoList"), "2026-09-12").unwrap();
        assert_eq!(list.items.len(), 1);
        let item = &list.items[0];
        assert_eq!(item.title, "交周报");
        assert_eq!(item.category.as_deref(), Some("工作"));
        assert_eq!(item.priority, 2);
        assert_eq!(item.notify_times, vec!["09:00"]);
        assert!(item.notify_enabled);

        // server 模式：按用户隔离落盘
        let tmp2 = tempfile::tempdir().unwrap();
        let mut ctx2 = test_ctx(tmp2.path());
        ctx2.user_data_dir = tmp2.path().join("users").join("local");
        ctx2.auth = Some(AuthState::new(tmp2.path().join("cfg")));
        let alice = CurrentUser { username: "alice".into(), role: "user".into() };
        let r = execute_chat_tool(&ctx2, Some(&alice), &tc, true).await;
        assert!(r.success);
        assert!(tapi::read_list_in(&tmp2.path().join("users/alice/todo"), "2026-09-12").is_some(),
            "应落在 alice 的个人目录");
        assert!(!tmp2.path().join("ToDoList/2026-09-12.json").exists(), "不应污染旧根目录");
    }

    #[tokio::test]
    async fn test_todo_create_denied_in_readonly_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        let tc = ToolCall { id: "t7".into(), name: "todo_create".into(),
            arguments: r#"{"text":"写周报"}"#.into() };
        let r = execute_chat_tool(&ctx, None, &tc, false).await;
        assert!(!r.success);
        assert!(r.output.contains("不允许创建"));
        assert!(tapi::read_list_in(&tmp.path().join("ToDoList"), &tapi::today_str()).is_none(),
            "拒绝路径不得落盘");
    }

    #[tokio::test]
    async fn test_todo_read_formats() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        let tc = ToolCall { id: "t5".into(), name: "todo_create".into(),
            arguments: r##"{"text":"#工作 修 bug"}"##.into() };
        execute_chat_tool(&ctx, None, &tc, true).await;
        let tc = ToolCall { id: "t6".into(), name: "todo_read".into(),
            arguments: r#"{"scope":"today"}"#.into() };
        let r = execute_chat_tool(&ctx, None, &tc, true).await;
        assert!(r.success);
        assert!(r.output.contains("修 bug") && r.output.contains("#工作"));
    }
}

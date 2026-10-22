//! 工作文件生成工具 — docx / xlsx / pptx / pdf
//! AI 通过 create_* 工具调用，产物落到当前会话工作目录（本地或云工作）。
//! 内容以"结构化生成"为准：简单 Markdown（标题/列表/粗体/代码块）与二维表格。

use docx_rs::{Docx, Paragraph, Run, RunFonts, Table, TableRow, TableCell};
use std::path::Path;

use crate::api::AppContext;
use crate::auth::CurrentUser;
use skyland_lib::llm::tool::{Tool, ToolResult};
use skyland_lib::llm::provider::ToolCall;

pub const FILEGEN_TOOL_NAMES: &[&str] = &[
    "create_document", "create_spreadsheet", "create_presentation", "create_pdf",
];

pub fn is_filegen_tool(name: &str) -> bool {
    FILEGEN_TOOL_NAMES.contains(&name)
}

pub fn tools_for_mode() -> Vec<Tool> {
    vec![
        Tool::new("create_document", "生成 Word 文档（.docx）到当前工作目录。content 使用简单 Markdown：# ## ### 标题、- 列表、**粗体**、```代码块```、普通段落。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "文件名（可含子目录），如 周报/第37周周报.docx" },
                    "title": { "type": "string", "description": "文档标题" },
                    "content": { "type": "string", "description": "正文（简单 Markdown）" }
                },
                "required": ["path", "title", "content"]
            })),
        Tool::new("create_spreadsheet", "生成 Excel 表格（.xlsx）到当前工作目录，专业排版开箱即用：首行自动表头（加粗+深蓝底+白字+冻结+自动筛选）、列宽自适应。单表传 rows；多工作表传 sheets:[{name,rows,chart,sum_row}]。chart={type:'bar'|'line'|'pie', title, data_col, label_col} 自动插入图表（柱/折线/饼）；sum_row:true 追加 SUM 合计公式行。rows 为二维数组，首行通常是表头，单元格为字符串或数字。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "文件名，如 预算表.xlsx" },
                    "sheet": { "type": "string", "description": "工作表名（可选，默认 Sheet1）" },
                    "rows": { "type": "array", "items": { "type": "array" }, "description": "二维数据（单表模式），如 [[\"项目\",\"金额\"],[\"A\",100]]" },
                    "sheets": { "type": "array", "description": "多工作表模式：[{name, rows, chart?, sum_row?}]（与 rows 二选一，优先）" },
                    "chart": { "type": "object", "description": "图表（单表模式）：{type:bar|line|pie, title?, data_col?, label_col?}" },
                    "sum_row": { "type": "boolean", "description": "末尾追加合计行（对数值列生成 SUM 公式）" }
                },
                "required": ["path"]
            })),
        Tool::new("create_presentation", "生成 PPT 演示文稿（.pptx）到当前工作目录。每张幻灯片 = 标题 + 要点列表。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "文件名，如 方案汇报.pptx" },
                    "slides": { "type": "array", "items": { "type": "object", "properties": {
                        "title": { "type": "string" },
                        "bullets": { "type": "array", "items": { "type": "string" } }
                    } }, "description": "幻灯片数组" }
                },
                "required": ["path", "slides"]
            })),
        Tool::new("create_pdf", "生成 PDF 文档到当前工作目录。中文需要系统存在中文字体（Windows 自带）；失败时建议改用 create_document。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "文件名，如 报告.pdf" },
                    "title": { "type": "string", "description": "文档标题" },
                    "content": { "type": "string", "description": "正文（简单 Markdown，同 create_document）" }
                },
                "required": ["path", "title", "content"]
            })),
    ]
}

/// 会话工作目录解析（工具执行落点）
/// - "@cloud" → 云工作：{cloud_workdir}/sessions/{conversation_id}/（server/client 模式）
/// - 其他相对路径 → {work_dir}/{path}（本地模式）
#[allow(dead_code)]
pub fn resolve_session_workdir(
    ctx: &AppContext,
    workdir: Option<&str>,
    conversation_id: Option<u64>,
) -> Option<std::path::PathBuf> {
    resolve_session_workdir_for(ctx, None, workdir, conversation_id)
}

pub fn resolve_session_workdir_for(
    ctx: &AppContext,
    user: Option<&CurrentUser>,
    workdir: Option<&str>,
    conversation_id: Option<u64>,
) -> Option<std::path::PathBuf> {
    let raw = workdir?.trim();
    if raw.is_empty() { return None; }
    if raw == "@cloud" {
        if ctx.server_mode == "standalone" { return None; } // 单机模式无云工作
        let conv = conversation_id.map(|c| c.to_string()).unwrap_or_else(|| "default".into());
        let cloud_root = if ctx.auth.is_some() {
            crate::api::user_data_dir_for(ctx, user).join("workspaces").join("cloud")
        } else {
            ctx.cloud_workdir.clone()
        };
        let dir = cloud_root.join("sessions").join(conv);
        std::fs::create_dir_all(&dir).ok();
        return std::fs::canonicalize(&dir).ok().or(Some(dir));
    }
    let p = Path::new(raw);
    if p.is_absolute() {
        // 绝对路径仅在单机（无多用户认证）模式下允许 —— 用户经资源管理器自行选择；
        // server/client 模式多用户共享服务端，绝对路径是多用户安全面，保持拒绝
        if ctx.auth.is_some() { return None; }
        let dir = p.to_path_buf();
        std::fs::create_dir_all(&dir).ok();
        return std::fs::canonicalize(&dir).ok().or(Some(dir));
    }
    for comp in p.components() {
        if !matches!(comp, std::path::Component::Normal(_)) { return None; }
    }
    let dir = if ctx.auth.is_some() {
        crate::api::user_data_dir_for(ctx, user)
            .join("workspaces")
            .join("local")
            .join(p)
    } else {
        ctx.work_dir.join(p)
    };
    std::fs::create_dir_all(&dir).ok();
    std::fs::canonicalize(&dir).ok().or(Some(dir))
}

/// POST /api/workspace/prepare — 工作区选定后初始化 .skyland 结构（conversations/cache）。
/// 对齐 Claude Code 在项目目录建 .claude 的机制：工作区自带会话数据与缓存归属。
pub async fn workspace_prepare_handler(
    axum::extract::State(ctx): axum::extract::State<crate::api::AppContext>,
    user: Option<axum::Extension<CurrentUser>>,
    axum::Json(req): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let raw = req.get("workdir").and_then(|v| v.as_str()).unwrap_or("").trim();
    if raw.is_empty() {
        return axum::Json(serde_json::json!({ "status": "error", "message": "缺少 workdir 参数" }));
    }
    let Some(dir) = resolve_session_workdir_for(&ctx, current_user, Some(raw), None) else {
        return axum::Json(serde_json::json!({ "status": "error", "message": "无效的工作区路径（server 模式仅支持相对子路径或 @cloud）" }));
    };
    let skyland = dir.join(".skyland");
    for sub in ["conversations", "cache"] {
        if std::fs::create_dir_all(skyland.join(sub)).is_err() {
            return axum::Json(serde_json::json!({ "status": "error", "message": format!("无法创建 {}", skyland.display()) }));
        }
    }
    axum::Json(serde_json::json!({
        "status": "ok",
        "path": dir.display().to_string(),
        "skyland": skyland.display().to_string(),
    }))
}

/// GET /api/workspace/list — 最近工作区列表（数据目录持久化 — 跟随数据目录而非浏览器 localStorage）
pub async fn workspace_list_handler() -> axum::Json<serde_json::Value> {
    let path = crate::datadir::resolve().0.join("config").join("workspaces.json");
    let list = std::fs::read_to_string(&path).ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or(serde_json::json!([]));
    axum::Json(serde_json::json!({ "status": "ok", "workspaces": list }))
}

/// POST /api/workspace/list — 保存最近工作区列表（数组 [{path, cloud}]，上限 50 条）
pub async fn workspace_list_save_handler(
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let Some(arr) = body.as_array() else {
        return axum::Json(serde_json::json!({ "status": "error", "message": "格式无效（需数组）" }));
    };
    if arr.len() > 50 {
        return axum::Json(serde_json::json!({ "status": "error", "message": "列表过长（上限 50）" }));
    }
    let path = crate::datadir::resolve().0.join("config").join("workspaces.json");
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return axum::Json(serde_json::json!({ "status": "error", "message": "无法创建配置目录" }));
        }
    }
    let content = serde_json::to_string_pretty(arr).unwrap_or_else(|_| "[]".into());
    match std::fs::write(&path, content) {
        Ok(_) => axum::Json(serde_json::json!({ "status": "ok" })),
        Err(e) => axum::Json(serde_json::json!({ "status": "error", "message": format!("写入失败: {}", e) })),
    }
}

/// 执行文件生成工具（workdir 来自会话解析，None 时拒绝——生成必须有明确落点）
pub async fn execute_filegen_tool(
    _ctx: &AppContext,
    _user: Option<&CurrentUser>,
    workdir: Option<&Path>,
    tc: &ToolCall,
) -> ToolResult {
    let tc_owned = tc.clone();
    let workdir_owned = workdir.map(|p| p.to_path_buf());
    let r = tokio::task::spawn_blocking(move || {
        let wd = workdir_owned.as_deref();
        match tc_owned.name.as_str() {
            "create_document" => create_document(wd, &tc_owned.arguments),
            "create_spreadsheet" => create_spreadsheet(wd, &tc_owned.arguments),
            "create_presentation" => create_presentation(wd, &tc_owned.arguments),
            "create_pdf" => create_pdf(wd, &tc_owned.arguments),
            _ => Err(format!("未知生成工具: {}", tc_owned.name)),
        }
    })
    .await
    .unwrap_or_else(|e| Err(format!("生成任务失败: {}", e)));
    match r {
        Ok(mut ok) => { ok.tool_call_id = tc.id.clone(); ok }
        Err(msg) => ToolResult { tool_call_id: tc.id.clone(), output: msg, success: false },
    }
}

/// 解析落点：workdir 必须已解析；文件名只允许相对安全路径；自动补扩展名
fn target_path(workdir: Option<&Path>, raw: &str, ext: &str) -> Result<std::path::PathBuf, String> {
    let Some(dir) = workdir else {
        return Err("未设置工作目录 — 请先在对话中选择工作目录（本地子目录或云工作）".into());
    };
    let name = raw.trim();
    if name.is_empty() || name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
        return Err(format!("非法文件名: {}", name));
    }
    let mut p = dir.join(name);
    if p.extension().and_then(|e| e.to_str()).map(|e| !e.eq_ignore_ascii_case(ext)).unwrap_or(true) {
        p.set_extension(ext);
    }
    if let Some(parent) = p.parent() { std::fs::create_dir_all(parent).ok(); }
    Ok(p)
}

// ─── 简易 Markdown → 段落序列 ───

enum Line {
    Heading(u32, String),
    Bullet(String),
    Paragraph(String),
    Code(String),
    /// Markdown 管道表格（| a | b |），分隔行已剔除
    Table(Vec<Vec<String>>),
}

fn parse_md(content: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut in_code = false;
    let mut code_buf = String::new();
    let mut table_buf: Vec<Vec<String>> = Vec::new();
    for raw_line in content.lines() {
        let line = raw_line.trim_end();
        if line.trim_start().starts_with("```") {
            if in_code {
                out.push(Line::Code(code_buf.clone()));
                code_buf.clear();
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code_buf.push_str(line);
            code_buf.push('\n');
            continue;
        }
        let trimmed = line.trim();
        // 表格行 — 连续 | 行聚成一块；--- 分隔行（|:---|:---|）剔除
        if trimmed.starts_with('|') && trimmed.len() > 1 {
            let cells: Vec<String> = trimmed
                .trim_start_matches('|').trim_end_matches('|')
                .split('|').map(|s| s.trim().to_string()).collect();
            let is_sep = cells.iter().all(|c| c.is_empty() || c.chars().all(|ch| ch == '-' || ch == ':'));
            if !is_sep { table_buf.push(cells); }
            continue;
        }
        if !table_buf.is_empty() { out.push(Line::Table(std::mem::take(&mut table_buf))); }
        if let Some(h) = trimmed.strip_prefix("### ") {
            out.push(Line::Heading(3, h.to_string()));
        } else if let Some(h) = trimmed.strip_prefix("## ") {
            out.push(Line::Heading(2, h.to_string()));
        } else if let Some(h) = trimmed.strip_prefix("# ") {
            out.push(Line::Heading(1, h.to_string()));
        } else if let Some(b) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            out.push(Line::Bullet(b.to_string()));
        } else if !trimmed.is_empty() {
            out.push(Line::Paragraph(trimmed.to_string()));
        }
    }
    if !table_buf.is_empty() { out.push(Line::Table(table_buf)); }
    if in_code && !code_buf.is_empty() { out.push(Line::Code(code_buf)); }
    out
}

/// 行内 **粗体** 分段 → (text, bold)
fn inline_runs(text: &str) -> Vec<(String, bool)> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("**") {
        if let Some(j) = rest[i + 2..].find("**") {
            if i > 0 { parts.push((rest[..i].to_string(), false)); }
            parts.push((rest[i + 2..i + 2 + j].to_string(), true));
            rest = &rest[i + 2 + j + 2..];
        } else { break; }
    }
    if !rest.is_empty() { parts.push((rest.to_string(), false)); }
    if parts.is_empty() { parts.push((String::new(), false)); }
    parts
}

fn strip_md(text: &str) -> String {
    inline_runs(text).into_iter().map(|(t, _)| t).collect()
}

// ─── docx ───

fn docx_paragraph(text: &str, style: Option<&str>, code: bool) -> Paragraph {
    let mut p = Paragraph::new();
    if let Some(s) = style { p = p.style(s); }
    if code { p = p.add_run(Run::new().add_text(text).fonts(RunFonts::new().ascii("Consolas").east_asia("Consolas")).size(18)); return p; }
    for (seg, bold) in inline_runs(text) {
        let mut run = Run::new().add_text(&seg);
        if bold { run = run.bold(); }
        p = p.add_run(run);
    }
    p
}

fn create_document(workdir: Option<&Path>, args: &str) -> Result<ToolResult, String> {
    let v: serde_json::Value = serde_json::from_str(args).map_err(|e| format!("参数解析失败: {}", e))?;
    let path = target_path(workdir, v["path"].as_str().unwrap_or(""), "docx")?;
    let title = v["title"].as_str().unwrap_or("文档").trim().to_string();
    let content = v["content"].as_str().unwrap_or("");

    let mut docx = Docx::new();
    docx = docx.add_paragraph(docx_paragraph(&title, Some("Title"), false));
    for line in parse_md(content) {
        docx = match line {
            Line::Heading(1, t) => docx.add_paragraph(docx_paragraph(&t, Some("Heading1"), false)),
            Line::Heading(2, t) => docx.add_paragraph(docx_paragraph(&t, Some("Heading2"), false)),
            Line::Heading(_, t) => docx.add_paragraph(docx_paragraph(&t, Some("Heading3"), false)),
            Line::Bullet(t) => docx.add_paragraph(docx_paragraph(&format!("• {}", strip_md(&t)), None, false)),
            Line::Code(c) => docx.add_paragraph(docx_paragraph(c.trim_end(), None, true)),
            Line::Paragraph(t) => docx.add_paragraph(docx_paragraph(&t, None, false)),
            Line::Table(rows) => {
                // 管道表格 → Word 表格（TableGrid 内置样式自动描边；首行加粗表头）
                let table_rows: Vec<TableRow> = rows.iter().enumerate().map(|(ri, cells)| {
                    let tcs: Vec<TableCell> = cells.iter().map(|c| {
                        let mut run = Run::new().add_text(&strip_md(c));
                        if ri == 0 { run = run.bold(); }
                        TableCell::new().add_paragraph(Paragraph::new().add_run(run))
                    }).collect();
                    TableRow::new(tcs)
                }).collect();
                if !table_rows.is_empty() {
                    docx.add_table(Table::new(table_rows).style("TableGrid"))
                } else {
                    docx
                }
            }
        };
    }

    let file = std::fs::File::create(&path).map_err(|e| format!("创建文件失败: {}", e))?;
    docx.build().pack(file).map_err(|e| format!("docx 打包失败: {}", e))?;
    Ok(ToolResult {
        tool_call_id: String::new(),
        output: format!("已生成 Word 文档: {}（标题：{}）", path.display(), title),
        success: true,
    })
}

// ─── xlsx ───

/// 工作表规格（多表模式）；chart/sum_row 单表模式也可直接挂在顶层
struct SheetSpec {
    name: String,
    rows: Vec<serde_json::Value>,
    chart: Option<(String, Option<String>, Option<String>, Option<usize>, Option<usize>)>, // (type,title,series_name,data_col,label_col)
    sum_row: bool,
}

/// 工作表名清洗：非法字符（[]:*?/\）替换、长度 ≤31、空名兜底 SheetN
fn sanitize_sheet_name(raw: &str, idx: usize) -> String {
    let cleaned: String = raw.trim().chars()
        .map(|c| if matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\') { '_' } else { c })
        .collect();
    let mut name = if cleaned.is_empty() { format!("Sheet{}", idx + 1) } else { cleaned };
    if name.chars().count() > 31 {
        name = name.chars().take(31).collect();
    }
    name
}

/// Excel 列字母（0=A, 25=Z, 26=AA…）
fn col_letter(mut c: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (c % 26) as u8) as char);
        c = if c >= 26 { c / 26 - 1 } else { break };
    }
    s
}

/// 单元格文本化（与旧版行为一致：字符串剥引号、null 空串、数字/布尔原样）
fn cell_text(cell: &serde_json::Value) -> String {
    match cell {
        serde_json::Value::Null => String::new(),
        other => other.to_string().trim_matches('"').to_string(),
    }
}

/// 该列数据行是否存在数字（表头行 r=0 不计）
fn col_has_number(rows: &[serde_json::Value], c: usize) -> bool {
    rows.iter().skip(1).any(|r| {
        r.as_array().and_then(|cols| cols.get(c)).and_then(|v| v.as_f64()).is_some()
    })
}

/// 首个含数字的数据列（从第 1 列起找，第 0 列通常是标签）
fn first_numeric_col(rows: &[serde_json::Value]) -> Option<usize> {
    let width = rows.iter().filter_map(|r| r.as_array().map(|a| a.len())).max().unwrap_or(0);
    (1..width).find(|&c| col_has_number(rows, c))
}

/// 列宽自适应：内容显示宽度（CJK 记 2）+2，范围 8~60
fn column_width(rows: &[serde_json::Value], c: usize) -> f64 {
    let w = rows.iter().filter_map(|r| r.as_array()).filter_map(|cols| cols.get(c))
        .map(|v| {
            let t = cell_text(v);
            t.chars().map(|ch| if ch.is_ascii() { 1 } else { 2 }).sum::<usize>()
        }).max().unwrap_or(4) as f64;
    (w + 2.0).clamp(8.0, 60.0)
}

fn create_spreadsheet(workdir: Option<&Path>, args: &str) -> Result<ToolResult, String> {
    let v: serde_json::Value = serde_json::from_str(args).map_err(|e| format!("参数解析失败: {}", e))?;
    let path = target_path(workdir, v["path"].as_str().unwrap_or(""), "xlsx")?;

    // 工作表规格：sheets[] 多表优先；否则旧版单表（rows+sheet，chart/sum_row 挂顶层）
    let parse_chart = |o: &serde_json::Value| -> Option<(String, Option<String>, Option<String>, Option<usize>, Option<usize>)> {
        o.as_object().map(|c| (
            c.get("type").and_then(|t| t.as_str()).unwrap_or("bar").to_string(),
            c.get("title").and_then(|t| t.as_str()).map(|s| s.to_string()),
            c.get("series_name").and_then(|t| t.as_str()).map(|s| s.to_string()),
            c.get("data_col").and_then(|t| t.as_u64()).map(|t| t as usize),
            c.get("label_col").and_then(|t| t.as_u64()).map(|t| t as usize),
        ))
    };
    let mut specs: Vec<SheetSpec> = Vec::new();
    if let Some(arr) = v["sheets"].as_array() {
        for (i, s) in arr.iter().enumerate() {
            let rows = s["rows"].as_array()
                .ok_or("sheets[].rows 缺少（二维数组）")?.clone();
            if rows.len() > 10_000 { return Err("行数超限（10000）".into()); }
            specs.push(SheetSpec {
                name: sanitize_sheet_name(s["name"].as_str().unwrap_or(""), i),
                chart: parse_chart(&s["chart"]),
                sum_row: s["sum_row"].as_bool().unwrap_or(false),
                rows,
            });
        }
        if specs.is_empty() { return Err("sheets 为空数组".into()); }
    } else {
        let rows = v["rows"].as_array()
            .ok_or("缺少 rows（二维数组）或 sheets（多工作表）")?.clone();
        if rows.len() > 10_000 { return Err("行数超限（10000）".into()); }
        specs.push(SheetSpec {
            name: sanitize_sheet_name(v["sheet"].as_str().unwrap_or(""), 0),
            chart: parse_chart(&v["chart"]),
            sum_row: v["sum_row"].as_bool().unwrap_or(false),
            rows,
        });
    }
    if specs.len() > 20 { return Err("工作表数量超限（20）".into()); }

    let mut wb = rust_xlsxwriter::Workbook::new();
    let header_fmt = rust_xlsxwriter::Format::new()
        .set_bold().set_font_color(rust_xlsxwriter::Color::RGB(0xFFFFFF))
        .set_background_color(rust_xlsxwriter::Color::RGB(0x1F4E78));
    let mut total_cells = 0usize;
    let mut chart_count = 0usize;
    let mut used_names: Vec<String> = Vec::new();
    for spec in &specs {
        let ws = wb.add_worksheet();
        // 重名去重（Excel 不允许同名表）
        let mut name = spec.name.clone();
        while used_names.contains(&name) {
            name = format!("{}_", name);
            if name.chars().count() > 31 { name = name.chars().take(31).collect(); }
        }
        used_names.push(name.clone());
        ws.set_name(&name).map_err(|e| format!("工作表名无效: {}", e))?;

        let rows = &spec.rows;
        let mut max_cols = 0usize;
        for (r, row) in rows.iter().enumerate() {
            if let Some(cols) = row.as_array() {
                max_cols = max_cols.max(cols.len());
                for (c, cell) in cols.iter().enumerate() {
                    if r == 0 {
                        if let Some(n) = cell.as_f64() {
                            let _ = ws.write_number_with_format(r as u32, c as u16, n, &header_fmt);
                        } else {
                            let _ = ws.write_string_with_format(r as u32, c as u16, &cell_text(cell), &header_fmt);
                        }
                    } else if let Some(n) = cell.as_f64() {
                        let _ = ws.write_number(r as u32, c as u16, n);
                    } else if let Some(b) = cell.as_bool() {
                        let _ = ws.write_boolean(r as u32, c as u16, b);
                    } else {
                        let _ = ws.write_string(r as u32, c as u16, &cell_text(cell));
                    }
                    total_cells += 1;
                }
            }
        }

        // 合计行：数值列生成 SUM 公式（数据行 = 2..=n，1 基）
        if spec.sum_row && rows.len() > 1 {
            let n = rows.len() as u32; // 合计行的 0 基行号
            let _ = ws.write_string(n, 0, "合计");
            for c in 1..max_cols {
                if col_has_number(rows, c) {
                    let l = col_letter(c);
                    let f = format!("=SUM({l}2:{l}{})", rows.len());
                    let _ = ws.write_formula(n, c as u16, f.as_str());
                }
            }
        }

        // 表头冻结 + 自动筛选 + 列宽
        if rows.len() > 1 {
            let _ = ws.set_freeze_panes(1, 0);
            let _ = ws.autofilter(0, 0, (rows.len() - 1) as u32, (max_cols.saturating_sub(1)) as u16);
        }
        for c in 0..max_cols {
            let _ = ws.set_column_width(c as u16, column_width(rows, c));
        }

        // 图表：类别列默认第 0 列，数值列默认首个数字列；表名含空格/中文一律单引号包裹
        if let Some((ctype, title, series_name, data_col, label_col)) = &spec.chart {
            let data_rows = rows.len().saturating_sub(1);
            let dc = (*data_col).or_else(|| first_numeric_col(rows));
            if data_rows >= 1 {
                if let Some(dc) = dc {
                    if dc < max_cols {
                        let q = format!("'{}'", name);
                        let last = (data_rows + 1) as u32;
                        let mut chart = match ctype.as_str() {
                            "line" => rust_xlsxwriter::Chart::new_line(),
                            "pie" => rust_xlsxwriter::Chart::new_pie(),
                            _ => rust_xlsxwriter::Chart::new_column(),
                        };
                        if let Some(t) = title { chart.title().set_name(t); }
                        let dl = col_letter(dc);
                        let series = chart.add_series();
                        let vals = format!("{}!${}${}:${}${}", q, dl, 2, dl, last);
                        series.set_values(vals.as_str());
                        let lc = (*label_col).unwrap_or(0);
                        if lc < max_cols && lc != dc && ctype != "pie" {
                            let ll = col_letter(lc);
                            let cats = format!("{}!${}${}:${}${}", q, ll, 2, ll, last);
                            series.set_categories(cats.as_str());
                        }
                        let sname = series_name.clone().or_else(|| {
                            rows.first().and_then(|r| r.as_array()).and_then(|cols| cols.get(dc))
                                .map(|h| cell_text(h))
                        }).unwrap_or_else(|| "数据".into());
                        series.set_name(sname.as_str());
                        let _ = ws.insert_chart(1, (max_cols + 2) as u16, &chart);
                        chart_count += 1;
                    }
                }
            }
        }
    }

    wb.save(&path).map_err(|e| format!("xlsx 保存失败: {}", e))?;
    let chart_note = if chart_count > 0 { format!("，{} 张图表", chart_count) } else { String::new() };
    Ok(ToolResult {
        tool_call_id: String::new(),
        output: format!("已生成 Excel 表格: {}（{} 个工作表{}，{} 单元格）", path.display(), specs.len(), chart_note, total_cells),
        success: true,
    })
}

// ─── pptx ───

fn create_presentation(workdir: Option<&Path>, args: &str) -> Result<ToolResult, String> {
    // PPTX = ZIP(OOXML)。pptx-rs 生态不成熟，v1 采用"最小 OOXML 直写"：
    // 每张幻灯片 = 标题文本框 + 要点文本框，够用且零重量级依赖。
    let v: serde_json::Value = serde_json::from_str(args).map_err(|e| format!("参数解析失败: {}", e))?;
    let path = target_path(workdir, v["path"].as_str().unwrap_or(""), "pptx")?;
    let slides = v["slides"].as_array().ok_or("缺少 slides 数组")?;
    if slides.is_empty() || slides.len() > 200 { return Err("slides 需 1-200 张".into()); }

    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(
        std::fs::File::create(&path).map_err(|e| format!("创建文件失败: {}", e))?));
    let opts = zip::write::SimpleFileOptions::default();

    let mut add = |name: &str, content: &str| -> Result<(), String> {
        zip.start_file(name, opts).map_err(|e| format!("zip 写入失败: {}", e))?;
        use std::io::Write;
        zip.write_all(content.as_bytes()).map_err(|e| format!("zip 写入失败: {}", e))
    };

    add("[Content_Types].xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
<Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>
<Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>
<Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>
</Types>"#)?;
    add("_rels/.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#)?;
    add("ppt/_rels/presentation.xml.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
</Relationships>"#)?;
    add("ppt/presentation.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
<p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/></p:sldMasterIdLst>
<p:sldSz cx="12192000" cy="6858000"/>
</p:presentation>"#)?;
    // slideMaster / layout / theme 的最小合法骨架（WPS/PowerPoint 可打开只读内容）
    add("ppt/slideMasters/slideMaster1.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld/></p:sldMaster>"#)?;
    add("ppt/slideLayouts/slideLayout1.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld/></p:sldLayout>"#)?;
    add("ppt/theme/theme1.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="SKYLAND"><a:themeElements/></a:theme>"#)?;

    for (idx, slide) in slides.iter().enumerate() {
        let title = slide["title"].as_str().unwrap_or("").replace('&', "&amp;").replace('<', "&lt;");
        let bullets: Vec<String> = slide["bullets"].as_array().map(|a| {
            a.iter().filter_map(|b| b.as_str().map(|s| s.replace('&', "&amp;").replace('<', "&lt;")))
                .collect()
        }).unwrap_or_default();
        let body_paras: String = bullets.iter().map(|b| {
            format!(r#"<a:p><a:pPr/><a:r><a:rPr lang="zh-CN" sz="1800"/><a:t>{}</a:t></a:r></a:p>"#, b)
        }).collect::<Vec<_>>().join("");
        let slide_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
<p:cSld><p:spTree>
<p:nvGrpSpPr/><p:grpSpPr/>
<p:sp><p:nvSpPr/><p:spPr><a:xfrm><a:off x="800000" y="500000"/><a:ext cx="10592000" cy="1200000"/></a:xfrm></p:spPr>
<p:txBody><a:bodyPr/><a:p><a:r><a:rPr lang="zh-CN" sz="3200" b="1"/><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr/><p:spPr><a:xfrm><a:off x="800000" y="1900000"/><a:ext cx="10592000" cy="4400000"/></a:xfrm></p:spPr>
<p:txBody><a:bodyPr/>{}</p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"#, title, body_paras);
        add(&format!("ppt/slides/slide{}.xml", idx + 1), &slide_xml)?;
    }
    let _ = add; // closure 借用结束
    let file = zip.finish().map_err(|e| format!("pptx 收尾失败: {}", e))?;
    drop(file);

    Ok(ToolResult {
        tool_call_id: String::new(),
        output: format!("已生成演示文稿: {}（{} 张幻灯片）", path.display(), slides.len()),
        success: true,
    })
}

// ─── pdf ───

fn create_pdf(workdir: Option<&Path>, args: &str) -> Result<ToolResult, String> {
    let v: serde_json::Value = serde_json::from_str(args).map_err(|e| format!("参数解析失败: {}", e))?;
    let path = target_path(workdir, v["path"].as_str().unwrap_or(""), "pdf")?;
    let title = v["title"].as_str().unwrap_or("文档").trim().to_string();
    let content = v["content"].as_str().unwrap_or("");

    // 中文字体：优先 Windows 自带，找不到时报可读错误（Linux 服务器需自备字体目录）
    let font_dirs = ["C:/Windows/Fonts", "/usr/share/fonts", "./config/fonts"];
    let font_candidates = ["simhei.ttf", "msyh.ttc", "simsun.ttc", "NotoSansCJK-Regular.ttc"];
    let mut font_family: Option<genpdf::fonts::FontFamily<genpdf::fonts::FontData>> = None;
    'outer: for dir in font_dirs {
        for f in font_candidates {
            if let Ok(fam) = genpdf::fonts::from_files(dir, f.trim_end_matches(".ttf").trim_end_matches(".ttc"), None) {
                font_family = Some(fam);
                break 'outer;
            }
        }
    }
    let Some(font_family) = font_family else {
        return Err("未找到可用中文字体（尝试 C:/Windows/Fonts 下的 simhei/msyh/simsun）。请改用 create_document 或安装字体。".into());
    };

    let mut doc = genpdf::Document::new(font_family);
    doc.set_title(&title);
    // 标题
    doc.push(genpdf::elements::Paragraph::new(
        genpdf::style::StyledString::new(title.clone(), genpdf::style::Style::new().bold().with_font_size(22))));
    doc.push(genpdf::elements::Break::new(1));
    for line in parse_md(content) {
        let text = match &line {
            Line::Heading(_, t) => strip_md(t),
            Line::Bullet(t) => format!("• {}", strip_md(t)),
            Line::Paragraph(t) => strip_md(t),
            Line::Code(c) => c.trim_end().to_string(),
            Line::Table(rows) => {
                // PDF 无原生表格 — 行内拼接为 "a | b" 文本（首行加粗）
                for (ri, cells) in rows.iter().enumerate() {
                    let text = cells.join(" | ");
                    let mut style = genpdf::style::Style::new();
                    if ri == 0 { style = style.bold(); }
                    style = style.with_font_size(11);
                    doc.push(genpdf::elements::Paragraph::new(genpdf::style::StyledString::new(text, style)));
                    doc.push(genpdf::elements::Break::new(0.3));
                }
                continue;
            }
        };
        let is_heading = matches!(line, Line::Heading(..));
        let mut style = genpdf::style::Style::new();
        if is_heading { style = style.bold(); }
        style = style.with_font_size(if is_heading { 15 } else { 11 });
        doc.push(genpdf::elements::Paragraph::new(genpdf::style::StyledString::new(text, style)));
        doc.push(genpdf::elements::Break::new(0.6));
    }
    let pdf_file = std::fs::File::create(&path).map_err(|e| format!("创建文件失败: {}", e))?;
    doc.render(pdf_file).map_err(|e| format!("PDF 渲染失败: {}", e))?;
    Ok(ToolResult {
        tool_call_id: String::new(),
        output: format!("已生成 PDF: {}（标题：{}）", path.display(), title),
        success: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_md_lines() {
        let lines = parse_md("# 标题\n正文段落\n- 列表项\n```py\ncode()\n```\n## 二级");
        assert_eq!(lines.len(), 5);
        assert!(matches!(lines[0], Line::Heading(1, ref t) if t == "标题"));
        assert!(matches!(lines[1], Line::Paragraph(ref t) if t == "正文段落"));
        assert!(matches!(lines[2], Line::Bullet(ref t) if t == "列表项"));
        assert!(matches!(lines[3], Line::Code(ref t) if t.contains("code()")));
        assert!(matches!(lines[4], Line::Heading(2, _)));
    }

    #[test]
    fn test_inline_runs_bold() {
        let parts = inline_runs("普通**加粗**普通");
        assert_eq!(parts, vec![("普通".to_string(), false), ("加粗".to_string(), true), ("普通".to_string(), false)]);
        assert_eq!(strip_md("**加粗**文本"), "加粗文本");
    }

    #[test]
    fn test_target_path_safety() {
        let dir = std::path::Path::new("/work");
        assert!(target_path(Some(dir), "../x.docx", "docx").is_err());
        assert!(target_path(Some(dir), "/abs.docx", "docx").is_err());
        assert!(target_path(None, "a.docx", "docx").is_err());
        let p = target_path(Some(dir), "周报/第37周", "docx").unwrap();
        assert!(p.to_string_lossy().ends_with("第37周.docx"));
    }

    #[test]
    fn test_document_and_spreadsheet_generation() {
        let tmp = tempfile::tempdir().unwrap();
        let wd = Some(tmp.path() as &Path);
        // docx
        let args = serde_json::json!({"path": "周报.docx", "title": "周报",
            "content": "# 标题\n- 要点一\n- **要点二**\n正文"}).to_string();
        let r = create_document(wd, &args).unwrap();
        assert!(r.success);
        assert!(tmp.path().join("周报.docx").exists());
        // xlsx
        let args = serde_json::json!({"path": "预算.xlsx",
            "rows": [["项目", "金额"], ["A", 100], ["B", 200.5]]}).to_string();
        let r = create_spreadsheet(wd, &args).unwrap();
        assert!(r.success);
        assert!(tmp.path().join("预算.xlsx").exists());
    }

    #[test]
    fn test_parse_md_table() {
        let md = "# 标题\n| 商品 | 销量 |\n| --- | --- |\n| A | 10 |\n| B | 20 |\n正文";
        let lines = parse_md(md);
        assert_eq!(lines.len(), 3);
        assert!(matches!(lines[1], Line::Table(ref t)
            if t.len() == 3 && t[0][0] == "商品" && t[0][1] == "销量" && t[2][1] == "20"));
    }

    #[test]
    fn test_col_letter() {
        assert_eq!(col_letter(0), "A");
        assert_eq!(col_letter(25), "Z");
        assert_eq!(col_letter(26), "AA");
        assert_eq!(col_letter(51), "AZ");
    }

    #[test]
    fn test_spreadsheet_multisheet_chart_sum() {
        let tmp = tempfile::tempdir().unwrap();
        let wd = Some(tmp.path() as &Path);
        let args = serde_json::json!({
            "path": "多表.xlsx",
            "sheets": [
                { "name": "销售明细", "rows": [["商品","销量","销售额"], ["A", 10, 100.0], ["B", 20, 300.5]],
                  "chart": { "type": "bar", "title": "销量对比", "data_col": 1, "label_col": 0 },
                  "sum_row": true },
                { "name": "汇总", "rows": [["区域","总额"], ["华南", 400.5]] }
            ]
        }).to_string();
        let r = create_spreadsheet(wd, &args).unwrap();
        assert!(r.success);
        assert!(r.output.contains("2 个工作表"), "{}", r.output);
        assert!(r.output.contains("1 张图表"), "{}", r.output);
        assert!(tmp.path().join("多表.xlsx").exists());
    }

    #[test]
    fn test_document_with_table() {
        let tmp = tempfile::tempdir().unwrap();
        let wd = Some(tmp.path() as &Path);
        let args = serde_json::json!({"path": "表格文档.docx", "title": "测试",
            "content": "# 一级\n| 列A | 列B |\n| --- | --- |\n| 1 | 2 |\n- 要点"}).to_string();
        let r = create_document(wd, &args).unwrap();
        assert!(r.success);
        assert!(tmp.path().join("表格文档.docx").exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_pdf_generation_chinese() {
        let tmp = tempfile::tempdir().unwrap();
        let wd = Some(tmp.path() as &Path);
        match create_pdf(wd, &serde_json::json!({"path": "报告.pdf", "title": "测试报告",
            "content": "# 一级\n中文内容测试"}).to_string()) {
            Ok(r) => { assert!(r.success); assert!(tmp.path().join("报告.pdf").exists()); }
            Err(e) => assert!(e.contains("字体"), "Windows 上应有中文字体: {}", e),
        };
    }

    #[test]
    fn test_resolve_session_workdir_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ctx = AppContext {
            providers: std::collections::HashMap::new(),
            default_model: String::new(),
            user_data_dir: tmp.path().to_path_buf(),
            work_dir: tmp.path().to_path_buf(),
            context_lengths: std::collections::HashMap::new(),
            image_dir: std::sync::Arc::new(std::sync::RwLock::new(tmp.path().to_path_buf())),
            todo_dir: std::sync::Arc::new(std::sync::RwLock::new(tmp.path().to_path_buf())),
            auth: None,
            server_mode: "standalone".into(),
            cloud_workdir: tmp.path().join("CloudWork"),
        };
        // standalone 模式：@cloud 被拒绝（设计行为）；绝对路径允许（用户经资源管理器自选）
        assert!(resolve_session_workdir(&ctx, Some("@cloud"), Some(1)).is_none());
        // 本地相对子路径：自动建目录
        let wd = resolve_session_workdir(&ctx, Some("子目录"), Some(1)).unwrap();
        assert!(wd.exists());
        // 绝对路径与穿越拒绝
        assert!(resolve_session_workdir(&ctx, Some("../x"), Some(1)).is_none());
        // server 模式：@cloud → CloudWork/sessions/{id}
        ctx.server_mode = "server".into();
        let cloud = resolve_session_workdir(&ctx, Some("@cloud"), Some(99002)).unwrap();
        assert!(cloud.to_string_lossy().contains("sessions"), "{}", cloud.display());
        assert!(cloud.to_string_lossy().ends_with("99002"));
        assert!(cloud.exists());
    }

    #[test]
    fn resolve_session_workdir_for_server_uses_user_root() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ctx = AppContext {
            providers: std::collections::HashMap::new(),
            default_model: String::new(),
            user_data_dir: tmp.path().join("users").join("local"),
            work_dir: tmp.path().to_path_buf(),
            context_lengths: std::collections::HashMap::new(),
            image_dir: std::sync::Arc::new(std::sync::RwLock::new(tmp.path().to_path_buf())),
            todo_dir: std::sync::Arc::new(std::sync::RwLock::new(tmp.path().join("todo"))),
            auth: None,
            server_mode: "server".into(),
            cloud_workdir: tmp.path().join("CloudWork"),
        };
        ctx.auth = Some(crate::auth::AuthState::new(tmp.path().join("auth")));
        let alice = CurrentUser { username: "alice".into(), role: "user".into() };
        let cloud = resolve_session_workdir_for(&ctx, Some(&alice), Some("@cloud"), Some(7)).unwrap();
        let cloud_text = cloud.to_string_lossy().replace('\\', "/");
        assert!(cloud_text.contains("/users/alice/workspaces/cloud/sessions/7"), "{}", cloud_text);
    }

    #[tokio::test]
    async fn test_execute_requires_workdir() {
        let tc = ToolCall { id: "t".into(), name: "create_document".into(),
            arguments: serde_json::json!({"path": "a.docx", "title": "t", "content": "c"}).to_string().into() };
        let r = {
            let ctx = AppContext {
                providers: std::collections::HashMap::new(),
                default_model: String::new(),
                user_data_dir: std::path::PathBuf::from("."),
                work_dir: std::path::PathBuf::from("."),
                context_lengths: std::collections::HashMap::new(),
                image_dir: std::sync::Arc::new(std::sync::RwLock::new(std::path::PathBuf::from("."))),
                todo_dir: std::sync::Arc::new(std::sync::RwLock::new(std::path::PathBuf::from("."))),
                auth: None,
                server_mode: "standalone".into(),
                cloud_workdir: std::path::PathBuf::from("./CloudWork"),
            };
            execute_filegen_tool(&ctx, None, None, &tc).await
        };
        assert!(!r.success);
        assert!(r.output.contains("工作目录"));
    }
}

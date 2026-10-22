//! 定时任务引擎 — 单次/重复任务，到点后 AI 完整执行
//! 存储：{user_data_dir}/scheduler/{username}/tasks.json（server 模式按用户隔离）；
//!       {user_data_dir}/scheduler/tasks.json（standalone）。
//! 调度：create_router 内 spawn 单例循环，每 20s 扫描到期任务 → 顺序执行
//!       （run_loop 全工具）→ 结果追加任务专属会话 → 推进 next_run → 前端轮询通知。

use axum::{Json, extract::{State, Path as AxumPath}};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::api::{user_data_dir_for, AppContext};
use crate::auth::CurrentUser;
use chrono::{Datelike, Local, NaiveTime, TimeZone};

use skyland_lib::llm::{Message, LlmProvider, tool::tools_for_mode, tool_executor};

// ─── 数据模型 ───

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TaskSchedule {
    /// 单次：指定时间戳（秒）
    Once { at: u64 },
    /// 每天 HH:MM
    Daily { time: String },
    /// 每周指定星期（1=周一..7=周日）的 HH:MM
    Weekly { weekdays: Vec<u32>, time: String },
    /// 每 N 分钟
    Interval { minutes: u32 },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub skill: Option<String>,
    pub schedule: TaskSchedule,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub created: u64,
    #[serde(default)]
    pub last_run: Option<u64>,
    #[serde(default)]
    pub next_run: Option<u64>,
    #[serde(default)]
    pub last_result: Option<String>,
    #[serde(default)]
    pub conversation_id: u64,
    #[serde(default)]
    pub run_count: u32,
}

fn default_true() -> bool { true }

pub fn now_ts() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ─── next_run 计算 ───

fn parse_hhmm(time: &str) -> Option<NaiveTime> {
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() != 2 { return None; }
    let h: u32 = parts[0].trim().parse().ok()?;
    let m: u32 = parts[1].trim().parse().ok()?;
    if h > 23 || m > 59 { return None; }
    NaiveTime::from_hms_opt(h, m, 0)
}

/// 计算下一次执行时间（now 为当前时间戳）
pub fn compute_next_run(schedule: &TaskSchedule, now: u64, last_run: Option<u64>) -> Option<u64> {
    match schedule {
        TaskSchedule::Once { at } => {
            // 尚未执行过且时间已设定 → 该时间即下次（已过也在到期时触发一次）
            if last_run.is_none() { Some(*at) } else { None }
        }
        TaskSchedule::Daily { time } => {
            let t = parse_hhmm(time)?;
            let now_dt = Local::now();
            for days_ahead in 0..2 {
                let dt = now_dt.date_naive().and_time(t) + chrono::Duration::days(days_ahead);
                if let Some(local) = Local.from_local_datetime(&dt).single() {
                    let ts = local.timestamp() as u64;
                    if ts > now { return Some(ts); }
                }
            }
            None
        }
        TaskSchedule::Weekly { weekdays, time } => {
            let t = parse_hhmm(time)?;
            if weekdays.is_empty() { return None; }
            let now_dt = Local::now();
            for days_ahead in 0..8 {
                let date = now_dt.date_naive() + chrono::Duration::days(days_ahead);
                let iso = date.weekday().number_from_monday(); // 1=Mon..7=Sun
                if !weekdays.contains(&iso) { continue; }
                let dt = date.and_time(t);
                if let Some(local) = Local.from_local_datetime(&dt).single() {
                    let ts = local.timestamp() as u64;
                    if ts > now { return Some(ts); }
                }
            }
            None
        }
        TaskSchedule::Interval { minutes } => {
            let step = (*minutes as u64).max(1) * 60;
            match last_run {
                Some(lr) => {
                    // 追到未来最近的一个周期点（离线期间错过的只补最近一次）
                    let mut next = lr + step;
                    while next <= now { next += step; }
                    Some(next)
                }
                None => Some(now + step),
            }
        }
    }
}

// ─── 存储 ───

fn scheduler_root(ctx: &AppContext) -> PathBuf {
    let dir = ctx.user_data_dir.join("tasks");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// 任务文件路径：server 模式按用户隔离
fn tasks_file_for(ctx: &AppContext, user: Option<&CurrentUser>) -> PathBuf {
    if ctx.auth.is_none() {
        return scheduler_root(ctx).join("tasks.json");
    }
    user_data_dir_for(ctx, user).join("tasks").join("tasks.json")
}

fn read_tasks(path: &Path) -> Vec<ScheduledTask> {
    std::fs::read_to_string(path).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_tasks(path: &Path, tasks: &[ScheduledTask]) {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).ok(); }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string_pretty(tasks).unwrap_or_default()).is_ok() {
        if std::fs::rename(&tmp, path).is_ok() { return; }
        let _ = std::fs::remove_file(&tmp);
    }
    let _ = std::fs::write(path, serde_json::to_string_pretty(tasks).unwrap_or_default());
}

/// 追加一条任务到任务文件（AI create_task 工具用）
pub(crate) fn append_task_file(ctx: &AppContext, user: Option<&CurrentUser>, task: &ScheduledTask) {
    let path = tasks_file_for(ctx, user);
    let mut tasks = read_tasks(&path);
    tasks.push(task.clone());
    write_tasks(&path, &tasks);
}

/// 列出 server 模式全部用户任务（调度循环用）— 返回 (用户名或 None, 任务列表)
fn scan_all_task_sets(ctx: &AppContext) -> Vec<(Option<String>, Vec<ScheduledTask>)> {
    let mut sets = Vec::new();
    if ctx.auth.is_some() {
        let users_root = crate::datadir::AppDataLayout::from_context(ctx).root().join("users");
        if let Ok(entries) = std::fs::read_dir(users_root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.path().is_dir() && !name.starts_with('.') {
                    sets.push((Some(name), read_tasks(&entry.path().join("tasks").join("tasks.json"))));
                }
            }
        }
    } else {
        sets.push((None, read_tasks(&tasks_file_for(ctx, None))));
    }
    sets
}

// ─── 调度循环 ───

static SCHED_STARTED: OnceLock<()> = OnceLock::new();
static RUNNING: OnceLock<Arc<Mutex<HashSet<String>>>> = OnceLock::new();

fn running_set() -> Arc<Mutex<HashSet<String>>> {
    RUNNING.get_or_init(|| Arc::new(Mutex::new(HashSet::new()))).clone()
}

/// 启动调度循环（幂等 — 多次调用只启动一次）。
/// 自建 runtime 独立线程：create_router 在 tokio::main（skyland-web）与
/// 普通线程（Tauri 壳，无 reactor）两种上下文被调用，tokio::spawn 在后者会 panic。
pub fn start_scheduler(ctx: AppContext) {
    if SCHED_STARTED.set(()).is_err() { return; }
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => { eprintln!("⚠ 调度器 runtime 创建失败: {}", e); return; }
        };
        rt.block_on(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(20));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                tick(&ctx).await;
            }
        });
    });
}

/// 单次扫描：找出所有到期任务并顺序执行
pub async fn tick(ctx: &AppContext) {
    let now = now_ts();
    let sets = scan_all_task_sets(ctx);
    for (username, mut tasks) in sets {
        let user = username.as_ref().map(|u| CurrentUser { username: u.clone(), role: "user".into() });
        let path = tasks_file_for(ctx, user.as_ref());
        let mut changed = false;
        for task in tasks.iter_mut() {
            if !task.enabled { continue; }
            // 首次补算 next_run
            if task.next_run.is_none() {
                task.next_run = compute_next_run(&task.schedule, now, task.last_run);
                changed = true;
                continue;
            }
            let due = task.next_run.map(|n| n <= now).unwrap_or(false);
            if !due { continue; }
            // 防重入（执行可能耗时数分钟，tick 每 20s 一次）
            {
                let set_arc = running_set();
                let mut set = set_arc.lock().unwrap_or_else(|e| e.into_inner());
                if set.contains(&task.id) { continue; }
                set.insert(task.id.clone());
            }
            execute_task(ctx, user.as_ref(), task).await;
            {
                let set_arc = running_set();
                let mut set = set_arc.lock().unwrap_or_else(|e| e.into_inner());
                set.remove(&task.id);
            }
            // 单次任务执行后停用；重复任务推进 next_run
            match &task.schedule {
                TaskSchedule::Once { .. } => { task.enabled = false; task.next_run = None; }
                _ => { task.next_run = compute_next_run(&task.schedule, now_ts(), Some(now_ts())); }
            }
            changed = true;
        }
        if changed { write_tasks(&path, &tasks); }
    }
}

/// 执行单个任务：LLM 全工具循环 → 结果写任务会话 → 更新任务状态
async fn execute_task(ctx: &AppContext, user: Option<&CurrentUser>, task: &mut ScheduledTask) {
    let started = now_ts();
    let result = run_task_llm(ctx, user, task).await;
    let summary: String = match &result {
        Ok(text) => text.chars().take(500).collect(),
        Err(e) => format!("执行失败: {}", e),
    };

    // 结果追加到任务专属会话（复用 conversations 存储）
    append_task_conversation(ctx, user, task, &summary);

    task.last_run = Some(started);
    task.last_result = Some(summary.chars().take(200).collect());
    task.run_count += 1;
}

async fn run_task_llm(ctx: &AppContext, user: Option<&CurrentUser>, task: &ScheduledTask) -> Result<String, String> {
    let provider = ctx.providers.get(&ctx.default_model).cloned()
        .ok_or_else(|| "无可用模型".to_string())?;

    let mut system = format!(
        "你是 SKYLAND 的定时任务执行器。任务名称：{}。\n\
         现在请完成用户给出的任务指令，产出最终结果文本。你可以使用全部工具（读写文件、检索知识库、查看/创建待办、网络搜索等）。\n\
         结果要求：结构化 Markdown，控制在 500 字以内，突出结论与行动项。",
        task.name
    );
    if let Some(skill_name) = &task.skill {
        if let Some(body) = crate::skills::read_skill_body_for(ctx, user, skill_name) {
            system.push_str(&format!("\n\n# 关联技能：{}\n{}", skill_name, body));
        }
    }

    let mut msgs = vec![Message::system(&system), Message::user(&task.prompt)];
    let mut tools = tools_for_mode("auto");
    // P1-2 修复：任务上下文经 execute_chat_tool 分发 — 只装配它能真正分发的工具。
    // （旧实现把 kb_* 写工具也塞给模型，调用必落"未知联动工具"，模型拿到的是必失败诱饵）
    tools.extend(crate::chat_tools::tools_for_mode("auto")
        .into_iter()
        .filter(|t| crate::chat_tools::task_dispatchable(&t.name)));

    // 联动工具需要身份与上下文 — 定时任务的写入工具经 run_loop 走通用 tool_executor，
    // 业务工具（kb/todo）在这里通过包装调度执行：
    let mut final_text = String::new();
    for _round in 0..20 {
        let resp = provider.chat(&msgs, &tools).await.map_err(|e| e.to_string())?;
        if resp.tool_calls.is_empty() {
            final_text = resp.content;
            break;
        }
        msgs.push(Message::assistant_with_tools(&resp.content, &resp.tool_calls));
        for tc in &resp.tool_calls {
            let result = if crate::chat_tools::is_chat_tool(&tc.name) {
                crate::chat_tools::execute_chat_tool(ctx, user, tc, true).await
            } else {
                tool_executor::execute_tool(tc).await.unwrap_or_else(|e| {
                    skyland_lib::llm::tool::ToolResult {
                        tool_call_id: tc.id.clone(),
                        output: format!("Error: {}", e),
                        success: false,
                    }
                })
            };
            let summary = format!("{} (success={})", result.output.chars().take(2000).collect::<String>(), result.success);
            msgs.push(Message::tool(&summary, &tc.id));
        }
    }
    if final_text.is_empty() {
        final_text = "任务已执行（模型未返回文本结果，详见工具调用）".into();
    }
    Ok(final_text)
}

fn append_task_conversation(ctx: &AppContext, user: Option<&CurrentUser>, task: &ScheduledTask, summary: &str) {
    let dir = user_data_dir_for(ctx, user).join("conversations");
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join(format!("{}.json", task.conversation_id));
    let mut data: serde_json::Value = std::fs::read_to_string(&path).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::json!({
            "id": task.conversation_id,
            "title": format!("[定时] {}", task.name),
            "messages": [],
        }));
    if let Some(msgs) = data["messages"].as_array_mut() {
        msgs.push(serde_json::json!({"role": "user", "content": task.prompt}));
        msgs.push(serde_json::json!({"role": "assistant", "content": summary}));
        // 防无限膨胀：保留最近 200 条
        if msgs.len() > 200 {
            let keep = msgs[msgs.len() - 200..].to_vec();
            *msgs = keep;
        }
    }
    data["saved_at"] = serde_json::json!(now_ts());
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&data).unwrap_or_default());
}

// ─── API ───

#[derive(Deserialize)]
pub struct CreateTaskRequest {
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub skill: Option<String>,
    pub schedule: TaskSchedule,
}

fn task_to_json(task: &ScheduledTask) -> serde_json::Value {
    serde_json::to_value(task).unwrap_or_default()
}

/// GET /api/tasks/list
pub async fn tasks_list_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let path = tasks_file_for(&ctx, user.as_deref());
    let tasks = read_tasks(&path);
    Json(serde_json::json!({ "status": "ok", "tasks": tasks }))
}

/// POST /api/tasks/create
pub async fn tasks_create_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<CurrentUser>>,
    Json(req): Json<CreateTaskRequest>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let name = req.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 60 {
        return Json(serde_json::json!({ "status": "error", "message": "任务名需 1-60 字符" }));
    }
    if req.prompt.trim().is_empty() || req.prompt.len() > 8000 {
        return Json(serde_json::json!({ "status": "error", "message": "任务指令需 1-8000 字节" }));
    }
    if let Some(s) = &req.skill {
        if crate::skills::read_skill_body_for(&ctx, current_user, s).is_none() {
            return Json(serde_json::json!({ "status": "error", "message": format!("技能不存在: {}", s) }));
        }
    }
    let now = now_ts();
    let next_run = compute_next_run(&req.schedule, now, None);
    let task = ScheduledTask {
        id: format!("task_{}_{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0), rand::random::<u32>()),
        name, prompt: req.prompt.trim().to_string(),
        skill: req.skill,
        schedule: req.schedule,
        enabled: true, created: now,
        last_run: None, next_run, last_result: None,
        conversation_id: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
        run_count: 0,
    };
    let path = tasks_file_for(&ctx, current_user);
    let mut tasks = read_tasks(&path);
    tasks.push(task.clone());
    write_tasks(&path, &tasks);
    Json(serde_json::json!({ "status": "ok", "task": task_to_json(&task) }))
}

/// PUT /api/tasks/:id — 更新（启停/改名/改计划）
pub async fn tasks_update_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<CurrentUser>>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let path = tasks_file_for(&ctx, user.as_deref());
    let mut tasks = read_tasks(&path);
    let Some(task) = tasks.iter_mut().find(|t| t.id == id) else {
        return Json(serde_json::json!({ "status": "error", "message": "任务不存在" }));
    };
    if let Some(v) = req.get("enabled").and_then(|v| v.as_bool()) {
        task.enabled = v;
        if v { task.next_run = compute_next_run(&task.schedule, now_ts(), task.last_run); }
    }
    if let Some(v) = req.get("name").and_then(|v| v.as_str()) {
        let n = v.trim().to_string();
        if n.is_empty() || n.chars().count() > 60 {
            return Json(serde_json::json!({ "status": "error", "message": "任务名需 1-60 字符" }));
        }
        task.name = n;
    }
    if let Some(v) = req.get("schedule") {
        if let Ok(s) = serde_json::from_value::<TaskSchedule>(v.clone()) {
            task.schedule = s;
            task.next_run = compute_next_run(&task.schedule, now_ts(), task.last_run);
        }
    }
    if let Some(v) = req.get("prompt").and_then(|v| v.as_str()) {
        if !v.trim().is_empty() && v.len() <= 8000 { task.prompt = v.trim().to_string(); }
    }
    let updated = task.clone();
    write_tasks(&path, &tasks);
    Json(serde_json::json!({ "status": "ok", "task": task_to_json(&updated) }))
}

/// DELETE /api/tasks/:id
pub async fn tasks_delete_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<CurrentUser>>,
    AxumPath(id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let path = tasks_file_for(&ctx, user.as_deref());
    let mut tasks = read_tasks(&path);
    let before = tasks.len();
    tasks.retain(|t| t.id != id);
    if tasks.len() == before {
        return Json(serde_json::json!({ "status": "error", "message": "任务不存在" }));
    }
    write_tasks(&path, &tasks);
    Json(serde_json::json!({ "status": "ok" }))
}

/// POST /api/tasks/:id/run — 手动立即执行（置 next_run=now，由调度循环拾起）
pub async fn tasks_run_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<CurrentUser>>,
    AxumPath(id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let path = tasks_file_for(&ctx, user.as_deref());
    let mut tasks = read_tasks(&path);
    let Some(task) = tasks.iter_mut().find(|t| t.id == id) else {
        return Json(serde_json::json!({ "status": "error", "message": "任务不存在" }));
    };
    task.next_run = Some(now_ts());
    task.enabled = true;
    write_tasks(&path, &tasks);
    Json(serde_json::json!({ "status": "ok", "message": "已加入执行队列" }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc as StdArc, RwLock as StdRwLock};

    fn test_ctx(dir: &std::path::Path) -> AppContext {
        AppContext {
            providers: std::collections::HashMap::new(),
            default_model: String::new(),
            user_data_dir: dir.join("users").join("local"),
            work_dir: dir.to_path_buf(),
            context_lengths: std::collections::HashMap::new(),
            image_dir: StdArc::new(StdRwLock::new(dir.to_path_buf())),
            todo_dir: StdArc::new(StdRwLock::new(dir.join("ToDoList"))),
            auth: None,
            server_mode: "standalone".into(),
            cloud_workdir: dir.join("CloudWork"),
        }
    }

    #[test]
    fn test_next_run_once() {
        let now = 1_000_000u64;
        let s = TaskSchedule::Once { at: now + 600 };
        assert_eq!(compute_next_run(&s, now, None), Some(now + 600));
        // 已执行过 → 不再触发
        assert_eq!(compute_next_run(&s, now, Some(now + 600)), None);
    }

    #[test]
    fn test_next_run_interval_progresses_and_catches_up() {
        let now = 1_000_000u64;
        let s = TaskSchedule::Interval { minutes: 5 };
        // 首次：now + 5min
        assert_eq!(compute_next_run(&s, now, None), Some(now + 300));
        // 离线错过多个周期 → 只追到未来最近一次
        let next = compute_next_run(&s, now, Some(now - 3600));
        assert!(next.unwrap() > now);
        let n = next.unwrap();
        assert!((n - now) <= 300, "补跑不应超过一个周期: {}", n - now);
    }

    #[test]
    fn test_next_run_daily() {
        let s = TaskSchedule::Daily { time: "23:59".into() };
        let next = compute_next_run(&s, now_ts(), None);
        assert!(next.is_some());
        assert!(next.unwrap() > now_ts() - 86400, "应在合理范围");
        let bad = TaskSchedule::Daily { time: "99:99".into() };
        assert_eq!(compute_next_run(&bad, now_ts(), None), None);
    }

    #[test]
    fn test_next_run_weekday() {
        let s = TaskSchedule::Weekly { weekdays: vec![1, 2, 3, 4, 5, 6, 7], time: "08:00".into() };
        let next = compute_next_run(&s, now_ts(), None);
        assert!(next.is_some());
        assert!(next.unwrap() > now_ts());
    }

    #[test]
    fn test_tasks_crud_and_isolation() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ctx = test_ctx(tmp.path());
        ctx.auth = Some(crate::auth::AuthState::new(tmp.path().join("auth")));
        let alice = CurrentUser { username: "alice".into(), role: "user".into() };

        // 写入 alice 的任务文件
        let path = tasks_file_for(&ctx, Some(&alice));
        let tasks = vec![ScheduledTask {
            id: "task_1".into(), name: "晨间简报".into(), prompt: "生成简报".into(),
            skill: Some("morning-brief".into()),
            schedule: TaskSchedule::Daily { time: "08:00".into() },
            enabled: true, created: 1, last_run: None, next_run: None,
            last_result: None, conversation_id: 111, run_count: 0,
        }];
        write_tasks(&path, &tasks);
        assert_eq!(path, tmp.path().join("users/alice/tasks/tasks.json"));
        assert_eq!(read_tasks(&path).len(), 1);

        // standalone 根目录
        ctx.auth = None;
        let root = tasks_file_for(&ctx, None);
        assert_eq!(root, tmp.path().join("users/local/tasks/tasks.json"));
        assert_eq!(read_tasks(&root).len(), 0);
    }

    #[tokio::test]
    async fn test_tick_executes_due_once_task() {
        // 到期 Once 任务：tick 后被执行（无模型 → 失败结果也记录），并停用
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        let now = now_ts();
        let task = ScheduledTask {
            id: "task_due".into(), name: "到期任务".into(), prompt: "hi".into(),
            skill: None, schedule: TaskSchedule::Once { at: now - 10 },
            enabled: true, created: now, last_run: None, next_run: Some(now - 10),
            last_result: None, conversation_id: 222, run_count: 0,
        };
        write_tasks(&tasks_file_for(&ctx, None), &[task]);
        tick(&ctx).await;
        let tasks = read_tasks(&tasks_file_for(&ctx, None));
        assert_eq!(tasks.len(), 1);
        assert!(!tasks[0].enabled, "Once 任务执行后应停用");
        assert!(tasks[0].last_run.is_some());
        assert!(tasks[0].last_result.is_some(), "失败也应有结果记录");
        assert!(tasks[0].last_result.as_ref().unwrap().contains("执行失败") || tasks[0].run_count == 1);
        // 会话文件已创建
        assert!(tmp.path().join("users/local/conversations").join("222.json").exists());
    }
}

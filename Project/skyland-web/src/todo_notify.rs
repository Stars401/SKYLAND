//! 待办通知引擎 — 服务进程内独立线程轮询今日清单，到期直接弹系统 Toast。
//!
//! 取代旧"前端 JS 引擎 + WebView 通知"链路（10 轮迭代不可用的根因见 native_notify.rs）。
//! 设计要点：
//! - 纯 std 线程 + 文件 IO，零 tokio 依赖（create_router 在 tokio::main 与
//!   Tauri 壳普通线程两种上下文被调用，本引擎不需要 async）；
//! - WebView 死活无关：页面关着、窗口藏进托盘，提醒照发；
//! - 触发语义与旧前端引擎（todo.js processItemNotifications）逐条对齐：
//!   notify_times 正点 + advance 提前 + due_date/due_time 兜底 + repeat 重复，
//!   last_notified 去重（谁先触发谁写回，前后端共享同一判据，天然不双发）；
//! - server 模式只发 admin 的提醒（Toast 只能弹在服务器桌面，发其他用户的
//!   私有事项到别人的屏幕既无意义又泄漏隐私）；LAN 远端用户由前端兜底引擎
//!   负责（todo.js 仅在 hostname 非 localhost 时武装）。

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use chrono::{Local, NaiveDate, NaiveTime, TimeZone};

use crate::api::AppContext;
use crate::auth::CurrentUser;
use crate::todo_api::{self, TodoItem};

static STARTED: OnceLock<()> = OnceLock::new();

/// 启动引擎（幂等）。client 模式不跑本地服务、不会走到这里；
/// standalone / server / 桌面壳统一经 create_router 装配。
pub fn start(ctx: AppContext) {
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(move || loop {
        tick(&ctx);
        std::thread::sleep(Duration::from_secs(20));
    });
}

fn tick(ctx: &AppContext) {
    let Some(dir) = notify_dir(ctx) else { return };
    let date = todo_api::today_str();
    let Some(mut list) = todo_api::read_list_in(&dir, &date) else { return };
    let now = now_secs();

    let mut fired = false;
    for item in list.items.iter_mut() {
        if !item.notify_enabled || item.done {
            continue;
        }
        if let Some(label) = due_label(item, &date, now) {
            let body = if label.is_empty() {
                item.title.clone()
            } else {
                format!("{}（{}）", item.title, label)
            };
            let (ok, via) = skyland_lib::native_notify::send("SKYLAND 待办提醒", &body);
            if ok {
                eprintln!("[待办提醒][{}] {}", via, body);
            } else {
                eprintln!("[待办提醒][失败-{}] {}", via, body);
            }
            // 无论 Toast 是否送达都推进 last_notified，避免失败重试风暴
            item.last_notified = Some(now);
            fired = true;
        }
    }
    if fired {
        todo_api::write_list_in(&dir, &date, &list);
    }
}

/// 引擎扫描目录：standalone → 待办根目录；server → admin 个人目录。
fn notify_dir(ctx: &AppContext) -> Option<PathBuf> {
    if ctx.auth.is_none() {
        return Some(todo_api::todo_dir_for_user(ctx, None));
    }
    let admin = admin_user(ctx)?;
    Some(todo_api::todo_dir_for_user(ctx, Some(&admin)))
}

/// server 模式下取 admin 身份（users.json 中 role == "admin" 的第一个）。
/// 尚未完成首启引导（无 admin）时返回 None — 引擎空转，引导完成即可用。
fn admin_user(ctx: &AppContext) -> Option<CurrentUser> {
    let auth = ctx.auth.as_ref()?;
    let users = auth.users.read().ok()?;
    let admin = users.iter().find(|u| u.role == "admin")?;
    Some(CurrentUser { username: admin.username.clone(), role: "admin".into() })
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    if h < 24 && m < 60 { Some((h, m)) } else { None }
}

/// 本地日期+时刻 → unix 秒（DST 歧义时刻取单值，解析失败返回 None）
fn local_ts(d: NaiveDate, h: u32, m: u32) -> Option<i64> {
    let t = NaiveTime::from_hms_opt(h, m, 0)?;
    let naive = d.and_time(t);
    Local.from_local_datetime(&naive).single().map(|dt| dt.timestamp())
}

/// 单条事项当前是否到期。返回 Toast 用的触发点标签。
/// 语义与旧前端引擎 todo.js#processItemNotifications 逐条对齐（见文件头注释）。
fn due_label(item: &TodoItem, date: &str, now: u64) -> Option<String> {
    let last = item.last_notified.unwrap_or(0) as i64;
    let now = now as i64;

    // 提醒时刻集合：notify_times + legacy 单值（去重）
    let mut times: Vec<String> = item.notify_times.clone();
    if let Some(t) = item.notify_time.as_deref() {
        if !t.is_empty() && !times.iter().any(|x| x == t) {
            times.push(t.to_string());
        }
    }
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;

    let mut sched: Vec<(i64, String)> = Vec::new();
    for t in &times {
        let Some((h, m)) = parse_hhmm(t) else { continue };
        let Some(ts) = local_ts(d, h, m) else { continue };
        sched.push((ts, t.clone()));
        for adv in &item.advance_minutes {
            sched.push((ts - *adv as i64 * 60, format!("{}·提前{}分钟", t, adv)));
        }
    }

    // 截止时间兜底触发（与既有触发点相距 <60s 视为重复，不双发）
    if let (Some(dd), Some(dt)) = (item.due_date.as_deref(), item.due_time.as_deref()) {
        if let (Some(dd), Some((h, m))) =
            (NaiveDate::parse_from_str(dd, "%Y-%m-%d").ok(), parse_hhmm(dt))
        {
            if let Some(ts) = local_ts(dd, h, m) {
                if !sched.iter().any(|(s, _)| (s - ts).abs() < 60) {
                    sched.push((ts, format!("截止 {}", dt)));
                }
            }
        }
    }

    if sched.is_empty() {
        return None;
    }
    sched.sort_by_key(|(ts, _)| *ts);

    // 到期窗口内取最晚触发点：正点提醒覆盖其提前提醒，不双发
    let mut trigger: Option<String> = None;
    for (ts, label) in &sched {
        if *ts <= now && *ts > last {
            trigger = Some(label.clone());
        }
    }
    if trigger.is_some() {
        return trigger;
    }

    // 全部计划时刻过后按间隔重复提醒，直到用户勾选完成
    if item.repeat_enabled && item.repeat_interval > 0 {
        let last_sched = sched[sched.len() - 1].0;
        if last_sched < now {
            let base = std::cmp::max(last, last_sched);
            if now - base >= item.repeat_interval as i64 * 60 {
                return Some(format!("重复提醒·每{}分钟", item.repeat_interval));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(times: Vec<&str>, advance: Vec<u32>, repeat: (bool, u32), last: Option<u64>) -> TodoItem {
        TodoItem {
            id: "t1".into(), title: "测试事项".into(), content: String::new(),
            done: false, expired: false, created: 0,
            notify_time: None, notify_enabled: true,
            notify_times: times.into_iter().map(String::from).collect(),
            advance_minutes: advance,
            repeat_enabled: repeat.0, repeat_interval: repeat.1,
            last_notified: last,
            priority: 1, category: None, due_date: None, due_time: None,
            sort_order: 0, visibility: "private".into(),
        }
    }

    fn ts(date: &str, h: u32, m: u32) -> u64 {
        let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
        local_ts(d, h, m).unwrap() as u64
    }

    #[test]
    fn fires_after_due_and_dedups_by_last_notified() {
        let due = ts("2026-09-17", 9, 0);
        let it = item(vec!["09:00"], vec![], (false, 0), None);
        // 未到期
        assert_eq!(due_label(&it, "2026-09-17", due - 1), None);
        // 到期触发
        assert_eq!(due_label(&it, "2026-09-17", due + 1).as_deref(), Some("09:00"));
        // 已通知过 → 不重发
        let it = item(vec!["09:00"], vec![], (false, 0), Some(due));
        assert_eq!(due_label(&it, "2026-09-17", due + 60), None);
    }

    #[test]
    fn advance_and_main_window_pick_latest_label() {
        let due = ts("2026-09-17", 9, 0);
        let it = item(vec!["09:00"], vec![30], (false, 0), None);
        // 提前 30 分钟窗口触发
        assert_eq!(due_label(&it, "2026-09-17", due - 1800 + 1).as_deref(),
                   Some("09:00·提前30分钟"));
        // 错过提前窗后（现在已过正点）触发 → 取窗口内最晚者 = 正点标签
        assert_eq!(due_label(&it, "2026-09-17", due + 1).as_deref(), Some("09:00"));
    }

    #[test]
    fn repeat_fires_on_interval_after_last_scheduled() {
        let due = ts("2026-09-17", 9, 0);
        let it = item(vec!["09:00"], vec![], (true, 10), Some(due));
        // 最后计划时刻 +10 分钟 → 触发重复
        assert!(due_label(&it, "2026-09-17", due + 601).unwrap().contains("重复提醒"));
        // 间隔未到 → 不触发
        assert_eq!(due_label(&it, "2026-09-17", due + 599), None);
    }

    #[test]
    fn due_datetime_is_fallback_trigger_with_dedup() {
        let due = ts("2026-09-17", 12, 0);
        let mut it = item(vec![], vec![], (false, 0), None);
        it.due_date = Some("2026-09-17".into());
        it.due_time = Some("12:00".into());
        assert_eq!(due_label(&it, "2026-09-17", due + 1).as_deref(), Some("截止 12:00"));
        // 与正点提醒相距 <60s → 被去重
        let mut it2 = item(vec!["12:00"], vec![], (false, 0), None);
        it2.due_date = Some("2026-09-17".into());
        it2.due_time = Some("12:00".into());
        assert_eq!(due_label(&it2, "2026-09-17", due + 1).as_deref(), Some("12:00"));
    }

    #[test]
    fn legacy_single_time_and_repeat_deduped() {
        let due = ts("2026-09-17", 8, 30);
        let mut it = item(vec!["08:30"], vec![], (false, 0), None);
        it.notify_time = Some("08:30".into()); // legacy 与数组重复 → 只发一次
        assert_eq!(due_label(&it, "2026-09-17", due + 1).as_deref(), Some("08:30"));
        // 非法时刻被丢弃
        let it = item(vec!["25:99"], vec![], (false, 0), None);
        assert_eq!(due_label(&it, "2026-09-17", due + 1), None);
    }
}

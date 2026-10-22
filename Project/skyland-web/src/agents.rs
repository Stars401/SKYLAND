//! 自定义子代理注册表 — UserData/agents/*.md（对齐 Claude Code 的 .claude/agents 目录）。
//! 文件形态与 SKILL.md 同构：YAML front matter（name / description / tools / rounds）
//! + Markdown 正文（该代理的附加系统提示）。
//! - name：类型名（小写字母数字-_），spawn_agent 的 subagent_type 直接引用
//! - description：适用场景（注入工具描述，模型据此选型）
//! - tools：read_file,glob,grep,web_search 的逗号子集；留空/缺省 = 全部只读工具
//! - rounds：轮次上限 1-20（默认 8）

use crate::api::AppContext;
use crate::auth::CurrentUser;
use axum::extract::{Extension, State};
use axum::Json;
use std::path::PathBuf;

#[derive(Clone)]
pub struct LoadedAgent {
    pub agent_type: String,
    pub description: String,
    pub tools: Vec<String>,
    pub max_rounds: usize,
    pub system_head: String,
}

const ALLOWED_TOOLS: &[&str] = &["read_file", "glob", "grep", "web_search"];
const MAX_AGENT_FILE_BYTES: usize = 512 * 1024;

pub fn agents_dir_for(ctx: &AppContext, user: Option<&CurrentUser>) -> PathBuf {
    crate::api::user_data_dir_for(ctx, user).join("agents")
}

pub fn agents_dir(ctx: &AppContext) -> PathBuf {
    agents_dir_for(ctx, None)
}

/// 扫描容器目录加载自定义代理（目录不存在/为空 → 空表；单个文件解析失败跳过不阻断）
pub fn load_custom_agents_for(ctx: &AppContext, user: Option<&CurrentUser>) -> Vec<LoadedAgent> {
    let dir = agents_dir_for(ctx, user);
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_file() || path.extension().map(|x| x == "md").unwrap_or(false) == false { continue; }
        let Ok(meta) = e.metadata() else { continue };
        if meta.len() as usize > MAX_AGENT_FILE_BYTES { continue; }
        let Ok(content) = std::fs::read_to_string(&path) else { continue };
        if let Some(agent) = parse_agent(&content) { out.push(agent); }
    }
    out
}

pub fn load_custom_agents(ctx: &AppContext) -> Vec<LoadedAgent> {
    load_custom_agents_for(ctx, None)
}

/// 解析单份代理定义：front matter 键值 + 正文；name 非法（空/含路径字符）视为无效
fn parse_agent(content: &str) -> Option<LoadedAgent> {
    let trimmed = content.trim_start();
    let rest = trimmed.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let fm = &rest[..end];
    let body = rest[end + 4..].trim().to_string();
    if body.is_empty() { return None; }

    let mut name: Option<String> = None;
    let mut description = String::new();
    let mut tools: Vec<String> = Vec::new();
    let mut rounds: usize = 8;
    for line in fm.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let k = k.trim();
        let v = v.trim().trim_matches('"').trim_matches('\'');
        match k {
            "name" => {
                let n = v.to_lowercase();
                if !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    name = Some(n);
                }
            }
            "description" => description = v.chars().take(200).collect(),
            "tools" => {
                tools = v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|t| ALLOWED_TOOLS.contains(&t.as_str()))
                    .collect();
            }
            "rounds" => {
                if let Ok(r) = v.parse::<usize>() { rounds = r.clamp(1, 20); }
            }
            _ => {}
        }
    }
    Some(LoadedAgent {
        agent_type: name?,
        description,
        tools,
        max_rounds: rounds,
        system_head: body,
    })
}

/// GET /api/agents — 自定义子代理清单（统一设置面板「AI 能力」页展示；顺带建目录便于用户放置文件）
pub async fn agents_list_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = agents_dir_for(&ctx, current_user);
    std::fs::create_dir_all(&dir).ok();
    let agents = load_custom_agents_for(&ctx, current_user);
    Json(serde_json::json!({
        "status": "ok",
        "dir": dir.display().to_string(),
        "agents": agents.iter().map(|a| serde_json::json!({
            "name": a.agent_type,
            "description": a.description,
            "tools": a.tools.iter().map(|t| t.as_str()).collect::<Vec<_>>().join(","),
            "rounds": a.max_rounds,
        })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_valid_and_invalid() {
        let md = "---\nname: Code-Reviewer\ndescription: 审查代码质量与安全隐患\ntools: read_file, glob\ncounds: 12\n---\n你是代码审查代理，逐条给出问题与建议。";
        // 注：上例 round 键名写错（counds）应被忽略 → 默认 8
        let a = parse_agent(md).expect("合法代理应解析成功");
        assert_eq!(a.agent_type, "code-reviewer");
        assert_eq!(a.tools, vec!["read_file".to_string(), "glob".to_string()]);
        assert_eq!(a.max_rounds, 8, "未知键忽略，rounds 用默认值");

        let md2 = "---\nname: ok\nrounds: 12\n---\n正文";
        let a2 = parse_agent(md2).unwrap();
        assert_eq!(a2.max_rounds, 12);
        assert!(a2.tools.is_empty(), "tools 缺省 = 全部只读工具");

        let md3 = "---\nrounds: 99\n---\n无 name";
        assert!(parse_agent(md3).is_none(), "缺 name 无效");
        let md4 = "---\nname: ../evil\n---\n路径字符非法";
        assert!(parse_agent(md4).is_none(), "name 含路径字符无效");
        let md5 = "---\nname: x\n---\n";
        assert!(parse_agent(md5).is_none(), "空正文无效");
    }

    #[test]
    fn tools_whitelist_filters_unknown() {
        let md = "---\nname: t\ntools: read_file, bash, web_search\n---\n正文";
        let a = parse_agent(md).unwrap();
        assert_eq!(a.tools, vec!["read_file".to_string(), "web_search".to_string()], "bash 不在只读白名单，必须被过滤");
    }

    #[test]
    fn agents_dir_is_isolated_by_current_user() {
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
        let alice = agents_dir_for(&ctx, Some(&crate::auth::CurrentUser {
            username: "alice".into(), role: "user".into(),
        }));
        let bob = agents_dir_for(&ctx, Some(&crate::auth::CurrentUser {
            username: "bob".into(), role: "user".into(),
        }));
        assert_eq!(alice, tmp.path().join("users/alice/agents"));
        assert_eq!(bob, tmp.path().join("users/bob/agents"));
    }
}

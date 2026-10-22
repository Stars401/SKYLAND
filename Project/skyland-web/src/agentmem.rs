//! 智能体记忆导入 — 扫描本机主流 AI 编码工具的配置/记忆文件，一键导入为 SKYLAND 技能
//! 支持：Claude Code（CLAUDE.md/settings.json）、OpenAI Codex CLI（config.toml/AGENTS.md）、
//!       Cursor（.cursorrules / rules 目录）、Gemini CLI（GEMINI.md）、通用 AGENTS.md。
//! 安全：仅允许导入"扫描结果白名单"内的路径（后端重新计算扫描集并匹配，防任意文件读取）。
//! 落点：UserData/skills/import-<agent>.md（与技能系统打通，use_skill 直接可用）。

use axum::{Json, extract::{Extension, State}};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::api::AppContext;
use crate::auth::CurrentUser;

#[derive(Serialize, Clone, Debug)]
pub struct MemoryEntry {
    pub agent: String,     // claude-code / codex / cursor / gemini-cli / common
    pub label: String,     // 显示名
    pub path: String,      // 绝对路径
    pub kind: String,      // memory(指令记忆) / config(配置) / skills(技能目录)
    pub exists: bool,
    pub size: u64,
    pub preview: String,   // 前 240 字符摘要（存在时）
}

/// 候选清单（存在才返回）
fn candidates(ctx: &AppContext) -> Vec<(String, String, String, String, String)> {
    // (agent, label, path, kind, skill_name)
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let home_p = PathBuf::from(&home);
    let wd = ctx.work_dir.clone();

    let mut v: Vec<(String, String, String, String, String)> = Vec::new();
    let mut push = |agent: &str, label: &str, p: PathBuf, kind: &str, skill: &str| {
        v.push((agent.into(), label.into(), p.to_string_lossy().to_string(), kind.into(), skill.into()));
    };

    if !home.is_empty() {
        push("claude-code", "Claude Code · 全局记忆 (CLAUDE.md)", home_p.join(".claude").join("CLAUDE.md"), "memory", "import-claude-memory");
        push("claude-code", "Claude Code · 全局设置 (settings.json)", home_p.join(".claude").join("settings.json"), "config", "import-claude-settings");
        push("claude-code", "Claude Code · 项目记忆 (工作目录 CLAUDE.md)", wd.join("CLAUDE.md"), "memory", "import-claude-project");
        push("codex", "OpenAI Codex CLI · 配置 (config.toml)", home_p.join(".codex").join("config.toml"), "config", "import-codex-config");
        push("codex", "OpenAI Codex CLI · 全局指令 (AGENTS.md)", home_p.join(".codex").join("AGENTS.md"), "memory", "import-codex-agents");
        push("cursor", "Cursor · 项目规则 (.cursorrules)", wd.join(".cursorrules"), "memory", "import-cursor-rules");
        push("gemini-cli", "Gemini CLI · 全局记忆 (GEMINI.md)", home_p.join(".gemini").join("GEMINI.md"), "memory", "import-gemini-memory");
    }
    push("common", "通用智能体规范 (AGENTS.md)", wd.join("AGENTS.md"), "memory", "import-agents-md");
    v
}

/// GET /api/memory/scan — 扫描本机智能体配置
pub async fn memory_scan_handler(
    State(ctx): State<AppContext>,
    headers: axum::http::HeaderMap,
) -> Json<serde_json::Value> {
    // 扫描目标是服务进程宿主的 Home 目录（常含 API key 的 CLI 配置）；
    // server 模式下对任意登录用户开放等于跨用户数据泄露 → 仅管理员
    if ctx.auth.is_some() && crate::auth::require_admin(&ctx, &headers).is_err() {
        return Json(serde_json::json!({ "status": "error", "message": "server 模式下仅管理员可扫描宿主智能体配置" }));
    }
    let entries: Vec<MemoryEntry> = candidates(&ctx).into_iter().map(|(agent, label, path, kind, _skill)| {
        let p = Path::new(&path);
        let (exists, size, preview) = match std::fs::metadata(p) {
            Ok(m) if m.is_file() => {
                let size = m.len();
                let preview = std::fs::read_to_string(p).ok()
                    .map(|c| c.chars().take(240).collect::<String>())
                    .unwrap_or_default();
                (true, size, preview)
            }
            _ => (false, 0, String::new()),
        };
        MemoryEntry { agent, label, path, kind, exists, size, preview }
    })
    .filter(|e| e.exists)
    .collect();
    Json(serde_json::json!({ "status": "ok", "entries": entries }))
}

#[derive(Deserialize)]
pub struct MemoryImportRequest {
    pub path: String,
}

const IMPORT_MAX_BYTES: u64 = 512 * 1024;

/// POST /api/memory/import — 白名单校验 + 转存为技能
pub async fn memory_import_handler(
    State(ctx): State<AppContext>,
    headers: axum::http::HeaderMap,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<MemoryImportRequest>,
) -> Json<serde_json::Value> {
    if ctx.auth.is_some() && crate::auth::require_admin(&ctx, &headers).is_err() {
        return Json(serde_json::json!({ "status": "error", "message": "server 模式下仅管理员可导入宿主智能体配置" }));
    }
    let current_user = user.as_ref().map(|value| &value.0);
    // 白名单：重新计算扫描集，仅允许其中的存在路径（防任意文件读取）
    let allowed: Vec<(String, String)> = candidates(&ctx).into_iter()
        .filter(|(_, _, path, _, _)| Path::new(path).is_file())
        .map(|(_, _, path, _, skill)| (path, skill))
        .collect();
    let Some((_, skill_name)) = allowed.iter().find(|(p, _)| p == &req.path) else {
        return Json(serde_json::json!({ "status": "error", "message": "路径不在可导入清单内（请先扫描后从结果中选择）" }));
    };
    let meta = std::fs::metadata(&req.path).ok();
    if let Some(m) = &meta {
        if m.len() > IMPORT_MAX_BYTES {
            return Json(serde_json::json!({ "status": "error", "message": "文件超过 512KB，请手工摘选后导入" }));
        }
    }
    let content = match std::fs::read_to_string(&req.path) {
        Ok(c) => c,
        Err(e) => return Json(serde_json::json!({ "status": "error", "message": format!("读取失败: {}", e) })),
    };

    // 技能名防碰撞：import-x / import-x-2 / import-x-3
    let skills_dir = crate::skills::skills_dir_for(&ctx, current_user);
    let mut final_name = skill_name.clone();
    let mut n = 2;
    while skills_dir.join(format!("{}.md", final_name)).exists() {
        final_name = format!("{}-{}", skill_name, n);
        n += 1;
        if n > 50 { return Json(serde_json::json!({ "status": "error", "message": "同名技能过多" })); }
    }

    let title = format!("导入 · {}", skill_name.trim_start_matches("import-").replace('-', " "));
    let skill_md = format!(
        "---\nname: {}\ntitle: {}\ndescription: 从本机智能体配置导入的记忆/规范（来源：{}）\n---\n\n{}",
        final_name, title, req.path, content.trim_end()
    );
    let target = skills_dir.join(format!("{}.md", final_name));
    std::fs::write(&target, &skill_md).map_err(|e| format!("写入失败: {}", e)).ok();

    Json(serde_json::json!({
        "status": "ok",
        "message": format!("已导入为技能 {}（{} 字符）", final_name, content.len()),
        "skill": final_name,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::AppContext;
    use std::sync::{Arc, RwLock};

    fn test_ctx(dir: &Path) -> AppContext {
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

    #[tokio::test]
    async fn test_scan_and_import_whitelist() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        // 工作目录里的 AGENTS.md 是候选之一 → 应被扫描发现
        std::fs::write(tmp.path().join("AGENTS.md"), "# AGENTS 规范\n Always reply in Chinese.").unwrap();

        let r = memory_scan_handler(State(ctx.clone()), axum::http::HeaderMap::new()).await;
        let body: serde_json::Value = r.0;
        let entries = body["entries"].as_array().unwrap();
        assert!(entries.iter().any(|e| e["path"].to_string().contains("AGENTS.md")), "AGENTS.md 应被发现");

        // 白名单内路径可导入为技能
        let req = MemoryImportRequest { path: tmp.path().join("AGENTS.md").to_string_lossy().to_string() };
        let r = memory_import_handler(State(ctx.clone()), axum::http::HeaderMap::new(), None, Json(req)).await;
        let body: serde_json::Value = r.0;
        assert_eq!(body["status"], "ok");
        let skill_name = body["skill"].as_str().unwrap().to_string();
        let saved = std::fs::read_to_string(tmp.path().join("skills").join(format!("{}.md", skill_name))).unwrap();
        assert!(saved.contains("Always reply in Chinese") && saved.contains("name:"));

        // 白名单外路径拒绝（防任意文件读取）
        let evil = tmp.path().join("secret.txt");
        std::fs::write(&evil, "top secret").unwrap();
        let req = MemoryImportRequest { path: evil.to_string_lossy().to_string() };
        let r = memory_import_handler(State(ctx), axum::http::HeaderMap::new(), None, Json(req)).await;
        let body: serde_json::Value = r.0;
        assert_eq!(body["status"], "error");
        assert!(body["message"].as_str().unwrap().contains("不在可导入清单"));
    }
}

//! Skill 系统 — 内置生活/工作技能 + 用户容器
//! 容器：{user_data_dir}/skills/，首启自动释放内置技能（include_str! 编译期嵌入）。
//! SKILL.md 形态：YAML front matter（name/title/description）+ Markdown 指令正文。
//! AI 通过 use_skill 工具读取正文并遵循；定时任务可关联 skill 在执行时注入。

use axum::{Json, extract::{Extension, State}};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::api::AppContext;
use crate::auth::CurrentUser;

/// 内置技能（编译期嵌入 — 单文件绿色部署也自带技能库）
pub const BUILTIN_SKILLS: &[(&str, &str)] = &[
    ("morning-brief", include_str!("skills_builtin/morning-brief.md")),
    ("daily-report", include_str!("skills_builtin/daily-report.md")),
    ("weekly-review", include_str!("skills_builtin/weekly-review.md")),
    ("meeting-notes", include_str!("skills_builtin/meeting-notes.md")),
    ("shopping-list", include_str!("skills_builtin/shopping-list.md")),
    ("translate-polish", include_str!("skills_builtin/translate-polish.md")),
    ("code-review", include_str!("skills_builtin/code-review.md")),
    ("doc-writer", include_str!("skills_builtin/doc-writer.md")),
    ("inbox-zero", include_str!("skills_builtin/inbox-zero.md")),
    ("learn-note", include_str!("skills_builtin/learn-note.md")),
    ("ai-draw", include_str!("skills_builtin/ai-draw.md")),
];

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SkillMeta {
    pub name: String,
    pub title: String,
    pub description: String,
    pub builtin: bool,
}

/// Skill 容器目录
pub fn skills_dir_for(ctx: &AppContext, user: Option<&CurrentUser>) -> PathBuf {
    let dir = crate::api::user_data_dir_for(ctx, user).join("skills");
    std::fs::create_dir_all(&dir).ok();
    dir
}

#[allow(dead_code)]
pub fn skills_dir(ctx: &AppContext) -> PathBuf {
    skills_dir_for(ctx, None)
}

/// 解析 SKILL.md 的 front matter + 正文
pub fn parse_skill(content: &str) -> Option<(SkillMeta, String)> {
    let trimmed = content.trim_start();
    let rest = trimmed.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let fm = &rest[..end];
    let body = rest[end + 4..].trim_start().to_string();

    let mut name = String::new();
    let mut title = String::new();
    let mut description = String::new();
    for line in fm.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") { name = v.trim().to_string(); }
        else if let Some(v) = line.strip_prefix("title:") { title = v.trim().to_string(); }
        else if let Some(v) = line.strip_prefix("description:") { description = v.trim().to_string(); }
    }
    if name.is_empty() { return None; }
    Some((SkillMeta { name, title, description, builtin: false }, body))
}

/// 技能升级账本 — 每个技能的全部历史发布版本（按发布顺序）：
/// - `*.md` = v0 初始版（首启释放）
/// - `*.r2.md` = 第二版（参数修正 + 日期 token 文档）
/// 容器内文件与任一历史版本逐字节一致 = 原封内置副本（用户未改动）→ 自动升级到最新；
/// 与全部历史版本有差异 = 用户改过 → 保持不动（用户优先）。
const LEGACY_SKILLS: &[(&str, &[&str])] = &[
    ("morning-brief", &[
        include_str!("skills_legacy/morning-brief.md"),
        include_str!("skills_legacy/morning-brief.r2.md"),
    ]),
    ("daily-report", &[
        include_str!("skills_legacy/daily-report.md"),
        include_str!("skills_legacy/daily-report.r2.md"),
    ]),
    ("weekly-review", &[
        include_str!("skills_legacy/weekly-review.md"),
        include_str!("skills_legacy/weekly-review.r2.md"),
    ]),
    ("meeting-notes", &[
        include_str!("skills_legacy/meeting-notes.md"),
        include_str!("skills_legacy/meeting-notes.r2.md"),
    ]),
    ("learn-note", &[
        include_str!("skills_legacy/learn-note.md"),
        include_str!("skills_legacy/learn-note.r2.md"),
    ]),
    ("translate-polish", &[
        include_str!("skills_legacy/translate-polish.md"),
        include_str!("skills_legacy/translate-polish.r2.md"),
    ]),
    ("code-review", &[
        include_str!("skills_legacy/code-review.md"),
        include_str!("skills_legacy/code-review.r2.md"),
    ]),
    ("doc-writer", &[
        include_str!("skills_legacy/doc-writer.md"),
        include_str!("skills_legacy/doc-writer.r2.md"),
    ]),
    ("inbox-zero", &[
        include_str!("skills_legacy/inbox-zero.md"),
        include_str!("skills_legacy/inbox-zero.r2.md"),
    ]),
    ("shopping-list", &[
        include_str!("skills_legacy/shopping-list.md"),
        include_str!("skills_legacy/shopping-list.r2.md"),
    ]),
];

/// 首启释放内置技能；已存在时仅在「与任一历史内置版完全一致」时升级（用户改动优先不覆盖）
pub fn ensure_builtin_skills_for(ctx: &AppContext, user: Option<&CurrentUser>) {
    let dir = skills_dir_for(ctx, user);
    for (name, content) in BUILTIN_SKILLS {
        let target = dir.join(format!("{}.md", name));
        if !target.exists() {
            let _ = std::fs::write(&target, content);
            continue;
        }
        if let Some((_, versions)) = LEGACY_SKILLS.iter().find(|(n, _)| n == name) {
            if let Ok(existing) = std::fs::read_to_string(&target) {
                if versions.iter().any(|old| existing == *old) {
                    let _ = std::fs::write(&target, content);
                }
            }
        }
    }
}

pub fn ensure_builtin_skills(ctx: &AppContext) {
    ensure_builtin_skills_for(ctx, None)
}

/// 列出全部技能（容器内；标注 builtin）
pub fn list_skills_for(ctx: &AppContext, user: Option<&CurrentUser>) -> Vec<SkillMeta> {
    let mut out = Vec::new();
    let dir = skills_dir_for(ctx, user);
    let builtin_names: Vec<&str> = BUILTIN_SKILLS.iter().map(|(n, _)| *n).collect();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Some((mut meta, _)) = parse_skill(&content) {
                    let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("").trim_end_matches(".md").to_string();
                    meta.builtin = builtin_names.contains(&fname.as_str());
                    out.push(meta);
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn list_skills(ctx: &AppContext) -> Vec<SkillMeta> {
    list_skills_for(ctx, None)
}

/// 读取技能正文（供 use_skill 工具 / 定时任务注入）。超长按字符截断。
pub fn read_skill_body_for(ctx: &AppContext, user: Option<&CurrentUser>, name: &str) -> Option<String> {
    // 名称安全：仅允许 [a-z0-9_-]
    if name.is_empty() || name.len() > 64
        || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        return None;
    }
    let path = skills_dir_for(ctx, user).join(format!("{}.md", name));
    let content = std::fs::read_to_string(path).ok()?;
    let (_, body) = parse_skill(&content)?;
    const MAX_CHARS: usize = 16 * 1024;
    Some(if body.chars().count() > MAX_CHARS {
        body.chars().take(MAX_CHARS).collect::<String>() + "\n\n... [技能内容已截断]"
    } else {
        body
    })
}

#[allow(dead_code)]
pub fn read_skill_body(ctx: &AppContext, name: &str) -> Option<String> {
    read_skill_body_for(ctx, None, name)
}

/// 技能名安全校验 — 与 read_skill_body_for 同规则（[a-z0-9_-]{1,64}）
fn valid_skill_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

const SKILL_DOC_MAX_BYTES: usize = 512 * 1024;

/// GET /api/skills/doc?name=xx — 技能 SKILL.md 原文（含 front matter）。
/// 知识库「技能库」视图用：用户可视化查看/编辑技能全文。
pub async fn skill_doc_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let Some(name) = q.get("name") else {
        return Json(serde_json::json!({ "status": "error", "message": "缺少 name 参数" }));
    };
    if !valid_skill_name(name) {
        return Json(serde_json::json!({ "status": "error", "message": "非法技能名" }));
    }
    let current_user = user.as_ref().map(|value| &value.0);
    let path = skills_dir_for(&ctx, current_user).join(format!("{}.md", name));
    match std::fs::read_to_string(&path) {
        Ok(content) => Json(serde_json::json!({ "status": "ok", "name": name, "content": content })),
        Err(_) => Json(serde_json::json!({ "status": "error", "message": "技能文件不存在" })),
    }
}

#[derive(serde::Deserialize)]
pub struct SkillDocSaveRequest {
    pub name: String,
    pub content: String,
}

/// POST /api/skills/doc — 保存技能原文（名称白名单 + 512KB 上限，落盘前校验路径）。
/// front matter 缺失不拒绝（允许草稿），但返回 warning 提示 use_skill 将不可用。
pub async fn skill_doc_save_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    Json(req): Json<SkillDocSaveRequest>,
) -> Json<serde_json::Value> {
    if !valid_skill_name(&req.name) {
        return Json(serde_json::json!({ "status": "error", "message": "非法技能名" }));
    }
    if req.content.len() > SKILL_DOC_MAX_BYTES {
        return Json(serde_json::json!({ "status": "error", "message": "内容超过 512KB 上限" }));
    }
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = skills_dir_for(&ctx, current_user);
    let path = dir.join(format!("{}.md", req.name));
    // 目录逃逸兜底：解析后必须仍在容器内
    let Ok(canon_dir) = dir.canonicalize() else {
        return Json(serde_json::json!({ "status": "error", "message": "技能容器不可用" }));
    };
    match path.canonicalize() {
        Ok(p) if !p.starts_with(&canon_dir) => {
            return Json(serde_json::json!({ "status": "error", "message": "路径越界" }));
        }
        _ => {}
    }
    let write = |bytes: &[u8]| -> Result<(), String> {
        let tmp = path.with_extension("md.tmp");
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    };
    if let Err(e) = write(req.content.as_bytes()) {
        return Json(serde_json::json!({ "status": "error", "message": format!("写入失败: {}", e) }));
    }
    let warning = if parse_skill(&req.content).is_some() {
        None
    } else {
        Some("缺少 front matter（--- 包裹的 name/title/description），use_skill 将无法识别该技能")
    };
    Json(serde_json::json!({ "status": "ok", "message": "技能已保存", "warning": warning }))
}

/// GET /api/skills/list — 技能清单（名称/标题/描述/是否内置）
pub async fn skills_list_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    ensure_builtin_skills_for(&ctx, current_user);
    Json(serde_json::json!({ "status": "ok", "skills": list_skills_for(&ctx, current_user) }))
}

/// GET /api/skills/dir — 容器路径（设置面板暴露用）
pub async fn skills_dir_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = skills_dir_for(&ctx, current_user);
    let display = dir.strip_prefix(&ctx.work_dir).ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| dir.to_string_lossy().replace('\\', "/"));
    Json(serde_json::json!({ "path": display, "directory": dir.display().to_string() }))
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn test_builtin_skills_embedded_and_valid() {
        assert!(BUILTIN_SKILLS.len() >= 10, "内置技能应不少于 10 个");
        for (name, content) in BUILTIN_SKILLS {
            let (meta, body) = parse_skill(content)
                .unwrap_or_else(|| panic!("内置技能 {} front matter 无效", name));
            assert_eq!(meta.name, *name);
            assert!(!meta.title.is_empty() && !meta.description.is_empty());
            assert!(body.len() > 100, "技能 {} 正文过短", name);
        }
    }

    #[test]
    fn test_ensure_release_and_list() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        ensure_builtin_skills(&ctx);
        ensure_builtin_skills(&ctx); // 幂等：二次释放不覆盖
        let skills = list_skills(&ctx);
        assert_eq!(skills.len(), BUILTIN_SKILLS.len());
        assert!(skills.iter().all(|s| s.builtin));
        // 用户修改不被覆盖
        let p = tmp.path().join("skills").join("daily-report.md");
        std::fs::write(&p, "user customized").unwrap();
        ensure_builtin_skills(&ctx);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "user customized");
    }

    #[test]
    fn test_ensure_upgrades_pristine_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        // 容器里放旧内置版（原封未动 — 逐版本测试：每个历史版本都应升级到最新）
        for (name, versions) in LEGACY_SKILLS {
            for (vi, old) in versions.iter().enumerate() {
                let dir = tmp.path().join("skills");
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(format!("{}.md", name)), old).unwrap();
                ensure_builtin_skills(&ctx);
                let now = std::fs::read_to_string(
                    dir.join(format!("{}.md", name))).unwrap();
                let latest = BUILTIN_SKILLS.iter().find(|(n, _)| n == name).unwrap().1;
                assert_ne!(now, *old, "{} 第{}个历史版本应已升级", name, vi);
                assert_eq!(now, latest, "{} 从第{}版升级后应为最新内置版", name, vi);
            }
        }
        // 用户在旧版基础上改过（哪怕一个字符）→ 不覆盖
        let p = tmp.path().join("skills").join("code-review.md");
        let mut user_touched = LEGACY_SKILLS.iter().find(|(n, _)| *n == "code-review")
            .unwrap().1.last().unwrap().to_string();
        user_touched.push_str("\n用户补充：团队规约另见 wiki");
        std::fs::write(&p, user_touched).unwrap();
        ensure_builtin_skills(&ctx);
        let now = std::fs::read_to_string(&p).unwrap();
        assert!(now.contains("团队规约另见 wiki"), "用户改动的技能不得被覆盖");
    }

    #[test]
    fn test_read_skill_rejects_bad_names() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path());
        assert!(read_skill_body(&ctx, "../config/llm.toml").is_none());
        assert!(read_skill_body(&ctx, "NonExistent").is_none());
        ensure_builtin_skills(&ctx);
        assert!(read_skill_body(&ctx, "daily-report").unwrap().contains("工作日报"));
    }

    #[test]
    fn skills_dir_is_isolated_by_current_user() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ctx = test_ctx(tmp.path());
        ctx.user_data_dir = tmp.path().join("users").join("local");
        ctx.auth = Some(crate::auth::AuthState::new(tmp.path().join("auth")));
        let alice = skills_dir_for(&ctx, Some(&crate::auth::CurrentUser {
            username: "alice".into(), role: "user".into(),
        }));
        let bob = skills_dir_for(&ctx, Some(&crate::auth::CurrentUser {
            username: "bob".into(), role: "user".into(),
        }));
        assert_eq!(alice, tmp.path().join("users/alice/skills"));
        assert_eq!(bob, tmp.path().join("users/bob/skills"));
    }
}

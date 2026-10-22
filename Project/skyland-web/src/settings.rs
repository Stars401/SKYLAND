use crate::api::AppContext;
use crate::auth::CurrentUser;
use axum::{Json, extract::{Extension, State}, http::StatusCode, response::{IntoResponse, Response}};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserSettings {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub appearance: BTreeMap<String, Value>,
    #[serde(default)]
    pub editor: BTreeMap<String, Value>,
    #[serde(default)]
    pub execution: BTreeMap<String, Value>,
    #[serde(default)]
    pub notifications: BTreeMap<String, Value>,
    #[serde(default)]
    pub paths: BTreeMap<String, Value>,
}

fn default_version() -> u32 { 1 }

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            version: default_version(),
            appearance: BTreeMap::new(),
            editor: BTreeMap::new(),
            execution: BTreeMap::new(),
            notifications: BTreeMap::new(),
            paths: BTreeMap::new(),
        }
    }
}

fn settings_path(ctx: &AppContext, user: Option<&CurrentUser>) -> PathBuf {
    crate::api::user_data_dir_for(ctx, user).join("settings.json")
}

pub fn load_user_settings(ctx: &AppContext, user: Option<&CurrentUser>) -> Result<UserSettings, String> {
    let path = settings_path(ctx, user);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).map_err(|e| format!("设置文件解析失败: {}", e)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(UserSettings::default()),
        Err(error) => Err(format!("读取设置失败: {}", error)),
    }
}

pub fn save_user_settings(ctx: &AppContext, user: Option<&CurrentUser>, settings: &UserSettings) -> Result<(), String> {
    let path = settings_path(ctx, user);
    let parent = path.parent().ok_or("设置路径无父目录")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("创建设置目录失败: {}", e))?;
    let mut normalized = settings.clone();
    normalized.version = default_version();
    let content = serde_json::to_string_pretty(&normalized).map_err(|e| format!("设置序列化失败: {}", e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("写入设置失败: {}", e))?;
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err("替换设置文件失败".into());
    }
    Ok(())
}

pub async fn settings_get_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
) -> Response {
    if ctx.auth.is_some() && user.is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    let current_user = user.as_ref().map(|value| &value.0);
    match load_user_settings(&ctx, current_user) {
        Ok(settings) => (StatusCode::OK, Json(settings)).into_response(),
        Err(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
    }
}

pub async fn settings_put_handler(
    State(ctx): State<AppContext>,
    user: Option<Extension<CurrentUser>>,
    Json(settings): Json<UserSettings>,
) -> Response {
    if ctx.auth.is_some() && user.is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    let current_user = user.as_ref().map(|value| &value.0);
    match save_user_settings(&ctx, current_user, &settings) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::AppContext;
    use crate::auth::CurrentUser;
    use std::sync::{Arc, RwLock};

    fn test_ctx(dir: &std::path::Path) -> AppContext {
        AppContext {
            providers: std::collections::HashMap::new(),
            default_model: String::new(),
            user_data_dir: dir.join("users").join("local"),
            work_dir: dir.to_path_buf(),
            context_lengths: std::collections::HashMap::new(),
            image_dir: Arc::new(RwLock::new(dir.join("users/local/images"))),
            todo_dir: Arc::new(RwLock::new(dir.join("users/local/todo"))),
            auth: None,
            server_mode: "server".into(),
            cloud_workdir: dir.join("users/local/workspaces/cloud"),
        }
    }

    #[test]
    fn user_settings_roundtrip_is_atomic_and_scoped() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ctx = test_ctx(tmp.path());
        ctx.auth = Some(crate::auth::AuthState::new(tmp.path().join("auth")));
        let user = CurrentUser { username: "alice".into(), role: "user".into() };
        let mut settings = UserSettings::default();
        settings.appearance.insert("theme".into(), serde_json::json!("dark"));
        save_user_settings(&ctx, Some(&user), &settings).unwrap();
        let loaded = load_user_settings(&ctx, Some(&user)).unwrap();
        assert_eq!(loaded.appearance["theme"], "dark");
        assert!(tmp.path().join("users/alice/settings.json").exists());
    }
}

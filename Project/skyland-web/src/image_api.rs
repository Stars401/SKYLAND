//! 图片生成 API 模块
//! 支持 ChatGPT-image2 (gpt-image-1) 和 NanoBanana 两种服务商
//! 配置持久化到 config/llm.toml，图片保存到 ImgGenerator/ 目录

use axum::{
    Json, extract::{State, Path as AxumPath},
    response::IntoResponse,
    http::{header, HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use crate::api::AppContext;
use crate::auth::CurrentUser;
use std::path::PathBuf;
use std::io::Write;
use base64::Engine as _;
use futures::StreamExt;

const MAX_IMAGE_BYTES: usize = 30 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DetectedImageFormat {
    mime: &'static str,
    extension: &'static str,
}

/// 图片生成请求
#[derive(Deserialize)]
pub struct ImageGenRequest {
    pub provider: String,
    pub prompt: String,
    pub size: Option<String>,
    pub quality: Option<String>,
    #[allow(dead_code)]
    pub style: Option<String>,
    pub reference_image: Option<String>, // base64 encoded
    pub n: Option<u32>,
    pub negative_prompt: Option<String>,
    pub seed: Option<i64>,
    #[allow(dead_code)]
    pub mode: Option<String>, // seamless, normal-map, specular-mask, white-model
}

/// 图片生成响应
#[derive(Serialize)]
pub struct ImageGenResponse {
    pub status: String,
    pub filename: Option<String>,
    pub url: Option<String>,
    pub message: Option<String>,
}

/// Provider 配置信息
#[derive(Serialize, Clone)]
pub struct ImageProviderInfo {
    pub name: String,
    pub api_key_masked: String,
    pub base_url: String,
    pub model: String,
    pub provider_type: String, // "chatgpt" or "nanobanana"
    pub is_full_url: bool,
}

/// Provider 保存请求
#[derive(Deserialize)]
pub struct ImageProviderSaveRequest {
    pub name: String,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub provider_type: String,
    pub is_full_url: Option<bool>,
}

/// 图片配置响应
#[derive(Serialize)]
pub struct ImageConfigResponse {
    pub providers: Vec<ImageProviderInfo>,
    /// 失败自动重试次数（[image_settings].retry_count，默认 5，0-10）
    pub retry_count: u32,
}

#[derive(Deserialize)]
pub struct ImageSaveRequest {
    pub source: String,
    pub folder: Option<String>,
}

#[derive(Deserialize)]
pub struct ImagePathSaveRequest {
    pub path: String,
    /// Only the Tauri desktop shell may set an absolute path. Standalone web
    /// mode remains restricted to project-relative directories.
    pub allow_external: Option<bool>,
}

/// 获取图片存储目录
fn img_dir(ctx: &AppContext) -> PathBuf {
    let full = ctx.image_dir.read()
        .map(|path| path.clone())
        .unwrap_or_else(|_| {
            ctx.user_data_dir.join("images")
        });
    std::fs::create_dir_all(&full).ok();
    full
}

pub(crate) fn img_dir_for(ctx: &AppContext, user: Option<&CurrentUser>) -> PathBuf {
    if ctx.auth.is_some() {
        if let Some(user) = user {
            let dir = crate::api::user_data_dir_for(ctx, Some(user)).join("images");
            std::fs::create_dir_all(&dir).ok();
            return dir;
        }
    }
    img_dir(ctx)
}

fn work_dir(ctx: &AppContext) -> PathBuf {
    ctx.work_dir.clone()
}

/// Resolve and create a directory only after checking the first existing
/// ancestor. This prevents a workspace symlink from redirecting creation
/// outside the workspace before the final canonical-path check.
fn ensure_workspace_dir(base: &std::path::Path, target: &std::path::Path) -> Result<PathBuf, String> {
    let base_canonical = std::fs::canonicalize(base)
        .map_err(|error| format!("无法访问工作目录: {}", error))?;
    let mut existing = target.to_path_buf();
    while !existing.exists() {
        if !existing.pop() {
            return Err("无法确定目标目录的父路径".into());
        }
    }
    let existing_canonical = std::fs::canonicalize(&existing)
        .map_err(|error| format!("无法访问目标目录的父路径: {}", error))?;
    if !existing_canonical.starts_with(&base_canonical) || !existing_canonical.is_dir() {
        return Err("目标目录必须位于工作目录内".into());
    }

    std::fs::create_dir_all(target)
        .map_err(|error| format!("创建目标目录失败: {}", error))?;
    let target_canonical = std::fs::canonicalize(target)
        .map_err(|error| format!("无法访问目标目录: {}", error))?;
    if !target_canonical.starts_with(&base_canonical) || !target_canonical.is_dir() {
        return Err("目标目录必须位于工作目录内".into());
    }
    Ok(target_canonical)
}

fn display_image_path(ctx: &AppContext) -> String {
    let dir = img_dir(ctx);
    let base = work_dir(ctx);
    dir.strip_prefix(&base)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| dir.to_string_lossy().replace('\\', "/"))
}

/// GET /api/image/path — return the effective image directory.
pub async fn image_path_handler(
    State(ctx): State<AppContext>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "path": display_image_path(&ctx),
        "directory": img_dir(&ctx).display().to_string(),
    }))
}

/// POST /api/image/path — update the image directory for this running app.
/// Standalone web mode accepts project-relative paths. The desktop shell can
/// additionally pass an absolute path after the native directory picker has
/// returned it.
pub async fn image_path_save_handler(
    State(ctx): State<AppContext>,
    headers: HeaderMap,
    Json(req): Json<ImagePathSaveRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if ctx.auth.is_some() && crate::auth::require_admin(&ctx, &headers).is_err() {
        return Err((StatusCode::FORBIDDEN, "server 模式下仅管理员可修改全局图片目录".into()));
    }
    let raw = req.path.trim();
    let relative = std::path::Path::new(raw);
    if raw.is_empty() || raw == "." || raw == ".." {
        return Err((StatusCode::BAD_REQUEST, "生图目录不能为空".into()));
    }

    if relative.is_absolute() {
        if req.allow_external != Some(true) || !crate::env_compat::desktop_mode() {
            return Err((StatusCode::BAD_REQUEST, "网页模式只能选择项目目录下的相对路径".into()));
        }
        let target = std::path::PathBuf::from(raw);
        let target_canonical = std::fs::canonicalize(&target).or_else(|_| {
            std::fs::create_dir_all(&target).and_then(|_| std::fs::canonicalize(&target))
        }).map_err(|error| {
            (StatusCode::BAD_REQUEST, format!("无法访问所选生图目录: {}", error))
        })?;
        if !target_canonical.is_dir() {
            return Err((StatusCode::BAD_REQUEST, "所选路径不是文件夹".into()));
        }
        let mut configured = ctx.image_dir.write()
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "生图目录配置锁不可用".into()))?;
        *configured = target_canonical.clone();
        return Ok(Json(serde_json::json!({
            "status": "ok",
            "path": target_canonical.display().to_string(),
            "directory": target_canonical.display().to_string(),
            "external": true,
        })));
    }

    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err((StatusCode::BAD_REQUEST, "生图目录不能包含 .、.. 或特殊路径前缀".into()));
        }
    }

    let base = work_dir(&ctx);
    let target = base.join(relative);
    let target_canonical = ensure_workspace_dir(&base, &target).map_err(|message| {
        let status = if message == "目标目录必须位于工作目录内" {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, message)
    })?;

    {
        let mut configured = ctx.image_dir.write()
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "生图目录配置锁不可用".into()))?;
        *configured = target_canonical.clone();
    }
    let display_path = display_image_path(&ctx);

    Ok(Json(serde_json::json!({
        "status": "ok",
        "path": display_path,
        "directory": target_canonical.display().to_string(),
    })))
}

/// 读取 llm.toml 配置路径
fn llm_config_path() -> Option<String> {
    // 桌面端启动注入的权威路径 — 无论文件是否已存在都直接采用，
    // 保证新建配置也落在正确位置（原实现回退到编译期烙死的
    // CARGO_MANIFEST_DIR 绝对路径，目标机器上不存在导致保存失败）
    if let Some(p) = crate::env_compat::config_path() {
        return Some(p);
    }
    // 网页模式：按相对路径查找既有配置（开发机 / 部署目录均可命中）
    let paths = [
        "config/llm.toml".to_string(),
        "../config/llm.toml".to_string(),
    ];
    for p in &paths {
        if std::fs::metadata(p).is_ok() {
            return Some(p.clone());
        }
    }
    // 无既有配置：在当前工作目录创建新配置（运行时相对路径，环境自适应）
    Some(paths[0].clone())
}

/// GET /api/image/config — 获取图片生成 provider 配置（密钥脱敏）
pub async fn image_config_handler() -> Json<ImageConfigResponse> {
    let mut providers = Vec::new();

    if let Some(path) = llm_config_path() {
        if let Ok(c) = std::fs::read_to_string(&path) {
            if let Ok(v) = c.parse::<toml::Value>() {
                if let Some(img_prs) = v.get("image_providers").and_then(|p| p.as_table()) {
                    for (name, cfg) in img_prs.iter() {
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
                        let model = cfg.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let ptype = cfg.get("provider_type").and_then(|v| v.as_str()).unwrap_or("chatgpt").to_string();
                        let is_full = cfg.get("is_full_url").and_then(|v| v.as_bool()).unwrap_or(false);

                        providers.push(ImageProviderInfo {
                            name: name.clone(),
                            api_key_masked: masked,
                            base_url,
                            model,
                            provider_type: ptype,
                            is_full_url: is_full,
                        });
                    }
                }
            }
        }
    }

    // 如果没有配置，返回默认预设
    if providers.is_empty() {
        providers.push(ImageProviderInfo {
            name: "chatgpt".into(),
            api_key_masked: "(未配置)".into(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-image-1".into(),
            provider_type: "chatgpt".into(),
            is_full_url: false,
        });
        providers.push(ImageProviderInfo {
            name: "nanobanana".into(),
            api_key_masked: "(未配置)".into(),
            base_url: "https://api.nanobanana.com/v1".into(),
            model: "nano-banana".into(),
            provider_type: "nanobanana".into(),
            is_full_url: false,
        });
        providers.push(ImageProviderInfo {
            name: "seedream".into(),
            api_key_masked: "(未配置)".into(),
            base_url: "https://ark.cn-beijing.volces.com".into(),
            model: "ep-xxxxxxxxxxxxxxxx".into(),
            provider_type: "volcengine_seedream".into(),
            is_full_url: false,
        });
        providers.push(ImageProviderInfo {
            name: "minimax".into(),
            api_key_masked: "(未配置)".into(),
            base_url: "https://api.minimaxi.com".into(),
            model: "image-01".into(),
            provider_type: "minimax".into(),
            is_full_url: false,
        });
    }

    Json(ImageConfigResponse { providers, retry_count: read_retry_count() })
}

/// POST /api/image/config/save — 保存图片 provider 配置
pub async fn image_config_save_handler(
    State(ctx): State<AppContext>,
    headers: HeaderMap,
    Json(req): Json<ImageProviderSaveRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if ctx.auth.is_some() && crate::auth::require_admin(&ctx, &headers).is_err() {
        return Err((StatusCode::FORBIDDEN, "server 模式下仅管理员可修改全局图片配置".into()));
    }
    if !matches!(req.provider_type.as_str(), "chatgpt" | "nanobanana" | "openai_compatible" | "volcengine_seedream" | "minimax") {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("暂不支持图片服务商协议类型: {}", req.provider_type),
        ));
    }
    let config_path = llm_config_path().ok_or((StatusCode::INTERNAL_SERVER_ERROR, "无法确定配置文件路径".to_string()))?;

    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut v: toml::Value = content.parse().unwrap_or(toml::Value::Table(toml::value::Table::new()));

    let provider_key = req.name.chars().map(|c| {
        if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }
    }).collect::<String>();

    let mut provider_table = toml::value::Table::new();
    // If api_key is empty, preserve existing key from current config
    let final_api_key = if req.api_key.is_empty() {
        // Read existing config to preserve the key
        let existing_key = v.get("image_providers")
            .and_then(|p| p.as_table())
            .and_then(|p| p.get(&provider_key))
            .and_then(|p| p.get("api_key"))
            .and_then(|k| k.as_str())
            .unwrap_or("");
        existing_key.to_string()
    } else {
        req.api_key.clone()
    };
    provider_table.insert("api_key".into(), toml::Value::String(final_api_key));
    provider_table.insert("base_url".into(), toml::Value::String(req.base_url.clone()));
    provider_table.insert("model".into(), toml::Value::String(req.model.clone()));
    provider_table.insert("provider_type".into(), toml::Value::String(req.provider_type.clone()));
    provider_table.insert("is_full_url".into(), toml::Value::Boolean(req.is_full_url.unwrap_or(false)));

    if let Some(table) = v.as_table_mut() {
        if let Some(img_providers) = table.get_mut("image_providers").and_then(|p| p.as_table_mut()) {
            img_providers.insert(provider_key.clone(), toml::Value::Table(provider_table));
        } else {
            let mut img_table = toml::value::Table::new();
            img_table.insert(provider_key.clone(), toml::Value::Table(provider_table));
            table.insert("image_providers".into(), toml::Value::Table(img_table));
        }
    }

    let new_content = toml::to_string(&v)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {}", e)))?;
    std::fs::write(&config_path, new_content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {}", e)))?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": format!("已保存图片生成配置: {}", provider_key)
    })))
}

#[derive(Deserialize)]
pub struct ImageSettingsSaveRequest {
    /// 失败自动重试次数（0-10）
    pub retry_count: i64,
}

/// POST /api/image/settings/save — 保存全局生图设置（失败自动重试次数）
pub async fn image_settings_save_handler(
    State(ctx): State<AppContext>,
    headers: HeaderMap,
    Json(req): Json<ImageSettingsSaveRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if ctx.auth.is_some() && crate::auth::require_admin(&ctx, &headers).is_err() {
        return Err((StatusCode::FORBIDDEN, "server 模式下仅管理员可修改全局图片设置".into()));
    }
    let rc = req.retry_count.clamp(0, 10);
    let config_path = llm_config_path()
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "无法确定配置文件路径".to_string()))?;
    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut v: toml::Value = content.parse().unwrap_or(toml::Value::Table(toml::value::Table::new()));
    let table = v.as_table_mut()
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "配置格式异常".to_string()))?;
    if !table.contains_key("image_settings") {
        table.insert("image_settings".to_string(), toml::Value::Table(toml::value::Table::new()));
    }
    let settings_ok = table.get_mut("image_settings")
        .and_then(|s| s.as_table_mut())
        .map(|st| st.insert("retry_count".into(), toml::Value::Integer(rc)))
        .is_some();
    if !settings_ok {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "image_settings 配置格式异常".into()));
    }
    let new_content = toml::to_string(&v)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {}", e)))?;
    std::fs::write(&config_path, new_content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {}", e)))?;
    Ok(Json(serde_json::json!({ "status": "ok", "retry_count": rc })))
}

/// POST /api/image/generate — 生成图片
pub async fn image_generate_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<CurrentUser>>,
    Json(req): Json<ImageGenRequest>,
) -> Result<Json<ImageGenResponse>, (StatusCode, String)> {
    let current_user = user.as_ref().map(|value| &value.0);
    let (filename, url, message) = generate_image_core_for(&ctx, current_user, &req).await?;
    Ok(Json(ImageGenResponse {
        status: "ok".into(),
        filename: Some(filename),
        url: Some(url),
        message,
    }))
}

/// 生图核心链（HTTP 端点与智能体 generate_image 工具共用）：
/// 校验 → 读服务商配置 → 失败自动重试环 → 写盘 → 元数据。
/// 返回 (文件名, 访问 URL, 提示消息)；错误带状态码供 HTTP 层透传。
#[allow(dead_code)]
pub async fn generate_image_core(
    ctx: &AppContext,
    req: &ImageGenRequest,
) -> Result<(String, String, Option<String>), (StatusCode, String)> {
    generate_image_core_for(ctx, None, req).await
}

pub async fn generate_image_core_for(
    ctx: &AppContext,
    user: Option<&CurrentUser>,
    req: &ImageGenRequest,
) -> Result<(String, String, Option<String>), (StatusCode, String)> {
    validate_image_request(req)?;
    // 从配置中读取 provider 信息
    let provider_info = read_provider_config(&req.provider)
        .ok_or((StatusCode::BAD_REQUEST, format!("Provider {} 未配置", req.provider)))?;

    if !matches!(
        provider_info.provider_type.as_str(),
        "chatgpt" | "nanobanana" | "openai_compatible" | "volcengine_seedream" | "minimax"
    ) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("暂不支持图片服务商协议类型: {}", provider_info.provider_type),
        ));
    }

    let api_key = if provider_info.api_key.starts_with("${") {
        std::env::var(&provider_info.api_key[2..provider_info.api_key.len()-1]).unwrap_or_default()
    } else {
        provider_info.api_key.clone()
    };

    if api_key.is_empty() {
        return Err((StatusCode::BAD_REQUEST, format!("Provider {} 的 API Key 未配置", req.provider)));
    }

    let size = req.size.clone().unwrap_or_else(|| "1024x1024".into());
    let quality = req.quality.clone().unwrap_or_else(|| "standard".into());
    let n = req.n.unwrap_or(1);

    // 按服务商分治的尺寸校验 — gpt-image-2 官方四约束（两边 16 倍数 / 最长边≤3840 /
    // 长短比≤3:1 / 总像素 655,360~8,294,400）；其余协议尺寸集未知，宽松放行由上游裁决
    if matches!(provider_info.provider_type.as_str(), "chatgpt" | "openai_compatible")
        && provider_info.model.to_lowercase().contains("gpt-image-2")
    {
        if let Some(err) = validate_gpt_image2_size(Some(size.as_str())) {
            return Err((StatusCode::BAD_REQUEST, err));
        }
    }

    // MiniMax 只支持比例参数 — 自定义像素尺寸动态映射到最近支持比例，并在响应中如实说明
    let mut success_message: Option<String> = None;
    if provider_info.provider_type == "minimax" {
        let s: &str = size.as_str();
        if !s.eq_ignore_ascii_case("auto") && !MINIMAX_DIRECT_SIZES.contains(&s) {
            success_message = Some(format!(
                "MiniMax 不支持自定义像素尺寸：已按最近比例 {} 生成（请求 {}）",
                minimax_aspect_ratio(s), s
            ));
        }
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("创建图片请求客户端失败: {}", e)))?;
    let base_url = provider_info.base_url.trim_end_matches('/');

    // Check if we have a reference image — if so, use /images/edits (multipart) instead of /images/generations (JSON)
    // This is critical: OpenAI's /images/generations endpoint silently ignores image fields.
    // The /images/edits endpoint accepts multipart/form-data with the image file.
    let has_ref_image = req.reference_image.as_ref().map(|s| !s.is_empty()).unwrap_or(false);

    if has_ref_image && matches!(provider_info.provider_type.as_str(), "volcengine_seedream" | "minimax") {
        return Err((
            StatusCode::BAD_REQUEST,
            "当前火山 Seedream 和 MiniMax 预设暂不支持节点参考图输入，请使用文本生图或 OpenAI 图片编辑协议".into(),
        ));
    }

    if has_ref_image && provider_info.provider_type == "nanobanana" {
        return Err((
            StatusCode::BAD_REQUEST,
            "当前 NanoBanana 配置仅支持文生图，不支持参考图编辑接口".into(),
        ));
    }

    // ─── 失败自动重试（[image_settings].retry_count，默认 5 次）───
    // 可重试：网络错误 / 5xx / 429 / 响应畸形 / 无图片数据 / 下载失败 / 字节校验失败
    // 不重试：400/401/403/404 等本端配置或参数错误（重试无意义，直接报给用户）
    let retry_count = read_retry_count() as usize;
    let mut attempt: usize = 0;
    let img_bytes: Result<Vec<u8>, (StatusCode, String)> = loop {
        attempt += 1;
        match generate_image_bytes(&ctx, user, &client, &provider_info, &api_key, &base_url, &req, &size, &quality, n, has_ref_image).await {
            Ok(bytes) => break Ok(bytes),
            Err(err) => {
                eprintln!("[image] 生成尝试 {}/{} 失败（{}）：{}",
                    attempt, retry_count + 1,
                    if err.retryable { "可重试" } else { "不可重试" }, err.message);
                if err.retryable && attempt <= retry_count {
                    let delay = std::cmp::min(1000u64 << ((attempt - 1) as u64).min(4), 16_000);
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    continue;
                }
                let msg = if attempt > 1 {
                    format!("{}（已尝试 {} 次均失败）", err.message, attempt)
                } else {
                    err.message
                };
                break Err((err.status_code, msg));
            }
        }
    };

    let img_bytes = img_bytes?;

    // Reserve the output name at the moment the bytes are written. A timestamp
    // alone is not unique when concurrent requests finish in the same millisecond.
    let dir = img_dir_for(&ctx, user);
    let preferred_name = format!("img_{}.png", unix_timestamp_millis());
    let filename = write_unique_image(&dir, &preferred_name, &img_bytes)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("文件写入失败: {}", e)))?;

    // 保存元数据
    save_metadata(&dir, &filename, &req.prompt, &req.provider, &size, &quality);

    let url = format!("/api/image/file/{}", filename);
    Ok((filename, url, success_message))
}

/// 单次生成尝试的错误 — retryable 标记是否值得自动重试
struct GenError {
    message: String,
    retryable: bool,
    status_code: StatusCode,
}

fn gen_err(message: String, retryable: bool, status_code: StatusCode) -> GenError {
    GenError { message, retryable, status_code }
}

/// 单次生成尝试：请求服务商 → 解析响应 → 提取图片字节（b64 解码或 URL 下载）→ 字节校验。
/// retryable 分类与外层重试环配套：网络/5xx/429/畸形/无数据等瞬时故障可重试；
/// 4xx 配置错误、非法参考图等本端参数错误不可重试。
#[allow(clippy::too_many_arguments)]
async fn generate_image_bytes(
    ctx: &AppContext,
    user: Option<&CurrentUser>,
    client: &reqwest::Client,
    provider_info: &ProviderConfig,
    api_key: &str,
    base_url: &str,
    req: &ImageGenRequest,
    size: &str,
    quality: &str,
    n: u32,
    has_ref_image: bool,
) -> Result<Vec<u8>, GenError> {
    let (status, resp_text) = if has_ref_image {
        // ─── Image Edit path: /images/edits with multipart/form-data ───
        let edit_url = image_endpoint_url(base_url, &provider_info.provider_type, provider_info.is_full_url, true);

        // Decode base64 image or load an image previously generated by SKYLAND.
        let ref_img = req.reference_image.as_ref().unwrap();
        let reference = reference_image_bytes(ctx, user, ref_img)
            .map_err(|(st, m)| gen_err(m, false, st))?;

        // Build multipart form
        let mut form = reqwest::multipart::Form::new()
            .text("model", provider_info.model.clone())
            .text("prompt", req.prompt.clone())
            .text("n", n.to_string())
            .text("size", size.to_string())
            .part("image", reqwest::multipart::Part::bytes(reference.bytes)
                .file_name(format!("input.{}", reference.format.extension))
                .mime_str(reference.format.mime)
                .map_err(|e| gen_err(format!("MIME错误: {}", e), false, StatusCode::INTERNAL_SERVER_ERROR))?);

        // Provider-specific form fields
        if matches!(provider_info.provider_type.as_str(), "chatgpt" | "openai_compatible") {
            form = form.text("quality", quality.to_string());
        }
        if let Some(ref np) = req.negative_prompt {
            if !np.is_empty() {
                form = form.text("negative_prompt", np.clone());
            }
        }
        if let Some(seed) = req.seed {
            if seed >= 0 {
                form = form.text("seed", seed.to_string());
            }
        }

        let resp = client
            .post(&edit_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| gen_err(format!("图片编辑请求失败: {}", e), true, StatusCode::BAD_GATEWAY))?;

        let st = resp.status();
        let rt = resp.text().await.unwrap_or_default();
        (st, rt)
    } else {
        // ─── Text-to-Image path: /images/generations with JSON ───
        // 端点 URL 统一由 image_endpoint_url 解析（完整 URL 模式下原样使用，不二次拼接）
        let url = image_endpoint_url(base_url, &provider_info.provider_type, provider_info.is_full_url, false);
        let (url, body) = match provider_info.provider_type.as_str() {
            "nanobanana" => {
                let body = serde_json::json!({
                    "model": provider_info.model,
                    "prompt": req.prompt,
                    "n": n,
                    "size": size,
                });
                (url, body)
            }
            "volcengine_seedream" => {
                let body = serde_json::json!({
                    "model": provider_info.model,
                    "prompt": req.prompt,
                    "sequential_image_generation": "disabled",
                    "response_format": "url",
                    "size": size,
                    "watermark": false,
                });
                (url, body)
            }
            "minimax" => {
                let body = serde_json::json!({
                    "model": provider_info.model,
                    "prompt": req.prompt,
                    "response_format": "url",
                    "aspect_ratio": minimax_aspect_ratio(size),
                });
                (url, body)
            }
            "chatgpt" | "openai_compatible" => {
                // ChatGPT-image2 (gpt-image-1)
                let mut body = serde_json::json!({
                    "model": provider_info.model,
                    "prompt": req.prompt,
                    "n": n,
                    "size": size,
                    "quality": quality,
                });
                if let Some(ref np) = req.negative_prompt {
                    if !np.is_empty() {
                        body["negative_prompt"] = serde_json::Value::String(np.clone());
                    }
                }
                if let Some(seed) = req.seed {
                    if seed >= 0 {
                        body["seed"] = serde_json::Value::Number(seed.into());
                    }
                }
                (url, body)
            }
            _ => {
                return Err(gen_err(
                    format!("暂不支持图片服务商协议类型: {}", provider_info.provider_type),
                    false,
                    StatusCode::BAD_REQUEST,
                ));
            }
        };

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| gen_err(format!("请求失败: {}", e), true, StatusCode::BAD_GATEWAY))?;

        let st = resp.status();
        let rt = resp.text().await.unwrap_or_default();
        (st, rt)
    };

    if !status.is_success() {
        // Truncate and clean error message — don't send raw HTML to frontend
        let code = status.as_u16();
        // 4xx 客户端/配置类错误重试无意义；5xx / 429 / 408 属瞬时故障可重试
        let retryable = code >= 500 || code == 429 || code == 408;
        let clean_msg = if resp_text.starts_with("<!") || resp_text.starts_with("<html") || resp_text.starts_with("<HTML") {
            format!("API 返回了 HTML 页面而非 JSON（状态 {}）。请检查 Base URL 是否正确（需包含 /v1），以及 API Key 是否有效。", status)
        } else {
            resp_text.chars().take(300).collect::<String>()
        };
        return Err(gen_err(format!("API 返回 {}: {}", status, clean_msg), retryable, StatusCode::BAD_GATEWAY));
    }

    // 解析响应
    let resp_json: serde_json::Value = serde_json::from_str(&resp_text)
        .map_err(|e| {
            let clean_msg = if resp_text.starts_with("<!") || resp_text.starts_with("<html") || resp_text.starts_with("<HTML") {
                "API 返回了 HTML 页面而非 JSON。请检查 Base URL 配置（需包含 /v1 后缀）。".to_string()
            } else {
                format!("{} — {}", e, resp_text.chars().take(200).collect::<String>())
            };
            // 响应畸形多为中转服务瞬时抖动 — 可重试
            gen_err(clean_msg, true, StatusCode::BAD_GATEWAY)
        })?;

    // 尝试从 response 中提取图片数据（b64_json / url / image_urls 三种形态）
    let image_url: Option<String>;
    let image_b64: Option<String>;

    if let Some(data_arr) = resp_json["data"].as_array() {
        if let Some(first) = data_arr.first() {
            // gpt-image-1 返回 b64_json
            image_b64 = first["b64_json"].as_str().map(String::from);
            image_url = first["url"].as_str().map(String::from);
        } else {
            image_b64 = None;
            image_url = None;
        }
    } else if let Some(urls) = resp_json["data"]["image_urls"].as_array() {
        image_b64 = None;
        image_url = urls.first().and_then(|value| value.as_str()).map(String::from);
    } else if let Some(urls) = resp_json["image_urls"].as_array() {
        image_b64 = None;
        image_url = urls.first().and_then(|value| value.as_str()).map(String::from);
    } else {
        image_b64 = resp_json["b64_json"].as_str().map(String::from);
        image_url = resp_json["url"].as_str().map(String::from);
    }

    if let Some(b64) = image_b64 {
        // 去除可能的 data:image/png;base64, 前缀
        let clean_b64 = if b64.starts_with("data:") {
            b64.split(',').nth(1).unwrap_or(&b64).to_string()
        } else {
            b64
        };
        let img_bytes = base64::engine::general_purpose::STANDARD.decode(&clean_b64)
            .map_err(|e| gen_err(format!("Base64 解码失败: {}", e), true, StatusCode::BAD_GATEWAY))?;
        validate_generated_image_bytes(&img_bytes)
            .map_err(|(st, m)| gen_err(format!("返回内容不是有效图片: {}", m), true, st))?;
        Ok(img_bytes)
    } else if let Some(url) = image_url {
        // 下载图片 URL
        validate_image_download_url(&url)
            .map_err(|(st, m)| gen_err(m, false, st))?;
        let img_resp = client.get(&url).send().await
            .map_err(|e| gen_err(format!("下载图片失败: {}", e), true, StatusCode::BAD_GATEWAY))?;
        let img_bytes = read_downloaded_image(img_resp).await
            .map_err(|(st, m)| gen_err(m, true, st))?;
        Ok(img_bytes)
    } else {
        Err(gen_err(
            format!("API 响应中未找到图片数据: {}", resp_text.chars().take(300).collect::<String>()),
            true,
            StatusCode::BAD_GATEWAY,
        ))
    }
}

/// MiniMax 支持比例的完整集合（预设映射表 + 常见比例），自定义尺寸就近匹配用
const MINIMAX_RATIO_SET: &[(&str, f64)] = &[
    ("1:1", 1.0), ("2:3", 2.0 / 3.0), ("3:2", 3.0 / 2.0),
    ("3:5", 3.0 / 5.0), ("5:3", 5.0 / 3.0),
    ("4:3", 4.0 / 3.0), ("3:4", 3.0 / 4.0),
    ("9:16", 9.0 / 16.0), ("16:9", 16.0 / 9.0),
];

/// 预设尺寸 → MiniMax 比例的精确映射（保持既有行为不变）
const MINIMAX_DIRECT_SIZES: &[&str] = &[
    "1024x1024", "1024x1536", "768x1152", "1536x1024", "1152x768",
    "1024x1792", "768x1344", "1792x1024", "1344x768",
];

/// size → MiniMax aspect_ratio：预设精确映射；自定义尺寸按宽高比就近匹配
/// （旧实现对未知尺寸静默兜底 "1:1"，用户会被误导；现在映射结果会随响应 message 告知）
fn minimax_aspect_ratio(size: &str) -> String {
    match size {
        "1024x1024" => return "1:1".into(),
        "1024x1536" | "768x1152" => return "2:3".into(),
        "1536x1024" | "1152x768" => return "3:2".into(),
        "1024x1792" | "768x1344" => return "3:5".into(),
        "1792x1024" | "1344x768" => return "5:3".into(),
        _ => {}
    }
    let parsed = size.split_once('x').or_else(|| size.split_once('X'))
        .and_then(|(w, h)| Some((w.trim().parse::<f64>().ok()?, h.trim().parse::<f64>().ok()?)));
    let Some((w, h)) = parsed else { return "1:1".into() };
    if w <= 0.0 || h <= 0.0 { return "1:1".into(); }
    let target = w / h;
    let mut best = MINIMAX_RATIO_SET[0];
    let mut best_d = (target / best.1).ln().abs();
    for cand in MINIMAX_RATIO_SET.iter().skip(1) {
        let d = (target / cand.1).ln().abs();
        if d < best_d {
            best = *cand;
            best_d = d;
        }
    }
    best.0.into()
}

/// gpt-image-2 官方尺寸约束校验（官方称支持"千级分辨率"，规则为）：
/// 两边均为 16px 倍数 / 最长边 ≤3840px / 长短边比 ≤3:1 / 总像素 655,360 ~ 8,294,400
/// 返回 Some(错误文案) = 不合法；None = 合法（或 auto）
fn validate_gpt_image2_size(size: Option<&str>) -> Option<String> {
    let Some(s) = size else { return None };
    if s.eq_ignore_ascii_case("auto") { return None; }
    let Some((w, h)) = s.split_once('x').or_else(|| s.split_once('X'))
        .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))) else {
        return Some(format!("尺寸格式无效：{}（须为 auto 或 WIDTHxHEIGHT）", s));
    };
    if w % 16 != 0 || h % 16 != 0 {
        return Some(format!("gpt-image-2 要求宽高均为 16 的倍数，{}x{} 不满足（如 2048x2048、2048x1152）", w, h));
    }
    if w.max(h) > 3840 {
        return Some(format!("gpt-image-2 最长边不能超过 3840px，得到 {}x{}", w, h));
    }
    let (long, short) = (w.max(h), w.min(h));
    if long > short * 3 {
        return Some(format!("gpt-image-2 长短边比不能超过 3:1，{}x{} 不满足", w, h));
    }
    let total = w as u64 * h as u64;
    if !(655_360..=8_294_400).contains(&total) {
        return Some(format!("gpt-image-2 总像素须在 655,360 ~ 8,294,400 之间，{}x{} 为 {} 像素", w, h, total));
    }
    None
}

/// 读取 [image_settings].retry_count — 失败自动重试次数，默认 5，钳制 0-10
fn read_retry_count() -> u32 {
    let Some(path) = llm_config_path() else { return 5 };
    let Ok(content) = std::fs::read_to_string(&path) else { return 5 };
    let Ok(v) = content.parse::<toml::Value>() else { return 5 };
    v.get("image_settings")
        .and_then(|s| s.get("retry_count"))
        .and_then(|v| v.as_integer())
        .map(|n| n.clamp(0, 10) as u32)
        .unwrap_or(5)
}

/// GET /api/image/history — 列出已生成图片
fn validate_image_request(req: &ImageGenRequest) -> Result<(), (StatusCode, String)> {
    const MAX_PROMPT_CHARS: usize = 100_000;
    if req.prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err((StatusCode::BAD_REQUEST, "提示词长度不能超过 100000 个字符".into()));
    }
    if let Some(value) = req.negative_prompt.as_deref() {
        if value.chars().count() > MAX_PROMPT_CHARS {
            return Err((StatusCode::BAD_REQUEST, "负面提示词长度不能超过 100000 个字符".into()));
        }
    }
    if let Some(size) = req.size.as_deref() {
        let valid_size = size.eq_ignore_ascii_case("auto") || size
            .split_once('x').or_else(|| size.split_once('X'))
            .and_then(|(width, height)| Some((width.parse::<u32>().ok()?, height.parse::<u32>().ok()?)))
            .map(|(width, height)| (1..=8192).contains(&width) && (1..=8192).contains(&height))
            .unwrap_or(false);
        if !valid_size {
            return Err((StatusCode::BAD_REQUEST, "图片尺寸必须是 auto 或 WIDTHxHEIGHT（每边 1-8192）".into()));
        }
    }
    if let Some(quality) = req.quality.as_deref() {
        if !matches!(quality, "standard" | "hd") {
            return Err((StatusCode::BAD_REQUEST, "图片质量只能是 standard 或 hd".into()));
        }
    }
    if let Some(n) = req.n {
        if n != 1 {
            return Err((StatusCode::BAD_REQUEST, "当前节点生图流程一次只支持生成 1 张图片".into()));
        }
    }
    if let Some(seed) = req.seed {
        if seed < 0 {
            return Err((StatusCode::BAD_REQUEST, "seed 必须是非负整数".into()));
        }
    }
    Ok(())
}

fn validate_image_download_url(raw: &str) -> Result<(), (StatusCode, String)> {
    let url = reqwest::Url::parse(raw)
        .map_err(|_| (StatusCode::BAD_GATEWAY, "服务商返回了无效的图片地址".into()))?;
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
        return Err((StatusCode::BAD_GATEWAY, "服务商返回的图片地址协议不受支持".into()));
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host == "metadata.google.internal" {
        return Err((StatusCode::BAD_GATEWAY, "服务商返回的图片地址不允许访问本机资源".into()));
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let blocked = match ip {
            std::net::IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified() || ip.is_multicast(),
            std::net::IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_unspecified() || ip.is_multicast(),
        };
        if blocked {
            return Err((StatusCode::BAD_GATEWAY, "服务商返回的图片地址不允许访问内网资源".into()));
        }
    }
    Ok(())
}

pub async fn image_history_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = img_dir_for(&ctx, current_user);
    let mut images = Vec::new();

    // 读取元数据
    let metadata = read_metadata(&dir);

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // 跳过隐藏文件和元数据文件
            if name.starts_with('.') { continue; }
            // 只列出图片文件
            let lower = name.to_lowercase();
            if !lower.ends_with(".png") && !lower.ends_with(".jpg") && !lower.ends_with(".jpeg") && !lower.ends_with(".webp") {
                continue;
            }
            let meta = metadata.iter().find(|m| m.filename == name);
            images.push(serde_json::json!({
                "filename": name,
                "url": format!("/api/image/file/{}", name),
                "prompt": meta.map(|m| m.prompt.as_str()).unwrap_or(""),
                "provider": meta.map(|m| m.provider.as_str()).unwrap_or(""),
                "size": meta.map(|m| m.size.as_str()).unwrap_or(""),
                "timestamp": meta.map(|m| m.timestamp).unwrap_or(0),
            }));
        }
    }

    // 按时间倒序
    images.sort_by(|a, b| {
        let ta = a["timestamp"].as_i64().unwrap_or(0);
        let tb = b["timestamp"].as_i64().unwrap_or(0);
        tb.cmp(&ta)
    });

    Json(serde_json::json!({ "images": images }))
}

/// GET /api/image/file/:filename — 提供图片文件
pub async fn image_file_handler(
    State(ctx): State<AppContext>,
    AxumPath(filename): AxumPath<String>,
    user: Option<axum::Extension<CurrentUser>>,
) -> impl IntoResponse {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = img_dir_for(&ctx, current_user);

    // 安全检查：防止路径遍历
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return (StatusCode::BAD_REQUEST, "无效的文件名").into_response();
    }

    let filepath = dir.join(&filename);
    if !filepath.exists() {
        return (StatusCode::NOT_FOUND, "文件不存在").into_response();
    }

    match std::fs::read(&filepath) {
        Ok(bytes) => {
            let content_type = if filename.ends_with(".png") {
                "image/png"
            } else if filename.ends_with(".jpg") || filename.ends_with(".jpeg") {
                "image/jpeg"
            } else if filename.ends_with(".webp") {
                "image/webp"
            } else if filename.ends_with(".gif") {
                "image/gif"
            } else {
                "application/octet-stream"
            };
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, content_type)],
                bytes,
            ).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {}", e)).into_response(),
    }
}

/// DELETE /api/image/file/:filename — 删除图片
pub async fn image_delete_handler(
    State(ctx): State<AppContext>,
    AxumPath(filename): AxumPath<String>,
    user: Option<axum::Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = img_dir_for(&ctx, current_user);

    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return Json(serde_json::json!({ "status": "error", "message": "无效的文件名" }));
    }

    let filepath = dir.join(&filename);
    if filepath.exists() {
        let _ = std::fs::remove_file(&filepath);
    }

    // 从元数据中删除
    remove_metadata(&dir, &filename);

    Json(serde_json::json!({ "status": "ok" }))
}

/// POST /api/image/save - copy a generated/local image into a workspace folder.
///
/// The browser is only allowed to reference images already owned by this
/// application (`/api/image/file/:filename`) or an in-memory data URL. Folder
/// names are single path components under the workspace directory, so this
/// endpoint cannot be used for path traversal or arbitrary file writes.
pub async fn image_save_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<CurrentUser>>,
    Json(req): Json<ImageSaveRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let current_user = user.as_ref().map(|value| &value.0);
    let source = req.source.trim();
    if source.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "图片来源不能为空".into()));
    }

    let folder = req.folder.as_deref().unwrap_or("").trim();
    if folder == "." || folder == ".." ||
        folder.contains("..") || folder.contains('/') || folder.contains('\\') ||
        std::path::Path::new(folder).is_absolute() {
        return Err((StatusCode::BAD_REQUEST, "文件夹必须是工作目录下的单级目录名".into()));
    }

    let image_root = img_dir_for(&ctx, current_user);
    let target_dir = if folder.is_empty() {
        image_root.clone()
    } else {
        image_root.join(folder)
    };
    let target_canonical = ensure_workspace_dir(&image_root, &target_dir).map_err(|message| {
        let status = if message == "目标目录必须位于工作目录内" {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, message)
    })?;

    let (bytes, preferred_name): (Vec<u8>, String) = if let Some(data) = source.strip_prefix("data:") {
        let (header, encoded) = data.split_once(',')
            .ok_or((StatusCode::BAD_REQUEST, "无效的 data 图片".into()))?;
        if !header.to_ascii_lowercase().contains(";base64") {
            return Err((StatusCode::BAD_REQUEST, "仅支持 base64 图片 data URL".into()));
        }
        let mime = header.split(';').next().unwrap_or("").to_ascii_lowercase();
        let extension = image_extension_from_mime(&mime)
            .ok_or((StatusCode::BAD_REQUEST, "不支持的图片格式".into()))?;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|_| (StatusCode::BAD_REQUEST, "图片 base64 解码失败".into()))?;
        if decoded.is_empty() || decoded.len() > MAX_IMAGE_BYTES {
            return Err((StatusCode::BAD_REQUEST, "图片为空或超过 30MB".into()));
        }
        (decoded, format!("image_{}.{}", unix_timestamp_millis(), extension))
    } else {
        let prefix = "/api/image/file/";
        let filename = source.strip_prefix(prefix).unwrap_or(source);
        if filename.is_empty() || filename.contains("..") || filename.contains('/') || filename.contains('\\') {
            return Err((StatusCode::BAD_REQUEST, "图片来源必须是本地图片文件".into()));
        }
        let source_path = image_root.join(filename);
        let source_canonical = std::fs::canonicalize(&source_path)
            .map_err(|_| (StatusCode::NOT_FOUND, "源图片不存在".into()))?;
        let root_canonical = std::fs::canonicalize(&image_root)
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "无法访问图片目录".into()))?;
        if !source_canonical.starts_with(&root_canonical) || !source_canonical.is_file() {
            return Err((StatusCode::BAD_REQUEST, "非法的图片来源".into()));
        }
        let name = source_canonical.file_name()
            .and_then(|value| value.to_str())
            .ok_or((StatusCode::BAD_REQUEST, "图片文件名无效".into()))?;
        let extension = source_canonical.extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .filter(|value| matches!(value.as_str(), "png" | "jpg" | "jpeg" | "webp"))
            .ok_or((StatusCode::BAD_REQUEST, "仅支持 png、jpg、jpeg、webp 图片".into()))?;
        let data = std::fs::read(&source_canonical)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("读取源图片失败: {}", e)))?;
        if data.is_empty() || data.len() > MAX_IMAGE_BYTES {
            return Err((StatusCode::BAD_REQUEST, "图片为空或超过 30MB".into()));
        }
        (data, format_image_name(name, &extension))
    };

    let output_name = write_unique_image(&target_canonical, &preferred_name, &bytes)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("保存图片失败: {}", e)))?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "filename": output_name,
        "folder": folder,
        "root": image_root.display().to_string(),
        "directory": target_canonical.display().to_string()
    })))
}

struct ReferenceImage {
    bytes: Vec<u8>,
    format: DetectedImageFormat,
}

fn reference_image_bytes(
    ctx: &AppContext,
    user: Option<&CurrentUser>,
    source: &str,
) -> Result<ReferenceImage, (StatusCode, String)> {
    let source = source.trim();
    let bytes = if let Some(data) = source.strip_prefix("data:") {
        let (header, encoded) = data
            .split_once(',')
            .ok_or((StatusCode::BAD_REQUEST, "无效的参考图片 data URL".into()))?;
        if !header.to_ascii_lowercase().contains(";base64") {
            return Err((StatusCode::BAD_REQUEST, "参考图片必须使用 base64 编码".into()));
        }
        base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|_| (StatusCode::BAD_REQUEST, "参考图片Base64解码失败".into()))?
    } else if let Some(filename) = source.strip_prefix("/api/image/file/") {
        if filename.is_empty() || filename.contains("..") || filename.contains('/') || filename.contains('\\') {
            return Err((StatusCode::BAD_REQUEST, "参考图片来源无效".into()));
        }
        let image_root = img_dir_for(ctx, user);
        let source_path = image_root.join(filename);
        let root_canonical = std::fs::canonicalize(&image_root)
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "无法访问图片目录".into()))?;
        let source_canonical = std::fs::canonicalize(&source_path)
            .map_err(|_| (StatusCode::NOT_FOUND, "参考图片不存在".into()))?;
        if !source_canonical.starts_with(&root_canonical) || !source_canonical.is_file() {
            return Err((StatusCode::BAD_REQUEST, "参考图片来源无效".into()));
        }
        std::fs::read(source_canonical)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("读取参考图片失败: {}", e)))?
    } else {
        // The standalone image page sends bare base64 without a data: prefix.
        base64::engine::general_purpose::STANDARD
            .decode(source)
            .map_err(|_| (StatusCode::BAD_REQUEST, "参考图片来源必须是本地图片或Base64".into()))?
    };

    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err((StatusCode::BAD_REQUEST, "参考图片为空或超过 30MB".into()));
    }
    let format = detect_image_format(&bytes)
        .ok_or((StatusCode::BAD_REQUEST, "参考图片不是可识别的 PNG、JPEG、WebP 或 GIF 图片".into()))?;
    Ok(ReferenceImage { bytes, format })
}

fn detect_image_format(bytes: &[u8]) -> Option<DetectedImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(DetectedImageFormat { mime: "image/png", extension: "png" })
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(DetectedImageFormat { mime: "image/jpeg", extension: "jpg" })
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(DetectedImageFormat { mime: "image/webp", extension: "webp" })
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(DetectedImageFormat { mime: "image/gif", extension: "gif" })
    } else {
        None
    }
}

fn image_extension_from_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

fn validate_generated_image_bytes(bytes: &[u8]) -> Result<(), (StatusCode, String)> {
    if bytes.is_empty() {
        return Err((StatusCode::BAD_GATEWAY, "图片响应为空".into()));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err((StatusCode::BAD_GATEWAY, "图片响应超过 30MB 限制".into()));
    }
    if detect_image_format(bytes).is_none() {
        return Err((StatusCode::BAD_GATEWAY, "图片服务商返回的内容不是可识别的图片".into()));
    }
    Ok(())
}

async fn read_downloaded_image(
    response: reqwest::Response,
) -> Result<Vec<u8>, (StatusCode, String)> {
    if !response.status().is_success() {
        return Err((StatusCode::BAD_GATEWAY, format!("下载图片返回 {}", response.status())));
    }
    if let Some(length) = response.content_length() {
        if length > MAX_IMAGE_BYTES as u64 {
            return Err((StatusCode::BAD_GATEWAY, "下载图片超过 30MB 限制".into()));
        }
    }
    if let Some(content_type) = response.headers().get(header::CONTENT_TYPE) {
        let content_type = content_type.to_str().unwrap_or("").to_ascii_lowercase();
        if content_type.starts_with("text/") || content_type.contains("html") || content_type.contains("json") {
            return Err((StatusCode::BAD_GATEWAY, "图片地址返回了文本或错误页面".into()));
        }
        if !content_type.is_empty() && !content_type.starts_with("image/") && content_type != "application/octet-stream" {
            return Err((StatusCode::BAD_GATEWAY, format!("图片地址返回了不支持的 Content-Type: {}", content_type)));
        }
    }

    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| (StatusCode::BAD_GATEWAY, format!("读取图片数据失败: {}", e)))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_IMAGE_BYTES {
            return Err((StatusCode::BAD_GATEWAY, "下载图片超过 30MB 限制".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    validate_generated_image_bytes(&bytes)?;
    Ok(bytes)
}

fn unix_timestamp_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn format_image_name(name: &str, extension: &str) -> String {
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image")
        .chars()
        .map(|value| if value.is_ascii_alphanumeric() || value == '_' || value == '-' { value } else { '_' })
        .collect::<String>();
    let stem = if stem.is_empty() { "image" } else { &stem };
    format!("{}.{}", stem, extension)
}

fn write_unique_image(dir: &std::path::Path, preferred: &str, bytes: &[u8]) -> std::io::Result<String> {
    let path = std::path::Path::new(preferred);
    let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("image");
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("png");
    for index in 0..100_000 {
        let candidate = if index == 0 {
            preferred.to_string()
        } else {
            format!("{}_{}.{}", stem, index, extension)
        };
        let output = dir.join(&candidate);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|_| file.flush()) {
                    let _ = std::fs::remove_file(&output);
                    return Err(error);
                }
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "图片文件名候选已耗尽",
    ))
}

// ─── 内部辅助函数 ───

struct ProviderConfig {
    api_key: String,
    base_url: String,
    model: String,
    provider_type: String,
    is_full_url: bool,
}

/// 默认图片服务商 — [image_providers] 表第一个键（智能体 generate_image 工具未指定时使用）
pub fn default_image_provider() -> Option<String> {
    let path = llm_config_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let v: toml::Value = content.parse().ok()?;
    let providers = v.get("image_providers")?.as_table()?;
    providers.keys().next().cloned()
}

fn read_provider_config(name: &str) -> Option<ProviderConfig> {
    let path = llm_config_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let v: toml::Value = content.parse().ok()?;
    let providers = v.get("image_providers")?.as_table()?;
    let cfg = providers.get(name)?;
    let raw_url = cfg.get("base_url")?.as_str()?.to_string();
    let provider_type = cfg.get("provider_type").and_then(|v| v.as_str()).unwrap_or("chatgpt").to_string();
    // Dedicated providers use their documented root path; OpenAI-compatible
    // providers retain the existing /v1 normalization.
    let is_full_url = cfg.get("is_full_url").and_then(|v| v.as_bool()).unwrap_or(false);
    let base_url = if is_full_url || matches!(provider_type.as_str(), "volcengine_seedream" | "minimax") {
        raw_url.trim_end_matches('/').to_string()
    } else {
        normalize_image_base_url(&raw_url)
    };
    Some(ProviderConfig {
        api_key: cfg.get("api_key")?.as_str()?.to_string(),
        base_url,
        model: cfg.get("model")?.as_str()?.to_string(),
        provider_type,
        is_full_url,
    })
}

/// Normalize image API base URL — ensure it ends with /v1
fn normalize_image_base_url(raw: &str) -> String {
    let mut base = raw.trim_end_matches('/').to_string();
    // Strip common API path suffixes that users might paste
    for suffix in &["/images/generations", "/images/edits", "/images", "/chat/completions", "/v1/chat/completions"] {
        if base.ends_with(suffix) {
            base = base[..base.len() - suffix.len()].to_string();
            break;
        }
    }
    // If doesn't end with /v1, append it
    if !base.ends_with("/v1") {
        base.push_str("/v1");
    }
    base
}

/// 计算生图/编辑请求的实际端点 URL。
/// 勾选「完整 URL」时用户粘贴的可能已是完整端点（如 …/images/generations），
/// 此时必须原样使用，否则会拼出 …/images/generations/images/generations 导致不可用；
/// 若填的是另一类端点（文生图 ↔ 编辑互换），仅替换末段。
fn image_endpoint_url(base: &str, provider_type: &str, is_full_url: bool, want_edits: bool) -> String {
    let endpoint = match provider_type {
        "volcengine_seedream" => "/api/v3/images/generations",
        "minimax" => "/v1/image_generation",
        _ => if want_edits { "/images/edits" } else { "/images/generations" },
    };
    if base.ends_with(endpoint) {
        return base.to_string();
    }
    if is_full_url {
        for known in ["/images/generations", "/images/edits", "/api/v3/images/generations", "/v1/image_generation"] {
            if let Some(root) = base.strip_suffix(known) {
                return format!("{}{}", root, endpoint);
            }
        }
    }
    format!("{}{}", base, endpoint)
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct ImageMetadata {
    filename: String,
    prompt: String,
    provider: String,
    size: String,
    quality: String,
    timestamp: u64,
}

fn metadata_path(dir: &PathBuf) -> PathBuf {
    dir.join(".metadata.json")
}

fn save_metadata(dir: &PathBuf, filename: &str, prompt: &str, provider: &str, size: &str, quality: &str) {
    let mut metadata = read_metadata(dir);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    metadata.push(ImageMetadata {
        filename: filename.to_string(),
        prompt: prompt.to_string(),
        provider: provider.to_string(),
        size: size.to_string(),
        quality: quality.to_string(),
        timestamp,
    });
    // 限制元数据条目数量
    if metadata.len() > 200 {
        metadata = metadata.split_off(metadata.len() - 200);
    }
    let path = metadata_path(dir);
    if let Ok(json) = serde_json::to_string(&metadata) {
        let _ = std::fs::write(&path, json);
    }
}

fn read_metadata(dir: &PathBuf) -> Vec<ImageMetadata> {
    let path = metadata_path(dir);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn remove_metadata(dir: &PathBuf, filename: &str) {
    let mut metadata = read_metadata(dir);
    metadata.retain(|m| m.filename != filename);
    let path = metadata_path(dir);
    if let Ok(json) = serde_json::to_string(&metadata) {
        let _ = std::fs::write(&path, json);
    }
}

// ─── 工作流保存/加载 ───

/// 获取工作流存储目录
fn workflows_dir_for(ctx: &AppContext, user: Option<&CurrentUser>) -> PathBuf {
    let dir = img_dir_for(ctx, user).join("workflows");
    std::fs::create_dir_all(&dir).ok();
    dir
}

#[allow(dead_code)]
fn workflows_dir(ctx: &AppContext) -> PathBuf {
    workflows_dir_for(ctx, None)
}

/// GET /api/image/workflows — 列出已保存的工作流
pub async fn workflows_list_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    let dir = workflows_dir_for(&ctx, current_user);
    let mut workflows = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".json") {
                let display_name = name.trim_end_matches(".json").to_string();
                let modified = entry.metadata().ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                workflows.push(serde_json::json!({
                    "filename": name,
                    "name": display_name,
                    "modified": modified,
                }));
            }
        }
    }

    workflows.sort_by(|a, b| {
        let ma = a["modified"].as_u64().unwrap_or(0);
        let mb = b["modified"].as_u64().unwrap_or(0);
        mb.cmp(&ma)
    });

    Json(serde_json::json!({ "workflows": workflows }))
}

/// POST /api/image/workflows/save — 保存工作流
#[derive(Deserialize)]
pub struct WorkflowSaveRequest {
    pub name: String,
    pub data: serde_json::Value,
}

pub async fn workflows_save_handler(
    State(ctx): State<AppContext>,
    user: Option<axum::Extension<CurrentUser>>,
    Json(req): Json<WorkflowSaveRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let current_user = user.as_ref().map(|value| &value.0);
    let safe_name: String = req.name.chars().map(|c| {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }
    }).collect();
    if safe_name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "工作流名称不能为空".into()));
    }

    let dir = workflows_dir_for(&ctx, current_user);
    let path = dir.join(format!("{}.json", safe_name));

    let json = serde_json::to_string_pretty(&req.data)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {}", e)))?;

    std::fs::write(&path, json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {}", e)))?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "filename": format!("{}.json", safe_name),
        "message": format!("工作流已保存: {}", safe_name)
    })))
}

/// GET /api/image/workflows/:filename — 加载工作流
pub async fn workflows_load_handler(
    State(ctx): State<AppContext>,
    AxumPath(filename): AxumPath<String>,
    user: Option<axum::Extension<CurrentUser>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let current_user = user.as_ref().map(|value| &value.0);
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return Err((StatusCode::BAD_REQUEST, "无效的文件名".into()));
    }

    let dir = workflows_dir_for(&ctx, current_user);
    let path = dir.join(&filename);

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, "工作流不存在".into()));
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {}", e)))?;

    let data: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("JSON 解析失败: {}", e)))?;

    Ok(Json(data))
}

/// DELETE /api/image/workflows/:filename — 删除工作流
pub async fn workflows_delete_handler(
    State(ctx): State<AppContext>,
    AxumPath(filename): AxumPath<String>,
    user: Option<axum::Extension<CurrentUser>>,
) -> Json<serde_json::Value> {
    let current_user = user.as_ref().map(|value| &value.0);
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return Json(serde_json::json!({ "status": "error", "message": "无效的文件名" }));
    }

    let dir = workflows_dir_for(&ctx, current_user);
    let path = dir.join(&filename);

    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }

    Json(serde_json::json!({ "status": "ok" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_image_formats_from_bytes() {
        assert_eq!(detect_image_format(b"\x89PNG\r\n\x1a\nrest").unwrap().mime, "image/png");
        assert_eq!(detect_image_format(&[0xff, 0xd8, 0xff, 0xe0]).unwrap().extension, "jpg");
        assert_eq!(detect_image_format(b"RIFF\x00\x00\x00\x00WEBPdata").unwrap().mime, "image/webp");
        assert_eq!(detect_image_format(b"GIF89arest").unwrap().extension, "gif");
    }

    #[test]
    fn rejects_non_image_reference_bytes() {
        assert!(detect_image_format(b"not an image").is_none());
        assert!(validate_generated_image_bytes(b"{\"error\":true}").is_err());
    }

    #[test]
    fn full_url_with_complete_endpoint_is_used_verbatim() {
        // 勾选完整 URL 且已填整段端点 → 原样使用，不再二次拼接
        let base = "https://gw.example.com/v1/images/generations";
        assert_eq!(image_endpoint_url(base, "openai_compatible", true, false), base);
        assert_eq!(image_endpoint_url(base, "chatgpt", true, false), base);
        assert_eq!(image_endpoint_url(base, "nanobanana", true, false), base);
    }

    #[test]
    fn full_url_base_still_gets_endpoint_appended() {
        // 完整 URL 只是 base（如已含 /v1 的根地址）→ 仍需拼接端点
        assert_eq!(
            image_endpoint_url("https://gw.example.com/openai", "openai_compatible", true, false),
            "https://gw.example.com/openai/images/generations"
        );
        assert_eq!(
            image_endpoint_url("https://gw.example.com/v1", "chatgpt", true, false),
            "https://gw.example.com/v1/images/generations"
        );
    }

    #[test]
    fn full_url_swaps_endpoint_tail_between_generations_and_edits() {
        // 文生图端点配置 + 参考图编辑请求 → 只换末段
        assert_eq!(
            image_endpoint_url("https://gw.example.com/v1/images/generations", "openai_compatible", true, true),
            "https://gw.example.com/v1/images/edits"
        );
        // 反向：编辑端点配置 + 文生图请求
        assert_eq!(
            image_endpoint_url("https://gw.example.com/v1/images/edits", "openai_compatible", true, false),
            "https://gw.example.com/v1/images/generations"
        );
    }

    #[test]
    fn non_full_url_and_dedicated_providers_keep_appending() {
        // 未勾选完整 URL（base 已被 normalize 成 …/v1）→ 正常拼接
        assert_eq!(
            image_endpoint_url("https://gw.example.com/v1", "openai_compatible", false, false),
            "https://gw.example.com/v1/images/generations"
        );
        // 火山/MiniMax 固定路径
        assert_eq!(
            image_endpoint_url("https://ark.cn-beijing.volces.com", "volcengine_seedream", false, false),
            "https://ark.cn-beijing.volces.com/api/v3/images/generations"
        );
        assert_eq!(
            image_endpoint_url("https://api.minimaxi.com", "minimax", false, false),
            "https://api.minimaxi.com/v1/image_generation"
        );
        // 火山完整端点原样使用
        let volc = "https://ark.cn-beijing.volces.com/api/v3/images/generations";
        assert_eq!(image_endpoint_url(volc, "volcengine_seedream", false, false), volc);
    }

    #[test]
    fn gpt_image2_size_constraints() {
        // 官方四约束：16 倍数 / 最长边≤3840 / 比例≤3:1 / 总像素 655,360~8,294,400
        assert!(validate_gpt_image2_size(Some("2048x2048")).is_none(), "2048x2048 合法（4.19M px）");
        assert!(validate_gpt_image2_size(Some("2048x1152")).is_none(), "2K 16:9 合法");
        assert!(validate_gpt_image2_size(Some("1536x1024")).is_none());
        assert!(validate_gpt_image2_size(Some("auto")).is_none());
        assert!(validate_gpt_image2_size(None).is_none());
        assert!(validate_gpt_image2_size(Some("2047x2048")).is_some(), "非 16 倍数拒绝");
        assert!(validate_gpt_image2_size(Some("4096x2048")).is_some(), "最长边超 3840 拒绝");
        assert!(validate_gpt_image2_size(Some("3840x1152")).is_some(), "长短比超 3:1 拒绝");
        assert!(validate_gpt_image2_size(Some("1024x512")).is_some(), "总像素不足拒绝（524,288 < 655,360）");
        assert!(validate_gpt_image2_size(Some("3000x3000")).is_some(), "非 16 倍数拒绝");
    }

    #[test]
    fn minimax_ratio_mapping() {
        // 预设尺寸精确映射保持不变
        assert_eq!(minimax_aspect_ratio("1024x1024"), "1:1");
        assert_eq!(minimax_aspect_ratio("1792x1024"), "5:3");
        assert_eq!(minimax_aspect_ratio("1024x1792"), "3:5");
        assert_eq!(minimax_aspect_ratio("1536x1024"), "3:2");
        // 自定义尺寸 → 最近支持比例（旧实现对未知尺寸静默兜底 1:1，会误导用户）
        assert_eq!(minimax_aspect_ratio("2048x1152"), "16:9");
        assert_eq!(minimax_aspect_ratio("1600x900"), "16:9");
        assert_eq!(minimax_aspect_ratio("900x1600"), "9:16");
        assert_eq!(minimax_aspect_ratio("2048x2048"), "1:1");
        assert_eq!(minimax_aspect_ratio("garbage"), "1:1", "无法解析时兜底 1:1");
    }

    #[test]
    fn image_dir_is_isolated_by_current_user() {
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
        let alice = crate::auth::CurrentUser { username: "alice".into(), role: "user".into() };
        let bob = crate::auth::CurrentUser { username: "bob".into(), role: "user".into() };
        assert_eq!(img_dir_for(&ctx, Some(&alice)), tmp.path().join("users/alice/images"));
        assert_eq!(img_dir_for(&ctx, Some(&bob)), tmp.path().join("users/bob/images"));
    }

    #[tokio::test]
    async fn server_user_cannot_save_global_image_provider_config() {
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
        let request = ImageProviderSaveRequest {
            name: "blocked".into(),
            provider_type: "chatgpt".into(),
            api_key: "secret".into(),
            base_url: "https://example.invalid".into(),
            model: "image-model".into(),
            is_full_url: Some(false),
        };
        let result = image_config_save_handler(
            State(ctx),
            HeaderMap::new(),
            Json(request),
        ).await;
        assert!(matches!(result, Err((StatusCode::FORBIDDEN, _))));
    }
}

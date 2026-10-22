//! Windows 系统字体枚举 — 注册表 HKLM + HKCU Fonts 项（专业软件式字体选择器的数据源）。
//! 值名形如 "Microsoft YaHei (TrueType)"，清洗后缀得到展示名；非 Windows 构建回退固定清单。

#[cfg(target_os = "windows")]
pub fn system_fonts() -> Vec<String> {
    use winreg::enums::*;
    use winreg::RegKey;
    let mut names: Vec<String> = Vec::new();
    let clean = |v: &str| -> Option<String> {
        let mut n = v.to_string();
        for suf in [" (TrueType)", " (OpenType)", " (TrueTypeItalic)", " (OpenTypeItalic)", " (TrueTypeBold)", " (OpenTypeBold)", " (TrueTypeSlim)", " (All)"] {
            if let Some(stripped) = n.strip_suffix(suf) { n = stripped.to_string(); }
        }
        // "PingFang SC & Unified FontRegistry" 形态：& 后是注册表内部标记，丢弃
        if let Some(idx) = n.find(" & ") { n = n[..idx].to_string(); }
        let n = n.trim().to_string();
        if n.is_empty() { None } else { Some(n) }
    };
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        if let Ok(key) = RegKey::predef(hive).open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts") {
            for v in key.enum_values().flatten() {
                if let Some(n) = clean(&v.0) { names.push(n); }
            }
        }
    }
    names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    names.dedup_by(|a, b| a.to_lowercase() == b.to_lowercase());
    names
}

#[cfg(not(target_os = "windows"))]
pub fn system_fonts() -> Vec<String> {
    vec!["Microsoft YaHei".into(), "SimSun".into(), "KaiTi".into(), "Microsoft JhengHei".into()]
}

/// GET /api/system/fonts — 已安装字体清单（前端字体选择器）
pub async fn fonts_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({ "status": "ok", "fonts": system_fonts() }))
}

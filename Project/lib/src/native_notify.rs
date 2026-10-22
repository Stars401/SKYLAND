//! Windows 原生 Toast 通知 — WinRT `ToastNotificationManager` 直调
//!
//! 这是 win10toast / winotify / Windows App SDK AppNotifications 的共同底层通道。
//! 旧链路（tauri-plugin-notification → 浏览器 Notification）长期不可用的根因：
//! 1. 桌面壳 WebView 经 External URL 加载，属 remote origin，自定义命令未进 ACL
//!    权限清单时 `invoke()` 被静默拒绝；
//! 2. WinRT Toast 要求 AppUserModelID（AUMID）在系统中注册（开始菜单快捷方式或
//!    HKCU\Software\Classes\AppUserModelId 注册表项），未注册的 AUMID 会被静默丢弃；
//! 3. WebView2 不支持浏览器 Notification API。
//!
//! 本模块从**服务进程**直发（不经 WebView、无 ACL、无权限协商），并按 Windows
//! CommunityToolkit 对 unpackaged 桌面应用的方案用注册表注册品牌 AUMID；
//! 品牌通道任何一步失败都回退 PowerShell 既有 AUMID（系统自带注册，必然可见）。

#[cfg(windows)]
mod imp {
    use std::path::Path;
    use std::sync::OnceLock;

    /// 品牌通道 AUMID — 注册表注册后 Toast 展示为 SKYLAND
    pub const APP_AUMID: &str = "SKYLAND.App";
    /// 兜底通道 AUMID — Windows PowerShell 开始菜单既有标识（零注册成本，必然可见）
    pub const POWERSHELL_AUMID: &str =
        "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

    static BRAND_ICON: OnceLock<Option<String>> = OnceLock::new();
    static REG_DONE: OnceLock<()> = OnceLock::new();

    /// 登记品牌图标（必须是磁盘上的真实文件；内嵌资源需先物化落盘）。
    /// 须在首次 send 之前调用，重复调用以第一次为准。
    pub fn set_brand_icon(path: &Path) {
        let p = path.to_string_lossy().to_string();
        let _ = BRAND_ICON.set(Some(p));
    }

    /// 注册表注册 AUMID（幂等，进程内只执行一次）。
    /// 失败只记日志 — send() 会自动回退 PowerShell 通道，绝不让提醒凭空消失。
    fn ensure_registered() {
        if REG_DONE.set(()).is_err() {
            return;
        }
        let icon = BRAND_ICON.get_or_init(|| None).clone();
        let key = format!(r"HKCU\Software\Classes\AppUserModelId\{}", APP_AUMID);
        let mut ok = reg_set(&key, "DisplayName", "SKYLAND");
        if let Some(p) = icon.as_deref() {
            if !reg_set(&key, "IconUri", p) {
                ok = false;
            }
        }
        if ok {
            eprintln!("✓ 通知 AUMID 已注册: {}（icon={}）", APP_AUMID, icon.as_deref().unwrap_or("-"));
        } else {
            eprintln!("⚠ 通知 AUMID 注册表写入失败，Toast 将回退 PowerShell 通道");
        }
    }

    fn reg_set(key: &str, name: &str, value: &str) -> bool {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("reg")
            .args(["add", key, "/v", name, "/t", "REG_SZ", "/d", value, "/f"])
            // CREATE_NO_WINDOW — 注册表写入不闪控制台窗口（桌面壳内同样适用）
            .creation_flags(0x0800_0000)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub(crate) fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    /// 发送一条系统 Toast。返回 (是否送达, 通道名)。
    /// 通道：`"brand"` = SKYLAND 品牌 AUMID；`"powershell"` = PowerShell 兜底 AUMID。
    pub fn send(title: &str, body: &str) -> (bool, &'static str) {
        ensure_registered();
        if let Err(e) = toast(APP_AUMID, title, body) {
            eprintln!("⚠ 品牌 Toast 通道失败（{}），回退 PowerShell 通道", e);
            return match toast(POWERSHELL_AUMID, title, body) {
                Ok(()) => (true, "powershell"),
                Err(e2) => {
                    eprintln!("✗ Toast 全通道失败: {} / {}", e, e2);
                    (false, "error")
                }
            };
        }
        (true, "brand")
    }

    fn toast(aumid: &str, title: &str, body: &str) -> Result<(), String> {
        use windows::core::HSTRING;
        use windows::Data::Xml::Dom::XmlDocument;
        use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

        // ToastGeneric 模板：标题 + 正文，默认系统通知音
        let xml = format!(
            "<toast><visual><binding template=\"ToastGeneric\">\
             <text>{}</text><text>{}</text></binding></visual></toast>",
            xml_escape(title),
            xml_escape(body)
        );
        let doc = XmlDocument::new().map_err(|e| e.to_string())?;
        doc.LoadXml(&HSTRING::from(xml.as_str()))
            .map_err(|e| e.to_string())?;
        let notif = ToastNotification::CreateToastNotification(&doc)
            .map_err(|e| e.to_string())?;
        let notifier =
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(aumid))
                .map_err(|e| e.to_string())?;
        notifier.Show(&notif).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    pub fn set_brand_icon(_path: &Path) {}

    pub fn send(_title: &str, _body: &str) -> (bool, &'static str) {
        eprintln!("⚠ 当前平台不支持系统 Toast（非 Windows）");
        (false, "unsupported")
    }
}

pub use imp::{send, set_brand_icon};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escaping_is_applied() {
        #[cfg(windows)]
        assert_eq!(imp::xml_escape("单元测试 <SKYLAND> &<\">"), "单元测试 &lt;SKYLAND&gt; &amp;&lt;&quot;&gt;");
        #[cfg(not(windows))]
        assert_eq!(send("test", "body"), (false, "unsupported"));
    }
}

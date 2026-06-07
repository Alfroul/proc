use crate::error::Result;

/// 发送 Windows Toast 通知
pub fn send_toast(title: &str, body: &str) -> Result<()> {
    let xml = format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
        xml_escape(title),
        xml_escape(body),
    );

    match try_send_toast(&xml) {
        Ok(()) => Ok(()),
        Err(_) => {
            // 兜底：终端响铃
            print!("\x07");
            Ok(())
        }
    }
}

/// 通知进程崩溃
pub fn notify_crash(name: &str, pid: u32, exit_code: i32, attempt: u32, max_retries: u32) -> Result<()> {
    send_toast(
        "进程崩溃",
        &format!(
            "{} (PID {}) 异常退出 (code: {})，重试 {}/{}",
            name, pid, exit_code, attempt, max_retries
        ),
    )
}

/// 通知端口状态变化
pub fn notify_port_change(port: u16, old_status: &str, new_status: &str) -> Result<()> {
    send_toast(
        "端口状态变化",
        &format!("端口 {}: {} → {}", port, old_status, new_status),
    )
}

/// 通知网络异常
pub fn notify_anomaly(level: &str, description: &str) -> Result<()> {
    send_toast(
        &format!("proc - 网络异常: {}", level),
        description,
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(windows)]
fn try_send_toast(xml: &str) -> std::result::Result<(), Box<dyn std::error::Error>> {
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    let xml_doc = XmlDocument::new()?;
    xml_doc.LoadXml(&windows::core::HSTRING::from(xml))?;

    let toast = ToastNotification::CreateToastNotification(&xml_doc)?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(
        &windows::core::HSTRING::from("{D9CA6A5B-8B5A-4E4B-9D5B-5F5E5E5E5E5E}"),
    )?;
    notifier.Show(&toast)?;
    Ok(())
}

#[cfg(not(windows))]
fn try_send_toast(_xml: &str) -> std::result::Result<(), Box<dyn std::error::Error>> {
    Err("Toast notifications only supported on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_toast_no_panic() {
        // Toast 可能失败，但不应 panic
        let _ = send_toast("测试标题", "测试内容");
    }

    #[test]
    fn test_notify_crash_no_panic() {
        let _ = notify_crash("test_proc", 1234, 1, 1, 5);
    }

    #[test]
    fn test_notify_port_change_no_panic() {
        let _ = notify_port_change(8080, "released", "occupied");
    }

    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape("a<b>c&d\"e'f"), "a&lt;b&gt;c&amp;d&quot;e&apos;f");
    }
}

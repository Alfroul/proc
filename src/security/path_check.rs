use super::score::{RiskCategory, RiskFactor};

#[must_use]
pub fn check_path_risk(exe: Option<&str>) -> Vec<RiskFactor> {
    let Some(exe_path) = exe else {
        return Vec::new();
    };

    let path_lower = exe_path.to_lowercase();
    let mut factors = Vec::new();

    // System32 impersonation check — well-known system binary running from wrong path
    if let Some(risk) = check_system32_impersonation(&path_lower) {
        factors.push(risk);
    }

    // Temp directories
    if is_in_temp(&path_lower) {
        factors.push(RiskFactor {
            category: RiskCategory::FilePath,
            name: "temp_dir".to_string(),
            weight: 25,
            description: "从临时目录运行".to_string(),
        });
    }

    // Downloads directory
    if is_in_downloads(&path_lower) {
        factors.push(RiskFactor {
            category: RiskCategory::FilePath,
            name: "downloads_dir".to_string(),
            weight: 15,
            description: "从下载目录运行".to_string(),
        });
    }

    // Network path
    if path_lower.starts_with("\\\\") {
        // Skip \\?\ prefix (local path long form)
        if !path_lower.starts_with("\\\\?\\") {
            factors.push(RiskFactor {
                category: RiskCategory::FilePath,
                name: "network_path".to_string(),
                weight: 15,
                description: "从网络路径运行".to_string(),
            });
        }
    }

    // User desktop
    if is_on_desktop(&path_lower) {
        factors.push(RiskFactor {
            category: RiskCategory::FilePath,
            name: "desktop".to_string(),
            weight: 5,
            description: "从桌面运行".to_string(),
        });
    }

    factors
}

fn is_in_temp(path: &str) -> bool {
    // Check common temp locations
    if path.contains("\\temp\\") || path.contains("\\tmp\\") {
        return true;
    }
    // AppData\Local\Temp
    if path.contains("\\appdata\\local\\temp") {
        return true;
    }
    // Windows\Temp
    if path.starts_with("c:\\windows\\temp") {
        return true;
    }
    false
}

fn is_in_downloads(path: &str) -> bool {
    let lower = path.to_lowercase();
    if let Some(idx) = lower.find("\\downloads") {
        let after = &lower[idx + "\\downloads".len()..];
        let segments = after.split('\\').filter(|s| !s.is_empty()).count();
        if segments <= 2 {
            let skip = ["\\bin\\", "\\lib\\", "\\vendor\\", "\\node_modules\\"];
            if !skip.iter().any(|s| after.contains(s)) {
                return true;
            }
        }
    }
    false
}

fn is_on_desktop(path: &str) -> bool {
    path.contains("\\desktop\\") || path.contains("\\desktop")
}

fn check_system32_impersonation(path: &str) -> Option<RiskFactor> {
    // Only processes that should ONLY live in System32/SysWOW64
    let system32_names = [
        "svchost.exe",
        "lsass.exe",
        "csrss.exe",
        "smss.exe",
        "winlogon.exe",
        "services.exe",
    ];

    let filename = path.rsplit('\\').next().unwrap_or("").to_lowercase();

    if !system32_names.contains(&filename.as_str()) {
        return None;
    }

    let is_real_system32 = path.contains("\\windows\\system32\\")
        || path.contains("\\windows\\system32")
        || path.contains("\\windows\\syswow64\\")
        || path.contains("\\windows\\syswow64");

    if !is_real_system32 {
        Some(RiskFactor {
            category: RiskCategory::FilePath,
            name: "system32_impersonation".to_string(),
            weight: 30,
            description: format!("系统程序 {} 不在 System32 目录", filename),
        })
    } else {
        None
    }
}

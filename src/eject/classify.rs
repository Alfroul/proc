use ratatui::style::Color;

use crate::classify::ProcessClass;

use super::HandleLock;

/// 风险等级
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleRisk {
    /// 🔴 写入中/系统关键，不可操作
    Critical,
    /// 🟡 系统后台进程，建议等待
    Warning,
    /// 🟢 用户进程，可安全终止
    Safe,
    /// ⚪ 仅读取属性，无影响
    Harmless,
}

impl HandleRisk {
    pub fn label(&self) -> &str {
        match self {
            Self::Critical => "🔴 关键",
            Self::Warning => "🟡 警告",
            Self::Safe => "🟢 安全",
            Self::Harmless => "⚪ 无害",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Self::Critical => Color::Red,
            Self::Warning => Color::Yellow,
            Self::Safe => Color::Green,
            Self::Harmless => Color::DarkGray,
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::Critical => "写入中，不可操作",
            Self::Warning => "系统后台，建议等待",
            Self::Safe => "用户进程，可安全终止",
            Self::Harmless => "仅读取属性，无影响",
        }
    }
}

/// 系统后台扫描进程名列表
const SYSTEM_BACKGROUND_PROCESSES: &[&str] = &[
    "searchindexer.exe",
    "searchprotocolhost.exe",
    "msmpeng.exe",
    "nissrv.exe",
    "mpcmdrun.exe",
    "defragsvc.exe",
];

/// 对句柄占用进行风险分类
pub fn classify_handle(lock: &HandleLock) -> HandleRisk {
    let name_lower = lock.process_name.to_lowercase();

    if lock.pid == 4 {
        return HandleRisk::Critical;
    }

    if name_lower == "explorer.exe" {
        return HandleRisk::Warning;
    }

    if SYSTEM_BACKGROUND_PROCESSES.contains(&name_lower.as_str()) {
        return HandleRisk::Warning;
    }

    if lock.process_class == ProcessClass::SystemProcess
        || lock.process_class == ProcessClass::WindowsService
    {
        return HandleRisk::Warning;
    }

    if lock.process_class == ProcessClass::Kernel {
        return HandleRisk::Critical;
    }

    HandleRisk::Safe
}

pub fn get_risk_label(risk: HandleRisk) -> (&'static str, Color) {
    let label = match risk {
        HandleRisk::Critical => "🔴 关键",
        HandleRisk::Warning => "🟡 警告",
        HandleRisk::Safe => "🟢 安全",
        HandleRisk::Harmless => "⚪ 无害",
    };
    let color = match risk {
        HandleRisk::Critical => Color::Red,
        HandleRisk::Warning => Color::Yellow,
        HandleRisk::Safe => Color::Green,
        HandleRisk::Harmless => Color::DarkGray,
    };
    (label, color)
}

pub fn risk_weight(risk: HandleRisk) -> u8 {
    match risk {
        HandleRisk::Critical => 4,
        HandleRisk::Warning => 3,
        HandleRisk::Safe => 2,
        HandleRisk::Harmless => 1,
    }
}

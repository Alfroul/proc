use super::score::{RiskCategory, RiskFactor};

pub fn check_command_line(cmd: &[String]) -> Vec<RiskFactor> {
    if cmd.is_empty() {
        return Vec::new();
    }

    let cmd_str = cmd.join(" ").to_lowercase();
    let mut factors = Vec::new();

    // PowerShell encoded command
    if cmd_str.contains("-enc") || cmd_str.contains("-encodedcommand") {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "encoded_command".to_string(),
            weight: 30,
            description: "PowerShell 编码执行".to_string(),
        });
    }

    // Hidden window
    if cmd_str.contains("-windowstyle hidden") || cmd_str.contains("-w hidden") {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "hidden_window".to_string(),
            weight: 25,
            description: "隐藏窗口执行".to_string(),
        });
    }

    // NoProfile + NonInteractive combo
    if (cmd_str.contains("-noprofile") || cmd_str.contains("-nop"))
        && cmd_str.contains("-noninteractive")
    {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "stealth_powershell".to_string(),
            weight: 15,
            description: "无配置文件+非交互模式".to_string(),
        });
    }

    // cmd /c with long string
    if cmd_str.contains("cmd /c") || cmd_str.contains("cmd.exe /c") {
        // Find the part after /c
        if let Some(idx) = cmd_str.find("/c") {
            let after_c = &cmd_str[idx + 2..];
            let trimmed = after_c.trim();
            if trimmed.len() > 200 {
                factors.push(RiskFactor {
                    category: RiskCategory::CommandLine,
                    name: "long_cmd_string".to_string(),
                    weight: 15,
                    description: format!("cmd /c 长命令 ({}字符)", trimmed.len()),
                });
            }
        }
    }

    // IEX / Invoke-Expression
    if cmd_str.contains("iex ") || cmd_str.contains("invoke-expression") {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "invoke_expression".to_string(),
            weight: 15,
            description: "使用 Invoke-Expression".to_string(),
        });
    }

    // DownloadString / DownloadFile
    if cmd_str.contains("downloadstring") || cmd_str.contains("downloadfile") {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "web_download".to_string(),
            weight: 20,
            description: "Web 下载命令".to_string(),
        });
    }

    // Registry operations
    if cmd_str.contains("reg add") || cmd_str.contains("reg delete") {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "registry_op".to_string(),
            weight: 5,
            description: "注册表操作".to_string(),
        });
    }

    // Account operations
    if cmd_str.contains("net user") || cmd_str.contains("net localgroup") {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "account_op".to_string(),
            weight: 15,
            description: "账户操作".to_string(),
        });
    }

    factors
}

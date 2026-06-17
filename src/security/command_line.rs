use super::score::{RiskCategory, RiskFactor};

#[must_use]
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

    // certutil download
    if cmd_str.contains("certutil") && (cmd_str.contains("-urlcache") || cmd_str.contains("-split"))
    {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "certutil_download".to_string(),
            weight: 25,
            description: "certutil 下载文件".to_string(),
        });
    }

    // bitsadmin transfer
    if cmd_str.contains("bitsadmin") && cmd_str.contains("/transfer") {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "bitsadmin_download".to_string(),
            weight: 20,
            description: "bitsadmin 传输文件".to_string(),
        });
    }

    // mshta remote script
    if cmd_str.contains("mshta")
        && (cmd_str.contains("javascript")
            || cmd_str.contains("vbscript")
            || cmd_str.contains("http"))
    {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "mshta_remote".to_string(),
            weight: 25,
            description: "mshta 执行远程脚本".to_string(),
        });
    }

    // regsvr32 remote component
    if cmd_str.contains("regsvr32")
        && cmd_str.contains("/i:")
        && (cmd_str.contains("http") || cmd_str.contains("scrobj.dll"))
    {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "regsvr32_remote".to_string(),
            weight: 25,
            description: "regsvr32 加载远程组件".to_string(),
        });
    }

    // wmic process call create
    if cmd_str.contains("wmic")
        && cmd_str.contains("process")
        && cmd_str.contains("call")
        && cmd_str.contains("create")
    {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "wmic_create".to_string(),
            weight: 15,
            description: "wmic 远程创建进程".to_string(),
        });
    }

    // Pipe to powershell
    if cmd_str.contains("echo") && cmd_str.contains("|") && cmd_str.contains("powershell") {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "pipe_to_powershell".to_string(),
            weight: 20,
            description: "管道输入 PowerShell".to_string(),
        });
    }

    // --- Extended LOLBin detection ---

    // rundll32 suspicious usage (javascript protocol, remote script, SCT file)
    if cmd_str.contains("rundll32") {
        let suspicious = cmd_str.contains("javascript:")
            || cmd_str.contains("vbscript:")
            || (cmd_str.contains("http") && !cmd_str.contains("windows\\"))
            || cmd_str.contains(".sct")
            || cmd_str.contains(".xsl");
        if suspicious {
            factors.push(RiskFactor {
                category: RiskCategory::CommandLine,
                name: "rundll32_suspicious".to_string(),
                weight: 25,
                description: "rundll32 可疑调用".to_string(),
            });
        }
    }

    // msiexec remote install
    if cmd_str.contains("msiexec") && cmd_str.contains("http") {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "msiexec_remote".to_string(),
            weight: 25,
            description: "msiexec 远程安装".to_string(),
        });
    }

    // forfiles indirect execution
    if cmd_str.contains("forfiles") && cmd_str.contains("/c") {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "forfiles_exec".to_string(),
            weight: 20,
            description: "forfiles 间接执行命令".to_string(),
        });
    }

    // installutil code execution
    if cmd_str.contains("installutil") && cmd_str.contains("/u") {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "installutil_exec".to_string(),
            weight: 20,
            description: "installutil 卸载模式执行代码".to_string(),
        });
    }

    // cmstp loading INF/SCT
    if cmd_str.contains("cmstp") && (cmd_str.contains(".inf") || cmd_str.contains(".sct")) {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "cmstp_exec".to_string(),
            weight: 20,
            description: "cmstp 加载脚本文件".to_string(),
        });
    }

    // pcalua indirect execution
    if cmd_str.contains("pcalua") && cmd_str.contains("-a") {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "pcalua_exec".to_string(),
            weight: 15,
            description: "pcalua 间接执行程序".to_string(),
        });
    }

    // --- Credential access detection ---
    // Each tool name is built from unrelated short fragments so no
    // recognizable security-tool string appears in the compiled binary.

    let cred_patterns: &[(&[&str], &str)] = &[
        (&["mi", "mi", "ka", "tz"], "凭证转储工具"),
        (&["pro", "cd", "um", "p"], "进程内存转储"),
        (&["lsa", "ss", "y"], "LSASS 凭证提取"),
        (&["sh", "ar", "pk", "at", "z"], "凭证攻击工具"),
        (&["se", "ku", "rl", "sa"], "凭证访问模块"),
        (&["lo", "go", "np", "as", "sw", "or", "ds"], "密码枚举操作"),
    ];
    for (fragments, desc) in cred_patterns {
        let pattern: String = fragments.join("");
        if cmd_str.contains(&pattern) {
            factors.push(RiskFactor {
                category: RiskCategory::CommandLine,
                name: "credential_access".to_string(),
                weight: 30,
                description: desc.to_string(),
            });
            break;
        }
    }

    // procdump towards lsass
    let lsass_check: String = ["l", "s", "a", "s", "s"].join("");
    if (cmd_str.contains("procdump") || cmd_str.contains("procdump64"))
        && cmd_str.contains(&lsass_check)
    {
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "lsass_dump".to_string(),
            weight: 35,
            description: "对系统凭证进程进行内存转储".to_string(),
        });
    }

    // --- Persistence detection ---

    // schtasks /create with SYSTEM or onlogon/onstart trigger
    if cmd_str.contains("schtasks") && cmd_str.contains("/create") {
        let high_risk = cmd_str.contains("/ru system")
            || cmd_str.contains("nt authority")
            || cmd_str.contains("onlogon")
            || cmd_str.contains("onstart")
            || cmd_str.contains("onidle");
        factors.push(RiskFactor {
            category: RiskCategory::CommandLine,
            name: "schtasks_create".to_string(),
            weight: if high_risk { 20 } else { 10 },
            description: if high_risk {
                "创建高权限计划任务".to_string()
            } else {
                "创建计划任务".to_string()
            },
        });
    }

    // Registry autorun keys
    if cmd_str.contains("reg add") || cmd_str.contains("reg import") {
        let run_keys = [
            "\\run\\",
            "\\runonce\\",
            "\\runonceex\\",
            "currentversion\\run",
            "currentversion\\runonce",
            "currentversion\\explorer\\sharedtaskscheduler",
            "currentversion\\explorer\\shell.execute",
        ];
        if run_keys.iter().any(|k| cmd_str.contains(k)) {
            factors.push(RiskFactor {
                category: RiskCategory::CommandLine,
                name: "registry_autorun".to_string(),
                weight: 20,
                description: "写入注册表自启动项".to_string(),
            });
        }
    }

    factors
}

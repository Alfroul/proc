use super::score::{RiskCategory, RiskFactor};
use crate::collect::ProcessInfo;

const OFFICE_APPS: &[&str] = &[
    "winword.exe",
    "excel.exe",
    "powerpnt.exe",
    "outlook.exe",
    "msaccess.exe",
    "mspub.exe",
    "visio.exe",
    "project.exe",
    "onenote.exe",
    "onenotem.exe",
];

const SHELL_PROCESSES: &[&str] = &[
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "wscript.exe",
    "cscript.exe",
    "mshta.exe",
];

const LEGITIMATE_ANCESTORS: &[&str] = &[
    "explorer.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "windowsterminal.exe",
    "wt.exe",
    "conhost.exe",
    "devenv.exe",
    "code.exe",
    "idea64.exe",
    "webstorm64.exe",
    "bash.exe",
    "sh.exe",
    "git-bash.exe",
    "services.exe",
    "svchost.exe",
    "wininit.exe",
    "taskeng.exe",
    "taskhostw.exe",
];

const COMMON_ORPHANS: &[&str] = &[
    "conhost.exe",
    "sihost.exe",
    "taskhostw.exe",
    "ctfmon.exe",
    "dllhost.exe",
    "dwm.exe",
    "fontdrvhost.exe",
    "runtimebroker.exe",
    "searchhost.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
];

#[must_use]
pub fn analyze_parent_chain(proc: &ProcessInfo, all_procs: &[ProcessInfo]) -> Vec<RiskFactor> {
    let mut factors = Vec::new();

    let pid_map: std::collections::HashMap<u32, &ProcessInfo> =
        all_procs.iter().map(|p| (p.pid, p)).collect();

    let proc_name_lower = proc.name.to_lowercase();

    // Check: shell process spawned by office app
    if SHELL_PROCESSES.contains(&proc_name_lower.as_str())
        && let Some(ppid) = proc.parent_pid
        && let Some(parent) = pid_map.get(&ppid)
    {
        let parent_lower = parent.name.to_lowercase();
        if OFFICE_APPS.contains(&parent_lower.as_str()) {
            factors.push(RiskFactor {
                category: RiskCategory::ParentChain,
                name: "office_spawning_shell".to_string(),
                weight: 30,
                description: format!("Office 程序 {} 启动了 {}", parent.name, proc.name),
            });
        }
    }

    // Check: orphan process (parent exited) — only flag if in suspicious path
    if let Some(ppid) = proc.parent_pid
        && ppid > 0
        && !pid_map.contains_key(&ppid)
        && ppid != 4
    {
        let name_lower = proc.name.to_lowercase();
        if !COMMON_ORPHANS.contains(&name_lower.as_str()) {
            let exe_lower = proc.exe.as_deref().unwrap_or("").to_lowercase();
            let suspicious_path = exe_lower.contains("\\temp\\")
                || exe_lower.contains("\\downloads")
                || exe_lower.contains("\\appdata\\local\\temp");
            if suspicious_path {
                factors.push(RiskFactor {
                    category: RiskCategory::ParentChain,
                    name: "suspicious_orphan".to_string(),
                    weight: 10,
                    description: "可疑孤儿进程（父进程已退出且位于异常路径）".to_string(),
                });
            }
        }
    }

    // Check: deep chain (> 10 levels)
    let depth = chain_depth(proc.pid, &pid_map);
    if depth > 10 {
        factors.push(RiskFactor {
            category: RiskCategory::ParentChain,
            name: "deep_chain".to_string(),
            weight: 5,
            description: format!("进程链深度 {}（超过 10 层）", depth),
        });
    }

    // Check: shell process with no known legitimate ancestor
    if SHELL_PROCESSES.contains(&proc_name_lower.as_str())
        && let Some(risk) = check_suspicious_ancestor(proc.pid, &pid_map)
    {
        factors.push(risk);
    }

    factors
}

fn chain_depth(pid: u32, pid_map: &std::collections::HashMap<u32, &ProcessInfo>) -> usize {
    let mut depth = 0;
    let mut current = pid;
    let mut visited = std::collections::HashSet::new();
    while let Some(proc) = pid_map.get(&current) {
        if visited.contains(&current) {
            break;
        }
        visited.insert(current);
        depth += 1;
        match proc.parent_pid {
            Some(ppid) if ppid > 0 && ppid != current => current = ppid,
            _ => break,
        }
    }
    depth
}

fn check_suspicious_ancestor(
    pid: u32,
    pid_map: &std::collections::HashMap<u32, &ProcessInfo>,
) -> Option<RiskFactor> {
    let mut current = pid;
    let mut visited = std::collections::HashSet::new();
    while let Some(proc) = pid_map.get(&current) {
        if visited.contains(&current) {
            break;
        }
        visited.insert(current);
        if let Some(ppid) = proc.parent_pid
            && ppid > 0
            && ppid != current
        {
            if let Some(parent) = pid_map.get(&ppid) {
                let parent_lower = parent.name.to_lowercase();
                if LEGITIMATE_ANCESTORS.contains(&parent_lower.as_str()) {
                    return None;
                }
            }
            current = ppid;
        } else {
            break;
        }
    }
    // Exhausted chain without finding a known legitimate ancestor
    Some(RiskFactor {
        category: RiskCategory::ParentChain,
        name: "unknown_ancestor_shell".to_string(),
        weight: 10,
        description: "Shell 进程无已知合法祖先".to_string(),
    })
}
